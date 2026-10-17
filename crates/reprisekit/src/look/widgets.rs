//! The common widgets, one function each. Controls and hosts draw widgets
//! through here; each function paints the look's part when the style has
//! one (its twin in `look::styled`) and Classic's otherwise.

use crate::classic;
use crate::classic::widgets::{Arrow, CheckState, ScrollHit};
pub use crate::flyout::draw_link;
use crate::look::Paint;
use crate::look::styled;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::style::{Part, State};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;
pub use crate::trackbar;

pub use crate::pointer::{Interaction, Lit};
pub mod explorerbar;
pub mod header;
pub mod item;
pub mod placesbar;
pub mod rebar;
pub mod statusbar;
pub mod taskpane;
pub mod toolbar;
pub mod tooltip;

/// A push button. `ptr` is what the pointer does to it; Classic draws
/// `Pressed` pushed and `Hot` at rest. A greyed label is etched on Classic's
/// button, unless the style registers a Disabled push button ink: then it is
/// inked once, flat, in that. A default button in an inactive window
/// (`p.active` false) draws as a plain one.
#[expect(clippy::too_many_arguments)]
pub fn draw_button(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    default: bool,
    ptr: Interaction,
    enabled: bool,
    focus: bool,
    px: u32,
) {
    let default = default && p.active;
    let label = &*p.label(label);
    if let Some(st) = p.look.style()
        && styled::button::draw_button(st, c, text, p, r, label, default, ptr, enabled, focus, px)
    {
        return;
    }
    let pushed = ptr == Interaction::Pressed;
    // A style with no push button part may still ink a greyed label flat on Classic's button.
    if !enabled
        && let Some(ink) = p
            .look
            .style()
            .and_then(|st| st.text_color(Part::PushButton, State::Disabled))
    {
        classic::widgets::draw_button(c, text, p, r, "", default, pushed, false, focus, px);
        let content = if pushed {
            Rect::new(r.x + p.s, r.y + p.s, r.width, r.height)
        } else if default {
            r.inset(p.s)
        } else {
            r
        };
        classic::widgets::draw_label(c, text, content, label, false, ink, Align::Center, px, p.s);
        return;
    }
    classic::widgets::draw_button(c, text, p, r, label, default, pushed, enabled, focus, px)
}

/// A push button's face with nothing on it, for a control that draws its own
/// content over it (the colour button's swatch and drop arrow).
pub fn draw_button_face(c: &mut Canvas, p: &Paint, r: Rect, enabled: bool, ptr: Interaction) {
    if let Some(st) = p.look.style()
        && styled::button::draw_button_face(st, c, r, enabled, ptr, p.s)
    {
        return;
    }
    c.fill_rect(r, p.cs.button_face);
    c.edge_t(r, EdgeStyle::ButtonRaised, p.cs, p.s);
}

/// A greyed control's label. `part` names the themed control it belongs to;
/// a styled look inks it once, flat, in the part's Disabled text colour, or
/// `gray_text` when the style registers none. A plain static label passes
/// `None` and stays etched everywhere ([`classic::widgets::draw_disabled_label`]).
#[expect(clippy::too_many_arguments)]
pub fn draw_disabled_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    align: Align,
    px: u32,
    part: Option<Part>,
) {
    let label = &*p.label(label);
    if let (Some(st), Some(part)) = (p.look.style(), part) {
        let ink = st
            .text_color(part, State::Disabled)
            .unwrap_or(p.cs.gray_text);
        classic::widgets::draw_label(c, text, r, label, false, ink, align, px, p.s);
        return;
    }
    classic::widgets::draw_disabled_label(c, text, p, r, label, align, px)
}

/// A two-state check box, held when `Pressed`.
pub fn draw_checkbox(c: &mut Canvas, p: &Paint, b: Rect, checked: bool, ptr: Interaction) {
    if let Some(st) = p.look.style()
        && styled::checkbox::draw_checkbox(st, c, p, b, checked, ptr)
    {
        return;
    }
    let state = if checked {
        CheckState::On
    } else {
        CheckState::Off
    };
    classic::widgets::draw_checkbox_held(c, p, b, state, ptr == Interaction::Pressed)
}

/// A list row's check box; Classic's is a flat box.
pub fn draw_list_check(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    if let Some(st) = p.look.style()
        && styled::checkbox::draw_checkbox(st, c, p, b, checked, Interaction::Idle)
    {
        return;
    }
    classic::widgets::draw_list_check(c, p, b, checked)
}

pub fn draw_checkbox_disabled(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    if let Some(st) = p.look.style()
        && styled::checkbox::draw_checkbox_disabled(st, c, p, b, checked)
    {
        return;
    }
    classic::widgets::draw_checkbox_disabled(c, p, b, checked)
}

/// A check box in any of its three states, held when `Pressed`.
pub fn draw_checkbox_state(
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    state: CheckState,
    ptr: Interaction,
) {
    if let Some(st) = p.look.style()
        && styled::checkbox::draw_checkbox_state(st, c, p, b, state, ptr)
    {
        return;
    }
    classic::widgets::draw_checkbox_held(c, p, b, state, ptr == Interaction::Pressed)
}

/// A greyed check box in any of its three states. Classic's greyed box has
/// no third state and shows `Mixed` empty.
pub fn draw_checkbox_state_disabled(c: &mut Canvas, p: &Paint, b: Rect, state: CheckState) {
    if let Some(st) = p.look.style()
        && styled::checkbox::draw_checkbox_state_disabled(st, c, p, b, state)
    {
        return;
    }
    classic::widgets::draw_checkbox_disabled(c, p, b, state == CheckState::On)
}

pub fn draw_radio(c: &mut Canvas, p: &Paint, b: Rect, checked: bool, ptr: Interaction) {
    if let Some(st) = p.look.style()
        && styled::radio::draw_radio(st, c, p, b, checked, ptr)
    {
        return;
    }
    classic::widgets::draw_radio(c, p, b, checked)
}

pub fn draw_radio_disabled(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    if let Some(st) = p.look.style()
        && styled::radio::draw_radio_disabled(st, c, p, b, checked)
    {
        return;
    }
    classic::widgets::draw_radio_disabled(c, p, b, checked)
}

pub fn draw_group_box(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    g: Rect,
    label_r: Rect,
    label: &str,
    px: u32,
) {
    if let Some(st) = p.look.style()
        && styled::groupbox::draw_group_box(st, c, text, p, g, label_r, &p.label(label), px)
    {
        return;
    }
    classic::widgets::draw_group_box(c, text, p, g, label_r, label, px)
}

/// A vertical scrollbar; the style draws the part `lit` names in its state.
pub fn draw_scrollbar(
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    top: usize,
    visible: usize,
    total: usize,
    lit: Lit<ScrollHit>,
) {
    if let Some(st) = p.look.style()
        && styled::scrollbar::draw_scrollbar(st, c, p, sb, top, visible, total, lit)
    {
        return;
    }
    classic::widgets::draw_scrollbar(c, p, sb, top, visible, total)
}

/// A horizontal scrollbar; the style draws the part `lit` names in its state.
pub fn draw_hscrollbar(
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    left: usize,
    visible: usize,
    total: usize,
    lit: Lit<ScrollHit>,
) {
    if let Some(st) = p.look.style()
        && styled::scrollbar::draw_hscrollbar(st, c, p, sb, left, visible, total, lit)
    {
        return;
    }
    classic::widgets::draw_hscrollbar(c, p, sb, left, visible, total)
}

/// The box where a vertical and a horizontal scrollbar meet (`r`, scaled):
/// `button_face`, with the size grip ([`draw_size_grip`]) when `grip`. It
/// follows the bars: beside Classic bars it is Classic's grip even when the
/// style has a size box, and a look with its own scrollbars but no size box
/// part leaves out the grip.
pub fn draw_scroll_corner(c: &mut Canvas, p: &Paint, r: Rect, grip: bool) {
    // Styled only when both bars draw styled.
    let styled = p.look.style().is_some_and(|st| {
        st.has_parts(&[
            Part::ScrollTrackV,
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollThumbV,
            Part::ScrollTrackH,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbH,
        ])
    });
    if grip && !styled {
        classic::widgets::draw_size_grip(c, p.cs, r, p.s);
        return;
    }
    c.fill_rect(r, p.cs.button_face);
    if grip && let Some(st) = p.look.style() {
        draw_size_box(st, c, p, r);
    }
}

/// The size grip in the lower-right corner of a resizable window (`r`, scaled).
pub fn draw_size_grip(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && st.part(Part::SizeBox).is_some()
    {
        c.fill_rect(r, p.cs.button_face);
        draw_size_box(st, c, p, r);
        return;
    }
    classic::widgets::draw_size_grip(c, p.cs, r, p.s)
}

/// The style's size box in `r`'s lower-right corner, 1 px in from the right and bottom edges.
pub(super) fn draw_size_box(st: &crate::style::Style, c: &mut Canvas, p: &Paint, r: Rect) {
    let Some(img) = st.part(Part::SizeBox) else {
        return;
    };
    // The frame's design size: an atlas painted at scale holds `scale` pixels per design pixel.
    let f = img.frame(State::Normal);
    let (w, h) = (f.width / img.scale * p.s, f.height / img.scale * p.s);
    let at = Rect::new(r.right() - w - p.s, r.bottom() - h - p.s, w, h);
    st.draw(c, at, Part::SizeBox, State::Normal, p.s);
}

/// A month calendar's frame round the box `r` (scaled), over its fill: the
/// style's calendar frame part, else a 2 px sunken edge in the scheme's colours.
pub fn draw_calendar_frame(c: &mut Canvas, p: &Paint, r: Rect, enabled: bool) {
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    if let Some(st) = p.look.style()
        && st.part(Part::CalendarFrame).is_some()
    {
        st.draw(c, r, Part::CalendarFrame, state, p.s);
        return;
    }
    c.edge_t(r, EdgeStyle::ButtonSunken, p.cs, p.s);
}

/// The box behind today's number when the style has one: 24x15 centred across
/// `number` (its scaled text box), a row above it. Returns the number's ink
/// (the style's page link ink, else its text's), or None, drawing nothing,
/// for a look that outlines today instead.
pub fn draw_calendar_today(c: &mut Canvas, p: &Paint, number: Rect) -> Option<Color> {
    let st = p.look.style()?;
    st.part(Part::CalendarToday)?;
    let s = p.s;
    let cx = number.x + number.width / 2;
    let r = Rect::new(cx - 12 * s, number.y - s, 24 * s, 15 * s);
    st.draw(c, r, Part::CalendarToday, State::Normal, s);
    Some(st.page.map_or(p.cs.window_text, |inks| inks.link))
}

/// A month calendar's step-back (`Arrow::Left`) or step-on arrow in `r`: the
/// style's bare arrow centred in `r`, else Classic's raised arrow button.
pub fn draw_calendar_arrow(c: &mut Canvas, p: &Paint, r: Rect, arrow: Arrow, hot: bool) {
    let part = if arrow == Arrow::Left {
        Part::CalendarPrev
    } else {
        Part::CalendarNext
    };
    if let Some(st) = p.look.style()
        && st.part(part).is_some()
    {
        let state = if hot { State::Hot } else { State::Normal };
        st.draw(c, r, part, state, p.s);
        return;
    }
    classic::widgets::draw_arrow_button(c, p, r, arrow, true)
}

/// The clock popup's analog clock at `time` (hour 0..23, minute, second) in the square `r`
/// (device px). Classic has none and draws nothing.
pub fn draw_clock_face(c: &mut Canvas, p: &Paint, r: Rect, time: (u32, u32, u32)) {
    if let Some(paint) = p.look.style().and_then(|st| st.clock_face) {
        c.blit_over(r.x, r.y, &paint(time, r.width / p.s.max(1), p.s));
    }
}

/// The copy dialog's flight in `r` (its height [`crate::copyart::STRIP_H`]): the style's
/// painter, else Classic's. Drawn at the canvas's own scale, so it stays sharp at 2.
pub fn draw_copy_flight(c: &mut Canvas, p: &Paint, r: Rect, f: &crate::copyart::Flight) {
    let d = p.s * c.scale();
    let paint = p
        .look
        .style()
        .and_then(|st| st.copy_flight)
        .unwrap_or(classic::copyflight::paint);
    let art = paint(f, p.cs, r.width, d).at_scale(c.scale());
    c.blit_over(r.x * p.s, r.y * p.s, &art);
}

/// The copy dialog's throughput graph in `r`, for a look with a [`crate::copyart::GraphLook`];
/// false (nothing drawn) for the others.
pub fn draw_copy_graph(c: &mut Canvas, p: &Paint, r: Rect, g: &crate::copyart::Graph) -> bool {
    let Some(look) = p.look.style().and_then(|st| st.copy_graph) else {
        return false;
    };
    let d = p.s * c.scale();
    let art = (look.paint)(g, p.cs, r.width, r.height, d).at_scale(c.scale());
    c.blit_over(r.x * p.s, r.y * p.s, &art);
    true
}

/// A month calendar's weekday band in `r`; returns the ink of the day
/// letters: the style's band part and text colour, else Classic's button
/// shadow band with face-coloured letters.
pub fn draw_calendar_band(c: &mut Canvas, p: &Paint, r: Rect, enabled: bool) -> Color {
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    if let Some(st) = p.look.style()
        && st.part(Part::CalendarBand).is_some()
    {
        st.draw(c, r, Part::CalendarBand, state, p.s);
        return st
            .text_color(Part::CalendarBand, state)
            .unwrap_or(p.cs.button_face);
    }
    c.fill_rect(r, p.cs.button_shadow);
    p.cs.button_face
}

/// A group box title's ink: the style's group box label colour, else `button_text`.
/// Both the group box and [`crate::controls::form::Control::GroupTitle`] use it.
pub fn group_label_ink(p: &Paint) -> Color {
    p.look
        .style()
        .and_then(|st| st.text_color(Part::GroupBox, State::Normal))
        .unwrap_or(p.cs.button_text)
}

/// A sunken field's frame alone. `fill` is the colour the caller paints just
/// inside it (`window`, or `button_face` for a disabled field). The style uses
/// its list border part, else its edit border part.
pub fn draw_field_frame(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) {
    if let Some(st) = p.look.style()
        && styled::field::draw_field_frame(st, c, p, r, fill)
    {
        return;
    }
    classic::widgets::draw_field_frame(c, p, r, fill)
}

/// A multi-line edit's frame: [`draw_field_frame`], with the style's edit line where it draws lists and edits apart.
pub fn draw_edit_frame(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) {
    if let Some(st) = p.look.style()
        && styled::field::draw_edit_frame(st, c, p, r, fill)
    {
        return;
    }
    classic::widgets::draw_field_frame(c, p, r, fill)
}

/// A lone caption close box at `r`: the style's [`Part::CloseButton`] filling
/// `r`, else Classic's 16x14 caption button centred in `r`.
pub fn draw_close_box(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && st.part(Part::CloseButton).is_some()
    {
        st.draw(c, r, Part::CloseButton, State::Normal, p.s);
        return;
    }
    let (w, h) = (
        classic::widgets::CLOSE_W * p.s,
        classic::widgets::CLOSE_H * p.s,
    );
    let b = Rect::new(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, w, h);
    classic::widgets::draw_caption_button(c, p.cs, b, crate::art::glyph::Glyph::Close, false, p.s);
}

/// The well round a dialog's picture, colour spectrum or appearance sample:
/// the style's [`Part::WellFrame`] round `window`, else `r` in `button_face`
/// inside Classic's `edge`.
pub fn draw_well(c: &mut Canvas, p: &Paint, r: Rect, edge: EdgeStyle) {
    if let Some(st) = p.look.style()
        && st.part(Part::WellFrame).is_some()
    {
        c.fill_rect(r, p.cs.window);
        st.draw(c, r, Part::WellFrame, State::Normal, p.s);
        return;
    }
    c.fill_rect(r, p.cs.button_face);
    c.edge_t(r, edge, p.cs, p.s);
}

/// What a combo box's frame shows: enabled, focused, what the pointer does to
/// the frame (lit from its field or its button), and whether the field is an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComboLook {
    pub enabled: bool,
    pub focused: bool,
    pub ptr: Interaction,
    pub editable: bool,
}

/// A combo box's field frame alone; `r` and `drop` (its button's rect) are
/// scaled. Enabled, [`draw_field_frame`] in the style's frame for focus and
/// pointer. Greyed, the style's edit border draws its Disabled frame around
/// the text part, while Classic's sunken edge stays and only `fill` changes. A
/// drop-down list (not `editable`) is the style's list body across the whole
/// control. The drop button goes over it ([`draw_combo_drop`]).
pub fn draw_combo_frame(
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    drop: Rect,
    fill: Color,
    look: ComboLook,
) {
    if let Some(st) = p.look.style()
        && styled::field::draw_combo_frame(st, c, p, r, drop, fill, look)
    {
        return;
    }
    classic::widgets::draw_field_frame(c, p, r, fill)
}

/// A spinner's field, plain or a buddy's, filled and framed. Enabled, `window`
/// inside the field frame. Greyed, Classic fills `button_face`; the style's
/// edit border draws its line alone and leaves the fill to what is behind it
/// (or, with `greyed_field_frame`, its Disabled frame round `window`), and
/// wells the button column. `up` and `down` are the buttons' rects; the
/// buttons go over it ([`draw_spin_buttons`]).
pub fn draw_spin_field(c: &mut Canvas, p: &Paint, r: Rect, up: Rect, down: Rect, enabled: bool) {
    if let Some(st) = p.look.style()
        && styled::field::draw_spin_field(st, c, p, r, up, down, enabled)
    {
        return;
    }
    let fill = if enabled {
        p.cs.window
    } else {
        p.cs.button_face
    };
    c.fill_rect(r, fill);
    classic::widgets::draw_field_frame(c, p, r, fill)
}

/// A sunken field, filled and framed. `fill` is what the caller paints the
/// returned inner rect with. At rest and unfocused; a single-line edit that
/// knows either draws [`draw_edit_field`].
pub fn draw_sunken_field(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) -> Rect {
    draw_edit_field(c, p, r, fill, false, Interaction::Idle)
}

/// [`draw_sunken_field`] for an enabled single-line edit that is `focused` or
/// under the pointer (`ptr`): the style's edit border draws Focused over
/// Pressed over Hot, else Normal; Classic's sunken edge is the same throughout.
pub fn draw_edit_field(
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    fill: Color,
    focused: bool,
    ptr: Interaction,
) -> Rect {
    if let Some(st) = p.look.style()
        && let Some(inner) = styled::field::draw_sunken_field(st, c, p, r, fill, focused, ptr)
    {
        return inner;
    }
    classic::widgets::draw_sunken_field(c, p, r, fill)
}

/// A greyed single-line edit's field; returns the inner rect for text, as
/// [`draw_sunken_field`] does. The style fills with what sits behind the field
/// ([`Paint::fill_back`]), or with `window` inside its Disabled frame when it
/// has `greyed_field_frame`. Classic fills `button_face`.
pub fn draw_disabled_field(c: &mut Canvas, p: &Paint, r: Rect) -> Rect {
    if let Some(st) = p.look.style()
        && let Some(inner) = styled::field::draw_disabled_field(st, c, p, r)
    {
        return inner;
    }
    let inner = classic::widgets::draw_sunken_field(c, p, r, p.cs.button_face);
    c.fill_rect(inner, p.cs.button_face);
    inner
}

/// A raised arrow button, Classic's in every look: a style has no arrow
/// button part, and scrollbars, spinners and combo boxes draw their own.
pub fn draw_arrow_button(c: &mut Canvas, p: &Paint, r: Rect, arrow: Arrow, enabled: bool) {
    classic::widgets::draw_arrow_button(c, p, r, arrow, enabled)
}

/// A combo box's drop button. A drop-down list (not `editable`) whose style
/// draws its list body whole ([`draw_combo_frame`]) draws nothing here.
pub fn draw_combo_drop(
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    enabled: bool,
    ptr: Interaction,
    editable: bool,
) {
    if let Some(st) = p.look.style()
        && styled::combodrop::draw_combo_drop(st, c, p, r, enabled, ptr, editable)
    {
        return;
    }
    classic::widgets::draw_arrow_button(c, p, r, Arrow::Down, enabled)
}

/// An open combo list's body in `r` (scaled): `window` inside a 1 px frame,
/// which Classic inks `button_text`; a style with a [`crate::style::DropList`]
/// inks its own frame and casts its shadow past the right and bottom edges.
pub fn draw_drop_list(c: &mut Canvas, p: &Paint, r: Rect) {
    if let Some(st) = p.look.style()
        && styled::combodrop::draw_drop_list(st, c, p, r)
    {
        return;
    }
    c.fill_rect(r, p.cs.window);
    c.ring_t(r, p.cs.button_text, p.cs.button_text, p.s);
}

/// One tab of a sheet's strip. `hot` is a closed tab under the pointer, which only the style draws apart.
#[expect(clippy::too_many_arguments)]
pub fn draw_tab(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    open: bool,
    hot: bool,
    page: Rect,
    px: u32,
) {
    if let Some(st) = p.look.style()
        && styled::tab::draw_tab(st, c, text, p, r, label, open, hot, page, px)
    {
        return;
    }
    classic::widgets::draw_tab(c, text, p, r, label, open, page, px)
}

/// The focus ring round the open tab of a focused strip, `ring` in device px.
/// A style with a tab part XORs its dots with the face, as its push buttons'
/// ring does; Classic inks them solid.
pub fn draw_tab_focus(c: &mut Canvas, p: &Paint, ring: Rect) {
    if let Some(st) = p.look.style()
        && styled::tab::draw_tab_focus(st, c, p, ring)
    {
        return;
    }
    classic::widgets::draw_tab_focus(c, ring, p.s, p.cs.button_text)
}

pub fn draw_page(c: &mut Canvas, p: &Paint, page: Rect) {
    if let Some(st) = p.look.style()
        && st.draw_page(c, page, p.s)
    {
        return;
    }
    classic::widgets::draw_page(c, p, page)
}

pub fn draw_progress(c: &mut Canvas, p: &Paint, r: Rect, fraction: f64, enabled: bool) {
    if let Some(st) = p.look.style()
        && styled::progress::draw_progress(st, c, p, r, fraction, enabled)
    {
        return;
    }
    classic::progress::draw(c, p, r, fraction, enabled)
}

/// The trackbar. The style draws its thumb in its state for `thumb` and
/// `focused` (whether the trackbar holds the keyboard); Classic holds the
/// thumb when `Pressed`. A vertical layout goes to [`draw_vertical_trackbar`].
pub fn draw_trackbar(
    c: &mut Canvas,
    p: &Paint,
    lay: &trackbar::Layout,
    enabled: bool,
    focused: bool,
    thumb: Interaction,
) {
    if lay.vertical {
        return draw_vertical_trackbar(c, p, lay, enabled, focused, thumb);
    }
    if let Some(st) = p.look.style()
        && styled::trackbar::draw_trackbar(st, c, p, lay, enabled, focused, thumb)
    {
        return;
    }
    classic::trackbar::draw_held(c, p, lay, enabled, thumb == Interaction::Pressed)
}

/// A vertical trackbar, from [`trackbar::layout_vertical`]; states as [`draw_trackbar`].
pub fn draw_vertical_trackbar(
    c: &mut Canvas,
    p: &Paint,
    lay: &trackbar::Layout,
    enabled: bool,
    focused: bool,
    thumb: Interaction,
) {
    if let Some(st) = p.look.style()
        && styled::trackbar::draw_vertical_trackbar(st, c, p, lay, enabled, focused, thumb)
    {
        return;
    }
    classic::trackbar::draw_held(c, p, lay, enabled, thumb == Interaction::Pressed)
}

/// A line of static text with its access key underlined while [`Paint::access_keys`], the same
/// in every look.
#[expect(clippy::too_many_arguments)]
pub fn draw_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    ink: Color,
    align: Align,
    px: u32,
) {
    classic::widgets::draw_label(c, text, r, &p.label(label), false, ink, align, px, p.s)
}

/// The dotted focus rectangle on `ring`'s edge, the same in every look.
pub fn draw_focus_ring(c: &mut Canvas, p: &Paint, ring: Rect, ink: Color) {
    classic::widgets::draw_focus_ring(c, ring, p.s, ink)
}

/// A caption tab in `r` over a window's caption band
/// ([`crate::controls::captiontabs`]) for a focused window (`window_active`)
/// or another: the style's caption tabs with titles in `caption_text`, else
/// Classic's with titles in `text` at `px`.
#[expect(clippy::too_many_arguments)]
pub fn draw_caption_tab(
    c: &mut Canvas,
    text: &TextRenderer,
    caption_text: &TextRenderer,
    p: &Paint,
    r: Rect,
    tab: &crate::frame::CaptionTab,
    window_active: bool,
    px: u32,
) {
    if let Some(st) = p.look.style()
        && styled::captiontab::draw_caption_tab(st, c, caption_text, r, tab, window_active, p.s)
    {
        return;
    }
    classic::widgets::draw_caption_tab(c, text, p.cs, r, tab, window_active, px, p.s)
}

/// The size of a caption tab's close box, device px: the style's square grown by `grow` logical
/// px, or Classic's caption button from `m`, which grew with the caption.
pub fn caption_tab_close(p: &Paint, m: &crate::theme::Metrics, grow: i32) -> (i32, i32) {
    match p.look.style().and_then(|st| st.caption_tabs) {
        Some(l) => ((l.close + grow) * p.s, (l.close + grow) * p.s),
        None => (m.caption_button_width, m.caption_button_height),
    }
}

/// A marquee over `ring`: the style's selection box, else the dotted focus rectangle in `ink`.
pub fn draw_marquee(c: &mut Canvas, p: &Paint, ring: Rect, ink: Color) {
    if let Some(st) = p.look.style()
        && styled::selbox::draw_marquee(st, c, ring, p.s)
    {
        return;
    }
    classic::widgets::draw_focus_ring(c, ring, p.s, ink)
}

/// The look's selection box, for a host that lays it out as solid rects;
/// None for a look that draws the dotted marquee and checkered outline.
pub fn selection_box(look: &crate::paint::Look) -> Option<crate::style::SelectionBox> {
    look.style().and_then(|st| st.selection_box)
}

/// The spinner's two arrow buttons; the style draws the button `lit` names in its state.
pub fn draw_spin_buttons(
    c: &mut Canvas,
    p: &Paint,
    up: Rect,
    down: Rect,
    enabled: bool,
    lit: Lit<Arrow>,
) {
    if let Some(st) = p.look.style()
        && styled::spin::draw_spin_buttons(st, c, p, up, down, enabled, lit)
    {
        return;
    }
    classic::widgets::draw_spin_buttons(c, p, up, down, enabled)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaGlyph {
    Previous,
    Play,
    Pause,
    Next,
}

/// A push button with a transport glyph instead of a label. The glyph is 7
/// rows tall, centred: Play a triangle 4 columns wide pointing right; Pause
/// two 2 px bars 2 px apart; Previous a 2 px bar then a left-pointing
/// triangle, Next the reverse. It sits a pixel down and right in a held
/// Classic button. Greyed, it is inked flat in the style's Disabled push
/// button ink if any, else etched: shadow over a hilight copy a pixel down and right.
#[expect(clippy::too_many_arguments)]
pub fn draw_media_button(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    glyph: MediaGlyph,
    enabled: bool,
    ptr: Interaction,
    focus: bool,
    px: u32,
) {
    draw_button(c, text, p, r, "", false, ptr, enabled, focus, px);
    let s = p.s;
    let style = p.look.style();
    let styled = style.is_some_and(|st| st.part(Part::PushButton).is_some());
    let shift = if !styled && ptr == Interaction::Pressed {
        s
    } else {
        0
    };
    let (w, h) = (if glyph == MediaGlyph::Play { 4 } else { 6 }, 7);
    let x = r.x + (r.width - w * s) / 2 + shift;
    let y = r.y + (r.height - h * s) / 2 + shift;
    ink_button_glyph(p, enabled, |d, ink| {
        draw_media_glyph(c, x + d, y + d, s, glyph, ink)
    });
}

/// A push button with `glyph` in a `size` px box centred instead of a label, inked as
/// [`draw_media_button`] inks its glyph. The glyph is drawn from shapes at the canvas's scale.
#[expect(clippy::too_many_arguments)]
pub fn draw_glyph_button(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    (glyph, size): (crate::shape::Glyph, i32),
    enabled: bool,
    ptr: Interaction,
    focus: bool,
    px: u32,
) {
    draw_button(c, text, p, r, "", false, ptr, enabled, focus, px);
    let s = p.s;
    let styled = p
        .look
        .style()
        .is_some_and(|st| st.part(Part::PushButton).is_some());
    let shift = if !styled && ptr == Interaction::Pressed {
        s
    } else {
        0
    };
    let x = r.x + (r.width - size * s) / 2 + shift;
    let y = r.y + (r.height - size * s) / 2 + shift;
    let cs = c.scale();
    ink_button_glyph(p, enabled, |d, ink| {
        let g = crate::shape::glyph(glyph, size, s * cs, 1.0, crate::shape::premul(ink, 255));
        c.blit_over(x + d, y + d, &g.at_scale(cs));
    });
}

/// Calls `ink` with an offset and a colour for each pass of a button glyph: once in the button's
/// text colour; greyed, once in the style's Disabled push button ink if any, else once in grey
/// text under a style, else etched: hilight a pixel down and right, then shadow over it.
fn ink_button_glyph(p: &Paint, enabled: bool, mut ink: impl FnMut(i32, Color)) {
    let style = p.look.style();
    let styled = style.is_some_and(|st| st.part(Part::PushButton).is_some());
    if enabled {
        ink(0, p.cs.button_text);
    } else if let Some(c) = style.and_then(|st| st.text_color(Part::PushButton, State::Disabled)) {
        ink(0, c);
    } else if styled {
        ink(0, p.cs.gray_text);
    } else {
        ink(p.s, p.cs.button_hilight);
        ink(0, p.cs.button_shadow);
    }
}

fn draw_media_glyph(c: &mut Canvas, x: i32, y: i32, s: i32, glyph: MediaGlyph, ink: Color) {
    // A triangle 4 columns wide, 7 rows tall at its base, its tip the middle row.
    let tri = |c: &mut Canvas, x: i32, right: bool| {
        for k in 0..4 {
            let col = if right { k } else { 3 - k };
            c.fill_rect(Rect::new(x + col * s, y + k * s, s, (7 - 2 * k) * s), ink);
        }
    };
    let bar = |c: &mut Canvas, x: i32| c.fill_rect(Rect::new(x, y, 2 * s, 7 * s), ink);
    match glyph {
        MediaGlyph::Play => tri(c, x, true),
        MediaGlyph::Pause => {
            bar(c, x);
            bar(c, x + 4 * s);
        }
        MediaGlyph::Previous => {
            bar(c, x);
            tri(c, x + 2 * s, false);
        }
        MediaGlyph::Next => {
            tri(c, x, true);
            bar(c, x + 4 * s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;

    /// Blue's size grip in a box `r`: face, with six 3x3 dots whose corners step
    /// 4 px apart, the lowest-right ending 1 px in from the right and bottom edges.
    /// Each dot is a 2x2 square of the scheme's grip grey over a white one a
    /// pixel down and right, its other corners face.
    #[track_caller]
    fn assert_blue_grip(c: &Canvas, r: Rect) {
        let face = crate::blue::Variant::Blue
            .scheme()
            .system
            .button_face
            .argb();
        let grey = crate::blue::Variant::Blue
            .scheme()
            .size_box
            .normal
            .inner
            .argb();
        let dot: [[u32; 3]; 3] = [
            [grey, grey, face],
            [grey, grey, 0xFFFF_FFFF],
            [face, 0xFFFF_FFFF, 0xFFFF_FFFF],
        ];
        let (x0, y0) = (r.right() - 12, r.bottom() - 12);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let (dx, dy) = (x - x0, y - y0);
                let on = (0..11).contains(&dx)
                    && (0..11).contains(&dy)
                    && dx % 4 < 3
                    && dy % 4 < 3
                    && dx / 4 + dy / 4 >= 2;
                let want = if on {
                    dot[(dy % 4) as usize][(dx % 4) as usize]
                } else {
                    face
                };
                assert_eq!(c.get(x, y), want, "({x}, {y}) is {want:08X}");
            }
        }
    }

    /// Classic's lone close box in a 28x28 rect is the 16x14 caption button
    /// centred in it, 6 px in from the left and 7 px from the top; the rest keeps what lay under it.
    #[test]
    fn classic_centres_the_close_box_in_its_rect() {
        let theme = crate::theme::Theme::default();
        let p = theme.paint();
        let r = Rect::new(0, 0, 28, 28);
        let mut c = Canvas::new(28, 28);
        draw_close_box(&mut c, &p, r);
        let mut want = Canvas::new(16, 14);
        let whole = want.bounds();
        classic::widgets::draw_caption_button(
            &mut want,
            p.cs,
            whole,
            crate::art::glyph::Glyph::Close,
            false,
            1,
        );
        for y in 0..28 {
            for x in 0..28 {
                let inside = (6..22).contains(&x) && (7..21).contains(&y);
                let px = if inside { want.get(x - 6, y - 7) } else { 0 };
                assert_eq!(c.get(x, y), px, "({x}, {y}) is {px:08X}");
            }
        }
    }

    /// In every look an inactive window's default button draws as a plain
    /// one, and an active window's draws apart from it.
    #[test]
    fn an_inactive_windows_default_button_draws_plain_in_every_look() {
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = theme_for(interface);
            let text = crate::text::TextRenderer::load(&theme.fonts);
            let r = Rect::new(0, 0, 75, 23);
            let button = |active, default| {
                let p = Paint {
                    active,
                    ..theme.paint()
                };
                let mut c = Canvas::new(75, 23);
                let (ptr, px) = (Interaction::Idle, theme.fonts.size_px);
                draw_button(&mut c, &text, &p, r, "OK", default, ptr, true, false, px);
                c
            };
            assert!(
                button(false, true) == button(false, false),
                "{interface:?}: inactive default is plain"
            );
            assert!(
                button(true, true) != button(true, false),
                "{interface:?}: active default is ringed"
            );
        }
    }

    /// Held in Classic, a check box's well turns face under its mark and a vertical trackbar's thumb speckles hilight from 2 px in.
    #[test]
    fn classic_shows_a_held_check_box_and_thumb() {
        let theme = crate::theme::Theme::default();
        let cs = &theme.colors;
        let p = theme.paint();
        let b = Rect::new(2, 2, 13, 13);
        let mut c = Canvas::new(40, 40);
        draw_checkbox(&mut c, &p, b, true, Interaction::Hot);
        assert_eq!(c.get(4, 4), cs.window.argb());
        draw_checkbox(&mut c, &p, b, true, Interaction::Pressed);
        assert_eq!(c.get(4, 4), cs.button_face.argb(), "the held well");
        assert_eq!(c.get(5, 8), cs.button_text.argb(), "the mark stays");

        let tb = crate::trackbar::TrackBar::new(0, 3);
        let lay = crate::trackbar::layout_vertical(&tb, 0, 0, 40);
        let t = lay.thumb;
        let mut c = Canvas::new(40, 60);
        draw_vertical_trackbar(&mut c, &p, &lay, true, false, Interaction::Idle);
        assert_eq!(c.get(t.x + 3, t.y + 2), cs.button_face.argb());
        draw_vertical_trackbar(&mut c, &p, &lay, true, false, Interaction::Pressed);
        assert_eq!(
            c.get(t.x + 3, t.y + 2),
            cs.button_hilight.argb(),
            "speckled"
        );
        assert_eq!(c.get(t.x + 2, t.y + 2), cs.button_face.argb());
    }

    /// Where Notepad's two 17 px bars meet, Blue fills the box plain button face, with the size grip's dots when resizable.
    #[test]
    fn a_blue_scroll_corner_is_plain_face_with_the_grip_only_when_asked() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(3, 3, 17, 17);
        let mut c = Canvas::new(23, 23);
        draw_scroll_corner(&mut c, &theme.paint(), r, false);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                assert_eq!(
                    c.get(x, y),
                    theme.colors.button_face.argb(),
                    "no grip: ({x}, {y})"
                );
            }
        }
        assert_eq!(c.get(2, 2), 0, "nothing outside the box");
        let mut c = Canvas::new(23, 23);
        draw_scroll_corner(&mut c, &theme.paint(), r, true);
        assert_blue_grip(&c, r);
        assert_eq!(c.get(2, 2), 0, "nothing outside the box");
    }

    /// A Blue size grip on its own (the file dialog's corner, a status bar's end)
    /// draws the same dots in its 16 px box; Classic draws its ridges.
    #[test]
    fn a_size_grip_draws_blues_dots_and_classics_ridges() {
        let r = Rect::new(2, 2, 16, 16);
        let mut c = Canvas::new(20, 20);
        draw_size_grip(&mut c, &theme_for(Interface::Blue).paint(), r);
        assert_blue_grip(&c, r);
        let classic = theme_for(Interface::Classic);
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_size_grip(&mut a, &classic.paint(), r);
        classic::widgets::draw_size_grip(&mut b, &classic.colors, r, 1);
        assert!(a == b, "Classic's size grip is its ridges");
    }

    #[test]
    fn a_blue_size_grip_on_a_scale_2_canvas_doubles_every_pixel() {
        let (p, r) = (theme_for(Interface::Blue), Rect::new(0, 0, 16, 16));
        let (mut one, mut two) = (Canvas::new(16, 16), Canvas::with_scale(16, 16, 2));
        draw_size_grip(&mut one, &p.paint(), r);
        draw_size_grip(&mut two, &p.paint(), r);
        assert!(crate::look::styled::fixture::doubles(&one, &two));
    }

    #[test]
    fn a_classic_scroll_corner_draws_the_grip_only_when_asked() {
        let theme = theme_for(Interface::Classic);
        let r = Rect::new(2, 2, 16, 16);
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_scroll_corner(&mut a, &theme.paint(), r, true);
        classic::widgets::draw_size_grip(&mut b, &theme.colors, r, 1);
        assert!(a == b, "a Classic corner with a grip is the size grip");
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_scroll_corner(&mut a, &theme.paint(), r, false);
        b.fill_rect(r, theme.colors.button_face);
        assert!(a == b, "a Classic corner without a grip is plain face");
    }

    #[test]
    fn a_style_with_only_the_tracks_keeps_the_classic_corner() {
        use crate::look::styled::fixture::{one_part_style, solid_frames};
        use crate::style::{Layout, PartImage};
        let img = || {
            PartImage::new(
                solid_frames(&[Color::rgb(10, 0, 0)]),
                &[State::Normal],
                Layout::Vertical,
            )
        };
        let style = one_part_style(Part::ScrollTrackV, img()).with_part(Part::ScrollTrackH, img());
        let look = crate::look::Look::Styled(std::sync::Arc::new(style));
        let cs = crate::theme::ColorScheme::default();
        let p = Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
            access_keys: true,
            text: crate::theme::TextUnits::ONE,
        };
        let r = Rect::new(2, 2, 16, 16);
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_scroll_corner(&mut a, &p, r, true);
        classic::widgets::draw_size_grip(&mut b, &cs, r, 1);
        assert!(
            a == b,
            "a corner beside Classic bars is Classic's size grip"
        );
    }

    /// Classic has no hot look, and no pressed look but the push button's, a
    /// check box's and a trackbar thumb's: every other widget draws the same whatever the pointer does.
    #[test]
    fn classic_draws_the_pointer_only_on_a_held_push_button() {
        let theme = theme_for(Interface::Classic);
        let p = theme.paint();
        let text = crate::text::TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let lay = trackbar::layout(&trackbar::TrackBar::new(3, 10), 4, 4, 80);
        let page = Rect::new(2, 22, 90, 30);
        let draw = |ptr: Interaction| {
            let mut c = Canvas::new(100, 60);
            let b = Rect::new(2, 2, 13, 13);
            draw_checkbox(&mut c, &p, b, true, ptr);
            draw_checkbox_state(&mut c, &p, Rect::new(18, 2, 13, 13), CheckState::Mixed, ptr);
            draw_radio(&mut c, &p, Rect::new(34, 2, 13, 13), false, ptr);
            draw_combo_drop(&mut c, &p, Rect::new(50, 2, 16, 17), true, ptr, false);
            draw_button_face(&mut c, &p, Rect::new(70, 2, 20, 20), true, ptr);
            let hot = ptr != Interaction::Idle;
            draw_tab(
                &mut c,
                &text,
                &p,
                Rect::new(4, 4, 40, 18),
                "Tab",
                false,
                hot,
                page,
                px,
            );
            let mut t = Canvas::new(100, 40);
            draw_trackbar(&mut t, &p, &lay, true, false, ptr);
            (c, t)
        };
        let idle = draw(Interaction::Idle);
        assert!(draw(Interaction::Hot) == idle, "Hot draws as Idle");
        let (held, thumb) = draw(Interaction::Pressed);
        assert!(thumb != idle.1, "a held thumb speckles");
        let boxes = Rect::new(2, 2, 29, 13);
        for y in 0..60 {
            for x in 0..100 {
                if !boxes.contains(x, y) {
                    assert_eq!(held.get(x, y), idle.0.get(x, y), "({x}, {y}) draws as Idle");
                }
            }
        }
        let button = |ptr| {
            let mut c = Canvas::new(80, 30);
            draw_button(
                &mut c,
                &text,
                &p,
                Rect::new(2, 2, 75, 23),
                "OK",
                false,
                ptr,
                true,
                false,
                px,
            );
            c
        };
        assert!(
            button(Interaction::Hot) == button(Interaction::Idle),
            "a hot button is at rest"
        );
        let mut pushed = Canvas::new(80, 30);
        classic::widgets::draw_button(
            &mut pushed,
            &text,
            &p,
            Rect::new(2, 2, 75, 23),
            "OK",
            false,
            true,
            true,
            false,
            px,
        );
        assert!(
            button(Interaction::Pressed) == pushed,
            "a held button is pushed"
        );
    }

    #[test]
    fn a_blue_disabled_arrow_button_still_paints_classic() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(2, 2, 16, 16);
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_arrow_button(&mut a, &theme.paint(), r, Arrow::Down, false);
        classic::widgets::draw_arrow_button(&mut b, &theme.paint(), r, Arrow::Down, false);
        assert!(
            a == b,
            "a Blue disabled arrow button paints Classic, the arrow button in every look"
        );
    }

    #[test]
    fn a_blue_calendar_band_at_paint_scale_2_doubles_every_pixel() {
        let draw = |c: &mut Canvas, p: &Paint| {
            draw_calendar_band(c, p, Rect::new(2, 2, 30, 18).scaled(p.s), false);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(34, 22, draw),
            "a calendar band at s = 2 is its s = 1 pixels doubled"
        );
    }

    #[test]
    fn a_blue_size_grip_at_paint_scale_2_doubles_every_pixel() {
        let draw =
            |c: &mut Canvas, p: &Paint| draw_size_grip(c, p, Rect::new(2, 2, 16, 16).scaled(p.s));
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(20, 20, draw),
            "a size grip at s = 2 is its s = 1 pixels doubled"
        );
    }
}
