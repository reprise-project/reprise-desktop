//! The forwarders the rebar paints through: the style's [`Part::Rebar`] and [`Part::RebarBand`], else Classic's ([`crate::classic::rebar`]).

use crate::classic;
use crate::look::Paint;
use crate::look::styled::rebar as styled;
use crate::raster::{Canvas, Rect};
use crate::style::Part;

/// The rebar over `r` with a band line starting at each of `lines`, top
/// first: the background, then the lines.
pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, lines: &[i32]) {
    if let Some(st) = p.look.style()
        && styled::draw(st, c, p, r, lines)
    {
        return;
    }
    classic::rebar::draw(c, p, r, lines)
}

/// The line that closes the rebar off, from `x0` to `x1` at `y`: Classic's
/// etched line; nothing under a style with a rebar background, which
/// ends in its own bottom edge.
pub fn draw_closing_line(c: &mut Canvas, p: &Paint, x0: i32, x1: i32, y: i32) {
    if p.look
        .style()
        .is_some_and(|st| st.part(Part::Rebar).is_some())
    {
        return;
    }
    classic::rebar::draw_closing_line(c, p, x0, x1, y)
}
