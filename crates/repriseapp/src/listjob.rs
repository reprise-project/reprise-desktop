//! A folder listing read on a thread, so a mount that stopped answering hangs the read and never
//! the program's loop. The loop waits a moment for it, else takes it once the thread wakes it.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use crate::Waker;

/// How long a loop waits for a listing before it goes on showing the old one.
pub const WAIT: Duration = Duration::from_millis(300);

/// Listing threads running at once in a program. A read on a mount that never answers keeps its
/// thread for good; past this many, later reads wait for a thread to come back.
pub const MAX_THREADS: usize = 8;

/// A read running on a listing thread, or waiting for one.
pub struct Job<T>(Receiver<T>);

impl<T: Send + 'static> Job<T> {
    /// Runs `read` on a listing thread that fires `wake`, if given, once it answers. While
    /// [`MAX_THREADS`] are busy it waits for one: its [`Job::wait`] gives up as on a dead mount.
    pub fn spawn(wake: Option<&Waker>, read: impl FnOnce() -> T + Send + 'static) -> Job<T> {
        let (tx, rx) = channel();
        let wake = wake.cloned();
        POOL.run(Box::new(move || {
            if tx.send(read()).is_ok()
                && let Some(w) = wake
            {
                w.wake();
            }
        }));
        Job(rx)
    }

    /// The answer, if it comes within `wait`.
    pub fn wait(&self, wait: Duration) -> Option<T> {
        self.0.recv_timeout(wait).ok()
    }

    /// The answer, if it has come.
    pub fn take(&self) -> Option<T> {
        self.0.try_recv().ok()
    }
}

type Task = Box<dyn FnOnce() + Send>;

static POOL: Pool = Pool::new(MAX_THREADS);

/// At most `max` threads; a thread whose task returns takes the next waiting one.
struct Pool {
    max: usize,
    state: Mutex<PoolState>,
}

struct PoolState {
    running: usize,
    waiting: VecDeque<Task>,
    /// The wait was logged; cleared once nothing waits.
    warned: bool,
}

impl Pool {
    const fn new(max: usize) -> Pool {
        Pool {
            max,
            state: Mutex::new(PoolState {
                running: 0,
                waiting: VecDeque::new(),
                warned: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn run(&'static self, task: Task) {
        let mut st = self.lock();
        if st.running >= self.max {
            if !st.warned {
                log::warn!(
                    "{} listing threads have not finished (a mount not answering?); later reads \
                     wait for one",
                    self.max
                );
                st.warned = true;
            }
            st.waiting.push_back(task);
            return;
        }
        st.running += 1;
        drop(st);
        let spawned = std::thread::Builder::new()
            .name("listing".into())
            .spawn(move || self.work(task));
        if let Err(e) = spawned {
            self.lock().running -= 1;
            log::warn!("cannot spawn a listing thread ({e})");
        }
    }

    /// Runs `task`, then each waiting one, until none waits.
    fn work(&self, mut task: Task) {
        /// Gives the thread back if a task panics.
        struct Leave<'a>(&'a Pool);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.lock().running -= 1;
                }
            }
        }
        let _leave = Leave(self);
        loop {
            task();
            let mut st = self.lock();
            match st.waiting.pop_front() {
                Some(next) => task = next,
                None => {
                    st.running -= 1;
                    st.warned = false;
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_answer_is_taken_after_the_wait_gives_up() {
        let (go, gate) = channel::<()>();
        let job = Job::spawn(None, move || {
            gate.recv().ok();
            7
        });
        assert_eq!(job.wait(Duration::from_millis(20)), None, "still reading");
        assert_eq!(job.take(), None);
        go.send(()).unwrap();
        assert_eq!(job.wait(Duration::from_secs(60)), Some(7));
    }

    /// Reads that never answer hold at most `max` threads; later ones wait and run on the
    /// threads that come back.
    #[test]
    fn hung_reads_hold_at_most_max_threads_and_later_ones_wait() {
        let pool: &'static Pool = Box::leak(Box::new(Pool::new(3)));
        let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
        let closed = gate.write().unwrap();
        let (done, finished) = channel();
        for i in 0..5 {
            let (gate, done) = (gate.clone(), done.clone());
            pool.run(Box::new(move || {
                drop(gate.read());
                done.send(i).ok();
            }));
        }
        {
            let st = pool.lock();
            assert_eq!(st.running, 3);
            assert_eq!(st.waiting.len(), 2, "reads past the cap wait");
            assert!(st.warned);
        }
        assert!(finished.recv_timeout(Duration::from_millis(50)).is_err());
        drop(closed);
        let mut got: Vec<i32> = (0..5)
            .map(|_| finished.recv_timeout(Duration::from_secs(60)).unwrap())
            .collect();
        got.sort();
        assert_eq!(got, [0, 1, 2, 3, 4]);
        let end = std::time::Instant::now() + Duration::from_secs(60);
        while pool.lock().running > 0 {
            assert!(std::time::Instant::now() < end, "threads never came back");
            std::thread::sleep(Duration::from_millis(5));
        }
        let st = pool.lock();
        assert!(st.waiting.is_empty());
        assert!(!st.warned);
    }
}
