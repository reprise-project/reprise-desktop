//! Classic's rebar painter: `button_face` behind the stack and an etched line (a `button_shadow` row
//! over a `button_hilight` row) along the top of every band. The rebar control ([`crate::controls::rebar`])
//! reaches it through [`crate::look::widgets::rebar`].

use crate::paint::Paint;
use crate::raster::{Canvas, Rect};

/// Face over `r` (scaled), then an etched line across its full width at each of `lines` (each band's top, scaled).
pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, lines: &[i32]) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.button_face);
    for &y in lines {
        c.etched_hline_t(r.x, r.right(), y, cs, s);
    }
}

/// The etched line that closes the rebar off, from `x0` to `x1` at `y` (scaled).
pub fn draw_closing_line(c: &mut Canvas, p: &Paint, x0: i32, x1: i32, y: i32) {
    c.etched_hline_t(x0, x1, y, p.cs, p.s);
}
