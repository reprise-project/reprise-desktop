//! Caps how often client content and desktop water re-blur glass (`[glass] blur_rate`). Frames
//! asked for only by client commits, water steps and the pointer reuse the last blur until the
//! cap is up; any other ask blurs at once.

use std::time::Duration;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;

/// The least time between two blurs at `rate` on an output refreshing every `interval`. Frames
/// start on refreshes with some jitter, so one up to a quarter interval early counts as on time.
pub fn cap(rate: u16, interval: Duration) -> Option<Duration> {
    (rate > 0).then(|| (Duration::from_secs(1) / u32::from(rate)).saturating_sub(interval / 4))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reblur {
    Now,
    Wait(Duration),
}

/// When glass re-blurs: at once, unless the frame is `content_only` and the
/// last free blur `since` is younger than `cap`. No cap never waits.
pub fn when(content_only: bool, since: Option<Duration>, cap: Option<Duration>) -> Reblur {
    match (since, cap) {
        (Some(t), Some(cap)) if content_only && t < cap => Reblur::Wait(cap - t),
        _ => Reblur::Now,
    }
}

/// The wait before a frame of its own re-blurs what a frame `held`: none while the water asks for
/// frames, which re-blur once the cap is up; an extra frame would hold the next water frame in turn.
pub fn catch_up(held: Option<Duration>, water_moves: bool) -> Option<Duration> {
    held.filter(|_| !water_moves)
}

impl State {
    pub fn arm_reblur(&mut self, wait: Duration) {
        if self.reblur_timer.is_some() {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(wait), |_, _, state| {
                state.reblur_timer = None;
                if state.glass.refresh_stale() {
                    state.redraw_all();
                }
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.reblur_timer = Some(t),
            Err(e) => log::warn!("glass: reblur timer: {}", e.error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: Option<Duration> = Some(Duration::from_millis(33));
    const HZ60: Duration = Duration::from_nanos(16_666_667);

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn the_cap_is_a_second_over_the_rate_less_a_quarter_refresh_and_zero_caps_nothing() {
        assert_eq!(cap(30, HZ60), Some(Duration::from_nanos(29_166_667)));
        assert_eq!(cap(1, Duration::ZERO), Some(Duration::from_secs(1)));
        assert_eq!(cap(0, HZ60), None);
    }

    #[test]
    fn a_frame_two_refreshes_on_blurs_at_30_even_a_little_early() {
        let early = Some(Duration::from_micros(33_100));
        assert_eq!(when(true, early, cap(30, HZ60)), Reblur::Now);
        assert!(matches!(
            when(true, Some(HZ60), cap(30, HZ60)),
            Reblur::Wait(_)
        ));
    }

    #[test]
    fn with_no_cap_content_blurs_at_once() {
        assert_eq!(when(true, ms(0), None), Reblur::Now);
    }

    #[test]
    fn content_within_the_cap_waits_for_the_rest_of_it() {
        assert_eq!(when(true, ms(0), CAP), Reblur::Wait(CAP.unwrap()));
        assert_eq!(
            when(true, ms(10), CAP),
            Reblur::Wait(Duration::from_millis(23))
        );
        assert_eq!(
            when(true, ms(32), CAP),
            Reblur::Wait(Duration::from_millis(1))
        );
    }

    #[test]
    fn content_blurs_once_the_cap_is_up() {
        assert_eq!(when(true, ms(33), CAP), Reblur::Now);
        assert_eq!(when(true, ms(500), CAP), Reblur::Now);
    }

    #[test]
    fn moving_water_catches_up_a_held_blur_and_still_water_arms_a_frame() {
        let held = Some(Duration::from_millis(12));
        assert_eq!(catch_up(held, true), None);
        assert_eq!(catch_up(held, false), held);
        assert_eq!(catch_up(None, false), None);
    }

    #[test]
    fn a_first_frame_never_waits() {
        assert_eq!(when(true, None, CAP), Reblur::Now);
    }

    #[test]
    fn a_frame_asked_for_by_anything_else_blurs_at_once() {
        assert_eq!(when(false, ms(1), CAP), Reblur::Now);
    }
}
