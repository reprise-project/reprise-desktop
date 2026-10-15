//! The snap bar's model: the layouts row that hangs from the top while a title bar is dragged.
//! Pure: the host passes the clock in, draws the row at [`SnapBar::rect`] cut to the usable area,
//! and asks for output frames until [`SnapBar::settled`].

use std::time::Instant;

use crate::anim::{Ease, Tween};
use crate::raster::Rect;

/// How much of a collapsed bar shows below the usable area's top.
pub const PEEK: i32 = 11;
/// An expanded bar's top below the usable area's top.
pub const DROP: i32 = 24;
/// Round an expanded bar: the pointer inside keeps it down.
pub const MARGIN: i32 = 12;
/// Sliding to collapsed or hidden.
pub const SLIDE_MS: u32 = 150;
/// Sliding down to expanded.
pub const EXPAND_MS: u32 = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Hidden,
    Collapsed,
    Expanded,
}

/// Whether a drag with the pointer on row `y` shows the bar over usable area `work`: anywhere above
/// the end of its top third.
pub fn wanted(work: Rect, y: i32) -> bool {
    y < work.y + work.height / 3
}

/// Where a bar `size` big stands at `stage` over `work`, centred across.
pub fn rect(work: Rect, size: (i32, i32), stage: Stage) -> Rect {
    let (w, h) = size;
    let y = match stage {
        Stage::Hidden => work.y - h,
        Stage::Collapsed => work.y - h + PEEK,
        Stage::Expanded => work.y + DROP,
    };
    Rect::new(work.x + (work.width - w) / 2, y, w, h)
}

/// One output's bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapBar {
    stage: Stage,
    y: Tween,
    work: Rect,
    size: (i32, i32),
}

impl SnapBar {
    /// Hidden above `work`.
    pub fn new(work: Rect, size: (i32, i32), now: Instant) -> SnapBar {
        let y = f64::from(rect(work, size, Stage::Hidden).y);
        SnapBar {
            stage: Stage::Hidden,
            y: Tween::new(y, y, now, 0, Ease::OutCubic),
            work,
            size,
        }
    }

    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The pointer of the drag is at (`x`, `y`): returns whether the bar changed stage.
    pub fn motion(&mut self, x: i32, y: i32, now: Instant) -> bool {
        let next = if !wanted(self.work, y) {
            Stage::Hidden
        } else if self.over(self.stage == Stage::Expanded, x, y) {
            Stage::Expanded
        } else {
            Stage::Collapsed
        };
        self.go(next, now)
    }

    /// Whether the pointer at (`x`, `y`) is on the bar's area at its stage, where the bar wins over
    /// what lies beneath (a caption a drop would join): expanded, the panel and [`MARGIN`] round
    /// it, up through the strip above; collapsed, the [`PEEK`] px strip across the row.
    pub fn holds(&self, x: i32, y: i32) -> bool {
        self.stage != Stage::Hidden && self.over(self.stage == Stage::Expanded, x, y)
    }

    fn over(&self, expanded: bool, x: i32, y: i32) -> bool {
        if expanded {
            let r = rect(self.work, self.size, Stage::Expanded);
            x >= r.x - MARGIN && x < r.right() + MARGIN && y < r.bottom() + MARGIN
        } else {
            let r = rect(self.work, self.size, Stage::Collapsed);
            x >= r.x && x < r.right() && y < self.work.y + PEEK
        }
    }

    /// The drag ended: slide out.
    pub fn end(&mut self, now: Instant) -> bool {
        self.go(Stage::Hidden, now)
    }

    fn go(&mut self, next: Stage, now: Instant) -> bool {
        if next == self.stage {
            return false;
        }
        let ms = if next == Stage::Expanded {
            EXPAND_MS
        } else {
            SLIDE_MS
        };
        self.stage = next;
        let to = f64::from(rect(self.work, self.size, next).y);
        self.y.retarget(to, now, ms);
        true
    }

    /// Where it is drawn at `now`, before the cut to the usable area.
    pub fn rect(&self, now: Instant) -> Rect {
        let r = rect(self.work, self.size, Stage::Hidden);
        Rect::new(r.x, self.y.value(now).round() as i32, r.width, r.height)
    }

    /// Where a release drops nothing at `now`: the bar as drawn, cut to the usable area; expanded,
    /// also the strip between the area's top and the panel, across the panel's width, so the top
    /// edge does not maximize from there.
    pub fn no_drop(&self, now: Instant) -> Rect {
        let mut r = self.rect(now);
        if self.stage == Stage::Expanded && r.y > self.work.y {
            r = Rect::new(r.x, self.work.y, r.width, r.bottom() - self.work.y);
        }
        r.intersect(&self.work)
    }

    pub fn settled(&self, now: Instant) -> bool {
        self.y.settled(now)
    }

    /// Whether its tiles take the pointer at `now`: only expanded and done sliding, so a tile never
    /// lights under a pointer the moving row passes.
    pub fn live(&self, now: Instant) -> bool {
        self.stage == Stage::Expanded && self.settled(now)
    }

    /// Hidden and all the way out.
    pub fn gone(&self, now: Instant) -> bool {
        self.stage == Stage::Hidden && self.settled(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const WORK: Rect = Rect::new(0, 0, 1920, 1032);
    const SIZE: (i32, i32) = (894, 90);

    #[test]
    fn the_bar_wants_the_top_third() {
        assert!(wanted(WORK, 0));
        assert!(wanted(WORK, 343));
        assert!(!wanted(WORK, 344));
        assert!(wanted(WORK, -5), "a pointer over a bar above the area");
    }

    #[test]
    fn collapsed_shows_eleven_px_and_expanded_hangs_24_down() {
        assert_eq!(
            rect(WORK, SIZE, Stage::Collapsed),
            Rect::new(513, -79, 894, 90)
        );
        assert_eq!(
            rect(WORK, SIZE, Stage::Expanded),
            Rect::new(513, 24, 894, 90)
        );
        assert_eq!(rect(WORK, SIZE, Stage::Hidden).bottom(), 0);
    }

    #[test]
    fn the_strip_expands_and_leaving_the_panel_collapses() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        assert!(b.motion(900, 300, t0));
        assert_eq!(b.stage(), Stage::Collapsed);
        assert!(!b.motion(900, 12, t0), "below the strip");
        assert!(b.motion(900, 6, t0));
        assert_eq!(b.stage(), Stage::Expanded);
        assert!(!b.motion(900, 125, t0), "inside the 12 px margin");
        assert!(!b.motion(900, 3, t0), "back up in the strip");
        assert!(b.motion(900, 140, t0));
        assert_eq!(b.stage(), Stage::Collapsed);
        assert!(b.motion(900, 600, t0));
        assert_eq!(b.stage(), Stage::Hidden);
    }

    #[test]
    fn expanded_the_strip_above_the_panel_drops_nothing() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        b.motion(900, 300, t0);
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(b.no_drop(t1), Rect::new(513, 0, 894, 11), "the peek");
        b.motion(900, 3, t1);
        let t2 = t1 + Duration::from_millis(500);
        assert_eq!(
            b.no_drop(t2),
            Rect::new(513, 0, 894, 114),
            "from the area's top to the panel's bottom"
        );
        b.end(t2);
        let t3 = t2 + Duration::from_millis(500);
        assert!(b.no_drop(t3).is_empty(), "slid out");
    }

    #[test]
    fn the_bar_holds_the_strip_then_the_panel_and_its_margin() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        assert!(!b.holds(900, 5), "hidden");
        b.motion(900, 300, t0);
        assert!(b.holds(900, 10) && b.holds(513, 0) && b.holds(1406, 0));
        assert!(!b.holds(900, 11), "below the peek");
        assert!(!b.holds(512, 5) && !b.holds(1407, 5), "beside the row");
        b.motion(900, 5, t0);
        assert_eq!(b.stage(), Stage::Expanded);
        assert!(b.holds(900, 5), "the strip above the panel");
        assert!(b.holds(501, 60) && b.holds(1418, 60) && b.holds(900, 125));
        assert!(!b.holds(500, 60) && !b.holds(1419, 60) && !b.holds(900, 126));
        b.end(t0);
        assert!(!b.holds(900, 5), "sliding out");
    }

    #[test]
    fn the_tiles_take_the_pointer_once_the_expand_settles() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        b.motion(900, 300, t0);
        assert!(!b.live(t0 + Duration::from_millis(500)), "collapsed");
        let t1 = t0 + Duration::from_millis(500);
        b.motion(900, 3, t1);
        assert!(!b.live(t1), "starting down");
        assert!(!b.live(t1 + Duration::from_millis(199)), "still sliding");
        assert!(b.live(t1 + Duration::from_millis(200)));
        b.end(t1 + Duration::from_millis(200));
        assert!(!b.live(t1 + Duration::from_millis(200)), "sliding out");
    }

    #[test]
    fn a_first_motion_in_the_strip_drops_straight_down_over_200_ms() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        assert!(b.motion(900, 5, t0));
        assert_eq!(b.stage(), Stage::Expanded);
        assert_eq!(b.rect(t0).y, -90);
        assert!(!b.settled(t0 + Duration::from_millis(199)));
        assert_eq!(b.rect(t0 + Duration::from_millis(200)).y, 24);
        assert!(b.live(t0 + Duration::from_millis(200)));
    }

    #[test]
    fn expanding_mid_slide_starts_from_where_the_bar_is() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        b.motion(900, 300, t0);
        let t1 = t0 + Duration::from_millis(75);
        let mid = b.rect(t1).y;
        assert_eq!(mid, -80);
        b.motion(900, 5, t1);
        assert_eq!(b.rect(t1).y, mid, "no jump");
        assert!(b.rect(t1 + Duration::from_millis(100)).y > mid);
        assert!(!b.settled(t1 + Duration::from_millis(199)));
        assert_eq!(b.rect(t1 + Duration::from_millis(200)).y, 24);
    }

    #[test]
    fn the_slide_eases_out_over_150_ms() {
        let t0 = Instant::now();
        let mut b = SnapBar::new(WORK, SIZE, t0);
        b.motion(900, 300, t0);
        let at = |ms| b.rect(t0 + Duration::from_millis(ms)).y;
        assert_eq!(at(0), -90);
        assert_eq!(at(75), -80, "-90 + 11 * (1 - 0.5^3), rounded");
        assert_eq!(at(150), -79);
        assert!(b.settled(t0 + Duration::from_millis(150)));
        b.end(t0 + Duration::from_millis(150));
        assert!(b.gone(t0 + Duration::from_millis(300)));
    }
}
