use std::time::Duration;

use smithay::{
    backend::{input::InputEvent, libinput::LibinputInputBackend, session::Session},
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
use super::{Ask, commit, inputfds, is_master, lock, reset, schedule_all};

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

pub(super) fn pause(state: &mut State) {
    state.cancel_drag();
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session paused");
    if let Some(t) = b.input_back.take() {
        state.handle.remove(t);
    }
    b.libinput.suspend();
    b.drm.pause();
}

pub(super) fn activate(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session active");
    b.back.activated(std::time::Instant::now());
    if let Err(e) = b.drm.activate(false) {
        log::error!("drm activate: {e}");
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
            if let Some(poll) = b
                .back
                .master_wait(is_master(&b.drm), std::time::Instant::now())
            {
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

/// Once the session is back, quiet and no commit is out: read back each CRTC's gamma, then
/// send the device reset (`reset::plan`) to the `commit-device` thread; `device_reset_done`
/// finishes the outputs. Another master may have left a CRTC lit on a connector one of ours now
/// claims, which fails every later commit.
pub(super) fn reset_device(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let busy = b.surfaces.values().any(|s| s.track.busy());
    if !b.back.ready() || !b.session.is_active() || busy || !b.quiet() {
        return;
    }
    gated(b, "device reset");
    let dev = &b.drm;
    let mut keep = Vec::new();
    for (&crtc, s) in b.surfaces.iter_mut() {
        // Another DRM master may have left its own LUT.
        if let Some(g) = s.hw.gamma {
            let ours = s.hw.lut.take();
            s.hw.lut = read_lut(dev, crtc, &g, ours.clone());
            if s.hw.lut.is_none() && s.hw.wanted_lut.is_none() {
                s.hw.wanted_lut = ours;
            }
        }
        if !s.hw.off && s.lit_at.is_some_and(|t| b.back.after_return(t)) {
            keep.push(reset::Keep {
                crtc,
                connector: s.connector.handle(),
                primary: lock(&s.compositor).surface().plane(),
            });
        }
    }
    b.back.sent();
    b.device
        .track
        .sent(commit::Kind::Reset, std::time::Instant::now());
    if !b
        .device
        .thread
        .send(commit::Job::Run(commit::Kind::Reset, keep))
    {
        log::error!("the device reset thread is gone; resetting on the main thread");
        b.device.track.unsent();
        b.back.done(true);
        if let Err(e) = b.drm.reset_state() {
            log::warn!("drm reset: {e}");
        }
        finish_reset(state);
    }
    watch(state);
}

/// The device reset's commit returned: unless the session went away or came back again
/// meanwhile, re-read each output's state, zero its buffers' ages and drop the frame whose
/// vblank `Track::reset` forgot, then let the outputs render.
pub(super) fn device_reset_done(state: &mut State, done: commit::Done<Vec<crtc::Handle>>) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    b.device
        .track
        .done(done.result.is_ok(), std::time::Instant::now());
    let took = done.ended.saturating_duration_since(done.started);
    match &done.result {
        Ok(kept) => log::debug!("device reset in {took:?}, left lit {kept:?}"),
        Err(e) => log::warn!("drm reset: {e}"),
    }
    if b.back.done(b.session.is_active()) {
        finish_reset(state);
        for crtc in done.result.unwrap_or_default() {
            let off = state
                .backend
                .as_ref()
                .and_then(|b| b.surfaces.get(&crtc))
                .is_some_and(|s| s.hw.off);
            if off {
                power_off(state, crtc);
            }
        }
    }
    on_quiet(state);
}

fn finish_reset(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "output state read");
    for s in b.surfaces.values() {
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
    let crtcs: Vec<crtc::Handle> = b.surfaces.keys().copied().collect();
    for crtc in crtcs {
        flush_gamma(state, crtc);
    }
    render_waiting(state);
}
