use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::session::Session,
    output::Output,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::crtc,
    },
};

use crate::compositor::State;

use super::links::{check_links, retrain_link};
use super::outputs::{Probe, scan_connectors};
use super::render::{callbacks_after_a_frame, render_waiting};
use super::session::reset_device;
use super::{Backend, Flip, commit, crtc_of, is_master};

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
        outputs.chain(std::iter::once(&self.device.track))
    }

    /// `crtc`'s output is held at `now` (`commit::held`).
    pub(super) fn held(&self, crtc: crtc::Handle, now: std::time::Instant) -> bool {
        let Some(s) = self.surfaces.get(&crtc) else {
            return false;
        };
        let all: Vec<_> = self.tracks().collect();
        commit::held(&s.track, &all, !self.retiring.is_empty(), now)
    }

    /// The session is ours and its return has been handled (`reset_device`).
    pub(super) fn settled(&self) -> bool {
        self.session.is_active() && !self.back.pending()
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
    crtc_of(output).is_some_and(|c| b.held(c, std::time::Instant::now()))
}

/// Deferred work, in the order it runs: the device reset before the scan reads connectors
/// again, each connector's probe, the scan of what they read, outputs enabled again, then the
/// reloaded config's output changes and the held output management applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Step {
    Reset,
    Probe,
    Scan,
    Reenable,
    Config,
}

/// The next step. `rescan` asks for a new scan, which starts over at the first probe;
/// `unprobed` counts the connectors left to probe while one is under way.
pub(super) fn next_step(
    reset: bool,
    rescan: bool,
    unprobed: Option<usize>,
    reenable: bool,
    config: bool,
) -> Option<Step> {
    match unprobed {
        _ if reset => Some(Step::Reset),
        _ if rescan => Some(Step::Probe),
        Some(0) => Some(Step::Scan),
        Some(_) => Some(Step::Probe),
        None if reenable => Some(Step::Reenable),
        None if config => Some(Step::Config),
        None => None,
    }
}

impl Backend {
    /// The step `on_quiet` may run now. The reset also waits for every job's `Done`, which
    /// calls `on_quiet` again.
    fn step_due(&self, outman: bool) -> Option<Step> {
        if !self.quiet() || !self.session.is_active() {
            return None;
        }
        let step = next_step(
            self.back.ready(),
            self.rescan,
            self.probe.as_ref().map(|p| p.left.len()),
            self.reenable.iter().any(|c| !self.retiring.contains_key(c)),
            self.config_from.is_some() || outman,
        )?;
        match step {
            Step::Config => self.modes_settle(),
            Step::Reset => !self.surfaces.values().any(|s| s.track.busy()),
            _ => true,
        }
        .then_some(step)
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
    if b.step.is_none() && b.step_due(outman).is_some() {
        step_after(state, Timer::immediate());
    }
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let crtcs: Vec<crtc::Handle> = b.surfaces.keys().copied().collect();
    for crtc in crtcs {
        retrain_link(state, crtc);
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
        Step::Reset => match b
            .back
            .master_wait(is_master(&b.drm), std::time::Instant::now())
        {
            Some(poll) => step_after(state, Timer::from_duration(poll)),
            None => reset_device(state),
        },
        Step::Probe => {
            if std::mem::take(&mut b.rescan) {
                gated(b, "connector list");
                let left = b
                    .drm
                    .resource_handles()
                    .map(|r| r.connectors().to_vec())
                    .unwrap_or_else(|e| {
                        log::warn!("connector scan: {e}");
                        Vec::new()
                    });
                b.probe = Some(Probe {
                    left,
                    infos: HashMap::new(),
                });
            }
            gated(b, "connector probe");
            let Some(p) = b.probe.as_mut() else {
                return;
            };
            let Some(conn) = p.left.pop() else {
                return;
            };
            match b.drm.get_connector(conn, true) {
                Ok(info) => {
                    p.infos.insert(conn, info);
                }
                Err(e) => log::warn!("connector probe: {e}"),
            }
        }
        Step::Scan => {
            let probed = b.probe.take().map(|p| p.infos).unwrap_or_default();
            log::debug!("connector scan");
            scan_connectors(state, &probed);
            check_links(state);
        }
        Step::Reenable => {
            let Some(&crtc) = b.reenable.iter().find(|c| !b.retiring.contains_key(c)) else {
                return;
            };
            b.reenable.remove(&crtc);
            if let Some(output) = b.parked.get(&crtc).map(|p| p.output.clone()) {
                state.set_in_layout(&output, true);
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
    let mut newly = b.device.track.stall(now);
    if newly {
        log::warn!("the device reset is still in the kernel after 2 s");
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
        .filter(|(crtc, s)| s.callback_timer.is_none() && b.held(**crtc, now))
        .map(|(crtc, _)| *crtc)
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
    for crtc in throttled {
        callbacks_after_a_frame(state, crtc, false);
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
