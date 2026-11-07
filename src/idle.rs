//! ext-idle-notify-v1, idle-inhibit-v1, wlr-output-power-management-v1 and
//! the compositor's own idle timer: start the screen saver after
//! `blank_after` seconds of no input (`saverview`), lock after
//! `lock_after` (dimming the desktop to grey over the `lock_fade` seconds
//! before that), power every output off after `screen_off_after`, run the
//! standby command after `suspend_after`, all paused while the session is
//! unlocked and any client holds an idle inhibitor or an
//! `org.freedesktop.ScreenSaver` cookie. A fullscreen window on screen holds
//! off only the saver; the other steps keep their timeouts. A remote-control
//! session holds off screen-off and suspend, even while locked. A remote client
//! elsewhere (`power::remote_elsewhere`) or a session off the seat holds off suspend.
//! The timeouts are `Config::idle_for` the current power source
//! (`power::PowerState::on_battery`).

use std::time::{Duration, Instant};

use smithay::{
    backend::input::{InputBackend, InputEvent},
    backend::renderer::utils::with_renderer_surface_state,
    output::Output,
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    },
    wayland::{
        compositor::{HookId, add_destruction_hook, get_parent, remove_destruction_hook},
        idle_inhibit::{IdleInhibitHandler, IdleInhibitManagerState},
        idle_notify::{IdleNotifierHandler, IdleNotifierState},
    },
};

use reprisescene::RectId;

use crate::compositor::State;
use crate::config::IdleConfig;
use crate::outputpower::{OutputPowerHandler, OutputPowerState};

/// When the dim before the lock starts, in ms of idle time; 0 when there
/// is no dim (no lock, or `lock_fade` 0). A fade longer than `lock_after`
/// starts at once and is cut short by the lock. Never 0 while a dim is
/// wanted: `pending` reads 0 as disabled.
pub fn dim_after_ms(cfg: &IdleConfig) -> u64 {
    if cfg.lock_after == 0 || cfg.lock_fade == 0 {
        return 0;
    }
    (cfg.lock_after as u64 * 1000)
        .saturating_sub(cfg.lock_fade as u64 * 1000)
        .max(1)
}

/// Indices into `outputs` the idle timer should act on. Each entry is
/// `(on, recorded)`: whether that output is on right now, and whether the
/// timer is the one that switched it off. Switching off takes every output
/// that is currently on; waking takes only the recorded ones that are still
/// off, so an output a client switched off (`wlopm --off`) stays dark.
pub fn power_targets(outputs: &[(bool, bool)], on: bool) -> Vec<usize> {
    outputs
        .iter()
        .enumerate()
        .filter(|&(_, &(enabled, rec))| if on { rec && !enabled } else { enabled })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Steps {
    pub locked_fired: bool,
    /// Outputs are powered off by the idle timer (not by a client through
    /// output-power-management).
    pub screen_off: bool,
    pub blanked: bool,
    pub suspended: bool,
    pub dimming: bool,
}

impl Steps {
    /// Milliseconds from now until the next idle deadline, or `None` when
    /// nothing is pending: every timeout disabled, every pending step
    /// already fired, or `inhibited`. `elapsed_ms` is the time since the
    /// last input; `saver_ms` the saver's own count, `None` while it is held.
    pub fn next_deadline_ms(
        &self,
        cfg: &IdleConfig,
        elapsed_ms: u64,
        saver_ms: Option<u64>,
        inhibited: bool,
    ) -> Option<u64> {
        if inhibited {
            return None;
        }
        let pending = |after_ms: u64, done: bool| -> Option<u64> {
            if after_ms == 0 || done {
                None
            } else {
                Some(after_ms.saturating_sub(elapsed_ms))
            }
        };
        let s = |after_s: u32| after_s as u64 * 1000;
        [
            pending(s(cfg.lock_after), self.locked_fired),
            pending(s(cfg.screen_off_after), self.screen_off),
            saver_ms.and_then(|ms| {
                (cfg.blank_after > 0 && !self.blanked)
                    .then(|| s(cfg.blank_after).saturating_sub(ms))
            }),
            pending(s(cfg.suspend_after), self.suspended),
            pending(dim_after_ms(cfg), self.dimming),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn is_idle(&self) -> bool {
        self.blanked || self.dimming || self.locked_fired || self.screen_off || self.suspended
    }

    /// The machine woke from sleep: the standby step may run again.
    pub fn resumed(&mut self) {
        self.suspended = false;
    }
}

/// How often activity reaches the idle-notify clients: each time restarts
/// every client's timer, which a 1000 Hz mouse would do 1000 times a
/// second. The last activity of a burst still reaches them, at most this
/// late.
const NOTIFY_EVERY: Duration = Duration::from_millis(50);

pub struct IdleState {
    pub notifier: IdleNotifierState<State>,
    notified_at: Option<Instant>,
    notify_timer: Option<RegistrationToken>,
    _inhibit: IdleInhibitManagerState,
    pub output_power: OutputPowerState,
    timer: Option<RegistrationToken>,
    inhibitors: Vec<(WlSurface, HookId)>,
    /// Some peer holds an `org.freedesktop.ScreenSaver` cookie. No surface
    /// stands behind it, so it counts whatever is on screen.
    /// Set by the tray's ScreenSaver service (`tray::screensaver`).
    pub bus_inhibited: bool,
    since_activity: Instant,
    /// The later of the last input and the end of the last fullscreen hold:
    /// the saver counts from here, so it does not start the moment a video
    /// leaves fullscreen.
    saver_since: Instant,
    /// What `inhibition_changed` last saw of `saver_held`.
    was_saver_held: bool,
    /// What `inhibition_changed` last told the notifier, so only a real
    /// change restarts the countdown.
    was_inhibited: bool,
    pub steps: Steps,
    /// `arm` ran since the last change to the steps: activity alone then
    /// leaves the timer as it is (it fires, finds nothing due and arms the
    /// remainder) instead of a new timer per input event.
    deadline_current: bool,
    dim: Vec<RectId>,
    dim_timer: Option<RegistrationToken>,
    dim_since: Instant,
    powered_off: Vec<Output>,
    /// The Console side's own clock in lock mode, which only the computer's input restarts.
    console: ConsoleIdle,
}

/// Lock mode's Console side idles on its own: only screen-off, after `screen_off_after`,
/// whatever the remote side runs or holds.
#[derive(Default)]
struct ConsoleIdle {
    since: Option<Instant>,
    /// The timer powered the Console outputs off.
    off: bool,
    timer: Option<RegistrationToken>,
}

impl IdleState {
    pub fn new(
        dh: &DisplayHandle,
        handle: smithay::reexports::calloop::LoopHandle<'static, State>,
    ) -> IdleState {
        IdleState {
            notifier: IdleNotifierState::new(dh, handle),
            notified_at: None,
            notify_timer: None,
            _inhibit: IdleInhibitManagerState::new::<State>(dh),
            output_power: OutputPowerState::new::<State, _>(
                dh,
                crate::sandbox::filter("zwlr_output_power_manager_v1"),
            ),
            timer: None,
            inhibitors: Vec::new(),
            bus_inhibited: false,
            since_activity: Instant::now(),
            saver_since: Instant::now(),
            was_saver_held: false,
            was_inhibited: false,
            steps: Steps::default(),
            deadline_current: false,
            dim: Vec::new(),
            dim_timer: None,
            dim_since: Instant::now(),
            powered_off: Vec::new(),
            console: ConsoleIdle::default(),
        }
    }
}

/// Whether `event` is the user's doing: everything but devices coming
/// and going and switches (a lid that closes is no activity; one that
/// opens wakes the outputs itself).
pub fn is_activity<B: InputBackend>(event: &InputEvent<B>) -> bool {
    !matches!(
        event,
        InputEvent::DeviceAdded { .. }
            | InputEvent::DeviceRemoved { .. }
            | InputEvent::SwitchToggle { .. }
            | InputEvent::Special(_)
    )
}

pub fn setup(state: &mut State) {
    notify_activity(state);
}

fn root_of(s: &WlSurface) -> WlSurface {
    let mut root = s.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// Whether an inhibitor on `s` counts: its surface shows, and for a
/// window, that window is mapped, not minimized and on the current
/// desktop.
fn shows(state: &State, s: &WlSurface) -> bool {
    if !s.is_alive() {
        return false;
    }
    let root = root_of(s);
    match state.window_for_surface(&root) {
        Some(w) => {
            state.win(w).is_some_and(|w| w.mapped && !w.hidden()) && state.on_current_desktop(w)
        }
        None => with_renderer_surface_state(&root, |st| st.buffer().is_some()).unwrap_or(false),
    }
}

/// Whether anything holds the idle timer off: an
/// `org.freedesktop.ScreenSaver` cookie, or a live idle-inhibit-v1
/// inhibitor whose surface shows. Nothing does while the desktop is
/// locked: the lock covers every surface, and a client cannot hold the
/// lock or the screen-off step off with something nobody can see.
fn inhibited(state: &State) -> bool {
    if state.desktop_locked() {
        return false;
    }
    state.idle.bus_inhibited || state.idle.inhibitors.iter().any(|(s, _)| shows(state, s))
}

/// Whether a fullscreen window shows, which holds the saver off. A video
/// player without an idle inhibitor counts; the desktop's lock covers it.
fn saver_held(state: &State) -> bool {
    !state.desktop_locked()
        && state
            .windows
            .iter()
            .any(|w| w.fullscreen && w.mapped && !w.hidden() && state.on_current_desktop(w.id))
}

/// The saver's elapsed ms, `None` while a fullscreen window holds it off.
fn saver_ms(state: &State) -> Option<u64> {
    (!saver_held(state)).then(|| state.idle.saver_since.elapsed().as_millis() as u64)
}

/// Tell idle-notify clients, and restart our own countdown from zero once
/// the last inhibitor goes away (time spent inhibited does not count).
/// Called when an inhibitor comes or goes, when a window maps, unmaps or
/// minimizes, when the desktop shown changes, and when the session locks
/// or unlocks, since those change which inhibitors count.
pub fn inhibition_changed(state: &mut State) {
    let held = saver_held(state);
    if held != state.idle.was_saver_held {
        log::info!(
            "idle: saver {}",
            if held {
                "held by a fullscreen window"
            } else {
                "released"
            }
        );
        state.idle.was_saver_held = held;
        if !held {
            state.idle.saver_since = Instant::now();
        }
        arm(state);
    }
    let inhibited = inhibited(state);
    if inhibited == state.idle.was_inhibited {
        return;
    }
    log::info!(
        "idle: {}",
        if inhibited {
            "inhibited"
        } else {
            "no longer inhibited"
        }
    );
    state.idle.was_inhibited = inhibited;
    state.idle.notifier.set_is_inhibited(inhibited);
    if !inhibited {
        state.idle.since_activity = Instant::now();
        state.idle.saver_since = Instant::now();
    }
    arm(state);
}

/// Called from every input path: lifts the blank and the dim, powers the
/// outputs back on, restarts the countdown. In lock mode the computer's own input restarts only
/// the Console side's clock, and the rest wakes only the Remote outputs.
pub fn notify_activity(state: &mut State) {
    if crate::console::physical(state) {
        console_activity(state);
        return;
    }
    notify_clients(state);
    let idle = &mut state.idle;
    idle.since_activity = Instant::now();
    idle.saver_since = idle.since_activity;
    let steps = &mut idle.steps;
    let mut changed = steps.locked_fired || steps.suspended || steps.dimming;
    steps.locked_fired = false;
    steps.suspended = false;
    steps.dimming = false;
    end_dim(state);
    if state.idle.steps.blanked {
        state.idle.steps.blanked = false;
        changed = true;
        blank(state, false);
    }
    // A saver Preview started is no idle step.
    crate::saverview::activity(state);
    if state.idle.steps.screen_off {
        state.idle.steps.screen_off = false;
        changed = true;
        set_outputs_powered(state, true);
    }
    if changed || !state.idle.deadline_current {
        arm(state);
    }
    let idle = state.idle.steps.is_idle();
    crate::power::hint(state, crate::power::Hint::Idle(idle));
}

/// The computer's own input in lock mode: its outputs come back on and its clock restarts.
fn console_activity(state: &mut State) {
    state.idle.console.since = Some(Instant::now());
    if std::mem::take(&mut state.idle.console.off) {
        power_outputs(state, console_outputs(state), true);
    }
    arm_console(state);
}

fn console_outputs(state: &State) -> Vec<Output> {
    (state.scene.outputs().iter())
        .filter(|o| crate::console::side_of(o) == crate::console::Side::Console)
        .cloned()
        .collect()
}

/// Lock mode began: the Console side's clock starts, dark if the outputs already were, and a
/// saver over every output goes, to come back on the Remote outputs alone.
pub fn lock_mode_began(state: &mut State) {
    state.idle.console.since = Some(Instant::now());
    state.idle.console.off = state.idle.steps.screen_off;
    if std::mem::take(&mut state.idle.steps.blanked) {
        blank(state, false);
    }
    arm_console(state);
    arm(state);
}

/// Lock mode ends: Console outputs its clock darkened wake on the next input of any kind.
pub fn lock_mode_ends(state: &mut State) {
    if let Some(t) = state.idle.console.timer.take() {
        state.handle.remove(t);
    }
    state.idle.console.since = None;
    if std::mem::take(&mut state.idle.console.off) {
        state.idle.steps.screen_off = true;
    }
}

/// The Console outputs go off until the computer's own input comes.
fn console_off(state: &mut State) {
    if let Some(t) = state.idle.console.timer.take() {
        state.handle.remove(t);
    }
    state.idle.console.off = true;
    power_outputs(state, console_outputs(state), false);
}

/// Whether the Console side's clock darkened its outputs.
#[cfg(test)]
pub fn console_dark(state: &State) -> bool {
    state.idle.console.off
}

fn arm_console(state: &mut State) {
    if let Some(t) = state.idle.console.timer.take() {
        state.handle.remove(t);
    }
    let after = state
        .config
        .idle_for(state.power.on_battery)
        .screen_off_after;
    let Some(since) = state.idle.console.since else {
        return;
    };
    if after == 0 || state.idle.console.off {
        return;
    }
    let left = Duration::from_secs(after.into()).saturating_sub(since.elapsed());
    let timer = Timer::from_duration(left.max(Duration::from_millis(1)));
    match state.handle.insert_source(timer, |_, _, state| {
        state.idle.console.timer = None;
        log::info!("idle: the computer's own outputs off");
        console_off(state);
        TimeoutAction::Drop
    }) {
        Ok(t) => state.idle.console.timer = Some(t),
        Err(e) => log::warn!("idle timer: {}", e.error),
    }
}

/// The machine woke from sleep. The countdown starts again, but a wake is no activity: the
/// outputs, the lock and idle-notify clients stay as they are until input comes.
pub fn resumed(state: &mut State) {
    let now = Instant::now();
    state.idle.since_activity = now;
    state.idle.saver_since = now;
    state.idle.steps.resumed();
    arm(state);
}

/// A power action's screen-off: the computer's own keys in lock mode darken the Console outputs,
/// anything else the desktop's.
pub fn screen_off_by(state: &mut State, origin: crate::lockview::Origin) {
    if origin == crate::lockview::Origin::Console && crate::console::remote_side(state) {
        console_off(state);
    } else {
        screen_off(state);
    }
}

/// Power the desktop's outputs off as the screen-off step, so the next input powers them back on.
pub fn screen_off(state: &mut State) {
    state.idle.steps.screen_off = true;
    set_outputs_powered(state, false);
    let idle = state.idle.steps.is_idle();
    crate::power::hint(state, crate::power::Hint::Idle(idle));
}

/// When activity reaches the idle-notify clients.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notify {
    Now,
    After(Duration),
    /// The armed timer carries it.
    Armed,
}

/// Activity `since` the clients last heard (None: never), with a timer
/// `armed` or not: at once when they last heard `NOTIFY_EVERY` ago or more,
/// else from a timer when that much has passed.
fn notify_step(since: Option<Duration>, armed: bool) -> Notify {
    // An armed timer stays even once due: its expiry may already be queued
    // in this dispatch, and calloop warns about an event for a removed source.
    if armed {
        return Notify::Armed;
    }
    match since.and_then(|t| NOTIFY_EVERY.checked_sub(t)) {
        None => Notify::Now,
        Some(left) => Notify::After(left),
    }
}

fn notify_clients(state: &mut State) {
    let since = state.idle.notified_at.map(|t| t.elapsed());
    let left = match notify_step(since, state.idle.notify_timer.is_some()) {
        Notify::Armed => return,
        Notify::Now => {
            state.idle.notified_at = Some(Instant::now());
            let seat = state.seat.clone();
            state.idle.notifier.notify_activity(&seat);
            return;
        }
        Notify::After(left) => left,
    };
    let timer = Timer::from_duration(left);
    match state.handle.insert_source(timer, |_, _, state| {
        state.idle.notify_timer = None;
        state.idle.notified_at = None;
        notify_clients(state);
        TimeoutAction::Drop
    }) {
        Ok(t) => state.idle.notify_timer = Some(t),
        Err(e) => log::warn!("idle notify timer: {}", e.error),
    }
}

/// A remote-control session (`State::remote_controlled`) holds off screen-off and suspend,
/// locked or not; a remote client elsewhere or a session off the seat holds off suspend.
fn config(state: &State) -> IdleConfig {
    let mut cfg = state.config.idle_for(state.power.on_battery);
    if state.remote_controlled() {
        cfg.screen_off_after = 0;
        cfg.suspend_after = 0;
    }
    if state.power.remote_elsewhere || !crate::backend::session_active(state) {
        cfg.suspend_after = 0;
    }
    cfg
}

pub fn arm(state: &mut State) {
    let elapsed = state.idle.since_activity.elapsed().as_millis() as u64;
    let next = state.idle.steps.next_deadline_ms(
        &config(state),
        elapsed,
        saver_ms(state),
        inhibited(state),
    );
    if let Some(t) = state.idle.timer.take() {
        state.handle.remove(t);
    }
    if let Some(ms) = next {
        let timer = Timer::from_duration(Duration::from_millis(ms.max(1)));
        match state.handle.insert_source(timer, |_, _, state| {
            state.idle.timer = None;
            fired(state);
            TimeoutAction::Drop
        }) {
            Ok(t) => state.idle.timer = Some(t),
            Err(e) => log::warn!("idle timer: {}", e.error),
        }
    }
    state.idle.deadline_current = true;
}

/// As if no input came for `by`: backdate the last activity and run the steps now due.
#[cfg(test)]
pub fn idle_for(state: &mut State, by: Duration) {
    state.idle.since_activity -= by;
    fired(state);
}

fn fired(state: &mut State) {
    state.idle.deadline_current = false;
    let elapsed = state.idle.since_activity.elapsed().as_millis() as u64;
    let cfg = config(state);
    let due = |after_s: u32| after_s > 0 && elapsed >= after_s as u64 * 1000;
    let saver_due = saver_ms(state).is_some_and(|ms| ms >= cfg.blank_after as u64 * 1000);
    if cfg.blank_after > 0 && saver_due && !state.idle.steps.blanked {
        state.idle.steps.blanked = true;
        log::info!("idle for {}s: blanking", cfg.blank_after);
        blank(state, true);
    }
    let dim_after = dim_after_ms(&cfg);
    if dim_after > 0 && !state.idle.steps.dimming && elapsed >= dim_after {
        state.idle.steps.dimming = true;
        if !state.console_locked() {
            log::info!("idle for {}s: dimming", dim_after / 1000);
            start_dim(state);
        }
    }
    if due(cfg.lock_after) && !state.idle.steps.locked_fired {
        state.idle.steps.locked_fired = true;
        end_dim(state);
        if !state.desktop_locked() {
            log::info!("idle for {}s: locking", cfg.lock_after);
            crate::lockview::lock_desktop(state);
        }
    }
    if due(cfg.screen_off_after) && !state.idle.steps.screen_off {
        log::info!("idle for {}s: outputs off", cfg.screen_off_after);
        screen_off(state);
    }
    if due(cfg.suspend_after) && !state.idle.steps.suspended {
        state.idle.steps.suspended = true;
        log::info!("idle for {}s: standby", cfg.suspend_after);
        crate::power::run_action(
            state,
            crate::config::PowerAction::Standby,
            crate::lockview::Origin::Console,
        );
    }
    let idle = state.idle.steps.is_idle();
    crate::power::hint(state, crate::power::Hint::Idle(idle));
    arm(state);
}

pub fn config_changed(state: &mut State) {
    arm(state);
    arm_console(state);
    crate::nightlight::update(state);
}

pub fn outputs_changed(state: &mut State) {
    let live: Vec<Output> = state.scene.outputs().to_vec();
    state.idle.powered_off.retain(|o| live.contains(o));
    refit_dim(state);
    crate::saverview::outputs_changed(state);
    crate::nightlight::outputs_changed(state);
    crate::power::outputs_changed(state);
}

/// The grey the desktop dims to before the lock. Scene rects take
/// premultiplied colour, so a partly faded rect is `DIM_GREY * t` at
/// alpha `t`.
const DIM_GREY: f32 = 0.5;
const DIM_TICK: Duration = Duration::from_millis(40);

fn start_dim(state: &mut State) {
    end_dim(state);
    state.idle.dim_since = Instant::now();
    state.idle.dim = cover_outputs(state, dim_color(0.0));
    let timer = state
        .handle
        .insert_source(Timer::from_duration(DIM_TICK), |_, _, state| {
            if state.idle.dim.is_empty() {
                state.idle.dim_timer = None;
                return TimeoutAction::Drop;
            }
            let t = dim_progress(state);
            for &r in &state.idle.dim {
                state.scene.rect_set_color(r, dim_color(t));
            }
            state.redraw_all();
            if t < 1.0 {
                TimeoutAction::ToDuration(DIM_TICK)
            } else {
                state.idle.dim_timer = None;
                TimeoutAction::Drop
            }
        });
    match timer {
        Ok(t) => state.idle.dim_timer = Some(t),
        Err(e) => log::warn!("idle dim timer: {}", e.error),
    }
    state.redraw_all();
}

/// Re-cover the outputs after a layout change while dimming: the rects are
/// rebuilt at the fade's current shade, so the clock keeps running. No-op
/// when nothing is dimming.
fn refit_dim(state: &mut State) {
    if state.idle.dim.is_empty() {
        return;
    }
    for r in std::mem::take(&mut state.idle.dim) {
        state.scene.destroy(r);
    }
    let t = dim_progress(state);
    state.idle.dim = cover_outputs(state, dim_color(t));
    state.redraw_all();
}

fn cover_outputs(state: &mut State, color: [f32; 4]) -> Vec<RectId> {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    let mut rects = Vec::new();
    for o in &outputs {
        let b = state.layout_box(Some(o));
        if b.width <= 0 {
            continue;
        }
        let r = state
            .scene
            .rect_create(state.layers.lock, (b.width, b.height), color);
        state.scene.set_position(r, b.x, b.y);
        rects.push(r);
    }
    rects
}

fn dim_progress(state: &State) -> f32 {
    let fade_ms = config(state).lock_fade as f32 * 1000.0;
    let elapsed = state.idle.dim_since.elapsed().as_millis() as f32;
    (elapsed / fade_ms.max(1.0)).min(1.0)
}

fn dim_color(t: f32) -> [f32; 4] {
    let g = DIM_GREY * t;
    [g, g, g, t]
}

pub fn end_dim(state: &mut State) {
    if state.idle.dim.is_empty() {
        return;
    }
    for r in std::mem::take(&mut state.idle.dim) {
        state.scene.destroy(r);
    }
    if let Some(t) = state.idle.dim_timer.take() {
        state.handle.remove(t);
    }
    state.redraw_all();
}

/// The screen saver step: `[screensaver] name` over every output, above
/// the lock screen, so a locked session shows it too, until the next
/// input (`saverview`).
fn blank(state: &mut State, on: bool) {
    if on {
        let kind = state.config.screensaver.kind;
        crate::saverview::start(state, kind, false);
    } else {
        crate::saverview::stop(state);
    }
}

fn set_output_powered(state: &mut State, output: &Output, on: bool) {
    if crate::backend::output_power(state, output) == Some(on) {
        return;
    }
    if !crate::backend::set_output_power(state, output, on) {
        log::warn!(
            "could not set {} power {}",
            output.name(),
            if on { "on" } else { "off" }
        );
        return;
    }
    if on {
        crate::nightlight::output_on(state, output);
    }
}

/// Switch the desktop's outputs (`console::on_desk`) off (or back on) for the idle timer. Only
/// the outputs the timer itself switched off come back on: an output a client switched off
/// must not light up on the next keystroke.
pub fn set_outputs_powered(state: &mut State, on: bool) {
    let outputs: Vec<Output> = (state.scene.outputs().iter())
        .filter(|o| crate::console::on_desk(state, o))
        .cloned()
        .collect();
    power_outputs(state, outputs, on);
}

fn power_outputs(state: &mut State, outputs: Vec<Output>, on: bool) {
    let pairs: Vec<(bool, bool)> = outputs
        .iter()
        .map(|o| {
            (
                crate::backend::output_power(state, o).unwrap_or(false),
                state.idle.powered_off.contains(o),
            )
        })
        .collect();
    let targets: Vec<Output> = power_targets(&pairs, on)
        .into_iter()
        .map(|i| outputs[i].clone())
        .collect();
    state.idle.powered_off.retain(|o| !outputs.contains(o));
    if !on {
        state.idle.powered_off.extend(targets.iter().cloned());
    }
    for o in &targets {
        set_output_powered(state, o, on);
    }
    OutputPowerState::refresh(state);
}

impl IdleNotifierHandler for State {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle.notifier
    }
}

impl IdleInhibitHandler for State {
    fn inhibit(&mut self, surface: WlSurface) {
        let hook = add_destruction_hook(&surface, |state: &mut State, surface| {
            state.idle.inhibitors.retain(|(s, _)| s != surface);
            log::info!(
                "idle inhibitor gone with its surface ({} active)",
                state.idle.inhibitors.len()
            );
            inhibition_changed(state);
        });
        self.idle.inhibitors.push((surface, hook));
        log::info!(
            "idle inhibitor added ({} active)",
            self.idle.inhibitors.len()
        );
        inhibition_changed(self);
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        if let Some(i) = self.idle.inhibitors.iter().position(|(s, _)| *s == surface) {
            let (s, hook) = self.idle.inhibitors.remove(i);
            remove_destruction_hook(&s, &hook);
        }
        log::info!(
            "idle inhibitor removed ({} active)",
            self.idle.inhibitors.len()
        );
        inhibition_changed(self);
    }
}

impl OutputPowerHandler for State {
    fn output_power_state(&mut self) -> &mut OutputPowerState {
        &mut self.idle.output_power
    }

    fn output_power(&mut self, output: &Output) -> Option<bool> {
        crate::backend::output_power(self, output)
    }

    fn set_output_power(&mut self, output: &Output, on: bool) {
        set_output_powered(self, output, on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_reaches_the_clients_at_most_every_notify_interval() {
        assert_eq!(notify_step(None, false), Notify::Now, "first activity");
        assert_eq!(
            notify_step(Some(Duration::from_millis(20)), false),
            Notify::After(Duration::from_millis(30))
        );
        assert_eq!(
            notify_step(Some(Duration::from_millis(80)), false),
            Notify::Now
        );
    }

    #[test]
    fn activity_with_a_timer_armed_leaves_it_to_the_timer_even_once_due() {
        assert_eq!(
            notify_step(Some(Duration::from_millis(20)), true),
            Notify::Armed
        );
        assert_eq!(
            notify_step(Some(Duration::from_millis(80)), true),
            Notify::Armed
        );
    }

    fn cfg(lock: u32, off: u32) -> IdleConfig {
        IdleConfig {
            lock_after: lock,
            screen_off_after: off,
            lock_fade: 0,
            blank_after: 0,
            suspend_after: 0,
        }
    }

    #[test]
    fn suspend_step_is_pending_like_the_others() {
        let c = IdleConfig {
            suspend_after: 30,
            ..cfg(0, 0)
        };
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 1000, Some(1000), false),
            Some(29_000)
        );
        assert_eq!(
            Steps {
                suspended: true,
                ..Steps::default()
            }
            .next_deadline_ms(&c, 1000, Some(1000), false),
            None
        );
        let mut woke = Steps {
            suspended: true,
            locked_fired: true,
            ..Steps::default()
        };
        woke.resumed();
        assert_eq!(
            woke.next_deadline_ms(&c, 0, Some(0), false),
            Some(30_000),
            "a wake makes standby pending again, a full timeout away"
        );
        assert!(woke.locked_fired, "a wake leaves the lock step as it was");
    }

    #[test]
    fn dim_step_precedes_the_lock_by_lock_fade() {
        let c = IdleConfig {
            lock_fade: 5,
            ..cfg(60, 0)
        };
        assert_eq!(dim_after_ms(&c), 55_000);
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 1000, Some(1000), false),
            Some(54_000)
        );
        assert_eq!(
            Steps {
                dimming: true,
                ..Steps::default()
            }
            .next_deadline_ms(&c, 56_000, Some(56_000), false),
            Some(4_000)
        );
        assert_eq!(
            dim_after_ms(&IdleConfig {
                lock_fade: 5,
                ..cfg(0, 0)
            }),
            0
        );
        assert_eq!(
            dim_after_ms(&IdleConfig {
                lock_fade: 90,
                ..cfg(60, 0)
            }),
            1
        );
    }

    #[test]
    fn earliest_pending_deadline_wins() {
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 0, Some(0), false),
            Some(600_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 100_000, Some(100_000), false),
            Some(500_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 30), 0, Some(0), false),
            Some(30_000)
        );
    }

    #[test]
    fn fired_deadlines_drop_out() {
        assert_eq!(
            Steps {
                locked_fired: true,
                ..Steps::default()
            }
            .next_deadline_ms(&cfg(600, 900), 600_000, Some(600_000), false),
            Some(300_000)
        );
        assert_eq!(
            Steps {
                locked_fired: true,
                screen_off: true,
                ..Steps::default()
            }
            .next_deadline_ms(&cfg(600, 900), 900_000, Some(900_000), false),
            None
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 700_000, Some(700_000), false),
            Some(0)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 1_000_000, Some(1_000_000), false),
            Some(0)
        );
        assert_eq!(
            Steps {
                screen_off: true,
                ..Steps::default()
            }
            .next_deadline_ms(&cfg(600, 30), 30_000, Some(30_000), false),
            Some(570_000)
        );
    }

    #[test]
    fn waking_only_touches_what_the_timer_switched_off() {
        let live = [(true, false), (false, false), (true, false)];
        assert_eq!(power_targets(&live, false), vec![0, 2]);
        let asleep = [(false, true), (false, false), (false, true)];
        assert_eq!(power_targets(&asleep, true), vec![0, 2]);
        let partly_awake = [(true, true), (false, false), (false, true)];
        assert_eq!(power_targets(&partly_awake, true), vec![2]);
        let never_slept = [(false, false), (false, false)];
        assert!(power_targets(&never_slept, true).is_empty());
    }

    #[test]
    fn any_fired_step_is_idle() {
        let mut s = Steps::default();
        assert!(!s.is_idle());
        s.screen_off = true;
        assert!(s.is_idle());
        s.screen_off = false;
        s.blanked = true;
        assert!(s.is_idle());
    }

    #[test]
    fn a_held_saver_leaves_the_lock_and_screen_off_on_their_timeouts() {
        let c = IdleConfig {
            blank_after: 60,
            ..cfg(600, 900)
        };
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 100_000, None, false),
            Some(500_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 100_000, Some(100_000), false),
            Some(0)
        );
    }

    #[test]
    fn the_saver_counts_from_the_end_of_its_hold() {
        let c = IdleConfig {
            blank_after: 60,
            ..cfg(600, 900)
        };
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 300_000, Some(10_000), false),
            Some(50_000)
        );
    }

    #[test]
    fn zero_disables_and_inhibit_pauses() {
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(0, 900), 0, Some(0), false),
            Some(900_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 0), 0, Some(0), false),
            Some(600_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(0, 0), 0, Some(0), false),
            None
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 0, Some(0), true),
            None
        );
    }

    mod remote {
        use crate::config::Config;
        use crate::eis::Kind;
        use crate::harness::Harness;

        fn laptop(lock_after: u32) -> Harness {
            let mut config = Config::default();
            config.idle.lock_after = lock_after;
            config.idle.screen_off_after = 2;
            config.idle.suspend_after = 3;
            config.idle.blank_after = 0;
            config.idle.lock_fade = 0;
            config.power.standby = String::new();
            let mut h = Harness::with_config(config);
            h.output("eDP-1", 800, 600);
            h
        }

        #[test]
        fn a_remote_sender_holds_screen_off_and_suspend() {
            let mut h = laptop(0);
            let peer = h.ei_remote_session();
            crate::idle::setup(&mut h.state);
            h.run_for(5000);
            assert!(!h.state.idle.steps.screen_off, "the outputs stay powered");
            assert!(!h.state.idle.steps.suspended, "no standby");

            crate::lockview::lock(&mut h.state);
            crate::idle::notify_activity(&mut h.state);
            h.run_for(5000);
            assert!(
                !h.state.idle.steps.screen_off,
                "locked, the outputs stay powered"
            );
            assert!(!h.state.idle.steps.suspended, "locked, no standby");

            h.ei_close(peer);
            h.run_for(100);
            assert!(
                h.state.idle.steps.screen_off,
                "the outputs go off once the sender is gone"
            );
            assert!(
                h.state.idle.steps.suspended,
                "standby once the sender is gone"
            );
        }

        #[test]
        fn an_inactive_session_never_suspends_on_idle() {
            let mut h = laptop(0);
            crate::idle::setup(&mut h.state);
            crate::backend::pause_session(&mut h.state);
            h.run_for(3500);
            assert!(!h.state.idle.steps.suspended, "no standby off the seat");
        }

        #[test]
        fn a_remote_client_elsewhere_holds_off_suspend_like_remote_control() {
            let mut h = laptop(0);
            crate::power::remote_elsewhere_changed(&mut h.state, true);
            crate::idle::setup(&mut h.state);
            h.run_for(3500);
            assert!(
                h.state.idle.steps.screen_off,
                "this console's outputs go off"
            );
            assert!(!h.state.idle.steps.suspended, "no standby");

            crate::power::remote_elsewhere_changed(&mut h.state, false);
            h.run_for(100);
            assert!(
                h.state.idle.steps.suspended,
                "standby once the client is gone"
            );
        }

        #[test]
        fn a_remote_session_starting_wakes_the_outputs() {
            let mut h = laptop(0);
            crate::idle::setup(&mut h.state);
            h.run_for(2500);
            assert!(h.state.idle.steps.screen_off, "the outputs go off at 2 s");
            assert!(!h.state.idle.steps.suspended, "no standby before 3 s");

            let _peer = h.ei_remote_session();
            assert!(
                !h.state.idle.steps.screen_off,
                "the outputs come back on as the session starts"
            );
            h.run_for(5000);
            assert!(!h.state.idle.steps.screen_off, "the outputs stay powered");
            assert!(!h.state.idle.steps.suspended, "no standby");
        }

        #[test]
        fn a_sender_other_than_the_remote_desktop_server_holds_nothing() {
            let mut h = laptop(0);
            let _peer = h.ei_keyboard(Kind::Sender);
            crate::idle::setup(&mut h.state);
            h.run_for(5000);
            assert!(h.state.idle.steps.screen_off, "the outputs go off");
            assert!(h.state.idle.steps.suspended, "standby");
        }

        #[test]
        fn a_remote_sender_does_not_hold_the_lock() {
            let mut h = laptop(2);
            let _peer = h.ei_remote_session();
            crate::idle::setup(&mut h.state);
            h.run_for(3000);
            assert!(h.state.console_locked());
        }
    }
}
