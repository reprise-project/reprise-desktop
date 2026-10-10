//! Rects are device px (already scaled by `p.s`).

use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::Part;

const PANE_SHORT: i32 = 3;

pub fn draw_bar(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::statusbar::draw_bar(st, c, p, r)
    {
        return;
    }
    c.fill_rect(r, p.cs.button_face);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneBox {
    pub text: Rect,
    pub icon: Rect,
}

pub fn draw_pane(
    c: &mut Canvas,
    p: &Paint,
    bar: Rect,
    pane: Rect,
    next: Option<i32>,
    grip: bool,
    first: bool,
) -> PaneBox {
    if let Some(st) = p.look.style()
        && styled::statusbar::has_pane(st)
    {
        let right = match next {
            Some(x) => x - PANE_SHORT * p.s,
            None if grip => bar.right() - PANE_SHORT * p.s,
            None => bar.right(),
        };
        let r = Rect::new(pane.x, bar.y, right - pane.x, bar.height);
        // The last pane's divider hides under the grip; its content box is still
        // the part's.
        let content = if next.is_none() && grip {
            st.content_rect(Part::StatusPane, r, p.s)
        } else {
            styled::statusbar::draw_pane(st, c, p, r).unwrap_or(pane)
        };
        // Every pane after the first starts its boxes a pixel left, three past the
        // divider before it.
        let dx = if first { 0 } else { p.s };
        let (x, w) = (pane.x - dx, pane.width + dx);
        return PaneBox {
            text: Rect::new(x, content.y, w, pane.height),
            icon: Rect::new(x, content.y, w, content.height),
        };
    }
    c.edge_t(pane, EdgeStyle::ThinSunken, p.cs, p.s);
    PaneBox {
        text: pane,
        icon: pane,
    }
}

pub fn draw_grip(c: &mut Canvas, p: &Paint, bar: Rect, g: Rect) {
    if let Some(st) = p.look.style()
        && st.part(Part::StatusBar).is_some()
        && st.part(Part::SizeBox).is_some()
    {
        super::draw_size_box(st, c, p, bar);
        return;
    }
    let s = p.s;
    // One row and column past the ridge box wipe the last pane's recess lines
    // under the grip.
    let row = Rect::new(g.x + 4 * s, g.bottom(), g.width - 3 * s, s);
    c.fill_rect(row, p.cs.button_face);
    let col = Rect::new(g.right(), g.y + 4 * s, s, g.height - 4 * s);
    c.fill_rect(col, p.cs.button_face);
    super::draw_size_grip(c, p, g);
}
