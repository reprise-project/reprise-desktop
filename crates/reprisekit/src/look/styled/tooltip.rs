//! A tooltip through the style's [`Part::Tooltip`]:
//! each drawer returns `None` when the style lacks its part, so the
//! forwarder in [`crate::look::widgets::tooltip`] falls back to Classic.

use crate::raster::{Canvas, Insets, Rect};
use crate::style::{Part, State, Style};
use crate::theme::Color;

/// How far the text sits inside the box's edges, and how far the box's
/// shadow reaches past them, design px.
pub fn insets(st: &Style) -> Option<(Insets, Insets)> {
    st.part(Part::Tooltip).map(|p| (p.content, p.outset))
}

/// The box in `r`, device px, at scale `s` (as [`Style::draw`]); the
/// text's ink, the part's own or `fallback`.
pub fn draw_box(st: &Style, c: &mut Canvas, r: Rect, s: i32, fallback: Color) -> Option<Color> {
    st.part(Part::Tooltip)?;
    st.draw(c, r, Part::Tooltip, State::Normal, s);
    Some(
        st.text_color(Part::Tooltip, State::Normal)
            .unwrap_or(fallback),
    )
}
