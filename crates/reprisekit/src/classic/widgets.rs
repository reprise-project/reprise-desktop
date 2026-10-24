//! The shared Classic controls; each draws into the (already scaled) rect it is given.

use crate::access::{self, Tree, accesskit};
use crate::art::glyph::{Glyph, draw_glyph_fitted};
use crate::paint::Paint;
use crate::pointer::Lit;
pub use crate::pointer::{Axis, ScrollHit};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::text::split_mnemonic;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, TextEdit};
use crate::theme::{Color, ColorScheme};
use accesskit::{NodeId, Role};

pub const BUTTON_W: i32 = 75;
pub const BUTTON_H: i32 = 23;
pub const BUTTON_GAP: i32 = 6;
pub const CLOSE_W: i32 = 16;
pub const CLOSE_H: i32 = 14;
pub const DROP_W: i32 = 16;
pub const LINE_H: i32 = 13;
pub const ROW_H: i32 = 21;
pub const FIELD_PAD: i32 = 3;

/// Draw a label with its access key underlined (`&File` underlines F).
#[expect(clippy::too_many_arguments)]
pub fn draw_label(
    c: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    label: &str,
    bold: bool,
    color: Color,
    align: Align,
    px: u32,
    s: i32,
) {
    let (plain, mnemonic) = split_mnemonic(label);
    let Some(m) = mnemonic else {
        text.draw(c, r, &plain, bold, color, align, px);
        return;
    };
    let Some(baseline) = text.baseline(r, bold, px) else {
        return;
    };
    let (dx, w, total) = text.span_and_measure(&plain, m, bold, px);
    let start = align.pen(r, total);
    text.draw_run(c, start, baseline, r, &plain, bold, color, px);
    let ul = Rect::new(start + dx, baseline + s, w, s);
    c.fill_rect(ul.intersect(&r), color);
}

pub fn draw_disabled_label(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    align: Align,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let shifted = Rect::new(r.x + s, r.y + s, r.width, r.height);
    draw_label(
        c,
        text,
        shifted,
        label,
        false,
        cs.button_hilight,
        align,
        px,
        s,
    );
    draw_label(c, text, r, label, false, cs.button_shadow, align, px, s);
}

pub fn draw_focus_ring(c: &mut Canvas, ring: Rect, s: i32, ink: Color) {
    let s = s.max(1);
    for x in (ring.x..ring.right()).step_by(2 * s as usize) {
        c.fill_rect(Rect::new(x, ring.y, s, s), ink);
        c.fill_rect(Rect::new(x, ring.bottom() - s, s, s), ink);
    }
    for y in (ring.y..ring.bottom()).step_by(2 * s as usize) {
        c.fill_rect(Rect::new(ring.x, y, s, s), ink);
        c.fill_rect(Rect::new(ring.right() - s, y, s, s), ink);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RingInk {
    Solid(Color),
    /// Dots are XORed with this colour so the ring reads on any background; a dot is flipped once
    /// even where a thin ring meets itself.
    Xor(Color),
}

impl From<Color> for RingInk {
    fn from(c: Color) -> RingInk {
        RingInk::Solid(c)
    }
}

/// Screen-aligned focus ring: a dot wherever screen x + y is odd, so rings on different canvases
/// share one pattern. `origin` is the canvas's top-left in logical screen px.
pub fn draw_focus_ring_screen(
    c: &mut Canvas,
    ring: Rect,
    s: i32,
    ink: impl Into<RingInk>,
    origin: (i32, i32),
) {
    let s = s.max(1);
    let (ox, oy) = origin;
    let mut dots: Vec<(i32, i32)> = Vec::new();
    for x in (ring.x..ring.right()).step_by(s as usize) {
        dots.extend([(x, ring.y), (x, ring.bottom() - s)]);
    }
    for y in (ring.y + s..ring.bottom() - s).step_by(s as usize) {
        dots.extend([(ring.x, y), (ring.right() - s, y)]);
    }
    dots.retain(|&(x, y)| (ox + x / s + oy + y / s) % 2 != 0);
    match ink.into() {
        RingInk::Solid(ink) => {
            for (x, y) in dots {
                c.fill_rect(Rect::new(x, y, s, s), ink);
            }
        }
        RingInk::Xor(ink) => {
            dots.sort_unstable();
            dots.dedup();
            let mask = ink.argb() & 0x00FF_FFFF;
            for (x, y) in dots {
                for dy in 0..s {
                    for dx in 0..s {
                        let v = c.get(x + dx, y + dy);
                        c.put_raw(x + dx, y + dy, v ^ mask);
                    }
                }
            }
        }
    }
}

/// Origin for [`draw_focus_ring_screen`] that counts the pattern from `ring`'s own top-left.
pub fn ring_origin(ring: Rect, s: i32) -> (i32, i32) {
    let s = s.max(1);
    (-ring.x.div_euclid(s), -ring.y.div_euclid(s))
}

/// Focus ring round an open tab, `ring` in device px: dots where x + y is odd from the ring's
/// top-left.
pub fn draw_tab_focus(c: &mut Canvas, ring: Rect, s: i32, ink: Color) {
    draw_focus_ring_screen(c, ring, s, ink, ring_origin(ring, s));
}

/// A push button: raised bevel, centred label with access key (etched when disabled), black default
/// ring, dotted focus rect 4 px inside.
/// `pushed` is the pointer holding it down: the bevel becomes a frame ring and the label moves a
/// pixel down and right.
#[expect(clippy::too_many_arguments)]
pub fn draw_button(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    default: bool,
    pushed: bool,
    enabled: bool,
    focus: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.button_face);
    let face = if default {
        c.ring_t(r, cs.button_text, cs.button_text, s);
        r.inset(s)
    } else {
        r
    };
    // The label is drawn in `content`, not `face`; the pushed frame goes on the button's own edge
    // so the default ring and its outer ring are the same pixel.
    let content = if pushed {
        c.ring_t(r, cs.window_frame, cs.window_frame, s);
        c.ring_t(r.inset(s), cs.button_shadow, cs.button_shadow, s);
        Rect::new(r.x + s, r.y + s, r.width, r.height)
    } else {
        c.edge_t(face, EdgeStyle::ButtonRaised, cs, s);
        face
    };
    if enabled {
        draw_label(
            c,
            text,
            content,
            label,
            false,
            cs.button_text,
            Align::Center,
            px,
            s,
        );
    } else {
        draw_disabled_label(c, text, p, content, label, Align::Center, px);
    }
    if focus {
        draw_focus_ring_screen(
            c,
            r.inset(4 * s),
            s,
            cs.button_text,
            (-(r.x / s), -(r.y / s)),
        );
    }
}

/// The sunken 2 px frame around a field or list; `r` is in device pixels, the caller fills the
/// inside.
pub fn draw_field_frame(c: &mut Canvas, p: &Paint, r: Rect, _fill: Color) {
    c.edge_t(r, EdgeStyle::ButtonSunken, p.cs, p.s);
}

/// The sunken white box of a text field or list; returns the inner rect for the text. `fill` paints
/// what a thinner frame shows.
pub fn draw_sunken_field(c: &mut Canvas, p: &Paint, r: Rect, fill: Color) -> Rect {
    c.fill_rect(r, p.cs.window);
    draw_field_frame(c, p, r, fill);
    r.inset(FIELD_PAD * p.s)
}

pub use crate::textedit::edit_first_visible;

/// Draw `edit` left-aligned in `inner` from byte `first` (`edit_first_visible`): selection,
/// blinking caret and, when focused, the canvas' preedit with its `TextSpot`. A secret field shows
/// no preedit.
#[expect(clippy::too_many_arguments)]
pub fn draw_edit(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    caret: Caret,
    first: usize,
) {
    let Paint { cs, s, .. } = *p;
    let own = edit;
    // With a preedit the text is drawn with it spliced in, selection gone (text-input-v3 removes
    // it).
    let preedit = c
        .preedit()
        .filter(|_| caret.focused() && !own.is_secret())
        .cloned();
    let composed;
    let (edit, first, run, bar_on) = match &preedit {
        Some(p) => {
            let (e, run, bar_on) = p.compose(own);
            composed = e;
            let at = composed.selection().start;
            let from = edit_first_visible(text, &composed, inner.width, px, first.min(at));
            (&composed, from, Some(run), bar_on)
        }
        None => (own, first, None, true),
    };
    let at = |i: usize| {
        if i < first {
            inner.x
        } else {
            inner.x + text.measure(&edit.text[first..i], false, px)
        }
    };
    if edit.text.is_empty() && !edit.cue().is_empty() {
        text.draw(c, inner, edit.cue(), false, cs.gray_text, Align::Left, px);
    } else {
        text.draw(
            c,
            inner,
            &edit.text[first..],
            false,
            cs.button_text,
            Align::Left,
            px,
        );
    }
    let line = text
        .line(inner, false, px)
        .map_or(inner, |(y, h)| Rect::new(inner.x, y, inner.width, h))
        .intersect(&inner);
    let sel = edit.selection();
    let sel = sel.start.max(first)..sel.end.max(first);
    if !sel.is_empty() {
        let (x0, x1) = (at(sel.start), at(sel.end).min(inner.right()));
        let run = Rect::new(x0, line.y, x1 - x0, line.height);
        c.fill_rect(run, cs.hilight);
        text.draw(
            c,
            Rect::new(x0, inner.y, inner.right() - x0, inner.height),
            &edit.text[sel.clone()],
            false,
            cs.hilight_text,
            Align::Left,
            px,
        );
    }
    if let Some(run) = run {
        let base = text
            .baseline(inner, false, px)
            .unwrap_or(inner.bottom() - 3 * s);
        let (x0, x1) = (at(run.start.max(first)), at(run.end.max(first)));
        let ul = Rect::new(x0, base + s, x1 - x0, s).intersect(&inner);
        c.fill_rect(ul, cs.button_text);
    }
    let cx = at(edit.caret()).min(inner.right() - s);
    let bar = Rect::new(cx, line.y, s, line.height);
    if caret.shown() && bar_on {
        c.fill_rect(bar, cs.button_text);
    }
    if caret.focused() {
        c.report_spot(own.spot(inner, bar));
    }
}

/// Where a drag over a single-line field would drop its text: a bar the caret's size at byte
/// `at`, over what [`draw_edit`] drew from `first`.
#[expect(clippy::too_many_arguments)]
pub fn draw_edit_drop_point(
    c: &mut Canvas,
    text: &TextRenderer,
    edit: &TextEdit,
    inner: Rect,
    px: u32,
    p: &Paint,
    first: usize,
    at: usize,
) {
    let s = p.s;
    let first = first.min(edit.text.len());
    let at = edit.text.floor_char_boundary(at.max(first));
    let line = text
        .line(inner, false, px)
        .map_or(inner, |(y, h)| Rect::new(inner.x, y, inner.width, h))
        .intersect(&inner);
    let x = (inner.x + text.measure(&edit.text[first..at], false, px)).min(inner.right() - s);
    c.fill_rect(
        Rect::new(x, line.y, s, line.height).intersect(&inner),
        p.cs.button_text,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Up,
    Down,
    Left,
    Right,
}

pub fn draw_arrow_button(c: &mut Canvas, p: &Paint, r: Rect, arrow: Arrow, enabled: bool) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.button_face);
    c.edge_t(r, EdgeStyle::WindowRaised, cs, s);
    let draw = |c: &mut Canvas, r: Rect, ink: Color| match arrow {
        Arrow::Up => draw_arrow_v(c, r, s, false, ink),
        Arrow::Down => draw_arrow_v(c, r, s, true, ink),
        Arrow::Left => draw_arrow_h(c, r, s, false, ink),
        Arrow::Right => draw_arrow_h(c, r, s, true, ink),
    };
    if enabled {
        draw(c, r, cs.button_text);
    } else {
        draw(
            c,
            Rect::new(r.x + s, r.y + s, r.width, r.height),
            cs.button_hilight,
        );
        draw(c, r, cs.button_shadow);
    }
}

/// A spinner's two raised arrow buttons at their logical rects.
pub fn draw_spin_buttons(c: &mut Canvas, p: &Paint, up: Rect, down: Rect, enabled: bool) {
    let Paint { cs, s, .. } = *p;
    for (b, is_up) in [(up, true), (down, false)] {
        let r = b.scaled(s);
        c.fill_rect(r, cs.button_face);
        c.edge_t(r, EdgeStyle::WindowRaised, cs, s);
        let cx = (b.x + (b.width - 1) / 2) * s;
        let (tip, base) = if is_up {
            let tip = (b.y + (b.height - 1) / 2) * s;
            (tip, tip + s)
        } else {
            let base = (b.y + (b.height - 2) / 2) * s;
            (base + s, base)
        };
        let arrow = |c: &mut Canvas, d: i32, ink| {
            c.fill_rect(Rect::new(cx + d, tip + d, s, s), ink);
            c.fill_rect(Rect::new(cx - s + d, base + d, 3 * s, s), ink);
        };
        if enabled {
            arrow(c, 0, cs.button_text);
        } else {
            arrow(c, s, cs.button_hilight);
            arrow(c, 0, cs.button_shadow);
        }
    }
}

pub fn draw_check(c: &mut Canvas, b: Rect, s: i32, ink: Color) {
    let (x0, y0) = (b.x + 3 * s, b.y + 3 * s);
    for i in 0..7 {
        let dy = if i < 3 { 2 + i } else { 4 - (i - 2) };
        c.fill_rect(Rect::new(x0 + i * s, y0 + dy * s, s, 3 * s), ink);
    }
}

pub fn draw_list_check(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(b, cs.window);
    c.ring_t(b, cs.window_text, cs.window_text, 2 * s);
    if checked {
        draw_check(c, b, s, cs.window_text);
    }
}

/// What a check box shows; `Mixed` is the checkerboard state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    #[default]
    Off,
    On,
    Mixed,
}

impl CheckState {
    /// What a click leaves behind: a two-state box flips, a `tristate` one cycles indeterminate,
    /// off, on.
    pub fn cycled(self, tristate: bool) -> Self {
        match (self, tristate) {
            (CheckState::Mixed, _) => CheckState::Off,
            (CheckState::Off, _) => CheckState::On,
            (CheckState::On, true) => CheckState::Mixed,
            (CheckState::On, false) => CheckState::Off,
        }
    }

    pub fn is_on(self) -> bool {
        self == CheckState::On
    }
}

pub fn draw_checkbox(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    let state = if checked {
        CheckState::On
    } else {
        CheckState::Off
    };
    draw_checkbox_state(c, p, b, state);
}

pub fn draw_checkbox_disabled(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(b, cs.button_face);
    c.edge_t(b, EdgeStyle::ButtonSunken, cs, s);
    if checked {
        draw_check(c, b, s, cs.button_shadow);
    }
}

pub fn draw_checkbox_state(c: &mut Canvas, p: &Paint, b: Rect, state: CheckState) {
    draw_checkbox_held(c, p, b, state, false)
}

pub fn draw_checkbox_held(c: &mut Canvas, p: &Paint, b: Rect, state: CheckState, held: bool) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(b, if held { cs.button_face } else { cs.window });
    if state == CheckState::Mixed {
        let k = s.max(1);
        for row in 0..(b.height + k - 1) / k {
            for col in 0..(b.width + k - 1) / k {
                if (row + col) % 2 != 0 {
                    let cell = Rect::new(b.x + col * k, b.y + row * k, k, k);
                    c.fill_rect(cell.intersect(&b), cs.button_face);
                }
            }
        }
    }
    c.edge_t(b, EdgeStyle::ButtonSunken, cs, s);
    match state {
        CheckState::Off => {}
        CheckState::On => draw_check(c, b, s, cs.button_text),
        CheckState::Mixed => draw_check(c, b, s, cs.button_shadow),
    }
}

const RADIO: i32 = 12;

/// The rim, a row of roles per pixel: `s` shadow, `d` dkshadow, `h` hilight, `f` face, `w` window,
/// `.` nothing.
const RADIO_RIM: [&str; RADIO as usize] = [
    "....ssss....",
    "..ssddddss..",
    ".sddwwwwddh.",
    ".sdwwwwwwfh.",
    "sdwwwwwwwwfh",
    "sdwwwwwwwwfh",
    "sdwwwwwwwwfh",
    "sdwwwwwwwwfh",
    ".sdwwwwwwfh.",
    ".sffwwwwffh.",
    "..hhffffhh..",
    "....hhhh....",
];

const RADIO_DOT: [(i32, i32, i32); 4] = [(4, 5, 2), (5, 4, 4), (6, 4, 4), (7, 5, 2)];

pub fn caption_gradient(
    c: &mut Canvas,
    caption: Rect,
    buttons: i32,
    left: Color,
    right: Color,
    s: i32,
) {
    c.fill_rect(caption, right);
    let ramp = caption_ramp(caption.width, buttons, s);
    c.hgradient_stepped(
        Rect::new(caption.x, caption.y, ramp, caption.height),
        left,
        right,
    );
}

pub fn caption_colors(cs: &ColorScheme, active: bool) -> (Color, Color, Color) {
    if active {
        (cs.active_title, cs.gradient_active_title, cs.title_text)
    } else {
        (
            cs.inactive_title,
            cs.gradient_inactive_title,
            cs.inactive_title_text,
        )
    }
}

/// A caption tab over the caption band: icon, title (bold when selected, cut with "...") and close
/// box; selected, hover and drop-target states.
#[expect(clippy::too_many_arguments)]
pub fn draw_caption_tab(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    r: Rect,
    tab: &crate::frame::CaptionTab,
    window_active: bool,
    px: u32,
    s: i32,
) {
    use crate::pointer::Interaction;
    let (_, _, ink) = caption_colors(cs, window_active);
    if tab.ghost {
        draw_focus_ring(c, r, s, ink);
    } else if tab.selected {
        c.ring_t(r, cs.button_shadow, cs.button_hilight, s);
    } else if tab.ptr != Interaction::Idle {
        c.ring_t(r, cs.button_hilight, cs.button_shadow, s);
    } else if tab.divider {
        let (top, h) = (r.y + s, r.height - 2 * s);
        c.fill_rect(Rect::new(r.right() - 2 * s, top, s, h), cs.button_shadow);
        c.fill_rect(Rect::new(r.right() - s, top, s, h), cs.button_hilight);
    }
    let mut x = r.x + 2 * s;
    if let Some(icon) = tab.icon {
        let (_, ih) = icon.device_size();
        c.blit_canvas(x, r.y + (r.height - ih as i32) / 2, icon);
        x += icon.device_size().0 as i32 + 3 * s;
    }
    let end = tab.close.map_or(r.right() - 3 * s, |(b, _)| b.x - 2 * s);
    let bold = tab.selected || tab.ghost;
    let room = Rect::new(x, r.y, (end - x).max(0), r.height);
    let title = text.ellipsize(tab.title, room.width, bold, px);
    text.draw(c, room, &title, bold, ink, Align::Left, px);
    if let Some((b, ptr)) = tab.close {
        draw_caption_button(c, cs, b, Glyph::Close, ptr == Interaction::Pressed, s);
    }
}

pub fn draw_caption_button(
    c: &mut Canvas,
    cs: &ColorScheme,
    r: Rect,
    glyph: Glyph,
    pressed: bool,
    s: i32,
) {
    c.fill_rect(r, cs.button_face);
    let edge = if pressed {
        EdgeStyle::ButtonSunken
    } else {
        EdgeStyle::ButtonRaised
    };
    c.edge_t(r, edge, cs, s);
    let o = if pressed { s } else { 0 };
    let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
    draw_glyph_fitted(c, at, s, glyph, cs.button_text, cs.button_face);
}

pub fn caption_ramp(width: i32, buttons: i32, s: i32) -> i32 {
    (width - (buttons * CLOSE_W + 4) * s).max(1)
}

pub const CAPTION_RAMP_START: i32 = 18;

pub fn window_caption_ramp(caption: Rect, first_button_x: i32, s: i32) -> Rect {
    let x = caption.x + CAPTION_RAMP_START * s;
    Rect::new(
        x,
        caption.y,
        (first_button_x - 2 * s - x).max(1),
        caption.height,
    )
}

pub fn draw_radio(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    let Paint { cs, s, .. } = *p;
    radio(c, cs, b, checked, s, true);
}

fn radio(c: &mut Canvas, cs: &ColorScheme, b: Rect, checked: bool, s: i32, enabled: bool) {
    c.paint_map(b.x, b.y, s, &RADIO_RIM, |role| match role {
        's' => Some(cs.button_shadow),
        'd' => Some(cs.button_dk_shadow),
        'h' => Some(cs.button_hilight),
        'f' => Some(cs.button_face),
        'w' if enabled => Some(cs.window),
        'w' => Some(cs.button_face),
        _ => None,
    });
    if checked {
        let dot = if enabled {
            cs.button_text
        } else {
            cs.button_shadow
        };
        for (y, x, w) in RADIO_DOT {
            c.fill_rect(Rect::new(b.x + x * s, b.y + y * s, w * s, s), dot);
        }
    }
}

pub fn draw_radio_disabled(c: &mut Canvas, p: &Paint, b: Rect, checked: bool) {
    let Paint { cs, s, .. } = *p;
    radio(c, cs, b, checked, s, false);
}

pub fn draw_scrollbar(
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    top: usize,
    visible: usize,
    total: usize,
) {
    scrollbar(c, p, sb, top, visible, total, false);
}

pub fn draw_hscrollbar(
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    left: usize,
    visible: usize,
    total: usize,
) {
    scrollbar(c, p, sb, left, visible, total, true);
}

/// Scrollbar pieces for drawing and hit testing, in the space `scrollbar_parts` was given (drawn
/// bar to draw, logical bar with `s = 1` to hit test).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollParts {
    pub back: Rect,
    pub forward: Rect,
    pub track: Rect,
    /// `None` when everything fits or the arrows leave no track.
    pub thumb: Option<Rect>,
}

/// Where a scrollbar's parts land; arrows are as long as the bar is thick (half the bar if shorter
/// than two), the thumb is sized and placed by the share of `total` in view (at least `thumb_min`
/// logical px). A track shorter than `thumb_min` is all thumb.
pub fn scrollbar_parts(
    sb: Rect,
    s: i32,
    pos: usize,
    visible: usize,
    total: usize,
    horizontal: bool,
    thumb_min: i32,
) -> ScrollParts {
    let arrow = if horizontal {
        sb.height.min(sb.width / 2)
    } else {
        sb.width.min(sb.height / 2)
    };
    let (back, forward, track) = if horizontal {
        (
            Rect::new(sb.x, sb.y, arrow, sb.height),
            Rect::new(sb.right() - arrow, sb.y, arrow, sb.height),
            Rect::new(sb.x + arrow, sb.y, (sb.width - 2 * arrow).max(0), sb.height),
        )
    } else {
        (
            Rect::new(sb.x, sb.y, sb.width, arrow),
            Rect::new(sb.x, sb.bottom() - arrow, sb.width, arrow),
            Rect::new(sb.x, sb.y + arrow, sb.width, (sb.height - 2 * arrow).max(0)),
        )
    };
    let len = if horizontal {
        track.width
    } else {
        track.height
    };
    let thumb = (total > visible && len > 0).then(|| {
        // Sized in logical px then scaled, so a draw at `s` is the `s = 1` one magnified.
        let s = s.max(1);
        let l1 = (len / s).max(1);
        let n = total.max(1) as i64;
        let round = |a: i64, b: i64| ((a + b / 2) / b) as i32;
        let th1 = round(l1 as i64 * visible as i64, n).max(thumb_min).min(l1);
        let range = (n - visible as i64).max(1);
        let at = s * round((l1 - th1) as i64 * pos.min(range as usize) as i64, range);
        let th = if th1 == l1 { len } else { th1 * s };
        if horizontal {
            Rect::new(track.x + at, sb.y, th, sb.height)
        } else {
            Rect::new(sb.x, track.y + at, sb.width, th)
        }
    });
    ScrollParts {
        back,
        forward,
        track,
        thumb,
    }
}

pub fn axis_lit(bars: Lit<(Axis, ScrollHit)>, axis: Axis) -> Lit<ScrollHit> {
    bars.and_then(|((a, hit), how)| (a == axis).then_some((hit, how)))
}

pub fn scrollbar_hit(p: &ScrollParts, x: i32, y: i32, horizontal: bool) -> Option<ScrollHit> {
    if p.back.contains(x, y) {
        return Some(ScrollHit::Back);
    }
    if p.forward.contains(x, y) {
        return Some(ScrollHit::Forward);
    }
    if !p.track.contains(x, y) {
        return None;
    }
    let thumb = p.thumb?;
    if thumb.contains(x, y) {
        return Some(ScrollHit::Thumb);
    }
    let (at, start) = if horizontal {
        (x, thumb.x)
    } else {
        (y, thumb.y)
    };
    Some(if at < start {
        ScrollHit::PageBack
    } else {
        ScrollHit::PageForward
    })
}

/// A point already on a scrollbar: the part [`scrollbar_hit`] names, the thumb with its grab
/// offset, or the track of a bar with no thumb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHit {
    Back,
    Forward,
    PageBack,
    PageForward,
    Thumb(i32),
    Track,
}

impl BarHit {
    pub fn part(self) -> Option<ScrollHit> {
        match self {
            BarHit::Back => Some(ScrollHit::Back),
            BarHit::Forward => Some(ScrollHit::Forward),
            BarHit::PageBack => Some(ScrollHit::PageBack),
            BarHit::PageForward => Some(ScrollHit::PageForward),
            BarHit::Thumb(_) => Some(ScrollHit::Thumb),
            BarHit::Track => None,
        }
    }
}

pub fn bar_hit(p: &ScrollParts, x: i32, y: i32, horizontal: bool) -> BarHit {
    match scrollbar_hit(p, x, y, horizontal) {
        Some(ScrollHit::Back) => BarHit::Back,
        Some(ScrollHit::Forward) => BarHit::Forward,
        Some(ScrollHit::PageBack) => BarHit::PageBack,
        Some(ScrollHit::PageForward) => BarHit::PageForward,
        Some(ScrollHit::Thumb) => {
            let t = p.thumb.unwrap_or_default();
            BarHit::Thumb(if horizontal { x - t.x } else { y - t.y })
        }
        None => BarHit::Track,
    }
}

/// The scroll position that puts the thumb's leading edge at `at`; inverse of `scrollbar_parts`'
/// placement.
pub fn scrollbar_pos_at(
    p: &ScrollParts,
    at: i32,
    visible: usize,
    total: usize,
    horizontal: bool,
) -> usize {
    let Some(thumb) = p.thumb else {
        return 0;
    };
    let (len, th, start) = if horizontal {
        (p.track.width, thumb.width, p.track.x)
    } else {
        (p.track.height, thumb.height, p.track.y)
    };
    let span = (len - th).max(0) as i64;
    if span == 0 {
        return 0;
    }
    let range = (total as i64 - visible as i64).max(0);
    let off = (at - start).clamp(0, span as i32) as i64;
    ((off * range + span / 2) / span) as usize
}

fn scrollbar(
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    pos: usize,
    visible: usize,
    total: usize,
    horizontal: bool,
) {
    let Paint { cs, s, .. } = *p;
    let thumb_min = p.look.controls().thumb_min;
    let parts = scrollbar_parts(sb, s, pos, visible, total, horizontal, thumb_min);
    let enabled = total > visible;
    c.dither_rect_t(sb, cs.button_face, cs.button_hilight, s);
    let (back, forward) = if horizontal {
        (Arrow::Left, Arrow::Right)
    } else {
        (Arrow::Up, Arrow::Down)
    };
    draw_arrow_button(c, p, parts.back, back, enabled);
    draw_arrow_button(c, p, parts.forward, forward, enabled);
    if let Some(thumb) = parts.thumb {
        c.fill_rect(thumb, cs.button_face);
        c.edge_t(thumb, EdgeStyle::WindowRaised, cs, s);
    }
}

pub fn draw_arrow_v(c: &mut Canvas, r: Rect, s: i32, down: bool, color: Color) {
    let x0 = r.x + (r.width - 7 * s) / 2;
    let y0 = r.y + (r.height - 4 * s) / 2;
    for i in 0..4 {
        let w = 7 - 2 * i;
        let dy = if down { i } else { 3 - i };
        c.fill_rect(Rect::new(x0 + i * s, y0 + dy * s, w * s, s), color);
    }
}

pub fn draw_small_arrow(c: &mut Canvas, x0: i32, y0: i32, s: i32, color: Color) {
    for i in 0..3 {
        let w = 5 - 2 * i;
        c.fill_rect(Rect::new(x0 + i * s, y0 + i * s, w * s, s), color);
    }
}

pub fn draw_arrow_h(c: &mut Canvas, r: Rect, s: i32, right: bool, color: Color) {
    let x0 = r.x + (r.width - 4 * s) / 2 - if right { 0 } else { s };
    let y0 = r.y + (r.height - 7 * s) / 2;
    for i in 0..4 {
        let h = 7 - 2 * i;
        let dx = if right { i } else { 3 - i };
        c.fill_rect(Rect::new(x0 + dx * s, y0 + i * s, s, h * s), color);
    }
}

pub fn placeholder(w: i32, h: i32) -> Canvas {
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.fill_rect(c.bounds(), Color::rgb(0, 0, 0));
    c.fill_rect(Rect::new(1, 1, w - 2, h - 2), Color::rgb(0xFF, 0x00, 0xFF));
    c
}

pub fn draw_size_grip(c: &mut Canvas, cs: &ColorScheme, r: Rect, s: i32) {
    c.fill_rect(r, cs.button_face);
    let n = r.width / s;
    for y in 0..n {
        for x in 0..n {
            let d = x + y;
            if d < n + 3 {
                continue;
            }
            let ink = match d % 4 {
                3 => cs.button_hilight,
                0 | 1 => cs.button_shadow,
                _ => continue,
            };
            c.fill_rect(Rect::new(r.x + x * s, r.y + y * s, s, s), ink);
        }
    }
}

/// The title's room in a dialog caption without an icon, `buttons` caption buttons at its right.
pub fn caption_title_rect(caption: Rect, buttons: i32, s: i32) -> Rect {
    let ramp = caption_ramp(caption.width, buttons, s);
    Rect::new(caption.x + 2 * s, caption.y, ramp - 2 * s, caption.height)
}

#[expect(clippy::too_many_arguments)]
pub fn draw_caption(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    caption: Rect,
    title: &str,
    active: bool,
    buttons: &[(Rect, Glyph)],
    px: u32,
    s: i32,
) {
    let (left, right, ink) = caption_colors(cs, active);
    let n = buttons.len() as i32;
    caption_gradient(c, caption, n, left, right, s);
    text.draw(
        c,
        caption_title_rect(caption, n, s),
        title,
        true,
        ink,
        Align::Left,
        px,
    );
    for &(r, glyph) in buttons {
        draw_caption_button(c, cs, r, glyph, false, s);
    }
}

pub fn draw_page(c: &mut Canvas, p: &Paint, page: Rect) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(page, cs.button_face);
    c.fill_rect(Rect::new(page.x, page.y, page.width, s), cs.button_hilight);
    c.fill_rect(Rect::new(page.x, page.y, s, page.height), cs.button_hilight);
    c.fill_rect(
        Rect::new(page.right() - 2 * s, page.y + s, s, page.height - s),
        cs.button_shadow,
    );
    c.fill_rect(
        Rect::new(page.right() - s, page.y, s, page.height),
        cs.button_dk_shadow,
    );
    c.fill_rect(
        Rect::new(page.x + s, page.bottom() - 2 * s, page.width - 2 * s, s),
        cs.button_shadow,
    );
    c.fill_rect(
        Rect::new(page.x, page.bottom() - s, page.width, s),
        cs.button_dk_shadow,
    );
}

#[expect(clippy::too_many_arguments)]
pub fn draw_tab(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    open: bool,
    page: Rect,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r, cs.button_face);
    c.fill_rect(
        Rect::new(r.x, r.y + 2 * s, s, r.height - 2 * s),
        cs.button_hilight,
    );
    c.fill_rect(
        Rect::new(r.x + 2 * s, r.y, r.width - 4 * s, s),
        cs.button_hilight,
    );
    c.fill_rect(Rect::new(r.x + s, r.y + s, s, s), cs.button_hilight);
    c.fill_rect(
        Rect::new(r.right() - 2 * s, r.y + s, s, s),
        cs.button_dk_shadow,
    );
    c.fill_rect(
        Rect::new(r.right() - 2 * s, r.y + 2 * s, s, r.height - 2 * s),
        cs.button_shadow,
    );
    c.fill_rect(
        Rect::new(r.right() - s, r.y + 2 * s, s, r.height - 2 * s),
        cs.button_dk_shadow,
    );
    if open {
        c.fill_rect(
            Rect::new(r.x + s, page.y, r.width - 3 * s, s),
            cs.button_face,
        );
    }
    text.draw(
        c,
        tab_label_rect(r, open, s),
        label,
        false,
        cs.button_text,
        Align::Center,
        px,
    );
}

/// The band a tab's label is centred in, down from its top edge.
pub fn tab_label_rect(r: Rect, open: bool, s: i32) -> Rect {
    let h = if open { r.height - s } else { r.height + s };
    Rect::new(r.x, r.y + s, r.width, h)
}

/// A document tab's close cross, 8x7 with 2 px strokes, centred in `b`.
pub fn draw_doc_cross(c: &mut Canvas, b: Rect, s: i32, ink: Color) {
    let (x0, y0) = (b.x + (b.width - 8 * s) / 2, b.y + (b.height - 7 * s) / 2);
    for i in 0..7 {
        c.fill_rect(Rect::new(x0 + i * s, y0 + i * s, 2 * s, s), ink);
        c.fill_rect(Rect::new(x0 + (6 - i) * s, y0 + i * s, 2 * s, s), ink);
    }
}

/// The new-tab plus, 8x8 with 2 px arms, centred in `b`.
pub fn draw_doc_plus(c: &mut Canvas, b: Rect, s: i32, ink: Color) {
    let (x0, y0) = (b.x + (b.width - 8 * s) / 2, b.y + (b.height - 8 * s) / 2);
    c.fill_rect(Rect::new(x0, y0 + 3 * s, 8 * s, 2 * s), ink);
    c.fill_rect(Rect::new(x0 + 3 * s, y0, 2 * s, 8 * s), ink);
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
    let Paint { cs, s, .. } = *p;
    c.etched_frame_t(g, cs, s);
    if label.is_empty() {
        return;
    }
    // The top line breaks for the label measured with its "&" still in it, so the break is 7 px
    // wider than the text when the label has a mnemonic.
    let gap = text.measure(label, false, px) + 4 * s;
    p.fill_back(c, Rect::new(label_r.x - 2 * s, g.y, gap, 2 * s));
    draw_label(
        c,
        text,
        label_r,
        &p.label(label),
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
}

/// A single-line edit's `access` part; `first` is the scroll offset the last draw kept. A masked
/// field passes `secret`, so a reader hears bullets, never the value.
#[allow(clippy::too_many_arguments)]
pub fn access_edit(
    edit: &TextEdit,
    first: usize,
    secret: bool,
    r: Rect,
    enabled: bool,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let role = if secret {
        Role::PasswordInput
    } else {
        Role::TextInput
    };
    let mut n = access::node(role, r);
    access::focusable(&mut n, enabled);
    if !edit.cue().is_empty() {
        n.set_placeholder(edit.cue());
    }
    let id = tree.push(parent, key, n);
    let inner = r.inset(FIELD_PAD);
    let first = if edit.text.is_char_boundary(first.min(edit.text.len())) {
        first.min(edit.text.len())
    } else {
        0
    };
    let x = inner.x - text.measure(&edit.text[..first], false, px);
    let w = text.measure(&edit.text, false, px);
    let line = access::Line {
        text: &edit.text,
        start: 0,
        rect: Rect::new(x, inner.y, w, inner.height),
        hard_break: false,
    };
    access::text_runs(tree, id, &[line], text, px, edit.anchor(), edit.caret());
    id
}

#[cfg(test)]
mod tests;
