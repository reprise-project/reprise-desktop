//! Frame callbacks for frames with nothing new to show.
//!
//! A screen renders only when something on it changed. Its clients still get
//! frame callbacks, paced at the refresh by a timer (`done`) as a page flip would
//! have, so a client redrawing nothing runs at the refresh rate. A timer that
//! comes due after a commit asked for a frame sends nothing: that frame sends
//! every callback, so no client gets one for content not yet shown.
//!
//! Renders follow the refresh too: an empty frame that asks for the next holds
//! it back a refresh interval (`next_render`), so an animation never renders
//! faster than the screen shows. A frame that is not shown sends no presentation
//! feedback.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Done {
    Now,
    After(Duration),
}

/// How early a callback may go out before a full refresh interval: timers tick
/// in whole milliseconds.
const SLACK: Duration = Duration::from_millis(1);

/// When the clients get their frame callbacks after a frame that was `shown`
/// or not, on a screen refreshing every `interval`, `since` they last got them.
/// A shown frame, and the frame after a page flip (`after_flip`), send at once;
/// any other waits until `interval` has passed since the last.
pub fn done(shown: bool, after_flip: bool, since: Option<Duration>, interval: Duration) -> Done {
    match since {
        Some(t) if !shown && !after_flip && t + SLACK < interval => Done::After(interval - t),
        _ => Done::Now,
    }
}

/// A screen's refresh interval for `refresh` in mHz; 60 Hz if none.
pub fn interval(refresh: i32) -> Duration {
    let mhz = if refresh > 0 { refresh as u64 } else { 60_000 };
    Duration::from_nanos(1_000_000_000_000 / mhz)
}

/// When the next render may start after one that started at `at`, `shown` or
/// not, with the next already `asked` for. A shown frame is paced by its page
/// flip; an empty one that asks again has no flip to wait for, so it waits out
/// the refresh `interval`. None: no wait.
pub fn next_render(shown: bool, asked: bool, at: Instant, interval: Duration) -> Option<Instant> {
    (!shown && asked).then(|| at + interval)
}

/// How long a render due `now` waits for `not_before` (`next_render`).
pub fn render_wait(not_before: Option<Instant>, now: Instant) -> Option<Duration> {
    not_before
        .map(|t| t.saturating_duration_since(now))
        .filter(|d| !d.is_zero())
}

#[derive(Default)]
pub struct Pace {
    pub last_done: Option<Instant>,
    pub flipped: bool,
    pub not_before: Option<Instant>,
}

impl Pace {
    pub fn done(&mut self, shown: bool, now: Instant, interval: Duration) -> Done {
        let after_flip = std::mem::replace(&mut self.flipped, shown);
        done(
            shown,
            after_flip,
            self.last_done.map(|t| now.saturating_duration_since(t)),
            interval,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HZ60: Duration = Duration::from_nanos(16_666_666);

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_shown_frame_sends_callbacks_at_once() {
        assert_eq!(done(true, false, Some(ms(2)), HZ60), Done::Now);
        assert_eq!(done(true, true, Some(ms(0)), HZ60), Done::Now);
    }

    #[test]
    fn the_frame_after_a_flip_sends_callbacks_at_once() {
        assert_eq!(done(false, true, Some(ms(9)), HZ60), Done::Now);
    }

    #[test]
    fn an_empty_frame_waits_out_the_refresh_interval() {
        assert_eq!(
            done(false, false, Some(ms(2)), HZ60),
            Done::After(HZ60 - ms(2))
        );
        assert_eq!(done(false, false, Some(ms(0)), HZ60), Done::After(HZ60));
    }

    #[test]
    fn an_empty_frame_after_a_full_interval_sends_at_once() {
        assert_eq!(done(false, false, Some(ms(17)), HZ60), Done::Now);
        assert_eq!(
            done(false, false, Some(ms(16)), HZ60),
            Done::Now,
            "within the slack"
        );
        assert_eq!(done(false, false, None, HZ60), Done::Now);
    }

    #[test]
    fn the_interval_follows_the_refresh() {
        assert_eq!(interval(60_000), HZ60);
        assert_eq!(interval(144_000), Duration::from_nanos(6_944_444));
        assert_eq!(interval(0), HZ60, "a screen without a refresh");
    }

    #[test]
    fn an_empty_render_that_asks_again_holds_the_next_a_refresh() {
        let t = Instant::now();
        assert_eq!(next_render(false, true, t, HZ60), Some(t + HZ60));
        assert_eq!(next_render(true, true, t, HZ60), None, "the flip paces it");
        assert_eq!(
            next_render(false, false, t, HZ60),
            None,
            "nothing asked: the animation is over"
        );
    }

    #[test]
    fn a_held_render_waits_only_what_is_left_of_the_refresh() {
        let t = Instant::now();
        let held = next_render(false, true, t, HZ60);
        assert_eq!(render_wait(held, t + ms(5)), Some(HZ60 - ms(5)));
        assert_eq!(render_wait(held, t + HZ60), None);
        assert_eq!(render_wait(held, t + ms(40)), None);
        assert_eq!(render_wait(None, t), None);
    }

    #[test]
    fn only_the_frame_right_after_a_shown_one_follows_a_flip() {
        let mut pace = Pace::default();
        let t = Instant::now();
        pace.last_done = Some(t);
        assert_eq!(pace.done(true, t, HZ60), Done::Now);
        assert_eq!(
            pace.done(false, t + ms(5), HZ60),
            Done::Now,
            "after the flip"
        );
        assert_eq!(
            pace.done(false, t + ms(5), HZ60),
            Done::After(HZ60 - ms(5)),
            "the next empty frame waits"
        );
    }
}
