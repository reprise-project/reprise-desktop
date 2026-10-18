//! An action asked for by hand races [`ACTION_DEADLINE`]: its thread and the host's timer share a
//! [`Gate`], and whichever comes first, the answer or the deadline, wins.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// How long an action asked for by hand waits on the file system before it gives up. A disk waking
/// from standby or a network mount reconnecting answers within about 10 s.
pub const ACTION_DEADLINE: Duration = Duration::from_secs(15);

const WAITING: u8 = 0;
const CHANGING: u8 = 1;
const LANDED: u8 = 2;
const GAVE_UP: u8 = 3;
const GAVE_UP_CHANGING: u8 = 4;

/// An action's race against its deadline, cloned into its thread and its timer.
#[derive(Clone, Default)]
pub struct Gate(Arc<AtomicU8>);

/// What becomes of an action's answer.
#[derive(Debug, PartialEq, Eq)]
pub enum Landing {
    /// In time: it runs.
    Landed,
    /// Past the deadline, nothing changed: it is dropped.
    Dropped,
    /// Past the deadline, after [`Gate::change`]: dropped, and the view refreshes.
    Refresh,
}

impl Gate {
    /// Called before the work's first change on `at`'s file system; false once the action gave up.
    /// The free-space query reaches the server on a network mount, so a dead mount holds the work
    /// here, before anything changes; a call blocked past this point may still land.
    pub fn change(&self, at: &Path) -> bool {
        let _ = crate::places::statvfs_totals(at);
        self.commit()
    }

    fn commit(&self) -> bool {
        match (self.0).compare_exchange(WAITING, CHANGING, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => true,
            Err(now) => now == CHANGING,
        }
    }

    /// The deadline passed: whether a change may still land, or None when the answer came first.
    pub fn give_up(&self) -> Option<bool> {
        let was = (self.0).try_update(Ordering::AcqRel, Ordering::Acquire, |s| match s {
            WAITING => Some(GAVE_UP),
            CHANGING => Some(GAVE_UP_CHANGING),
            _ => None,
        });
        match was {
            Ok(was) => Some(was == CHANGING),
            Err(_) => None,
        }
    }

    pub fn given_up(&self) -> bool {
        self.0.load(Ordering::Acquire) >= GAVE_UP
    }

    /// The answer came: called once by the action's thread.
    pub fn land(&self) -> Landing {
        let was = (self.0).try_update(Ordering::AcqRel, Ordering::Acquire, |s| {
            (s < LANDED).then_some(LANDED)
        });
        match was {
            Err(GAVE_UP) => Landing::Dropped,
            Err(GAVE_UP_CHANGING) => Landing::Refresh,
            _ => Landing::Landed,
        }
    }
}

/// The give-up's error text for an action that `changes` files; `changing` when the work had
/// passed [`Gate::change`].
pub fn not_responding(changes: bool, changing: bool) -> String {
    let rest = match (changes, changing) {
        (false, _) => "",
        (true, false) => " Nothing was changed.",
        (true, true) => " The change may still be made once it answers.",
    };
    format!("The location is not responding.{rest}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gate_lets_the_first_of_answer_and_deadline_win() {
        let g = Gate::default();
        assert!(g.commit());
        assert!(g.commit(), "a second step of the same change goes on");
        assert_eq!(g.give_up(), Some(true), "a change was under way");
        assert_eq!(g.land(), Landing::Refresh);

        let g = Gate::default();
        assert_eq!(g.give_up(), Some(false));
        assert!(g.given_up());
        assert!(!g.commit());
        assert_eq!(g.land(), Landing::Dropped);

        let g = Gate::default();
        assert_eq!(g.land(), Landing::Landed);
        assert_eq!(g.give_up(), None, "the answer came first");
        assert!(!g.given_up());
    }

    #[test]
    fn the_give_up_says_whether_a_change_may_still_land() {
        assert_eq!(
            not_responding(false, false),
            "The location is not responding."
        );
        assert_eq!(
            not_responding(true, false),
            "The location is not responding. Nothing was changed."
        );
        assert_eq!(
            not_responding(true, true),
            "The location is not responding. The change may still be made once it answers."
        );
    }
}
