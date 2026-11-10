use std::time::Duration;

type Job = Box<dyn FnOnce() + Send>;

/// Threads that read and decode tray items' named icons, shared by every item: a flood of
/// fetches runs at most `threads` decodes at once, and a hung read holds one, never starts more.
pub(super) struct Decoders {
    jobs: async_channel::Sender<Job>,
    within: Duration,
}

impl Decoders {
    /// `threads` workers taking at most `queue` waiting decodes, each given `within` from its
    /// call to finish.
    pub(super) fn new(threads: usize, queue: usize, within: Duration) -> Self {
        let (jobs, rx) = async_channel::bounded::<Job>(queue);
        for _ in 0..threads {
            let rx = rx.clone();
            let spawned = std::thread::Builder::new()
                .name("tray-icon".into())
                .spawn(move || {
                    while let Ok(job) = rx.recv_blocking() {
                        job();
                    }
                });
            if let Err(e) = spawned {
                log::warn!("tray: no icon thread: {e}");
            }
        }
        Decoders { jobs, within }
    }

    /// `decode`'s result, or None when it isn't done in time, its wait for room in the queue
    /// included. A decode whose caller has given up is skipped.
    pub(super) async fn run<T: Send + 'static>(
        &self,
        decode: impl FnOnce() -> T + Send + 'static,
    ) -> Option<T> {
        let (tx, rx) = async_channel::bounded(1);
        let job: Job = Box::new(move || {
            if !tx.is_closed() {
                let _ = tx.send_blocking(decode());
            }
        });
        let decoded = async {
            self.jobs.send(job).await.ok()?;
            rx.recv().await.ok()
        };
        let late = async {
            async_io::Timer::after(self.within).await;
            None
        };
        futures_lite::future::or(decoded, late).await
    }
}
