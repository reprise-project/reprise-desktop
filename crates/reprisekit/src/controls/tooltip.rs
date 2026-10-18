//! Tooltips: the plain yellow tip, the multi-line infotip under a menu row, the
//! balloon tip with its stem and the tray's notification balloon, with the
//! placement rules that keep them on screen. The host decides when to show one
//! (a [`SHOW_MS`] rest, 5 s autopop).

use crate::classic::widgets::{
    BUTTON_GAP, BUTTON_H, BUTTON_W, FIELD_PAD, draw_edit, edit_first_visible,
};
use crate::look::styled;
use crate::look::widgets::{draw_button, draw_edit_field, tooltip};
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
    draw_box(
        theme,
        text,
        &tip_lines(label),
        (PAD, theme.fonts.size_px as i32),
        1,
    )
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
    let y = cursor.1 - h / 2;
    (
        x.min(screen.right() - 1 - w).max(screen.x),
        y.min(screen.bottom() - h).max(screen.y),
    )
}

fn info_lines(text: &TextRenderer, theme: &Theme, body: &str) -> Vec<String> {
    let s = theme.scale.max(1);
    wrap(text, body, INFO_TEXT_W * s, theme.fonts.size_px)
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
    let (w, h) = size;
    let x = anchor.0.min(screen.right() - 1 - w).max(screen.x);
    let y = anchor.1.min(screen.bottom() - h).max(screen.y);
    (x, y)
}

fn balloon_lines(text: &TextRenderer, theme: &Theme, body: &str) -> Vec<String> {
    let s = theme.scale.max(1);
    wrap(text, body, BALLOON_TEXT_W * s, theme.fonts.size_px)
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
    let (w, h) = size;
    let (dx, dy) = STEM_BELOW_POINTER;
    let x = (cursor.0 + dx - STEM_X)
        .min(screen.right() - 1 - w)
        .max(screen.x);
    let y = (cursor.1 + dy).min(screen.bottom() - h).max(screen.y);
    (x, y)
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
    labels: Vec<String>,
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
/// right end. A row too wide shrinks its labelled buttons evenly. With `reply`
/// the row holds the reply field and send button instead, as tall as the button
/// row so expanding the reply leaves the size alone. A look with its own balloon
/// ([`BalloonLook`]) places the icon, text and close box and sizes the tail.
pub fn notify_layout(
    theme: &Theme,
    text: &TextRenderer,
    title: &str,
    body: &str,
    actions: &[&str],
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
    let mut widths: Vec<i32> = actions[..shown]
        .iter()
        .map(|a| (text.measure(a, false, size) + 2 * NOTIFY_LABEL_PAD * s).max(BUTTON_W * s))
        .collect();
    let parts = shown + usize::from(more_w > 0);
    let room = w - 2 * pad - more_w - parts.saturating_sub(1) as i32 * gap;
    if shown > 0 && widths.iter().sum::<i32>() > room {
        widths.fill(room / shown as i32);
    }
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
        for (a, bw) in actions[..shown].iter().zip(&widths).rev() {
            x -= bw;
            buttons.push(Rect::new(x, y, *bw, BUTTON_H * s));
            let room = bw - 2 * NOTIFY_LABEL_PAD * s;
            labels.push(text.ellipsize(a, room, false, size).into_owned());
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
/// down and `hot` under the pointer. An expanded reply row draws `reply`'s text
/// with its caret and the input method's preedit, the grey hint while both are
/// empty; send greys with nothing to send. A focused field reports its spot.
pub fn draw_notify_balloon(
    theme: &Theme,
    text: &TextRenderer,
    lay: &NotifyLayout,
    icon: Option<&Canvas>,
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
            let close = Rect::new(lay.close.x, lay.close.y, lay.close.width, lay.close.height);
            let lit = hot == Some(NotifyHit::Close);
            styled::balloon::draw_close(st, b, &mut c, close, lit, s);
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
    // A peer's label is plain text: `&` is not an access key marker.
    let mut button = |r: Rect, label: &str, hit: NotifyHit, enabled: bool| {
        let label = crate::text::escape_mnemonic(label);
        let ptr = if !enabled {
            Interaction::Idle
        } else if pushed == Some(hit) {
            Interaction::Pressed
        } else if hot == Some(hit) {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        draw_button(&mut c, text, p, r, &label, false, ptr, enabled, false, size);
    };
    for (i, (r, label)) in lay.buttons.iter().zip(&lay.labels).enumerate() {
        button(*r, label, NotifyHit::Button(i), true);
    }
    if let Some(r) = lay.more {
        button(r, NOTIFY_MORE, NotifyHit::More, true);
    }
    if let Some(r) = lay.send {
        let room = r.width - 2 * NOTIFY_LABEL_PAD * s;
        let label = text.ellipsize(&lay.send_label, room, false, size);
        button(r, &label, NotifyHit::Send, !edit.text.trim().is_empty());
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
mod tests {
    use crate::theme::Color;
    const TIP_BG: Color = Color::rgb(255, 255, 225);
    const TIP_INK: Color = Color::rgb(0, 0, 0);

    use super::*;
    use crate::theme::FontConfig;

    #[test]
    #[ignore]
    fn render_tooltip() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_tip(&theme, &text, "Click here to begin");
        crate::raster::write_preview(&c, "tooltip-preview.png");
        let c = draw_balloon_tip(
            &theme,
            &text,
            "Turn Off",
            "Shuts down Reprise so that you can safely turn off the computer.",
        );
        crate::raster::write_preview(&c, "balloon-tip-preview.png");
        let reply = NotifyReply {
            placeholder: "Write a reply",
            send: "Send",
        };
        let lay = notify_layout(
            &theme,
            &text,
            "Ana",
            "Lunch on Friday?",
            &["Reply"],
            Some(reply),
        );
        let typed = TextEdit::new("Sounds good", false);
        let c = draw_notify_balloon(
            &theme,
            &text,
            &lay,
            None,
            None,
            None,
            Some((&typed, Caret::Shown, None)),
        );
        crate::raster::write_preview(&c, "notify-reply-preview.png");
    }

    #[test]
    fn balloon_tip_has_a_stem_a_bubble_and_wraps_long_bodies() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let body = "Shuts down the desktop so that you can safely turn off the computer.";
        let (w, h) = balloon_size(&text, &theme, "Turn Off", body);
        assert!(w <= BALLOON_TEXT_W + 2 * BALLOON_PAD_X + 2);
        assert_eq!(
            h,
            STEM_H + BALLOON_BODY_Y + 2 * BALLOON_LINE_H + BALLOON_BOTTOM
        );
        let (_, one) = balloon_size(&text, &theme, "Log Off", "Closes your programs.");
        assert_eq!(one, h - BALLOON_LINE_H);
        let long = format!("{body} {body} {body}");
        let (lw, lh) = balloon_size(&text, &theme, "Turn Off", &long);
        assert!(
            lw <= BALLOON_TEXT_W + 2 * BALLOON_PAD_X + 2,
            "wrapped, not widened"
        );
        assert!(lh >= h + 2 * BALLOON_LINE_H);
        let c = draw_balloon_tip(&theme, &text, "Turn Off", body);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        // The apex: ink at (STEM_X, 0), clear left of it and clear above
        // the bubble elsewhere; the stem's right side reaches STEM_W at the
        // bubble.
        assert_eq!(c.get(STEM_X, 0), TIP_INK.argb());
        assert_eq!(c.get(STEM_X - 1, 0), 0);
        assert_eq!(c.get(STEM_X + 5, 4), 0);
        assert_eq!(c.get(STEM_X + STEM_W - 1, STEM_H - 1), TIP_INK.argb());
        assert_eq!(c.get(STEM_X + 4, STEM_H - 1), TIP_BG.argb());
        // The join is open; the bubble's border runs left of the stem.
        assert_eq!(c.get(STEM_X + 4, STEM_H), TIP_BG.argb());
        assert_eq!(c.get(STEM_X - 4, STEM_H), TIP_INK.argb());
        // Bubble corners are rounded: the corner pixel is clear, the middle
        // of the left border is ink, the inside is yellow.
        assert_eq!(c.get(0, STEM_H), 0);
        assert_eq!(c.get(0, STEM_H + 20), TIP_INK.argb());
        assert_eq!(c.get(w - 1, h - 1), 0);
        assert_eq!(c.get(w / 2, h - 3), TIP_BG.argb());
    }

    #[test]
    fn info_tip_wraps_at_300_px_and_sits_where_asked() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let short = "Contains music and other audio files.";
        let (w, h) = info_size(&text, &theme, short);
        assert_eq!(h, INFO_LINE_H + 2 * INFO_PAD + 2);
        assert!(w < INFO_TEXT_W);
        let long = "Opens the settings that change how the desktop looks and behaves, which \
                    programs start with it, and how it connects to networks, printers and \
                    the other people who share it.";
        let (lw, lh) = info_size(&text, &theme, long);
        assert!(lw <= INFO_TEXT_W + 2 * INFO_PAD + 2);
        assert_eq!(
            lh,
            3 * INFO_LINE_H + 2 * INFO_PAD + 2,
            "this one wraps onto three lines"
        );
        let c = draw_info_tip(&theme, &text, short);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        assert_eq!(c.get(0, 0), TIP_INK.argb());
        assert_eq!(c.get(1, 1), TIP_BG.argb());
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(place_below((279, 903), (291, 27), screen), (279, 903));
        assert_eq!(
            place_below((1900, 1070), (291, 27), screen),
            (1919 - 291, 1080 - 27)
        );
    }

    #[test]
    fn balloon_hangs_its_apex_below_the_pointer_and_stays_on_screen() {
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(place_balloon((1017, 392), (289, 63), screen), (1003, 412));
        let (x, y) = place_balloon((1900, 1070), (289, 63), screen);
        assert_eq!((x + 289, y + 63), (1919, 1080));
        assert_eq!(place_balloon((5, 5), (289, 63), screen), (0, 25));
    }

    #[test]
    fn tip_is_text_plus_padding_inside_a_one_pixel_border() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let label = "Click here to begin";
        let (w, h) = tip_size(&text, &theme, label);
        assert_eq!(w, text.measure(label, false, 11) + 6);
        assert_eq!(h, 17);
        let c = draw_tip(&theme, &text, label);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        assert_eq!(c.get(0, 0), TIP_INK.argb());
        assert_eq!(c.get(w - 1, h - 1), TIP_INK.argb());
        assert_eq!(c.get(1, 1), TIP_BG.argb());
        assert_eq!(c.get(w - 2, h - 2), TIP_BG.argb());
        // Ink somewhere in the middle row.
        assert!((1..w - 1).any(|x| c.get(x, h / 2) == TIP_INK.argb()));
    }

    #[test]
    fn scaled_tip_doubles_border_and_padding() {
        let theme = Theme::default().scaled(2);
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw_tip(&theme, &text, "Hi");
        assert_eq!(c.height as i32, 22 + 12);
        assert_eq!(c.get(1, 1), TIP_INK.argb());
        assert_eq!(c.get(2, 2), TIP_BG.argb());
    }

    #[test]
    fn a_multi_line_tip_stacks_its_lines() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let label = "Local Area Connection 2\nSpeed: 100.0 Mbps\nStatus: Connected";
        let (w, h) = tip_size(&text, &theme, label);
        // A one-line tip's own height (17 px) plus the 13 px line pitch
        // for each further line.
        assert_eq!(h, 17 + 2 * INFO_LINE_H);
        let c = draw_tip(&theme, &text, label);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        // Line 2 begins exactly one pitch below line 1's own ink row (found
        // rather than assumed, since a font's ascent nudges it down from the
        // padding edge), and line 3 one pitch below that; the row right
        // above each is blank.
        let has_ink = |y: i32| (1..w - 1).any(|x| c.get(x, y) == TIP_INK.argb());
        let line1 = (1..h - 1).find(|&y| has_ink(y)).expect("line 1 has ink");
        assert!(
            !has_ink(line1 + INFO_LINE_H - 1),
            "gap above line 2 is blank"
        );
        assert!(
            has_ink(line1 + INFO_LINE_H),
            "line 2 starts at the line pitch"
        );
        assert!(
            has_ink(line1 + 2 * INFO_LINE_H),
            "line 3 starts two pitches down"
        );
    }

    #[test]
    fn a_grown_tip_pitches_its_lines_with_the_text() {
        let text = TextRenderer::load(&FontConfig::default());
        let label = "Local Area Connection\nStatus: Connected";
        let base = Theme::default();
        let grown = base.with_text_size(200);
        let g = grown.line_growth();
        assert!(g > 0, "a 200% line outgrows the 100% one");
        let (_, h100) = tip_size(&text, &base, label);
        let (w, h) = tip_size(&text, &grown, label);
        let px = (grown.fonts.size_px - base.fonts.size_px) as i32;
        assert_eq!(
            h,
            h100 + px + g,
            "the first line grows with the font, the next by the pitch"
        );
        let c = draw_tip(&grown, &text, label);
        let has_ink = |y: i32| (1..w - 1).any(|x| c.get(x, y) == TIP_INK.argb());
        let line1 = (1..h - 1).find(|&y| has_ink(y)).expect("line 1 has ink");
        let pitch = INFO_LINE_H + g;
        assert!(!has_ink(line1 + pitch - 1), "a blank row above line 2");
        assert!(has_ink(line1 + pitch), "line 2 starts a grown pitch down");

        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let glass200 = glass.with_text_size(200);
        let g = glass200.line_growth();
        let (_, h100) = tip_size(&text, &glass, label);
        let (_, h) = tip_size(&text, &glass200, label);
        assert_eq!(
            h,
            h100 + 2 * g,
            "a look's box grows every line by the pitch"
        );
        let body = "Opens the settings.";
        assert_eq!(
            info_size(&text, &glass200, body).1,
            info_size(&text, &glass, body).1 + g
        );
        let lines = [
            String::from("Type: Text Document"),
            String::from("Size: 1 KB"),
        ];
        assert_eq!(
            crate::controls::infotip::size(&text, &glass200, &lines).1,
            crate::controls::infotip::size(&text, &glass, &lines).1 + 2 * g
        );
    }

    #[test]
    fn a_grown_balloon_tip_grows_its_title_and_lines() {
        let text = TextRenderer::load(&FontConfig::default());
        let body = "Closes your programs.";
        let base = Theme::default();
        let grown = base.with_text_size(200);
        let g = grown.line_growth();
        let (_, h100) = balloon_size(&text, &base, "Log Off", body);
        let (_, h) = balloon_size(&text, &grown, "Log Off", body);
        assert_eq!(h, h100 + 2 * g, "the title's gap and the body line grow");

        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let glass200 = glass.with_text_size(200);
        let (b, b200) = (
            balloon_look(&glass).unwrap(),
            balloon_look(&glass200).unwrap(),
        );
        assert_eq!(b200.title_px, 2 * b.title_px);
        assert_eq!(b200.body_y, b.body_y + b.title_px as i32);
        assert_eq!(b200.line_h, b.line_h + glass200.line_growth());
        assert_eq!(
            balloon_look(&glass.with_text_size(100)),
            Some(b),
            "100% keeps the look's own"
        );
    }

    #[test]
    fn a_tip_draws_at_most_tip_lines_of_tip_chars() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let tall = "x\n".repeat(10_000);
        let (_, h) = tip_size(&text, &theme, &tall);
        assert_eq!(h, 17 + (TIP_LINES as i32 - 1) * INFO_LINE_H);
        let wide = "x".repeat(TIP_CHARS + 10_000);
        let (w, _) = tip_size(&text, &theme, &wide);
        assert_eq!(w, text.measure(&wide[..TIP_CHARS], false, 11) + 6);
        let c = draw_tip(&theme, &text, &tall);
        assert_eq!(c.height as i32, h);
    }

    #[test]
    fn placed_above_the_pointer_and_kept_on_screen() {
        let screen = Rect::new(0, 0, 1920, 1080);
        // Near the bottom-left corner the box starts one pixel left of
        // the pointer, its bottom `ABOVE_POINTER` above it.
        assert_eq!(place((28, 1066), (94, 17), screen, false), (27, 1041));
        // Near the right edge the box is pushed left to end one pixel
        // before the screen's last column.
        assert_eq!(place((1885, 1066), (156, 17), screen, false), (1763, 1041));
        // Never left of, or above, the screen.
        assert_eq!(place((0, 5), (50, 17), screen, false), (0, 0));
        // Follows the output's origin.
        let second = Rect::new(1920, 0, 1920, 1080);
        assert_eq!(place((1925, 1066), (50, 17), second, false), (1924, 1041));
    }

    #[test]
    fn on_a_top_bar_a_tip_opens_under_the_pointer() {
        let screen = Rect::new(0, 0, 1920, 1080);
        // Its top 21 px under the hotspot, one pixel left of the pointer.
        assert_eq!(place((28, 14), (94, 17), screen, true), (27, 35));
        assert_eq!(
            place((1885, 14), (156, 17), screen, true),
            (1763, 35),
            "pushed left off the right edge"
        );
        assert_eq!(
            place((40, 1075), (50, 17), screen, true),
            (39, 1063),
            "kept above the screen's foot"
        );
    }

    #[test]
    fn a_tip_on_a_side_bar_stands_beside_it_level_with_the_pointer() {
        let screen = Rect::new(0, 0, 1920, 1080);
        assert_eq!(
            place_beside((30, 500), 60, true, (120, 20), screen),
            (62, 490),
            "2 px right of a left bar, its middle at the pointer"
        );
        assert_eq!(
            place_beside((1890, 500), 1860, false, (120, 20), screen),
            (1738, 490),
            "2 px left of a right bar"
        );
        assert_eq!(
            place_beside((30, 1075), 60, true, (120, 20), screen).1,
            1060,
            "kept above the output's foot"
        );
        assert_eq!(
            place_beside((30, 500), 60, true, (1900, 20), screen).0,
            19,
            "pushed left to end one pixel before the screen's last column"
        );
    }

    #[test]
    fn task_tip_only_when_the_title_was_cut() {
        let text = TextRenderer::load(&FontConfig::default());
        assert_eq!(task_tip(&text, 11, "Short", false, 160), None);
        let long = "A window title far too long for a taskbar button to show";
        assert_eq!(
            task_tip(&text, 11, long, false, 160),
            Some(long.to_string())
        );
        assert_eq!(task_tip(&text, 11, long, true, 160), Some(long.to_string()));
    }

    fn balloon(theme: &Theme, text: &TextRenderer, title: &str, body: &str) -> Canvas {
        let lay = notify_layout(theme, text, title, body, &[], None);
        draw_notify_balloon(theme, text, &lay, None, None, None, None)
    }

    fn notify_height(text: &TextRenderer, theme: &Theme, body: &str) -> u32 {
        balloon(theme, text, "Hi", body).height
    }

    /// A body of bare newlines would size the balloon by its line count, so
    /// the count is capped; past the cap the balloon stops growing.
    #[test]
    fn notify_body_lines_are_capped() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let at_cap = notify_height(&t, &theme, &"x\n".repeat(NOTIFY_BODY_LINES));
        let past = notify_height(&t, &theme, &"x\n".repeat(100_000));
        assert_eq!(at_cap, past);
    }

    #[test]
    fn a_taller_notify_body_makes_a_taller_balloon() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let long = "one two three four five six seven eight nine ten eleven twelve";
        let short = balloon(&theme, &t, "Hi", "one line");
        let tall = balloon(&theme, &t, "Hi", long);
        assert_eq!((short.width, tall.width), (300, 300));
        assert!(
            tall.height > short.height,
            "{} !> {}",
            tall.height,
            short.height
        );
    }

    #[test]
    fn the_notify_balloon_is_tooltip_yellow_with_clear_corners() {
        let t = TextRenderer::load(&FontConfig::default());
        let c = balloon(&Theme::default(), &t, "Update", "Ready.");
        assert_eq!(c.get(NOTIFY_W / 2, 20), TIP_BG.argb());
        assert_eq!(c.get(0, 0), 0);
    }

    #[test]
    fn a_scale_2_notify_balloon_sits_in_the_same_logical_corner() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let usable = Rect::new(0, 0, 1920, 1050);
        let one = notify_layout(&theme, &t, "Update", "Ready.", &[], None);
        let want = (1920 - 300 - 8, 1050 - one.height - 2);
        assert_eq!(place_notify(usable, &one, 1, Edge::Bottom), want);
        let two = notify_layout(&theme.scaled(2), &t, "Update", "Ready.", &[], None);
        assert_eq!(place_notify(usable, &two, 2, Edge::Bottom), want);
    }

    #[test]
    fn under_a_top_bar_the_notify_balloon_sits_top_right_its_tail_rising() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let usable = Rect::new(0, 30, 1920, 1050);
        let bottom = notify_layout(&theme, &t, "Update", "Ready.", &["Open"], None);
        let top = bottom.clone().on_edge(Edge::Top);
        assert_eq!(
            place_notify(usable, &top, 1, Edge::Top),
            (1612, 32),
            "8 px in from the right, 2 px under the bar"
        );
        assert_eq!(top.height, bottom.height);
        let dy = NOTIFY_TAIL_H;
        assert_eq!(
            top.buttons[0],
            Rect::new(
                bottom.buttons[0].x,
                bottom.buttons[0].y + dy,
                BUTTON_W,
                BUTTON_H
            )
        );
        assert_eq!(top.close.y, bottom.close.y + dy);
        let b = top.buttons[0];
        assert_eq!(notify_hit(&top, b.x + 3, b.y + 3), NotifyHit::Button(0));
        let up = draw_notify_balloon(&theme, &t, &top, None, None, None, None);
        let down = draw_notify_balloon(&theme, &t, &bottom, None, None, None, None);
        // The bubble inside its top and bottom border rows, 12 px lower.
        for y in 1..bottom.bubble_h - 1 {
            for x in 0..NOTIFY_W {
                assert_eq!(up.get(x, y + dy), down.get(x, y), "bubble ({x}, {y})");
            }
        }
        // The tail turned over above the bubble.
        for r in 0..dy {
            for x in 0..NOTIFY_W {
                assert_eq!(
                    up.get(x, dy - 1 - r),
                    down.get(x, bottom.bubble_h + r),
                    "tail row {r} at x {x}"
                );
            }
        }
        let seam = NOTIFY_W - NOTIFY_PAD - NOTIFY_TAIL_W + 5;
        assert_eq!(
            up.get(seam, dy),
            TIP_BG.argb(),
            "the bubble's top border opens where the tail joins"
        );
    }

    #[test]
    fn action_buttons_sit_right_aligned_in_a_row_under_the_body() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let plain = notify_layout(&theme, &t, "Mail", "New message", &[], None);
        let lay = notify_layout(&theme, &t, "Mail", "New message", &["Open", "Reply"], None);
        // The row adds its 8 px gap and a 23 px button to the bubble.
        assert_eq!(lay.bubble_h, plain.bubble_h + NOTIFY_ROW_GAP + BUTTON_H);
        assert_eq!(lay.height, lay.bubble_h + NOTIFY_TAIL_H);
        assert_eq!(lay.more, None);
        let [open, reply] = lay.buttons[..] else {
            panic!("two buttons: {:?}", lay.buttons);
        };
        // Short labels get the standard 75x23 button, 6 px apart, the last
        // one 10 px in from the right edge and 10 px up from the bubble's
        // bottom.
        assert_eq!((open.width, open.height), (BUTTON_W, BUTTON_H));
        assert_eq!(reply.right(), NOTIFY_W - NOTIFY_PAD);
        assert_eq!(reply.x - open.right(), BUTTON_GAP);
        assert_eq!(reply.bottom(), lay.bubble_h - NOTIFY_PAD);
        assert_eq!(
            notify_hit(&lay, open.x + 3, open.y + 3),
            NotifyHit::Button(0)
        );
        assert_eq!(
            notify_hit(&lay, reply.x + 3, reply.y + 3),
            NotifyHit::Button(1)
        );
        assert_eq!(notify_hit(&lay, 60, 30), NotifyHit::Body);
        assert_eq!(
            notify_hit(&lay, lay.close.x + 4, lay.close.y + 4),
            NotifyHit::Close
        );

        // The pushed button's outer ring is window-frame black; the other
        // keeps its raised white top edge.
        let c = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            Some(NotifyHit::Button(0)),
            None,
            None,
        );
        let cs = &theme.colors;
        assert_eq!(c.get(open.x + 5, open.y), cs.window_frame.argb());
        assert_eq!(c.get(reply.x + 5, reply.y), cs.button_hilight.argb());
    }

    #[test]
    fn a_fourth_action_brings_the_more_button() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let acts = ["One", "Two", "Three", "Four", "Five"];
        let lay = notify_layout(&theme, &t, "Hi", "Body", &acts, None);
        assert_eq!(lay.buttons.len(), NOTIFY_BUTTONS);
        let more = lay.more.expect("a ... button");
        assert_eq!((more.width, more.height), (BUTTON_H, BUTTON_H));
        assert_eq!(more.right(), NOTIFY_W - NOTIFY_PAD);
        assert_eq!(notify_hit(&lay, more.x + 2, more.y + 2), NotifyHit::More);
        assert!(lay.buttons.iter().all(|b| b.width == BUTTON_W));
        assert_eq!(more.x - lay.buttons[2].right(), BUTTON_GAP);

        // Long labels overrun the 280 px row, so the labelled three share
        // what the square button and the gaps leave, evenly.
        let long = ["Open in a new window", "Mark as read", "Archive", "Delete"];
        let lay = notify_layout(&theme, &t, "Hi", "Body", &long, None);
        let room = NOTIFY_W - 2 * NOTIFY_PAD - BUTTON_H - 3 * BUTTON_GAP;
        assert!(lay.buttons.iter().all(|b| b.width == room / 3));
        assert!(lay.buttons[0].x >= NOTIFY_PAD);
    }

    /// Blue's tip, in every variant, is a black 1 px frame round info_bk
    /// with its ink from 3 px in and 4 px down.
    #[test]
    fn a_blue_tip_is_black_round_its_info_colour_in_every_variant() {
        use crate::theme::Color;
        for v in crate::blue::Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            let text = TextRenderer::load(&theme.fonts);
            let c = draw_tip(&theme, &text, "Tuesday");
            let (w, h) = (c.width as i32, c.height as i32);
            let black = Color::rgb(0, 0, 0).argb();
            for (x, y) in [(0, 0), (w - 1, h - 1), (0, h / 2), (w / 2, 0)] {
                assert_eq!(c.get(x, y), black, "{v:?}: the frame at {x},{y}");
            }
            assert_eq!(c.get(1, 1), theme.colors.info_bk.argb(), "{v:?}: the fill");
            if crate::text::test_tahoma() {
                assert_eq!(h, 17, "{v:?}: 17 px tall");
                let first = (1..w - 1).find(|&x| (1..h - 1).any(|y| c.get(x, y) == black));
                let top = (1..h - 1).find(|&y| (1..w - 1).any(|x| c.get(x, y) == black));
                assert_eq!(
                    (first, top),
                    (Some(3), Some(4)),
                    "{v:?}: the ink starts 3 px in, 4 down"
                );
            }
        }
    }

    #[test]
    fn beside_a_side_bar_the_balloon_sits_in_the_foot_corner_by_the_tray() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let lay = notify_layout(&theme, &t, "Update", "Ready.", &["Open"], None);
        let left = lay.clone().on_edge(Edge::Left);
        assert_eq!(
            (left.bubble_y, left.close, left.buttons.clone()),
            (lay.bubble_y, lay.close, lay.buttons.clone()),
            "the tail hangs under the bubble, as over a bottom bar"
        );
        let usable = Rect::new(60, 0, 1860, 1080);
        assert_eq!(
            place_notify(usable, &left, 1, Edge::Left),
            (68, 1080 - lay.height - 2),
            "8 px off the bar, 2 px up from the foot"
        );
        let usable = Rect::new(0, 0, 1860, 1080);
        assert_eq!(
            place_notify(usable, &lay, 1, Edge::Right),
            (1860 - lay.width - 8, 1080 - lay.height - 2)
        );
    }
    const REPLY: NotifyReply = NotifyReply {
        placeholder: "Write a reply",
        send: "Send",
    };

    /// The expanded reply row takes the button row's place at its height:
    /// a 21 px field from the left padding to 6 px short of a 75x23 Send
    /// button that ends 10 px in from the right.
    #[test]
    fn the_reply_row_stands_where_the_buttons_stood() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let acts = ["Reply", "Mark as read"];
        let row = notify_layout(&theme, &t, "Ana", "Lunch?", &acts, None);
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &acts, Some(REPLY));
        assert_eq!(lay.height, row.height, "same size either way");
        assert!(lay.buttons.is_empty() && lay.more.is_none());
        let send = lay.send.expect("a send button");
        assert_eq!(
            send,
            Rect::new(
                NOTIFY_W - NOTIFY_PAD - BUTTON_W,
                row.buttons[0].y,
                BUTTON_W,
                BUTTON_H
            )
        );
        let field = lay.field.expect("a field");
        assert_eq!(
            field,
            Rect::new(NOTIFY_PAD, send.y + 1, send.x - BUTTON_GAP - NOTIFY_PAD, 21)
        );
        assert_eq!(notify_hit(&lay, field.x + 5, field.y + 5), NotifyHit::Field);
        assert_eq!(notify_hit(&lay, send.x + 5, send.y + 5), NotifyHit::Send);
        let top = lay.clone().on_edge(Edge::Top);
        assert_eq!(top.field.map(|f| f.y), Some(field.y + NOTIFY_TAIL_H));
        assert_eq!(top.send.map(|b| b.y), Some(send.y + NOTIFY_TAIL_H));
    }

    /// An empty field shows the grey hint and a greyed Send; typed text
    /// replaces the hint in button-text ink, Send comes alive, and a
    /// focused field draws its caret in the blink's shown half only.
    #[test]
    fn the_reply_field_shows_its_hint_until_there_is_text() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], Some(REPLY));
        let (field, send) = (lay.field.unwrap(), lay.send.unwrap());
        let inks = |c: &Canvas, r: Rect, ink: crate::theme::Color| {
            (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == ink.argb()))
        };
        let inside = field.inset(3);
        let empty = TextEdit::default();
        let c = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            None,
            None,
            Some((&empty, Caret::Unfocused, None)),
        );
        assert!(inks(&c, inside, cs.gray_text), "the hint is grey");
        assert!(
            !inks(&c, inside, cs.button_text),
            "and nothing else inks it"
        );
        assert_eq!(
            c.get(field.x + 2, field.y + 2),
            cs.window.argb(),
            "a white field"
        );
        assert!(!inks(&c, send, cs.button_text), "Send is greyed");

        let typed = TextEdit::new("Sure", false);
        let c = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            None,
            None,
            Some((&typed, Caret::Shown, None)),
        );
        assert!(inks(&c, inside, cs.button_text), "the text inks the field");
        assert!(!inks(&c, inside, cs.gray_text), "the hint has gone");
        assert!(inks(&c, send, cs.button_text), "Send is live");
        // The caret: a 1 px bar right after the text.
        let w = t.measure("Sure", false, theme.fonts.size_px);
        let cx = field.x + 3 + w;
        let bar =
            |c: &Canvas| (field.y..field.bottom()).any(|y| c.get(cx, y) == cs.button_text.argb());
        assert!(bar(&c), "the caret shows");
        let hidden = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            None,
            None,
            Some((&typed, Caret::Hidden, None)),
        );
        assert!(!bar(&hidden), "the blink's hidden half");
    }

    /// An input method's preedit draws underlined after the typed text,
    /// and the focused field reports its spot for the candidate popup;
    /// an unfocused one shows no preedit and reports nothing.
    #[test]
    fn the_reply_field_shows_the_preedit_and_reports_its_spot() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let ink = theme.colors.button_text.argb();
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], Some(REPLY));
        let field = lay.field.unwrap();
        let typed = TextEdit::new("ok", false);
        let preedit = Preedit::new("nihao".into(), 5, 5);
        let draw = |caret| {
            draw_notify_balloon(
                &theme,
                &t,
                &lay,
                None,
                None,
                None,
                Some((&typed, caret, preedit.as_ref())),
            )
        };
        let mut c = draw(Caret::Hidden);
        let spot = c.take_spot().expect("a focused field reports its spot");
        let px = theme.fonts.size_px;
        let (ok, all) = (t.measure("ok", false, px), t.measure("oknihao", false, px));
        assert_eq!(
            spot.caret.x,
            field.x + 3 + all,
            "the caret ends the preedit"
        );
        assert!(field.contains(spot.caret.x, spot.caret.y));
        let underline = (field.y..field.bottom())
            .any(|y| (field.x + 3 + ok..field.x + 3 + all).all(|x| c.get(x, y) == ink));
        assert!(underline, "the preedit is underlined");

        let mut idle = draw(Caret::Unfocused);
        assert_eq!(idle.take_spot(), None, "an unfocused field reports nothing");
        let plain = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            None,
            None,
            Some((&typed, Caret::Unfocused, None)),
        );
        assert!(idle.pixels == plain.pixels, "and draws no preedit");

        let empty = TextEdit::default();
        let first = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            None,
            None,
            Some((&empty, Caret::Shown, preedit.as_ref())),
        );
        let inside = field.inset(3);
        let grey = theme.colors.gray_text.argb();
        let hint = (inside.x..inside.right())
            .any(|x| (inside.y..inside.bottom()).any(|y| first.get(x, y) == grey));
        assert!(!hint, "the hint goes while a preedit is composed");
    }

    /// Blue and Glass draw the reply row through their own field and
    /// button parts at the same places: the field's inside is the window
    /// colour and the send button covers the balloon's yellow.
    #[test]
    fn every_interface_draws_the_reply_row() {
        use crate::interface::Interface;
        for iface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(iface);
            let t = TextRenderer::load(&theme.fonts);
            let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], Some(REPLY));
            let field = lay.field.unwrap();
            let typed = TextEdit::default();
            let c = draw_notify_balloon(
                &theme,
                &t,
                &lay,
                None,
                None,
                None,
                Some((&typed, Caret::Shown, None)),
            );
            let mid = (field.x + field.width / 2, field.y + field.height / 2);
            assert_eq!(
                c.get(mid.0, mid.1),
                theme.colors.window.argb(),
                "{iface:?}: the field's inside"
            );
            let send = lay.send.unwrap();
            assert_ne!(
                c.get(send.x + send.width / 2, send.y + 3),
                theme.colors.info_bk.argb(),
                "{iface:?}: the send button covers the balloon"
            );
        }
    }

    #[test]
    fn enter_sends_the_trimmed_reply_and_escape_collapses() {
        let mut f = ReplyField::default();
        assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::None, "nothing typed");
        f.key(ReplyKey::Edit(EditKey::Insert("  ".into())));
        assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::None, "only blanks");
        assert!(f.paste("on my way\nsecond line"));
        assert_eq!(f.edit.text, "  on my way");
        assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::Send("on my way".into()));
        assert_eq!(f.key(ReplyKey::Escape), ReplyEvent::Collapse);
    }

    /// A click focuses the field and puts the caret at the boundary
    /// nearest the pointer: before the text left of it, after it right.
    #[test]
    fn a_click_on_the_field_focuses_it_and_places_the_caret() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], Some(REPLY));
        let field = lay.field.unwrap();
        let mut f = ReplyField {
            edit: TextEdit::new("Sure thing", false),
            focused: false,
        };
        f.click(&theme, &t, &lay, field.x + 1);
        assert!(f.focused);
        assert_eq!(f.edit.caret(), 0);
        f.click(&theme, &t, &lay, field.right() - 2);
        assert_eq!(f.edit.caret(), "Sure thing".len());
        let w = t.measure("Sure", false, theme.fonts.size_px);
        f.click(&theme, &t, &lay, field.x + FIELD_PAD + w);
        assert_eq!(f.edit.caret(), 4);
    }

    use crate::access::ROOT;
    use crate::access::accesskit::{Action, Live, Role};

    fn parts(
        tree: &crate::access::Tree,
        id: crate::access::accesskit::NodeId,
    ) -> Vec<(Role, String)> {
        let n = tree.get(id).unwrap();
        n.children()
            .iter()
            .map(|&c| {
                let n = tree.get(c).unwrap();
                let name = n.label().or(n.value()).unwrap_or_default();
                (n.role(), name.to_string())
            })
            .collect()
    }

    #[test]
    fn the_balloon_is_a_live_alert_naming_its_parts() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let acts = ["Open", "Save & Archive", "Later", "Snooze"];
        let body = "Lunch at noon? The usual place, unless it rains again today.";
        let lay = notify_layout(&theme, &t, "Ana", body, &acts, None);
        let says = NotifySays {
            app: "Chat",
            title: "Ana",
            body,
            actions: &acts,
            opens: true,
            urgent: false,
        };
        let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
        let px = theme.fonts.size_px;
        let alert = access_notify(1, &lay, &says, None, &t, px, &mut tree, ROOT);
        let n = tree.get(alert).unwrap();
        assert_eq!(n.role(), Role::Alert);
        assert_eq!(n.label(), Some(format!("Chat: Ana. {body}").as_str()));
        assert_eq!(n.live(), Some(Live::Polite));
        let named = |r: Role, s: &str| (r, s.to_string());
        assert_eq!(
            parts(&tree, alert),
            [
                named(Role::Label, "Ana"),
                named(Role::Label, body),
                named(Role::Button, "Open"),
                named(Role::Button, "Save & Archive"),
                named(Role::Button, "Later"),
                named(Role::Button, "More actions"),
                named(Role::Button, "Close"),
            ]
        );
        let kids = n.children();
        let body_node = tree.get(kids[1]).unwrap();
        assert!(body_node.supports_action(Action::Click), "the body opens");
        let open = tree.get(kids[2]).unwrap();
        assert_eq!(open.bounds(), Some(crate::access::bounds(lay.buttons[0])));
        assert!(open.supports_action(Action::Click));
        let close = tree.get(kids[6]).unwrap();
        assert_eq!(close.bounds(), Some(crate::access::bounds(lay.close)));
        assert!(
            kids.iter()
                .all(|&k| tree.get(k).unwrap().live() == Some(Live::Off)),
            "only the alert is announced, not each part again"
        );
    }

    #[test]
    fn an_urgent_balloon_is_assertive_and_follows_the_bubble() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let lay = notify_layout(&theme, &t, "Battery low", "", &[], None).on_edge(Edge::Top);
        let says = NotifySays {
            app: "",
            title: "Battery low",
            body: "",
            actions: &[],
            opens: false,
            urgent: true,
        };
        let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
        let alert = access_notify(1, &lay, &says, None, &t, 11, &mut tree, ROOT);
        let n = tree.get(alert).unwrap();
        assert_eq!(n.live(), Some(Live::Assertive));
        assert_eq!(n.label(), Some("Battery low"));
        assert_eq!(n.bounds().map(|b| b.y0), Some(f64::from(NOTIFY_TAIL_H)));
        assert_eq!(
            parts(&tree, alert),
            [
                (Role::Label, "Battery low".to_string()),
                (Role::Button, "Close".to_string())
            ]
        );
        let only_app = NotifySays {
            app: "Mail",
            title: " ",
            ..says
        };
        assert_eq!(only_app.spoken(), "Mail");
    }

    #[test]
    fn the_reply_row_reads_as_a_field_and_send() {
        let t = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], Some(REPLY));
        let says = NotifySays {
            app: "Chat",
            title: "Ana",
            body: "Lunch?",
            actions: &["Reply"],
            opens: false,
            urgent: false,
        };
        let px = theme.fonts.size_px;
        let mut field = ReplyField::default();
        let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
        let alert = access_notify(1, &lay, &says, Some(&field), &t, px, &mut tree, ROOT);
        assert_eq!(
            tree.get(alert).unwrap().role(),
            Role::Alert,
            "not focused yet"
        );
        let kids = tree.get(alert).unwrap().children().to_vec();
        let input = tree.get(kids[2]).unwrap();
        assert_eq!(input.role(), Role::TextInput);
        assert_eq!(input.placeholder(), Some("Write a reply"));
        assert_eq!(
            input.bounds(),
            Some(crate::access::bounds(lay.field.unwrap()))
        );
        let send = tree.get(kids[3]).unwrap();
        assert_eq!((send.role(), send.label()), (Role::Button, Some("Send")));
        assert!(send.is_disabled(), "nothing to send");
        assert!(
            !tree.get(kids[1]).unwrap().supports_action(Action::Click),
            "a body with no default action does not offer a click"
        );

        field.focused = true;
        field.key(ReplyKey::Edit(EditKey::Insert("on my way".into())));
        let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
        let focus = access_notify(1, &lay, &says, Some(&field), &t, px, &mut tree, ROOT);
        let input = tree.get(focus).unwrap();
        assert_eq!(input.role(), Role::TextInput);
        let run = tree.get(input.children()[0]).unwrap();
        assert_eq!(run.value(), Some("on my way"));
        let kids = tree.get(alert).unwrap().children().to_vec();
        assert!(
            !tree.get(kids[3]).unwrap().is_disabled(),
            "Send comes alive"
        );
    }
}
