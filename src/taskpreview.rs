//! The task button's preview panel on the scene graph. With `[previews] taskbar` on, resting the
//! pointer on a task button for the tooltip delay opens a `reprisekit::controls::taskpreview` panel on
//! the menu layer, its edge on the taskbar's on the screen side, with each window behind the button
//! shown live (`livepreview`). It replaces the button's text tip. The panel stays while the pointer is
//! on the button, the panel or the bar between them, moves at once to another task button the
//! pointer reaches, over the bar's empty rows and the gaps between buttons too, and goes when the
//! pointer leaves them or anything is pressed. The hovered cell shows a close button: a left click on
//! it closes that window, elsewhere on a cell it activates the window. The popup is laid out and drawn
//! in the look's own way (`interfaces::task_preview_grid`, `interfaces::draw_task_preview`); under
//! glass its canvas reaches over a shadow round the panel and the panel shows the blurred backdrop
//! (`glass::Backdrop`). Resting on a cell peeks at its window (`peek`); the peek holds across the gaps
//! between cells and ends when the pointer leaves the panel. The lock hides it, and none opens while locked.
//! The taskbar's keyboard focus (`barkeys`) on a task button shows its panel after the same delay, and that panel stays
//! until the focus moves on, whatever the pointer does. Enter on a group opens the panel at once with the arrows
//! (`open_keys`): they move the hot cell, Enter brings its window up, Delete closes it, Escape goes back to the button.
//! A window with a media player (`previewmedia`) gets Previous, Play or Pause, and Next under its picture, greyed where
//! the player can't; a click presses the player and the panel stays. With the arrows, Tab steps through the hot
//! cell's buttons, Enter and Space press the one focused and Escape goes back to the cell. On a one-window panel shown
//! for the bar's focus, Tab from the button steps onto its buttons (`tab_in`), and Tab past the last goes on along the
//! bar. The buttons follow the
//! players while the panel is up, and a window gaining its first player or losing its last lays the panel out again.

use std::{sync::Arc, time::Duration};

use reprisekit::{
    controls::taskpreview::{self, Cell, Media, PanelKey, TaskPreviewLayout},
    controls::volumepopup::{NowPlaying, Transport},
    interfaces,
    pointer::Pointer,
    taskbar::{AlongTasks, Edge},
};
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::{
        RegistrationToken,
        timer::{TimeoutAction, Timer},
    },
    utils::Rectangle,
};

use crate::{
    barplace::BarPlacement,
    compositor::{SceneOwner, State},
    glass::{self, Backdrop, Pane, PaneShape},
    livepreview::ThumbId,
    output::upload_scaled,
    peek::{self, PeekTarget},
    raster::{Canvas, Rect},
    taskbar::{TaskKey, TaskbarHit},
    window::WindowId,
};

const SHOW_MS: u64 = reprisekit::controls::tooltip::SHOW_MS as u64;
const SMALL_ICON: i32 = 16;
const WELL_ICON: i32 = 32;

#[derive(Default)]
pub struct TaskPreviewView {
    timer: Option<RegistrationToken>,
    /// The task button waiting for its panel: its bar and index.
    pending: Option<(TreeId, usize)>,
    shown: Option<Panel>,
    peeking: bool,
    /// The windows of the `pending` button a live picture showed when it armed (a closing overview
    /// or 3D switcher): they stay awake (`State::mark_suspended`) until the panel shows them or the
    /// wait ends.
    awake: Vec<WindowId>,
    /// The pictures `hold` took out of the panel and the windows it kept awake, until `release`.
    held: Vec<ThumbId>,
    held_awake: Vec<WindowId>,
    /// The task button last pressed with the pointer, by its bar and key so it is followed through any
    /// change to the row: its panel waits until the pointer leaves it.
    pressed: Option<(TreeId, TaskKey<WindowId>)>,
    /// The pending or shown panel follows the taskbar's keyboard focus (`focus`): the pointer leaving
    /// takes it down no more.
    keyed: bool,
}

struct Panel {
    /// The bar (`Bar::tree`) and button it belongs to, and the bar's output.
    bar: TreeId,
    button: usize,
    output: Output,
    windows: Vec<WindowId>,
    layout: TaskPreviewLayout,
    /// In `layers.menu`, tagged `SceneOwner::TaskPreview`, at the panel's top-left: the glass (when drawn translucent), the canvas, then the pictures.
    tree: TreeId,
    buffer: BufferId,
    thumbs: Vec<Option<ThumbId>>,
    hot: Option<usize>,
    /// The panel has the arrows (`open_keys`), `hot` the keyboard's cell.
    keys: bool,
    /// The keyboard reached the one window's media buttons with Tab from its task button (`tab_in`):
    /// keys past them go back to the bar.
    media_only: bool,
    /// Each window's player by bus name, and what its buttons show.
    players: Vec<Option<(String, NowPlaying)>>,
    /// The media button the pointer is on or holds, by cell.
    ptr: Pointer<(usize, Transport)>,
    /// The hot cell's media button with the keyboard, after Tab.
    media_focus: Option<Transport>,
}

impl Panel {
    fn now_playing(&self) -> Vec<Option<NowPlaying>> {
        (self.players.iter())
            .map(|p| p.as_ref().map(|(_, np)| np.clone()))
            .collect()
    }

    /// The media button under the pointer at `at`, in layout coordinates.
    fn media_at(&self, at: (f64, f64)) -> Option<(usize, Transport)> {
        let r = self.layout.rect;
        let (x, y) = (at.0.floor() as i32 - r.x, at.1.floor() as i32 - r.y);
        taskpreview::media_hit(&self.layout, &self.now_playing(), x, y)
    }
}

/// The pointer is over `hit` on bar `bar`. A task button starts the delay, or takes over a panel at once, except the one just pressed (`pressed`); anything else on the bar takes it down, but for the rows between the button and its panel and the bar's empty rows along the task buttons (`open_along`).
pub fn hover(state: &mut State, bar: TreeId, hit: TaskbarHit) {
    arm(state, bar, hit, false);
}

/// The taskbar's keyboard focus moved to `at` on bar `bar`: a task button arms its panel as a resting
/// pointer does; anything else takes the panel down.
pub fn focus(state: &mut State, bar: TreeId, at: TaskbarHit) {
    state.task_preview.pressed = None;
    match at {
        TaskbarHit::Task(_) if state.config.previews.taskbar => arm(state, bar, at, true),
        _ => hide(state),
    }
}

/// The taskbar lost the keyboard: a panel its focus armed goes.
pub fn keys_gone(state: &mut State) {
    if state.task_preview.keyed {
        hide(state);
    }
}

fn arm(state: &mut State, bar: TreeId, hit: TaskbarHit, keys: bool) {
    stop_peek(state);
    let target = match hit {
        TaskbarHit::Task(i) if state.config.previews.taskbar => (bar, i),
        _ if bridged(state, bar) => {
            hover_panel(state, None);
            return;
        }
        TaskbarHit::Nothing => match open_along(state, bar) {
            Some(AlongTasks::Button(i)) => (bar, i),
            Some(AlongTasks::Gap) => {
                hover_panel(state, None);
                return;
            }
            None => {
                leave(state);
                return;
            }
        },
        _ => {
            leave(state);
            return;
        }
    };
    let pressed = state.task_preview.pressed.filter(|&(b, _)| b == bar);
    let slots = state
        .bar(bar)
        .map(|b| b.slots.as_slice())
        .unwrap_or_default();
    if pressed.is_some_and(|(_, key)| crate::taskbar::find_task(slots, &key) == Some(target.1)) {
        return;
    }
    state.task_preview.pressed = None;
    let v = &state.task_preview;
    let shown = v.shown.as_ref().map(|p| (p.bar, p.button));
    if shown == Some(target) || v.pending == Some(target) {
        state.task_preview.keyed |= keys;
        return;
    }
    state.task_preview.keyed = keys;
    if shown.is_some() {
        show(state, target.0, target.1);
        return;
    }
    let slot = state.bar(target.0).and_then(|b| b.slots.get(target.1));
    let awake = (slot
        .map(|s| s.windows.as_slice())
        .unwrap_or_default()
        .iter())
    .copied()
    .filter(|&w| state.live_previews.shows(w))
    .collect();
    let was = std::mem::replace(&mut state.task_preview.awake, awake);
    state.task_preview.pending = Some(target);
    disarm(state);
    resuspend(state, was);
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(SHOW_MS)),
        |_, _, state| {
            state.task_preview.timer = None;
            let awake = std::mem::take(&mut state.task_preview.awake);
            if let Some((bar, i)) = state.task_preview.pending.take() {
                show(state, bar, i);
            }
            resuspend(state, awake);
            TimeoutAction::Drop
        },
    );
    match token {
        Ok(t) => state.task_preview.timer = Some(t),
        Err(e) => log::warn!("task preview timer: {}", e.error),
    }
}

/// With a pointer's panel shown on bar `bar`, where the pointer lies along its task buttons, so the
/// pointer crossing the bar's empty rows or a gap to the next button switches the panel at once.
fn open_along(state: &State, bar: TreeId) -> Option<AlongTasks> {
    let v = &state.task_preview;
    v.shown.as_ref().filter(|p| p.bar == bar && !v.keyed)?;
    let b = state.bar(bar)?;
    let (at, o) = (state.pointer_location(), state.bar_drawn(&b.output).shown);
    b.layout
        .along_tasks(at.x.floor() as i32 - o.x, at.y.floor() as i32 - o.y)
}

/// Whether the pointer is on bar `bar` between the shown panel's button and the panel
/// (`TaskPreviewLayout::bridges`), with the panel following the pointer.
fn bridged(state: &State, bar: TreeId) -> bool {
    let at = state.pointer_location();
    let v = &state.task_preview;
    (v.shown.as_ref())
        .filter(|p| p.bar == bar && !v.keyed)
        .is_some_and(|p| p.layout.bridges(at.x.floor() as i32, at.y.floor() as i32))
}

pub fn hover_panel(state: &mut State, cell: Option<usize>) {
    let at = state.pointer_location();
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    let window = cell.and_then(|i| p.windows.get(i).copied());
    let over = cell.and(p.media_at((at.x, at.y)));
    let lit = p.ptr.motion(over);
    if p.hot != cell {
        p.hot = cell;
        p.media_focus = None;
        redraw(state);
    } else if lit {
        redraw(state);
    }
    match window {
        Some(w) => {
            state.task_preview.peeking = true;
            peek::hover(state, PeekTarget::Window(w));
        }
        None if !state.peek.active() => stop_peek(state),
        None => {}
    }
}

fn stop_peek(state: &mut State) {
    if std::mem::take(&mut state.task_preview.peeking) {
        peek::leave(state);
    }
}

fn disarm(state: &mut State) {
    if let Some(t) = state.task_preview.timer.take() {
        state.handle.remove(t);
    }
}

pub fn hit(state: &State, lx: f64, ly: f64) -> Option<usize> {
    let p = state.task_preview.shown.as_ref()?;
    let r = p.layout.rect;
    taskpreview::hit(&p.layout, lx.floor() as i32 - r.x, ly.floor() as i32 - r.y)
}

/// A left press on the panel, on `cell`: a media button holds until the release (`media_release`), a greyed one
/// does nothing; its close button closes its window, elsewhere activates it, and the panel goes.
pub fn clicked(state: &mut State, cell: Option<usize>) {
    if media_press(state) {
        return;
    }
    let Some((i, p)) = cell.zip(state.task_preview.shown.as_ref()) else {
        hide(state);
        return;
    };
    let Some(w) = p.windows.get(i).copied() else {
        hide(state);
        return;
    };
    let r = p.layout.rect;
    let at = state.pointer_location();
    let (cx, cy) = (at.x.floor() as i32, at.y.floor() as i32);
    let on_close = taskpreview::close_hit(&p.layout, p.hot, cx - r.x, cy - r.y) == Some(i);
    pick(state, w, on_close);
}

/// A left press on a media button: true when the pointer is on one, which holds it if its player can do what it does.
fn media_press(state: &mut State) -> bool {
    let at = state.pointer_location();
    let Some(p) = state.task_preview.shown.as_mut() else {
        return false;
    };
    let Some((i, t)) = p.media_at((at.x, at.y)) else {
        return false;
    };
    if p.players[i].as_ref().is_some_and(|(_, np)| np.can(t)) && p.ptr.press(Some((i, t)), false) {
        state.interaction = crate::input::Interaction::PreviewPress;
        redraw(state);
    }
    true
}

/// The pointer moved with a media button held: it draws pushed while the pointer is on it.
pub fn media_drag(state: &mut State) {
    let at = state.pointer_location();
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    let over = p.media_at((at.x, at.y));
    if p.ptr.motion(over) {
        redraw(state);
    }
}

/// The held media button let go: over it, its player gets the press.
pub fn media_release(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    let pressed = p.ptr.release();
    let call = pressed.and_then(|(i, t)| Some((p.players.get(i)?.as_ref()?.0.clone(), t)));
    if let Some((bus, t)) = call {
        crate::previewmedia::send(state, &bus, t);
    }
    redraw(state);
}

pub fn media_cancel(state: &mut State) {
    if let Some(p) = state.task_preview.shown.as_mut()
        && p.ptr.clear()
    {
        redraw(state);
    }
}

/// The players changed: the panel's buttons follow, and a panel whose windows gained their first player or lost
/// their last is laid out again.
pub fn players_changed(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return;
    };
    let players = crate::previewmedia::players_of(state, &p.windows);
    let row = players.iter().any(Option::is_some);
    if row != p.layout.grid.media.is_some() {
        reshow(state);
        return;
    }
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    if players == p.players {
        return;
    }
    p.players = players;
    let hot_player = p.hot.and_then(|i| p.players.get(i)?.as_ref());
    let focus_kept = p
        .media_focus
        .zip(hot_player)
        .is_some_and(|(t, (_, np))| np.can(t));
    if !focus_kept {
        p.media_focus = None;
    }
    redraw(state);
}

/// Lay the shown panel out again on the same button, keeping its hot cell and keyboard; its pictures are held over
/// as `pick` holds them.
fn reshow(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return;
    };
    let (bar, button, hot, keys) = (p.bar, p.button, p.hot, p.keys && !p.media_only);
    hold(state);
    show(state, bar, button);
    if let Some(p) = state.task_preview.shown.as_mut() {
        p.hot = hot.filter(|&h| h < p.windows.len());
        p.keys = keys;
    }
    redraw(state);
    release(state);
}

/// Take the panel down, then close `w` or bring it up.
fn pick(state: &mut State, w: WindowId, close: bool) {
    hold(state);
    hide(state);
    if close {
        state.close(w);
    } else if state.task_windows().contains(&w) {
        state.bring_up(w);
    }
    release(state);
}

/// Enter on group button `i` of bar `bar`: its panel shows at once and takes the arrows, `start`'s
/// cell hot. False when no panel shows.
pub fn open_keys(state: &mut State, bar: TreeId, i: usize, start: Option<WindowId>) -> bool {
    let shown = state.task_preview.shown.as_ref().map(|p| (p.bar, p.button));
    if shown != Some((bar, i)) {
        let awake = std::mem::take(&mut state.task_preview.awake);
        show(state, bar, i);
        resuspend(state, awake);
    }
    let v = &mut state.task_preview;
    let Some(p) = v.shown.as_mut() else {
        return false;
    };
    v.keyed = true;
    p.keys = true;
    let at = (p.windows.iter()).position(|&w| Some(w) == start);
    hover_panel(state, Some(at.unwrap_or(0)));
    crate::access::touch(state);
    true
}

/// Whether the panel has the arrows (`open_keys`).
pub fn has_keys(state: &State) -> bool {
    (state.task_preview.shown.as_ref()).is_some_and(|p| p.keys)
}

/// Tab from the taskbar's focus on a task button whose shown panel holds one window with a player: the
/// keyboard steps onto the first media button it can press. False when there is none.
pub fn tab_in(state: &mut State) -> bool {
    let v = &state.task_preview;
    let Some(p) = (v.shown.as_ref()).filter(|p| v.keyed && !p.keys && p.windows.len() == 1) else {
        return false;
    };
    let Some(t) = (p.players.first().and_then(Option::as_ref))
        .and_then(|(_, np)| taskpreview::media_step(np, None, false))
    else {
        return false;
    };
    if let Some(p) = state.task_preview.shown.as_mut() {
        p.keys = true;
        p.media_only = true;
    }
    hover_panel(state, Some(0));
    set_media_focus(state, Some(t));
    crate::access::touch(state);
    true
}

/// The keyboard leaves the media buttons `tab_in` reached for their task button.
fn tab_out(state: &mut State) {
    if let Some(p) = state.task_preview.shown.as_mut() {
        p.keys = false;
        p.media_only = false;
        p.media_focus = None;
    }
    hover_panel(state, None);
    redraw(state);
    crate::access::touch(state);
}

/// Key `sym` for the panel with the arrows; false when it has none, and the taskbar takes the key.
pub fn key(state: &mut State, sym: u32) -> bool {
    let Some(p) = state.task_preview.shown.as_ref().filter(|p| p.keys) else {
        return false;
    };
    if p.media_only {
        return media_key(state, sym);
    }
    let at = p.hot.unwrap_or(0);
    let w = p.windows.get(at).copied();
    let player = p.players.get(at).cloned().flatten();
    let focus = p.media_focus;
    match taskpreview::key(&p.layout, at, sym) {
        None => {}
        Some(PanelKey::To(n)) => {
            set_media_focus(state, None);
            hover_panel(state, Some(n));
        }
        Some(PanelKey::Activate) => match (focus, player) {
            (Some(t), Some((bus, np))) => {
                if np.can(t) {
                    crate::previewmedia::send(state, &bus, t);
                }
            }
            _ => w.into_iter().for_each(|w| pick(state, w, false)),
        },
        Some(PanelKey::Close) => w.into_iter().for_each(|w| pick(state, w, true)),
        Some(PanelKey::Back) if focus.is_some() => set_media_focus(state, None),
        Some(PanelKey::Back) => hide(state),
        Some(PanelKey::Tab { back }) => {
            if let Some((_, np)) = player {
                set_media_focus(state, taskpreview::media_step(&np, focus, back));
            }
        }
    }
    true
}

/// Key `sym` on the media buttons `tab_in` reached: Enter and Space press the one focused, Tab and
/// Shift+Tab step, Escape or Shift+Tab before the first goes back to the button. Tab past the last
/// and any other key go back to the bar, which takes the key (false).
fn media_key(state: &mut State, sym: u32) -> bool {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return false;
    };
    let player = p.players.first().cloned().flatten();
    let (Some(focus), Some((bus, np))) = (p.media_focus, player) else {
        tab_out(state);
        return false;
    };
    match taskpreview::key(&p.layout, 0, sym) {
        Some(PanelKey::Activate) => {
            if np.can(focus) {
                crate::previewmedia::send(state, &bus, focus);
            }
            true
        }
        Some(PanelKey::Tab { back }) => match taskpreview::media_step(&np, Some(focus), back) {
            Some(t) => {
                set_media_focus(state, Some(t));
                true
            }
            None => {
                tab_out(state);
                back
            }
        },
        Some(PanelKey::Back) => {
            tab_out(state);
            true
        }
        _ => {
            tab_out(state);
            false
        }
    }
}

fn set_media_focus(state: &mut State, t: Option<Transport>) {
    let Some(p) = state.task_preview.shown.as_mut() else {
        return;
    };
    if p.media_focus != t {
        p.media_focus = t;
        redraw(state);
    }
}

/// Take the panel's pictures, and the windows a pending panel keeps awake, out of it before
/// something that takes it down and may bring one of its windows up; `release` drops them after,
/// so a minimized window is never suspended in between.
pub fn hold(state: &mut State) {
    release(state);
    let v = &mut state.task_preview;
    v.held_awake = std::mem::take(&mut v.awake);
    if let Some(p) = v.shown.as_mut() {
        v.held = p.thumbs.drain(..).flatten().collect();
    }
}

pub fn release(state: &mut State) {
    for id in std::mem::take(&mut state.task_preview.held) {
        state.thumb_drop(id);
    }
    let awake = std::mem::take(&mut state.task_preview.held_awake);
    resuspend(state, awake);
}

/// `windows` stopped waiting for the panel: each nothing else shows is suspended again.
fn resuspend(state: &mut State, windows: Vec<WindowId>) {
    for w in windows {
        if state.mark_suspended(w) {
            state.send_configure(w);
        }
    }
}

/// A pointer press on task button `i` of bar `bar`: its panel stays down until the pointer has
/// left the button, so a pointer resting where it clicked brings none back.
pub fn pressed(state: &mut State, bar: TreeId, i: usize) {
    let key = state
        .bar(bar)
        .and_then(|b| crate::taskbar::task_key(&b.slots, i));
    state.task_preview.pressed = key.map(|k| (bar, k));
}

/// The pointer is off the task buttons and the panel: as `hide`, and the pressed button shows its
/// panel again on the next rest. A panel the taskbar's keyboard focus armed stays.
pub fn leave(state: &mut State) {
    state.task_preview.pressed = None;
    if !state.task_preview.keyed {
        hide(state);
    } else if !has_keys(state) {
        hover_panel(state, None);
        stop_peek(state);
    }
}

/// Take the panel down and forget a button waiting for one. `leave` calls it on every pointer motion
/// off the task buttons, so with no panel, button or peek it touches nothing.
pub fn hide(state: &mut State) {
    let v = &state.task_preview;
    if v.pending.is_none() && v.shown.is_none() && !v.peeking {
        return;
    }
    stop_peek(state);
    state.task_preview.pending = None;
    disarm(state);
    let awake = std::mem::take(&mut state.task_preview.awake);
    resuspend(state, awake);
    let Some(p) = state.task_preview.shown.take() else {
        return;
    };
    for id in p.thumbs.into_iter().flatten() {
        state.thumb_drop(id);
    }
    state.destroy_redrawn(p.tree);
    crate::access::touch(state);
    crate::autohideview::poke(state);
}

/// `w`'s title, app id or icon changed: a panel showing it draws it again.
pub fn retitled(state: &mut State, w: WindowId) {
    if (state.task_preview.shown.as_ref()).is_some_and(|p| p.windows.contains(&w)) {
        redraw(state);
    }
}

/// `w` is going away or leaving the screen: a panel showing it goes too. Called from `forget_window` and `desktops_changed`.
pub fn forget_window(state: &mut State, w: WindowId) {
    if state
        .task_preview
        .shown
        .as_ref()
        .is_some_and(|p| p.windows.contains(&w))
    {
        hide(state);
    }
}

/// The output the panel is on, or the one the bar of the button waiting for one is on.
pub fn open_on(state: &State) -> Option<Output> {
    let v = &state.task_preview;
    match (v.shown_on(), v.pending) {
        (Some(o), _) => Some(o.clone()),
        (None, Some((bar, _))) => state.bar(bar).map(|b| b.output.clone()),
        (None, None) => None,
    }
}

/// The glass under a panel `w` x `h`: the whole panel, its corners rounded as the grid's `corner` cuts them.
fn panel_pane(w: i32, h: i32, corner: &'static [i32]) -> Pane {
    Pane {
        shapes: vec![PaneShape {
            rect: Rect::new(0, 0, w, h),
            corner,
            square_bottom: false,
            square_top: false,
        }],
        holes: Vec::new(),
    }
}

/// The part of output box `b` on the screen side of bar `p`: where the preview panel may go.
fn preview_bounds(b: Rect, p: &BarPlacement) -> Rect {
    let edge = p.screen_side();
    match p.edge {
        Edge::Bottom => Rect::new(b.x, b.y, b.width, edge - b.y),
        Edge::Top => Rect::new(b.x, edge, b.width, b.bottom() - edge),
        Edge::Left => Rect::new(edge, b.y, b.right() - edge, b.height),
        Edge::Right => Rect::new(b.x, b.y, edge - b.x, b.height),
    }
}

fn show(state: &mut State, bar: TreeId, i: usize) {
    hide(state);
    if state.desktop_locked() {
        return;
    }
    let live = state.task_windows();
    let Some((output, mut windows, btn)) = state.bar(bar).and_then(|b| {
        Some((
            b.output.clone(),
            b.slots.get(i)?.windows.clone(),
            *b.layout.tasks.get(i)?,
        ))
    }) else {
        return;
    };
    windows.retain(|w| live.contains(w));
    if windows.is_empty() {
        return;
    }
    let p = state.bar_placement(&output);
    let button = Rect::new(p.shown.x + btn.x, p.shown.y + btn.y, btn.width, btn.height);
    let bounds = preview_bounds(state.layout_box(Some(&output)), &p);
    let players = crate::previewmedia::players_of(state, &windows);
    let grid = interfaces::task_preview_grid(&state.config.theme);
    let grid = if players.iter().any(Option::is_some) {
        grid.with_media()
    } else {
        grid
    };
    let layout = TaskPreviewLayout::compute(grid, windows.len(), button, bounds, p.edge);
    let scale = state.draw_scale_at(Some(&output));
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.menu);
    scene.set_data(tree, Some(SceneOwner::TaskPreview));
    crate::glass::leave_out(scene, tree);
    crate::output::keep_on(scene, tree, &output);
    scene.set_position(tree, layout.rect.x, layout.rect.y);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    let area = layout.canvas();
    scene.set_position(buffer, area.x, area.y);
    let input = layout.input_area();
    scene.set_input(
        buffer,
        Input::Rect(Rectangle::new(
            (input.x, input.y).into(),
            (input.width, input.height).into(),
        )),
    );
    // The panel never reshapes while open, so its node keeps the glass alive without the handle.
    let pane = glass::wanted(&state.config)
        .then(|| panel_pane(layout.rect.width, layout.rect.height, grid.corner));
    Backdrop::new(&state.glass).set(&mut state.scene, &state.glass, glass_tree, pane);
    let thumbs = windows
        .iter()
        .enumerate()
        .map(|(k, &w)| {
            let well = layout.well(k)?;
            state.thumb_create(w, tree, well, scale)
        })
        .collect();
    state.task_preview.shown = Some(Panel {
        bar,
        button: i,
        output,
        windows,
        layout,
        tree,
        buffer,
        thumbs,
        hot: None,
        keys: false,
        media_only: false,
        players,
        ptr: Pointer::new(),
        media_focus: None,
    });
    redraw(state);
}

/// Icon files landed: the open preview draws its windows' icons again.
pub(crate) fn icons_landed(state: &mut State) {
    redraw(state);
}

fn redraw(state: &mut State) {
    let Some(p) = state.task_preview.shown.as_ref() else {
        return;
    };
    let scale = state.draw_scale_at(Some(&p.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let windows = p.windows.clone();
    let has_thumb: Vec<bool> = p.thumbs.iter().map(Option::is_some).collect();
    let (layout, hot, buffer, tree) = (p.layout.scaled(scale), p.hot, p.buffer, p.tree);
    let now_playing = p.now_playing();
    let lit = p.ptr.lit();
    let focus = (p.hot.zip(p.media_focus)).filter(|_| p.keys);
    let was = state.scene.outputs_of(tree);
    let mut titles = Vec::new();
    let mut app_ids = Vec::new();
    let mut active = Vec::new();
    let live = state.active_task_window();
    for &w in &windows {
        let Some(win) = state.win(w) else {
            return;
        };
        titles.push(win.title());
        app_ids.push(win.app_id());
        active.push(live == Some(w));
    }
    let mut icon = |app_id: &str, px: i32| {
        crate::taskbarview::app_icon(&mut state.icons, &state.app_index, app_id, px)
            .map(Arc::unwrap_or_clone)
    };
    let icons: Vec<Option<Canvas>> = app_ids
        .iter()
        .map(|a| icon(a, SMALL_ICON * scale))
        .collect();
    let pictures: Vec<Option<Canvas>> = app_ids
        .iter()
        .zip(&has_thumb)
        .map(|(a, &thumb)| {
            if thumb {
                None
            } else {
                icon(a, WELL_ICON * scale)
            }
        })
        .collect();
    let cells: Vec<Cell> = (0..windows.len())
        .map(|k| Cell {
            title: &titles[k],
            icon: icons[k].as_ref(),
            active: active[k],
            picture: pictures[k].as_ref(),
        })
        .collect();
    let area = layout.canvas();
    let mut canvas = Canvas::new(area.width.max(1) as u32, area.height.max(1) as u32);
    let media = Media {
        players: &now_playing,
        lit,
        focus,
    };
    interfaces::draw_task_preview(
        &mut canvas,
        &theme,
        state.text.for_output(theme.scale),
        &layout,
        &cells,
        hot,
        &media,
    );
    upload_scaled(&mut state.scene, buffer, &canvas, scale);
    state.redraw_nodes(was, &[tree.into()]);
    crate::access::touch(state);
}

impl TaskPreviewView {
    /// The Quick Launch pins were put in `order` (old indices in new order): a pressed pinned
    /// button keeps its mark under its new pin index.
    pub fn pins_reordered(&mut self, order: &[usize]) {
        self.pressed =
            (self.pressed).and_then(|(b, k)| Some((b, crate::taskbar::pin_renamed(k, order)?)));
    }

    pub fn is_shown(&self) -> bool {
        self.shown.is_some()
    }

    /// Whether `w` stays awake for a panel about to show it.
    pub fn keeps_awake(&self, w: WindowId) -> bool {
        self.awake.contains(&w) || self.held_awake.contains(&w)
    }

    pub fn shown_on(&self) -> Option<&Output> {
        self.shown.as_ref().map(|p| &p.output)
    }
}

/// The panel's tree: a button per window named by its title, the hovered one with its close button, and the media
/// buttons of a window with a player.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let p = state.task_preview.shown.as_ref()?;
    let mut tree = crate::access::root(Role::Window, "Window previews", p.layout.rect);
    let titles: Vec<String> = (p.windows.iter())
        .map(|&w| state.win(w).map(|x| x.title()).unwrap_or_default())
        .collect();
    let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
    let now_playing = p.now_playing();
    let media = Media {
        players: &now_playing,
        lit: None,
        focus: (p.hot.zip(p.media_focus)).filter(|_| p.keys),
    };
    taskpreview::access(&p.layout, &titles, p.hot, p.keys, &media, &mut tree, ROOT);
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisebus::mpris::Players;

    #[test]
    fn picking_a_minimized_window_in_the_preview_panel_never_suspends_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "one", 300, 200);
        h.map_toplevel(&mut c, "two", 300, 200);
        h.state.set_minimized(a.id, true);
        h.roundtrip(&mut c);
        assert_eq!(c.side.shells[a.shell].suspended.last(), Some(&true));
        let bar = &h.state.bars[0];
        let (tree, i) = (bar.tree, bar.slots.iter().position(|s| s.windows == [a.id]));
        show(&mut h.state, tree, i.expect("its task button"));
        h.roundtrip(&mut c);
        let heard = &c.side.shells[a.shell].suspended;
        assert_eq!(heard.last(), Some(&false), "its picture wakes it");
        let heard = heard.len();
        clicked(&mut h.state, Some(0));
        h.roundtrip(&mut c);
        assert!(!h.state.win(a.id).unwrap().minimized, "it came up");
        assert!(
            !c.side.shells[a.shell].suspended[heard..].contains(&true),
            "never suspended between the pictures and the restore"
        );
    }

    #[test]
    fn pressing_a_minimized_windows_task_button_under_its_panel_never_suspends_it() {
        for pinned in [false, true] {
            let mut h = crate::harness::Harness::new();
            h.state.config.taskbar.pinned = pinned;
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            h.map_toplevel(&mut c, "two", 300, 200);
            h.state.set_minimized(a.id, true);
            h.roundtrip(&mut c);
            let bar = &h.state.bars[0];
            let i = (bar.slots.iter().position(|s| s.windows == [a.id])).expect("its task button");
            let (tree, r) = (bar.tree, bar.layout.tasks[i]);
            let origin = h.state.bar_drawn(&bar.output).shown;
            show(&mut h.state, tree, i);
            h.roundtrip(&mut c);
            let heard = &c.side.shells[a.shell].suspended;
            assert_eq!(heard.last(), Some(&false), "its picture wakes it");
            let heard = heard.len();
            h.pointer_to(
                f64::from(origin.x + r.x + r.width / 2),
                f64::from(origin.y + r.y + r.height / 2),
            );
            h.button(crate::input::BTN_LEFT, true);
            h.roundtrip(&mut c);
            assert!(
                !h.state.task_preview.is_shown(),
                "the press takes the panel down"
            );
            h.button(crate::input::BTN_LEFT, false);
            h.roundtrip(&mut c);
            assert!(
                !h.state.win(a.id).unwrap().minimized,
                "it came up, pinned {pinned}"
            );
            assert!(
                !c.side.shells[a.shell].suspended[heard..].contains(&true),
                "never suspended between the pictures and the restore, pinned {pinned}"
            );
            h.state.set_minimized(a.id, true);
            h.roundtrip(&mut c);
            let heard = c.side.shells[a.shell].suspended.last();
            assert_eq!(
                heard,
                Some(&true),
                "no picture outlives the press, pinned {pinned}"
            );
        }
    }

    #[test]
    fn after_a_click_on_a_task_button_its_panel_waits_for_the_pointer_to_come_back() {
        for pinned in [false, true] {
            let mut h = crate::harness::Harness::new();
            h.state.config.taskbar.pinned = pinned;
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let bar = &h.state.bars[0];
            let origin = h.state.bar_drawn(&bar.output).shown;
            let centre = |w| {
                let i = (bar.slots.iter().position(|s| s.windows == [w])).expect("a task button");
                let r = bar.layout.tasks[i];
                let x = f64::from(origin.x + r.x + r.width / 2);
                (i, x, f64::from(origin.y + r.y + r.height / 2))
            };
            let (ia, ax, ay) = centre(a.id);
            let (ib, bx, by) = centre(b.id);
            let on = |s: &State, i| (s.task_preview.shown.as_ref()).is_some_and(|p| p.button == i);
            h.pointer_to(ax, ay);
            h.until("the panel on a rest", |s| on(s, ia));
            // Brings it up, then minimizes it.
            for _ in 0..2 {
                h.button(crate::input::BTN_LEFT, true);
                h.button(crate::input::BTN_LEFT, false);
                h.run_for(2 * SHOW_MS);
                assert!(
                    !h.state.task_preview.is_shown(),
                    "no panel while the pointer rests where it clicked, pinned {pinned}"
                );
            }
            h.pointer_to(512.0, 300.0);
            h.pointer_to(ax, ay);
            h.until("the panel once the pointer is back", |s| on(s, ia));
            h.button(crate::input::BTN_LEFT, true);
            h.button(crate::input::BTN_LEFT, false);
            h.pointer_to(bx, by);
            h.until("the panel on the next button", |s| on(s, ib));
        }
    }

    /// The index of `w`'s task button on the first bar and the screen point at its centre.
    fn centre(h: &crate::harness::Harness, w: WindowId) -> (usize, f64, f64) {
        let bar = &h.state.bars[0];
        let origin = h.state.bar_drawn(&bar.output).shown;
        let i = (bar.slots.iter().position(|s| s.windows == [w])).expect("a task button");
        let r = bar.layout.tasks[i];
        let x = f64::from(origin.x + r.x + r.width / 2);
        (i, x, f64::from(origin.y + r.y + r.height / 2))
    }

    fn on(s: &State, i: usize) -> bool {
        (s.task_preview.shown.as_ref()).is_some_and(|p| p.button == i)
    }

    #[test]
    fn a_pressed_task_buttons_panel_stays_down_wherever_a_closing_window_moves_it() {
        for next in [true, false] {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let d = h.map_toplevel(&mut c, "three", 300, 200);
            let (_, bx, by) = centre(&h, b.id);
            h.pointer_to(bx, by);
            h.button(crate::input::BTN_LEFT, true);
            h.button(crate::input::BTN_LEFT, false);
            c.destroy(a);
            h.roundtrip(&mut c);
            let (ib, bx, by) = centre(&h, b.id);
            let (id, dx, dy) = centre(&h, d.id);
            assert_eq!((ib, id), (0, 1), "the row closed up");
            if next {
                h.pointer_to(dx, dy);
                h.until("the panel of the button that took its slot", |s| on(s, id));
                continue;
            }
            h.pointer_to(bx, by);
            h.run_for(2 * SHOW_MS);
            assert!(
                !h.state.task_preview.is_shown(),
                "no panel for the pressed button in its new slot"
            );
            h.pointer_to(512.0, 300.0);
            h.pointer_to(bx, by);
            h.until("its panel once the pointer is back", |s| on(s, ib));
        }
    }

    /// A Quick Launch pin whose app id is `id`, so a window of that app sits on its button.
    fn pin(id: &str) -> crate::quicklaunch::QuickItem {
        let text = format!("[Desktop Entry]\nType=Application\nName={id}\nExec=x\n");
        let file = format!("{id}.desktop");
        crate::quicklaunch::QuickItem {
            entry: crate::quicklaunch::parse(&text, &file.parse().unwrap(), "foot").unwrap(),
            app: reprisefs::apps::parse_desktop(&text, &file.parse().unwrap()),
            file,
        }
    }

    #[test]
    fn a_task_button_dragged_to_another_slot_keeps_its_panel_down_until_the_pointer_comes_back() {
        for pins in [false, true] {
            let mut h = crate::harness::Harness::new();
            // A task button drags only while the taskbar is pinned.
            h.state.config.taskbar.pinned = true;
            if pins {
                h.state.quicklaunch = vec![pin("one"), pin("two")];
            }
            h.output("TEST-1", 1024, 768);
            let mut c = h.connect();
            let a = h.map_toplevel(&mut c, "one", 300, 200);
            let b = h.map_toplevel(&mut c, "two", 300, 200);
            let (ia, ax, ay) = centre(&h, a.id);
            let (ib, bx, _) = centre(&h, b.id);
            let pinned = h.state.bars[0].slots[ia].pin.is_some();
            assert_eq!(pinned, pins, "a window of a pinned app sits on its pin");
            h.pointer_to(ax, ay);
            h.button(crate::input::BTN_LEFT, true);
            for n in 1..=8 {
                h.pointer_to(ax + (bx + 10.0 - ax) * f64::from(n) / 8.0, ay);
            }
            h.button(crate::input::BTN_LEFT, false);
            h.run_for(2 * SHOW_MS);
            let (na, ax, ay) = centre(&h, a.id);
            let (nb, bx, by) = centre(&h, b.id);
            assert_eq!((na, nb), (ib, ia), "the buttons swapped, pins {pins}");
            assert!(
                !h.state.task_preview.is_shown(),
                "no panel while the pointer rests where it dropped it, pins {pins}"
            );
            h.pointer_to(bx, by);
            h.until("the panel of the button in its old slot", |s| on(s, nb));
            h.pointer_to(512.0, 300.0);
            h.pointer_to(ax, ay);
            h.until("its panel once the pointer is back", |s| on(s, na));
        }
    }

    /// A minimized window shown live by the overview or the 3D switcher, which then closes with
    /// the pointer resting on the window's task button; with the count of configures the window
    /// heard before the close.
    fn closed_over_its_button(
        view: &str,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        crate::harness::Toplevel,
        usize,
    ) {
        let config = crate::config::Config::parse(
            "[appearance]\ninterface = \"glass\"\n[effects]\nreduce_motion = true\n",
        )
        .unwrap();
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "one", 300, 200);
        h.map_toplevel(&mut c, "two", 300, 200);
        h.state.set_minimized(a.id, true);
        h.roundtrip(&mut c);
        let (_, x, y) = centre(&h, a.id);
        if view == "overview" {
            h.state.open_overview();
        } else {
            h.state.open_flip(crate::keys::MOD_LOGO);
            h.until("the stack", |s| {
                (s.switcher.stack.as_ref()).is_some_and(|st| st.revealed())
            });
        }
        h.roundtrip(&mut c);
        let heard = &c.side.shells[a.shell].suspended;
        assert_eq!(heard.last(), Some(&false), "the {view} shows it live");
        let heard = heard.len();
        h.pointer_to(x, y);
        if view == "overview" {
            h.state.close_overview();
        } else {
            h.state.switcher_escape();
            let output = h.state.bars[0].output.clone();
            h.state.flip_frame(&output);
            assert!(h.state.switcher.leaving.is_none(), "the cards landed");
        }
        h.roundtrip(&mut c);
        assert!(
            !h.state.live_previews.shows(a.id),
            "the {view} took its picture"
        );
        (h, c, a, heard)
    }

    #[test]
    fn a_minimized_window_handed_from_a_closing_overview_or_stack_to_its_panel_hears_nothing() {
        for view in ["overview", "stack"] {
            let (mut h, mut c, a, heard) = closed_over_its_button(view);
            h.until("its panel", |s| s.task_preview.is_shown());
            h.roundtrip(&mut c);
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [false; 0],
                "the {view}'s picture hands over to the panel's with no configure"
            );
            h.pointer_to(512.0, 300.0);
            h.roundtrip(&mut c);
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [true],
                "suspended once the panel goes, after the {view}"
            );
        }
    }

    #[test]
    fn a_minimized_window_waiting_for_its_panel_stays_awake_into_an_overview_opened_again() {
        let (mut h, mut c, a, heard) = closed_over_its_button("overview");
        h.state.open_overview();
        h.roundtrip(&mut c);
        assert!(h.state.live_previews.shows(a.id), "its cell shows it live");
        assert_eq!(
            c.side.shells[a.shell].suspended[heard..],
            [false; 0],
            "the wait hands over to the cell's picture with no configure"
        );
    }

    #[test]
    fn a_minimized_window_waiting_for_its_panel_is_suspended_when_the_pointer_leaves_first() {
        for view in ["overview", "stack"] {
            let (mut h, mut c, a, heard) = closed_over_its_button(view);
            h.pointer_to(512.0, 300.0);
            h.run_for(2 * SHOW_MS);
            h.roundtrip(&mut c);
            assert!(!h.state.task_preview.is_shown(), "no panel");
            assert_eq!(
                c.side.shells[a.shell].suspended[heard..],
                [true],
                "one Suspended once nothing will show it, after the {view}"
            );
        }
    }

    #[test]
    fn an_open_panel_draws_a_windows_new_title() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_titled(&mut c, "editor", "Alpha", (300, 200));
        h.dispatch();
        let (i, _, _) = centre(&h, a.id);
        let tree = h.state.bars[0].tree;
        let drawn = |h: &crate::harness::Harness| {
            let p = h.state.task_preview.shown.as_ref().expect("the panel");
            h.state.scene.buffer_canvas(p.buffer).expect("its canvas")
        };
        show(&mut h.state, tree, i);
        let before = drawn(&h);
        h.until("the titles drawn", |s| s.title_draws_at_once(a.id));
        c.side.shells[a.shell]
            .toplevel
            .set_title("Evening Mist".into());
        c.side.shells[a.shell].surface.commit();
        h.roundtrip(&mut c);
        h.dispatch();
        let retitled = drawn(&h);
        assert!(retitled != before, "the panel redrew");
        show(&mut h.state, tree, i);
        assert!(retitled == drawn(&h), "as a panel opened on the new title");
    }

    #[test]
    fn the_preview_bounds_are_the_screen_side_of_the_bar() {
        use reprisekit::taskbar::Edge;
        let b = Rect::new(1920, 0, 1280, 720);
        let bottom = crate::barplace::place(b, Edge::Bottom, 30);
        assert_eq!(preview_bounds(b, &bottom), Rect::new(1920, 0, 1280, 690));
        let top = crate::barplace::place(b, Edge::Top, 30);
        assert_eq!(preview_bounds(b, &top), Rect::new(1920, 30, 1280, 690));
        let left = crate::barplace::place(b, Edge::Left, 60);
        assert_eq!(preview_bounds(b, &left), Rect::new(1980, 0, 1220, 720));
        let right = crate::barplace::place(b, Edge::Right, 60);
        assert_eq!(preview_bounds(b, &right), Rect::new(1920, 0, 1220, 720));
    }

    #[test]
    fn the_panel_glass_covers_the_panel_with_the_grid_corners() {
        let p = panel_pane(200, 150, &[3, 1, 1]);
        assert_eq!(p.shapes.len(), 1);
        assert_eq!(p.shapes[0].rect, Rect::new(0, 0, 200, 150));
        assert_eq!(p.shapes[0].corner, &[3, 1, 1], "the grid's cut");
        assert!(p.holes.is_empty(), "the pictures sit over glass");
        assert!(panel_pane(10, 10, &[]).shapes[0].corner.is_empty());
    }

    #[test]
    fn the_popups_glass_rounds_its_corners_above_scale_1_as_its_chrome_does() {
        // Above scale 1 the popup's chrome draws its corners round over 7 px; the glass under it is one shape rounded over as many.
        use reprisekit::glass::{Material, preview};
        let p = panel_pane(235, 166, preview::GRID.corner);
        let whole = vec![(Rect::new(0, 0, 235, 166), [7.0; 4])];
        for scale in [1.25, 2.0] {
            assert_eq!(
                crate::glass::drawn_shapes(&p, Material::Frosted, scale),
                whole,
                "at {scale}"
            );
        }
        assert!(crate::glass::drawn_shapes(&p, Material::Frosted, 1.0).len() > 1);
    }

    const KEY_ESC: u32 = 1;
    const KEY_ENTER: u32 = 28;
    const KEY_LEFT: u32 = 105;
    const KEY_RIGHT: u32 = 106;
    const KEY_DELETE: u32 = 111;

    fn tap(h: &mut crate::harness::Harness, code: u32) {
        h.key(code, true);
        h.key(code, false);
        h.dispatch();
    }

    /// Two "test.app" windows on one button and a "test.other" window on the next, under
    /// `[taskbar] pinned` and combined buttons, the taskbar's keyboard on the group.
    fn group_with_keys(
        previews: bool,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        [crate::harness::Toplevel; 3],
        usize,
    ) {
        let mut config = crate::config::Config::default();
        config.taskbar.pinned = true;
        config.taskbar.buttons = crate::config::TaskbarButtons::Combine;
        config.previews.taskbar = previews;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_titled(&mut c, "test.app", "Alpha", (300, 200));
        let b = h.map_titled(&mut c, "test.app", "Beta", (300, 200));
        let o = h.map_titled(&mut c, "test.other", "Other", (300, 200));
        h.dispatch();
        let bar = &h.state.bars[0];
        let tree = bar.tree;
        let i = (bar.slots.iter())
            .position(|s| s.windows.len() == 2)
            .expect("one button holds both windows");
        h.pointer_to(512.0, 300.0);
        h.state.focus_bar_item(tree, TaskbarHit::Task(i));
        (h, c, [a, b, o], i)
    }

    fn hot_window(s: &State) -> Option<WindowId> {
        let p = s.task_preview.shown.as_ref()?;
        p.windows.get(p.hot?).copied()
    }

    #[test]
    fn the_taskbars_focus_on_a_task_button_shows_its_panel_after_the_hover_delay() {
        let (mut h, _c, _, i) = group_with_keys(true);
        assert!(!h.state.task_preview.is_shown(), "not before the delay");
        h.until("the focused button's panel", |s| on(s, i));
        h.pointer_to(600.0, 200.0);
        assert!(on(&h.state, i), "the pointer moving elsewhere leaves it up");
        tap(&mut h, KEY_RIGHT);
        assert!(on(&h.state, i + 1), "the next button's panel at once");
        tap(&mut h, KEY_LEFT);
        tap(&mut h, KEY_LEFT);
        assert!(
            !h.state.task_preview.is_shown(),
            "off the task buttons it goes"
        );
        tap(&mut h, KEY_RIGHT);
        h.until("the panel once back on the button", |s| on(s, i));
        tap(&mut h, KEY_ESC);
        assert!(h.state.bar_shared.keys.is_none());
        assert!(
            !h.state.task_preview.is_shown(),
            "the taskbar giving the keyboard back takes it down"
        );
    }

    #[test]
    fn enter_on_a_group_opens_its_panel_and_the_arrows_pick_the_window_enter_brings_up() {
        let (mut h, _c, [a, b, _], i) = group_with_keys(true);
        tap(&mut h, KEY_ENTER);
        assert!(on(&h.state, i), "the panel opens at once");
        assert!(has_keys(&h.state));
        assert_eq!(h.state.focused, None, "no window is brought up yet");
        let first = hot_window(&h.state).expect("a hot cell");
        assert_eq!(first, b.id, "the window a click would bring up");
        tap(&mut h, KEY_RIGHT);
        assert_eq!(hot_window(&h.state), Some(a.id));
        h.pointer_to(600.0, 200.0);
        assert_eq!(
            hot_window(&h.state),
            Some(a.id),
            "the pointer takes nothing"
        );
        tap(&mut h, KEY_ENTER);
        assert_eq!(h.state.focused, Some(a.id));
        assert!(!h.state.task_preview.is_shown());
        assert!(
            h.state.bar_shared.keys.is_none(),
            "the window has the keyboard"
        );
    }

    #[test]
    fn delete_closes_the_hot_window_and_escape_goes_back_to_the_button() {
        let (mut h, mut c, [_, b, _], i) = group_with_keys(true);
        tap(&mut h, KEY_ENTER);
        assert_eq!(hot_window(&h.state), Some(b.id));
        tap(&mut h, KEY_DELETE);
        h.roundtrip(&mut c);
        assert_eq!(c.side.closes, [b.shell], "asked to close");
        assert!(!h.state.task_preview.is_shown());
        let at = h.state.bar_shared.keys.map(|k| k.at);
        assert_eq!(
            at,
            Some(TaskbarHit::Task(i)),
            "the button keeps the keyboard"
        );
        tap(&mut h, KEY_ENTER);
        assert!(has_keys(&h.state));
        tap(&mut h, KEY_ESC);
        assert!(!h.state.task_preview.is_shown());
        let at = h.state.bar_shared.keys.map(|k| k.at);
        assert_eq!(
            at,
            Some(TaskbarHit::Task(i)),
            "Escape goes back to the button"
        );
        assert_eq!(h.state.focused, None);
    }

    #[test]
    fn the_button_keeps_the_keyboard_once_the_window_delete_closed_goes() {
        let (mut h, mut c, [_, b, o], i) = group_with_keys(true);
        tap(&mut h, KEY_ENTER);
        tap(&mut h, KEY_DELETE);
        h.roundtrip(&mut c);
        c.destroy(b);
        h.roundtrip(&mut c);
        assert!(h.state.win(b.id).is_none(), "the window is gone");
        assert_eq!(h.state.focused, None);
        let at = h.state.bar_shared.keys.map(|k| k.at);
        assert_eq!(at, Some(TaskbarHit::Task(i)), "the button keeps it");
        tap(&mut h, KEY_RIGHT);
        let at = h.state.bar_shared.keys.map(|k| k.at);
        assert_eq!(at, Some(TaskbarHit::Task(i + 1)), "Right moves along");
        h.roundtrip(&mut c);
        assert_eq!(c.side.keyboard_focus, None);
        assert!(c.side.keys.is_empty(), "no window hears the arrows");
        tap(&mut h, KEY_ESC);
        assert_eq!(h.state.focused, Some(o.id), "Escape gives it back");
    }

    #[test]
    fn the_window_the_bar_took_the_keyboard_from_closing_leaves_it_on_the_bar() {
        let (mut h, mut c, [_, b, o], i) = group_with_keys(true);
        c.destroy(o);
        h.roundtrip(&mut c);
        let at = h.state.bar_shared.keys.map(|k| k.at);
        assert_eq!(at, Some(TaskbarHit::Task(i)));
        tap(&mut h, KEY_ESC);
        assert_eq!(h.state.focused, Some(b.id), "the top window takes it");
    }

    #[test]
    fn a_reader_follows_the_hot_cell_of_the_panel_with_the_arrows() {
        let (mut h, _c, [a, _, _], _) = group_with_keys(true);
        tap(&mut h, KEY_ENTER);
        tap(&mut h, KEY_RIGHT);
        assert_eq!(hot_window(&h.state), Some(a.id));
        let tree = access(&h.state).expect("the panel's tree");
        let alpha = tree.find(reprisekit::access::accesskit::Role::Button, "Alpha");
        assert_eq!(alpha, Some(tree.focus()));
        let p = h.state.task_preview.shown.as_ref().unwrap();
        let (c, r) = (p.layout.cells[p.hot.unwrap()], p.layout.rect);
        let cell = Rect::new(r.x + c.x, r.y + c.y, c.width, c.height);
        assert_eq!(crate::access::focus_rect(&h.state), Some(cell));
    }

    const PLAYER: &str = "org.mpris.MediaPlayer2.mpv";

    /// One player, `status`, with Previous live as `previous` says.
    fn player(about: reprisebus::mpris::About, status: &str, previous: bool) -> Players {
        let mut p = Players::default();
        p.add(
            PLAYER,
            &crate::previewmedia::tests::status(status, previous),
            about,
        );
        p
    }

    fn by_pid(pid: i32) -> reprisebus::mpris::About {
        reprisebus::mpris::About {
            pid: Some(pid as u32),
            ..Default::default()
        }
    }

    /// The screen point at the middle of the shown panel's cell `i` media button `t`.
    fn media_point(h: &crate::harness::Harness, i: usize, t: Transport) -> (f64, f64) {
        let p = h.state.task_preview.shown.as_ref().expect("the panel");
        let b = taskpreview::media_button(&p.layout, i, t).expect("a media row");
        let r = p.layout.rect;
        let x = f64::from(r.x + b.x + b.width / 2);
        (x, f64::from(r.y + b.y + b.height / 2))
    }

    fn sent(h: &crate::harness::Harness) -> Vec<(String, &'static str)> {
        h.state.preview_players.sent.clone()
    }

    #[test]
    fn a_window_whose_process_plays_gets_media_buttons_a_click_presses_and_the_panel_stays() {
        const PID: i32 = 5_000_101;
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect_as(PID);
        let a = h.map_toplevel(&mut c, "player", 300, 200);
        let mut o = h.connect();
        h.map_toplevel(&mut o, "other", 300, 200);
        h.state.preview_players.players = player(by_pid(PID), "Playing", false);
        let (i, _, _) = centre(&h, a.id);
        let tree = h.state.bars[0].tree;
        show(&mut h.state, tree, i);
        let p = h.state.task_preview.shown.as_ref().unwrap();
        assert!(p.layout.media(0).is_some(), "a media row");
        let np = p.players[0]
            .as_ref()
            .map(|(bus, np)| (bus.as_str(), np.playing));
        assert_eq!(np, Some((PLAYER, true)));
        let (x, y) = media_point(&h, 0, Transport::PlayPause);
        h.pointer_to(x, y);
        h.button(crate::input::BTN_LEFT, true);
        assert!(sent(&h).is_empty(), "nothing before the release");
        h.button(crate::input::BTN_LEFT, false);
        assert_eq!(sent(&h), [(PLAYER.to_string(), "PlayPause")]);
        assert!(h.state.task_preview.is_shown(), "the panel stays");
        let (x, y) = media_point(&h, 0, Transport::Previous);
        h.pointer_to(x, y);
        h.button(crate::input::BTN_LEFT, true);
        h.button(crate::input::BTN_LEFT, false);
        assert_eq!(sent(&h).len(), 1, "a greyed Previous presses nothing");
        assert!(h.state.task_preview.is_shown());
    }

    #[test]
    fn a_press_let_go_off_its_button_presses_nothing() {
        const PID: i32 = 5_000_102;
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect_as(PID);
        let a = h.map_toplevel(&mut c, "player", 300, 200);
        h.state.preview_players.players = player(by_pid(PID), "Paused", true);
        let (i, _, _) = centre(&h, a.id);
        let tree = h.state.bars[0].tree;
        show(&mut h.state, tree, i);
        let (x, y) = media_point(&h, 0, Transport::Next);
        h.pointer_to(x, y);
        h.button(crate::input::BTN_LEFT, true);
        h.pointer_to(x + 40.0, y);
        h.button(crate::input::BTN_LEFT, false);
        assert!(sent(&h).is_empty());
        assert!(h.state.task_preview.is_shown());
    }

    #[test]
    fn the_buttons_follow_the_player_and_the_panel_grows_and_shrinks_its_media_row() {
        const PID: i32 = 5_000_103;
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect_as(PID);
        let a = h.map_toplevel(&mut c, "player", 300, 200);
        let (i, _, _) = centre(&h, a.id);
        let tree = h.state.bars[0].tree;
        show(&mut h.state, tree, i);
        let height = |h: &crate::harness::Harness| {
            let p = h.state.task_preview.shown.as_ref().expect("the panel");
            (
                p.layout.rect.height,
                p.players[0].as_ref().map(|(_, np)| np.playing),
            )
        };
        let (plain, none) = height(&h);
        assert_eq!(none, None);
        h.state.preview_players.players = player(by_pid(PID), "Playing", true);
        players_changed(&mut h.state);
        assert_eq!(height(&h), (plain + 28, Some(true)), "a player came");
        h.state.preview_players.players = player(by_pid(PID), "Paused", true);
        players_changed(&mut h.state);
        assert_eq!(height(&h), (plain + 28, Some(false)), "it paused");
        h.state.preview_players.players = Players::default();
        players_changed(&mut h.state);
        assert_eq!(height(&h), (plain, None), "it went");
    }

    const KEY_TAB: u32 = 15;
    const KEY_SPACE: u32 = 57;

    #[test]
    fn tab_from_a_one_window_button_steps_through_its_media_buttons_and_on_along_the_bar() {
        let (mut h, _c, _, i) = group_with_keys(true);
        h.state.preview_players.players = player(
            reprisebus::mpris::About {
                pid: Some(5_000_105),
                desktop_entry: "test.other".into(),
                ..Default::default()
            },
            "Playing",
            false,
        );
        tap(&mut h, KEY_RIGHT);
        h.until("the one window's panel", |s| on(s, i + 1));
        let focus = |h: &crate::harness::Harness| {
            (h.state.task_preview.shown.as_ref()).and_then(|p| p.media_focus)
        };
        let at = |h: &crate::harness::Harness| h.state.bar_shared.keys.map(|k| k.at);
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), Some(Transport::PlayPause), "Previous is greyed");
        assert!(has_keys(&h.state), "the panel draws the focus");
        tap(&mut h, KEY_SPACE);
        assert_eq!(sent(&h), [(PLAYER.to_string(), "PlayPause")]);
        tap(&mut h, KEY_ESC);
        assert_eq!(focus(&h), None);
        assert!(on(&h.state, i + 1), "Escape went back to the button");
        assert_eq!(at(&h), Some(TaskbarHit::Task(i + 1)));
        tap(&mut h, KEY_TAB);
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), Some(Transport::Next));
        let tree = access(&h.state).expect("the panel's tree");
        let focused = tree
            .get(tree.focus())
            .and_then(|n| n.label().map(str::to_string));
        assert_eq!(focused.as_deref(), Some("Next"), "a reader follows");
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), None);
        assert_ne!(
            at(&h),
            Some(TaskbarHit::Task(i + 1)),
            "past the last, the bar's focus moves on"
        );
        assert_eq!(h.state.focused, None, "no window came up");
    }

    #[test]
    fn tab_takes_the_keyboard_to_the_hot_windows_media_buttons_and_escape_back() {
        let (mut h, _c, _, _) = group_with_keys(true);
        h.state.preview_players.players = player(
            reprisebus::mpris::About {
                pid: Some(5_000_104),
                desktop_entry: "test.app".into(),
                ..Default::default()
            },
            "Playing",
            false,
        );
        tap(&mut h, KEY_ENTER);
        let focus = |h: &crate::harness::Harness| {
            (h.state.task_preview.shown.as_ref()).and_then(|p| p.media_focus)
        };
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), Some(Transport::PlayPause), "Previous is greyed");
        tap(&mut h, KEY_SPACE);
        assert_eq!(sent(&h), [(PLAYER.to_string(), "PlayPause")]);
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), Some(Transport::Next));
        tap(&mut h, KEY_ENTER);
        assert_eq!(sent(&h).last(), Some(&(PLAYER.to_string(), "Next")));
        let tree = access(&h.state).expect("the panel's tree");
        let focused = tree
            .get(tree.focus())
            .and_then(|n| n.label().map(str::to_string));
        assert_eq!(
            focused.as_deref(),
            Some("Next"),
            "a reader follows the button"
        );
        tap(&mut h, KEY_TAB);
        assert_eq!(focus(&h), None, "past the last, back on the cell");
        tap(&mut h, KEY_TAB);
        tap(&mut h, KEY_ESC);
        assert_eq!(focus(&h), None);
        assert!(
            h.state.task_preview.is_shown(),
            "Escape went back to the cell"
        );
        tap(&mut h, KEY_ESC);
        assert!(!h.state.task_preview.is_shown());
        assert_eq!(h.state.focused, None, "no window came up");
    }

    #[test]
    fn with_previews_off_enter_on_a_group_lists_its_windows() {
        let (mut h, _c, _, i) = group_with_keys(false);
        tap(&mut h, KEY_ENTER);
        assert!(!h.state.task_preview.is_shown());
        assert_eq!(
            h.state.menu_root(),
            Some(crate::menuview::MenuRoot::WindowList(i))
        );
        assert_eq!(h.state.focused, None);
    }

    /// Two windows on their own task buttons on a 1920x1080 output in `look`, `text`% and `scale`,
    /// the pointer resting on the first until its panel shows; returns both buttons and the panel.
    fn resting_on_the_first_button(
        look: &str,
        text: u32,
        scale: i32,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        [Rect; 2],
        Rect,
    ) {
        resting_on_the_first_button_at(look, text, scale, Edge::Bottom)
    }

    /// As `resting_on_the_first_button`, with the bar on `edge`.
    fn resting_on_the_first_button_at(
        look: &str,
        text: u32,
        scale: i32,
        edge: Edge,
    ) -> (
        crate::harness::Harness,
        crate::harness::TestClient,
        [Rect; 2],
        Rect,
    ) {
        let toml =
            format!("[appearance]\ninterface = \"{look}\"\n[accessibility]\ntext_size = {text}\n");
        let mut config = crate::config::Config::parse(&toml).unwrap();
        config.taskbar.position = edge;
        let mut h = crate::harness::Harness::with_config(config);
        let out = h.output("TEST-1", 1920, 1080);
        let scale = smithay::output::Scale::Integer(scale);
        out.change_current_state(None, None, Some(scale), None);
        h.state.update_layout();
        let mut c = h.connect();
        let ws = [
            h.map_toplevel(&mut c, "one", 300, 200).id,
            h.map_toplevel(&mut c, "two", 300, 200).id,
        ];
        let buttons = ws.map(|w| {
            let (i, _, _) = centre(&h, w);
            let bar = &h.state.bars[0];
            let (o, r) = (h.state.bar_drawn(&bar.output).shown, bar.layout.tasks[i]);
            Rect::new(o.x + r.x, o.y + r.y, r.width, r.height)
        });
        let (i, x, y) = centre(&h, ws[0]);
        assert_eq!(i, 0, "the first window's button comes first");
        h.pointer_to(x, y);
        h.until("the panel on a rest", |s| on(s, 0));
        let panel = h.state.task_preview.shown.as_ref().unwrap().layout.rect;
        (h, c, buttons, panel)
    }

    const LOOKS: [&str; 3] = ["classic", "blue", "glass"];

    #[test]
    fn a_slow_move_up_from_the_button_to_its_panel_keeps_the_panel() {
        for look in LOOKS {
            for text in [100, 200] {
                for scale in [1, 2] {
                    let (mut h, _c, [b, _], panel) = resting_on_the_first_button(look, text, scale);
                    let at = |h: &crate::harness::Harness| {
                        (look, text, scale, h.state.pointer_location())
                    };
                    let x = f64::from(b.x + b.width / 2);
                    let step = 1.0 / f64::from(scale);
                    let mut y = f64::from(b.y + b.height / 2);
                    while y >= f64::from(panel.bottom() - 4) {
                        y -= step;
                        h.pointer_to(x, y);
                        assert!(on(&h.state, 0), "{:?}: the panel stays open", at(&h));
                    }
                }
            }
        }
    }

    #[test]
    fn leaving_the_button_sideways_switches_or_closes_the_panel() {
        for look in LOOKS {
            for text in [100, 200] {
                for scale in [1, 2] {
                    let what = (look, text, scale);
                    let step = 1.0 / f64::from(scale);
                    let walk = |h: &mut crate::harness::Harness, from: (f64, f64), to_x: f64| {
                        let (mut x, y) = from;
                        while x < to_x {
                            x += step;
                            h.pointer_to(x, y);
                        }
                    };
                    let (mut h, _c, [a, b], _) = resting_on_the_first_button(look, text, scale);
                    let row = f64::from(a.y + a.height / 2);
                    walk(
                        &mut h,
                        (f64::from(a.x + a.width / 2), row),
                        f64::from(b.x + 2),
                    );
                    h.until("the next button's panel", |s| on(s, 1));
                    let panel = h.state.task_preview.shown.as_ref().unwrap().layout.rect;
                    let past = f64::from(b.right().max(panel.right()) + 2);
                    walk(&mut h, (f64::from(b.x + b.width / 2), row), past);
                    assert!(
                        !h.state.task_preview.is_shown(),
                        "{what:?}: closed on the empty bar"
                    );

                    let (mut h, _c, [a, b], panel) = resting_on_the_first_button(look, text, scale);
                    let x = f64::from(a.x + a.width / 2);
                    let gap = f64::from(a.y) - step;
                    h.pointer_to(x, gap);
                    assert!(on(&h.state, 0), "{what:?}: open above the button");
                    let past = f64::from(a.right().max(panel.right()) + 2);
                    walk(&mut h, (x, gap), past);
                    assert!(!on(&h.state, 0), "{what:?}: off the panel past it");
                    let shown = h.state.task_preview.shown.as_ref();
                    let end = shown.map_or(b.right(), |p| p.layout.rect.right().max(b.right()));
                    walk(&mut h, (past, gap), f64::from(end + 2));
                    assert!(
                        !h.state.task_preview.is_shown(),
                        "{what:?}: closed past the last button and its panel"
                    );
                }
            }
        }
    }

    /// Walks the pointer from `from` to `to` in steps of `step`, along x then y.
    fn walk_to(h: &mut crate::harness::Harness, from: (f64, f64), to: (f64, f64), step: f64) {
        let (mut x, mut y) = from;
        let toward = |v: f64, end: f64| {
            if (end - v).abs() <= step {
                end
            } else {
                v + step.copysign(end - v)
            }
        };
        while x != to.0 {
            x = toward(x, to.0);
            h.pointer_to(x, y);
            assert!(h.state.task_preview.is_shown(), "open at {:?}", (x, y));
        }
        while y != to.1 {
            y = toward(y, to.1);
            h.pointer_to(x, y);
            assert!(h.state.task_preview.is_shown(), "open at {:?}", (x, y));
        }
    }

    #[test]
    fn moving_from_one_task_button_to_the_next_switches_the_panel_at_once() {
        for edge in [Edge::Bottom, Edge::Left] {
            for look in LOOKS {
                for text in [100, 200] {
                    for scale in [1, 2] {
                        let what = (edge, look, text, scale);
                        let step = 1.0 / f64::from(scale);
                        let mid =
                            |r: Rect| (f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
                        let (mut h, _c, [a, b], _) =
                            resting_on_the_first_button_at(look, text, scale, edge);
                        walk_to(&mut h, mid(a), mid(b), step);
                        assert!(on(&h.state, 1), "{what:?}: straight across");

                        let (mut h, _c, [a, b], _) =
                            resting_on_the_first_button_at(look, text, scale, edge);
                        let off = match edge {
                            Edge::Bottom => (mid(a).0, f64::from(a.y) - step),
                            _ => (f64::from(a.right()), mid(a).1),
                        };
                        walk_to(&mut h, mid(a), off, step);
                        let beside = match edge {
                            Edge::Bottom => (mid(b).0, off.1),
                            _ => (off.0, mid(b).1),
                        };
                        walk_to(&mut h, off, beside, step);
                        walk_to(&mut h, beside, mid(b), step);
                        assert!(on(&h.state, 1), "{what:?}: over the bar's edge rows");
                    }
                }
            }
        }
    }
}
