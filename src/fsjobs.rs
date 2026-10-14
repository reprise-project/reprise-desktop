//! File work off the event loop: `spawn` runs the work on a thread and hands the
//! closure it returns back to the loop over a channel, where it runs with the state.

use smithay::reexports::calloop::channel::{self, Event, Sender};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::compositor::State;

pub type OnLoop = Box<dyn FnOnce(&mut State) + Send>;

type Work = Box<dyn FnOnce() -> OnLoop + Send>;

/// Job threads running at once. A job on a file system that never answers keeps its thread for
/// good; past this many, later jobs wait for a thread to come back instead of stranding more.
pub const MAX_THREADS: usize = 8;

/// The loop's end of the jobs' channel. No sender until `setup`, or when the
/// channel could not join the loop: `spawn` then runs the work inline.
#[derive(Default)]
pub struct FsJobs {
    tx: Option<Sender<OnLoop>>,
    /// Job threads not yet finished, counted down by each thread as it ends.
    running: Arc<AtomicUsize>,
    /// Jobs waiting for a thread while [`MAX_THREADS`] are running.
    waiting: VecDeque<Work>,
    /// The wait was logged; cleared once the waiting jobs have started.
    warned: bool,
}

/// Join the jobs' channel to the loop. Runs once, after `State::new`.
pub fn setup(state: &mut State) {
    let (tx, rx) = channel::channel::<OnLoop>();
    let joined = state.handle.insert_source(rx, |ev, _, state| {
        if let Event::Msg(run) = ev {
            run_one(run, state);
            state.fsjobs.start_waiting();
        }
    });
    match joined {
        Ok(_) => state.fsjobs.tx = Some(tx),
        Err(e) => log::warn!(
            "fsjobs: no channel ({}); file work runs on the loop",
            e.error
        ),
    }
}

/// Run one finished job under a guard, so a panic leaves later jobs to run.
fn run_one(run: OnLoop, state: &mut State) {
    let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(state)));
    if ran.is_err() {
        log::error!("fsjobs: a job panicked");
    }
}

impl FsJobs {
    /// The channel's sending end, for a worker of its own that hands results to the loop.
    pub fn sender(&self) -> Option<Sender<OnLoop>> {
        self.tx.clone()
    }

    /// Runs `work` on a thread of its own, or queues it while [`MAX_THREADS`] are running.
    /// False only if the thread fails to spawn and the work is lost.
    fn spawn(&mut self, tx: Sender<OnLoop>, work: Work) -> bool {
        if self.running.load(Ordering::Acquire) >= MAX_THREADS {
            if !self.warned {
                log::warn!(
                    "fsjobs: {MAX_THREADS} file jobs have not finished (a file system not \
                     answering?); later file work waits for one"
                );
                self.warned = true;
            }
            self.waiting.push_back(work);
            return true;
        }
        let running = Arc::clone(&self.running);
        running.fetch_add(1, Ordering::AcqRel);
        let spawned = std::thread::Builder::new()
            .name("fsjobs".into())
            .spawn(move || {
                let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                    .unwrap_or_else(|_| Box::new(|_| log::error!("fsjobs: a job panicked")));
                // Counted down before the send, so the loop finds the thread free when the
                // result arrives.
                running.fetch_sub(1, Ordering::AcqRel);
                // The loop may have gone meanwhile; the result goes with it.
                let _ = tx.send(done);
            });
        if let Err(e) = spawned {
            self.running.fetch_sub(1, Ordering::AcqRel);
            log::warn!("fsjobs: cannot spawn a thread ({e}); the file work was dropped");
            return false;
        }
        true
    }

    /// Starts waiting jobs on the threads that have come back.
    fn start_waiting(&mut self) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        while self.running.load(Ordering::Acquire) < MAX_THREADS {
            let Some(work) = self.waiting.pop_front() else {
                break;
            };
            self.spawn(tx.clone(), work);
        }
        if self.waiting.is_empty() && self.warned {
            log::info!("fsjobs: the waiting file work has started");
            self.warned = false;
        }
    }
}

impl State {
    /// Run `work` on a thread; what it returns runs on the loop afterwards. Without
    /// the channel it runs inline; returns false only if the thread fails to spawn
    /// and the work is lost.
    pub fn spawn_fs_job<W>(&mut self, work: W) -> bool
    where
        W: FnOnce() -> OnLoop + Send + 'static,
    {
        let Some(tx) = self.fsjobs.tx.clone() else {
            work()(self);
            return true;
        };
        self.fsjobs.spawn(tx, Box::new(work))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    /// A named pipe in a scratch folder: opening it to read blocks until a writer opens it, as a
    /// call into a dead mount blocks.
    fn fifo(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-fsjobs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hang");
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        path
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Jobs blocked on a file system that does not answer hold at most [`MAX_THREADS`] threads;
    /// later jobs wait instead of stranding more, and start once the blocked ones return.
    #[test]
    fn hung_jobs_hold_at_most_max_threads_and_later_ones_wait() {
        use std::os::unix::fs::OpenOptionsExt;
        let hang = fifo("cap");
        let (tx, _rx) = channel::channel::<OnLoop>();
        let mut jobs = FsJobs {
            tx: Some(tx.clone()),
            ..Default::default()
        };
        for _ in 0..MAX_THREADS + 3 {
            let path = hang.clone();
            let work: Work = Box::new(move || {
                let _ = std::fs::read(&path);
                Box::new(|_| {})
            });
            assert!(jobs.spawn(tx.clone(), work));
        }
        let running = Arc::clone(&jobs.running);
        until("the threads to start", || {
            running.load(Ordering::Acquire) == MAX_THREADS
        });
        assert_eq!(jobs.waiting.len(), 3, "jobs past the cap wait");
        assert!(jobs.warned);
        jobs.start_waiting();
        assert_eq!(jobs.waiting.len(), 3, "no thread came back");

        // A writer opening and closing the pipe answers each blocked read with an empty file.
        let stop = Arc::new(AtomicBool::new(false));
        let (feed, halt) = (hang.clone(), Arc::clone(&stop));
        let writer = std::thread::spawn(move || {
            while !halt.load(Ordering::Acquire) {
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&feed);
                std::thread::sleep(Duration::from_millis(2));
            }
        });
        let end = Instant::now() + Duration::from_secs(5);
        while !(jobs.waiting.is_empty() && running.load(Ordering::Acquire) == 0) {
            assert!(Instant::now() < end, "timed out waiting for the jobs");
            jobs.start_waiting();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!jobs.warned, "the wait is over");
        stop.store(true, Ordering::Release);
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(hang.parent().unwrap());
    }
}
