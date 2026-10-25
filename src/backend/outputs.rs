use std::{
    collections::HashMap,
    ffi::OsStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use smithay::{
    backend::{
        allocator::Modifier,
        drm::{DrmDevice, VrrSupport},
        session::Session,
    },
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::{ModeTypeFlags, connector, crtc},
    },
};
use smithay_drm_extras::{display_info, drm_scanner::DrmScanEvent};

use crate::{compositor::State, output::ModeInfo};

use super::gamma::{find_gamma, flush_gamma, read_lut};
use super::render::send_job;
use super::steps::{gated, watch};
use super::{
    Backend, COLOR_FORMATS, Compositor, Hw, Kms, OutputCrtc, Parked, Scanout, Surface, Work,
    commit, crtc_of, lock, schedule, surface_of,
};

pub struct Head {
    pub output: Output,
    pub enabled: bool,
    pub modes: Vec<ModeInfo>,
    pub current: Option<usize>,
    pub vrr_capable: bool,
    pub vrr_on: bool,
}

/// Whether a drm change uevent with this `HOTPLUG` value asks for a connector scan. The
/// kernel's hotplug and connector property events carry `HOTPLUG=1`; the synthetic change a VT
/// switch triggers so udev reapplies device access does not.
pub(super) fn asks_scan(hotplug: Option<&OsStr>) -> bool {
    hotplug.is_some_and(|v| v == "1")
}

/// Scan the connectors with `probed`'s infos for the ones probed already; the rest are probed
/// now.
pub(super) fn scan_connectors(
    state: &mut State,
    probed: &HashMap<connector::Handle, connector::Info>,
) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "connector scan");
    let dev = Probed {
        drm: &b.drm,
        infos: probed,
    };
    let events = match b.scanner.scan_connectors(&dev) {
        Ok(events) => events,
        Err(e) => {
            log::warn!("connector scan: {e}");
            return;
        }
    };
    for event in events {
        match event {
            DrmScanEvent::Connected {
                connector,
                crtc: Some(crtc),
            } => connector_connected(state, connector, crtc),
            DrmScanEvent::Disconnected {
                crtc: Some(crtc), ..
            } => connector_disconnected(state, crtc),
            DrmScanEvent::Connected {
                connector,
                crtc: None,
            } => log::warn!("{}: no free CRTC", connector_name(&connector)),
            _ => {}
        }
    }
}

/// The device, with connectors' infos from earlier probes in place of a forced probe now.
struct Probed<'a> {
    drm: &'a DrmDevice,
    infos: &'a HashMap<connector::Handle, connector::Info>,
}

impl std::os::fd::AsFd for Probed<'_> {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.drm.as_fd()
    }
}

impl smithay::reexports::drm::Device for Probed<'_> {}

impl smithay::reexports::drm::control::Device for Probed<'_> {
    fn get_connector(
        &self,
        handle: connector::Handle,
        force_probe: bool,
    ) -> std::io::Result<connector::Info> {
        match self.infos.get(&handle) {
            Some(info) => Ok(info.clone()),
            None => self.drm.get_connector(handle, force_probe),
        }
    }
}

/// A connector scan in steps: each connector's forced probe (an EDID read) on its own turn.
pub(super) struct Probe {
    pub(super) left: Vec<connector::Handle>,
    pub(super) infos: HashMap<connector::Handle, connector::Info>,
}

fn connector_name(c: &connector::Info) -> String {
    format!("{}-{}", c.interface().as_str(), c.interface_id())
}

pub(super) fn mode_infos(connector: &connector::Info) -> Vec<ModeInfo> {
    connector
        .modes()
        .iter()
        .map(|m| ModeInfo {
            width: i32::from(m.size().0),
            height: i32::from(m.size().1),
            refresh_mhz: WlMode::from(*m).refresh,
            preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
        })
        .collect()
}

fn connector_connected(state: &mut State, connector: connector::Info, crtc: crtc::Handle) {
    let name = connector_name(&connector);
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if connector.modes().is_empty() {
        log::warn!("{name}: no modes");
        return;
    }
    gated(b, "EDID read");
    let info = display_info::for_connector(&b.drm, connector.handle());
    let field = |f: Option<String>| f.unwrap_or_else(|| "Unknown".into());
    let (w_mm, h_mm) = connector.size().unwrap_or((0, 0));
    let output = Output::new(
        name,
        PhysicalProperties {
            size: (w_mm as i32, h_mm as i32).into(),
            subpixel: connector.subpixel().into(),
            make: field(info.as_ref().and_then(|i| i.make())),
            model: field(info.as_ref().and_then(|i| i.model())),
            serial_number: field(info.as_ref().and_then(|i| i.serial())),
        },
    );
    let modes = mode_infos(&connector);
    if let Some(i) = crate::output::pick_mode(&modes, None) {
        output.set_preferred(WlMode::from(connector.modes()[i]));
    }
    output.user_data().insert_if_missing(|| OutputCrtc(crtc));
    b.parked.insert(
        crtc,
        Parked {
            output: output.clone(),
            connector,
            vrr_capable: false,
        },
    );
    state.set_in_layout(&output, true);
}

fn connector_disconnected(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    let driven = b.surfaces.get(&crtc).map(|s| s.output.clone());
    if let Some(output) = driven {
        log::info!("output {} gone", output.name());
        state.leave_layout(&output);
    }
    if let Some(b) = state.backend.as_mut()
        && b.parked.remove(&crtc).is_some()
    {
        crate::outputmgmt::publish(state);
    }
}

pub fn heads(state: &State) -> Vec<Head> {
    let Some(b) = state.backend.as_ref() else {
        return Vec::new();
    };
    let current = |output: &Output, connector: &connector::Info| {
        let mode = output.current_mode()?;
        connector
            .modes()
            .iter()
            .position(|m| WlMode::from(*m) == mode)
    };
    let mut heads: Vec<Head> = b
        .surfaces
        .values()
        .map(|s| Head {
            output: s.output.clone(),
            enabled: true,
            modes: mode_infos(&s.connector),
            current: current(&s.output, &s.connector),
            vrr_capable: s.vrr_capable,
            vrr_on: s.vrr_on,
        })
        .chain(b.parked.values().map(|p| Head {
            output: p.output.clone(),
            enabled: false,
            modes: mode_infos(&p.connector),
            current: current(&p.output, &p.connector),
            vrr_capable: p.vrr_capable,
            vrr_on: false,
        }))
        .collect();
    heads.sort_by_key(|h| h.output.name());
    heads
}

pub fn enable(state: &mut State, output: &Output, index: usize) -> bool {
    let Some(crtc) = crtc_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let Some(p) = b.parked.get(&crtc) else {
        return false;
    };
    let Some(&drm_mode) = p.connector.modes().get(index) else {
        return false;
    };
    if b.retiring.contains_key(&crtc) {
        log::warn!(
            "{}: its last commit has not returned; enabling it once it does",
            output.name()
        );
        b.reenable.insert(crtc);
        return false;
    }
    if !b.quiet() {
        log::warn!("{}: display busy; enabling it once quiet", output.name());
        b.reenable.insert(crtc);
        return false;
    }
    gated(b, "output enable");
    let connector = p.connector.handle();
    let name = output.name();
    let mode = WlMode::from(drm_mode);
    let old = output.current_mode();
    output.change_current_state(Some(mode), None, None, None);
    let compositor = match new_compositor(b, crtc, drm_mode, connector, output) {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "{name}: cannot drive it at {}x{}@{:.3}: {e}",
                mode.size.w,
                mode.size.h,
                f64::from(mode.refresh) / 1000.0
            );
            if let Some(old) = old {
                output.change_current_state(Some(old), None, None, None);
            }
            return false;
        }
    };
    let vrr_capable = compositor
        .vrr_supported(connector)
        .is_ok_and(|v| v != VrrSupport::NotSupported);
    let vrr_on = compositor.vrr_enabled();
    let compositor = Arc::new(Mutex::new(Scanout::new(compositor)));
    let delay = b
        .delay
        .as_deref()
        .and_then(|v| commit::Delay::parse(v, &name));
    if let Some(d) = delay {
        log::warn!(
            "{name}: {} makes each commit wait {d:?}",
            commit::Delay::VAR
        );
    }
    let kms = Kms {
        compositor: Arc::clone(&compositor),
        fd: b.drm.device_fd().clone(),
        crtc,
    };
    let gamma = find_gamma(&b.drm, crtc);
    let lut = gamma.and_then(|g| read_lut(&b.drm, crtc, &g, None));
    let thread = match commit::Thread::spawn(&name, crtc, kms, delay, b.replies.clone()) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("{name}: no commit thread: {e}");
            if let Some(old) = old {
                output.change_current_state(Some(old), None, None, None);
            }
            return false;
        }
    };
    let Some(p) = b.parked.remove(&crtc) else {
        return false;
    };
    let global = output.create_global::<State>(&state.dh);
    log::info!(
        "output {name} {}x{}@{:.3} scale {} {:?}",
        mode.size.w,
        mode.size.h,
        f64::from(mode.refresh) / 1000.0,
        output.current_scale().fractional_scale(),
        output.current_transform()
    );
    b.surfaces.insert(
        crtc,
        Surface {
            output: output.clone(),
            connector: p.connector,
            vrr_capable,
            vrr_on,
            global,
            thread,
            compositor,
            track: commit::Track::default(),
            flight: None,
            presenting: None,
            lit_at: None,
            commit_times: Vec::new(),
            needs_redraw: true,
            scene_changed: true,
            backdrop_changed: true,
            pointer_only: false,
            pointer_shown: false,
            scene_seen: 0,
            last_blur: None,
            idle_scheduled: false,
            callback_timer: None,
            render_timer: None,
            step_timer: None,
            last_flip: None,
            pace: Default::default(),
            frames: 0,
            held: 0,
            cursor_frames: 0,
            counted_from: None,
            stats: Default::default(),
            damage_seen: None,
            hw: Hw {
                unlit: true,
                gamma,
                lut,
                ..Hw::default()
            },
        },
    );
    state.scene.add_output(output);
    schedule(state, output);
    true
}

/// A compositor for `crtc` at `mode`, retried with implicit modifiers when the explicit ones
/// leave no working primary plane. Its first frame commits the modeset.
fn new_compositor(
    b: &mut Backend,
    crtc: crtc::Handle,
    mode: smithay::reexports::drm::control::Mode,
    connector: connector::Handle,
    output: &Output,
) -> Result<Compositor, String> {
    let mut new = |implicit: bool| {
        let surface = b
            .drm
            .create_surface(crtc, mode, &[connector])
            .map_err(|e| e.to_string())?;
        let formats = b
            .render_formats
            .iter()
            .copied()
            .filter(|f| !implicit || f.modifier == Modifier::Invalid);
        Compositor::new(
            output,
            surface,
            None,
            b.allocator.clone(),
            b.exporter.clone(),
            COLOR_FORMATS,
            formats,
            b.drm.cursor_size(),
            Some(b.gbm.clone()),
        )
        .map_err(|e| e.to_string())
    };
    new(false).or_else(|e| {
        log::warn!("{}: {e}; trying implicit modifiers", output.name());
        new(true)
    })
}

pub fn disable(state: &mut State, output: &Output) -> bool {
    let Some(crtc) = crtc_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let Some(s) = b.surfaces.remove(&crtc) else {
        return false;
    };
    let Surface {
        output,
        connector,
        vrr_capable,
        global,
        mut thread,
        compositor,
        track,
        callback_timer,
        render_timer,
        step_timer,
        ..
    } = s;
    if track.busy() {
        log::info!(
            "{}: disabled with a commit out; its thread finishes it first",
            output.name()
        );
    }
    // The thread then holds the last reference: it drops the compositor (a blocking clear).
    drop(compositor);
    if let Some(h) = thread.exit() {
        b.retiring.insert(crtc, h);
    }
    b.parked.insert(
        crtc,
        Parked {
            output: output.clone(),
            connector,
            vrr_capable,
        },
    );
    for t in [callback_timer, render_timer, step_timer]
        .into_iter()
        .flatten()
    {
        state.handle.remove(t);
    }
    state.scene.remove_output(&output);
    state.captures.output_gone(&output);
    state.dh.disable_global::<State>(global.clone());
    let dh = state.dh.clone();
    state
        .handle
        .insert_source(
            Timer::from_duration(Duration::from_secs(10)),
            move |_, _, _| {
                dh.remove_global::<State>(global.clone());
                TimeoutAction::Drop
            },
        )
        .map_err(|e| log::warn!("output global removal: {}", e.error))
        .ok();
    true
}

pub fn set_mode(state: &mut State, output: &Output, index: usize) -> bool {
    let Some(crtc) = crtc_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let quiet = b.quiet();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    let Some(&drm_mode) = s.connector.modes().get(index) else {
        return false;
    };
    let mode = WlMode::from(drm_mode);
    if output.current_mode() == Some(mode) {
        return true;
    }
    if !quiet || s.track.busy() {
        log::warn!("{}: display busy, mode change refused", output.name());
        return false;
    }
    if let Err(e) = lock(&s.compositor).use_mode(drm_mode) {
        log::warn!(
            "{}: mode {}x{}@{:.3} refused: {e}",
            output.name(),
            mode.size.w,
            mode.size.h,
            f64::from(mode.refresh) / 1000.0
        );
        return false;
    }
    output.change_current_state(Some(mode), None, None, None);
    log::info!(
        "output {} now {}x{}@{:.3}",
        output.name(),
        mode.size.w,
        mode.size.h,
        f64::from(mode.refresh) / 1000.0
    );
    crate::capture::constraints_changed(state, output);
    schedule(state, output);
    true
}

pub fn set_vrr(state: &mut State, output: &Output, on: bool) -> bool {
    let Some(crtc) = crtc_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let quiet = b.quiet();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    if s.vrr_on == on {
        return true;
    }
    if on && !s.vrr_capable {
        log::warn!(
            "output {}: adaptive sync asked for, not supported",
            output.name()
        );
        return false;
    }
    if !quiet || s.track.busy() {
        log::warn!(
            "output {}: display busy, adaptive sync unchanged",
            output.name()
        );
        return false;
    }
    let result = lock(&s.compositor).use_vrr(on);
    match result {
        Ok(()) => {
            s.vrr_on = on;
            true
        }
        Err(e) => {
            log::warn!("output {}: adaptive sync: {e:?}", output.name());
            false
        }
    }
}

pub fn set_output_power(state: &mut State, output: &Output, on: bool) -> bool {
    let Some((b, crtc)) = surface_of(state, output) else {
        return false;
    };
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    if s.hw.off != on {
        return true;
    }
    if on {
        s.hw.off = false;
        s.track.power_on();
        s.hw.unlit = true;
        s.hw.link_tries = 0;
        log::info!("{}: power on", output.name());
        flush_gamma(state, crtc);
        schedule(state, output);
        return true;
    }
    if !s.track.power_off() {
        s.hw.off = true;
        log::info!("{}: power off once its commit is done", output.name());
        return true;
    }
    let off = power_off(state, crtc);
    watch(state);
    off
}

/// Mark `crtc` off and, when the session is ours and no device reset is out, have its thread
/// clear the CRTC.
pub(super) fn power_off(state: &mut State, crtc: crtc::Handle) -> bool {
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    // During the device reset, `device_reset_done` clears an output the reset left lit.
    let active = b.session.is_active() && !b.device.track.busy();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    if active {
        send_job(s, commit::Kind::Clear, Work::None);
    }
    s.hw.off = true;
    let timer = s.callback_timer.take();
    log::info!("{}: power off", s.output.name());
    if let Some(t) = timer {
        state.handle.remove(t);
    }
    true
}

pub fn output_power(state: &State, output: &Output) -> Option<bool> {
    let crtc = output.user_data().get::<OutputCrtc>()?.0;
    let s = state.backend.as_ref()?.surfaces.get(&crtc)?;
    Some(!s.hw.off)
}
