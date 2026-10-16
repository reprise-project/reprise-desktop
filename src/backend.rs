//! The DRM backend: a libseat session, the primary GPU via udev, one KMS output
//! per connected connector with hotplug, a GLES renderer and libinput. A frame is
//! drawn only when asked for (`schedule`), at most one per vblank. Which connectors
//! are driven (`Backend::surfaces`) or parked (`Backend::parked`) is `output`'s call;
//! the backend only enables, disables, changes mode (`set_mode`) or VRR (`set_vrr`).
//! One GPU, no leases, no multi-GPU copies. Each driven CRTC owns its `DrmCompositor`; a
//! modeset is never committed outside an output's own frame.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use smithay::{
    backend::{
        allocator::{
            Format as DrmFormat, Fourcc, Modifier,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmEventTime, DrmNode, VrrSupport,
            compositor::{CursorMove, DrmCompositor, FrameFlags, PrimaryPlaneElement},
            exporter::gbm::GbmFramebufferExporter,
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
type Compositor = DrmCompositor<Allocator, Exporter, FrameData, DrmDeviceFd>;

const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

pub struct Backend {
    pub session: LibSeatSession,
    libinput: Libinput,
    pub renderer: GlesRenderer,
    pub render_node: DrmNode,
    drm: DrmDevice,
    allocator: Allocator,
    exporter: Exporter,
    gbm: GbmDevice<DrmDeviceFd>,
    render_formats: Vec<DrmFormat>,
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
    vrr_on: bool,
    global: GlobalId,
    compositor: Arc<Mutex<Compositor>>,
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
    /// Asks for the frame desktop water steps next in (`schedule_step`).
    step_timer: Option<RegistrationToken>,
    last_flip: Option<std::time::Instant>,
    pace: crate::framepace::Pace,
    frames: u32,
    held: u32,
    counted_from: Option<std::time::Instant>,
    stats: crate::debugview::FrameStats,
    damage_seen: Option<CommitCounter>,
    hw: Hw,
}

fn lock(c: &Mutex<Compositor>) -> MutexGuard<'_, Compositor> {
    c.lock().unwrap_or_else(PoisonError::into_inner)
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
    let render_formats = renderer
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .collect();
    let allocator = GbmAllocator::new(
        gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );
    let exporter = GbmFramebufferExporter::new(gbm.clone(), Some(render_node).into());

    state
        .surface_protos
        .init_syncobj(&state.dh, drm.device_fd().clone());
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
                scan_connectors(state);
                check_links(state);
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
        allocator,
        exporter,
        gbm,
        render_formats,
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
    let events = match b.scanner.scan_connectors(&b.drm) {
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
    let Some(p) = b.parked.remove(&crtc) else {
        return false;
    };
    let vrr_capable = compositor
        .vrr_supported(p.connector.handle())
        .is_ok_and(|v| v != VrrSupport::NotSupported);
    let vrr_on = compositor.vrr_enabled();
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
            compositor: Arc::new(Mutex::new(compositor)),
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
            step_timer: None,
            last_flip: None,
            pace: Default::default(),
            frames: 0,
            held: 0,
            counted_from: None,
            stats: Default::default(),
            damage_seen: None,
            hw: Hw {
                unlit: true,
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
        compositor,
        callback_timer,
        render_timer,
        step_timer,
        ..
    } = s;
    drop(compositor);
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
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
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
    if let Err(e) = b.drm.activate(false) {
        log::error!("drm activate: {e}");
    }
    let dev = &b.drm;
    for (&crtc, s) in b.surfaces.iter_mut() {
        let mut c = lock(&s.compositor);
        if let Err(e) = c.reset_state() {
            log::warn!("{}: reset state: {e}", s.output.name());
        }
        c.reset_buffers();
        drop(c);
        s.pending_vblank = false;
        s.hw.unlit = true;
        // Another DRM master may have left its own LUT; reading it back is not a commit.
        if let Some(Some(g)) = s.hw.gamma {
            let ours = s.hw.lut.take();
            s.hw.lut = read_lut(dev, crtc, &g, ours.clone());
            if s.hw.lut.is_none() && s.hw.wanted_lut.is_none() {
                s.hw.wanted_lut = ours;
            }
        }
    }
    let crtcs: Vec<crtc::Handle> = b.surfaces.keys().copied().collect();
    for crtc in crtcs {
        flush_gamma(state, crtc);
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

/// Ask for a desktop water step's frame on `output` that starts at or after `at`
/// (`framepace::ask_at`). A wait already armed stands. Glass re-blurs the moving water at
/// `[glass] blur_rate`, like a program's content.
pub fn schedule_step(state: &mut State, output: &Output, at: std::time::Instant) {
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
    if s.step_timer.is_some() {
        return;
    }
    let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
    let now = std::time::Instant::now();
    let Some(wait) = crate::framepace::ask_at(at, now, s.last_flip, s.pending_vblank, interval)
    else {
        request(state, output, Ask::Content);
        return;
    };
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            let output = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&crtc))
                .map(|s| {
                    s.step_timer = None;
                    s.output.clone()
                });
            if let Some(output) = output {
                request(state, &output, Ask::Content);
            }
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => {
            if let Some(s) = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&crtc))
            {
                s.step_timer = Some(t);
            }
        }
        Err(e) => {
            log::warn!("step timer: {}", e.error);
            request(state, output, Ask::Content);
        }
    }
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
    let submitted = lock(&s.compositor).frame_submitted();
    match submitted {
        Ok(Some(Some(mut feedback))) => {
            let refresh = s.output.current_mode().map_or(60_000, |m| m.refresh.max(1));
            let refresh = Duration::from_micros(1_000_000_000 / refresh as u64);
            feedback.presented::<_, Monotonic>(time, Refresh::fixed(refresh), seq, kind);
        }
        Ok(_) => {}
        Err(e) => log::warn!("{}: frame submit: {e:?}", s.output.name()),
    }
    s.pending_vblank = false;
    s.last_flip = Some(std::time::Instant::now());
    if counting {
        s.stats.flipped(time, s.needs_redraw);
    }
    let (output, redraw) = (s.output.clone(), s.needs_redraw);
    retrain_link(state, crtc);
    flush_gamma(state, crtc);
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
    state.overview_fade_frame();
    state.flip_frame(&output);
    state.water_frame(&output);
    crate::autohideview::frame(state, &output);
    crate::snapbarview::frame(state, &output);
    let border = crate::captureborder::elements(state, &output);
    let pointer = state.pointer_location();
    let focus = state.magnifier_focus(&output);
    let State {
        backend,
        scene,
        cursor,
        saver,
        cursors,
        config,
        glass,
        water,
        interaction,
        magnifier,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
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
        crate::reblur::cap(
            config.glass.blur_rate,
            crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh)),
        ),
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
    if !border.is_empty() {
        use smithay::backend::renderer::element::Element;
        let below_pointer = elements
            .iter()
            .position(|e| e.kind() != smithay::backend::renderer::element::Kind::Cursor)
            .unwrap_or(elements.len());
        elements.splice(
            below_pointer..below_pointer,
            border.into_iter().map(OutputElement::Border),
        );
    }
    b.shm_textures.update(&mut b.renderer);
    let mut empty = false;
    let shared = Arc::clone(&s.compositor);
    let mut compositor = lock(&shared);
    let rendered = compositor.render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT);
    let drawn = match rendered {
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
                match compositor.queue_frame(Some(feedback)) {
                    Ok(()) => {
                        s.pending_vblank = true;
                        s.hw.unlit = false;
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
    drop(compositor);
    drop(elements);
    let held = hold.filter(|_| glass.any_stale());
    if held.is_some() {
        s.held += 1;
    }
    let held = crate::reblur::catch_up(held, water.next_step(&output).is_some());
    finish(state, crtc, &output, drawn, empty, held, started);
    if state.magnifier.panning(&output) || water_fell {
        schedule(state, &output);
    }
    if let Some(at) = state.water.next_step(&output) {
        schedule_step(state, &output, at);
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
        saver,
        cursors,
        magnifier,
        config,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
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
    let mut compositor = lock(&s.compositor);
    let moved = compositor.reposition_cursor(element);
    let queued = match moved {
        CursorMove::Unavailable => return None,
        CursorMove::Unchanged => return Some(false),
        CursorMove::Prepared => compositor.queue_frame(None),
    };
    drop(compositor);
    match queued {
        Ok(()) => {
            s.pending_vblank = true;
            s.hw.unlit = false;
            count_frame(s, config.debug.overlay, started);
            Some(true)
        }
        Err(e) => {
            log::warn!("{}: queue pointer frame: {e:?}", output.name());
            Some(false)
        }
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
    /// What the CRTC's gamma holds; None while unknown.
    lut: Option<Lut>,
    /// The ramp waiting for the CRTC to take it (`gamma_step`).
    wanted_lut: Option<Lut>,
    /// Powered on or activated, and no frame queued since: the next commit is a modeset.
    unlit: bool,
    /// A uevent found the connector's link-status BAD: retrain once the CRTC is ready.
    retrain: bool,
    /// Retrains since the link last read GOOD or the output came on.
    link_tries: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Lut {
    Identity,
    Ramp(Vec<u16>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GammaStep {
    Skip,
    Wait,
    Commit,
}

#[derive(Clone, Copy, Debug)]
struct CrtcNow {
    active: bool,
    off: bool,
    unlit: bool,
    flipping: bool,
}

impl CrtcNow {
    fn of(active: bool, s: &Surface) -> CrtcNow {
        CrtcNow {
            active,
            off: s.hw.off,
            unlit: s.hw.unlit,
            flipping: s.pending_vblank,
        }
    }

    /// Free for a blocking commit: the session is ours, the output is on, its first frame since power-on
    /// is queued, and no flip is in flight.
    fn ready(self) -> bool {
        self.active && !self.off && !self.unlit && !self.flipping
    }
}

/// A gamma set is a blocking commit: it waits while the session is away, the output is off, a flip is in
/// flight, or no frame was queued since power-on. A LUT another master left (`held` None) is replaced before
/// that first frame, which would show it.
fn gamma_step(wanted: &Lut, held: Option<&Lut>, crtc: CrtcNow) -> GammaStep {
    if held == Some(wanted) {
        return GammaStep::Skip;
    }
    let crtc = CrtcNow {
        unlit: crtc.unlit && held.is_some(),
        ..crtc
    };
    if crtc.ready() {
        GammaStep::Commit
    } else {
        GammaStep::Wait
    }
}

/// What a CRTC whose GAMMA_LUT reads `value` holds, given the blob we last put there (`ours`, holding `held`).
fn lut_from_blob(value: u64, ours: Option<u64>, held: Option<Lut>) -> Option<Lut> {
    if value == 0 {
        Some(Lut::Identity)
    } else if ours == Some(value) {
        held
    } else {
        None
    }
}

/// DRM_MODE_LINK_STATUS_GOOD and _BAD.
const LINK_GOOD: u64 = 0;
const LINK_BAD: u64 = 1;
/// Retrains in a row that may leave the link BAD before they stop until it reads GOOD or the output comes on.
const LINK_TRIES: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkStep {
    Good,
    Retrain,
    /// The last allowed retrain left it BAD: say so once.
    GiveUp,
    GaveUp,
}

fn link_step(bad: bool, tries: u8) -> LinkStep {
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

/// On a hotplug uevent the kernel expects every connector whose link-status went BAD to get a modeset with
/// the same mode and link-status GOOD, which trains the link again (drm-kms "link-status").
fn check_links(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let dev = &b.drm;
    let mut due = Vec::new();
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
                due.push(crtc);
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
    for crtc in due {
        retrain_link(state, crtc);
    }
}

/// Set a BAD link-status GOOD once the CRTC is ready: a blocking modeset with the current mode.
fn retrain_link(state: &mut State, crtc: crtc::Handle) {
    use smithay::reexports::drm::control::Device as _;
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.session.is_active();
    let dev = &b.drm;
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return;
    };
    if !s.hw.retrain || !CrtcNow::of(active, s).ready() {
        return;
    }
    s.hw.retrain = false;
    let conn = s.connector.handle();
    let Some((prop, LINK_BAD)) = link_status(dev, conn) else {
        return;
    };
    s.hw.link_tries += 1;
    match dev.set_property(conn, prop, LINK_GOOD) {
        Ok(()) => log::info!("{}: link retrained", s.output.name()),
        Err(e) => log::warn!("{}: link retrain: {e}", s.output.name()),
    }
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

/// What the CRTC's gamma holds now: GAMMA_LUT is read back, a legacy table is unknown.
fn read_lut(dev: &DrmDevice, crtc: crtc::Handle, gamma: &Gamma, held: Option<Lut>) -> Option<Lut> {
    use smithay::reexports::drm::control::Device as _;
    let Gamma::Lut { prop, blob, .. } = *gamma else {
        return None;
    };
    let props = dev.get_properties(crtc).ok()?;
    let (handles, values) = props.as_props_and_values();
    let i = handles.iter().position(|&h| h == prop)?;
    lut_from_blob(values[i], blob, held)
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
        s.hw.unlit = true;
        s.hw.link_tries = 0;
        log::info!("{}: power on", output.name());
        flush_gamma(state, crtc);
        schedule(state, output);
        return true;
    }
    if active && let Err(e) = lock(&s.compositor).clear() {
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
        None => find_gamma(&b.drm, crtc),
    };
    gamma.map(|g| g.size() as usize)
}

/// Put `ramp` (None: identity) on `output`'s CRTC, now or once it is ready; nothing when it holds it already.
/// False when the output has no gamma table or the CRTC refused the ramp.
pub fn set_gamma(state: &mut State, output: &Output, ramp: Option<&[u16]>) -> bool {
    let Some((b, crtc)) = surface_of(state, output) else {
        return false;
    };
    let active = b.session.is_active();
    let dev = &b.drm;
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    let hw = &mut s.hw;
    if hw.gamma.is_none() {
        let gamma = find_gamma(dev, crtc);
        hw.lut = gamma.and_then(|g| read_lut(dev, crtc, &g, None));
        hw.gamma = Some(gamma);
    }
    let Some(Some(g)) = hw.gamma else {
        return false;
    };
    if ramp.is_some_and(|r| r.len() != g.size() as usize * 3) {
        log::warn!("{}: gamma ramp of the wrong length", output.name());
        return false;
    }
    let wanted = ramp.map_or(Lut::Identity, |r| Lut::Ramp(r.to_vec()));
    match gamma_step(&wanted, s.hw.lut.as_ref(), CrtcNow::of(active, s)) {
        GammaStep::Skip => {
            s.hw.wanted_lut = None;
            true
        }
        GammaStep::Wait => {
            s.hw.wanted_lut = Some(wanted);
            true
        }
        GammaStep::Commit => {
            s.hw.wanted_lut = None;
            commit_gamma(dev, crtc, &mut s.hw, wanted, output)
        }
    }
}

/// Put a ramp that waited on the CRTC if it can take it now (`gamma_step`).
fn flush_gamma(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.session.is_active();
    let dev = &b.drm;
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return;
    };
    let Some(wanted) = s.hw.wanted_lut.take() else {
        return;
    };
    match gamma_step(&wanted, s.hw.lut.as_ref(), CrtcNow::of(active, s)) {
        GammaStep::Skip => {}
        GammaStep::Wait => s.hw.wanted_lut = Some(wanted),
        GammaStep::Commit => {
            let output = s.output.clone();
            commit_gamma(dev, crtc, &mut s.hw, wanted, &output);
        }
    }
}

fn commit_gamma(
    dev: &DrmDevice,
    crtc: crtc::Handle,
    hw: &mut Hw,
    wanted: Lut,
    output: &Output,
) -> bool {
    use smithay::reexports::drm::control::{Device as _, property};
    let Some(Some(g)) = hw.gamma.as_mut() else {
        return false;
    };
    let size = g.size() as usize;
    let ramp = match &wanted {
        Lut::Ramp(r) => Some(r.as_slice()),
        Lut::Identity => None,
    };
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
        Ok(()) => {
            log::debug!("{}: gamma set", output.name());
            hw.lut = Some(wanted);
            true
        }
        Err(e) => {
            log::warn!("{}: set gamma: {e}", output.name());
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Ask, Asked, CrtcNow, GammaStep, LINK_TRIES, LinkStep, Lut, gamma_step, link_step,
        lut_from_blob,
    };

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

    const READY: CrtcNow = CrtcNow {
        active: true,
        off: false,
        unlit: false,
        flipping: false,
    };

    fn not_ready() -> [(CrtcNow, &'static str); 4] {
        [
            (
                CrtcNow {
                    active: false,
                    ..READY
                },
                "session away",
            ),
            (CrtcNow { off: true, ..READY }, "output off"),
            (
                CrtcNow {
                    flipping: true,
                    ..READY
                },
                "flip in flight",
            ),
            (
                CrtcNow {
                    unlit: true,
                    ..READY
                },
                "no frame since power-on",
            ),
        ]
    }

    #[test]
    fn a_ramp_the_crtc_holds_is_never_committed() {
        let warm = Lut::Ramp(vec![1, 2, 3]);
        for (crtc, _) in not_ready().into_iter().chain([(READY, "ready")]) {
            assert_eq!(
                gamma_step(&Lut::Identity, Some(&Lut::Identity), crtc),
                GammaStep::Skip
            );
            assert_eq!(gamma_step(&warm, Some(&warm), crtc), GammaStep::Skip);
        }
    }

    #[test]
    fn a_new_ramp_commits_on_a_ready_crtc_else_waits() {
        let warm = Lut::Ramp(vec![1, 2, 3]);
        let warmer = Lut::Ramp(vec![1, 2, 2]);
        assert_eq!(
            gamma_step(&warm, Some(&Lut::Identity), READY),
            GammaStep::Commit
        );
        assert_eq!(gamma_step(&warmer, Some(&warm), READY), GammaStep::Commit);
        assert_eq!(
            gamma_step(&Lut::Identity, Some(&warm), READY),
            GammaStep::Commit
        );
        for (crtc, why) in not_ready() {
            assert_eq!(
                gamma_step(&warm, Some(&Lut::Identity), crtc),
                GammaStep::Wait,
                "{why}"
            );
        }
    }

    #[test]
    fn another_masters_lut_is_replaced_before_the_first_frame() {
        let unlit = CrtcNow {
            unlit: true,
            ..READY
        };
        assert_eq!(gamma_step(&Lut::Identity, None, unlit), GammaStep::Commit);
        for (crtc, why) in not_ready().into_iter().take(3) {
            assert_eq!(
                gamma_step(&Lut::Identity, None, crtc),
                GammaStep::Wait,
                "{why}"
            );
        }
    }

    #[test]
    fn a_gamma_lut_reads_back_as_identity_ours_or_unknown() {
        let warm = Lut::Ramp(vec![1, 2, 3]);
        assert_eq!(
            lut_from_blob(0, Some(7), Some(warm.clone())),
            Some(Lut::Identity)
        );
        assert_eq!(
            lut_from_blob(7, Some(7), Some(warm.clone())),
            Some(warm.clone())
        );
        assert_eq!(
            lut_from_blob(9, Some(7), Some(warm)),
            None,
            "another master's blob"
        );
        assert_eq!(lut_from_blob(9, None, None), None);
    }

    #[test]
    fn a_good_link_is_never_retrained() {
        for tries in 0..=LINK_TRIES + 1 {
            assert_eq!(link_step(false, tries), LinkStep::Good);
        }
    }

    #[test]
    fn a_bad_link_is_retrained_three_times_in_a_row_then_left() {
        assert_eq!(LINK_TRIES, 3);
        for tries in 0..LINK_TRIES {
            assert_eq!(link_step(true, tries), LinkStep::Retrain, "try {tries}");
        }
        assert_eq!(link_step(true, LINK_TRIES), LinkStep::GiveUp);
        assert_eq!(link_step(true, LINK_TRIES + 1), LinkStep::GaveUp);
    }

    #[test]
    fn a_retrain_waits_for_a_ready_crtc() {
        assert!(READY.ready());
        for (crtc, why) in not_ready() {
            assert!(!crtc.ready(), "{why}");
        }
    }
}
