use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use smithay::{
    backend::session::Session,
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::compositor::State;

use super::links::{check_links, retrain_link};
use super::outputs::{Probe, scan_connectors, set_mode};
use super::render::{callbacks_after_a_frame, render_waiting};
use super::session::reset_device;
use super::{Backend, Flip, Head, commit, head_of, is_master};

impl Backend {
    /// No commit holds the kernel's modeset locks (`commit::quiet`).
    pub(super) fn quiet(&self) -> bool {
        commit::quiet(
            self.tracks(),
            !self.retiring.is_empty(),
            std::time::Instant::now(),
        )
    }

    /// No commit holds the modeset locks and no frame holds a compositor (`commit::modes_settle`).
    fn modes_settle(&self) -> bool {
        let all: Vec<_> = self.tracks().collect();
        commit::modes_settle(
            all.iter().copied(),
            !self.retiring.is_empty(),
            std::time::Instant::now(),
        )
    }

    /// Every output's track and the device reset's.
    fn tracks(&self) -> impl Iterator<Item = &commit::Track<Flip>> {
        let outputs = self.surfaces.values().map(|s| &s.track);
        outputs.chain(self.devices.iter().map(|(_, d)| &d.reset.track))
    }

    /// `head`'s output is held at `now` (`commit::held`).
    pub(super) fn held(&self, head: Head, now: std::time::Instant) -> bool {
        let Some(s) = self.surfaces.get(&head) else {
            return false;
        };
        let all: Vec<_> = self.tracks().collect();
        commit::held(&s.track, &all, !self.retiring.is_empty(), now)
    }

    /// The session is ours and its return has been handled (`reset_device`).
    pub(super) fn settled(&self) -> bool {
        self.session.is_active() && !self.devices.iter().any(|(_, d)| d.back.pending())
    }
}

/// Main is about to issue `op`, an ioctl that takes every modeset lock.
pub(super) fn gated(b: &Backend, op: &str) {
    debug_assert!(b.quiet(), "{op} while a commit holds the modeset locks");
}

/// Whether main may change outputs now (`commit::quiet`).
pub fn quiet(state: &State) -> bool {
    state.backend.as_ref().is_none_or(Backend::quiet)
}

/// Whether a mode change may go now; else a reload's output changes wait
/// (`defer_output_config`) and an output management apply is held.
pub fn modes_settle(state: &State) -> bool {
    state.backend.as_ref().is_none_or(Backend::modes_settle)
}

/// Keep the config from before a reload whose output changes wait (`modes_settle`); of several
/// reloads meanwhile, the first one's.
pub fn defer_output_config(state: &mut State, old: &crate::config::Config) {
    if let Some(b) = state.backend.as_mut() {
        log::info!("display busy: the configured output changes wait for it");
        b.config_from.get_or_insert_with(|| Box::new(old.clone()));
    }
}

/// `output`'s next frame waits past `commit::STALL` (`commit::held`): its own job or flip,
/// or another output's, has stalled.
pub fn stalled(state: &State, output: &Output) -> bool {
    let Some(b) = state.backend.as_ref() else {
        return false;
    };
    head_of(output).is_some_and(|c| b.held(c, std::time::Instant::now()))
}

/// Deferred work, in the order it runs: the device reset before the scan reads connectors
/// again, each connector's probe, the scan of what they read, outputs enabled again, modes a
/// changed connector list asked for while the display was busy, then the reloaded config's
/// output changes and the held output management applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Reset,
    Probe,
    Scan,
    Reenable,
    Mode,
    Config,
}

/// The next step. `rescan` asks for a new scan, which starts over at the first probe;
/// `unprobed` counts the connectors left to probe while one is under way; `mode` says a
/// `PendingMode` is due.
pub(super) fn next_step(
    reset: bool,
    rescan: bool,
    unprobed: Option<usize>,
    reenable: bool,
    mode: bool,
    config: bool,
) -> Option<Step> {
    match unprobed {
        _ if reset => Some(Step::Reset),
        _ if rescan => Some(Step::Probe),
        Some(0) => Some(Step::Scan),
        Some(_) => Some(Step::Probe),
        None if reenable => Some(Step::Reenable),
        None if mode => Some(Step::Mode),
        None if config => Some(Step::Config),
        None => None,
    }
}

/// Tries of a `PendingMode` before the output keeps its unlisted mode.
pub(super) const MODE_TRIES: u32 = 6;

/// A mode `set_mode` refused, tried again by `Step::Mode` once modes settle. A refusal there
/// backs the next try off like a failed modeset (`commit::backoff_after`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingMode<M> {
    pub mode: M,
    tries: u32,
    at: Option<Instant>,
}

impl<M> PendingMode<M> {
    pub fn new(mode: M) -> Self {
        PendingMode {
            mode,
            tries: 0,
            at: None,
        }
    }

    /// The next try may go at `now`.
    pub fn due(&self, now: Instant) -> bool {
        self.at.is_none_or(|at| now >= at)
    }

    /// When a backed-off try may go, while it waits.
    pub fn waits_until(&self, now: Instant) -> Option<Instant> {
        self.at.filter(|&at| at > now)
    }

    /// The try at `now` was refused: the next one and its wait, or None after `MODE_TRIES`.
    pub fn refused(self, now: Instant) -> Option<(Self, Duration)> {
        let tries = self.tries + 1;
        if tries >= MODE_TRIES {
            return None;
        }
        let wait = commit::backoff_after(self.tries);
        let next = PendingMode {
            tries,
            at: Some(now + wait),
            ..self
        };
        Some((next, wait))
    }
}

impl Backend {
    /// The step `on_quiet` may run now. The reset also waits for every job's `Done`, which
    /// calls `on_quiet` again.
    fn step_due(&self, outman: bool) -> Option<Step> {
        if !self.quiet() || !self.session.is_active() {
            return None;
        }
        let devices = || self.devices.iter().map(|(_, d)| d);
        let unprobed = (devices().filter_map(|d| d.probe.as_ref()))
            .map(|p| p.left.len())
            .reduce(|a, b| a + b);
        let step = next_step(
            devices().any(|d| d.back.ready()),
            devices().any(|d| d.rescan),
            unprobed,
            self.reenable.iter().any(|c| !self.retiring.contains_key(c)),
            self.mode_due(Instant::now()).is_some(),
            self.config_from.is_some() || outman,
        )?;
        match step {
            Step::Mode | Step::Config => self.modes_settle(),
            Step::Reset => !self.surfaces.values().any(|s| s.track.busy()),
            _ => true,
        }
        .then_some(step)
    }

    /// An output whose `PendingMode` may be tried at `now`.
    fn mode_due(&self, now: Instant) -> Option<Head> {
        self.surfaces
            .iter()
            .find(|(_, s)| s.pending_mode.is_some_and(|p| p.due(now)))
            .map(|(c, _)| *c)
    }

    /// The soonest a backed-off `PendingMode` may be tried.
    fn mode_waits_until(&self, now: Instant) -> Option<Instant> {
        self.surfaces
            .values()
            .filter_map(|s| s.pending_mode?.waits_until(now))
            .min()
    }
}

/// Run the device work that waited for quiet (`Backend::step_due`), one step per loop turn so
/// no turn holds clients' round trips for all of it. A step that sends a blocking job leaves
/// the rest for its `Done`.
pub(super) fn on_quiet(state: &mut State) {
    let outman = state.outputs_mgmt.holds();
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if b.step.is_none() {
        if b.step_due(outman).is_some() {
            step_after(state, Timer::immediate());
        } else if let Some(at) = b.mode_waits_until(Instant::now()) {
            step_after(state, Timer::from_deadline(at));
        }
    }
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let heads: Vec<Head> = b.surfaces.keys().copied().collect();
    for head in heads {
        retrain_link(state, head);
    }
    watch(state);
}

fn step_after(state: &mut State, timer: Timer) {
    let token = state.handle.insert_source(timer, |_, _, state| {
        if let Some(b) = state.backend.as_mut() {
            b.step = None;
        }
        quiet_step(state);
        on_quiet(state);
        TimeoutAction::Drop
    });
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    match token {
        Ok(t) => b.step = Some(t),
        Err(e) => log::warn!("deferred work: {}", e.error),
    }
}

fn quiet_step(state: &mut State) {
    use smithay::reexports::drm::control::Device as _;
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let Some(step) = b.step_due(state.outputs_mgmt.holds()) else {
        return;
    };
    match step {
        Step::Reset => {
            let now = std::time::Instant::now();
            let Some((node, wait)) = (b.devices.iter())
                .find(|(_, d)| d.back.ready())
                .map(|(node, d)| (node, d.back.master_wait(is_master(&d.drm), now)))
            else {
                return;
            };
            match wait {
                Some(poll) => step_after(state, Timer::from_duration(poll)),
                None => reset_device(state, node),
            }
        }
        Step::Probe => {
            let rescan = b.devices.iter().find(|(_, d)| d.rescan).map(|(n, _)| n);
            if let Some(node) = rescan {
                gated(b, "connector list");
                let Some(d) = b.devices.get_mut(node) else {
                    return;
                };
                let left = d
                    .drm
                    .resource_handles()
                    .map(|r| r.connectors().to_vec())
                    .unwrap_or_else(|e| {
                        log::warn!("connector scan: {e}");
                        Vec::new()
                    });
                d.rescan = false;
                d.probe = Some(Probe {
                    left,
                    infos: HashMap::new(),
                });
            }
            gated(b, "connector probe");
            let probing = (b.devices.iter_mut())
                .find(|(_, d)| d.probe.as_ref().is_some_and(|p| !p.left.is_empty()));
            let Some((_, d)) = probing else {
                return;
            };
            let Some(p) = d.probe.as_mut() else {
                return;
            };
            let Some(conn) = p.left.pop() else {
                return;
            };
            match d.drm.get_connector(conn, true) {
                Ok(info) => {
                    p.infos.insert(conn, info);
                }
                Err(e) => log::warn!("connector probe: {e}"),
            }
        }
        Step::Scan => {
            let probed: Vec<_> = (b.devices.iter_mut())
                .filter_map(|(node, d)| Some((node, d.probe.take()?.infos)))
                .collect();
            log::debug!("connector scan");
            for (node, infos) in probed {
                scan_connectors(state, node, &infos);
            }
            check_links(state);
        }
        Step::Reenable => {
            let Some(&head) = b.reenable.iter().find(|c| !b.retiring.contains_key(c)) else {
                return;
            };
            b.reenable.remove(&head);
            if let Some(output) = b.parked.get(&head).map(|p| p.output.clone()) {
                state.set_in_layout(&output, true);
            }
        }
        Step::Mode => {
            let now = Instant::now();
            let Some(head) = b.mode_due(now) else {
                return;
            };
            let Some(s) = b.surfaces.get_mut(&head) else {
                return;
            };
            let Some(pending) = s.pending_mode.take() else {
                return;
            };
            let output = s.output.clone();
            let Some(i) = s.connector.modes().iter().position(|m| *m == pending.mode) else {
                return;
            };
            if set_mode(state, &output, i) {
                state.update_layout();
                crate::outputmgmt::publish(state);
                return;
            }
            let Some(s) = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&head))
            else {
                return;
            };
            match pending.refused(now) {
                Some((next, wait)) => {
                    log::warn!(
                        "{}: mode change tried again in {} s",
                        output.name(),
                        wait.as_secs()
                    );
                    s.pending_mode = Some(next);
                }
                None => log::warn!(
                    "{}: keeps a mode its connector no longer lists",
                    output.name()
                ),
            }
        }
        Step::Config => {
            if let Some(old) = b.config_from.take()
                && state.apply_output_config(&old)
            {
                state.update_layout();
            }
            crate::outputmgmt::release_held(state);
        }
    }
}

/// How often `watch` looks for stalled outputs.
const WATCH_EVERY: Duration = Duration::from_millis(500);

/// Look for stalled outputs every `WATCH_EVERY` while any output has a job or flip out.
pub(super) fn watch(state: &mut State) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    if b.watch.is_some() || b.tracks().all(commit::Track::idle) {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(WATCH_EVERY), |_, _, state| {
            if watch_tick(state) {
                return TimeoutAction::ToDuration(WATCH_EVERY);
            }
            if let Some(b) = state.backend.as_mut() {
                b.watch = None;
            }
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => {
            if let Some(b) = state.backend.as_mut() {
                b.watch = Some(t);
            }
        }
        Err(e) => log::warn!("stall watch: {}", e.error),
    }
}

/// Label outputs past `commit::STALL`. On every held output (`commit::held`) lock and sleep
/// stop waiting for its cover and its clients get a frame callback each tick. False once
/// nothing is out.
fn watch_tick(state: &mut State) -> bool {
    let now = std::time::Instant::now();
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let mut newly = false;
    for (_, d) in b.devices.iter_mut() {
        if d.reset.track.stall(now) {
            log::warn!("the device reset is still in the kernel after 2 s");
            newly = true;
        }
    }
    for s in b.surfaces.values_mut() {
        if s.track.stall(now) {
            let name = s.output.name();
            match s.track.phase() {
                commit::Phase::Busy { kind, .. } => {
                    log::warn!("{name}: {kind:?} still in the kernel after 2 s")
                }
                _ => log::warn!("{name}: no vblank 2 s after its flip"),
            }
            newly = true;
        }
    }
    let throttled: Vec<_> = b
        .surfaces
        .iter()
        .filter(|(head, s)| s.callback_timer.is_none() && b.held(**head, now))
        .map(|(head, _)| *head)
        .collect();
    if newly {
        let held: Vec<Output> = state
            .scene
            .outputs()
            .iter()
            .filter(|o| stalled(state, o))
            .cloned()
            .collect();
        for output in &held {
            crate::sessionlock::stalled(state, output);
            crate::power::stalled(state, output);
        }
    }
    for head in throttled {
        callbacks_after_a_frame(state, head, false);
    }
    on_quiet(state);
    if quiet(state) {
        render_waiting(state);
    }
    state
        .backend
        .as_ref()
        .is_some_and(|b| b.tracks().any(|t| !t.idle()))
}
