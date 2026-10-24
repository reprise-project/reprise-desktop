//! The DRM backend: a libseat session, the primary GPU via udev, one KMS output
//! per connected connector with hotplug, a GLES renderer and libinput. A frame is
//! drawn only when asked for (`schedule`), at most one per vblank. Which connectors
//! are driven (`Backend::surfaces`) or parked (`Backend::parked`) is `output`'s call;
//! the backend only enables, disables, changes mode (`set_mode`) or VRR (`set_vrr`).
//! One GPU, no leases, no multi-GPU copies. Each driven CRTC owns its `DrmCompositor`; a
//! modeset is never committed outside an output's own frame. Main renders; each output's
//! commit thread (`commit`) runs its KMS commits and drops its compositor.

mod commit;
mod gamma;
mod inputfds;
mod links;
mod outputs;
mod render;
mod reset;
mod schedule;
mod session;
mod steps;
#[cfg(test)]
mod tests;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError},
    time::Duration,
};

use smithay::{
    backend::{
        allocator::{
            Format as DrmFormat, Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
        },
        drm::{
            DrmDevice, DrmDeviceFd, DrmEvent, DrmNode, compositor::DrmCompositor,
            exporter::gbm::GbmFramebufferExporter,
        },
        egl::{EGLContext, EGLDevice, EGLDisplay},
        renderer::{
            ImportDma, ImportMemWl,
            gles::{Capability, GlesRenderer},
            utils::CommitCounter,
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    desktop::utils::OutputPresentationFeedback,
    output::Output,
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    reexports::{
        calloop::{
            Interest, Mode, PostAction, RegistrationToken,
            channel::{Event as ChannelEvent, Sender, channel},
            generic::Generic,
        },
        drm::control::{connector, crtc},
        input::Libinput,
        rustix::fs::OFlags,
        udev::{EventType, MonitorBuilder},
        wayland_server::backend::GlobalId,
    },
    utils::DeviceFd,
    wayland::dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufState},
};
use smithay_drm_extras::drm_scanner::DrmScanner;

use crate::compositor::State;

use gamma::{Gamma, Lut, write_gamma};
pub use gamma::{gamma_size, set_gamma};
use links::LINK_GOOD;
use outputs::{Probe, asks_scan, scan_connectors};
pub use outputs::{disable, enable, heads, output_power, set_mode, set_output_power, set_vrr};
pub use render::{answer_frames, take_stats};
use render::{committed, retired, vblank};
#[cfg(test)]
pub use schedule::take_asks;
pub use schedule::{
    Ask, cursor_outputs, schedule, schedule_all, schedule_on, schedule_pointer, schedule_step,
};
pub use session::on_input;
use session::{activate, device_reset_done, pause};
use steps::on_quiet;
pub use steps::{defer_output_config, modes_settle, quiet, stalled};

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

/// libdrm's `drmIsMaster`: the kernel checks master before the token, so token 0 fails with
/// EACCES only on a fd that is not master.
fn is_master(dev: &impl std::os::fd::AsFd) -> bool {
    use std::os::fd::AsRawFd;
    const DRM_IOCTL_AUTH_MAGIC: libc::c_ulong = 0x4004_6411;
    let magic: u32 = 0;
    // SAFETY: `magic` is the kernel's `drm_auth` layout; the kernel only reads it.
    let r = unsafe { libc::ioctl(dev.as_fd().as_raw_fd(), DRM_IOCTL_AUTH_MAGIC, &magic) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EACCES)
}

fn surface_of<'a>(
    state: &'a mut State,
    output: &Output,
) -> Option<(&'a mut Backend, crtc::Handle)> {
    let crtc = output.user_data().get::<OutputCrtc>()?.0;
    let b = state.backend.as_mut()?;
    b.surfaces.contains_key(&crtc).then_some((b, crtc))
}
