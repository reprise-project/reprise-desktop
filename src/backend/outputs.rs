use std::{
    collections::HashMap,
    ffi::OsStr,
    sync::{Arc, Mutex},
    time::Duration,
};

use smithay::{
    backend::{
        allocator::Modifier,
        drm::{DrmDevice, DrmNode, VrrSupport},
        session::Session,
    },
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::{ModeTypeFlags, connector},
    },
};
use smithay_drm_extras::{display_info, drm_scanner::DrmScanEvent};

use crate::{compositor::State, output::ModeInfo};

use super::gamma::{find_gamma, flush_gamma, read_lut};
use super::render::send_job;
use super::steps::{PendingMode, gated, on_quiet, watch};
use super::{
    Backend, COLOR_FORMATS, Compositor, Head, Hw, Kms, OutputHead, Parked, Scanout, Surface, Work,
    commit, head_of, lock, schedule, surface_of,
};

pub struct HeadState {
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
    node: DrmNode,
    probed: &HashMap<connector::Handle, connector::Info>,
) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "connector scan");
    let Some(d) = b.devices.get_mut(node) else {
        return;
    };
    let dev = Probed {
        drm: &d.drm,
        infos: probed,
    };
    let events = match d.scanner.scan_connectors(&dev) {
        Ok(events) => events,
        Err(e) => {
            log::warn!("connector scan: {e}");
            return;
        }
    };
    for event in events {
        let (scan, connector, crtc) = match event {
            DrmScanEvent::Connected { connector, crtc } => (Scan::Connected, connector, crtc),
            DrmScanEvent::Changed { connector, crtc } => (Scan::Changed, connector, crtc),
            DrmScanEvent::Disconnected { connector, crtc } => (Scan::Disconnected, connector, crtc),
        };
        match crtc {
            Some(crtc) => apply_scan(state, scan, connector, Head { node, crtc }),
            None if scan == Scan::Connected => {
                log::warn!("{}: no free CRTC", connector_name(&connector));
            }
            None => {}
        }
    }
    for (connector, head) in unheld(state, node) {
        apply_scan(state, Scan::Connected, connector, head);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scan {
    Connected,
    /// The connector's mode list changed while it stayed connected.
    Changed,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScanAction {
    /// A new output, parked and put into the layout.
    Connect,
    /// A new output, parked only: the connector lists no modes yet.
    Park,
    /// Take the connector's new mode list on the output already holding the CRTC.
    Refresh,
    Disconnect,
    Ignore,
}

/// What a scan event with a CRTC does, given whether that CRTC is driven or parked.
pub(super) fn scan_action(driven: bool, parked: bool, scan: Scan, modes: bool) -> ScanAction {
    let new = if modes {
        ScanAction::Connect
    } else {
        ScanAction::Park
    };
    match scan {
        Scan::Disconnected => ScanAction::Disconnect,
        Scan::Changed if driven || parked => ScanAction::Refresh,
        Scan::Connected | Scan::Changed if !driven && !parked => new,
        Scan::Connected | Scan::Changed => ScanAction::Ignore,
    }
}

fn apply_scan(state: &mut State, scan: Scan, connector: connector::Info, head: Head) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    let driven = b.surfaces.contains_key(&head);
    let parked = b.parked.contains_key(&head);
    match scan_action(driven, parked, scan, !connector.modes().is_empty()) {
        ScanAction::Connect => connector_connected(state, connector, head, true),
        ScanAction::Park => connector_connected(state, connector, head, false),
        ScanAction::Refresh => connector_changed(state, connector, head),
        ScanAction::Disconnect => connector_disconnected(state, head),
        ScanAction::Ignore => {}
    }
}

/// Connected connectors with a CRTC that no output holds: one whose CRTC another output freed
/// after it was first seen, which brings no new scan event.
fn unheld(state: &State, node: DrmNode) -> Vec<(connector::Info, Head)> {
    let Some(b) = state.backend.as_ref() else {
        return Vec::new();
    };
    let Some(d) = b.devices.get(node) else {
        return Vec::new();
    };
    let held = |c: connector::Handle| {
        let surfaces = b.surfaces.iter().map(|(h, s)| (h, &s.connector));
        (surfaces.chain(b.parked.iter().map(|(h, p)| (h, &p.connector))))
            .any(|(h, i)| h.node == node && i.handle() == c)
    };
    let head = |crtc| Head { node, crtc };
    d.scanner
        .connectors()
        .iter()
        .filter(|(_, info)| info.state() == connector::State::Connected)
        .filter_map(|(&handle, info)| Some((info, head(d.scanner.crtc_for_connector(&handle)?))))
        .filter(|(info, head)| {
            !b.surfaces.contains_key(head) && !b.parked.contains_key(head) && !held(info.handle())
        })
        .map(|(info, head)| (info.clone(), head))
        .collect()
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

/// Park a new output for `connector` on `head`, and put it into the layout when `into_layout`.
fn connector_connected(
    state: &mut State,
    connector: connector::Info,
    head: Head,
    into_layout: bool,
) {
    let name = connector_name(&connector);
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    gated(b, "EDID read");
    let Some(d) = b.devices.of(head) else {
        return;
    };
    let info = display_info::for_connector(&d.drm, connector.handle());
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
    set_preferred(&output, &connector);
    output.user_data().insert_if_missing(|| OutputHead(head));
    b.parked.insert(
        head,
        Parked {
            output: output.clone(),
            connector,
            vrr_capable: false,
        },
    );
    if !b.devices.is_primary(head.node) {
        log::info!(
            "{} on {}: parked; only the GPU that renders drives monitors",
            output.name(),
            head.node
        );
    } else if into_layout {
        state.set_in_layout(&output, true);
    } else {
        log::debug!("{}: no modes yet; parked", output.name());
        crate::outputmgmt::publish(state);
    }
}

fn set_preferred(output: &Output, connector: &connector::Info) {
    if let Some(i) = crate::output::pick_mode(&mode_infos(connector), None) {
        output.set_preferred(WlMode::from(connector.modes()[i]));
    }
}

/// Give the output holding `head` the connector's new mode list. A driven output whose mode is
/// gone takes another (`link_mode`); a parked one that had no modes is connected afresh, which
/// also reads the EDID its first probe lacked.
fn connector_changed(state: &mut State, connector: connector::Info, head: Head) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if let Some(s) = b.surfaces.get_mut(&head) {
        let output = s.output.clone();
        log::info!("{}: mode list changed", output.name());
        set_preferred(&output, &connector);
        let modes = mode_infos(&connector);
        s.connector = connector;
        let want = state.config.mode_for_output_name(&output.name());
        let switch = output.current_mode().and_then(|m| {
            let current = ModeInfo {
                width: m.size.w,
                height: m.size.h,
                refresh_mhz: m.refresh,
                preferred: false,
            };
            super::links::link_mode(&modes, &current, want.as_ref())
        });
        s.pending_mode = None;
        if let Some(i) = switch {
            log::warn!(
                "{}: its mode is gone from the connector; taking another",
                output.name()
            );
            if set_mode(state, &output, i) {
                state.update_layout();
            } else if let Some(s) = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&crtc))
            {
                log::warn!(
                    "{}: keeps a mode its connector no longer lists until the display is idle",
                    output.name()
                );
                s.pending_mode = Some(PendingMode::new(s.connector.modes()[i]));
                on_quiet(state);
            }
        }
        crate::outputmgmt::publish(state);
        return;
    }
    if let Some(p) = b.parked.get_mut(&head) {
        if !p.connector.modes().is_empty() {
            set_preferred(&p.output, &connector);
            p.connector = connector;
            crate::outputmgmt::publish(state);
            return;
        }
        b.parked.remove(&head);
    }
    let into_layout = !connector.modes().is_empty();
    connector_connected(state, connector, head, into_layout);
}

pub(super) fn connector_disconnected(state: &mut State, head: Head) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    let driven = b.surfaces.get(&head).map(|s| s.output.clone());
    if let Some(output) = driven {
        log::info!("output {} gone", output.name());
        state.leave_layout(&output);
    }
    if let Some(b) = state.backend.as_mut()
        && b.parked.remove(&head).is_some()
    {
        crate::outputmgmt::publish(state);
    }
}

pub fn heads(state: &State) -> Vec<HeadState> {
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
    let mut heads: Vec<HeadState> = b
        .surfaces
        .values()
        .map(|s| HeadState {
            output: s.output.clone(),
            enabled: true,
            modes: mode_infos(&s.connector),
            current: current(&s.output, &s.connector),
            vrr_capable: s.vrr_capable,
            vrr_on: s.vrr_on,
        })
        .chain(
            (b.parked.iter())
                .filter(|(h, _)| b.devices.is_primary(h.node))
                .map(|(_, p)| HeadState {
                    output: p.output.clone(),
                    enabled: false,
                    modes: mode_infos(&p.connector),
                    current: current(&p.output, &p.connector),
                    vrr_capable: p.vrr_capable,
                    vrr_on: false,
                }),
        )
        .collect();
    heads.sort_by_key(|h| h.output.name());
    heads
}

pub fn enable(state: &mut State, output: &Output, index: usize) -> bool {
    let Some(head) = head_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let Some(p) = b.parked.get(&head) else {
        return false;
    };
    let Some(&drm_mode) = p.connector.modes().get(index) else {
        return false;
    };
    if !b.devices.is_primary(head.node) {
        log::warn!("{}: its GPU does not render; it stays off", output.name());
        return false;
    }
    if b.retiring.contains_key(&head) {
        log::warn!(
            "{}: its last commit has not returned; enabling it once it does",
            output.name()
        );
        b.reenable.insert(head);
        return false;
    }
    if !b.quiet() {
        log::warn!("{}: display busy; enabling it once quiet", output.name());
        b.reenable.insert(head);
        return false;
    }
    gated(b, "output enable");
    let connector = p.connector.handle();
    let name = output.name();
    let mode = WlMode::from(drm_mode);
    let old = output.current_mode();
    output.change_current_state(Some(mode), None, None, None);
    let compositor = match new_compositor(b, head, drm_mode, connector, output) {
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
    let Some(d) = b.devices.of(head) else {
        return false;
    };
    let kms = Kms {
        compositor: Arc::clone(&compositor),
        fd: d.drm.device_fd().clone(),
        crtc: head.crtc,
    };
    let gamma = find_gamma(&d.drm, head.crtc);
    let lut = gamma.and_then(|g| read_lut(&d.drm, head.crtc, &g, None));
    let thread = match commit::Thread::spawn(&name, head, kms, delay, b.replies.clone()) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("{name}: no commit thread: {e}");
            if let Some(old) = old {
                output.change_current_state(Some(old), None, None, None);
            }
            return false;
        }
    };
    let Some(p) = b.parked.remove(&head) else {
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
        head,
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
            backdrop_changed: crate::glass::Under::Anywhere,
            pointer_only: false,
            catch_up: false,
            pointer_shown: false,
            scene_seen: 0,
            last_blur: None,
            asked_at: None,
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
            pending_mode: None,
        },
    );
    state.scene.add_output(output);
    schedule(state, output);
    true
}

/// A compositor for `head` at `mode`, retried with implicit modifiers when the explicit ones
/// leave no working primary plane. Its first frame commits the modeset.
fn new_compositor(
    b: &mut Backend,
    head: Head,
    mode: smithay::reexports::drm::control::Mode,
    connector: connector::Handle,
    output: &Output,
) -> Result<Compositor, String> {
    let d = b.devices.get_mut(head.node).ok_or("its GPU is gone")?;
    let mut new = |implicit: bool| {
        let surface = d
            .drm
            .create_surface(head.crtc, mode, &[connector])
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
            d.allocator.clone(),
            d.exporter.clone(),
            COLOR_FORMATS,
            formats,
            d.drm.cursor_size(),
            Some(d.gbm.clone()),
        )
        .map_err(|e| e.to_string())
    };
    new(false).or_else(|e| {
        log::warn!("{}: {e}; trying implicit modifiers", output.name());
        new(true)
    })
}

pub fn disable(state: &mut State, output: &Output) -> bool {
    let Some(head) = head_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let Some(s) = b.surfaces.remove(&head) else {
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
        b.retiring.insert(head, h);
    }
    b.parked.insert(
        head,
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
    let Some(head) = head_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let quiet = b.quiet();
    let Some(s) = b.surfaces.get_mut(&head) else {
        return false;
    };
    let Some(&drm_mode) = s.connector.modes().get(index) else {
        return false;
    };
    let mode = WlMode::from(drm_mode);
    if output.current_mode() == Some(mode) {
        s.pending_mode = None;
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
    s.pending_mode = None;
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
    let Some(head) = head_of(output) else {
        return false;
    };
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    let quiet = b.quiet();
    let Some(s) = b.surfaces.get_mut(&head) else {
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
    let Some((b, head)) = surface_of(state, output) else {
        return false;
    };
    let Some(s) = b.surfaces.get_mut(&head) else {
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
        flush_gamma(state, head);
        schedule(state, output);
        return true;
    }
    if !s.track.power_off() {
        s.hw.off = true;
        log::info!("{}: power off once its commit is done", output.name());
        return true;
    }
    let off = power_off(state, head);
    watch(state);
    off
}

/// Mark `head` off and, when the session is ours and no device reset is out, have its thread
/// clear the CRTC.
pub(super) fn power_off(state: &mut State, head: Head) -> bool {
    let Some(b) = state.backend.as_mut() else {
        return false;
    };
    // During the device reset, `device_reset_done` clears an output the reset left lit.
    let resetting = b.devices.of(head).is_some_and(|d| d.reset.track.busy());
    let active = b.session.is_active() && !resetting;
    let Some(s) = b.surfaces.get_mut(&head) else {
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
    let head = output.user_data().get::<OutputHead>()?.0;
    let s = state.backend.as_ref()?.surfaces.get(&head)?;
    Some(!s.hw.off)
}
