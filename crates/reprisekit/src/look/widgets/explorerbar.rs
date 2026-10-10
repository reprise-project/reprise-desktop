//! The forwarders an explorer bar paints through: the style's `ExplorerBarHeader` and
//! `ExplorerBarClose` parts when it has them, Classic's drawing otherwise.

use crate::art::glyph::{Glyph, draw_glyph_strokes};
use crate::look::Paint;
use crate::look::styled;
use crate::pointer::Interaction;
use crate::raster::{Canvas, EdgeStyle, Rect};

pub fn draw_strip(c: &mut Canvas, p: &Paint, bar: Rect, header: Rect) {
    if let Some(st) = p.look.style()
        && styled::explorerbar::draw_strip(st, c, header, p.s)
    {
        return;
    }
    let Paint { cs, s, .. } = *p;
    c.fill_rect(header, cs.button_face);
    c.etched_frame_t(bar, cs, s);
    let (x, y, w) = (bar.x, header.bottom(), bar.width);
    c.fill_rect(Rect::new(x + 2 * s, y, w - 3 * s, s), cs.button_shadow);
    c.fill_rect(Rect::new(x + s, y + s, w - 3 * s, s), cs.button_hilight);
}

pub fn draw_close(c: &mut Canvas, p: &Paint, r: Rect, ptr: Interaction) {
    if let Some(st) = p.look.style()
        && styled::explorerbar::draw_close(st, c, r, ptr, p.s)
    {
        return;
    }
    let Paint { cs, s, .. } = *p;
    if ptr != Interaction::Idle {
        c.edge_t(r, EdgeStyle::ThinRaised, cs, s);
    }
    draw_glyph_strokes(c, r.x, r.y, s, Glyph::Close, cs.button_text, cs.button_face);
}
