//! Threads for work a mount that stopped answering can hang: a bounded number at once, later work
//! waiting in a queue for one to come back.

use reprisebus::mutex::lock;
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub(crate) type Task = Box<dyn FnOnce() + Send>;

/// At most `fresh` threads whose task began under `stuck` ago and `most` threads in all. A
/// thread whose task returns takes the next waiting one while there is room.
pub(crate) struct Pool {
    /// Names the threads and the log lines.
    name: &'static str,
    fresh: usize,
    most: usize,
    stuck: Duration,
    state: Mutex<PoolState>,
}

struct PoolState {
    /// When the task on each thread began; `None` is a free slot.
    started: Vec<Option<Instant>>,
    waiting: VecDeque<Task>,
    /// The wait was logged; cleared once nothing waits.
    warned: bool,
    /// A timer thread starts waiting tasks as busy ones turn stuck.
    ticking: bool,
}

impl PoolState {
    fn threads(&self) -> usize {
        self.started.iter().flatten().count()
    }
}

impl Pool {
    pub(crate) const fn new(
        name: &'static str,
        fresh: usize,
        most: usize,
        stuck: Duration,
    ) -> Pool {
        Pool {
            name,
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
        lock(&self.state)
    }

    /// Another task may start now.
    fn room(&self, st: &PoolState, now: Instant) -> bool {
        let fresh = st
            .started
            .iter()
            .flatten()
            .filter(|t| now.duration_since(**t) < self.stuck)
            .count();
        fresh < self.fresh && st.threads() < self.most
    }

    /// Runs `task` on a thread of the pool, or queues it until one has room.
    pub(crate) fn run(&'static self, task: Task) {
        let mut st = self.lock();
        st.waiting.push_back(task);
        self.start_waiting(&mut st);
        self.hold(&mut st);
    }

    /// Logs tasks left waiting and keeps a timer to start them as busy tasks turn stuck.
    fn hold(&'static self, st: &mut PoolState) {
        if st.waiting.is_empty() {
            return;
        }
        if !st.warned {
            log::warn!(
                "{} threads have not finished (a mount not answering?); later ones wait for one",
                self.name
            );
            st.warned = true;
        }
        if st.threads() < self.most && !st.ticking {
            st.ticking = true;
            let spawned = std::thread::Builder::new()
                .name(format!("{} timer", self.name))
                .spawn(move || self.tick());
            if let Err(e) = spawned {
                st.ticking = false;
                log::warn!("cannot spawn the {} timer ({e})", self.name);
            }
        }
    }

    /// Starts waiting tasks on new threads while there is room. A task whose thread fails to
    /// spawn goes back to the head of the queue for the next thread that comes back or `run`.
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
            let task = std::sync::Arc::new(Mutex::new(Some(task)));
            let mine = task.clone();
            let spawned = std::thread::Builder::new()
                .name(self.name.into())
                .spawn(move || {
                    if let Some(task) = lock(&mine).take() {
                        self.work(slot, task);
                    }
                });
            if let Err(e) = spawned {
                st.started[slot] = None;
                log::warn!("cannot spawn a {} thread ({e})", self.name);
                if let Some(task) = lock(&task).take() {
                    st.waiting.push_front(task);
                }
                break;
            }
        }
        if st.waiting.is_empty() {
            st.warned = false;
        }
    }

    /// Wakes as each fresh task turns stuck and starts waiting tasks, until none waits or no
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;

    fn leak(fresh: usize, most: usize, stuck: Duration) -> &'static Pool {
        Box::leak(Box::new(Pool::new("test", fresh, most, stuck)))
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
    fn fresh_tasks_hold_at_most_max_threads_and_later_ones_wait() {
        let pool = leak(3, 10, Duration::from_secs(600));
        let gate = Arc::new(std::sync::RwLock::new(()));
        let closed = gate.write().unwrap();
        let (running, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (done, finished) = channel();
        for i in 0..5 {
            let (gate, done) = (gate.clone(), done.clone());
            let (running, peak) = (running.clone(), peak.clone());
            pool.run(Box::new(move || {
                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                drop(gate.read());
                running.fetch_sub(1, Ordering::SeqCst);
                done.send(i).ok();
            }));
        }
        {
            let st = pool.lock();
            assert_eq!(st.threads(), 3);
            assert_eq!(st.waiting.len(), 2, "tasks past the cap wait");
            assert!(st.warned);
        }
        assert!(finished.recv_timeout(Duration::from_millis(50)).is_err());
        drop(closed);
        let mut got: Vec<i32> = (0..5)
            .map(|_| finished.recv_timeout(Duration::from_secs(60)).unwrap())
            .collect();
        got.sort();
        assert_eq!(got, [0, 1, 2, 3, 4]);
        assert_eq!(peak.load(Ordering::SeqCst), 3, "never more than 3 at once");
        until("threads never came back", || pool.lock().threads() == 0);
        let st = pool.lock();
        assert!(st.waiting.is_empty());
        assert!(!st.warned);
    }

    #[test]
    fn a_task_left_waiting_starts_once_the_busy_ones_turn_stuck() {
        let pool = leak(2, 4, Duration::from_millis(50));
        let (_hold, gate) = channel::<()>();
        let gate = Arc::new(Mutex::new(gate));
        for _ in 0..2 {
            let gate = gate.clone();
            pool.run(Box::new(move || drop(gate.lock().map(|g| g.recv()))));
        }
        let (done, finished) = channel();
        pool.run(Box::new(move || done.send(()).unwrap()));
        assert_eq!(pool.lock().waiting.len(), 1, "two fresh tasks fill the cap");
        finished
            .recv_timeout(Duration::from_secs(60))
            .expect("the local task runs beside the stuck ones");
        until("the third thread never came back", || {
            pool.lock().threads() == 2
        });
        until("the timer never stopped", || !pool.lock().ticking);
    }

    #[test]
    fn stuck_tasks_hold_at_most_max_all_threads() {
        let pool = leak(2, 4, Duration::from_millis(10));
        let gate = Arc::new(std::sync::RwLock::new(()));
        let closed = gate.write().unwrap();
        let (done, finished) = channel();
        for i in 0..6 {
            let (gate, done) = (gate.clone(), done.clone());
            pool.run(Box::new(move || {
                drop(gate.read());
                done.send(i).ok();
            }));
        }
        until("stuck tasks never filled the bound", || {
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
