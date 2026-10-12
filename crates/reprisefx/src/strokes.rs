//! Flat bars at any angle in one batch, edges anti-aliased: the Starfield's stars and Mystify's lines.
//! Compile once per renderer ([`StrokeFx::new`]); per frame and output, [`Strokes::element`].

use std::cell::Cell;
use std::ffi::CString;

use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{GlesError, GlesFrame, GlesRenderer, ffi};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale};

/// A bar from `from` to `to`, `width` wide, squared off half a width past each end, in physical
/// pixels from the element's corner. `color` is premultiplied RGBA.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub from: [f32; 2],
    pub to: [f32; 2],
    pub width: f32,
    pub color: [f32; 4],
}

impl Stroke {
    /// A square `side` wide around `at`, its side and edges moved to whole pixels so it draws with
    /// no partly covered pixels.
    pub fn dot(at: [f32; 2], side: f32, color: [f32; 4]) -> Stroke {
        let side = side.round().max(1.0);
        let corner = |c: f32| (c - side / 2.0).round() + side / 2.0;
        let at = [corner(at[0]), corner(at[1])];
        Stroke {
            from: at,
            to: at,
            width: side,
            color,
        }
    }
}

/// Floats per vertex: position, the bar's own (along, across) position, its half length and half
/// width, colour.
const FLOATS: usize = 10;

const VERT: &str = r#"#version 100
uniform mat3 matrix;
uniform vec2 origin;
attribute vec2 position;
attribute vec4 local;
attribute vec4 color;
varying vec4 v_local;
varying vec4 v_color;
void main() {
    v_local = local;
    v_color = color;
    gl_Position = vec4(matrix * vec3(position + origin, 1.0), 1.0);
}
"#;

const FRAG: &str = r#"#version 100
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
uniform float alpha;
varying vec4 v_local;
varying vec4 v_color;
void main() {
    vec2 cover = clamp(v_local.zw + 0.5 - abs(v_local.xy), 0.0, 1.0);
    gl_FragColor = v_color * (cover.x * cover.y * alpha);
}
"#;

/// Two triangles per stroke, [`FLOATS`] per vertex, grown half a pixel past the bar so its edges
/// can be covered partly.
pub fn vertices(strokes: &[Stroke]) -> Vec<f32> {
    let mut out = Vec::with_capacity(strokes.len() * 6 * FLOATS);
    for s in strokes {
        let (dx, dy) = (s.to[0] - s.from[0], s.to[1] - s.from[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let (ax, ay) = if len > 0.0 {
            (dx / len, dy / len)
        } else {
            (1.0, 0.0)
        };
        let (nx, ny) = (-ay, ax);
        let half_w = s.width / 2.0;
        let half_l = len / 2.0 + half_w;
        let (cx, cy) = ((s.from[0] + s.to[0]) / 2.0, (s.from[1] + s.to[1]) / 2.0);
        let (u, v) = (half_l + 0.5, half_w + 0.5);
        let corner = |su: f32, sv: f32| {
            let (lu, lv) = (su * u, sv * v);
            [
                cx + ax * lu + nx * lv,
                cy + ay * lu + ny * lv,
                lu,
                lv,
                half_l,
                half_w,
                s.color[0],
                s.color[1],
                s.color[2],
                s.color[3],
            ]
        };
        for (su, sv) in [
            (-1.0, -1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (-1.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
        ] {
            out.extend_from_slice(&corner(su, sv));
        }
    }
    out
}

/// The stroke program and where its inputs are, for one renderer.
#[derive(Clone, Copy, Debug)]
pub struct StrokeFx {
    program: u32,
    matrix: i32,
    origin: i32,
    alpha: i32,
    position: u32,
    local: u32,
    color: u32,
}

impl StrokeFx {
    pub fn new(renderer: &mut GlesRenderer) -> Result<StrokeFx, GlesError> {
        renderer
            .with_context(link)?
            .ok_or(GlesError::ProgramLinkError)
    }
}

/// Compile and link the stroke program on `gl`, whose context `with_context` made current.
fn link(gl: &ffi::Gles2) -> Option<StrokeFx> {
    let shader = |kind: ffi::types::GLenum, src: &str| {
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
            log::warn!("stroke shader did not compile");
            // SAFETY: as above.
            unsafe { gl.DeleteShader(id) };
            return None;
        }
        Some(id)
    };
    let vert = shader(ffi::VERTEX_SHADER, VERT)?;
    let frag = shader(ffi::FRAGMENT_SHADER, FRAG);
    let mut ok = 0;
    // SAFETY: the context is current; only the new shaders and program are touched, and the
    // shaders are deleted once linked (or not).
    let program = unsafe {
        let program = gl.CreateProgram();
        gl.AttachShader(program, vert);
        if let Some(frag) = frag {
            gl.AttachShader(program, frag);
            gl.LinkProgram(program);
            gl.DetachShader(program, frag);
            gl.DeleteShader(frag);
            gl.GetProgramiv(program, ffi::LINK_STATUS, &mut ok);
        }
        gl.DetachShader(program, vert);
        gl.DeleteShader(vert);
        program
    };
    let name = |n: &str| CString::new(n).expect("no nul");
    // SAFETY: the context is current and `program` is the one just made.
    let (position, local, color) = unsafe {
        (
            gl.GetAttribLocation(program, name("position").as_ptr()),
            gl.GetAttribLocation(program, name("local").as_ptr()),
            gl.GetAttribLocation(program, name("color").as_ptr()),
        )
    };
    if ok == 0 || position < 0 || local < 0 || color < 0 {
        log::warn!("stroke program did not link");
        // SAFETY: as above.
        unsafe { gl.DeleteProgram(program) };
        return None;
    }
    // SAFETY: as above.
    let uniform = |n: &str| unsafe { gl.GetUniformLocation(program, name(n).as_ptr()) };
    Some(StrokeFx {
        program,
        matrix: uniform("matrix"),
        origin: uniform("origin"),
        alpha: uniform("alpha"),
        position: position as u32,
        local: local as u32,
        color: color as u32,
    })
}

/// One drawing's element Id and commit; after [`Strokes::changed`] its elements damage their whole
/// area.
#[derive(Debug)]
pub struct Strokes {
    id: Id,
    commit: Cell<CommitCounter>,
}

impl Default for Strokes {
    fn default() -> Strokes {
        Strokes {
            id: Id::new(),
            commit: Cell::new(CommitCounter::default()),
        }
    }
}

impl Strokes {
    /// Mark the strokes as changed: the next elements damage their whole area.
    pub fn changed(&self) {
        let mut c = self.commit.get();
        c.increment();
        self.commit.set(c);
    }

    /// `strokes` over `geometry` (physical, on the output) at `alpha`.
    pub fn element(
        &self,
        fx: &StrokeFx,
        geometry: Rectangle<i32, Physical>,
        strokes: &[Stroke],
        alpha: f32,
    ) -> StrokeElement {
        StrokeElement {
            id: self.id.clone(),
            commit: self.commit.get(),
            fx: *fx,
            geometry,
            vertices: vertices(strokes),
            alpha,
        }
    }
}

/// The strokes for one frame; see [`Strokes::element`].
#[derive(Debug)]
pub struct StrokeElement {
    id: Id,
    commit: CommitCounter,
    fx: StrokeFx,
    geometry: Rectangle<i32, Physical>,
    vertices: Vec<f32>,
    alpha: f32,
}

impl Element for StrokeElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        let s = self.geometry.size;
        Rectangle::from_size((f64::from(s.w), f64::from(s.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(
        &self,
        _scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            return DamageSet::default();
        }
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry.size)])
    }

    fn opaque_regions(&self, _scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::default()
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for StrokeElement {
    /// Draws every stroke whatever the damage: outside it, the framebuffer already holds the same
    /// strokes, since any change of them damages the whole element.
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        _src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        if damage.is_empty() || self.vertices.is_empty() {
            return Ok(());
        }
        let matrix = *frame.projection();
        let fx = self.fx;
        let v = &self.vertices;
        let count = (v.len() / FLOATS) as i32;
        let stride = (FLOATS * std::mem::size_of::<f32>()) as i32;
        let (x, y) = (dst.loc.x as f32, dst.loc.y as f32);
        let alpha = self.alpha;
        // SAFETY: the frame's context is current; the attribute arrays point into `v`, which
        // outlives the draw, and are disabled again before returning. The renderer sets its own
        // program, attributes and divisors before each of its draws.
        frame.with_context(|gl| unsafe {
            gl.UseProgram(fx.program);
            gl.UniformMatrix3fv(fx.matrix, 1, ffi::FALSE, matrix.as_ptr());
            gl.Uniform2f(fx.origin, x, y);
            gl.Uniform1f(fx.alpha, alpha);
            gl.Enable(ffi::BLEND);
            gl.BlendFunc(ffi::ONE, ffi::ONE_MINUS_SRC_ALPHA);
            gl.BindBuffer(ffi::ARRAY_BUFFER, 0);
            let divisors = gl.VertexAttribDivisor.is_loaded();
            for (loc, size, offset) in [(fx.position, 2, 0), (fx.local, 4, 2), (fx.color, 4, 6)] {
                gl.EnableVertexAttribArray(loc);
                if divisors {
                    gl.VertexAttribDivisor(loc, 0);
                }
                gl.VertexAttribPointer(
                    loc,
                    size,
                    ffi::FLOAT,
                    ffi::FALSE,
                    stride,
                    v.as_ptr().add(offset) as *const _,
                );
            }
            gl.DrawArrays(ffi::TRIANGLES, 0, count);
            for loc in [fx.position, fx.local, fx.color] {
                gl.DisableVertexAttribArray(loc);
            }
        })
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corners(v: &[f32]) -> Vec<[f32; 2]> {
        v.chunks(FLOATS).map(|c| [c[0], c[1]]).collect()
    }

    #[test]
    fn a_dot_is_a_square_grown_half_a_pixel() {
        let v = vertices(&[Stroke {
            from: [10.0, 20.0],
            to: [10.0, 20.0],
            width: 2.0,
            color: [1.0; 4],
        }]);
        assert_eq!(v.len(), 6 * FLOATS);
        let c = corners(&v);
        let xs: Vec<f32> = c.iter().map(|p| p[0]).collect();
        let ys: Vec<f32> = c.iter().map(|p| p[1]).collect();
        let span = |a: &[f32]| {
            (
                a.iter().cloned().fold(f32::MAX, f32::min),
                a.iter().cloned().fold(f32::MIN, f32::max),
            )
        };
        assert_eq!(span(&xs), (8.5, 11.5), "1 each side, plus half a pixel");
        assert_eq!(span(&ys), (18.5, 21.5));
        assert_eq!(&v[4..6], &[1.0, 1.0], "half length and half width");
    }

    #[test]
    fn a_dot_lands_on_whole_pixels() {
        let d = Stroke::dot([10.3, 20.6], 2.4, [1.0; 4]);
        assert_eq!((d.from, d.to, d.width), ([10.0, 21.0], [10.0, 21.0], 2.0));
        let d = Stroke::dot([10.3, 20.6], 3.0, [1.0; 4]);
        assert_eq!(d.from, [10.5, 20.5], "an odd side centres on a pixel");
        assert_eq!(Stroke::dot([0.0, 0.0], 0.3, [1.0; 4]).width, 1.0);
    }

    #[test]
    fn a_bar_runs_along_its_ends_with_square_caps() {
        let v = vertices(&[Stroke {
            from: [0.0, 0.0],
            to: [0.0, 10.0],
            width: 1.0,
            color: [0.5, 0.0, 0.0, 0.5],
        }]);
        let c = corners(&v);
        for p in &c {
            assert!(
                (p[0].abs() - 1.0).abs() < 1e-6,
                "across: 0.5 + 0.5, got {p:?}"
            );
            assert!(
                p[1] == -1.0 || p[1] == 11.0,
                "along: cap 0.5 + 0.5, got {p:?}"
            );
        }
        let first = &v[..FLOATS];
        assert_eq!(&first[4..6], &[5.5, 0.5]);
        assert_eq!(&first[6..], &[0.5, 0.0, 0.0, 0.5], "colour as given");
    }
}
