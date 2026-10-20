//! The DRM backend: a libseat session, the primary GPU via udev, one KMS output
//! per connected connector with hotplug, a GLES renderer and libinput. A frame is
//! drawn only when asked for (`schedule`), at most one per vblank. Which connectors
//! are driven (`Backend::surfaces`) or parked (`Backend::parked`) is `output`'s call;
//! the backend only enables, disables, changes mode (`set_mode`) or VRR (`set_vrr`).
//! One GPU, no leases, no multi-GPU copies. Each driven CRTC owns its `DrmCompositor`; a
//! modeset is never committed outside an output's own frame. Main renders; each output's
//! commit thread (`commit`) runs its KMS commits and drops its compositor.

mod commit;
mod inputfds;
mod reset;

use std::{
    collections::HashMap,
    ffi::OsStr,
    sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError},
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
        libinput::LibinputInputBackend,
        renderer::{
            ImportDma, ImportMemWl,
            gles::{Capability, GlesRenderer},
            utils::{CommitCounter, DamageSnapshot},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    desktop::utils::OutputPresentationFeedback,
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    reexports::{
        calloop::{
            Interest, Mode, PostAction, RegistrationToken,
            channel::{Event as ChannelEvent, Sender, channel},
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        drm::control::{ModeTypeFlags, connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        udev::{EventType, MonitorBuilder},
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
    config::OutputMode,
    framepace::Done,
    output::ModeInfo,
    reblur::Reblur,
    render::{OutputElement, Parts},
};

type Allocator = GbmAllocator<DrmDeviceFd>;
type Exporter = GbmFramebufferExporter<DrmDeviceFd>;
type Compositor = DrmCompositor<Allocator, Exporter, (), DrmDeviceFd>;
type Scanout = commit::Scanout<Compositor>;
/// A vblank's time, sequence and presentation kind.
type Flip = (Duration, u64, wp_presentation_feedback::Kind);

const COLOR_FORMATS: [Fourcc; 2] = [Fourcc::Xrgb8888, Fourcc::Argb8888];

pub struct Backend {
    pub session: LibSeatSession,
    libinput: Libinput,
    input_fds: inputfds::Held,
    /// The turns after a return that release the held device files, then resume libinput
    /// (`input_back`).
    input_back: Option<RegistrationToken>,
    pub renderer: GlesRenderer,
    pub render_node: DrmNode,
    drm: DrmDevice,
    allocator: Allocator,
    exporter: Exporter,
    gbm: GbmDevice<DrmDeviceFd>,
    render_formats: Vec<DrmFormat>,
    replies: Sender<(crtc::Handle, commit::Reply<Out>)>,
    /// Disabled outputs whose threads still hold their compositors.
    retiring: HashMap<crtc::Handle, std::thread::JoinHandle<()>>,
    /// CRTCs refused while retiring or not quiet, enabled again once quiet (`on_quiet`).
    reenable: std::collections::HashSet<crtc::Handle>,
    /// `REPRISE_COMMIT_DELAY` as the session started.
    delay: Option<String>,
    /// The session came back: reset the device once no commit is out (`reset_device`).
    back: reset::Return,
    /// The `commit-device` thread, which runs the reset's commit.
    device: DeviceThread,
    /// A hotplug uevent asked for a connector scan, run once quiet; one that came while the
    /// session was away runs on its return.
    rescan: bool,
    /// The scan under way: its connectors are probed one per loop turn (`on_quiet`).
    probe: Option<Probe>,
    /// Runs the next step of the deferred work (`on_quiet`).
    step: Option<RegistrationToken>,
    /// The config before a reload whose output changes wait for quiet.
    config_from: Option<Box<crate::config::Config>>,
    /// Checks for stalled outputs while any has a job or flip out (`watch`).
    watch: Option<RegistrationToken>,
    scanner: DrmScanner,
    surfaces: HashMap<crtc::Handle, Surface>,
    parked: HashMap<crtc::Handle, Parked>,
    pub dmabuf: Option<(DmabufState, DmabufGlobal)>,
    pub shm_textures: crate::shmtex::ShmTextures,
    pub uploads: crate::dmaupload::Uploads,
}

struct Surface {
    output: Output,
    connector: connector::Info,
    vrr_capable: bool,
    vrr_on: bool,
    global: GlobalId,
    thread: commit::Thread<Work>,
    compositor: Arc<Mutex<Scanout>>,
    track: commit::Track<Flip>,
    /// The frame on the commit thread.
    flight: Option<InFlight>,
    /// The flipped frame's feedback, presented at its vblank.
    presenting: Option<OutputPresentationFeedback>,
    /// When the last frame that took went to the kernel.
    lit_at: Option<std::time::Instant>,
    /// Commit timings since the last frame-rate log line.
    commit_times: Vec<commit::Times>,
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
    /// Frames since the last frame-rate log line that only moved the cursor plane.
    cursor_frames: u32,
    counted_from: Option<std::time::Instant>,
    stats: crate::debugview::FrameStats,
    damage_seen: Option<CommitCounter>,
    hw: Hw,
}

/// A frame sent to the commit thread, finished on its `Done`.
struct InFlight {
    started: std::time::Instant,
    held: Option<Duration>,
    feedback: Option<OutputPresentationFeedback>,
    /// Only the cursor plane moved (`pointer_frame`).
    pointer: bool,
}

/// Main's lock of an output's compositor. Main locks only while the output's thread is idle
/// (`Track::busy` false), so this never waits; debug builds assert it.
fn lock(c: &Mutex<Scanout>) -> MutexGuard<'_, Scanout> {
    match c.try_lock() {
        Ok(g) => g,
        Err(TryLockError::Poisoned(p)) => p.into_inner(),
        Err(TryLockError::WouldBlock) => {
            debug_assert!(false, "main locked a compositor its commit thread holds");
            c.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
}

/// What a job brings beyond its kind.
enum Work {
    None,
    Gamma {
        gamma: Gamma,
        wanted: Lut,
    },
    Retrain {
        conn: connector::Handle,
        prop: smithay::reexports::drm::control::property::Handle,
    },
}

/// A ramp the CRTC took, and the GAMMA_LUT blob now holding it.
struct Out {
    gamma: Option<(Lut, Option<u64>)>,
}

/// The commit thread's side of an output: its compositor and the device.
struct Kms {
    compositor: Arc<Mutex<Scanout>>,
    fd: DrmDeviceFd,
    crtc: crtc::Handle,
}

impl commit::Committer for Kms {
    type Payload = Work;
    type Output = Out;

    fn commit(&mut self, kind: commit::Kind, work: Work) -> Result<Out, String> {
        use smithay::reexports::drm::control::Device as _;
        let c = || {
            self.compositor
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
        };
        let gamma = match (kind, work) {
            (commit::Kind::Flip { .. }, _) => {
                c().queue_frame()?;
                None
            }
            (commit::Kind::Clear, _) => {
                c().clear()?;
                None
            }
            (commit::Kind::Gamma, Work::Gamma { gamma, wanted }) => {
                let blob = write_gamma(&self.fd, self.crtc, gamma, &wanted)?;
                Some((wanted, blob))
            }
            (commit::Kind::Retrain, Work::Retrain { conn, prop }) => {
                self.fd
                    .set_property(conn, prop, LINK_GOOD)
                    .map_err(|e| e.to_string())?;
                None
            }
            (kind, _) => return Err(format!("{kind:?} without its work")),
        };
        Ok(Out { gamma })
    }
}

impl commit::Frames for Compositor {
    fn queue(&mut self) -> Result<(), String> {
        self.queue_frame(()).map_err(|e| format!("{e:?}"))
    }

    fn submitted(&mut self) -> Result<(), String> {
        self.frame_submitted()
            .map(drop)
            .map_err(|e| format!("{e:?}"))
    }

    fn clear(&mut self) -> Result<(), String> {
        DrmCompositor::clear(self).map_err(|e| e.to_string())
    }
}

/// The thread that turns the whole device off after a session return, and main's view of it.
struct DeviceThread {
    thread: commit::Thread<Vec<reset::Keep>>,
    track: commit::Track<Flip>,
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
    let (streaks_made, made) = smithay::reexports::calloop::ping::make_ping()?;
    state.glass.on_streaks(move || streaks_made.ping());
    let uploads =
        crate::dmaupload::Uploads::new(render_node.dev_path().unwrap_or_else(|| path.clone()));
    let streak_uploads = uploads.clone();
    (state.glass).fill_streaks(Arc::new(move |bytes, size| {
        streak_uploads.fill_bytes(bytes, size)
    }));
    state
        .handle
        .insert_source(made, |_, _, state| state.redraw_all())
        .map_err(|e| e.error)?;
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

    let input_fds = inputfds::Held::default();
    let mut libinput =
        Libinput::new_with_udev(inputfds::InputFds::new(session.clone(), input_fds.clone()));
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
    let (replies, done) = channel::<(crtc::Handle, commit::Reply<Out>)>();
    state
        .handle
        .insert_source(done, |event, _, state| match event {
            ChannelEvent::Msg((crtc, commit::Reply::Done(done))) => committed(state, crtc, done),
            ChannelEvent::Msg((crtc, commit::Reply::Exited)) => retired(state, crtc),
            ChannelEvent::Closed => {}
        })
        .map_err(|e| e.error)?;
    let (device_replies, device_done) = channel::<((), commit::Reply<Vec<crtc::Handle>>)>();
    state
        .handle
        .insert_source(device_done, |event, _, state| {
            if let ChannelEvent::Msg(((), commit::Reply::Done(done))) = event {
                device_reset_done(state, done);
            }
        })
        .map_err(|e| e.error)?;
    let device = reset::Device {
        fd: drm.device_fd().clone(),
        atomic: drm.is_atomic(),
    };
    let device = DeviceThread {
        thread: commit::Thread::spawn("device", (), device, None, device_replies)?,
        track: commit::Track::default(),
    };
    let udev = MonitorBuilder::new()?.match_subsystem("drm")?.listen()?;
    state.handle.insert_source(
        Generic::new(udev, Interest::READ, Mode::Level),
        move |_, udev, state| {
            for event in udev.iter() {
                match event.event_type() {
                    EventType::Change
                        if event.devnum() == Some(node.dev_id())
                            && asks_scan(event.property_value("HOTPLUG")) =>
                    {
                        if let Some(b) = state.backend.as_mut() {
                            b.rescan = true;
                            if !b.quiet() {
                                log::info!("display busy: the connector scan waits for it");
                            }
                        }
                        on_quiet(state);
                    }
                    EventType::Add
                        if event
                            .devnode()
                            .and_then(|p| p.file_name())
                            .is_some_and(|n| n.to_string_lossy().starts_with("card")) =>
                    {
                        log::info!(
                            "GPU {} added; only the primary one is driven",
                            event.syspath().display()
                        )
                    }
                    _ => {}
                }
            }
            Ok(PostAction::Continue)
        },
    )?;

    state.backend = Some(Backend {
        session,
        libinput,
        input_fds,
        input_back: None,
        renderer,
        render_node,
        drm,
        allocator,
        exporter,
        gbm,
        render_formats,
        replies,
        retiring: HashMap::new(),
        reenable: Default::default(),
        delay: std::env::var(commit::Delay::VAR).ok(),
        back: reset::Return::default(),
        device,
        rescan: false,
        probe: None,
        step: None,
        config_from: None,
        watch: None,
        scanner: DrmScanner::new(),
        surfaces: HashMap::new(),
        parked: HashMap::new(),
        dmabuf: Some((dmabuf_state, dmabuf_global)),
        shm_textures,
        uploads,
    });
    scan_connectors(state, &HashMap::new());
    if state.scene.outputs().is_empty() {
        log::warn!("no connected output on {}", path.display());
    }
    Ok(())
}

/// Whether a drm change uevent with this `HOTPLUG` value asks for a connector scan. The
/// kernel's hotplug and connector property events carry `HOTPLUG=1`; the synthetic change a VT
/// switch triggers so udev reapplies device access does not.
fn asks_scan(hotplug: Option<&OsStr>) -> bool {
    hotplug.is_some_and(|v| v == "1")
}

/// Scan the connectors with `probed`'s infos for the ones probed already; the rest are probed
/// now.
fn scan_connectors(state: &mut State, probed: &HashMap<connector::Handle, connector::Info>) {
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
struct Probe {
    left: Vec<connector::Handle>,
    infos: HashMap<connector::Handle, connector::Info>,
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
    if let Some(t) = b.input_back.take() {
        state.handle.remove(t);
    }
    b.libinput.suspend();
    b.drm.pause();
}

fn activate(state: &mut State) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    log::info!("session active");
    b.back.activated(std::time::Instant::now());
    // logind answers this only once it has finished the switch; a device reset sent before
    // that is refused (EACCES) and its modesets fail.
    inputfds::release_one(&mut b.session, &b.input_fds);
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

/// Releases the device files still held after `activate` one per loop turn, then resumes
/// libinput in a turn of its own: each release is a synchronous logind call and the resume
/// reopens every device, ~40 ms in one turn.
fn input_back(state: &mut State) {
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
fn reset_device(state: &mut State) {
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
/// meanwhile, re-read each output's state, drop its buffers and the frame whose vblank
/// `Track::reset` forgot, then let the outputs render.
fn device_reset_done(state: &mut State, done: commit::Done<Vec<crtc::Handle>>) {
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
        let mut c = lock(&s.compositor);
        if let Err(e) = c.reset_state() {
            log::warn!("{}: reset state: {e}", s.output.name());
        }
        c.reset_buffers();
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

impl Backend {
    /// No commit holds the kernel's modeset locks (`commit::quiet`).
    fn quiet(&self) -> bool {
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
    fn held(&self, crtc: crtc::Handle, now: std::time::Instant) -> bool {
        let Some(s) = self.surfaces.get(&crtc) else {
            return false;
        };
        let all: Vec<_> = self.tracks().collect();
        commit::held(&s.track, &all, !self.retiring.is_empty(), now)
    }

    /// The session is ours and its return has been handled (`reset_device`).
    fn settled(&self) -> bool {
        self.session.is_active() && !self.back.pending()
    }
}

/// Main is about to issue `op`, an ioctl that takes every modeset lock.
fn gated(b: &Backend, op: &str) {
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
enum Step {
    Reset,
    Probe,
    Scan,
    Reenable,
    Config,
}

/// The next step. `rescan` asks for a new scan, which starts over at the first probe;
/// `unprobed` counts the connectors left to probe while one is under way.
fn next_step(
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
fn on_quiet(state: &mut State) {
    let outman = state.outputs_mgmt.holds();
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if b.step.is_none() && b.step_due(outman).is_some() {
        let token = state
            .handle
            .insert_source(Timer::immediate(), |_, _, state| {
                if let Some(b) = state.backend.as_mut() {
                    b.step = None;
                }
                quiet_step(state);
                on_quiet(state);
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => b.step = Some(t),
            Err(e) => log::warn!("deferred work: {}", e.error),
        }
    }
    let crtcs: Vec<crtc::Handle> = b.surfaces.keys().copied().collect();
    for crtc in crtcs {
        retrain_link(state, crtc);
    }
    watch(state);
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
        Step::Reset => reset_device(state),
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
fn watch(state: &mut State) {
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
    queue_render(state, crtc);
}

/// Render `crtc` from an idle callback, unless one is armed or a frame is in flight.
fn queue_render(state: &mut State, crtc: crtc::Handle) {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    if !s.track.idle() || s.idle_scheduled {
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
    let Some(wait) = crate::framepace::ask_at(at, now, s.last_flip, !s.track.idle(), interval)
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
    let flip = flip_time(state, meta);
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    if let Some(flip) = s.track.vblank(flip) {
        flipped(state, crtc, flip);
    }
}

/// The frame queued on `crtc` reached the screen at `flip`.
fn flipped(state: &mut State, crtc: crtc::Handle, (time, seq, kind): Flip) {
    let counting = state.config.debug.overlay;
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let submitted = lock(&s.compositor).frame_submitted();
    if let Err(e) = submitted {
        log::warn!("{}: frame submit: {e}", s.output.name());
    }
    if let Some(mut feedback) = s.presenting.take() {
        let refresh = s.output.current_mode().map_or(60_000, |m| m.refresh.max(1));
        let refresh = Duration::from_micros(1_000_000_000 / refresh as u64);
        feedback.presented::<_, Monotonic>(time, Refresh::fixed(refresh), seq, kind);
    }
    s.last_flip = Some(std::time::Instant::now());
    if counting {
        s.stats.flipped(time, s.needs_redraw);
    }
    let (output, redraw) = (s.output.clone(), s.needs_redraw);
    retrain_link(state, crtc);
    flush_gamma(state, crtc);
    crate::power::presented(state, &output);
    crate::sessionlock::presented(state, &output);
    crate::sessionfade::presented(state, &output);
    if redraw {
        render(state, crtc);
    } else {
        state.glass.output_stopped(&output);
    }
    watch(state);
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
            // A queued cursor frame finishes on its `Done` (`committed`).
            if !drawn {
                finish(state, crtc, &output, false, true, None, started);
            }
            watch(state);
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
    let (fade, fade_next) = crate::sessionfade::element(state, &output);
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
    if let Some(fade) = fade {
        elements.insert(0, OutputElement::Border(fade));
    }
    b.shm_textures.update(&mut b.renderer);
    let mut empty = false;
    let shared = Arc::clone(&s.compositor);
    let mut compositor = lock(&shared);
    let rendered = compositor.render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT);
    let modeset = compositor.surface().commit_pending();
    let mut feedback = None;
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
                feedback = Some(scene.take_presentation_feedback(&output, &r.states));
                true
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
    let flight = InFlight {
        started,
        held,
        feedback,
        pointer: false,
    };
    let flipping = drawn && send_flip(s, modeset, flight);
    if !flipping {
        finish(state, crtc, &output, false, empty, held, started);
    }
    if state.magnifier.panning(&output)
        || water_fell
        || fade_next == crate::sessionfade::Next::Frame
    {
        schedule(state, &output);
    }
    if let Some(at) = state.water.next_step(&output) {
        schedule_step(state, &output, at);
    }
    let asked = (state.backend.as_ref())
        .and_then(|b| b.surfaces.get(&crtc))
        .is_none_or(|s| s.needs_redraw);
    if !flipping && !asked {
        state.glass.output_stopped(&output);
    }
    watch(state);
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
    let modeset = compositor.surface().commit_pending();
    drop(compositor);
    match moved {
        CursorMove::Unavailable => None,
        CursorMove::Unchanged => Some(false),
        CursorMove::Prepared => {
            let flight = InFlight {
                started,
                held: None,
                feedback: None,
                pointer: true,
            };
            Some(send_flip(s, modeset, flight))
        }
    }
}

/// Hand the frame `compositor` has ready to the output's commit thread; false when the thread
/// is gone. The frame finishes on its `Done` (`committed`).
fn send_flip(s: &mut Surface, modeset: bool, flight: InFlight) -> bool {
    if !send_job(s, commit::Kind::Flip { modeset }, Work::None) {
        return false;
    }
    s.flight = Some(flight);
    true
}

/// Hand `kind` to the output's commit thread; false when the thread is gone.
fn send_job(s: &mut Surface, kind: commit::Kind, work: Work) -> bool {
    s.track.sent(kind, std::time::Instant::now());
    if !s.thread.send(commit::Job::Run(kind, work)) {
        log::error!("{}: the commit thread is gone", s.output.name());
        s.track.unsent();
        return false;
    }
    true
}

/// A disabled output's thread dropped its compositor and ended.
fn retired(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if let Some(h) = b.retiring.remove(&crtc) {
        let _ = h.join();
        log::debug!("{crtc:?}: its commit thread dropped the compositor and ended");
    }
    on_quiet(state);
    render_waiting(state);
}

/// The commit thread finished `done` on `crtc`.
fn committed(state: &mut State, crtc: crtc::Handle, done: commit::Done<Out>) {
    let overlay = state.config.debug.overlay;
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let waited = s.track.waited(done.ended);
    if s.track.stalled()
        && let Some(w) = waited
    {
        let kind = done.kind;
        log::warn!(
            "{}: {kind:?} returned after {:.1} s",
            s.output.name(),
            w.as_secs_f64()
        );
    }
    if let commit::Phase::Busy { since, .. } = s.track.phase() {
        let t = commit::Times {
            to_thread: done.started.saturating_duration_since(since),
            ioctl: done.ended.saturating_duration_since(done.started),
            to_main: done.ended.elapsed(),
        };
        if overlay {
            s.stats.committed(t.to_thread + t.to_main, t.ioctl);
        }
        if log::log_enabled!(log::Level::Debug) {
            s.commit_times.push(t);
        }
    }
    let ok = done.result.is_ok();
    let name = s.output.name();
    match (&done.result, done.kind) {
        (Err(e), commit::Kind::Flip { modeset: true }) => log::warn!("{name}: modeset: {e}"),
        (Err(e), commit::Kind::Flip { .. }) => log::warn!("{name}: queue frame: {e}"),
        (Err(e), commit::Kind::Clear) => log::warn!("{name}: power off: {e}"),
        (Err(e), commit::Kind::Gamma) => log::warn!("{name}: set gamma: {e}"),
        (Err(e), commit::Kind::Retrain) => log::warn!("{name}: link retrain: {e}"),
        (Err(e), commit::Kind::Reset) => log::warn!("{name}: device reset: {e}"),
        (Ok(_), commit::Kind::Retrain) => log::info!("{name}: link retrained"),
        (Ok(out), commit::Kind::Gamma) => {
            log::debug!("{name}: gamma set");
            if let Some((lut, blob)) = out.gamma.clone() {
                s.hw.lut = Some(lut);
                if let Some(Gamma::Lut { blob: b, .. }) = s.hw.gamma.as_mut() {
                    *b = blob;
                }
            }
        }
        (Ok(_), _) => {}
    }
    let gamma_failed = !ok && done.kind == commit::Kind::Gamma;
    let after = s.track.done(ok, std::time::Instant::now());
    if let Some(wait) = after.backoff {
        log::warn!(
            "{name}: the next modeset waits {} s (failed or slow)",
            wait.as_secs()
        );
    }
    let flipped_frame = matches!(done.kind, commit::Kind::Flip { .. });
    let mut flight = s.flight.take().filter(|_| flipped_frame);
    let feedback = flight.as_mut().and_then(|f| f.feedback.take());
    if ok && flipped_frame {
        s.lit_at = Some(done.started);
        s.hw.unlit = false;
        s.presenting = feedback;
    } else if let Some(mut f) = feedback {
        f.discarded();
    }
    let output = s.output.clone();
    if after.off {
        power_off(state, crtc);
    }
    if let Some(f) = flight {
        if ok
            && let Some(s) = state
                .backend
                .as_mut()
                .and_then(|b| b.surfaces.get_mut(&crtc))
        {
            count_frame(s, overlay, f.started, f.pointer);
        }
        finish(state, crtc, &output, ok, false, f.held, f.started);
    }
    if gamma_failed {
        crate::nightlight::gamma_failed(state, &output);
    }
    match after.vblank {
        Some(flip) => flipped(state, crtc, flip),
        None => {
            flush_gamma(state, crtc);
            retrain_link(state, crtc);
            queue_render(state, crtc);
        }
    }
    on_quiet(state);
    if done.kind.blocking() || waited.is_some_and(|w| w > commit::STALL) {
        render_waiting(state);
    }
    watch(state);
}

/// Render every output that asked for a frame while something held them all.
fn render_waiting(state: &mut State) {
    let waiting: Vec<crtc::Handle> = state
        .backend
        .iter()
        .flat_map(|b| &b.surfaces)
        .filter(|(_, s)| s.needs_redraw)
        .map(|(c, _)| *c)
        .collect();
    for c in waiting {
        queue_render(state, c);
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
    crate::sessionfade::rendered(state, output, empty);
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
    let retry = s
        .track
        .retry_at()
        .filter(|_| lock(&s.compositor).surface().commit_pending());
    let not_before = s.pace.not_before.max(retry);
    let Some(wait) = crate::framepace::render_wait(not_before, std::time::Instant::now()) else {
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
    (s.track.idle() && b.quiet() && !b.back.pending() && !s.hw.off && b.session.is_active())
        .then(|| s.output.clone())
}

fn count_frame(s: &mut Surface, overlay: bool, started: std::time::Instant, cursor_only: bool) {
    if overlay {
        s.stats.rendered(started.elapsed());
    }
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let now = std::time::Instant::now();
    let from = *s.counted_from.get_or_insert(now);
    s.frames += 1;
    s.cursor_frames += u32::from(cursor_only);
    if s.frames == 120 {
        let secs = now.duration_since(from).as_secs_f64().max(1e-3);
        let t = commit::tail(&s.commit_times);
        log::debug!(
            "{}: 120 frames in {secs:.2} s ({:.1}/s), {} held the glass's blur, {} cursor-only; commit p99 {} us to the thread, {} us in the kernel, {} us back",
            s.output.name(),
            119.0 / secs,
            s.held,
            s.cursor_frames,
            t.to_thread.as_micros(),
            t.ioctl.as_micros(),
            t.to_main.as_micros()
        );
        s.commit_times.clear();
        s.frames = 0;
        s.held = 0;
        s.cursor_frames = 0;
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
            let held = b.held(crtc, std::time::Instant::now());
            let Some(s) = b.surfaces.get_mut(&crtc) else {
                return TimeoutAction::Drop;
            };
            s.callback_timer = None;
            if waits_for_frame(s.needs_redraw || s.idle_scheduled, s.hw.off, held, active) {
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

/// A commit since asked for a frame: its callback waits for that frame, or the client would
/// draw again before the screen showed its last. An output off, held or away from its session
/// shows no frame, so its callbacks go out on the timer.
fn waits_for_frame(asked: bool, off: bool, held: bool, active: bool) -> bool {
    asked && !off && !held && active
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

/// How long exit waits for the outputs' threads to drop their compositors (a blocking clear
/// each); a thread still in the kernel after it is left running.
const EXIT_WAIT: Duration = Duration::from_millis(500);

impl Drop for Backend {
    fn drop(&mut self) {
        let mut threads: Vec<_> = self.retiring.drain().map(|(_, h)| h).collect();
        for (_, s) in self.surfaces.drain() {
            let Surface {
                mut thread,
                compositor,
                ..
            } = s;
            drop(compositor);
            threads.extend(thread.exit());
        }
        threads.extend(self.device.thread.exit());
        let left = commit::join_until(threads, EXIT_WAIT);
        if left > 0 {
            log::warn!("{left} output(s) still in a commit at exit; leaving them to the kernel");
        }
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
    /// The CRTC's gamma table, looked up when the output was enabled.
    gamma: Option<Gamma>,
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
            flipping: !s.track.idle(),
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

/// The mode a BAD link trains with: None while `current` is still among `modes`, else the
/// configured mode `want` if offered, the current size at its nearest refresh, or the
/// preferred mode.
fn link_mode(modes: &[ModeInfo], current: &ModeInfo, want: Option<&OutputMode>) -> Option<usize> {
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
fn check_links(state: &mut State) {
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
        if let Some(s) = state
            .backend
            .as_mut()
            .and_then(|b| b.surfaces.get_mut(&crtc))
        {
            s.hw.unlit = true;
        }
        state.update_layout();
    }
    for crtc in due {
        retrain_link(state, crtc);
    }
}

/// Set a BAD link-status GOOD once the CRTC is ready: a blocking modeset with the current mode.
fn retrain_link(state: &mut State, crtc: crtc::Handle) {
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

fn create_blob(dev: &impl std::os::fd::AsFd, data: &[u8]) -> std::io::Result<u64> {
    use std::os::fd::AsRawFd;
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
fn power_off(state: &mut State, crtc: crtc::Handle) -> bool {
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

pub fn gamma_size(state: &State, output: &Output) -> Option<usize> {
    let crtc = output.user_data().get::<OutputCrtc>()?.0;
    let b = state.backend.as_ref()?;
    let s = b.surfaces.get(&crtc)?;
    s.hw.gamma.map(|g| g.size() as usize)
}

/// Put `ramp` (None: identity) on `output`'s CRTC, now or once it is ready; nothing when it holds it already.
/// False when the output has no gamma table; a ramp the kernel refuses later goes to
/// `nightlight::gamma_failed`.
pub fn set_gamma(state: &mut State, output: &Output, ramp: Option<&[u16]>) -> bool {
    let Some((b, crtc)) = surface_of(state, output) else {
        return false;
    };
    let active = b.settled();
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return false;
    };
    let Some(g) = s.hw.gamma else {
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
            let sent = commit_gamma(s, wanted);
            watch(state);
            sent
        }
    }
}

/// Put a ramp that waited on the CRTC if it can take it now (`gamma_step`).
fn flush_gamma(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    let active = b.settled();
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
            commit_gamma(s, wanted);
        }
    }
}

/// Hand `wanted` to the output's commit thread (`write_gamma`); false when it is gone.
fn commit_gamma(s: &mut Surface, wanted: Lut) -> bool {
    let Some(gamma) = s.hw.gamma else {
        return false;
    };
    send_job(s, commit::Kind::Gamma, Work::Gamma { gamma, wanted })
}

/// Put `wanted` on `crtc`'s gamma, on the commit thread. Returns the GAMMA_LUT blob now
/// holding it; the one `gamma` held before is destroyed.
fn write_gamma(
    fd: &DrmDeviceFd,
    crtc: crtc::Handle,
    gamma: Gamma,
    wanted: &Lut,
) -> Result<Option<u64>, String> {
    use smithay::reexports::drm::control::{Device as _, property};
    let size = gamma.size() as usize;
    let ramp = match wanted {
        Lut::Ramp(r) => Some(r.as_slice()),
        Lut::Identity => None,
    };
    match gamma {
        Gamma::Lut { prop, blob, .. } => {
            let new = match ramp {
                Some(r) => Some(
                    create_blob(fd, &lut_bytes(r, size)).map_err(|e| format!("gamma blob: {e}"))?,
                ),
                None => None,
            };
            let value = property::Value::Blob(new.unwrap_or(0)).into();
            match fd.set_property(crtc, prop, value) {
                Ok(()) => {
                    if let Some(old) = blob {
                        let _ = fd.destroy_property_blob(old);
                    }
                    Ok(new)
                }
                Err(e) => {
                    if let Some(id) = new {
                        let _ = fd.destroy_property_blob(id);
                    }
                    Err(e.to_string())
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
            fd.set_gamma(crtc, red, green, blue)
                .map(|()| None)
                .map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Ask, Asked, CrtcNow, GammaStep, LINK_TRIES, LinkStep, Lut, ModeInfo, OutputMode, Step,
        asks_scan, gamma_step, link_mode, link_step, lut_from_blob, next_step, waits_for_frame,
    };

    fn asked(asks: &[Ask]) -> Asked {
        let mut a = Asked::default();
        for ask in asks {
            a.add(*ask);
        }
        a
    }

    #[test]
    fn a_held_output_answers_frames_on_the_timer() {
        assert!(waits_for_frame(true, false, false, true));
        assert!(
            !waits_for_frame(true, false, true, true),
            "a held output shows no frame for its clients to wait on"
        );
        assert!(!waits_for_frame(true, true, false, true));
        assert!(!waits_for_frame(true, false, false, false));
        assert!(!waits_for_frame(false, false, false, true));
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
    fn deferred_work_runs_one_step_at_a_time_reset_probes_scan_enables_config() {
        let mut reset = true;
        let mut rescan = true;
        let mut unprobed: Option<usize> = None;
        let mut reenable = 2;
        let mut config = true;
        let mut steps = Vec::new();
        while let Some(step) = next_step(reset, rescan, unprobed, reenable > 0, config) {
            steps.push(step);
            match step {
                Step::Reset => reset = false,
                Step::Probe if rescan => (rescan, unprobed) = (false, Some(2)),
                Step::Probe => unprobed = unprobed.map(|n| n - 1),
                Step::Scan => unprobed = None,
                Step::Reenable => reenable -= 1,
                Step::Config => config = false,
            }
            assert!(steps.len() < 20, "never ends: {steps:?}");
        }
        use Step::*;
        assert_eq!(
            steps,
            [Reset, Probe, Probe, Probe, Scan, Reenable, Reenable, Config],
            "the first probe step lists three connectors and probes one"
        );
        assert_eq!(
            next_step(false, true, Some(1), true, true),
            Some(Probe),
            "a new uevent starts the scan over"
        );
        assert_eq!(next_step(false, false, Some(0), true, true), Some(Scan));
    }

    #[test]
    fn a_link_retrains_its_mode_while_listed_else_the_closest_one() {
        let m = |width, height, hz: i32, preferred| ModeInfo {
            width,
            height,
            refresh_mhz: hz * 1000,
            preferred,
        };
        let current = m(3840, 2160, 60, false);
        let lower = [
            m(3840, 2160, 30, false),
            m(3840, 2160, 24, false),
            m(1920, 1080, 60, true),
        ];
        let with_current = [m(1920, 1080, 60, true), current];
        assert_eq!(
            link_mode(&with_current, &current, None),
            None,
            "still listed"
        );
        assert_eq!(
            link_mode(&lower, &current, None),
            Some(0),
            "same size, nearest refresh"
        );
        let want = OutputMode {
            width: 1920,
            height: 1080,
            refresh_mhz: None,
        };
        assert_eq!(
            link_mode(&lower, &current, Some(&want)),
            Some(2),
            "the configured mode"
        );
        let smaller = [m(2560, 1440, 60, false), m(1920, 1080, 60, true)];
        assert_eq!(
            link_mode(&smaller, &current, None),
            Some(1),
            "the preferred mode"
        );
        assert_eq!(
            link_mode(&smaller[..1], &current, None),
            Some(0),
            "else the first"
        );
        assert_eq!(link_mode(&[], &current, None), None, "nothing to switch to");
    }

    #[test]
    fn a_retrain_waits_for_a_ready_crtc() {
        assert!(READY.ready());
        for (crtc, why) in not_ready() {
            assert!(!crtc.ready(), "{why}");
        }
    }

    #[test]
    fn only_a_hotplug_uevent_asks_for_a_connector_scan() {
        assert!(asks_scan(Some("1".as_ref())), "a hotplug scans");
        assert!(
            !asks_scan(None),
            "the synthetic change of a VT switch does not"
        );
        assert!(!asks_scan(Some("0".as_ref())));
    }
}
