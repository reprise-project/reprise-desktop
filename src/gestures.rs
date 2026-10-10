//! The touchpad swipes the compositor keeps (`[gestures]`): with the configured finger count, a
//! horizontal swipe slides the desktops with the fingers and a vertical one fades the window overview
//! in (up) or out (down). Other finger counts, pinches and holds go on to the client under the pointer (`pointers`).
//!
//! A kept swipe locks to an axis once it has travelled [`LOCK`], then reports its progress along that
//! axis: the travel over [`DISTANCE`], signed. On lift it commits past [`COMMIT_AT`] of the way or on a
//! flick of at least [`FLICK`] the way it went, and springs back otherwise; a flick back springs back
//! from any distance. A cancelled swipe (a finger added or lifted midway) springs back.

/// The travel for the whole way, in libinput's swipe units (pixels at 1000 dpi, accelerated, and
/// slowed to about 0.3 of that on a touchpad): some 3 to 4 cm of finger travel at an even pace.
pub const DISTANCE: f64 = 400.0;
pub const COMMIT_AT: f64 = 0.3;
/// The travel after which a swipe picks its axis: the one it went further along.
pub const LOCK: f64 = 12.0;
/// The speed of a flick, in swipe units per millisecond, taken over the last [`FLICK_MS`].
pub const FLICK: f64 = 0.3;
pub const FLICK_MS: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// Progress is positive toward the next desktop: the fingers move left.
    Horizontal,
    Vertical,
}

/// A kept swipe under way: its travel, the axis once locked, and its recent motion along that axis for the flick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracker {
    dx: f64,
    dy: f64,
    axis: Option<Axis>,
    recent: Vec<(u64, f64)>,
    locked_at: u64,
}

impl Tracker {
    /// The fingers moved by `(dx, dy)` at `ms`: the axis and the progress along it, or None while no axis is picked.
    pub fn update(&mut self, dx: f64, dy: f64, ms: u64) -> Option<(Axis, f64)> {
        self.dx += dx;
        self.dy += dy;
        if self.axis.is_none() && self.dx.hypot(self.dy) >= LOCK {
            self.axis = Some(if self.dx.abs() >= self.dy.abs() {
                Axis::Horizontal
            } else {
                Axis::Vertical
            });
            self.locked_at = ms;
        }
        let axis = self.axis?;
        let along = match axis {
            Axis::Horizontal => -dx,
            Axis::Vertical => -dy,
        };
        self.recent.push((ms, along));
        self.recent.retain(|&(t, _)| t + FLICK_MS >= ms && t <= ms);
        self.progress()
    }

    /// The axis and the progress along it: the travel over [`DISTANCE`], signed as [`Axis`] says, not clamped.
    pub fn progress(&self) -> Option<(Axis, f64)> {
        let axis = self.axis?;
        let travel = match axis {
            Axis::Horizontal => -self.dx,
            Axis::Vertical => -self.dy,
        };
        Some((axis, travel / DISTANCE))
    }

    /// The speed along the axis over the last [`FLICK_MS`] before `ms`, or since the axis was picked if sooner, in the progress's sense.
    fn speed(&self, ms: u64) -> f64 {
        let moved: f64 = self
            .recent
            .iter()
            .filter(|&&(t, _)| t + FLICK_MS >= ms)
            .map(|&(_, d)| d)
            .sum();
        let span = ms.saturating_sub(self.locked_at).clamp(10, FLICK_MS);
        moved / span as f64
    }

    /// The fingers lifted at `ms` (`cancelled` when libinput gave up): the axis, the progress and whether
    /// the swipe commits the way the progress points. None when no axis was picked.
    pub fn release(&self, ms: u64, cancelled: bool) -> Option<(Axis, f64, bool)> {
        let (axis, p) = self.progress()?;
        if cancelled || p == 0.0 {
            return Some((axis, p, false));
        }
        let v = self.speed(ms) * p.signum();
        let commit = if v <= -FLICK {
            false
        } else {
            v >= FLICK || p.abs() >= COMMIT_AT
        };
        Some((axis, p, commit))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swipe(steps: &[(f64, f64)]) -> Tracker {
        let mut t = Tracker::default();
        for (i, &(dx, dy)) in steps.iter().enumerate() {
            t.update(dx, dy, i as u64 * 10);
        }
        t
    }

    fn end(steps: &[(f64, f64)]) -> Option<(Axis, f64, bool)> {
        let t = swipe(steps);
        let ms = steps.len() as u64 * 10;
        t.release(ms, false)
    }

    #[test]
    fn the_axis_waits_for_the_lock_travel_and_then_stays() {
        let mut t = Tracker::default();
        assert_eq!(t.update(-5.0, 3.0, 0), None);
        assert_eq!(
            t.update(-8.0, 0.0, 10),
            Some((Axis::Horizontal, 13.0 / DISTANCE))
        );
        assert_eq!(
            t.update(0.0, -300.0, 20),
            Some((Axis::Horizontal, 13.0 / DISTANCE))
        );
        let mut t = Tracker::default();
        assert_eq!(
            t.update(4.0, -20.0, 0),
            Some((Axis::Vertical, 20.0 / DISTANCE))
        );
    }

    #[test]
    fn progress_is_signed_toward_the_next_desktop_and_up() {
        let t = swipe(&[(-200.0, 0.0)]);
        assert_eq!(t.progress(), Some((Axis::Horizontal, 0.5)));
        let t = swipe(&[(80.0, 0.0)]);
        assert_eq!(t.progress(), Some((Axis::Horizontal, -0.2)));
        let t = swipe(&[(0.0, 400.0)]);
        assert_eq!(t.progress(), Some((Axis::Vertical, -1.0)));
    }

    #[test]
    fn a_slow_swipe_commits_past_thirty_percent_and_springs_back_short_of_it() {
        let slow = |n: usize| {
            let mut steps = vec![(-4.0, 0.0); n];
            steps.extend(std::iter::repeat_n((0.0, 0.0), 11));
            steps
        };
        let (axis, p, commit) = end(&slow(32)).unwrap();
        assert_eq!(axis, Axis::Horizontal);
        assert!((p - 0.32).abs() < 1e-9);
        assert!(commit);
        assert_eq!(end(&slow(28)).map(|r| r.2), Some(false));
    }

    #[test]
    fn a_flick_commits_from_a_short_distance_and_a_flick_back_springs_back() {
        // 72 units counted after the lock in the last 100 ms: 0.72 per ms, past FLICK, from 20 % of the way.
        assert_eq!(end(&[(-8.0, 0.0); 10]), Some((Axis::Horizontal, 0.2, true)));
        // 10 units over the 50 ms since the lock: 0.2 per ms, and 5 % of the way.
        assert_eq!(end(&[(-2.0, 0.0); 10]).map(|r| r.2), Some(false));
        // A flick shorter than the window counts over its own time: 20 units in the 20 ms since the lock.
        assert_eq!(end(&[(-10.0, 0.0); 3]).map(|r| r.2), Some(true));
        let mut steps = vec![(-30.0, 0.0); 10];
        steps.extend([(8.0, 0.0); 10]);
        let (_, p, commit) = end(&steps).unwrap();
        assert!(p > COMMIT_AT);
        assert!(!commit, "a flick back springs back");
        assert_eq!(end(&[(0.0, -8.0); 10]), Some((Axis::Vertical, 0.2, true)));
    }

    #[test]
    fn the_flick_counts_only_the_last_hundred_milliseconds() {
        let mut t = Tracker::default();
        t.update(-80.0, 0.0, 0);
        assert_eq!(t.release(200, false), Some((Axis::Horizontal, 0.2, false)));
        assert_eq!(t.release(50, false).map(|r| r.2), Some(true));
    }

    #[test]
    fn a_cancelled_or_unlocked_swipe_does_nothing() {
        let t = swipe(&[(-320.0, 0.0)]);
        assert_eq!(t.release(10, true), Some((Axis::Horizontal, 0.8, false)));
        assert_eq!(swipe(&[(3.0, 3.0)]).release(10, false), None);
    }
}
