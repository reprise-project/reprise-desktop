use crate::config::HotCorner;
use crate::raster::Rect;

/// Logical px, along either axis, the pointer leaves the corner by before it
/// can fire again.
pub const REARM: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corner {
    armed: bool,
}

impl Default for Corner {
    fn default() -> Self {
        Corner { armed: true }
    }
}

impl Corner {
    /// The pointer is at `pos` in a layout whose box is `bounds`: true
    /// when this motion reaches `corner` while armed. With no corner set
    /// it never fires.
    pub fn motion(&mut self, corner: HotCorner, pos: (f64, f64), bounds: Rect) -> bool {
        let Some((cx, cy)) = point(corner, bounds) else {
            return false;
        };
        let (dx, dy) = ((pos.0 - cx).abs(), (pos.1 - cy).abs());
        if dx < 1.0 && dy < 1.0 {
            let fire = self.armed;
            self.armed = false;
            return fire;
        }
        if dx > REARM || dy > REARM {
            self.armed = true;
        }
        false
    }
}

/// The layout's outer corner, so a corner between two outputs never fires.
fn point(corner: HotCorner, b: Rect) -> Option<(f64, f64)> {
    let (l, t) = (f64::from(b.x), f64::from(b.y));
    let (r, btm) = (f64::from(b.right() - 1), f64::from(b.bottom() - 1));
    Some(match corner {
        HotCorner::None => return None,
        HotCorner::TopLeft => (l, t),
        HotCorner::TopRight => (r, t),
        HotCorner::BottomLeft => (l, btm),
        HotCorner::BottomRight => (r, btm),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn fires_once_in_the_corner_and_again_after_leaving_it() {
        let mut c = Corner::default();
        assert!(!c.motion(HotCorner::TopLeft, (5.0, 5.0), B));
        assert!(c.motion(HotCorner::TopLeft, (0.0, 0.3), B));
        assert!(!c.motion(HotCorner::TopLeft, (0.0, 0.0), B), "resting");
        assert!(
            !c.motion(HotCorner::TopLeft, (4.0, 4.0), B),
            "not far enough"
        );
        assert!(!c.motion(HotCorner::TopLeft, (0.0, 0.0), B));
        assert!(!c.motion(HotCorner::TopLeft, (20.0, 0.0), B), "rearms");
        assert!(c.motion(HotCorner::TopLeft, (0.0, 0.0), B));
    }

    #[test]
    fn each_corner_is_its_own_pixel_and_none_never_fires() {
        let at = [
            (HotCorner::TopRight, (1919.0, 0.0)),
            (HotCorner::BottomLeft, (0.0, 1079.0)),
            (HotCorner::BottomRight, (1919.5, 1079.5)),
        ];
        for (corner, pos) in at {
            let mut c = Corner::default();
            assert!(
                !c.motion(corner, (0.0, 0.0), B),
                "{corner:?} not at top-left"
            );
            assert!(c.motion(corner, pos, B), "{corner:?}");
        }
        let mut c = Corner::default();
        assert!(!c.motion(HotCorner::None, (0.0, 0.0), B));
    }

    #[test]
    fn the_corner_is_the_layout_s_outer_one() {
        let both = Rect::new(0, 0, 3840, 1080);
        let mut c = Corner::default();
        assert!(!c.motion(HotCorner::TopRight, (1919.0, 0.0), both));
        assert!(c.motion(HotCorner::TopRight, (3839.0, 0.0), both));
    }
}
