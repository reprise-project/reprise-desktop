use crate::mutex::lock;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// A decode on its thread; true once a spare stands in for that thread, which then ends.
type Job = Box<dyn FnOnce() -> bool + Send>;

/// Threads that read and decode tray items' named icons and notification pictures, shared by
/// every app. A read still running past `within` (a mount that stopped answering) gets a spare
/// thread in its place, up to `spare` at once, and its own thread ends when the read returns; an
/// app floods only its own share of the queue.
pub(super) struct Decoders {
    jobs: async_channel::Sender<Job>,
    /// Where spares take jobs from.
    queue: async_channel::Receiver<Job>,
    /// Decodes queued or running, by app.
    held: Arc<Mutex<HashMap<String, usize>>>,
    /// Spares running, each in place of a thread a read past `within` still holds.
    spares: Arc<AtomicUsize>,
    pub(super) limits: Limits,
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub(super) threads: usize,
    /// Decodes waiting for a thread, every app's.
    pub(super) queue: usize,
    /// Decodes one app may have queued or running; past it a call is refused at once.
    pub(super) per_app: usize,
    /// How long a decode may wait for a thread.
    pub(super) wait: Duration,
    /// How long a decode may run once it has one.
    pub(super) within: Duration,
    /// Spare threads that may run at once.
    pub(super) spare: usize,
}

/// A job's [`AtomicU8`]: running, ended, or past `within` with a spare in its thread's place.
const RUNNING: u8 = 0;
const ENDED: u8 = 1;
const STOOD_IN: u8 = 2;

/// What a job tells its caller: that it has a thread, then its result.
enum Progress<T> {
    Started,
    Done(T),
}

/// One of an app's held decodes, given back however the job ends: run, skipped or dropped.
struct Held {
    app: String,
    held: Arc<Mutex<HashMap<String, usize>>>,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut held = lock(&self.held);
        if let Some(n) = held.get_mut(&self.app) {
            *n -= 1;
            if *n == 0 {
                held.remove(&self.app);
            }
        }
    }
}

impl Decoders {
    pub(super) fn new(limits: Limits) -> Self {
        let (jobs, queue) = async_channel::bounded::<Job>(limits.queue);
        let decoders = Decoders {
            jobs,
            queue,
            held: Arc::default(),
            spares: Arc::default(),
            limits,
        };
        for _ in 0..limits.threads {
            if let Err(e) = decoders.start_thread() {
                log::warn!("tray: no icon thread: {e}");
            }
        }
        decoders
    }

    fn start_thread(&self) -> std::io::Result<()> {
        let (queue, spares) = (self.queue.clone(), self.spares.clone());
        std::thread::Builder::new()
            .name("tray-icon".into())
            .spawn(move || {
                while let Ok(job) = queue.recv_blocking() {
                    if job() {
                        spares.fetch_sub(1, Ordering::AcqRel);
                        return;
                    }
                }
            })
            .map(drop)
    }

    /// `decode`'s result for `app`; None past the app's share, `wait` or `within`. A decode whose
    /// caller has given up is skipped.
    pub(super) async fn run<T: Send + 'static>(
        &self,
        app: &str,
        decode: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let held = self.hold(app)?;
        let (tx, rx) = async_channel::bounded(2);
        let state = Arc::new(AtomicU8::new(RUNNING));
        let ends = state.clone();
        let job: Job = Box::new(move || {
            let _held = held;
            if tx.try_send(Progress::Started).is_ok() {
                match catch_unwind(AssertUnwindSafe(decode)) {
                    Ok(t) => drop(tx.send_blocking(Progress::Done(t))),
                    Err(_) => log::warn!("tray: an icon decode panicked"),
                }
            }
            ends.swap(ENDED, Ordering::AcqRel) == STOOD_IN
        });
        let started = async {
            self.jobs.send(job).await.ok()?;
            match rx.recv().await {
                Ok(Progress::Started) => Some(()),
                _ => None,
            }
        };
        within(self.limits.wait, started).await?;
        let done = async {
            match rx.recv().await {
                Ok(Progress::Done(t)) => Some(t),
                _ => None,
            }
        };
        let done = within(self.limits.within, done).await;
        if done.is_none() {
            self.stand_in(&state);
        }
        done
    }

    /// A spare in place of the thread running the job `state` tracks, unless the job ended or
    /// `spare` spares run already.
    fn stand_in(&self, state: &AtomicU8) {
        let spare = self.limits.spare;
        let free = |n: usize| (n < spare).then_some(n + 1);
        let taken = self
            .spares
            .try_update(Ordering::AcqRel, Ordering::Acquire, free);
        if taken.is_err() {
            return;
        }
        let swapped =
            state.compare_exchange(RUNNING, STOOD_IN, Ordering::AcqRel, Ordering::Acquire);
        if swapped.is_err() {
            self.spares.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        if let Err(e) = self.start_thread() {
            log::warn!("tray: no spare icon thread: {e}");
            let back =
                state.compare_exchange(STOOD_IN, RUNNING, Ordering::AcqRel, Ordering::Acquire);
            if back.is_ok() {
                self.spares.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    #[cfg(test)]
    pub(super) fn spares(&self) -> usize {
        self.spares.load(Ordering::Acquire)
    }

    fn hold(&self, app: &str) -> Option<Held> {
        let mut held = lock(&self.held);
        let n = held.entry(app.to_string()).or_default();
        if *n >= self.limits.per_app {
            return None;
        }
        *n += 1;
        Some(Held {
            app: app.to_string(),
            held: self.held.clone(),
        })
    }
}

async fn within<T>(limit: Duration, f: impl Future<Output = Option<T>>) -> Option<T> {
    let late = async {
        async_io::Timer::after(limit).await;
        None
    };
    futures_lite::future::or(f, late).await
}
