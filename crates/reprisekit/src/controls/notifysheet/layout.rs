//! Sheet layouts for each shape, and a Droplet part way open.

use crate::controls::tooltip::{NOTIFY_BODY_LINES, NOTIFY_BUTTONS, NotifyParts, NotifyReply};
use crate::raster::Rect;
use crate::style::{NotifyLook, SheetShape, Well};
use crate::text::{TextRenderer, wrap};
use crate::theme::Theme;

use super::{SheetContent, SheetLayout, notify_look};

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
pub(super) const PILL_PAD: i32 = 14;
const PILL_MIN: i32 = 64;
const MORE_W: i32 = 34;
const CLOSE: i32 = 20;
/// The app line and title end this far short of the close button.
const CLOSE_GAP: i32 = 4;
const BADGE: i32 = 18;
/// The badge centres this far in from the slot's lower-right corner.
const BADGE_IN: i32 = 4;
/// The app icon's inset inside the badge's ring.
pub(super) const BADGE_ICON_IN: i32 = 2;
/// The reply field's text stands in this far from its rounded ends.
pub(super) const FIELD_TEXT_IN: i32 = PILL_H / 2;
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
/// place of the pills. `None` when the look has no sheets ([`super::has_sheets`]).
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
