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
//! A held arrow repeats on the host's ticks ([`ArrowRepeat`]). A page
//! that shows through a shorter view scrolls under a [`PageScroll`].

use std::time::{Duration, Instant};

use crate::classic::widgets::{BarHit, LINE_H, ScrollParts, scrollbar_parts, scrollbar_pos_at};
use crate::pointer::{Axis, Interaction, Lit, Pointer, ScrollHit};
use crate::raster::Rect;

/// How long an arrow is held before it steps again, from the first tick
/// after the press (a host ticks every 50 ms or so).
pub const REPEAT_AFTER: Duration = Duration::from_millis(350);
pub const REPEAT_EVERY: Duration = Duration::from_millis(50);

/// A held arrow's repeat: [`REPEAT_AFTER`] from the first tick after the press, then every
/// [`REPEAT_EVERY`] until the release.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArrowRepeat {
    /// The next step: `Some(None)` from the press until the first tick sets its time.
    next: Option<Option<Instant>>,
}

impl ArrowRepeat {
    /// A press on part `hit`: an arrow starts the repeat, any other part stops it.
    pub fn press(&mut self, hit: BarHit) {
        self.next = matches!(hit, BarHit::Back | BarHit::Forward).then_some(None);
    }

    pub fn release(&mut self) {
        self.next = None;
    }

    pub fn held(&self) -> bool {
        self.next.is_some()
    }

    /// A host tick at `now`: true when the held arrow steps now.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.next {
            None => false,
            Some(None) => {
                self.next = Some(Some(now + REPEAT_AFTER));
                false
            }
            Some(Some(at)) if now < at => false,
            Some(Some(_)) => {
                self.next = Some(Some(now + REPEAT_EVERY));
                true
            }
        }
    }
}

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

/// The parts of horizontal bar `hb` showing `visible` of `total` px from offset `left`.
pub(crate) fn hparts(hb: Rect, left: i32, visible: i32, total: i32, thumb_min: i32) -> ScrollParts {
    let (visible, total) = (visible.max(0) as usize, total.max(0) as usize);
    scrollbar_parts(hb, 1, left as usize, visible, total, true, thumb_min)
}

/// The offset a thumb of horizontal bar `p`, grabbed `grab` px right of its left edge, scrolls to
/// with the pointer at `x`.
pub(crate) fn left_at(p: &ScrollParts, x: i32, grab: i32, visible: i32, total: i32) -> i32 {
    let (visible, total) = (visible.max(0) as usize, total.max(0) as usize);
    scrollbar_pos_at(p, x - grab, visible, total, true) as i32
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

/// A page taller (or, [`PageScroll::across`], wider) than the view it shows through, scrolled in
/// its 100% px under a bar down the view's side (along its bottom): how much it is cut and
/// scrolled, the held thumb, the lit part and a held arrow. The owner holds `top` to its range
/// and moves its content by it. An arrow steps [`LINE_H`], the track a view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageScroll {
    /// How much taller (wider) than its view the page was built.
    pub short: i32,
    /// How far the page is scrolled up (left): 0 to the owner's range.
    pub top: i32,
    horizontal: bool,
    drag: ThumbDrag,
    bars: Pointer<ScrollHit>,
    repeat: ArrowRepeat,
}

impl PageScroll {
    /// A page scrolled sideways under a bar along the view's bottom.
    pub fn across() -> PageScroll {
        PageScroll {
            horizontal: true,
            ..PageScroll::default()
        }
    }

    fn axis(&self) -> Axis {
        if self.horizontal {
            Axis::Horizontal
        } else {
            Axis::Vertical
        }
    }

    /// A press on part `hit` of the bar over a view `visible` long: the top it asks for. A held
    /// arrow repeats on [`PageScroll::tick`]; a thumb is grabbed.
    pub fn press(&mut self, hit: BarHit, visible: i32) -> i32 {
        self.bars.press(hit.part(), matches!(hit, BarHit::Thumb(_)));
        self.repeat.press(hit);
        let by = self.drag.press(self.axis(), hit);
        self.top + by.by(LINE_H as i64, visible as i64) as i32
    }

    /// The top the held thumb asks for with the pointer at `at` on a bar laid out as `parts`
    /// over `visible` of `total`; `None` unless the thumb is held.
    pub fn drag(
        &self,
        parts: &ScrollParts,
        at: (i32, i32),
        visible: i32,
        total: i32,
    ) -> Option<i32> {
        let pos = self
            .drag
            .pos(self.axis(), parts, at, visible as usize, total as usize)?;
        Some(pos as i32)
    }

    /// The pointer moved onto `hit` of the bar, `None` off it. True when the bar's look changed.
    pub fn motion(&mut self, hit: Option<BarHit>) -> bool {
        self.bars.motion(hit.and_then(BarHit::part))
    }

    pub fn leave(&mut self) -> bool {
        self.bars.leave()
    }

    /// The pointer let go: true when a thumb was held or the bar's look changed.
    pub fn release(&mut self) -> bool {
        let held = self.drag.release();
        self.repeat.release();
        let lit = self.bars.lit();
        self.bars.release();
        held || lit != self.bars.lit()
    }

    /// Whether the thumb is held.
    pub fn holding(&self) -> bool {
        self.drag.holding()
    }

    /// Whether an arrow is held: the host ticks [`PageScroll::tick`] until the release.
    pub fn ticking(&self) -> bool {
        self.repeat.held()
    }

    /// A host tick at `now`: the top a held arrow asks for once it repeats, while the pointer is
    /// on it.
    pub fn tick(&mut self, now: Instant) -> Option<i32> {
        if !self.repeat.due(now) {
            return None;
        }
        match self.bars.lit() {
            Some((ScrollHit::Back, Interaction::Pressed)) => Some(self.top - LINE_H),
            Some((ScrollHit::Forward, Interaction::Pressed)) => Some(self.top + LINE_H),
            _ => None,
        }
    }

    pub fn lit(&self) -> Lit<ScrollHit> {
        self.bars.lit()
    }

    /// The bar's parts in `bar`, over a view `visible` of a page `total` long.
    pub fn parts(&self, bar: Rect, visible: i32, total: i32, thumb_min: i32) -> ScrollParts {
        let (top, visible, total) = (self.top as usize, visible as usize, total as usize);
        scrollbar_parts(bar, 1, top, visible, total, self.horizontal, thumb_min)
    }

    /// How far the page scrolls so the span `r` (top, bottom) shows whole in the view from
    /// `view.0` to `view.1`, as little as it takes; its top shows when it is taller than the view.
    pub fn reveal(view: (i32, i32), r: (i32, i32)) -> i32 {
        if r.0 < view.0 {
            r.0 - view.0
        } else if r.1 > view.1 {
            (r.1 - view.1).min(r.0 - view.0)
        } else {
            0
        }
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
    fn reveal_scrolls_as_little_as_shows_a_span_whole() {
        assert_eq!(PageScroll::reveal((100, 300), (120, 140)), 0, "in view");
        assert_eq!(PageScroll::reveal((100, 300), (80, 120)), -20, "above");
        assert_eq!(PageScroll::reveal((100, 300), (280, 330)), 30, "below");
        assert_eq!(
            PageScroll::reveal((100, 300), (150, 500)),
            50,
            "taller: its top"
        );
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
