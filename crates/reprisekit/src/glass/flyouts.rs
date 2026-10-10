//! Glass's flyouts: the panel a tray icon or the clock opens is a small
//! window, the window's 8 px border round white contents, casting the
//! active window's shadow (13 px left, 12 up, 18 right and 19 down). With
//! no blurred backdrop under it the border lies over E9F5FF, the colour
//! the frame takes with transparency off.
//!
//! The month grid's parts: today's number stands in a 24x15 box, a 0066CC
//! line rounded at the corners round a fill shading from ECF5FF to
//! D0E5FF inside a lighter line. The arrows that step the month are bare
//! 4x7 triangles, near black shading darker down the glyph, 0066CC under
//! the pointer.

use super::art;
use super::panel::{self, Fade, LightBox};
use super::shadow;
use crate::raster::{Canvas, Insets, Rect};
use crate::style::{FlyoutLook, Layout, Part, PartImage, State};
use crate::theme::Color;

pub const SHADOW: Insets = shadow::ACTIVE_DESIGN.margins;

/// The flyout look, the same light or dark: the glass keeps its colour.
pub fn look() -> FlyoutLook {
    let [r, g, b] = art::OPAQUE_UNDER[0];
    FlyoutLook {
        shadow: SHADOW,
        opaque_under: Color::rgb(r, g, b),
    }
}

/// The frame the shadow part is cut round: wide enough that no two of the
/// shadow's corners meet, so its middle row and column are the edges'.
const FRAME: i32 = 40;

/// The active window's shadow round a `FRAME` square, painted at `d`, as
/// a nine-grid: the corners fixed, the edges stretched, the middle clear.
pub fn shadow_part(d: i32) -> PartImage {
    let m = SHADOW;
    let frame = Rect::new(0, 0, FRAME, FRAME);
    let (w, h) = (m.left + FRAME + m.right, m.top + FRAME + m.bottom);
    let mut c = Canvas::new((w * d) as u32, (h * d) as u32);
    for (i, p) in shadow::place(frame, true, false).iter().enumerate() {
        let piece = shadow::piece(true, i, d);
        let (pw, ph) = (piece.width as i32, piece.height as i32);
        let dest = p.dest.scaled(d);
        let src = p.src.scaled(d);
        for y in 0..dest.height {
            for x in 0..dest.width {
                // An edge piece is a design pixel long: it repeats along.
                let sx = if src.width < dest.width {
                    x % pw
                } else {
                    src.x + x
                };
                let sy = if src.height < dest.height {
                    y % ph
                } else {
                    src.y + y
                };
                c.put_raw(
                    dest.x + m.left * d + x,
                    dest.y + m.top * d + y,
                    piece.get(sx, sy),
                );
            }
        }
    }
    let corners = &shadow::ACTIVE.pieces;
    let mut img = PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(
            corners[shadow::TOP_LEFT].w,
            corners[shadow::TOP_LEFT].h,
            corners[shadow::BOTTOM_RIGHT].w,
            corners[shadow::BOTTOM_RIGHT].h,
        ))
        .at_scale(d);
    img.border_only = true;
    img
}

const fn ink(v: u32, a: u8) -> (Color, u8) {
    (Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8), a)
}

const TODAY: LightBox = LightBox {
    outline: ink(0x0066CC, 255),
    line: Some((ink(0xF2F8FF, 255), ink(0xDFEEFF, 255))),
    fill: (ink(0xECF5FF, 255), ink(0xD0E5FF, 255)),
    radius: 2.0,
    fade: Fade::Ring,
};

pub const TODAY_SIZE: (i32, i32) = (24, 15);

/// The step-back arrow's rows, one shade each, right-aligned in its 4
/// columns: 1, 2, 3, 4, 3, 2 and 1 pixels long.
const ARROW_SHADES: [u32; 7] = [
    0x3B3B3B, 0x3A3A3A, 0x383838, 0x313131, 0x262626, 0x191919, 0x0B0B0B,
];
const ARROW_HOT: u32 = 0x0066CC;

fn arrows(next: bool) -> Canvas {
    let mut c = Canvas::new(4, 14);
    for (frame, hot) in [false, true].into_iter().enumerate() {
        for (y, &shade) in ARROW_SHADES.iter().enumerate() {
            let len = 4 - (3 - y as i32).abs();
            let rgb = if hot { ARROW_HOT } else { shade };
            for k in 0..len {
                let x = if next { k } else { 3 - k };
                c.put_raw(x, frame as i32 * 7 + y as i32, 0xFF00_0000 | rgb);
            }
        }
    }
    c
}

pub fn parts(d: i32) -> [(Part, PartImage); 3] {
    let (w, h) = TODAY_SIZE;
    let today = PartImage::new(
        panel::light_box(&TODAY, w, h, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(3, 3, 3, 3))
    .at_scale(d);
    let arrow = |next| {
        let mut img = PartImage::new(arrows(next), &[State::Normal, State::Hot], Layout::Vertical);
        img.true_size = true;
        img
    };
    [
        (Part::CalendarToday, today),
        (Part::CalendarPrev, arrow(false)),
        (Part::CalendarNext, arrow(true)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arrows_are_4_by_7_triangles_hot_in_link_blue() {
        let a = arrows(false);
        // The back arrow's tip on its middle row, its foot on the right.
        assert_eq!(a.get(0, 3), 0xFF31_3131);
        assert_eq!(a.get(0, 2) >> 24, 0, "a row up, the tip is gone");
        assert_eq!(a.get(3, 0), 0xFF3B_3B3B, "the top row is one pixel");
        assert_eq!(a.get(3, 6), 0xFF0B_0B0B, "the darkest at the foot");
        assert_eq!(a.get(0, 10), 0xFF00_66CC, "hot");
        let n = arrows(true);
        assert_eq!(n.get(3, 3), 0xFF31_3131, "the on arrow points right");
        assert_eq!(n.get(0, 0), 0xFF3B_3B3B);
    }

    #[test]
    fn today_is_a_rounded_link_blue_line_round_a_pale_blue_fill() {
        let c = panel::light_box(&TODAY, 24, 15, 1);
        assert_eq!(c.get(12, 0), 0xFF00_66CC, "the line along the top");
        assert_eq!(c.get(0, 7), 0xFF00_66CC, "down the left");
        assert!(c.get(0, 0) >> 24 < 0xFF, "the corner is rounded");
        assert_eq!(c.get(12, 2), 0xFFEC_F5FF, "the fill's top");
        assert_eq!(c.get(12, 12), 0xFFD0_E5FF, "its foot");
    }

    #[test]
    fn the_shadow_part_is_clear_over_the_frame_and_a_window_shadow_round_it() {
        let p = shadow_part(1);
        let (w, h) = (p.atlas.width as i32, p.atlas.height as i32);
        assert_eq!((w, h), (13 + 40 + 18, 12 + 40 + 19));
        assert_eq!(p.atlas.get(13 + 20, 12 + 20), 0, "clear over the frame");
        let left = shadow::ACTIVE.pieces[shadow::LEFT].get(12, 0);
        assert_eq!(
            p.atlas.get(12, 12 + 20) >> 24,
            u32::from(left),
            "the left edge"
        );
        assert!(
            p.atlas.get(13 + 20, 12 + 40) >> 24 > 0,
            "shade under the frame"
        );
    }
}
