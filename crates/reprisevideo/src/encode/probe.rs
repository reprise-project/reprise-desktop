//! What encodes H.264, found once at start: the FFmpeg backends that open and code a test frame
//! on the session's render node, and the stream modifiers each one's GPU maps correctly.

use super::colour;
use super::ffmpeg::Ffmpeg;
use super::gl::{self, Gl};
use super::hwmap::{self, Converter, Devices, Gpu};
use super::{Encoded, Encoder};
use crate::stream::{
    ABGR8888, ARGB8888, DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Plane, XBGR8888, XRGB8888,
};
use drm::control::Device as _;
use drm::node::{DrmNode, NodeType};
use glow::HasContext;
use repriseconf::remote::{EncoderChoice, Quality};
use std::fs::File;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wayland_client as wl;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_feedback_v1::{
    self, ZwpLinuxDmabufFeedbackV1,
};
use wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::{
    self, ZwpLinuxDmabufV1,
};

/// The render node used when nothing names the stream's device.
pub const RENDER_NODE: &str = "/dev/dri/renderD128";

/// The formats the stream offers, each of which a modifier must map in.
const FORMATS: [u32; 4] = [XRGB8888, ARGB8888, XBGR8888, ABGR8888];

/// The side of the frames a backend and a modifier are tried with, above the smallest picture
/// hardware encoders take (128 on AMD).
pub const TEST_SIDE: u32 = 256;
const TEST_RATE: u32 = 60;

/// An FFmpeg H.264 encoder and how frames reach it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// Low power is Intel's fixed-function encoder, which leaves the shader cores to the
    /// compositor; other drivers offer only the full one.
    Vaapi {
        low_power: bool,
    },
    Nvenc,
    Vulkan,
    X264,
}

impl Backend {
    /// Every backend, in the order they are tried.
    pub const ALL: [Backend; 5] = [
        Backend::Vaapi { low_power: true },
        Backend::Vaapi { low_power: false },
        Backend::Nvenc,
        Backend::Vulkan,
        Backend::X264,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Backend::Vaapi { .. } => "VA-API H.264",
            Backend::Nvenc => "NVENC H.264",
            Backend::Vulkan => "Vulkan H.264",
            Backend::X264 => "Software H.264",
        }
    }

    pub fn codec(self) -> &'static str {
        match self {
            Backend::Vaapi { .. } => "h264_vaapi",
            Backend::Nvenc => "h264_nvenc",
            Backend::Vulkan => "h264_vulkan",
            Backend::X264 => "libx264",
        }
    }

    /// The GPU API dma-bufs reach it through. None for the CPU.
    pub fn gpu(self) -> Option<Gpu> {
        match self {
            Backend::Vaapi { .. } => Some(Gpu::Vaapi),
            Backend::Nvenc => Some(Gpu::Cuda),
            Backend::Vulkan => Some(Gpu::Vulkan),
            Backend::X264 => None,
        }
    }

    /// Whether it codes AVC444: the views are drawn for VA-API, CUDA and on the CPU.
    pub fn codes_avc444(self) -> bool {
        matches!(self, Backend::Vaapi { .. } | Backend::Nvenc | Backend::X264)
    }

    /// The `[remote_desktop] encoder` value that names it.
    pub fn choice(self) -> EncoderChoice {
        match self {
            Backend::Vaapi { .. } => EncoderChoice::Vaapi,
            Backend::Nvenc => EncoderChoice::Nvenc,
            Backend::Vulkan => EncoderChoice::Vulkan,
            Backend::X264 => EncoderChoice::Software,
        }
    }
}

/// How long a backend may take to code its test frame and check its modifiers before it is
/// left out.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// The working backends in [`Backend::ALL`]'s order with what `works` found, each choice tried on
/// its own thread and left out without an answer in `timeout`; low-power VA-API skips full.
pub fn working<T: Send + 'static>(
    works: impl Fn(Backend) -> Option<T> + Send + Sync + 'static,
    timeout: Duration,
) -> Vec<(Backend, T)> {
    let works = Arc::new(works);
    let mut groups: Vec<Vec<Backend>> = Vec::new();
    for backend in Backend::ALL {
        match groups.last_mut() {
            Some(group) if group[0].choice() == backend.choice() => group.push(backend),
            _ => groups.push(vec![backend]),
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, group) in groups.iter().cloned().enumerate() {
        let (tx, works) = (tx.clone(), works.clone());
        let spawned = std::thread::Builder::new()
            .name("reprise-video-probe".into())
            .spawn(move || {
                let found = (group.into_iter()).find_map(|b| works(b).map(|t| (b, t)));
                let _ = tx.send((index, found));
            });
        if let Err(e) = spawned {
            log::warn!("{}: no probe thread: {e}", groups[index][0].label());
        }
    }
    drop(tx);
    let deadline = Instant::now() + timeout;
    let mut answers: Vec<Option<Option<(Backend, T)>>> = groups.iter().map(|_| None).collect();
    while answers.iter().any(Option::is_none) {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((index, found)) => answers[index] = Some(found),
            Err(_) => break,
        }
    }
    for (group, answer) in groups.iter().zip(&answers) {
        if answer.is_none() {
            log::warn!(
                "{} did not finish its test in {} s and is left out",
                group[0].label(),
                timeout.as_secs_f32()
            );
        }
    }
    answers.into_iter().flatten().flatten().collect()
}

/// The render node the screen's buffers come from, as the compositor on `conn` names it, and the
/// backends that code on it with the modifiers each maps.
pub fn probe(conn: Option<&wl::Connection>) -> (PathBuf, Vec<(Backend, Vec<u64>)>) {
    let device = render_node(conn.and_then(session_device), |dev| {
        DrmNode::from_dev_id(dev)
            .ok()?
            .dev_path_with_type(NodeType::Render)
    });
    let on = device.clone();
    let found = quietly(|| {
        working(
            move |backend| match test_frame(backend, &on) {
                Ok(()) => Some(modifiers(backend, &on)),
                Err(e) => {
                    log::debug!("{}: {e}", backend.label());
                    None
                }
            },
            PROBE_TIMEOUT,
        )
    });
    (device, found)
}

/// `f` with FFmpeg's log silenced, as backends that are not there report errors. Calls may
/// overlap across threads, a probe thread outliving the call that started it.
fn quietly<T>(f: impl FnOnce() -> T) -> T {
    use ffmpeg::util::log;
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            let mut hush = HUSH.lock().unwrap();
            if let Some(level) = hush.leave() {
                log::set_level(level);
            }
        }
    }
    let mut hush = HUSH.lock().unwrap();
    hush.enter(log::get_level().unwrap_or(log::Level::Error));
    log::set_level(log::Level::Quiet);
    drop(hush);
    let _leave = Leave;
    f()
}

static HUSH: Mutex<Hush> = Mutex::new(Hush {
    callers: 0,
    level: None,
});

/// The callers in [`quietly`], and the log level the first found, which the last gives back.
struct Hush {
    callers: usize,
    level: Option<ffmpeg::util::log::Level>,
}

impl Hush {
    fn enter(&mut self, level: ffmpeg::util::log::Level) {
        if self.callers == 0 {
            self.level = Some(level);
        }
        self.callers += 1;
    }

    /// The level to give back once the last caller is done.
    fn leave(&mut self) -> Option<ffmpeg::util::log::Level> {
        self.callers -= 1;
        match self.callers {
            0 => self.level.take(),
            _ => None,
        }
    }
}

/// Open `backend` on `device` and code one keyframe.
fn test_frame(backend: Backend, device: &Path) -> Result<(), super::EncodeError> {
    let mut encoder = Ffmpeg::new(backend, device, TEST_RATE, Quality::Balanced)?;
    let frame = match backend.gpu() {
        Some(_) => {
            let gbm = gbm_device(device)?;
            let mut painter = Painter::new(device);
            // NVIDIA renders into no LINEAR buffer.
            test_buffer(&gbm, &mut painter, XRGB8888, Some(DRM_FORMAT_MOD_LINEAR))
                .or_else(|_| test_buffer(&gbm, &mut painter, XRGB8888, None))?
        }
        None => Frame::unleased(
            FrameBuffer::Shm {
                data: pattern(XRGB8888, TEST_SIDE * 4).into(),
                fourcc: XRGB8888,
                stride: TEST_SIDE * 4,
            },
            (TEST_SIDE, TEST_SIDE),
        ),
    };
    match encoder.encode(&frame, true)? {
        Encoded::Avc420 { data, .. } if !data.is_empty() => Ok(()),
        _ => Err(super::EncodeError("the test frame coded to nothing".into())),
    }
}

/// The render node of the session's dma-buf device `dev`, else [`RENDER_NODE`].
fn render_node(
    dev: Option<libc::dev_t>,
    node_of: impl FnOnce(libc::dev_t) -> Option<PathBuf>,
) -> PathBuf {
    dev.and_then(node_of)
        .filter(|node| node.exists())
        .unwrap_or_else(|| PathBuf::from(RENDER_NODE))
}

/// The compositor's main dma-buf device (linux-dmabuf default feedback), which the portal
/// allocates the screen's buffers on.
fn session_device(conn: &wl::Connection) -> Option<libc::dev_t> {
    let (globals, mut queue) = registry_queue_init::<Feedback>(conn).ok()?;
    let qh = queue.handle();
    let dmabuf: ZwpLinuxDmabufV1 = globals.bind(&qh, 4..=5, ()).ok()?;
    let feedback = dmabuf.get_default_feedback(&qh, ());
    let mut state = Feedback::default();
    while !state.done {
        queue.blocking_dispatch(&mut state).ok()?;
    }
    feedback.destroy();
    dmabuf.destroy();
    state.device
}

#[derive(Default)]
struct Feedback {
    device: Option<libc::dev_t>,
    done: bool,
}

/// A `dev_t` as the protocol sends it, in native byte order.
fn dev_from_bytes(bytes: &[u8]) -> Option<libc::dev_t> {
    Some(libc::dev_t::from_ne_bytes(bytes.try_into().ok()?))
}

impl wl::Dispatch<wl_registry::WlRegistry, GlobalListContents> for Feedback {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &wl::Connection,
        _: &wl::QueueHandle<Self>,
    ) {
    }
}

impl wl::Dispatch<ZwpLinuxDmabufV1, ()> for Feedback {
    fn event(
        _: &mut Self,
        _: &ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &wl::Connection,
        _: &wl::QueueHandle<Self>,
    ) {
    }
}

impl wl::Dispatch<ZwpLinuxDmabufFeedbackV1, ()> for Feedback {
    fn event(
        state: &mut Self,
        _: &ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &wl::Connection,
        _: &wl::QueueHandle<Self>,
    ) {
        match event {
            zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } => {
                state.device = dev_from_bytes(&device);
            }
            zwp_linux_dmabuf_feedback_v1::Event::Done => state.done = true,
            _ => {}
        }
    }
}

/// LINEAR, then each modifier the device's planes or GBM name that every stream format maps
/// into `backend`'s GPU in and converts correctly from. The CPU reads LINEAR only.
pub fn modifiers(backend: Backend, device: &Path) -> Vec<u64> {
    let linear = vec![DRM_FORMAT_MOD_LINEAR];
    let Some(gpu) = backend.gpu() else {
        return linear;
    };
    let (Ok(devices), Ok(gbm)) = (Devices::open(gpu, device), gbm_device(device)) else {
        return linear;
    };
    let mut candidates = plane_modifiers(device);
    for format in FORMATS {
        let implicit = gbm::Format::try_from(format).ok().and_then(|f| {
            gbm.create_buffer_object::<()>(64, 64, f, gbm::BufferObjectFlags::RENDERING)
                .ok()
        });
        if let Some(bo) = implicit {
            candidates.push(bo.modifier().into());
        }
    }
    let mut painter = Painter::new(device);
    quietly(|| {
        mappable(&candidates, |format, modifier| {
            converts(&devices, &gbm, &mut painter, format, modifier).is_ok()
        })
    })
}

/// LINEAR first, then each of `candidates` (once, not LINEAR, not INVALID) that `maps` takes in
/// every stream format.
fn mappable(candidates: &[u64], mut maps: impl FnMut(u32, u64) -> bool) -> Vec<u64> {
    let invalid: u64 = gbm::Modifier::Invalid.into();
    let mut candidates = candidates.to_vec();
    candidates.sort_unstable();
    candidates.dedup();
    let mut found = vec![DRM_FORMAT_MOD_LINEAR];
    for m in candidates {
        if m != DRM_FORMAT_MOD_LINEAR && m != invalid && FORMATS.iter().all(|&f| maps(f, m)) {
            found.push(m);
        }
    }
    found
}

fn gbm_device(device: &Path) -> Result<gbm::Device<File>, super::EncodeError> {
    let err = |e: std::io::Error| super::EncodeError(format!("{}: {e}", device.display()));
    let file = File::options()
        .read(true)
        .write(true)
        .open(device)
        .map_err(err)?;
    gbm::Device::new(file).map_err(err)
}

/// A dma-buf of `format` and `modifier` (else the driver's choice) holding [`pattern`], as a
/// stream frame.
fn test_buffer(
    gbm: &gbm::Device<File>,
    painter: &mut Painter,
    format: u32,
    modifier: Option<u64>,
) -> Result<Frame, super::EncodeError> {
    let err = |e: &dyn std::fmt::Display| super::EncodeError(e.to_string());
    let gbm_format = gbm::Format::try_from(format).map_err(|e| err(&e))?;
    let flags = gbm::BufferObjectFlags::RENDERING;
    let mut bo = match modifier {
        Some(m) => gbm.create_buffer_object_with_modifiers2::<()>(
            TEST_SIDE,
            TEST_SIDE,
            gbm_format,
            [gbm::Modifier::from(m)].into_iter(),
            flags,
        ),
        None => gbm.create_buffer_object::<()>(TEST_SIDE, TEST_SIDE, gbm_format, flags),
    }
    .map_err(|e| err(&e))?;
    let mapped = bo.map_mut(0, 0, TEST_SIDE, TEST_SIDE, |map| {
        let stride = map.stride();
        map.buffer_mut()[..(stride * TEST_SIDE) as usize].copy_from_slice(&pattern(format, stride));
    });
    let planes = (0..bo.plane_count() as i32)
        .map(|p| Plane {
            object: 0,
            offset: bo.offset(p),
            stride: bo.stride_for_plane(p),
        })
        .collect();
    let fd = Arc::new(bo.fd().map_err(|e| err(&e))?);
    let frame = Frame::unleased(
        FrameBuffer::Dmabuf {
            objects: vec![fd],
            fourcc: format,
            modifier: bo.modifier().into(),
            planes,
        },
        (TEST_SIDE, TEST_SIDE),
    );
    if mapped.is_err() {
        painter.paint(&frame)?;
    }
    Ok(frame)
}

/// Draws [`pattern`] into test buffers the CPU cannot map, as NVIDIA's tiled ones are; the GL
/// context is made for the first.
struct Painter<'a> {
    node: &'a Path,
    gl: Option<(Gl, glow::Program)>,
}

/// [`pattern`] by position: red is x, green y.
const PAINT: &str = "#version 300 es
precision highp float;
out vec4 colour;
void main() {
    colour = vec4(floor(gl_FragCoord.xy) / 255.0, 128.0 / 255.0, 1.0);
}
";

impl<'a> Painter<'a> {
    fn new(node: &'a Path) -> Painter<'a> {
        Painter { node, gl: None }
    }

    fn paint(&mut self, frame: &Frame) -> Result<(), super::EncodeError> {
        let FrameBuffer::Dmabuf {
            objects,
            fourcc,
            modifier,
            planes,
        } = &frame.buffer
        else {
            return Err(super::EncodeError("only a dma-buf is painted".into()));
        };
        let (gl, program) = match &mut self.gl {
            Some(made) => made,
            none => {
                let gl = Gl::new(self.node)?;
                // SAFETY: the new context is current.
                let program = unsafe { gl::program(&gl.gl, PAINT)? };
                none.insert((gl, program))
            }
        };
        gl.current()?;
        let image = gl.import(objects, *fourcc, *modifier, planes, frame.size)?;
        let (w, h) = frame.size;
        let g = &gl.gl;
        // SAFETY: GL calls on the current context; what is made here is deleted here.
        let painted = unsafe {
            let texture = g.create_texture().map_err(gl::gl_err("texture"))?;
            gl.bind_image(texture, image);
            let fb = g.create_framebuffer().map_err(gl::gl_err("framebuffer"));
            let painted = fb.and_then(|fb| {
                g.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
                g.framebuffer_texture_2d(
                    glow::FRAMEBUFFER,
                    glow::COLOR_ATTACHMENT0,
                    glow::TEXTURE_2D,
                    Some(texture),
                    0,
                );
                let status = g.check_framebuffer_status(glow::FRAMEBUFFER);
                if status == glow::FRAMEBUFFER_COMPLETE {
                    g.viewport(0, 0, w as i32, h as i32);
                    g.use_program(Some(*program));
                    g.draw_arrays(glow::TRIANGLES, 0, 3);
                    g.finish();
                }
                g.bind_framebuffer(glow::FRAMEBUFFER, None);
                g.delete_framebuffer(fb);
                match (status, g.get_error()) {
                    (glow::FRAMEBUFFER_COMPLETE, glow::NO_ERROR) => Ok(()),
                    (glow::FRAMEBUFFER_COMPLETE, e) => {
                        Err(super::EncodeError(format!("painting: GL error {e:#x}")))
                    }
                    (s, _) => Err(super::EncodeError(format!("painting: framebuffer {s:#x}"))),
                }
            });
            g.delete_texture(texture);
            painted
        };
        gl.destroy_image(image);
        painted
    }
}

/// The test picture in `format`, rows of `stride` bytes: red rises to the right, green
/// downwards, blue stays at 128.
fn pattern(format: u32, stride: u32) -> Vec<u8> {
    let mut out = vec![0u8; (stride * TEST_SIDE) as usize];
    for y in 0..TEST_SIDE {
        for x in 0..TEST_SIDE {
            let (r, g, b) = (x as u8, y as u8, 128);
            let px = match format {
                XBGR8888 | ABGR8888 => [r, g, b, 255],
                _ => [b, g, r, 255],
            };
            let at = (y * stride + x * 4) as usize;
            out[at..at + 4].copy_from_slice(&px);
        }
    }
    out
}

/// The luma of [`pattern`] at (`x`, `y`).
fn pattern_luma(x: u32, y: u32) -> f64 {
    colour::luma(f64::from(x), f64::from(y), 128.0)
}

/// The luma at (`x`, `y`) of a converted picture: NV12, or RGB for an encoder that converts
/// itself.
fn luma_at(picture: &ffmpeg::frame::Video, x: u32, y: u32) -> Result<f64, super::EncodeError> {
    use ffmpeg::format::Pixel;
    let (plane, stride) = (picture.data(0), picture.stride(0));
    let at = y as usize * stride;
    let rgb = |r: usize, g: usize, b: usize| {
        let px = &plane[at + x as usize * 4..];
        colour::luma(f64::from(px[r]), f64::from(px[g]), f64::from(px[b]))
    };
    match picture.format() {
        Pixel::NV12 | Pixel::YUV420P => Ok(f64::from(plane[at + x as usize])),
        Pixel::BGRZ | Pixel::BGRA => Ok(rgb(2, 1, 0)),
        Pixel::RGBZ | Pixel::RGBA => Ok(rgb(0, 1, 2)),
        other => Err(super::EncodeError(format!(
            "{other:?} pictures are not checked"
        ))),
    }
}

/// Map a [`pattern`] dma-buf of `format` and `modifier`, convert it on the GPU and check its
/// luma: an import that ignores the modifier reads the pixels in the wrong places.
fn converts(
    devices: &Devices,
    gbm: &gbm::Device<File>,
    painter: &mut Painter,
    format: u32,
    modifier: u64,
) -> Result<(), super::EncodeError> {
    let frame = test_buffer(gbm, painter, format, Some(modifier))?;
    let side = (TEST_SIDE, TEST_SIDE);
    let mut converter = Converter::new(Some(devices), format, side, side, TEST_RATE)?;
    let converted = hwmap::download(&converter.convert(&frame)?)?;
    for y in (8..TEST_SIDE).step_by(16) {
        for x in (8..TEST_SIDE).step_by(16) {
            let got = luma_at(&converted, x, y)?;
            if (got - pattern_luma(x, y)).abs() > 4.0 {
                return Err(super::EncodeError(format!(
                    "modifier {modifier:#x}: luma {got} at ({x}, {y})"
                )));
            }
        }
    }
    Ok(())
}

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl drm::Device for Card {}
impl drm::control::Device for Card {}

/// The modifiers the planes of `render_node`'s card list for the stream's formats.
fn plane_modifiers(render_node: &Path) -> Vec<u64> {
    let card = DrmNode::from_path(render_node)
        .ok()
        .and_then(|n| n.dev_path_with_type(NodeType::Primary))
        .and_then(|path| File::open(path).ok())
        .map(Card);
    let Some(card) = card else {
        return Vec::new();
    };
    let _ = drm::Device::set_client_capability(&card, drm::ClientCapability::UniversalPlanes, true);
    let mut found = Vec::new();
    for plane in card.plane_handles().unwrap_or_default() {
        let Ok(props) = card.get_properties(plane) else {
            continue;
        };
        let (handles, values) = props.as_props_and_values();
        for (&handle, &value) in handles.iter().zip(values) {
            let is_in_formats = card
                .get_property(handle)
                .is_ok_and(|info| info.name().to_bytes() == b"IN_FORMATS");
            if is_in_formats && let Ok(blob) = card.get_property_blob(value) {
                found.extend(in_formats(&blob, &FORMATS));
            }
        }
    }
    found
}

/// The modifiers an `IN_FORMATS` blob (`struct drm_format_modifier_blob`) lists for any of
/// `formats`.
fn in_formats(blob: &[u8], formats: &[u32]) -> Vec<u64> {
    let u32_at = |at: usize| {
        blob.get(at..at + 4)
            .map(|b| u32::from_ne_bytes(b.try_into().unwrap()) as usize)
    };
    let u64_at = |at: usize| {
        blob.get(at..at + 8)
            .map(|b| u64::from_ne_bytes(b.try_into().unwrap()))
    };
    let (Some(count_formats), Some(formats_at), Some(count_mods), Some(mods_at)) =
        (u32_at(8), u32_at(12), u32_at(16), u32_at(20))
    else {
        return Vec::new();
    };
    let listed: Vec<Option<usize>> = (0..count_formats)
        .map(|i| u32_at(formats_at + i * 4))
        .collect();
    let wanted: Vec<usize> = (listed.iter().enumerate())
        .filter(|(_, f)| f.is_some_and(|f| formats.contains(&(f as u32))))
        .map(|(i, _)| i)
        .collect();
    let mut found = Vec::new();
    for i in 0..count_mods {
        let at = mods_at + i * 24;
        let (Some(mask), Some(offset), Some(modifier)) =
            (u64_at(at), u32_at(at + 8), u64_at(at + 16))
        else {
            break;
        };
        let covers = |index: usize| {
            (offset..offset + 64).contains(&index) && mask & (1 << (index - offset)) != 0
        };
        if wanted.iter().any(|&index| covers(index)) {
            found.push(modifier);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const Y_TILED: u64 = 0x0100_0000_0000_0002;
    const CCS: u64 = 0x0100_0000_0000_0006;
    const CCS_CC: u64 = 0x0100_0000_0000_0008;

    /// The backends `works` takes, probed with a timeout no test reaches.
    fn found(works: impl Fn(Backend) -> bool + Send + Sync + 'static) -> Vec<Backend> {
        let found = working(move |b| works(b).then_some(()), Duration::from_secs(60));
        found.into_iter().map(|(b, ())| b).collect()
    }

    #[test]
    fn the_probe_keeps_vaapi_then_nvenc_then_vulkan_then_x264() {
        let tried = Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = tried.clone();
        let none = found(move |b| {
            log.lock().unwrap().push(b);
            false
        });
        assert_eq!(none, []);
        let mut tried = tried.lock().unwrap().clone();
        tried.sort_by_key(|b| Backend::ALL.iter().position(|a| a == b));
        assert_eq!(tried, Backend::ALL, "every backend is tried");
        let lp = Backend::Vaapi { low_power: true };
        let full = Backend::Vaapi { low_power: false };
        assert_eq!(
            found(move |b| {
                assert_ne!(
                    b, full,
                    "the full entrypoint is left untried once low power works"
                );
                true
            }),
            [lp, Backend::Nvenc, Backend::Vulkan, Backend::X264],
        );
        assert_eq!(
            found(move |b| b != lp),
            [full, Backend::Nvenc, Backend::Vulkan, Backend::X264],
            "the full entrypoint where there is no low-power one"
        );
        assert_eq!(
            found(|b| matches!(b, Backend::Vulkan | Backend::X264)),
            [Backend::Vulkan, Backend::X264]
        );
    }

    #[test]
    fn a_backend_that_hangs_is_left_out_after_the_timeout() {
        let (_open, gate) = std::sync::mpsc::channel::<()>();
        let gate = std::sync::Mutex::new(gate);
        let started = Instant::now();
        let found = working(
            move |b| {
                if b == Backend::Nvenc {
                    let _ = gate.lock().unwrap().recv();
                }
                Some(b.label())
            },
            Duration::from_millis(200),
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the probe does not wait for it"
        );
        assert_eq!(
            found,
            [
                (Backend::Vaapi { low_power: true }, "VA-API H.264"),
                (Backend::Vulkan, "Vulkan H.264"),
                (Backend::X264, "Software H.264")
            ]
        );
    }

    #[test]
    fn the_log_level_comes_back_after_the_last_quiet_caller() {
        use ffmpeg::util::log::Level;
        let mut hush = Hush {
            callers: 0,
            level: None,
        };
        hush.enter(Level::Error);
        hush.enter(Level::Quiet);
        assert_eq!(
            hush.leave(),
            None,
            "the probe returned; a late thread still runs"
        );
        assert_eq!(hush.leave(), Some(Level::Error));
        hush.enter(Level::Warning);
        assert_eq!(hush.leave(), Some(Level::Warning));
    }

    #[test]
    fn a_backend_that_fails_its_test_frame_is_skipped() {
        let found = quietly(|| found(|b| test_frame(b, Path::new("/dev/null")).is_ok()));
        assert_eq!(found, [Backend::X264], "the CPU codes it");
    }

    #[test]
    fn only_mappable_modifiers_are_offered() {
        let invalid: u64 = gbm::Modifier::Invalid.into();
        let maps = |format, modifier| match modifier {
            CCS_CC => false,
            CCS => format != ABGR8888,
            _ => true,
        };
        assert_eq!(
            mappable(&[CCS_CC, Y_TILED, invalid, CCS, Y_TILED], maps),
            vec![DRM_FORMAT_MOD_LINEAR, Y_TILED],
            "LINEAR first; a modifier one format cannot map in is left out"
        );
        assert_eq!(
            modifiers(Backend::X264, Path::new(RENDER_NODE)),
            vec![DRM_FORMAT_MOD_LINEAR],
            "the CPU reads LINEAR only"
        );
    }

    #[test]
    fn the_test_pattern_is_full_range_bt709_luma() {
        assert!((pattern_luma(0, 0) - 9.24).abs() < 0.01, "blue 128 alone");
        assert!((pattern_luma(255, 255) - 245.8).abs() < 0.1);
        let px = |format, x: u32, y: u32| {
            let at = ((y * TEST_SIDE + x) * 4) as usize;
            pattern(format, TEST_SIDE * 4)[at..at + 4].to_vec()
        };
        assert_eq!(px(XRGB8888, 2, 1), [128, 1, 2, 255], "B, G, R, X in memory");
        assert_eq!(px(ABGR8888, 2, 1), [2, 1, 128, 255], "R, G, B, A in memory");
    }

    #[test]
    fn the_session_devices_render_node_comes_before_render_d128() {
        let tmp = tempfile::tempdir().unwrap();
        let node = tmp.path().join("renderD129");
        std::fs::write(&node, b"").unwrap();
        let fallback = PathBuf::from(RENDER_NODE);
        assert_eq!(render_node(Some(7), |_| Some(node.clone())), node);
        assert_eq!(
            render_node(None, |_| Some(node.clone())),
            fallback,
            "no device named"
        );
        assert_eq!(
            render_node(Some(7), |_| None),
            fallback,
            "a device with no render node"
        );
        let gone = tmp.path().join("renderD130");
        assert_eq!(
            render_node(Some(7), |_| Some(gone)),
            fallback,
            "a node that is not there"
        );
    }

    #[test]
    fn a_main_device_is_a_native_dev_t() {
        let dev: libc::dev_t = 0xe2_80;
        assert_eq!(dev_from_bytes(&dev.to_ne_bytes()), Some(dev));
        assert_eq!(dev_from_bytes(&[1, 2, 3]), None);
    }

    #[test]
    fn in_formats_lists_the_modifiers_of_the_stream_formats() {
        let nv12 = u32::from_le_bytes(*b"NV12");
        let mut blob = Vec::new();
        for v in [1u32, 0, 2, 24, 3, 32] {
            blob.extend(v.to_ne_bytes());
        }
        blob.extend(nv12.to_ne_bytes());
        blob.extend(XRGB8888.to_ne_bytes());
        for (mask, modifier) in [(0b01u64, 7u64), (0b10, 8), (0b11, 9)] {
            blob.extend(mask.to_ne_bytes());
            blob.extend(0u32.to_ne_bytes());
            blob.extend(0u32.to_ne_bytes());
            blob.extend(modifier.to_ne_bytes());
        }
        assert_eq!(in_formats(&blob, &FORMATS), vec![8, 9]);
    }

    /// `gpu` cuts a part at an odd origin out of a [`pattern`] frame: each luma is the
    /// pattern's at the same place in the frame.
    fn cuts_out_a_part(gpu: Gpu) {
        let node = Path::new(RENDER_NODE);
        let devices = Devices::open(gpu, node).expect("the GPU's device");
        let gbm = gbm_device(node).unwrap();
        let frame = test_buffer(&gbm, &mut Painter::new(node), XRGB8888, None).unwrap();
        let (origin, size) = ((37, 51), (128, 96));
        let mut converter =
            Converter::cropped(Some(&devices), XRGB8888, origin, size, TEST_RATE).unwrap();
        assert!(converter.on_gpu());
        let cut = hwmap::download(&converter.convert(&frame).unwrap()).unwrap();
        assert_eq!((cut.width(), cut.height()), size);
        for y in (4..size.1).step_by(8) {
            for x in (4..size.0).step_by(8) {
                let got = luma_at(&cut, x, y).unwrap();
                let want = pattern_luma(x + origin.0, y + origin.1);
                assert!(
                    (got - want).abs() <= 3.0,
                    "luma {got} at ({x}, {y}), {want} in the frame"
                );
            }
        }
    }

    #[test]
    #[ignore = "needs VA-API on /dev/dri/renderD128; run with --ignored where it has one"]
    fn vaapi_cuts_a_part_out_of_the_frame() {
        cuts_out_a_part(Gpu::Vaapi);
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn cuda_cuts_a_part_out_of_the_frame() {
        cuts_out_a_part(Gpu::Cuda);
    }

    #[test]
    #[ignore = "needs Vulkan with libplacebo on /dev/dri/renderD128"]
    fn vulkan_cuts_a_part_out_of_the_frame() {
        cuts_out_a_part(Gpu::Vulkan);
    }

    #[test]
    #[ignore = "needs an NVIDIA GPU with NVENC"]
    fn nvenc_passes_the_probe_with_the_drivers_tiled_modifiers() {
        let node = Path::new(RENDER_NODE);
        let started = Instant::now();
        quietly(|| test_frame(Backend::Nvenc, node)).expect("NVENC codes the test frame");
        let found = modifiers(Backend::Nvenc, node);
        let took = started.elapsed();
        eprintln!("NVENC's probe took {took:?} and found {found:#x?}");
        assert!(found.len() > 1, "a tiled modifier besides LINEAR");
        assert!(took < PROBE_TIMEOUT, "{took:?}");
    }
}
