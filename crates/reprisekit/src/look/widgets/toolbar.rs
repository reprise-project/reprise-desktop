//! The forwarders a toolbar paints through: the style's
//! [`Part::ToolbarButton`], [`Part::ToolbarDrop`] and
//! [`Part::ToolbarSeparator`] when it has them, Classic's drawing
//! otherwise ([`crate::classic::toolbar`]). Every rect is already scaled.

use crate::classic;
use crate::look::Paint;
use crate::look::styled::toolbar as styled;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State};
use crate::text::TextRenderer;
use crate::theme::Color;

/// The frame a toolbar button draws: Disabled over everything, then
/// Pressed while held, then CheckedHot or Checked while latched down (the
/// pointer on it or not), then Hot, then Normal.
pub fn state(enabled: bool, hot: bool, pressed: bool, latched: bool) -> State {
    match (enabled, pressed, latched, hot) {
        (false, ..) => State::Disabled,
        (true, true, ..) => State::Pressed,
        (true, false, true, true) => State::CheckedHot,
        (true, false, true, false) => State::Checked,
        (true, false, false, true) => State::Hot,
        (true, false, false, false) => State::Normal,
    }
}

/// The strip's background over `r`: `button_face` in Classic; nothing
/// under a style with toolbar parts, whose rebar draws what sits behind.
pub fn draw_background(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && st.part(Part::ToolbarButton).is_some()
    {
        return;
    }
    c.fill_rect(r, p.cs.button_face);
}

/// A button's frame over `r` in `state`. `split` is where a split
/// button's drop part starts: the style draws its main part and its drop
/// part as two frames; Classic draws one bevel over the whole button.
pub fn draw_button(c: &mut Canvas, p: &Paint, r: Rect, split: Option<i32>, state: State) {
    if let Some(st) = p.look.style()
        && styled::draw_button(st, c, p, r, split, state)
    {
        return;
    }
    classic::toolbar::draw_button(c, p, r, state)
}

/// A separator in its slot `r`; `big` for Explorer's 30 px strip.
pub fn draw_separator(c: &mut Canvas, p: &Paint, big: bool, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::draw_separator(st, c, p, r)
    {
        return;
    }
    classic::toolbar::draw_separator(c, p, big, r)
}

/// How far a button's icon, label and arrow move right and down in
/// `state`: one pixel held in every look, and latched down too under a
/// style with toolbar parts.
pub fn content_shift(p: &Paint, state: State) -> i32 {
    let latched = matches!(state, State::Checked | State::CheckedHot);
    let styled = p
        .look
        .style()
        .is_some_and(|st| st.part(Part::ToolbarButton).is_some());
    if state == State::Pressed || (latched && styled) {
        p.s
    } else {
        0
    }
}

pub fn draw_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    state: State,
    px: u32,
) {
    if let Some(st) = p.look.style()
        && styled::draw_label(st, c, text, p, r, label, state, px)
    {
        return;
    }
    classic::toolbar::draw_label(c, text, p, r, label, state, px)
}

/// The ink a drop arrow takes: `button_text`, or once greyed the style's
/// own greyed arrow ink, else `gray_text`.
pub fn arrow_ink(p: &Paint, enabled: bool) -> Color {
    if enabled {
        return p.cs.button_text;
    }
    p.look
        .style()
        .and_then(|st| st.text_color(Part::ToolbarDrop, State::Disabled))
        .unwrap_or(p.cs.gray_text)
}
