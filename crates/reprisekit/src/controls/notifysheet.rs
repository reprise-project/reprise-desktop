//! A look's notification sheet, its stack by the tray and the "+N more" chip
//! past it. Classic and Blue show the balloon in [`super::tooltip`] instead.

use super::tooltip::{
    NOTIFY_BODY_LINES, NOTIFY_BUTTONS, NOTIFY_MORE, NotifyHit, NotifyParts, NotifyReply,
    NotifySays, ReplyField, access_parts, parts_hit,
};
use crate::access::Tree;
use crate::access::accesskit::NodeId;
use crate::classic::widgets::{draw_edit, edit_first_visible};
use crate::paint::Paint;
use crate::raster::{Canvas, Rect};
use crate::shape::{pack, rounded, scale, unpack};
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

/// The Lozenge's metrics, design px.
const LOZENGE_W: i32 = 360;
const PAD: i32 = 14;
const SLOT: i32 = 46;
const TEXT_GAP: i32 = 12;
const APP_PX: u32 = 12;
const TITLE_PX: u32 = 16;
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
const CLOSE_TOP: i32 = 12;
/// The app line and title end this far short of the close button.
const CLOSE_GAP: i32 = 4;
const BADGE: i32 = 18;
/// The badge centres this far in from the slot's lower-right corner.
const BADGE_IN: i32 = 4;
/// The app icon's inset inside the badge's ring.
const BADGE_ICON_IN: i32 = 2;
/// The reply field's text stands in this far from its rounded ends.
const FIELD_TEXT_IN: i32 = PILL_H / 2;

/// Lay out a sheet of `shape` for `c`; `expanded` shows the reply row in place of
/// the pills. `None` when the look has no sheets ([`has_sheets`]).
pub fn sheet_layout(
    theme: &Theme,
    text: &TextRenderer,
    shape: SheetShape,
    c: &SheetContent,
    expanded: bool,
) -> Option<SheetLayout> {
    let look = notify_look(theme)?;
    Some(match shape {
        SheetShape::Lozenge => lozenge_layout(theme, text, &look, c, expanded),
    })
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

fn lozenge_layout(
    theme: &Theme,
    text: &TextRenderer,
    look: &NotifyLook,
    c: &SheetContent,
    expanded: bool,
) -> SheetLayout {
    let s = theme.scale.max(1);
    let su = s as u32;
    let m = (look.margin)(SheetShape::Lozenge);
    let (ox, oy) = (m.left * s, m.top * s);
    let at = |x: i32, y: i32, w: i32, h: i32| Rect::new(ox + x * s, oy + y * s, w * s, h * s);
    let (app_px, title_px, body_px, pill_px) =
        (APP_PX * su, TITLE_PX * su, BODY_PX * su, PILL_PX * su);
    let text_x = PAD + SLOT + TEXT_GAP;
    let text_w = LOZENGE_W - text_x - PAD;
    let close_x = LOZENGE_W - PAD - CLOSE;
    let head_w = close_x - CLOSE_GAP - text_x;
    let (title_y, body_y) = (PAD + APP_H, PAD + APP_H + TITLE_H);

    let app = if c.when.is_empty() {
        c.app.to_string()
    } else {
        format!("{} \u{b7} {}", c.app, c.when)
    };
    let app_line = text.ellipsize(&app, head_w * s, false, app_px).into_owned();
    let title = text
        .ellipsize(c.title, head_w * s, true, title_px)
        .into_owned();
    let mut lines = wrap(text, c.body, text_w * s, body_px);
    lines.truncate(NOTIFY_BODY_LINES);
    let text_bottom = body_y + lines.len() as i32 * LINE_H;

    let reply = c.reply.filter(|_| expanded);
    let actions = if reply.is_some() { &[][..] } else { c.actions };
    let shown = actions.len().min(NOTIFY_BUTTONS);
    let more_w = if actions.len() > NOTIFY_BUTTONS {
        MORE_W
    } else {
        0
    };
    let row_y = text_bottom + ROW_GAP;
    let right = LOZENGE_W - PAD;
    let (mut buttons, mut labels, mut more, mut field, mut send) =
        (Vec::new(), Vec::new(), None, None, None);
    let row = if let Some(r) = reply {
        let sw = pill_widths(text, &[r.send], pill_px, s, text_w)[0];
        let sx = right - sw;
        send = Some(at(sx, row_y, sw, PILL_H));
        field = Some(at(text_x, row_y, sx - PILL_GAP - text_x, PILL_H));
        true
    } else if shown > 0 || more_w > 0 {
        let parts = shown + usize::from(more_w > 0);
        let room = text_w - more_w - (parts as i32 - 1) * PILL_GAP;
        let widths = pill_widths(text, &actions[..shown], pill_px, s, room);
        let mut x = right;
        if more_w > 0 {
            x -= more_w;
            more = Some(at(x, row_y, more_w, PILL_H));
            x -= PILL_GAP;
        }
        for (a, w) in actions[..shown].iter().zip(&widths).rev() {
            x -= w;
            buttons.push(at(x, row_y, *w, PILL_H));
            let room = (w - 2 * PILL_PAD) * s;
            labels.push(text.ellipsize(a, room, true, pill_px).into_owned());
            x -= PILL_GAP;
        }
        buttons.reverse();
        labels.reverse();
        true
    } else {
        false
    };
    let bottom = if row { row_y + PILL_H } else { text_bottom };
    let h = (PAD + SLOT + PAD).max(bottom + PAD);

    let sheet = at(0, 0, LOZENGE_W, h);
    let icon = at(PAD, PAD, SLOT, SLOT);
    let badge = (c.has_picture && c.has_app_icon).then(|| {
        let half = BADGE / 2 * s;
        let (cx, cy) = (icon.right() - BADGE_IN * s, icon.bottom() - BADGE_IN * s);
        Rect::new(cx - half, cy - half, BADGE * s, BADGE * s)
    });
    let parts = NotifyParts {
        bubble: sheet,
        title: at(text_x, title_y, head_w, TITLE_H),
        body: at(text_x, body_y, text_w, lines.len().max(1) as i32 * LINE_H),
        close: at(close_x, CLOSE_TOP, CLOSE, CLOSE),
        buttons,
        more,
        field,
        send,
        placeholder: reply.map_or_else(String::new, |r| r.placeholder.to_string()),
        send_label: reply.map_or_else(String::new, |r| r.send.to_string()),
    };
    SheetLayout {
        width: (m.left + LOZENGE_W + m.right) * s,
        height: (m.top + m.bottom) * s + h * s,
        shape: SheetShape::Lozenge,
        expanded,
        sheet,
        footprint: sheet,
        icon,
        well: Well::Squircle,
        badge,
        picture: c.has_picture.then_some(icon),
        parts,
        lines,
        app_px,
        title_px,
        body_px,
        app_line,
        app_box: at(text_x, PAD, head_w, APP_H),
        title,
        labels,
        line_h: LINE_H * s,
        pill_px,
    }
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
}

#[derive(Clone, Copy, PartialEq)]
struct GlassKey {
    spec: SheetSpec,
    scale: i32,
    dark: bool,
}

impl GlassKey {
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
        let mut c = self.glass(&look, lay, tint, hot.is_some(), theme.scale.max(1));
        draw_parts(&mut c, theme, text, &look, lay, icons, pushed, hot, reply);
        c
    }

    /// The glass under `lay`, painted unless kept; glass for another sheet is dropped.
    fn glass(
        &mut self,
        look: &NotifyLook,
        lay: &SheetLayout,
        tint: Option<Color>,
        lit: bool,
        s: i32,
    ) -> Canvas {
        let key = GlassKey {
            spec: sheet_spec(lay, tint, lit, s),
            scale: s,
            dark: look.dark,
        };
        self.kept.retain(|(k, _)| k.under_either_rim(&key));
        if let Some((_, c)) = self.kept.iter().find(|(k, _)| *k == key) {
            return c.clone();
        }
        let c = (look.sheet)(look, &key.spec, s);
        self.builds += 1;
        self.kept.push((key, c.clone()));
        c
    }

    /// How many times the glass was painted.
    pub fn builds(&self) -> u32 {
        self.builds
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
        let round = SQUIRCLE_ROUND * f64::from(r.width);
        c.blit_over(r.x, r.y, &round_corners(pic, round));
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

    let pill = |c: &mut Canvas, r: Rect, label: &str, state: State| {
        let face = (look.pill)(look, (r.width / s, r.height / s), state, s);
        c.blit_over(r.x, r.y, &face);
        let ink = if state == State::Disabled {
            inks.app
        } else {
            inks.title
        };
        text.draw(c, r, label, true, ink, Align::Center, lay.pill_px);
    };
    for (i, (r, label)) in lay.parts.buttons.iter().zip(&lay.labels).enumerate() {
        pill(c, *r, label, pill_state(NotifyHit::Button(i), pushed, hot));
    }
    if let Some(r) = lay.parts.more {
        pill(c, r, NOTIFY_MORE, pill_state(NotifyHit::More, pushed, hot));
    }
    if let Some(r) = lay.parts.send {
        let state = if edit.text.trim().is_empty() {
            State::Disabled
        } else {
            pill_state(NotifyHit::Send, pushed, hot)
        };
        let room = r.width - 2 * PILL_PAD * s;
        let label = text.ellipsize(&lay.parts.send_label, room, true, lay.pill_px);
        pill(c, r, &label, state);
    }
    if hot.is_some() {
        let r = lay.parts.close;
        let state = pill_state(NotifyHit::Close, pushed, hot);
        c.blit_over(r.x, r.y, &(look.close)(look, state, s));
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
