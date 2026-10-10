//! The forwarders list, tree and icon views mark their items through: the style's item
//! box ([`crate::style::Part::ListItem`]) when it has one, else Classic's highlight.

use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, Rect};

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

/// A tree expander centred on device point `at`; false under a look
/// without one, which keeps Classic's box.
pub fn draw_expander(c: &mut Canvas, p: &Paint, at: (i32, i32), expanded: bool, hot: bool) -> bool {
    p.look
        .style()
        .is_some_and(|st| styled::item::draw_expander(st, c, at, expanded, hot, p.s))
}
