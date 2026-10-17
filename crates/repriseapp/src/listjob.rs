//! A folder listing read on a thread, so a mount that stopped answering hangs the read and never
//! the program's loop. The loop waits a moment for it, else takes it once the thread wakes it.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::Waker;

/// How long a loop waits for a listing before it goes on showing the old one.
pub const WAIT: Duration = Duration::from_millis(300);

/// Listing threads with a fresh read at once in a program; past this many, later reads wait.
pub const MAX_THREADS: usize = 8;

/// A read that has not answered in this long (a mount not answering) stops counting against
/// [`MAX_THREADS`]: its thread keeps waiting and another may start.
pub const STUCK: Duration = Duration::from_secs(1);

/// Listing threads alive at most, stuck ones included; past this many, later reads wait for one
/// to come back.
pub const MAX_ALL_THREADS: usize = 32;

/// A read running on a listing thread, or waiting for one.
pub struct Job<T>(Receiver<T>);

impl<T: Send + 'static> Job<T> {
    /// Runs `read` on a listing thread that fires `wake`, if given, once it answers. While no
    /// thread is free it waits for one: its [`Job::wait`] gives up as on a dead mount.
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

static POOL: Pool = Pool::new(MAX_THREADS, MAX_ALL_THREADS, STUCK);

/// At most `fresh` threads whose read began under `stuck` ago and `most` threads in all. A
/// thread whose task returns takes the next waiting one while there is room.
struct Pool {
    fresh: usize,
    most: usize,
    stuck: Duration,
    state: Mutex<PoolState>,
}

struct PoolState {
    /// When the read on each listing thread began; `None` is a free slot.
    started: Vec<Option<Instant>>,
    waiting: VecDeque<Task>,
    /// The wait was logged; cleared once nothing waits.
    warned: bool,
    /// A timer thread starts waiting reads as busy ones turn stuck.
    ticking: bool,
}

impl PoolState {
    fn threads(&self) -> usize {
        self.started.iter().flatten().count()
    }
}

impl Pool {
    const fn new(fresh: usize, most: usize, stuck: Duration) -> Pool {
        Pool {
            fresh,
            most,
            stuck,
            state: Mutex::new(PoolState {
                started: Vec::new(),
                waiting: VecDeque::new(),
                warned: false,
                ticking: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Another read may start now.
    fn room(&self, st: &PoolState, now: Instant) -> bool {
        let fresh = st
            .started
            .iter()
            .flatten()
            .filter(|t| now.duration_since(**t) < self.stuck)
            .count();
        fresh < self.fresh && st.threads() < self.most
    }

    fn run(&'static self, task: Task) {
        let mut st = self.lock();
        st.waiting.push_back(task);
        self.start_waiting(&mut st);
        self.hold(&mut st);
    }

    /// Logs reads left waiting and keeps a timer to start them as busy reads turn stuck.
    fn hold(&'static self, st: &mut PoolState) {
        if st.waiting.is_empty() {
            return;
        }
        if !st.warned {
            log::warn!(
                "listing threads have not finished (a mount not answering?); later reads wait \
                 for one"
            );
            st.warned = true;
        }
        if st.threads() < self.most && !st.ticking {
            st.ticking = true;
            let spawned = std::thread::Builder::new()
                .name("listing timer".into())
                .spawn(move || self.tick());
            if let Err(e) = spawned {
                st.ticking = false;
                log::warn!("cannot spawn the listing timer ({e})");
            }
        }
    }

    /// Starts waiting reads on new threads while there is room.
    fn start_waiting(&'static self, st: &mut PoolState) {
        let now = Instant::now();
        while !st.waiting.is_empty() && self.room(st, now) {
            let Some(task) = st.waiting.pop_front() else {
                break;
            };
            let slot = match st.started.iter().position(Option::is_none) {
                Some(i) => i,
                None => {
                    st.started.push(None);
                    st.started.len() - 1
                }
            };
            st.started[slot] = Some(now);
            let spawned = std::thread::Builder::new()
                .name("listing".into())
                .spawn(move || self.work(slot, task));
            if let Err(e) = spawned {
                st.started[slot] = None;
                log::warn!("cannot spawn a listing thread ({e})");
            }
        }
        if st.waiting.is_empty() {
            st.warned = false;
        }
    }

    /// Wakes as each fresh read turns stuck and starts waiting reads, until none waits or no
    /// thread may be added.
    fn tick(&'static self) {
        loop {
            let mut st = self.lock();
            self.start_waiting(&mut st);
            let now = Instant::now();
            let next = st
                .started
                .iter()
                .flatten()
                .map(|t| *t + self.stuck)
                .filter(|at| *at > now)
                .min();
            let Some(next) = next.filter(|_| !st.waiting.is_empty() && st.threads() < self.most)
            else {
                st.ticking = false;
                return;
            };
            drop(st);
            std::thread::sleep(next - now);
        }
    }

    /// Runs `task`, then each waiting one while there is room, on the thread in `slot`.
    fn work(&'static self, slot: usize, mut task: Task) {
        /// Gives the slot back if a task panics.
        struct Leave<'a>(&'a Pool, usize);
        impl Drop for Leave<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.lock().started[self.1] = None;
                }
            }
        }
        let _leave = Leave(self, slot);
        loop {
            task();
            let mut st = self.lock();
            st.started[slot] = None;
            let now = Instant::now();
            if !self.room(&st, now) {
                self.hold(&mut st);
                return;
            }
            match st.waiting.pop_front() {
                Some(next) => {
                    st.started[slot] = Some(now);
                    task = next;
                }
                None => {
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

    fn leak(fresh: usize, most: usize, stuck: Duration) -> &'static Pool {
        Box::leak(Box::new(Pool::new(fresh, most, stuck)))
    }

    /// Spins until `done` holds, at most a minute.
    fn until(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(60);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn fresh_reads_hold_at_most_max_threads_and_later_ones_wait() {
        let pool = leak(3, 10, Duration::from_secs(600));
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
            assert_eq!(st.threads(), 3);
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
        until("threads never came back", || pool.lock().threads() == 0);
        let st = pool.lock();
        assert!(st.waiting.is_empty());
        assert!(!st.warned);
    }

    #[test]
    fn a_read_left_waiting_starts_once_the_busy_reads_turn_stuck() {
        let pool = leak(2, 4, Duration::from_millis(50));
        let (_hold, gate) = channel::<()>();
        let gate = std::sync::Arc::new(Mutex::new(gate));
        for _ in 0..2 {
            let gate = gate.clone();
            pool.run(Box::new(move || drop(gate.lock().map(|g| g.recv()))));
        }
        let (done, finished) = channel();
        pool.run(Box::new(move || done.send(()).unwrap()));
        assert_eq!(pool.lock().waiting.len(), 1, "two fresh reads fill the cap");
        finished
            .recv_timeout(Duration::from_secs(60))
            .expect("the local read runs beside the stuck ones");
        until("the third thread never came back", || {
            pool.lock().threads() == 2
        });
        until("the timer never stopped", || !pool.lock().ticking);
    }

    #[test]
    fn stuck_reads_hold_at_most_max_all_threads() {
        let pool = leak(2, 4, Duration::from_millis(10));
        let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
        let closed = gate.write().unwrap();
        let (done, finished) = channel();
        for i in 0..6 {
            let (gate, done) = (gate.clone(), done.clone());
            pool.run(Box::new(move || {
                drop(gate.read());
                done.send(i).ok();
            }));
        }
        until("stuck reads never filled the bound", || {
            pool.lock().threads() == 4
        });
        until("the timer never stopped", || !pool.lock().ticking);
        std::thread::sleep(Duration::from_millis(50));
        {
            let st = pool.lock();
            assert_eq!(st.threads(), 4, "no thread past the bound");
            assert_eq!(st.waiting.len(), 2);
        }
        drop(closed);
        let mut got: Vec<i32> = (0..6)
            .map(|_| finished.recv_timeout(Duration::from_secs(60)).unwrap())
            .collect();
        got.sort();
        assert_eq!(got, [0, 1, 2, 3, 4, 5]);
        until("threads never came back", || pool.lock().threads() == 0);
        assert!(pool.lock().waiting.is_empty());
    }
}
