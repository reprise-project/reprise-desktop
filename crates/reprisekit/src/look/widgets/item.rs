//! The forwarders list, tree and icon views mark their items through: the style's item
//! box ([`crate::style::Part::ListItem`]) when it has one, else Classic's highlight.

use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, Rect};
use crate::theme::Color;

pub use crate::look::styled::item::Mark;

/// Whether `p`'s look boxes its items: a box spans the row or cell, Classic's highlight only the label.
pub fn boxed(p: &Paint) -> bool {
    p.look.style().is_some_and(styled::item::boxes_items)
}

/// `m`'s box across `r` (device px); true when the look boxes its items, whether or not `m`
/// draws anything. A selection greys with `m.dim` when the window is inactive (`p.active`).
pub fn draw_item(c: &mut Canvas, p: &Paint, r: Rect, m: Mark) -> bool {
    let m = Mark {
        dim: m.dim || !p.active,
        ..m
    };
    p.look
        .style()
        .is_some_and(|st| styled::item::draw_item(st, c, r, m, p.s))
}

/// A selected desktop icon's box across `r`; false under a look without
/// one.
pub fn draw_desktop_item(c: &mut Canvas, p: &Paint, r: Rect) -> bool {
    p.look
        .style()
        .is_some_and(|st| styled::item::draw_desktop_item(st, c, r, p.s))
}

/// A selected desktop label's `fill` across `r`, inside a 1 px darker edge under a look with
/// [`Style::desktop_label_edge`](crate::style::Style::desktop_label_edge), so the box holds
/// its shape on a wallpaper of the fill's own colour.
pub fn fill_desktop_label(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) {
    match p.look.style().and_then(|st| st.desktop_label_edge) {
        Some(darken) => {
            c.fill_rect(r, fill.toward(Color::rgb(0, 0, 0), darken));
            c.fill_rect(r.inset(p.s), fill);
        }
        None => c.fill_rect(r, fill),
    }
}

/// A tree expander centred on device point `at`, grown with the text ([`Paint::small`]); false
/// under a look without one, which keeps Classic's box.
pub fn draw_expander(c: &mut Canvas, p: &Paint, at: (i32, i32), expanded: bool, hot: bool) -> bool {
    let look = p.small_look(c);
    let p = &p.small(&look);
    p.look
        .style()
        .is_some_and(|st| styled::item::draw_expander(st, c, at, expanded, hot, p.s))
}
