//! ext-idle-notify-v1, idle-inhibit-v1, wlr-output-power-management-v1 and
//! the compositor's own idle timer: start the screen saver after
//! `blank_after` seconds of no input (`saverview`), lock after
//! `lock_after` (dimming the desktop to grey over the `lock_fade` seconds
//! before that), power every output off after `screen_off_after`, run the
//! standby command after `suspend_after`, all paused while the session is
//! unlocked and any client holds an idle inhibitor or an
//! `org.freedesktop.ScreenSaver` cookie.
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
    /// last input.
    pub fn next_deadline_ms(
        &self,
        cfg: &IdleConfig,
        elapsed_ms: u64,
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
            pending(s(cfg.blank_after), self.blanked),
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
            was_inhibited: false,
            steps: Steps::default(),
            deadline_current: false,
            dim: Vec::new(),
            dim_timer: None,
            dim_since: Instant::now(),
            powered_off: Vec::new(),
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
/// inhibitor whose surface shows. Nothing does while the session is
/// locked: the lock covers every surface, and a client cannot hold the
/// lock or the screen-off step off with something nobody can see.
fn inhibited(state: &State) -> bool {
    if state.is_locked() {
        return false;
    }
    state.idle.bus_inhibited || state.idle.inhibitors.iter().any(|(s, _)| shows(state, s))
}

/// Tell idle-notify clients, and restart our own countdown from zero once
/// the last inhibitor goes away (time spent inhibited does not count).
/// Called when an inhibitor comes or goes, when a window maps, unmaps or
/// minimizes, when the desktop shown changes, and when the session locks
/// or unlocks, since those change which inhibitors count.
pub fn inhibition_changed(state: &mut State) {
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
    }
    arm(state);
}

/// Called from every input path: lifts the blank and the dim, powers the
/// outputs back on, restarts the countdown.
pub fn notify_activity(state: &mut State) {
    notify_clients(state);
    let idle = &mut state.idle;
    idle.since_activity = Instant::now();
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

/// Tell the idle-notify clients of activity: at once when they last heard
/// `NOTIFY_EVERY` ago or more, else from a timer when that much has passed.
fn notify_clients(state: &mut State) {
    let left = (state.idle.notified_at).and_then(|t| NOTIFY_EVERY.checked_sub(t.elapsed()));
    let Some(left) = left else {
        if let Some(t) = state.idle.notify_timer.take() {
            state.handle.remove(t);
        }
        state.idle.notified_at = Some(Instant::now());
        let seat = state.seat.clone();
        state.idle.notifier.notify_activity(&seat);
        return;
    };
    if state.idle.notify_timer.is_some() {
        return;
    }
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

fn config(state: &State) -> IdleConfig {
    state.config.idle_for(state.power.on_battery)
}

pub fn arm(state: &mut State) {
    let elapsed = state.idle.since_activity.elapsed().as_millis() as u64;
    let next = state
        .idle
        .steps
        .next_deadline_ms(&config(state), elapsed, inhibited(state));
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

fn fired(state: &mut State) {
    state.idle.deadline_current = false;
    let elapsed = state.idle.since_activity.elapsed().as_millis() as u64;
    let cfg = config(state);
    let due = |after_s: u32| after_s > 0 && elapsed >= after_s as u64 * 1000;
    if due(cfg.blank_after) && !state.idle.steps.blanked {
        state.idle.steps.blanked = true;
        log::info!("idle for {}s: blanking", cfg.blank_after);
        blank(state, true);
    }
    let dim_after = dim_after_ms(&cfg);
    if dim_after > 0 && !state.idle.steps.dimming && elapsed >= dim_after {
        state.idle.steps.dimming = true;
        if !state.is_locked() {
            log::info!("idle for {}s: dimming", dim_after / 1000);
            start_dim(state);
        }
    }
    if due(cfg.lock_after) && !state.idle.steps.locked_fired {
        state.idle.steps.locked_fired = true;
        end_dim(state);
        if !state.is_locked() {
            log::info!("idle for {}s: locking", cfg.lock_after);
            crate::lockview::lock(state);
        }
    }
    if due(cfg.screen_off_after) && !state.idle.steps.screen_off {
        state.idle.steps.screen_off = true;
        log::info!("idle for {}s: outputs off", cfg.screen_off_after);
        set_outputs_powered(state, false);
    }
    if due(cfg.suspend_after) && !state.idle.steps.suspended {
        state.idle.steps.suspended = true;
        log::info!("idle for {}s: standby", cfg.suspend_after);
        crate::power::run_action(state, crate::config::PowerAction::Standby);
    }
    let idle = state.idle.steps.is_idle();
    crate::power::hint(state, crate::power::Hint::Idle(idle));
    arm(state);
}

pub fn config_changed(state: &mut State) {
    arm(state);
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

/// Switch outputs off (or back on) for the idle timer. Only the outputs the
/// timer itself switched off come back on: an output a client switched off
/// must not light up on the next keystroke.
pub fn set_outputs_powered(state: &mut State, on: bool) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
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
    if on {
        state.idle.powered_off.clear();
    } else {
        state.idle.powered_off = targets.clone();
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
            Steps::default().next_deadline_ms(&c, 1000, false),
            Some(29_000)
        );
        assert_eq!(
            Steps {
                suspended: true,
                ..Steps::default()
            }
            .next_deadline_ms(&c, 1000, false),
            None
        );
    }

    #[test]
    fn dim_step_precedes_the_lock_by_lock_fade() {
        let c = IdleConfig {
            lock_fade: 5,
            ..cfg(60, 0)
        };
        assert_eq!(dim_after_ms(&c), 55_000);
        assert_eq!(
            Steps::default().next_deadline_ms(&c, 1000, false),
            Some(54_000)
        );
        assert_eq!(
            Steps {
                dimming: true,
                ..Steps::default()
            }
            .next_deadline_ms(&c, 56_000, false),
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
            Steps::default().next_deadline_ms(&cfg(600, 900), 0, false),
            Some(600_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 100_000, false),
            Some(500_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 30), 0, false),
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
            .next_deadline_ms(&cfg(600, 900), 600_000, false),
            Some(300_000)
        );
        assert_eq!(
            Steps {
                locked_fired: true,
                screen_off: true,
                ..Steps::default()
            }
            .next_deadline_ms(&cfg(600, 900), 900_000, false),
            None
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 700_000, false),
            Some(0)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 1_000_000, false),
            Some(0)
        );
        assert_eq!(
            Steps {
                screen_off: true,
                ..Steps::default()
            }
            .next_deadline_ms(&cfg(600, 30), 30_000, false),
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
    fn zero_disables_and_inhibit_pauses() {
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(0, 900), 0, false),
            Some(900_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 0), 0, false),
            Some(600_000)
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(0, 0), 0, false),
            None
        );
        assert_eq!(
            Steps::default().next_deadline_ms(&cfg(600, 900), 0, true),
            None
        );
    }
}
