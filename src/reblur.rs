//! Caps how often client content re-blurs glass (`[glass] blur_rate`). Frames
//! asked for only by client commits and the pointer reuse the last blur until
//! the cap is up; any other ask blurs at once.

use std::time::Duration;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;

pub fn cap(rate: u16) -> Option<Duration> {
    (rate > 0).then(|| Duration::from_secs(1) / u32::from(rate))
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

    fn ms(n: u64) -> Option<Duration> {
        Some(Duration::from_millis(n))
    }

    #[test]
    fn the_cap_is_a_second_over_the_rate_and_zero_caps_nothing() {
        assert_eq!(cap(30), Some(Duration::from_nanos(33_333_333)));
        assert_eq!(cap(60), Some(Duration::from_nanos(16_666_666)));
        assert_eq!(cap(1), Some(Duration::from_secs(1)));
        assert_eq!(cap(0), None);
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
    fn a_first_frame_never_waits() {
        assert_eq!(when(true, None, CAP), Reblur::Now);
    }

    #[test]
    fn a_frame_asked_for_by_anything_else_blurs_at_once() {
        assert_eq!(when(false, ms(1), CAP), Reblur::Now);
    }
}
