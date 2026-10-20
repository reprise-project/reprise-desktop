//! One PipeWire `Video/Source` stream per screencast, on its own thread:
//! the thread owns a Wayland connection with a capture session and a
//! PipeWire main loop with the stream, which also watches the Wayland
//! fd. The thread keeps one buffer out with the compositor and, as soon
//! as the frame is ready, stamps the header, hands it on and asks for
//! the next; nothing blocks the loop. A source that does not change
//! keeps the buffer out, and the consumer holds the last frame. The
//! consumer therefore reads frames the compositor wrote directly, no
//! copy in between.
//!
//! Buffers: when the capture session names a dma-buf device, one
//! `EnumFormat` per dma-buf format goes first with its modifiers as an
//! unfixated choice (MANDATORY | DONT_FIXATE), the shm formats after.
//! A consumer that picks a dma-buf format sends the modifier choice back
//! still unfixated; a test allocation on the GBM device picks one and
//! the format is re-sent fixated. `Buffers` then allows DmaBuf and MemFd,
//! and each buffer is a GBM bo whose planes are the PipeWire data
//! planes. A consumer that cannot import drops the modifier and
//! renegotiates; once its list is empty only the shm pods match and the
//! stream goes on over MemFd. No GBM device, or a failed test
//! allocation, means shm only.
//!
//! Cursor modes: hidden captures plainly; embedded asks the compositor
//! to paint the cursor into the frame; metadata opens a pointer cursor
//! session beside the capture and fills `SPA_META_Cursor` on each buffer
//! with the position, hotspot and (while the pointer is over the source)
//! the cursor's image as a BGRA bitmap, captured into a small shm buffer
//! of the cursor plane's size. Only outputs offer a cursor session; a
//! window falls back to hidden.

use crate::wayland::{
    Asked, CaptureBuffer, CaptureError, Constraints, CursorInfo, DmaBuffer, Gbm, ShmBuffer, Target,
    Wayland, fourcc_of_shm_format, shm_format_of_fourcc,
};
use pipewire::spa;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::{self, Pod};
use pipewire::spa::support::system::IoFlags;
use pipewire::spa::sys as spa_sys;
use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle};
use pipewire::stream::{StreamFlags, StreamRc, StreamState};
use pipewire::sys as pw_sys;
use std::cell::RefCell;
use std::io::Cursor;
use std::os::fd::AsRawFd;
use std::rc::Rc;
use std::time::Duration;
use wayland_client::protocol::wl_shm;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::FailureReason;

/// What the capture thread reports back once PipeWire named its node:
/// node id, object serial, width, height.
type NodeReport = Result<(u32, u64, u32, u32), String>;

/// How the pointer appears in the stream (the portal's cursor_mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMode {
    Hidden,
    Embedded,
    Metadata,
}

/// The cursor's image is small and its session answers at once; a
/// short wait keeps a missing cursor plane from stalling the frame.
const CURSOR_WAIT: Duration = Duration::from_millis(50);

/// The largest cursor image carried in the metadata: the cursor plane
/// is 64 px on most hardware, 256 on some; the meta is sized for the
/// plane the compositor announced, capped here.
const MAX_CURSOR_PX: u32 = 256;

/// A running stream: its PipeWire node id and the handle to stop it.
pub struct Cast {
    pub node_id: u32,
    pub serial: u64,
    pub width: u32,
    pub height: u32,
    stop: pipewire::channel::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Errs once the capture thread is gone: its sender dies with it.
    ended: Option<std::sync::mpsc::Receiver<()>>,
}

impl Cast {
    /// Start capturing `target`; returns once PipeWire assigned the node.
    pub fn start(target: Target, cursor: CursorMode) -> Result<Cast, String> {
        let (tx, rx) = std::sync::mpsc::channel::<NodeReport>();
        let (stop, stop_rx) = pipewire::channel::channel::<()>();
        let (ended_tx, ended) = std::sync::mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("reprise-portal-cast".into())
            .spawn(move || {
                let _ended = ended_tx;
                if let Err(e) = run(target, cursor, tx.clone(), stop_rx) {
                    log::warn!("cast thread: {e}");
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        let report = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(|_| "the capture thread did not report a node in time".to_string())
            .and_then(|r| r);
        let (node_id, serial, width, height) = match report {
            Ok(r) => r,
            Err(e) => {
                // The thread may still be running its loop with the capture
                // session (and the compositor's privacy indicator) open:
                // tell it to quit, but do not wait, since a thread stuck
                // before the loop would never see the message.
                let _ = stop.send(());
                drop(thread);
                return Err(e);
            }
        };
        Ok(Cast {
            node_id,
            serial,
            width,
            height,
            stop,
            thread: Some(thread),
            ended: Some(ended),
        })
    }

    /// A receiver whose `recv` returns once the capture thread has
    /// ended, whichever way: taken once, by whoever watches for the
    /// compositor stopping the session.
    pub fn take_ended(&mut self) -> Option<std::sync::mpsc::Receiver<()>> {
        self.ended.take()
    }

    /// Stop the capture thread and wait for it to end. That can take a
    /// frame and PipeWire's teardown, so callers run it off the executor.
    pub fn stop(mut self) {
        let _ = self.stop.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Dropping a cast without `stop` tells the thread to quit and does not
/// wait for it.
impl Drop for Cast {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

/// Per-stream state the PipeWire callbacks share.
struct Data {
    wl: Wayland,
    session: crate::wayland::CaptureSession,
    constraints: Constraints,
    format: wl_shm::Format,
    /// The GBM device dma-bufs come from; `None` keeps the stream on shm.
    gbm: Option<Gbm>,
    /// The negotiated dma-buf modifier and the plane count of a bo with
    /// it; `None` once the format is negotiated means MemFd.
    modifier: Option<(u64, u32)>,
    /// The wl_buffer behind each PipeWire buffer, keyed by the buffer's
    /// address.
    buffers: Vec<(usize, Buffer)>,
    /// Metadata cursor mode: the buffer the cursor's image is captured
    /// into and the meta's size for it.
    cursor: Option<CursorPlane>,
    reported: Option<std::sync::mpsc::Sender<NodeReport>>,
    /// The loop to quit when the compositor stops the session.
    main_loop: pipewire::main_loop::MainLoopWeak,
    /// The buffer out with the compositor, by address, and its frame.
    asked: Option<(usize, Asked)>,
    seq: u64,
    /// Frames answered so far, for the debug log of the first captures.
    calls: u64,
}

/// A frame buffer of either kind.
enum Buffer {
    Shm(ShmBuffer),
    Dma(DmaBuffer),
}

impl Buffer {
    fn capture_buffer(&self) -> &dyn CaptureBuffer {
        match self {
            Buffer::Shm(b) => b,
            Buffer::Dma(b) => b,
        }
    }

    /// The bytes of the first plane, for the chunk after a capture.
    fn size(&self) -> u32 {
        match self {
            Buffer::Shm(b) => b.size as u32,
            Buffer::Dma(b) => (b.planes[0].stride * b.height).min(b.planes[0].len),
        }
    }
}

/// The cursor session's buffer and what the meta for it looks like.
struct CursorPlane {
    buf: ShmBuffer,
    /// The buffer mapped once; each frame's image is read out of it.
    map: crate::wayland::ShmMap,
    format: wl_shm::Format,
    /// The last id stamped; a new one tells the consumer the bitmap
    /// changed (the image is re-sent every frame, so it always does).
    id: u32,
}

/// Open the cursor session and a buffer for its image. Fails when the
/// compositor offers none, as for a window source, and so announces no
/// size or format.
fn cursor_plane(
    wl: &mut Wayland,
    session: &mut crate::wayland::CaptureSession,
) -> Result<CursorPlane, String> {
    wl.open_cursor_session(session)?;
    let c = wl.cursor_constraints();
    if c.width == 0 || c.height == 0 {
        return Err("the cursor session has no image (software cursor?)".into());
    }
    let (w, h) = (c.width.min(MAX_CURSOR_PX), c.height.min(MAX_CURSOR_PX));
    let format = c
        .pick_format()
        .ok_or("the cursor session offered no shm format")?;
    log::info!("cursor session: {w}x{h} {format:?}");
    let buf = wl.shm_buffer(w, h, format)?;
    let map = buf.map()?;
    Ok(CursorPlane {
        buf,
        map,
        format,
        id: 0,
    })
}

fn run(
    target: Target,
    cursor: CursorMode,
    report: std::sync::mpsc::Sender<NodeReport>,
    stop_rx: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    let mut wl = Wayland::connect()?;
    let mut session = wl.open_session(&target, cursor == CursorMode::Embedded)?;
    let constraints = wl.constraints();
    let plane = if cursor == CursorMode::Metadata {
        match cursor_plane(&mut wl, &mut session) {
            Ok(p) => Some(p),
            Err(e) => {
                log::info!("{e}; the cursor stays hidden");
                None
            }
        }
    } else {
        None
    };
    let format = constraints
        .pick_format()
        .ok_or("the compositor offered no shm format")?;
    if constraints.width == 0 || constraints.height == 0 {
        return Err("the capture source is empty".into());
    }
    let gbm = match wl.open_gbm() {
        Ok(g) => {
            log::info!("dma-buf buffers from {}", g.path.display());
            Some(g)
        }
        Err(e) => {
            log::info!("{e}; shm buffers only");
            None
        }
    };

    pipewire::init();
    let main_loop = pipewire::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pipewire::context::ContextRc::new(&main_loop, None).map_err(|e| e.to_string())?;
    let core = context.connect_rc(None).map_err(|e| e.to_string())?;
    let stream = StreamRc::new(
        core,
        "reprise-screencast",
        pipewire::properties::properties! {
            *pipewire::keys::MEDIA_CLASS => "Video/Source",
            *pipewire::keys::NODE_NAME => "reprise-screencast",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;

    let data = Data {
        wl,
        session,
        constraints: constraints.clone(),
        format,
        gbm,
        modifier: None,
        buffers: Vec::new(),
        cursor: plane,
        reported: Some(report),
        main_loop: main_loop.downgrade(),
        asked: None,
        seq: 0,
        calls: 0,
    };
    let data_gbm = data.gbm.is_some();
    let fd = Fd(data.wl.fd());
    // The stream's callbacks and the Wayland fd's share the state; each
    // runs from the loop on its own, so a borrow never nests.
    let data = Rc::new(RefCell::new(data));
    let _listener = stream
        .add_local_listener_with_user_data(data.clone())
        .state_changed(|stream, d, _old, new| on_state(stream, &mut d.borrow_mut(), new))
        .param_changed(|stream, d, id, pod| on_param(stream, &mut d.borrow_mut(), id, pod))
        .add_buffer(|_stream, d, buf| {
            let d = &mut *d.borrow_mut();
            // A buffer left without memory would reach the consumer
            // unusable, so the cast ends and the session closes instead.
            if let Err(e) = on_add_buffer(d, buf) {
                log::warn!("add_buffer: {e}; ending the cast");
                d.quit();
            }
        })
        .remove_buffer(|_stream, d, buf| on_remove_buffer(&mut d.borrow_mut(), buf))
        .process(|stream, d| on_process(stream, &mut d.borrow_mut()))
        .register()
        .map_err(|e| e.to_string())?;
    let replies = stream.clone();
    let _replies = main_loop.loop_().add_io(fd, IoFlags::IN, move |_| {
        on_process(&replies, &mut data.borrow_mut());
    });

    let params = format_params(&constraints, format, data_gbm, None);
    let mut refs: Vec<&Pod> = params.iter().map(|p| &**p).collect();
    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            StreamFlags::ALLOC_BUFFERS,
            &mut refs,
        )
        .map_err(|e| e.to_string())?;

    let weak = main_loop.downgrade();
    let _stop = stop_rx.attach(main_loop.loop_(), move |_| {
        if let Some(l) = weak.upgrade() {
            l.quit();
        }
    });
    main_loop.run();
    let _ = stream.disconnect();
    Ok(())
}

impl Data {
    fn quit(&self) {
        if let Some(l) = self.main_loop.upgrade() {
            l.quit();
        }
    }
}

fn on_state(stream: &pipewire::stream::Stream, d: &mut Data, new: StreamState) {
    log::debug!("stream state {new:?}");
    match new {
        StreamState::Paused => {
            if let Some(tx) = d.reported.take() {
                let serial = stream_serial(stream);
                let _ = tx.send(Ok((
                    stream.node_id(),
                    serial,
                    d.constraints.width,
                    d.constraints.height,
                )));
            }
        }
        StreamState::Error(e) => {
            if let Some(tx) = d.reported.take() {
                let _ = tx.send(Err(format!("stream error: {e}")));
            }
        }
        _ => {}
    }
}

/// PipeWire's `object.serial` for the stream's node, read from its
/// properties once connected (the frontend prefers it over the id).
fn stream_serial(stream: &pipewire::stream::Stream) -> u64 {
    stream
        .properties()
        .get("object.serial")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

fn on_param(stream: &pipewire::stream::Stream, d: &mut Data, id: u32, pod: Option<&Pod>) {
    if id != spa_sys::SPA_PARAM_Format {
        return;
    }
    let Some(pod) = pod else { return };
    let mut info = spa::param::video::VideoInfoRaw::new();
    if info.parse(pod).is_err() {
        log::warn!("could not parse the negotiated format");
        return;
    }
    let size = info.size();
    log::info!(
        "format negotiated: {:?} {}x{}",
        info.format(),
        size.width,
        size.height
    );
    d.format = match info.format() {
        spa::param::video::VideoFormat::BGRA => wl_shm::Format::Argb8888,
        spa::param::video::VideoFormat::BGRx => wl_shm::Format::Xrgb8888,
        spa::param::video::VideoFormat::RGBA => wl_shm::Format::Abgr8888,
        spa::param::video::VideoFormat::RGBx => wl_shm::Format::Xbgr8888,
        _ => d.format,
    };
    let fourcc = fourcc_of_shm_format(d.format);
    d.modifier = None;
    match modifier_prop(pod) {
        Some(ModifierProp::Unfixated(candidates)) => {
            // The consumer left the choice to us: a test allocation
            // picks the driver's preferred modifier, and the format
            // goes back fixated. Without a device, or when nothing
            // allocates, the dma-buf pods are withdrawn and the
            // consumer falls through to shm.
            let picked = d.gbm.as_ref().and_then(|g| {
                g.allocate(size.width, size.height, fourcc, &candidates)
                    .map_err(|e| log::warn!("test allocation: {e}"))
                    .ok()
                    .map(|bo| (u64::from(bo.modifier()), bo.plane_count()))
            });
            let fixated = match picked {
                Some((m, planes)) => {
                    log::info!(
                        "modifier {m:#x} picked from {} candidates, {planes} plane(s)",
                        candidates.len()
                    );
                    Some((fourcc, m))
                }
                None => {
                    log::info!("no dma-buf modifier allocates; shm only");
                    d.gbm = None;
                    None
                }
            };
            let params = format_params(&d.constraints, d.format, d.gbm.is_some(), fixated);
            let mut refs: Vec<&Pod> = params.iter().map(|p| &**p).collect();
            if let Err(e) = stream.update_params(&mut refs) {
                log::warn!("update_params: {e}");
            }
            return;
        }
        Some(ModifierProp::Fixed(m)) => {
            match d
                .gbm
                .as_ref()
                .map(|g| g.plane_count(size.width, size.height, fourcc, m))
            {
                Some(Ok(planes)) => d.modifier = Some((m, planes)),
                Some(Err(e)) => log::warn!("modifier {m:#x} negotiated: {e}"),
                None => log::warn!("modifier {m:#x} negotiated without a device"),
            }
        }
        None => {}
    }
    log::info!(
        "buffers: {}",
        match d.modifier {
            Some((m, planes)) =>
                format!("dma-buf modifier {m:#x}, {planes} plane(s), MemFd fallback"),
            None => "MemFd".to_string(),
        }
    );
    let cursor_meta = d
        .cursor
        .as_ref()
        .map(|c| cursor_meta_size(c.buf.width, c.buf.height));
    let params = buffer_params(
        size.width,
        size.height,
        d.modifier.map(|(_, p)| p),
        cursor_meta,
    );
    let mut refs: Vec<&Pod> = params.iter().map(|p| &**p).collect();
    if let Err(e) = stream.update_params(&mut refs) {
        log::warn!("update_params: {e}");
    }
}

/// The modifier property of a negotiated format, if there is one.
enum ModifierProp {
    /// The consumer's choice, still to be fixated: default first.
    Unfixated(Vec<u64>),
    Fixed(u64),
}

fn modifier_prop(pod: &Pod) -> Option<ModifierProp> {
    let prop = pod
        .as_object()
        .ok()?
        .find_prop(Id(spa_sys::SPA_FORMAT_VIDEO_modifier))?;
    let (_, value) = PodDeserializer::deserialize_from::<pod::Value>(prop.value().as_bytes())
        .map_err(|e| log::warn!("modifier prop: {e:?}"))
        .ok()?;
    match value {
        pod::Value::Long(m) => Some(ModifierProp::Fixed(m as u64)),
        pod::Value::Choice(pod::ChoiceValue::Long(Choice(_, choice))) => match choice {
            ChoiceEnum::Enum {
                default,
                alternatives,
            } if prop.flags().contains(pod::PodPropFlags::DONT_FIXATE) => {
                let mut mods = vec![default as u64];
                mods.extend(alternatives.iter().map(|m| *m as u64));
                mods.dedup();
                Some(ModifierProp::Unfixated(mods))
            }
            ChoiceEnum::Enum { default, .. } | ChoiceEnum::None(default) => {
                Some(ModifierProp::Fixed(default as u64))
            }
            other => {
                log::warn!("unexpected modifier choice {other:?}");
                None
            }
        },
        other => {
            log::warn!("unexpected modifier value {other:?}");
            None
        }
    }
}

/// Back a PipeWire buffer: a dma-buf when one was negotiated and the
/// buffer allows it (`datas[0].type_` is the mask of allowed types under
/// `ALLOC_BUFFERS`), a memfd otherwise or when the allocation fails.
/// Errs when the buffer got neither.
fn on_add_buffer(d: &mut Data, buf: *mut pw_sys::pw_buffer) -> Result<(), String> {
    let (w, h) = (d.constraints.width, d.constraints.height);
    // SAFETY: PipeWire passes add_buffer a live pw_buffer whose spa_buffer
    // it owns until remove_buffer.
    let (sb, n_datas) = unsafe { ((*buf).buffer, (*(*buf).buffer).n_datas as usize) };
    if n_datas < 1 {
        return Err("the buffer has no data planes".into());
    }
    // SAFETY: `datas` has `n_datas` entries, at least one.
    let allowed = unsafe { (*(*sb).datas).type_ };
    let wants_dmabuf = allowed & (1 << spa_sys::SPA_DATA_DmaBuf) != 0;
    if let (Some((modifier, _)), Some(gbm), true) = (d.modifier, d.gbm.as_ref(), wants_dmabuf) {
        let fourcc = fourcc_of_shm_format(d.format);
        match d.wl.dmabuf_buffer(gbm, w, h, fourcc, &[modifier]) {
            Ok(dma) if dma.planes.len() <= n_datas => {
                if d.buffers.is_empty() {
                    log::info!(
                        "buffer type dmabuf: {:#x} modifier {:#x}, {} plane(s), stride {}",
                        dma.fourcc,
                        dma.modifier,
                        dma.planes.len(),
                        dma.planes[0].stride
                    );
                }
                // SAFETY: `i` stays below `planes.len() <= n_datas`, so each
                // `datas` entry exists; ALLOC_BUFFERS gives every one a chunk.
                unsafe {
                    for (i, plane) in dma.planes.iter().enumerate() {
                        let data = &mut *(*sb).datas.add(i);
                        data.type_ = spa_sys::SPA_DATA_DmaBuf;
                        data.flags = spa_sys::SPA_DATA_FLAG_READABLE;
                        data.fd = plane.fd.as_raw_fd() as i64;
                        data.data = std::ptr::null_mut();
                        data.maxsize = plane.len;
                        data.mapoffset = 0;
                        let chunk = &mut *data.chunk;
                        chunk.offset = plane.offset;
                        // An aux plane (CCS) has fewer rows than the image.
                        chunk.size = (plane.stride * h).min(plane.len);
                        chunk.stride = plane.stride as i32;
                    }
                }
                d.buffers.push((buf as usize, Buffer::Dma(dma)));
                return Ok(());
            }
            Ok(dma) => log::warn!(
                "dma-buf has {} planes, the buffer {n_datas}; using memfd",
                dma.planes.len()
            ),
            Err(e) => log::warn!("dma-buf allocation: {e}; using memfd"),
        }
    }
    if allowed & (1 << spa_sys::SPA_DATA_MemFd) == 0 {
        return Err(format!(
            "the buffer allows neither dma-buf nor memfd ({allowed:#x})"
        ));
    }
    let shm = d.wl.shm_buffer(w, h, d.format)?;
    if d.buffers.is_empty() {
        log::info!("buffer type memfd: {:?} stride {}", d.format, shm.stride);
    }
    // SAFETY: `datas` has at least one entry, and ALLOC_BUFFERS gives it a
    // chunk.
    unsafe {
        let data = &mut *(*sb).datas;
        data.type_ = spa_sys::SPA_DATA_MemFd;
        data.flags = spa_sys::SPA_DATA_FLAG_READABLE | spa_sys::SPA_DATA_FLAG_MAPPABLE;
        data.fd = shm.fd.as_raw_fd() as i64;
        data.data = std::ptr::null_mut();
        data.maxsize = shm.size as u32;
        data.mapoffset = 0;
        let chunk = &mut *data.chunk;
        chunk.offset = 0;
        chunk.size = shm.size as u32;
        chunk.stride = shm.stride as i32;
    }
    d.buffers.push((buf as usize, Buffer::Shm(shm)));
    Ok(())
}

fn on_remove_buffer(d: &mut Data, buf: *mut pw_sys::pw_buffer) {
    if d.asked.as_ref().is_some_and(|(k, _)| *k == buf as usize) {
        d.asked = None;
    }
    d.buffers.retain(|(k, _)| *k != buf as usize);
    // The fds belong to the buffers just dropped.
    // SAFETY: PipeWire passes remove_buffer the live pw_buffer it passed
    // add_buffer; its `datas` has `n_datas` entries.
    unsafe {
        let sb = (*buf).buffer;
        for i in 0..(*sb).n_datas as usize {
            (*(*sb).datas.add(i)).fd = -1;
        }
    }
}

/// The Wayland connection's fd, for the loop to watch.
struct Fd(std::os::fd::RawFd);

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0
    }
}

/// Every graph cycle, and whenever the compositor sent events: hand the
/// answered buffer on, then ask for a frame into the next free one.
fn on_process(stream: &pipewire::stream::Stream, d: &mut Data) {
    if d.wl.stopped() {
        d.quit();
        return;
    }
    if d.asked.is_some() {
        let Some(result) = d.wl.answer() else {
            d.wl.flush();
            return;
        };
        let (buf, asked) = d.asked.take().expect("checked above");
        drop(asked);
        hand_on(stream, d, buf as *mut pw_sys::pw_buffer, result);
    }
    // SAFETY: the buffer is kept in `d.asked` until `hand_on` queues it
    // back, or queued back here.
    let buf = unsafe { stream.dequeue_raw_buffer() };
    if !buf.is_null() {
        match d.buffers.iter().position(|(k, _)| *k == buf as usize) {
            Some(i) => {
                let asked = d.wl.ask(&d.session, d.buffers[i].1.capture_buffer());
                d.asked = Some((buf as usize, asked));
            }
            // SAFETY: `buf` was dequeued from this stream just above.
            None => unsafe { stream.queue_raw_buffer(buf) },
        }
    }
    d.wl.flush();
}

/// Queue `buf` back on `stream` with the compositor's `result` for it:
/// the frame, or an empty or corrupted chunk.
fn hand_on(
    stream: &pipewire::stream::Stream,
    d: &mut Data,
    buf: *mut pw_sys::pw_buffer,
    result: Result<(), CaptureError>,
) {
    let Some(i) = d.buffers.iter().position(|(k, _)| *k == buf as usize) else {
        return;
    };
    d.calls += 1;
    if d.calls <= 8 || d.calls.is_multiple_of(200) {
        log::debug!("frame #{}: {:?}", d.calls, result);
    }
    // SAFETY: `buf` was dequeued and is ours until queued back; it is in
    // `d.buffers`, so on_add_buffer saw at least one data entry, which
    // ALLOC_BUFFERS gives a chunk.
    unsafe {
        let sb = (*buf).buffer;
        let chunk = &mut *(*(*sb).datas).chunk;
        match result {
            Ok(()) => {
                chunk.flags = 0;
                chunk.size = d.buffers[i].1.size();
                d.seq += 1;
                stamp_header(sb, d.seq, 0);
                stamp_cursor(sb, d);
            }
            Err(CaptureError::Failed(FailureReason::BufferConstraints)) => {
                // The source changed size: adopt the new constraints and
                // re-announce the format; PipeWire reallocates the buffers.
                chunk.flags = spa_sys::SPA_CHUNK_FLAG_CORRUPTED as i32;
                chunk.size = 0;
                d.constraints = d.wl.constraints();
                d.modifier = None;
                log::info!(
                    "constraints changed: {}x{}",
                    d.constraints.width,
                    d.constraints.height
                );
                let params = format_params(&d.constraints, d.format, d.gbm.is_some(), None);
                let mut refs: Vec<&Pod> = params.iter().map(|p| &**p).collect();
                let _ = stream.update_params(&mut refs);
            }
            Err(CaptureError::Failed(FailureReason::Stopped)) | Err(CaptureError::Gone) => {
                chunk.flags = spa_sys::SPA_CHUNK_FLAG_CORRUPTED as i32;
                chunk.size = 0;
                d.quit();
            }
            Err(CaptureError::Failed(_)) | Err(CaptureError::Timeout) => {
                chunk.flags = spa_sys::SPA_CHUNK_FLAG_CORRUPTED as i32;
                chunk.size = 0;
            }
        }
        stream.queue_raw_buffer(buf);
    }
}

/// Fill the `SPA_META_Cursor` if the consumer asked for one: the
/// pointer's position and hotspot, and its image while it is over the
/// source (captured now, into the cursor plane's buffer).
unsafe fn stamp_cursor(sb: *mut spa_sys::spa_buffer, d: &mut Data) {
    // SAFETY: `sb` is the buffer the caller dequeued from the stream and
    // still holds.
    let Some(meta) = (unsafe { find_meta(sb, spa_sys::SPA_META_Cursor) }) else {
        return;
    };
    let Some(plane) = d.cursor.as_mut() else {
        return;
    };
    let info = d.wl.cursor_info();
    let captured = if info.entered {
        match d.wl.capture_cursor(&d.session, &plane.buf, CURSOR_WAIT) {
            Ok(captured) => captured,
            Err(e) => {
                log::debug!("cursor capture: {e:?}");
                false
            }
        }
    } else {
        false
    };
    plane.id = plane.id.wrapping_add(1).max(1);
    let bitmap = captured.then(|| Bitmap {
        format: spa_video_format(plane.format),
        width: plane.buf.width,
        height: plane.buf.height,
        stride: plane.buf.stride,
        pixels: plane.map.as_slice(),
    });
    write_cursor_meta(
        // SAFETY: `meta` is an entry of `sb`'s meta array whose `data`
        // `find_meta` checked is non-null; it holds `size` bytes that
        // nothing else touches while we hold the buffer.
        unsafe { std::slice::from_raw_parts_mut((*meta).data as *mut u8, (*meta).size as usize) },
        plane.id,
        info,
        bitmap,
    );
}

/// The first meta of `type_` in `sb` that has memory behind it.
unsafe fn find_meta(sb: *mut spa_sys::spa_buffer, type_: u32) -> Option<*mut spa_sys::spa_meta> {
    // SAFETY: relies on the caller passing a live `sb`; its `metas` array
    // has `n_metas` entries.
    unsafe {
        (0..(*sb).n_metas as usize)
            .map(|i| (*sb).metas.add(i))
            .find(|m| (**m).type_ == type_ && !(**m).data.is_null())
    }
}

/// A cursor image for the meta.
struct Bitmap<'a> {
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
    pixels: &'a [u8],
}

/// The bytes `SPA_META_Cursor` needs for a `w`x`h` BGRA bitmap:
/// the cursor struct, the bitmap struct, the pixels.
fn cursor_meta_size(w: u32, h: u32) -> u32 {
    (std::mem::size_of::<spa_sys::spa_meta_cursor>()
        + std::mem::size_of::<spa_sys::spa_meta_bitmap>()) as u32
        + w * h * 4
}

/// Lay a `spa_meta_cursor` into `out`: id and position always; the
/// bitmap (and hotspot) only when there is one and it fits, else the
/// bitmap offset is 0, which the consumer reads as no cursor image.
fn write_cursor_meta(out: &mut [u8], id: u32, info: CursorInfo, bitmap: Option<Bitmap>) {
    let cur_len = std::mem::size_of::<spa_sys::spa_meta_cursor>();
    let bm_len = std::mem::size_of::<spa_sys::spa_meta_bitmap>();
    if out.len() < cur_len {
        return;
    }
    let mut cur = spa_sys::spa_meta_cursor {
        id,
        flags: 0,
        position: spa_sys::spa_point {
            x: info.x,
            y: info.y,
        },
        hotspot: spa_sys::spa_point {
            x: info.hotspot.0,
            y: info.hotspot.1,
        },
        bitmap_offset: 0,
    };
    if let Some(b) = bitmap {
        let need = cur_len + bm_len + b.pixels.len();
        if out.len() >= need && b.pixels.len() >= (b.stride * b.height) as usize {
            cur.bitmap_offset = cur_len as u32;
            let bm = spa_sys::spa_meta_bitmap {
                format: b.format,
                size: spa_sys::spa_rectangle {
                    width: b.width,
                    height: b.height,
                },
                stride: b.stride as i32,
                offset: bm_len as u32,
            };
            // SAFETY: both structs are plain C data; the byte views are
            // exactly their sizes.
            let bm_bytes =
                unsafe { std::slice::from_raw_parts(&bm as *const _ as *const u8, bm_len) };
            out[cur_len..cur_len + bm_len].copy_from_slice(bm_bytes);
            out[cur_len + bm_len..need]
                .copy_from_slice(&b.pixels[..b.pixels.len().min(need - cur_len - bm_len)]);
        }
    }
    // SAFETY: `cur` is plain C data and the view is exactly its size.
    let cur_bytes = unsafe { std::slice::from_raw_parts(&cur as *const _ as *const u8, cur_len) };
    out[..cur_len].copy_from_slice(cur_bytes);
}

/// The SPA video format of a wl_shm format (BGRA for ARGB8888, as the
/// stream's own mapping).
fn spa_video_format(f: wl_shm::Format) -> u32 {
    spa_format(f).unwrap_or(spa_sys::SPA_VIDEO_FORMAT_BGRA)
}

/// Fill the `SPA_META_Header` if the consumer asked for one.
unsafe fn stamp_header(sb: *mut spa_sys::spa_buffer, seq: u64, flags: u32) {
    // SAFETY: relies on the caller passing a live `sb`.
    for i in 0..unsafe { (*sb).n_metas } as usize {
        // SAFETY: `i < n_metas`, so `m` is an entry of `sb`'s meta array; `h`
        // is only written once `size` covers a whole `spa_meta_header`.
        unsafe {
            let m = (*sb).metas.add(i);
            if (*m).type_ == spa_sys::SPA_META_Header
                && (*m).size as usize >= std::mem::size_of::<spa_sys::spa_meta_header>()
            {
                let h = (*m).data as *mut spa_sys::spa_meta_header;
                let mut ts = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
                (*h).flags = flags;
                (*h).offset = 0;
                (*h).pts = ts.tv_sec * 1_000_000_000 + ts.tv_nsec;
                (*h).dts_offset = 0;
                (*h).seq = seq;
            }
        }
    }
}

/// A serialized pod that derefs to `Pod`.
pub struct OwnedPod(Vec<u8>);

impl OwnedPod {
    fn serialize(value: &pod::Value) -> OwnedPod {
        let mut bytes = Vec::new();
        pod::serialize::PodSerializer::serialize(Cursor::new(&mut bytes), value).expect("pod");
        OwnedPod(bytes)
    }
}

impl std::ops::Deref for OwnedPod {
    type Target = Pod;
    fn deref(&self) -> &Pod {
        Pod::from_bytes(&self.0).expect("valid pod")
    }
}

fn spa_format(f: wl_shm::Format) -> Option<u32> {
    Some(match f {
        wl_shm::Format::Argb8888 => spa_sys::SPA_VIDEO_FORMAT_BGRA,
        wl_shm::Format::Xrgb8888 => spa_sys::SPA_VIDEO_FORMAT_BGRx,
        wl_shm::Format::Abgr8888 => spa_sys::SPA_VIDEO_FORMAT_RGBA,
        wl_shm::Format::Xbgr8888 => spa_sys::SPA_VIDEO_FORMAT_RGBx,
        _ => return None,
    })
}

fn prop(key: u32, value: pod::Value) -> pod::Property {
    pod::Property {
        key,
        flags: pod::PropertyFlags::empty(),
        value,
    }
}

/// The modifier property of an `EnumFormat` pod.
enum Modifiers<'a> {
    /// A fixated modifier (MANDATORY).
    Fixed(u64),
    /// The compositor's list, for the consumer to intersect with its
    /// own and send back unfixated (MANDATORY | DONT_FIXATE).
    Choice(&'a [u64]),
}

/// The `EnumFormat` pods, most wanted first: the fixated dma-buf format
/// if there is one, then (with a device) one pod per dma-buf format
/// with its explicit modifiers as a choice, then the shm formats with
/// `preferred` ahead of the rest.
fn format_params(
    c: &Constraints,
    preferred: wl_shm::Format,
    dmabuf: bool,
    fixated: Option<(u32, u64)>,
) -> Vec<OwnedPod> {
    let invalid: u64 = gbm::Modifier::Invalid.into();
    let mut pods = Vec::new();
    if let Some((fourcc, m)) = fixated
        && let Some(f) = shm_format_of_fourcc(fourcc)
    {
        pods.extend(format_pod(c, f, Some(Modifiers::Fixed(m))));
    }
    if dmabuf {
        for df in &c.dmabuf_formats {
            let mods: Vec<u64> = df
                .modifiers
                .iter()
                .copied()
                .filter(|m| *m != invalid)
                .collect();
            let Some(f) = shm_format_of_fourcc(df.fourcc) else {
                continue;
            };
            if mods.is_empty() {
                continue;
            }
            pods.extend(format_pod(c, f, Some(Modifiers::Choice(&mods))));
        }
    }
    let mut formats: Vec<wl_shm::Format> = vec![preferred];
    formats.extend(c.shm_formats.iter().copied().filter(|f| *f != preferred));
    pods.extend(formats.into_iter().filter_map(|f| format_pod(c, f, None)));
    pods
}

/// One `EnumFormat` pod for `f` at the session's size; `None` when the
/// format has no SPA name or the modifier choice is empty.
fn format_pod(
    c: &Constraints,
    f: wl_shm::Format,
    modifiers: Option<Modifiers>,
) -> Option<OwnedPod> {
    let id = spa_format(f)?;
    let mut properties = vec![
        prop(
            spa_sys::SPA_FORMAT_mediaType,
            pod::Value::Id(Id(spa_sys::SPA_MEDIA_TYPE_video)),
        ),
        prop(
            spa_sys::SPA_FORMAT_mediaSubtype,
            pod::Value::Id(Id(spa_sys::SPA_MEDIA_SUBTYPE_raw)),
        ),
        prop(spa_sys::SPA_FORMAT_VIDEO_format, pod::Value::Id(Id(id))),
    ];
    match modifiers {
        Some(Modifiers::Fixed(m)) => properties.push(pod::Property {
            key: spa_sys::SPA_FORMAT_VIDEO_modifier,
            flags: pod::PropertyFlags::MANDATORY,
            value: pod::Value::Long(m as i64),
        }),
        Some(Modifiers::Choice(mods)) => {
            let (first, rest) = mods.split_first()?;
            let mut alternatives = vec![*first as i64];
            alternatives.extend(rest.iter().map(|m| *m as i64));
            properties.push(pod::Property {
                key: spa_sys::SPA_FORMAT_VIDEO_modifier,
                flags: pod::PropertyFlags::MANDATORY | pod::PropertyFlags::DONT_FIXATE,
                value: pod::Value::Choice(pod::ChoiceValue::Long(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Enum {
                        default: *first as i64,
                        alternatives,
                    },
                ))),
            })
        }
        None => {}
    }
    properties.extend([
        prop(
            spa_sys::SPA_FORMAT_VIDEO_size,
            pod::Value::Rectangle(Rectangle {
                width: c.width,
                height: c.height,
            }),
        ),
        prop(
            spa_sys::SPA_FORMAT_VIDEO_framerate,
            pod::Value::Fraction(Fraction { num: 0, denom: 1 }),
        ),
        prop(
            spa_sys::SPA_FORMAT_VIDEO_maxFramerate,
            pod::Value::Choice(pod::ChoiceValue::Fraction(Choice(
                ChoiceFlags::empty(),
                ChoiceEnum::Range {
                    default: Fraction { num: 60, denom: 1 },
                    min: Fraction { num: 1, denom: 1 },
                    max: Fraction { num: 60, denom: 1 },
                },
            ))),
        ),
    ]);
    Some(OwnedPod::serialize(&pod::Value::Object(pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_Format,
        id: spa_sys::SPA_PARAM_EnumFormat,
        properties,
    })))
}

/// `Buffers` (2..8 of them; DmaBuf with `dmabuf_planes` blocks and
/// MemFd when a modifier was negotiated, MemFd alone otherwise) and
/// `Meta` params: the header, and the cursor of `cursor_meta` bytes in
/// metadata cursor mode.
fn buffer_params(
    width: u32,
    height: u32,
    dmabuf_planes: Option<u32>,
    cursor_meta: Option<u32>,
) -> Vec<OwnedPod> {
    let (blocks, data_types) = match dmabuf_planes {
        Some(planes) => (
            planes.max(1) as i32,
            vec![1 << spa_sys::SPA_DATA_DmaBuf, 1 << spa_sys::SPA_DATA_MemFd],
        ),
        None => (1, vec![1 << spa_sys::SPA_DATA_MemFd]),
    };
    let buffers = OwnedPod::serialize(&pod::Value::Object(pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_ParamBuffers,
        id: spa_sys::SPA_PARAM_Buffers,
        properties: vec![
            prop(
                spa_sys::SPA_PARAM_BUFFERS_buffers,
                pod::Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Range {
                        default: 4,
                        min: 2,
                        max: 8,
                    },
                ))),
            ),
            prop(spa_sys::SPA_PARAM_BUFFERS_blocks, pod::Value::Int(blocks)),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_size,
                pod::Value::Int((width * height * 4) as i32),
            ),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_stride,
                pod::Value::Int((width * 4) as i32),
            ),
            prop(spa_sys::SPA_PARAM_BUFFERS_align, pod::Value::Int(16)),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_dataType,
                pod::Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: data_types[0],
                        flags: data_types,
                    },
                ))),
            ),
        ],
    }));
    let header = OwnedPod::serialize(&pod::Value::Object(pod::Object {
        type_: spa_sys::SPA_TYPE_OBJECT_ParamMeta,
        id: spa_sys::SPA_PARAM_Meta,
        properties: vec![
            prop(
                spa_sys::SPA_PARAM_META_type,
                pod::Value::Id(Id(spa_sys::SPA_META_Header)),
            ),
            prop(
                spa_sys::SPA_PARAM_META_size,
                pod::Value::Int(std::mem::size_of::<spa_sys::spa_meta_header>() as i32),
            ),
        ],
    }));
    let mut params = vec![buffers, header];
    if let Some(size) = cursor_meta {
        params.push(OwnedPod::serialize(&pod::Value::Object(pod::Object {
            type_: spa_sys::SPA_TYPE_OBJECT_ParamMeta,
            id: spa_sys::SPA_PARAM_Meta,
            properties: vec![
                prop(
                    spa_sys::SPA_PARAM_META_type,
                    pod::Value::Id(Id(spa_sys::SPA_META_Cursor)),
                ),
                prop(spa_sys::SPA_PARAM_META_size, pod::Value::Int(size as i32)),
            ],
        })));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32(b: &[u8], at: usize) -> u32 {
        u32::from_ne_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn find_meta_skips_a_meta_without_data() {
        let mut bytes = [0u8; 8];
        let mut metas = [
            spa_sys::spa_meta {
                type_: spa_sys::SPA_META_Cursor,
                size: 0,
                data: std::ptr::null_mut(),
            },
            spa_sys::spa_meta {
                type_: spa_sys::SPA_META_Cursor,
                size: bytes.len() as u32,
                data: bytes.as_mut_ptr().cast(),
            },
        ];
        let mut sb = spa_sys::spa_buffer {
            n_metas: 1,
            n_datas: 0,
            metas: metas.as_mut_ptr(),
            datas: std::ptr::null_mut(),
        };
        // SAFETY: `sb` points at `metas`, which outlives both calls.
        let first = unsafe { find_meta(&mut sb, spa_sys::SPA_META_Cursor) };
        assert_eq!(first, None, "a meta with null data is not found");
        sb.n_metas = 2;
        // SAFETY: as above.
        let second = unsafe { find_meta(&mut sb, spa_sys::SPA_META_Cursor) };
        assert_eq!(second, Some(metas[1..].as_mut_ptr()));
    }

    #[test]
    fn cursor_meta_carries_position_and_a_bitmap_when_it_fits() {
        let info = CursorInfo {
            entered: true,
            x: 100,
            y: 200,
            hotspot: (3, 4),
        };
        let px = vec![0xAAu8; 2 * 2 * 4];
        let bm = Bitmap {
            format: spa_sys::SPA_VIDEO_FORMAT_BGRA,
            width: 2,
            height: 2,
            stride: 8,
            pixels: &px,
        };
        let mut out = vec![0u8; cursor_meta_size(2, 2) as usize];
        write_cursor_meta(&mut out, 7, info, Some(bm));
        let cur = std::mem::size_of::<spa_sys::spa_meta_cursor>();
        let bml = std::mem::size_of::<spa_sys::spa_meta_bitmap>();
        assert_eq!(read_u32(&out, 0), 7, "id");
        assert_eq!(read_u32(&out, 8) as i32, 100);
        assert_eq!(read_u32(&out, 12) as i32, 200);
        assert_eq!(read_u32(&out, 16) as i32, 3);
        assert_eq!(read_u32(&out, 24) as usize, cur, "bitmap offset");
        assert_eq!(read_u32(&out, cur), spa_sys::SPA_VIDEO_FORMAT_BGRA);
        assert_eq!(read_u32(&out, cur + 4), 2);
        assert_eq!(read_u32(&out, cur + 16) as usize, bml, "pixel offset");
        assert_eq!(&out[cur + bml..], &px[..]);
    }

    #[test]
    fn a_left_pointer_or_a_small_meta_has_no_bitmap() {
        let info = CursorInfo {
            entered: false,
            x: 1,
            y: 2,
            hotspot: (0, 0),
        };
        let mut out = vec![0xFFu8; cursor_meta_size(2, 2) as usize];
        write_cursor_meta(&mut out, 1, info, None);
        assert_eq!(read_u32(&out, 24), 0);
        let px = vec![0u8; 16];
        let bm = Bitmap {
            format: 12,
            width: 2,
            height: 2,
            stride: 8,
            pixels: &px,
        };
        let mut small = vec![0u8; std::mem::size_of::<spa_sys::spa_meta_cursor>() + 4];
        write_cursor_meta(&mut small, 2, info, Some(bm));
        assert_eq!(read_u32(&small, 0), 2);
        assert_eq!(read_u32(&small, 24), 0, "does not fit: no bitmap");
    }
}
