//! Desktop water on the GPU: a quarter-size height field in two ping-pong RGBA8
//! textures, a normals texture, and the element drawing the wallpaper through
//! them. A texel holds the height in R/G (high/low byte) and the previous one in
//! B/A, so no float render targets are needed. Passes draw opaque so blending
//! never touches A. The model is `reprisekit::water`.

use std::cell::Cell;

use reprisekit::water::{SPLASHES_PER_STEP, Splash, Step};
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, ffi,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::backend::renderer::{
    Bind, ExportMem, Frame, ImportMem, Renderer, Texture, TextureFilter,
};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Size, Transform};

use crate::{blur, shaders};

const _: () = assert!(SPLASHES_PER_STEP == 4);

pub const REST: [u8; 4] = [128, 0, 128, 0];

#[derive(Clone, Debug)]
pub struct WaterFx {
    pub(crate) step: GlesTexProgram,
    pub(crate) normals: GlesTexProgram,
    pub(crate) draw: GlesTexProgram,
    rest: GlesTexture,
}

impl WaterFx {
    pub fn new(renderer: &mut GlesRenderer) -> Result<WaterFx, GlesError> {
        let (step, normals, draw) = shaders::compile_water(renderer)?;
        let rest = renderer.import_memory(&REST, Fourcc::Abgr8888, (1, 1).into(), false)?;
        Ok(WaterFx {
            step,
            normals,
            draw,
            rest,
        })
    }
}

#[derive(Debug, Default)]
pub struct Pool {
    front: Option<GlesTexture>,
    back: Option<GlesTexture>,
    normals: Option<GlesTexture>,
    commit: Cell<CommitCounter>,
    steps: u32,
}

impl Pool {
    pub fn new() -> Pool {
        Pool::default()
    }

    pub fn size(&self) -> Option<Size<i32, Buffer>> {
        self.front.as_ref().map(|t| t.size())
    }

    pub fn commit(&self) -> CommitCounter {
        self.commit.get()
    }

    pub fn fit(
        &mut self,
        r: &mut GlesRenderer,
        fx: &WaterFx,
        size: Size<i32, Buffer>,
    ) -> Result<bool, GlesError> {
        if self.size() == Some(size) {
            return Ok(false);
        }
        let mut front = None;
        let mut back = None;
        let mut normals = None;
        blur::fit(&mut front, r, size)?;
        blur::fit(&mut back, r, size)?;
        let n = blur::fit(&mut normals, r, size)?;
        linear(r, &n)?;
        self.front = front;
        self.back = back;
        self.normals = normals;
        self.flatten(r, fx)?;
        Ok(true)
    }

    pub fn flatten(&mut self, r: &mut GlesRenderer, fx: &WaterFx) -> Result<(), GlesError> {
        let Some(size) = self.size() else {
            return Ok(());
        };
        let full = Rectangle::from_size((size.w, size.h).into());
        for t in [&mut self.front, &mut self.back].into_iter().flatten() {
            fill(r, &fx.rest, t, &[full])?;
        }
        self.shade(r, fx, 1.0)
    }

    pub fn step(
        &mut self,
        r: &mut GlesRenderer,
        fx: &WaterFx,
        splashes: &[Splash],
        obstacles: &[Rectangle<i32, Physical>],
        step: Step,
    ) -> Result<(), GlesError> {
        let (Some(front), Some(back)) = (self.front.clone(), self.back.as_mut()) else {
            return Ok(());
        };
        let size = front.size();
        let mut seg = [[0.0f32; 4]; SPLASHES_PER_STEP];
        let mut radius = [1.0f32; 4];
        let mut depth = [0.0f32; 4];
        for (i, s) in splashes.iter().take(SPLASHES_PER_STEP).enumerate() {
            seg[i] = [s.from.0, s.from.1, s.to.0, s.to.1];
            radius[i] = s.radius.max(0.5);
            depth[i] = s.depth;
        }
        let uniforms = [
            Uniform::new("size", [size.w as f32, size.h as f32]),
            Uniform::new("damping", step.damping),
            Uniform::new("c2", step.c2),
            Uniform::new("seed", (self.steps % 4096) as f32),
            Uniform::new("seg0", seg[0]),
            Uniform::new("seg1", seg[1]),
            Uniform::new("seg2", seg[2]),
            Uniform::new("seg3", seg[3]),
            Uniform::new("radius", radius),
            Uniform::new("depth", depth),
        ];
        let src = Rectangle::from_size(size.to_f64());
        let full = Rectangle::from_size((size.w, size.h).into());
        nearest(r, |r| {
            blur::pass_from(r, &front, src, back, Some(&fx.step), &uniforms, &[full])
        })?;
        let inside: Vec<_> = obstacles
            .iter()
            .filter_map(|o| o.intersection(full))
            .collect();
        if !inside.is_empty() {
            fill(r, &fx.rest, back, &inside)?;
        }
        std::mem::swap(&mut self.front, &mut self.back);
        self.steps = self.steps.wrapping_add(1);
        Ok(())
    }

    pub fn shade(
        &mut self,
        r: &mut GlesRenderer,
        fx: &WaterFx,
        gain: f32,
    ) -> Result<(), GlesError> {
        let (Some(front), Some(normals)) = (self.front.clone(), self.normals.as_mut()) else {
            return Ok(());
        };
        let size = front.size();
        let uniforms = [
            Uniform::new("size", [size.w as f32, size.h as f32]),
            Uniform::new("gain", gain),
        ];
        let src = Rectangle::from_size(size.to_f64());
        let full = Rectangle::from_size((size.w, size.h).into());
        nearest(r, |r| {
            blur::pass_from(
                r,
                &front,
                src,
                normals,
                Some(&fx.normals),
                &uniforms,
                &[full],
            )
        })?;
        let mut c = self.commit.get();
        c.increment();
        self.commit.set(c);
        Ok(())
    }

    pub(crate) fn normals(&self) -> Option<&GlesTexture> {
        self.normals.as_ref()
    }

    pub fn heights(&self, r: &mut GlesRenderer) -> Result<Vec<f32>, GlesError> {
        let Some(front) = self.front.clone() else {
            return Ok(Vec::new());
        };
        let size = front.size();
        let mut t = front;
        let fb = r.bind(&mut t)?;
        let mapping = r.copy_framebuffer(&fb, Rectangle::from_size(size), Fourcc::Abgr8888)?;
        drop(fb);
        let bytes = r.map_texture(&mapping)?;
        Ok(bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                let u = u32::from(p[0]) * 256 + u32::from(p[1]);
                if u == 32768 {
                    0.0
                } else {
                    u as f32 / 65535.0 * 2.0 - 1.0
                }
            })
            .collect())
    }
}

/// Give `t` LINEAR min and mag filters. Smithay never sets the normals' filters
/// and GL's default min filter leaves a texture without mipmaps incomplete.
fn linear(r: &mut GlesRenderer, t: &GlesTexture) -> Result<(), GlesError> {
    let id = t.tex_id();
    r.with_context(|gl| {
        // SAFETY: the context is current inside `with_context`, and `id`
        // is a live 2D texture of it; the binding is put back to 0.
        unsafe {
            gl.BindTexture(ffi::TEXTURE_2D, id);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
            gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }
    })
}

/// Run `f` with the renderer sampling NEAREST, then restore LINEAR; the packed
/// bytes must not be interpolated. Smithay has no getter to read the filter back.
fn nearest<T>(
    r: &mut GlesRenderer,
    f: impl FnOnce(&mut GlesRenderer) -> Result<T, GlesError>,
) -> Result<T, GlesError> {
    r.downscale_filter(TextureFilter::Nearest)?;
    r.upscale_filter(TextureFilter::Nearest)?;
    let out = f(r);
    r.downscale_filter(TextureFilter::Linear)?;
    r.upscale_filter(TextureFilter::Linear)?;
    out
}

fn fill(
    r: &mut GlesRenderer,
    rest: &GlesTexture,
    to: &mut GlesTexture,
    parts: &[Rectangle<i32, Physical>],
) -> Result<(), GlesError> {
    let size = to.size();
    let mut target = r.bind(to)?;
    let mut frame = r.render(&mut target, (size.w, size.h).into(), Transform::Normal)?;
    let src = Rectangle::from_size((1.0, 1.0).into());
    for p in parts {
        let local = [Rectangle::from_size(p.size)];
        frame.render_texture_from_to(
            rest,
            src,
            *p,
            &local,
            &local,
            Transform::Normal,
            1.0,
            None,
            &[],
        )?;
    }
    let _ = frame.finish()?;
    Ok(())
}

impl Pool {
    #[allow(clippy::too_many_arguments)]
    pub fn element(
        &self,
        fx: &WaterFx,
        id: &Id,
        wallpaper: &GlesTexture,
        extra: CommitCounter,
        geometry: Rectangle<i32, Physical>,
        refraction: f32,
        shine: f32,
        alpha: f32,
    ) -> Option<WaterElement> {
        let normals = self.normals()?.clone();
        let count = |c: CommitCounter| c.distance(Some(CommitCounter::default())).unwrap_or(0);
        let commit = CommitCounter::from(count(self.commit()) + count(extra));
        let size = geometry.size;
        Some(WaterElement {
            id: id.clone(),
            commit,
            program: fx.draw.clone(),
            wallpaper: wallpaper.clone(),
            normals,
            geometry,
            refraction: [
                refraction / size.w.max(1) as f32,
                refraction / size.h.max(1) as f32,
            ],
            shine,
            alpha,
        })
    }
}

#[derive(Debug)]
pub struct WaterElement {
    id: Id,
    commit: CommitCounter,
    program: GlesTexProgram,
    wallpaper: GlesTexture,
    normals: GlesTexture,
    geometry: Rectangle<i32, Physical>,
    refraction: [f32; 2],
    shine: f32,
    alpha: f32,
}

impl Element for WaterElement {
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
        if self.alpha >= 1.0 {
            OpaqueRegions::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
        } else {
            OpaqueRegions::default()
        }
    }

    fn alpha(&self) -> f32 {
        self.alpha
    }

    fn kind(&self) -> Kind {
        Kind::Unspecified
    }
}

impl RenderElement<GlesRenderer> for WaterElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let normals = self.normals.tex_id();
        // SAFETY: the frame's context is current; unit 1 is bound to the
        // normals for this draw only and unbound below, and unit 0, which
        // the frame draws `tex` from, is made active again.
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, normals);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        let uniforms = [
            Uniform::new("normals", 1i32),
            Uniform::new("refraction", self.refraction),
            Uniform::new("shine", self.shine),
        ];
        let drawn = frame.render_texture_from_to(
            &self.wallpaper,
            src,
            dst,
            damage,
            opaque_regions,
            Transform::Normal,
            self.alpha,
            Some(&self.program),
            &uniforms,
        );
        // SAFETY: as above.
        frame.with_context(|gl| unsafe {
            gl.ActiveTexture(ffi::TEXTURE1);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
            gl.ActiveTexture(ffi::TEXTURE0);
        })?;
        drawn
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
