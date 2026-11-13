//! A folder listing read on a thread, so a mount that stopped answering hangs the read and never
//! the program's loop. The loop waits a moment for it, else takes it once the thread wakes it.

use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use crate::Waker;
use crate::pool::Pool;

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
static POOL: Pool = Pool::new("listing", MAX_THREADS, MAX_ALL_THREADS, STUCK);

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
