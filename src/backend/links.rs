use smithay::{
    backend::drm::DrmDevice,
    reexports::drm::control::{connector, crtc},
};

use crate::{compositor::State, config::OutputMode, output::ModeInfo};

use super::outputs::mode_infos;
use super::render::send_job;
use super::steps::gated;
use super::{CrtcNow, Work, commit, set_mode, surface_mut};

/// DRM_MODE_LINK_STATUS_GOOD and _BAD.
pub(super) const LINK_GOOD: u64 = 0;

const LINK_BAD: u64 = 1;

/// Retrains in a row that may leave the link BAD before they stop until it reads GOOD or the output comes on.
pub(super) const LINK_TRIES: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LinkStep {
    Good,
    Retrain,
    /// The last allowed retrain left it BAD: say so once.
    GiveUp,
    GaveUp,
}

pub(super) fn link_step(bad: bool, tries: u8) -> LinkStep {
    match (bad, tries) {
        (false, _) => LinkStep::Good,
        (true, t) if t < LINK_TRIES => LinkStep::Retrain,
        (true, LINK_TRIES) => LinkStep::GiveUp,
        (true, _) => LinkStep::GaveUp,
    }
}

fn link_status(
    dev: &DrmDevice,
    conn: connector::Handle,
) -> Option<(smithay::reexports::drm::control::property::Handle, u64)> {
    use smithay::reexports::drm::control::Device as _;
    let props = dev.get_properties(conn).ok()?;
    let (handles, values) = props.as_props_and_values();
    handles.iter().zip(values).find_map(|(&h, &v)| {
        let info = dev.get_property(h).ok()?;
        (info.name().to_str() == Ok("link-status")).then_some((h, v))
    })
}

/// The mode a BAD link trains with: None while `current` is still among `modes`, else the
/// configured mode `want` if offered, the current size at its nearest refresh, or the
/// preferred mode.
pub(super) fn link_mode(
    modes: &[ModeInfo],
    current: &ModeInfo,
    want: Option<&OutputMode>,
) -> Option<usize> {
    let same = |m: &ModeInfo| {
        (m.width, m.height, m.refresh_mhz) == (current.width, current.height, current.refresh_mhz)
    };
    if modes.iter().any(same) {
        return None;
    }
    let size = OutputMode {
        width: current.width,
        height: current.height,
        refresh_mhz: Some(current.refresh_mhz),
    };
    want.and_then(|w| w.pick(modes))
        .or_else(|| size.pick(modes))
        .or_else(|| crate::output::pick_mode(modes, None))
}

/// On a hotplug uevent the kernel expects every connector whose link-status went BAD to get a modeset with
/// link-status GOOD, which trains the link again (drm-kms "link-status"): with the same mode, or a new
/// one when the scan no longer lists it (`link_mode`), which is set first.
pub(super) fn check_links(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "link-status read");
    let dev = &b.drm;
    let mut due = Vec::new();
    let mut switch = Vec::new();
    for (&crtc, s) in b.surfaces.iter_mut() {
        let status = link_status(dev, s.connector.handle()).map(|(_, v)| v);
        log::debug!("{}: link-status {status:?}", s.output.name());
        match link_step(status == Some(LINK_BAD), s.hw.link_tries) {
            LinkStep::Good => {
                s.hw.link_tries = 0;
                s.hw.retrain = false;
            }
            LinkStep::Retrain => {
                log::warn!("{}: link-status BAD; retraining", s.output.name());
                s.hw.retrain = true;
                let fresh = b.scanner.connectors().get(&s.connector.handle());
                let want = state.config.mode_for_output_name(&s.output.name());
                let mode = fresh.zip(s.output.current_mode()).and_then(|(info, m)| {
                    let current = ModeInfo {
                        width: m.size.w,
                        height: m.size.h,
                        refresh_mhz: m.refresh,
                        preferred: false,
                    };
                    link_mode(&mode_infos(info), &current, want.as_ref())
                });
                match (fresh, mode) {
                    (Some(info), Some(i)) => {
                        s.connector = info.clone();
                        switch.push((crtc, s.output.clone(), i));
                    }
                    _ => due.push(crtc),
                }
            }
            LinkStep::GiveUp => {
                log::warn!(
                    "{}: link-status still BAD after {LINK_TRIES} retrains; leaving it",
                    s.output.name()
                );
                s.hw.link_tries += 1;
                s.hw.retrain = false;
            }
            LinkStep::GaveUp => s.hw.retrain = false,
        }
    }
    for (crtc, output, i) in switch {
        log::warn!(
            "{}: its mode is gone from the connector; taking another",
            output.name()
        );
        if !set_mode(state, &output, i) {
            due.push(crtc);
            continue;
        }
        if let Some(s) = surface_mut(state, crtc) {
            s.hw.unlit = true;
        }
        state.update_layout();
    }
    for crtc in due {
        retrain_link(state, crtc);
    }
}

/// Set a BAD link-status GOOD once the CRTC is ready: a blocking modeset with the current mode.
pub(super) fn retrain_link(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.settled();
    let quiet = b.quiet();
    let dev = &b.drm;
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return;
    };
    // Reading link-status takes every modeset lock: it waits for quiet (`on_quiet`).
    if !s.hw.retrain || !quiet || !CrtcNow::of(active, s).ready() {
        return;
    }
    s.hw.retrain = false;
    let conn = s.connector.handle();
    let Some((prop, LINK_BAD)) = link_status(dev, conn) else {
        return;
    };
    s.hw.link_tries += 1;
    send_job(s, commit::Kind::Retrain, Work::Retrain { conn, prop });
}
