//! The display switcher's geometry and its tile pictures, shared by every look's painter. The
//! model and Classic's and Blue's painter are [`crate::controls::projector`].

use crate::projection::Projection;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

/// A tile, the gap between tiles and the padding round them, logical px.
pub const TILE: (i32, i32) = (96, 78);
pub const GAP: i32 = 8;
pub const PAD: i32 = 12;
/// The sheet's corner table (`backdrop::corner_radius`), the snap layouts panel's.
pub const PANEL_CORNER: &[i32] = crate::snaplayouts::PANEL_CORNER;

/// From a tile's top-left.
const PICTURE: Rect = Rect::new(8, 8, 80, 35);
const CAPTION: Rect = Rect::new(4, 48, 88, 22);
/// From the picture's top-left.
const SCREEN: (i32, i32) = (36, 30);
const SCREEN_X: [i32; 2] = [2, 42];
const BAND: Rect = Rect::new(7, 18, 66, 6);
/// From a screen's top-left.
const NECK: Rect = Rect::new(16, 30, 4, 3);
const BASE: Rect = Rect::new(10, 33, 16, 2);
const DIGIT: Rect = Rect::new(1, 1, 34, 13);
const WINDOW: Rect = Rect::new(10, 16, 16, 11);
const TITLE_BAR: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectorLayout {
    pub width: i32,
    pub height: i32,
    pub tiles: [Rect; 4],
    pub scale: i32,
}

impl ProjectorLayout {
    /// A row of a tile per [`Projection::ALL`], logical px.
    pub fn compute() -> ProjectorLayout {
        let (w, h) = TILE;
        ProjectorLayout {
            width: 2 * PAD + 4 * w + 3 * GAP,
            height: 2 * PAD + h,
            tiles: std::array::from_fn(|i| Rect::new(PAD + i as i32 * (w + GAP), PAD, w, h)),
            scale: 1,
        }
    }

    pub fn scaled(&self, n: i32) -> ProjectorLayout {
        let n = n.max(1);
        ProjectorLayout {
            width: self.width * n,
            height: self.height * n,
            tiles: self.tiles.map(|r| r.scaled(n)),
            scale: self.scale * n,
        }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<usize> {
        self.tiles.iter().position(|t| t.contains(x, y))
    }

    pub fn picture(&self, i: usize) -> Rect {
        within(self.tiles[i], PICTURE, self.scale)
    }

    pub fn caption(&self, i: usize) -> Rect {
        within(self.tiles[i], CAPTION, self.scale)
    }
}

/// `r` (design px) placed from `outer`'s top-left at scale `s`.
fn within(outer: Rect, r: Rect, s: i32) -> Rect {
    let r = r.scaled(s);
    Rect::new(outer.x + r.x, outer.y + r.y, r.width, r.height)
}

/// The two screens of a picture at `at` (device px, scale `s`), display 1 first.
pub fn screens(at: Rect, s: i32) -> [Rect; 2] {
    SCREEN_X.map(|x| within(at, Rect::new(x, 0, SCREEN.0, SCREEN.1), s))
}

/// The colours a tile's picture is drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ink {
    /// The monitors' outlines and stands, and the digit on a dark screen.
    pub frame: Color,
    /// A lit screen; None leaves every screen dark.
    pub screen: Option<Color>,
    /// The digit on a lit screen, Extend's band and Duplicate's windows.
    pub mark: Color,
}

/// `choice`'s picture at `at` (device px, scale `s`): monitors 1 and 2, lit where `choice` shows
/// the desktop, their digits in `text` at `size_px`; Extend lays one band across both, Duplicate
/// the same window on each.
pub fn draw_picture(
    c: &mut Canvas,
    text: &TextRenderer,
    size_px: u32,
    at: Rect,
    choice: Projection,
    ink: &Ink,
    s: i32,
) {
    for (k, screen) in screens(at, s).into_iter().enumerate() {
        let lit = match choice {
            Projection::Only(i) => i == k,
            Projection::Extend | Projection::Duplicate => true,
        };
        let fill = ink.screen.filter(|_| lit);
        if let Some(fill) = fill {
            c.fill_rect(screen.inset(s), fill);
        }
        c.ring_t(screen, ink.frame, ink.frame, s);
        c.fill_rect(within(screen, NECK, s), ink.frame);
        c.fill_rect(within(screen, BASE, s), ink.frame);
        let digit = if fill.is_some() { ink.mark } else { ink.frame };
        let n = (k + 1).to_string();
        let r = within(screen, DIGIT, s);
        text.draw(c, r, &n, false, digit, Align::Center, size_px);
        if choice == Projection::Duplicate {
            let w = within(screen, WINDOW, s);
            c.ring_t(w, ink.mark, ink.mark, s);
            c.fill_rect(Rect::new(w.x, w.y, w.width, TITLE_BAR * s), ink.mark);
        }
    }
    if choice == Projection::Extend {
        c.fill_rect(within(at, BAND, s), ink.mark);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_tiles_lie_in_one_row() {
        let l = ProjectorLayout::compute();
        assert_eq!((l.width, l.height), (432, 102));
        assert_eq!(l.tiles[0], Rect::new(12, 12, 96, 78));
        assert_eq!(l.tiles[3], Rect::new(324, 12, 96, 78));
        assert_eq!(l.picture(1), Rect::new(124, 20, 80, 35));
        assert_eq!(l.caption(1), Rect::new(120, 60, 88, 22));
    }

    #[test]
    fn a_hit_finds_the_tile_and_misses_the_gaps() {
        let l = ProjectorLayout::compute();
        assert_eq!(l.hit(12, 12), Some(0));
        assert_eq!(l.hit(220, 50), Some(2));
        assert_eq!(l.hit(110, 50), None, "the gap between tiles");
        assert_eq!(l.hit(50, 5), None, "the padding");
        let l2 = l.scaled(2);
        assert_eq!(l2.hit(440, 100), Some(2));
        assert_eq!(l2.hit(220, 100), None);
    }

    #[test]
    fn scale_2_doubles_every_rect() {
        let (l, l2) = (
            ProjectorLayout::compute(),
            ProjectorLayout::compute().scaled(2),
        );
        assert_eq!((l2.width, l2.height, l2.scale), (864, 204, 2));
        for i in 0..4 {
            assert_eq!(l2.tiles[i], l.tiles[i].scaled(2));
            assert_eq!(l2.picture(i), l.picture(i).scaled(2));
            assert_eq!(l2.caption(i), l.caption(i).scaled(2));
        }
        assert_eq!(
            screens(l2.picture(0), 2),
            screens(l.picture(0), 1).map(|r| r.scaled(2))
        );
    }
}
