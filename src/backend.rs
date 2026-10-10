//! The DRM backend: a libseat session, the primary GPU via udev, one KMS output
//! per connected connector with hotplug, a GLES renderer and libinput. A frame is
//! drawn only when asked for (`schedule`), at most one per vblank. Which connectors
//! are driven (`Backend::surfaces`) or parked (`Backend::parked`) is `output`'s call;
//! the backend only enables, disables, changes mode (`set_mode`) or VRR (`set_vrr`).
//! One GPU, no leases, no multi-GPU copies.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, VrrSupport,
            compositor::{CursorMove, FrameFlags, PrimaryPlaneElement},
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
        },
        egl::{EGLContext, EGLDevice, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            ImportDma, ImportMemWl,
            gles::{Capability, GlesRenderer},
            utils::{CommitCounter, DamageSnapshot},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu},
    },
    desktop::utils::OutputPresentationFeedback,
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    reexports::{
        calloop::{
            Interest, Mode, PostAction, RegistrationToken,
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::backend::GlobalId,
    },
    utils::{DeviceFd, Monotonic},
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
    wayland::presentation::Refresh,
};
use smithay_drm_extras::{
    display_info,
    drm_scanner::{DrmScanEvent, DrmScanner},
};

use crate::{
    compositor::State,
    framepace::Done,
    output::ModeInfo,
    reblur::Reblur,
    render::{OutputElement, Parts},
};

type Allocator = GbmAllocator<DrmDeviceFd>;
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
type FrameData = Option<OutputPresentationFeedback>;
type OutputManager = DrmOutputManager<Allocator, Exporter, FrameData, DrmDeviceFd>;

pub struct Backend {
    pub session: LibSeatSession,
    libinput: Libinput,
    pub renderer: GlesRenderer,
    pub render_node: DrmNode,
    drm: OutputManager,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    parked: HashMap<crtc::Handle, Parked>,
    pub dmabuf: Option<(DmabufState, DmabufGlobal)>,
    pub shm_textures: crate::shmtex::ShmTextures,
}

struct Surface {
    output: Output,
    connector: connector::Info,
    vrr_capable: bool,
    global: GlobalId,
    drm_output: DrmOutput<Allocator, Exporter, FrameData, DrmDeviceFd>,
    pending_vblank: bool,
    needs_redraw: bool,
    pointer_only: bool,
    pointer_shown: bool,
    scene_seen: u64,
    scene_changed: bool,
    last_blur: Option<std::time::Instant>,
    idle_scheduled: bool,
    callback_timer: Option<RegistrationToken>,
    render_timer: Option<RegistrationToken>,
    pace: crate::framepace::Pace,
    frames: u32,
    held: u32,
    counted_from: Option<std::time::Instant>,
    stats: crate::debugview::FrameStats,
    damage_seen: Option<CommitCounter>,
    hw: Hw,
}

struct Parked {
    output: Output,
    connector: connector::Info,
    vrr_capable: bool,
}

struct OutputCrtc(crtc::Handle);

fn crtc_of(output: &Output) -> Option<crtc::Handle> {
    output.user_data().get::<OutputCrtc>().map(|c| c.0)
}

pub struct Head {
    pub output: Output,
    pub enabled: bool,
    pub modes: Vec<ModeInfo>,
    pub current: Option<usize>,
    pub vrr_capable: bool,
    pub vrr_on: bool,
}

pub fn init(state: &mut State) -> Result<(), Box<dyn std::error::Error>> {
    let (mut session, notifier) = LibSeatSession::new()?;
    let seat = session.seat();
    let path = primary_gpu(&seat)?
        .or_else(|| all_gpus(&seat).ok()?.into_iter().next())
        .ok_or("no GPU found")?;
    let node = DrmNode::from_path(&path)?;
    log::info!("GPU {}", path.display());

    let fd = session.open(
        &path,
        OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
    )?;
    let fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) = DrmDevice::new(fd.clone(), true)?;
    let gbm = GbmDevice::new(fd)?;

    // SAFETY: the EGL display is created on and only used from this thread and outlives the renderer.
    let display = unsafe { EGLDisplay::new(gbm.clone())? };
    let render_node = EGLDevice::device_for_display(&display)
        .ok()
        .and_then(|d| d.try_get_render_node().ok().flatten())
        .unwrap_or(node);
    let context = EGLContext::new(&display)?;
    // No fencing: it fences every texture draw and upload for other contexts sharing the textures, and none shares this one.
    // SAFETY: the context is current only on this thread.
    let capabilities = unsafe { GlesRenderer::supported_capabilities(&context)? }
        .into_iter()
        .filter(|c| *c != Capability::Fencing);
    // SAFETY: as above.
    let mut renderer = unsafe { GlesRenderer::with_capabilities(context, capabilities)? };
    log::info!("renderer: gles2 on {render_node}");
    state.glass.set_renderer(&mut renderer);
    state.water.set_renderer(&mut renderer);

    let shm_textures = crate::shmtex::ShmTextures::new(gbm.clone(), &renderer);
    let render_formats = renderer.egl_context().dmabuf_render_formats().clone();
    let drm = DrmOutputManager::new(
        drm,
        GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        ),
        GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into()),
        Some(gbm),
        [Fourcc::Xrgb8888, Fourcc::Argb8888],
        render_formats,
    );

    state
        .surface_protos
        .init_syncobj(&state.dh, drm.device().device_fd().clone());
    state.shm_state.update_formats(renderer.shm_formats());
    let feedback =
        DmabufFeedbackBuilder::new(render_node.dev_id(), renderer.dmabuf_formats()).build()?;
    let mut dmabuf_state = DmabufState::new();
    let dmabuf_global =
        dmabuf_state.create_global_with_default_feedback::<State>(&state.dh, &feedback);

    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| format!("libinput: cannot assign seat {seat}"))?;
    // Read here rather than through Smithay's libinput source, which drops tablet pad events (`inputqueue`).
    state
        .handle
        .insert_source(
            Generic::new(libinput.clone(), Interest::READ, Mode::Level),
            |_, libinput, state| {
                let mut libinput = libinput.as_ref().clone();
                libinput.dispatch()?;
                crate::inputqueue::take_queued(state, libinput);
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| e.error)?;
    crate::inputqueue::take_queued(state, libinput.clone());

    state
        .handle
        .insert_source(notifier, |event, _, state| match event {
            SessionEvent::PauseSession => pause(state),
            SessionEvent::ActivateSession => activate(state),
        })?;
    state
        .handle
        .insert_source(drm_notifier, move |event, meta, state| match event {
            DrmEvent::VBlank(crtc) => vblank(state, crtc, meta.as_ref()),
            DrmEvent::Error(e) => log::error!("drm: {e}"),
        })?;
    let udev = UdevBackend::new(&seat)?;
    state
        .handle
        .insert_source(udev, move |event, _, state| match event {
            UdevEvent::Changed { device_id } if device_id == node.dev_id() => {
                scan_connectors(state)
            }
            UdevEvent::Added { path, .. } => {
                log::info!(
                    "GPU {} added; only the primary one is driven",
                    path.display()
                )
            }
            _ => {}
        })?;

    state.backend = Some(Backend {
        session,
        libinput,
        renderer,
        render_node,
        drm,
        scanner: DrmScanner::new(),
        surfaces: HashMap::new(),
        parked: HashMap::new(),
        dmabuf: Some((dmabuf_state, dmabuf_global)),
        shm_textures,
    });
    scan_connectors(state);
    if state.scene.outputs().is_empty() {
        log::warn!("no connected output on {}", path.display());
    }
    Ok(())
}

fn scan_connectors(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let events = match b.scanner.scan_connectors(b.drm.device()) {
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

fn connector_name(c: &connector::Info) -> String {
    format!("{}-{}", c.interface().as_str(), c.interface_id())
}

fn mode_infos(connector: &connector::Info) -> Vec<ModeInfo> {
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
    let info = display_info::for_connector(b.drm.device(), connector.handle());
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
            vrr_on: s.drm_output.with_compositor(|c| c.vrr_enabled()),
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
    let name = output.name();
    let mode = WlMode::from(drm_mode);
    let old = output.current_mode();
    output.change_current_state(Some(mode), None, None, None);
    let global = output.create_global::<State>(&state.dh);
    let drm_output = match b.drm.lock().initialize_output::<_, OutputElement>(
        crtc,
        drm_mode,
        &[p.connector.handle()],
        output,
        None,
        &mut b.renderer,
        &DrmOutputRenderElements::default(),
    ) {
        Ok(o) => o,
        Err(e) => {
            log::warn!(
                "{name}: cannot drive it at {}x{}@{:.3}: {e}",
                mode.size.w,
                mode.size.h,
                f64::from(mode.refresh) / 1000.0
            );
            state.dh.remove_global::<State>(global);
            if let Some(old) = old {
                output.change_current_state(Some(old), None, None, None);
            }
            return false;
        }
    };
    let Some(p) = b.parked.remove(&crtc) else {
        return false;
    };
    let vrr_capable = drm_output
        .with_compositor(|c| c.vrr_supported(p.connector.handle()))
        .is_ok_and(|v| v != VrrSupport::NotSupported);
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
            global,
            drm_output,
            pending_vblank: false,
            needs_redraw: true,
            scene_changed: true,
            pointer_only: false,
            pointer_shown: false,
            scene_seen: 0,
            last_blur: None,
            idle_scheduled: false,
            callback_timer: None,
            render_timer: None,
            pace: Default::default(),
            frames: 0,
            held: 0,
            counted_from: None,
            stats: Default::default(),
            damage_seen: None,
            hw: Hw::default(),
        },
    );
    state.scene.add_output(output);
    schedule(state, output);
    true
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
        drm_output,
        callback_timer,
        render_timer,
        ..
    } = s;
    drop(drm_output);
    b.parked.insert(
        crtc,
        Parked {
            output: output.clone(),
            connector,
            vrr_capable,
        },
    );
    for t in [callback_timer, render_timer].into_iter().flatten() {
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
    if let Err(e) = s.drm_output.use_mode::<_, OutputElement>(
        drm_mode,
        &mut b.renderer,
        &DrmOutputRenderElements::default(),
    ) {
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
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return false;
    };
    if s.drm_output.with_compositor(|c| c.vrr_enabled()) == on {
        return true;
    }
    if on && !s.vrr_capable {
        log::warn!(
            "output {}: adaptive sync asked for, not supported",
            output.name()
        );
        return false;
    }
    match s.drm_output.with_compositor(|c| c.use_vrr(on)) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("output {}: adaptive sync: {e:?}", output.name());
            false
        }
    }
}

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

fn pause(state: &mut State) {
    state.cancel_drag();
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session paused");
    b.libinput.suspend();
    b.drm.pause();
}

fn activate(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session active");
    if b.libinput.resume().is_err() {
        log::warn!("libinput did not resume");
    }
    let libinput = b.libinput.clone();
    if let Err(e) = b.drm.lock().activate(false) {
        log::error!("drm activate: {e}");
    }
    for s in b.surfaces.values_mut() {
        s.drm_output.reset_buffers();
        s.pending_vblank = false;
    }
    crate::inputqueue::take_queued(state, libinput);
    scan_connectors(state);
    for output in state.scene.outputs().to_vec() {
        crate::nightlight::output_on(state, &output);
    }
    schedule_all(state, Ask::Scene);
}

pub fn schedule(state: &mut State, output: &Output) {
    request(state, output, Ask::Scene);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Scene,
    Content,
    Pointer,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Asked {
    redraw: bool,
    pointer_only: bool,
    scene: bool,
}

impl Asked {
    fn add(&mut self, ask: Ask) {
        self.pointer_only = (self.pointer_only || !self.redraw) && ask == Ask::Pointer;
        self.redraw = true;
        self.scene |= ask == Ask::Scene;
    }
}

fn request(state: &mut State, output: &Output, ask: Ask) {
    let Some(crtc) = crtc_of(output) else {
        return;
    };
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let mut asked = Asked {
        redraw: s.needs_redraw,
        pointer_only: s.pointer_only,
        scene: s.scene_changed,
    };
    asked.add(ask);
    s.needs_redraw = asked.redraw;
    s.pointer_only = asked.pointer_only;
    s.scene_changed = asked.scene;
    if s.pending_vblank || s.idle_scheduled {
        return;
    }
    s.idle_scheduled = true;
    state.handle.insert_idle(move |state| {
        if let Some(s) = state
            .backend
            .as_mut()
            .and_then(|b| b.surfaces.get_mut(&crtc))
        {
            s.idle_scheduled = false;
        }
        render(state, crtc);
    });
}

pub fn schedule_all(state: &mut State, ask: Ask) {
    let outputs: Vec<Output> = state.scene.outputs().to_vec();
    for o in &outputs {
        request(state, o, ask);
    }
}

fn flip_time(
    state: &State,
    meta: Option<&DrmEventMetadata>,
) -> (Duration, u64, wp_presentation_feedback::Kind) {
    use wp_presentation_feedback::Kind;
    let seq = meta.map_or(0, |m| u64::from(m.sequence));
    match meta.map(|m| m.time) {
        Some(DrmEventTime::Monotonic(t)) if !t.is_zero() => {
            (t, seq, Kind::Vsync | Kind::HwClock | Kind::HwCompletion)
        }
        _ => (state.clock.now().into(), seq, Kind::Vsync),
    }
}

fn vblank(state: &mut State, crtc: crtc::Handle, meta: Option<&DrmEventMetadata>) {
    let (time, seq, kind) = flip_time(state, meta);
    let counting = state.config.debug.overlay;
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    match s.drm_output.frame_submitted() {
        Ok(Some(Some(mut feedback))) => {
            let refresh = s.output.current_mode().map_or(60_000, |m| m.refresh.max(1));
            let refresh = Duration::from_micros(1_000_000_000 / refresh as u64);
            feedback.presented::<_, Monotonic>(time, Refresh::fixed(refresh), seq, kind);
        }
        Ok(_) => {}
        Err(e) => log::warn!("{}: frame submit: {e:?}", s.output.name()),
    }
    s.pending_vblank = false;
    if counting {
        s.stats.flipped(time, s.needs_redraw);
    }
    let (output, redraw) = (s.output.clone(), s.needs_redraw);
    crate::power::presented(state, &output);
    crate::sessionlock::presented(state, &output);
    if redraw {
        render(state, crtc);
    }
}

fn render(state: &mut State, crtc: crtc::Handle) {
    state.settle_moves();
    let Some(output) = due(state, crtc) else {
        return;
    };
    if held_back(state, crtc) {
        return;
    }
    let started = std::time::Instant::now();
    if let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    {
        s.needs_redraw = false;
        if std::mem::take(&mut s.pointer_only)
            && let Some(drawn) = pointer_frame(state, crtc, &output, started)
        {
            finish(state, crtc, &output, drawn, !drawn, None, started);
            return;
        }
    }
    state.window_anims_frame(&output);
    state.switch_anim_frame(&output);
    state.flip_frame(&output);
    state.water_frame(&output);
    crate::autohideview::frame(state, &output);
    crate::snapbarview::frame(state, &output);
    let pointer = state.pointer_location();
    let focus = state.magnifier_focus(&output);
    let State {
        backend,
        scene,
        cursor,
        cursors,
        config,
        glass,
        water,
        interaction,
        magnifier,
        ..
    } = state;
    let Some(b) = backend.as_mut() else {
        return;
    };
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return;
    };
    s.pointer_shown =
        reprisescene::output_rect(&output).is_some_and(|g| g.to_f64().contains(pointer));
    s.scene_seen = scene.changes();
    let now = std::time::Instant::now();
    let reblur = crate::reblur::when(
        !std::mem::take(&mut s.scene_changed),
        s.last_blur.map(|t| now - t),
        crate::reblur::cap(config.glass.blur_rate),
    );
    let hold = match reblur {
        Reblur::Now => {
            s.last_blur = Some(now);
            None
        }
        Reblur::Wait(wait) => Some(wait),
    };
    glass.begin(
        &output,
        config.glass,
        crate::glass::quality(interaction, &config.glass),
        hold.is_some(),
    );
    let clear = crate::render::clear_color(&config.theme.colors);
    let filter = crate::magnifier::filter_matrix(&config.accessibility);
    let wanted = crate::glass::wanted(config);
    let water_fell = water.step(&mut b.renderer, &output);
    glass.update_screen_backdrop(&mut b.renderer, scene, &output, clear, wanted);
    let mut build = |renderer: &mut GlesRenderer| {
        crate::magnifier::output_elements(
            magnifier,
            renderer,
            Parts {
                scene,
                cursor,
                cursors,
                pointer,
            },
            &output,
            clear,
            filter,
            focus,
        )
    };
    let mut elements = build(&mut b.renderer);
    let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
    if glass.settle(&output, &elements, scale) {
        drop(elements);
        glass.update_backdrop(&mut b.renderer, scene, &output, clear, wanted);
        elements = build(&mut b.renderer);
    }
    b.shm_textures.update(&mut b.renderer);
    let mut empty = false;
    let drawn =
        match s
            .drm_output
            .render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT)
        {
            Ok(r) => {
                scene.update_surfaces(&output, &r.states);
                if r.is_empty {
                    empty = true;
                    false
                } else {
                    if config.debug.overlay
                        && let PrimaryPlaneElement::Swapchain(e) = &r.primary_element
                    {
                        count_damage(s, &e.damage, &output);
                    }
                    let feedback = scene.take_presentation_feedback(&output, &r.states);
                    match s.drm_output.queue_frame(Some(feedback)) {
                        Ok(()) => {
                            s.pending_vblank = true;
                            count_frame(s, config.debug.overlay, started);
                            true
                        }
                        Err(e) => {
                            log::warn!("{}: queue frame: {e:?}", output.name());
                            false
                        }
                    }
                }
            }
            Err(e) => {
                log::warn!("{}: render: {e:?}", output.name());
                false
            }
        };
    drop(elements);
    let held = hold.filter(|_| glass.any_stale());
    if held.is_some() {
        s.held += 1;
    }
    finish(state, crtc, &output, drawn, empty, held, started);
    if state.magnifier.panning(&output) || water_fell {
        schedule(state, &output);
    }
}

/// A frame asked for by pointer motion alone (`Ask::Pointer`): when the last
/// frame put the pointer on its own plane, move the plane and queue that.
/// Some(whether a frame was queued); None when the frame must be drawn normally.
fn pointer_frame(
    state: &mut State,
    crtc: crtc::Handle,
    output: &Output,
    started: std::time::Instant,
) -> Option<bool> {
    let pointer = state.pointer_location();
    let State {
        backend,
        scene,
        cursor,
        cursors,
        magnifier,
        config,
        ..
    } = state;
    let b = backend.as_mut()?;
    let s = b.surfaces.get_mut(&crtc)?;
    if magnifier.zooms(output) || s.callback_timer.is_some() || s.scene_seen != scene.changes() {
        return None;
    }
    let geo = reprisescene::output_rect(output)?;
    if !geo.to_f64().contains(pointer) {
        return (!s.pointer_shown).then_some(false);
    }
    let parts = Parts {
        scene,
        cursor,
        cursors,
        pointer,
    };
    let elements =
        crate::render::pointer_elements(&mut b.renderer, parts, output, pointer - geo.loc.to_f64());
    let [element] = elements.as_slice() else {
        return None;
    };
    match s.drm_output.reposition_cursor(element) {
        CursorMove::Unavailable => None,
        CursorMove::Unchanged => Some(false),
        CursorMove::Prepared => match s.drm_output.queue_frame(None) {
            Ok(()) => {
                s.pending_vblank = true;
                count_frame(s, config.debug.overlay, started);
                Some(true)
            }
            Err(e) => {
                log::warn!("{}: queue pointer frame: {e:?}", output.name());
                Some(false)
            }
        },
    }
}

/// After a frame on `crtc` that was `drawn` or `empty`, with its glass
/// `held` for a wait: pace the next render, send the frame callbacks and
/// fill the captures that waited for it.
fn finish(
    state: &mut State,
    crtc: crtc::Handle,
    output: &Output,
    drawn: bool,
    empty: bool,
    held: Option<Duration>,
    started: std::time::Instant,
) {
    if let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    {
        let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
        s.pace.not_before = crate::framepace::next_render(drawn, s.needs_redraw, started, interval);
    }
    if let Some(wait) = held {
        state.arm_reblur(wait);
    }
    crate::sessionlock::rendered(state, output, drawn, empty);
    crate::power::rendered(state, output, drawn, empty);
    callbacks_after_a_frame(state, crtc, drawn);
    if drawn {
        crate::capture::output_drawn(state, output);
    }
}

fn held_back(state: &mut State, crtc: crtc::Handle) -> bool {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return false;
    };
    let Some(wait) = crate::framepace::render_wait(s.pace.not_before, std::time::Instant::now())
    else {
        s.pace.not_before = None;
        return false;
    };
    if s.render_timer.is_some() {
        return true;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            if let Some(s) = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&crtc))
            {
                s.render_timer = None;
            }
            render(state, crtc);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => {
            s.render_timer = Some(t);
            true
        }
        Err(e) => {
            log::warn!("render timer: {}", e.error);
            false
        }
    }
}

fn due(state: &State, crtc: crtc::Handle) -> Option<Output> {
    let b = state.backend.as_ref()?;
    let s = b.surfaces.get(&crtc)?;
    (!s.pending_vblank && !s.hw.off && b.session.is_active()).then(|| s.output.clone())
}

fn count_frame(s: &mut Surface, overlay: bool, started: std::time::Instant) {
    if overlay {
        s.stats.rendered(started.elapsed());
    }
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let now = std::time::Instant::now();
    let from = *s.counted_from.get_or_insert(now);
    s.frames += 1;
    if s.frames == 120 {
        let secs = now.duration_since(from).as_secs_f64().max(1e-3);
        log::debug!(
            "{}: 120 frames in {secs:.2} s ({:.1}/s), {} held the glass's blur",
            s.output.name(),
            119.0 / secs,
            s.held
        );
        s.frames = 0;
        s.held = 0;
        s.counted_from = None;
    }
}

fn count_damage(
    s: &mut Surface,
    damage: &DamageSnapshot<i32, smithay::utils::Buffer>,
    output: &Output,
) {
    let commit = damage.current_commit();
    if s.damage_seen.replace(commit) == Some(commit) {
        s.stats.damage(0, 0);
        return;
    }
    match damage.raw().next() {
        Some(rects) => {
            let (mut n, mut area) = (0, 0u64);
            for r in rects {
                n += 1;
                area += (r.size.w.max(0) as u64) * (r.size.h.max(0) as u64);
            }
            s.stats.damage(n, area);
        }
        None => {
            let full = output
                .current_mode()
                .map_or(0, |m| (m.size.w.max(0) as u64) * (m.size.h.max(0) as u64));
            s.stats.damage(1, full);
        }
    }
}

pub fn take_stats(state: &mut State, output: &Output) -> Option<crate::debugview::FrameStats> {
    let crtc = crtc_of(output)?;
    let s = state.backend.as_mut()?.surfaces.get_mut(&crtc)?;
    Some(s.stats.take())
}

fn callbacks_after_a_frame(state: &mut State, crtc: crtc::Handle, shown: bool) {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let interval = crate::framepace::interval(s.output.current_mode().map_or(0, |m| m.refresh));
    let wait = match s.pace.done(shown, std::time::Instant::now(), interval) {
        Done::Now if shown => {
            if let Some(t) = s.callback_timer.take() {
                state.handle.remove(t);
            }
            callbacks_done(state, crtc);
            return;
        }
        Done::Now => Duration::ZERO,
        Done::After(wait) => wait,
    };
    if s.callback_timer.is_some() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            let Some(b) = state.backend.as_mut() else {
                return TimeoutAction::Drop;
            };
            let active = b.session.is_active();
            let Some(s) = b.surfaces.get_mut(&crtc) else {
                return TimeoutAction::Drop;
            };
            s.callback_timer = None;
            // A commit since asked for a frame: its callback waits for that frame, or the client would draw again before the screen showed its last.
            if (s.needs_redraw || s.idle_scheduled) && !s.hw.off && active {
                return TimeoutAction::Drop;
            }
            callbacks_done(state, crtc);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => s.callback_timer = Some(t),
        Err(e) => log::warn!("frame callback timer: {}", e.error),
    }
}

pub fn answer_frames(state: &mut State) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    let idle: Vec<crtc::Handle> = b
        .surfaces
        .iter()
        .filter(|(_, s)| !s.needs_redraw && !s.idle_scheduled && !s.hw.off)
        .map(|(c, _)| *c)
        .collect();
    for crtc in idle {
        callbacks_after_a_frame(state, crtc, false);
    }
}

fn callbacks_done(state: &mut State, crtc: crtc::Handle) {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    s.pace.last_done = Some(std::time::Instant::now());
    let output = s.output.clone();
    send_frame_callbacks(state, &output);
}

fn send_frame_callbacks(state: &mut State, output: &Output) {
    let now = state.now();
    state
        .scene
        .send_frames(output, now, Some(Duration::from_secs(1)));
    if let smithay::input::pointer::CursorImageStatus::Surface(s) = &state.cursor {
        smithay::desktop::utils::send_frames_surface_tree(
            s,
            output,
            now,
            Some(Duration::ZERO),
            |_, _| Some(output.clone()),
        );
    }
}

impl Backend {
    pub fn import_dmabuf(&mut self, dmabuf: &smithay::backend::allocator::dmabuf::Dmabuf) -> bool {
        self.renderer.import_dmabuf(dmabuf, None).is_ok()
    }

    pub fn change_vt(&mut self, vt: i32) {
        if let Err(e) = self.session.change_vt(vt) {
            log::warn!("switch to VT {vt}: {e}");
        }
    }
}

#[derive(Default)]
struct Hw {
    off: bool,
    gamma: Option<Option<Gamma>>,
}

#[derive(Clone, Copy, Debug)]
enum Gamma {
    Lut {
        prop: smithay::reexports::drm::control::property::Handle,
        size: u32,
        blob: Option<u64>,
    },
    Legacy {
        size: u32,
    },
}

impl Gamma {
    fn size(&self) -> u32 {
        match *self {
            Gamma::Lut { size, .. } | Gamma::Legacy { size } => size,
        }
    }
}

fn find_gamma(dev: &DrmDevice, crtc: crtc::Handle) -> Option<Gamma> {
    use smithay::reexports::drm::control::{Device as _, property};
    let mut lut = None;
    let mut lut_size = None;
    if let Ok(props) = dev.get_properties(crtc) {
        for (prop, value) in props {
            let Ok(info) = dev.get_property(prop) else {
                continue;
            };
            match info.name().to_str() {
                Ok("GAMMA_LUT") if matches!(info.value_type(), property::ValueType::Blob) => {
                    lut = Some(prop)
                }
                Ok("GAMMA_LUT_SIZE") => lut_size = u32::try_from(value).ok(),
                _ => {}
            }
        }
    }
    if let (Some(prop), Some(size)) = (lut, lut_size.filter(|&s| s > 1)) {
        return Some(Gamma::Lut {
            prop,
            size,
            blob: None,
        });
    }
    let size = dev.get_crtc(crtc).ok()?.gamma_length();
    (size > 1).then_some(Gamma::Legacy { size })
}

fn create_blob(dev: &DrmDevice, data: &[u8]) -> std::io::Result<u64> {
    use std::os::fd::{AsFd, AsRawFd};
    #[repr(C)]
    struct CreateBlob {
        data: u64,
        length: u32,
        blob_id: u32,
    }
    const DRM_IOCTL_MODE_CREATEPROPBLOB: libc::c_ulong = 0xC010_64BD;
    let mut arg = CreateBlob {
        data: data.as_ptr() as u64,
        length: data.len() as u32,
        blob_id: 0,
    };
    // SAFETY: `arg` is the kernel's `drm_mode_create_blob` layout and points
    // at `data`, which outlives the call; the kernel copies it.
    let r = unsafe {
        libc::ioctl(
            dev.as_fd().as_raw_fd(),
            DRM_IOCTL_MODE_CREATEPROPBLOB,
            &mut arg,
        )
    };
    if r < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(u64::from(arg.blob_id))
}

fn lut_bytes(ramp: &[u16], size: usize) -> Vec<u8> {
    let (r, rest) = ramp.split_at(size);
    let (g, b) = rest.split_at(size);
    (0..size)
        .flat_map(|i| [r[i], g[i], b[i], 0])
        .flat_map(u16::to_ne_bytes)
        .collect()
}

fn surface_of<'a>(
    state: &'a mut State,
    output: &Output,
) -> Option<(&'a mut Backend, crtc::Handle)> {
    let crtc = output.user_data().get::<OutputCrtc>()?.0;
    let b = state.backend.as_mut()?;
    b.surfaces.contains_key(&crtc).then_some((b, crtc))
}

pub fn set_output_power(state: &mut State, output: &Output, on: bool) -> bool {
    let Some((b, crtc)) = surface_of(state, output) else {
        return false;
    };
    let active = b.session.is_active();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    if s.hw.off != on {
        return true;
    }
    if on {
        s.hw.off = false;
        log::info!("{}: power on", output.name());
        schedule(state, output);
        return true;
    }
    if active && let Err(e) = s.drm_output.with_compositor(|c| c.clear()) {
        log::warn!("{}: power off: {e}", output.name());
        return false;
    }
    s.hw.off = true;
    s.pending_vblank = false;
    let timer = s.callback_timer.take();
    log::info!("{}: power off", output.name());
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

pub fn gamma_size(state: &State, output: &Output) -> Option<usize> {
    let crtc = output.user_data().get::<OutputCrtc>()?.0;
    let b = state.backend.as_ref()?;
    let s = b.surfaces.get(&crtc)?;
    let gamma = match s.hw.gamma {
        Some(g) => g,
        None => find_gamma(b.drm.device(), crtc),
    };
    gamma.map(|g| g.size() as usize)
}

pub fn set_gamma(state: &mut State, output: &Output, ramp: Option<&[u16]>) -> bool {
    use smithay::reexports::drm::control::{Device as _, property};
    let Some((b, crtc)) = surface_of(state, output) else {
        return false;
    };
    if !b.session.is_active() {
        return false;
    }
    let dev = b.drm.device();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    let gamma = s.hw.gamma.get_or_insert_with(|| find_gamma(dev, crtc));
    let Some(g) = gamma.as_mut() else {
        return false;
    };
    let size = g.size() as usize;
    if ramp.is_some_and(|r| r.len() != size * 3) {
        log::warn!("{}: gamma ramp of the wrong length", output.name());
        return false;
    }
    let result = match g {
        Gamma::Lut { prop, blob, .. } => {
            let new = match ramp {
                Some(r) => match create_blob(dev, &lut_bytes(r, size)) {
                    Ok(id) => Some(id),
                    Err(e) => {
                        log::warn!("{}: gamma blob: {e}", output.name());
                        return false;
                    }
                },
                None => None,
            };
            match dev.set_property(crtc, *prop, property::Value::Blob(new.unwrap_or(0)).into()) {
                Ok(()) => {
                    if let Some(old) = std::mem::replace(blob, new) {
                        let _ = dev.destroy_property_blob(old);
                    }
                    Ok(())
                }
                Err(e) => {
                    if let Some(id) = new {
                        let _ = dev.destroy_property_blob(id);
                    }
                    Err(e)
                }
            }
        }
        Gamma::Legacy { .. } => {
            let identity: Vec<u16>;
            let r = match ramp {
                Some(r) => r,
                None => {
                    identity = (0..3)
                        .flat_map(|_| {
                            (0..size).map(move |i| (i * 0xffff / (size - 1).max(1)) as u16)
                        })
                        .collect();
                    &identity
                }
            };
            let (red, rest) = r.split_at(size);
            let (green, blue) = rest.split_at(size);
            dev.set_gamma(crtc, red, green, blue)
        }
    };
    match result {
        Ok(()) => true,
        Err(e) => {
            log::warn!("{}: set gamma: {e}", output.name());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Ask, Asked};

    fn asked(asks: &[Ask]) -> Asked {
        let mut a = Asked::default();
        for ask in asks {
            a.add(*ask);
        }
        a
    }

    #[test]
    fn only_pointer_asks_make_a_pointer_only_frame() {
        assert!(asked(&[Ask::Pointer]).pointer_only);
        assert!(asked(&[Ask::Pointer, Ask::Pointer]).pointer_only);
        assert!(!asked(&[Ask::Pointer, Ask::Content]).pointer_only);
        assert!(!asked(&[Ask::Content, Ask::Pointer]).pointer_only);
        assert!(!asked(&[Ask::Scene]).pointer_only);
        let a = asked(&[Ask::Scene, Ask::Pointer]);
        assert!(a.redraw && a.scene && !a.pointer_only);
    }

    #[test]
    fn a_frame_resets_the_asks() {
        let mut a = asked(&[Ask::Content]);
        a.redraw = false;
        a.add(Ask::Pointer);
        assert!(a.pointer_only, "a stale pointer_only=false does not stick");
    }
}
