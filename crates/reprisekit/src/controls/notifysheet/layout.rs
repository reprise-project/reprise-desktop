//! Sheet layouts for each shape, and a Droplet part way open.

use crate::controls::tooltip::{
    NOTIFY_BODY_LINES, NOTIFY_BUTTONS, NotifyParts, NotifyReply, flow_buttons, flow_rows,
};
use crate::look::widgets::BUTTON_ICON;
use crate::raster::Rect;
use crate::style::{NotifyLook, SheetShape, Well};
use crate::text::{TextRenderer, wrap};
use crate::theme::{TextUnits, Theme};

use super::{SheetContent, SheetLayout, notify_look};

/// The parts every shape shares, design px at 100% text; the fonts, the heights that hold a
/// line, the pill sizes and the text column grow with the text size ([`Grown`]).
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
const ICON_PILL_W: i32 = BUTTON_ICON + 2 * PILL_PAD;
const MORE_W: i32 = 34;
const CLOSE: i32 = 20;
/// The app line and title end this far short of the close button.
const CLOSE_GAP: i32 = 4;
const BADGE: i32 = 18;
/// The badge centres this far in from the slot's lower-right corner.
const BADGE_IN: i32 = 4;
/// The app icon's inset inside the badge's ring.
pub(super) const BADGE_ICON_IN: i32 = 2;
/// A picture beside the title stands this far left of it.
const PICTURE_GAP: i32 = 8;

/// A shape's layout at 100% text, design px from the sheet's top-left.
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
    /// The reply field's left; pills may reach the padding.
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

/// The sizes that grow with `theme`'s text, design px but for the fonts, which are device px.
struct Grown {
    t: TextUnits,
    app_px: u32,
    title_px: u32,
    body_px: u32,
    app_h: i32,
    title_h: i32,
    line_h: i32,
    pills: Pills,
}

/// What a pill holds: its label font in device px, the rest design px.
struct Pills {
    px: u32,
    h: i32,
    min: i32,
    more: i32,
}

impl Grown {
    fn new(theme: &Theme, title_px: u32) -> Grown {
        let t = theme.text;
        let su = theme.scale.max(1) as u32;
        Grown {
            t,
            app_px: t.px(APP_PX) * su,
            title_px: t.px(title_px) * su,
            body_px: t.px(BODY_PX) * su,
            app_h: t.len(APP_H),
            title_h: t.len(TITLE_H),
            line_h: t.len(LINE_H),
            pills: Pills {
                px: t.px(PILL_PX) * su,
                h: t.len(PILL_H),
                min: t.len(PILL_MIN),
                more: t.len(MORE_W),
            },
        }
    }

    /// How much wider a text column `w` wide at 100% is at the text size.
    fn widen(&self, w: i32) -> i32 {
        self.t.len(w) - w
    }
}

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

/// The pills right-aligned between `left` and `right` from `y`, design px, or with `reply` the
/// reply field from `field_left` and its send pill. Pills that overrun the row wrap onto rows
/// under it; the reply row stands on the first, the rows keeping the height the pills give them.
struct ActionRow {
    buttons: Vec<Rect>,
    labels: Vec<Option<String>>,
    more: Option<Rect>,
    field: Option<Rect>,
    send: Option<Rect>,
    /// The rows' bottom, design px; `None` with neither pills nor a reply.
    bottom: Option<i32>,
}

#[expect(clippy::too_many_arguments)]
fn action_row(
    text: &TextRenderer,
    actions: &[&str],
    icons: &[bool],
    reply: Option<NotifyReply>,
    (field_left, left, right, y): (i32, i32, i32, i32),
    s: i32,
    pills: &Pills,
    at: impl Fn(i32, i32, i32, i32) -> Rect,
) -> ActionRow {
    let room = right - left;
    let shown = actions.len().min(NOTIFY_BUTTONS);
    let mut widths = pill_widths(text, &actions[..shown], icons, pills, s);
    if actions.len() > NOTIFY_BUTTONS {
        widths.push(pills.more);
    }
    let flow = flow_buttons(&widths, room, PILL_GAP);
    let rows = flow_rows(&flow).max(i32::from(reply.is_some()));
    let row_y = |row: i32| y + row * (pills.h + PILL_GAP);
    let mut row = ActionRow {
        buttons: Vec::new(),
        labels: Vec::new(),
        more: None,
        field: None,
        send: None,
        bottom: (rows > 0).then(|| row_y(rows - 1) + pills.h),
    };
    if let Some(r) = reply {
        let sw = pill_widths(text, &[r.send], &[], pills, s)[0].min(room);
        let sx = right - sw;
        row.send = Some(at(sx, y, sw, pills.h));
        row.field = Some(at(field_left, y, sx - PILL_GAP - field_left, pills.h));
        return row;
    }
    for (i, f) in flow.iter().enumerate() {
        let pill = at(left + f.x, row_y(f.row), f.width, pills.h);
        if i == shown {
            row.more = Some(pill);
            continue;
        }
        row.buttons.push(pill);
        let room = (f.width - 2 * PILL_PAD) * s;
        let iconic = icons.get(i).copied().unwrap_or(false);
        row.labels.push((!iconic).then(|| {
            text.ellipsize(actions[i], room, true, pills.px)
                .into_owned()
        }));
    }
    row
}

/// Pill widths, design px: label plus padding, or for an action `icons` marks its icon plus
/// padding.
fn pill_widths(
    text: &TextRenderer,
    labels: &[&str],
    icons: &[bool],
    pills: &Pills,
    s: i32,
) -> Vec<i32> {
    labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            if icons.get(i).copied().unwrap_or(false) {
                ICON_PILL_W
            } else {
                ((text.measure(l, true, pills.px) + s - 1) / s + 2 * PILL_PAD).max(pills.min)
            }
        })
        .collect()
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
    let g = Grown::new(theme, f.title_px);
    let (app_px, title_px, body_px) = (g.app_px, g.title_px, g.body_px);
    let m = (look.margin)(f.shape);
    let dw = g.widen(f.width - f.pad - f.text.0);
    let w = f.width + dw;
    let text_y = f.text.1 + g.app_h - APP_H;
    let picture = f
        .picture
        .filter(|_| c.has_picture)
        .map(|side| Rect::new(f.text.0, text_y, side, side));
    let text_x = picture.map_or(f.text.0, |p| p.right() + PICTURE_GAP);
    let text_w = w - f.pad - text_x;
    let close_x = f.close.0 + dw;
    let head_w = close_x - CLOSE_GAP - f.app.0;
    let title_w = if f.title_by_close {
        close_x - CLOSE_GAP - text_x
    } else {
        text_w
    };
    let (title_y, body_y) = (text_y, text_y + g.title_h);

    let app_shown = text
        .ellipsize(&app_line(c), head_w * s, false, app_px)
        .into_owned();
    let title = text
        .ellipsize(c.title, title_w * s, true, title_px)
        .into_owned();
    let lines = body_lines(text, c.body, text_w * s, body_px);
    let text_bottom =
        (body_y + lines.len() as i32 * g.line_h).max(picture.map_or(0, |p| p.bottom()));

    let reply = c.reply.filter(|_| expanded);
    let row_y = text_bottom + ROW_GAP;
    let ActionRow {
        buttons,
        labels,
        more,
        field,
        send,
        bottom: row_bottom,
    } = action_row(
        text,
        c.actions,
        c.action_icons,
        reply,
        (f.row_x, f.pad, w - f.pad, row_y),
        s,
        &g.pills,
        Rect::new,
    );
    let bottom = row_bottom.unwrap_or(text_bottom);
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
        title: at(Rect::new(text_x, title_y, title_w, g.title_h)),
        body: at(Rect::new(
            text_x,
            body_y,
            text_w,
            lines.len().max(1) as i32 * g.line_h,
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
        app_box: at(Rect::new(f.app.0, f.app.1, head_w, g.app_h)),
        title,
        labels,
        line_h: g.line_h * s,
        pill_px: g.pills.px,
    }
}

/// The Droplet at 100% text, design px: a capsule row of round slot, title and app line, one
/// body line and the picture; expanded, the body wraps and the pills follow. Text taller than
/// the slot makes the row taller, the slot, picture and close centred on it.
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
const DROPLET_PAD: i32 = 14;
const DROPLET_OPEN_CLOSE_Y: i32 = 12;

fn droplet_layout(
    theme: &Theme,
    text: &TextRenderer,
    look: &NotifyLook,
    c: &SheetContent,
    expanded: bool,
) -> SheetLayout {
    let s = theme.scale.max(1);
    let g = Grown::new(theme, DROPLET_TITLE_PX);
    let (app_px, title_px, body_px) = (g.app_px, g.title_px, g.body_px);
    let m = (look.margin)(SheetShape::Droplet);
    let (ox, oy) = (m.left * s, m.top * s);
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::new(ox + x * s, oy + y * s, w * s, h * s);
    let text_x = DROPLET_TEXT_X;
    let w = DROPLET_W + g.widen(DROPLET_W - DROPLET_END - text_x);

    let has_body = c.body.split_whitespace().next().is_some();
    let block = g.title_h + if has_body { g.line_h } else { 0 };
    let row_h = DROPLET_SLOT.max(block);
    let shut_h = DROPLET_H.max(row_h + 2 * DROPLET_IN);
    let slot_y = DROPLET_IN + (row_h - DROPLET_SLOT) / 2;
    let title_y = DROPLET_IN + (row_h - block) / 2;
    let body_y = title_y + g.title_h;

    let picture_x = w - DROPLET_PICTURE_END - DROPLET_SLOT;
    let close_x = if c.has_picture {
        picture_x - CLOSE_GAP - CLOSE
    } else {
        w - DROPLET_END - CLOSE
    };
    let shut_close = (close_x, (shut_h - CLOSE) / 2);
    let open_close = if c.has_picture {
        (close_x, DROPLET_OPEN_CLOSE_Y)
    } else {
        (w - DROPLET_PAD - CLOSE, DROPLET_OPEN_CLOSE_Y)
    };
    let head_right = shut_close.0 - CLOSE_GAP;
    let text_right = match (expanded, c.has_picture) {
        (false, _) => head_right,
        (true, true) => picture_x - DROPLET_IN,
        (true, false) => w - DROPLET_END,
    };
    let text_w = text_right - text_x;
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

    let head = at(text_x, title_y, head_right - text_x, g.title_h);
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
    let actions = if expanded { c.actions } else { &[][..] };
    let text_bottom = body_y + lines.len() as i32 * g.line_h;
    let row_y = text_bottom.max(slot_y + DROPLET_SLOT) + ROW_GAP;
    let row = action_row(
        text,
        actions,
        c.action_icons,
        reply,
        (text_x, DROPLET_PAD, w - DROPLET_PAD, row_y),
        s,
        &g.pills,
        at,
    );
    let h = if let Some(bottom) = row.bottom {
        shut_h.max(bottom + DROPLET_PAD)
    } else if expanded {
        shut_h.max(text_bottom + DROPLET_PAD)
    } else {
        shut_h
    };

    let sheet = at(0, 0, w, h);
    let (close_x, close_y) = if expanded { open_close } else { shut_close };
    let parts = NotifyParts {
        bubble: sheet,
        title: head,
        body: at(text_x, body_y, text_w, lines.len().max(1) as i32 * g.line_h),
        close: at(close_x, close_y, CLOSE, CLOSE),
        buttons: row.buttons,
        more: row.more,
        field: row.field,
        send: row.send,
        placeholder: reply.map_or_else(String::new, |r| r.placeholder.to_string()),
        send_label: reply.map_or_else(String::new, |r| r.send.to_string()),
    };
    SheetLayout {
        width: (m.left + w + m.right) * s,
        height: (m.top + m.bottom + h) * s,
        shape: SheetShape::Droplet,
        expanded,
        sheet,
        footprint: sheet,
        icon: at(DROPLET_IN, slot_y, DROPLET_SLOT, DROPLET_SLOT),
        well: Well::Round,
        badge: None,
        picture: c
            .has_picture
            .then(|| at(picture_x, slot_y, DROPLET_SLOT, DROPLET_SLOT)),
        parts,
        lines,
        app_px,
        title_px,
        body_px,
        app_line: app_shown,
        app_box,
        title,
        labels: row.labels,
        line_h: g.line_h * s,
        pill_px: g.pills.px,
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
    let kept: Vec<(Rect, Option<String>)> = p
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
