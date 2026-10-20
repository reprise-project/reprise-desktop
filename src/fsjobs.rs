//! File work off the event loop: `spawn` runs the work on a thread and hands the
//! closure it returns back to the loop over a channel, where it runs with the state.

use smithay::reexports::calloop::channel::{self, Event, Sender};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::compositor::State;
use reprisefs::deadline::Landing;
pub use reprisefs::deadline::{ACTION_DEADLINE, Gate};

pub type OnLoop = Box<dyn FnOnce(&mut State) + Send>;

type Work = Box<dyn FnOnce() -> OnLoop + Send>;

/// Job threads running at once per pool. A job on a file system that never answers keeps its
/// thread; past this many, later jobs wait for a thread to come back instead of stranding more.
pub const MAX_THREADS: usize = 8;

/// The loop's end of the jobs' channel. No sender until `setup`, or when the
/// channel could not join the loop: `spawn` then runs the work inline.
#[derive(Default)]
pub struct FsJobs {
    tx: Option<Sender<OnLoop>>,
    /// Background file work: listings, rescans, icon reads.
    files: Pool,
    /// Actions asked for by hand, apart from `files` so neither starves the other.
    actions: Pool,
    /// How long an action or a drop waits before it gives up; None for [`ACTION_DEADLINE`].
    /// Tests shorten it.
    pub deadline: Option<Duration>,
}

/// Threads of one kind of job, at most [`MAX_THREADS`] at once.
#[derive(Default)]
struct Pool {
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
    pub fn deadline(&self) -> Duration {
        self.deadline.unwrap_or(ACTION_DEADLINE)
    }

    /// The channel's sending end, for a worker of its own that hands results to the loop.
    pub fn sender(&self) -> Option<Sender<OnLoop>> {
        self.tx.clone()
    }

    /// Runs `work` on a thread of its own at once, past [`MAX_THREADS`]: for work no mount can
    /// hold. False only if the thread fails to spawn and the work is lost.
    fn spawn_now(&self, tx: Sender<OnLoop>, work: Work) -> bool {
        thread(tx, work, None)
    }

    /// Starts waiting jobs on the threads that have come back.
    fn start_waiting(&mut self) {
        let Some(tx) = self.tx.clone() else {
            return;
        };
        self.files.start_waiting(&tx, "file jobs");
        self.actions.start_waiting(&tx, "file actions");
    }
}

impl Pool {
    /// Runs `work` on a thread of its own, or queues it while [`MAX_THREADS`] are running.
    /// False only if the thread fails to spawn and the work is lost.
    fn spawn(&mut self, tx: Sender<OnLoop>, work: Work, what: &str) -> bool {
        if self.running.load(Ordering::Acquire) >= MAX_THREADS {
            if !self.warned {
                log::warn!(
                    "fsjobs: {MAX_THREADS} {what} have not finished (a file system not \
                     answering?); later ones wait for one"
                );
                self.warned = true;
            }
            self.waiting.push_back(work);
            return true;
        }
        self.running.fetch_add(1, Ordering::AcqRel);
        let spawned = thread(tx, work, Some(Arc::clone(&self.running)));
        if !spawned {
            self.running.fetch_sub(1, Ordering::AcqRel);
        }
        spawned
    }

    fn start_waiting(&mut self, tx: &Sender<OnLoop>, what: &str) {
        while self.running.load(Ordering::Acquire) < MAX_THREADS {
            let Some(work) = self.waiting.pop_front() else {
                break;
            };
            self.spawn(tx.clone(), work, what);
        }
        if self.waiting.is_empty() && self.warned {
            log::info!("fsjobs: the waiting {what} have started");
            self.warned = false;
        }
    }
}

/// Runs `work` on a new thread and sends what it returns to the loop. `running`, counted up by
/// the caller, is counted down as the thread ends. False when the thread fails to spawn.
fn thread(tx: Sender<OnLoop>, work: Work, running: Option<Arc<AtomicUsize>>) -> bool {
    let spawned = std::thread::Builder::new()
        .name("fsjobs".into())
        .spawn(move || {
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|_| Box::new(|_| log::error!("fsjobs: a job panicked")));
            // Counted down before the send, so the loop finds the thread free when the
            // result arrives.
            if let Some(running) = running {
                running.fetch_sub(1, Ordering::AcqRel);
            }
            // The loop may have gone meanwhile; the result goes with it.
            let _ = tx.send(done);
        });
    if let Err(e) = &spawned {
        log::warn!("fsjobs: cannot spawn a thread ({e}); the work was dropped");
    }
    spawned.is_ok()
}

/// A read that has not landed in this long (a mount not answering) lets a newer [`Reads`] read
/// start.
pub const STUCK: Duration = Duration::from_secs(1);

/// Reads of one thing in flight at most, stuck ones included.
pub const MAX_READS: usize = 3;

/// Re-reads of one thing on file jobs, each carrying `T` (what it reads): one runs at a time,
/// unless the running ones have been stuck past [`STUCK`]; then a newer one may start, up to
/// [`MAX_READS`] in flight.
pub struct Reads<T = ()> {
    next: u64,
    /// Each read in flight, when it started and what it reads.
    running: Vec<(u64, Instant, T)>,
    /// The newest read that landed.
    landed: u64,
}

impl<T> Default for Reads<T> {
    fn default() -> Self {
        Reads {
            next: 0,
            running: Vec::new(),
            landed: 0,
        }
    }
}

impl<T> Reads<T> {
    /// A new read of `what`'s id, or None while one runs fresh or [`MAX_READS`] are in flight.
    pub fn start(&mut self, now: Instant, what: T) -> Option<u64> {
        let fresh =
            (self.running.iter()).any(|(_, t, _)| now.saturating_duration_since(*t) < STUCK);
        if fresh || self.running.len() >= MAX_READS {
            return None;
        }
        self.next += 1;
        self.running.push((self.next, now, what));
        Some(self.next)
    }

    /// How long until a refused [`Reads::start`] may succeed, None while [`MAX_READS`] are in
    /// flight.
    pub fn retry_in(&self, now: Instant) -> Option<Duration> {
        if self.running.len() >= MAX_READS {
            return None;
        }
        let Some(newest) = self.running.iter().map(|(_, t, _)| *t).max() else {
            return Some(Duration::ZERO);
        };
        Some(STUCK.saturating_sub(now.saturating_duration_since(newest)))
    }

    /// What each read in flight reads, and whether it is stuck at `now`.
    pub fn in_flight(&self, now: Instant) -> impl Iterator<Item = (bool, &T)> {
        (self.running.iter()).map(move |(_, t, w)| (now.saturating_duration_since(*t) >= STUCK, w))
    }

    /// What read `id` reads, while it is in flight.
    pub fn reading_mut(&mut self, id: u64) -> Option<&mut T> {
        (self.running.iter_mut()).find_map(|(i, _, w)| (*i == id).then_some(w))
    }

    /// Ends read `id`. False when a newer read landed first: its answer is stale.
    pub fn land(&mut self, id: u64) -> bool {
        self.running.retain(|(i, _, _)| *i != id);
        let current = id > self.landed;
        self.landed = self.landed.max(id);
        current
    }
}

/// What an action does to files, which decides what a give-up says and what a late answer does.
#[derive(Clone, Copy)]
pub struct Action {
    /// The error box's title.
    pub title: &'static str,
    /// Some for an action that changes files: what refreshes the view when a change it had
    /// already started lands past the deadline.
    pub late: Option<fn(&mut State)>,
}

impl Action {
    /// An action whose work only reads; a late answer is dropped.
    pub fn reads(title: &'static str) -> Action {
        Action { title, late: None }
    }

    /// An action whose work changes files after [`Gate::change`].
    pub fn changes(title: &'static str, late: fn(&mut State)) -> Action {
        Action {
            title,
            late: Some(late),
        }
    }

    /// The give-up's error text; `changing` when the work had passed [`Gate::change`].
    pub fn not_responding(&self, changing: bool) -> String {
        reprisefs::deadline::not_responding(self.late.is_some(), changing)
    }
}

/// `work` for an action's thread: skipped when the action gave up while it waited for a thread,
/// and its answer dropped (or `late` run in its place) when it lands past the deadline.
fn gated<W>(gate: Gate, late: Option<fn(&mut State)>, work: W) -> Work
where
    W: FnOnce(&Gate) -> OnLoop + Send + 'static,
{
    Box::new(move || {
        if gate.given_up() {
            return Box::new(|_| {});
        }
        let done = work(&gate);
        match (gate.land(), late) {
            (Landing::Landed, _) => done,
            (Landing::Refresh, Some(late)) => {
                log::info!("fsjobs: a file action landed past its deadline; refreshing");
                Box::new(late)
            }
            _ => {
                log::info!("fsjobs: a file action answered past its deadline; dropped");
                Box::new(|_| {})
            }
        }
    })
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
        self.fsjobs.files.spawn(tx, Box::new(work), "file jobs")
    }

    /// [`State::spawn_fs_job`] for an action asked for by hand: past [`ACTION_DEADLINE`] an error box
    /// says the location is not responding and the answer is dropped whenever it comes.
    pub fn spawn_fs_action<W>(&mut self, action: Action, work: W) -> bool
    where
        W: FnOnce(&Gate) -> OnLoop + Send + 'static,
    {
        let Some(tx) = self.fsjobs.tx.clone() else {
            work(&Gate::default())(self);
            return true;
        };
        let gate = Gate::default();
        let deadline = self.fsjobs.deadline();
        let timer = Timer::from_duration(deadline);
        let watch = gate.clone();
        let armed = self.handle.insert_source(timer, move |_, _, state| {
            if let Some(changing) = watch.give_up() {
                log::warn!("fsjobs: {} gave up after {deadline:?}", action.title);
                let text = action.not_responding(changing);
                state.open_dialog(crate::dialog::Dialog::error(action.title, text));
            }
            TimeoutAction::Drop
        });
        if let Err(e) = armed {
            log::warn!("fsjobs: no deadline for {}: {}", action.title, e.error);
        }
        let work = gated(gate, action.late, work);
        self.fsjobs.actions.spawn(tx, work, "file actions")
    }

    /// [`State::spawn_fs_job`] for work that opens no file (painting art, bus calls) or only the
    /// one path a launch starts: it starts at once on a thread of its own, never waiting behind
    /// file jobs stuck on a mount.
    pub fn spawn_job<W>(&mut self, work: W) -> bool
    where
        W: FnOnce() -> OnLoop + Send + 'static,
    {
        let Some(tx) = self.fsjobs.tx.clone() else {
            work()(self);
            return true;
        };
        self.fsjobs.spawn_now(tx, Box::new(work))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

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
            assert!(jobs.files.spawn(tx.clone(), work, "file jobs"));
        }
        let running = Arc::clone(&jobs.files.running);
        until("the threads to start", || {
            running.load(Ordering::Acquire) == MAX_THREADS
        });
        assert_eq!(jobs.files.waiting.len(), 3, "jobs past the cap wait");
        assert!(jobs.files.warned);
        jobs.start_waiting();
        assert_eq!(jobs.files.waiting.len(), 3, "no thread came back");

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
        while !(jobs.files.waiting.is_empty() && running.load(Ordering::Acquire) == 0) {
            assert!(Instant::now() < end, "timed out waiting for the jobs");
            jobs.start_waiting();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!jobs.files.warned, "the wait is over");
        stop.store(true, Ordering::Release);
        writer.join().unwrap();
        let _ = std::fs::remove_dir_all(hang.parent().unwrap());
    }

    #[test]
    fn work_that_opens_no_file_starts_while_file_jobs_are_stuck() {
        use std::os::unix::fs::OpenOptionsExt;
        let hang = fifo("now");
        let (tx, _rx) = channel::channel::<OnLoop>();
        let mut jobs = FsJobs {
            tx: Some(tx.clone()),
            ..Default::default()
        };
        for _ in 0..MAX_THREADS + 1 {
            let path = hang.clone();
            let work: Work = Box::new(move || {
                let _ = std::fs::read(&path);
                Box::new(|_| {})
            });
            assert!(jobs.files.spawn(tx.clone(), work, "file jobs"));
        }
        let running = Arc::clone(&jobs.files.running);
        until("the threads to start", || {
            running.load(Ordering::Acquire) == MAX_THREADS
        });
        let painted = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&painted);
        let paint: Work = Box::new(move || {
            done.store(true, Ordering::Release);
            Box::new(|_| {})
        });
        assert!(jobs.spawn_now(tx.clone(), paint));
        until("the work to run", || painted.load(Ordering::Acquire));
        assert_eq!(jobs.files.waiting.len(), 1, "only the file job waits");
        // A writer opening the pipe answers the blocked reads, so their threads end.
        while running.load(Ordering::Acquire) > 0 {
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&hang);
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = std::fs::remove_dir_all(hang.parent().unwrap());
    }

    #[test]
    fn a_stuck_read_lets_a_newer_one_start_up_to_max_reads() {
        let t0 = Instant::now();
        let mut r = Reads::default();
        let a = r.start(t0, ()).unwrap();
        assert_eq!(
            r.start(t0 + STUCK / 2, ()),
            None,
            "the first read runs fresh"
        );
        assert_eq!(r.retry_in(t0 + STUCK / 2), Some(STUCK / 2));
        let b = r.start(t0 + STUCK, ()).expect("the first read is stuck");
        assert_eq!(r.start(t0 + STUCK, ()), None, "the second read runs fresh");
        let c = r.start(t0 + STUCK * 2, ()).unwrap();
        assert_eq!(
            r.start(t0 + STUCK * 9, ()),
            None,
            "{MAX_READS} reads in flight"
        );
        assert_eq!(r.retry_in(t0 + STUCK * 9), None);
        assert!(r.land(b), "the newest landing read is current");
        assert!(
            !r.land(a),
            "an older read landing after a newer one is stale"
        );
        assert!(r.start(t0 + STUCK * 9, ()).is_some(), "a slot came back");
        assert!(r.land(c));
    }
    /// Opens `hang` to write until `done`, which answers each read blocked on it.
    fn answer(hang: &std::path::Path, mut done: impl FnMut() -> bool) {
        use std::os::unix::fs::OpenOptionsExt;
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "timed out answering the reads");
            let _ = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(hang);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn an_action_answered_past_its_deadline_changes_nothing() {
        let hang = fifo("late");
        let made = hang.with_file_name("New Folder");
        let (tx, _rx) = channel::channel::<OnLoop>();
        let mut jobs = FsJobs {
            tx: Some(tx.clone()),
            ..Default::default()
        };
        let gate = Gate::default();
        let entered = Arc::new(AtomicBool::new(false));
        let (path, dir, inside) = (hang.clone(), made.clone(), Arc::clone(&entered));
        let work = gated(gate.clone(), Some(|_| {}), move |gate: &Gate| -> OnLoop {
            inside.store(true, Ordering::Release);
            let _ = std::fs::read(&path);
            if gate.change(dir.parent().unwrap()) {
                std::fs::create_dir(&dir).unwrap();
            }
            Box::new(|_| {})
        });
        assert!(jobs.actions.spawn(tx, work, "file actions"));
        let running = Arc::clone(&jobs.actions.running);
        until("the action to start", || entered.load(Ordering::Acquire));
        assert_eq!(gate.give_up(), Some(false), "no change had started");
        answer(&hang, || running.load(Ordering::Acquire) == 0);
        assert!(!made.exists(), "the folder is not made after the give-up");
        let _ = std::fs::remove_dir_all(hang.parent().unwrap());
    }

    #[test]
    fn actions_start_while_file_jobs_are_stuck_and_wait_only_for_actions() {
        let hang = fifo("pools");
        let (tx, _rx) = channel::channel::<OnLoop>();
        let mut jobs = FsJobs {
            tx: Some(tx.clone()),
            ..Default::default()
        };
        let stuck = |path: std::path::PathBuf| -> Work {
            Box::new(move || {
                let _ = std::fs::read(&path);
                Box::new(|_| {})
            })
        };
        for _ in 0..MAX_THREADS {
            assert!(
                jobs.files
                    .spawn(tx.clone(), stuck(hang.clone()), "file jobs")
            );
        }
        let files = Arc::clone(&jobs.files.running);
        until("the file jobs to start", || {
            files.load(Ordering::Acquire) == MAX_THREADS
        });
        let ran = Arc::new(AtomicBool::new(false));
        let done = Arc::clone(&ran);
        let action = gated(Gate::default(), None, move |_: &Gate| -> OnLoop {
            done.store(true, Ordering::Release);
            Box::new(|_| {})
        });
        assert!(jobs.actions.spawn(tx.clone(), action, "file actions"));
        until("the action to run", || ran.load(Ordering::Acquire));
        let actions = Arc::clone(&jobs.actions.running);
        until("the action to end", || actions.load(Ordering::Acquire) == 0);

        for _ in 0..MAX_THREADS + 1 {
            let work = gated(Gate::default(), None, {
                let path = hang.clone();
                move |_: &Gate| -> OnLoop {
                    let _ = std::fs::read(&path);
                    Box::new(|_| {})
                }
            });
            assert!(jobs.actions.spawn(tx.clone(), work, "file actions"));
        }
        assert_eq!(jobs.actions.waiting.len(), 1, "a ninth stuck action waits");
        answer(&hang, || {
            jobs.start_waiting();
            jobs.actions.waiting.is_empty()
                && actions.load(Ordering::Acquire) == 0
                && files.load(Ordering::Acquire) == 0
        });
        let _ = std::fs::remove_dir_all(hang.parent().unwrap());
    }
}
