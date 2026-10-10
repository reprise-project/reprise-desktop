//! The caption-button glyphs of the classic frame (minimize, maximize, restore, close,
//! help ?), drawn as strokes in 16x14 button units.

use crate::raster::{Canvas, Rect};
use crate::theme::Color;

/// One caption glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Restore,
    Minimize,
    Maximize,
    Close,
    Help,
}

/// The strokes of one caption glyph, in 16x14 button units scaled by `s`,
/// with (x, y) the button's top-left. `ink` draws the strokes and `face`
/// fills the restore glyph's front box; the window menu passes its own row
/// colors so the glyph inverts with the highlight.
pub fn draw_glyph_strokes(
    c: &mut Canvas,
    x: i32,
    y: i32,
    s: i32,
    glyph: Glyph,
    ink: Color,
    face: Color,
) {
    let r = |dx: i32, dy: i32, w: i32, h: i32| Rect::new(x + dx * s, y + dy * s, w * s, h * s);
    match glyph {
        Glyph::Minimize => {
            c.fill_rect(r(4, 9, 6, 2), ink);
        }
        Glyph::Maximize => {
            // 9x9 box with a 2 px top edge
            c.fill_rect(r(3, 2, 9, 2), ink);
            c.fill_rect(r(3, 10, 9, 1), ink);
            c.fill_rect(r(3, 2, 1, 9), ink);
            c.fill_rect(r(11, 2, 1, 9), ink);
        }
        Glyph::Restore => {
            // back box (6x6) top-right, front box (6x6) bottom-left
            c.fill_rect(r(5, 2, 6, 2), ink);
            c.fill_rect(r(5, 7, 6, 1), ink);
            c.fill_rect(r(5, 2, 1, 3), ink);
            c.fill_rect(r(10, 2, 1, 6), ink);
            c.fill_rect(r(3, 5, 6, 6), face);
            c.fill_rect(r(3, 5, 6, 2), ink);
            c.fill_rect(r(3, 10, 6, 1), ink);
            c.fill_rect(r(3, 5, 1, 6), ink);
            c.fill_rect(r(8, 5, 1, 6), ink);
        }
        Glyph::Close => {
            // 8x7 X with 2 px wide strokes
            for i in 0..7 {
                c.fill_rect(r(4 + i, 3 + i, 2, 1), ink);
                c.fill_rect(r(4 + (6 - i), 3 + i, 2, 1), ink);
            }
        }
        Glyph::Help => {
            c.fill_rect(r(6, 2, 4, 1), ink);
            c.fill_rect(r(5, 3, 2, 2), ink);
            c.fill_rect(r(9, 3, 2, 2), ink);
            c.fill_rect(r(8, 5, 2, 1), ink);
            c.fill_rect(r(7, 6, 2, 2), ink);
            c.fill_rect(r(7, 9, 2, 2), ink);
        }
    }
}
