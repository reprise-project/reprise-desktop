//! A source followed on a thread of its own: each run reads it until it ends, sending what it reads
//! through a [`Feed`], and the thread runs it again after a pause that doubles from [`RETRY_MIN`] to
//! [`RETRY_MAX`] while runs end without delivering, back to the minimum once one delivers.

use std::sync::mpsc::Sender;
use std::time::Duration;

pub const RETRY_MIN: Duration = Duration::from_secs(2);
pub const RETRY_MAX: Duration = Duration::from_secs(60);

/// How a run ended: `Closed` (the receiver is gone, or the source said to stop) ends the thread.
#[derive(Debug, PartialEq, Eq)]
pub enum Run {
    Ended { delivered: bool },
    Closed,
}

/// The thread's end of the channel: sends a value and wakes the receiver only when it differs from
/// the last one sent.
pub struct Feed<T> {
    tx: Sender<T>,
    waker: Box<dyn Fn() + Send>,
    last: Option<T>,
    /// Unavailable since the last run that delivered; the reason is logged once per outage, not on
    /// every retry.
    down: bool,
}

impl<T: Clone + Default + PartialEq> Feed<T> {
    pub fn new(tx: Sender<T>, waker: impl Fn() + Send + 'static) -> Feed<T> {
        Feed {
            tx,
            waker: Box::new(waker),
            last: None,
            down: false,
        }
    }

    /// False once the receiver is gone.
    pub fn send(&mut self, value: T) -> bool {
        if self.last.as_ref() == Some(&value) {
            return true;
        }
        if self.tx.send(value.clone()).is_err() {
            return false;
        }
        self.last = Some(value);
        (self.waker)();
        true
    }

    /// [`Feed::unavailable`], logging `why` unless the outage was logged.
    pub fn unavailable_because(&mut self, why: std::fmt::Arguments) -> Run {
        if self.outage_starts() {
            log::info!("{why}");
        }
        self.unavailable()
    }

    /// Send the default value, for a source that is not there.
    pub fn unavailable(&mut self) -> Run {
        if self.send(T::default()) {
            Run::Ended { delivered: false }
        } else {
            Run::Closed
        }
    }

    fn outage_starts(&mut self) -> bool {
        !std::mem::replace(&mut self.down, true)
    }

    /// Note how a run ended: one that delivered ends the outage.
    fn ended(&mut self, delivered: bool) {
        if delivered {
            self.down = false;
        }
    }
}

/// The pause before the next run: the minimum after a run that delivered, else doubled up to the
/// maximum.
pub fn next_pause(pause: Duration, delivered: bool) -> Duration {
    if delivered {
        RETRY_MIN
    } else {
        (pause * 2).min(RETRY_MAX)
    }
}

/// Call `run` on a thread called `name` until it returns [`Run::Closed`], pausing between runs.
pub fn follow<T: Clone + Default + PartialEq + Send + 'static>(
    name: &str,
    tx: Sender<T>,
    waker: impl Fn() + Send + 'static,
    mut run: impl FnMut(&mut Feed<T>) -> Run + Send + 'static,
) {
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let mut feed = Feed::new(tx, waker);
            let mut pause = RETRY_MIN;
            while let Run::Ended { delivered } = run(&mut feed) {
                feed.ended(delivered);
                pause = next_pause(pause, delivered);
                std::thread::sleep(pause);
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not start the {name} thread: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn the_retry_pause_doubles_to_a_minute_and_resets_on_a_live_run() {
        let mut p = RETRY_MIN;
        let mut seen = Vec::new();
        for _ in 0..7 {
            p = next_pause(p, false);
            seen.push(p.as_secs());
        }
        assert_eq!(seen, [4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(next_pause(p, true), RETRY_MIN);
    }

    #[test]
    fn the_feed_sends_and_wakes_only_on_a_change() {
        let (tx, rx) = channel();
        let (woke, wakes) = channel();
        let mut feed = Feed::new(tx, move || woke.send(()).unwrap());
        assert!(feed.send(0));
        assert!(feed.send(0));
        assert!(feed.send(7));
        assert_eq!(rx.try_iter().collect::<Vec<_>>(), [0, 7]);
        assert_eq!(wakes.try_iter().count(), 2);
        drop(rx);
        assert!(!feed.send(1), "the receiver is gone");
        assert_eq!(feed.unavailable(), Run::Closed);
    }

    #[test]
    fn an_outage_is_logged_once_until_a_run_delivers() {
        let (tx, _rx) = channel::<u8>();
        let mut feed = Feed::new(tx, || {});
        assert!(feed.outage_starts());
        feed.ended(false);
        assert!(!feed.outage_starts(), "a retry that fails again is quiet");
        feed.ended(true);
        assert!(feed.outage_starts(), "a new outage after a live run");
    }
}
