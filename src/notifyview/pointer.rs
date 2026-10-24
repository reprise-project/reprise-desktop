//! Pointer hits, hover, presses and releases on balloons and the chip.

use reprisekit::controls::notifysheet::{opening_shows_close, sheet_click};
use reprisekit::controls::tooltip::{NOTIFY_BUTTONS, NotifyHit, ReplyField};
use std::time::Instant;

use crate::compositor::State;
use crate::input::{BTN_LEFT, BTN_RIGHT, Hit, Interaction};
use crate::raster::Rect;
use crate::tray::notify::DEFAULT_ACTION;

use super::clock::{hold_clocks, rearm};
use super::layout::Drawn;
use super::motion::{aim_open, leave, still};
use super::paint::paint;
use super::reply::{reply_touched, send_reply};
use super::stack::{Item, display_next};
use super::{Balloon, NotifyView, REASON_DISMISSED, close, invoke, replying, sheets};

/// The balloon under layout point (`x`, `y`) where it is drawn now: its index, layout, the part
/// hit and its device x in the canvas.
fn stack_hit(state: &State, x: f64, y: f64) -> Option<(usize, Drawn, NotifyHit, i32)> {
    let now = Instant::now();
    hit_in(&state.notify, x, y, |b| b.hit_at(now))
}

/// `stack_hit` with each balloon at `place`, logical.
fn hit_in(
    v: &NotifyView,
    x: f64,
    y: f64,
    place: impl Fn(&Balloon) -> (i32, i32),
) -> Option<(usize, Drawn, NotifyHit, i32)> {
    v.stack.iter().enumerate().find_map(|(i, b)| {
        let drawn = b.drawn.as_ref()?;
        let (ox, oy) = drawn.origin();
        let (ax, ay) = place(b);
        let lx = ((x - f64::from(ax)) * f64::from(b.scale)).floor() as i32 + ox;
        let ly = ((y - f64::from(ay)) * f64::from(b.scale)).floor() as i32 + oy;
        let (w, h) = drawn.size();
        if !(0..w).contains(&lx) || !(0..h).contains(&ly) {
            return None;
        }
        let hit = drawn.hit(lx, ly)?;
        Some((i, drawn.clone(), hit, lx))
    })
}

/// The chip's rect, logical, when the pointer at (`x`, `y`) is on it.
fn chip_hit(v: &NotifyView, x: f64, y: f64) -> Option<Rect> {
    let c = v.chip.as_ref()?;
    let r = Rect::new(c.at.0, c.at.1, c.size.0, c.size.1);
    r.contains(x.floor() as i32, y.floor() as i32).then_some(r)
}

/// A press of `button` on a balloon or the chip at layout point (`x`, `y`); true when the pointer
/// now holds a button. Any button but the left dismisses a balloon, except a right press on the
/// reply field, which opens its edit menu.
pub fn press(state: &mut State, x: f64, y: f64, button: u32) -> bool {
    let target = press_target(&mut state.notify, x, y, Instant::now());
    if hold_clocks(state) {
        rearm(state);
    }
    let Some((i, drawn, hit, lx)) = target else {
        if let Some(cell) = chip_hit(&state.notify, x, y)
            && let Some(output) = state.first_output()
        {
            crate::notifycenter::toggle(state, &output, cell);
        }
        return false;
    };
    reply_touched(state);
    if button == BTN_RIGHT && hit == NotifyHit::Field {
        field_press(state, i, &drawn, lx, true);
        state.open_edit_menu(Some((x as i32, y as i32)));
        return false;
    }
    let hit = if button == BTN_LEFT {
        hit
    } else {
        NotifyHit::Close
    };
    let n = match state.notify.stack[i].item.clone() {
        Item::Burst(burst) => {
            let b = state.notify.stack.remove(i);
            leave(state, b);
            if hit == NotifyHit::Close {
                display_next(state);
            } else {
                paint(state);
                crate::notifycenter::reveal(state, &burst.app);
            }
            return false;
        }
        Item::Note(n) => n,
    };
    let id = n.id;
    match hit {
        NotifyHit::Close => close(state, id, REASON_DISMISSED),
        NotifyHit::Body => {
            if n.has_action(DEFAULT_ACTION) {
                invoke(state, id, DEFAULT_ACTION);
            } else {
                close(state, id, REASON_DISMISSED);
            }
        }
        NotifyHit::Button(_) | NotifyHit::Send => {
            state.notify.stack[i].held = Some((hit, true));
            paint(state);
            return true;
        }
        NotifyHit::Field => field_press(state, i, &drawn, lx, false),
        NotifyHit::More => open_more_menu(state, id),
    }
    false
}

/// A press on balloon `i`'s reply field at canvas x `lx`: it takes the keyboard and the caret
/// goes there, unless `for_menu` and the press is inside the selection.
fn field_press(state: &mut State, i: usize, drawn: &Drawn, lx: i32, for_menu: bool) {
    let theme = state.config.theme.scaled(lay_scale(state));
    let text = &state.text;
    if let Some(field) = state.notify.stack[i].reply.as_mut() {
        let press = |field: &mut ReplyField| match drawn {
            Drawn::Balloon(lay) => field.click(&theme, text.for_output(theme.scale), lay, lx),
            Drawn::Sheet(lay) => sheet_click(lay, &theme, text.for_output(theme.scale), field, lx),
        };
        if for_menu {
            field.press_for_menu(press);
        } else {
            press(field);
        }
    }
    state.restart_caret();
    paint(state);
}

/// The replying balloon's scale, else the newest's.
fn lay_scale(state: &State) -> i32 {
    let v = &state.notify;
    replying(v)
        .or((!v.stack.is_empty()).then_some(0))
        .map_or(1, |i| v.stack[i].scale)
}

/// The pointer moved, `over` the stack or not: the part under it lights (on a sheet, its close
/// button too) and sheets hold their clocks; crossing the replying balloon restarts `REPLY_IDLE`.
pub fn hover(state: &mut State, x: f64, y: f64, over: bool) {
    let (dirty, touched) = point_at(&mut state.notify, x, y, over, Instant::now());
    show_marks(state, dirty);
    if touched {
        reply_touched(state);
    }
}

/// The stack came to rest under the still pointer: the sheet now under it lights as on a move,
/// and one slid away from it goes dark. A crossing it did not make leaves `REPLY_IDLE` alone.
pub(super) fn rehover(state: &mut State) {
    if !sheets(state) || !matches!(state.interaction, Interaction::Idle) {
        return;
    }
    let p = state.pointer_location();
    let over = matches!(state.scene_hit(p), Hit::Notification);
    if let Some((dirty, _)) = rest_point(&mut state.notify, p.x, p.y, over, Instant::now()) {
        show_marks(state, dirty);
    }
}

/// `point_at` once nothing in the stack moves, each sheet at rest; `None` while one moves.
pub(super) fn rest_point(
    v: &mut NotifyView,
    x: f64,
    y: f64,
    over: bool,
    now: Instant,
) -> Option<(bool, bool)> {
    still(v, now).then(|| point_at(v, x, y, over, now))
}

/// Turns the Droplets and the clocks after `point_at`, and repaints when its marks or an opening
/// changed.
fn show_marks(state: &mut State, dirty: bool) {
    let dirty = aim_open(state) | dirty;
    if hold_clocks(state) {
        rearm(state);
    }
    if dirty {
        paint(state);
    }
}

/// `hover`'s marks on the stack and chip; returns whether to repaint and whether the pointer
/// crossed the replying balloon.
pub(super) fn point_at(
    v: &mut NotifyView,
    x: f64,
    y: f64,
    over: bool,
    now: Instant,
) -> (bool, bool) {
    let under = over.then(|| hit_in(v, x, y, |b| b.hit_at(now))).flatten();
    let on_chip = over && under.is_none() && chip_hit(v, x, y).is_some();
    let (mut dirty, mut touched) = (false, false);
    for (i, b) in v.stack.iter_mut().enumerate() {
        let mine = under.as_ref().filter(|(j, ..)| *j == i);
        let hot = mine
            .filter(|(_, d, h, _)| *h != NotifyHit::Body || matches!(d, Drawn::Sheet(_)))
            .map(|(_, _, h, _)| *h);
        let was_in = std::mem::replace(&mut b.pointer_in, mine.is_some());
        if b.hot != hot {
            b.hot = hot;
            dirty = true;
        }
        touched |= b.reply.is_some() && (mine.is_some() || was_in);
    }
    if let Some(c) = v.chip.as_mut()
        && c.hot != on_chip
    {
        c.hot = on_chip;
        dirty = true;
    }
    v.pointer_on = under.is_some() || on_chip;
    (dirty, touched)
}

/// The still pointer at (`x`, `y`) holds the stack while a sheet is under it, drawn or at rest,
/// or the chip, and lets it go otherwise; true on a change.
pub(super) fn hold_at(v: &mut NotifyView, x: f64, y: f64, now: Instant) -> bool {
    let on = hit_in(v, x, y, |b| b.hit_at(now)).is_some()
        || hit_in(v, x, y, |b| b.at).is_some()
        || chip_hit(v, x, y).is_some();
    std::mem::replace(&mut v.pointer_on, on) != on
}

/// `stack_hit` for a press, which takes the hold first. A close spot takes no press while its sheet
/// slides into place, sparing the sheet sliding up, or before an opening Droplet's close shows.
pub(super) fn press_target(
    v: &mut NotifyView,
    x: f64,
    y: f64,
    now: Instant,
) -> Option<(usize, Drawn, NotifyHit, i32)> {
    hold_at(v, x, y, now);
    hit_in(v, x, y, |b| b.hit_at(now)).filter(|(i, _, hit, _)| {
        let m = &v.stack[*i].motion;
        *hit != NotifyHit::Close
            || m.placed(now) && (m.open.settled(now) || opening_shows_close(m.open.value(now)))
    })
}

pub(super) fn held(v: &NotifyView) -> Option<(usize, NotifyHit, bool)> {
    v.stack
        .iter()
        .enumerate()
        .find_map(|(i, b)| b.held.map(|(part, down)| (i, part, down)))
}

/// The pointer moved while it holds a balloon button: the button shows pushed only while the
/// pointer is over it.
pub fn drag(state: &mut State, x: f64, y: f64) {
    let Some((i, part, down)) = held(&state.notify) else {
        return;
    };
    let over = stack_hit(state, x, y).is_some_and(|(j, _, h, _)| j == i && h == part);
    if over != down {
        state.notify.stack[i].held = Some((part, over));
        paint(state);
    }
}

/// The pointer let go of a balloon button: its action runs, or Send sends, if the release is over
/// it.
pub fn release(state: &mut State, x: f64, y: f64) {
    let Some((i, part, _)) = held(&state.notify) else {
        return;
    };
    state.notify.stack[i].held = None;
    let over = stack_hit(state, x, y).is_some_and(|(j, _, h, _)| j == i && h == part);
    paint(state);
    if !over {
        return;
    }
    let b = &state.notify.stack[i];
    match part {
        NotifyHit::Send => {
            if let Some(text) = b.reply.as_ref().and_then(ReplyField::to_send) {
                send_reply(state, text);
            }
        }
        NotifyHit::Button(k) => {
            let Some(n) = b.item.note() else {
                return;
            };
            let (id, key) = (n.id, n.button_actions().nth(k).map(|(k, _)| k.clone()));
            if let Some(key) = key {
                invoke(state, id, &key);
            }
        }
        _ => {}
    }
}

/// The pointer is taken away mid-press: a held balloon button lets go without acting.
pub fn cancel_press(state: &mut State) {
    if let Some((i, ..)) = held(&state.notify) {
        state.notify.stack[i].held = None;
        paint(state);
    }
}

/// Open the "..." button's menu: the actions past the balloon's buttons, its right edge at the
/// balloon's left so the balloon does not cover it.
fn open_more_menu(state: &mut State, id: u32) {
    let Some(b) = state.notify.stack.iter().find(|b| b.item.is(id)) else {
        return;
    };
    let (Some(n), Some(drawn)) = (b.item.note(), b.drawn.as_ref()) else {
        return;
    };
    let rest: Vec<(String, String)> = n.button_actions().skip(NOTIFY_BUTTONS).cloned().collect();
    let oy = drawn.origin().1;
    let more_y = drawn.parts().more.map_or(0, |m| (m.y - oy) / b.scale);
    let at = (b.at.0, b.at.1 + more_y);
    crate::notifycenter::open_actions_menu(state, id, rest, at, true);
}
