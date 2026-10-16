//! The fluid's passes on raw GL: one framebuffer pointed at each pass's target, drawn back to back
//! on the renderer's context with no frame, fence or flush between them.

use std::ffi::CString;
use std::rc::Rc;

use smithay::backend::renderer::gles::{GlesError, ffi};
use smithay::utils::{Physical, Rectangle};

const VERT: &str = r#"#version 100
attribute vec2 position;
varying vec2 v_coords;
void main() {
    v_coords = position;
    gl_Position = vec4(position * 2.0 - 1.0, 0.0, 1.0);
}
"#;

/// A linked pass program, its `position` attribute and its uniforms' locations.
#[derive(Clone, Debug)]
pub(crate) struct Prog {
    id: u32,
    position: u32,
    uniforms: Rc<[(String, i32)]>,
}

impl Prog {
    /// `name`'s location; -1, which GL ignores, for one the program lacks.
    fn at(&self, name: &str) -> i32 {
        self.uniforms
            .iter()
            .find(|(n, _)| n == name)
            .map_or(-1, |&(_, at)| at)
    }
}

/// A uniform's name as GL lists it, without an array's `[0]`.
fn uniform_name(listed: &str) -> &str {
    listed.strip_suffix("[0]").unwrap_or(listed)
}

/// A uniform's value; samplers are `Int` texture units.
#[derive(Clone, Copy)]
pub(crate) enum Value<'a> {
    Float(f32),
    Vec2([f32; 2]),
    Vec4([f32; 4]),
    Int(i32),
    Mat4s(&'a [[f32; 16]]),
}

/// Links `frag` with the pass vertex shader, which gives `v_coords` in the target's 0..1.
pub(crate) fn link(gl: &ffi::Gles2, frag: &str) -> Result<Prog, GlesError> {
    let vert = shader(gl, ffi::VERTEX_SHADER, VERT)?;
    let frag = match shader(gl, ffi::FRAGMENT_SHADER, frag) {
        Ok(f) => f,
        Err(e) => {
            // SAFETY: the context is current and `vert` a shader of it.
            unsafe { gl.DeleteShader(vert) };
            return Err(e);
        }
    };
    let mut ok = 0;
    let position = CString::new("position").expect("no nul");
    // SAFETY: the context is current; only the new shaders and program are touched, and the
    // shaders are deleted once linked.
    let (id, position) = unsafe {
        let id = gl.CreateProgram();
        gl.AttachShader(id, vert);
        gl.AttachShader(id, frag);
        gl.LinkProgram(id);
        gl.DetachShader(id, vert);
        gl.DetachShader(id, frag);
        gl.DeleteShader(vert);
        gl.DeleteShader(frag);
        gl.GetProgramiv(id, ffi::LINK_STATUS, &mut ok);
        (id, gl.GetAttribLocation(id, position.as_ptr()))
    };
    if ok == 0 || position < 0 {
        // SAFETY: as above.
        unsafe { gl.DeleteProgram(id) };
        return Err(GlesError::ProgramLinkError);
    }
    Ok(Prog {
        id,
        position: position as u32,
        uniforms: uniforms(gl, id).into(),
    })
}

/// Every active uniform of program `id` and its location.
fn uniforms(gl: &ffi::Gles2, id: u32) -> Vec<(String, i32)> {
    let mut count = 0;
    // SAFETY: the context is current and `id` a linked program of it.
    unsafe { gl.GetProgramiv(id, ffi::ACTIVE_UNIFORMS, &mut count) };
    (0..count.max(0) as u32)
        .map(|i| {
            let mut buf = [0u8; 64];
            let (mut len, mut size, mut kind) = (0, 0, 0);
            // SAFETY: as above; `buf` holds 64 bytes and GL writes at most that, NUL included.
            unsafe {
                gl.GetActiveUniform(
                    id,
                    i,
                    64,
                    &mut len,
                    &mut size,
                    &mut kind,
                    buf.as_mut_ptr().cast(),
                )
            };
            let listed = String::from_utf8_lossy(&buf[..len.max(0) as usize]).into_owned();
            let name = uniform_name(&listed).to_string();
            let c = CString::new(name.as_str()).expect("no nul");
            // SAFETY: as above.
            (name, unsafe { gl.GetUniformLocation(id, c.as_ptr()) })
        })
        .collect()
}

fn shader(gl: &ffi::Gles2, kind: ffi::types::GLenum, src: &str) -> Result<u32, GlesError> {
    let src = CString::new(src).expect("no nul");
    let mut ok = 0;
    // SAFETY: the context is current; the shader is new and `src` outlives the call.
    let id = unsafe {
        let id = gl.CreateShader(kind);
        gl.ShaderSource(id, 1, &src.as_ptr(), std::ptr::null());
        gl.CompileShader(id);
        gl.GetShaderiv(id, ffi::COMPILE_STATUS, &mut ok);
        id
    };
    if ok == 0 {
        // SAFETY: as above.
        unsafe { gl.DeleteShader(id) };
        return Err(GlesError::ShaderCompileError);
    }
    Ok(id)
}

/// A framebuffer for the passes, made once per renderer.
pub(crate) fn framebuffer(gl: &ffi::Gles2) -> u32 {
    let mut fbo = 0;
    // SAFETY: the context is current.
    unsafe { gl.GenFramebuffers(1, &mut fbo) };
    fbo
}

/// One texture of a pass: its GL name and texels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tex {
    pub(crate) id: u32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

impl Tex {
    pub(crate) fn full(self) -> Rectangle<i32, Physical> {
        Rectangle::from_size((self.w, self.h).into())
    }
}

/// Points `fbo` at `to` for drawing.
fn target(gl: &ffi::Gles2, fbo: u32, to: Tex) {
    // SAFETY: the context is current; `fbo` and `to` are live objects of it.
    unsafe {
        gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);
        gl.FramebufferTexture2D(
            ffi::FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            to.id,
            0,
        );
        gl.Viewport(0, 0, to.w, to.h);
    }
}

/// How a draw's output meets what `to` holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Blend {
    Replace,
    /// RGBA added.
    Add,
    /// RGB times one minus the output's alpha, plus the output's RGB; alpha kept.
    Pull,
}

/// Draws `rects` of `to` through `prog`, reading `from` as `tex` on unit 0 and `extra` on units
/// 1, 2, ...
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    gl: &ffi::Gles2,
    fbo: u32,
    prog: &Prog,
    from: Tex,
    extra: &[Tex],
    to: Tex,
    uniforms: &[(&str, Value<'_>)],
    rects: &[Rectangle<i32, Physical>],
    blend: Blend,
) {
    let (w, h) = (to.w as f32, to.h as f32);
    let mut v = Vec::with_capacity(rects.len() * 12);
    for r in rects {
        let (x0, y0) = (r.loc.x as f32 / w, r.loc.y as f32 / h);
        let (x1, y1) = (
            (r.loc.x + r.size.w) as f32 / w,
            (r.loc.y + r.size.h) as f32 / h,
        );
        v.extend_from_slice(&[x0, y0, x1, y0, x0, y1, x0, y1, x1, y0, x1, y1]);
    }
    target(gl, fbo, to);
    // SAFETY: the context is current and every name live in it; the attribute array points into
    // `v` for the draw only. The renderer sets its own program, attributes and target per draw.
    unsafe {
        match blend {
            Blend::Replace => gl.Disable(ffi::BLEND),
            Blend::Add => {
                gl.Enable(ffi::BLEND);
                gl.BlendFunc(ffi::ONE, ffi::ONE);
            }
            Blend::Pull => {
                gl.Enable(ffi::BLEND);
                gl.BlendFuncSeparate(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA, ffi::ZERO, ffi::ONE);
            }
        }
        gl.Disable(ffi::SCISSOR_TEST);
        gl.UseProgram(prog.id);
        for (unit, t) in std::iter::once(from)
            .chain(extra.iter().copied())
            .enumerate()
        {
            gl.ActiveTexture(ffi::TEXTURE0 + unit as u32);
            gl.BindTexture(ffi::TEXTURE_2D, t.id);
        }
        set(gl, prog, "tex", &Value::Int(0));
        for (name, value) in uniforms {
            set(gl, prog, name, value);
        }
        gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
        gl.EnableVertexAttribArray(prog.position);
        if gl.VertexAttribDivisor.is_loaded() {
            gl.VertexAttribDivisor(prog.position, 0);
        }
        gl.VertexAttribPointer(
            prog.position,
            2,
            ffi::FLOAT,
            ffi::FALSE,
            0,
            v.as_ptr().cast(),
        );
        gl.DrawArrays(ffi::TRIANGLES, 0, (v.len() / 2) as i32);
        gl.DisableVertexAttribArray(prog.position);
        gl.Disable(ffi::BLEND);
        for unit in (0..=extra.len()).rev() {
            gl.ActiveTexture(ffi::TEXTURE0 + unit as u32);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }
    }
}

/// # Safety
/// The context is current and `prog` in use.
unsafe fn set(gl: &ffi::Gles2, prog: &Prog, name: &str, value: &Value<'_>) {
    let at = prog.at(name);
    // SAFETY: as the caller promises; GL ignores location -1.
    unsafe {
        match value {
            Value::Float(f) => gl.Uniform1f(at, *f),
            Value::Vec2(v) => gl.Uniform2f(at, v[0], v[1]),
            Value::Vec4(v) => gl.Uniform4f(at, v[0], v[1], v[2], v[3]),
            Value::Int(i) => gl.Uniform1i(at, *i),
            Value::Mat4s(m) => {
                gl.UniformMatrix4fv(at, m.len() as i32, ffi::FALSE, m.as_ptr().cast())
            }
        }
    }
}

/// True when `fbo` can draw to `to`.
pub(crate) fn renders(gl: &ffi::Gles2, fbo: u32, to: Tex) -> bool {
    target(gl, fbo, to);
    // SAFETY: the context is current and `fbo` bound.
    let ok = unsafe { gl.CheckFramebufferStatus(ffi::FRAMEBUFFER) } == ffi::FRAMEBUFFER_COMPLETE;
    release(gl, fbo);
    ok
}

/// Detaches `fbo`'s texture, so a dropped one is freed, and binds the default framebuffer.
fn release(gl: &ffi::Gles2, fbo: u32) {
    // SAFETY: the context is current and `fbo` is a live framebuffer of it.
    unsafe {
        gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);
        gl.FramebufferTexture2D(
            ffi::FRAMEBUFFER,
            ffi::COLOR_ATTACHMENT0,
            ffi::TEXTURE_2D,
            0,
            0,
        );
        gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
    }
}

/// Clears `to` to zero.
pub(crate) fn zero(gl: &ffi::Gles2, fbo: u32, to: Tex) {
    target(gl, fbo, to);
    // SAFETY: the context is current and `fbo` draws to `to`.
    unsafe {
        gl.Disable(ffi::SCISSOR_TEST);
        gl.ClearColor(0.0, 0.0, 0.0, 0.0);
        gl.Clear(ffi::COLOR_BUFFER_BIT);
    }
}

/// Ends a run of passes on `fbo`: released (`release`) and the work sent to the GPU.
pub(crate) fn done(gl: &ffi::Gles2, fbo: u32) {
    release(gl, fbo);
    // SAFETY: the context is current.
    unsafe {
        gl.UseProgram(0);
        gl.Flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_array_uniform_is_found_by_its_plain_name() {
        assert_eq!(uniform_name("forces[0]"), "forces");
        assert_eq!(uniform_name("size"), "size");
        let prog = Prog {
            id: 0,
            position: 0,
            uniforms: vec![("size".to_string(), 3), ("force".to_string(), 5)].into(),
        };
        assert_eq!(
            (prog.at("force"), prog.at("size"), prog.at("tile")),
            (5, 3, -1)
        );
    }
}
