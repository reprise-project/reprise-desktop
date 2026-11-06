//! The shared screen's PipeWire stream, read on its own thread. A [`Frame`] keeps its buffer
//! out of the stream, away from the producer, until its last reference drops.

use bytes::Bytes;
use pipewire::spa;
use pipewire::spa::pod::deserialize::PodDeserializer;
use pipewire::spa::pod::{self, Pod};
use pipewire::spa::sys as spa_sys;
use pipewire::spa::utils::{Choice, ChoiceEnum, ChoiceFlags, Fraction, Id, Rectangle};
use pipewire::stream::{StreamFlags, StreamRc, StreamState};
use pipewire::sys as pw_sys;
use std::cell::RefCell;
use std::io::Cursor as IoCursor;
use std::mem::size_of;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
pub const XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub const ARGB8888: u32 = u32::from_le_bytes(*b"AR24");
pub const XBGR8888: u32 = u32::from_le_bytes(*b"XB24");
pub const ABGR8888: u32 = u32::from_le_bytes(*b"AB24");

/// The SPA video formats taken, each with its DRM fourcc.
const FORMATS: [(u32, u32); 4] = [
    (spa_sys::SPA_VIDEO_FORMAT_BGRx, XRGB8888),
    (spa_sys::SPA_VIDEO_FORMAT_BGRA, ARGB8888),
    (spa_sys::SPA_VIDEO_FORMAT_RGBx, XBGR8888),
    (spa_sys::SPA_VIDEO_FORMAT_RGBA, ABGR8888),
];

/// The damage meta's room, as the producer offers it.
pub const MAX_DAMAGE_RECTS: i32 = 16;
/// The largest cursor image the cursor meta is sized for.
const MAX_CURSOR_PX: u32 = 256;

/// A rectangle of the picture, in stream pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn full((width, height): (u32, u32)) -> Rect {
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}

/// One picture of the stream and what changed since the one before. A clone shares the buffer.
#[derive(Clone)]
pub struct Frame {
    pub buffer: FrameBuffer,
    pub size: (u32, u32),
    pub damage: Vec<Rect>,
    pub cursor: Option<Cursor>,
    /// None for a frame made outside a stream.
    _lease: Option<Arc<Lease>>,
}

impl Frame {
    /// A whole frame of `buffer` with no stream behind it.
    pub fn unleased(buffer: FrameBuffer, size: (u32, u32)) -> Frame {
        Frame {
            buffer,
            size,
            damage: vec![Rect::full(size)],
            cursor: None,
            _lease: None,
        }
    }
}

/// Where a frame's pixels are; `fourcc` is a DRM format.
#[derive(Clone)]
pub enum FrameBuffer {
    /// A dma-buf; `planes` holds the main plane first, then any the modifier adds, each in one
    /// of the distinct buffers `objects`.
    Dmabuf {
        objects: Vec<Arc<OwnedFd>>,
        fourcc: u32,
        modifier: u64,
        planes: Vec<Plane>,
    },
    Shm {
        data: Bytes,
        fourcc: u32,
        stride: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plane {
    /// The plane's index in its frame's `objects`.
    pub object: usize,
    pub offset: u32,
    pub stride: u32,
}

/// The pointer as the frame saw it: its hotspot's position in stream pixels, and its image
/// while it is over the shared output.
#[derive(Clone, Debug, PartialEq)]
pub struct Cursor {
    pub position: (i32, i32),
    pub image: Option<CursorImage>,
}

/// Premultiplied BGRA rows of `width * 4` bytes, top-down.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorImage {
    pub width: u32,
    pub height: u32,
    pub hotspot: (i32, i32),
    pub bgra: Bytes,
}

/// The stream's picture size after a format negotiation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeChange {
    pub size: (u32, u32),
}

/// The running stream; dropping it ends the stream's thread.
pub struct Stream {
    control: pipewire::channel::Sender<Control>,
}

impl Stream {
    pub fn offer(&self) -> Offer {
        Offer(self.control.clone())
    }
}

/// Changes the dma-buf modifiers the stream takes; the producer then negotiates again.
#[derive(Clone)]
pub struct Offer(pipewire::channel::Sender<Control>);

impl Offer {
    pub fn modifiers(&self, modifiers: &[u64]) {
        let _ = self.0.send(Control::Modifiers {
            modifiers: modifiers.to_vec(),
            again: false,
        });
    }

    /// Offer `modifiers` even if they are already offered, so the producer negotiates again.
    pub fn again(&self, modifiers: &[u64]) {
        let _ = self.0.send(Control::Modifiers {
            modifiers: modifiers.to_vec(),
            again: true,
        });
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Stop);
    }
}

enum Control {
    Release { buf: usize, generation: u64 },
    Modifiers { modifiers: Vec<u64>, again: bool },
    Stop,
}

/// Gives a frame's buffer back to the stream when dropped.
struct Lease {
    control: pipewire::channel::Sender<Control>,
    buf: usize,
    generation: u64,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Release {
            buf: self.buf,
            generation: self.generation,
        });
    }
}

/// Consume `node`'s stream on the PipeWire remote `fd`, offering dma-bufs with `modifiers` and
/// memfds; the receivers close when it ends, and `gone` runs first when it ends because its
/// screen went.
pub fn spawn(
    fd: OwnedFd,
    node: u32,
    modifiers: &[u64],
    gone: impl Fn() + Send + 'static,
) -> Result<
    (
        Stream,
        UnboundedReceiver<Frame>,
        UnboundedReceiver<SizeChange>,
    ),
    String,
> {
    let modifiers = modifiers.to_vec();
    let (frames_tx, frames) = unbounded_channel();
    let (sizes_tx, sizes) = unbounded_channel();
    let (control, control_rx) = pipewire::channel::channel();
    let back = control.clone();
    std::thread::Builder::new()
        .name("reprise-rdp-stream".into())
        .spawn(move || {
            let out = Out {
                frames: frames_tx,
                sizes: sizes_tx,
                control: back,
            };
            if let Err(e) = run(fd, node, modifiers, out, control_rx, gone) {
                log::warn!("screen stream: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    Ok((Stream { control }, frames, sizes))
}

struct Out {
    frames: UnboundedSender<Frame>,
    sizes: UnboundedSender<SizeChange>,
    control: pipewire::channel::Sender<Control>,
}

/// The negotiated picture.
#[derive(Clone, Copy)]
struct Negotiated {
    size: (u32, u32),
    fourcc: u32,
    modifier: Option<u64>,
}

/// A buffer the stream gave us, by address; `generation` tells a buffer apart from a later one
/// at the same address.
struct Held {
    buf: usize,
    generation: u64,
    memory: Memory,
    /// Frames from this buffer not yet dropped; the buffer goes back to the stream with the last.
    frames: u32,
}

/// Count a frame of `buf` at `generation` dropped; true when it was the buffer's last, and the
/// buffer goes back to the stream.
fn release(held: &mut [Held], buf: usize, generation: u64) -> bool {
    match held
        .iter_mut()
        .find(|h| h.buf == buf && h.generation == generation)
    {
        Some(h) => {
            h.frames -= 1;
            h.frames == 0
        }
        None => false,
    }
}

/// Whether frames from `buf` are still out.
fn out(held: &[Held], buf: usize) -> bool {
    held.iter().any(|h| h.buf == buf && h.frames > 0)
}

enum Memory {
    Shm(Arc<Mapping>),
    Dmabuf {
        objects: Vec<Arc<OwnedFd>>,
        /// Each data plane's index in `objects`.
        plane_objects: Vec<usize>,
    },
}

struct Data {
    out: Out,
    format: Option<Negotiated>,
    sent_size: Option<(u32, u32)>,
    held: Vec<Held>,
    generations: u64,
    /// The next frame is whole: the first after a negotiation.
    fresh: bool,
    seq: Option<u64>,
    main_loop: pipewire::main_loop::MainLoopWeak,
}

impl Data {
    fn quit(&self) {
        if let Some(l) = self.main_loop.upgrade() {
            l.quit();
        }
    }
}

fn run(
    fd: OwnedFd,
    node: u32,
    modifiers: Vec<u64>,
    out: Out,
    control: pipewire::channel::Receiver<Control>,
    gone: impl Fn() + 'static,
) -> Result<(), String> {
    pipewire::init();
    let main_loop = pipewire::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pipewire::context::ContextRc::new(&main_loop, None).map_err(|e| e.to_string())?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;
    // A stream whose node went only pauses, waiting for another: the screen it showed is gone.
    let registry = core.get_registry_rc().map_err(|e| e.to_string())?;
    let quit = main_loop.downgrade();
    let _node_gone = registry
        .add_listener_local()
        .global_remove(move |id| {
            if id == node {
                log::info!("screen stream: the shared screen is gone");
                gone();
                if let Some(l) = quit.upgrade() {
                    l.quit();
                }
            }
        })
        .register();
    let stream = StreamRc::new(
        core,
        "Remote Desktop",
        pipewire::properties::properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
            *pipewire::keys::APP_NAME => "Remote Desktop",
        },
    )
    .map_err(|e| e.to_string())?;
    let params = format_params(&modifiers);
    let data = Rc::new(RefCell::new(Data {
        out,
        format: None,
        sent_size: None,
        held: Vec::new(),
        generations: 0,
        fresh: true,
        seq: None,
        main_loop: main_loop.downgrade(),
    }));
    let _listener = stream
        .add_local_listener_with_user_data(data.clone())
        .state_changed(|_, d, old, new| {
            log::debug!("screen stream {old:?} -> {new:?}");
            match new {
                StreamState::Error(e) => {
                    log::warn!("screen stream: {e}");
                    d.borrow().quit();
                }
                StreamState::Unconnected => d.borrow().quit(),
                _ => {}
            }
        })
        .param_changed(|stream, d, id, pod| on_param(stream, &mut d.borrow_mut(), id, pod))
        .remove_buffer(|_, d, buf| d.borrow_mut().held.retain(|h| h.buf != buf as usize))
        .process(|stream, d| on_process(stream, &mut d.borrow_mut()))
        .register()
        .map_err(|e| e.to_string())?;
    let mut refs: Vec<&Pod> = params.iter().map(|p| p.pod()).collect();
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            StreamFlags::AUTOCONNECT,
            &mut refs,
        )
        .map_err(|e| e.to_string())?;
    let returned = stream.clone();
    let weak = main_loop.downgrade();
    let offered = RefCell::new(modifiers);
    let _control = control.attach(main_loop.loop_(), move |c| match c {
        Control::Release { buf, generation } => {
            if release(&mut data.borrow_mut().held, buf, generation) {
                // SAFETY: the buffer was dequeued by on_process and is still the stream's, as its
                // generation shows; its last lease just dropped.
                unsafe { returned.queue_raw_buffer(buf as *mut pw_sys::pw_buffer) };
            }
        }
        Control::Modifiers {
            modifiers: m,
            again,
        } => {
            if !again && m == *offered.borrow() {
                return;
            }
            let params = format_params(&m);
            let mut refs: Vec<&Pod> = params.iter().map(|p| p.pod()).collect();
            match returned.update_params(&mut refs) {
                Ok(()) => log::info!("screen stream: offering modifiers {m:#x?}"),
                Err(e) => log::warn!("screen stream: update_params: {e}"),
            }
            *offered.borrow_mut() = m;
        }
        Control::Stop => {
            if let Some(l) = weak.upgrade() {
                l.quit();
            }
        }
    });
    main_loop.run();
    let _ = stream.disconnect();
    Ok(())
}

fn on_param(stream: &pipewire::stream::Stream, d: &mut Data, id: u32, pod: Option<&Pod>) {
    if id != spa_sys::SPA_PARAM_Format {
        return;
    }
    let Some(pod) = pod else {
        d.format = None;
        return;
    };
    let mut info = spa::param::video::VideoInfoRaw::new();
    if info.parse(pod).is_err() {
        log::warn!("screen stream: unreadable format");
        return;
    }
    let Some(&(_, fourcc)) = FORMATS.iter().find(|(f, _)| *f == info.format().as_raw()) else {
        log::warn!("screen stream: unexpected format {:?}", info.format());
        return;
    };
    let modifier = match modifier_prop(pod) {
        Modifier::Fixed(m) => Some(m),
        Modifier::Unfixated => return,
        Modifier::None => None,
    };
    let size = (info.size().width, info.size().height);
    log::info!(
        "screen stream: {:?} {}x{}, {}",
        info.format(),
        size.0,
        size.1,
        match modifier {
            Some(m) => format!("dma-buf modifier {m:#x}"),
            None => "memfd".into(),
        }
    );
    d.format = Some(Negotiated {
        size,
        fourcc,
        modifier,
    });
    d.fresh = true;
    if d.sent_size != Some(size) {
        d.sent_size = Some(size);
        let _ = d.out.sizes.send(SizeChange { size });
    }
    let params = buffer_params(modifier.is_some());
    let mut refs: Vec<&Pod> = params.iter().map(|p| p.pod()).collect();
    if let Err(e) = stream.update_params(&mut refs) {
        log::warn!("screen stream: update_params: {e}");
    }
}

enum Modifier {
    Fixed(u64),
    /// A choice the producer still has to fixate.
    Unfixated,
    None,
}

fn modifier_prop(pod: &Pod) -> Modifier {
    let Some(prop) =
        (pod.as_object().ok()).and_then(|o| o.find_prop(Id(spa_sys::SPA_FORMAT_VIDEO_modifier)))
    else {
        return Modifier::None;
    };
    match PodDeserializer::deserialize_from::<pod::Value>(prop.value().as_bytes()) {
        Ok((_, pod::Value::Long(m))) => Modifier::Fixed(m as u64),
        Ok((_, pod::Value::Choice(pod::ChoiceValue::Long(Choice(_, ChoiceEnum::None(m)))))) => {
            Modifier::Fixed(m as u64)
        }
        Ok(_) => Modifier::Unfixated,
        Err(_) => Modifier::None,
    }
}

fn on_process(stream: &pipewire::stream::Stream, d: &mut Data) {
    loop {
        // SAFETY: the buffer is ours until queued back, which `hand_on` does at once or a lease
        // does through the control channel.
        let buf = unsafe { stream.dequeue_raw_buffer() };
        if buf.is_null() {
            return;
        }
        if out(&d.held, buf as usize) {
            // Delivered again while its frames are out: pw_stream counted both deliveries busy
            // but takes a dequeued buffer back once, so this delivery is returned unused.
            // SAFETY: `buf` was just dequeued; return_buffer puts it first in the queue, so the
            // dequeue takes it again.
            unsafe {
                let res = pw_sys::pw_stream_return_buffer(stream.as_raw_ptr(), buf);
                assert!(
                    res >= 0,
                    "pw_stream_return_buffer takes a dequeued buffer back"
                );
                let again = stream.dequeue_raw_buffer();
                assert_eq!(again, buf, "a returned buffer is dequeued first");
            }
        }
        if !hand_on(d, buf) && !out(&d.held, buf as usize) {
            // SAFETY: dequeued just above and not handed on.
            unsafe { stream.queue_raw_buffer(buf) };
        }
    }
}

/// Send `buf`'s picture on as a [`Frame`]; false when it holds none and goes straight back.
fn hand_on(d: &mut Data, buf: *mut pw_sys::pw_buffer) -> bool {
    let Some(format) = d.format else {
        return false;
    };
    // SAFETY: `buf` is a live dequeued pw_buffer; its spa_buffer has `n_datas` entries.
    let (sb, data) = unsafe {
        let sb = (*buf).buffer;
        if (*sb).n_datas < 1 {
            return false;
        }
        (sb, &*(*sb).datas)
    };
    // SAFETY: every data entry of a negotiated buffer has a chunk.
    let chunk = unsafe { &*data.chunk };
    if chunk.flags & spa_sys::SPA_CHUNK_FLAG_CORRUPTED as i32 != 0 || chunk.size == 0 {
        return false;
    }
    let Some(i) = held_index(d, buf, sb) else {
        return false;
    };
    // SAFETY: `sb` stays live while the buffer is dequeued; each meta's data holds `size` bytes.
    let (seq, damage_meta, cursor_meta) = unsafe {
        (
            find_meta(sb, spa_sys::SPA_META_Header).and_then(|m| {
                (m.len() >= size_of::<spa_sys::spa_meta_header>()).then(|| {
                    std::ptr::read_unaligned(m.as_ptr() as *const spa_sys::spa_meta_header).seq
                })
            }),
            find_meta(sb, spa_sys::SPA_META_VideoDamage),
            find_meta(sb, spa_sys::SPA_META_Cursor),
        )
    };
    let stride = chunk.stride.max(0) as u32;
    let (start, len) = (
        chunk.offset as usize,
        stride as usize * format.size.1 as usize,
    );
    if let Memory::Shm(map) = &d.held[i].memory
        && start + len > map.len
    {
        return false;
    }
    let missed = std::mem::take(&mut d.fresh) || seq_gap(d.seq, seq);
    if missed {
        log::debug!(
            "screen stream: whole frame after seq {:?}, now {seq:?}",
            d.seq
        );
    }
    d.seq = seq;
    d.held[i].frames += 1;
    let lease = Arc::new(Lease {
        control: d.out.control.clone(),
        buf: buf as usize,
        generation: d.held[i].generation,
    });
    let buffer = match &d.held[i].memory {
        Memory::Shm(map) => FrameBuffer::Shm {
            data: Bytes::from_owner(ShmView {
                map: map.clone(),
                start,
                len,
                _lease: lease.clone(),
            }),
            fourcc: format.fourcc,
            stride,
        },
        Memory::Dmabuf {
            objects,
            plane_objects,
        } => FrameBuffer::Dmabuf {
            objects: objects.clone(),
            fourcc: format.fourcc,
            modifier: format.modifier.unwrap_or(DRM_FORMAT_MOD_LINEAR),
            // SAFETY: as above; each of the `n_datas` entries has a chunk.
            planes: unsafe { std::slice::from_raw_parts((*sb).datas, (*sb).n_datas as usize) }
                .iter()
                .zip(plane_objects)
                .map(|(data, &object)| {
                    // SAFETY: as above.
                    let chunk = unsafe { &*data.chunk };
                    Plane {
                        object,
                        offset: chunk.offset,
                        stride: chunk.stride.max(0) as u32,
                    }
                })
                .collect(),
        },
    };
    let frame = Frame {
        buffer,
        size: format.size,
        damage: frame_damage(damage_meta, format.size, missed),
        cursor: cursor_meta.and_then(read_cursor),
        _lease: Some(lease),
    };
    if d.out.frames.send(frame).is_err() {
        d.quit();
    }
    true
}

/// The index of `buf` in `d.held`, added on its first frame.
fn held_index(
    d: &mut Data,
    buf: *mut pw_sys::pw_buffer,
    sb: *const spa_sys::spa_buffer,
) -> Option<usize> {
    if let Some(i) = d.held.iter().position(|h| h.buf == buf as usize) {
        return Some(i);
    }
    // SAFETY: `sb` is the live spa_buffer of `buf`, with `n_datas` entries, at least one.
    let datas = unsafe { std::slice::from_raw_parts((*sb).datas, (*sb).n_datas as usize) };
    let data = &datas[0];
    if data.fd < 0 {
        return None;
    }
    // SAFETY: the stream owns the fds while the buffer exists; they are only borrowed to read
    // and duplicate.
    let fd = |data: &spa_sys::spa_data| unsafe { BorrowedFd::borrow_raw(data.fd as i32) };
    let memory = match data.type_ {
        spa_sys::SPA_DATA_MemFd => Memory::Shm(Arc::new(
            Mapping::new(fd(data), data.mapoffset, data.maxsize)
                .map_err(|e| log::warn!("screen stream: map: {e}"))
                .ok()?,
        )),
        spa_sys::SPA_DATA_DmaBuf => {
            if datas.iter().any(|data| data.fd < 0) {
                return None;
            }
            let ids: Vec<_> = datas.iter().map(|data| buffer_id(fd(data))).collect();
            let plane_objects = objects_of(&ids);
            let mut objects = Vec::new();
            for (data, &object) in datas.iter().zip(&plane_objects) {
                if object == objects.len() {
                    objects.push(Arc::new(fd(data).try_clone_to_owned().ok()?));
                }
            }
            Memory::Dmabuf {
                objects,
                plane_objects,
            }
        }
        other => {
            log::warn!("screen stream: unexpected buffer type {other}");
            return None;
        }
    };
    d.generations += 1;
    d.held.push(Held {
        buf: buf as usize,
        generation: d.generations,
        memory,
        frames: 0,
    });
    Some(d.held.len() - 1)
}

/// The buffer behind a dma-buf fd: fds of one buffer share an inode.
fn buffer_id(fd: BorrowedFd<'_>) -> Option<(u64, u64)> {
    let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat writes a whole stat on success.
    let st =
        unsafe { (libc::fstat(fd.as_raw_fd(), st.as_mut_ptr()) == 0).then(|| st.assume_init())? };
    Some((st.st_dev, st.st_ino))
}

/// Each plane's object index: planes on one buffer share an object, numbered in order of first
/// use. A plane whose buffer is unknown gets its own.
fn objects_of(ids: &[Option<(u64, u64)>]) -> Vec<usize> {
    let mut objects: Vec<Option<(u64, u64)>> = Vec::new();
    (ids.iter())
        .map(|&id| {
            let shared = id.and_then(|id| objects.iter().position(|&o| o == Some(id)));
            shared.unwrap_or_else(|| {
                objects.push(id);
                objects.len() - 1
            })
        })
        .collect()
}

/// The bytes of the first meta of `type_` in `sb` that has memory behind it.
///
/// # Safety
/// `sb` is a live spa_buffer whose metas stay valid for the returned lifetime.
unsafe fn find_meta<'a>(sb: *const spa_sys::spa_buffer, type_: u32) -> Option<&'a [u8]> {
    // SAFETY: per the caller; `metas` has `n_metas` entries, each with `size` bytes at `data`.
    unsafe {
        (0..(*sb).n_metas as usize)
            .map(|i| &*(*sb).metas.add(i))
            .find(|m| m.type_ == type_ && !m.data.is_null())
            .map(|m| std::slice::from_raw_parts(m.data as *const u8, m.size as usize))
    }
}

/// Whether frames went missing between header sequence numbers `last` and `now`.
fn seq_gap(last: Option<u64>, now: Option<u64>) -> bool {
    matches!((last, now), (Some(l), Some(n)) if n != l.wrapping_add(1))
}

/// A frame's damage from its `SPA_META_VideoDamage`: the whole picture when it has none or
/// frames were `missed`, else its regions up to the first empty one, clamped to `size`.
fn frame_damage(meta: Option<&[u8]>, size: (u32, u32), missed: bool) -> Vec<Rect> {
    let Some(meta) = meta.filter(|_| !missed) else {
        return vec![Rect::full(size)];
    };
    const REGION: usize = size_of::<spa_sys::spa_meta_region>();
    meta.as_chunks::<REGION>()
        .0
        .iter()
        // SAFETY: each chunk is exactly one plain-C spa_meta_region.
        .map(|c| unsafe { std::ptr::read_unaligned(c.as_ptr() as *const spa_sys::spa_meta_region) })
        .take_while(|r| r.region.size.width > 0 && r.region.size.height > 0)
        .filter_map(|r| {
            let (x, y) = (
                r.region.position.x.max(0) as u32,
                r.region.position.y.max(0) as u32,
            );
            let right = (i64::from(r.region.position.x) + i64::from(r.region.size.width))
                .clamp(0, i64::from(size.0)) as u32;
            let bottom = (i64::from(r.region.position.y) + i64::from(r.region.size.height))
                .clamp(0, i64::from(size.1)) as u32;
            (right > x && bottom > y).then(|| Rect {
                x,
                y,
                width: right - x,
                height: bottom - y,
            })
        })
        .collect()
}

/// The pointer in a `SPA_META_Cursor`; None for a meta without cursor data (id 0).
fn read_cursor(meta: &[u8]) -> Option<Cursor> {
    let cur_len = size_of::<spa_sys::spa_meta_cursor>();
    let bm_len = size_of::<spa_sys::spa_meta_bitmap>();
    if meta.len() < cur_len {
        return None;
    }
    // SAFETY: the meta starts with a plain-C spa_meta_cursor, length checked above.
    let cur = unsafe { std::ptr::read_unaligned(meta.as_ptr() as *const spa_sys::spa_meta_cursor) };
    if cur.id == 0 {
        return None;
    }
    let at = cur.bitmap_offset as usize;
    let image = (at >= cur_len && at + bm_len <= meta.len())
        .then(|| {
            // SAFETY: a plain-C spa_meta_bitmap lies at `at`, bounds checked above.
            let bm = unsafe {
                std::ptr::read_unaligned(meta[at..].as_ptr() as *const spa_sys::spa_meta_bitmap)
            };
            let (w, h) = (bm.size.width, bm.size.height);
            let swap = match bm.format {
                spa_sys::SPA_VIDEO_FORMAT_BGRA | spa_sys::SPA_VIDEO_FORMAT_BGRx => false,
                spa_sys::SPA_VIDEO_FORMAT_RGBA | spa_sys::SPA_VIDEO_FORMAT_RGBx => true,
                _ => return None,
            };
            let opaque = matches!(
                bm.format,
                spa_sys::SPA_VIDEO_FORMAT_BGRx | spa_sys::SPA_VIDEO_FORMAT_RGBx
            );
            let stride = bm.stride.max(0) as usize;
            let row = w as usize * 4;
            let start = at + bm.offset as usize;
            if w == 0
                || h == 0
                || stride < row
                || start + stride * (h as usize - 1) + row > meta.len()
            {
                return None;
            }
            let mut bgra = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                for px in meta[start + y * stride..][..row].as_chunks::<4>().0 {
                    let (b, r) = if swap { (px[2], px[0]) } else { (px[0], px[2]) };
                    bgra.extend_from_slice(&[b, px[1], r, if opaque { 255 } else { px[3] }]);
                }
            }
            Some(CursorImage {
                width: w,
                height: h,
                hotspot: (cur.hotspot.x, cur.hotspot.y),
                bgra: bgra.into(),
            })
        })
        .flatten();
    Some(Cursor {
        position: (cur.position.x, cur.position.y),
        image,
    })
}

/// A memfd mapped read-only for as long as frames point into it.
struct Mapping {
    /// `len` bytes from the buffer's offset, inside the page-aligned mapping `base`/`base_len`.
    ptr: *mut libc::c_void,
    len: usize,
    base: *mut libc::c_void,
    base_len: usize,
}

// SAFETY: the mapping is read-only memory owned by this value; any thread may read it.
unsafe impl Send for Mapping {}
// SAFETY: as for Send: shared reads only.
unsafe impl Sync for Mapping {}

impl Mapping {
    fn new(fd: BorrowedFd, offset: u32, len: u32) -> std::io::Result<Mapping> {
        // SAFETY: sysconf has no preconditions.
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u32;
        let delta = offset % page;
        let base_len = len as usize + delta as usize;
        // SAFETY: a fresh shared read-only mapping of `fd`; the result is checked below.
        let base = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                base_len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                libc::off_t::from(offset - delta),
            )
        };
        if base == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Mapping {
            // SAFETY: `delta` lies inside the `base_len` bytes just mapped.
            ptr: unsafe { base.add(delta as usize) },
            len: len as usize,
            base,
            base_len,
        })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: `base`/`base_len` are this value's own mapping, unmapped once.
        unsafe { libc::munmap(self.base, self.base_len) };
    }
}

/// A frame's rows in a [`Mapping`], keeping the frame's buffer out of the stream.
struct ShmView {
    map: Arc<Mapping>,
    start: usize,
    len: usize,
    _lease: Arc<Lease>,
}

impl AsRef<[u8]> for ShmView {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: `start + len` was checked against the mapping's length, which lives as long
        // as `map`.
        unsafe { std::slice::from_raw_parts((self.map.ptr as *const u8).add(self.start), self.len) }
    }
}

/// A serialized pod.
struct OwnedPod(Vec<u8>);

impl OwnedPod {
    fn serialize(value: &pod::Value) -> OwnedPod {
        let mut bytes = Vec::new();
        pod::serialize::PodSerializer::serialize(IoCursor::new(&mut bytes), value).expect("pod");
        OwnedPod(bytes)
    }

    fn pod(&self) -> &Pod {
        Pod::from_bytes(&self.0).expect("valid pod")
    }
}

fn prop(key: u32, value: pod::Value) -> pod::Property {
    pod::Property {
        key,
        flags: pod::PropertyFlags::empty(),
        value,
    }
}

/// The `EnumFormat` pods: each format as a dma-buf with `modifiers` (left to the producer to
/// fixate), then each as shared memory.
fn format_params(modifiers: &[u64]) -> Vec<OwnedPod> {
    let dmabuf = modifiers.split_first().map(|(first, _)| pod::Property {
        key: spa_sys::SPA_FORMAT_VIDEO_modifier,
        flags: pod::PropertyFlags::MANDATORY | pod::PropertyFlags::DONT_FIXATE,
        value: pod::Value::Choice(pod::ChoiceValue::Long(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Enum {
                default: *first as i64,
                alternatives: modifiers.iter().map(|m| *m as i64).collect(),
            },
        ))),
    });
    let mut pods = Vec::new();
    for modifier in dmabuf.into_iter().map(Some).chain([None]) {
        for (format, _) in FORMATS {
            let mut properties = vec![
                prop(
                    spa_sys::SPA_FORMAT_mediaType,
                    pod::Value::Id(Id(spa_sys::SPA_MEDIA_TYPE_video)),
                ),
                prop(
                    spa_sys::SPA_FORMAT_mediaSubtype,
                    pod::Value::Id(Id(spa_sys::SPA_MEDIA_SUBTYPE_raw)),
                ),
                prop(spa_sys::SPA_FORMAT_VIDEO_format, pod::Value::Id(Id(format))),
            ];
            properties.extend(modifier.clone());
            properties.extend([
                prop(
                    spa_sys::SPA_FORMAT_VIDEO_size,
                    pod::Value::Choice(pod::ChoiceValue::Rectangle(Choice(
                        ChoiceFlags::empty(),
                        ChoiceEnum::Range {
                            default: Rectangle {
                                width: 1920,
                                height: 1080,
                            },
                            min: Rectangle {
                                width: 1,
                                height: 1,
                            },
                            max: Rectangle {
                                width: 16384,
                                height: 16384,
                            },
                        },
                    ))),
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
                            max: Fraction { num: 360, denom: 1 },
                        },
                    ))),
                ),
            ]);
            pods.push(OwnedPod::serialize(&pod::Value::Object(pod::Object {
                type_: spa_sys::SPA_TYPE_OBJECT_Format,
                id: spa_sys::SPA_PARAM_EnumFormat,
                properties,
            })));
        }
    }
    pods
}

/// `Buffers` of the negotiated kind, at least three so a held frame never starves the
/// producer, and the header, damage and cursor metas.
fn buffer_params(dmabuf: bool) -> Vec<OwnedPod> {
    let data_type = 1
        << if dmabuf {
            spa_sys::SPA_DATA_DmaBuf
        } else {
            spa_sys::SPA_DATA_MemFd
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
                        min: 3,
                        max: 8,
                    },
                ))),
            ),
            prop(
                spa_sys::SPA_PARAM_BUFFERS_dataType,
                pod::Value::Choice(pod::ChoiceValue::Int(Choice(
                    ChoiceFlags::empty(),
                    ChoiceEnum::Flags {
                        default: data_type,
                        flags: vec![data_type],
                    },
                ))),
            ),
        ],
    }));
    let meta = |type_: u32, size: pod::Value| {
        OwnedPod::serialize(&pod::Value::Object(pod::Object {
            type_: spa_sys::SPA_TYPE_OBJECT_ParamMeta,
            id: spa_sys::SPA_PARAM_Meta,
            properties: vec![
                prop(spa_sys::SPA_PARAM_META_type, pod::Value::Id(Id(type_))),
                prop(spa_sys::SPA_PARAM_META_size, size),
            ],
        }))
    };
    let range = |min: i32, max: i32| {
        pod::Value::Choice(pod::ChoiceValue::Int(Choice(
            ChoiceFlags::empty(),
            ChoiceEnum::Range {
                default: max,
                min,
                max,
            },
        )))
    };
    let region = size_of::<spa_sys::spa_meta_region>() as i32;
    let cursor =
        (size_of::<spa_sys::spa_meta_cursor>() + size_of::<spa_sys::spa_meta_bitmap>()) as i32;
    let pixels = (MAX_CURSOR_PX * MAX_CURSOR_PX * 4) as i32;
    vec![
        buffers,
        meta(
            spa_sys::SPA_META_Header,
            pod::Value::Int(size_of::<spa_sys::spa_meta_header>() as i32),
        ),
        meta(
            spa_sys::SPA_META_VideoDamage,
            range(region, region * MAX_DAMAGE_RECTS),
        ),
        meta(spa_sys::SPA_META_Cursor, range(cursor, cursor + pixels)),
    ]
}

#[cfg(test)]
impl Offer {
    /// An offer no stream hears.
    pub fn detached() -> Offer {
        Offer(pipewire::channel::channel().0)
    }
}

#[cfg(test)]
impl Frame {
    /// A frame of `data` with no stream behind it.
    pub fn detached(data: Vec<u8>, size: (u32, u32), damage: Vec<Rect>) -> Frame {
        let buffer = FrameBuffer::Shm {
            data: data.into(),
            fourcc: XRGB8888,
            stride: size.0 * 4,
        };
        Frame {
            damage,
            ..Frame::unleased(buffer, size)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_at_an_offset_inside_a_page_maps_from_that_offset() {
        use std::io::Write;
        use std::os::fd::{AsFd, FromRawFd};
        // SAFETY: a fresh memfd, owned from here on.
        let mut file =
            unsafe { std::fs::File::from_raw_fd(libc::memfd_create(c"frame".as_ptr(), 0)) };
        let data: Vec<u8> = (0..3 * 4096u32).map(|i| (i % 251) as u8).collect();
        file.write_all(&data).unwrap();
        let map = Mapping::new(file.as_fd(), 4096 + 100, 300).unwrap();
        // SAFETY: `ptr` maps `len` readable bytes while `map` lives.
        let got = unsafe { std::slice::from_raw_parts(map.ptr as *const u8, map.len) };
        assert_eq!(got, &data[4196..4496]);
    }

    fn held(buf: usize, generation: u64) -> Held {
        Held {
            buf,
            generation,
            memory: Memory::Dmabuf {
                objects: Vec::new(),
                plane_objects: Vec::new(),
            },
            frames: 0,
        }
    }

    #[test]
    fn a_buffer_given_twice_goes_back_once_after_both_frames_drop() {
        let mut bufs = vec![held(0x10, 1)];
        bufs[0].frames = 2;
        assert!(
            !release(&mut bufs, 0x10, 1),
            "the second frame still reads the buffer"
        );
        assert!(out(&bufs, 0x10));
        assert!(release(&mut bufs, 0x10, 1), "the last frame gives it back");
        assert!(!out(&bufs, 0x10));
    }

    #[test]
    fn a_frame_of_a_removed_buffer_gives_nothing_back() {
        let mut bufs = vec![held(0x10, 2)];
        bufs[0].frames = 1;
        assert!(!release(&mut bufs, 0x10, 1), "generation 1 was removed");
        assert_eq!(bufs[0].frames, 1);
    }

    #[test]
    fn planes_on_one_buffer_share_an_object() {
        let (a, b) = (Some((1, 10)), Some((1, 11)));
        assert_eq!(objects_of(&[a, a]), vec![0, 0], "both planes in one buffer");
        assert_eq!(objects_of(&[a, b, a]), vec![0, 1, 0]);
        assert_eq!(
            objects_of(&[None, a, None, a]),
            vec![0, 1, 2, 1],
            "an unknown buffer is its own"
        );
    }

    fn region(x: i32, y: i32, width: u32, height: u32) -> spa_sys::spa_meta_region {
        spa_sys::spa_meta_region {
            region: spa_sys::spa_region {
                position: spa_sys::spa_point { x, y },
                size: spa_sys::spa_rectangle { width, height },
            },
        }
    }

    fn bytes<T>(values: &[T]) -> Vec<u8> {
        // SAFETY: plain-C values viewed as their bytes.
        unsafe {
            std::slice::from_raw_parts(values.as_ptr() as *const u8, std::mem::size_of_val(values))
        }
        .to_vec()
    }

    const SIZE: (u32, u32) = (1920, 1080);

    #[test]
    fn damage_meta_is_read_into_rects() {
        let meta = bytes(&[
            region(10, 20, 30, 40),
            region(1900, 1070, 100, 100),
            region(0, 0, 0, 0),
            region(5, 5, 5, 5),
        ]);
        assert_eq!(
            frame_damage(Some(&meta), SIZE, false),
            vec![
                Rect {
                    x: 10,
                    y: 20,
                    width: 30,
                    height: 40
                },
                Rect {
                    x: 1900,
                    y: 1070,
                    width: 20,
                    height: 10
                },
            ],
            "the list ends at the empty region; rects clamp to the frame"
        );
    }

    #[test]
    fn an_empty_damage_list_is_a_cursor_only_frame() {
        let meta = bytes(&[region(0, 0, 0, 0); 16]);
        assert_eq!(frame_damage(Some(&meta), SIZE, false), vec![]);
    }

    #[test]
    fn no_damage_meta_means_full_frame() {
        assert_eq!(frame_damage(None, SIZE, false), vec![Rect::full(SIZE)]);
    }

    #[test]
    fn a_missed_frame_means_full_frame() {
        let meta = bytes(&[region(10, 20, 30, 40)]);
        assert_eq!(
            frame_damage(Some(&meta), SIZE, true),
            vec![Rect::full(SIZE)]
        );
        assert!(seq_gap(Some(7), Some(9)), "7 then 9 skipped 8");
        assert!(!seq_gap(Some(7), Some(8)));
        assert!(!seq_gap(Some(u64::MAX), Some(0)), "the counter wraps");
        assert!(!seq_gap(None, Some(3)));
    }

    #[test]
    fn the_cursor_meta_gives_position_hotspot_and_bgra() {
        let cur_len = size_of::<spa_sys::spa_meta_cursor>();
        let mut meta = bytes(&[spa_sys::spa_meta_cursor {
            id: 3,
            flags: 0,
            position: spa_sys::spa_point { x: 400, y: 300 },
            hotspot: spa_sys::spa_point { x: 1, y: 0 },
            bitmap_offset: cur_len as u32,
        }]);
        meta.extend(bytes(&[spa_sys::spa_meta_bitmap {
            format: spa_sys::SPA_VIDEO_FORMAT_RGBA,
            size: spa_sys::spa_rectangle {
                width: 2,
                height: 1,
            },
            stride: 12,
            offset: size_of::<spa_sys::spa_meta_bitmap>() as u32,
        }]));
        meta.extend([1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0]);
        assert_eq!(
            read_cursor(&meta),
            Some(Cursor {
                position: (400, 300),
                image: Some(CursorImage {
                    width: 2,
                    height: 1,
                    hotspot: (1, 0),
                    bgra: vec![3, 2, 1, 4, 7, 6, 5, 8].into(),
                }),
            })
        );
        meta[cur_len - 4..cur_len].fill(0);
        assert_eq!(
            read_cursor(&meta),
            Some(Cursor {
                position: (400, 300),
                image: None
            }),
            "bitmap offset 0: no image, the pointer is off the output"
        );
        meta[..4].fill(0);
        assert_eq!(read_cursor(&meta), None, "id 0: no cursor data");
    }
}
