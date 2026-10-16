//! The fluid styles' stable-fluids sim: velocity and pressure at output / `SIM_DIV`, dye at
//! output / `DYE_DIV`, half-float; each look shades the dye for [`FluidElement`] to draw.

use std::cell::Cell;

use reprisekit::fluid::{DYE_DIV, Force, Look, PRESSURE_ITERS, Profile, SIM_DIV};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, ffi,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size, Transform};

use crate::flip;
use crate::fluid_gl::{self, Blend, Prog, Tex, Value};
use crate::fluid_shaders::{self, Programs};

/// The dye grid's format: four half floats, signed.
const FORMAT: Fourcc = Fourcc::Abgr16161616f;

/// The sim's programs and a framebuffer for its passes, once per renderer. Needs GLES 3 and
/// half-float render targets.
#[derive(Clone, Debug)]
pub struct FluidFx {
    pub(crate) programs: Programs,
    fbo: u32,
}

impl FluidFx {
    pub fn new(r: &mut GlesRenderer) -> Result<FluidFx, GlesError> {
        // GlesError::GLVersionNotSupported takes a type Smithay keeps private.
        if !r.with_context(flip::gles3)? {
            return Err(GlesError::GLExtensionNotSupported(&["OpenGL ES 3.0"]));
        }
        let fbo = r.with_context(fluid_gl::framebuffer)?;
        for internal in [ffi::RGBA16F, ffi::RG16F, ffi::R16F] {
            let scratch = grid(r, (1, 1).into(), ffi::NEAREST, internal)?;
            if !r.with_context(|gl| fluid_gl::renders(gl, fbo, tex(&scratch)))? {
                return Err(GlesError::GLExtensionNotSupported(&[
                    "GL_EXT_color_buffer_half_float",
                ]));
            }
        }
        Ok(FluidFx {
            programs: fluid_shaders::compile(r)?,
            fbo,
        })
    }

    /// Runs `f`'s passes on the renderer's context, then sends them to the GPU.
    fn run<T>(&self, r: &mut GlesRenderer, f: impl FnOnce(&Run<'_>) -> T) -> Result<T, GlesError> {
        r.with_context(|gl| {
            let out = f(&Run {
                gl,
                fbo: self.fbo,
                p: &self.programs,
            });
            fluid_gl::done(gl, self.fbo);
            out
        })
    }
}

/// One output's fluid textures: half floats in only the channels they use, clamped at the edges;
/// pairs hold the current state at index 0. Pressure, divergence and curl filter NEAREST.
#[derive(Debug)]
pub(crate) struct Grids {
    pub(crate) output: Size<i32, Physical>,
    /// Sim grid; velocity in sim cells per second.
    pub(crate) velocity: [GlesTexture; 2],
    /// Sim grid; pressure, kept as the next step's first guess.
    pub(crate) pressure: [GlesTexture; 2],
    /// Sim grid; R.
    pub(crate) divergence: GlesTexture,
    /// Sim grid; R.
    pub(crate) curl: GlesTexture,
    /// Dye grid; RGB dye, A glitter points (seeded by splats, faded like the dye).
    pub(crate) dye: [GlesTexture; 2],
    /// Dye grid, LINEAR; the look's light in RGB and its cover in A (see `DRAW`).
    display: GlesTexture,
    /// Dye grid, LINEAR; Neon's sparks before they streak.
    sparks: GlesTexture,
    /// LINEAR ping-pong pairs at a quarter and an eighth of the dye grid.
    bloom: [[GlesTexture; 2]; 2],
}

/// How the display texture was last shaded.
#[derive(Clone, Copy, Debug, Default)]
struct Shown {
    /// RGB multiplies the backdrop instead of adding to it.
    light: bool,
    bloom: bool,
}

/// Neon's tile side, logical px.
const TILE: f32 = 24.0;
/// Display light past which it blooms.
const BLOOM_THRESHOLD: f32 = 0.35;
/// Each bloom level's weight, times `Look::bloom`.
const BLOOM_LEVELS: [f32; 2] = [1.0, 1.5];

/// One output's fluid.
#[derive(Debug)]
pub struct FluidPool {
    pub(crate) grids: Option<Grids>,
    commit: Cell<CommitCounter>,
    pressure_iters: u32,
    /// Seconds of velocity and dye fade not yet applied.
    unfaded: [f32; 2],
    /// The last step's output scale.
    scale: f32,
    steps: u32,
    shown: Shown,
}

impl Default for FluidPool {
    fn default() -> FluidPool {
        FluidPool::new()
    }
}

impl FluidPool {
    pub fn new() -> FluidPool {
        FluidPool {
            grids: None,
            commit: Cell::default(),
            pressure_iters: PRESSURE_ITERS,
            unfaded: [0.0; 2],
            scale: 1.0,
            steps: 0,
            shown: Shown::default(),
        }
    }

    pub fn commit(&self) -> CommitCounter {
        self.commit.get()
    }

    /// The sim grid's size. For tests.
    #[doc(hidden)]
    pub fn sim_size(&self) -> Option<Size<i32, Buffer>> {
        self.grids.as_ref().map(|g| g.velocity[0].size())
    }

    /// The dye grid's size. For tests.
    #[doc(hidden)]
    pub fn dye_size(&self) -> Option<Size<i32, Buffer>> {
        self.grids.as_ref().map(|g| g.dye[0].size())
    }

    /// Sets the Jacobi iterations per step. For tests.
    #[doc(hidden)]
    pub fn set_pressure_iters(&mut self, n: u32) {
        self.pressure_iters = n;
    }

    /// Makes empty grids for an output of `output` physical px; true when it did.
    pub fn fit(
        &mut self,
        r: &mut GlesRenderer,
        fx: &FluidFx,
        output: Size<i32, Physical>,
    ) -> Result<bool, GlesError> {
        if self.grids.as_ref().is_some_and(|g| g.output == output) {
            return Ok(false);
        }
        let sim = grid_size(output, SIM_DIV);
        let dye = grid_size(output, DYE_DIV);
        let mut g = |size, filter, internal| grid(r, size, filter, internal);
        let (n, l) = (ffi::NEAREST, ffi::LINEAR);
        let (rgba, rg, red) = (ffi::RGBA16F, ffi::RG16F, ffi::R16F);
        let near = grid_size(output, DYE_DIV * 4);
        let far = grid_size(output, DYE_DIV * 8);
        self.grids = Some(Grids {
            output,
            velocity: [g(sim, l, rg)?, g(sim, l, rg)?],
            pressure: [g(sim, n, red)?, g(sim, n, red)?],
            divergence: g(sim, n, red)?,
            curl: g(sim, n, red)?,
            dye: [g(dye, l, rgba)?, g(dye, l, rgba)?],
            display: g(dye, l, rgba)?,
            sparks: g(dye, l, rgba)?,
            bloom: [
                [g(near, l, rgba)?, g(near, l, rgba)?],
                [g(far, l, rgba)?, g(far, l, rgba)?],
            ],
        });
        self.clear(r, fx)?;
        Ok(true)
    }

    pub fn clear(&mut self, r: &mut GlesRenderer, fx: &FluidFx) -> Result<(), GlesError> {
        let Some(g) = self.grids.as_ref() else {
            return Ok(());
        };
        fx.run(r, |run| {
            for t in g
                .velocity
                .iter()
                .chain(&g.pressure)
                .chain(&g.dye)
                .chain(g.bloom.iter().flatten())
                .chain([&g.divergence, &g.curl, &g.display, &g.sparks])
            {
                fluid_gl::zero(run.gl, run.fbo, tex(t));
            }
        })?;
        self.shown = Shown::default();
        self.bump();
        Ok(())
    }

    /// One step of `dt` seconds with `forces` from an output at `scale`.
    pub fn step(
        &mut self,
        r: &mut GlesRenderer,
        fx: &FluidFx,
        forces: &[Force],
        scale: f64,
        look: &Look,
        dt: f32,
    ) -> Result<(), GlesError> {
        if self.grids.is_none() {
            return Ok(());
        }
        let forces = match forces.iter().rposition(|f| matches!(f, Force::Clear)) {
            Some(i) => {
                self.clear(r, fx)?;
                &forces[i + 1..]
            }
            None => forces,
        };
        let iters = self.pressure_iters;
        let [v, d] = &mut self.unfaded;
        let decay = [
            fade(v, dt, look.velocity_half_life),
            fade(d, dt, look.dye_half_life),
        ];
        self.scale = scale as f32;
        self.steps = self.steps.wrapping_add(1);
        let seed = (self.steps % 4096) as f32;
        let g = self.grids.as_mut().expect("checked above");
        fx.run(r, |run| {
            g.splat(run, forces, scale as f32, look, seed);
            g.project(run, look.vorticity, dt, iters);
            g.advect(run, decay, dt);
        })?;
        self.bump();
        Ok(())
    }

    /// Shades the dye as `profile` into the display texture, and blooms it;
    /// `time_s` turns Glow's rim colours.
    pub fn shade(
        &mut self,
        r: &mut GlesRenderer,
        fx: &FluidFx,
        profile: Profile,
        flavour: usize,
        time_s: f32,
    ) -> Result<(), GlesError> {
        let Some(g) = self.grids.as_mut() else {
            return Ok(());
        };
        let p = &fx.programs;
        let program = match profile {
            Profile::Ink => &p.ink,
            Profile::Glow => &p.glow,
            Profile::Sparks => &p.sparks,
            Profile::Neon => &p.neon,
        };
        let light = profile == Profile::Ink && flavour == 0;
        let u = [
            ("size", size(&g.dye[0])),
            ("velocity", Value::Int(1)),
            ("sim", Value::Vec2(g.sim())),
            ("time", Value::Float(time_s)),
            ("tile", Value::Float(TILE * self.scale / DYE_DIV as f32)),
            ("light", Value::Float(if light { 1.0 } else { 0.0 })),
            ("sparks", Value::Int(2)),
        ];
        let bloom = !light && profile.look().bloom > 0.0;
        fx.run(r, |run| {
            if profile == Profile::Neon {
                run.pass(&p.sparkle, &g.dye[0], &[], &g.sparks, &u);
            }
            run.pass(
                program,
                &g.dye[0],
                &[&g.velocity[0], &g.sparks],
                &g.display,
                &u,
            );
            if bloom {
                g.bloom(run);
            }
        })?;
        self.shown = Shown { light, bloom };
        self.bump();
        Ok(())
    }

    /// The element drawing `wallpaper` over `geometry` with the fluid as last
    /// shaded; `level` (`reprisekit::fluid::level`) fades the fluid and the dim.
    #[allow(clippy::too_many_arguments)]
    pub fn element(
        &self,
        fx: &FluidFx,
        id: &Id,
        wallpaper: &GlesTexture,
        extra: CommitCounter,
        geometry: Rectangle<i32, Physical>,
        look: &Look,
        flavour: usize,
        level: f32,
    ) -> Option<FluidElement> {
        let g = self.grids.as_ref()?;
        let count = |c: CommitCounter| c.distance(Some(CommitCounter::default())).unwrap_or(0);
        let shown = self.shown;
        let bloom = if shown.bloom { look.bloom } else { 0.0 };
        Some(FluidElement {
            id: id.clone(),
            commit: CommitCounter::from(count(self.commit()) + count(extra)),
            program: fx.programs.draw.clone(),
            wallpaper: wallpaper.clone(),
            display: g.display.clone(),
            bloom: [g.bloom[0][0].clone(), g.bloom[1][0].clone()],
            geometry,
            backdrop: look.backdrop[flavour.min(look.backdrop.len() - 1)],
            dim: look.dim * level,
            level,
            light: shown.light,
            bloom_weights: BLOOM_LEVELS.map(|w| w * bloom),
        })
    }

    /// The dye grid, row by row from the top. For tests.
    #[doc(hidden)]
    pub fn dye(&self, r: &mut GlesRenderer) -> Result<Vec<[f32; 4]>, GlesError> {
        match &self.grids {
            Some(g) => read(r, &g.dye[0]),
            None => Ok(Vec::new()),
        }
    }

    /// The divergence of the current velocity, sim grid row by row. For tests.
    #[doc(hidden)]
    pub fn divergence(&self, r: &mut GlesRenderer, fx: &FluidFx) -> Result<Vec<f32>, GlesError> {
        let Some(g) = &self.grids else {
            return Ok(Vec::new());
        };
        fx.run(r, |run| {
            let u = [("size", Value::Vec2(g.sim()))];
            run.pass(&run.p.divergence, &g.velocity[0], &[], &g.divergence, &u);
        })?;
        Ok(read(r, &g.divergence)?.into_iter().map(|t| t[0]).collect())
    }

    fn bump(&self) {
        let mut c = self.commit.get();
        c.increment();
        self.commit.set(c);
    }
}

impl Grids {
    fn sim(&self) -> [f32; 2] {
        let s = self.velocity[0].size();
        [s.w as f32, s.h as f32]
    }

    fn splat(&mut self, run: &Run<'_>, forces: &[Force], scale: f32, look: &Look, seed: f32) {
        let shorter = self.output.w.min(self.output.h) as f32;
        let per_cell = scale / SIM_DIV as f32;
        let mut m = vec![[0.0f32; 16]; forces.len()];
        let mut reach = Vec::with_capacity(forces.len());
        for (slot, f) in m.iter_mut().zip(forces) {
            let columns = match *f {
                Force::Splat {
                    from,
                    at,
                    vel,
                    radius,
                    colour,
                    amount,
                } => {
                    let r = radius * look.splat_radius * shorter;
                    let [cr, cg, cb] = colour.map(|c| c * amount);
                    let (a, b) = (
                        (from.0 * scale, from.1 * scale),
                        (at.0 * scale, at.1 * scale),
                    );
                    reach.push(Reach::Splat(
                        a.0.min(b.0),
                        a.1.min(b.1),
                        a.0.max(b.0),
                        a.1.max(b.1),
                        r,
                    ));
                    [
                        [b.0, b.1, r, 0.0],
                        [vel.0 * per_cell, vel.1 * per_cell, 0.0, 0.0],
                        [cr, cg, cb, 0.0],
                        [a.0, a.1, 0.0, 0.0],
                    ]
                }
                Force::Swirl {
                    centre,
                    radius,
                    spin,
                } => {
                    let (c, r) = ((centre.0 * scale, centre.1 * scale), radius * scale);
                    reach.push(Reach::Swirl(c.0 - r, c.1 - r, c.0 + r, c.1 + r));
                    [
                        [c.0, c.1, r, 1.0],
                        [0.0, 0.0, spin, 0.0],
                        [0.0; 4],
                        [0.0; 4],
                    ]
                }
                Force::Clear => unreachable!("a step applies only the forces after the last Clear"),
            };
            *slot = std::array::from_fn(|i| columns[i / 4][i % 4]);
        }
        // One blended draw per force over the texels it reaches; overlaps add up as they land.
        let uniforms = |i: usize, t: &GlesTexture, cell: i32, dye: bool| {
            [
                ("size", size(t)),
                ("force", Value::Mat4s(&m[i..=i])),
                ("cell", Value::Float(cell as f32)),
                ("dye", Value::Float(if dye { 1.0 } else { 0.0 })),
                ("seed", Value::Float(seed)),
                ("glitter", Value::Float(look.glitter)),
            ]
        };
        for (i, reach) in reach.iter().enumerate() {
            let grids = [
                (&self.velocity, SIM_DIV, false, Blend::Pull),
                (&self.dye, DYE_DIV, true, Blend::Add),
            ];
            for (pair, cell, dye, blend) in grids {
                if let Some(r) = reach.texels(pair[0].size(), cell, dye) {
                    let u = uniforms(i, &pair[0], cell, dye);
                    run.blend(&run.p.splat, pair, &u, r, blend);
                }
            }
        }
    }

    fn project(&mut self, run: &Run<'_>, vorticity: f32, dt: f32, iters: u32) {
        let p = run.p;
        let size = ("size", Value::Vec2(self.sim()));
        run.pass(&p.curl, &self.velocity[0], &[], &self.curl, &[size]);
        let size = || ("size", Value::Vec2(self.sim()));
        let u = [
            size(),
            ("curl", Value::Int(1)),
            ("strength", Value::Float(vorticity)),
            ("dt", Value::Float(dt)),
        ];
        let curl = self.curl.clone();
        let sim = self.sim();
        run.swap(&p.vorticity, &mut self.velocity, &[&curl], &u);
        let size = || ("size", Value::Vec2(sim));
        run.pass(
            &p.divergence,
            &self.velocity[0],
            &[],
            &self.divergence,
            &[size()],
        );
        let u = [size(), ("divergence", Value::Int(1))];
        for _ in 0..iters {
            run.swap(&p.jacobi, &mut self.pressure, &[&self.divergence], &u);
        }
        let u = [size(), ("pressure", Value::Int(1))];
        run.swap(&p.gradient, &mut self.velocity, &[&self.pressure[0]], &u);
    }

    /// Blooms the display texture into `bloom[0][0]` and `bloom[1][0]`.
    fn bloom(&mut self, run: &Run<'_>) {
        let p = run.p;
        let down = |from: &GlesTexture, threshold: f32| {
            [("size", size(from)), ("threshold", Value::Float(threshold))]
        };
        run.pass(
            &p.down,
            &self.display,
            &[],
            &self.bloom[0][0],
            &down(&self.display, BLOOM_THRESHOLD),
        );
        blur(run, &mut self.bloom[0]);
        let [near, far] = &mut self.bloom;
        run.pass(&p.down, &near[0], &[], &far[0], &down(&near[0], 0.0));
        blur(run, far);
    }

    fn advect(&mut self, run: &Run<'_>, [v, d]: [f32; 2], dt: f32) {
        let sim = self.sim();
        let uniforms = |t: &GlesTexture, decay: [f32; 4]| {
            [
                ("size", size(t)),
                ("velocity", Value::Int(1)),
                ("sim", Value::Vec2(sim)),
                ("dt", Value::Float(dt)),
                ("decay", Value::Vec4(decay)),
            ]
        };
        let velocity = self.velocity[0].clone();
        let u = uniforms(&velocity, [v, v, 1.0, 1.0]);
        run.swap(&run.p.advect, &mut self.velocity, &[&velocity], &u);
        // The dye rides the velocity it started the step with, as the velocity did.
        let u = uniforms(&self.dye[0], [d; 4]);
        run.swap(&run.p.advect_dye, &mut self.dye, &[&velocity], &u);
    }
}

/// `size`: a texture's texels.
fn size(t: &GlesTexture) -> Value<'static> {
    let s = t.size();
    Value::Vec2([s.w as f32, s.h as f32])
}

fn tex(t: &GlesTexture) -> Tex {
    let s = t.size();
    Tex {
        id: t.tex_id(),
        w: s.w,
        h: s.h,
    }
}

/// A run of passes on the renderer's context, through `fbo`.
struct Run<'a> {
    gl: &'a ffi::Gles2,
    fbo: u32,
    p: &'a Programs,
}

impl Run<'_> {
    /// Draws all of `from` through `prog` over all of `to`, `extra` on units 1, 2, ...
    fn pass(
        &self,
        prog: &Prog,
        from: &GlesTexture,
        extra: &[&GlesTexture],
        to: &GlesTexture,
        uniforms: &[(&str, Value<'_>)],
    ) {
        let extra: Vec<Tex> = extra.iter().map(|t| tex(t)).collect();
        let to = tex(to);
        fluid_gl::draw(
            self.gl,
            self.fbo,
            prog,
            tex(from),
            &extra,
            to,
            uniforms,
            &[to.full()],
            Blend::Replace,
        );
    }

    /// As `pass`, from `pair[0]` into `pair[1]`, then swaps them.
    fn swap(
        &self,
        prog: &Prog,
        pair: &mut [GlesTexture; 2],
        extra: &[&GlesTexture],
        uniforms: &[(&str, Value<'_>)],
    ) {
        self.pass(prog, &pair[0], extra, &pair[1], uniforms);
        pair.swap(0, 1);
    }

    /// Draws `rect` of `pair[0]` through `prog`, blended onto what it holds.
    fn blend(
        &self,
        prog: &Prog,
        pair: &[GlesTexture; 2],
        uniforms: &[(&str, Value<'_>)],
        rect: Rectangle<i32, Physical>,
        blend: Blend,
    ) {
        // `tex` is the other buffer: the target is never bound to a unit while drawn.
        let (to, other) = (tex(&pair[0]), tex(&pair[1]));
        fluid_gl::draw(
            self.gl,
            self.fbo,
            prog,
            other,
            &[],
            to,
            uniforms,
            &[rect],
            blend,
        );
    }
}

/// Physical px a force can change: a splat's segment box and radius, a swirl's box.
enum Reach {
    Splat(f32, f32, f32, f32, f32),
    Swirl(f32, f32, f32, f32),
}

/// Splats past this many radii add under 1/8000 of their peak.
const SPLAT_REACH: f32 = 3.0;

impl Reach {
    /// The texels of a grid of `size` texels, `cell` px each, the force changes; swirls change
    /// no dye.
    fn texels(
        &self,
        size: Size<i32, Buffer>,
        cell: i32,
        dye: bool,
    ) -> Option<Rectangle<i32, Physical>> {
        let cell = cell as f32;
        let (x0, y0, x1, y1) = match *self {
            Reach::Splat(x0, y0, x1, y1, r) => {
                let m = SPLAT_REACH * r.max(cell);
                (x0 - m, y0 - m, x1 + m, y1 + m)
            }
            Reach::Swirl(..) if dye => return None,
            Reach::Swirl(x0, y0, x1, y1) => (x0, y0, x1, y1),
        };
        let x0 = ((x0 / cell).floor() as i32).clamp(0, size.w);
        let y0 = ((y0 / cell).floor() as i32).clamp(0, size.h);
        let x1 = ((x1 / cell).ceil() as i32 + 1).clamp(0, size.w);
        let y1 = ((y1 / cell).ceil() as i32 + 1).clamp(0, size.h);
        (x1 > x0 && y1 > y0).then(|| Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()))
    }
}

/// Blurs `pair[0]` across, into `pair[1]`, then down, back into `pair[0]`.
fn blur(run: &Run<'_>, pair: &mut [GlesTexture; 2]) {
    let size = size(&pair[0]);
    let [a, b] = pair;
    let s = |dir: [f32; 2]| [("size", size), ("dir", Value::Vec2(dir))];
    run.pass(&run.p.blur, a, &[], b, &s([1.0, 0.0]));
    run.pass(&run.p.blur, b, &[], a, &s([0.0, 1.0]));
}

/// The least fade a step applies. Storing a half float rounds by up to an ulp (up to 1/1024
/// of it, either way per device), so smaller fades stall or run fast; this keeps that error small.
const LEAST_FADE: f32 = 1.0 / 128.0;

/// The factor to fade by this step: 1 until the fade of the `unfaded` seconds
/// plus `dt` reaches [`LEAST_FADE`], then all of it at once.
fn fade(unfaded: &mut f32, dt: f32, half_life: f32) -> f32 {
    *unfaded += dt;
    let f = 0.5f32.powf(*unfaded / half_life);
    if 1.0 - f < LEAST_FADE {
        return 1.0;
    }
    *unfaded = 0.0;
    f
}

fn grid_size(output: Size<i32, Physical>, div: i32) -> Size<i32, Buffer> {
    (
        (output.w.max(1) as u32).div_ceil(div as u32) as i32,
        (output.h.max(1) as u32).div_ceil(div as u32) as i32,
    )
        .into()
}

/// A texture of `internal` texels (a sized GL format) clamped at the edges, `filter` NEAREST or
/// LINEAR.
fn grid(
    r: &mut GlesRenderer,
    size: Size<i32, Buffer>,
    filter: ffi::types::GLenum,
    internal: ffi::types::GLenum,
) -> Result<GlesTexture, GlesError> {
    let id = r.with_context(|gl| {
        let mut id = 0;
        // SAFETY: the context is current inside `with_context`; the texture is new and the
        // binding goes back to none.
        unsafe {
            gl.GenTextures(1, &mut id);
            gl.BindTexture(ffi::TEXTURE_2D, id);
            gl.TexStorage2D(ffi::TEXTURE_2D, 1, internal, size.w, size.h);
            for (k, v) in [
                (ffi::TEXTURE_WRAP_S, ffi::CLAMP_TO_EDGE),
                (ffi::TEXTURE_WRAP_T, ffi::CLAMP_TO_EDGE),
                (ffi::TEXTURE_MIN_FILTER, filter),
                (ffi::TEXTURE_MAG_FILTER, filter),
            ] {
                gl.TexParameteri(ffi::TEXTURE_2D, k, v as i32);
            }
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }
        id
    })?;
    // SAFETY: `id` is a texture of the renderer's context, made above and owned from here on.
    Ok(unsafe { GlesTexture::from_raw(r, Some(internal), false, id, size) })
}

/// Binds `ids` to texture units 1, 2, ... and makes unit 0 active again.
fn units(gl: &ffi::Gles2, ids: &[u32]) {
    for (i, &id) in ids.iter().enumerate() {
        // SAFETY: the frame's context is current; units 1.. hold a pass's
        // extra inputs for its draw only and go back to none after it.
        unsafe {
            gl.ActiveTexture(ffi::TEXTURE1 + i as u32);
            gl.BindTexture(ffi::TEXTURE_2D, id);
        }
    }
    // SAFETY: as above; the frame draws `tex` from unit 0.
    unsafe { gl.ActiveTexture(ffi::TEXTURE0) };
}

/// Reads `t` back as floats by raw GL: Smithay's `map_texture` maps 4 bytes
/// a texel whatever the format.
fn read(r: &mut GlesRenderer, t: &GlesTexture) -> Result<Vec<[f32; 4]>, GlesError> {
    let size = t.size();
    let id = t.tex_id();
    let mut out = vec![[0.0f32; 4]; (size.w * size.h) as usize];
    let status = r.with_context(|gl| {
        // SAFETY: the context is current and `id` a live texture of it; the FBO is ours
        // and deleted, `out` holds `size` RGBA floats, the binding goes back to none.
        unsafe {
            gl.GetError();
            let mut fbo = 0;
            gl.GenFramebuffers(1, &mut fbo);
            gl.BindFramebuffer(ffi::FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                id,
                0,
            );
            let status = gl.CheckFramebufferStatus(ffi::FRAMEBUFFER);
            if status == ffi::FRAMEBUFFER_COMPLETE {
                gl.ReadPixels(
                    0,
                    0,
                    size.w,
                    size.h,
                    ffi::RGBA,
                    ffi::FLOAT,
                    out.as_mut_ptr().cast(),
                );
            }
            gl.BindFramebuffer(ffi::FRAMEBUFFER, 0);
            gl.DeleteFramebuffers(1, &fbo);
            status == ffi::FRAMEBUFFER_COMPLETE && gl.GetError() == ffi::NO_ERROR
        }
    })?;
    if status {
        Ok(out)
    } else {
        Err(GlesError::UnsupportedPixelFormat(FORMAT))
    }
}

/// The wallpaper with one output's fluid over it.
#[derive(Debug)]
pub struct FluidElement {
    id: Id,
    commit: CommitCounter,
    program: GlesTexProgram,
    wallpaper: GlesTexture,
    display: GlesTexture,
    bloom: [GlesTexture; 2],
    geometry: Rectangle<i32, Physical>,
    backdrop: [f32; 3],
    dim: f32,
    level: f32,
    light: bool,
    bloom_weights: [f32; 2],
}

impl Element for FluidElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        Rectangle::from_size(self.wallpaper.size().to_f64())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        if commit == Some(self.commit) {
            DamageSet::default()
        } else {
            DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        }
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        OpaqueRegions::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for FluidElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let ids = [
            self.display.tex_id(),
            self.bloom[0].tex_id(),
            self.bloom[1].tex_id(),
        ];
        frame.with_context(|gl| units(gl, &ids))?;
        let uniforms = [
            Uniform::new("display", 1i32),
            Uniform::new("bloom0", 2i32),
            Uniform::new("bloom1", 3i32),
            Uniform::new("backdrop", self.backdrop),
            Uniform::new("dim", self.dim),
            Uniform::new("level", self.level),
            Uniform::new("light", if self.light { 1.0f32 } else { 0.0 }),
            Uniform::new("bloom", self.bloom_weights),
        ];
        let drawn = frame.render_texture_from_to(
            &self.wallpaper,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            1.0,
            Some(&self.program),
            &uniforms,
        );
        frame.with_context(|gl| units(gl, &[0; 3]))?;
        drawn
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_at_144_hz_keeps_the_half_life_in_steps_half_floats_can_hold() {
        let (dt, half_life) = (1.0 / 144.0, 20.0);
        let mut unfaded = 0.0;
        let mut total = 1.0f64;
        for _ in 0..600 {
            let f = fade(&mut unfaded, dt, half_life);
            assert!(
                f == 1.0 || 1.0 - f >= LEAST_FADE,
                "a fade of {f} rounds away"
            );
            total *= f64::from(f);
        }
        let want = 0.5f64.powf(600.0 / 144.0 / 20.0);
        assert!(
            (total - want).abs() < 0.002,
            "{total} after 600 steps, want {want}"
        );
    }
}
