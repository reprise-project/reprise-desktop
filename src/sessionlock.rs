//! ext-session-lock-v1: an external locker (swaylock, hyprlock, gtklock)
//! draws the lock screen in place of the built-in dialog.
//!
//! The lock stays Reprise's: a locker's lock engages the same `lockview`
//! lock as the dialog, with its covers and refused focus, and the locker's
//! surfaces go on top of the covers with the keyboard and pointer. A
//! locker that crashes leaves the session locked, as the protocol asks, and
//! the dialog comes back. A lock the compositor ends itself
//! (`loginctl unlock-session`, a password typed into the dialog) sends the
//! locker `finished`.
//!
//! `locked` goes out once every output showing the layout has presented a
//! frame with the cover, or after `LOCKED_WAIT_MS`.
//!
//! A second locker is refused while one holds the lock. A locker may take
//! over a lock that has none (the dialog's or a crashed locker's) and
//! unlock it; any client that can bind the global can already run
//! `loginctl unlock-session`.
//!
//! `[power] locker` names a locker for Reprise's own lock to start: the lock
//! engages at once with a bare cover, and the dialog shows only if no locker
//! has taken over within `LOCKER_WAIT_MS`.

use std::time::Duration;

use smithay::{
    backend::{input::InputTime, renderer::utils::with_renderer_surface_state},
    output::Output,
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_protocols::ext::session_lock::v1::server::ext_session_lock_v1::ExtSessionLockV1,
        wayland_server::{
            DisplayHandle, Resource,
            protocol::{wl_output::WlOutput, wl_surface::WlSurface},
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::add_destruction_hook,
        session_lock::{LockSurface, SessionLockHandler, SessionLockManagerState, SessionLocker},
    },
};

use reprisescene::{SurfaceId, TreeId};

use crate::compositor::State;

/// How long a locker waits for `locked` when an output never presents the
/// cover (a hung display, say). The same bound a sleep waits for it.
const LOCKED_WAIT_MS: u64 = 2000;

/// How long the lock waits for the configured locker before the dialog shows.
const LOCKER_WAIT_MS: u64 = 5000;

/// The global and the two timers, kept in `State::lock`.
pub struct SessionLock {
    pub manager: SessionLockManagerState,
    /// Sends `locked` when an output is slow to present the cover.
    locked_timer: Option<RegistrationToken>,
    /// Shows the dialog when the configured locker is slow to lock.
    locker_timer: Option<RegistrationToken>,
}

impl SessionLock {
    pub fn new(dh: &DisplayHandle) -> SessionLock {
        SessionLock {
            manager: SessionLockManagerState::new::<State, _>(
                dh,
                crate::sandbox::filter("ext_session_lock_manager_v1"),
            ),
            locked_timer: None,
            locker_timer: None,
        }
    }
}

pub struct ClientLock {
    lock: ExtSessionLockV1,
    /// The confirmation `locked` goes out through; None once it went.
    locker: Option<SessionLocker>,
    /// Holds every lock surface's node, above the covers.
    tree: TreeId,
    surfaces: Vec<LockSurfaceNode>,
    /// The outputs yet to present the cover; `None` once `locked` is sent.
    wait: Option<CoverWait<Output>>,
}

/// One of the locker's surfaces, one per output.
struct LockSurfaceNode {
    surface: LockSurface,
    output: Output,
    node: SurfaceId,
    /// The size last configured, (0, 0) before the first.
    sent: (u32, u32),
    mapped: bool,
}

/// The outputs that have yet to show the lock cover (for `locked`, and for
/// a sleep in `power`). A frame queued after the cover went up marks its
/// output; that frame's vblank, or a frame with nothing new to draw, ends
/// its wait.
pub(crate) struct CoverWait<K> {
    waiting: Vec<(K, bool)>,
}

impl<K: PartialEq> CoverWait<K> {
    pub(crate) fn new(outputs: Vec<K>) -> Self {
        CoverWait {
            waiting: outputs.into_iter().map(|o| (o, false)).collect(),
        }
    }

    pub(crate) fn queued(&mut self, output: &K) {
        for (o, q) in &mut self.waiting {
            if o == output {
                *q = true;
            }
        }
    }

    /// `output` presented a frame: the cover is on screen if one with it was queued.
    pub(crate) fn presented(&mut self, output: &K) {
        self.waiting.retain(|(o, q)| !(o == output && *q));
    }

    /// `output` rendered nothing new (or went away): it already shows the cover.
    pub(crate) fn forget(&mut self, output: &K) {
        self.waiting.retain(|(o, _)| o != output);
    }

    pub(crate) fn retain(&mut self, keep: impl Fn(&K) -> bool) {
        self.waiting.retain(|(o, _)| keep(o));
    }

    pub(crate) fn done(&self) -> bool {
        self.waiting.is_empty()
    }
}

/// Start `[power] locker`, if set, for a lock that just engaged, holding
/// the dialog back while it starts. Returns whether it started.
pub fn start_locker(state: &mut State) -> bool {
    let cmd = state.config.power.locker.trim().to_string();
    if cmd.is_empty() || !state.is_locked() {
        return false;
    }
    if !crate::spawn::spawn(&cmd, &state.env, None) {
        return false;
    }
    if let Some(l) = state.lock.active.as_mut() {
        l.awaiting = true;
    }
    stop_timer(state, Which::Locker);
    let token = state.handle.insert_source(
        Timer::from_duration(Duration::from_millis(LOCKER_WAIT_MS)),
        |_, _, state| {
            state.lock.session.locker_timer = None;
            locker_timed_out(state);
            TimeoutAction::Drop
        },
    );
    state.lock.session.locker_timer = token.ok();
    crate::lockview::redraw(state);
    true
}

#[derive(Clone, Copy)]
enum Which {
    Locked,
    Locker,
}

fn stop_timer(state: &mut State, which: Which) {
    let s = &mut state.lock.session;
    let t = match which {
        Which::Locked => s.locked_timer.take(),
        Which::Locker => s.locker_timer.take(),
    };
    if let Some(t) = t {
        state.handle.remove(t);
    }
}

pub fn stop_timers(state: &mut State) {
    stop_timer(state, Which::Locked);
    stop_timer(state, Which::Locker);
}

fn locker_timed_out(state: &mut State) {
    let Some(l) = state.lock.active.as_mut() else {
        return;
    };
    if !l.awaiting {
        return;
    }
    l.awaiting = false;
    let cmd = &state.config.power.locker;
    log::warn!("session lock: {cmd:?} did not lock in time; showing the dialog");
    crate::lockview::redraw(state);
}

fn client(state: &mut State) -> Option<&mut ClientLock> {
    state.lock.active.as_mut()?.client.as_mut()
}

pub fn holds_input(state: &State) -> bool {
    state
        .lock
        .active
        .as_ref()
        .is_some_and(|l| l.client.is_some())
}

pub fn is_lock_surface(state: &State, surface: &WlSurface) -> bool {
    state
        .lock
        .active
        .as_ref()
        .and_then(|l| l.client.as_ref())
        .is_some_and(|c| c.surfaces.iter().any(|s| s.surface.wl_surface() == surface))
}

impl SessionLockHandler for State {
    fn lock_state(&mut self) -> &mut SessionLockManagerState {
        &mut self.lock.session.manager
    }

    fn lock(&mut self, confirmation: SessionLocker) {
        // A locker whose lock went away without unlocking leaves the
        // session locked; this lock may take over.
        check_alive(self);
        if holds_input(self) {
            log::warn!("session lock: refused a second locker while one holds the lock");
            // Dropped unconfirmed: the locker hears `finished`.
            return;
        }
        crate::lockview::engage(self);
        if !self.is_locked() {
            return;
        }
        let tree = self.scene.tree_create(self.layers.lock);
        let outputs: Vec<Output> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| self.layout_box(Some(o)).width > 0 && !crate::backend::stalled(self, o))
            .cloned()
            .collect();
        let c = ClientLock {
            lock: confirmation.ext_session_lock().clone(),
            locker: Some(confirmation),
            tree,
            surfaces: Vec::new(),
            wait: Some(CoverWait::new(outputs)),
        };
        if let Some(l) = self.lock.active.as_mut() {
            // Whatever was typed into the dialog goes; a check in flight
            // is dropped, its thread gets EPIPE.
            l.pending = None;
            l.state.reset();
            l.awaiting = false;
            l.client = Some(c);
        }
        stop_timer(self, Which::Locker);
        crate::lockview::redraw(self);
        log::info!("session lock: a locker holds the lock");
        if client(self).is_some_and(|c| c.wait.as_ref().is_some_and(CoverWait::done)) {
            send_locked(self);
        } else {
            let token = self.handle.insert_source(
                Timer::from_duration(Duration::from_millis(LOCKED_WAIT_MS)),
                |_, _, state| {
                    state.lock.session.locked_timer = None;
                    if client(state).is_some_and(|c| c.wait.is_some()) {
                        log::warn!("session lock: an output did not present the cover in time");
                        send_locked(state);
                    }
                    TimeoutAction::Drop
                },
            );
            self.lock.session.locked_timer = token.ok();
            self.redraw_all();
        }
    }

    fn unlock(&mut self) {
        if let Some(c) = self.lock.active.as_mut().and_then(|l| l.client.take()) {
            release(self, c, false);
        }
        log::info!("session lock: the locker unlocked");
        crate::lockview::unlock(self);
    }

    fn new_surface(&mut self, surface: LockSurface, output: WlOutput) {
        let Some(output) = Output::from_resource(&output) else {
            return;
        };
        let Some(c) = client(self) else {
            return;
        };
        if surface.ext_session_lock() != &c.lock {
            return;
        }
        let tree = c.tree;
        let wl = surface.wl_surface().clone();
        let node = self.scene.surface_create(tree, wl.clone());
        add_destruction_hook::<State, _>(&wl, |state, _| {
            // After this dispatch, by when a crashed locker's lock is gone
            // too and `check_alive` sees it.
            state.handle.insert_idle(|state| {
                forget_dead_surfaces(state);
                check_alive(state);
            });
        });
        if let Some(c) = client(self) {
            c.surfaces.push(LockSurfaceNode {
                surface,
                output,
                node,
                sent: (0, 0),
                mapped: false,
            });
        }
        fit_all(self, false);
    }
}

fn send_locked(state: &mut State) {
    let Some(c) = client(state) else {
        return;
    };
    if c.wait.take().is_none() {
        return;
    }
    if let Some(locker) = c.locker.take() {
        locker.lock();
    }
    stop_timer(state, Which::Locked);
    log::info!("session lock: locked");
}

/// `output` rendered: a queued frame (`drawn`) counts toward `locked` once
/// presented; an `empty` frame shows the cover already.
pub fn rendered(state: &mut State, output: &Output, drawn: bool, empty: bool) {
    let Some(wait) = client(state).and_then(|c| c.wait.as_mut()) else {
        return;
    };
    if drawn {
        wait.queued(output);
    } else if empty {
        wait.forget(output);
    }
    if wait.done() {
        send_locked(state);
    }
}

pub fn presented(state: &mut State, output: &Output) {
    let Some(wait) = client(state).and_then(|c| c.wait.as_mut()) else {
        return;
    };
    wait.presented(output);
    if wait.done() {
        send_locked(state);
    }
}

/// `output`'s commit stalled (`backend::stalled`): `locked` stops waiting for its cover.
pub fn stalled(state: &mut State, output: &Output) {
    let Some(wait) = client(state).and_then(|c| c.wait.as_mut()) else {
        return;
    };
    wait.forget(output);
    if wait.done() {
        send_locked(state);
    }
}

/// Place each lock surface on its output and configure it to the output's
/// size (sent at once with `send`). An output outside the layout still gets
/// a size, or the locker waits for it forever, but shows nothing.
fn fit_all(state: &mut State, send: bool) {
    let outputs: Vec<(Output, crate::raster::Rect)> = state
        .scene
        .outputs()
        .iter()
        .map(|o| (o.clone(), state.layout_box(Some(o))))
        .collect();
    let State { lock, scene, .. } = state;
    let Some(c) = lock.active.as_mut().and_then(|l| l.client.as_mut()) else {
        return;
    };
    for s in &mut c.surfaces {
        let b = outputs
            .iter()
            .find(|(o, _)| *o == s.output)
            .map(|(_, b)| *b)
            .unwrap_or_default();
        let (w, h) = if b.width > 0 {
            (b.width, b.height)
        } else {
            s.output
                .current_mode()
                .map_or((1, 1), |m| (m.size.w, m.size.h))
        };
        scene.set_position(s.node, b.x, b.y);
        scene.set_enabled(s.node, b.width > 0);
        let size = (w.max(1) as u32, h.max(1) as u32);
        if s.sent != size {
            s.sent = size;
            s.surface.with_pending_state(|p| p.size = Some(size.into()));
            if send {
                s.surface.send_configure();
            }
        }
    }
}

/// Refit every lock surface after a layout change, keep them above the
/// covers, and stop waiting on outputs that left.
pub fn refit(state: &mut State) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    if let Some(wait) = client(state).and_then(|c| c.wait.as_mut()) {
        let gone: Vec<Output> = wait
            .waiting
            .iter()
            .map(|(o, _)| o.clone())
            .filter(|o| !outputs.contains(o))
            .collect();
        for o in &gone {
            wait.forget(o);
        }
        if wait.done() {
            send_locked(state);
        }
    }
    fit_all(state, true);
    raise(state);
}

pub fn raise(state: &mut State) {
    if let Some(tree) = client(state).map(|c| c.tree) {
        state.scene.raise_to_top(tree);
    }
}

/// A commit on any surface: a lock surface's first buffer maps it and
/// takes the keyboard unless another lock surface has it.
pub fn commit(state: &mut State, surface: &WlSurface) {
    let has_buffer =
        with_renderer_surface_state(surface, |s| s.buffer().is_some()).unwrap_or(false);
    let Some(s) = client(state).and_then(|c| {
        c.surfaces
            .iter_mut()
            .find(|s| s.surface.wl_surface() == surface)
    }) else {
        return;
    };
    let newly = has_buffer && !s.mapped;
    s.mapped = has_buffer;
    if !newly {
        return;
    }
    let focused = state.seat.get_keyboard().and_then(|k| k.current_focus());
    if !focused.is_some_and(|f| is_lock_surface(state, &f)) {
        state.enter_keyboard(Some(surface.clone()));
    }
    let p = state.pointer_location();
    motion(state, p, InputTime::now());
}

fn forget_dead_surfaces(state: &mut State) {
    let State { lock, scene, .. } = state;
    let Some(c) = lock.active.as_mut().and_then(|l| l.client.as_mut()) else {
        return;
    };
    c.surfaces.retain(|s| {
        let alive = s.surface.alive();
        if !alive {
            scene.destroy(s.node);
        }
        alive
    });
    ensure_keyboard(state);
}

/// A locker whose lock went away without unlocking died: the session stays
/// locked and the dialog comes back.
pub fn check_alive(state: &mut State) {
    let dead = state
        .lock
        .active
        .as_ref()
        .and_then(|l| l.client.as_ref())
        .is_some_and(|c| !c.lock.is_alive());
    if !dead {
        return;
    }
    if let Some(c) = state.lock.active.as_mut().and_then(|l| l.client.take()) {
        release(state, c, false);
    }
    state.enter_keyboard(None);
    let p = state.pointer_location();
    state.pointer_motion(None, p, InputTime::now());
    state.set_cursor("default");
    log::warn!("session lock: the locker went away without unlocking; showing the dialog");
    crate::lockview::redraw(state);
}

/// Let go of a locker: take its surfaces off the screen. `finish` sends it
/// `finished`, for a lock the compositor ends.
pub fn release(state: &mut State, mut c: ClientLock, finish: bool) {
    state.scene.destroy(c.tree);
    if finish {
        match c.locker.take() {
            Some(locker) => drop(locker),
            None if c.lock.is_alive() => c.lock.finished(),
            None => {}
        }
    }
    stop_timer(state, Which::Locked);
}

fn ensure_keyboard(state: &mut State) {
    let focused = state.seat.get_keyboard().and_then(|k| k.current_focus());
    if focused.is_some_and(|f| is_lock_surface(state, &f)) {
        return;
    }
    let p = state.pointer_location();
    let here = state.output_at(p.x, p.y);
    let Some(c) = client(state) else {
        return;
    };
    let mapped = || c.surfaces.iter().filter(|s| s.mapped);
    let target = mapped()
        .find(|s| Some(&s.output) == here.as_ref())
        .or_else(|| mapped().next())
        .map(|s| s.surface.wl_surface().clone());
    if target.is_some() {
        state.enter_keyboard(target);
    }
}

/// Point the seat at the lock surface under the cursor. Returns false when
/// no locker holds the lock.
pub fn motion(state: &mut State, pos: Point<f64, Logical>, time: InputTime) -> bool {
    let Some(tree) = client(state).map(|c| c.tree) else {
        return false;
    };
    let hit = state
        .scene
        .node_at(tree, pos)
        .and_then(|h| h.surface.map(|s| (s, pos - h.local)));
    if hit.is_none() {
        state.set_cursor("default");
    }
    state.pointer_motion(hit, pos, time);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_waits_for_a_frame_queued_after_the_cover() {
        let mut w = CoverWait::new(vec!["DP-1", "HDMI-A-1"]);
        w.presented(&"DP-1");
        assert!(!w.done());
        w.queued(&"DP-1");
        w.presented(&"DP-1");
        assert!(!w.done(), "HDMI-A-1 has yet to present");
        w.queued(&"HDMI-A-1");
        w.presented(&"HDMI-A-1");
        assert!(w.done());
    }

    #[test]
    fn an_empty_frame_or_a_gone_output_stops_the_wait() {
        let mut w = CoverWait::new(vec!["DP-1", "DP-2"]);
        w.forget(&"DP-1");
        assert!(!w.done());
        w.forget(&"DP-2");
        assert!(w.done());
        assert!(CoverWait::<&str>::new(Vec::new()).done());
    }
}
