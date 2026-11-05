//! The AVC444 views as encoder input. The GPU draws them from a stream dma-buf with OpenGL ES
//! into two NV12 dma-bufs that VA-API maps; the software encoder's come from the CPU split.

use super::EncodeError;
use super::avc444::{self, Flat, Planes444};
use super::colour;
use super::hwmap::{self, Devices, Gpu, Ref};
use crate::stream::{DRM_FORMAT_MOD_LINEAR, Frame, FrameBuffer, Plane, Rect};
use ffmpeg::ffi;
use ffmpeg::format::Pixel;
use ffmpeg::frame;
use glow::HasContext;
use khronos_egl as egl;
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
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

/// Splits stream frames of one size and format.
pub struct Views {
    pub size: (u32, u32),
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
    Cpu {
        main: frame::Video,
        aux: frame::Video,
    },
}

impl Views {
    /// Views of pictures of `size` in DRM `fourcc`, drawn on `devices`' GPU (render node `node`)
    /// or, without devices, on the CPU. Only VA-API takes them from the GPU.
    pub fn new(
        devices: Option<&Devices>,
        node: &Path,
        fourcc: u32,
        size: (u32, u32),
    ) -> Result<Views, EncodeError> {
        let view = avc444::view_size(size);
        let route = match devices {
            Some(d) if d.gpu == Gpu::Vaapi => {
                let split = Box::new(GpuSplit::new(node, size)?);
                let mapped = hwmap::frames(
                    &d.device,
                    ffi::AVPixelFormat::VAAPI,
                    ffi::AVPixelFormat::NV12,
                    view,
                    0,
                )?;
                let map = |v: &View| {
                    let drm = hwmap::drm_frame(
                        std::slice::from_ref(&v.fd),
                        NV12,
                        v.modifier,
                        &v.planes(),
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
            Some(_) => {
                return Err(EncodeError("the views are drawn for VA-API only".into()));
            }
            None => Route::Cpu {
                main: frame::Video::new(Pixel::NV12, view.0, view.1),
                aux: frame::Video::new(Pixel::NV12, view.0, view.1),
            },
        };
        Ok(Views {
            size,
            fourcc,
            route,
        })
    }

    /// The frames context the encoder's input comes from; None on the CPU.
    pub fn frames(&self) -> Option<*mut ffi::AVBufferRef> {
        match &self.route {
            Route::Gpu { mapped, .. } => Some(mapped.as_ptr()),
            Route::Cpu { .. } => None,
        }
    }

    /// The encoder's pixel format.
    pub fn pixel(&self) -> Pixel {
        match &self.route {
            Route::Gpu { .. } => Pixel::VAAPI,
            Route::Cpu { .. } => Pixel::NV12,
        }
    }

    /// `frame`'s views, as new references to frames the next split overwrites. Clients read
    /// the auxiliary view inside `read` only; outside it repeats the main view.
    pub fn split(&mut self, frame: &Frame, read: Rect) -> Result<Split, EncodeError> {
        let flat = match &mut self.route {
            Route::Gpu { split, .. } => split.draw(frame, read)?,
            Route::Cpu { main, aux } => {
                for f in [&mut *main, &mut *aux] {
                    // SAFETY: a frame this route owns.
                    let writable = unsafe { ffi::av_frame_make_writable(f.as_mut_ptr()) };
                    hwmap::check(writable, "frame buffer")?;
                }
                let mut planes = None;
                hwmap::read_rgb(frame, self.size, |pixels, stride| {
                    planes = Some(Planes444::from_rgb(
                        pixels,
                        stride as usize,
                        self.size,
                        self.fourcc,
                    ));
                })?;
                let planes = planes.expect("read_rgb calls back on success");
                let (m, mut a) = avc444::split_v1(&planes);
                avc444::keep_main_outside(&m, &mut a, read);
                fill(main, &m);
                fill(aux, &a);
                tag(main);
                tag(aux);
                Flat::of(&planes)
            }
        };
        let (main, aux) = match &self.route {
            Route::Gpu { main, aux, .. } | Route::Cpu { main, aux } => (main, aux),
        };
        Ok(Split {
            main: reference(main)?,
            aux: reference(aux)?,
            flat,
        })
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

/// `glEGLImageTargetTexture2DOES`.
type ImageTarget = unsafe extern "system" fn(target: u32, image: *const libc::c_void);

const VERTEX: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID & 1) << 2), float((gl_VertexID & 2) << 1)) - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";
const FRAGMENT: &str = include_str!("split.frag");
/// Luma, chroma and the flat macroblocks; see split.frag.
const PASSES: usize = 3;

/// An NV12 view in one dma-buf, chroma below luma, and its planes as GL textures of four bytes
/// to a texel.
struct View {
    bo: gbm::BufferObject<()>,
    fd: Arc<OwnedFd>,
    /// The rows before the chroma plane: the luma rows, rounded up to whole tiles.
    luma_rows: u32,
    modifier: u64,
    luma: Texture,
    chroma: Texture,
}

impl View {
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

/// A texture on an image of a dma-buf, or of its own.
struct Texture {
    image: Option<egl::Image>,
    texture: glow::Texture,
}

/// One GL context on the render node that draws the views of pictures of one size.
struct GpuSplit {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    gl: glow::Context,
    image_target: ImageTarget,
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
    size: (u32, u32),
    _gbm: gbm::Device<File>,
}

const PLATFORM_GBM: egl::Enum = 0x31D7;
const LINUX_DMA_BUF: egl::Enum = 0x3270;
const LINUX_DRM_FOURCC: egl::Attrib = 0x3271;
/// Per plane: fd, offset, pitch, modifier low and high bits.
const PLANE_ATTRIBS: [[egl::Attrib; 5]; 4] = [
    [0x3272, 0x3273, 0x3274, 0x3443, 0x3444],
    [0x3275, 0x3276, 0x3277, 0x3445, 0x3446],
    [0x3278, 0x3279, 0x327A, 0x3447, 0x3448],
    [0x3440, 0x3441, 0x3442, 0x3449, 0x344A],
];

fn gl_err(what: &str) -> impl FnOnce(String) -> EncodeError + '_ {
    move |e| EncodeError(format!("{what}: {e}"))
}

fn egl_err(what: &str) -> impl FnOnce(egl::Error) -> EncodeError + '_ {
    move |e| EncodeError(format!("{what}: {e}"))
}

impl GpuSplit {
    fn new(node: &Path, size: (u32, u32)) -> Result<GpuSplit, EncodeError> {
        let file = File::options()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|e| EncodeError(format!("{}: {e}", node.display())))?;
        let gbm = gbm::Device::new(file).map_err(|e| EncodeError(format!("GBM: {e}")))?;
        // SAFETY: loading the system's libEGL, which follows the EGL API.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| EncodeError(format!("libEGL: {e}")))?;
        // SAFETY: a live GBM device, which outlives the display (both live in the GpuSplit).
        let display = unsafe {
            egl.get_platform_display(
                PLATFORM_GBM,
                gbm::AsRaw::as_raw(&gbm) as *mut _,
                &[egl::ATTRIB_NONE],
            )
        }
        .map_err(egl_err("EGL display"))?;
        egl.initialize(display).map_err(egl_err("EGL"))?;
        let context = (|| {
            egl.bind_api(egl::OPENGL_ES_API)?;
            let config = egl
                .choose_first_config(
                    display,
                    &[egl::RENDERABLE_TYPE, egl::OPENGL_ES3_BIT, egl::NONE],
                )?
                .ok_or(egl::Error::BadConfig)?;
            let context = egl.create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
            )?;
            egl.make_current(display, None, None, Some(context))?;
            Ok(context)
        })()
        .map_err(|e: egl::Error| {
            let _ = egl.terminate(display);
            EncodeError(format!("GL context: {e}"))
        })?;
        // SAFETY: the context is current on this thread; EGL hands out GL entry points.
        let gl = unsafe {
            glow::Context::from_loader_function_cstr(|name| {
                egl.get_proc_address(&name.to_string_lossy())
                    .map_or(std::ptr::null(), |f| f as *const libc::c_void)
            })
        };
        let Some(image_target) = egl
            .get_proc_address("glEGLImageTargetTexture2DOES")
            // SAFETY: the extension function's signature.
            .map(|f| unsafe { std::mem::transmute::<extern "system" fn(), ImageTarget>(f) })
        else {
            let _ = egl.make_current(display, None, None, None);
            let _ = egl.destroy_context(display, context);
            let _ = egl.terminate(display);
            return Err(EncodeError("no glEGLImageTargetTexture2DOES".into()));
        };
        let mut split = GpuSplit {
            egl,
            display,
            context,
            gl,
            image_target,
            programs: Vec::new(),
            read_rects: Vec::new(),
            vertices: None,
            views: Vec::new(),
            blocks: None,
            flags: None,
            framebuffers: Vec::new(),
            size,
            _gbm: gbm,
        };
        let (w, h) = avc444::view_size(size);
        let gl = &split.gl;
        // SAFETY: GL calls on the current context; drop deletes what was made if one fails.
        unsafe {
            for pass in 0..PASSES {
                let p = program(gl, pass, size)?;
                split.programs.push(p);
                split
                    .read_rects
                    .push(gl.get_uniform_location(p, "read_rect"));
            }
            split.vertices = Some(gl.create_vertex_array().map_err(gl_err("vertex array"))?);
            split.blocks = Some(own_texture(gl, (w / 4, h / 2))?);
            split.flags = Some(own_texture(gl, (w / MACROBLOCK, h / MACROBLOCK))?);
            for _ in 0..2 {
                let v = new_view(&split._gbm, &split.egl, display, gl, image_target, (w, h))?;
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
        let FrameBuffer::Dmabuf {
            objects,
            fourcc,
            modifier,
            planes,
        } = &frame.buffer
        else {
            return Err(EncodeError("a shared-memory frame cannot be mapped".into()));
        };
        if frame.size != self.size {
            return Err(EncodeError("the frame changed size".into()));
        }
        (self.egl)
            .make_current(self.display, None, None, Some(self.context))
            .map_err(egl_err("GL context"))?;
        let image = import(
            &self.egl,
            self.display,
            objects,
            *fourcc,
            *modifier,
            planes,
            frame.size,
        )?;
        let gl = &self.gl;
        let blocks = self.blocks.as_ref().expect("made with the views");
        // SAFETY: GL calls on the current context; the source texture and image are deleted
        // before returning.
        let flags = unsafe {
            let source = gl.create_texture().map_err(gl_err("texture"));
            let drawn = source.and_then(|source| {
                gl.bind_texture(glow::TEXTURE_2D, Some(source));
                (self.image_target)(glow::TEXTURE_2D, image.as_ptr());
                nearest(gl);
                gl.bind_vertex_array(self.vertices);
                let corners = [read.x, read.y, read.x + read.width, read.y + read.height];
                let [x0, y0, x1, y1] = corners.map(|v| v as i32);
                let passes = self.programs.iter().zip(&self.read_rects);
                for (pass, ((program, read_rect), (fb, (w, h)))) in
                    passes.zip(&self.framebuffers).enumerate()
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
                let (w, h) = self.framebuffers[PASSES - 1].1;
                let mut texels = vec![0u8; (w * h * 4) as usize];
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
                gl.delete_texture(source);
                match gl.get_error() {
                    glow::NO_ERROR => Ok(texels),
                    e => Err(EncodeError(format!("drawing the views: GL error {e:#x}"))),
                }
            });
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            drawn
        };
        let _ = self.egl.destroy_image(self.display, image);
        let texels = flags?;
        Ok(Flat {
            columns: avc444::view_size(self.size).0 / MACROBLOCK,
            blocks: (texels.as_chunks::<4>().0.iter())
                .map(|t| t[0] > 127)
                .collect(),
        })
    }
}

impl Drop for GpuSplit {
    fn drop(&mut self) {
        let _ = (self.egl).make_current(self.display, None, None, Some(self.context));
        let gl = &self.gl;
        // SAFETY: objects of this context, deleted once.
        unsafe {
            for (fb, _) in &self.framebuffers {
                gl.delete_framebuffer(*fb);
            }
            let views = self.views.iter().flat_map(|v| [&v.luma, &v.chroma]);
            for t in views.chain(&self.blocks).chain(&self.flags) {
                gl.delete_texture(t.texture);
                if let Some(image) = t.image {
                    let _ = self.egl.destroy_image(self.display, image);
                }
            }
            for &p in &self.programs {
                gl.delete_program(p);
            }
            if let Some(v) = self.vertices {
                gl.delete_vertex_array(v);
            }
        }
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// Pass `pass` of split.frag for pictures of `size`, its uniforms set.
unsafe fn program(
    gl: &glow::Context,
    pass: usize,
    size: (u32, u32),
) -> Result<glow::Program, EncodeError> {
    let fragment = format!("#version 300 es\n#define PASS {pass}\n{FRAGMENT}");
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        let program = gl.create_program().map_err(gl_err("program"))?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, &fragment),
        ] {
            let shader = gl.create_shader(kind).map_err(gl_err("shader"))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(EncodeError(format!("pass {pass} does not compile: {log}")));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(EncodeError(format!("pass {pass} does not link: {log}")));
        }
        gl.use_program(Some(program));
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

/// An NV12 view of `size` and its luma and chroma planes as GL textures.
unsafe fn new_view(
    gbm: &gbm::Device<File>,
    egl: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    gl: &glow::Context,
    image_target: ImageTarget,
    size: (u32, u32),
) -> Result<View, EncodeError> {
    let (w, h) = size;
    let luma_rows = h.next_multiple_of(TILE_ROWS);
    let (bo, modifier) = [I915_Y_TILED, DRM_FORMAT_MOD_LINEAR]
        .into_iter()
        .find_map(|modifier| {
            let bo = gbm.create_buffer_object_with_modifiers2::<()>(
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
        let image = import(egl, display, objects, ABGR8888, modifier, &plane, size)?;
        // SAFETY: GL calls on the caller's current context.
        unsafe {
            let texture = gl.create_texture().map_err(gl_err("texture"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            image_target(glow::TEXTURE_2D, image.as_ptr());
            Ok::<_, EncodeError>(Texture {
                image: Some(image),
                texture,
            })
        }
    };
    Ok(View {
        luma: texture(0, (w / 4, h))?,
        chroma: texture(stride * luma_rows, (w / 4, h / 2))?,
        bo,
        fd,
        luma_rows,
        modifier,
    })
}

/// A dma-buf as an EGL image.
fn import(
    egl: &egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    objects: &[Arc<OwnedFd>],
    fourcc: u32,
    modifier: u64,
    planes: &[Plane],
    (w, h): (u32, u32),
) -> Result<egl::Image, EncodeError> {
    if planes.is_empty() || planes.len() > PLANE_ATTRIBS.len() {
        return Err(EncodeError(format!(
            "{} planes do not import",
            planes.len()
        )));
    }
    let mut attribs = vec![
        egl::WIDTH as egl::Attrib,
        w as egl::Attrib,
        egl::HEIGHT as egl::Attrib,
        h as egl::Attrib,
        LINUX_DRM_FOURCC,
        fourcc as egl::Attrib,
    ];
    for (plane, [fd, offset, pitch, lo, hi]) in planes.iter().zip(PLANE_ATTRIBS) {
        let object = objects
            .get(plane.object)
            .ok_or_else(|| EncodeError("a plane lies outside the frame's buffers".into()))?;
        attribs.extend([
            fd,
            object.as_raw_fd() as egl::Attrib,
            offset,
            plane.offset as egl::Attrib,
            pitch,
            plane.stride as egl::Attrib,
            lo,
            (modifier & 0xFFFF_FFFF) as egl::Attrib,
            hi,
            (modifier >> 32) as egl::Attrib,
        ]);
    }
    attribs.push(egl::ATTRIB_NONE);
    // SAFETY: EGL_NO_CONTEXT and a null buffer are what a dma-buf import takes.
    let (no_context, no_buffer) = unsafe {
        (
            egl::Context::from_ptr(egl::NO_CONTEXT),
            egl::ClientBuffer::from_ptr(std::ptr::null_mut()),
        )
    };
    egl.create_image(display, no_context, LINUX_DMA_BUF, no_buffer, &attribs)
        .map_err(|e| EncodeError(format!("dma-buf import (modifier {modifier:#x}): {e}")))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::encode::probe::RENDER_NODE;
    use crate::stream::XRGB8888;

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
        let mut split = GpuSplit::new(Path::new(RENDER_NODE), size).unwrap();
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
}
