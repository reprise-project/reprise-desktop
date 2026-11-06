//! The AVC444 views as encoder input, drawn by OpenGL ES into NV12 dma-bufs for VA-API or
//! textures copied into CUDA frames for NVENC, or split on the CPU for the software encoder.

use super::EncodeError;
use super::avc444::{self, Flat, Planes444};
use super::colour;
use super::cuda;
use super::gl::{self, Gl, gl_err};
use super::hwmap::{self, Devices, Gpu, Ref, Rescale};
use crate::stream::{DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Plane, Rect};
use ffmpeg::ffi;
use ffmpeg::format::Pixel;
use ffmpeg::frame;
use glow::HasContext;
use khronos_egl as egl;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// DRM_FORMAT_NV12.
const NV12: u32 = u32::from_le_bytes(*b"NV12");
/// DRM_FORMAT_ABGR8888: R, G, B, A in memory, the view planes' bytes four at a time.
const ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
const MACROBLOCK: u32 = 16;

/// One picture's views and which of its macroblocks the main view alone shows exactly.
pub struct Split {
    pub main: frame::Video,
    pub aux: frame::Video,
    pub flat: Flat,
}

/// Splits stream frames of one size and format into views of a target size.
pub struct Views {
    pub size: (u32, u32),
    pub target: (u32, u32),
    pub fourcc: u32,
    route: Route,
}

enum Route {
    /// Drawn on the GPU into buffers mapped once as the encoder's input frames.
    Gpu {
        main: frame::Video,
        aux: frame::Video,
        mapped: Ref,
        split: Box<GpuSplit>,
    },
    Cuda(CudaViews),
    Cpu {
        main: frame::Video,
        aux: frame::Video,
        /// None where the target is the frame's size.
        rescale: Option<Rescale>,
    },
}

impl Views {
    /// Views of `size` pictures in DRM `fourcc` scaled to `target`, drawn on `devices`' GPU (render
    /// node `node`), else on the CPU; VA-API and CUDA take them from the GPU.
    pub fn new(
        devices: Option<&Devices>,
        node: &Path,
        fourcc: u32,
        size: (u32, u32),
        target: (u32, u32),
    ) -> Result<Views, EncodeError> {
        let view = avc444::view_size(target);
        let route = match devices {
            Some(d) if d.gpu == Gpu::Vaapi => {
                let split = Box::new(GpuSplit::new(node, size, target, Target::Dmabufs)?);
                let mapped = hwmap::frames(
                    &d.device,
                    ffi::AVPixelFormat::VAAPI,
                    ffi::AVPixelFormat::NV12,
                    view,
                    0,
                )?;
                let map = |v: &View| {
                    let Some(buffer) = &v.buffer else {
                        return Err(EncodeError("the view has no dma-buf".into()));
                    };
                    let drm = hwmap::drm_frame(
                        std::slice::from_ref(&buffer.fd),
                        NV12,
                        buffer.modifier,
                        &buffer.planes(),
                        view,
                    )?;
                    let mut f = hwmap::map(&drm, &mapped, Gpu::Vaapi)?;
                    tag(&mut f);
                    Ok::<_, EncodeError>(f)
                };
                Route::Gpu {
                    main: map(&split.views[0])?,
                    aux: map(&split.views[1])?,
                    mapped,
                    split,
                }
            }
            Some(d) if d.gpu == Gpu::Cuda => {
                Route::Cuda(CudaViews::new(node, &d.device, size, target, view, true)?)
            }
            Some(_) => {
                return Err(EncodeError(
                    "the views are drawn for VA-API and CUDA only".into(),
                ));
            }
            None => Route::Cpu {
                main: frame::Video::new(Pixel::NV12, view.0, view.1),
                aux: frame::Video::new(Pixel::NV12, view.0, view.1),
                rescale: match target == size {
                    true => None,
                    false => Some(Rescale::new(fourcc, size, target)?),
                },
            },
        };
        Ok(Views {
            size,
            target,
            fourcc,
            route,
        })
    }

    /// The frames context the encoder's input comes from; None on the CPU.
    pub fn frames(&self) -> Option<*mut ffi::AVBufferRef> {
        match &self.route {
            Route::Gpu { mapped, .. } => Some(mapped.as_ptr()),
            Route::Cuda(c) => Some(c.frames.as_ptr()),
            Route::Cpu { .. } => None,
        }
    }

    /// The encoder's pixel format.
    pub fn pixel(&self) -> Pixel {
        match &self.route {
            Route::Gpu { .. } => Pixel::VAAPI,
            Route::Cuda(_) => Pixel::CUDA,
            Route::Cpu { .. } => Pixel::NV12,
        }
    }

    /// `frame`'s views, as new references to frames the next split overwrites. Clients read
    /// the auxiliary view inside `read` only; outside it repeats the main view.
    pub fn split(&mut self, frame: &Frame, read: Rect) -> Result<Split, EncodeError> {
        match &mut self.route {
            Route::Gpu {
                main, aux, split, ..
            } => {
                let flat = split.draw(frame, read)?;
                referenced(main, aux, flat)
            }
            Route::Cuda(c) => c.views(frame, read),
            Route::Cpu { main, aux, rescale } => {
                for f in [&mut *main, &mut *aux] {
                    // SAFETY: a frame this route owns.
                    let writable = unsafe { ffi::av_frame_make_writable(f.as_mut_ptr()) };
                    hwmap::check(writable, "frame buffer")?;
                }
                let mut planes = None;
                let (target, fourcc) = (self.target, self.fourcc);
                let planes_of = |pixels: &[u8], stride: u32| {
                    planes = Some(Planes444::from_rgb(pixels, stride as usize, target, fourcc));
                };
                match rescale {
                    Some(rescale) => rescale.read(frame, planes_of)?,
                    None => hwmap::read_rgb(frame, self.size, planes_of)?,
                }
                let planes = planes.expect("read_rgb calls back on success");
                let (m, mut a) = avc444::split_v1(&planes);
                avc444::keep_main_outside(&m, &mut a, read);
                fill(main, &m);
                fill(aux, &a);
                tag(main);
                tag(aux);
                referenced(main, aux, Flat::of(&planes))
            }
        }
    }
}

/// New references to `main` and `aux`, with `flat`.
fn referenced(main: &frame::Video, aux: &frame::Video, flat: Flat) -> Result<Split, EncodeError> {
    Ok(Split {
        main: reference(main)?,
        aux: reference(aux)?,
        flat,
    })
}

/// Views drawn into textures and copied on the GPU into CUDA frames, for NVENC.
pub struct CudaViews {
    /// Unregistered before the split deletes its textures.
    textures: cuda::Textures,
    split: Box<GpuSplit>,
    frames: Ref,
    /// The size copied out of the views.
    coded: (u32, u32),
}

impl CudaViews {
    /// Views of `size` pictures scaled to `target` on render node `node`, copied out at `coded` (at
    /// most the views' size) into CUDA device `cuda`'s frames: both views with `aux`, else the main.
    pub fn new(
        node: &Path,
        cuda: &Ref,
        size: (u32, u32),
        target: (u32, u32),
        coded: (u32, u32),
        aux: bool,
    ) -> Result<CudaViews, EncodeError> {
        let split = Box::new(GpuSplit::new(node, size, target, Target::Textures)?);
        let planes: Vec<glow::Texture> = (split.views.iter())
            .take(if aux { 2 } else { 1 })
            .flat_map(|v| [v.luma.texture, v.chroma.texture])
            .collect();
        Ok(CudaViews {
            textures: cuda::Textures::register(&split.gl, cuda, &planes)?,
            frames: hwmap::frames(
                cuda,
                ffi::AVPixelFormat::CUDA,
                ffi::AVPixelFormat::NV12,
                coded,
                4,
            )?,
            split,
            coded,
        })
    }

    pub fn frames(&self) -> *mut ffi::AVBufferRef {
        self.frames.as_ptr()
    }

    /// The main view of `frame` alone: its picture in 4:2:0.
    pub fn main(&mut self, frame: &Frame) -> Result<frame::Video, EncodeError> {
        self.split.draw_main(frame)?;
        let main = self.frame()?;
        self.textures.copy_nv12(&[&main], self.coded)?;
        Ok(main)
    }

    fn views(&mut self, frame: &Frame, read: Rect) -> Result<Split, EncodeError> {
        let flat = self.split.draw(frame, read)?;
        let (main, aux) = (self.frame()?, self.frame()?);
        self.textures.copy_nv12(&[&main, &aux], self.coded)?;
        Ok(Split { main, aux, flat })
    }

    fn frame(&self) -> Result<frame::Video, EncodeError> {
        let mut out = frame::Video::empty();
        // SAFETY: `out` is empty and gets a frame of this route's CUDA frames.
        let got = unsafe { ffi::av_hwframe_get_buffer(self.frames.as_ptr(), out.as_mut_ptr(), 0) };
        hwmap::check(got, "CUDA frame")?;
        tag(&mut out);
        Ok(out)
    }
}

fn tag(f: &mut frame::Video) {
    f.set_color_range(colour::RANGE);
    f.set_color_space(ffmpeg::color::Space::BT709);
}

/// `view`'s planes copied into `f`, an NV12 frame of its size.
fn fill(f: &mut frame::Video, view: &avc444::Nv12) {
    let w = view.width as usize;
    for (plane, bytes) in [(0, &view.y), (1, &view.uv)] {
        let stride = f.stride(plane);
        let data = f.data_mut(plane);
        for (y, line) in bytes.chunks_exact(w).enumerate() {
            data[y * stride..][..w].copy_from_slice(line);
        }
    }
}

fn reference(f: &frame::Video) -> Result<frame::Video, EncodeError> {
    let mut copy = frame::Video::empty();
    // SAFETY: `copy` is empty and takes a new reference to `f`'s buffers.
    let shared = unsafe { ffi::av_frame_ref(copy.as_mut_ptr(), f.as_ptr()) };
    hwmap::check(shared, "frame reference")?;
    Ok(copy)
}

pub(super) const FRAGMENT: &str = include_str!("split.frag");
/// Luma, chroma and the flat macroblocks; see split.frag.
pub(super) const PASSES: usize = 3;

/// An NV12 view as GL textures of four bytes to a texel, one per plane.
struct View {
    luma: Texture,
    chroma: Texture,
    /// The dma-buf the planes lie in, for VA-API.
    buffer: Option<ViewBuffer>,
}

/// An NV12 view's dma-buf, chroma below luma.
struct ViewBuffer {
    bo: gbm::BufferObject<()>,
    fd: Arc<OwnedFd>,
    /// The rows before the chroma plane: the luma rows, rounded up to whole tiles.
    luma_rows: u32,
    modifier: u64,
}

impl ViewBuffer {
    fn planes(&self) -> [Plane; 2] {
        let stride = self.bo.stride();
        [
            Plane {
                object: 0,
                offset: 0,
                stride,
            },
            Plane {
                object: 0,
                offset: stride * self.luma_rows,
                stride,
            },
        ]
    }
}

/// Where the views are drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Dmabufs,
    Textures,
}

/// A texture on an image of a dma-buf, or of its own.
struct Texture {
    image: Option<egl::Image>,
    texture: glow::Texture,
}

/// One GL context on the render node that draws the views of pictures of one size, scaled to
/// another.
struct GpuSplit {
    programs: Vec<glow::Program>,
    /// Each pass's `read_rect`, in the passes that draw the auxiliary view.
    read_rects: Vec<Option<glow::UniformLocation>>,
    vertices: Option<glow::VertexArray>,
    /// The main view, then the auxiliary one.
    views: Vec<View>,
    /// One texel per two 2x2 blocks side by side, red where both are flat.
    blocks: Option<Texture>,
    /// One texel per macroblock, red where it is flat.
    flags: Option<Texture>,
    /// Each pass's framebuffer and size.
    framebuffers: Vec<(glow::Framebuffer, (u32, u32))>,
    /// The frames' size.
    source: (u32, u32),
    /// The picture's size in the views.
    size: (u32, u32),
    /// Dropped after the objects above are deleted.
    gl: Rc<Gl>,
}

impl GpuSplit {
    fn new(
        node: &Path,
        source: (u32, u32),
        size: (u32, u32),
        target: Target,
    ) -> Result<GpuSplit, EncodeError> {
        let mut split = GpuSplit {
            programs: Vec::new(),
            read_rects: Vec::new(),
            vertices: None,
            views: Vec::new(),
            blocks: None,
            flags: None,
            framebuffers: Vec::new(),
            source,
            size,
            gl: Rc::new(Gl::new(node)?),
        };
        let (w, h) = avc444::view_size(size);
        let gl = &split.gl.gl;
        // SAFETY: GL calls on the current context; drop deletes what was made if one fails.
        unsafe {
            for pass in 0..PASSES {
                let p = program(gl, pass, size, source != size)?;
                split.programs.push(p);
                split
                    .read_rects
                    .push(gl.get_uniform_location(p, "read_rect"));
            }
            split.vertices = Some(gl.create_vertex_array().map_err(gl_err("vertex array"))?);
            split.blocks = Some(own_texture(gl, (w / 4, h / 2))?);
            split.flags = Some(own_texture(gl, (w / MACROBLOCK, h / MACROBLOCK))?);
            for _ in 0..2 {
                let v = match target {
                    Target::Dmabufs => dmabuf_view(&split.gl, (w, h))?,
                    Target::Textures => View {
                        luma: own_texture(gl, (w / 4, h))?,
                        chroma: own_texture(gl, (w / 4, h / 2))?,
                        buffer: None,
                    },
                };
                split.views.push(v);
            }
            let (main, aux) = (&split.views[0], &split.views[1]);
            let blocks = split.blocks.as_ref().expect("made above");
            let flags = split.flags.as_ref().expect("made above");
            let passes = [
                (vec![&main.luma, &aux.luma], (w / 4, h)),
                (vec![&main.chroma, &aux.chroma, blocks], (w / 4, h / 2)),
                (vec![flags], (w / MACROBLOCK, h / MACROBLOCK)),
            ];
            for (textures, size) in passes {
                let fb = framebuffer(gl, &textures)?;
                split.framebuffers.push((fb, size));
            }
        }
        Ok(split)
    }

    /// Draw `frame`'s views, the auxiliary one inside `read` only; the main view's flat
    /// macroblocks come back.
    fn draw(&mut self, frame: &Frame, read: Rect) -> Result<Flat, EncodeError> {
        let texels = self.passes(frame, read, PASSES)?;
        Ok(Flat {
            columns: avc444::view_size(self.size).0 / MACROBLOCK,
            blocks: (texels.as_chunks::<4>().0.iter())
                .map(|t| t[0] > 127)
                .collect(),
        })
    }

    /// Draw `frame`'s views without the flags. The CUDA map waits for this GL work, so it
    /// ends with a flush only.
    fn draw_main(&mut self, frame: &Frame) -> Result<(), EncodeError> {
        self.passes(frame, Rect::full((0, 0)), PASSES - 1)
            .map(|_| ())
    }

    /// The first `count` passes over `frame`, which must cover the split's size, and the last
    /// pass's texels once all are.
    fn passes(&mut self, frame: &Frame, read: Rect, count: usize) -> Result<Vec<u8>, EncodeError> {
        let FrameBuffer::Dmabuf {
            objects,
            fourcc,
            modifier,
            planes,
        } = &frame.buffer
        else {
            return Err(EncodeError("a shared-memory frame cannot be mapped".into()));
        };
        if frame.size.0 < self.source.0 || frame.size.1 < self.source.1 {
            return Err(EncodeError("the frame changed size".into()));
        }
        self.gl.current()?;
        let image = self
            .gl
            .import(objects, *fourcc, *modifier, planes, self.source)?;
        let scaled = self.source != self.size;
        let gl = &self.gl.gl;
        let blocks = self.blocks.as_ref().expect("made with the views");
        // SAFETY: GL calls on the current context; the source texture and image are deleted
        // before returning.
        let texels = unsafe {
            let source = gl.create_texture().map_err(gl_err("texture"));
            let drawn = source.and_then(|source| {
                self.gl.bind_image(source, image);
                match scaled {
                    true => linear(gl),
                    false => nearest(gl),
                }
                gl.bind_vertex_array(self.vertices);
                let corners = [read.x, read.y, read.x + read.width, read.y + read.height];
                let [x0, y0, x1, y1] = corners.map(|v| v as i32);
                let passes = self.programs.iter().zip(&self.read_rects);
                for (pass, ((program, read_rect), (fb, (w, h)))) in
                    passes.zip(&self.framebuffers).enumerate().take(count)
                {
                    if pass == PASSES - 1 {
                        gl.bind_texture(glow::TEXTURE_2D, Some(blocks.texture));
                    }
                    gl.bind_framebuffer(glow::FRAMEBUFFER, Some(*fb));
                    gl.viewport(0, 0, *w as i32, *h as i32);
                    gl.use_program(Some(*program));
                    if read_rect.is_some() {
                        gl.uniform_4_i32(read_rect.as_ref(), x0, y0, x1, y1);
                    }
                    gl.draw_arrays(glow::TRIANGLES, 0, 3);
                }
                let mut texels = Vec::new();
                if count == PASSES {
                    let (w, h) = self.framebuffers[PASSES - 1].1;
                    texels = vec![0u8; (w * h * 4) as usize];
                    gl.read_pixels(
                        0,
                        0,
                        w as i32,
                        h as i32,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelPackData::Slice(Some(&mut texels)),
                    );
                    gl.finish();
                } else {
                    gl.flush();
                }
                gl.delete_texture(source);
                match gl.get_error() {
                    glow::NO_ERROR => Ok(texels),
                    e => Err(EncodeError(format!("drawing the views: GL error {e:#x}"))),
                }
            });
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            drawn
        };
        self.gl.destroy_image(image);
        texels
    }
}

impl Drop for GpuSplit {
    fn drop(&mut self) {
        let _ = self.gl.current();
        let gl = &self.gl.gl;
        // SAFETY: objects of this context, deleted once.
        unsafe {
            for (fb, _) in &self.framebuffers {
                gl.delete_framebuffer(*fb);
            }
            let views = self.views.iter().flat_map(|v| [&v.luma, &v.chroma]);
            for t in views.chain(&self.blocks).chain(&self.flags) {
                gl.delete_texture(t.texture);
                if let Some(image) = t.image {
                    self.gl.destroy_image(image);
                }
            }
            for &p in &self.programs {
                gl.delete_program(p);
            }
            if let Some(v) = self.vertices {
                gl.delete_vertex_array(v);
            }
        }
    }
}

/// The source of pass `pass` of split.frag, sampling a picture of another size if `scaled`.
pub(super) fn fragment(pass: usize, scaled: bool) -> String {
    let scaled = if scaled { "#define SCALED\n" } else { "" };
    format!("#version 300 es\n{scaled}#define PASS {pass}\n{FRAGMENT}")
}

/// Pass `pass` of split.frag for pictures of `size`, its uniforms set.
unsafe fn program(
    gl: &glow::Context,
    pass: usize,
    size: (u32, u32),
    scaled: bool,
) -> Result<glow::Program, EncodeError> {
    let fragment = fragment(pass, scaled);
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        let program =
            gl::program(gl, &fragment).map_err(|e| EncodeError(format!("pass {pass}: {}", e.0)))?;
        let at = |name| gl.get_uniform_location(program, name);
        gl.uniform_2_i32(at("size").as_ref(), size.0 as i32, size.1 as i32);
        let fixed = |[r, g, b]: [i32; 3]| (r as f32, g as f32, b as f32);
        for (name, k) in [("ky", colour::Y), ("ku", colour::U), ("kv", colour::V)] {
            let (r, g, b) = fixed(k);
            gl.uniform_3_f32(at(name).as_ref(), r, g, b);
        }
        gl.uniform_1_f32(at("y_offset").as_ref(), colour::Y_OFFSET as f32);
        gl.uniform_1_i32(at("src").as_ref(), 0);
        Ok(program)
    }
}

/// Nearest sampling for the bound texture, which texelFetch reads.
unsafe fn nearest(gl: &glow::Context) {
    for filter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
        // SAFETY: GL calls on the caller's current context.
        unsafe { gl.tex_parameter_i32(glow::TEXTURE_2D, filter, glow::NEAREST as i32) };
    }
}

/// Linear sampling for the bound texture, clamped at its edges, which a scaled picture reads.
unsafe fn linear(gl: &glow::Context) {
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        for filter in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, filter, glow::LINEAR as i32);
        }
        for wrap in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, wrap, glow::CLAMP_TO_EDGE as i32);
        }
    }
}

/// A framebuffer drawing into `textures` in order, checked complete.
unsafe fn framebuffer(
    gl: &glow::Context,
    textures: &[&Texture],
) -> Result<glow::Framebuffer, EncodeError> {
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        let fb = gl.create_framebuffer().map_err(gl_err("framebuffer"))?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
        let mut buffers = Vec::new();
        for (i, t) in textures.iter().enumerate() {
            let attachment = glow::COLOR_ATTACHMENT0 + i as u32;
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                attachment,
                glow::TEXTURE_2D,
                Some(t.texture),
                0,
            );
            buffers.push(attachment);
        }
        gl.draw_buffers(&buffers);
        let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        if status != glow::FRAMEBUFFER_COMPLETE {
            gl.delete_framebuffer(fb);
            return Err(EncodeError(format!("framebuffer incomplete: {status:#x}")));
        }
        Ok(fb)
    }
}

/// An RGBA texture of `size`.
unsafe fn own_texture(gl: &glow::Context, size: (u32, u32)) -> Result<Texture, EncodeError> {
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        let texture = gl.create_texture().map_err(gl_err("texture"))?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_storage_2d(
            glow::TEXTURE_2D,
            1,
            glow::RGBA8,
            size.0 as i32,
            size.1 as i32,
        );
        nearest(gl);
        Ok(Texture {
            image: None,
            texture,
        })
    }
}

/// Intel's Y tiling places bytes the same way whatever the format, so an R8 buffer's planes can
/// be drawn as RGBA in it; other tilings depend on the format, so other GPUs get LINEAR.
const I915_Y_TILED: u64 = 0x0100_0000_0000_0002;
/// Rows of Y tiles, which the chroma plane starts on.
const TILE_ROWS: u32 = 32;

/// An NV12 view of `size` in a dma-buf, and its luma and chroma planes as GL textures.
unsafe fn dmabuf_view(gl: &Gl, size: (u32, u32)) -> Result<View, EncodeError> {
    let (w, h) = size;
    let luma_rows = h.next_multiple_of(TILE_ROWS);
    let (bo, modifier) = [I915_Y_TILED, DRM_FORMAT_MOD_LINEAR]
        .into_iter()
        .find_map(|modifier| {
            let bo = gl.gbm.create_buffer_object_with_modifiers2::<()>(
                w,
                luma_rows + h / 2,
                gbm::Format::R8,
                [gbm::Modifier::from(modifier)].into_iter(),
                gbm::BufferObjectFlags::RENDERING,
            );
            bo.ok().map(|bo| (bo, modifier))
        })
        .ok_or_else(|| EncodeError("no view buffer".into()))?;
    let fd = Arc::new(
        bo.fd()
            .map_err(|e| EncodeError(format!("view buffer: {e}")))?,
    );
    let stride = bo.stride();
    let objects = std::slice::from_ref(&fd);
    let texture = |offset, size: (u32, u32)| {
        let plane = [Plane {
            object: 0,
            offset,
            stride,
        }];
        let image = gl.import(objects, ABGR8888, modifier, &plane, size)?;
        // SAFETY: GL calls on the caller's current context.
        unsafe {
            let texture = gl.gl.create_texture().map_err(gl_err("texture"))?;
            gl.bind_image(texture, image);
            Ok::<_, EncodeError>(Texture {
                image: Some(image),
                texture,
            })
        }
    };
    Ok(View {
        luma: texture(0, (w / 4, h))?,
        chroma: texture(stride * luma_rows, (w / 4, h / 2))?,
        buffer: Some(ViewBuffer {
            bo,
            fd,
            luma_rows,
            modifier,
        }),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::encode::probe::RENDER_NODE;
    use crate::stream::XRGB8888;
    use std::fs::File;

    /// A LINEAR XRGB8888 dma-buf of `size` holding `pixel`'s colours, as a stream frame.
    pub(crate) fn dmabuf_frame(
        gbm: &gbm::Device<File>,
        size: (u32, u32),
        pixel: impl Fn(u32, u32) -> [u8; 3],
    ) -> (Frame, Vec<u8>, gbm::BufferObject<()>) {
        let (w, h) = size;
        let mut bo = gbm
            .create_buffer_object_with_modifiers2::<()>(
                w,
                h,
                gbm::Format::Xrgb8888,
                [gbm::Modifier::Linear].into_iter(),
                gbm::BufferObjectFlags::RENDERING,
            )
            .unwrap();
        let stride = bo.stride();
        let mut data = vec![0u8; (stride * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let [r, g, b] = pixel(x, y);
                let at = (y * stride + x * 4) as usize;
                data[at..at + 4].copy_from_slice(&[b, g, r, 255]);
            }
        }
        let row = (w * 4) as usize;
        bo.map_mut(0, 0, w, h, |m| {
            let s = m.stride() as usize;
            for y in 0..h as usize {
                m.buffer_mut()[y * s..][..row].copy_from_slice(&data[y * stride as usize..][..row]);
            }
        })
        .unwrap();
        let frame = Frame::unleased(
            FrameBuffer::Dmabuf {
                objects: vec![Arc::new(bo.fd().unwrap())],
                fourcc: XRGB8888,
                modifier: DRM_FORMAT_MOD_LINEAR,
                planes: vec![Plane {
                    object: 0,
                    offset: 0,
                    stride,
                }],
            },
            size,
        );
        (frame, data, bo)
    }

    /// A view read back from its dma-buf.
    fn read_back(view: &View, (w, h): (u32, u32)) -> avc444::Nv12 {
        let view = view.buffer.as_ref().expect("a dma-buf view");
        let rows = view.luma_rows + h / 2;
        let bytes = view
            .bo
            .map(0, 0, w, rows, |m| {
                let s = m.stride() as usize;
                (0..rows as usize)
                    .flat_map(|y| m.buffer()[y * s..][..w as usize].to_vec())
                    .collect::<Vec<u8>>()
            })
            .unwrap();
        let row = |y: u32| &bytes[(y * w) as usize..][..w as usize];
        avc444::Nv12 {
            y: (0..h).flat_map(row).copied().collect(),
            uv: (view.luma_rows..rows).flat_map(row).copied().collect(),
            width: w,
            height: h,
        }
    }

    #[test]
    #[ignore = "needs OpenGL ES on /dev/dri/renderD128; run with --ignored where it has one"]
    fn gpu_split_equals_cpu_split() {
        let size = (100, 50);
        let mut split = GpuSplit::new(Path::new(RENDER_NODE), size, size, Target::Dmabufs).unwrap();
        let gbm = gbm::Device::new(
            File::options()
                .read(true)
                .write(true)
                .open(RENDER_NODE)
                .unwrap(),
        )
        .unwrap();
        // Random colours, greys in the top right, a coloured 1-px column and row, and the
        // picture's edge cutting macroblocks.
        let mut seed = 0x9E37_79B9_7F4A_7C15_u64;
        let mut noise = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.to_le_bytes()
        };
        let random: Vec<[u8; 3]> = (0..100 * 50)
            .map(|_| {
                let n = noise();
                [n[0], n[1], n[2]]
            })
            .collect();
        let pixel = |x: u32, y: u32| match (x, y) {
            (37, _) => [255, 0, 255],
            (_, 21) => [0, 200, 0],
            (48.., ..32) => [(x * 3) as u8; 3],
            _ => random[(y * 100 + x) as usize],
        };
        let (frame, data, _bo) = dmabuf_frame(&gbm, size, pixel);
        let read = Rect {
            x: 16,
            y: 16,
            width: 48,
            height: 32,
        };
        let flat = split.draw(&frame, read).unwrap();
        let stride = data.len() / 50;
        let planes = Planes444::from_rgb(&data, stride, size, XRGB8888);
        let (main, mut aux) = avc444::split_v1(&planes);
        avc444::keep_main_outside(&main, &mut aux, read);
        assert_eq!(
            read_back(&split.views[0], avc444::view_size(size)),
            main,
            "the main view"
        );
        assert_eq!(
            read_back(&split.views[1], avc444::view_size(size)),
            aux,
            "the auxiliary view"
        );
        assert_eq!(flat, Flat::of(&planes));
        let grey: Vec<bool> = (0..28).map(|i| (3..7).contains(&i)).collect();
        assert_eq!(
            flat.blocks, grey,
            "only the grey macroblocks clear of the lines are flat"
        );
    }

    #[test]
    #[ignore = "needs OpenGL ES on /dev/dri/renderD128; run with --ignored where it has one"]
    fn gl_split_codes_a_half_size_target() {
        use crate::encode::ffmpeg::tests::{colour_pixel, patch_error, shown_444};
        let (source, half) = ((1920, 1080), (960, 540));
        let node = Path::new(RENDER_NODE);
        let mut split = GpuSplit::new(node, source, half, Target::Dmabufs).unwrap();
        let gbm = gbm::Device::new(File::options().read(true).write(true).open(node).unwrap());
        let (frame, _, _bo) = dmabuf_frame(&gbm.unwrap(), source, colour_pixel());
        split.draw(&frame, Rect::full(half)).unwrap();
        let view = avc444::view_size(half);
        assert_eq!(view, (960, 544));
        let (main, aux) = (
            read_back(&split.views[0], view),
            read_back(&split.views[1], view),
        );
        let back = avc444::tests::crop(&avc444::combine_v1(&main, &aux), half);
        let error = patch_error(shown_444(&back), 2);
        assert!(error <= 3, "a patch is off by {error}");
    }
}
