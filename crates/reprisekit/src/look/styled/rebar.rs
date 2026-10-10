//! The rebar through the style's `Part::Rebar` and `Part::RebarBand`: the
//! background once, then a band line at each line the rebar hands over
//! (Blue's first band wears none). [`draw`] returns `false` when the style
//! lacks either part, so the forwarder falls back to Classic.

use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

/// The band line's height in the look's own pixels.
const LINE_H: i32 = 2;

/// [`crate::look::widgets::rebar::draw`] through `st`; `false` when `st`
/// has no rebar parts.
pub fn draw(st: &Style, c: &mut Canvas, p: &Paint, r: Rect, lines: &[i32]) -> bool {
    if !st.has_parts(&[Part::Rebar, Part::RebarBand]) {
        return false;
    }
    st.draw(c, r, Part::Rebar, State::Normal, p.s);
    for &y in lines {
        let line = Rect::new(r.x, y, r.width, LINE_H * p.s);
        st.draw(c, line, Part::RebarBand, State::Normal, p.s);
    }
    true
}
