//! A document tab's parts in a styled look: a glass tab over the frame's glass
//! ([`Part::GlassTab`]), the wash of a lit close box or new-tab part, and the glyphs.

use crate::look::Paint;
use crate::look::styled::tab::label_rect;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::shape::{self, Glyph};
use crate::style::{Part, State, Style};
use crate::theme::Color;

/// How much of the ink a lit close box or new-tab part lays under its glyph, out of FF.
const WASH_HOT: u8 = 0x26;
const WASH_HELD: u8 = 0x4C;
const WASH_RADIUS: f64 = 2.0;

/// A glass tab over `r` (device px), its frame run down to `page`'s bottom when open so it meets
/// what lies under the strip; returns the rect its label is centred in.
pub fn draw_glass_tab(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    open: bool,
    hot: bool,
    page: Rect,
) -> Rect {
    let (state, frame) = match (open, hot) {
        (true, _) => (
            State::Checked,
            Rect::new(r.x, r.y, r.width, page.bottom() - r.y),
        ),
        (false, true) => (State::Hot, r),
        (false, false) => (State::Normal, r),
    };
    st.draw(c, frame, Part::GlassTab, state, p.s);
    label_rect(st, Part::GlassTab, r, open, p.s)
}

/// The wash under a close box or new-tab part `b` (device px) the pointer is on or holds: `ink`
/// laid over a box rounded by 2 px.
pub fn draw_wash(c: &mut Canvas, b: Rect, ptr: Interaction, ink: Color, s: i32) {
    let alpha = match ptr {
        Interaction::Idle => return,
        Interaction::Hot => WASH_HOT,
        Interaction::Pressed => WASH_HELD,
    };
    let k = f64::from(s);
    let (w, h) = (b.width / s, b.height / s);
    let (dw, dh) = (f64::from(w) * k, f64::from(h) * k);
    let px = shape::premul(ink, alpha);
    let wash = shape::draw(w, h, s, |x, y| {
        shape::scale(px, shape::rounded(x, y, dw, dh, WASH_RADIUS * k))
    });
    c.blit_over(b.x, b.y, &wash);
}

/// `g`, `size` design px square with 2 px strokes, centred in `b` (device px).
pub fn draw_glyph(c: &mut Canvas, b: Rect, g: Glyph, size: i32, ink: Color, s: i32) {
    let img = shape::glyph(g, size, s, 2.0, shape::premul(ink, 0xFF));
    let n = size * s;
    c.blit_over(b.x + (b.width - n) / 2, b.y + (b.height - n) / 2, &img);
}
