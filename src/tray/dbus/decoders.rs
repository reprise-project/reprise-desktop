use crate::mutex::lock;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex};
use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

/// Threads that read and decode tray items' named icons and notification pictures, shared by
/// every app: a hung read holds
/// one thread and never starts another, and an app floods only its own share of the queue.
pub(super) struct Decoders {
    jobs: async_channel::Sender<Job>,
    /// Decodes queued or running, by app.
    held: Arc<Mutex<HashMap<String, usize>>>,
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
}

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
        let (jobs, rx) = async_channel::bounded::<Job>(limits.queue);
        for _ in 0..limits.threads {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new()
                .name("tray-icon".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv_blocking() {
                        if catch_unwind(AssertUnwindSafe(job)).is_err() {
                            log::warn!("tray: an icon decode panicked");
                        }
                    }
                });
            if let Err(e) = spawned {
                log::warn!("tray: no icon thread: {e}");
            }
        }
        Decoders {
            jobs,
            held: Arc::default(),
            limits,
        }
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
        let job: Job = Box::new(move || {
            let _held = held;
            if tx.try_send(Progress::Started).is_ok() {
                let _ = tx.send_blocking(Progress::Done(decode()));
            }
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
        within(self.limits.within, done).await
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
