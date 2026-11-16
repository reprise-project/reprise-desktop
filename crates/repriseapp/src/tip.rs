//! A pointer resting on something whose tip comes up after a delay, with the window ticking
//! (`Reply::tick`) until it does.

use std::time::{Duration, Instant};

/// The pointer resting on `on` at `at` since `since`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rest<K> {
    pub on: K,
    pub at: (i32, i32),
    pub since: Instant,
}

impl<K: Copy + PartialEq> Rest<K> {
    /// `rest` once the pointer moved to `at` over `on` (None: over nothing with a tip): kept while
    /// the tip for that same `on` is `up`, else started again at `now`.
    pub fn moved(
        rest: Option<Rest<K>>,
        on: Option<K>,
        at: (i32, i32),
        up: bool,
        now: Instant,
    ) -> Option<Rest<K>> {
        let on = on?;
        match rest {
            Some(r) if r.on == on && up => Some(r),
            _ => Some(Rest { on, at, since: now }),
        }
    }

    /// The rest has lasted `delay` at `now`.
    pub fn due(&self, now: Instant, delay: Duration) -> bool {
        now.duration_since(self.since) >= delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rest_restarts_on_each_motion_until_its_tip_is_up() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(100);
        let first = Rest::moved(None, Some(1), (5, 5), false, t0).unwrap();
        let again = Rest::moved(Some(first), Some(1), (6, 5), false, t1).unwrap();
        assert_eq!(
            (again.since, again.at),
            (t1, (6, 5)),
            "not up yet: the wait starts over"
        );
        let up = Rest::moved(Some(first), Some(1), (6, 5), true, t1).unwrap();
        assert_eq!(up, first, "up: the tip stays where it came up");
        let other = Rest::moved(Some(first), Some(2), (9, 9), true, t1).unwrap();
        assert_eq!(
            (other.on, other.since),
            (2, t1),
            "another target waits afresh"
        );
        assert_eq!(Rest::moved(Some(first), None, (0, 0), true, t1), None);
        assert!(!first.due(t1, Duration::from_millis(600)));
        assert!(first.due(t0 + Duration::from_millis(600), Duration::from_millis(600)));
    }
}
