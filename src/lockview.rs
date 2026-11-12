//! The lock screen on the scene, one per side in lock mode: the login screen's locked state hosted
//! through `logonview` in `layers.lock`, its input routing and its PAM children.

use std::os::fd::BorrowedFd;
use std::process::Command;
use std::time::{Duration, Instant};

use smithay::{
    backend::input::InputTime,
    input::keyboard::{Keycode, KeysymHandle, ModifiersState, xkb},
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

use reprisekit::interface::Interface;
use reprisekit::logon::{Logon, LogonAction, LogonUser, Purpose, UserNote};
use reprisescene::{BufferId, TreeId};
use zeroize::Zeroizing;

use crate::compose::Compose;
use crate::compositor::State;
use crate::console::Side;
use crate::input::{BTN_LEFT, Hit, Interaction};
use crate::lock;
use crate::lockfinger;
use crate::logonview::{self, Fit, Host, LogonView};
use crate::sessionlock::{ClientLock, SessionLock};
use crate::wallpaperview::LockWall;
use crate::waterview::{Pane, Picture};
use crate::window::WindowId;

/// The lock's one `State` field (`lock`): the session-lock global and, while locked, the locks.
pub struct LockScreen {
    pub session: SessionLock,
    /// The lock on the computer's own outputs, the only one outside lock mode.
    pub console: Option<Lock>,
    /// Lock mode's lock on the Remote outputs: the remote side's own screen and password check.
    pub remote: Option<Lock>,
    /// The remote side's failed checks; it outlives each remote lock.
    pub remote_backoff: lock::Backoff,
    pub helpers: Helpers,
    /// The session is paused (another VT or user has the seat); the fingerprint reader is not its.
    pub seat_away: bool,
    /// Where reprise-login's screen lists this user (`find_listed`), so the tiles agree.
    pub listed_at: usize,
    pub back: crate::lockback::LoginBack,
    /// Covers an unlock let go, fading out over the desktop.
    pub leaving: Vec<Leaving>,
}

/// A cover a right password or fingerprint let go: it takes no input and fades out.
pub struct Leaving {
    pub output: Output,
    pub tree: TreeId,
    /// When the lock let it go, and when its first frame of fading was drawn.
    pub at: Instant,
    pub since: Option<Instant>,
}

/// How long the covers take to fade out once the lock lets go, ms.
const UNLOCK_MS: u64 = 200;

impl LockScreen {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> LockScreen {
        LockScreen {
            session: SessionLock::new(dh),
            console: None,
            remote: None,
            remote_backoff: lock::Backoff::default(),
            helpers: Helpers::default(),
            seat_away: false,
            listed_at: 0,
            back: Default::default(),
            leaving: Vec::new(),
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
        l.view
            .covers
            .iter()
            .any(|c| c.output == *output)
            .then_some(l)
    }
}

/// The commands the PAM conversations run, for a user, how long a password check may take, and the
/// first wait before a failed fingerprint conversation starts again; tests put scripts in their place.
pub struct Helpers {
    pub password: fn(&str, lockfinger::PasswordFor) -> Command,
    pub finger: fn(&str) -> Option<Command>,
    pub verify_timeout: Duration,
    pub finger_retry: Duration,
    /// Whether the power menu offers Restart and Turn Off (`lockpower::alone`); runs off the loop.
    pub alone: fn() -> bool,
}

impl Default for Helpers {
    fn default() -> Helpers {
        Helpers {
            password: lockfinger::password_command,
            finger: lockfinger::finger_command,
            verify_timeout: Duration::from_secs(90),
            finger_retry: Duration::from_secs(2),
            alone: crate::lockpower::alone,
        }
    }
}

/// The longest wait before a failed fingerprint conversation starts again; each failure doubles it.
const FINGER_RETRY_MAX: Duration = Duration::from_secs(30);
/// Fingerprint conversations in a row that end without a PAM result before retrying stops.
const FINGER_LOST_MAX: u32 = 5;

pub struct Lock {
    /// The login screen in its locked state, with its covers and compose sequence, apart from
    /// the other side's.
    pub view: LogonView,
    pub user: String,
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
    /// The external locker drawing the lock screen, if one holds the lock (see `sessionlock`); the screen is hidden meanwhile.
    pub client: Option<ClientLock>,
    /// `[power] locker` was started for this lock and has yet to take over or time out; the screen is hidden and ignores input.
    pub awaiting: bool,
}

impl Lock {
    /// Whether the built-in screen is up: no locker holds the lock or is starting to.
    pub fn screen_shown(&self) -> bool {
        screen_shown(self.client.is_some(), self.awaiting)
    }
}

fn screen_shown(client: bool, awaiting: bool) -> bool {
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

/// The check took `verify_timeout`: kill its child and let the screen take a password again.
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
    let accepted = lines
        .into_iter()
        .any(|line| lock::finger(&mut l.view.model, line));
    if accepted || ended {
        // This callback's Remove takes the source down; the child is reaped here.
        if let Some(mut f) = l.finger.take() {
            f.source.disarm();
        }
    }
    if accepted {
        log::info!("lock: fingerprint accepted");
        unlock_side(state, Side::Console, true);
        return PostAction::Remove;
    }
    if ended {
        l.view.model.hint = None;
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
    update(state);
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
            start_screen_finger(state);
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

/// Never leave the screen stuck on "Verifying...": end the check, show `message` and redraw.
fn abort_verification(state: &mut State, side: Side, message: &str) {
    if let Some(l) = state.lock.on_mut(side) {
        l.pending = None;
        l.view.model.fail(message.into());
    }
    update(state);
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
            l.view.model.ask(q);
            if !notices.is_empty() {
                l.view.model.info(notices.join("\n"));
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
        Some(Ok(passed)) => {
            l.view.model.set_busy(false);
            if side == Side::Remote {
                state.lock.remote_backoff.succeeded();
            }
            unlock_side(state, side, true);
            if passed == lock::Passed::Expired {
                crate::notifyview::notice(state, lock::EXPIRED_SUMMARY, lock::EXPIRED_BODY);
            }
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
        // Lock mode may have begun: the screen drops its power button and menu.
        redraw(state);
        return;
    }
    engage(state);
    crate::sessionlock::start_locker(state);
    start_screen_finger(state);
}

/// The session left the seat or came back. Away, the fingerprint conversation and its retry end so
/// the reader is free for whoever has the seat; back, the screen starts one again.
pub fn seat_away(state: &mut State, away: bool) {
    state.lock.seat_away = away;
    if !away {
        start_screen_finger(state);
        return;
    }
    if let Some(l) = state.lock.console.as_mut() {
        l.finger = None;
        l.finger_retry = None;
        l.view.model.hint = None;
    }
    update(state);
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

/// Seal the desktop and cover the Remote outputs with a screen taking the remote's input; no-op outside
/// lock mode or when locked. No fingerprint conversation: the reader is the computer's.
pub fn lock_remote(state: &mut State) {
    if !crate::console::remote_side(state) || state.lock.remote.is_some() {
        return;
    }
    crate::idle::end_dim(state);
    seal(state, |s| raise(s, Side::Remote));
    refit_side(state, Side::Remote);
    state.set_cursor("default");
    crate::idle::inhibition_changed(state);
    state.redraw_all();
    log::info!("lock mode: the remote side locked");
}

/// Start the fingerprint conversation if the built-in screen shows and none runs. Never under a locker,
/// which may claim the reader through its own pam_fprintd, and fprintd lets one claim it; nor off the seat.
pub fn start_screen_finger(state: &mut State) {
    if state.lock.seat_away {
        return;
    }
    let Some(l) = state.lock.console.as_ref() else {
        return;
    };
    if !l.screen_shown() || l.finger.is_some() || l.user.is_empty() {
        return;
    }
    let user = l.user.clone();
    let finger = start_finger(state, &user);
    if let Some(l) = state.lock.console.as_mut() {
        l.finger = finger;
    }
}

/// Engage the lock unless locked or in the greeter: cover every Console output, clear client focus,
/// route input to the screen. A locker's own lock comes here, so it starts no other locker.
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
    let_go_now(state);
    seal(state, |s| raise(s, Side::Console));
    refit_side(state, Side::Console);
    // The covers are opaque from their first frame; only the screen on them comes in.
    if let Some(l) = state.lock.console.as_mut() {
        l.view.model.enter();
    }
    redraw(state);
    state.set_cursor("default");
    crate::power::hint(state, crate::power::Hint::Locked(true));
    crate::idle::inhibition_changed(state);
    state.redraw_all();
    log::info!("session locked");
}

/// Release what apps' input holds, close what the desktop has open and end its gestures, run
/// `raise`, then take every grab and focus from clients. Lock mode's end seals with no `raise`.
pub fn seal(state: &mut State, raise: impl FnOnce(&mut State)) {
    state.app_input_release_held();
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
    // unlock; the region overlay would take the pointer from the lock screen.
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
    let me = LogonUser {
        name: user.clone(),
        real_name: lock::real_name(),
        picture: state.start_pictures.largest(),
        note: None,
        stock: state.lock.listed_at,
    };
    let model = Logon::locked(
        state.config.interface,
        me,
        crate::loginlink::offered(state, side),
        crate::lockpower::offered(state, side),
    );
    let compose = state.compose.as_ref().map(Compose::over);
    state.read_user_pictures(vec![PICTURE]);
    *state.lock.slot(side) = Some(Lock {
        view: LogonView::new(model, compose),
        user,
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
    });
    crate::lockback::read(state);
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

/// Take `side`'s lock's scene nodes, sources and child down without touching focus; with `fade`,
/// its covers fade out where [`fades`] allows.
fn teardown(state: &mut State, side: Side, fade: bool) -> Option<Refocus> {
    let fade = fade && side == Side::Console && fades(state);
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
        crate::lockpower::close(state);
    }
    for c in &l.view.covers {
        match fade {
            true => let_go(state, c.output.clone(), c.tree),
            false => destroy_cover(state, &c.output, c.tree),
        }
    }
    Some((l.prev_focus, l.prev_focused_layer, l.prev_desktop))
}

fn destroy_cover(state: &mut State, output: &Output, tree: TreeId) {
    state.scene.destroy(tree);
    state.water.drop_pane(&output.name(), Pane::Lock);
}

/// Whether an unlock fades the covers out: the screen moves, and the session has the seat to
/// draw the fade on. A resume through reprise-login unlocks while away and drops them at once.
fn fades(state: &State) -> bool {
    state.config.interface != Interface::Classic
        && logonview::moves(state) == reprisekit::logon::Moves::Full
        && crate::backend::session_active(state)
}

/// Let `tree`, a cover on `output`, fade out from the next frame, the pointer passing through it.
/// A timer takes it down should no frame come (an output that is off).
fn let_go(state: &mut State, output: Output, tree: TreeId) {
    let leaves: Vec<_> = state.scene.children(tree).to_vec();
    for n in leaves {
        state.scene.set_input(n, reprisescene::Input::Passthrough);
    }
    let timer = state
        .handle
        .insert_source(Timer::from_duration(STALLED), |_, _, state| {
            let_go_stalled(state);
            TimeoutAction::Drop
        });
    if let Err(e) = timer {
        log::warn!("lock: cannot time the unlock fade: {}", e.error);
    }
    state.lock.leaving.push(Leaving {
        output,
        tree,
        at: Instant::now(),
        since: None,
    });
}

/// How long a cover let go may wait for its first frame of fading.
const STALLED: Duration = Duration::from_millis(UNLOCK_MS * 2);

/// Take down the covers whose fade stalled: no frame since `STALLED` after they were let go, or
/// none since their fade's end.
fn let_go_stalled(state: &mut State) {
    let now = Instant::now();
    for l in std::mem::take(&mut state.lock.leaving) {
        let stalled = match l.since {
            Some(since) => now.saturating_duration_since(since) >= Duration::from_millis(UNLOCK_MS),
            None => now.saturating_duration_since(l.at) >= STALLED,
        };
        match stalled {
            true => destroy_cover(state, &l.output, l.tree),
            false => state.lock.leaving.push(l),
        }
    }
}

/// Take down every cover still fading out.
fn let_go_now(state: &mut State) {
    for l in std::mem::take(&mut state.lock.leaving) {
        destroy_cover(state, &l.output, l.tree);
    }
}

/// Fade the covers leaving `output` to their opacity for this frame; one that faded out goes.
fn fade_out(state: &mut State, output: &Output) {
    let now = logonview::now();
    let mut more = false;
    for mut l in std::mem::take(&mut state.lock.leaving) {
        if l.output != *output {
            state.lock.leaving.push(l);
            continue;
        }
        let since = *l.since.get_or_insert(now);
        let t = now.saturating_duration_since(since).as_secs_f32() * 1000.0 / UNLOCK_MS as f32;
        if t >= 1.0 {
            destroy_cover(state, &l.output, l.tree);
            continue;
        }
        state.scene.set_opacity(l.tree, 1.0 - t);
        state.lock.leaving.push(l);
        more = true;
    }
    if more {
        crate::backend::schedule(state, output);
    }
}

/// Unlock from logind's `Unlock` or a locker: the session and, in lock mode, the remote side.
pub fn unlock(state: &mut State) {
    unlock_side(state, Side::Console, false);
    unlock_side(state, Side::Remote, false);
}

/// Lock mode ended: the remote side's lock goes with its outputs, handing nothing focus.
pub fn drop_remote(state: &mut State) {
    if teardown(state, Side::Remote, false).is_some() {
        crate::idle::inhibition_changed(state);
        log::info!("lock mode: the remote side's lock ends with it");
    }
}

/// Unlock `side`, from its correct password or a whole unlock, and hand focus back unless the
/// desktop is still locked. The console's unlock ends lock mode and keeps the remote side's focus.
/// With `fade` (a right password or fingerprint), the covers fade out after it.
fn unlock_side(state: &mut State, side: Side, fade: bool) {
    let Some(mut prev) = teardown(state, side, fade) else {
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

/// The user picture's read size in device px: the largest tile any interface draws, at scale 2.
const PICTURE: i32 = 256;

const CONSOLE: Host = Host {
    view: |s| Some((&mut s.lock.console.as_mut()?.view, &s.text)),
    act: |s, a| act(s, Side::Console, a),
    caps_lock: |s| caps_lock(s, Side::Console),
    moves: logonview::moves,
};

const REMOTE: Host = Host {
    view: |s| Some((&mut s.lock.remote.as_mut()?.view, &s.text)),
    act: |s, a| act(s, Side::Remote, a),
    caps_lock: |s| caps_lock(s, Side::Remote),
    // Its frames go through the remote side's encoder, where motion buys nothing.
    moves: |_| reprisekit::logon::Moves::Off,
};

pub(crate) fn host(side: Side) -> &'static Host {
    match side {
        Side::Console => &CONSOLE,
        Side::Remote => &REMOTE,
    }
}

/// The output `side`'s screen shows on: the computer's primary, the remote side's first
/// screen. The side's other outputs show the backdrop.
fn screen_output(state: &State, side: Side) -> Option<Output> {
    match side {
        Side::Console => state.console_primary(),
        Side::Remote => (crate::virtualout::remote_outputs(state).into_iter())
            .find(|o| state.layout_box(Some(o)).width > 0),
    }
}

/// Cover `side`'s outputs with its lock, or refit the covers: an opaque backdrop, under Glass
/// the wallpaper over it, and the screen on [`screen_output`]. An output with no layout box
/// loses its cover.
fn refit_side(state: &mut State, side: Side) {
    if state.lock.on(side).is_none() {
        return;
    }
    let outputs: Vec<Output> = (logonview::shown_outputs(state).into_iter())
        .filter(|o| crate::console::side_of(o) == side)
        .collect();
    let screen = screen_output(state, side);
    let login = state.lock.back.shows();
    let glass = state.config.interface == Interface::Glass;
    if !glass || login {
        unwall(state, side);
    }
    let color = (logonview::backdrop(state))
        .unwrap_or_else(|| state.config.theme.colors.background.as_f32());
    prepare(state, side);
    let fit = Fit {
        layer: state.layers.lock,
        outputs,
        screen_on: screen.as_ref(),
        color: Some(color),
        wall: glass || login,
    };
    let changed = logonview::refit(state, host(side), fit);
    for o in &changed.removed {
        state.water.drop_pane(&o.name(), Pane::Lock);
    }
    if !changed.added.is_empty() {
        crate::sessionlock::raise(state);
    }
    for (output, tree, wall) in walls(state, side) {
        match login {
            true => crate::lockback::paint(state, &output, wall),
            false => paint_wall(state, &output, tree, wall),
        }
    }
}

/// Each of `side`'s covers that holds a wall, with its tree.
fn walls(state: &State, side: Side) -> Vec<(Output, TreeId, BufferId)> {
    (state.lock.on(side).iter())
        .flat_map(|l| &l.view.covers)
        .filter_map(|c| Some((c.output.clone(), c.tree, c.wall?)))
        .collect()
}

/// Take the water off `side`'s walls before the covers lose them.
fn unwall(state: &mut State, side: Side) {
    for (output, _, wall) in walls(state, side) {
        state.water_lock_uncover(&output, wall);
    }
}

/// Fill `wall` with the wallpaper as `output` shows it, or empty it without one. With `[wallpaper]
/// water` on, the cover's `tree` gets the water node, which draws in place of `wall`.
fn paint_wall(state: &mut State, output: &Output, tree: TreeId, wall: BufferId) {
    let shown = state.lock_wallpaper(output);
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
    let side = crate::console::side_of(output);
    let found = walls(state, side).into_iter().find(|(o, _, _)| o == output);
    if let Some((output, tree, wall)) = found.filter(|_| !state.lock.back.shows()) {
        paint_wall(state, &output, tree, wall);
        state.redraw_all();
    }
}

/// `output` is about to render: the lock's screen there steps what plays on it, and covers an
/// unlock let go fade.
pub fn frame(state: &mut State, output: &Output) {
    if !state.lock.leaving.is_empty() {
        fade_out(state, output);
    }
    let side = crate::console::side_of(output);
    if state.lock.on(side).is_some() {
        logonview::frame(state, host(side), output);
    }
}

/// Re-cover each locked side's outputs after a layout, theme or wallpaper change.
pub fn refit(state: &mut State) {
    if state.lock.console.is_none() && state.lock.remote.is_none() {
        return;
    }
    for side in [Side::Console, Side::Remote] {
        refit_side(state, side);
    }
    crate::sessionlock::refit(state);
    state.redraw_all();
}

/// Caps Lock of the keys `side`'s screen reads: the console's own in lock mode, else the seat's.
fn caps_lock(state: &State, side: Side) -> bool {
    if side == Side::Console && crate::console::remote_side(state) {
        return state.console.keys.caps_lock;
    }
    logonview::seat_caps_lock(state)
}

/// The line under the user's name on `side`'s screen: the console's reads Connected remotely
/// while lock mode gives the desktop to a remote client.
fn note(state: &State, side: Side) -> UserNote {
    match side == Side::Console && crate::console::remote_side(state) {
        true => UserNote::Remote,
        false => UserNote::Locked,
    }
}

/// Bring `side`'s screen up to what it offers now (interface, Switch User, power, note), hidden
/// while a locker holds the lock or starts to; the power menu closes with the power it offered. A
/// new interface waits for a check in progress to end, as it rebuilds the screen.
fn prepare(state: &mut State, side: Side) {
    let switch_user = crate::loginlink::offered(state, side);
    let power = crate::lockpower::offered(state, side);
    let interface = state.config.interface;
    let over_picture = state.lock.back.shows();
    let note = note(state, side);
    let Some(l) = state.lock.on_mut(side) else {
        return;
    };
    if l.view.model.interface != interface && l.pending.is_none() {
        let user = l.view.model.users[0].clone();
        l.view.model = Logon::locked(interface, user, switch_user, power);
    }
    l.view.model.purpose = Purpose::Unlock { switch_user, power };
    l.view.model.users[0].note = Some(note);
    l.view.model.over_picture = over_picture;
    l.view.hidden = !l.screen_shown();
    if side == Side::Console && (!power || l.view.hidden) {
        crate::lockpower::close(state);
    }
}

/// Redraw both sides' screens with the Caps Lock, layout and Switch User they read now.
pub fn redraw(state: &mut State) {
    for side in [Side::Console, Side::Remote] {
        if state.lock.on(side).is_some() {
            prepare(state, side);
            logonview::redraw(state, host(side));
        }
    }
}

/// [`redraw`] after a change to the models alone: see [`logonview::update`].
pub fn update(state: &mut State) {
    for side in [Side::Console, Side::Remote] {
        if state.lock.on(side).is_some() {
            prepare(state, side);
            logonview::update(state, host(side));
        }
    }
}

/// The screen as a locker leaves it: nothing typed, no check or message, its user unchanged.
pub fn reset(l: &mut Lock) {
    let m = &l.view.model;
    let user = m.users[0].clone();
    l.view.model = Logon::locked(m.interface, user, m.offers_switch_user(), m.offers_power());
}

/// Under reprise-login, finds off the loop where its screen lists this user, so Blue's lock and
/// login screen give a user without a picture the same stock one.
pub fn find_listed(state: &mut State) {
    if state.login_link.is_none() {
        return;
    }
    let user = lock::current_user();
    state.spawn_job(move || {
        let at = lock::listed_at(repriselogin::users::Source::read(), &user);
        Box::new(move |state: &mut State| listed(state, at))
    });
}

fn listed(state: &mut State, at: usize) {
    state.lock.listed_at = at;
    for l in [state.lock.console.as_mut(), state.lock.remote.as_mut()]
        .into_iter()
        .flatten()
    {
        l.view.model.users[0].stock = at;
    }
    redraw(state);
}

/// The user picture landed: both sides' screens show it.
pub fn picture_read(state: &mut State) {
    let picture = state.start_pictures.largest();
    let mut changed = false;
    for l in [state.lock.console.as_mut(), state.lock.remote.as_mut()]
        .into_iter()
        .flatten()
    {
        if l.view.model.users[0].picture != picture {
            l.view.model.users[0].picture = picture.clone();
            changed = true;
        }
    }
    if changed {
        redraw(state);
    }
}

/// A power request from the lock was refused: the console's screen says why.
pub fn refused(state: &mut State, text: String) {
    if let Some(l) = state.lock.console.as_mut() {
        l.view.model.info(text);
        update(state);
    }
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
/// is that prompt's answer to the waiting one. An empty field submits nothing.
fn submit(state: &mut State, side: Side) {
    let password_command = state.lock.helpers.password;
    let Some(l) = state.lock.on_mut(side) else {
        return;
    };
    let m = &mut l.view.model;
    if m.password.text.is_empty() {
        return;
    }
    let password = Zeroizing::new(m.password.text.clone());
    m.sent();
    m.message = None;
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
        update(state);
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
            None => l.view.model.fail(lock::UNAVAILABLE.into()),
        }
    }
    update(state);
}

/// The remote side checks under the Remote Desktop PAM service, which has no pam_faillock.
fn password_for(side: Side) -> lockfinger::PasswordFor {
    match side {
        Side::Console => lockfinger::PasswordFor::Unlock,
        Side::Remote => lockfinger::PasswordFor::RemoteDesktop,
    }
}

/// The side whose lock the seat's input reaches: the remote side's in lock mode.
fn desk_side(state: &State) -> Side {
    match crate::console::remote_side(state) {
        true => Side::Remote,
        false => Side::Console,
    }
}

/// What the screen's parts do on `side`. Cancel and Escape only clear the field and end a waiting
/// prompt's conversation; Switch User leaves the session locked.
fn act(state: &mut State, side: Side, action: LogonAction) {
    match action {
        LogonAction::Submit { .. } => submit(state, side),
        LogonAction::Cancel => {
            if let Some(l) = state.lock.on_mut(side)
                && !l.view.model.busy
            {
                l.pending = None;
            }
        }
        LogonAction::SwitchUser if side == Side::Console => crate::loginlink::switch_user(state),
        LogonAction::SwitchUser => {}
        LogonAction::Power | LogonAction::PowerMenu => crate::lockpower::open(state, side),
        LogonAction::NextLayout => state.next_layout(),
    }
}

/// A seat key while the desktop is locked: true sends it on to a locker holding the lock; otherwise
/// the screen takes presses and every key is consumed.
pub fn key(state: &mut State, pressed: bool, handle: &KeysymHandle<'_>) -> bool {
    let side = desk_side(state);
    if side == Side::Console && crate::sessionlock::holds_input(state) {
        crate::sessionlock::check_alive(state);
        if crate::sessionlock::holds_input(state) {
            return true;
        }
    }
    if !screen_shows(state, side) {
        return false;
    }
    let typed = match pressed {
        true => {
            let Ok(xkb) = handle.xkb().lock() else {
                return false;
            };
            // SAFETY: the state reference does not outlive the lock guard.
            decode(state, side, unsafe { xkb.state() }, handle.raw_code())
        }
        false => None,
    };
    logonview::key(state, host(side), typed);
    false
}

/// A key for the console's screen decoded by `xkb`, the state after it: the console's own keys in
/// lock mode (`console::keys`).
pub fn key_with(state: &mut State, pressed: bool, xkb: &xkb::State, code: Keycode) {
    let side = Side::Console;
    if pressed
        && screen_shows(state, side)
        && let Some(t) = decode(state, side, xkb, code)
    {
        logonview::key(state, host(side), Some(t));
    }
}

pub(crate) fn screen_shows(state: &State, side: Side) -> bool {
    state.lock.on(side).is_some_and(Lock::screen_shown)
}

/// The key `code` under `xkb`, through `side`'s compose sequence; None while it types nothing.
fn decode(
    state: &mut State,
    side: Side,
    xkb: &xkb::State,
    code: Keycode,
) -> Option<logonview::Typed> {
    let mut mods = ModifiersState::default();
    mods.update_with(xkb);
    logonview::decode_with(state, host(side), &mods, xkb, code)
}

/// Seat text typed without a key: into the desktop's screen, or to the locker holding the lock.
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
    if screen_shows(state, side) {
        logonview::type_text(state, host(side), text);
    }
}

/// A held seat Backspace repeats in the desktop's screen; not under a locker.
pub fn repeat_backspace(state: &mut State) {
    let side = desk_side(state);
    if !crate::sessionlock::holds_input(state) && screen_shows(state, side) {
        logonview::repeat_backspace(state, host(side));
    }
}

/// A press on `side`'s lock: a locker's surfaces take it through the seat; with the power menu
/// open it picks a row or closes the menu; else a left press clicks the screen's part under it,
/// and beside the screen splashes.
pub fn button(state: &mut State, side: Side, button: u32, pos: Point<f64, Logical>) {
    crate::sessionlock::check_alive(state);
    if crate::sessionlock::holds_input(state) {
        return;
    }
    if crate::lockpower::menu_open(state) {
        match state.scene_hit(pos) {
            Hit::Menu { depth, row } => state.menu_row_press(depth, row, button),
            _ => state.close_menu(),
        }
        return;
    }
    if button != BTN_LEFT {
        return;
    }
    state.water_lock_click(pos);
    if screen_shows(state, side) {
        logonview::click(state, host(side), pos);
    }
}

/// A seat press while the desktop is locked.
pub fn desk_button(state: &mut State, button: u32, pos: Point<f64, Logical>) {
    self::button(state, desk_side(state), button, pos);
}

/// The pointer moved while locked: to the locker's surface under it, else over the power menu or
/// the screen, to no client.
pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) {
    if crate::sessionlock::motion(state, pos, time) {
        return;
    }
    if crate::lockpower::menu_open(state)
        && let Hit::Menu { depth, row } = state.scene_hit(pos)
    {
        state.pointer_motion(None, pos, time);
        state.set_cursor("default");
        state.menu_hover(depth, row);
        return;
    }
    let side = desk_side(state);
    if screen_shows(state, side) {
        logonview::motion(state, host(side), pos, time);
    } else {
        state.pointer_motion(None, pos, time);
        state.set_cursor("default");
    }
}

/// Whether a press at `pos` on `output`'s cover splashes the lock water: the screen shows and the
/// press is beside its parts.
pub fn splashes(l: &Lock, pos: Point<f64, Logical>) -> bool {
    l.screen_shown() && !l.view.covers_point(pos)
}

/// Reads and writes for tests.
#[cfg(test)]
impl Lock {
    pub fn input(&self) -> &str {
        &self.view.model.password.text
    }

    pub fn set_input(&mut self, text: &str) {
        self.view.model.password.wipe();
        self.view
            .model
            .password
            .key(reprisekit::textedit::EditKey::Insert(text.into()));
    }

    pub fn verifying(&self) -> bool {
        self.view.model.busy
    }

    pub fn message(&self) -> Option<String> {
        self.view.model.message.clone()
    }

    pub fn covered(&self) -> Vec<String> {
        self.view.covers.iter().map(|c| c.output.name()).collect()
    }

    /// The outputs whose cover holds the screen.
    pub fn screens(&self) -> Vec<String> {
        (self.view.covers.iter())
            .filter(|c| c.screen.is_some())
            .map(|c| c.output.name())
            .collect()
    }

    /// The middle of `part` on the screen, in layout coordinates.
    pub fn part(&self, part: reprisekit::logon::Part) -> Option<Point<f64, Logical>> {
        let r = self.view.layout.as_ref()?.rect(part)?;
        let (x, y) = (r.x + r.width / 2, r.y + r.height / 2);
        Some(Point::from((f64::from(x), f64::from(y))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::logon;

    #[test]
    fn a_panic_while_the_lock_covers_the_outputs_restarts_locked() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers.finger = |_| None;
        logonview::DRAW_PANICS.set(true);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lock(&mut h.state)));
        logonview::DRAW_PANICS.set(false);
        assert!(r.is_err(), "the screen's draw panics");
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

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.arg("-c").arg(script);
        c
    }

    /// A harness locked behind the screen, no fingerprint conversation, `password` as the password's.
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
        l.set_input(password);
        submit(&mut h.state, Side::Console);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while (h.state.lock.console.as_ref()).is_some_and(Lock::verifying) {
            assert!(std::time::Instant::now() < end, "the check ends");
            h.run_for(5);
        }
    }

    fn message(h: &crate::harness::Harness) -> Option<String> {
        h.state.lock.console.as_ref()?.message()
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
    fn a_wrong_password_repaints_the_lock_in_part_and_leaves_it_as_a_whole_redraw_draws_it() {
        for interface in ["classic", "blue", "glass"] {
            let config = format!(
                "[appearance]\ninterface = \"{interface}\"\n[effects]\nlogon_animation = false\n"
            );
            let config = crate::config::Config::parse(&config).unwrap();
            let mut h = crate::harness::Harness::with_config(config);
            h.output("TEST-1", 1024, 768);
            h.state.lock.helpers = Helpers {
                password: |_, _| sh("read p; echo '= 7'"),
                finger: |_| None,
                ..Helpers::default()
            };
            lock(&mut h.state);
            logonview::DRAW_PANICS.set(true);
            check(&mut h, "hunter1");
            logonview::DRAW_PANICS.set(false);
            assert_eq!(message(&h).as_deref(), Some(lock::INCORRECT), "{interface}");
            let o = h.state.scene.outputs()[0].clone();
            let (b, s) = (
                h.state.layout_box(Some(&o)),
                h.state.draw_scale_at(Some(&o)),
            );
            let theme = h.state.config.theme.scaled(s);
            let v = &h.state.lock.console.as_ref().unwrap().view;
            let whole = logonview::screen(&mut v.model.clone(), &theme, &h.state.text, b, s).1;
            let shown = v.shown.as_ref().expect("the screen was drawn");
            assert!(
                shown.pixels == whole.pixels,
                "{interface}: the screen is a whole redraw's"
            );
        }
    }

    #[test]
    fn text_typed_without_a_key_goes_into_the_screens_field() {
        let mut h = locked(|_, _| sh("read p; echo '= 7'"));
        type_text(&mut h.state, "pé");
        type_text(&mut h.state, "\r");
        let l = h.state.lock.console.as_ref().expect("locked");
        assert_eq!(l.input(), "pé", "control text is dropped");
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

    /// Turn the loop until `done` holds ([`crate::harness::Harness::until`]).
    fn turn_until(h: &mut crate::harness::Harness, what: &str, done: impl Fn(&Lock) -> bool) {
        h.until(what, |s| s.lock.console.as_ref().is_some_and(&done));
    }

    #[test]
    fn a_second_prompt_relabels_the_field_and_its_answer_unlocks() {
        let mut h = locked(|_, _| {
            sh(
                r#"read p; echo '?? Verification code:'; read c; [ "$p $c" = "hunter2 123456" ] && echo '= 0' || echo '= 7'"#,
            )
        });
        let l = h.state.lock.console.as_mut().expect("locked");
        l.set_input("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| {
            l.view.model.prompt.is_some()
        });
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(
            l.view.model.prompt,
            Some(lock::Prompt {
                text: "Verification code:".into(),
                echo: true
            })
        );
        assert!(!l.verifying());
        assert_eq!(l.input(), "", "the field is cleared for the answer");
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
        l.set_input("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| {
            l.view.model.prompt.is_some()
        });
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.message().as_deref(), Some("Code sent to your phone"));
        unlock(&mut h.state);
    }

    #[test]
    fn escape_under_a_second_prompt_ends_its_conversation() {
        let mut h = locked(|_, _| sh("read p; echo '? One-time code:'; exec sleep 60"));
        let l = h.state.lock.console.as_mut().expect("locked");
        l.set_input("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| {
            l.view.model.prompt.is_some()
        });
        logonview::key(&mut h.state, &CONSOLE, Some(logonview::Typed::Escape));
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(l.pending.is_none());
        assert_eq!(l.view.model.prompt, None);
        check(&mut h, "hunter2");
        assert!(
            h.state
                .lock
                .console
                .as_ref()
                .unwrap()
                .view
                .model
                .prompt
                .is_some(),
            "the next password starts a new conversation"
        );
    }

    #[test]
    fn caps_lock_shows_its_notice_while_on() {
        const KEY_CAPSLOCK: u32 = 58;
        let mut h = locked(|_, _| sh("exit 0"));
        let caps = |h: &crate::harness::Harness| {
            h.state.lock.console.as_ref().unwrap().view.model.caps_lock
        };
        assert!(!caps(&h));
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(caps(&h), "the screen redraws with the notice");
        h.key(KEY_CAPSLOCK, true);
        h.key(KEY_CAPSLOCK, false);
        assert!(!caps(&h));
    }

    #[test]
    fn a_layout_switch_while_locked_redraws_the_badge() {
        let mut h = locked(|_, _| sh("exit 0"));
        let badge = |h: &crate::harness::Harness| {
            h.state
                .lock
                .console
                .as_ref()
                .unwrap()
                .view
                .model
                .layout
                .clone()
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
        assert!(!l.verifying());
        assert!(l.pending.is_none());
        assert_eq!(l.message().as_deref(), Some(lock::UNAVAILABLE));
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
        while (h.state.lock.console.as_ref()).is_some_and(|l| l.view.model.hint.is_none()) {
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
    fn only_the_screen_runs_the_fingerprint_conversation() {
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
        c.hang_up();
        h.run_for(50);
        assert!(h.state.lock.console.as_ref().unwrap().screen_shown());
        assert!(
            finger(&h),
            "the screen that replaces a gone locker starts one"
        );
        unlock(&mut h.state);
        lock(&mut h.state);
        assert!(finger(&h), "the screen's own lock starts one");
        let mut c = h.connect();
        let (_lock, _surface) = h.map_lock_surface(&mut c);
        assert!(!finger(&h), "a locker taking over the screen ends it");
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
        start_screen_finger(&mut h.state);
        h.until("the conversation starts again", |_| {
            STARTS.load(Ordering::SeqCst) >= 3
        });
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.message().as_deref(), Some("Failed to match fingerprint"));
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
        start_screen_finger(&mut h.state);
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
        start_screen_finger(&mut h.state);
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

    /// Under reprise-login, with the lock's Switch User offered.
    fn linked() -> (crate::harness::Harness, std::os::unix::net::UnixStream) {
        let mut h = locked(|_, _| sh("read p; echo '= 7'"));
        unlock(&mut h.state);
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::loginlink::install(&mut h.state, crate::loginlink::LoginLink::from_stream(ours));
        lock(&mut h.state);
        (h, theirs)
    }

    fn press(h: &mut crate::harness::Harness, part: logon::Part) {
        let at = (h.state.lock.console.as_ref())
            .and_then(|l| l.part(part))
            .unwrap_or_else(|| panic!("{part:?} shows"));
        button(&mut h.state, Side::Console, BTN_LEFT, at);
    }

    #[test]
    fn escape_cancel_and_switch_user_never_unlock() {
        let (mut h, _theirs) = linked();
        let l = h.state.lock.console.as_mut().unwrap();
        l.set_input("hunter2");
        logonview::key(&mut h.state, &CONSOLE, Some(logonview::Typed::Escape));
        assert!(h.state.console_locked(), "Escape");
        assert_eq!(h.state.lock.console.as_ref().unwrap().input(), "");
        h.state.lock.console.as_mut().unwrap().set_input("hunter2");
        press(&mut h, logon::Part::Cancel);
        assert!(h.state.console_locked(), "Cancel");
        assert_eq!(h.state.lock.console.as_ref().unwrap().input(), "");
        press(&mut h, logon::Part::SwitchUser);
        assert!(h.state.console_locked(), "Switch User");
        check(&mut h, "hunter2");
        assert!(h.state.console_locked(), "a PAM failure");
    }

    #[test]
    fn the_lock_lists_only_its_own_user_with_the_name_fixed() {
        let (h, _theirs) = linked();
        let m = &h.state.lock.console.as_ref().unwrap().view.model;
        assert_eq!(m.users.len(), 1);
        assert_eq!(m.users[0].name, lock::current_user());
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let full = reprisefs::sysinfo::full_name(uid).unwrap_or_default();
        assert_eq!(m.users[0].real_name, full, "the tile shows the full name");
        assert!(m.name.is_read_only());
        assert!(m.unlocking());
    }

    #[test]
    fn losing_the_login_link_leaves_the_session_locked() {
        let (mut h, theirs) = linked();
        theirs.shutdown(std::net::Shutdown::Both).unwrap();
        h.run_for(50);
        assert!(h.state.console_locked());
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.covered(), ["TEST-1"], "the cover stays");
    }

    #[test]
    fn switch_user_leaves_every_output_covered_under_the_lock_layer() {
        let (mut h, _theirs) = linked();
        h.output("TEST-2", 800, 600);
        refit(&mut h.state);
        press(&mut h, logon::Part::SwitchUser);
        let (scene, layers) = (&h.state.scene, &h.state.layers);
        let top = scene.children(scene.root());
        let lock_layer = reprisescene::NodeId::from(layers.lock);
        let at = top
            .iter()
            .position(|&n| n == lock_layer)
            .expect("the lock layer");
        assert_eq!(
            top[at + 1..],
            [layers.saver.into()],
            "only the saver is above it"
        );
        assert!(scene.enabled(layers.lock));
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.covered(), ["TEST-1", "TEST-2"]);
        for c in &l.view.covers {
            let name = c.output.name();
            assert!(
                crate::console::tests::under_lock_layer(&h, c.tree.into()),
                "{name} is covered in the lock layer"
            );
            assert!(
                scene.enabled(c.tree) && scene.enabled(c.rect),
                "{name}'s cover shows"
            );
            let alpha = scene.rect_color(c.rect).map(|c| c[3]);
            assert_eq!(alpha, Some(1.0), "{name}'s backdrop is opaque");
            let b = h.state.layout_box(Some(&c.output));
            assert_eq!(
                (scene.position(c.tree), scene.rect_size(c.rect)),
                (Some((b.x, b.y).into()), Some((b.width, b.height).into())),
                "{name}'s cover spans it"
            );
        }
    }

    #[test]
    fn an_interface_change_during_a_check_waits_for_its_verdict() {
        let mut h = locked(|_, _| sh("read p; echo '?? Verification code:'; read c; echo '= 7'"));
        let l = h.state.lock.console.as_mut().expect("locked");
        l.set_input("hunter2");
        submit(&mut h.state, Side::Console);
        turn_until(&mut h, "the prompt arrives", |l| {
            l.view.model.prompt.is_some()
        });
        h.state.config.interface = Interface::Blue;
        redraw(&mut h.state);
        let m = &h.state.lock.console.as_ref().unwrap().view.model;
        assert_eq!(m.interface, Interface::Classic, "the screen waits");
        assert!(m.prompt.is_some(), "its prompt stays");
        check(&mut h, "123456");
        let m = &h.state.lock.console.as_ref().unwrap().view.model;
        assert_eq!(m.interface, Interface::Blue, "the verdict is in");
    }

    #[test]
    fn the_screen_shows_on_the_primary_output_and_the_backdrop_on_the_others() {
        let mut h = locked(|_, _| sh("exit 0"));
        h.output("TEST-2", 800, 600);
        refit(&mut h.state);
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.covered(), ["TEST-1", "TEST-2"]);
        assert_eq!(l.screens(), ["TEST-1"]);
        unlock(&mut h.state);
        h.state.config.outputs = vec![crate::config::OutputConfig {
            name: "TEST-2".into(),
            scale: 1.0,
            mode: None,
            adaptive_sync: None,
            transform: Default::default(),
            position: None,
            primary: true,
        }];
        lock(&mut h.state);
        let l = h.state.lock.console.as_ref().unwrap();
        assert_eq!(l.screens(), ["TEST-2"], "the primary output has it");
    }

    #[test]
    fn a_hidden_screen_takes_no_press() {
        let mut h = locked(|_, _| sh("exit 0"));
        let at = h
            .state
            .lock
            .console
            .as_ref()
            .unwrap()
            .part(logon::Part::Cancel);
        let at = at.expect("Cancel shows");
        h.state.lock.console.as_mut().unwrap().awaiting = true;
        redraw(&mut h.state);
        let l = h.state.lock.console.as_mut().unwrap();
        assert!(l.view.hidden);
        assert_eq!(l.view.part_at(at), None);
        l.set_input("secret");
        button(&mut h.state, Side::Console, BTN_LEFT, at);
        assert_eq!(h.state.lock.console.as_ref().unwrap().input(), "secret");
    }

    #[test]
    fn an_interface_change_while_locked_redraws_the_screen_in_the_new_one() {
        let mut h = locked(|_, _| sh("exit 0"));
        h.state.config.interface = Interface::Glass;
        refit(&mut h.state);
        let m = &h.state.lock.console.as_ref().unwrap().view.model;
        assert_eq!(m.interface, Interface::Glass);
        assert!(m.unlocking() && m.name.is_read_only(), "still the lock");
    }

    #[test]
    fn the_screen_keeps_the_water_out() {
        let mut h = locked(|_, _| sh("exit 0"));
        let l = h.state.lock.console.as_ref().unwrap();
        let on = l.part(logon::Part::Submit).expect("OK shows");
        let beside = Point::from((5.0, 5.0));
        assert!(!splashes(l, on), "over the screen's parts");
        assert!(splashes(l, beside), "over the backdrop");
        h.state.lock.console.as_mut().unwrap().awaiting = true;
        redraw(&mut h.state);
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(!splashes(l, beside), "a locker's own screen has no water");
    }

    /// Locked under Glass behind its screen, "hunter2" the password.
    fn locked_glass() -> crate::harness::Harness {
        locked_glass_with("")
    }

    /// A Glass lock on TEST-1 that takes hunter2, `extra` added to its config.
    fn locked_glass_with(extra: &str) -> crate::harness::Harness {
        let config = format!("[appearance]\ninterface = \"glass\"\n{extra}");
        let mut h =
            crate::harness::Harness::with_config(crate::config::Config::parse(&config).unwrap());
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers = Helpers {
            password: |_, _| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#),
            finger: |_| None,
            ..Helpers::default()
        };
        lock(&mut h.state);
        h
    }

    #[test]
    fn a_window_under_a_glass_lock_with_water_asks_no_frames() {
        use crate::harness::{Harness, TestClient, Toplevel};
        fn asks(h: &mut Harness, c: &mut TestClient, t: Toplevel) -> bool {
            crate::backend::take_asks();
            h.draw_self(c, t, 0xff40_4040);
            (crate::backend::take_asks().iter())
                .any(|(_, a)| matches!(a, crate::backend::Ask::Commit(_)))
        }
        let config = "[appearance]\ninterface = \"glass\"\n[effects]\nreduce_motion = true\n[wallpaper]\nwater = \"rain\"\n";
        let mut h = Harness::with_config(crate::config::Config::parse(config).unwrap());
        h.output("TEST-1", 320, 200);
        h.state
            .set_wallpaper(Some(std::path::Path::new("reprise:tarn/blue")));
        let o = h.state.scene.outputs()[0].clone();
        h.until("the lock wallpaper composes", |s| {
            !matches!(s.lock_wallpaper(&o), LockWall::None | LockWall::Composing)
        });
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "printer", 200, 100);
        assert!(asks(&mut h, &mut c, t), "shown");
        lock(&mut h.state);
        let lock_pane = ("TEST-1".to_string(), Pane::Lock);
        assert!(
            h.state.water.pane_keys().contains(&lock_pane),
            "the cover has water"
        );
        assert!(
            !asks(&mut h, &mut c, t),
            "under the water over the cover's opaque backdrop"
        );
    }

    /// The asks the frame about to render on TEST-1 makes.
    fn next_frame(h: &mut crate::harness::Harness) -> Vec<String> {
        let o = h.state.scene.outputs()[0].clone();
        crate::backend::take_asks();
        crate::backend::advance(&mut h.state, &o);
        (crate::backend::take_asks().into_iter())
            .map(|(o, _)| o)
            .collect()
    }

    fn middle(h: &crate::harness::Harness) -> Point<f64, Logical> {
        let b = h.state.layout_box(None);
        Point::from((f64::from(b.width / 2), f64::from(b.height / 2)))
    }

    #[test]
    fn the_lock_screen_enters_over_opaque_covers_on_its_first_frame() {
        let mut h = locked_glass();
        let l = h.state.lock.console.as_ref().unwrap();
        let p = l.view.play.as_ref().expect("the screen enters");
        assert_eq!(p.t.change, logon::Change::Enter);
        assert!(p.since.is_none());
        let c = &l.view.covers[0];
        assert_eq!(
            h.state.scene.opacity(c.tree),
            Some(1.0),
            "the cover is whole"
        );
        assert_eq!(next_frame(&mut h), ["TEST-1"]);
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(l.view.play.as_ref().unwrap().since.is_some());
    }

    #[test]
    fn the_unlock_fade_takes_no_input_and_the_lock_is_off_at_once() {
        let mut h = locked_glass();
        logonview::hold_clock();
        check(&mut h, "hunter2");
        assert!(!h.state.console_locked(), "unlocked at once");
        assert_eq!(h.state.lock.leaving.len(), 1, "the cover fades out");
        assert!(
            matches!(h.state.scene_hit(middle(&h)), Hit::Desktop { .. }),
            "a press reaches the desktop through the fading cover"
        );
        let tree = h.state.lock.leaving[0].tree;
        assert_eq!(next_frame(&mut h), ["TEST-1"]);
        h.state.lock.leaving[0].since = Some(logonview::now() - Duration::from_millis(100));
        next_frame(&mut h);
        let half = h.state.scene.opacity(tree).unwrap();
        assert!(
            half > 0.3 && half < 0.7,
            "half way it is half faded: {half}"
        );
        h.state.lock.leaving[0].since =
            Some(logonview::now() - Duration::from_millis(UNLOCK_MS + 1));
        assert_eq!(next_frame(&mut h), Vec::<String>::new());
        assert!(h.state.lock.leaving.is_empty());
        assert_eq!(h.state.scene.opacity(tree), None, "the cover is gone");
    }

    #[test]
    fn an_unlock_while_away_drops_the_covers_at_once() {
        let mut h = locked_glass();
        seat_away(&mut h.state, true);
        check(&mut h, "hunter2");
        assert!(!h.state.console_locked());
        assert!(h.state.lock.leaving.is_empty());
    }

    #[test]
    fn a_second_unlock_soon_after_the_first_fades_out_whole() {
        let mut h = locked_glass();
        check(&mut h, "hunter2");
        next_frame(&mut h);
        h.run_for(250);
        lock(&mut h.state);
        check(&mut h, "hunter2");
        next_frame(&mut h);
        h.run_for(200);
        assert_eq!(
            h.state.lock.leaving.len(),
            1,
            "the first unlock's timer leaves the second fade to its frames"
        );
    }

    #[test]
    fn with_logon_animation_off_the_lock_neither_enters_nor_fades() {
        let mut h = locked_glass_with("[effects]\nlogon_animation = false\n");
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(l.view.play.is_none());
        assert_eq!(next_frame(&mut h), Vec::<String>::new());
        check(&mut h, "hunter2");
        assert!(!h.state.console_locked());
        assert!(h.state.lock.leaving.is_empty(), "the covers go at once");
    }

    #[test]
    fn a_classic_lock_neither_enters_nor_fades() {
        let mut h = locked(|_, _| sh(r#"read p; [ "$p" = hunter2 ] && echo '= 0' || echo '= 7'"#));
        let l = h.state.lock.console.as_ref().unwrap();
        assert!(l.view.play.is_none());
        check(&mut h, "hunter2");
        assert!(!h.state.console_locked());
        assert!(h.state.lock.leaving.is_empty());
    }
}
