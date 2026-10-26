//! The lock screen on the scene: one opaque cover, the wallpaper over it with `[wallpaper] lock_screen`,
//! one dialog per output in `layers.lock`, key and pointer routing while locked, the pipe the PAM thread
//! answers through, and the fingerprint conversation's child. Pure state lives in `lock.rs` and `lockfinger.rs`.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use smithay::{
    backend::input::InputTime,
    input::keyboard::KeysymHandle,
    output::Output,
    reexports::{
        calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken, generic::Generic},
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
use crate::lock::{self, LockButton, LockKey, LockState, PamVerifier};
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
}

impl LockScreen {
    pub fn new(dh: &smithay::reexports::wayland_server::DisplayHandle) -> LockScreen {
        LockScreen {
            session: SessionLock::new(dh),
            active: None,
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
    pub finger: Option<Finger>,
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
    fn disarm(mut self) {
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

/// The source polling one attempt's result pipe. The verifier thread owns the write end, so EOF means the thread is gone.
pub struct Pending(Source);

/// One lock's fingerprint conversation (see `lockfinger`) and the source watching its stdout. Dropping
/// it removes the source, then ends the child, which releases the reader.
pub struct Finger {
    source: Source,
    conversation: lockfinger::Conversation,
}

/// Start the fingerprint conversation for `user`. None when the PAM service is not installed or the
/// child or its loop source cannot be had; the password still unlocks.
fn start_finger(state: &State, user: &str) -> Option<Finger> {
    if !lockfinger::service_installed() {
        return None;
    }
    let conversation = match lockfinger::Conversation::start(user) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("lock: cannot start the fingerprint conversation: {e}");
            return None;
        }
    };
    // SAFETY: the conversation's stdout is open for as long as the conversation, which outlives this borrow.
    let fd = unsafe { BorrowedFd::borrow_raw(conversation.fd()) }.try_clone_to_owned();
    let token = fd.ok().and_then(|fd| {
        state
            .handle
            .insert_source(
                Generic::new(fd, Interest::READ, Mode::Level),
                |_, _, state| Ok(on_finger(state)),
            )
            .ok()
    });
    let Some(token) = token else {
        log::warn!("lock: cannot watch the fingerprint conversation");
        return None;
    };
    Some(Finger {
        source: Source {
            token: Some(token),
            handle: state.handle.clone(),
        },
        conversation,
    })
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
        if let Some(f) = l.finger.take() {
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

/// Never leave the dialog stuck on "Verifying...": clear it, show `message` and redraw.
fn abort_verification(state: &mut State, message: &str) {
    if let Some(l) = state.lock.active.as_mut() {
        if let Some(p) = l.pending.take() {
            p.0.disarm();
        }
        l.state.fail_with(message);
    }
    redraw(state);
}

/// The verifier thread's verdict (see `lock::decode_verdict`). The read end is non-blocking; the write
/// end is not, so its single write of at most `PIPE_BUF` bytes is all-or-nothing and one read takes it whole.
fn on_result(state: &mut State, fd: BorrowedFd<'_>) -> PostAction {
    let mut b = [0u8; libc::PIPE_BUF];
    // SAFETY: `b` has room for the `b.len()` bytes requested.
    let n = unsafe { libc::read(fd.as_raw_fd(), b.as_mut_ptr().cast(), b.len()) };
    if n >= 0 {
        match lock::decode_verdict(&b[..n as usize]) {
            Some(Ok(())) => {
                if let Some(l) = state.lock.active.as_mut() {
                    if let Some(p) = l.pending.take() {
                        p.0.disarm();
                    }
                    l.state.verifying = false;
                }
                unlock(state);
            }
            Some(Err(notices)) => abort_verification(state, &lock::failure_text(&notices)),
            None => {
                // The thread died (or never spawned) without writing; `verifying` must not stick.
                log::error!("lock: verifier thread ended without a result");
                abort_verification(state, lock::INCORRECT);
            }
        }
        return PostAction::Remove;
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EAGAIN) | Some(libc::EINTR) => PostAction::Continue,
        _ => {
            log::error!("lock: result pipe read failed: {err}");
            abort_verification(state, lock::INCORRECT);
            PostAction::Remove
        }
    }
}

/// Lock the session, then start `[power] locker` if one is set. No-op when already locked.
pub fn lock(state: &mut State) {
    if state.is_locked() {
        return;
    }
    engage(state);
    crate::sessionlock::start_locker(state);
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
    let finger = if user.is_empty() {
        None
    } else {
        start_finger(state, &user)
    };
    state.lock.active = Some(Lock {
        state: LockState::new(),
        user,
        outputs: Vec::new(),
        pending: None,
        finger,
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
    // The fingerprint child is interrupted and reaped, freeing the reader before a new lock starts its own.
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

pub fn redraw(state: &mut State) {
    let outputs: Vec<Output> = match state.lock.active.as_ref() {
        Some(l) => l.outputs.iter().map(|lo| lo.output.clone()).collect(),
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

fn submit(state: &mut State) {
    let pipe = result_pipe(state);
    let Some(l) = state.lock.active.as_mut() else {
        return;
    };
    l.state.verifying = true;
    l.state.message = None;
    let password = l.state.take_input();
    let user = l.user.clone();
    match pipe {
        Some((pending, write)) => {
            l.pending = Some(pending);
            if !lock::verify_async(Box::new(PamVerifier), user, password, write) {
                // Dropping `pending` unwatches the pipe before its EOF could read as a wrong password.
                l.pending = None;
                l.state.fail_with(lock::UNAVAILABLE);
            }
        }
        None => l.state.fail(),
    }
    redraw(state);
}

/// One attempt's result pipe: the read end, non-blocking and watched by `on_result`, and the write end,
/// blocking for the verifier thread's single unretried write.
fn result_pipe(state: &State) -> Option<(Pending, OwnedFd)> {
    let mut fds = [0i32; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        log::error!("lock: pipe2 failed: {}", std::io::Error::last_os_error());
        return None;
    }
    // SAFETY: pipe2 succeeded, so both fds are open and owned by nothing else.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: `read` is an open fd.
    let nonblocking = unsafe {
        let flags = libc::fcntl(read.as_raw_fd(), libc::F_GETFL);
        flags != -1 && libc::fcntl(read.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    };
    if !nonblocking {
        log::error!(
            "lock: cannot make result pipe non-blocking: {}",
            std::io::Error::last_os_error()
        );
        return None;
    }
    let token = state
        .handle
        .insert_source(
            Generic::new(read, Interest::READ, Mode::Level),
            |_, fd, state| Ok(on_result(state, fd.as_fd())),
        )
        .map_err(|e| log::error!("lock: cannot watch the result pipe: {}", e.error))
        .ok()?;
    Some((
        Pending(Source {
            token: Some(token),
            handle: state.handle.clone(),
        }),
        write,
    ))
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
        Some(l) => l.state.key(key),
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
        l.state.button(b)
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
