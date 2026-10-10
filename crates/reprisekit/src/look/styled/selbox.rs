//! The marquee through the style's [`crate::style::SelectionBox`].

use crate::raster::{Canvas, Rect};
use crate::style::Style;

/// The style's selection box over `r` (device px), its line `s` px wide, the fill laid over what
/// `c` holds. False when the style has none, so the caller keeps the dotted marquee.
pub fn draw_marquee(st: &Style, c: &mut Canvas, r: Rect, s: i32) -> bool {
    let Some(b) = st.selection_box else {
        return false;
    };
    for (q, color, alpha) in b.rects(r, s) {
        if alpha == 255 {
            c.fill_rect(q, color);
            continue;
        }
        let q = q.intersect(&c.bounds());
        for y in q.y..q.bottom() {
            for x in q.x..q.right() {
                let v = c.get(x, y);
                let under = crate::theme::Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
                c.put(x, y, under.toward(color, alpha));
            }
        }
    }
    true
}
