//! The forwarders a list header paints through: the style's
//! [`Part::HeaderItem`], [`Part::HeaderFiller`] and
//! [`Part::HeaderSortArrow`] when it has them, Classic's drawing
//! otherwise. Classic's header is a raised strip of raised buttons, with
//! no look under the pointer; a held one is flat in a `button_shadow`
//! outline with its title a pixel down and right, and the sort arrow is
//! a solid `button_shadow` triangle.
//!
//! Rects are device px (already scaled by `p.s`).

use crate::look::Paint;
use crate::look::styled;
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};

#[cfg(doc)]
use crate::style::Part;

/// Classic's sort arrow: rows of 1, 3, 5 and 7 px, 6 rows under the
/// header's top.
const ARROW_W: i32 = 7;
const ARROW_H: i32 = 4;
const ARROW_Y: i32 = 6;

pub fn draw_strip(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::header::draw_strip(st, c, p, r)
    {
        return;
    }
    c.fill_rect(r, p.cs.button_face);
    c.edge_t(r, EdgeStyle::ButtonRaised, p.cs, p.s);
}

/// One column's title button in `r`, lit for `ptr`, `sorted` when it
/// carries the sort arrow; the rect its title goes in, which the caller
/// insets by its own text margin. Classic's title sits the same sorted
/// or not.
pub fn draw_item(c: &mut Canvas, p: &Paint, r: Rect, ptr: Interaction, sorted: bool) -> Rect {
    if let Some(st) = p.look.style()
        && let Some(t) = styled::header::draw_item(st, c, p, r, ptr, sorted)
    {
        return t;
    }
    c.fill_rect(r, p.cs.button_face);
    if ptr == Interaction::Pressed {
        let s = p.s;
        for side in [
            Rect::new(r.x, r.y, r.width, s),
            Rect::new(r.x, r.bottom() - s, r.width, s),
            Rect::new(r.x, r.y, s, r.height),
            Rect::new(r.right() - s, r.y, s, r.height),
        ] {
            c.fill_rect(side, p.cs.button_shadow);
        }
    } else {
        c.edge_t(r, EdgeStyle::ButtonRaised, p.cs, p.s);
    }
    classic_title_rect(p, r, ptr)
}

/// The rect [`draw_item`] returns, without drawing.
pub fn item_title_rect(p: &Paint, r: Rect, ptr: Interaction, sorted: bool) -> Rect {
    p.look
        .style()
        .and_then(|st| styled::header::title_rect(st, p, r, ptr, sorted))
        .unwrap_or_else(|| classic_title_rect(p, r, ptr))
}

fn classic_title_rect(p: &Paint, r: Rect, ptr: Interaction) -> Rect {
    match ptr {
        Interaction::Pressed => Rect::new(r.x + p.s, r.y + p.s, r.width, r.height),
        _ => r,
    }
}

/// One column's title button in a dialog's list (the tray's Customize
/// list): the style's item when it has one, else a face button with a
/// 1 px edge, hilight top and left and shadow bottom and right, flatter
/// than [`draw_item`]'s. Returns the rect its title goes in.
pub fn draw_thin_item(c: &mut Canvas, p: &Paint, r: Rect, ptr: Interaction) -> Rect {
    if let Some(st) = p.look.style()
        && let Some(t) = styled::header::draw_item(st, c, p, r, ptr, false)
    {
        return t;
    }
    c.fill_rect(r, p.cs.button_face);
    c.edge_t(r, EdgeStyle::ThinRaised, p.cs, p.s);
    r
}

pub fn sort_arrow_width(p: &Paint) -> i32 {
    p.look
        .style()
        .and_then(styled::header::sort_arrow_width)
        .unwrap_or(ARROW_W)
}

pub fn draw_sort_arrow(c: &mut Canvas, p: &Paint, x: i32, header: Rect, asc: bool) {
    if let Some(st) = p.look.style()
        && styled::header::draw_sort_arrow(st, c, p, x, header, asc)
    {
        return;
    }
    let (y, s) = (header.y + ARROW_Y * p.s, p.s);
    for row in 0..ARROW_H {
        let w = 1 + 2 * row;
        let inset = (ARROW_H - 1 - row) * s;
        let ry = if asc { row } else { ARROW_H - 1 - row };
        c.fill_rect(
            Rect::new(x + inset, y + ry * s, w * s, s),
            p.cs.button_shadow,
        );
    }
}
