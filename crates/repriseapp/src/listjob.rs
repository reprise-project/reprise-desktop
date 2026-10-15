//! A folder listing read on a thread, so a mount that stopped answering hangs the read and never
//! the program's loop. The loop waits a moment for it, else takes it once the thread wakes it.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use crate::Waker;

/// How long a loop waits for a listing before it goes on showing the old one.
pub const WAIT: Duration = Duration::from_millis(300);

/// A read running on its own thread.
pub struct Job<T>(Receiver<T>);

impl<T: Send + 'static> Job<T> {
    /// Runs `read` on a thread that fires `wake`, if given, once it answers.
    pub fn spawn(wake: Option<&Waker>, read: impl FnOnce() -> T + Send + 'static) -> Job<T> {
        let (tx, rx) = channel();
        let wake = wake.cloned();
        let spawned = std::thread::Builder::new()
            .name("listing".into())
            .spawn(move || {
                if tx.send(read()).is_ok()
                    && let Some(w) = wake
                {
                    w.wake();
                }
            });
        if let Err(e) = spawned {
            log::warn!("cannot spawn a listing thread ({e})");
        }
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
}
