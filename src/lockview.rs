//! The lock screen on the scene, one lock per side in lock mode: covers, wallpaper and dialog in
//! `layers.lock`, input routing and the PAM children. Pure state lives in `lock.rs` and `lockfinger.rs`.

use std::os::fd::BorrowedFd;
use std::process::Command;
use std::time::{Duration, Instant};

use smithay::{
    backend::input::InputTime,
    input::keyboard::{Keycode, KeysymHandle, xkb},
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

use crate::compose::{Compose, Fed};
use crate::compositor::State;
use crate::console::Side;
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

/// The lock's one `State` field (`lock`): the session-lock global and, while locked, the locks.
pub struct LockScreen {
    pub session: SessionLock,
    /// The lock on the computer's own outputs, the only one outside lock mode.
    pub console: Option<Lock>,
    /// Lock mode's lock on the Remote outputs: the remote side's own dialog and password check.
    pub remote: Option<Lock>,
    /// The remote side's failed checks; it outlives each remote lock.
    pub remote_backoff: lock::Backoff,
    pub helpers: Helpers,
    /// The session is paused (another VT or user has the seat); the fingerprint reader is not its.
    pub seat_away: bool,
}

impl LockScreen {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> LockScreen {
        LockScreen {
            session: SessionLock::new(dh),
            console: None,
            remote: None,
            remote_backoff: lock::Backoff::default(),
            helpers: Helpers::default(),
            seat_away: false,
        }
    }

    pub fn on(&self, side: Side) -> Option<&Lock> {
        match side {
            Side::Console => self.console.as_ref(),
            Side::Remote => self.remote.as_ref(),
        }
    }

    pub fn on_mut(&mut self, side: Side) -> Option<&mut Lock> {
        self.slot(side).as_mut()
    }

    fn slot(&mut self, side: Side) -> &mut Option<Lock> {
        match side {
            Side::Console => &mut self.console,
            Side::Remote => &mut self.remote,
        }
    }

    /// The lock whose cover is on `output`.
    pub fn covering(&self, output: &Output) -> Option<&Lock> {
        let l = self.on(crate::console::side_of(output))?;
        l.outputs.iter().any(|lo| lo.output == *output).then_some(l)
    }
}

/// The commands the PAM conversations run, for a user, how long a password check may take, and the
/// first wait before a failed fingerprint conversation starts again; tests put scripts in their place.
pub struct Helpers {
    pub password: fn(&str, lockfinger::PasswordFor) -> Command,
    pub finger: fn(&str) -> Option<Command>,
    pub verify_timeout: Duration,
    pub finger_retry: Duration,
}

impl Default for Helpers {
    fn default() -> Helpers {
        Helpers {
            password: lockfinger::password_command,
            finger: lockfinger::finger_command,
            verify_timeout: Duration::from_secs(90),
            finger_retry: Duration::from_secs(2),
        }
    }
}

/// The longest wait before a failed fingerprint conversation starts again; each failure doubles it.
const FINGER_RETRY_MAX: Duration = Duration::from_secs(30);
/// Fingerprint conversations in a row that end without a PAM result before retrying stops.
const FINGER_LOST_MAX: u32 = 5;

/// The button rects of one output's dialog, in layout coordinates, for hit testing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub ok: Rect,
    pub cancel: Rect,
    pub switch_user: Option<Rect>,
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
    /// Starts the fingerprint conversation again after one ended without success.
    pub finger_retry: Option<Source>,
    /// The wait before the next fingerprint retry.
    pub finger_delay: Duration,
    /// Fingerprint conversations in a row that ended without a PAM result.
    pub finger_lost: u32,
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
    /// The dialog's dead key and Compose sequence, apart from the other side's.
    pub compose: Option<Compose>,
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
fn verify_timed_out(state: &mut State, side: Side) -> TimeoutAction {
    if let Some(p) = state.lock.on_mut(side).and_then(|l| l.pending.as_mut()) {
        if let Some(t) = p.timer.as_mut() {
            t.disarm();
        }
        p.helper.conversation.kill();
    }
    log::error!("lock: the password conversation did not answer in time; ended it");
    abort_verification(state, side, lock::UNAVAILABLE);
    TimeoutAction::Drop
}

/// Watch `conversation`'s stdout with `on_read`. None when the loop refuses the source; the child ends.
fn watch(
    state: &State,
    conversation: lockfinger::Conversation,
    on_read: impl Fn(&mut State) -> PostAction + 'static,
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

/// What the fingerprint conversation said since the last wake. Its PAM success unlocks; any other end
/// drops it and starts it again later, unless PAM said there is no reader or finger to verify, or
/// `FINGER_LOST_MAX` ends in a row came without a result.
fn on_finger(state: &mut State) -> PostAction {
    let Some(l) = state.lock.console.as_mut() else {
        return PostAction::Remove;
    };
    let Some(f) = l.finger.as_mut() else {
        return PostAction::Remove;
    };
    let (lines, eof) = f.conversation.read();
    let mut result = None;
    for line in &lines {
        if let lockfinger::Line::Result(rc) = line {
            log::info!("lock: the fingerprint conversation ended with PAM code {rc}");
            result = Some(*rc);
        }
    }
    let ended = eof || result.is_some();
    let accepted = lines.into_iter().any(|line| l.state.finger(line));
    if accepted || ended {
        // This callback's Remove takes the source down; the child is reaped here.
        if let Some(mut f) = l.finger.take() {
            f.source.disarm();
        }
    }
    if accepted {
        log::info!("lock: fingerprint accepted");
        unlock_side(state, Side::Console);
        return PostAction::Remove;
    }
    if ended {
        l.state.finger = None;
        l.finger_lost = if result.is_some() {
            0
        } else {
            l.finger_lost + 1
        };
        if l.finger_lost >= FINGER_LOST_MAX {
            log::warn!(
                "lock: the fingerprint conversation keeps ending without a result; not starting it again"
            );
        } else if result != Some(reprisepam::ffi::PAM_AUTHINFO_UNAVAIL) {
            arm_finger_retry(state);
        }
    }
    redraw(state);
    if ended {
        PostAction::Remove
    } else {
        PostAction::Continue
    }
}

/// Start the fingerprint conversation again after the lock's `finger_delay`, which then doubles up to
/// `FINGER_RETRY_MAX`. The start is skipped while a locker holds the lock.
fn arm_finger_retry(state: &mut State) {
    let Some(l) = state.lock.console.as_mut() else {
        return;
    };
    let delay = l.finger_delay;
    l.finger_delay = (delay * 2).min(FINGER_RETRY_MAX);
    let armed = state
        .handle
        .insert_source(Timer::from_duration(delay), |_, _, state| {
            if let Some(mut t) = state
                .lock
                .console
                .as_mut()
                .and_then(|l| l.finger_retry.take())
            {
                t.disarm();
            }
            start_dialog_finger(state);
            TimeoutAction::Drop
        });
    l.finger_retry = match armed {
        Ok(token) => Some(Source {
            token: Some(token),
            handle: state.handle.clone(),
        }),
        Err(e) => {
            log::warn!("lock: cannot time the fingerprint retry: {}", e.error);
            None
        }
    };
}

/// Never leave the dialog stuck on "Verifying...": end the check, show `message` and redraw.
fn abort_verification(state: &mut State, side: Side, message: &str) {
    if let Some(l) = state.lock.on_mut(side) {
        l.pending = None;
        l.state.fail_with(message);
    }
    redraw(state);
}

/// What `side`'s password conversation said: a prompt relabels the field, a verdict unlocks or says
/// why not, and an end without one reads as "could not be checked", never as accepted.
fn on_result(state: &mut State, side: Side) -> PostAction {
    let Some(l) = state.lock.on_mut(side) else {
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
            let notices = p.answer.take_notices();
            l.state.ask(q);
            if !notices.is_empty() {
                l.state.message = Some(notices.join("\n"));
            }
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
            if side == Side::Remote {
                state.lock.remote_backoff.succeeded();
            }
            unlock_side(state, side);
        }
        Some(Err(notices)) => abort_verification(state, side, &lock::failure_text(&notices)),
        None => {
            log::error!("lock: the password conversation ended without a result");
            abort_verification(state, side, lock::UNAVAILABLE);
        }
    }
    PostAction::Remove
}

/// Lock the session, then start `[power] locker` if one is set. No-op when already locked, and in
/// the greeter, which has no session to lock.
pub fn lock(state: &mut State) {
    if state.host == crate::compositor::Host::Greeter {
        return;
    }
    crate::crash::set_locked(true);
    if state.console_locked() {
        return;
    }
    engage(state);
    crate::sessionlock::start_locker(state);
    start_dialog_finger(state);
}

/// The session left the seat or came back. Away, the fingerprint conversation and its retry end so
/// the reader is free for whoever has the seat; back, the dialog starts one again.
pub fn seat_away(state: &mut State, away: bool) {
    state.lock.seat_away = away;
    if !away {
        start_dialog_finger(state);
        return;
    }
    if let Some(l) = state.lock.console.as_mut() {
        l.finger = None;
        l.finger_retry = None;
        l.state.finger = None;
    }
    redraw(state);
}

/// Lock the desktop: the remote side in lock mode, else the session.
pub fn lock_desktop(state: &mut State) {
    if crate::console::remote_side(state) {
        lock_remote(state);
    } else {
        lock(state);
    }
}

/// Lock what `origin` answers to (`State::locked_for`).
pub fn lock_for(state: &mut State, origin: Origin) {
    match origin {
        Origin::Console => lock(state),
        Origin::Desktop => lock_desktop(state),
    }
}

/// Lock the session and, in lock mode, the remote side.
pub fn lock_all(state: &mut State) {
    lock(state);
    lock_remote(state);
}

/// Seal the desktop and cover the Remote outputs with a dialog taking the remote's input; no-op outside
/// lock mode or when locked. No fingerprint conversation: the reader is the computer's.
pub fn lock_remote(state: &mut State) {
    if !crate::console::remote_side(state) || state.lock.remote.is_some() {
        return;
    }
    crate::idle::end_dim(state);
    seal(state, |s| raise(s, Side::Remote));
    cover_side(state, Side::Remote);
    state.set_cursor("default");
    crate::idle::inhibition_changed(state);
    state.redraw_all();
    log::info!("lock mode: the remote side locked");
}

/// Start the fingerprint conversation if the built-in dialog shows and none runs. Never under a locker,
/// which may claim the reader through its own pam_fprintd, and fprintd lets one claim it; nor off the seat.
pub fn start_dialog_finger(state: &mut State) {
    if state.lock.seat_away {
        return;
    }
    let Some(l) = state.lock.console.as_ref() else {
        return;
    };
    if !l.dialog_shown() || l.finger.is_some() || l.user.is_empty() {
        return;
    }
    let user = l.user.clone();
    let finger = start_finger(state, &user);
    if let Some(l) = state.lock.console.as_mut() {
        l.finger = finger;
    }
}

/// Engage the lock unless locked or in the greeter: cover every Console output, clear client focus,
/// route input to the dialog. A locker's own lock comes here, so it starts no other locker.
pub fn engage(state: &mut State) {
    if state.host == crate::compositor::Host::Greeter {
        return;
    }
    crate::crash::set_locked(true);
    if state.console_locked() {
        return;
    }
    crate::state::save_screen_locked(true);
    crate::idle::end_dim(state);
    seal(state, |s| raise(s, Side::Console));
    cover_side(state, Side::Console);
    state.set_cursor("default");
    crate::power::hint(state, crate::power::Hint::Locked(true));
    crate::idle::inhibition_changed(state);
    state.redraw_all();
    log::info!("session locked");
}

/// Close what the desktop has open and end its gestures, run `raise`, then take every grab and
/// focus from clients. Lock mode's end seals again with no `raise`.
pub fn seal(state: &mut State, raise: impl FnOnce(&mut State)) {
    state.close_menu();
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
    state.hide_key_sheet();
    state.close_start_panel();
    // The emoji picker and clipboard history would type or paste into whatever has the keyboard after
    // unlock; the region overlay would take the pointer from the lock dialog.
    state.close_panels();
    // A client popup grab the drag holds would be set again at its end, under the lock.
    state.dnd.dismiss_held_popup();
    // Before the focus clearing below: the drag's end puts the pointer back on a client, which it takes away.
    state.cancel_drag();
    cancel_interaction(state);
    state.release_all_buttons(InputTime::now());
    state.cancel_client_gesture();
    state.caption_clicks.reset();
    raise(state);
    crate::saverview::locked(state);
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
}

/// Put `side`'s lock up, remembering what had the keyboard for the unlock.
fn raise(state: &mut State, side: Side) {
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
    let compose = state.compose.as_ref().map(Compose::over);
    *state.lock.slot(side) = Some(Lock {
        state: LockState {
            caps_lock: caps_lock(state, side),
            layout: layout(state),
            switch_user: crate::loginlink::offered(state, side),
            ..LockState::new()
        },
        user,
        outputs: Vec::new(),
        pending: None,
        finger: None,
        finger_retry: None,
        finger_delay: state.lock.helpers.finger_retry,
        finger_lost: 0,
        prev_focus,
        prev_focused_layer,
        prev_desktop,
        client: None,
        awaiting: false,
        compose,
    });
}

fn cancel_interaction(state: &mut State) {
    match state.interaction {
        Interaction::Drag { .. } => state.drag_view_cancel(),
        Interaction::Marquee => state.end_marquee(),
        Interaction::WidgetDrag(held) => state.widget_drag_cancel(held),
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

/// What had the keyboard when a lock engaged: the window, the layer surface, whether the desktop.
type Refocus = (Option<WindowId>, Option<WlSurface>, bool);

/// Take `side`'s lock's scene nodes, sources and child down without touching focus.
fn teardown(state: &mut State, side: Side) -> Option<Refocus> {
    let mut l = state.lock.slot(side).take()?;
    l.pending = None;
    // The fingerprint child is interrupted and frees the reader as it ends. A new lock's conversation
    // that finds the reader still claimed fails, which `on_finger` logs.
    l.finger = None;
    // The compositor ends the lock: a locker still holding it hears `finished`.
    if let Some(c) = l.client.take() {
        crate::sessionlock::release(state, c, true);
    }
    if side == Side::Console {
        crate::sessionlock::stop_timers(state);
    }
    for lo in &l.outputs {
        state.scene.destroy(lo.tree);
        state.water.drop_pane(&lo.output.name(), Pane::Lock);
    }
    Some((l.prev_focus, l.prev_focused_layer, l.prev_desktop))
}

/// Unlock from logind's `Unlock` or a locker: the session and, in lock mode, the remote side.
pub fn unlock(state: &mut State) {
    unlock_side(state, Side::Console);
    unlock_side(state, Side::Remote);
}

/// Lock mode ended: the remote side's lock goes with its outputs, handing nothing focus.
pub fn drop_remote(state: &mut State) {
    if teardown(state, Side::Remote).is_some() {
        crate::idle::inhibition_changed(state);
        log::info!("lock mode: the remote side's lock ends with it");
    }
}

/// Unlock `side`, from its correct password or a whole unlock, and hand focus back unless the
/// desktop is still locked. The console's unlock ends lock mode and keeps the remote side's focus.
fn unlock_side(state: &mut State, side: Side) {
    let Some(mut prev) = teardown(state, side) else {
        return;
    };
    if side == Side::Console {
        crate::power::unlocked(state);
        if crate::console::remote_side(state) {
            let remote = remote_focus(state);
            crate::console::console_unlocked(state);
            let shown = |w: WindowId| state.win(w).is_some_and(|x| x.mapped && !x.hidden());
            match remote {
                Some(RemoteFocus::Window(w)) if shown(w) && state.window_output(w).is_some() => {
                    prev = (Some(w), None, false);
                }
                Some(RemoteFocus::Desktop) => prev = (None, None, true),
                _ => {}
            }
        }
    }
    crate::idle::inhibition_changed(state);
    if !state.desktop_locked() {
        refocus(state, prev);
    }
    state.redraw_all();
    match side {
        Side::Console => {
            crate::state::save_screen_locked(false);
            crate::crash::set_locked(false);
            log::info!("session unlocked");
        }
        Side::Remote => log::info!("lock mode: the remote side unlocked"),
    }
}

enum RemoteFocus {
    Window(WindowId),
    Desktop,
}

/// What the remote side types into: what its lock took the keyboard from, else the window with
/// the keyboard, else the desktop once it clicked there. None when it picked nothing.
fn remote_focus(state: &State) -> Option<RemoteFocus> {
    if let Some(l) = state.lock.remote.as_ref() {
        return match l.prev_focus {
            Some(w) => Some(RemoteFocus::Window(w)),
            None => l.prev_desktop.then_some(RemoteFocus::Desktop),
        };
    }
    match state.seat.get_keyboard()?.current_focus() {
        Some(s) => state.window_for_surface(&s).map(RemoteFocus::Window),
        // `focused` stays the window from before lock mode until the remote side clicks the desktop.
        None => state.focused.is_none().then_some(RemoteFocus::Desktop),
    }
}

/// Focus a layer surface that claimed exclusive keyboard interactivity while locked, else what held
/// the keyboard when the lock engaged, else the top window.
fn refocus(state: &mut State, (prev, prev_layer, prev_desktop): Refocus) {
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
}

impl State {
    /// The lock is engaged: it covers the computer's own outputs and holds their input.
    pub fn console_locked(&self) -> bool {
        self.lock.console.is_some()
    }

    /// The console's lock holds the input being handled: the console is locked, or lock mode
    /// keeps the computer's own input for the console's lock (`console::physical`).
    pub fn console_holds_input(&self) -> bool {
        self.console_locked() || crate::console::physical(self)
    }

    /// The desktop's lock: the console's outside lock mode, the remote side's in it.
    pub fn desktop_locked(&self) -> bool {
        match crate::console::remote_side(self) {
            true => self.lock.remote.is_some(),
            false => self.console_locked(),
        }
    }

    /// The lock of `output`'s side.
    pub fn locked_on(&self, output: &Output) -> bool {
        match crate::console::side_of(output) {
            Side::Console => self.console_locked(),
            Side::Remote => self.desktop_locked(),
        }
    }

    /// The lock of the side whose output holds `w`'s frame centre; the console's off every output.
    pub fn window_locked(&self, w: WindowId) -> bool {
        (self.window_output(w)).map_or(self.console_locked(), |o| self.locked_on(o))
    }

    /// The output holding `w`'s frame centre.
    pub fn window_output(&self, w: WindowId) -> Option<&Output> {
        let r = self.win(w)?.frame_rect();
        let centre = (f64::from(r.x + r.width / 2), f64::from(r.y + r.height / 2));
        (self.scene.outputs().iter())
            .find(|o| reprisescene::output_rect(o).is_some_and(|b| b.to_f64().contains(centre)))
    }

    /// The lock that `origin`'s request answers to.
    pub fn locked_for(&self, origin: Origin) -> bool {
        match origin {
            Origin::Console => self.console_locked(),
            Origin::Desktop => self.desktop_locked(),
        }
    }
}

/// Where a power or session-end request comes from: the computer's own keys, lid or battery, or
/// the desktop's menus and dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Console,
    Desktop,
}

fn cover_side(state: &mut State, side: Side) {
    let outputs: Vec<Output> = (state.scene.outputs().iter())
        .filter(|o| crate::console::side_of(o) == side)
        .cloned()
        .collect();
    for o in &outputs {
        cover_output(state, o);
    }
}

/// Cover `output` with its side's lock, or refit the cover: an opaque rect, the wallpaper and a
/// centred dialog. An output with no layout box loses its cover.
fn cover_output(state: &mut State, output: &Output) {
    let side = crate::console::side_of(output);
    if state.lock.on(side).is_none() {
        return;
    }
    #[cfg(test)]
    if tests::COVER_PANICS.get() {
        panic!("cover_output");
    }
    let b = state.layout_box(Some(output));
    if b.width <= 0 {
        forget_output(state, side, output);
        return;
    }
    let color = state.config.theme.colors.background.as_f32();
    let existing = (state.lock.on(side))
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
            if let Some(l) = state.lock.on_mut(side) {
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
    redraw_output(state, side, output);
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
    let Some((tree, wall)) = (state.lock.covering(output))
        .and_then(|l| l.outputs.iter().find(|lo| lo.output == *output))
        .map(|lo| (lo.tree, lo.wall))
    else {
        return;
    };
    paint_wall(state, output, tree, wall);
    redraw_output(state, crate::console::side_of(output), output);
}

fn forget_output(state: &mut State, side: Side, output: &Output) {
    let Some(l) = state.lock.on_mut(side) else {
        return;
    };
    let Some(i) = l.outputs.iter().position(|lo| lo.output == *output) else {
        return;
    };
    let lo = l.outputs.remove(i);
    state.water.drop_pane(&output.name(), Pane::Lock);
    state.scene.destroy(lo.tree);
}

/// Re-cover each locked side's outputs after a layout, theme or wallpaper change.
pub fn refit(state: &mut State) {
    if state.lock.console.is_none() && state.lock.remote.is_none() {
        return;
    }
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for side in [Side::Console, Side::Remote] {
        let gone: Vec<Output> = (state.lock.on(side))
            .map(|l| {
                (l.outputs.iter())
                    .map(|lo| lo.output.clone())
                    .filter(|o| !outputs.contains(o))
                    .collect()
            })
            .unwrap_or_default();
        for o in &gone {
            forget_output(state, side, o);
        }
    }
    for o in &outputs {
        cover_output(state, o);
    }
    crate::sessionlock::refit(state);
    state.redraw_all();
}

/// Caps Lock of the keys `side`'s dialog reads: the console's own in lock mode, else the seat's.
fn caps_lock(state: &State, side: Side) -> bool {
    if side == Side::Console && crate::console::remote_side(state) {
        return state.console.keys.caps_lock;
    }
    state
        .seat
        .get_keyboard()
        .is_some_and(|k| k.modifier_state().caps_lock)
}

/// The badge's text: the active layout's short name, None while nothing switches (the taskbar cell's rule).
fn layout(state: &State) -> Option<String> {
    let layouts = &state.key_input.layouts;
    (layouts.switchable())
        .then(|| layouts.active().map(|d| d.short.clone()))
        .flatten()
}

/// Redraw both sides' dialogs with the Caps Lock, layout and Switch User they read now.
pub fn redraw(state: &mut State) {
    if state.lock.console.is_none() && state.lock.remote.is_none() {
        return;
    }
    for side in [Side::Console, Side::Remote] {
        let caps = caps_lock(state, side);
        let layout = layout(state);
        let switch_user = crate::loginlink::offered(state, side);
        let Some(l) = state.lock.on_mut(side) else {
            continue;
        };
        l.state.caps_lock = caps;
        l.state.layout = layout;
        l.state.switch_user = switch_user;
        let outputs: Vec<Output> = l.outputs.iter().map(|lo| lo.output.clone()).collect();
        for o in &outputs {
            redraw_output(state, side, o);
        }
    }
    state.redraw_all();
}

const LOCK_ICON: &str = "system-lock-screen";

fn redraw_output(state: &mut State, side: Side, output: &Output) {
    let scale = state.draw_scale_at(Some(output));
    let px = state.config.theme.scaled(scale).icon_size();
    let icon = state.icons.get_shared(LOCK_ICON, px);
    let b = state.layout_box(Some(output));
    let theme = state.config.theme.scaled(scale);
    let Some(l) = state.lock.on(side) else {
        return;
    };
    let logical = lock::dialog_layout(&state.config.theme.scaled(1), l.state.switch_user);
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
        switch_user: logical.switch_user.map(at),
    };
    crate::output::upload_scaled(&mut state.scene, dialog, &canvas, scale);
    state.scene.set_position(dialog, x - b.x, y - b.y);
    state.scene.set_enabled(dialog, shown);
    if let Some(lo) =
        (state.lock.on_mut(side)).and_then(|l| l.outputs.iter_mut().find(|lo| lo.output == *output))
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
        if b.switch_user.is_some_and(|r| r.contains(x, y)) {
            return Some(LockButton::SwitchUser);
        }
    }
    None
}

/// End `side`'s check before `verify_timeout`, or None when the loop refuses the timer.
fn verify_timer(state: &State, side: Side) -> Option<Source> {
    state
        .handle
        .insert_source(
            Timer::from_duration(state.lock.helpers.verify_timeout),
            move |_, _, state| verify_timed_out(state, side),
        )
        .map_err(|e| log::warn!("lock: cannot time the password check: {}", e.error))
        .ok()
        .map(|token| Source {
            token: Some(token),
            handle: state.handle.clone(),
        })
}

/// Hand `side`'s field to PAM: the password starts a conversation; under a further prompt the field
/// is that prompt's answer to the waiting one.
fn submit(state: &mut State, side: Side) {
    let password_command = state.lock.helpers.password;
    let Some(l) = state.lock.on_mut(side) else {
        return;
    };
    l.state.verifying = true;
    l.state.message = None;
    let password = l.state.take_input();
    if let Some(p) = l.pending.as_mut() {
        if let Err(e) = p.helper.conversation.answer(password.as_bytes()) {
            log::error!("lock: cannot answer the password conversation's prompt: {e}");
            abort_verification(state, side, lock::UNAVAILABLE);
            return;
        }
        drop(password);
        let timer = verify_timer(state, side);
        if let Some(p) = state.lock.on_mut(side).and_then(|l| l.pending.as_mut()) {
            p.timer = timer;
        }
        redraw(state);
        return;
    }
    if side == Side::Remote {
        let now = Instant::now();
        let wait = state.lock.remote_backoff.wait(now);
        if !wait.is_zero() {
            drop(password);
            abort_verification(state, side, &lock::backoff_text(wait));
            return;
        }
        state.lock.remote_backoff.failed(now);
    }
    let Some(l) = state.lock.on_mut(side) else {
        return;
    };
    let command = password_command(&l.user, password_for(side));
    let helper = match lockfinger::Conversation::start_password(command, password.as_bytes()) {
        Ok(c) => watch(state, c, move |s| on_result(s, side)),
        Err(e) => {
            log::error!("lock: cannot start the password conversation: {e}");
            None
        }
    };
    drop(password);
    let timer = helper.as_ref().and_then(|_| verify_timer(state, side));
    if let Some(l) = state.lock.on_mut(side) {
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

/// The remote side checks under the Remote Desktop PAM service, which has no pam_faillock.
fn password_for(side: Side) -> lockfinger::PasswordFor {
    match side {
        Side::Console => lockfinger::PasswordFor::Unlock,
        Side::Remote => lockfinger::PasswordFor::RemoteDesktop,
    }
}

/// Escape or Cancel under a further prompt drops it: its conversation ends and the field takes a
/// password again.
fn end_abandoned_prompt(l: &mut Lock) {
    if l.state.prompt.is_none() && !l.state.verifying {
        l.pending = None;
    }
}

/// The side whose lock the seat's input reaches: the remote side's in lock mode.
fn desk_side(state: &State) -> Side {
    match crate::console::remote_side(state) {
        true => Side::Remote,
        false => Side::Console,
    }
}

/// A seat key while the desktop is locked: true sends it on to a locker holding the lock; otherwise
/// the dialog takes presses and every key is consumed.
pub fn key(state: &mut State, pressed: bool, handle: &KeysymHandle<'_>) -> bool {
    let side = desk_side(state);
    if side == Side::Console && crate::sessionlock::holds_input(state) {
        crate::sessionlock::check_alive(state);
        if crate::sessionlock::holds_input(state) {
            return true;
        }
    }
    redraw_on_caps(state, side);
    if !pressed || !dialog_shows(state, side) {
        return false;
    }
    let typed = {
        let Ok(xkb) = handle.xkb().lock() else {
            return false;
        };
        // SAFETY: the state reference does not outlive the lock guard.
        decode(state, side, unsafe { xkb.state() }, handle.raw_code())
    };
    if let Some(t) = typed {
        type_key(state, side, t);
    }
    false
}

/// A key for the console's dialog decoded by `xkb`, the state after it: the console's own keys in
/// lock mode (`console::keys`).
pub fn key_with(state: &mut State, pressed: bool, xkb: &xkb::State, code: Keycode) {
    let side = Side::Console;
    if pressed
        && dialog_shows(state, side)
        && let Some(t) = decode(state, side, xkb, code)
    {
        type_key(state, side, t);
    }
}

fn dialog_shows(state: &State, side: Side) -> bool {
    state.lock.on(side).is_some_and(Lock::dialog_shown)
}

fn redraw_on_caps(state: &mut State, side: Side) {
    if dialog_shows(state, side)
        && (state.lock.on(side)).is_some_and(|l| l.state.caps_lock != caps_lock(state, side))
    {
        redraw(state);
    }
}

/// What a pressed key types into the field.
enum Typed {
    Enter,
    Backspace,
    Escape,
    Text(KeyText),
}

/// The key `code` under `xkb`, through `side`'s compose sequence; None while it types nothing.
fn decode(state: &mut State, side: Side, xkb: &xkb::State, code: Keycode) -> Option<Typed> {
    // Dead keys and the Compose key: a sequence types nothing until it ends, then its result.
    let fed = match state.lock.on_mut(side).and_then(|l| l.compose.as_mut()) {
        Some(c) => c.feed(xkb.key_get_one_sym(code).raw()),
        None => Fed::Plain,
    };
    let typed = match fed {
        Fed::Swallowed => return None,
        Fed::Composed(t) => Typed::Text(t),
        Fed::Plain => {
            let special = (xkb.key_get_syms(code).iter()).find_map(|sym| match sym.raw() {
                keys::KEY_Return | keys::KEY_KP_Enter => Some(Typed::Enter),
                keys::KEY_BackSpace => Some(Typed::Backspace),
                keys::KEY_Escape => Some(Typed::Escape),
                _ => None,
            });
            match special {
                Some(t) => t,
                None => Typed::Text(KeyText::of_key(xkb, code)?),
            }
        }
    };
    match &typed {
        Typed::Text(t) if t.as_str().chars().any(char::is_control) => None,
        _ => Some(typed),
    }
}

fn type_key(state: &mut State, side: Side, t: Typed) {
    let key = match &t {
        Typed::Enter => LockKey::Enter,
        Typed::Backspace => LockKey::Backspace,
        Typed::Escape => LockKey::Escape,
        Typed::Text(t) => LockKey::Text(t.as_str()),
    };
    edit(state, side, key);
}

/// Seat text typed without a key: into the desktop's dialog field, or to the locker holding the lock.
pub fn type_text(state: &mut State, text: &str) {
    if text.chars().any(char::is_control) {
        return;
    }
    let side = desk_side(state);
    if side == Side::Console {
        crate::sessionlock::check_alive(state);
        if crate::sessionlock::holds_input(state) {
            state.type_text(text);
            return;
        }
    }
    if dialog_shows(state, side) {
        edit(state, side, LockKey::Text(text));
    }
}

/// Apply `key` to `side`'s dialog field: submit it, or redraw.
fn edit(state: &mut State, side: Side, key: LockKey<'_>) {
    let should_submit = match state.lock.on_mut(side) {
        Some(l) => {
            let submit = l.state.key(key);
            end_abandoned_prompt(l);
            submit
        }
        None => return,
    };
    if should_submit {
        submit(state, side);
    } else {
        redraw(state);
    }
}

/// A held seat Backspace repeats in the desktop's dialog field; not under a locker.
pub fn repeat_backspace(state: &mut State) {
    let side = desk_side(state);
    if !crate::sessionlock::holds_input(state) && dialog_shows(state, side) {
        edit(state, side, LockKey::Backspace);
    }
}

/// A press on `side`'s lock: a locker's surfaces take it through the seat; else a left press on OK
/// submits, on Cancel clears, on Switch User asks for the login screen, beside the dialog splashes.
pub fn button(state: &mut State, side: Side, button: u32, pos: Point<f64, Logical>) {
    crate::sessionlock::check_alive(state);
    if button != BTN_LEFT || crate::sessionlock::holds_input(state) {
        return;
    }
    state.water_lock_click(pos);
    let should_submit = {
        let Some(l) = state.lock.on_mut(side) else {
            return;
        };
        let Some(b) = hit(l, pos.x, pos.y) else {
            return;
        };
        if b == LockButton::SwitchUser {
            if side == Side::Console {
                crate::loginlink::switch_user(state);
            }
            return;
        }
        let submit = l.state.button(b);
        end_abandoned_prompt(l);
        submit
    };
    if should_submit {
        submit(state, side);
    } else {
        redraw(state);
    }
}

/// A seat press while the desktop is locked.
pub fn desk_button(state: &mut State, button: u32, pos: Point<f64, Logical>) {
    self::button(state, desk_side(state), button, pos);
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

    thread_local! {
        pub(super) static COVER_PANICS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    #[test]
    fn a_panic_while_the_lock_covers_the_outputs_restarts_locked() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers.finger = |_| None;
        COVER_PANICS.set(true);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lock(&mut h.state)));
        COVER_PANICS.set(false);
        assert!(r.is_err(), "the cover panics");
        let argv = crate::crash::restart_argv(
            [std::ffi::OsString::from("reprise")],
            1,
            crate::crash::locked(),
        );
        assert!(
            argv.contains(&"--locked".into()),
            "restart argv {argv:?} carries --locked"
        );
    }

    #[test]
    fn the_restart_flag_is_set_by_a_lock_and_cleared_by_its_unlock() {
        let mut h = locked(|_, _| sh("true"));
        assert!(crate::crash::locked());
        unlock(&mut h.state);
        assert!(!crate::crash::locked());
    }

    fn buttons(ok: Rect, cancel: Rect) -> Buttons {
        Buttons {
            ok,
            cancel,
            switch_user: None,
        }
    }

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    /// A harness locked behind the dialog, no fingerprint conversation, `password` as the password's.
    fn locked(password: fn(&str, lockfinger::PasswordFor) -> Command) -> crate::harness::Harness {
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
        let l = h.state.lock.console.as_mut().expect("locked");
        l.state.input.push_str(password);
        submit(&mut h.state, Side::Console);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (h.state.lock.console.as_ref()).is_some_and(|l| l.state.verifying) {
            assert!(std::time::Instant::now() < end, "the check ends");
            h.run_for(5);
        }
    }

    fn message(h: &crate::harness::Harness) -> Option<String> {
        h.state.lock.console.as_ref()?.state.message.clone()
    }

    #[test]
    fn the_password_conversation_unlocks_on_its_success_only() {
        let mut h = locked(|_, _| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#));
        check(&mut h, "hunter1");
        assert!(h.state.console_locked());
        assert_eq!(message(&h).as_deref(), Some(lock::INCORRECT));
        check(&mut h, "hunter2");
        assert!(!h.state.console_locked());
    }

    #[test]
    fn text_typed_without_a_key_goes_into_the_dialogs_field() {
        let mut h = locked(|_, _| sh("read p; echo '= 7'"));
        type_text(&mut h.state, "pé");
        type_text(&mut h.state, "\r");
        let l = h.state.lock.console.as_ref().expect("locked");
        assert_eq!(l.state.input, "pé", "control text is dropped");
    }

    #[test]
    fn a_rejection_shows_the_conversations_text() {
        let mut h = locked(|_, _| sh("read p; echo 'E Account locked.'; echo '= 7'"));
        check(&mut h, "hunter2");
        assert!(h.state.console_locked());
        assert_eq!(message(&h).as_deref(), Some("Account locked."));
    }

    #[test]
    fn a_conversation_that_ends_without_a_result_reads_as_unavailable() {
        let mut h = locked(|_, _| sh("exit 0"));
        check(&mut h, "hunter2");
        assert!(h.state.console_locked());
        assert_eq!(message(&h).as_deref(), Some(lock::UNAVAILABLE));
    }

    /// Turn the loop until `done` holds (or 5 s pass).
    fn turn_until(h: &mut crate::harness::Harness, what: &str, done: impl Fn(&Lock) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !h.state.lock.console.as_ref().is_some_and(&done) {
            assert!(std::time::Instant::now() < end, "{what}");
            h.run_for(5);
        }
    }

    #[test]
    fn a_second_prompt_relabels_the_field_and_its_answer_unlocks() {
        let mut h = locked(|_, _| {
            sh(
                r#"read p; echo '?? Verification code:'; read c; [ "$p $c" = "hunter2 123456" ] && echo '= 0' || echo '= 7'"#,
            )
        });
        let l = h.state.lock.console.as_mut().expect("locked");
        l.state.input.push_str("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| l.state.prompt.is_some());
        let l = h.state.lock.console.as_ref().unwrap();
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
        assert!(!h.state.console_locked());
    }

    #[test]
    fn the_notices_before_a_second_prompt_show_with_it() {
        let mut h = locked(|_, _| {
            sh(
                "read p; echo 'I Code sent to your phone'; echo '?? Verification code:'; exec sleep 60",
            )
        });
        let l = h.state.lock.console.as_mut().expect("locked");
        l.state.input.push_str("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| l.state.prompt.is_some());
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.state.message.as_deref(), Some("Code sent to your phone"));
        unlock(&mut h.state);
    }

    #[test]
    fn escape_under_a_second_prompt_ends_its_conversation() {
        let mut h = locked(|_, _| sh("read p; echo '? One-time code:'; exec sleep 60"));
        let l = h.state.lock.console.as_mut().expect("locked");
        l.state.input.push_str("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| l.state.prompt.is_some());
        let l = h.state.lock.console.as_mut().unwrap();
        l.state.key(LockKey::Escape);
        end_abandoned_prompt(l);
        assert!(l.pending.is_none());
        assert_eq!(l.state.prompt, None);
        check(&mut h, "hunter2");
        assert!(
            h.state
                .lock
                .console
                .as_ref()
                .unwrap()
                .state
                .prompt
                .is_some(),
            "the next password starts a new conversation"
        );
    }

    #[test]
    fn caps_lock_shows_its_notice_while_on() {
        const KEY_CAPSLOCK: u32 = 58;
        let mut h = locked(|_, _| sh("exit 0"));
        let caps =
            |h: &crate::harness::Harness| h.state.lock.console.as_ref().unwrap().state.caps_lock;
        assert!(!caps(&h));
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(caps(&h), "the dialog redraws with the notice");
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(!caps(&h));
    }

    #[test]
    fn a_layout_switch_while_locked_redraws_the_badge() {
        let mut h = locked(|_, _| sh("exit 0"));
        let badge = |h: &crate::harness::Harness| {
            h.state.lock.console.as_ref().unwrap().state.layout.clone()
        };
        assert_eq!(badge(&h), None, "one layout shows no badge");
        h.state.config.keyboard.layout = "us,hu".into();
        h.state.apply_keyboard();
        let described = |short: &str| reprisefs::xkb::Described {
            short: short.into(),
            description: short.into(),
        };
        h.state.key_input.layouts.labels = vec![described("EN"), described("HU")];
        h.state.set_layout(1);
        assert_eq!(badge(&h).as_deref(), Some("HU"));
        h.state.set_layout(0);
        assert_eq!(badge(&h).as_deref(), Some("EN"));
    }

    #[test]
    fn a_check_that_never_answers_ends_as_unavailable_after_the_timeout() {
        let mut h = locked(|_, _| sh("exec sleep 60"));
        h.state.lock.helpers.verify_timeout = std::time::Duration::from_millis(100);
        check(&mut h, "hunter2");
        let l = h.state.lock.console.as_ref().expect("still locked");
        assert!(!l.state.verifying);
        assert!(l.pending.is_none());
        assert_eq!(l.state.message.as_deref(), Some(lock::UNAVAILABLE));
    }

    #[test]
    fn unlocking_does_not_wait_for_a_fingerprint_child_that_ignores_the_interrupt() {
        let _reaping = crate::mutex::lock(&crate::lockfinger::tests::REAPING);
        let mut h = locked(|_, _| sh("exit 0"));
        h.state.lock.helpers.finger = |_| Some(sh("trap '' INT; echo 'I ready'; exec sleep 60"));
        let user = h.state.lock.console.as_ref().unwrap().user.clone();
        let finger = start_finger(&h.state, &user).expect("a fingerprint conversation");
        let pid = finger.conversation.pid();
        h.state.lock.console.as_mut().unwrap().finger = Some(finger);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (h.state.lock.console.as_ref()).is_some_and(|l| l.state.finger.is_none()) {
            assert!(std::time::Instant::now() < end, "the child says ready");
            h.run_for(5);
        }
        let start = std::time::Instant::now();
        unlock(&mut h.state);
        assert!(!h.state.console_locked());
        assert!(
            !crate::lockfinger::tests::ended(pid),
            "unlock returns while the child runs"
        );
        let end = start + std::time::Duration::from_secs(5);
        while !crate::lockfinger::tests::ended(pid) {
            assert!(std::time::Instant::now() < end, "the child is killed");
            h.run_for(20);
        }
        assert!(start.elapsed() >= lockfinger::RELEASE_GRACE);
    }

    #[test]
    fn only_the_dialog_runs_the_fingerprint_conversation() {
        let mut h = locked(|_, _| sh("exit 0"));
        let finger =
            |h: &crate::harness::Harness| h.state.lock.console.as_ref().unwrap().finger.is_some();
        assert!(!finger(&h));
        unlock(&mut h.state);
        h.state.lock.helpers.finger = |_| Some(sh("echo 'I ready'; exec sleep 60"));
        let mut c = h.connect();
        let (_lock, _surface) = h.map_lock_surface(&mut c);
        assert!(h.state.console_locked());
        assert!(!finger(&h), "a locker's lock starts none");
        drop(c);
        h.run_for(50);
        assert!(h.state.lock.console.as_ref().unwrap().dialog_shown());
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
    fn a_failed_fingerprint_conversation_starts_again_with_a_growing_wait() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static STARTS: AtomicUsize = AtomicUsize::new(0);
        let mut h = locked(|_, _| sh("exit 0"));
        h.state.lock.helpers.finger = |_| {
            STARTS.fetch_add(1, Ordering::SeqCst);
            Some(sh("echo 'E Failed to match fingerprint'; exit 1"))
        };
        h.state.lock.console.as_mut().unwrap().finger_delay = Duration::from_millis(20);
        start_dialog_finger(&mut h.state);
        let end = Instant::now() + Duration::from_secs(5);
        while STARTS.load(Ordering::SeqCst) < 3 {
            assert!(Instant::now() < end, "the conversation starts again");
            h.run_for(5);
        }
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(
            l.state.message.as_deref(),
            Some("Failed to match fingerprint")
        );
        assert!(
            l.finger_delay >= Duration::from_millis(80),
            "each retry doubles the wait"
        );
        unlock(&mut h.state);
    }

    #[test]
    fn a_fingerprint_conversation_that_keeps_ending_without_a_result_stops_starting() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static STARTS: AtomicU32 = AtomicU32::new(0);
        let mut h = locked(|_, _| sh("exit 0"));
        h.state.lock.helpers.finger = |_| {
            STARTS.fetch_add(1, Ordering::SeqCst);
            Some(sh("exit 1"))
        };
        h.state.lock.console.as_mut().unwrap().finger_delay = Duration::from_millis(1);
        start_dialog_finger(&mut h.state);
        turn_until(&mut h, "the retries give up", |l| {
            l.finger_lost >= FINGER_LOST_MAX
        });
        h.run_for(200);
        assert_eq!(STARTS.load(Ordering::SeqCst), FINGER_LOST_MAX);
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(l.finger.is_none() && l.finger_retry.is_none());
    }

    #[test]
    fn a_fingerprint_conversation_without_a_reader_is_not_started_again() {
        let mut h = locked(|_, _| sh("exit 0"));
        h.state.lock.helpers.finger = |_| Some(sh("echo '= 9'"));
        start_dialog_finger(&mut h.state);
        turn_until(&mut h, "the conversation ends", |l| l.finger.is_none());
        assert!(
            h.state
                .lock
                .console
                .as_ref()
                .unwrap()
                .finger_retry
                .is_none()
        );
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
