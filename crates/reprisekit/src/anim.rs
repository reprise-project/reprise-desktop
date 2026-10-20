//! Time-driven levels for small fades, slides and tweens. Pure: the host passes
//! the clock in, ticks while `settled` is false and draws with `level` or
//! `x`.

use std::time::Instant;

use crate::raster::Rect;

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

/// The side a menu unrolls from as it opens: the one facing what opened it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnrollFrom {
    Top,
    Bottom,
    Left,
    Right,
}

impl UnrollFrom {
    /// For a menu at `menu` opened from the point `at` (a click or a key's anchor): up when it
    /// hangs above `at`, sideways when it stands beside it, down otherwise.
    pub fn facing(menu: Rect, at: (i32, i32)) -> UnrollFrom {
        let level = at.1 > menu.y;
        if at.1 >= menu.bottom() {
            UnrollFrom::Bottom
        } else if level && at.0 >= menu.right() {
            UnrollFrom::Right
        } else if level && at.0 < menu.x {
            UnrollFrom::Left
        } else {
            UnrollFrom::Top
        }
    }

    /// For a submenu at `menu` opened beside its parent at `parent`.
    pub fn beside(menu: Rect, parent: Rect) -> UnrollFrom {
        if 2 * menu.x + menu.width >= 2 * parent.x + parent.width {
            UnrollFrom::Left
        } else {
            UnrollFrom::Right
        }
    }
}

/// One frame of a box of `size` unrolling from `from` at `t` (0 to 1, linear in time):
/// the box draws moved by `shift` from its place and cut to `clip` (box-local), so its far
/// edge leads in from the near one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unroll {
    pub shift: (i32, i32),
    pub clip: Rect,
}

impl Unroll {
    pub fn at(from: UnrollFrom, (w, h): (i32, i32), t: f32) -> Unroll {
        let e = Ease::OutCubic.apply(t.clamp(0.0, 1.0));
        let shown = |full: i32| ((full as f32 * e).round() as i32).clamp(0, full);
        match from {
            UnrollFrom::Top => {
                let l = shown(h);
                Unroll {
                    shift: (0, l - h),
                    clip: Rect::new(0, h - l, w, l),
                }
            }
            UnrollFrom::Bottom => {
                let l = shown(h);
                Unroll {
                    shift: (0, h - l),
                    clip: Rect::new(0, 0, w, l),
                }
            }
            UnrollFrom::Left => {
                let l = shown(w);
                Unroll {
                    shift: (l - w, 0),
                    clip: Rect::new(w - l, 0, l, h),
                }
            }
            UnrollFrom::Right => {
                let l = shown(w);
                Unroll {
                    shift: (w - l, 0),
                    clip: Rect::new(0, 0, l, h),
                }
            }
        }
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

    #[test]
    fn an_unroll_runs_from_hidden_to_whole_with_the_far_edge_leading() {
        let size = (100, 60);
        for from in [
            UnrollFrom::Top,
            UnrollFrom::Bottom,
            UnrollFrom::Left,
            UnrollFrom::Right,
        ] {
            let start = Unroll::at(from, size, 0.0);
            assert_eq!(
                start.clip.width * start.clip.height,
                0,
                "{from:?}: nothing shows at the start"
            );
            assert_eq!(
                Unroll::at(from, size, 1.0),
                Unroll {
                    shift: (0, 0),
                    clip: Rect::new(0, 0, 100, 60)
                },
                "{from:?}: whole and in place at the end"
            );
        }
        let half = |from| Unroll::at(from, size, 0.5);
        assert_eq!(
            half(UnrollFrom::Top),
            Unroll {
                shift: (0, -7),
                clip: Rect::new(0, 7, 100, 53)
            },
            "from the top, the bottom rows show first just under the top edge"
        );
        assert_eq!(
            half(UnrollFrom::Bottom),
            Unroll {
                shift: (0, 7),
                clip: Rect::new(0, 0, 100, 53)
            },
            "from the bottom, the top rows show first just over the bottom edge"
        );
        assert_eq!(
            half(UnrollFrom::Left),
            Unroll {
                shift: (-12, 0),
                clip: Rect::new(12, 0, 88, 60)
            }
        );
        assert_eq!(
            half(UnrollFrom::Right),
            Unroll {
                shift: (12, 0),
                clip: Rect::new(0, 0, 88, 60)
            }
        );
    }

    #[test]
    fn an_unroll_eases_out_and_never_shrinks() {
        let mut last = 0;
        for i in 0..=20 {
            let u = Unroll::at(UnrollFrom::Top, (10, 200), i as f32 / 20.0);
            assert!(u.clip.height >= last, "step {i}");
            assert_eq!(
                u.clip.y + u.shift.1,
                0,
                "the shown part starts at the top edge"
            );
            last = u.clip.height;
        }
        assert!(
            Unroll::at(UnrollFrom::Top, (10, 200), 0.25).clip.height > 100,
            "more than half shows a quarter of the way in"
        );
    }

    #[test]
    fn a_menu_unrolls_away_from_what_opened_it() {
        let menu = Rect::new(100, 100, 80, 120);
        assert_eq!(UnrollFrom::facing(menu, (100, 100)), UnrollFrom::Top);
        assert_eq!(UnrollFrom::facing(menu, (130, 90)), UnrollFrom::Top);
        assert_eq!(UnrollFrom::facing(menu, (150, 220)), UnrollFrom::Bottom);
        assert_eq!(UnrollFrom::facing(menu, (180, 150)), UnrollFrom::Right);
        assert_eq!(UnrollFrom::facing(menu, (90, 150)), UnrollFrom::Left);
        assert_eq!(UnrollFrom::facing(menu, (120, 150)), UnrollFrom::Top);
        let parent = Rect::new(0, 100, 100, 200);
        assert_eq!(
            UnrollFrom::beside(Rect::new(97, 120, 80, 50), parent),
            UnrollFrom::Left
        );
        assert_eq!(
            UnrollFrom::beside(Rect::new(-77, 120, 80, 50), parent),
            UnrollFrom::Right
        );
    }
}
