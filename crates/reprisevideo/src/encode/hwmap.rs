//! Stream frames as encoder input: dma-bufs mapped onto the encoder's GPU or drawn into CUDA
//! frames, as NV12 in the [`colour`] YUV; the software encoder's frames go through swscale.

use super::EncodeError;
use super::colour::{self, FULL_RANGE};
use super::dmabuf::DmabufRead;
use super::split::CudaViews;
use crate::stream::{ABGR8888, ARGB8888, Frame, FrameBuffer, Plane, XBGR8888, XRGB8888};
use ffmpeg::ffi;
use ffmpeg::format::Pixel;
use ffmpeg::frame;
use ffmpeg::software::scaling::Flags;
use std::ffi::{CStr, CString};
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::Arc;

/// An owned reference to an FFmpeg buffer: a device or frames context.
pub struct Ref(NonNull<ffi::AVBufferRef>);

impl Ref {
    /// Takes over `ptr`, an owned reference or null.
    fn new(ptr: *mut ffi::AVBufferRef) -> Option<Ref> {
        NonNull::new(ptr).map(Ref)
    }

    pub fn as_ptr(&self) -> *mut ffi::AVBufferRef {
        self.0.as_ptr()
    }

    /// A new reference for FFmpeg to own.
    pub fn new_ref(&self) -> *mut ffi::AVBufferRef {
        // SAFETY: `self` holds a live reference.
        unsafe { ffi::av_buffer_ref(self.as_ptr()) }
    }

    /// A second reference to the same buffer.
    pub fn try_clone(&self) -> Result<Ref, EncodeError> {
        Ref::new(self.new_ref())
            .ok_or_else(|| EncodeError("no memory for a buffer reference".into()))
    }
}

impl Drop for Ref {
    fn drop(&mut self) {
        let mut ptr = self.as_ptr();
        // SAFETY: this value's own reference, released once.
        unsafe { ffi::av_buffer_unref(&mut ptr) };
    }
}

pub(super) fn check(ret: libc::c_int, what: &str) -> Result<libc::c_int, EncodeError> {
    match ret {
        0.. => Ok(ret),
        _ => Err(EncodeError(format!("{what}: {}", ffmpeg::Error::from(ret)))),
    }
}

/// The GPU API a dma-buf reaches the encoder through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gpu {
    Vaapi,
    Vulkan,
    /// Drawn by OpenGL ES (split.rs's views) and copied into CUDA frames on the GPU.
    Cuda,
}

impl Gpu {
    fn pixel(self) -> ffi::AVPixelFormat {
        match self {
            Gpu::Vaapi => ffi::AVPixelFormat::VAAPI,
            Gpu::Vulkan => ffi::AVPixelFormat::VULKAN,
            Gpu::Cuda => ffi::AVPixelFormat::CUDA,
        }
    }

    /// The filter and options turning mapped RGB of `source` into BT.709 NV12 of `target`; naming
    /// the primaries and transfer on both sides keeps drivers from converting them or using BT.601.
    pub(super) fn to_nv12(
        self,
        source: (u32, u32),
        target: (u32, u32),
    ) -> Result<(&'static str, CString), EncodeError> {
        let (name, mut options) = match self {
            Gpu::Vaapi => (
                "scale_vaapi",
                format!(
                    "format=nv12:out_color_matrix=bt709:out_range={}:out_color_primaries=bt709:\
                     out_color_transfer=iec61966-2-1",
                    if FULL_RANGE { "full" } else { "tv" }
                ),
            ),
            Gpu::Vulkan => (
                "libplacebo",
                format!(
                    "format=nv12:colorspace=bt709:range={}:dithering=none",
                    if FULL_RANGE { "pc" } else { "tv" }
                ),
            ),
            Gpu::Cuda => return Err(EncodeError("CUDA frames are drawn, not filtered".into())),
        };
        if target != source {
            options += &format!(":w={}:h={}", target.0, target.1);
        }
        Ok((name, CString::new(options).expect("no NUL in the options")))
    }
}

/// The device one encoder runs on, of the GPU behind its render node.
pub struct Devices {
    pub(super) gpu: Gpu,
    pub(super) device: Ref,
    node: PathBuf,
}

impl Devices {
    /// `gpu` on render node `node`. CUDA is found through Vulkan, which names the node's GPU.
    pub fn open(gpu: Gpu, node: &Path) -> Result<Devices, EncodeError> {
        let vulkan = || {
            derive(
                ffi::AVHWDeviceType::VULKAN,
                &create(ffi::AVHWDeviceType::DRM, node)?,
            )
        };
        let device = match gpu {
            Gpu::Vaapi => create(ffi::AVHWDeviceType::VAAPI, node)?,
            Gpu::Vulkan => vulkan()?,
            Gpu::Cuda => derive(ffi::AVHWDeviceType::CUDA, &vulkan()?)?,
        };
        Ok(Devices {
            gpu,
            device,
            node: node.to_path_buf(),
        })
    }
}

fn create(kind: ffi::AVHWDeviceType, node: &Path) -> Result<Ref, EncodeError> {
    let name = CString::new(node.as_os_str().as_bytes())
        .map_err(|_| EncodeError(format!("{}: not a device path", node.display())))?;
    let mut ptr = ptr::null_mut();
    // SAFETY: `ptr` receives an owned reference; `name` outlives the call.
    let ret =
        unsafe { ffi::av_hwdevice_ctx_create(&mut ptr, kind, name.as_ptr(), ptr::null_mut(), 0) };
    check(
        ret,
        &format!("{}: {} device", node.display(), type_name(kind)),
    )?;
    Ref::new(ptr).ok_or_else(|| EncodeError("no device".into()))
}

fn derive(kind: ffi::AVHWDeviceType, from: &Ref) -> Result<Ref, EncodeError> {
    let mut ptr = ptr::null_mut();
    // SAFETY: `ptr` receives an owned reference; `from` is a live device.
    let ret = unsafe { ffi::av_hwdevice_ctx_create_derived(&mut ptr, kind, from.as_ptr(), 0) };
    check(ret, &format!("{} device", type_name(kind)))?;
    Ref::new(ptr).ok_or_else(|| EncodeError("no device".into()))
}

fn type_name(kind: ffi::AVHWDeviceType) -> String {
    // SAFETY: FFmpeg returns a static name or null.
    let name = unsafe { ffi::av_hwdevice_get_type_name(kind) };
    match name.is_null() {
        true => format!("{kind:?}"),
        // SAFETY: a static NUL-terminated name.
        false => unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned(),
    }
}

/// A frames context of `format` frames holding `sw` pixels, with `pool` frames allocated up
/// front (none for one that only takes mapped frames).
pub(super) fn frames(
    device: &Ref,
    format: ffi::AVPixelFormat,
    sw: ffi::AVPixelFormat,
    (width, height): (u32, u32),
    pool: i32,
) -> Result<Ref, EncodeError> {
    // SAFETY: a fresh frames context on a live device, filled in before its init.
    unsafe {
        let r = Ref::new(ffi::av_hwframe_ctx_alloc(device.as_ptr()))
            .ok_or_else(|| EncodeError("no frames context".into()))?;
        let fc = (*r.as_ptr()).data as *mut ffi::AVHWFramesContext;
        (*fc).format = format;
        (*fc).sw_format = sw;
        (*fc).width = width as i32;
        (*fc).height = height as i32;
        (*fc).initial_pool_size = pool;
        check(ffi::av_hwframe_ctx_init(r.as_ptr()), "frames context")?;
        Ok(r)
    }
}

/// The pixel format of DRM `fourcc` in memory.
pub(super) fn sw_pixel(fourcc: u32) -> Result<ffi::AVPixelFormat, EncodeError> {
    match fourcc {
        XRGB8888 => Ok(ffi::AVPixelFormat::BGR0),
        ARGB8888 => Ok(ffi::AVPixelFormat::BGRA),
        XBGR8888 => Ok(ffi::AVPixelFormat::RGB0),
        ABGR8888 => Ok(ffi::AVPixelFormat::RGBA),
        _ => Err(EncodeError(format!("format {fourcc:#x} is not supported"))),
    }
}

/// The size the encoder codes for a picture of `size`: 4:2:0 needs even sides, so an odd last
/// column or row is left out.
pub fn even((w, h): (u32, u32)) -> (u32, u32) {
    (w & !1, h & !1)
}

/// A frame's descriptor and the dma-bufs it names, freed with the FFmpeg frame that holds it.
struct DrmHold {
    desc: ffi::AVDRMFrameDescriptor,
    _objects: Vec<Arc<OwnedFd>>,
}

unsafe extern "C" fn free_drm(opaque: *mut libc::c_void, _: *mut u8) {
    // SAFETY: `opaque` is the box `drm_frame` leaked for this buffer, freed once.
    drop(unsafe { Box::from_raw(opaque as *mut DrmHold) });
}

/// A DRM_PRIME frame of `size` describing a dma-buf; the frame keeps the buffers open.
pub(super) fn drm_frame(
    objects: &[Arc<OwnedFd>],
    fourcc: u32,
    modifier: u64,
    planes: &[Plane],
    (width, height): (u32, u32),
) -> Result<frame::Video, EncodeError> {
    let max = ffi::AV_DRM_MAX_PLANES.0 as usize;
    if objects.is_empty() || objects.len() > max || planes.is_empty() || planes.len() > max {
        return Err(EncodeError(format!(
            "{} buffers with {} planes do not fit a frame",
            objects.len(),
            planes.len()
        )));
    }
    if planes.iter().any(|p| p.object >= objects.len()) {
        return Err(EncodeError(
            "a plane lies outside the frame's buffers".into(),
        ));
    }
    // SAFETY: all-zero is a valid descriptor (no objects, no layers).
    let mut desc: ffi::AVDRMFrameDescriptor = unsafe { std::mem::zeroed() };
    desc.nb_objects = objects.len() as i32;
    for (slot, fd) in desc.objects.iter_mut().zip(objects) {
        // SAFETY: an lseek on an open dma-buf, which reports its size.
        let size = unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_END) };
        slot.fd = fd.as_raw_fd();
        slot.size = size.max(0) as usize;
        slot.format_modifier = modifier;
    }
    desc.nb_layers = 1;
    let layer = &mut desc.layers[0];
    layer.format = fourcc;
    layer.nb_planes = planes.len() as i32;
    for (slot, plane) in layer.planes.iter_mut().zip(planes) {
        slot.object_index = plane.object as i32;
        slot.offset = plane.offset as isize;
        slot.pitch = plane.stride as isize;
    }
    let hold = Box::into_raw(Box::new(DrmHold {
        desc,
        _objects: objects.to_vec(),
    }));
    let mut out = frame::Video::empty();
    // SAFETY: the buffer wraps the boxed descriptor and frees the box when the last frame
    // referencing it goes; the frame takes that one reference.
    unsafe {
        let data = (&raw mut (*hold).desc) as *mut u8;
        let size = std::mem::size_of::<ffi::AVDRMFrameDescriptor>();
        let buf = ffi::av_buffer_create(data, size, Some(free_drm), hold as *mut _, 0);
        if buf.is_null() {
            drop(Box::from_raw(hold));
            return Err(EncodeError("no frame buffer".into()));
        }
        let f = out.as_mut_ptr();
        (*f).format = ffi::AVPixelFormat::DRM_PRIME.0;
        (*f).width = width as i32;
        (*f).height = height as i32;
        (*f).data[0] = data;
        (*f).buf[0] = buf;
    }
    Ok(out)
}

/// Turns stream frames of one size and format into what the encoder takes, scaled to a target
/// size.
pub struct Converter {
    pub size: (u32, u32),
    pub target: (u32, u32),
    pub fourcc: u32,
    /// The top-left of the part of each frame read, on the CPU; (0, 0) on a GPU.
    origin: (u32, u32),
    route: Route,
}

enum Route {
    /// Mapped onto the GPU and converted to NV12 there by a filter.
    Filter {
        gpu: Gpu,
        /// Frames dma-bufs are mapped to; a fresh context per converter, as a failed import
        /// downgrades its context for good.
        mapped: Ref,
        graph: ffmpeg::filter::Graph,
    },
    /// Drawn as the AVC444 main view, which is the picture in 4:2:0, into CUDA frames.
    Cuda(CudaViews),
    Cpu {
        sws: Scaler,
        out: frame::Video,
    },
}

impl Converter {
    /// A converter for frames of DRM `fourcc` read at `size` and coded at `target` (both even)
    /// and `frame_rate`, on `devices` or, without them, the CPU.
    pub fn new(
        devices: Option<&Devices>,
        fourcc: u32,
        size: (u32, u32),
        target: (u32, u32),
        frame_rate: u32,
    ) -> Result<Converter, EncodeError> {
        let sw = sw_pixel(fourcc)?;
        let route = match devices {
            Some(d) if d.gpu == Gpu::Cuda => Route::Cuda(CudaViews::new(
                &d.node, &d.device, size, target, target, None,
            )?),
            Some(d) => {
                let mapped = frames(&d.device, d.gpu.pixel(), sw, size, 0)?;
                Route::Filter {
                    gpu: d.gpu,
                    graph: graph(&d.device, &mapped, d.gpu, (size, target), frame_rate)?,
                    mapped,
                }
            }
            None => {
                let to = ffi::AVPixelFormat::YUV420P;
                let sws = Scaler::new(sw, size, to, target, super::cpu_threads_for(size))?;
                // SAFETY: a live scaler.
                unsafe { to_bt709(sws.0.as_ptr()) };
                Route::Cpu {
                    sws,
                    out: frame::Video::new(Pixel::YUV420P, target.0, target.1),
                }
            }
        };
        Ok(Converter {
            size,
            target,
            fourcc,
            origin: (0, 0),
            route,
        })
    }

    /// Read each frame's `size` from `origin` on rather than from its top-left. Only the CPU
    /// reads a part of a frame.
    pub fn crop_at(&mut self, origin: (u32, u32)) -> Result<(), EncodeError> {
        match self.route {
            Route::Cpu { .. } => {
                self.origin = origin;
                Ok(())
            }
            _ => Err(EncodeError("only the CPU reads a part of a frame".into())),
        }
    }

    /// The frames context the encoder's input comes from; None on the CPU.
    pub fn frames(&mut self) -> Option<*mut ffi::AVBufferRef> {
        match &mut self.route {
            Route::Cuda(views) => Some(views.frames()),
            Route::Filter { graph, .. } => {
                let mut sink = graph.get("out")?;
                // SAFETY: the sink of a configured graph; the context lives with the graph.
                Some(unsafe { ffi::av_buffersink_get_hw_frames_ctx(sink.as_mut_ptr()) })
            }
            Route::Cpu { .. } => None,
        }
    }

    /// The encoder's pixel format.
    pub fn pixel(&self) -> Pixel {
        match &self.route {
            Route::Cuda(_) => Pixel::CUDA,
            Route::Filter { gpu, .. } => Pixel::from(gpu.pixel()),
            Route::Cpu { .. } => Pixel::YUV420P,
        }
    }

    /// `frame` cropped to `self.size` and scaled to `self.target`, as the encoder takes it: NV12
    /// on the GPU or YUV 4:2:0 in memory.
    pub fn convert(&mut self, frame: &Frame) -> Result<frame::Video, EncodeError> {
        match &mut self.route {
            Route::Filter { gpu, mapped, graph } => {
                let input = mapped_frame(frame, self.size, mapped, *gpu)?;
                filter(graph, &input)
            }
            Route::Cuda(views) => views.main(frame),
            Route::Cpu { sws, out } => {
                // SAFETY: `out` is a frame this converter owns.
                let writable = unsafe { ffi::av_frame_make_writable(out.as_mut_ptr()) };
                check(writable, "frame buffer")?;
                let mut scaled = Ok(());
                let (x, y) = self.origin;
                let read = (x + self.size.0, y + self.size.1);
                read_rgb(frame, read, |pixels, stride| {
                    let from = y as usize * stride as usize + x as usize * 4;
                    scaled = sws.scale(&pixels[from..], stride, out);
                })?;
                scaled?;
                let mut copy = frame::Video::empty();
                // SAFETY: `copy` is empty and takes a new reference to `out`'s buffers.
                let shared = unsafe { ffi::av_frame_ref(copy.as_mut_ptr(), out.as_ptr()) };
                check(shared, "frame reference")?;
                copy.set_color_range(colour::RANGE);
                copy.set_color_space(ffmpeg::color::Space::BT709);
                Ok(copy)
            }
        }
    }
}

/// A bilinear swscale context, slicing each picture across its threads.
pub(super) struct Scaler(NonNull<ffi::SwsContext>);

impl Scaler {
    /// Scales `from` pixels of `size` to `to` pixels of `target` on `threads` threads; one
    /// starts no pool.
    pub(super) fn new(
        from: ffi::AVPixelFormat,
        size: (u32, u32),
        to: ffi::AVPixelFormat,
        target: (u32, u32),
        threads: usize,
    ) -> Result<Scaler, EncodeError> {
        let none = || EncodeError("colour conversion: no scaler".into());
        // SAFETY: fields of a context not yet initialized; drop frees it if the init fails.
        unsafe {
            let sws = Scaler(NonNull::new(ffi::sws_alloc_context()).ok_or_else(none)?);
            let c = sws.0.as_ptr();
            (*c).flags = (Flags::BILINEAR | Flags::ACCURATE_RND).bits() as libc::c_uint;
            (*c).threads = threads as libc::c_int;
            ((*c).src_w, (*c).src_h) = (size.0 as libc::c_int, size.1 as libc::c_int);
            ((*c).dst_w, (*c).dst_h) = (target.0 as libc::c_int, target.1 as libc::c_int);
            ((*c).src_format, (*c).dst_format) = (from.0, to.0);
            check(
                ffi::sws_init_context(c, ptr::null_mut(), ptr::null_mut()),
                "colour conversion",
            )?;
            Ok(sws)
        }
    }

    /// The picture in `pixels`, rows `stride` bytes apart, scaled into `out`, a writable frame
    /// of the target.
    pub(super) fn scale(
        &mut self,
        pixels: &[u8],
        stride: u32,
        out: &mut frame::Video,
    ) -> Result<(), EncodeError> {
        unsafe extern "C" fn borrowed(_: *mut libc::c_void, _: *mut u8) {}
        let mut src = frame::Video::empty();
        // Only sws_scale_frame slices across threads; the borrowed buffer saves it a copy.
        // SAFETY: `src` describes `pixels`, which outlive it, and its buffer frees nothing.
        unsafe {
            let (s, c) = (src.as_mut_ptr(), self.0.as_ptr());
            ((*s).format, (*s).width, (*s).height) = ((*c).src_format, (*c).src_w, (*c).src_h);
            let format = ffi::AVPixelFormat((*c).src_format);
            let row = ffi::av_image_get_linesize(format, (*c).src_w, 0).max(0) as usize;
            let last = stride as usize * ((*s).height as usize).saturating_sub(1);
            if pixels.len() < last + row {
                return Err(EncodeError("the frame is smaller than its size".into()));
            }
            (*s).data[0] = pixels.as_ptr().cast_mut();
            (*s).linesize[0] = stride as libc::c_int;
            (*s).buf[0] = ffi::av_buffer_create(
                (*s).data[0],
                pixels.len(),
                Some(borrowed),
                ptr::null_mut(),
                ffi::AV_BUFFER_FLAG_READONLY,
            );
            if (*s).buf[0].is_null() {
                return Err(EncodeError("colour conversion: no source buffer".into()));
            }
            check(
                ffi::sws_scale_frame(c, out.as_mut_ptr(), s),
                "colour conversion",
            )?;
        }
        Ok(())
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        let mut c = self.0.as_ptr();
        // SAFETY: the context this owns, freed once.
        unsafe { ffi::sws_free_context(&mut c) };
    }
}

/// Sets scaler `c` to write [`colour`]'s YUV from full-range RGB.
unsafe fn to_bt709(c: *mut ffi::SwsContext) {
    // SAFETY: the caller's live scaler; the coefficient tables are static.
    unsafe {
        let bt709 = ffi::sws_getCoefficients(ffi::SWS_CS_ITU709);
        let full = i32::from(FULL_RANGE);
        ffi::sws_setColorspaceDetails(c, bt709, 1, bt709, full, 0, 1 << 16, 1 << 16);
    }
}

/// Pictures of DRM `fourcc` scaled on the CPU to a target size, in the same format.
pub(super) struct Rescale {
    size: (u32, u32),
    sws: Scaler,
    out: frame::Video,
}

impl Rescale {
    pub(super) fn new(
        fourcc: u32,
        size: (u32, u32),
        target: (u32, u32),
    ) -> Result<Rescale, EncodeError> {
        let pixel = sw_pixel(fourcc)?;
        Ok(Rescale {
            size,
            sws: Scaler::new(pixel, size, pixel, target, super::cpu_threads_for(size))?,
            out: frame::Video::new(Pixel::from(pixel), target.0, target.1),
        })
    }

    /// [`read_rgb`] of `frame` at the source size, scaled: `f` gets the target's pixels.
    pub(super) fn read(
        &mut self,
        frame: &Frame,
        f: impl FnOnce(&[u8], u32),
    ) -> Result<(), EncodeError> {
        let Rescale { size, sws, out } = self;
        let mut scaled = Ok(());
        read_rgb(frame, *size, |pixels, stride| {
            scaled = sws.scale(pixels, stride, out);
        })?;
        scaled?;
        f(out.data(0), out.stride(0) as u32);
        Ok(())
    }
}

/// A dma-buf `frame` mapped into a `gpu` frame of `mapped`, tagged as full-range sRGB.
fn mapped_frame(
    frame: &Frame,
    size: (u32, u32),
    mapped: &Ref,
    gpu: Gpu,
) -> Result<frame::Video, EncodeError> {
    let FrameBuffer::Dmabuf {
        objects,
        fourcc,
        modifier,
        planes,
    } = &frame.buffer
    else {
        return Err(EncodeError("a shared-memory frame cannot be mapped".into()));
    };
    let drm = drm_frame(objects, *fourcc, *modifier, planes, size)?;
    let mut input = map(&drm, mapped, gpu)?;
    let (space, range) = INPUT_COLOUR;
    // SAFETY: plain fields of a frame this function owns.
    unsafe {
        let f = input.as_mut_ptr();
        (*f).colorspace = space;
        (*f).color_range = range;
        (*f).color_primaries = ffi::AVColorPrimaries::BT709;
        (*f).color_trc = ffi::AVColorTransferCharacteristic::IEC61966_2_1;
    }
    Ok(input)
}

/// Calls `f` with the frame's first plane and its stride, mapping a linear dma-buf for reading.
/// The plane holds at least `size`.
pub(super) fn read_rgb(
    frame: &Frame,
    (w, rows): (u32, u32),
    f: impl FnOnce(&[u8], u32),
) -> Result<(), EncodeError> {
    let short = || EncodeError("the frame is smaller than its size".into());
    match &frame.buffer {
        FrameBuffer::Shm { data, stride, .. } => {
            if *stride < w * 4 || data.len() < *stride as usize * rows as usize {
                return Err(short());
            }
            f(data, *stride);
            Ok(())
        }
        FrameBuffer::Dmabuf {
            objects,
            modifier,
            planes,
            ..
        } => {
            if *modifier != crate::stream::DRM_FORMAT_MOD_LINEAR {
                return Err(EncodeError(format!(
                    "modifier {modifier:#x} cannot be read by the CPU"
                )));
            }
            let Some(&plane) = planes.first() else {
                return Err(short());
            };
            let Some(fd) = objects.get(plane.object) else {
                return Err(short());
            };
            if plane.stride < w * 4 {
                return Err(short());
            }
            let len = plane.offset as usize + plane.stride as usize * rows as usize;
            let map = DmabufRead::new(fd, len)?;
            f(&map.bytes()[plane.offset as usize..], plane.stride);
            Ok(())
        }
    }
}

/// `drm` mapped into a `gpu` frame of `mapped`, without a copy.
pub(super) fn map(drm: &frame::Video, mapped: &Ref, gpu: Gpu) -> Result<frame::Video, EncodeError> {
    let mut out = frame::Video::empty();
    let flags = ffi::AV_HWFRAME_MAP_READ.0 | ffi::AV_HWFRAME_MAP_DIRECT.0;
    // SAFETY: `out` is empty and takes a new reference to `mapped`; `drm` is a DRM_PRIME
    // frame that the mapping keeps a reference to.
    unsafe {
        let o = out.as_mut_ptr();
        (*o).hw_frames_ctx = mapped.new_ref();
        (*o).format = gpu.pixel().0;
        check(
            ffi::av_hwframe_map(o, drm.as_ptr(), flags as i32),
            "dma-buf map",
        )?;
    }
    Ok(out)
}

/// The colour space and range a mapped frame is tagged with: full-range RGB.
const INPUT_COLOUR: (ffi::AVColorSpace, ffi::AVColorRange) =
    (ffi::AVColorSpace::RGB, ffi::AVColorRange::JPEG);

/// A graph from frames of `mapped` of the source size through the GPU's NV12 conversion to the
/// target size on `device`.
fn graph(
    device: &Ref,
    mapped: &Ref,
    gpu: Gpu,
    ((w, h), target): ((u32, u32), (u32, u32)),
    frame_rate: u32,
) -> Result<ffmpeg::filter::Graph, EncodeError> {
    let mut graph = ffmpeg::filter::Graph::new();
    let (convert, options) = gpu.to_nv12((w, h), target)?;
    let find = |name| {
        ffmpeg::filter::find(name).ok_or_else(|| EncodeError(format!("FFmpeg has no {name}")))
    };
    let (buffer, convert, sink) = (find("buffer")?, find(convert)?, find("buffersink")?);
    let (space, range) = INPUT_COLOUR;
    // SAFETY: filters allocated in `graph`, each given what it needs before its init: the
    // source its parameters (it takes its own reference to `mapped`), the conversion the
    // device to run on. Links join the three in order.
    unsafe {
        let g = graph.as_mut_ptr();
        let alloc = |filter: &ffmpeg::Filter, name: &CStr| {
            let ctx = ffi::avfilter_graph_alloc_filter(g, filter.as_ptr(), name.as_ptr());
            match ctx.is_null() {
                true => Err(EncodeError(format!("no {} filter", filter.name()))),
                false => Ok(ctx),
            }
        };
        let source = alloc(&buffer, c"in")?;
        let par = ffi::av_buffersrc_parameters_alloc();
        if par.is_null() {
            return Err(EncodeError("no source parameters".into()));
        }
        (*par).format = gpu.pixel().0;
        (*par).width = w as i32;
        (*par).height = h as i32;
        (*par).time_base = ffi::AVRational {
            num: 1,
            den: frame_rate.max(1) as i32,
        };
        (*par).sample_aspect_ratio = ffi::AVRational { num: 1, den: 1 };
        (*par).color_space = space;
        (*par).color_range = range;
        (*par).hw_frames_ctx = mapped.as_ptr();
        let ret = ffi::av_buffersrc_parameters_set(source, par);
        ffi::av_free(par as *mut _);
        check(ret, "conversion source")?;
        check(
            ffi::avfilter_init_str(source, ptr::null()),
            "conversion source",
        )?;
        let converter = alloc(&convert, c"convert")?;
        (*converter).hw_device_ctx = device.new_ref();
        check(
            ffi::avfilter_init_str(converter, options.as_ptr()),
            convert.name(),
        )?;
        let out = alloc(&sink, c"out")?;
        check(ffi::avfilter_init_str(out, ptr::null()), "conversion sink")?;
        check(
            ffi::avfilter_link(source, 0, converter, 0),
            "conversion link",
        )?;
        check(ffi::avfilter_link(converter, 0, out, 0), "conversion link")?;
    }
    graph
        .validate()
        .map_err(|e| EncodeError(format!("conversion graph: {e}")))?;
    Ok(graph)
}

fn filter(
    graph: &mut ffmpeg::filter::Graph,
    input: &frame::Video,
) -> Result<frame::Video, EncodeError> {
    let err = |e: ffmpeg::Error| EncodeError(format!("colour conversion: {e}"));
    graph
        .get("in")
        .expect("the graph's source")
        .source()
        .add(input)
        .map_err(err)?;
    let mut out = frame::Video::empty();
    graph
        .get("out")
        .expect("the graph's sink")
        .sink()
        .frame(&mut out)
        .map_err(err)?;
    Ok(out)
}

/// `hw` read back into memory, for the probe's check of a conversion.
pub fn download(hw: &frame::Video) -> Result<frame::Video, EncodeError> {
    let mut out = frame::Video::empty();
    // SAFETY: `out` is empty and receives a memory copy of `hw`.
    let copied = unsafe { ffi::av_hwframe_transfer_data(out.as_mut_ptr(), hw.as_ptr(), 0) };
    check(copied, "download")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `data`, rows `stride` bytes apart, through sws_scale on one thread.
    fn one_thread(
        (from, size): (ffi::AVPixelFormat, (u32, u32)),
        (to, target): (ffi::AVPixelFormat, (u32, u32)),
        data: &[u8],
        stride: usize,
    ) -> frame::Video {
        let (from, to) = (Pixel::from(from), Pixel::from(to));
        let flags = Flags::BILINEAR | Flags::ACCURATE_RND;
        let get = ffmpeg::software::scaling::Context::get;
        let mut sws = get(from, size.0, size.1, to, target.0, target.1, flags).unwrap();
        let mut out = frame::Video::new(to, target.0, target.1);
        // SAFETY: a live scaler; `data` holds the source rows and `out` the target's.
        unsafe {
            if to == Pixel::YUV420P {
                to_bt709(sws.as_mut_ptr());
            }
            let o = out.as_mut_ptr();
            ffi::sws_scale(
                sws.as_mut_ptr(),
                [data.as_ptr()].as_ptr(),
                [stride as libc::c_int].as_ptr(),
                0,
                size.1 as i32,
                (*o).data.as_ptr(),
                (*o).linesize.as_ptr(),
            );
        }
        out
    }

    /// Plane `i` of `f` without the padding past `bytes` a row.
    fn rows(f: &frame::Video, i: usize, bytes: usize, rows: usize) -> Vec<u8> {
        (0..rows)
            .flat_map(|r| &f.data(i)[r * f.stride(i)..][..bytes])
            .copied()
            .collect()
    }

    #[test]
    fn sliced_scaling_equals_one_thread() {
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let (bgra, yuv) = (ffi::AVPixelFormat::BGRA, ffi::AVPixelFormat::YUV420P);
        for (size, to, target) in [
            ((322, 182), yuv, (322, 182)),
            ((1922, 1082), yuv, (1922, 1082)),
            ((322, 182), yuv, (160, 90)),
            ((321, 181), bgra, (200, 117)),
        ] {
            let stride = size.0 as usize * 4 + 16;
            let data: Vec<u8> = (0..stride * size.1 as usize)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    (seed >> 24) as u8
                })
                .collect();
            let want = one_thread((bgra, size), (to, target), &data, stride);
            for threads in [1, 3, 8] {
                let mut sws = Scaler::new(bgra, size, to, target, threads).unwrap();
                if to == yuv {
                    // SAFETY: a live scaler.
                    unsafe { to_bt709(sws.0.as_ptr()) };
                }
                let mut out = frame::Video::new(Pixel::from(to), target.0, target.1);
                sws.scale(&data, stride as u32, &mut out).unwrap();
                let (w, h) = (target.0 as usize, target.1 as usize);
                let planes = match to == yuv {
                    true => vec![(0, w, h), (1, w / 2, h / 2), (2, w / 2, h / 2)],
                    false => vec![(0, w * 4, h)],
                };
                for (i, bytes, n) in planes {
                    assert!(
                        rows(&out, i, bytes, n) == rows(&want, i, bytes, n),
                        "plane {i} of {size:?} to {target:?} on {threads} threads equals one thread's"
                    );
                }
            }
        }
    }
}
