//! An OpenGL ES 3 context on a render node through EGL on GBM, and dma-bufs as EGL images.

use super::EncodeError;
use crate::stream::Plane;
use glow::HasContext;
use khronos_egl as egl;
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;

/// `glEGLImageTargetTexture2DOES`.
type ImageTarget = unsafe extern "system" fn(target: u32, image: *const libc::c_void);

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

/// A vertex shader covering the viewport with one triangle, for passes of a fragment shader.
const VERTEX: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID & 1) << 2), float((gl_VertexID & 2) << 1)) - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";

pub fn gl_err(what: &str) -> impl FnOnce(String) -> EncodeError + '_ {
    move |e| EncodeError(format!("{what}: {e}"))
}

pub fn egl_err(what: &str) -> impl FnOnce(egl::Error) -> EncodeError + '_ {
    move |e| EncodeError(format!("{what}: {e}"))
}

/// A GL context, current on the thread that made it until another is.
pub struct Gl {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    context: egl::Context,
    pub gl: glow::Context,
    image_target: ImageTarget,
    pub gbm: gbm::Device<File>,
}

impl Gl {
    /// A context on render node `node`.
    pub fn new(node: &Path) -> Result<Gl, EncodeError> {
        let file = File::options()
            .read(true)
            .write(true)
            .open(node)
            .map_err(|e| EncodeError(format!("{}: {e}", node.display())))?;
        let gbm = gbm::Device::new(file).map_err(|e| EncodeError(format!("GBM: {e}")))?;
        // SAFETY: loading the system's libEGL, which follows the EGL API.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| EncodeError(format!("libEGL: {e}")))?;
        // SAFETY: a live GBM device, which outlives the display (both live in the Gl).
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
        Ok(Gl {
            egl,
            display,
            context,
            gl,
            image_target,
            gbm,
        })
    }

    /// Makes the context current on this thread.
    pub fn current(&self) -> Result<(), EncodeError> {
        (self.egl)
            .make_current(self.display, None, None, Some(self.context))
            .map_err(egl_err("GL context"))
    }

    /// Binds `image` to `texture` as its storage.
    ///
    /// # Safety
    /// The context is current and `texture` is one of its textures.
    pub unsafe fn bind_image(&self, texture: glow::Texture, image: egl::Image) {
        // SAFETY: GL calls on the caller's current context with a live image.
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            (self.image_target)(glow::TEXTURE_2D, image.as_ptr());
        }
    }

    /// A dma-buf as an EGL image.
    pub fn import(
        &self,
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
        (self.egl)
            .create_image(self.display, no_context, LINUX_DMA_BUF, no_buffer, &attribs)
            .map_err(|e| EncodeError(format!("dma-buf import (modifier {modifier:#x}): {e}")))
    }

    pub fn destroy_image(&self, image: egl::Image) {
        let _ = self.egl.destroy_image(self.display, image);
    }
}

impl Drop for Gl {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}

/// A program of [`VERTEX`] and `fragment`, linked and in use.
///
/// # Safety
/// `gl`'s context is current.
pub unsafe fn program(gl: &glow::Context, fragment: &str) -> Result<glow::Program, EncodeError> {
    // SAFETY: GL calls on the caller's current context.
    unsafe {
        let program = gl.create_program().map_err(gl_err("program"))?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = gl.create_shader(kind).map_err(gl_err("shader"))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(EncodeError(format!("a shader does not compile: {log}")));
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
            return Err(EncodeError(format!("a program does not link: {log}")));
        }
        gl.use_program(Some(program));
        Ok(program)
    }
}
