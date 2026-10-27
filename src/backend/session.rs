use std::time::Duration;

use smithay::{
    backend::{
        drm::DrmNode,
        input::{InputEvent, InputTime},
        libinput::LibinputInputBackend,
        session::Session,
    },
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::crtc,
    },
};

use crate::compositor::State;

use super::gamma::{flush_gamma, read_lut};
use super::outputs::power_off;
use super::render::render_waiting;
use super::steps::{gated, on_quiet, watch};
use super::{Ask, Backend, Head, commit, inputfds, is_master, lock, reset, schedule_all};

pub fn on_input(state: &mut State, mut event: InputEvent<LibinputInputBackend>) {
    match &mut event {
        InputEvent::DeviceAdded { device } => {
            state.pointers.added(device, &state.config);
            crate::tabletpad::added(state, device);
        }
        InputEvent::DeviceRemoved { device } => {
            state.pointers.removed(device);
            crate::tabletpad::removed(state, device);
        }
        _ => {}
    }
    state.process_input(event)
}

pub(crate) fn pause(state: &mut State) {
    state.cancel_drag();
    state.release_all_buttons(InputTime::now());
    state.stop_field_repeat();
    state.stop_media_repeat();
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session paused");
    if let Some(t) = b.input_back.take() {
        state.handle.remove(t);
    }
    b.libinput.suspend();
    for (_, d) in b.devices.iter_mut() {
        d.drm.pause();
    }
}

pub(super) fn activate(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session active");
    let now = std::time::Instant::now();
    for (_, d) in b.devices.iter_mut() {
        d.back.activated(now);
        if let Err(e) = d.drm.activate(false) {
            log::error!("drm activate: {e}");
        }
    }
    for s in b.surfaces.values_mut() {
        s.track.reset();
        s.hw.unlit = true;
    }
    on_quiet(state);
    for output in state.scene.outputs().to_vec() {
        crate::nightlight::output_on(state, &output);
    }
    schedule_all(state, Ask::Scene);
    input_back(state);
}

/// The wait before checking DRM master again on every device (`Return::master_wait`), or
/// `None` once each is master or has waited long enough.
pub(super) fn master_wait(b: &Backend) -> Option<Duration> {
    let now = std::time::Instant::now();
    (b.devices.iter())
        .filter_map(|(_, d)| d.back.master_wait(is_master(&d.drm), now))
        .min()
}

/// Releases the device files held through the pause one per loop turn, then resumes
/// libinput in a turn of its own: each release is a synchronous logind call and the resume
/// reopens every device, ~40 ms in one turn. The releases start once logind has handed DRM
/// master back (`Return::master_wait`); until then it answers them only when the switch ends.
pub(super) fn input_back(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if let Some(t) = b.input_back.take() {
        state.handle.remove(t);
    }
    let token = state
        .handle
        .insert_source(Timer::immediate(), |_, _, state| {
            let Some(b) = state.backend.as_mut() else {
                return TimeoutAction::Drop;
            };
            if let Some(poll) = master_wait(b) {
                return TimeoutAction::ToDuration(poll);
            }
            match inputfds::release_one(&mut b.session, &b.input_fds) {
                inputfds::Back::Released => return TimeoutAction::ToDuration(Duration::ZERO),
                inputfds::Back::Paused => {}
                inputfds::Back::Resume => {
                    if b.libinput.resume().is_err() {
                        log::warn!("libinput did not resume");
                    }
                    let libinput = b.libinput.clone();
                    crate::inputqueue::take_queued(state, libinput);
                }
            }
            if let Some(b) = state.backend.as_mut() {
                b.input_back = None;
            }
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => b.input_back = Some(t),
        Err(e) => log::error!("input resume timer: {e}"),
    }
}

/// Once the session is back, quiet and no commit is out: read back each of `node`'s CRTCs'
/// gamma, then send the device reset (`reset::plan`) to its `commit-device` thread;
/// `device_reset_done` finishes the outputs. Another master may have left a CRTC lit on a
/// connector one of ours now claims, which fails every later commit.
pub(super) fn reset_device(state: &mut State, node: DrmNode) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let busy = b.surfaces.values().any(|s| s.track.busy());
    let ready = b.devices.get(node).is_some_and(|d| d.back.ready());
    if !ready || !b.session.is_active() || busy || !b.quiet() {
        return;
    }
    gated(b, "device reset");
    let Some(d) = b.devices.get_mut(node) else {
        return;
    };
    let mut keep = Vec::new();
    for (&head, s) in b.surfaces.iter_mut().filter(|(h, _)| h.node == node) {
        // Another DRM master may have left its own LUT.
        if let Some(g) = s.hw.gamma {
            let ours = s.hw.lut.take();
            s.hw.lut = read_lut(&d.drm, head.crtc, &g, ours.clone());
            if s.hw.lut.is_none() && s.hw.wanted_lut.is_none() {
                s.hw.wanted_lut = ours;
            }
        }
        if !s.hw.off && s.lit_at.is_some_and(|t| d.back.after_return(t)) {
            keep.push(reset::Keep {
                crtc: head.crtc,
                connector: s.connector.handle(),
                primary: lock(&s.compositor).surface().plane(),
            });
        }
    }
    d.back.sent();
    d.reset
        .track
        .sent(commit::Kind::Reset, std::time::Instant::now());
    if !d
        .reset
        .thread
        .send(commit::Job::Run(commit::Kind::Reset, keep))
    {
        log::error!("the device reset thread is gone; resetting on the main thread");
        d.reset.track.unsent();
        d.back.done(true);
        if let Err(e) = d.drm.reset_state() {
            log::warn!("drm reset: {e}");
        }
        finish_reset(state, node);
    }
    watch(state);
}

/// `node`'s reset commit returned: unless the session went away or came back again
/// meanwhile, re-read each of its outputs' state, zero its buffers' ages and drop the frame
/// whose vblank `Track::reset` forgot, then let the outputs render.
pub(super) fn device_reset_done(
    state: &mut State,
    node: DrmNode,
    done: commit::Done<Vec<crtc::Handle>>,
) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.session.is_active();
    let Some(d) = b.devices.get_mut(node) else {
        return;
    };
    d.reset
        .track
        .done(done.result.is_ok(), std::time::Instant::now());
    let took = done.ended.saturating_duration_since(done.started);
    match &done.result {
        Ok(kept) => log::debug!("device reset in {took:?}, left lit {kept:?}"),
        Err(e) => log::warn!("drm reset: {e}"),
    }
    if d.back.done(active) {
        finish_reset(state, node);
        for crtc in done.result.unwrap_or_default() {
            let head = Head { node, crtc };
            let off = state
                .backend
                .as_ref()
                .and_then(|b| b.surfaces.get(&head))
                .is_some_and(|s| s.hw.off);
            if off {
                power_off(state, head);
            }
        }
    }
    on_quiet(state);
}

fn finish_reset(state: &mut State, node: DrmNode) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "output state read");
    for (_, s) in b.surfaces.iter().filter(|(h, _)| h.node == node) {
        // Held buffers may be kept: `due` renders nothing until the return is handled and every
        // render is queued in its own turn, so no pre-reset frame is queued after this.
        debug_assert!(!s.track.busy(), "a commit job outlived the device reset");
        let mut c = lock(&s.compositor);
        if let Err(e) = c.reset_state() {
            log::warn!("{}: reset state: {e}", s.output.name());
        }
        c.reset_buffer_ages_keep_held();
        if let Err(e) = c.frame_submitted() {
            log::warn!("{}: drop pending frame: {e}", s.output.name());
        }
    }
    let heads: Vec<Head> = (b.surfaces.keys().copied())
        .filter(|h| h.node == node)
        .collect();
    for head in heads {
        flush_gamma(state, head);
    }
    render_waiting(state);
}
