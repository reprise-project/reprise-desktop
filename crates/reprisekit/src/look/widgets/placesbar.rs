use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::State;

pub fn draw_panel(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::placesbar::draw_panel(st, c, r, p.s)
    {
        return;
    }
    c.fill_rect(r, p.cs.button_shadow);
    c.edge_t(r, EdgeStyle::ButtonSunken, p.cs, p.s);
}

/// A tile's frame in `state` ([`crate::controls::placesbar::PlacesBar::state`]). Classic frames
/// a selected (or held) tile and leaves a lit one plain.
pub fn draw_tile(c: &mut Canvas, p: &Paint, band: Rect, state: State) {
    if let Some(st) = p.look.style()
        && styled::placesbar::draw_tile(st, c, band, state, p.s)
    {
        return;
    }
    if state == State::Hot {
        return;
    }
    let ring = Rect::new(band.x + 2 * p.s, band.y, band.width - 4 * p.s, band.height);
    c.ring_t(ring, p.cs.button_dk_shadow, p.cs.button_hilight, p.s);
}
