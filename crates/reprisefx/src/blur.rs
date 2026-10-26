//! The blur: what lies behind a piece of glass, copied out of the frame
//! being drawn at half its resolution (a quarter at output scale 2 and
//! up), blurred by a separable Gaussian in two passes, with the reflection
//! streaks laid over it.
//!
//! The copy out of the frame (`blit`) follows niri's
//! `src/render_helpers/framebuffer_effect.rs` (GPL-3.0-or-later).

use reprisekit::glass::Material;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, ffi,
};
use smithay::backend::renderer::{Bind, Offscreen, Renderer, Texture};
use smithay::utils::{Buffer, Physical, Rectangle, Size, Transform};

use crate::space::Space;

pub const SIGMA: [f32; 5] = [2.40, 3.83, 5.45, 7.40, 9.47];

pub const MOLTEN_SIGMA: f32 = 0.5;

const MAX_PAIRS: f32 = 16.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Quality {
    #[default]
    Full,
    /// While a window is dragged or sized with `[glass] fast_drag` on: at a
    /// quarter of the resolution, the same spread. A sigma at or below
    /// `[glass] blur = 1`'s is too light to lose half its texels, so it
    /// stays at the full blur. At output scale 2 and up the full blur is
    /// already this coarse.
    Coarse,
}

pub fn quality(dragging: bool, fast_drag: bool) -> Quality {
    if dragging && fast_drag {
        Quality::Coarse
    } else {
        Quality::Full
    }
}

pub fn sigma(blur: u8, material: Material) -> f32 {
    let s = SIGMA[usize::from(blur.clamp(1, SIGMA.len() as u8) - 1)];
    match material {
        Material::Molten => s * MOLTEN_SIGMA,
        Material::Frosted | Material::Smoked => s,
    }
}

/// Output pixels per texel of the backdrop a host draws for an output at
/// `scale`: at 2 and up the output's pixels are small enough that a
/// quarter of them holds all the blur shows.
pub fn backdrop_downscale(scale: f64) -> i32 {
    if scale >= 2.0 { 4 } else { 2 }
}

/// How far the blur reads, in physical pixels: what changes this close to
/// glass shows in it, and the glass element reaches this far past its
/// shapes so the frame there is drawn before it is copied. A blurred texel
/// reads `2·pairs` texels either way, and the downscale and the linear
/// upscale one more each.
pub fn reach(blur: u8, material: Material, quality: Quality, scale: f64) -> i32 {
    let p = Plan::new(blur, material, quality, scale);
    p.factor * (2 * p.pairs as i32 + 2)
}

pub fn texel(blur: u8, material: Material, quality: Quality, scale: f64) -> i32 {
    Plan::new(blur, material, quality, scale).factor
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Plan {
    pub factor: i32,
    pub sigma: f32,
    pub pairs: f32,
}

impl Plan {
    /// The blur on an output at `scale`: the same spread in physical
    /// pixels on every grid, `sigma` being in texels of `factor` pixels.
    pub fn new(blur: u8, material: Material, quality: Quality, scale: f64) -> Plan {
        let s = sigma(blur, material);
        let coarse = match quality {
            Quality::Coarse if s > SIGMA[0] => 4,
            _ => 2,
        };
        let factor = coarse.max(backdrop_downscale(scale));
        let sigma = s * 2.0 / factor as f32;
        Plan {
            factor,
            sigma,
            pairs: (3.0 * sigma / 2.0).ceil().clamp(1.0, MAX_PAIRS),
        }
    }
}

pub(crate) fn capture_rect(
    geometry: Rectangle<i32, Physical>,
    output: Size<i32, Physical>,
    factor: i32,
) -> Option<Rectangle<i32, Physical>> {
    let down = |v: i32| v.div_euclid(factor) * factor;
    let up = |v: i32| (v + factor - 1).div_euclid(factor) * factor;
    let x0 = down(geometry.loc.x).max(0);
    let y0 = down(geometry.loc.y).max(0);
    let x1 = up(geometry.loc.x + geometry.size.w).min(output.w);
    let y1 = up(geometry.loc.y + geometry.size.h).min(output.h);
    (x1 > x0 && y1 > y0).then(|| Rectangle::new((x0, y0).into(), (x1 - x0, y1 - y0).into()))
}

#[derive(Debug, Default)]
pub(crate) struct Levels {
    down: Option<GlesTexture>,
    small: Option<GlesTexture>,
    across: Option<GlesTexture>,
    pub picture: Option<GlesTexture>,
    pub region: Rectangle<i32, Physical>,
    pub drawn: Option<(u64, Plan, Space)>,
}

/// `slot` holding a texture of `size`, made when it has none or another
/// size. Its edges clamp, so the blur reads the edge texel past them.
pub(crate) fn fit(
    slot: &mut Option<GlesTexture>,
    renderer: &mut GlesRenderer,
    size: Size<i32, Buffer>,
) -> Result<GlesTexture, GlesError> {
    if let Some(t) = slot.as_ref().filter(|t| t.size() == size) {
        return Ok(t.clone());
    }
    let t: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, size)?;
    clamp(renderer, &t)?;
    *slot = Some(t.clone());
    Ok(t)
}

pub(crate) fn clamp(renderer: &mut GlesRenderer, t: &GlesTexture) -> Result<(), GlesError> {
    let id = t.tex_id();
    renderer.with_context(|gl| {
        // SAFETY: the context is current inside `with_context`, and `id`
        // is a live 2D texture of it.
        unsafe {
            gl.BindTexture(ffi::TEXTURE_2D, id);
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_S,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.TexParameteri(
                ffi::TEXTURE_2D,
                ffi::TEXTURE_WRAP_T,
                ffi::CLAMP_TO_EDGE as i32,
            );
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }
    })
}

fn blit(
    frame: &mut GlesFrame<'_, '_>,
    region: Rectangle<i32, Physical>,
    to: &GlesTexture,
) -> Result<(), GlesError> {
    let size = to.size();
    let id = to.tex_id();
    frame.with_context(|gl| {
        // SAFETY: the frame's context is current while the frame lives.
        // The framebuffer object made here is deleted before returning,
        // and the draw binding and scissor test the frame set are put
        // back; the read binding (the frame's target) is not touched.
        unsafe {
            while gl.GetError() != ffi::NO_ERROR {}
            let mut current = 0i32;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current);
            gl.Disable(ffi::SCISSOR_TEST);
            let mut fbo = 0;
            gl.GenFramebuffers(1, &mut fbo);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                id,
                0,
            );
            gl.BlitFramebuffer(
                region.loc.x,
                region.loc.y,
                region.loc.x + region.size.w,
                region.loc.y + region.size.h,
                0,
                0,
                size.w,
                size.h,
                ffi::COLOR_BUFFER_BIT,
                ffi::LINEAR,
            );
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current as u32);
            gl.Enable(ffi::SCISSOR_TEST);
            gl.DeleteFramebuffers(1, &fbo);
            if gl.GetError() == ffi::NO_ERROR {
                Ok(())
            } else {
                Err(GlesError::BlitError)
            }
        }
    })?
}

/// Run `f`'s draws into `to` instead of the frame's target: passes chained in one frame skip a
/// finish each (a flush and a fence export). `to` must be the size of the frame's target.
pub(crate) fn drawing_into<T>(
    frame: &mut GlesFrame<'_, '_>,
    to: &GlesTexture,
    f: impl FnOnce(&mut GlesFrame<'_, '_>) -> Result<T, GlesError>,
) -> Result<T, GlesError> {
    let id = to.tex_id();
    let fbo = frame.with_context(|gl| {
        // SAFETY: the frame's context is current while the frame lives; the
        // framebuffer object made here is unbound and deleted below.
        unsafe {
            let mut current = 0i32;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut current);
            let mut fbo = 0;
            gl.GenFramebuffers(1, &mut fbo);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.FramebufferTexture2D(
                ffi::DRAW_FRAMEBUFFER,
                ffi::COLOR_ATTACHMENT0,
                ffi::TEXTURE_2D,
                id,
                0,
            );
            let complete =
                gl.CheckFramebufferStatus(ffi::DRAW_FRAMEBUFFER) == ffi::FRAMEBUFFER_COMPLETE;
            (fbo, complete, current as u32)
        }
    })?;
    let (fbo, complete, current) = fbo;
    let drawn = match complete {
        true => f(frame),
        false => Err(GlesError::FramebufferBindingError),
    };
    frame.with_context(|gl| {
        // SAFETY: as above; the frame's own draw binding is put back.
        unsafe {
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, current);
            gl.DeleteFramebuffers(1, &fbo);
        }
    })?;
    drawn
}

/// `src` of `from` drawn over all of a target of `size`, `parts` of it only.
pub(crate) fn draw_from(
    frame: &mut GlesFrame<'_, '_>,
    from: &GlesTexture,
    src: Rectangle<f64, Buffer>,
    size: Size<i32, Buffer>,
    program: Option<&GlesTexProgram>,
    uniforms: &[Uniform<'_>],
    parts: &[Rectangle<i32, Physical>],
) -> Result<(), GlesError> {
    let full = Rectangle::from_size((size.w, size.h).into());
    frame.render_texture_from_to(
        from,
        src,
        full,
        parts,
        &[full],
        Transform::Normal,
        1.0,
        program,
        uniforms,
    )
}

/// The streaks blended over `parts` of a target of `size`.
pub(crate) fn streak_over(
    frame: &mut GlesFrame<'_, '_>,
    s: &Streaks<'_>,
    size: Size<i32, Buffer>,
    parts: &[Rectangle<i32, Physical>],
) -> Result<(), GlesError> {
    let st = s.texture.size();
    let kx = f64::from(st.w) / f64::from(s.output.w);
    let ky = f64::from(st.h) / f64::from(s.output.h);
    let r = s.shown;
    let src = Rectangle::new(
        (f64::from(r.loc.x) * kx, f64::from(r.loc.y) * ky).into(),
        (f64::from(r.size.w) * kx, f64::from(r.size.h) * ky).into(),
    );
    let full = Rectangle::from_size((size.w, size.h).into());
    frame.render_texture_from_to(
        s.texture,
        src,
        full,
        parts,
        &[],
        s.transform,
        1.0,
        None,
        &[],
    )
}

pub(crate) fn gaussian(plan: Plan, dir: (f32, f32)) -> [Uniform<'static>; 3] {
    [
        Uniform::new("dir", dir),
        Uniform::new("sigma", plan.sigma),
        Uniform::new("pairs", plan.pairs),
    ]
}

pub(crate) struct Capture<'a> {
    pub region: Rectangle<i32, Physical>,
    pub plan: Plan,
    pub blur: &'a GlesTexProgram,
    pub streaks: Option<Streaks<'a>>,
}

pub(crate) struct Streaks<'a> {
    pub texture: &'a GlesTexture,
    pub output: Size<i32, Physical>,
    pub shown: Rectangle<i32, Physical>,
    pub transform: Transform,
}

impl Levels {
    pub fn prepare(
        &mut self,
        renderer: &mut GlesRenderer,
        region: Rectangle<i32, Physical>,
        plan: Plan,
    ) -> Result<(), GlesError> {
        let (w, h, f) = (region.size.w, region.size.h, plan.factor);
        let half: Size<i32, Buffer> = ((w + 1) / 2, (h + 1) / 2).into();
        let out: Size<i32, Buffer> = ((w + f - 1) / f, (h + f - 1) / f).into();
        fit(&mut self.down, renderer, half)?;
        if f > 2 {
            fit(&mut self.small, renderer, out)?;
        } else {
            self.small = None;
        }
        fit(&mut self.across, renderer, out)?;
        fit(&mut self.picture, renderer, out)?;
        Ok(())
    }

    pub fn capture(
        &mut self,
        frame: &mut GlesFrame<'_, '_>,
        c: &Capture<'_>,
    ) -> Result<(), GlesError> {
        let (Some(down), Some(across), Some(mut picture)) =
            (self.down.clone(), self.across.clone(), self.picture.clone())
        else {
            return Ok(());
        };
        blit(frame, c.region, &down)?;
        let mut guard = smithay::backend::renderer::FrameContext::renderer(frame);
        let renderer: &mut GlesRenderer = guard.as_mut();
        let size = picture.size();
        let whole = |t: &GlesTexture| Rectangle::from_size(t.size().to_f64());
        let full = Rectangle::from_size((size.w, size.h).into());
        // One frame for the passes, on the picture that none of them reads.
        let mut target = renderer.bind(&mut picture)?;
        let mut f = renderer.render(&mut target, (size.w, size.h).into(), Transform::Normal)?;
        let source = match self.small.clone() {
            Some(small) => {
                drawing_into(&mut f, &small, |f| {
                    draw_from(f, &down, whole(&down), size, None, &[], &[full])
                })?;
                small
            }
            None => down,
        };
        let across_uniforms = gaussian(c.plan, (1.0 / size.w as f32, 0.0));
        drawing_into(&mut f, &across, |f| {
            draw_from(
                f,
                &source,
                whole(&source),
                size,
                Some(c.blur),
                &across_uniforms,
                &[full],
            )
        })?;
        draw_from(
            &mut f,
            &across,
            whole(&across),
            size,
            Some(c.blur),
            &gaussian(c.plan, (0.0, 1.0 / size.h as f32)),
            &[full],
        )?;
        if let Some(s) = &c.streaks {
            streak_over(&mut f, s, size, &[full])?;
        }
        f.finish_unsynced()?;
        self.region = c.region;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reach_grows_with_the_strength() {
        let r: Vec<i32> = (1..=5)
            .map(|b| reach(b, Material::Frosted, Quality::Full, 1.0))
            .collect();
        assert_eq!(r, vec![20, 28, 40, 52, 64]);
        let c: Vec<i32> = (1..=5)
            .map(|b| reach(b, Material::Frosted, Quality::Coarse, 1.0))
            .collect();
        assert_eq!(
            c,
            vec![20, 32, 48, 56, 72],
            "the coarse blur's wider texels"
        );
        assert_eq!(
            reach(0, Material::Frosted, Quality::Full, 1.0),
            reach(1, Material::Frosted, Quality::Full, 1.0),
            "below range is the lightest"
        );
        assert_eq!(
            reach(9, Material::Frosted, Quality::Full, 1.0),
            reach(5, Material::Frosted, Quality::Full, 1.0),
            "above range is the strongest"
        );
    }

    const F: Material = Material::Frosted;

    #[test]
    fn the_coarse_blur_halves_the_texels_and_the_sigma() {
        let full = Plan::new(3, F, Quality::Full, 1.0);
        let coarse = Plan::new(3, F, Quality::Coarse, 1.0);
        assert_eq!((full.factor, coarse.factor), (2, 4));
        assert_eq!(coarse.sigma * 2.0, full.sigma, "the same spread on screen");
        assert!(coarse.pairs < full.pairs, "fewer reads per pass");
        assert_eq!(
            Plan::new(1, F, Quality::Coarse, 1.0),
            Plan::new(1, F, Quality::Full, 1.0),
            "blur 1 stays full"
        );
        assert!(Plan::new(5, F, Quality::Full, 1.0).pairs <= MAX_PAIRS);
        assert_eq!(quality(true, true), Quality::Coarse);
        assert_eq!(quality(true, false), Quality::Full);
        assert_eq!(quality(false, true), Quality::Full);
    }

    #[test]
    fn at_scale_2_the_full_blur_is_the_coarse_one_on_a_quarter_backdrop() {
        assert_eq!((backdrop_downscale(1.0), backdrop_downscale(1.75)), (2, 2));
        assert_eq!((backdrop_downscale(2.0), backdrop_downscale(3.0)), (4, 4));
        for b in 1..=5u8 {
            let one = Plan::new(b, F, Quality::Full, 1.0);
            let two = Plan::new(b, F, Quality::Full, 2.0);
            assert_eq!(two.factor, 4);
            assert_eq!(
                two.sigma * 4.0,
                one.sigma * 2.0,
                "blur {b}: the same spread in output pixels"
            );
            assert_eq!(Plan::new(b, F, Quality::Coarse, 2.0), two, "blur {b}");
        }
        assert_eq!(
            reach(2, F, Quality::Full, 2.0),
            reach(2, F, Quality::Coarse, 1.0)
        );
    }

    #[test]
    fn molten_blurs_lighter_and_stays_full_while_dragged_below_blur_1() {
        for b in 1..=5u8 {
            assert_eq!(sigma(b, Material::Molten), sigma(b, F) * MOLTEN_SIGMA);
            assert_eq!(sigma(b, Material::Smoked), sigma(b, F));
        }
        let m = |b, q| Plan::new(b, Material::Molten, q, 1.0);
        assert_eq!(
            m(2, Quality::Coarse),
            m(2, Quality::Full),
            "Molten at blur 2 is lighter than blur 1: no coarse blur"
        );
        assert_eq!(m(3, Quality::Coarse).factor, 4);
        assert!(reach(2, Material::Molten, Quality::Full, 1.0) < reach(2, F, Quality::Full, 1.0));
    }

    #[test]
    fn a_capture_snaps_to_the_downscaled_grid_and_stays_on_the_output() {
        let out = Size::from((1920, 1080));
        let r = |x, y, w, h| Rectangle::<i32, Physical>::new((x, y).into(), (w, h).into());
        assert_eq!(
            capture_rect(r(101, 51, 10, 10), out, 2),
            Some(r(100, 50, 12, 12))
        );
        assert_eq!(
            capture_rect(r(101, 51, 10, 10), out, 4),
            Some(r(100, 48, 12, 16))
        );
        assert_eq!(
            capture_rect(r(-30, -30, 50, 50), out, 2),
            Some(r(0, 0, 20, 20))
        );
        assert_eq!(
            capture_rect(r(1900, 1070, 50, 50), out, 4),
            Some(r(1900, 1068, 20, 12))
        );
        assert_eq!(capture_rect(r(2000, 0, 50, 50), out, 2), None);
    }
}
