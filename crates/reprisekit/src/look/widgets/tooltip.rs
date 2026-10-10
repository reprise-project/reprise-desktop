//! The forwarders a tooltip paints through: the style's
//! [`Part::Tooltip`] when it has them, Classic's drawing otherwise.
//!
//! [`Part::Tooltip`]: crate::style::Part::Tooltip

use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, Insets, Rect};
use crate::theme::Color;

/// The look's own box: how far the text sits inside its edges and how
/// far its shadow reaches past them, design px. None for Classic's flat
/// box, whose padding each tip picks.
pub fn insets(p: &Paint) -> Option<(Insets, Insets)> {
    p.look.style().and_then(styled::tooltip::insets)
}

pub fn draw_box(c: &mut Canvas, p: &Paint, r: Rect) -> Color {
    if let Some(st) = p.look.style()
        && let Some(ink) = styled::tooltip::draw_box(st, c, r, p.s, p.cs.info_text)
    {
        return ink;
    }
    c.fill_rect(r, p.cs.info_text);
    c.fill_rect(r.inset(p.s), p.cs.info_bk);
    p.cs.info_text
}
