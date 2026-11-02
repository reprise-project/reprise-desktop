//! Volume sliders as positions: `per_full` steps up to 100 %, and with `[sounds] allow_boost`
//! on, the same steps on to [`BOOST_TOP`].

/// The loudest a playback volume goes with boost on, linear as [`crate::Device::volume`].
pub const BOOST_TOP: f64 = 1.5;

/// The loudest a playback volume goes: 1.0, or [`BOOST_TOP`] with boost on.
pub fn top(boost: bool) -> f64 {
    if boost { BOOST_TOP } else { 1.0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scale {
    /// Positions from 0 to 100 %.
    pub per_full: usize,
    pub boost: bool,
}

impl Scale {
    pub fn new(per_full: usize, boost: bool) -> Scale {
        Scale {
            per_full: per_full.max(1),
            boost,
        }
    }

    /// The last position's index: `per_full`, or as far again as [`BOOST_TOP`] reaches.
    pub fn last(self) -> usize {
        (self.per_full as f64 * top(self.boost)).round() as usize
    }

    pub fn count(self) -> usize {
        self.last() + 1
    }

    /// The position of 100 % while the slider runs past it.
    pub fn full_at(self) -> Option<usize> {
        self.boost.then_some(self.per_full)
    }

    /// `v`'s nearest position, a louder one at the top.
    pub fn pos(self, v: f64) -> usize {
        let last = self.last() as f64;
        (v * self.per_full as f64).round().clamp(0.0, last) as usize
    }

    pub fn volume(self, pos: usize) -> f64 {
        pos.min(self.last()) as f64 / self.per_full as f64
    }

    /// `pos` in whole percent.
    pub fn percent(self, pos: usize) -> usize {
        (pos.min(self.last()) * 100 + self.per_full / 2) / self.per_full
    }
}

/// `v` stepped by `delta` and kept within 0 and [`top`].
pub fn stepped(v: f64, delta: f64, boost: bool) -> f64 {
    (v + delta).clamp(0.0, top(boost))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boosted_scale_runs_on_past_100_to_150() {
        let plain = Scale::new(20, false);
        assert_eq!((plain.count(), plain.full_at()), (21, None));
        let boost = Scale::new(20, true);
        assert_eq!((boost.count(), boost.full_at()), (31, Some(20)));
        assert_eq!(Scale::new(60, true).count(), 91);
    }

    #[test]
    fn positions_map_to_volumes_and_back() {
        let s = Scale::new(20, true);
        assert_eq!(s.pos(1.0), 20);
        assert_eq!(s.pos(1.25), 25);
        assert_eq!(s.pos(2.0), 30, "louder than the top sits at the top");
        assert_eq!(s.volume(24), 1.2);
        assert_eq!(s.percent(30), 150);
        let plain = Scale::new(20, false);
        assert_eq!(plain.pos(1.3), 20, "without boost 130 % shows at the top");
        assert_eq!(plain.volume(30), 1.0);
        assert_eq!(Scale::new(60, false).percent(31), 52);
    }

    #[test]
    fn a_step_stops_at_100_or_150() {
        assert_eq!(stepped(0.98, 0.05, false), 1.0);
        assert_eq!(stepped(1.0, 0.05, true), 1.05);
        assert_eq!(stepped(1.48, 0.05, true), 1.5);
        assert_eq!(stepped(0.02, -0.05, true), 0.0);
    }
}
