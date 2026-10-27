//! Tooltips: the plain yellow tip, the multi-line infotip under a menu row, the
//! balloon tip with its stem and the tray's notification balloon, with the
//! placement rules that keep them on screen. The host decides when to show one
//! (a [`SHOW_MS`] rest, 5 s autopop).

use crate::classic::widgets::{
    BUTTON_GAP, BUTTON_H, BUTTON_W, FIELD_PAD, draw_edit, edit_first_visible,
};
use crate::look::styled;
use crate::look::widgets::{
    ICON_BUTTON_W, draw_button, draw_edit_field, draw_icon_button, tooltip,
};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Insets, Rect, rounded_border, rounded_fill};
use crate::style::{BalloonLook, DropShadow};
use crate::taskbar::Edge;
use crate::text::{Align, TextRenderer, wrap};
use crate::textedit::{Caret, EditKey, Preedit, TextEdit, index_at};
use crate::theme::Theme;

/// The pitch of a tip's lines in device px: [`INFO_LINE_H`], grown as the text outgrows its
/// 100% line ([`Theme::line_growth`]).
pub(crate) fn line_pitch(theme: &Theme) -> i32 {
    (INFO_LINE_H + theme.line_growth()) * theme.scale.max(1)
}

/// Where a box's text goes: how far inside each edge (design px) and the first
/// line's text box height (device px). A look with its own box
/// ([`tooltip::insets`]) sets the insets and boxes every line at [`line_pitch`];
/// Classic's is `pad` inside the 1 px border, the first line `line_h` tall.
fn text_frame(theme: &Theme, pad: i32, line_h: i32) -> (Insets, i32) {
    match tooltip::insets(&theme.paint()) {
        Some((content, _)) => (content, line_pitch(theme)),
        None => (Insets::new(pad + 1, pad + 1, pad + 1, pad + 1), line_h),
    }
}

/// Size of the box holding `lines`, in `theme`'s units: the widest line plus the
/// insets, one text box per line at [`line_pitch`], the first as tall as
/// [`text_frame`] says. The plain tip, the menu's infotip and Explorer's are this
/// box. A look's shadow lies outside it.
pub(crate) fn box_size(
    text: &TextRenderer,
    theme: &Theme,
    lines: &[String],
    pad: i32,
    line_h: i32,
) -> (i32, i32) {
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let (m, line_h) = text_frame(theme, pad, line_h);
    let widest = lines
        .iter()
        .map(|l| text.measure(l, false, size))
        .max()
        .unwrap_or(0);
    let n = lines.len().max(1) as i32;
    (
        widest + (m.left + m.right) * s,
        line_h + (n - 1) * line_pitch(theme) + (m.top + m.bottom) * s,
    )
}

/// Draw that box into a canvas at `canvas_scale`: the look's box
/// ([`tooltip::draw_box`]; Classic's a 1 px `info_text` frame round `info_bk`).
/// Classic's plain tip text box is the font size, so its ink sits a pixel lower
/// than centred (3 above, 1 below). The canvas reaches past the box by the
/// look's shadow, right and down; the box keeps its top-left.
pub(crate) fn draw_box(
    theme: &Theme,
    text: &TextRenderer,
    lines: &[String],
    (pad, line_h): (i32, i32),
    canvas_scale: i32,
) -> Canvas {
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let p = theme.paint();
    let shadow = tooltip::insets(&p).map_or(Insets::ZERO, |(_, o)| o.scaled(s));
    let (w, h) = box_size(text, theme, lines, pad, line_h);
    let (m, line_h) = text_frame(theme, pad, line_h);
    let mut c = Canvas::with_scale(
        (w + shadow.right).max(1) as u32,
        (h + shadow.bottom).max(1) as u32,
        canvas_scale,
    );
    let ink = tooltip::draw_box(&mut c, &p, Rect::new(0, 0, w, h));
    let mut y = m.top * s;
    for line in lines {
        let r = Rect::new(m.left * s, y, w - (m.left + m.right) * s, line_h);
        text.draw(&mut c, r, line, false, ink, Align::Left, size);
        y += line_pitch(theme);
    }
    c
}

/// Padding around the text inside the border, and the gap between the tip's
/// bottom edge and the pointer hotspot (logical px).
const PAD: i32 = 2;
const ABOVE_POINTER: i32 = 8;

/// The rest under the pointer before a host shows a tip, in ms.
pub const SHOW_MS: u32 = 500;

/// A plain tip draws at most `TIP_LINES` lines of `TIP_CHARS` chars, so a peer's
/// text cannot grow its canvas without bound.
pub const TIP_LINES: usize = 8;
pub const TIP_CHARS: usize = 255;

/// The balloon tip's metrics: text 3 px inside the border, the title 10 px down,
/// the body 40 px down, 15 px lines wrapped at `BALLOON_TEXT_W`, a 16 px stem
/// whose apex is 16 px from the left border and 20 px below (2 right of) the
/// hotspot.
const BALLOON_PAD_X: i32 = 3;
const BALLOON_TITLE_Y: i32 = 10;
const BALLOON_BODY_Y: i32 = 40;
const BALLOON_LINE_H: i32 = 15;
const BALLOON_BOTTOM: i32 = 7;
const BALLOON_TEXT_W: i32 = 320;
const BALLOON_RADIUS: i32 = 5;
const STEM_H: i32 = 16;
const STEM_W: i32 = 19;
const STEM_X: i32 = 16;
pub const STEM_BELOW_POINTER: (i32, i32) = (2, 20);

/// Infotips wrap at 300 px and sit 3 px under the row, 57 px into it.
const INFO_TEXT_W: i32 = 296;
const INFO_PAD: i32 = 2;
pub(crate) const INFO_LINE_H: i32 = 13;
pub const INFO_BELOW_ROW: (i32, i32) = (57, 3);
pub const INFO_BELOW_POINTER: i32 = 21;

/// The full title of a task button `button_w` wide, only if the button had to cut
/// it short. Uses `draw_taskbar`'s label box (28 px of icon and padding) and
/// weight (bold when pressed).
pub fn task_tip(
    text: &TextRenderer,
    size_px: u32,
    title: &str,
    bold: bool,
    button_w: i32,
) -> Option<String> {
    let label_w = (button_w - 28).max(0);
    if text.ellipsize(title, label_w, bold, size_px) == title {
        None
    } else {
        Some(title.to_string())
    }
}

/// The lines a plain tip draws for `label`: split on `
/// `, cut to [`TIP_LINES`]
/// lines of [`TIP_CHARS`] chars.
fn tip_lines(label: &str) -> Vec<String> {
    label
        .split('\n')
        .take(TIP_LINES)
        .map(|l| l.chars().take(TIP_CHARS).collect())
        .collect()
}

/// Size of the tip for `label`: the widest line plus padding and the 1 px
/// border, each line after the first adding [`INFO_LINE_H`].
pub fn tip_size(text: &TextRenderer, theme: &Theme, label: &str) -> (i32, i32) {
    box_size(
        text,
        theme,
        &tip_lines(label),
        PAD,
        theme.fonts.size_px as i32,
    )
}

pub fn draw_tip(theme: &Theme, text: &TextRenderer, label: &str) -> Canvas {
    draw_tip_at(theme, text, label, 1)
}

pub fn draw_tip_at(theme: &Theme, text: &TextRenderer, label: &str, canvas_scale: i32) -> Canvas {
    draw_box(
        theme,
        text,
        &tip_lines(label),
        (PAD, theme.fonts.size_px as i32),
        canvas_scale,
    )
}

/// Where a tip of `size` goes for a pointer at `cursor`: its left edge at the
/// pointer, its bottom `ABOVE_POINTER` px above the hotspot (or with `below`,
/// its top `INFO_BELOW_POINTER` px under it); pushed left to stay one pixel clear
/// of `screen`'s right edge, and never off the top or bottom.
pub fn place(cursor: (i32, i32), size: (i32, i32), screen: Rect, below: bool) -> (i32, i32) {
    let (w, h) = size;
    let x = (cursor.0 - 1).min(screen.right() - 1 - w).max(screen.x);
    let y = if below {
        (cursor.1 + INFO_BELOW_POINTER).min(screen.bottom() - h)
    } else {
        cursor.1 - ABOVE_POINTER - h
    };
    (x, y.max(screen.y))
}

/// Where a tip of `size` goes for a pointer at `cursor` on a side bar whose
/// facing edge is at x `edge`: 2 px off it, right of a left bar (`right`) and
/// left of a right one, level with the pointer, kept inside `screen` as [`place`]
/// does.
pub fn place_beside(
    cursor: (i32, i32),
    edge: i32,
    right: bool,
    size: (i32, i32),
    screen: Rect,
) -> (i32, i32) {
    let (w, h) = size;
    let x = if right { edge + 2 } else { edge - 2 - w };
    keep_in(screen, (x, cursor.1 - h / 2), size)
}

/// `at` moved so a box of `size` stays inside `screen`, a pixel clear of its right edge.
fn keep_in(screen: Rect, at: (i32, i32), (w, h): (i32, i32)) -> (i32, i32) {
    (
        at.0.min(screen.right() - 1 - w).max(screen.x),
        at.1.min(screen.bottom() - h).max(screen.y),
    )
}

/// `body` wrapped at `width` design px.
fn wrap_at(text: &TextRenderer, theme: &Theme, body: &str, width: i32) -> Vec<String> {
    let s = theme.scale.max(1);
    wrap(text, body, width * s, theme.fonts.size_px)
}

fn info_lines(text: &TextRenderer, theme: &Theme, body: &str) -> Vec<String> {
    wrap_at(text, theme, body, INFO_TEXT_W)
}

pub fn info_size(text: &TextRenderer, theme: &Theme, body: &str) -> (i32, i32) {
    let line_h = line_pitch(theme);
    box_size(
        text,
        theme,
        &info_lines(text, theme, body),
        INFO_PAD,
        line_h,
    )
}

pub fn draw_info_tip(theme: &Theme, text: &TextRenderer, body: &str) -> Canvas {
    let line_h = line_pitch(theme);
    draw_box(
        theme,
        text,
        &info_lines(text, theme, body),
        (INFO_PAD, line_h),
        1,
    )
}

pub fn place_below(anchor: (i32, i32), size: (i32, i32), screen: Rect) -> (i32, i32) {
    keep_in(screen, anchor, size)
}

fn balloon_lines(text: &TextRenderer, theme: &Theme, body: &str) -> Vec<String> {
    wrap_at(text, theme, body, BALLOON_TEXT_W)
}

/// Size of a balloon tip in `theme`'s units: the stem, then a bubble as wide as
/// its widest line plus padding and border, tall enough for the title, the gap
/// and the body. A look with its own balloon ([`BalloonLook`]) adds its shadow
/// right and below.
pub fn balloon_size(text: &TextRenderer, theme: &Theme, title: &str, body: &str) -> (i32, i32) {
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let lines = balloon_lines(text, theme, body);
    let n = lines.len().max(1) as i32;
    let widest = |title_px: u32, bold: bool| {
        lines
            .iter()
            .map(|l| text.measure(l, false, size))
            .chain([text.measure(title, bold, title_px)])
            .max()
            .unwrap_or(0)
    };
    match balloon_look(theme) {
        Some(b) => {
            let shadow = b.shadow.map_or(0, |_| DropShadow::N as i32);
            let w = widest(b.title_px * s as u32, false) + (2 * b.icon_at.0 + shadow) * s;
            let h = (b.tail.2 + b.body_y + b.bottom + shadow) * s + n * b.line_h * s;
            (w, h)
        }
        None => {
            let w = widest(size, true) + (2 * BALLOON_PAD_X + 2) * s;
            let g = theme.line_growth();
            let h =
                (STEM_H + BALLOON_BODY_Y + g + BALLOON_BOTTOM) * s + n * (BALLOON_LINE_H + g) * s;
            (w, h)
        }
    }
}

/// Draw the balloon tip into a canvas of `balloon_size`: the bubble below the
/// stem (left side vertical, right side sloping down), the title and the body.
/// Classic's bubble is tooltip yellow with a bold title; a look with its own
/// balloon draws as [`BalloonLook`] says, the text from the icon's left edge.
pub fn draw_balloon_tip(theme: &Theme, text: &TextRenderer, title: &str, body: &str) -> Canvas {
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let (w, h) = balloon_size(text, theme, title, body);
    let mut c = Canvas::new(w as u32, h as u32);
    if let Some(b) = balloon_look(theme) {
        let shadow = b.shadow.map_or(0, |_| DropShadow::N as i32 * s);
        let stem = b.tail.2 * s;
        let bubble = Rect::new(0, stem, w - shadow, h - stem - shadow);
        let tail = styled::balloon::Tail {
            x: STEM_X * s,
            right: true,
            down: false,
        };
        styled::balloon::draw_bubble(&b, &mut c, bubble, tail, s);
        let tx = b.icon_at.0 * s;
        let tw = bubble.width - 2 * tx;
        let title_px = b.title_px * s as u32;
        let th = (b.title_px as i32 + 4) * s;
        let title_box = Rect::new(tx, stem + b.title_y * s, tw, th);
        text.draw(
            &mut c,
            title_box,
            title,
            false,
            b.title,
            Align::Left,
            title_px,
        );
        let mut y = stem + b.body_y * s;
        for line in balloon_lines(text, theme, body) {
            let r = Rect::new(tx, y, tw, b.line_h * s);
            text.draw(&mut c, r, &line, false, b.text, Align::Left, size);
            y += b.line_h * s;
        }
        return c;
    }
    let (bg, ink) = (theme.colors.info_bk, theme.colors.info_text);
    let bubble = Rect::new(0, STEM_H * s, w, h - STEM_H * s);
    rounded_fill(&mut c, bubble, BALLOON_RADIUS * s, bg);
    rounded_border(&mut c, bubble, BALLOON_RADIUS * s, ink, s);
    // The stem: row r below the apex spans from the vertical left side to
    // a right edge that reaches STEM_W at the bubble's top border.
    let x0 = STEM_X * s;
    for r in 0..STEM_H * s {
        let width = (STEM_W * s * (r + 1) / (STEM_H * s)).max(s);
        c.fill_rect(Rect::new(x0, r, width, 1), bg);
        c.fill_rect(Rect::new(x0, r, s, 1), ink);
        c.fill_rect(Rect::new(x0 + width - s, r, s, 1), ink);
    }
    // Open the bubble's top border where the stem joins it.
    c.fill_rect(Rect::new(x0 + s, STEM_H * s, STEM_W * s - 2 * s, s), bg);
    let tx = (1 + BALLOON_PAD_X) * s;
    let tw = w - 2 * tx;
    let line_h = (BALLOON_LINE_H + theme.line_growth()) * s;
    text.draw(
        &mut c,
        Rect::new(tx, (STEM_H + BALLOON_TITLE_Y) * s, tw, line_h),
        title,
        true,
        ink,
        Align::Left,
        size,
    );
    let mut y = (STEM_H + BALLOON_BODY_Y + theme.line_growth()) * s;
    for line in balloon_lines(text, theme, body) {
        text.draw(
            &mut c,
            Rect::new(tx, y, tw, line_h),
            &line,
            false,
            ink,
            Align::Left,
            size,
        );
        y += line_h;
    }
    c
}

pub fn place_balloon(cursor: (i32, i32), size: (i32, i32), screen: Rect) -> (i32, i32) {
    let (dx, dy) = STEM_BELOW_POINTER;
    keep_in(screen, (cursor.0 + dx - STEM_X, cursor.1 + dy), size)
}

/// The notification balloon's metrics (logical px): 300 px wide, 10 px
/// padding, an 18x12 tail under the body's right end and 6 px corners.
const NOTIFY_W: i32 = 300;
const NOTIFY_PAD: i32 = 10;
const NOTIFY_TAIL_W: i32 = 18;
const NOTIFY_TAIL_H: i32 = 12;
const NOTIFY_RADIUS: i32 = 6;
/// Body lines the notification balloon shows at most, so a body of bare newlines
/// cannot size the canvas.
pub const NOTIFY_BODY_LINES: usize = 10;
/// Action buttons the balloon shows at most; past them a "..." button holds the
/// rest.
pub const NOTIFY_BUTTONS: usize = 3;
/// The button row: 8 px under the body's last line, each label 8 px
/// inside its button, the "..." button square.
const NOTIFY_ROW_GAP: i32 = 8;
const NOTIFY_LABEL_PAD: i32 = 8;
const NOTIFY_MORE_W: i32 = BUTTON_H;
pub(crate) const NOTIFY_MORE: &str = "...";
/// The reply row's field: as tall as a dialog's edit box (21 px), centred
/// on the row a button's height tall.
const NOTIFY_FIELD_H: i32 = 21;
/// The grey hint's inset from the field's text box, where the caret
/// stands before any text.
const NOTIFY_HINT_X: i32 = 1;

/// The reply row an expanded balloon shows in place of its buttons: a
/// one-line field with a grey `placeholder` while it is empty, and a
/// push button labelled `send` at the row's right end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotifyReply<'a> {
    pub placeholder: &'a str,
    pub send: &'a str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyLayout {
    pub width: i32,
    /// The canvas: the bubble, the tail under (or over) it and the
    /// shadow under it.
    pub height: i32,
    pub bubble_w: i32,
    pub bubble_h: i32,
    /// The bubble's top: 0 with the tail under it, the tail's height with
    /// the tail rising over it ([`NotifyLayout::on_edge`]).
    pub bubble_y: i32,
    /// The close cross's hit box (the look's close box, when it has one).
    pub close: Rect,
    pub buttons: Vec<Rect>,
    /// Each button's label; None shows its action's icon.
    labels: Vec<Option<String>>,
    pub more: Option<Rect>,
    /// The reply row's field and send button, when it is expanded
    /// ([`NotifyReply`]); then there are no other buttons.
    pub field: Option<Rect>,
    pub send: Option<Rect>,
    placeholder: String,
    send_label: String,
    lines: Vec<String>,
    title: String,
    /// The icon's top-left, the title's text box and size, the body's
    /// first line box's top, all from the bubble's top-left.
    icon_at: (i32, i32),
    title_box: Rect,
    title_px: u32,
    body_y: i32,
    line_h: i32,
    text_x: i32,
    text_w: i32,
    tail_h: i32,
    /// How far the look's shadow reaches past the bubble.
    shadow: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotifyHit {
    Close,
    Button(usize),
    More,
    Field,
    Send,
    Body,
}

/// The look's own balloon, its title and lines grown with `theme`'s text size.
fn balloon_look(theme: &Theme) -> Option<BalloonLook> {
    let b = theme.look.style().and_then(|st| st.balloon)?;
    let title = theme.text.px(b.title_px) - b.title_px;
    Some(BalloonLook {
        title_px: b.title_px + title,
        body_y: b.body_y + title as i32,
        line_h: b.line_h + theme.line_growth(),
        ..b
    })
}

/// Lay out the balloon for `title`, `body` and the action labels `actions`
/// (every one but the body's own action). The first [`NOTIFY_BUTTONS`] get a
/// button each, right-aligned under the body; more put a "..." button at the
/// right end. An action `icons` marks shows its icon in an icon button in place
/// of its label. A row too wide shrinks its labelled buttons evenly. With `reply`
/// the row holds the reply field and send button instead, as tall as the button
/// row so expanding the reply leaves the size alone. A look with its own balloon
/// ([`BalloonLook`]) places the icon, text and close box and sizes the tail.
pub fn notify_layout(
    theme: &Theme,
    text: &TextRenderer,
    title: &str,
    body: &str,
    actions: &[&str],
    icons: &[bool],
    reply: Option<NotifyReply>,
) -> NotifyLayout {
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let w = NOTIFY_W * s;
    let pad = NOTIFY_PAD * s;
    let look = balloon_look(theme);
    // The icon, the text's left edge and width, the title's box and
    // size, the body's first line and pitch, the room under the body,
    // the close box, the tail's length and the shadow's reach.
    let (icon_at, text_x, text_w, title_px, line_h, close, tail_h, shadow) = match &look {
        Some(b) => {
            let (cx, cy, cw, ch) = b.close;
            (
                (b.icon_at.0 * s, b.icon_at.1 * s),
                b.text_x * s,
                w - (b.text_x + b.right) * s,
                b.title_px * s as u32,
                b.line_h * s,
                Rect::new(w - cx * s, cy * s, cw * s, ch * s),
                b.tail.2 * s,
                b.shadow.map_or(0, |_| DropShadow::N as i32 * s),
            )
        }
        None => {
            let text_x = pad + theme.icon_size() + pad;
            let (cx, cy) = (w - pad - 8 * s, pad + s);
            (
                (pad, pad),
                text_x,
                w - text_x - pad,
                size,
                size as i32 + 4 * s,
                Rect::new(cx - 2 * s, cy - 2 * s, 11 * s, 11 * s),
                NOTIFY_TAIL_H * s,
                0,
            )
        }
    };
    let mut lines = wrap(text, body, text_w, size);
    lines.truncate(NOTIFY_BODY_LINES);
    let n = lines.len().max(1) as i32;
    // Classic's title leaves 12 px for the 7x7 close cross at the far
    // top-right; a look's ends 4 px short of its close box.
    let (title_box, body_y, text_bottom, end_pad) = match &look {
        Some(b) => {
            let tw = close.x - 4 * s - text_x;
            let tb = Rect::new(text_x, b.title_y * s, tw, (b.title_px as i32 + 4) * s);
            let body_y = b.body_y * s;
            (tb, body_y, body_y + n * line_h, b.bottom * s)
        }
        None => {
            // Padding, the title line and a 4 px gap, the body lines.
            let body_y = pad + line_h + 4 * s;
            let tb = Rect::new(text_x, pad, text_w - 12 * s, line_h);
            (tb, body_y, body_y + n * line_h, pad)
        }
    };
    let title = text
        .ellipsize(title, title_box.width, look.is_none(), title_px)
        .into_owned();

    let actions = if reply.is_some() { &[][..] } else { actions };
    let shown = actions.len().min(NOTIFY_BUTTONS);
    let more_w = if actions.len() > NOTIFY_BUTTONS {
        NOTIFY_MORE_W * s
    } else {
        0
    };
    let gap = BUTTON_GAP * s;
    let iconic = |i: usize| icons.get(i).copied().unwrap_or(false);
    let mut widths: Vec<i32> = actions[..shown]
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if iconic(i) {
                ICON_BUTTON_W * s
            } else {
                (text.measure(a, false, size) + 2 * NOTIFY_LABEL_PAD * s).max(BUTTON_W * s)
            }
        })
        .collect();
    let parts = shown + usize::from(more_w > 0);
    let room = w - 2 * pad - more_w - parts.saturating_sub(1) as i32 * gap;
    shrink_labelled(&mut widths, iconic, room);
    let (mut buttons, mut labels, mut more) = (Vec::new(), Vec::new(), None);
    let (mut field, mut send) = (None, None);
    let bubble_h = if let Some(r) = reply {
        let y = text_bottom + NOTIFY_ROW_GAP * s;
        let label_w = text.measure(r.send, false, size) + 2 * NOTIFY_LABEL_PAD * s;
        let sw = label_w.max(BUTTON_W * s);
        let sx = w - pad - sw;
        send = Some(Rect::new(sx, y, sw, BUTTON_H * s));
        let fh = NOTIFY_FIELD_H * s;
        let fy = y + (BUTTON_H * s - fh) / 2;
        field = Some(Rect::new(pad, fy, sx - gap - pad, fh));
        y + BUTTON_H * s + pad
    } else if parts == 0 {
        text_bottom + end_pad
    } else {
        let y = text_bottom + NOTIFY_ROW_GAP * s;
        let mut x = w - pad;
        if more_w > 0 {
            x -= more_w;
            more = Some(Rect::new(x, y, more_w, BUTTON_H * s));
            x -= gap;
        }
        for (i, (a, bw)) in actions[..shown].iter().zip(&widths).enumerate().rev() {
            x -= bw;
            buttons.push(Rect::new(x, y, *bw, BUTTON_H * s));
            let room = bw - 2 * NOTIFY_LABEL_PAD * s;
            labels.push((!iconic(i)).then(|| text.ellipsize(a, room, false, size).into_owned()));
            x -= gap;
        }
        buttons.reverse();
        labels.reverse();
        y + BUTTON_H * s + pad
    };
    NotifyLayout {
        width: w + shadow,
        height: bubble_h + tail_h.max(shadow),
        bubble_w: w,
        bubble_h,
        bubble_y: 0,
        close,
        buttons,
        labels,
        more,
        field,
        send,
        placeholder: reply.map_or_else(String::new, |r| r.placeholder.to_string()),
        send_label: reply.map_or_else(String::new, |r| r.send.to_string()),
        lines,
        title,
        icon_at,
        title_box,
        title_px,
        body_y,
        line_h,
        text_x,
        text_w,
        tail_h,
        shadow,
    }
}

/// Shares what `room` leaves after the icon buttons evenly among the labelled ones, when the
/// row's `widths` overrun it.
pub(crate) fn shrink_labelled(widths: &mut [i32], iconic: impl Fn(usize) -> bool, room: i32) {
    let labelled = (0..widths.len()).filter(|&i| !iconic(i)).count() as i32;
    if labelled == 0 || widths.iter().sum::<i32>() <= room {
        return;
    }
    let fixed: i32 = (0..widths.len())
        .filter(|&i| iconic(i))
        .map(|i| widths[i])
        .sum();
    for (i, w) in widths.iter_mut().enumerate() {
        if !iconic(i) {
            *w = (room - fixed) / labelled;
        }
    }
}

/// The rects a notification's pointer and access parts use, from its canvas's top-left.
/// `bubble` is the box under the pointer, `body` the wrapped lines' box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyParts {
    pub bubble: Rect,
    pub title: Rect,
    pub body: Rect,
    pub close: Rect,
    pub buttons: Vec<Rect>,
    pub more: Option<Rect>,
    pub field: Option<Rect>,
    pub send: Option<Rect>,
    pub placeholder: String,
    pub send_label: String,
}

impl NotifyParts {
    pub fn offset(&self, dx: i32, dy: i32) -> NotifyParts {
        let m = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.width, r.height);
        NotifyParts {
            bubble: m(self.bubble),
            title: m(self.title),
            body: m(self.body),
            close: m(self.close),
            buttons: self.buttons.iter().copied().map(m).collect(),
            more: self.more.map(m),
            field: self.field.map(m),
            send: self.send.map(m),
            placeholder: self.placeholder.clone(),
            send_label: self.send_label.clone(),
        }
    }
}

impl NotifyLayout {
    pub fn parts(&self) -> NotifyParts {
        let top = self.bubble_y;
        let tb = self.title_box;
        let lines = self.lines.len().max(1) as i32;
        NotifyParts {
            bubble: Rect::new(0, top, self.bubble_w, self.bubble_h),
            title: Rect::new(tb.x, top + tb.y, tb.width, tb.height),
            body: Rect::new(
                self.text_x,
                top + self.body_y,
                self.text_w,
                lines * self.line_h,
            ),
            close: self.close,
            buttons: self.buttons.clone(),
            more: self.more,
            field: self.field,
            send: self.send,
            placeholder: self.placeholder.clone(),
            send_label: self.send_label.clone(),
        }
    }

    /// This balloon for a bar along `edge`: over a Bottom bar, and beside
    /// a side bar, where it sits in a foot corner by the tray, its tail
    /// hangs under the bubble; under a Top bar it rises over the bubble,
    /// pointing up at the tray, and the bubble and all on it sit the
    /// tail's height lower.
    pub fn on_edge(self, edge: Edge) -> NotifyLayout {
        let to = match edge {
            Edge::Bottom | Edge::Left | Edge::Right => 0,
            Edge::Top => self.tail_h,
        };
        let dy = to - self.bubble_y;
        let moved = |r: Rect| Rect::new(r.x, r.y + dy, r.width, r.height);
        let below = if to > 0 {
            self.shadow
        } else {
            self.tail_h.max(self.shadow)
        };
        NotifyLayout {
            bubble_y: to,
            height: to + self.bubble_h + below,
            close: moved(self.close),
            buttons: self.buttons.iter().copied().map(moved).collect(),
            more: self.more.map(moved),
            field: self.field.map(moved),
            send: self.send.map(moved),
            ..self
        }
    }
}

pub fn notify_hit(lay: &NotifyLayout, x: i32, y: i32) -> NotifyHit {
    parts_hit(&lay.parts(), x, y)
}

/// What a notification's parts are under (`x`, `y`): the close box first, the body last.
pub fn parts_hit(p: &NotifyParts, x: i32, y: i32) -> NotifyHit {
    if p.close.contains(x, y) {
        return NotifyHit::Close;
    }
    if let Some(i) = p.buttons.iter().position(|b| b.contains(x, y)) {
        return NotifyHit::Button(i);
    }
    if p.more.is_some_and(|m| m.contains(x, y)) {
        return NotifyHit::More;
    }
    if p.field.is_some_and(|f| f.contains(x, y)) {
        return NotifyHit::Field;
    }
    if p.send.is_some_and(|b| b.contains(x, y)) {
        return NotifyHit::Send;
    }
    NotifyHit::Body
}

/// Draw the notification balloon laid out as `lay`: a rounded bubble with a
/// tail near its right end (under it, or over it after [`NotifyLayout::on_edge`]),
/// `icon`, title, close cross, wrapped body and action buttons, `pushed` held
/// down and `hot` under the pointer. A button laid out for an icon shows its
/// `actions` icon (device px). An expanded reply row draws `reply`'s text
/// with its caret and the input method's preedit, the grey hint while both are
/// empty; send greys with nothing to send. A focused field reports its spot.
#[expect(clippy::too_many_arguments)]
pub fn draw_notify_balloon(
    theme: &Theme,
    text: &TextRenderer,
    lay: &NotifyLayout,
    icon: Option<&Canvas>,
    actions: &[Option<Canvas>],
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let size = theme.fonts.size_px;
    let style = theme.look.style();
    let look = style.and_then(|st| st.balloon);
    let (w, body_h) = (lay.bubble_w, lay.bubble_h);
    let pad = NOTIFY_PAD * s;
    let mut c = Canvas::new(lay.width as u32, lay.height as u32);
    let top = lay.bubble_y;
    let bubble = Rect::new(0, top, w, body_h);
    let (title_ink, ink) = match (&look, style) {
        (Some(b), Some(st)) => {
            // The tail's straight side stands in from the bubble's right.
            let tail = styled::balloon::Tail {
                x: w - s - b.tail.0 * s,
                right: false,
                down: top == 0,
            };
            styled::balloon::draw_bubble(b, &mut c, bubble, tail, s);
            let lit = hot == Some(NotifyHit::Close);
            styled::balloon::draw_close(st, b, &mut c, lay.close, lit, s);
            (b.title, b.text)
        }
        _ => {
            let (bg, ink) = (cs.info_bk, cs.info_text);
            rounded_fill(&mut c, bubble, NOTIFY_RADIUS * s, bg);
            rounded_border(&mut c, bubble, NOTIFY_RADIUS * s, ink, s);
            // The tail: row r away from the body narrows from
            // NOTIFY_TAIL_W to a point under (or, rising, over) its left
            // edge, outlined in ink.
            let (x0, tail_w, tail_h) = (
                w - pad - NOTIFY_TAIL_W * s,
                NOTIFY_TAIL_W * s,
                NOTIFY_TAIL_H * s,
            );
            for r in 0..tail_h {
                let width = (tail_w * (tail_h - r) / tail_h).max(1);
                let y = if top > 0 { top - 1 - r } else { body_h + r };
                c.fill_rect(Rect::new(x0, y, width, 1), bg);
                c.fill_rect(Rect::new(x0, y, s, 1), ink);
                c.fill_rect(Rect::new(x0 + width - s, y, s, 1), ink);
            }
            // Open the body's border where the tail joins it.
            let seam = if top > 0 { top } else { body_h - s };
            c.fill_rect(Rect::new(x0 + s, seam, tail_w - 2 * s, s), bg);
            let (cx, cy) = (w - pad - 8 * s, top + pad + s);
            for i in 0..7 {
                c.fill_rect(Rect::new(cx + i * s, cy + i * s, s, s), ink);
                c.fill_rect(Rect::new(cx + (6 - i) * s, cy + i * s, s, s), ink);
            }
            (ink, ink)
        }
    };

    if let Some(ic) = icon {
        c.blit_canvas(lay.icon_at.0, top + lay.icon_at.1, ic);
    }
    let tb = lay.title_box;
    text.draw(
        &mut c,
        Rect::new(tb.x, top + tb.y, tb.width, tb.height),
        &lay.title,
        look.is_none(),
        title_ink,
        Align::Left,
        lay.title_px,
    );
    let mut y = top + lay.body_y;
    for line in &lay.lines {
        text.draw(
            &mut c,
            Rect::new(lay.text_x, y, lay.text_w, lay.line_h),
            line,
            false,
            ink,
            Align::Left,
            size,
        );
        y += lay.line_h;
    }
    let none = TextEdit::default();
    let (edit, caret, preedit) = reply.unwrap_or((&none, Caret::Unfocused, None));
    c.set_preedit(preedit.cloned());
    if let Some(f) = lay.field {
        let ptr = if hot == Some(NotifyHit::Field) {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        let inner = draw_edit_field(&mut c, p, f, cs.window, caret.focused(), ptr);
        let composing = preedit.is_some() && caret.focused();
        if edit.text.is_empty() && !composing && !lay.placeholder.is_empty() {
            let hint = Rect::new(
                inner.x + NOTIFY_HINT_X * s,
                inner.y,
                inner.width,
                inner.height,
            );
            let shown = text.ellipsize(&lay.placeholder, hint.width, false, size);
            text.draw(&mut c, hint, &shown, false, cs.gray_text, Align::Left, size);
        }
        let first = edit_first_visible(text, edit, inner.width, size, 0);
        draw_edit(&mut c, text, edit, inner, size, p, caret, first);
    }
    let ptr = |hit: NotifyHit, enabled: bool| {
        if !enabled {
            Interaction::Idle
        } else if pushed == Some(hit) {
            Interaction::Pressed
        } else if hot == Some(hit) {
            Interaction::Hot
        } else {
            Interaction::Idle
        }
    };
    // A peer's label is plain text: `&` is not an access key marker.
    let button = |c: &mut Canvas, r: Rect, label: &str, hit: NotifyHit, enabled: bool| {
        let label = crate::text::escape_mnemonic(label);
        let ptr = ptr(hit, enabled);
        draw_button(c, text, p, r, &label, false, ptr, enabled, false, size);
    };
    for (i, (r, label)) in lay.buttons.iter().zip(&lay.labels).enumerate() {
        let hit = NotifyHit::Button(i);
        match (label, actions.get(i).and_then(Option::as_ref)) {
            (Some(label), _) => button(&mut c, *r, label, hit, true),
            (None, Some(icon)) => {
                draw_icon_button(&mut c, text, p, *r, icon, true, ptr(hit, true), false, size)
            }
            (None, None) => button(&mut c, *r, "", hit, true),
        }
    }
    if let Some(r) = lay.more {
        button(&mut c, r, NOTIFY_MORE, NotifyHit::More, true);
    }
    if let Some(r) = lay.send {
        let room = r.width - 2 * NOTIFY_LABEL_PAD * s;
        let label = text.ellipsize(&lay.send_label, room, false, size);
        button(
            &mut c,
            r,
            &label,
            NotifyHit::Send,
            !edit.text.trim().is_empty(),
        );
    }
    c
}

/// An expanded reply row's state: what is typed and whether the field has the
/// keyboard. The balloon gets it only from a click ([`ReplyField::click`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplyField {
    pub edit: TextEdit,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyKey {
    Enter,
    Escape,
    Edit(EditKey),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplyEvent {
    None,
    Redraw,
    Send(String),
    Collapse,
}

impl ReplyField {
    /// The reply Send would send: the text without its outer blanks,
    /// None while that is empty.
    pub fn to_send(&self) -> Option<String> {
        let t = self.edit.text.trim();
        (!t.is_empty()).then(|| t.to_string())
    }

    /// Enter sends (nothing with nothing typed), Escape collapses, and
    /// an editing key edits.
    pub fn key(&mut self, key: ReplyKey) -> ReplyEvent {
        match key {
            ReplyKey::Enter => self.to_send().map_or(ReplyEvent::None, ReplyEvent::Send),
            ReplyKey::Escape => ReplyEvent::Collapse,
            ReplyKey::Edit(e) => {
                self.edit.key(e);
                ReplyEvent::Redraw
            }
        }
    }

    /// Text from the clipboard: its first line goes in at the caret.
    pub fn paste(&mut self, text: &str) -> bool {
        self.edit.paste(text, &[])
    }

    /// A press on the field of `lay` at device x `x`: the field takes the
    /// keyboard and the caret goes to the nearest character boundary.
    pub fn click(&mut self, theme: &Theme, text: &TextRenderer, lay: &NotifyLayout, x: i32) {
        self.click_in(theme, text, lay.field, x);
    }

    /// [`ReplyField::click`] on a `field` laid out by any notification.
    pub fn click_in(&mut self, theme: &Theme, text: &TextRenderer, field: Option<Rect>, x: i32) {
        let s = theme.scale.max(1);
        let inner = field.map(|f| {
            Rect::new(
                f.x + FIELD_PAD * s,
                f.y,
                f.width - 2 * FIELD_PAD * s,
                f.height,
            )
        });
        self.click_text(text, inner, theme.fonts.size_px, x);
    }

    /// A right press ahead of the edit menu: `press` places the caret as a left press would
    /// ([`ReplyField::click_in`], `sheet_click`), and a press inside the selection keeps it.
    pub fn press_for_menu(&mut self, press: impl FnOnce(&mut ReplyField)) {
        let (sel, caret) = (self.edit.selection(), self.edit.caret());
        let anchor = if caret == sel.start {
            sel.end
        } else {
            sel.start
        };
        press(self);
        let at = self.edit.caret();
        self.edit.set_selection(anchor, caret);
        self.edit.press_for_menu(at);
    }

    /// A press at device x `x` on a field whose text box is `inner`, its text at `px`.
    pub fn click_text(&mut self, text: &TextRenderer, inner: Option<Rect>, px: u32, x: i32) {
        self.focused = true;
        let Some(inner) = inner else {
            return;
        };
        let first = edit_first_visible(text, &self.edit, inner.width, px, 0);
        let at = first + index_at(text, &self.edit.text[first..], x - inner.x, px);
        self.edit.set_selection(at, at);
    }
}

/// Where a notification balloon laid out as `lay` at `scale` goes: in the
/// corner of `usable` (the output less the taskbar) nearest the tray, 8 px
/// in from its side and 2 px from its foot or top: bottom-right over a
/// Bottom bar and beside a Right one, bottom-left beside a Left bar,
/// top-right under a Top bar. Kept inside it. Its footprint is the
/// layout over `scale`.
pub fn place_notify(usable: Rect, lay: &NotifyLayout, scale: i32, edge: Edge) -> (i32, i32) {
    let s = scale.max(1);
    let (w, h) = (lay.width / s, lay.height / s);
    let (x, y) = match edge {
        Edge::Bottom | Edge::Right => (usable.right() - w - 8, usable.bottom() - h - 2),
        Edge::Top => (usable.right() - w - 8, usable.y + 2),
        Edge::Left => (usable.x + 8, usable.bottom() - h - 2),
    };
    (x.min(usable.right() - w).max(usable.x), y.max(usable.y))
}

/// What a balloon says to a screen reader, in full where the drawing wraps or cuts it. `opens`:
/// a click on the body runs something; `urgent`: the alert interrupts.
#[derive(Clone, Copy, Debug)]
pub struct NotifySays<'a> {
    pub app: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: &'a [&'a str],
    pub opens: bool,
    pub urgent: bool,
}

impl NotifySays<'_> {
    /// The alert's name, announced when it appears or changes: "App: title. body".
    pub fn spoken(&self) -> String {
        let text: Vec<&str> = [self.title, self.body]
            .into_iter()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        let text = text.join(". ");
        match (self.app.is_empty(), text.is_empty()) {
            (true, _) => text,
            (false, true) => self.app.to_string(),
            (false, false) => format!("{}: {text}", self.app),
        }
    }
}

const NOTIFY_MORE_NAME: &str = "More actions";

/// The balloon's `access` part for `lay` at scale 1: a live alert, announced when added under a
/// new `key` or renamed. Returns the reply field while it has the keyboard, else the alert.
#[allow(clippy::too_many_arguments)]
pub fn access_notify(
    key: impl std::hash::Hash,
    lay: &NotifyLayout,
    says: &NotifySays,
    reply: Option<&ReplyField>,
    text: &TextRenderer,
    px: u32,
    tree: &mut crate::access::Tree,
    parent: crate::access::accesskit::NodeId,
) -> crate::access::accesskit::NodeId {
    access_parts(key, &lay.parts(), says, reply, text, px, tree, parent)
}

/// [`access_notify`] over any notification's parts `p`.
#[allow(clippy::too_many_arguments)]
pub fn access_parts(
    key: impl std::hash::Hash,
    p: &NotifyParts,
    says: &NotifySays,
    reply: Option<&ReplyField>,
    text: &TextRenderer,
    px: u32,
    tree: &mut crate::access::Tree,
    parent: crate::access::accesskit::NodeId,
) -> crate::access::accesskit::NodeId {
    use crate::access::{self, accesskit::Action, accesskit::Live, accesskit::Role};
    // Peer text is plain: `&` is no access key marker.
    let plain = crate::text::escape_mnemonic;
    let mut alert = access::node(Role::Alert, p.bubble);
    alert.set_label(says.spoken());
    alert.set_live(if says.urgent {
        Live::Assertive
    } else {
        Live::Polite
    });
    let id = tree.push(parent, ("alert", key), alert);
    tree.push(
        id,
        "title",
        access::label(p.title, &plain(says.title), true),
    );
    if !says.body.is_empty() {
        let mut body = access::label(p.body, &plain(says.body), true);
        if says.opens {
            body.add_action(Action::Click);
        }
        tree.push(id, "body", body);
    }
    for (i, &r) in p.buttons.iter().enumerate() {
        let name = plain(says.actions.get(i).copied().unwrap_or_default());
        tree.push(id, ("button", i), access::button(r, &name, true, false));
    }
    if let Some(r) = p.more {
        tree.push(id, "more", access::button(r, NOTIFY_MORE_NAME, true, false));
    }
    let mut focus = id;
    if let (Some(f), Some(field)) = (p.field, reply) {
        let first = edit_first_visible(text, &field.edit, f.width - 2 * FIELD_PAD, px, 0);
        let fid = crate::classic::widgets::access_edit(
            &field.edit,
            first,
            false,
            f,
            true,
            text,
            px,
            tree,
            id,
            "reply",
        );
        if let Some(n) = tree.get_mut(fid)
            && !p.placeholder.is_empty()
        {
            n.set_placeholder(p.placeholder.as_str());
        }
        if field.focused {
            focus = fid;
        }
    }
    if let Some(r) = p.send {
        let can = reply.is_some_and(|f| f.to_send().is_some());
        let name = plain(&p.send_label);
        tree.push(id, "send", access::button(r, &name, can, false));
    }
    tree.push(id, "close", access::button(p.close, "Close", true, false));
    // Liveness is inherited: the parts would each be announced too.
    let parts = tree
        .get(id)
        .map(|n| n.children().to_vec())
        .unwrap_or_default();
    for k in parts {
        if let Some(n) = tree.get_mut(k) {
            n.set_live(Live::Off);
        }
    }
    focus
}

#[cfg(test)]
mod tests;
