//! A balloon drawn as a look's [`BalloonLook`] says: the bubble with its
//! stepped corners, shaded fill, tail and shadow, and the close cross
//! with its box under the pointer ([`Part::BalloonClose`]).
//!
//! Rects are device px, already scaled by `s`; the look's steps are drawn
//! `s` px square.

use crate::raster::{Canvas, Rect};
use crate::style::{BalloonLook, Part, State, Style};
use crate::theme::Color;

/// Where a bubble's tail goes: its straight side's column (device px),
/// whether the tail widens to the right of it (else to the left), and
/// whether it hangs under the bubble (else rises over it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tail {
    pub x: i32,
    pub right: bool,
    pub down: bool,
}

fn fill_at(b: &BalloonLook, y: i32, h: i32, s: i32) -> Color {
    let span = (h - 2 * s - 1).max(1);
    let k = ((y - s).clamp(0, span) * 255 + span / 2) / span;
    b.fill.0.toward(b.fill.1, k as u8)
}

/// How far the frame's outer edge stands in on logical row `y` of a
/// bubble `h` rows tall, and where the frame ends on that row (the row
/// nearer the edge's start, or the same for a straight row).
fn row_edge(b: &BalloonLook, y: i32, h: i32) -> (i32, i32) {
    let n = b.corner.len() as i32;
    let at = |y: i32| -> i32 {
        let from_edge = y.min(h - 1 - y);
        if from_edge < n {
            b.corner[from_edge as usize]
        } else {
            0
        }
    };
    let o = at(y);
    let from_edge = y.min(h - 1 - y);
    if from_edge == 0 {
        return (o, i32::MAX);
    }
    let outer = if y < h - 1 - y { at(y - 1) } else { at(y + 1) };
    (o, outer.max(o))
}

/// Draw `b`'s bubble filling `r` with its tail at `tail` and its shadow
/// past `r`'s right and bottom edges.
pub fn draw_bubble(b: &BalloonLook, c: &mut Canvas, r: Rect, tail: Tail, s: i32) {
    if let Some(sh) = b.shadow {
        super::combodrop::draw_drop_shadow(c, sh, r, s);
    }
    let (w, h) = (r.width / s, r.height / s);
    let cell = |c: &mut Canvas, x: i32, y: i32, color: Color| {
        c.fill_rect(Rect::new(r.x + x * s, r.y + y * s, s, s), color);
    };
    for y in 0..h {
        let (o, end) = row_edge(b, y, h);
        let end = end.min(w / 2);
        // The frame from the outer edge to `end` on either side, the
        // fill between, one device row at a time.
        let row = Rect::new(r.x + o * s, r.y + y * s, (w - 2 * o) * s, s);
        for dy in 0..s {
            let fill = fill_at(b, y * s + dy, r.height, s);
            c.fill_rect(Rect::new(row.x, row.y + dy, row.width, 1), fill);
        }
        for x in o..=end {
            cell(c, x, y, b.frame);
            cell(c, w - 1 - x, y, b.frame);
        }
    }

    // The tail: the frame row opens over its width less its two sides;
    // row k past the frame keeps its straight side, the fill, and the 2 px
    // slope closing in by one each row.
    let (_, tail_w, tail_h) = b.tail;
    let tx = (tail.x - r.x) / s;
    let dir = if tail.right { 1 } else { -1 };
    let (edge_y, step) = if tail.down { (h - 1, 1) } else { (0, -1) };
    let ink = fill_at(b, if tail.down { r.height - s - 1 } else { s }, r.height, s);
    for i in 1..tail_w - 1 {
        cell(c, tx + dir * i, edge_y, ink);
    }
    for k in 0..tail_h {
        let y = edge_y + step * (k + 1);
        let slope = tail_w - 2 - k;
        for i in 1..slope.max(1) {
            cell(c, tx + dir * i, y, ink);
        }
        cell(c, tx, y, b.frame);
        for i in [slope, slope + 1] {
            if i >= 0 {
                cell(c, tx + dir * i, y, b.frame);
            }
        }
    }
}

pub fn draw_close(st: &Style, b: &BalloonLook, c: &mut Canvas, r: Rect, hot: bool, s: i32) {
    if hot && st.part(Part::BalloonClose).is_some() {
        st.draw(c, r, Part::BalloonClose, State::Hot, s);
    }
    let ink = if hot { b.cross_ink.1 } else { b.cross_ink.0 };
    let (cx, cy) = (r.x + b.cross_at.0 * s, r.y + b.cross_at.1 * s);
    for (y, bits) in b.cross.iter().enumerate() {
        for x in 0..16 {
            if bits >> x & 1 == 1 {
                c.fill_rect(Rect::new(cx + x * s, cy + y as i32 * s, s, s), ink);
            }
        }
    }
}
