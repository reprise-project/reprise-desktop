//! Time-driven levels for small fades, slides and tweens. Pure: the host passes
//! the clock in, ticks while `settled` is false and draws with `level` or
//! `x`.

use std::time::Instant;

/// A highlight fading toward lit or dark at its own speed each way. The
/// level runs linearly in time and is eased (smoothstep) as it is read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HoverFade {
    from: f32,
    lit: bool,
    since: Option<Instant>,
    ms: u32,
}

impl HoverFade {
    pub fn set(&mut self, lit: bool, now: Instant, in_ms: u32, out_ms: u32) {
        if lit == self.lit && self.since.is_some() {
            return;
        }
        self.from = self.linear(now);
        self.lit = lit;
        self.ms = if lit { in_ms } else { out_ms };
        self.since = Some(now);
    }

    pub fn level(&self, now: Instant) -> f32 {
        let t = self.linear(now);
        t * t * (3.0 - 2.0 * t)
    }

    pub fn settled(&self, now: Instant) -> bool {
        self.linear(now) == if self.lit { 1.0 } else { 0.0 }
    }

    pub fn lit(&self) -> bool {
        self.lit
    }

    fn linear(&self, now: Instant) -> f32 {
        let Some(since) = self.since else { return 0.0 };
        let target = if self.lit { 1.0 } else { 0.0 };
        if self.ms == 0 {
            return target;
        }
        let step = now.saturating_duration_since(since).as_secs_f32() * 1000.0 / self.ms as f32;
        if self.lit {
            (self.from + step).min(1.0)
        } else {
            (self.from - step).max(0.0)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ease {
    #[default]
    Out,
    InOut,
    Linear,
    OutCubic,
    /// Overshoots 1 by a few percent, then settles.
    OutBack,
}

impl Ease {
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Ease::Out => 1.0 - (1.0 - t) * (1.0 - t),
            Ease::InOut => t * t * (3.0 - 2.0 * t),
            Ease::Linear => t,
            Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
            Ease::OutBack if t <= 0.0 || t >= 1.0 => t.clamp(0.0, 1.0),
            Ease::OutBack => {
                const C1: f32 = 1.4;
                let u = t - 1.0;
                1.0 + u * u * ((C1 + 1.0) * u + C1)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slide {
    from: i32,
    to: i32,
    since: Option<Instant>,
    ms: u32,
    ease: Ease,
}

impl Slide {
    pub fn new(from: i32, to: i32, now: Instant, ms: u32, ease: Ease) -> Slide {
        Slide {
            from,
            to,
            since: Some(now),
            ms,
            ease,
        }
    }

    pub fn at(x: i32) -> Slide {
        Slide {
            from: x,
            to: x,
            ..Slide::default()
        }
    }

    pub fn x(&self, now: Instant) -> i32 {
        let t = self.ease.apply(self.linear(now));
        (self.from as f32 + (self.to - self.from) as f32 * t).round() as i32
    }

    pub fn offset(&self, now: Instant) -> i32 {
        self.x(now) - self.to
    }

    pub fn settled(&self, now: Instant) -> bool {
        self.linear(now) >= 1.0
    }

    pub fn retarget(&mut self, to: i32, now: Instant) {
        self.from = self.x(now);
        self.to = to;
        self.since = Some(now);
    }

    fn linear(&self, now: Instant) -> f32 {
        let Some(since) = self.since else { return 1.0 };
        if self.ms == 0 {
            return 1.0;
        }
        let t = now.saturating_duration_since(since).as_secs_f32() * 1000.0 / self.ms as f32;
        t.min(1.0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tween {
    from: f64,
    to: f64,
    since: Option<Instant>,
    ms: u32,
    ease: Ease,
}

impl Tween {
    pub fn new(from: f64, to: f64, now: Instant, ms: u32, ease: Ease) -> Tween {
        Tween {
            from,
            to,
            since: Some(now),
            ms,
            ease,
        }
    }

    pub fn at(v: f64) -> Tween {
        Tween {
            from: v,
            to: v,
            ..Tween::default()
        }
    }

    pub fn value(&self, now: Instant) -> f64 {
        let t = self.linear(now);
        if t >= 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * f64::from(self.ease.apply(t as f32))
    }

    pub fn target(&self) -> f64 {
        self.to
    }

    pub fn settled(&self, now: Instant) -> bool {
        self.linear(now) >= 1.0
    }

    pub fn retarget(&mut self, to: f64, now: Instant, ms: u32) {
        self.from = self.value(now);
        self.to = to;
        self.since = Some(now);
        self.ms = ms;
    }

    /// Aim at `to` the way a queue of steps runs: a settled tween takes
    /// `first` ms, and one still under way gets `then` ms more than it had
    /// left, eased on from where it is.
    pub fn queue(&mut self, to: f64, now: Instant, first: u32, then: u32) {
        let ms = if self.settled(now) {
            first
        } else {
            let left = f64::from(self.ms) * (1.0 - self.linear(now));
            left.round() as u32 + then
        };
        self.retarget(to, now, ms);
    }

    fn linear(&self, now: Instant) -> f64 {
        let Some(since) = self.since else { return 1.0 };
        if self.ms == 0 {
            return 1.0;
        }
        let t = now.saturating_duration_since(since).as_secs_f64() * 1000.0 / f64::from(self.ms);
        t.min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn at(t0: Instant, ms: u64) -> Instant {
        t0 + Duration::from_millis(ms)
    }

    #[test]
    fn out_back_overshoots_then_settles() {
        assert_eq!(
            (Ease::OutBack.apply(0.0), Ease::OutBack.apply(1.0)),
            (0.0, 1.0)
        );
        assert!(Ease::OutBack.apply(0.6) > 1.0);
        let t0 = Instant::now();
        let t = Tween::new(0.0, 1.0, t0, 100, Ease::OutBack);
        assert!(t.value(at(t0, 60)) > 1.0);
        assert_eq!(t.value(at(t0, 100)), 1.0);
    }

    #[test]
    fn a_fade_starts_dark_lights_over_its_time_and_settles() {
        let t0 = Instant::now();
        let mut f = HoverFade::default();
        assert_eq!(f.level(t0), 0.0);
        assert!(f.settled(t0));
        f.set(true, t0, 100, 200);
        assert_eq!(f.level(t0), 0.0);
        assert!(!f.settled(at(t0, 50)));
        let mid = f.level(at(t0, 50));
        assert!(
            (0.45..0.55).contains(&mid),
            "half way is half lit, eased symmetric: {mid}"
        );
        assert_eq!(f.level(at(t0, 100)), 1.0);
        assert!(f.settled(at(t0, 100)));
        assert_eq!(f.level(at(t0, 5000)), 1.0);
    }

    #[test]
    fn going_dark_takes_the_out_time_and_a_reversal_starts_where_it_is() {
        let t0 = Instant::now();
        let mut f = HoverFade::default();
        f.set(true, t0, 100, 200);
        f.set(false, at(t0, 100), 100, 200);
        assert_eq!(
            f.level(at(t0, 200)),
            0.5,
            "half way out after 100 of 200 ms"
        );
        f.set(true, at(t0, 200), 100, 200);
        let back = f.level(at(t0, 200));
        assert!((back - 0.5).abs() < 1e-6, "no jump on reversal: {back}");
        assert_eq!(
            f.level(at(t0, 250)),
            1.0,
            "the remaining half takes half the in time"
        );
    }

    #[test]
    fn an_eased_out_slide_starts_fast_and_lands_on_its_target() {
        let t0 = Instant::now();
        let s = Slide::new(0, 100, t0, 100, Ease::Out);
        assert_eq!(s.x(t0), 0);
        assert_eq!(s.offset(t0), -100, "drawn 100 px before its target");
        assert!(!s.settled(t0));
        assert_eq!(
            s.x(at(t0, 50)),
            75,
            "half the time, three quarters of the way"
        );
        assert_eq!(s.x(at(t0, 100)), 100);
        assert_eq!(s.offset(at(t0, 100)), 0);
        assert!(s.settled(at(t0, 100)));
        assert_eq!(s.x(at(t0, 5000)), 100);
    }

    #[test]
    fn a_slide_eased_in_and_out_is_half_way_at_half_time() {
        let t0 = Instant::now();
        let s = Slide::new(200, 100, t0, 200, Ease::InOut);
        assert_eq!(s.x(t0), 200);
        assert_eq!(
            s.x(at(t0, 20)),
            197,
            "slow start: 2.8% of the way after 10%"
        );
        assert_eq!(s.x(at(t0, 100)), 150);
        assert_eq!(s.x(at(t0, 180)), 103, "slow end");
        assert_eq!(s.x(at(t0, 200)), 100);
        assert!(s.settled(at(t0, 200)));
    }

    #[test]
    fn a_retargeted_slide_runs_again_from_where_it_is_drawn() {
        let t0 = Instant::now();
        let mut s = Slide::new(0, 100, t0, 100, Ease::Out);
        s.retarget(-50, at(t0, 50));
        assert_eq!(s.x(at(t0, 50)), 75, "no jump on retarget");
        assert_eq!(s.offset(at(t0, 50)), 125);
        assert!(!s.settled(at(t0, 149)));
        assert_eq!(s.x(at(t0, 150)), -50, "a whole run from the retarget");
    }

    #[test]
    fn queued_steps_add_their_time_to_what_is_left() {
        let t0 = Instant::now();
        let mut t = Tween::at(0.0);
        t.queue(1.0, t0, 150, 200);
        assert!(t.settled(at(t0, 150)), "a first step takes its own time");
        t.queue(2.0, at(t0, 150), 150, 200);
        // Another 60 ms in: 90 ms left, and 200 for the next step.
        t.queue(3.0, at(t0, 210), 150, 200);
        assert!(!t.settled(at(t0, 499)));
        assert!(t.settled(at(t0, 500)));
        assert_eq!(t.value(at(t0, 500)), 3.0);
    }

    #[test]
    fn a_linear_tween_moves_evenly() {
        let t0 = Instant::now();
        let t = Tween::new(0.0, 1.0, t0, 200, Ease::Linear);
        assert!((t.value(at(t0, 50)) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn a_slide_standing_still_is_settled() {
        let t0 = Instant::now();
        let s = Slide::at(40);
        assert_eq!(s.x(t0), 40);
        assert_eq!(s.offset(t0), 0);
        assert!(s.settled(t0));
    }

    #[test]
    fn zero_durations_jump() {
        let t0 = Instant::now();
        let mut f = HoverFade::default();
        f.set(true, t0, 0, 0);
        assert_eq!(f.level(t0), 1.0);
        assert!(f.settled(t0));
        f.set(false, t0, 0, 0);
        assert_eq!(f.level(t0), 0.0);
    }

    #[test]
    fn a_tween_eases_out_to_its_target_and_retargets_from_where_it_is() {
        let t0 = Instant::now();
        let mut t = Tween::new(1.0, 3.0, t0, 100, Ease::Out);
        assert_eq!(t.value(t0), 1.0);
        assert_eq!(t.value(at(t0, 50)), 2.5, "half the time, three quarters");
        assert!(!t.settled(at(t0, 50)));
        t.retarget(0.0, at(t0, 50), 200);
        assert_eq!(t.value(at(t0, 50)), 2.5, "no jump on retarget");
        assert_eq!(t.target(), 0.0);
        assert!(!t.settled(at(t0, 249)));
        assert_eq!(t.value(at(t0, 250)), 0.0, "the new run takes its own time");
        assert!(t.settled(at(t0, 250)));
        assert!(Tween::at(4.0).settled(t0));
        assert_eq!(Tween::at(4.0).value(t0), 4.0);
    }
}
