//! What a press on a scrollbar does, the one model every host scrolls
//! with: an arrow steps a line, the track a page, and the thumb is
//! grabbed. A grabbed thumb follows the pointer along its bar until the
//! release, wherever the pointer goes, on the bar or off it.
//!
//! The geometry is `classic::widgets`' (`scrollbar_parts`, `bar_hit`,
//! `scrollbar_pos_at`), which every look lays its bars out with; what
//! lights under the pointer is [`crate::pointer::Pointer`]'s. A control
//! keeps a [`ThumbDrag`] next to the position it scrolls, or hands one
//! to its host, which feeds it the press, every motion and the release.

use crate::classic::widgets::{BarHit, ScrollParts, scrollbar_pos_at};
use crate::pointer::Axis;

/// What a press on a part of a scrollbar asks of the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    /// An arrow: one line back (-1) or forward (1).
    Lines(i64),
    /// The track before or after the thumb: one page that way.
    Pages(i64),
    /// The thumb: nothing moves until it is dragged.
    Grab,
    /// The bare track of a bar with no thumb.
    Nothing,
}

impl Press {
    /// The press on `hit`.
    pub fn of(hit: BarHit) -> Press {
        match hit {
            BarHit::Back => Press::Lines(-1),
            BarHit::Forward => Press::Lines(1),
            BarHit::PageBack => Press::Pages(-1),
            BarHit::PageForward => Press::Pages(1),
            BarHit::Thumb(_) => Press::Grab,
            BarHit::Track => Press::Nothing,
        }
    }

    /// How far the press scrolls, with a line and a page `line` and
    /// `page` long in the view's units (rows, px); 0 for the thumb.
    pub fn by(self, line: i64, page: i64) -> i64 {
        match self {
            Press::Lines(n) => n * line,
            Press::Pages(n) => n * page,
            Press::Grab | Press::Nothing => 0,
        }
    }
}

/// `pos` held to the range of a bar `visible` long over `total`: 0 to
/// `total - visible`, 0 when it all fits. A view keeps its offset while
/// its content narrows or its box grows; read through this, it shows the
/// content's end there rather than blank past it.
pub fn held(pos: i32, visible: i32, total: i32) -> i32 {
    pos.clamp(0, (total - visible).max(0))
}

/// The thumb the pointer holds, if any: which bar, and how far from the
/// thumb's leading edge it was grabbed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThumbDrag {
    held: Option<(Axis, i32)>,
}

impl ThumbDrag {
    /// A press on part `hit` of the `axis` bar: on the thumb it grabs it,
    /// anywhere else it lets go of whatever was held.
    pub fn press(&mut self, axis: Axis, hit: BarHit) -> Press {
        self.held = match hit {
            BarHit::Thumb(grab) => Some((axis, grab)),
            _ => None,
        };
        Press::of(hit)
    }

    /// The release lets go. True when a thumb was held.
    pub fn release(&mut self) -> bool {
        self.held.take().is_some()
    }

    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    /// The grab offset of the `axis` bar's thumb while it is held: the
    /// pointer's distance from the thumb's leading edge at the press,
    /// which a control's `top_at` takes.
    pub fn grab(&self, axis: Axis) -> Option<i32> {
        self.held.filter(|h| h.0 == axis).map(|h| h.1)
    }

    /// The position the held thumb of the `axis` bar, laid out as
    /// `parts` over `visible` of `total`, scrolls to with the pointer at
    /// (`x`, `y`). Only the coordinate along the bar counts, so a pointer
    /// dragged off the bar's side still steers it, and one past an end
    /// holds it there. `None` unless that bar's thumb is held.
    pub fn pos(
        &self,
        axis: Axis,
        parts: &ScrollParts,
        (x, y): (i32, i32),
        visible: usize,
        total: usize,
    ) -> Option<usize> {
        let grab = self.grab(axis)?;
        let horizontal = axis == Axis::Horizontal;
        let at = if horizontal { x } else { y };
        Some(scrollbar_pos_at(
            parts,
            at - grab,
            visible,
            total,
            horizontal,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::widgets::{bar_hit, scrollbar_parts};
    use crate::raster::Rect;

    /// A 16x109 bar over 20 rows with 6 in view: the thumb is 23 px of
    /// the 77 px track between the arrows.
    fn bar(top: usize) -> ScrollParts {
        scrollbar_parts(Rect::new(0, 0, 16, 109), 1, top, 6, 20, false, 8)
    }

    #[test]
    fn a_grabbed_thumb_follows_the_pointer_along_the_bar() {
        let p = bar(0);
        let thumb = p.thumb.unwrap();
        let mut d = ThumbDrag::default();
        let hit = bar_hit(&p, 8, thumb.y + 5, false);
        assert_eq!(hit, BarHit::Thumb(5));
        assert_eq!(d.press(Axis::Vertical, hit), Press::Grab);
        assert_eq!(d.pos(Axis::Vertical, &p, (8, thumb.y + 5), 6, 20), Some(0));
        // Half the 54 px span down is half the 14 rows past the first.
        assert_eq!(
            d.pos(Axis::Vertical, &p, (8, thumb.y + 5 + 27), 6, 20),
            Some(7)
        );
        assert_eq!(
            d.pos(Axis::Vertical, &p, (8, thumb.y + 5 + 54), 6, 20),
            Some(14)
        );
        assert_eq!(d.grab(Axis::Vertical), Some(5));
        assert_eq!(d.grab(Axis::Horizontal), None, "only the bar it grabbed");
        assert_eq!(d.pos(Axis::Horizontal, &p, (8, 60), 6, 20), None);
    }

    #[test]
    fn the_drag_carries_on_off_the_bar_until_the_release() {
        let p = bar(0);
        let mut d = ThumbDrag::default();
        d.press(Axis::Vertical, BarHit::Thumb(5));
        // Far to the side the pointer still steers by its y; past either
        // end it holds the thumb at that end.
        assert_eq!(
            d.pos(Axis::Vertical, &p, (300, 16 + 5 + 27), 6, 20),
            Some(7)
        );
        assert_eq!(d.pos(Axis::Vertical, &p, (-40, 900), 6, 20), Some(14));
        assert_eq!(d.pos(Axis::Vertical, &p, (-40, -900), 6, 20), Some(0));
        assert!(d.holding());
        assert!(d.release(), "a held thumb lets go");
        assert_eq!(d.pos(Axis::Vertical, &p, (8, 60), 6, 20), None);
        assert!(!d.release(), "nothing left to let go");
    }

    #[test]
    fn the_track_pages_and_the_arrows_step_a_line() {
        let p = bar(7);
        let thumb = p.thumb.unwrap();
        let mut d = ThumbDrag::default();
        let before = bar_hit(&p, 8, thumb.y - 2, false);
        let after = bar_hit(&p, 8, thumb.bottom() + 2, false);
        assert_eq!(d.press(Axis::Vertical, before), Press::Pages(-1));
        assert_eq!(d.press(Axis::Vertical, after), Press::Pages(1));
        assert_eq!(Press::Pages(1).by(1, 6), 6, "a page is the rows in view");
        assert_eq!(
            d.press(Axis::Vertical, bar_hit(&p, 8, 3, false)),
            Press::Lines(-1)
        );
        assert_eq!(Press::Lines(-1).by(1, 6), -1);
        assert_eq!(Press::Grab.by(1, 6), 0, "the thumb moves nothing");
        assert!(!d.holding(), "no press but the thumb's grabs");
    }

    #[test]
    fn a_press_off_the_thumb_lets_go_of_the_held_one() {
        let mut d = ThumbDrag::default();
        d.press(Axis::Horizontal, BarHit::Thumb(3));
        assert_eq!(d.grab(Axis::Horizontal), Some(3));
        d.press(Axis::Vertical, BarHit::PageForward);
        assert!(!d.holding());
    }
}
