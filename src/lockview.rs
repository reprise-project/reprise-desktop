//! The lock screen on the scene: one opaque cover, the wallpaper over it with `[wallpaper] lock_screen`,
//! one dialog per output in `layers.lock`, key and pointer routing while locked, and the password's and
//! fingerprint's PAM conversation children. Pure state lives in `lock.rs` and `lockfinger.rs`.

use std::os::fd::BorrowedFd;
use std::process::Command;
use std::time::{Duration, Instant};

use smithay::{
    backend::input::InputTime,
    input::keyboard::KeysymHandle,
    output::Output,
    reexports::{
        calloop::{
            Interest, LoopHandle, Mode, PostAction, RegistrationToken,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, SERIAL_COUNTER},
};

use reprisescene::{BufferId, RectId, TreeId};

use crate::compose::Fed;
use crate::compositor::State;
use crate::editkeys::KeyText;
use crate::input::{BTN_LEFT, Interaction};
use crate::keys;
use crate::lock::{self, LockButton, LockKey, LockState};
use crate::lockfinger;
use crate::raster::Rect;
use crate::sessionlock::{ClientLock, SessionLock};
use crate::wallpaperview::LockWall;
use crate::waterview::{Pane, Picture};
use crate::window::WindowId;

/// The lock's one `State` field (`lock`): the session-lock global and, while locked, the lock.
pub struct LockScreen {
    pub session: SessionLock,
    pub active: Option<Lock>,
    pub helpers: Helpers,
}

impl LockScreen {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> LockScreen {
        LockScreen {
            session: SessionLock::new(dh),
            active: None,
            helpers: Helpers::default(),
        }
    }
}

/// The commands the PAM conversations run, for a user, and how long a password check may take; tests
/// put scripts in their place.
pub struct Helpers {
    pub password: fn(&str) -> Command,
    pub finger: fn(&str) -> Option<Command>,
    pub verify_timeout: Duration,
}

impl Default for Helpers {
    fn default() -> Helpers {
        Helpers {
            password: lockfinger::password_command,
            finger: lockfinger::finger_command,
            verify_timeout: Duration::from_secs(90),
        }
    }
}

/// The OK and Cancel button rects of one output's dialog, in layout coordinates, for hit testing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub ok: Rect,
    pub cancel: Rect,
}

pub struct LockOutput {
    pub output: Output,
    tree: TreeId,
    rect: RectId,
    /// The wallpaper between the cover and the dialog; empty without one or with `lock_screen` off.
    wall: BufferId,
    dialog: BufferId,
    pub buttons: Buttons,
    /// The dialog's rect, layout; the lock water stays still under it.
    pub dialog_rect: Rect,
}

pub struct Lock {
    pub state: LockState,
    pub user: String,
    pub outputs: Vec<LockOutput>,
    pub pending: Option<Pending>,
    pub finger: Option<Helper>,
    /// The window focused when the lock engaged; refocused on unlock if it still exists and is mapped.
    pub prev_focus: Option<WindowId>,
    /// The layer surface holding the keyboard when the lock engaged; restored in preference to
    /// `prev_focus` on unlock if it still exists and is mapped.
    pub prev_focused_layer: Option<WlSurface>,
    /// The desktop held the keyboard when the lock engaged; unlock leaves it there.
    pub prev_desktop: bool,
    /// The external locker drawing the lock screen, if one holds the lock (see `sessionlock`); the dialog is hidden meanwhile.
    pub client: Option<ClientLock>,
    /// `[power] locker` was started for this lock and has yet to take over or time out; the dialog is hidden and ignores input.
    pub awaiting: bool,
}

impl Lock {
    /// Whether the built-in dialog is up: no locker holds the lock or is starting to.
    pub fn dialog_shown(&self) -> bool {
        dialog_shown(self.client.is_some(), self.awaiting)
    }
}

fn dialog_shown(client: bool, awaiting: bool) -> bool {
    !client && !awaiting
}

/// A loop source a lock owns: removed when dropped, unless its callback already removed it (`disarm`).
pub struct Source {
    token: Option<RegistrationToken>,
    handle: LoopHandle<'static, State>,
}

impl Source {
    fn disarm(&mut self) {
        self.token = None;
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        if let Some(t) = self.token.take() {
            self.handle.remove(t);
        }
    }
}

/// A PAM conversation's child (see `lockfinger`) and the source watching its stdout. Dropping it removes
/// the source and interrupts the child, which releases what it holds (the fingerprint reader), without
/// waiting for it.
pub struct Helper {
    source: Source,
    conversation: lockfinger::Conversation,
}

impl Drop for Helper {
    fn drop(&mut self) {
        reap_later(&self.source.handle);
    }
}

/// Turn `lockfinger::reap_ended` until every ended conversation is reaped or killed.
fn reap_later(handle: &LoopHandle<'static, State>) {
    let armed = handle.insert_source(
        Timer::from_duration(lockfinger::RELEASE_GRACE),
        |_, _, _| match lockfinger::reap_ended(Instant::now()) {
            Some(due) => TimeoutAction::ToInstant(due),
            None => TimeoutAction::Drop,
        },
    );
    if let Err(e) = armed {
        log::warn!(
            "lock: cannot time the end of a PAM conversation: {}",
            e.error
        );
    }
}

/// One password check in flight: its conversation, the notices it sent so far, and the timer that ends
/// a PAM stack that hangs (a dead network login server, a wedged keyring daemon). The timer is off
/// while a further prompt waits for the user.
pub struct Pending {
    helper: Helper,
    answer: lockfinger::PasswordAnswer,
    timer: Option<Source>,
}

/// The check took `verify_timeout`: kill its child and let the dialog take a password again.
fn verify_timed_out(state: &mut State) -> TimeoutAction {
    if let Some(p) = state.lock.active.as_mut().and_then(|l| l.pending.as_mut()) {
        if let Some(t) = p.timer.as_mut() {
            t.disarm();
        }
        p.helper.conversation.kill();
    }
    log::error!("lock: the password conversation did not answer in time; ended it");
    abort_verification(state, lock::UNAVAILABLE);
    TimeoutAction::Drop
}

/// Watch `conversation`'s stdout with `on_read`. None when the loop refuses the source; the child ends.
fn watch(
    state: &State,
    conversation: lockfinger::Conversation,
    on_read: fn(&mut State) -> PostAction,
) -> Option<Helper> {
    // SAFETY: the conversation's stdout is open for as long as the conversation, which outlives this borrow.
    let fd = unsafe { BorrowedFd::borrow_raw(conversation.fd()) }.try_clone_to_owned();
    let token = fd.ok().and_then(|fd| {
        state
            .handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                move |_, _, state| Ok(on_read(state)),
            )
            .ok()
    });
    let Some(token) = token else {
        log::warn!("lock: cannot watch a PAM conversation");
        drop(conversation);
        reap_later(&state.handle);
        return None;
    };
    Some(Helper {
        source: Source {
            token: Some(token),
            handle: state.handle.clone(),
        },
        conversation,
    })
}

/// Start the fingerprint conversation for `user`. None when the PAM service is not installed or the
/// child or its loop source cannot be had; the password still unlocks.
fn start_finger(state: &State, user: &str) -> Option<Helper> {
    let command = (state.lock.helpers.finger)(user)?;
    match lockfinger::Conversation::start(command) {
        Ok(c) => watch(state, c, on_finger),
        Err(e) => {
            log::warn!("lock: cannot start the fingerprint conversation: {e}");
            None
        }
    }
}

/// What the fingerprint conversation said since the last wake. Its PAM success unlocks; its end drops it.
fn on_finger(state: &mut State) -> PostAction {
    let Some(l) = state.lock.active.as_mut() else {
        return PostAction::Remove;
    };
    let Some(f) = l.finger.as_mut() else {
        return PostAction::Remove;
    };
    let (lines, eof) = f.conversation.read();
    for line in &lines {
        if let lockfinger::Line::Result(rc) = line {
            log::info!("lock: the fingerprint conversation ended with PAM code {rc}");
        }
    }
    let accepted = lines.into_iter().any(|line| l.state.finger(line));
    if accepted || eof {
        // This callback's Remove takes the source down; the child is reaped here.
        if let Some(mut f) = l.finger.take() {
            f.source.disarm();
        }
    }
    if accepted {
        log::info!("lock: fingerprint accepted");
        unlock(state);
        return PostAction::Remove;
    }
    if eof {
        l.state.finger = None;
    }
    redraw(state);
    if eof {
        PostAction::Remove
    } else {
        PostAction::Continue
    }
}

/// Never leave the dialog stuck on "Verifying...": end the check, show `message` and redraw.
fn abort_verification(state: &mut State, message: &str) {
    if let Some(l) = state.lock.active.as_mut() {
        l.pending = None;
        l.state.fail_with(message);
    }
    redraw(state);
}

/// What the password conversation said since the last wake: a further prompt relabels the field for
/// its answer; its verdict unlocks or shows why not; an end without one reads as "could not be
/// checked", never as accepted.
fn on_result(state: &mut State) -> PostAction {
    let Some(l) = state.lock.active.as_mut() else {
        return PostAction::Remove;
    };
    let Some(p) = l.pending.as_mut() else {
        return PostAction::Remove;
    };
    let (lines, eof) = p.helper.conversation.read();
    let mut prompt = None;
    let verdict = lines.into_iter().find_map(|line| match line {
        lockfinger::Line::Prompt(q) => {
            prompt = Some(q);
            None
        }
        line => p.answer.line(line),
    });
    if verdict.is_none() && !eof {
        if let Some(q) = prompt {
            p.timer = None;
            l.state.ask(q);
            redraw(state);
        }
        return PostAction::Continue;
    }
    // This callback's Remove takes the source down.
    if let Some(mut p) = l.pending.take() {
        p.helper.source.disarm();
    }
    match verdict {
        Some(Ok(())) => {
            l.state.verifying = false;
            unlock(state);
        }
        Some(Err(notices)) => abort_verification(state, &lock::failure_text(&notices)),
        None => {
            log::error!("lock: the password conversation ended without a result");
            abort_verification(state, lock::UNAVAILABLE);
        }
    }
    PostAction::Remove
}

/// Lock the session, then start `[power] locker` if one is set. No-op when already locked.
pub fn lock(state: &mut State) {
    if state.is_locked() {
        return;
    }
    engage(state);
    crate::sessionlock::start_locker(state);
    start_dialog_finger(state);
}

/// Start the fingerprint conversation if the built-in dialog shows and none runs. Never under a locker,
/// which may claim the reader through its own pam_fprintd, and fprintd lets one claim it.
pub fn start_dialog_finger(state: &mut State) {
    let Some(l) = state.lock.active.as_ref() else {
        return;
    };
    if !l.dialog_shown() || l.finger.is_some() || l.user.is_empty() {
        return;
    }
    let user = l.user.clone();
    let finger = start_finger(state, &user);
    if let Some(l) = state.lock.active.as_mut() {
        l.finger = finger;
    }
}

/// Engage the lock: cover every output, clear client focus, route input to the dialog. No-op when
/// already locked. A locker's own lock comes through here directly so it does not start another
/// locker. Window captures go blank while it holds (`capture::render_window`).
pub fn engage(state: &mut State) {
    if state.is_locked() {
        return;
    }
    state.close_menu();
    if let Some(c) = state.compose.as_mut() {
        c.reset();
    }
    crate::idle::end_dim(state);
    state.release_capture();
    state.switcher_cancel();
    state.close_overview();
    crate::taskpreview::hide(state);
    crate::snappickerview::hide(state);
    crate::trayflyout::close(state);
    crate::trayreveal::collapse_all(state);
    crate::notifycenter::close(state);
    crate::calendarview::close(state);
    crate::displaysview::close(state);
    // The panel would sit under the lock cover but its caret timer would keep redrawing it.
    state.close_launcher();
    state.close_start_panel();
    // The emoji picker and clipboard history would type or paste into whatever has the keyboard after
    // unlock; the region overlay would take the pointer from the lock dialog.
    state.close_panels();
    // Before the lock is up: the drag's end puts the pointer back on a client, which the focus clearing below takes away.
    state.cancel_drag();
    cancel_interaction(state);
    state.release_all_buttons(InputTime::now());
    state.caption_clicks.reset();
    let prev_focus = state.focused;
    let prev_focused_layer = crate::layer::holds_keyboard(state)
        .then(|| state.layers_shell.focused.clone())
        .flatten()
        .or_else(|| {
            let kb = state.seat.get_keyboard()?.current_focus()?;
            crate::layer::is_mapped_layer(state, &kb).then_some(kb)
        });
    let prev_desktop = prev_focus.is_none() && prev_focused_layer.is_none();
    let user = lock::current_user();
    if user.is_empty() {
        // Engaged all the same: a lock that does not engage leaves the session open to whoever walks up,
        // while this one can be left from another VT with `loginctl unlock-session`.
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        log::error!("lock: no login name for uid {uid}; the password cannot be checked");
    }
    state.lock.active = Some(Lock {
        state: LockState::new(),
        user,
        outputs: Vec::new(),
        pending: None,
        finger: None,
        prev_focus,
        prev_focused_layer,
        prev_desktop,
        client: None,
        awaiting: false,
    });
    // A client's popup grab keeps the keyboard on its popup, refuses the focus changes below and would
    // leave a locker without keys. Ending it closes the popup.
    if let Some(k) = state.seat.get_keyboard() {
        k.unset_grab(state);
    }
    if let Some(p) = state.seat.get_pointer() {
        p.unset_grab(state, SERIAL_COUNTER.next_serial(), InputTime::now());
    }
    state.enter_keyboard(None);
    let p = state.pointer_location();
    state.pointer_motion(None, p, InputTime::now());
    state.layers_shell.focused = None;
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for o in &outputs {
        cover_output(state, o);
    }
    state.set_cursor("default");
    crate::power::hint(state, crate::power::Hint::Locked(true));
    crate::idle::inhibition_changed(state);
    state.redraw_all();
    log::info!("session locked");
}

fn cancel_interaction(state: &mut State) {
    match state.interaction {
        Interaction::Drag { .. } => state.drag_view_cancel(),
        Interaction::Marquee => state.end_marquee(),
        Interaction::Stir => {
            state.end_marquee();
            state.stir_cancel(reprisekit::fluid::Source::Pointer);
        }
        Interaction::BarMove { .. } | Interaction::BarWidth { .. } => state.cancel_bar_drag(),
        _ => {}
    }
    if let Interaction::Move { window, .. }
    | Interaction::Resize { window, .. }
    | Interaction::PressButton { window, .. } = state.interaction
    {
        state.cancel_interaction_for(window);
    }
    // A held taskbar button lets go without acting and stops drawing held.
    if let was @ (Interaction::TaskPress { .. }
    | Interaction::QuickPress { .. }
    | Interaction::ArrowPress { .. }
    | Interaction::Band { .. }) = std::mem::take(&mut state.interaction)
    {
        state.cancel_taskbar_press(was);
    }
    state.interaction = Interaction::Idle;
}

/// Take the lock's scene nodes, sources and child down without touching focus. Returns the window and
/// layer surface that held focus when the lock engaged, if any, and whether the desktop held it.
fn teardown(state: &mut State) -> Option<(Option<WindowId>, Option<WlSurface>, bool)> {
    let mut l = state.lock.active.take()?;
    l.pending = None;
    // The fingerprint child is interrupted and frees the reader as it ends. A new lock's conversation
    // that finds the reader still claimed fails, which `on_finger` logs.
    l.finger = None;
    // The compositor ends the lock: a locker still holding it hears `finished`.
    if let Some(c) = l.client.take() {
        crate::sessionlock::release(state, c, true);
    }
    crate::sessionlock::stop_timers(state);
    for lo in &l.outputs {
        state.scene.destroy(lo.tree);
        state.water.drop_pane(&lo.output.name(), Pane::Lock);
    }
    Some((l.prev_focus, l.prev_focused_layer, l.prev_desktop))
}

/// Unlock, from a correct password or logind's `Unlock`, and hand focus back. A layer surface that
/// claimed (or held) exclusive keyboard interactivity while locked outranks everything; otherwise the
/// layer surface, window or desktop that had the keyboard when the lock engaged, else the top window.
pub fn unlock(state: &mut State) {
    let Some((prev, prev_layer, prev_desktop)) = teardown(state) else {
        return;
    };
    crate::power::unlocked(state);
    crate::idle::inhibition_changed(state);
    if crate::layer::holds_keyboard(state) {
        // Recorded, not entered, while locked (by `layer::focus_layer` or mapping with exclusive interactivity).
        if let Some(s) = state.layers_shell.focused.clone() {
            crate::layer::focus_layer(state, &s);
        }
    } else if let Some(s) = prev_layer.filter(|s| crate::layer::is_mapped_layer(state, s)) {
        crate::layer::focus_layer(state, &s);
    } else if prev_desktop && state.focused.is_none() {
        state.enter_keyboard(None);
    } else {
        let alive = prev.filter(|&w| state.win(w).is_some_and(|w| w.mapped && !w.hidden()));
        match alive {
            Some(w) => state.focus_window(w),
            None => state.focus_top(),
        }
    }
    state.ime_sync_grab();
    crate::autohideview::poke(state);
    let p = state.pointer_location();
    state.repoint(p, InputTime::now());
    state.redraw_all();
    log::info!("session unlocked");
}

impl State {
    pub fn is_locked(&self) -> bool {
        self.lock.active.is_some()
    }
}

/// Cover `output` (or refit its cover to its box, the theme's background and the wallpaper) with an
/// opaque rect, the wallpaper and a centred dialog. An output with no layout box loses its cover.
fn cover_output(state: &mut State, output: &Output) {
    if !state.is_locked() {
        return;
    }
    let b = state.layout_box(Some(output));
    if b.width <= 0 {
        forget_output(state, output);
        return;
    }
    let color = state.config.theme.colors.background.as_f32();
    let existing = state
        .lock
        .active
        .as_ref()
        .and_then(|l| l.outputs.iter().find(|lo| lo.output == *output))
        .map(|lo| (lo.tree, lo.rect, lo.wall));
    match existing {
        Some((tree, rect, wall)) => {
            state.scene.set_position(tree, b.x, b.y);
            state.scene.rect_set_size(rect, (b.width, b.height));
            state.scene.rect_set_color(rect, color);
            paint_wall(state, output, tree, wall);
        }
        None => {
            let tree = state.scene.tree_create(state.layers.lock);
            state.scene.set_position(tree, b.x, b.y);
            let rect = state.scene.rect_create(tree, (b.width, b.height), color);
            let wall = state.scene.buffer_create(tree);
            paint_wall(state, output, tree, wall);
            let dialog = state.scene.buffer_create(tree);
            if let Some(l) = state.lock.active.as_mut() {
                l.outputs.push(LockOutput {
                    output: output.clone(),
                    tree,
                    rect,
                    wall,
                    dialog,
                    buttons: Buttons::default(),
                    dialog_rect: Rect::default(),
                });
            }
            crate::sessionlock::raise(state);
        }
    }
    redraw_output(state, output);
}

/// Fill `wall` with the wallpaper as `output` shows it, or empty it without one or with `lock_screen`
/// off. With `[wallpaper] water` on, the cover's `tree` gets the water node, which draws in place of `wall`.
fn paint_wall(state: &mut State, output: &Output, tree: TreeId, wall: BufferId) {
    let shown = match state.config.wallpaper.lock_screen {
        true => state.lock_wallpaper(output),
        false => LockWall::None,
    };
    let b = state.layout_box(Some(output));
    let water = state.water_on() && !matches!(shown, LockWall::None);
    let picture = match shown {
        LockWall::Shown(t) => {
            if let Some(backend) = state.backend.as_ref() {
                state
                    .scene
                    .buffer_set_texture(wall, &backend.renderer, t.clone());
                state
                    .scene
                    .buffer_set_dest_size(wall, Some((b.width, b.height).into()));
            }
            Some(Picture::Texture(t))
        }
        LockWall::Shared(from) => {
            state.scene.buffer_share(wall, from);
            state
                .scene
                .buffer_set_dest_size(wall, Some((b.width, b.height).into()));
            match water {
                true => (state
                    .water_texture(output, Pane::Desktop)
                    .map(Picture::Texture))
                .or_else(|| {
                    let scale = state.draw_scale_at(Some(output));
                    state.wallpaper_canvas(output, scale).map(Picture::Canvas)
                }),
                false => None,
            }
        }
        LockWall::Composed(c) => {
            let scale = state.draw_scale_at(Some(output));
            crate::output::upload_scaled(&mut state.scene, wall, &c, scale);
            Some(Picture::Canvas(c))
        }
        // The picture before stays, stretched, until the job's picture lands (`repaint_wall`).
        LockWall::Composing => {
            if state.scene.buffer_has_pixels(wall) {
                state
                    .scene
                    .buffer_set_dest_size(wall, Some((b.width, b.height).into()));
            }
            None
        }
        LockWall::None => {
            state.scene.buffer_clear(wall);
            None
        }
    };
    if !water {
        state.water_lock_uncover(output, wall);
        return;
    }
    state.water_lock_cover(output, tree, wall);
    if let Some(p) = picture {
        state.water_picture(output, Pane::Lock, p, false);
    }
}

/// A job's wallpaper picture for `output` landed: show it on its lock screen too.
pub fn repaint_wall(state: &mut State, output: &Output) {
    let Some((tree, wall)) = (state.lock.active.as_ref())
        .and_then(|l| l.outputs.iter().find(|lo| lo.output == *output))
        .map(|lo| (lo.tree, lo.wall))
    else {
        return;
    };
    paint_wall(state, output, tree, wall);
    redraw_output(state, output);
}

fn forget_output(state: &mut State, output: &Output) {
    let Some(l) = state.lock.active.as_mut() else {
        return;
    };
    let Some(i) = l.outputs.iter().position(|lo| lo.output == *output) else {
        return;
    };
    let lo = l.outputs.remove(i);
    state.water.drop_pane(&output.name(), Pane::Lock);
    state.scene.destroy(lo.tree);
}

/// Re-cover every output after a layout, theme or wallpaper change.
pub fn refit(state: &mut State) {
    if !state.is_locked() {
        return;
    }
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    let gone: Vec<Output> = state
        .lock
        .active
        .as_ref()
        .map(|l| {
            l.outputs
                .iter()
                .map(|lo| lo.output.clone())
                .filter(|o| !outputs.contains(o))
                .collect()
        })
        .unwrap_or_default();
    for o in &gone {
        forget_output(state, o);
    }
    for o in &outputs {
        cover_output(state, o);
    }
    crate::sessionlock::refit(state);
    state.redraw_all();
}

fn caps_lock(state: &State) -> bool {
    state
        .seat
        .get_keyboard()
        .is_some_and(|k| k.modifier_state().caps_lock)
}

pub fn redraw(state: &mut State) {
    let caps = caps_lock(state);
    let outputs: Vec<Output> = match state.lock.active.as_mut() {
        Some(l) => {
            l.state.caps_lock = caps;
            l.outputs.iter().map(|lo| lo.output.clone()).collect()
        }
        None => return,
    };
    for o in &outputs {
        redraw_output(state, o);
    }
    state.redraw_all();
}

const LOCK_ICON: &str = "system-lock-screen";

fn redraw_output(state: &mut State, output: &Output) {
    let scale = state.draw_scale_at(Some(output));
    let px = state.config.theme.scaled(scale).icon_size();
    let icon = state.icons.get_shared(LOCK_ICON, px);
    let b = state.layout_box(Some(output));
    let logical = lock::dialog_layout(&state.config.theme.scaled(1));
    let theme = state.config.theme.scaled(scale);
    let Some(l) = state.lock.active.as_ref() else {
        return;
    };
    let Some(lo) = l.outputs.iter().find(|lo| lo.output == *output) else {
        return;
    };
    let dialog = lo.dialog;
    let canvas = lock::draw_dialog(
        &theme,
        state.text.for_output(theme.scale),
        icon.as_deref(),
        &l.user,
        &l.state,
    );
    let shown = l.dialog_shown();
    let (w, h) = logical.size;
    let x = b.x + (b.width - w) / 2;
    let y = b.y + (b.height - h) / 2;
    let at = |r: Rect| Rect::new(x + r.x, y + r.y, r.width, r.height);
    let buttons = Buttons {
        ok: at(logical.ok),
        cancel: at(logical.cancel),
    };
    crate::output::upload_scaled(&mut state.scene, dialog, &canvas, scale);
    state.scene.set_position(dialog, x - b.x, y - b.y);
    state.scene.set_enabled(dialog, shown);
    if let Some(lo) = state
        .lock
        .active
        .as_mut()
        .and_then(|l| l.outputs.iter_mut().find(|lo| lo.output == *output))
    {
        lo.buttons = buttons;
        lo.dialog_rect = Rect::new(x, y, w, h);
    }
}

/// Which dialog button, if any, is at a layout point; None while the dialog is hidden.
pub fn hit(l: &Lock, lx: f64, ly: f64) -> Option<LockButton> {
    button_at(
        l.dialog_shown(),
        l.outputs.iter().map(|o| &o.buttons),
        lx,
        ly,
    )
}

fn button_at<'a>(
    shown: bool,
    outputs: impl IntoIterator<Item = &'a Buttons>,
    lx: f64,
    ly: f64,
) -> Option<LockButton> {
    if !shown {
        return None;
    }
    let (x, y) = (lx.floor() as i32, ly.floor() as i32);
    for b in outputs {
        if b.ok.contains(x, y) {
            return Some(LockButton::Ok);
        }
        if b.cancel.contains(x, y) {
            return Some(LockButton::Cancel);
        }
    }
    None
}

/// End the check before `verify_timeout`, or None when the loop refuses the timer.
fn verify_timer(state: &State) -> Option<Source> {
    state
        .handle
        .insert_source(
            Timer::from_duration(state.lock.helpers.verify_timeout),
            |_, _, state| verify_timed_out(state),
        )
        .map_err(|e| log::warn!("lock: cannot time the password check: {}", e.error))
        .ok()
        .map(|token| Source {
            token: Some(token),
            handle: state.handle.clone(),
        })
}

/// Hand the field to PAM: the password starts a conversation; under a further prompt the field is
/// that prompt's answer to the waiting one.
fn submit(state: &mut State) {
    let Some(l) = state.lock.active.as_mut() else {
        return;
    };
    l.state.verifying = true;
    l.state.message = None;
    let password = l.state.take_input();
    if let Some(p) = l.pending.as_mut() {
        if let Err(e) = p.helper.conversation.answer(password.as_bytes()) {
            log::error!("lock: cannot answer the password conversation's prompt: {e}");
            abort_verification(state, lock::UNAVAILABLE);
            return;
        }
        drop(password);
        let timer = verify_timer(state);
        if let Some(p) = state.lock.active.as_mut().and_then(|l| l.pending.as_mut()) {
            p.timer = timer;
        }
        redraw(state);
        return;
    }
    let command = (state.lock.helpers.password)(&l.user);
    let helper = match lockfinger::Conversation::start_password(command, password.as_bytes()) {
        Ok(c) => watch(state, c, on_result),
        Err(e) => {
            log::error!("lock: cannot start the password conversation: {e}");
            None
        }
    };
    drop(password);
    let timer = helper.as_ref().and_then(|_| verify_timer(state));
    if let Some(l) = state.lock.active.as_mut() {
        match helper {
            Some(helper) => {
                l.pending = Some(Pending {
                    helper,
                    answer: Default::default(),
                    timer,
                })
            }
            None => l.state.fail_with(lock::UNAVAILABLE),
        }
    }
    redraw(state);
}

/// Escape or Cancel under a further prompt drops it: its conversation ends and the field takes a
/// password again.
fn end_abandoned_prompt(l: &mut Lock) {
    if l.state.prompt.is_none() && !l.state.verifying {
        l.pending = None;
    }
}

/// A key while locked, from the keyboard filter. Returns whether it goes on to the focused surface: a
/// locker's, when one holds the lock. Otherwise the dialog takes presses and every key is consumed.
pub fn key(state: &mut State, pressed: bool, handle: &KeysymHandle<'_>) -> bool {
    if crate::sessionlock::holds_input(state) {
        crate::sessionlock::check_alive(state);
        if crate::sessionlock::holds_input(state) {
            return true;
        }
    }
    let shown = state.lock.active.as_ref().is_some_and(Lock::dialog_shown);
    if shown
        && state
            .lock
            .active
            .as_ref()
            .is_some_and(|l| l.state.caps_lock != caps_lock(state))
    {
        redraw(state);
    }
    if !pressed || !shown {
        return false;
    }
    // Dead keys and the Compose key: a sequence types nothing until it ends, then its result.
    let fed = match state.compose.as_mut() {
        Some(c) => c.feed(handle.modified_sym().raw()),
        None => Fed::Plain,
    };
    let text: KeyText;
    let key = match fed {
        Fed::Swallowed => return false,
        Fed::Composed(t) => {
            text = t;
            LockKey::Text(text.as_str())
        }
        Fed::Plain => {
            let special = handle
                .modified_syms()
                .iter()
                .find_map(|sym| match sym.raw() {
                    keys::KEY_Return | keys::KEY_KP_Enter => Some(LockKey::Enter),
                    keys::KEY_BackSpace => Some(LockKey::Backspace),
                    keys::KEY_Escape => Some(LockKey::Escape),
                    _ => None,
                });
            match special {
                Some(key) => key,
                None => {
                    let Ok(xkb) = handle.xkb().lock() else {
                        return false;
                    };
                    // SAFETY: the state reference does not outlive the lock guard.
                    let t = KeyText::of_key(unsafe { xkb.state() }, handle.raw_code());
                    drop(xkb);
                    let Some(t) = t else {
                        return false;
                    };
                    text = t;
                    LockKey::Text(text.as_str())
                }
            }
        }
    };
    if let LockKey::Text(s) = key
        && s.chars().any(char::is_control)
    {
        return false;
    }
    let should_submit = match state.lock.active.as_mut() {
        Some(l) => {
            let submit = l.state.key(key);
            end_abandoned_prompt(l);
            submit
        }
        None => return false,
    };
    if should_submit {
        submit(state);
    } else {
        redraw(state);
    }
    false
}

/// A press while locked: a locker's surfaces take it through the seat; otherwise a left press on OK
/// submits, on Cancel clears, and beside the dialog splashes the lock water.
pub fn button(state: &mut State, button: u32, pos: Point<f64, Logical>) {
    crate::sessionlock::check_alive(state);
    if button != BTN_LEFT || crate::sessionlock::holds_input(state) {
        return;
    }
    state.water_lock_click(pos);
    let should_submit = {
        let Some(l) = state.lock.active.as_mut() else {
            return;
        };
        let Some(b) = hit(l, pos.x, pos.y) else {
            return;
        };
        let submit = l.state.button(b);
        end_abandoned_prompt(l);
        submit
    };
    if should_submit {
        submit(state);
    } else {
        redraw(state);
    }
}

/// The pointer moved while locked: to the locker's surface under it, else to no client.
pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) {
    if !crate::sessionlock::motion(state, pos, time) {
        state.pointer_motion(None, pos, time);
        state.set_cursor("default");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buttons(ok: Rect, cancel: Rect) -> Buttons {
        Buttons { ok, cancel }
    }

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    /// A harness locked behind the dialog, no fingerprint conversation, `password` as the password's.
    fn locked(password: fn(&str) -> Command) -> crate::harness::Harness {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers = Helpers {
            password,
            finger: |_| None,
            ..Helpers::default()
        };
        lock(&mut h.state);
        h
    }

    /// Type `password`, submit it and turn the loop until the check ends (or 5 s pass).
    fn check(h: &mut crate::harness::Harness, password: &str) {
        let l = h.state.lock.active.as_mut().expect("locked");
        l.state.input.push_str(password);
        submit(&mut h.state);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (h.state.lock.active.as_ref()).is_some_and(|l| l.state.verifying) {
            assert!(std::time::Instant::now() < end, "the check ends");
            h.run_for(5);
        }
    }

    fn message(h: &crate::harness::Harness) -> Option<String> {
        h.state.lock.active.as_ref()?.state.message.clone()
    }

    #[test]
    fn the_password_conversation_unlocks_on_its_success_only() {
        let mut h = locked(|_| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#));
        check(&mut h, "hunter1");
        assert!(h.state.is_locked());
        assert_eq!(message(&h).as_deref(), Some(lock::INCORRECT));
        check(&mut h, "hunter2");
        assert!(!h.state.is_locked());
    }

    #[test]
    fn a_rejection_shows_the_conversations_text() {
        let mut h = locked(|_| sh("read p; echo 'E Account locked.'; echo '= 7'"));
        check(&mut h, "hunter2");
        assert!(h.state.is_locked());
        assert_eq!(message(&h).as_deref(), Some("Account locked."));
    }

    #[test]
    fn a_conversation_that_ends_without_a_result_reads_as_unavailable() {
        let mut h = locked(|_| sh("exit 0"));
        check(&mut h, "hunter2");
        assert!(h.state.is_locked());
        assert_eq!(message(&h).as_deref(), Some(lock::UNAVAILABLE));
    }

    /// Turn the loop until `done` holds (or 5 s pass).
    fn turn_until(h: &mut crate::harness::Harness, what: &str, done: impl Fn(&Lock) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !h.state.lock.active.as_ref().is_some_and(&done) {
            assert!(std::time::Instant::now() < end, "{what}");
            h.run_for(5);
        }
    }

    #[test]
    fn a_second_prompt_relabels_the_field_and_its_answer_unlocks() {
        let mut h = locked(|_| {
            sh(
                r#"read p; echo '?? Verification code:'; read c; [ "$p $c" = "hunter2 123456" ] && echo '= 0' || echo '= 7'"#,
            )
        });
        let l = h.state.lock.active.as_mut().expect("locked");
        l.state.input.push_str("hunter2");
        submit(&mut h.state);
        turn_until(&mut h, "the prompt arrives", |l| l.state.prompt.is_some());
        let l = h.state.lock.active.as_ref().unwrap();
        assert_eq!(
            l.state.prompt,
            Some(lock::Prompt {
                text: "Verification code:".into(),
                echo: true
            })
        );
        assert!(!l.state.verifying);
        assert_eq!(l.state.input, "", "the field is cleared for the answer");
        assert!(l.pending.as_ref().unwrap().timer.is_none());
        check(&mut h, "123456");
        assert!(!h.state.is_locked());
    }

    #[test]
    fn escape_under_a_second_prompt_ends_its_conversation() {
        let mut h = locked(|_| sh("read p; echo '? One-time code:'; exec sleep 60"));
        let l = h.state.lock.active.as_mut().expect("locked");
        l.state.input.push_str("hunter2");
        submit(&mut h.state);
        turn_until(&mut h, "the prompt arrives", |l| l.state.prompt.is_some());
        let l = h.state.lock.active.as_mut().unwrap();
        l.state.key(LockKey::Escape);
        end_abandoned_prompt(l);
        assert!(l.pending.is_none());
        assert_eq!(l.state.prompt, None);
        check(&mut h, "hunter2");
        assert!(
            h.state.lock.active.as_ref().unwrap().state.prompt.is_some(),
            "the next password starts a new conversation"
        );
    }

    #[test]
    fn caps_lock_shows_its_notice_while_on() {
        const KEY_CAPSLOCK: u32 = 58;
        let mut h = locked(|_| sh("exit 0"));
        let caps =
            |h: &crate::harness::Harness| h.state.lock.active.as_ref().unwrap().state.caps_lock;
        assert!(!caps(&h));
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(caps(&h), "the dialog redraws with the notice");
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(!caps(&h));
    }

    #[test]
    fn a_check_that_never_answers_ends_as_unavailable_after_the_timeout() {
        let mut h = locked(|_| sh("exec sleep 60"));
        h.state.lock.helpers.verify_timeout = std::time::Duration::from_millis(100);
        check(&mut h, "hunter2");
        let l = h.state.lock.active.as_ref().expect("still locked");
        assert!(!l.state.verifying);
        assert!(l.pending.is_none());
        assert_eq!(l.state.message.as_deref(), Some(lock::UNAVAILABLE));
    }

    #[test]
    fn unlocking_does_not_wait_for_a_fingerprint_child_that_ignores_the_interrupt() {
        let mut h = locked(|_| sh("exit 0"));
        h.state.lock.helpers.finger = |_| Some(sh("trap '' INT; echo 'I ready'; exec sleep 60"));
        let user = h.state.lock.active.as_ref().unwrap().user.clone();
        let finger = start_finger(&h.state, &user).expect("a fingerprint conversation");
        let pid = finger.conversation.pid();
        h.state.lock.active.as_mut().unwrap().finger = Some(finger);
        h.run_for(50);
        let start = std::time::Instant::now();
        unlock(&mut h.state);
        assert!(start.elapsed() < std::time::Duration::from_millis(50));
        assert!(!h.state.is_locked());
        let end = start + std::time::Duration::from_secs(5);
        while !crate::lockfinger::tests::ended(pid) {
            assert!(std::time::Instant::now() < end, "the child is killed");
            h.run_for(20);
        }
        assert!(start.elapsed() >= lockfinger::RELEASE_GRACE);
    }

    #[test]
    fn only_the_dialog_runs_the_fingerprint_conversation() {
        let mut h = locked(|_| sh("exit 0"));
        let finger =
            |h: &crate::harness::Harness| h.state.lock.active.as_ref().unwrap().finger.is_some();
        assert!(!finger(&h));
        unlock(&mut h.state);
        h.state.lock.helpers.finger = |_| Some(sh("echo 'I ready'; exec sleep 60"));
        let mut c = h.connect();
        let (_lock, _surface) = h.map_lock_surface(&mut c);
        assert!(h.state.is_locked());
        assert!(!finger(&h), "a locker's lock starts none");
        drop(c);
        h.run_for(50);
        assert!(h.state.lock.active.as_ref().unwrap().dialog_shown());
        assert!(
            finger(&h),
            "the dialog that replaces a gone locker starts one"
        );
        unlock(&mut h.state);
        lock(&mut h.state);
        assert!(finger(&h), "the dialog's own lock starts one");
        let mut c = h.connect();
        let (_lock, _surface) = h.map_lock_surface(&mut c);
        assert!(!finger(&h), "a locker taking over the dialog ends it");
    }

    #[test]
    fn hit_resolves_ok_cancel_and_outside_across_outputs() {
        let a = buttons(Rect::new(0, 0, 10, 10), Rect::new(20, 0, 10, 10));
        let b = buttons(Rect::new(100, 100, 10, 10), Rect::new(120, 100, 10, 10));
        let all = [a, b];
        let at = |x, y| button_at(true, &all, x, y);
        assert_eq!(at(5.0, 5.0), Some(LockButton::Ok));
        assert_eq!(at(25.0, 5.0), Some(LockButton::Cancel));
        assert_eq!(at(105.0, 105.0), Some(LockButton::Ok));
        assert_eq!(at(125.0, 105.0), Some(LockButton::Cancel));
        assert_eq!(at(50.0, 50.0), None);
        assert_eq!(at(-1.0, -1.0), None);
    }

    #[test]
    fn a_hidden_dialog_has_no_buttons() {
        let all = [buttons(Rect::new(0, 0, 10, 10), Rect::new(20, 0, 10, 10))];
        assert!(dialog_shown(false, false));
        assert!(!dialog_shown(false, true), "a locker is starting");
        assert!(!dialog_shown(true, false), "a locker holds the lock");
        assert_eq!(button_at(false, &all, 5.0, 5.0), None);
        assert_eq!(button_at(false, &all, 25.0, 5.0), None);
    }
}
