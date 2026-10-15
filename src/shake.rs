//! Shaking a window by its caption: the detector a caption drag feeds and
//! what a shake minimizes or puts back. Sizes are logical pixels.

use std::time::{Duration, Instant};

/// How far the pointer must travel each way before a turn counts.
pub const STROKE_PX: f64 = 40.0;
/// Turns that make a shake.
pub const TURNS: usize = 3;
/// The span the turns must fall within.
pub const WITHIN: Duration = Duration::from_millis(750);

/// Counts quick turns along each axis of a drag; either axis can shake.
#[derive(Debug, Default)]
pub struct Detector {
    x: Axis,
    y: Axis,
    shook: Option<Instant>,
}

impl Detector {
    /// The pointer is at `pos` at `at`: true when this move completes a
    /// shake, after which the count starts over.
    pub fn feed(&mut self, pos: (f64, f64), at: Instant) -> bool {
        let shook = self.x.feed(pos.0, at) | self.y.feed(pos.1, at);
        if shook {
            *self = Detector {
                shook: Some(at),
                ..Detector::default()
            };
        }
        shook
    }

    /// Whether the drag looks like a shake at `at`: two turns, or a shake,
    /// within `WITHIN`. Snapping and tab drops wait while it does.
    pub fn shaking(&self, at: Instant) -> bool {
        let recent = |t: &Instant| at.saturating_duration_since(*t) <= WITHIN;
        self.shook.as_ref().is_some_and(recent)
            || [&self.x, &self.y]
                .iter()
                .any(|a| a.turns.iter().filter(|t| recent(t)).count() >= 2)
    }
}

#[derive(Debug, Default)]
struct Axis {
    /// The farthest point of the stroke under way; None before the first move.
    extreme: Option<f64>,
    /// The stroke's direction, 0 until it is `STROKE_PX` long.
    dir: f64,
    turns: Vec<Instant>,
}

impl Axis {
    fn feed(&mut self, v: f64, at: Instant) -> bool {
        let Some(extreme) = self.extreme else {
            self.extreme = Some(v);
            return false;
        };
        let d = v - extreme;
        if self.dir == 0.0 {
            if d.abs() >= STROKE_PX {
                self.dir = d.signum();
                self.extreme = Some(v);
            }
        } else if d * self.dir > 0.0 {
            self.extreme = Some(v);
        } else if d.abs() >= STROKE_PX {
            self.dir = -self.dir;
            self.extreme = Some(v);
            self.turns.push(at);
        }
        self.turns.retain(|t| at.duration_since(*t) <= WITHIN);
        self.turns.len() >= TURNS
    }
}

/// What the last minimizing shake of `window` minimized, bottom to top.
#[derive(Clone, Debug, PartialEq)]
pub struct Memo<T> {
    pub window: T,
    pub minimized: Vec<T>,
}

#[derive(Debug, PartialEq)]
pub enum Toggle<T> {
    /// Minimize these, bottom to top.
    Minimize(Vec<T>),
    /// Put these back, bottom to top.
    Restore(Vec<T>),
    Nothing,
}

/// What a shake of `shaken` does: put back what its last shake minimized
/// while any of that is still minimized, else minimize `others` (the
/// visible windows it may minimize, bottom to top).
pub fn toggle<T: Copy + PartialEq>(
    memo: Option<&Memo<T>>,
    shaken: T,
    minimized: impl Fn(T) -> bool,
    others: &[T],
) -> Toggle<T> {
    if let Some(m) = memo.filter(|m| m.window == shaken) {
        let back: Vec<T> = (m.minimized.iter().copied())
            .filter(|&w| minimized(w))
            .collect();
        if !back.is_empty() {
            return Toggle::Restore(back);
        }
    }
    if others.is_empty() {
        Toggle::Nothing
    } else {
        Toggle::Minimize(others.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds x positions `step_ms` apart; true once any of them shakes.
    fn shakes(xs: &[f64], step_ms: u64) -> bool {
        let t0 = Instant::now();
        let mut d = Detector::default();
        (xs.iter().enumerate())
            .any(|(i, &x)| d.feed((x, 100.0), t0 + Duration::from_millis(i as u64 * step_ms)))
    }

    #[test]
    fn three_quick_turns_of_forty_pixels_shake() {
        assert!(shakes(&[0.0, 50.0, 0.0, 50.0, 0.0], 60));
    }

    #[test]
    fn two_turns_do_not_shake() {
        assert!(!shakes(&[0.0, 50.0, 0.0, 50.0], 60));
    }

    #[test]
    fn strokes_under_forty_pixels_do_not_count() {
        assert!(!shakes(&[0.0, 39.0, 0.0, 39.0, 0.0, 39.0, 0.0], 30));
    }

    #[test]
    fn turns_spread_past_the_window_do_not_shake() {
        assert!(!shakes(&[0.0, 50.0, 0.0, 50.0, 0.0], 400));
    }

    #[test]
    fn a_turn_is_measured_from_the_far_end_of_the_stroke() {
        // Out to 120, back 40 to 80 turns; 50 more to 130 turns again.
        assert!(shakes(&[0.0, 60.0, 120.0, 80.0, 130.0, 90.0], 50));
    }

    #[test]
    fn a_vertical_shake_counts() {
        let t0 = Instant::now();
        let mut d = Detector::default();
        let ys = [0.0, 60.0, 0.0, 60.0, 0.0];
        let hit = (ys.iter().enumerate())
            .any(|(i, &y)| d.feed((10.0, y), t0 + Duration::from_millis(i as u64 * 50)));
        assert!(hit);
    }

    #[test]
    fn the_count_starts_over_after_a_shake() {
        let t0 = Instant::now();
        let mut d = Detector::default();
        let at = |i: u64| t0 + Duration::from_millis(i * 50);
        let xs = [0.0, 50.0, 0.0, 50.0, 0.0];
        let first: Vec<bool> = (xs.iter().enumerate())
            .map(|(i, &x)| d.feed((x, 0.0), at(i as u64)))
            .collect();
        assert_eq!(first, [false, false, false, false, true]);
        assert!(!d.feed((50.0, 0.0), at(5)));
        assert!(!d.feed((0.0, 0.0), at(6)));
    }

    #[test]
    fn a_slow_drag_across_the_screen_never_shakes() {
        let xs: Vec<f64> = (0..200).map(|i| f64::from(i) * 8.0).collect();
        assert!(!shakes(&xs, 16));
    }

    #[test]
    fn two_turns_and_a_fresh_shake_count_as_shaking_until_they_age() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut d = Detector::default();
        for (i, x) in [0.0, 50.0, 0.0].into_iter().enumerate() {
            d.feed((x, 0.0), at(i as u64 * 50));
        }
        assert!(!d.shaking(at(100)), "one turn is a drag adjusting");
        d.feed((50.0, 0.0), at(150));
        assert!(d.shaking(at(150)));
        assert!(!d.shaking(at(150) + WITHIN + Duration::from_millis(60)));
        assert!(d.feed((0.0, 0.0), at(200)));
        assert!(d.shaking(at(200) + WITHIN));
        assert!(!d.shaking(at(201) + WITHIN));
    }

    #[test]
    fn a_first_shake_minimizes_the_others() {
        assert_eq!(
            toggle(None, 1, |_| false, &[2, 3]),
            Toggle::Minimize(vec![2, 3])
        );
    }

    #[test]
    fn a_second_shake_restores_only_what_the_first_minimized_in_order() {
        let memo = Memo {
            window: 1,
            minimized: vec![2, 3, 4],
        };
        // 3 was put back by hand; 5 was minimized by something else.
        let min = |w| w == 2 || w == 4 || w == 5;
        assert_eq!(
            toggle(Some(&memo), 1, min, &[6]),
            Toggle::Restore(vec![2, 4])
        );
    }

    #[test]
    fn another_windows_shake_minimizes_afresh() {
        let memo = Memo {
            window: 1,
            minimized: vec![2],
        };
        assert_eq!(
            toggle(Some(&memo), 3, |_| true, &[4]),
            Toggle::Minimize(vec![4])
        );
    }

    #[test]
    fn a_shake_after_everything_came_back_minimizes_again() {
        let memo = Memo {
            window: 1,
            minimized: vec![2],
        };
        assert_eq!(
            toggle(Some(&memo), 1, |_| false, &[2]),
            Toggle::Minimize(vec![2])
        );
        assert_eq!(toggle(Some(&memo), 1, |_| false, &[]), Toggle::Nothing);
    }
}
