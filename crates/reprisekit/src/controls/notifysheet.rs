//! A look's notification sheet, its stack by the tray and the "+N more" chip
//! past it. Classic and Blue show the balloon in [`super::tooltip`] instead.

use super::tooltip::{
    NOTIFY_BODY_LINES, NOTIFY_BUTTONS, NOTIFY_MORE, NotifyHit, NotifyParts, NotifyReply,
    NotifySays, ReplyField, access_parts, parts_hit,
};
use crate::access::Tree;
use crate::access::accesskit::NodeId;
use crate::anim::Ease;
use crate::backdrop::{corner_radius, round_corner};
use crate::classic::widgets::{draw_edit, edit_first_visible};
use crate::paint::Paint;
use crate::raster::{Canvas, Rect};
use crate::shape::{mix, over, pack, rounded, scale, smooth, unpack};
use crate::style::{NotifyLook, SQUIRCLE_ROUND, SheetShape, SheetSpec, State, Well};
use crate::taskbar::Edge;
use crate::text::{Align, TextRenderer, wrap};
use crate::textedit::{Caret, Preedit, TextEdit};
use crate::theme::{Color, Theme};

fn notify_look(theme: &Theme) -> Option<NotifyLook> {
    theme.look.style().and_then(|st| st.notify)
}

pub fn has_sheets(theme: &Theme) -> bool {
    notify_look(theme).is_some()
}

/// What a sheet shows. `when` is the app line's time ("now"); `has_picture` and
/// `has_app_icon` say which of [`SheetIcons`] the host will pass.
pub struct SheetContent<'a> {
    pub app: &'a str,
    pub when: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: &'a [&'a str],
    pub reply: Option<NotifyReply<'a>>,
    pub has_picture: bool,
    pub has_app_icon: bool,
}

/// A sheet in device px from its canvas's top-left; the canvas is `footprint` (`sheet` and any
/// well standing out of it) grown by the look's shadow margin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetLayout {
    pub width: i32,
    pub height: i32,
    pub shape: SheetShape,
    pub expanded: bool,
    pub sheet: Rect,
    pub footprint: Rect,
    pub icon: Rect,
    pub well: Well,
    pub badge: Option<Rect>,
    pub picture: Option<Rect>,
    pub parts: NotifyParts,
    pub lines: Vec<String>,
    pub app_px: u32,
    pub title_px: u32,
    pub body_px: u32,
    app_line: String,
    app_box: Rect,
    title: String,
    labels: Vec<String>,
    line_h: i32,
    pill_px: u32,
}

/// The parts every shape shares, design px.
const APP_PX: u32 = 12;
const BODY_PX: u32 = 14;
const APP_H: i32 = 16;
const TITLE_H: i32 = 20;
const LINE_H: i32 = 19;
const ROW_GAP: i32 = 10;
const PILL_H: i32 = 26;
const PILL_GAP: i32 = 6;
const PILL_PX: u32 = 13;
const PILL_PAD: i32 = 14;
const PILL_MIN: i32 = 64;
const MORE_W: i32 = 34;
const CLOSE: i32 = 20;
/// The app line and title end this far short of the close button.
const CLOSE_GAP: i32 = 4;
const BADGE: i32 = 18;
/// The badge centres this far in from the slot's lower-right corner.
const BADGE_IN: i32 = 4;
/// The app icon's inset inside the badge's ring.
const BADGE_ICON_IN: i32 = 2;
/// The reply field's text stands in this far from its rounded ends.
const FIELD_TEXT_IN: i32 = PILL_H / 2;
/// A picture beside the title stands this far left of it.
const PICTURE_GAP: i32 = 8;

/// A shape's layout, design px from the sheet's top-left.
struct Frame {
    shape: SheetShape,
    width: i32,
    pad: i32,
    /// The icon's well; a slot above the sheet's top edge breaks out of it.
    slot: Rect,
    well: Well,
    /// The app line's top-left.
    app: (i32, i32),
    /// The title's top-left; the body runs under it from the same left.
    text: (i32, i32),
    title_px: u32,
    /// The title ends short of the close button rather than at the padding.
    title_by_close: bool,
    close: (i32, i32),
    /// The side of a round picture at `text`, pushing the title right of it. `None` puts the
    /// picture in the slot with the app icon as a badge.
    picture: Option<i32>,
    /// The pill row's and reply field's left.
    row_x: i32,
}

/// The icon slot inside the padding and the text column right of it.
const LOZENGE: Frame = Frame {
    shape: SheetShape::Lozenge,
    width: 360,
    pad: 14,
    slot: Rect::new(14, 14, 46, 46),
    well: Well::Squircle,
    app: (72, 14),
    text: (72, 30),
    title_px: 16,
    title_by_close: true,
    close: (360 - 14 - CLOSE, 12),
    picture: None,
    row_x: 72,
};

/// The icon breaking half out of the top-left edge and the text full width under the app line.
const HALO: Frame = Frame {
    shape: SheetShape::Halo,
    width: 360,
    pad: 18,
    slot: Rect::new(9, -25, 50, 50),
    well: Well::Squircle,
    app: (69, 8),
    text: (18, 33),
    title_px: 15,
    title_by_close: false,
    close: (360 - 16 - CLOSE, 13),
    picture: Some(28),
    row_x: 18,
};

/// Lay out a sheet of `shape` for `c`; `expanded` opens a Droplet and shows the reply row in
/// place of the pills. `None` when the look has no sheets ([`has_sheets`]).
pub fn sheet_layout(
    theme: &Theme,
    text: &TextRenderer,
    shape: SheetShape,
    c: &SheetContent,
    expanded: bool,
) -> Option<SheetLayout> {
    let look = notify_look(theme)?;
    let frame = match shape {
        SheetShape::Lozenge => &LOZENGE,
        SheetShape::Halo => &HALO,
        SheetShape::Droplet => return Some(droplet_layout(theme, text, &look, c, expanded)),
    };
    Some(column_layout(theme, text, &look, frame, c, expanded))
}

fn app_line(c: &SheetContent) -> String {
    if c.when.is_empty() {
        c.app.to_string()
    } else {
        format!("{} \u{b7} {}", c.app, c.when)
    }
}

/// `body` wrapped to `width`; past `NOTIFY_BODY_LINES` the last line kept ends in "...".
fn body_lines(text: &TextRenderer, body: &str, width: i32, px: u32) -> Vec<String> {
    let mut lines = wrap(text, body, width, px);
    if lines.len() > NOTIFY_BODY_LINES {
        lines.truncate(NOTIFY_BODY_LINES);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && text.measure(&format!("{last}..."), false, px) > width {
                last.pop();
            }
            *last = format!("{}...", last.trim_end());
        }
    }
    lines
}

/// The pills or, with `reply`, the reply field and its send pill, right-aligned between
/// `left` and `right` from `y`, design px.
struct ActionRow {
    buttons: Vec<Rect>,
    labels: Vec<String>,
    more: Option<Rect>,
    field: Option<Rect>,
    send: Option<Rect>,
    shown: bool,
}

fn action_row(
    text: &TextRenderer,
    actions: &[&str],
    reply: Option<NotifyReply>,
    (left, right, y): (i32, i32, i32),
    s: i32,
    pill_px: u32,
    at: impl Fn(i32, i32, i32, i32) -> Rect,
) -> ActionRow {
    let room = right - left;
    let shown = actions.len().min(NOTIFY_BUTTONS);
    let more_w = if actions.len() > NOTIFY_BUTTONS {
        MORE_W
    } else {
        0
    };
    let mut row = ActionRow {
        buttons: Vec::new(),
        labels: Vec::new(),
        more: None,
        field: None,
        send: None,
        shown: true,
    };
    if let Some(r) = reply {
        let sw = pill_widths(text, &[r.send], pill_px, s, room)[0];
        let sx = right - sw;
        row.send = Some(at(sx, y, sw, PILL_H));
        row.field = Some(at(left, y, sx - PILL_GAP - left, PILL_H));
    } else if shown > 0 || more_w > 0 {
        let parts = shown + usize::from(more_w > 0);
        let room = room - more_w - (parts as i32 - 1) * PILL_GAP;
        let widths = pill_widths(text, &actions[..shown], pill_px, s, room);
        let mut x = right;
        if more_w > 0 {
            x -= more_w;
            row.more = Some(at(x, y, more_w, PILL_H));
            x -= PILL_GAP;
        }
        for (a, w) in actions[..shown].iter().zip(&widths).rev() {
            x -= w;
            row.buttons.push(at(x, y, *w, PILL_H));
            let room = (w - 2 * PILL_PAD) * s;
            row.labels
                .push(text.ellipsize(a, room, true, pill_px).into_owned());
            x -= PILL_GAP;
        }
        row.buttons.reverse();
        row.labels.reverse();
    } else {
        row.shown = false;
    }
    row
}

/// Pill widths, design px: label plus padding, shrunk evenly when they overrun `room`.
fn pill_widths(text: &TextRenderer, labels: &[&str], px: u32, s: i32, room: i32) -> Vec<i32> {
    let mut w: Vec<i32> = labels
        .iter()
        .map(|l| ((text.measure(l, true, px) + s - 1) / s + 2 * PILL_PAD).max(PILL_MIN))
        .collect();
    let n = w.len() as i32;
    if n > 0 && w.iter().sum::<i32>() > room {
        w.fill(room / n);
    }
    w
}

fn column_layout(
    theme: &Theme,
    text: &TextRenderer,
    look: &NotifyLook,
    f: &Frame,
    c: &SheetContent,
    expanded: bool,
) -> SheetLayout {
    let s = theme.scale.max(1);
    let su = s as u32;
    let m = (look.margin)(f.shape);
    let w = f.width;
    let (app_px, title_px, body_px, pill_px) =
        (APP_PX * su, f.title_px * su, BODY_PX * su, PILL_PX * su);
    let picture = f
        .picture
        .filter(|_| c.has_picture)
        .map(|side| Rect::new(f.text.0, f.text.1, side, side));
    let text_x = picture.map_or(f.text.0, |p| p.right() + PICTURE_GAP);
    let text_w = w - f.pad - text_x;
    let close_x = f.close.0;
    let head_w = close_x - CLOSE_GAP - f.app.0;
    let title_w = if f.title_by_close {
        close_x - CLOSE_GAP - text_x
    } else {
        text_w
    };
    let (title_y, body_y) = (f.text.1, f.text.1 + TITLE_H);

    let app_shown = text
        .ellipsize(&app_line(c), head_w * s, false, app_px)
        .into_owned();
    let title = text
        .ellipsize(c.title, title_w * s, true, title_px)
        .into_owned();
    let lines = body_lines(text, c.body, text_w * s, body_px);
    let text_bottom = (body_y + lines.len() as i32 * LINE_H).max(picture.map_or(0, |p| p.bottom()));

    let reply = c.reply.filter(|_| expanded);
    let actions = if reply.is_some() { &[][..] } else { c.actions };
    let row_y = text_bottom + ROW_GAP;
    let ActionRow {
        buttons,
        labels,
        more,
        field,
        send,
        shown: row,
    } = action_row(
        text,
        actions,
        reply,
        (f.row_x, w - f.pad, row_y),
        s,
        pill_px,
        Rect::new,
    );
    let bottom = if row { row_y + PILL_H } else { text_bottom };
    let h = f.slot.bottom().max(bottom) + f.pad;

    let sheet = Rect::new(0, 0, w, h);
    let foot = sheet.union(&f.slot);
    let (ox, oy) = ((m.left - foot.x) * s, (m.top - foot.y) * s);
    let at = |r: Rect| Rect::new(ox + r.x * s, oy + r.y * s, r.width * s, r.height * s);
    let icon = at(f.slot);
    let badge = (f.picture.is_none() && c.has_picture && c.has_app_icon).then(|| {
        let half = BADGE / 2 * s;
        let (cx, cy) = (icon.right() - BADGE_IN * s, icon.bottom() - BADGE_IN * s);
        Rect::new(cx - half, cy - half, BADGE * s, BADGE * s)
    });
    let parts = NotifyParts {
        bubble: at(sheet),
        title: at(Rect::new(text_x, title_y, title_w, TITLE_H)),
        body: at(Rect::new(
            text_x,
            body_y,
            text_w,
            lines.len().max(1) as i32 * LINE_H,
        )),
        close: at(Rect::new(close_x, f.close.1, CLOSE, CLOSE)),
        buttons: buttons.into_iter().map(at).collect(),
        more: more.map(at),
        field: field.map(at),
        send: send.map(at),
        placeholder: reply.map_or_else(String::new, |r| r.placeholder.to_string()),
        send_label: reply.map_or_else(String::new, |r| r.send.to_string()),
    };
    SheetLayout {
        width: (m.left + foot.width + m.right) * s,
        height: (m.top + foot.height + m.bottom) * s,
        shape: f.shape,
        expanded,
        sheet: at(sheet),
        footprint: at(foot),
        icon,
        well: f.well,
        badge,
        picture: if f.picture.is_some() {
            picture.map(at)
        } else {
            c.has_picture.then_some(icon)
        },
        parts,
        lines,
        app_px,
        title_px,
        body_px,
        app_line: app_shown,
        app_box: at(Rect::new(f.app.0, f.app.1, head_w, APP_H)),
        title,
        labels,
        line_h: LINE_H * s,
        pill_px,
    }
}

/// The Droplet, design px: a capsule row of round slot, title and app line, one body line and
/// the picture; expanded, the body wraps and the pills follow.
const DROPLET_W: i32 = 360;
const DROPLET_H: i32 = 68;
const DROPLET_SLOT: i32 = 48;
const DROPLET_IN: i32 = 10;
const DROPLET_TEXT_X: i32 = DROPLET_IN + DROPLET_SLOT + DROPLET_IN;
/// Expanded without a picture, the body stops where the end curve begins.
const DROPLET_END: i32 = 34;
const DROPLET_PICTURE_END: i32 = 16;
const DROPLET_TITLE_PX: u32 = 15;
const APP_GAP: i32 = 8;
/// The title keeps at least this share of the head row from a long app line, fifths.
const TITLE_SHARE: i32 = 3;
const DROPLET_CLOSE: (i32, i32) = (DROPLET_W - DROPLET_END - CLOSE, 24);
const DROPLET_PAD: i32 = 14;
const DROPLET_OPEN_CLOSE: (i32, i32) = (DROPLET_W - DROPLET_PAD - CLOSE, 12);

fn droplet_layout(
    theme: &Theme,
    text: &TextRenderer,
    look: &NotifyLook,
    c: &SheetContent,
    expanded: bool,
) -> SheetLayout {
    let s = theme.scale.max(1);
    let su = s as u32;
    let m = (look.margin)(SheetShape::Droplet);
    let (ox, oy) = (m.left * s, m.top * s);
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::new(ox + x * s, oy + y * s, w * s, h * s);
    let (app_px, title_px, body_px, pill_px) = (
        APP_PX * su,
        DROPLET_TITLE_PX * su,
        BODY_PX * su,
        PILL_PX * su,
    );
    let picture_x = DROPLET_W - DROPLET_PICTURE_END - DROPLET_SLOT;
    let text_x = DROPLET_TEXT_X;
    let (shut_close, open_close) = if c.has_picture {
        let x = picture_x - CLOSE_GAP - CLOSE;
        ((x, DROPLET_CLOSE.1), (x, DROPLET_OPEN_CLOSE.1))
    } else {
        (DROPLET_CLOSE, DROPLET_OPEN_CLOSE)
    };
    let head_right = shut_close.0 - CLOSE_GAP;
    let text_right = match (expanded, c.has_picture) {
        (false, _) => head_right,
        (true, true) => picture_x - DROPLET_IN,
        (true, false) => DROPLET_W - DROPLET_END,
    };
    let text_w = text_right - text_x;

    let has_body = c.body.split_whitespace().next().is_some();
    let block = TITLE_H + if has_body { LINE_H } else { 0 };
    let title_y = DROPLET_IN + (DROPLET_SLOT - block) / 2;
    let body_y = title_y + TITLE_H;
    let lines = match (has_body, expanded) {
        (false, _) => Vec::new(),
        (true, true) => body_lines(text, c.body, text_w * s, body_px),
        (true, false) => {
            let one = c.body.split_whitespace().collect::<Vec<_>>().join(" ");
            vec![
                text.ellipsize(&one, text_w * s, false, body_px)
                    .into_owned(),
            ]
        }
    };

    let head = at(text_x, title_y, head_right - text_x, TITLE_H);
    let app = app_line(c);
    let app_w = text.measure(&app, false, app_px);
    let gap = APP_GAP * s;
    let room = (head.width - gap - app_w).max(head.width * TITLE_SHARE / 5);
    let title = text.ellipsize(c.title, room, true, title_px).into_owned();
    let used = match text.measure(&title, true, title_px) {
        0 => 0,
        w => w + gap,
    };
    let app_shown = text
        .ellipsize(&app, head.width - used, false, app_px)
        .into_owned();
    let app_box = Rect::new(head.x + used, head.y, head.width - used, head.height);
    let lift = text
        .baseline(head, true, title_px)
        .zip(text.baseline(app_box, false, app_px))
        .map_or(0, |(t, a)| t - a);
    let app_box = Rect::new(app_box.x, app_box.y + lift, app_box.width, app_box.height);

    let reply = c.reply.filter(|_| expanded);
    let actions = match (expanded, reply) {
        (true, None) => c.actions,
        _ => &[][..],
    };
    let text_bottom = body_y + lines.len() as i32 * LINE_H;
    let row_y = text_bottom.max(DROPLET_IN + DROPLET_SLOT) + ROW_GAP;
    let row = action_row(
        text,
        actions,
        reply,
        (text_x, DROPLET_W - DROPLET_PAD, row_y),
        s,
        pill_px,
        at,
    );
    let h = if row.shown {
        DROPLET_H.max(row_y + PILL_H + DROPLET_PAD)
    } else if expanded {
        DROPLET_H.max(text_bottom + DROPLET_PAD)
    } else {
        DROPLET_H
    };

    let sheet = at(0, 0, DROPLET_W, h);
    let (close_x, close_y) = if expanded { open_close } else { shut_close };
    let parts = NotifyParts {
        bubble: sheet,
        title: head,
        body: at(text_x, body_y, text_w, lines.len().max(1) as i32 * LINE_H),
        close: at(close_x, close_y, CLOSE, CLOSE),
        buttons: row.buttons,
        more: row.more,
        field: row.field,
        send: row.send,
        placeholder: reply.map_or_else(String::new, |r| r.placeholder.to_string()),
        send_label: reply.map_or_else(String::new, |r| r.send.to_string()),
    };
    SheetLayout {
        width: (m.left + DROPLET_W + m.right) * s,
        height: (m.top + m.bottom + h) * s,
        shape: SheetShape::Droplet,
        expanded,
        sheet,
        footprint: sheet,
        icon: at(DROPLET_IN, DROPLET_IN, DROPLET_SLOT, DROPLET_SLOT),
        well: Well::Round,
        badge: None,
        picture: c
            .has_picture
            .then(|| at(picture_x, DROPLET_IN, DROPLET_SLOT, DROPLET_SLOT)),
        parts,
        lines,
        app_px,
        title_px,
        body_px,
        app_line: app_shown,
        app_box,
        title,
        labels: row.labels,
        line_h: LINE_H * s,
        pill_px,
    }
}

/// `expanded` at `t` (0..1) of the way open from `collapsed`: its sheet, footprint and canvas
/// as tall as the lerp of the two, less the parts that would end below the sheet.
pub fn open_layout(collapsed: &SheetLayout, expanded: &SheetLayout, t: f64) -> SheetLayout {
    let t = t.clamp(0.0, 1.0);
    let lerp = |a: i32, b: i32| (f64::from(a) + f64::from(b - a) * t).round() as i32;
    let mut lay = expanded.clone();
    lay.sheet.height = lerp(collapsed.sheet.height, expanded.sheet.height);
    lay.footprint.height = lerp(collapsed.footprint.height, expanded.footprint.height);
    lay.height = lerp(collapsed.height, expanded.height);
    let bottom = lay.sheet.bottom();
    let fits = |r: &Rect| r.bottom() <= bottom;
    let p = &mut lay.parts;
    p.bubble = lay.sheet;
    let kept: Vec<(Rect, String)> = p
        .buttons
        .drain(..)
        .zip(lay.labels.drain(..))
        .filter(|(r, _)| fits(r))
        .collect();
    (p.buttons, lay.labels) = kept.into_iter().unzip();
    p.more = p.more.filter(fits);
    p.send = p.send.filter(fits);
    p.field = p.field.filter(fits);
    lay
}

/// The app icon and the notification's own picture, device px.
#[derive(Clone, Copy, Debug, Default)]
pub struct SheetIcons<'a> {
    pub app: Option<&'a Canvas>,
    pub picture: Option<&'a Canvas>,
}

fn opaque_bounds(c: &Canvas) -> Rect {
    let (w, h) = (c.width as i32, c.height as i32);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if c.get(x, y) >> 24 != 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    if x1 <= x0 {
        return Rect::new(0, 0, w, h);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// `src` scaled down to fit `w` x `h`, or covering it and cut to it with `cover`.
fn fitted(src: &Canvas, w: i32, h: i32, cover: bool) -> Canvas {
    let (sw, sh) = (src.width as i32, src.height as i32);
    if (sw, sh) == (w, h) || sw <= 0 || sh <= 0 {
        return src.clone();
    }
    let (kx, ky) = (f64::from(w) / f64::from(sw), f64::from(h) / f64::from(sh));
    let k = if cover {
        kx.max(ky)
    } else {
        kx.min(ky).min(1.0)
    };
    let (nw, nh) = (
        (f64::from(sw) * k).round() as i32,
        (f64::from(sh) * k).round() as i32,
    );
    let out = src.resample_box(nw.max(1) as u32, nh.max(1) as u32);
    if !cover {
        return out;
    }
    out.copy_rect(Rect::new((nw - w) / 2, (nh - h) / 2, w, h))
}

fn round_corners(mut c: Canvas, r: f64) -> Canvas {
    let (w, h) = (c.width as i32, c.height as i32);
    for y in 0..h {
        for x in 0..w {
            let cov = rounded(f64::from(x), f64::from(y), f64::from(w), f64::from(h), r);
            if cov < 1.0 {
                let p = c.get(x, y);
                c.put_raw(x, y, pack(scale(unpack(p), cov)));
            }
        }
    }
    c
}

fn blit_centred(c: &mut Canvas, r: Rect, icon: &Canvas) {
    let i = fitted(icon, r.width, r.height, false);
    let (iw, ih) = (i.width as i32, i.height as i32);
    c.blit_over(r.x + (r.width - iw) / 2, r.y + (r.height - ih) / 2, &i);
}

/// Blit the well `kind` under the slot `r`, its shadow room centred on it.
fn blit_well(c: &mut Canvas, look: &NotifyLook, kind: Well, r: Rect, s: i32) {
    let w = (look.well)(look, kind, r.width / s, s);
    let pad = (w.width as i32 - r.width) / 2;
    c.blit_over(r.x - pad, r.y - pad, &w);
}

fn pill_state(hit: NotifyHit, pushed: Option<NotifyHit>, hot: Option<NotifyHit>) -> State {
    if pushed == Some(hit) {
        State::Pressed
    } else if hot == Some(hit) {
        State::Hot
    } else {
        State::Normal
    }
}

/// Draw `lay` with `pushed` held down and `hot` lit; while `hot` is set the rim lifts and the
/// close button shows. `tint` is for a look that colours the sheet.
#[expect(clippy::too_many_arguments)]
pub fn draw_sheet(
    theme: &Theme,
    text: &TextRenderer,
    lay: &SheetLayout,
    icons: SheetIcons,
    tint: Option<Color>,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
) -> Canvas {
    SheetGlass::default().draw(theme, text, lay, icons, tint, pushed, hot, reply)
}

/// A sheet's glass under each rim, kept while the sheet keeps its size, shape, tint and scale, so
/// a redraw for its text, caret or a lit pill paints only the parts. A host keeps one per sheet.
#[derive(Default)]
pub struct SheetGlass {
    kept: Vec<(GlassKey, Canvas)>,
    builds: u32,
    /// The glass a Droplet at rest shows, kept through [`SheetGlass::keep_ends`].
    rest: Option<GlassKey>,
}

#[derive(Clone, Copy, PartialEq)]
struct GlassKey {
    spec: SheetSpec,
    scale: i32,
    dark: bool,
}

impl GlassKey {
    fn new(lay: &SheetLayout, tint: Option<Color>, lit: bool, s: i32, dark: bool) -> GlassKey {
        GlassKey {
            spec: sheet_spec(lay, tint, lit, s),
            scale: s,
            dark,
        }
    }

    fn lit(&self) -> bool {
        self.spec.hot_close
    }

    fn under_either_rim(&self, other: &GlassKey) -> bool {
        let lit = |k: &GlassKey| GlassKey {
            spec: SheetSpec {
                hot_close: true,
                ..k.spec
            },
            ..*k
        };
        lit(self) == lit(other)
    }
}

impl SheetGlass {
    /// [`draw_sheet`] on the kept glass.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        theme: &Theme,
        text: &TextRenderer,
        lay: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        pushed: Option<NotifyHit>,
        hot: Option<NotifyHit>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Canvas {
        let Some(look) = notify_look(theme) else {
            return Canvas::new(lay.width.max(1) as u32, lay.height.max(1) as u32);
        };
        let s = theme.scale.max(1);
        let key = GlassKey::new(lay, tint, hot.is_some(), s, look.dark);
        self.kept.retain(|(k, _)| k.under_either_rim(&key));
        let mut c = self.get(&look, key);
        draw_parts(&mut c, theme, text, &look, lay, icons, pushed, hot, reply);
        c
    }

    fn get(&mut self, look: &NotifyLook, key: GlassKey) -> Canvas {
        self.ensure(look, key);
        self.find(&key)
            .cloned()
            .unwrap_or_else(|| unreachable!("painted above"))
    }

    fn ensure(&mut self, look: &NotifyLook, key: GlassKey) {
        if self.find(&key).is_none() {
            let c = (look.sheet)(look, &key.spec, key.scale);
            self.builds += 1;
            self.kept.push((key, c));
        }
    }

    fn find(&self, key: &GlassKey) -> Option<&Canvas> {
        self.kept.iter().find(|(k, _)| k == key).map(|(_, c)| c)
    }

    /// Keeps only the glass of `ends` under the rim `lit`, and the glass at rest.
    fn keep_ends(&mut self, ends: [GlassKey; 2], lit: bool) {
        let rest = self.rest;
        self.kept.retain(|(k, _)| {
            ends.iter().any(|e| k.under_either_rim(e)) && (k.lit() == lit || Some(*k) == rest)
        });
    }

    /// How many times the glass was painted.
    pub fn builds(&self) -> u32 {
        self.builds
    }

    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.kept.iter().map(|(_, c)| c.pixels.len() * 4).sum()
    }
}

fn sheet_spec(lay: &SheetLayout, tint: Option<Color>, lit: bool, s: i32) -> SheetSpec {
    let design = |r: Rect| Rect::new(r.x / s, r.y / s, r.width / s, r.height / s);
    let sh = design(lay.sheet);
    let breakout = (lay.footprint != lay.sheet).then(|| {
        let i = design(lay.icon);
        Rect::new(i.x - sh.x, i.y - sh.y, i.width, i.height)
    });
    SheetSpec {
        shape: lay.shape,
        size: (sh.width, sh.height),
        tint,
        hot_close: lit,
        breakout,
    }
}

/// A sheet `t` (0..1) of the way from `collapsed` to `expanded`, drawn once; see
/// [`SheetOpening::draw`].
#[expect(clippy::too_many_arguments)]
pub fn draw_sheet_opening(
    theme: &Theme,
    text: &TextRenderer,
    collapsed: &SheetLayout,
    expanded: &SheetLayout,
    t: f64,
    icons: SheetIcons,
    tint: Option<Color>,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
) -> Canvas {
    SheetOpening::default().draw(
        &mut SheetGlass::default(),
        theme,
        text,
        collapsed,
        expanded,
        t,
        icons,
        tint,
        pushed,
        hot,
        reply,
    )
}

/// A Droplet's parts for opening, built when its content changes; each end's glass comes from
/// its [`SheetGlass`]. A host keeps one per Droplet and warms it before the pointer arrives.
#[derive(Default)]
pub struct SheetOpening {
    layers: Option<Layers>,
    /// Painted here, not by [`build_warm`].
    builds: u32,
    /// The key and rim of the [`WarmJob`] out for these layers.
    asked: Option<(OpeningKey, bool)>,
    /// Content whose job panicked on its worker; only [`SheetOpening::warm`] paints it.
    failed: Option<OpeningKey>,
}

/// What [`build_warm`] needs to warm a [`SheetOpening`], owned, so it can go to a worker thread.
pub struct WarmJob {
    theme: Theme,
    key: OpeningKey,
    lit: bool,
}

/// What [`build_warm`] painted, for [`SheetOpening::install`].
pub struct WarmLayers {
    layers: Layers,
    glass: Vec<(GlassKey, Canvas)>,
    lit: bool,
}

/// Paints `job`'s layers with `text`, a renderer loaded from `job`'s fonts ([`WarmJob::fonts`]).
pub fn build_warm(job: WarmJob, text: &TextRenderer) -> WarmLayers {
    let WarmJob { theme, key, lit } = job;
    let look = notify_look(&theme).expect("a job is only made under a look with sheets");
    let (app, picture) = (key.app.clone(), key.picture.clone());
    let icons = SheetIcons {
        app: app.as_ref(),
        picture: picture.as_ref(),
    };
    let layers = Layers::parts(key, &theme, text, &look, icons, None);
    let glass = layers
        .key
        .ends(lit)
        .map(|k| (k, (look.sheet)(&look, &k.spec, k.scale)))
        .into();
    WarmLayers { layers, glass, lit }
}

impl WarmJob {
    pub fn fonts(&self) -> &crate::theme::FontConfig {
        &self.theme.fonts
    }
}

/// Leaves out the app line, which ages: a frame draws it on top.
#[derive(Clone, PartialEq)]
struct OpeningKey {
    collapsed: SheetLayout,
    expanded: SheetLayout,
    scale: i32,
    dark: bool,
    app: Option<Canvas>,
    picture: Option<Canvas>,
    tint: Option<Color>,
    reply: Option<(TextEdit, Caret, Option<Preedit>)>,
}

impl OpeningKey {
    fn new(
        theme: &Theme,
        look: &NotifyLook,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> OpeningKey {
        let bare = |lay: &SheetLayout| SheetLayout {
            app_line: String::new(),
            ..lay.clone()
        };
        OpeningKey {
            collapsed: bare(collapsed),
            expanded: bare(expanded),
            scale: theme.scale.max(1),
            dark: look.dark,
            app: icons.app.cloned(),
            picture: icons.picture.cloned(),
            tint,
            reply: reply.map(|(e, c, p)| (e.clone(), c, p.cloned())),
        }
    }

    /// The collapsed and expanded glass under the rim `lit`.
    fn ends(&self, lit: bool) -> [GlassKey; 2] {
        [&self.collapsed, &self.expanded]
            .map(|lay| GlassKey::new(lay, self.tint, lit, self.scale, self.dark))
    }
}

/// Which of the expanded pill row fits the sheet part way open.
type Kept = (usize, bool, bool, bool);

fn kept(p: &NotifyParts) -> Kept {
    (
        p.buttons.len(),
        p.more.is_some(),
        p.send.is_some(),
        p.field.is_some(),
    )
}

struct Layers {
    key: OpeningKey,
    /// The collapsed parts on clear, and the expanded ones, unlit, for each `Kept` an opening
    /// passes through.
    shut_parts: Canvas,
    open_parts: Vec<(Kept, Canvas)>,
}

impl SheetOpening {
    /// Builds what a frame needs under the rim `lit` (the pointer on the sheet) unless it is
    /// built already; true when it painted.
    #[expect(clippy::too_many_arguments)]
    pub fn warm(
        &mut self,
        glass: &mut SheetGlass,
        theme: &Theme,
        text: &TextRenderer,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
        lit: bool,
    ) -> bool {
        let Some(look) = notify_look(theme) else {
            return false;
        };
        let key = OpeningKey::new(theme, &look, collapsed, expanded, icons, tint, reply);
        let glass_builds = glass.builds;
        let mut built = false;
        if self.layers.as_ref().is_none_or(|l| l.key != key) {
            if self.asked.as_ref().is_some_and(|(k, _)| *k != key) {
                self.asked = None;
            }
            self.layers = Some(Layers::parts(key, theme, text, &look, icons, reply));
            built = true;
        }
        let Some(layers) = self.layers.as_ref() else {
            unreachable!("built above");
        };
        for k in layers.key.ends(lit) {
            glass.ensure(&look, k);
        }
        built |= glass.builds > glass_builds;
        self.builds += u32::from(built);
        built
    }

    /// What [`SheetOpening::warm`] would paint, as a job for [`build_warm`] elsewhere; `None`
    /// when it is painted already or out as a job.
    #[expect(clippy::too_many_arguments)]
    pub fn warm_job(
        &mut self,
        glass: &SheetGlass,
        theme: &Theme,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        icons: SheetIcons,
        tint: Option<Color>,
        lit: bool,
    ) -> Option<WarmJob> {
        let look = notify_look(theme)?;
        let key = OpeningKey::new(theme, &look, collapsed, expanded, icons, tint, None);
        let ready = self.layers.as_ref().is_some_and(|l| l.key == key)
            && key.ends(lit).iter().all(|k| glass.find(k).is_some());
        if ready
            || self.failed.as_ref() == Some(&key)
            || self
                .asked
                .as_ref()
                .is_some_and(|(k, r)| *k == key && *r == lit)
        {
            return None;
        }
        self.asked = Some((key.clone(), lit));
        Some(WarmJob {
            theme: theme.clone(),
            key,
            lit,
        })
    }

    /// Takes what [`build_warm`] painted, unless the layers were since painted for other content;
    /// the other rim's glass goes, but for the glass at rest.
    pub fn install(&mut self, glass: &mut SheetGlass, built: WarmLayers) {
        let WarmLayers {
            layers,
            glass: ends,
            lit,
        } = built;
        let asked = self.asked.as_ref().is_some_and(|(k, _)| *k == layers.key);
        if asked {
            self.asked = None;
        }
        let same = self.layers.as_ref().is_some_and(|l| l.key == layers.key);
        if !same && !asked {
            return;
        }
        let keys = layers.key.ends(lit);
        if !same {
            self.layers = Some(layers);
        }
        for (k, c) in ends {
            if glass.find(&k).is_none() {
                glass.kept.push((k, c));
            }
        }
        glass.keep_ends(keys, lit);
    }

    /// The job out was lost with its worker; the next [`SheetOpening::warm_job`] asks again.
    pub fn forget_job(&mut self) {
        self.asked = None;
    }

    /// The job out panicked; [`SheetOpening::warm_job`] asks nothing more for its content.
    pub fn job_failed(&mut self) {
        self.failed = self.asked.take().map(|(k, _)| k);
    }

    /// How many times these layers were painted on the caller's thread.
    pub fn builds(&self) -> u32 {
        self.builds
    }

    /// The sheet `t` (0..1) of the way from `collapsed` to `expanded` on `glass`: either end as
    /// drawn at rest, and between them the blend of [`blend_opening`] with the app line on top.
    #[expect(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        glass: &mut SheetGlass,
        theme: &Theme,
        text: &TextRenderer,
        collapsed: &SheetLayout,
        expanded: &SheetLayout,
        t: f64,
        icons: SheetIcons,
        tint: Option<Color>,
        pushed: Option<NotifyHit>,
        hot: Option<NotifyHit>,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Canvas {
        let open = open_layout(collapsed, expanded, t);
        let Some(look) = notify_look(theme) else {
            return Canvas::new(open.width.max(1) as u32, open.height.max(1) as u32);
        };
        let s = theme.scale.max(1);
        let t = t.clamp(0.0, 1.0);
        let lit = hot.is_some();
        let ends = [collapsed, expanded].map(|lay| GlassKey::new(lay, tint, lit, s, look.dark));
        if t <= 0.0 || t >= 1.0 {
            // At rest the next move needs the lit rim while collapsed, the plain one while open.
            let shut = t <= 0.0;
            let (lay, rest) = if shut {
                (collapsed, ends[0])
            } else {
                (expanded, ends[1])
            };
            let mut c = glass.get(&look, rest);
            glass.rest = Some(rest);
            glass.keep_ends(ends, shut);
            let (hot, reply) = if shut { (None, None) } else { (hot, reply) };
            draw_parts(&mut c, theme, text, &look, lay, icons, pushed, hot, reply);
            return c;
        }
        self.warm(
            glass, theme, text, collapsed, expanded, icons, tint, reply, lit,
        );
        glass.rest = None;
        glass.keep_ends(ends, lit);
        let Some(layers) = self.layers.as_mut() else {
            unreachable!("warmed above");
        };
        let under = glass_at(glass, &layers.key, lit, open.sheet, s);
        let want = kept(&open.parts);
        let i = match layers.open_parts.iter().position(|(k, _)| *k == want) {
            Some(i) => i,
            None => {
                let mut c =
                    Canvas::new(expanded.width.max(1) as u32, expanded.height.max(1) as u32);
                let bare = SheetLayout {
                    app_line: String::new(),
                    ..open.clone()
                };
                draw_parts(&mut c, theme, text, &look, &bare, icons, None, None, reply);
                layers.open_parts.push((want, c));
                layers.open_parts.len() - 1
            }
        };
        let mut open_parts = layers.open_parts[i].1.clone();
        let edit_empty = reply.is_none_or(|(e, ..)| e.text.trim().is_empty());
        relight(
            &mut open_parts,
            text,
            &look,
            &open,
            edit_empty,
            pushed,
            hot,
            s,
        );
        let r = f64::from(corner_radius((look.corner)(open.shape))) * f64::from(s);
        let mut c = blend_opening(
            under,
            &layers.shut_parts,
            &open_parts,
            open.sheet,
            r,
            opening_fades(t),
        );
        text.draw(
            &mut c,
            open.app_box,
            &open.app_line,
            false,
            look.inks.app,
            Align::Left,
            open.app_px,
        );
        c
    }

    #[cfg(test)]
    fn bytes(&self) -> usize {
        self.layers.as_ref().map_or(0, |l| {
            let parts = std::iter::once(&l.shut_parts).chain(l.open_parts.iter().map(|(_, c)| c));
            parts.map(|c| c.pixels.len() * 4).sum()
        })
    }
}

impl Layers {
    fn parts(
        key: OpeningKey,
        theme: &Theme,
        text: &TextRenderer,
        look: &NotifyLook,
        icons: SheetIcons,
        reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
    ) -> Layers {
        let (c, e) = (&key.collapsed, &key.expanded);
        let clear =
            |lay: &SheetLayout| Canvas::new(lay.width.max(1) as u32, lay.height.max(1) as u32);
        let mut shut_parts = clear(c);
        draw_parts(
            &mut shut_parts,
            theme,
            text,
            look,
            c,
            icons,
            None,
            None,
            None,
        );
        let mut open_parts: Vec<(Kept, Canvas)> = Vec::new();
        for i in 1..=100 {
            let open = open_layout(c, e, f64::from(i) / 100.0);
            let want = kept(&open.parts);
            if open_parts.iter().all(|(k, _)| *k != want) {
                let mut layer = clear(e);
                draw_parts(
                    &mut layer, theme, text, look, &open, icons, None, None, reply,
                );
                open_parts.push((want, layer));
            }
        }
        Layers {
            open_parts,
            shut_parts,
            key,
        }
    }
}

/// The glass under the rim `lit` for a sheet `sheet` tall: each end's caps kept and its middle
/// row stretched or its middle cut out, then mixed by how far the height has come.
fn glass_at(glass: &SheetGlass, key: &OpeningKey, lit: bool, sheet: Rect, s: i32) -> Canvas {
    let [kc, ke] = key.ends(lit);
    let (Some(c), Some(e)) = (glass.find(&kc), glass.find(&ke)) else {
        unreachable!("warmed before a frame");
    };
    let body = |lay: &SheetLayout| lay.sheet.height / s * s;
    let (hc, he, h) = (
        body(&key.collapsed),
        body(&key.expanded),
        sheet.height / s * s,
    );
    let height = c.height as i32 + (h - hc);
    let u = if he > hc {
        ((h - hc) * 256 / (he - hc)).clamp(0, 256) as u32
    } else {
        256
    };
    let (cut_c, cut_e) = (sheet.y + hc / 2, sheet.y + h / 2);
    let w = e.width as usize;
    let mut out = Canvas::new(e.width, height.max(1) as u32);
    fn row(c: &Canvas, y: i32) -> &[u32] {
        let w = c.width as usize;
        &c.pixels[y as usize * w..][..w]
    }
    for (y, to) in (0..height).zip(out.pixels.chunks_exact_mut(w)) {
        let yc = if y < cut_c {
            y
        } else if y < cut_c + (h - hc) {
            cut_c
        } else {
            y - (h - hc)
        };
        let ye = if y < cut_e { y } else { y + (he - h) };
        for ((to, &a), &b) in to.iter_mut().zip(row(c, yc)).zip(row(e, ye)) {
            *to = lerp_px(a, b, u);
        }
    }
    out
}

/// `a` with `u` 256ths of `b` in its place, per premultiplied channel.
fn lerp_px(a: u32, b: u32, u: u32) -> u32 {
    if a == b {
        return a;
    }
    (0..4).fold(0, |px, i| {
        let (ca, cb) = ((a >> (8 * i)) & 0xFF, (b >> (8 * i)) & 0xFF);
        px | (((ca * (256 - u) + cb * u + 128) >> 8) << (8 * i))
    })
}

/// `glass` with the collapsed parts `gone` and the expanded parts `come` of the way in, inside
/// the sheet `sh` of corner radius `r`; parts both draw alike stay solid.
fn blend_opening(
    mut out: Canvas,
    shut: &Canvas,
    open: &Canvas,
    sh: Rect,
    r: f64,
    (gone, come): (f64, f64),
) -> Canvas {
    let (w, h) = (f64::from(sh.width), f64::from(sh.height));
    let area = sh.intersect(&Rect::new(0, 0, out.width as i32, out.height as i32));
    let layer = |c: &Canvas, x: i32, y: i32| {
        if x < c.width as i32 && y < c.height as i32 {
            c.get(x, y)
        } else {
            0
        }
    };
    let k = 1.0 - gone - come;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let (pa, pb) = (layer(shut, x, y), layer(open, x, y));
            if pa == 0 && pb == 0 {
                continue;
            }
            let g = out.get(x, y);
            let on = |p: u32| {
                if p == 0 {
                    g
                } else {
                    pack(over(unpack(p), unpack(g)))
                }
            };
            let (a, b) = (on(pa), on(pb));
            let inside = rounded(f64::from(x - sh.x), f64::from(y - sh.y), w, h, r);
            if a == b && inside >= 1.0 {
                out.put_raw(x, y, a);
                continue;
            }
            let gp = unpack(g);
            let parts = if a == b {
                unpack(a)
            } else {
                let (a, b) = (unpack(a), unpack(b));
                let ch = |g: f64, a: f64, b: f64| g * k + a * gone + b * come;
                (
                    ch(gp.0, a.0, b.0),
                    std::array::from_fn(|i| ch(gp.1[i], a.1[i], b.1[i])),
                )
            };
            out.put_raw(x, y, pack(mix(gp, parts, inside)));
        }
    }
    out
}

/// The collapsed ink is gone by `SHUT_BY`, before the expanded ink, in from `OPEN_FROM`, is half
/// in; the two never add past one. Parts both layers draw alike stay solid throughout.
const SHUT_BY: f64 = 0.4;
const OPEN_FROM: f64 = 0.2;

/// Whether a sheet `t` (0..1) of the way open draws its close button.
pub fn opening_shows_close(t: f64) -> bool {
    opening_fades(t.clamp(0.0, 1.0)).1 > 0.0
}

/// How much of the collapsed and of the expanded parts show `t` (0..1) of the way open.
fn opening_fades(t: f64) -> (f64, f64) {
    (
        (1.0 - t / SHUT_BY).max(0.0),
        ((t - OPEN_FROM) / (1.0 - OPEN_FROM)).clamp(0.0, 1.0),
    )
}

#[expect(clippy::too_many_arguments)]
fn draw_parts(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    look: &NotifyLook,
    lay: &SheetLayout,
    icons: SheetIcons,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    reply: Option<(&TextEdit, Caret, Option<&Preedit>)>,
) {
    let s = theme.scale.max(1);

    blit_well(c, look, lay.well, lay.icon, s);
    let picture = lay.picture.zip(icons.picture);
    let in_slot = picture.is_some_and(|(r, _)| r == lay.icon);
    if let Some((r, pic)) = picture {
        let pic = pic.copy_rect(opaque_bounds(pic));
        let pic = fitted(&pic, r.width, r.height, true);
        let round = match lay.shape {
            SheetShape::Lozenge => SQUIRCLE_ROUND,
            SheetShape::Halo => {
                blit_well(c, look, Well::Round, r, s);
                0.5
            }
            SheetShape::Droplet => {
                blit_well(c, look, Well::Squircle, r, s);
                SQUIRCLE_ROUND
            }
        };
        c.blit_over(r.x, r.y, &round_corners(pic, round * f64::from(r.width)));
    }
    if let Some(app) = icons.app {
        match lay.badge.filter(|_| in_slot) {
            Some(b) => {
                blit_well(c, look, Well::Badge, b, s);
                blit_centred(c, b.inset(BADGE_ICON_IN * s), app);
            }
            None if !in_slot => blit_centred(c, lay.icon, app),
            None => {}
        }
    }

    let inks = look.inks;
    let left = Align::Left;
    text.draw(
        c,
        lay.app_box,
        &lay.app_line,
        false,
        inks.app,
        left,
        lay.app_px,
    );
    let tb = lay.parts.title;
    text.draw(c, tb, &lay.title, true, inks.title, left, lay.title_px);
    let body = lay.parts.body;
    for (i, line) in lay.lines.iter().enumerate() {
        let r = Rect::new(
            body.x,
            body.y + i as i32 * lay.line_h,
            body.width,
            lay.line_h,
        );
        text.draw(c, r, line, false, inks.text, left, lay.body_px);
    }

    let none = TextEdit::default();
    let (edit, caret, preedit) = reply.unwrap_or((&none, Caret::Unfocused, None));
    c.set_preedit(preedit.cloned());
    if let (Some(f), Some(inner)) = (lay.parts.field, field_text(lay, s)) {
        let face = (look.pill)(look, (f.width / s, f.height / s), State::Pressed, s);
        c.blit_over(f.x, f.y, &face);
        let mut cs = theme.colors.clone();
        (cs.button_text, cs.gray_text) = (inks.title, inks.app);
        let p = &Paint {
            cs: &cs,
            ..theme.paint()
        };
        let px = lay.body_px;
        let composing = preedit.is_some() && caret.focused();
        if edit.text.is_empty() && !composing && !lay.parts.placeholder.is_empty() {
            let shown = text.ellipsize(&lay.parts.placeholder, inner.width, false, px);
            text.draw(c, inner, &shown, false, inks.app, left, px);
        }
        let first = edit_first_visible(text, edit, inner.width, px, 0);
        draw_edit(c, text, edit, inner, px, p, caret, first);
    }

    let edit_empty = edit.text.trim().is_empty();
    for (r, label, state) in pill_row(text, lay, edit_empty, pushed, hot, s) {
        draw_pill(c, text, look, lay, r, &label, state, s);
    }
    if hot.is_some() {
        draw_close(c, look, lay, pushed, hot, s);
    }
}

/// Each pill on `lay` with its label and state.
fn pill_row(
    text: &TextRenderer,
    lay: &SheetLayout,
    edit_empty: bool,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    s: i32,
) -> Vec<(Rect, String, State)> {
    let mut row: Vec<_> = lay
        .parts
        .buttons
        .iter()
        .zip(&lay.labels)
        .enumerate()
        .map(|(i, (r, label))| {
            (
                *r,
                label.clone(),
                pill_state(NotifyHit::Button(i), pushed, hot),
            )
        })
        .collect();
    if let Some(r) = lay.parts.more {
        row.push((
            r,
            NOTIFY_MORE.to_string(),
            pill_state(NotifyHit::More, pushed, hot),
        ));
    }
    if let Some(r) = lay.parts.send {
        let state = if edit_empty {
            State::Disabled
        } else {
            pill_state(NotifyHit::Send, pushed, hot)
        };
        let room = r.width - 2 * PILL_PAD * s;
        let label = text.ellipsize(&lay.parts.send_label, room, true, lay.pill_px);
        row.push((r, label.into_owned(), state));
    }
    row
}

#[expect(clippy::too_many_arguments)]
fn draw_pill(
    c: &mut Canvas,
    text: &TextRenderer,
    look: &NotifyLook,
    lay: &SheetLayout,
    r: Rect,
    label: &str,
    state: State,
    s: i32,
) {
    let face = (look.pill)(look, (r.width / s, r.height / s), state, s);
    c.blit_over(r.x, r.y, &face);
    let ink = if state == State::Disabled {
        look.inks.app
    } else {
        look.inks.title
    };
    text.draw(c, r, label, true, ink, Align::Center, lay.pill_px);
}

fn draw_close(
    c: &mut Canvas,
    look: &NotifyLook,
    lay: &SheetLayout,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    s: i32,
) {
    let r = lay.parts.close;
    let state = pill_state(NotifyHit::Close, pushed, hot);
    c.blit_over(r.x, r.y, &(look.close)(look, state, s));
}

/// `parts`, drawn unlit by [`draw_parts`] on clear, as the pointer lights them: each pill whose
/// state differs is drawn again on its cleared rect, and the close button goes on top.
#[expect(clippy::too_many_arguments)]
fn relight(
    parts: &mut Canvas,
    text: &TextRenderer,
    look: &NotifyLook,
    lay: &SheetLayout,
    edit_empty: bool,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    s: i32,
) {
    let unlit = pill_row(text, lay, edit_empty, None, None, s);
    let lit = pill_row(text, lay, edit_empty, pushed, hot, s);
    for ((r, label, state), (.., was)) in lit.into_iter().zip(unlit) {
        if state == was {
            continue;
        }
        let area = r.intersect(&Rect::new(0, 0, parts.width as i32, parts.height as i32));
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                parts.put_raw(x, y, 0);
            }
        }
        draw_pill(parts, text, look, lay, r, &label, state, s);
    }
    if hot.is_some() {
        draw_close(parts, look, lay, pushed, hot, s);
    }
}

fn field_text(lay: &SheetLayout, s: i32) -> Option<Rect> {
    let d = FIELD_TEXT_IN * s;
    lay.parts
        .field
        .map(|f| Rect::new(f.x + d, f.y, f.width - 2 * d, f.height))
}

/// A press on the sheet's reply field at canvas x `x`: it takes the keyboard and the
/// caret goes to the nearest character boundary.
pub fn sheet_click(
    lay: &SheetLayout,
    theme: &Theme,
    text: &TextRenderer,
    reply: &mut ReplyField,
    x: i32,
) {
    reply.click_text(text, field_text(lay, theme.scale.max(1)), lay.body_px, x);
}

/// `None` on the shadow margin and beside a well standing out of the sheet. The close spot
/// takes the pointer whether or not the button shows.
pub fn sheet_hit(lay: &SheetLayout, x: i32, y: i32) -> Option<NotifyHit> {
    let on = lay.sheet.contains(x, y) || (lay.footprint != lay.sheet && lay.icon.contains(x, y));
    on.then(|| parts_hit(&lay.parts, x, y))
}

/// The sheet's `access` part ([`access_parts`]) for `lay` laid out at scale 1,
/// its canvas at `at` in the access root.
#[expect(clippy::too_many_arguments)]
pub fn access_sheet(
    key: impl std::hash::Hash,
    lay: &SheetLayout,
    at: (i32, i32),
    says: &NotifySays,
    reply: Option<&ReplyField>,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let parts = lay.parts.offset(at.0, at.1);
    access_parts(key, &parts, says, reply, text, px, tree, parent)
}

/// The chip's metrics, design px.
const CHIP_H: i32 = 28;
const CHIP_PX: u32 = 12;
const CHIP_PAD: i32 = 12;

pub fn chip_label(more: usize) -> String {
    format!("+{more} more")
}

/// The chip's size for `more` waiting, device px.
pub fn chip_layout(theme: &Theme, text: &TextRenderer, more: usize) -> (i32, i32) {
    let s = theme.scale.max(1);
    let label = text.measure(&chip_label(more), false, CHIP_PX * s as u32);
    (((label + s - 1) / s + 2 * CHIP_PAD) * s, CHIP_H * s)
}

pub fn draw_chip(theme: &Theme, text: &TextRenderer, more: usize, hot: bool) -> Canvas {
    let s = theme.scale.max(1);
    let (w, h) = chip_layout(theme, text, more);
    let Some(look) = notify_look(theme) else {
        return Canvas::new(w as u32, h as u32);
    };
    let state = if hot { State::Hot } else { State::Normal };
    let mut c = (look.chip)(&look, (w / s, h / s), state, s);
    let r = Rect::new(0, 0, w, h);
    let label = chip_label(more);
    let px = CHIP_PX * s as u32;
    text.draw(&mut c, r, &label, false, look.inks.text, Align::Center, px);
    c
}

/// A top-left, logical px.
pub type Point = (i32, i32);

pub const STACK_GAP: i32 = 8;
pub const STACK_INSET: i32 = 8;

/// Top-lefts for `footprints` (newest first, nearest the tray) and the `chip` past them, logical
/// px, from `usable`'s corner by the tray along `edge`, kept inside it.
pub fn place_stack(
    usable: Rect,
    footprints: &[(i32, i32)],
    chip: Option<(i32, i32)>,
    edge: Edge,
) -> (Vec<Point>, Option<Point>) {
    let (left, down) = match edge {
        Edge::Bottom | Edge::Right => (false, false),
        Edge::Left => (true, false),
        Edge::Top => (false, true),
    };
    let mut y = if down {
        usable.y + STACK_INSET
    } else {
        usable.bottom() - STACK_INSET
    };
    let mut place = |(w, h): (i32, i32)| {
        let x = if left {
            usable.x + STACK_INSET
        } else {
            usable.right() - STACK_INSET - w
        };
        let top = if down { y } else { y - h };
        y = if down {
            y + h + STACK_GAP
        } else {
            y - h - STACK_GAP
        };
        (
            x.min(usable.right() - w).max(usable.x),
            top.min(usable.bottom() - h).max(usable.y),
        )
    };
    let at = footprints.iter().map(|&f| place(f)).collect();
    (at, chip.map(place))
}

pub const RISE: i32 = 14;

/// A sheet `k` of the way in to `rest` and its opacity: scaled from `scale_from` about a centre
/// `rise` px toward the tray (`toward_tray` +1 down, -1 up).
pub fn pose(k: f64, rest: Rect, rise: i32, toward_tray: i32, scale_from: f64) -> (Rect, f32) {
    let k = k.clamp(0.0, 1.0);
    let f = scale_from + (1.0 - scale_from) * k;
    let (w, h) = (
        (f64::from(rest.width) * f).round(),
        (f64::from(rest.height) * f).round(),
    );
    let cx = f64::from(rest.x) + f64::from(rest.width) / 2.0;
    let cy = f64::from(rest.y)
        + f64::from(rest.height) / 2.0
        + f64::from(rise) * (1.0 - k) * f64::from(toward_tray);
    let r = Rect::new(
        (cx - w / 2.0).round() as i32,
        (cy - h / 2.0).round() as i32,
        w as i32,
        h as i32,
    );
    (r, k as f32)
}

const BEAD: i32 = 12;
/// The drip holds its bead until `BEAD_UNTIL`, then swells; the bead is gone `BEAD_FADE` into
/// the swell.
const BEAD_UNTIL: f64 = 0.25;
const BEAD_FADE: f64 = 0.3;

/// The drip's duration for a sheet whose plain arrival takes `open_ms`.
pub fn drip_ms(open_ms: u32) -> u32 {
    open_ms * 12 / 5
}

/// A droplet `k` of the way in: a bead at the tray-side cap that swells with an overshoot into
/// `rest`. `tray_right` is true for every edge but Left.
pub fn drip_pose(k: f64, rest: Rect, tray_right: bool) -> (Rect, f32) {
    let k = k.clamp(0.0, 1.0);
    let opacity = (k / 0.1).min(1.0) as f32;
    if k >= 1.0 {
        return (rest, 1.0);
    }
    let cy = rest.y + rest.height / 2;
    let bead_x = if tray_right {
        rest.right() - rest.height / 2 - BEAD / 2
    } else {
        rest.x + rest.height / 2 - BEAD / 2
    };
    if k < BEAD_UNTIL {
        return (Rect::new(bead_x, cy - BEAD / 2, BEAD, BEAD), opacity);
    }
    let s = ((k - BEAD_UNTIL) / (1.0 - BEAD_UNTIL)) as f32;
    let e = f64::from(Ease::OutBack.apply(s));
    let e_h = f64::from(Ease::OutBack.apply((s - 0.08).max(0.0) / 0.92));
    let lerp = |a: i32, b: i32, t: f64| f64::from(a) + f64::from(b - a) * t;
    let w = lerp(BEAD, rest.width, e).round() as i32;
    let h = lerp(BEAD, rest.height, e_h).round() as i32;
    let x = if tray_right {
        lerp(bead_x + BEAD, rest.right(), e).round() as i32 - w
    } else {
        lerp(bead_x, rest.x, e).round() as i32
    };
    (Rect::new(x, cy - h / 2, w, h), opacity)
}

/// How much of a drip `k` of the way in shows as the bead, the sheet showing the rest.
pub fn drip_bead_share(k: f64) -> f32 {
    let swell = (k - BEAD_UNTIL) / (1.0 - BEAD_UNTIL);
    (1.0 - smooth(swell / BEAD_FADE)) as f32
}

/// The bead's body for a drip posed at `p` ([`drip_pose`]): round, as tall as `p`, at its
/// tray-side end.
pub fn drip_bead_at(p: Rect, tray_right: bool) -> Rect {
    let side = p.height.min(p.width);
    let x = if tray_right { p.right() - side } else { p.x };
    Rect::new(x, p.y + (p.height - side) / 2, side, side)
}

/// The glass bead a Droplet drips from and its body's rect in the canvas, device px.
pub fn drip_bead(theme: &Theme) -> Option<(Canvas, Rect)> {
    let look = notify_look(theme)?;
    let s = theme.scale.max(1);
    let c = (look.bead)(&look, BEAD, s);
    let side = BEAD * s;
    let at = |n: u32| (n as i32 - side) / 2;
    let body = Rect::new(at(c.width), at(c.height), side, side);
    Some((c, body))
}

/// The largest round corner table whose radius fits half of `height`; radius 6 below that.
pub fn drip_corner(height: i32) -> &'static [i32] {
    const R6: [i32; 5] = round_corner();
    const R12: [i32; 11] = round_corner();
    const R20: [i32; 19] = round_corner();
    const R34: [i32; 33] = round_corner();
    match height / 2 {
        34.. => &R34,
        20.. => &R20,
        12.. => &R12,
        _ => &R6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::ROOT;
    use crate::access::accesskit::{Live, Role};
    use crate::controls::tooltip::{NOTIFY_BODY_LINES, notify_layout};
    use crate::interface::Interface;
    use crate::interfaces::{theme_for, theme_with};
    use crate::theme::FontConfig;
    use crate::{blue, glass};

    fn text() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn content<'a>(app: &'a str, title: &'a str, body: &'a str) -> SheetContent<'a> {
        SheetContent {
            app,
            when: "now",
            title,
            body,
            actions: &[],
            reply: None,
            has_picture: false,
            has_app_icon: true,
        }
    }

    fn lozenge(c: &SheetContent, expanded: bool) -> SheetLayout {
        sheet_layout(
            &theme_for(Interface::Glass),
            &text(),
            SheetShape::Lozenge,
            c,
            expanded,
        )
        .unwrap()
    }

    fn glass(dark: bool, scale: i32) -> Theme {
        let g = glass::Settings {
            mode: if dark {
                glass::Mode::Dark
            } else {
                glass::Mode::Light
            },
            ..glass::Settings::default()
        };
        theme_with(Interface::Glass, blue::Variant::Blue, &g).scaled(scale)
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height].iter().chain(c.pixels.iter()).fold(
            0xcbf2_9ce4_8422_2325_u64,
            |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            },
        )
    }

    const REPLY: NotifyReply = NotifyReply {
        placeholder: "Write a reply",
        send: "Send",
    };

    #[test]
    fn a_lozenge_is_360_wide_with_a_46_px_icon_slot() {
        let lay = lozenge(&content("Mail", "New message", "Hi"), false);
        assert_eq!((lay.sheet.width, lay.footprint), (360, lay.sheet));
        assert_eq!(
            lay.icon,
            Rect::new(lay.sheet.x + 14, lay.sheet.y + 14, 46, 46)
        );
        assert_eq!(lay.well, Well::Squircle);
    }

    #[test]
    fn a_picture_puts_the_app_icon_in_an_18_px_badge() {
        let both = lozenge(
            &SheetContent {
                has_picture: true,
                has_app_icon: true,
                ..content("Chat", "Ana", "Lunch?")
            },
            false,
        );
        let b = both.badge.unwrap();
        assert_eq!((b.width, b.height), (18, 18));
        assert_eq!(
            (b.x + 9, b.y + 9),
            (both.icon.right() - 4, both.icon.bottom() - 4)
        );
        assert_eq!(both.picture, Some(both.icon));
        let only = lozenge(
            &SheetContent {
                has_picture: true,
                has_app_icon: false,
                ..content("Chat", "Ana", "Lunch?")
            },
            false,
        );
        assert_eq!(only.badge, None);
        let icon = lozenge(&content("Chat", "Ana", "Lunch?"), false);
        assert_eq!((icon.badge, icon.picture), (None, None));
    }

    #[test]
    fn the_close_spot_takes_a_tap_before_hover_shows_it() {
        let lay = lozenge(&content("Mail", "New", "Hi"), false);
        let c = lay.parts.close;
        assert_eq!(
            c,
            Rect::new(lay.sheet.right() - 34, lay.sheet.y + 12, 20, 20)
        );
        assert_eq!(sheet_hit(&lay, c.x + 2, c.y + 2), Some(NotifyHit::Close));
    }

    #[test]
    fn the_shadow_margin_takes_no_pointer() {
        let lay = lozenge(&content("Mail", "New", "Hi"), false);
        assert_eq!(sheet_hit(&lay, 0, 0), None);
        assert_eq!(
            sheet_hit(&lay, lay.sheet.x + 100, lay.sheet.y + 40),
            Some(NotifyHit::Body)
        );
    }

    #[test]
    fn title_is_bold_16_body_14_and_app_12() {
        let lay = lozenge(&content("Mail", "New", "Hi"), false);
        assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 16, 14));
    }

    #[test]
    fn the_body_wraps_to_the_line_cap() {
        let long = "word ".repeat(400);
        assert_eq!(
            lozenge(&content("Mail", "t", &long), false).lines.len(),
            NOTIFY_BODY_LINES
        );
    }

    #[test]
    fn a_short_sheet_is_as_tall_as_its_icon_slot_and_padding() {
        let lay = lozenge(&content("Mail", "New", ""), false);
        assert_eq!(lay.sheet.height, 14 + 46 + 14);
        let long = lozenge(&content("Mail", "New", &"word ".repeat(400)), false);
        assert!(long.sheet.height > lay.sheet.height + 9 * 14);
        assert_eq!(long.sheet.bottom(), long.parts.body.bottom() + 14);
    }

    #[test]
    fn pills_sit_right_aligned_under_the_body_and_a_fourth_brings_more() {
        let acts = ["Open", "Later"];
        let lay = lozenge(
            &SheetContent {
                actions: &acts,
                ..content("Mail", "New", "Hi")
            },
            false,
        );
        let [a, b] = lay.parts.buttons[..] else {
            panic!("two pills: {:?}", lay.parts.buttons)
        };
        assert_eq!((a.height, b.height), (26, 26));
        assert_eq!((b.right(), a.right() + 6), (lay.sheet.right() - 14, b.x));
        assert!(a.y > lay.parts.body.bottom());
        assert_eq!(lay.sheet.bottom(), a.bottom() + 14);
        assert_eq!(
            sheet_hit(&lay, b.x + 3, b.y + 3),
            Some(NotifyHit::Button(1))
        );
        let four = ["A", "B", "C", "D"];
        let more = lozenge(
            &SheetContent {
                actions: &four,
                ..content("Mail", "New", "Hi")
            },
            false,
        );
        assert_eq!(more.parts.buttons.len(), 3);
        let m = more.parts.more.unwrap();
        assert_eq!(m.right(), more.sheet.right() - 14);
    }

    #[test]
    fn expanded_the_reply_row_replaces_the_pills() {
        let acts = ["Reply"];
        let c = SheetContent {
            actions: &acts,
            reply: Some(REPLY),
            ..content("Chat", "Ana", "Lunch?")
        };
        let shut = lozenge(&c, false);
        assert_eq!((shut.parts.buttons.len(), shut.parts.field), (1, None));
        let open = lozenge(&c, true);
        assert!(open.parts.buttons.is_empty());
        let (f, s) = (open.parts.field.unwrap(), open.parts.send.unwrap());
        assert_eq!(s.right(), open.sheet.right() - 14);
        assert_eq!((f.x, f.right() + 6), (open.icon.right() + 12, s.x));
        assert_eq!(open.parts.send_label, "Send");
        assert_eq!(sheet_hit(&open, f.x + 4, f.y + 4), Some(NotifyHit::Field));
    }

    fn picture_sheet(pic: &Canvas, app: Option<&Canvas>, with_pic: bool) -> (SheetLayout, Canvas) {
        let theme = glass(false, 1);
        let t = text();
        let c = SheetContent {
            has_picture: true,
            has_app_icon: app.is_some(),
            ..content("Chat", "Ana", "Lunch?")
        };
        let lay = sheet_layout(&theme, &t, SheetShape::Lozenge, &c, false).unwrap();
        let icons = SheetIcons {
            app,
            picture: with_pic.then_some(pic),
        };
        let drawn = draw_sheet(&theme, &t, &lay, icons, None, None, None, None);
        (lay, drawn)
    }

    #[test]
    fn a_letterboxed_wide_picture_fills_the_slot() {
        let mut pic = Canvas::new(96, 96);
        pic.fill_rect(Rect::new(0, 21, 96, 54), Color::rgb(0xE0, 0x6A, 0x2E));
        let (lay, c) = picture_sheet(&pic, None, true);
        let i = lay.icon;
        assert_eq!(
            c.get(i.x + 23, i.y + 1),
            0xFFE0_6A2E,
            "the top edge is picture"
        );
        assert_eq!(
            c.get(i.x + 23, i.bottom() - 2),
            0xFFE0_6A2E,
            "so is the foot"
        );
        assert!(c.get(i.x, i.y) >> 24 < 0xFF, "its corners are rounded");
    }

    #[test]
    fn without_its_picture_the_app_icon_fills_the_slot_unbadged() {
        let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 46);
        let (lay, c) = picture_sheet(&app, Some(&app), false);
        let (i, b) = (lay.icon, lay.badge.unwrap());
        assert_eq!(c.get(i.x + 23, i.y + 23), 0xFF2E_7DE0);
        assert_eq!(c.get(b.x + 9, b.y), 0xFF2E_7DE0, "no badge ring");
    }

    #[test]
    fn a_click_in_the_reply_field_places_the_caret() {
        let (theme, t) = (theme_for(Interface::Glass), text());
        let acts = ["Reply"];
        let c = SheetContent {
            actions: &acts,
            reply: Some(REPLY),
            ..content("Chat", "Ana", "Lunch?")
        };
        let lay = lozenge(&c, true);
        let f = lay.parts.field.unwrap();
        let mut reply = ReplyField {
            edit: TextEdit::new("Sure thing", false),
            focused: false,
        };
        sheet_click(&lay, &theme, &t, &mut reply, f.x + 2);
        assert!(reply.focused);
        assert_eq!(reply.edit.caret(), 0);
        sheet_click(&lay, &theme, &t, &mut reply, f.right() - 2);
        assert_eq!(reply.edit.caret(), "Sure thing".len());
        let sure = t.measure("Sure", false, lay.body_px);
        sheet_click(&lay, &theme, &t, &mut reply, f.x + 13 + sure);
        assert_eq!(
            reply.edit.caret(),
            4,
            "14 px text from inside the rounded end"
        );
    }

    #[test]
    fn a_balloons_parts_are_its_rects() {
        let lay = notify_layout(
            &Theme::default(),
            &text(),
            "t",
            "b",
            &["Open", "Later"],
            None,
        );
        let p = lay.parts();
        assert_eq!(
            (p.close, &p.buttons, p.more),
            (lay.close, &lay.buttons, lay.more)
        );
    }

    #[test]
    fn every_sheet_is_a_live_alert_naming_its_parts() {
        let t = text();
        let acts = ["Reply", "Later"];
        let lay = lozenge(
            &SheetContent {
                actions: &acts,
                ..content("Chat", "Ana", "Lunch?")
            },
            false,
        );
        let says = NotifySays {
            app: "Chat",
            title: "Ana",
            body: "Lunch?",
            actions: &acts,
            opens: false,
            urgent: false,
        };
        let mut tree = Tree::window("", (lay.width, lay.height));
        let alert = access_sheet(1, &lay, (0, 0), &says, None, &t, 12, &mut tree, ROOT);
        let n = tree.get(alert).unwrap();
        assert_eq!(n.role(), Role::Alert);
        assert_eq!(n.label(), Some("Chat: Ana. Lunch?"));
        assert_eq!(n.live(), Some(Live::Polite));
        assert_eq!(n.bounds(), Some(crate::access::bounds(lay.sheet)));
        let kids: Vec<(Role, String)> = n
            .children()
            .iter()
            .map(|&k| {
                let k = tree.get(k).unwrap();
                (
                    k.role(),
                    k.label().or(k.value()).unwrap_or_default().to_string(),
                )
            })
            .collect();
        let named = |r: Role, s: &str| (r, s.to_string());
        assert_eq!(
            kids,
            [
                named(Role::Label, "Ana"),
                named(Role::Label, "Lunch?"),
                named(Role::Button, "Reply"),
                named(Role::Button, "Later"),
                named(Role::Button, "Close"),
            ]
        );
        let mut tree = Tree::window("", (800, 600));
        let alert = access_sheet(1, &lay, (300, 200), &says, None, &t, 12, &mut tree, ROOT);
        let moved = Rect::new(lay.sheet.x + 300, lay.sheet.y + 200, 360, lay.sheet.height);
        assert_eq!(
            tree.get(alert).unwrap().bounds(),
            Some(crate::access::bounds(moved)),
            "at the sheet canvas's offset"
        );
    }

    fn swatch(c: Color, side: i32) -> Canvas {
        let mut k = Canvas::new(side as u32, side as u32);
        k.fill_rect(Rect::new(0, 0, side, side), c);
        k
    }

    fn pin_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
        let s = theme.scale;
        let acts = ["Reply", "Later"];
        let base = content("Chat", "Ana", "Lunch at noon? The usual place.");
        let (c, expanded, hot) = match case {
            0 => (base, false, None),
            1 => (base, false, Some(NotifyHit::Close)),
            2 => (
                SheetContent {
                    actions: &acts,
                    has_picture: true,
                    ..base
                },
                false,
                Some(NotifyHit::Button(0)),
            ),
            _ => (
                SheetContent {
                    actions: &acts,
                    reply: Some(REPLY),
                    ..base
                },
                true,
                Some(NotifyHit::Field),
            ),
        };
        let lay = sheet_layout(theme, t, SheetShape::Lozenge, &c, expanded).unwrap();
        let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 46 * s);
        let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 46 * s);
        let icons = SheetIcons {
            app: Some(&app),
            picture: c.has_picture.then_some(&picture),
        };
        let typed = TextEdit::new("Sounds good", false);
        let reply = (case == 3).then_some((&typed, Caret::Shown, None));
        draw_sheet(theme, t, &lay, icons, None, None, hot, reply)
    }

    const CASES: [&str; 4] = [
        "resting",
        "hot close",
        "two actions, picture with badge",
        "reply row",
    ];

    const LOZENGE_PINS: [u64; 16] = [
        0x841f8deb53b6c29b,
        0x139b3f3dd55a2564,
        0x8ece672977df1c73,
        0x37440ad983172b73,
        0x27f9c499ad78b0b4,
        0x00fc97110915fe6c,
        0xc15a3106b27fa121,
        0x53ffc6ce7a94c962,
        0xeb8e2b0c83168d0b,
        0x6ee3b1fbd69cf289,
        0x5cfa4f92b72ef9b1,
        0x4ebec1f115eaee42,
        0xaf867fd881b60a18,
        0x89a6a152c4b44b12,
        0x32c233c4d2699634,
        0xee1ac479ac4980fe,
    ];

    #[test]
    fn lozenge_pins() {
        if !crate::text::test_tahoma() {
            return;
        }
        let t = text();
        let mut got = Vec::new();
        for (case, name) in CASES.iter().enumerate() {
            for dark in [false, true] {
                for s in [1, 2] {
                    let c = pin_case(case, &glass(dark, s), &t);
                    got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
                }
            }
        }
        let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
        for (i, (case, h)) in got.iter().enumerate() {
            assert_eq!(*h, LOZENGE_PINS[i], "{case}; now:\n{}", now.join("\n"));
        }
    }

    #[test]
    fn the_sheet_canvas_is_the_layout_and_its_corners_are_clear() {
        for s in [1, 2] {
            let theme = glass(false, s);
            let t = text();
            let lay = sheet_layout(
                &theme,
                &t,
                SheetShape::Lozenge,
                &content("a", "b", "c"),
                false,
            )
            .unwrap();
            let c = draw_sheet(
                &theme,
                &t,
                &lay,
                SheetIcons::default(),
                None,
                None,
                None,
                None,
            );
            assert_eq!((c.width as i32, c.height as i32), (lay.width, lay.height));
            assert_eq!(lay.sheet.width, 360 * s);
            let corner = c.get(lay.sheet.x, lay.sheet.y);
            assert_eq!(
                corner & 0xFF_FFFF,
                0,
                "x{s}: only shadow outside the corner"
            );
            let close = lay.parts.close;
            let hot = draw_sheet(
                &theme,
                &t,
                &lay,
                SheetIcons::default(),
                None,
                None,
                Some(NotifyHit::Body),
                None,
            );
            let mid = (close.x + close.width / 2, close.y + close.height / 2);
            assert_ne!(
                c.get(mid.0, mid.1),
                hot.get(mid.0, mid.1),
                "x{s}: the close button shows on hover"
            );
        }
    }

    #[test]
    fn classic_and_blue_have_no_sheet_layout() {
        for i in [Interface::Classic, Interface::Blue] {
            assert!(!has_sheets(&theme_for(i)));
            assert!(
                sheet_layout(
                    &theme_for(i),
                    &text(),
                    SheetShape::Lozenge,
                    &content("a", "b", "c"),
                    false
                )
                .is_none()
            );
        }
        assert!(has_sheets(&theme_for(Interface::Glass)));
    }

    #[test]
    fn the_chip_reads_plus_n_more_and_fits_its_label() {
        assert_eq!(chip_label(2), "+2 more");
        let (theme, t) = (theme_for(Interface::Glass), text());
        let (w, h) = chip_layout(&theme, &t, 12);
        assert_eq!(h, 28);
        assert!(w > t.measure("+12 more", false, 12));
        let c = draw_chip(&theme, &t, 12, false);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        assert_ne!(fnv(&c), fnv(&draw_chip(&theme, &t, 12, true)), "hot lifts");
        let (w2, h2) = chip_layout(&theme.scaled(2), &t, 12);
        assert_eq!((w2 % 2, h2), (0, 2 * h), "whole design px at scale 2");
    }

    #[test]
    fn the_stack_grows_away_from_the_tray_on_every_edge() {
        let u = Rect::new(0, 0, 1920, 1050);
        let three = [(360, 96); 3];
        let chip = Some((90, 28));
        assert_eq!(
            place_stack(u, &three, chip, Edge::Bottom),
            (
                vec![(1552, 946), (1552, 842), (1552, 738)],
                Some((1822, 702))
            )
        );
        assert_eq!(
            place_stack(u, &three, chip, Edge::Right).0,
            vec![(1552, 946), (1552, 842), (1552, 738)]
        );
        assert_eq!(
            place_stack(u, &three, None, Edge::Left).0,
            vec![(8, 946), (8, 842), (8, 738)]
        );
        assert_eq!(
            place_stack(u, &three, chip, Edge::Top),
            (vec![(1552, 8), (1552, 112), (1552, 216)], Some((1822, 320)))
        );
    }

    #[test]
    fn a_stack_too_tall_stays_inside_the_usable_area() {
        let u = Rect::new(0, 30, 800, 300);
        let (at, _) = place_stack(u, &[(360, 200); 3], None, Edge::Bottom);
        assert!(at.iter().all(|&(_, y)| y >= 30), "{at:?}");
    }

    #[test]
    fn an_arriving_sheet_rises_14_px_and_fades_in() {
        let rest = Rect::new(100, 500, 360, 120);
        assert_eq!(
            pose(0.0, rest, RISE, 1, 0.95),
            (Rect::new(109, 517, 342, 114), 0.0)
        );
        assert_eq!(pose(1.0, rest, RISE, 1, 0.95), (rest, 1.0));
        assert_eq!(
            pose(1.3, rest, RISE, 1, 0.95),
            (rest, 1.0),
            "k past 1 is clamped"
        );
    }

    #[test]
    fn under_a_top_bar_the_sheet_rises_from_above() {
        assert_eq!(
            pose(0.0, Rect::new(100, 500, 360, 120), RISE, -1, 0.95),
            (Rect::new(109, 489, 342, 114), 0.0)
        );
    }

    #[test]
    fn the_lozenge_style_draws_the_lozenge_shape() {
        assert_eq!(glass::NotifyStyle::Lozenge.shape(), SheetShape::Lozenge);
    }

    fn halo(c: &SheetContent) -> SheetLayout {
        sheet_layout(
            &theme_for(Interface::Glass),
            &text(),
            SheetShape::Halo,
            c,
            false,
        )
        .unwrap()
    }

    #[test]
    fn the_halo_style_draws_the_halo_shape() {
        assert_eq!(glass::NotifyStyle::Halo.shape(), SheetShape::Halo);
    }

    #[test]
    fn the_halo_icon_stands_half_outside_the_sheet() {
        let lay = halo(&content("Mail", "New", "Hi"));
        assert_eq!(
            lay.icon,
            Rect::new(lay.sheet.x + 9, lay.sheet.y - 25, 50, 50)
        );
        assert_eq!(
            (lay.footprint.y, lay.footprint.bottom()),
            (lay.icon.y, lay.sheet.bottom())
        );
        assert_eq!(
            (lay.sheet.width, lay.well, lay.badge),
            (360, Well::Squircle, None)
        );
    }

    #[test]
    fn the_breakout_takes_the_pointer() {
        let lay = halo(&content("Mail", "New", "Hi"));
        assert_eq!(
            sheet_hit(&lay, lay.icon.x + 25, lay.icon.y + 5),
            Some(NotifyHit::Body)
        );
        assert_eq!(
            sheet_hit(&lay, lay.sheet.right() - 40, lay.icon.y + 5),
            None,
            "beside the well, above the sheet"
        );
    }

    #[test]
    fn a_halo_sender_picture_sits_beside_the_title() {
        let with = halo(&SheetContent {
            has_picture: true,
            ..content("Chat", "Ana", "Lunch?")
        });
        let p = with.picture.unwrap();
        assert_eq!(
            (p.width, p.height, with.parts.title.x),
            (28, 28, p.right() + 8)
        );
        assert_eq!((p.x, p.y), (with.sheet.x + 18, with.parts.title.y));
        assert_eq!(with.parts.body.x, with.parts.title.x);
        assert_eq!(with.badge, None, "the app icon keeps the well");
        assert_eq!(
            halo(&content("Chat", "Ana", "Lunch?")).parts.title.x,
            with.sheet.x + 18
        );
    }

    #[test]
    fn the_halo_text_runs_full_width_under_the_app_line() {
        let lay = halo(&content("Mail", "New", "Hi"));
        let (sh, t) = (lay.sheet, lay.parts.title);
        assert_eq!(lay.app_box.x, sh.x + 69);
        assert_eq!(lay.app_box.y, sh.y + 8);
        assert_eq!((t.y, t.right()), (sh.y + 33, sh.right() - 18));
        assert_eq!(lay.parts.body.right(), sh.right() - 18);
        assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 15, 14));
        assert_eq!(
            lay.parts.close,
            Rect::new(sh.right() - 36, sh.y + 13, 20, 20)
        );
        assert!(
            !lay.parts.close.intersects(t),
            "the title row clears the close button"
        );
        assert_eq!(sh.bottom(), lay.parts.body.bottom() + 18);
    }

    #[test]
    fn halo_pills_span_the_sheet_under_the_body() {
        let acts = ["Open", "Later"];
        let lay = halo(&SheetContent {
            actions: &acts,
            has_picture: true,
            ..content("Mail", "New", "Hi")
        });
        let b = lay.parts.buttons[1];
        assert_eq!(b.right(), lay.sheet.right() - 18);
        assert!(lay.parts.buttons[0].y > lay.picture.unwrap().bottom());
        assert_eq!(lay.sheet.bottom(), b.bottom() + 18);
        let c = SheetContent {
            actions: &acts,
            reply: Some(REPLY),
            ..content("Chat", "Ana", "Lunch?")
        };
        let open = sheet_layout(
            &theme_for(Interface::Glass),
            &text(),
            SheetShape::Halo,
            &c,
            true,
        )
        .unwrap();
        assert_eq!(open.parts.field.unwrap().x, open.sheet.x + 18);
    }

    fn halo_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
        let s = theme.scale;
        let acts = ["Reply", "Later"];
        let base = content("Chat", "Ana", "Lunch at noon? The usual place.");
        let (red, blue) = (Color::rgb(0xD0, 0x30, 0x30), Color::rgb(0x30, 0x60, 0xD0));
        let (c, hot, tint) = match case {
            0 => (base, None, None),
            1 => (base, Some(NotifyHit::Close), None),
            2 => (
                SheetContent {
                    actions: &acts,
                    ..base
                },
                Some(NotifyHit::Button(0)),
                None,
            ),
            3 => (
                SheetContent {
                    has_picture: true,
                    ..base
                },
                None,
                None,
            ),
            4 => (base, None, Some(red)),
            _ => (base, None, Some(blue)),
        };
        let lay = sheet_layout(theme, t, SheetShape::Halo, &c, false).unwrap();
        let app = disc(tint.unwrap_or(Color::rgb(0x80, 0x80, 0x80)), 48 * s);
        let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 28 * s);
        let icons = SheetIcons {
            app: Some(&app),
            picture: c.has_picture.then_some(&picture),
        };
        draw_sheet(theme, t, &lay, icons, tint, None, hot, None)
    }

    /// A round icon, clear outside the disc as a theme icon is.
    fn disc(c: Color, side: i32) -> Canvas {
        let r = f64::from(side) * 0.42;
        let mid = f64::from(side) / 2.0;
        crate::shape::draw(side, side, 1, |x, y| {
            scale(
                crate::shape::premul(c, 255),
                crate::shape::ellipse(x, y, mid, mid, r, r),
            )
        })
    }

    const HALO_CASES: [&str; 6] = [
        "resting",
        "hot close",
        "two actions",
        "picture",
        "red tint",
        "blue tint",
    ];

    const HALO_PINS: [u64; 24] = [
        0xc87baade66c86c50,
        0xc15e81e1b26ed426,
        0x08f7453f23193b67,
        0x6f1a7ab14cccb98e,
        0xd18bdae0cbfce7dd,
        0x662a91cdd0d9eb67,
        0xb089ae5e075f0bc6,
        0x26fb58f810a4bf27,
        0x8bc0bdcefce6bda7,
        0x526bde193ecf3240,
        0x2b96f63ca064b37a,
        0xfbc687a88aa59f4d,
        0x6b3812eb5fca657b,
        0xd4e27dd1ea4ef6f3,
        0xa58186a1c834bce6,
        0x0c69f2445e672382,
        0x9d409da44a21dcf5,
        0xf6c935a74d8c9579,
        0x62ff16259d512b5e,
        0x53d25fb60cd67965,
        0x2cafd025beeb7306,
        0xa1bad3ac5ac3cf29,
        0x4120aa3ac20c5e05,
        0x0c4ac6dca907400a,
    ];

    #[test]
    fn halo_pins() {
        if !crate::text::test_tahoma() {
            return;
        }
        let t = text();
        let mut got = Vec::new();
        for (case, name) in HALO_CASES.iter().enumerate() {
            for dark in [false, true] {
                for s in [1, 2] {
                    let c = halo_case(case, &glass(dark, s), &t);
                    got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
                }
            }
        }
        let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
        for (i, (case, h)) in got.iter().enumerate() {
            assert_eq!(*h, HALO_PINS[i], "{case}; now:\n{}", now.join("\n"));
        }
    }

    #[test]
    fn a_halo_canvas_is_its_layout_with_the_icon_in_the_well() {
        for s in [1, 2] {
            let theme = glass(false, s);
            let t = text();
            let c = SheetContent {
                has_picture: true,
                ..content("Chat", "Ana", "Lunch?")
            };
            let lay = sheet_layout(&theme, &t, SheetShape::Halo, &c, false).unwrap();
            let blue = Color::rgb(0x2E, 0x7D, 0xE0);
            let (app, pic) = (
                swatch(blue, 50 * s),
                swatch(Color::rgb(0xE0, 0x6A, 0x2E), 28 * s),
            );
            let icons = SheetIcons {
                app: Some(&app),
                picture: Some(&pic),
            };
            let drawn = draw_sheet(&theme, &t, &lay, icons, Some(blue), None, None, None);
            assert_eq!(
                (drawn.width as i32, drawn.height as i32),
                (lay.width, lay.height)
            );
            let i = lay.icon;
            assert_eq!(drawn.get(i.x + 25 * s, i.y + 4 * s), 0xFF2E_7DE0, "x{s}");
            let p = lay.picture.unwrap();
            assert_eq!(drawn.get(p.x + 14 * s, p.y + 14 * s), 0xFFE0_6A2E, "x{s}");
            assert_ne!(drawn.get(p.x, p.y), 0xFFE0_6A2E, "x{s}: a round picture");
        }
    }

    #[test]
    #[ignore]
    fn render_halo_sheets() {
        let t = text();
        for dark in [false, true] {
            for s in [1, 2] {
                for case in 0..HALO_CASES.len() {
                    let c = halo_case(case, &glass(dark, s), &t);
                    let mode = if dark { "dark" } else { "light" };
                    crate::raster::write_preview(
                        &on_backdrop(&c),
                        &format!("notifysheet-halo-{case}-{mode}-x{s}.png"),
                    );
                }
            }
        }
    }

    fn on_backdrop(c: &Canvas) -> Canvas {
        let mut bg = Canvas::new(c.width, c.height);
        let w = c.width as i32;
        bg.fill_rect(
            Rect::new(0, 0, w / 2, c.height as i32),
            Color::rgb(0x3A, 0x6E, 0xA5),
        );
        bg.fill_rect(
            Rect::new(w / 2, 0, w - w / 2, c.height as i32),
            Color::rgb(0xC8, 0xD4, 0xE0),
        );
        bg.blit_canvas(0, 0, c);
        bg
    }

    fn droplet(c: &SheetContent, expanded: bool) -> SheetLayout {
        sheet_layout(
            &theme_for(Interface::Glass),
            &text(),
            SheetShape::Droplet,
            c,
            expanded,
        )
        .unwrap()
    }

    const LONG_ACTS: [&str; 2] = ["Reply", "Later"];

    fn long() -> SheetContent<'static> {
        SheetContent {
            actions: &LONG_ACTS,
            ..content(
                "Chat",
                "Ana",
                "Lunch at noon?\nThe usual place by the river.\nBring the photos.",
            )
        }
    }

    #[test]
    fn a_collapsed_droplet_is_one_68_px_row() {
        let lay = droplet(&long(), false);
        assert_eq!(lay.sheet.height, 68);
        assert_eq!(lay.lines.len(), 1, "body ellipsized");
        assert!(lay.parts.buttons.is_empty());
    }

    #[test]
    fn hover_expands_to_the_wrapped_body_and_actions() {
        let lay = droplet(&long(), true);
        assert_eq!((lay.lines.len(), lay.parts.buttons.len()), (3, 2));
        assert!(lay.sheet.height > 68);
    }

    #[test]
    fn the_droplet_picture_sits_at_the_right_end() {
        let lay = droplet(
            &SheetContent {
                has_picture: true,
                ..long()
            },
            false,
        );
        assert_eq!(
            lay.picture,
            Some(Rect::new(lay.sheet.right() - 64, lay.sheet.y + 10, 48, 48))
        );
    }

    #[test]
    fn the_droplet_icon_slot_is_round_48() {
        let lay = droplet(&long(), false);
        assert_eq!(
            lay.icon,
            Rect::new(lay.sheet.x + 10, lay.sheet.y + 10, 48, 48)
        );
        assert_eq!(lay.well, Well::Round);
        assert_eq!(lay.badge, None);
    }

    #[test]
    fn an_opening_droplet_lerps_its_height() {
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        assert_eq!(open_layout(&c, &e, 0.0).sheet.height, 68);
        assert_eq!(
            open_layout(&c, &e, 0.5).sheet.height,
            (f64::from(68 + e.sheet.height) / 2.0).round() as i32
        );
        assert!(open_layout(&c, &e, 0.0).parts.buttons.is_empty());
        assert_eq!(open_layout(&c, &e, 1.0).sheet, e.sheet);
    }

    #[test]
    fn the_droplet_text_column_ends_short_of_the_close_and_picture() {
        let plain = droplet(&long(), false);
        assert_eq!(plain.parts.body.x, plain.sheet.x + 68);
        let close = plain.parts.close;
        assert_eq!(plain.parts.body.right(), close.x - 4);
        assert_eq!(plain.parts.title.right(), close.x - 4);
        let open = droplet(&long(), true);
        assert_eq!(open.parts.body.right(), open.sheet.right() - 34);
        assert_eq!(
            open.parts.title, plain.parts.title,
            "the head row keeps its end"
        );
        let pic = droplet(
            &SheetContent {
                has_picture: true,
                ..long()
            },
            false,
        );
        let p = pic.picture.unwrap();
        assert_eq!(pic.parts.close.right() + 4, p.x);
        assert_eq!(pic.parts.body.right(), pic.parts.close.x - 4);
    }

    #[test]
    fn the_droplet_close_never_covers_the_picture() {
        for expanded in [false, true] {
            let lay = droplet(
                &SheetContent {
                    has_picture: true,
                    ..long()
                },
                expanded,
            );
            let (c, p) = (lay.parts.close, lay.picture.unwrap());
            assert!(
                c.intersect(&p).is_empty(),
                "expanded={expanded}: {c:?} on {p:?}"
            );
        }
    }

    #[test]
    fn a_body_cut_at_the_line_cap_ends_in_an_ellipsis() {
        let long = "word ".repeat(400);
        let c = content("Mail", "t", &long);
        for lay in [lozenge(&c, false), halo(&c), droplet(&c, true)] {
            assert_eq!(lay.lines.len(), NOTIFY_BODY_LINES);
            let last = lay.lines.last().unwrap();
            assert!(last.ends_with("..."), "{:?}: {last:?}", lay.shape);
            assert!(text().measure(last, false, lay.body_px) <= lay.parts.body.width);
        }
        let fits = lozenge(&content("Mail", "t", "word word"), false);
        assert_eq!(fits.lines, ["word word"]);
    }

    #[test]
    fn the_droplet_close_spot_moves_up_when_expanded() {
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        assert_eq!(
            c.parts.close,
            Rect::new(c.sheet.right() - 54, c.sheet.y + 24, 20, 20)
        );
        assert_eq!(
            e.parts.close,
            Rect::new(e.sheet.right() - 34, e.sheet.y + 12, 20, 20)
        );
        let x = c.parts.close;
        assert_eq!(sheet_hit(&c, x.x + 2, x.y + 2), Some(NotifyHit::Close));
    }

    #[test]
    fn a_bodyless_droplet_centres_its_title_on_the_icon() {
        let lay = droplet(&content("Mail", "Synced", ""), false);
        let (t, i) = (lay.parts.title, lay.icon);
        assert_eq!(t.y + t.height / 2, i.y + i.height / 2);
        assert!(lay.lines.is_empty());
    }

    #[test]
    fn the_droplet_title_is_15_px() {
        let lay = droplet(&long(), false);
        assert_eq!((lay.app_px, lay.title_px, lay.body_px), (12, 15, 14));
    }

    #[test]
    fn an_opening_droplet_draws_its_ends_at_0_and_1() {
        let (theme, t) = (glass(false, 1), text());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let icons = SheetIcons::default();
        let draw = |lay: &SheetLayout| draw_sheet(&theme, &t, lay, icons, None, None, None, None);
        let open =
            |k: f64| draw_sheet_opening(&theme, &t, &c, &e, k, icons, None, None, None, None);
        assert_eq!(
            fnv(&open(0.0)),
            fnv(&draw(&c)),
            "t = 0 is the collapsed sheet"
        );
        assert_eq!(
            fnv(&open(1.0)),
            fnv(&draw(&e)),
            "t = 1 is the expanded sheet"
        );
        let half = open(0.5);
        assert_eq!(half.height as i32, open_layout(&c, &e, 0.5).height);
    }

    #[test]
    fn an_opening_droplet_draws_no_part_it_has_dropped() {
        let (theme, t) = (glass(false, 1), text());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let open = open_layout(&c, &e, 0.5);
        let pill = e.parts.buttons[0];
        assert!(open.parts.buttons.is_empty() && pill.y < open.sheet.bottom());
        let hot = Some(NotifyHit::Body);
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let icons = SheetIcons::default();
        let drawn = opening.draw(
            &mut kept, &theme, &t, &c, &e, 0.5, icons, None, None, hot, None,
        );
        let key = &opening.layers.as_ref().unwrap().key;
        let glass = glass_at(&kept, key, true, open.sheet, 1);
        assert_eq!((drawn.width, drawn.height), (glass.width, glass.height));
        for y in 0..drawn.height as i32 {
            for x in 0..drawn.width as i32 {
                let off_sheet = !open.sheet.contains(x, y);
                let under_pill = (pill.x..pill.right()).contains(&x) && y >= pill.y;
                if off_sheet || under_pill {
                    assert_eq!(drawn.get(x, y), glass.get(x, y), "({x}, {y}) is glass");
                }
            }
        }
    }

    fn warm(
        opening: &mut SheetOpening,
        kept: &mut SheetGlass,
        c: &SheetContent,
        lit: bool,
    ) -> bool {
        let (theme, t) = (glass(false, 1), text());
        let (shut, open) = (droplet(c, false), droplet(c, true));
        let icons = SheetIcons::default();
        opening.warm(kept, &theme, &t, &shut, &open, icons, None, None, lit)
    }

    #[test]
    fn a_droplet_warmed_at_rest_opens_under_any_hot_part_without_a_build() {
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        assert!(
            warm(&mut opening, &mut kept, &long(), true),
            "the first warm paints"
        );
        assert!(
            !warm(&mut opening, &mut kept, &long(), true),
            "a second finds it warm"
        );
        let (theme, t) = (glass(false, 1), text());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let hots = [NotifyHit::Body, NotifyHit::Close, NotifyHit::Button(0)];
        for (k, hot) in [0.1, 0.35, 0.6, 0.9]
            .into_iter()
            .zip(hots.into_iter().cycle())
        {
            let icons = SheetIcons::default();
            opening.draw(
                &mut kept,
                &theme,
                &t,
                &c,
                &e,
                k,
                icons,
                None,
                Some(hot),
                Some(hot),
                None,
            );
        }
        assert_eq!(opening.builds, 1, "hover and presses reuse the warm layers");
        let other = content("Chat", "Ben", "Running late, start without me.");
        assert!(
            warm(&mut opening, &mut kept, &other, true),
            "new content paints once"
        );
        assert!(!warm(&mut opening, &mut kept, &other, true));
        assert_eq!(opening.builds, 2);
    }

    fn warm_off_thread(
        opening: &mut SheetOpening,
        kept: &mut SheetGlass,
        theme: &Theme,
        c: &SheetContent,
        lit: bool,
    ) -> Option<()> {
        let lay = |expanded| sheet_layout(theme, &text(), SheetShape::Droplet, c, expanded);
        let (shut, open) = (lay(false).unwrap(), lay(true).unwrap());
        let job = opening.warm_job(kept, theme, &shut, &open, SheetIcons::default(), None, lit)?;
        let built = std::thread::spawn(move || {
            build_warm(job, &TextRenderer::load(&FontConfig::default()))
        })
        .join()
        .unwrap();
        opening.install(kept, built);
        Some(())
    }

    #[test]
    fn a_droplet_warmed_on_a_worker_paints_nothing_on_the_callers_thread() {
        let (theme, t) = (glass(false, 1), text());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let icons = SheetIcons::default();
        let ask = |opening: &mut SheetOpening| {
            opening
                .warm_job(&kept, &theme, &c, &e, icons, None, true)
                .is_some()
        };
        assert!(ask(&mut opening));
        assert!(!ask(&mut opening), "one job out per key and rim");
        opening.forget_job();
        assert!(
            ask(&mut opening),
            "a job lost with its worker is asked again"
        );
        opening.job_failed();
        assert!(!ask(&mut opening), "a job that panicked is not asked again");
        let other = |opening: &mut SheetOpening| {
            let lay = |expanded| droplet(&content("Chat", "Ben", "Late."), expanded);
            let job = opening.warm_job(&kept, &theme, &lay(false), &lay(true), icons, None, true);
            job.is_some()
        };
        assert!(other(&mut opening), "other content is");
        let mut opening = SheetOpening::default();
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
        assert!(
            warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).is_none(),
            "warm already"
        );
        for (k, pushed, hot) in [
            (0.2, None, Some(NotifyHit::Body)),
            (0.5, Some(NotifyHit::Close), Some(NotifyHit::Close)),
            (1.0, None, Some(NotifyHit::Button(0))),
        ] {
            opening.draw(
                &mut kept, &theme, &t, &c, &e, k, icons, None, pushed, hot, None,
            );
        }
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), false).unwrap();
        for k in [0.7, 0.3, 0.0] {
            opening.draw(
                &mut kept, &theme, &t, &c, &e, k, icons, None, None, None, None,
            );
        }
        assert_eq!(
            (opening.builds(), kept.builds()),
            (0, 0),
            "the worker painted the parts and both rims"
        );
    }

    #[test]
    fn a_droplet_keeps_its_glass_at_rest_and_the_ends_of_one_rim() {
        let (theme, t) = (glass(false, 1), text());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let icons = SheetIcons::default();
        let frame = |opening: &mut SheetOpening, kept: &mut SheetGlass, k: f64, lit: bool| {
            let hot = lit.then_some(NotifyHit::Body);
            opening.draw(kept, &theme, &t, &c, &e, k, icons, None, None, hot, None);
            let rims: Vec<bool> = kept.kept.iter().map(|(k, _)| k.lit()).collect();
            rims
        };
        assert_eq!(frame(&mut opening, &mut kept, 0.0, false), [false]);
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
        assert_eq!(
            kept.kept.len(),
            3,
            "at rest, and both ends lit for the pointer"
        );
        assert_eq!(frame(&mut opening, &mut kept, 0.5, true), [true, true]);
        assert_eq!(frame(&mut opening, &mut kept, 1.0, true), [true]);
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), false).unwrap();
        assert_eq!(
            kept.kept.len(),
            3,
            "at rest, and both ends plain for the close"
        );
        assert_eq!(frame(&mut opening, &mut kept, 0.5, false), [false, false]);
    }

    #[test]
    fn an_app_line_ageing_keeps_a_droplets_layers_and_draws_on_top() {
        let (theme, t) = (glass(false, 2), text());
        let later = SheetContent {
            when: "1 min ago",
            ..long()
        };
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        warm_off_thread(&mut opening, &mut kept, &theme, &long(), true).unwrap();
        assert!(
            warm_off_thread(&mut opening, &mut kept, &theme, &later, true).is_none(),
            "no new job for an older app line"
        );
        let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &later, expanded);
        let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
        let hot = Some(NotifyHit::Body);
        let icons = SheetIcons::default();
        let aged = opening.draw(
            &mut kept, &theme, &t, &c, &e, 0.5, icons, None, None, hot, None,
        );
        let fresh = draw_sheet_opening(&theme, &t, &c, &e, 0.5, icons, None, None, hot, None);
        assert_eq!(opening.builds(), 0);
        assert!(aged == fresh);
    }

    #[test]
    fn a_droplet_parts_layer_painted_in_the_frame_leaves_the_app_line_to_the_top_pass() {
        let (theme, t) = (glass(false, 2), text());
        let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
        let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
        let (icons, hot) = (SheetIcons::default(), Some(NotifyHit::Body));
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let mut frame = |opening: &mut SheetOpening| {
            opening.draw(
                &mut kept, &theme, &t, &c, &e, 0.9, icons, None, None, hot, None,
            )
        };
        frame(&mut opening);
        let warmed = std::mem::take(&mut opening.layers.as_mut().unwrap().open_parts);
        frame(&mut opening);
        let painted = &opening.layers.as_ref().unwrap().open_parts;
        let [(key, layer)] = painted.as_slice() else {
            panic!("one layer painted in the frame");
        };
        let cached = warmed.iter().find(|(k, _)| k == key).unwrap();
        assert!(*layer == cached.1);
    }

    #[test]
    fn a_late_warm_result_for_earlier_content_leaves_the_layers_painted_since() {
        let (theme, t) = (glass(false, 1), text());
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let icons = SheetIcons::default();
        let job = opening
            .warm_job(&kept, &theme, &c, &e, icons, None, true)
            .unwrap();
        let other = content("Chat", "Ben", "Running late, start without me.");
        let (oc, oe) = (droplet(&other, false), droplet(&other, true));
        let hot = Some(NotifyHit::Body);
        let frame = |opening: &mut SheetOpening, kept: &mut SheetGlass, k: f64| {
            opening.draw(kept, &theme, &t, &oc, &oe, k, icons, None, None, hot, None);
        };
        frame(&mut opening, &mut kept, 0.3);
        assert_eq!(
            opening.builds(),
            1,
            "the new content is painted in the frame"
        );
        opening.install(&mut kept, build_warm(job, &t));
        frame(&mut opening, &mut kept, 0.6);
        assert_eq!(opening.builds(), 1, "the late result left the new layers");
    }

    #[test]
    fn a_frame_from_worker_layers_matches_one_from_layers_painted_in_place() {
        let (theme, t) = (glass(false, 2), text());
        let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
        let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
        let icons = SheetIcons::default();
        let hot = Some(NotifyHit::Button(0));
        let (mut there, mut there_glass) = (SheetOpening::default(), SheetGlass::default());
        warm_off_thread(&mut there, &mut there_glass, &theme, &long(), true).unwrap();
        let (mut here, mut here_glass) = (SheetOpening::default(), SheetGlass::default());
        let a = there.draw(
            &mut there_glass,
            &theme,
            &t,
            &c,
            &e,
            0.6,
            icons,
            None,
            None,
            hot,
            None,
        );
        let b = here.draw(
            &mut here_glass,
            &theme,
            &t,
            &c,
            &e,
            0.6,
            icons,
            None,
            None,
            hot,
            None,
        );
        assert!(a == b);
    }

    #[test]
    fn lit_parts_laid_on_the_unlit_layer_match_parts_drawn_lit() {
        let (theme, t) = (glass(false, 2), text());
        let look = notify_look(&theme).unwrap();
        let e = sheet_layout(&theme, &t, SheetShape::Droplet, &long(), true).unwrap();
        let icons = SheetIcons::default();
        let clear = || Canvas::new(e.width as u32, e.height as u32);
        for (pushed, hot) in [
            (None, Some(NotifyHit::Body)),
            (None, Some(NotifyHit::Button(1))),
            (Some(NotifyHit::Close), Some(NotifyHit::Close)),
        ] {
            let mut want = clear();
            draw_parts(&mut want, &theme, &t, &look, &e, icons, pushed, hot, None);
            let mut got = clear();
            draw_parts(&mut got, &theme, &t, &look, &e, icons, None, None, None);
            relight(&mut got, &t, &look, &e, true, pushed, hot, 2);
            assert!(got == want, "{hot:?} pushed {pushed:?}");
        }
    }

    #[test]
    fn an_opening_droplets_glass_stays_within_6_levels_of_the_glass_painted_at_its_height() {
        let (theme, t) = (glass(false, 1), text());
        let look = notify_look(&theme).unwrap();
        let (c, e) = (droplet(&long(), false), droplet(&long(), true));
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let icons = SheetIcons::default();
        opening.warm(&mut kept, &theme, &t, &c, &e, icons, None, None, true);
        let key = &opening.layers.as_ref().unwrap().key;
        for k in [0.25, 0.5, 0.75] {
            let open = open_layout(&c, &e, k);
            let (got, want) = (
                glass_at(&kept, key, true, open.sheet, 1),
                (look.sheet)(&look, &sheet_spec(&open, None, true, 1), 1),
            );
            assert_eq!((got.width, got.height), (want.width, want.height));
            let worst = got
                .pixels
                .iter()
                .zip(&want.pixels)
                .flat_map(|(&a, &b)| {
                    (0..4).map(move |i| ((a >> (8 * i)) & 0xFF).abs_diff((b >> (8 * i)) & 0xFF))
                })
                .max();
            assert!(worst <= Some(6), "t = {k}: off by {worst:?}");
        }
    }

    /// Prints what warming an opening at rest, its first frame and later ones cost at scales 1
    /// and 2. Run with `--release -- --ignored --nocapture`.
    #[test]
    #[ignore = "timing"]
    fn opening_frame_cost() {
        let t = text();
        for s in [1, 2] {
            let theme = glass(false, s);
            let lay = |expanded| {
                sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded).unwrap()
            };
            let (c, e) = (lay(false), lay(true));
            let hot = Some(NotifyHit::Body);
            let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
            let at = std::time::Instant::now();
            let icons = SheetIcons::default();
            let job = opening
                .warm_job(&kept, &theme, &c, &e, icons, None, true)
                .unwrap();
            let asked = at.elapsed();
            let built = build_warm(job, &t);
            let warm = at.elapsed() - asked;
            let at = std::time::Instant::now();
            opening.install(&mut kept, built);
            let installed = at.elapsed();
            let mut frame = |k: f64| {
                let at = std::time::Instant::now();
                opening.draw(
                    &mut kept,
                    &theme,
                    &t,
                    &c,
                    &e,
                    k,
                    SheetIcons::default(),
                    None,
                    None,
                    hot,
                    None,
                );
                at.elapsed()
            };
            let first = frame(0.05);
            let ticks: Vec<_> = (1..=9).map(|i| frame(f64::from(i) / 10.0)).collect();
            let worst = ticks.iter().max().unwrap();
            let last = frame(1.0);
            println!(
                "scale {s}: job made {asked:?}, warm on a worker {warm:?}, installed {installed:?}, first frame {first:?}, later frames up to {worst:?}, last frame {last:?}"
            );
        }
    }

    /// Prints a Droplet's held glass and parts at scale 2, at rest collapsed and open, each warmed
    /// for its next move. Run with `--release -- --ignored --nocapture`.
    #[test]
    #[ignore = "memory"]
    fn droplet_memory() {
        let (theme, t) = (glass(false, 2), text());
        let lay = |expanded| sheet_layout(&theme, &t, SheetShape::Droplet, &long(), expanded);
        let (c, e) = (lay(false).unwrap(), lay(true).unwrap());
        let (mut opening, mut kept) = (SheetOpening::default(), SheetGlass::default());
        let icons = SheetIcons::default();
        let mb = |b: usize| b as f64 / (1024.0 * 1024.0);
        let end = |lay: &SheetLayout| lay.width as usize * lay.height as usize * 4;
        let hot = Some(NotifyHit::Body);
        for (k, hot) in [(0.0, None), (1.0, hot)] {
            opening.draw(
                &mut kept, &theme, &t, &c, &e, k, icons, None, None, hot, None,
            );
            warm_off_thread(&mut opening, &mut kept, &theme, &long(), k < 1.0).unwrap();
            println!(
                "at rest t = {k}: glass {:.2} MB, parts {:.2} MB",
                mb(kept.bytes()),
                mb(opening.bytes())
            );
        }
        println!(
            "one end's glass: collapsed {:.2} MB, expanded {:.2} MB",
            mb(end(&c)),
            mb(end(&e))
        );
    }

    #[test]
    fn the_collapsed_ink_is_gone_before_the_expanded_ink_is_half_in() {
        assert_eq!(
            (opening_fades(0.0), opening_fades(1.0)),
            ((1.0, 0.0), (0.0, 1.0))
        );
        let (shut, open) = opening_fades(0.4);
        assert!(shut == 0.0 && open < 0.5);
        for i in 0..=20 {
            let (a, b) = opening_fades(f64::from(i) / 20.0);
            assert!(a + b <= 1.0, "never brighter than either end");
        }
    }

    fn droplet_case(case: usize, theme: &Theme, t: &TextRenderer) -> Canvas {
        let s = theme.scale;
        let base = long();
        let c = SheetContent {
            has_picture: matches!(case, 3 | 5),
            ..base
        };
        let lay = |expanded| sheet_layout(theme, t, SheetShape::Droplet, &c, expanded).unwrap();
        let app = swatch(Color::rgb(0x2E, 0x7D, 0xE0), 48 * s);
        let picture = swatch(Color::rgb(0xE0, 0x6A, 0x2E), 48 * s);
        let icons = SheetIcons {
            app: Some(&app),
            picture: c.has_picture.then_some(&picture),
        };
        match case {
            0 | 3 => draw_sheet(theme, t, &lay(false), icons, None, None, None, None),
            1 => draw_sheet(
                theme,
                t,
                &lay(true),
                icons,
                None,
                None,
                Some(NotifyHit::Body),
                None,
            ),
            2 | 5 => draw_sheet(
                theme,
                t,
                &lay(true),
                icons,
                None,
                None,
                Some(NotifyHit::Close),
                None,
            ),
            _ => draw_sheet_opening(
                theme,
                t,
                &lay(false),
                &lay(true),
                0.5,
                icons,
                None,
                None,
                Some(NotifyHit::Body),
                None,
            ),
        }
    }

    const DROPLET_CASES: [&str; 6] = [
        "collapsed",
        "expanded",
        "expanded, hot close",
        "picture",
        "opening t = 0.5",
        "expanded, picture, hot close",
    ];

    const DROPLET_PINS: [u64; 24] = [
        0x425bc144996b943f,
        0x1b5e94833245fcf1,
        0xb7ffd9c5ecc872c0,
        0xb9b970e50ca4aeb3,
        0x77a28eba7566d48d,
        0x309fb736c325eb07,
        0x7dd3e1fbb7ea804d,
        0x5541d5a466e923f5,
        0xd60ae18f4120a49b,
        0x55ac48a5b7d76edf,
        0xdd25fb326db09377,
        0x8fcc436ede36624f,
        0xdf9448f5843dc0db,
        0x782d7e3e6ebd8bb2,
        0x635209b4461c9ef5,
        0x1374d93fde986aed,
        0xe2695eb474531d93,
        0x044298801479ee55,
        0x4ea1de53291eb3c7,
        0x3a2d8890877b5201,
        0x54929f8ba950da9e,
        0x7ce3739d49072a81,
        0xd9ea5a9475d49e13,
        0xdd1759115362f434,
    ];

    #[test]
    fn droplet_pins() {
        if !crate::text::test_tahoma() {
            return;
        }
        let t = text();
        let mut got = Vec::new();
        for (case, name) in DROPLET_CASES.iter().enumerate() {
            for dark in [false, true] {
                for s in [1, 2] {
                    let c = droplet_case(case, &glass(dark, s), &t);
                    got.push((format!("{name} dark={dark} x{s}"), fnv(&c)));
                }
            }
        }
        let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
        for (i, (case, h)) in got.iter().enumerate() {
            assert_eq!(*h, DROPLET_PINS[i], "{case}; now:\n{}", now.join("\n"));
        }
    }

    /// Light then dark, each at scale 1 and 2.
    const BEAD_PINS: [u64; 4] = [
        0xc57c0479b1a5e635,
        0xe326070904f5e5f3,
        0xf58e8278f11aff6a,
        0x96f63b889c79bf5f,
    ];

    #[test]
    fn bead_pins() {
        let mut got = Vec::new();
        for dark in [false, true] {
            for s in [1, 2] {
                got.push(fnv(&drip_bead(&glass(dark, s)).unwrap().0));
            }
        }
        let now: Vec<String> = got.iter().map(|h| format!("{h:#018x},")).collect();
        assert_eq!(got, BEAD_PINS, "now:\n{}", now.join("\n"));
    }

    #[test]
    fn the_bead_centres_its_round_body_in_clear_corners() {
        let (c, body) = drip_bead(&glass(false, 2)).unwrap();
        let (w, h) = (c.width as i32, c.height as i32);
        assert_eq!(body.width, 24);
        assert_eq!((body.x * 2 + body.width, body.y * 2 + body.height), (w, h));
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(c.get(x, y), 0, "({x}, {y}) is clear");
        }
        let alpha = |x: i32, y: i32| c.get(x, y) >> 24;
        let mid = body.x + body.width / 2;
        let corner = alpha(body.x, body.y);
        assert!(
            corner < alpha(mid, mid) / 2,
            "the body square's corner is off the drop"
        );
        let rim = (1..body.width / 2)
            .map(|i| alpha(body.x + i, body.y + i))
            .collect::<Vec<_>>();
        assert!(
            rim.windows(2)
                .any(|p| p[0] < p[1] && p[1] < alpha(mid, mid)),
            "the curved edge is part covered at scale 2: {rim:?}"
        );
    }

    #[test]
    fn the_bead_shows_whole_until_the_swell_and_is_gone_a_third_in() {
        assert_eq!(drip_bead_share(0.0), 1.0);
        assert_eq!(drip_bead_share(BEAD_UNTIL), 1.0);
        let gone = BEAD_UNTIL + BEAD_FADE * (1.0 - BEAD_UNTIL);
        assert_eq!(drip_bead_share(gone), 0.0);
        let mid = drip_bead_share((BEAD_UNTIL + gone) / 2.0);
        assert!(0.0 < mid && mid < 1.0, "{mid}");
    }

    #[test]
    fn the_bead_sits_on_the_drips_bead_then_at_its_tray_side_end() {
        let rest = Rect::new(100, 200, 360, 68);
        let (p, _) = drip_pose(0.1, rest, true);
        assert_eq!(drip_bead_at(p, true), p);
        let (p, _) = drip_pose(0.4, rest, true);
        let b = drip_bead_at(p, true);
        assert_eq!(
            (b.right(), b.height, b.width),
            (p.right(), p.height, p.height)
        );
        let (p, _) = drip_pose(0.4, rest, false);
        assert_eq!(drip_bead_at(p, false).x, p.x);
    }

    #[test]
    #[ignore]
    fn render_beads() {
        for dark in [false, true] {
            for s in [1, 2] {
                let (c, _) = drip_bead(&glass(dark, s)).unwrap();
                let mode = if dark { "dark" } else { "light" };
                crate::raster::write_preview(
                    &on_backdrop(&c),
                    &format!("notifysheet-bead-{mode}-x{s}.png"),
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn render_droplets() {
        let t = text();
        for dark in [false, true] {
            for s in [1, 2] {
                for case in 0..DROPLET_CASES.len() {
                    let c = droplet_case(case, &glass(dark, s), &t);
                    let mode = if dark { "dark" } else { "light" };
                    crate::raster::write_preview(
                        &on_backdrop(&c),
                        &format!("notifysheet-droplet-{case}-{mode}-x{s}.png"),
                    );
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn render_sheets() {
        let t = text();
        for dark in [false, true] {
            for s in [1, 2] {
                for case in 0..4 {
                    let c = pin_case(case, &glass(dark, s), &t);
                    let mut bg = Canvas::new(c.width, c.height);
                    let w = c.width as i32;
                    bg.fill_rect(
                        Rect::new(0, 0, w / 2, c.height as i32),
                        Color::rgb(0x3A, 0x6E, 0xA5),
                    );
                    bg.fill_rect(
                        Rect::new(w / 2, 0, w - w / 2, c.height as i32),
                        Color::rgb(0xC8, 0xD4, 0xE0),
                    );
                    bg.blit_canvas(0, 0, &c);
                    let mode = if dark { "dark" } else { "light" };
                    crate::raster::write_preview(
                        &bg,
                        &format!("notifysheet-{case}-{mode}-x{s}.png"),
                    );
                }
            }
        }
    }

    #[test]
    fn a_droplet_starts_as_a_bead_at_the_tray() {
        let rest = Rect::new(1000, 900, 360, 68);
        assert_eq!(
            drip_pose(0.0, rest, true),
            (Rect::new(1320, 928, 12, 12), 0.0)
        );
        assert_eq!(
            drip_pose(0.2, rest, true),
            (Rect::new(1320, 928, 12, 12), 1.0)
        );
        assert_eq!(drip_pose(0.0, rest, false).0, Rect::new(1028, 928, 12, 12));
    }

    #[test]
    fn a_droplet_swells_past_its_size_and_settles_at_rest() {
        let rest = Rect::new(1000, 900, 360, 68);
        assert!(drip_pose(0.8, rest, true).0.width > 360, "overshoot");
        assert_eq!(drip_pose(1.0, rest, true), (rest, 1.0));
        assert_eq!(drip_pose(1.0, rest, false), (rest, 1.0));
    }

    #[test]
    fn the_drip_takes_twelve_fifths_of_the_open_time() {
        assert_eq!((drip_ms(250), drip_ms(0)), (600, 0));
    }

    #[test]
    fn a_short_drip_gets_a_corner_that_fits() {
        use crate::backdrop::corner_radius;
        assert_eq!(
            (
                corner_radius(drip_corner(68)),
                corner_radius(drip_corner(30)),
                corner_radius(drip_corner(8))
            ),
            (34.0, 12.0, 6.0)
        );
    }

    /// A Lozenge whose app line reads `when`, expanded to its reply row with `reply`.
    fn reply_sheet(theme: &Theme, when: &str, body: &str, reply: bool) -> SheetLayout {
        let c = SheetContent {
            when,
            actions: &["Open", "Mark read"],
            reply: reply.then_some(REPLY),
            ..content("Chat", "Ana", body)
        };
        sheet_layout(theme, &text(), SheetShape::Lozenge, &c, reply).unwrap()
    }

    #[test]
    fn a_sheet_on_kept_glass_matches_one_drawn_whole() {
        let (theme, t) = (glass(false, 2), text());
        let lay = reply_sheet(&theme, "now", "Hi", false);
        let mut kept = SheetGlass::default();
        for hot in [
            None,
            Some(NotifyHit::Button(0)),
            Some(NotifyHit::Body),
            None,
        ] {
            let icons = SheetIcons::default();
            let whole = draw_sheet(&theme, &t, &lay, icons, None, None, hot, None);
            let drawn = kept.draw(&theme, &t, &lay, icons, None, None, hot, None);
            assert_eq!(fnv(&drawn), fnv(&whole), "{hot:?}");
        }
        assert_eq!(kept.builds(), 2, "one glass per rim");
    }

    #[test]
    fn a_caret_blink_or_an_age_tick_paints_no_glass_and_a_resize_paints_one() {
        let (theme, t) = (glass(false, 2), text());
        let edit = TextEdit::new("On my way", false);
        let mut kept = SheetGlass::default();
        let mut draw = |lay: &SheetLayout, shown: bool| {
            let reply = Some((&edit, Caret::new(true, shown), None));
            kept.draw(
                &theme,
                &t,
                lay,
                SheetIcons::default(),
                None,
                None,
                None,
                reply,
            );
            kept.builds()
        };
        let now = reply_sheet(&theme, "now", "Hi", true);
        assert_eq!(draw(&now, true), 1);
        assert_eq!(draw(&now, false), 1, "a caret blink");
        let later = reply_sheet(&theme, "1 min ago", "Hi", true);
        assert_eq!(draw(&later, false), 1, "an age tick");
        let long = "A body long enough to wrap onto a second line and grow the sheet taller";
        let taller = reply_sheet(&theme, "1 min ago", long, true);
        assert!(taller.sheet.height > later.sheet.height);
        assert_eq!(draw(&taller, false), 2, "a taller sheet");
    }

    /// A three-sheet stack's paint cost at scale 2 for a hover move and a caret blink, every sheet
    /// whole against the changed one on kept glass. Run `--release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn stack_paint_cost() {
        use std::time::Instant;
        let (theme, t) = (glass(false, 2), text());
        let edit = TextEdit::new("On my way", false);
        let lays = [
            reply_sheet(&theme, "now", "Hi", true),
            reply_sheet(&theme, "now", "Lunch at one?", false),
            reply_sheet(&theme, "1 min ago", "The build is green", false),
        ];
        let icons = SheetIcons::default();
        let ms = |f: &mut dyn FnMut()| {
            let start = Instant::now();
            for _ in 0..5 {
                f();
            }
            start.elapsed().as_secs_f64() * 1000.0 / 5.0
        };
        // Sheet 0 is replying; the pointer is on sheet 1 and moves to its first pill.
        let state = |hover: NotifyHit, shown: bool| {
            [
                (None, Some((&edit, Caret::new(true, shown), None))),
                (Some(hover), None),
                (None, None),
            ]
        };
        let whole = |hover: NotifyHit, shown: bool| {
            for (lay, (hot, reply)) in lays.iter().zip(state(hover, shown)) {
                draw_sheet(&theme, &t, lay, icons, None, None, hot, reply);
            }
        };
        let hover = ms(&mut || whole(NotifyHit::Button(0), true));
        let blink = ms(&mut || whole(NotifyHit::Body, false));
        println!("every sheet whole: hover {hover:.1} ms, caret blink {blink:.1} ms");
        let one = |kept: &mut SheetGlass, i: usize, hover: NotifyHit, shown: bool| {
            let (hot, reply) = state(hover, shown)[i];
            kept.draw(&theme, &t, &lays[i], icons, None, None, hot, reply);
        };
        let first = ms(&mut || {
            let mut fresh: [SheetGlass; 3] = Default::default();
            for (i, kept) in fresh.iter_mut().enumerate() {
                one(kept, i, NotifyHit::Body, true);
            }
        });
        let mut kept: [SheetGlass; 3] = Default::default();
        for (i, k) in kept.iter_mut().enumerate() {
            one(k, i, NotifyHit::Body, true);
        }
        let hover = ms(&mut || one(&mut kept[1], 1, NotifyHit::Button(0), true));
        let blink = ms(&mut || one(&mut kept[0], 0, NotifyHit::Body, false));
        println!(
            "changed sheet on kept glass: hover {hover:.1} ms, caret blink {blink:.1} ms \
             (first paint of the stack {first:.1} ms)"
        );
    }
}
