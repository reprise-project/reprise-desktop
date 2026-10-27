//! Drawing a sheet: icons, text, pills, close and reply field.

use crate::classic::widgets::{draw_edit, edit_first_visible};
use crate::controls::tooltip::{NOTIFY_MORE, NotifyHit};
use crate::paint::Paint;
use crate::raster::{Canvas, Rect};
use crate::shape::{pack, rounded, scale, unpack};
use crate::style::{NotifyLook, SQUIRCLE_ROUND, SheetShape, State, Well};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, Preedit, TextEdit};
use crate::theme::{Color, Theme};

use super::SheetLayout;
use super::glass::SheetGlass;
use super::layout::{BADGE_ICON_IN, FIELD_TEXT_IN, PILL_PAD};

/// The app icon, the notification's own picture and each pill's action icon, device px.
#[derive(Clone, Copy, Debug, Default)]
pub struct SheetIcons<'a> {
    pub app: Option<&'a Canvas>,
    pub picture: Option<&'a Canvas>,
    pub actions: &'a [Option<Canvas>],
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

#[expect(clippy::too_many_arguments)]
pub(super) fn draw_parts(
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
    for (i, (r, label, state)) in pill_row(text, lay, edit_empty, pushed, hot, s)
        .into_iter()
        .enumerate()
    {
        draw_pill(
            c,
            text,
            look,
            lay,
            r,
            face(&label, icons.actions, i),
            state,
            s,
        );
    }
    if hot.is_some() {
        draw_close(c, look, lay, pushed, hot, s);
    }
}

/// A pill's label, or with none the icon of action `i`.
fn face<'a>(label: &'a Option<String>, icons: &'a [Option<Canvas>], i: usize) -> Face<'a> {
    match label {
        Some(l) => Face::Label(l),
        None => Face::Icon(icons.get(i).and_then(Option::as_ref)),
    }
}

#[derive(Clone, Copy)]
enum Face<'a> {
    Label(&'a str),
    Icon(Option<&'a Canvas>),
}

/// Each pill on `lay` with its label (None for an action's icon) and state; the buttons come
/// first, in order.
fn pill_row(
    text: &TextRenderer,
    lay: &SheetLayout,
    edit_empty: bool,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    s: i32,
) -> Vec<(Rect, Option<String>, State)> {
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
            Some(NOTIFY_MORE.to_string()),
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
        row.push((r, Some(label.into_owned()), state));
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
    face: Face,
    state: State,
    s: i32,
) {
    let pill = (look.pill)(look, (r.width / s, r.height / s), state, s);
    c.blit_over(r.x, r.y, &pill);
    let ink = if state == State::Disabled {
        look.inks.app
    } else {
        look.inks.title
    };
    match face {
        Face::Label(label) => text.draw(c, r, label, true, ink, Align::Center, lay.pill_px),
        Face::Icon(Some(icon)) => {
            let (w, h) = (icon.width as i32, icon.height as i32);
            c.blit_over(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, icon);
        }
        Face::Icon(None) => {}
    }
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
pub(super) fn relight(
    parts: &mut Canvas,
    text: &TextRenderer,
    look: &NotifyLook,
    lay: &SheetLayout,
    actions: &[Option<Canvas>],
    edit_empty: bool,
    pushed: Option<NotifyHit>,
    hot: Option<NotifyHit>,
    s: i32,
) {
    let unlit = pill_row(text, lay, edit_empty, None, None, s);
    let lit = pill_row(text, lay, edit_empty, pushed, hot, s);
    for (i, ((r, label, state), (.., was))) in lit.into_iter().zip(unlit).enumerate() {
        if state == was {
            continue;
        }
        parts.clear_rect(r);
        draw_pill(
            parts,
            text,
            look,
            lay,
            r,
            face(&label, actions, i),
            state,
            s,
        );
    }
    if hot.is_some() {
        draw_close(parts, look, lay, pushed, hot, s);
    }
}

pub(super) fn field_text(lay: &SheetLayout, s: i32) -> Option<Rect> {
    let d = FIELD_TEXT_IN * s;
    lay.parts
        .field
        .map(|f| Rect::new(f.x + d, f.y, f.width - 2 * d, f.height))
}
