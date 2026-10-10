//! Short-lived Wayland connections: list outputs and toplevels, and capture one
//! into `wl_shm` buffers or GBM dma-bufs through ext-image-copy-capture (plus a
//! cursor session for the metadata cursor mode). One connection per thread.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};
use wayland_client::backend::WaylandError;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop, event_created_child,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::{
    ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
    ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
};
use wayland_protocols::ext::image_capture_source::v1::client::{
    ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
    ext_image_capture_source_v1::ExtImageCaptureSourceV1,
    ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::{
    ext_image_copy_capture_cursor_session_v1::{self, ExtImageCopyCaptureCursorSessionV1},
    ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1, FailureReason},
    ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
    ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_buffer_params_v1::{self, ZwpLinuxBufferParamsV1},
    zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{
    zxdg_output_manager_v1::ZxdgOutputManagerV1,
    zxdg_output_v1::{self, ZxdgOutputV1},
};

/// `wl_shm` numbers these two 0 and 1; every other shm format is its
/// DRM fourcc.
const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;
const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;

#[derive(Clone, Debug, Default)]
pub struct OutputInfo {
    pub name: String,
    pub description: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, Default)]
pub struct ToplevelInfo {
    pub identifier: String,
    pub title: String,
    pub app_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Output(String),
    Toplevel(String),
}

struct State {
    outputs: Vec<(wl_output::WlOutput, Option<ZxdgOutputV1>, OutputInfo)>,
    toplevels: Vec<(ExtForeignToplevelHandleV1, ToplevelInfo)>,
    toplevels_done: bool,
    synced: bool,
    pointer: Option<wl_pointer::WlPointer>,
    main: SessionState,
    cursor: SessionState,
    cursor_info: CursorInfo,
}

#[derive(Default)]
struct SessionState {
    size: (u32, u32),
    shm_formats: Vec<wl_shm::Format>,
    dmabuf_device: Option<libc::dev_t>,
    dmabuf_formats: Vec<DmabufFormat>,
    constraints_done: bool,
    stopped: bool,
    frame: Option<Result<(), FailureReason>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Main,
    Cursor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CursorInfo {
    pub entered: bool,
    pub x: i32,
    pub y: i32,
    pub hotspot: (i32, i32),
}

impl State {
    fn new() -> State {
        State {
            outputs: Vec::new(),
            toplevels: Vec::new(),
            toplevels_done: false,
            synced: false,
            pointer: None,
            main: SessionState::default(),
            cursor: SessionState::default(),
            cursor_info: CursorInfo::default(),
        }
    }

    fn session(&mut self, which: Which) -> &mut SessionState {
        match which {
            Which::Main => &mut self.main,
            Which::Cursor => &mut self.cursor,
        }
    }
}

pub struct Wayland {
    conn: Connection,
    queue: EventQueue<State>,
    qh: QueueHandle<State>,
    state: State,
    shm: wl_shm::WlShm,
    dmabuf: Option<ZwpLinuxDmabufV1>,
    _seat: Option<wl_seat::WlSeat>,
    copy_capture: Option<ExtImageCopyCaptureManagerV1>,
    output_sources: Option<ExtOutputImageCaptureSourceManagerV1>,
    toplevel_sources: Option<ExtForeignToplevelImageCaptureSourceManagerV1>,
    toplevel_list: Option<ExtForeignToplevelListV1>,
}

impl Wayland {
    pub fn connect() -> Result<Wayland, String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland: {e}"))?;
        let (globals, queue) =
            registry_queue_init::<State>(&conn).map_err(|e| format!("registry: {e}"))?;
        let qh = queue.handle();
        let mut state = State::new();
        let shm: wl_shm::WlShm = globals
            .bind(&qh, 1..=1, ())
            .map_err(|e| format!("wl_shm: {e}"))?;
        // A missing manager is reported once here and shows up as a
        // capability, not an error: a compositor without window sources
        // still shares outputs.
        fn optional<I, U>(
            globals: &GlobalList,
            qh: &QueueHandle<State>,
            version: std::ops::RangeInclusive<u32>,
            udata: U,
        ) -> Option<I>
        where
            I: Proxy + 'static,
            State: Dispatch<I, U> + 'static,
            U: Send + Sync + 'static,
        {
            match globals.bind::<I, State, U>(qh, version, udata) {
                Ok(i) => Some(i),
                Err(e) => {
                    log::info!("{}: {e}", I::interface().name);
                    None
                }
            }
        }
        let copy_capture = optional::<ExtImageCopyCaptureManagerV1, _>(&globals, &qh, 1..=1, ());
        let output_sources =
            optional::<ExtOutputImageCaptureSourceManagerV1, _>(&globals, &qh, 1..=1, ());
        let toplevel_sources =
            optional::<ExtForeignToplevelImageCaptureSourceManagerV1, _>(&globals, &qh, 1..=1, ());
        let toplevel_list = optional::<ExtForeignToplevelListV1, _>(&globals, &qh, 1..=1, ());
        let xdg_outputs = optional::<ZxdgOutputManagerV1, _>(&globals, &qh, 3..=3, ());
        let seat = optional::<wl_seat::WlSeat, _>(&globals, &qh, 1..=1, ());
        // v4: the global sends no format events; the capture session
        // names the formats and modifiers itself.
        let dmabuf = optional::<ZwpLinuxDmabufV1, _>(&globals, &qh, 4..=4, ());
        bind_outputs(&globals, &qh, &mut state, xdg_outputs.as_ref());
        let mut wl = Wayland {
            conn,
            queue,
            qh,
            state,
            shm,
            dmabuf,
            _seat: seat,
            copy_capture,
            output_sources,
            toplevel_sources,
            toplevel_list,
        };
        // Two roundtrips: the outputs' names and geometry, then the
        // toplevel handles' titles (each handle's events follow its
        // creation).
        wl.roundtrip()?;
        wl.roundtrip()?;
        Ok(wl)
    }

    fn roundtrip(&mut self) -> Result<(), String> {
        self.state.synced = false;
        self.conn.display().sync(&self.qh, ());
        dispatch_until(&mut self.queue, &mut self.state, ANSWER_WAIT, |s| s.synced)
            .map_err(|e| format!("roundtrip: {e}"))
    }

    pub fn can_capture_outputs(&self) -> bool {
        self.copy_capture.is_some() && self.output_sources.is_some()
    }

    pub fn can_capture_toplevels(&self) -> bool {
        self.copy_capture.is_some()
            && self.toplevel_sources.is_some()
            && self.toplevel_list.is_some()
    }

    pub fn outputs(&self) -> Vec<OutputInfo> {
        self.state
            .outputs
            .iter()
            .map(|(_, _, i)| i.clone())
            .collect()
    }

    pub fn toplevels(&self) -> Vec<ToplevelInfo> {
        self.state
            .toplevels
            .iter()
            .map(|(_, i)| i.clone())
            .collect()
    }

    pub fn open_session(
        &mut self,
        target: &Target,
        paint_cursor: bool,
    ) -> Result<CaptureSession, String> {
        let manager = self
            .copy_capture
            .as_ref()
            .ok_or("no ext_image_copy_capture_manager_v1")?;
        let source: ExtImageCaptureSourceV1 = match target {
            Target::Output(name) => {
                let (out, _, _) = self
                    .state
                    .outputs
                    .iter()
                    .find(|(_, _, i)| &i.name == name)
                    .ok_or_else(|| format!("no output {name}"))?;
                self.output_sources
                    .as_ref()
                    .ok_or("no output capture source manager")?
                    .create_source(out, &self.qh, ())
            }
            Target::Toplevel(id) => {
                let (handle, _) = self
                    .state
                    .toplevels
                    .iter()
                    .find(|(_, i)| &i.identifier == id)
                    .ok_or_else(|| format!("no toplevel {id}"))?;
                self.toplevel_sources
                    .as_ref()
                    .ok_or("no toplevel capture source manager")?
                    .create_source(handle, &self.qh, ())
            }
        };
        let options = if paint_cursor {
            Options::PaintCursors
        } else {
            Options::empty()
        };
        let session = manager.create_session(&source, options, &self.qh, Which::Main);
        self.await_constraints(Which::Main)?;
        Ok(CaptureSession {
            session,
            source,
            cursor: None,
        })
    }

    /// Open a pointer cursor session beside `session` (metadata cursor
    /// mode). Fails when the compositor has no seat pointer or offers
    /// no cursor for the source (a window); the caller then falls back
    /// to a hidden cursor.
    pub fn open_cursor_session(&mut self, session: &mut CaptureSession) -> Result<(), String> {
        let manager = self
            .copy_capture
            .as_ref()
            .ok_or("no ext_image_copy_capture_manager_v1")?;
        let pointer = self
            .state
            .pointer
            .as_ref()
            .ok_or("the seat has no pointer")?;
        let cursor = manager.create_pointer_cursor_session(&session.source, pointer, &self.qh, ());
        let capture = cursor.get_capture_session(&self.qh, Which::Cursor);
        self.await_constraints(Which::Cursor).map_err(|e| {
            capture.destroy();
            cursor.destroy();
            format!("cursor session: {e}")
        })?;
        session.cursor = Some((cursor, capture));
        Ok(())
    }

    fn await_constraints(&mut self, which: Which) -> Result<(), String> {
        self.state.session(which).constraints_done = false;
        self.state.session(which).stopped = false;
        dispatch_until(&mut self.queue, &mut self.state, ANSWER_WAIT, |s| {
            let s = s.session(which);
            s.constraints_done || s.stopped
        })
        .map_err(|e| format!("constraints: {e}"))?;
        if self.state.session(which).stopped {
            return Err("the session stopped before its constraints arrived".into());
        }
        Ok(())
    }

    pub fn constraints(&self) -> Constraints {
        Self::constraints_of(&self.state.main)
    }

    pub fn cursor_constraints(&self) -> Constraints {
        Self::constraints_of(&self.state.cursor)
    }

    fn constraints_of(s: &SessionState) -> Constraints {
        Constraints {
            width: s.size.0,
            height: s.size.1,
            shm_formats: s.shm_formats.clone(),
            dmabuf_device: s.dmabuf_device,
            dmabuf_formats: s.dmabuf_formats.clone(),
        }
    }

    /// Open the GBM device the main session's dma-bufs must come from.
    /// Fails when the compositor has no linux-dmabuf or the session
    /// named no device; the caller then stays on shm.
    pub fn open_gbm(&self) -> Result<Gbm, String> {
        if self.dmabuf.is_none() {
            return Err("no zwp_linux_dmabuf_v1".into());
        }
        let dev = self
            .state
            .main
            .dmabuf_device
            .ok_or("the session named no dma-buf device")?;
        Gbm::open(dev)
    }

    /// Allocate a `w`x`h` dma-buf in `fourcc` from `gbm` with one of
    /// `modifiers` and wrap it as a `wl_buffer`. The modifier GBM
    /// picked is in the result.
    pub fn dmabuf_buffer(
        &self,
        gbm: &Gbm,
        w: u32,
        h: u32,
        fourcc: u32,
        modifiers: &[u64],
    ) -> Result<DmaBuffer, String> {
        let dmabuf = self.dmabuf.as_ref().ok_or("no zwp_linux_dmabuf_v1")?;
        let bo = gbm.allocate(w, h, fourcc, modifiers)?;
        let modifier: u64 = bo.modifier().into();
        let mut planes = Vec::new();
        let params = dmabuf.create_params(&self.qh, ());
        for i in 0..bo.plane_count() as i32 {
            let fd = bo
                .fd_for_plane(i)
                .map_err(|e| format!("gbm plane {i} fd: {e}"))?;
            let (offset, stride) = (bo.offset(i), bo.stride_for_plane(i));
            let len = plane_len(fd.as_fd(), offset)?;
            params.add(
                fd.as_fd(),
                i as u32,
                offset,
                stride,
                (modifier >> 32) as u32,
                modifier as u32,
            );
            planes.push(Plane {
                fd,
                offset,
                stride,
                len,
            });
        }
        // create_immed: a bad buffer is a protocol error rather than a
        // `failed` event, but the compositor announced this exact
        // format and modifier, and GBM allocated from its own device.
        let buffer = params.create_immed(
            w as i32,
            h as i32,
            fourcc,
            zwp_linux_buffer_params_v1::Flags::empty(),
            &self.qh,
            (),
        );
        params.destroy();
        Ok(DmaBuffer {
            bo,
            buffer,
            width: w,
            height: h,
            fourcc,
            modifier,
            planes,
        })
    }

    pub fn cursor_info(&self) -> CursorInfo {
        self.state.cursor_info
    }

    pub fn shm_buffer(&self, w: u32, h: u32, format: wl_shm::Format) -> Result<ShmBuffer, String> {
        let (stride, size) = shm_layout(w, h)?;
        let fd = memfd(size as usize)?;
        let pool = self.shm.create_pool(fd.as_fd(), size, &self.qh, ());
        let buffer = pool.create_buffer(0, w as i32, h as i32, stride, format, &self.qh, ());
        pool.destroy();
        Ok(ShmBuffer {
            fd,
            buffer,
            width: w,
            height: h,
            stride: stride as u32,
            size: size as usize,
        })
    }

    /// Capture one frame into `buf`, waiting at most `wait` for the
    /// compositor. A source with no damage (a window nothing repaints)
    /// sends no frame at all, so the wait is bounded: the caller's loop
    /// must stay responsive, and `Timeout` just means nothing new. A
    /// `BufferConstraints` failure means the size or formats changed:
    /// read `constraints()` again and make new buffers.
    pub fn capture(
        &mut self,
        session: &CaptureSession,
        buf: &dyn CaptureBuffer,
        wait: Duration,
    ) -> Result<(), CaptureError> {
        self.capture_on(&session.session, Which::Main, buf, wait)
    }

    pub fn capture_cursor(
        &mut self,
        session: &CaptureSession,
        buf: &ShmBuffer,
        wait: Duration,
    ) -> Result<bool, CaptureError> {
        let Some((_, capture)) = session.cursor.as_ref() else {
            return Ok(false);
        };
        let capture = capture.clone();
        self.capture_on(&capture, Which::Cursor, buf, wait)
            .map(|()| true)
    }

    fn capture_on(
        &mut self,
        session: &ExtImageCopyCaptureSessionV1,
        which: Which,
        buf: &dyn CaptureBuffer,
        wait: Duration,
    ) -> Result<(), CaptureError> {
        let frame = session.create_frame(&self.qh, which);
        let (w, h) = buf.size();
        frame.attach_buffer(buf.wl_buffer());
        frame.damage_buffer(0, 0, w as i32, h as i32);
        frame.capture();
        self.state.session(which).frame = None;
        let deadline = Instant::now() + wait;
        let result = loop {
            if let Some(r) = self.state.session(which).frame.take() {
                break r.map_err(CaptureError::Failed);
            }
            if self.state.session(which).stopped {
                break Err(CaptureError::Failed(FailureReason::Stopped));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break Err(CaptureError::Timeout);
            }
            if let Err(e) = pump(&mut self.queue, &mut self.state, left) {
                break Err(e);
            }
        };
        frame.destroy();
        result
    }

    pub fn stopped(&self) -> bool {
        self.state.main.stopped
    }

    pub fn flush(&self) {
        let _ = self.conn.flush();
    }
}

fn shm_layout(w: u32, h: u32) -> Result<(i32, i32), String> {
    let too_big = || format!("a {w}x{h} buffer is too big for wl_shm");
    let stride = i32::try_from(w)
        .ok()
        .and_then(|w| w.checked_mul(4))
        .ok_or_else(too_big)?;
    let size = i32::try_from(h)
        .ok()
        .and_then(|h| h.checked_mul(stride))
        .ok_or_else(too_big)?;
    Ok((stride, size))
}

/// How long a request the compositor must answer may take before the caller gives
/// up.
const ANSWER_WAIT: Duration = Duration::from_secs(2);

fn dispatch_until(
    queue: &mut EventQueue<State>,
    state: &mut State,
    wait: Duration,
    done: impl Fn(&mut State) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + wait;
    while !done(state) {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(format!("no answer from the compositor within {wait:?}"));
        }
        pump(queue, state, left).map_err(|_| "the connection is gone".to_string())?;
    }
    Ok(())
}

fn pump(
    queue: &mut EventQueue<State>,
    state: &mut State,
    left: Duration,
) -> Result<(), CaptureError> {
    if queue
        .dispatch_pending(state)
        .map_err(|_| CaptureError::Gone)?
        > 0
    {
        return Ok(());
    }
    // A full socket takes the rest of the requests on a later round;
    // meanwhile the poll also wakes when it drains.
    let unflushed = match queue.flush() {
        Ok(()) => false,
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => true,
        Err(_) => return Err(CaptureError::Gone),
    };
    let Some(guard) = queue.prepare_read() else {
        // Events were queued between the dispatch and here.
        return Ok(());
    };
    let mut pfd = libc::pollfd {
        fd: guard.connection_fd().as_raw_fd(),
        events: if unflushed {
            libc::POLLIN | libc::POLLOUT
        } else {
            libc::POLLIN
        },
        revents: 0,
    };
    let ms = left.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: `pfd` is one valid pollfd for the call, and the guard
    // keeps its fd open.
    let n = unsafe { libc::poll(&mut pfd, 1, ms) };
    if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
        return Err(CaptureError::Gone);
    }
    if n <= 0 || pfd.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) == 0 {
        // Timed out, interrupted or only writable: drop the guard
        // without reading; the next round flushes and checks the
        // caller's deadline.
        drop(guard);
        return Ok(());
    }
    match guard.read() {
        Ok(_) => {}
        // Only part of a message had arrived.
        Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
        Err(_) => return Err(CaptureError::Gone),
    }
    queue
        .dispatch_pending(state)
        .map_err(|_| CaptureError::Gone)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Failed(FailureReason),
    Timeout,
    Gone,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmabufFormat {
    pub fourcc: u32,
    pub modifiers: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub width: u32,
    pub height: u32,
    pub shm_formats: Vec<wl_shm::Format>,
    pub dmabuf_device: Option<libc::dev_t>,
    pub dmabuf_formats: Vec<DmabufFormat>,
}

pub fn shm_format_of_fourcc(fourcc: u32) -> Option<wl_shm::Format> {
    match fourcc {
        DRM_FORMAT_ARGB8888 => Some(wl_shm::Format::Argb8888),
        DRM_FORMAT_XRGB8888 => Some(wl_shm::Format::Xrgb8888),
        _ => wl_shm::Format::try_from(fourcc).ok(),
    }
}

pub fn fourcc_of_shm_format(f: wl_shm::Format) -> u32 {
    match f {
        wl_shm::Format::Argb8888 => DRM_FORMAT_ARGB8888,
        wl_shm::Format::Xrgb8888 => DRM_FORMAT_XRGB8888,
        other => other as u32,
    }
}

impl Constraints {
    /// The first 32 bpp 8888 format offered, ARGB first; `None` without
    /// one, since every shm buffer here has a stride of four bytes a pixel.
    pub fn pick_format(&self) -> Option<wl_shm::Format> {
        [
            wl_shm::Format::Argb8888,
            wl_shm::Format::Xrgb8888,
            wl_shm::Format::Abgr8888,
            wl_shm::Format::Xbgr8888,
        ]
        .into_iter()
        .find(|want| self.shm_formats.contains(want))
    }
}

pub struct CaptureSession {
    session: ExtImageCopyCaptureSessionV1,
    source: ExtImageCaptureSourceV1,
    cursor: Option<(
        ExtImageCopyCaptureCursorSessionV1,
        ExtImageCopyCaptureSessionV1,
    )>,
}

impl Drop for CaptureSession {
    fn drop(&mut self) {
        if let Some((cursor, capture)) = self.cursor.take() {
            capture.destroy();
            cursor.destroy();
        }
        self.session.destroy();
        self.source.destroy();
    }
}

pub trait CaptureBuffer {
    fn wl_buffer(&self) -> &wl_buffer::WlBuffer;
    fn size(&self) -> (u32, u32);
}

impl CaptureBuffer for ShmBuffer {
    fn wl_buffer(&self) -> &wl_buffer::WlBuffer {
        &self.buffer
    }
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

impl CaptureBuffer for DmaBuffer {
    fn wl_buffer(&self) -> &wl_buffer::WlBuffer {
        &self.buffer
    }
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

pub struct Gbm {
    device: gbm::Device<OwnedFd>,
    pub path: std::path::PathBuf,
}

impl Gbm {
    /// Open the DRM node with device number `dev`. The compositor may
    /// name either node type; the render node is preferred when both
    /// match, since it needs no DRM master.
    pub fn open(dev: libc::dev_t) -> Result<Gbm, String> {
        use std::os::unix::fs::MetadataExt;
        let dir = std::fs::read_dir("/dev/dri").map_err(|e| format!("/dev/dri: {e}"))?;
        let mut found: Option<std::path::PathBuf> = None;
        for entry in dir.flatten() {
            let path = entry.path();
            let Ok(md) = std::fs::metadata(&path) else {
                continue;
            };
            if md.rdev() != dev {
                continue;
            }
            let render = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("renderD"));
            if render || found.is_none() {
                found = Some(path);
            }
        }
        let path = found.ok_or_else(|| format!("no DRM node with dev {dev:#x}"))?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let device = gbm::Device::new(OwnedFd::from(file))
            .map_err(|e| format!("gbm_create_device on {}: {e}", path.display()))?;
        Ok(Gbm { device, path })
    }

    /// The plane count of a `w`x`h` `fourcc` buffer with `modifier` on this
    /// device. A driver that does not answer the plane count query (virgl)
    /// is asked through a test allocation instead.
    pub fn plane_count(&self, w: u32, h: u32, fourcc: u32, modifier: u64) -> Result<u32, String> {
        let format = gbm::Format::try_from(fourcc).map_err(|_| format!("fourcc {fourcc:#x}"))?;
        if let Some(n) = self
            .device
            .format_modifier_plane_count(format, gbm::Modifier::from(modifier))
        {
            return Ok(n);
        }
        self.allocate(w, h, fourcc, &[modifier])
            .map(|bo| bo.plane_count())
            .map_err(|e| format!("no plane count for {format:?} with {modifier:#x}: {e}"))
    }

    /// Allocate a bo the compositor can render into, letting GBM pick
    /// from `modifiers` (the implicit modifier is never passed on).
    pub fn allocate(
        &self,
        w: u32,
        h: u32,
        fourcc: u32,
        modifiers: &[u64],
    ) -> Result<gbm::BufferObject<()>, String> {
        let format = gbm::Format::try_from(fourcc).map_err(|_| format!("fourcc {fourcc:#x}"))?;
        let invalid: u64 = gbm::Modifier::Invalid.into();
        let mods: Vec<gbm::Modifier> = modifiers
            .iter()
            .filter(|m| **m != invalid)
            .map(|m| gbm::Modifier::from(*m))
            .collect();
        if mods.is_empty() {
            return Err("no explicit modifiers to allocate with".into());
        }
        self.device
            .create_buffer_object_with_modifiers2(
                w,
                h,
                format,
                mods.into_iter(),
                gbm::BufferObjectFlags::RENDERING,
            )
            .map_err(|e| format!("gbm_bo_create_with_modifiers2 {w}x{h} {format:?}: {e}"))
    }
}

pub struct Plane {
    pub fd: OwnedFd,
    pub offset: u32,
    pub stride: u32,
    /// Bytes from `offset` to the end of the dma-buf: the most the plane can hold,
    /// since GBM does not report a plane's own size.
    pub len: u32,
}

pub struct DmaBuffer {
    bo: gbm::BufferObject<()>,
    pub buffer: wl_buffer::WlBuffer,
    pub width: u32,
    pub height: u32,
    pub fourcc: u32,
    pub modifier: u64,
    pub planes: Vec<Plane>,
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

impl DmaBuffer {
    pub fn read(&self) -> Result<Vec<u8>, String> {
        let (w, h) = (self.width, self.height);
        self.bo
            .map(0, 0, w, h, |m| {
                let stride = m.stride() as usize;
                let src = m.buffer();
                let mut out = Vec::with_capacity((w * h * 4) as usize);
                for row in 0..h as usize {
                    out.extend_from_slice(&src[row * stride..row * stride + (w * 4) as usize]);
                }
                out
            })
            .map_err(|e| format!("gbm_bo_map: {e}"))
    }
}

pub struct ShmBuffer {
    pub fd: OwnedFd,
    pub buffer: wl_buffer::WlBuffer,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub size: usize,
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

pub struct ShmMap {
    ptr: *const u8,
    len: usize,
}

// SAFETY: the mapping is read on the thread that owns the buffer; the
// pointer carries no thread affinity of its own.
unsafe impl Send for ShmMap {}

impl ShmMap {
    /// The mapped bytes. Call this only between a capture's `Ready` and
    /// the next capture into the buffer: the compositor writes the file
    /// while a capture is in flight, and the slice must not change under
    /// its borrow. The buffer must not be shared with another writer.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: `ptr` is a live `len`-byte mapping until `Drop`, and by
        // the contract above no one writes the file while the slice lives.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for ShmMap {
    fn drop(&mut self) {
        // SAFETY: the pointer and length are what `mmap` returned.
        unsafe { libc::munmap(self.ptr as *mut _, self.len) };
    }
}

/// Map the first `len` bytes of `fd` read-only and shared, so writes the
/// compositor makes to the file show through.
fn map_memfd(fd: BorrowedFd<'_>, len: usize) -> Result<ShmMap, String> {
    // SAFETY: a plain read-only shared mapping of a file we own; the
    // result is checked against MAP_FAILED.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd.as_raw_fd(),
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err("mmap failed".into());
    }
    Ok(ShmMap {
        ptr: ptr as *const u8,
        len,
    })
}

impl ShmBuffer {
    pub fn map(&self) -> Result<ShmMap, String> {
        map_memfd(self.fd.as_fd(), self.size)
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        read_fd(self.fd.as_fd(), self.size)
    }
}

fn read_fd(fd: BorrowedFd<'_>, size: usize) -> Result<Vec<u8>, String> {
    use std::os::unix::fs::FileExt;
    let file = std::fs::File::from(fd.try_clone_to_owned().map_err(|e| format!("dup: {e}"))?);
    let mut out = vec![0u8; size];
    file.read_exact_at(&mut out, 0)
        .map_err(|e| format!("reading the capture buffer: {e}"))?;
    Ok(out)
}

fn memfd(size: usize) -> Result<OwnedFd, String> {
    let name = c"reprise-capture";
    // SAFETY: `name` is a NUL-terminated string that outlives the call.
    let fd =
        unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 {
        return Err("memfd_create failed".into());
    }
    // SAFETY: `fd` is a fresh descriptor that nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: `fd` is an open memfd; ftruncate takes no pointers.
    if unsafe { libc::ftruncate(fd.as_raw_fd(), size as libc::off_t) } < 0 {
        return Err("ftruncate failed".into());
    }
    // The fd goes to the sharing app as its data plane; sealed, it cannot
    // shrink or grow the file under the compositor's mapping.
    let seals = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
    // SAFETY: `fd` is an open memfd; F_ADD_SEALS takes an int argument.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, seals) } < 0 {
        return Err("sealing the memfd failed".into());
    }
    Ok(fd)
}

/// The bytes from `offset` to the end of the dma-buf behind `fd`, read
/// with `lseek(SEEK_END)`, the size query dma-bufs support.
fn plane_len(fd: BorrowedFd<'_>, offset: u32) -> Result<u32, String> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::from(fd.try_clone_to_owned().map_err(|e| format!("dup: {e}"))?);
    let total = file
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("dma-buf size: {e}"))?;
    total
        .checked_sub(u64::from(offset))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| format!("plane offset {offset} past a {total}-byte dma-buf"))
}

fn bind_outputs(
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    state: &mut State,
    xdg: Option<&ZxdgOutputManagerV1>,
) {
    let list = globals.contents().clone_list();
    for g in list
        .iter()
        .filter(|g| g.interface == wl_output::WlOutput::interface().name)
    {
        let version = g.version.min(4);
        let out: wl_output::WlOutput = globals.registry().bind(g.name, version, qh, ());
        let xo = xdg.map(|m| m.get_xdg_output(&out, qh, ()));
        state.outputs.push((out, xo, OutputInfo::default()));
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        out: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, xo, info)) = state.outputs.iter_mut().find(|(o, _, _)| o == out) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => info.name = name,
            wl_output::Event::Description { description } if info.description.is_empty() => {
                info.description = description
            }
            // Without xdg-output the mode is the best size we have.
            wl_output::Event::Mode {
                width,
                height,
                flags: WEnum::Value(f),
                ..
            } if xo.is_none() && f.contains(wl_output::Mode::Current) => {
                info.width = width;
                info.height = height;
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, ()> for State {
    fn event(
        state: &mut Self,
        xo: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some((_, _, info)) = state
            .outputs
            .iter_mut()
            .find(|(_, x, _)| x.as_ref() == Some(xo))
        else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                info.x = x;
                info.y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                info.width = width;
                info.height = height;
            }
            zxdg_output_v1::Event::Name { name } if info.name.is_empty() => info.name = name,
            zxdg_output_v1::Event::Description { description } => info.description = description,
            _ => {}
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } => {
                state.toplevels.push((toplevel, ToplevelInfo::default()));
            }
            ext_foreign_toplevel_list_v1::Event::Finished => state.toplevels_done = true,
            _ => {}
        }
    }
    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        h: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_handle_v1::Event::Closed = event {
            state.toplevels.retain(|(x, _)| x != h);
            h.destroy();
            return;
        }
        let Some((_, info)) = state.toplevels.iter_mut().find(|(x, _)| x == h) else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => info.title = title,
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => info.app_id = app_id,
            ext_foreign_toplevel_handle_v1::Event::Identifier { identifier } => {
                info.identifier = identifier
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, Which> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        which: &Which,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let s = state.session(*which);
        match event {
            // The size comes first in every constraints round,
            // so it starts the lists over.
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                s.size = (width, height);
                s.shm_formats.clear();
                s.dmabuf_formats.clear();
                s.dmabuf_device = None;
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(f),
            } => s.shm_formats.push(f),
            ext_image_copy_capture_session_v1::Event::DmabufDevice { device } => {
                if let Ok(raw) = <[u8; std::mem::size_of::<libc::dev_t>()]>::try_from(&device[..]) {
                    s.dmabuf_device = Some(libc::dev_t::from_ne_bytes(raw));
                }
            }
            ext_image_copy_capture_session_v1::Event::DmabufFormat { format, modifiers } => {
                let modifiers = modifiers
                    .as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| u64::from_ne_bytes(*c))
                    .collect();
                s.dmabuf_formats.push(DmabufFormat {
                    fourcc: format,
                    modifiers,
                });
            }
            ext_image_copy_capture_session_v1::Event::Done => s.constraints_done = true,
            ext_image_copy_capture_session_v1::Event::Stopped => s.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, Which> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        which: &Which,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let s = state.session(*which);
        match event {
            ext_image_copy_capture_frame_v1::Event::Ready => s.frame = Some(Ok(())),
            ext_image_copy_capture_frame_v1::Event::Failed {
                reason: WEnum::Value(r),
            } => s.frame = Some(Err(r)),
            ext_image_copy_capture_frame_v1::Event::Failed { .. } => {
                s.frame = Some(Err(FailureReason::Unknown))
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureCursorSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureCursorSessionV1,
        event: ext_image_copy_capture_cursor_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let c = &mut state.cursor_info;
        match event {
            ext_image_copy_capture_cursor_session_v1::Event::Enter => c.entered = true,
            ext_image_copy_capture_cursor_session_v1::Event::Leave => c.entered = false,
            ext_image_copy_capture_cursor_session_v1::Event::Position { x, y } => {
                c.x = x;
                c.y = y;
            }
            ext_image_copy_capture_cursor_session_v1::Event::Hotspot { x, y } => c.hotspot = (x, y),
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
            && caps.contains(wl_seat::Capability::Pointer)
            && state.pointer.is_none()
        {
            state.pointer = Some(seat.get_pointer(qh, ()));
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            state.synced = true;
        }
    }
}

delegate_noop!(State: ignore wl_pointer::WlPointer);

delegate_noop!(State: ignore wl_shm::WlShm);
delegate_noop!(State: ignore wl_shm_pool::WlShmPool);
delegate_noop!(State: ignore wl_buffer::WlBuffer);
delegate_noop!(State: ignore ZwpLinuxDmabufV1);
delegate_noop!(State: ignore ZwpLinuxBufferParamsV1);
delegate_noop!(State: ignore ZxdgOutputManagerV1);
delegate_noop!(State: ignore ExtImageCopyCaptureManagerV1);
delegate_noop!(State: ignore ExtOutputImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtForeignToplevelImageCaptureSourceManagerV1);
delegate_noop!(State: ignore ExtImageCaptureSourceV1);

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    fn offering(shm_formats: Vec<wl_shm::Format>) -> Constraints {
        Constraints {
            width: 1,
            height: 1,
            shm_formats,
            dmabuf_device: None,
            dmabuf_formats: Vec::new(),
        }
    }

    #[test]
    fn pick_format_takes_only_a_32_bpp_format() {
        use wl_shm::Format::*;
        assert_eq!(
            offering(vec![Rgb565, Xbgr8888, Argb8888]).pick_format(),
            Some(Argb8888)
        );
        assert_eq!(
            offering(vec![Rgb565, Xbgr8888]).pick_format(),
            Some(Xbgr8888)
        );
        assert_eq!(
            offering(vec![Rgb565, Rgb888]).pick_format(),
            None,
            "no 8888 format: None"
        );
    }

    #[test]
    fn a_mapped_memfd_shows_later_writes() {
        let fd = memfd(4096).expect("memfd");
        let map = map_memfd(fd.as_fd(), 4096).expect("mmap");
        assert_eq!(map.as_slice()[..4], [0, 0, 0, 0]);
        let bytes = [1u8, 2, 3, 4];
        // SAFETY: `bytes` holds the 4 bytes written and `fd` is open.
        let n = unsafe { libc::pwrite(fd.as_raw_fd(), bytes.as_ptr() as *const _, 4, 0) };
        assert_eq!(n, 4);
        assert_eq!(map.as_slice()[..4], bytes);
    }

    #[test]
    fn a_plane_len_runs_from_its_offset_to_the_buffer_end() {
        let fd = memfd(4096).expect("memfd");
        assert_eq!(plane_len(fd.as_fd(), 0), Ok(4096));
        assert_eq!(plane_len(fd.as_fd(), 3072), Ok(1024));
        assert!(
            plane_len(fd.as_fd(), 5000).is_err(),
            "an offset past the end errs"
        );
    }

    #[test]
    fn a_silent_compositor_times_the_wait_out() {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let conn = Connection::from_socket(ours).expect("connection");
        let mut queue = conn.new_event_queue::<State>();
        let mut state = State::new();
        conn.display().sync(&queue.handle(), ());
        let start = Instant::now();
        let r = dispatch_until(&mut queue, &mut state, Duration::from_millis(100), |s| {
            s.synced
        });
        assert!(r.is_err(), "the wait ends in an error");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "the wait ends at its deadline"
        );
        assert!(!state.synced);
    }

    #[test]
    fn a_full_socket_is_not_a_lost_connection() {
        let (ours, _theirs) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let conn = Connection::from_socket(ours).expect("connection");
        let mut queue = conn.new_event_queue::<State>();
        let mut state = State::new();
        let mut blocked = false;
        for _ in 0..200 {
            for _ in 0..1000 {
                conn.display().sync(&queue.handle(), ());
            }
            if let Err(WaylandError::Io(e)) = conn.flush()
                && e.kind() == std::io::ErrorKind::WouldBlock
            {
                blocked = true;
                break;
            }
        }
        assert!(blocked, "the silent peer's socket fills up");
        assert_eq!(
            pump(&mut queue, &mut state, Duration::from_millis(10)),
            Ok(())
        );
    }

    #[test]
    fn an_shm_buffer_over_2_gib_is_refused() {
        assert_eq!(shm_layout(1920, 1080), Ok((7680, 8_294_400)));
        assert!(shm_layout(40_000, 40_000).is_err(), "6.4 GB wraps u32");
        assert!(shm_layout(30_000, 20_000).is_err(), "2.4 GB wraps i32");
        assert!(shm_layout(u32::MAX, 1).is_err(), "the width alone wraps");
    }

    #[test]
    fn reading_past_the_end_of_the_memfd_fails() {
        let fd = memfd(4096).expect("memfd");
        assert_eq!(read_fd(fd.as_fd(), 4096).map(|v| v.len()), Ok(4096));
        assert!(
            read_fd(fd.as_fd(), 8192).is_err(),
            "a short read is an error"
        );
    }

    #[test]
    fn the_capture_memfd_is_sealed_against_resizing() {
        let fd = memfd(4096).expect("memfd");
        // SAFETY: `fd` is an open memfd; F_GET_SEALS takes no argument.
        let seals = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GET_SEALS) };
        assert_eq!(
            seals & (libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL),
            libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL
        );
        // SAFETY: `fd` is an open memfd; ftruncate takes no pointers.
        let truncate = |len| unsafe { libc::ftruncate(fd.as_raw_fd(), len) };
        assert!(truncate(8192) < 0, "grow is refused");
        assert!(truncate(1024) < 0, "shrink is refused");
    }
}
