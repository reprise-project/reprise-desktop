//! The 3D switcher's card: a window's picture laid on a tilted plane in
//! perspective (`reprisekit::flip`), as one Smithay render element.
//!
//! The element covers the card's bounds on the output and draws them
//! through the tilted-card program (`shaders`), which maps every pixel
//! back into the picture. It reports its whole geometry as damage on
//! every frame: the stack moves or its pictures change while it is up,
//! and it is up only briefly.
//!
//! The host compiles the program once per renderer ([`FlipFx::new`]),
//! keeps a [`FlipCard`] per card for the stack's life (one element Id),
//! and per frame and output makes [`FlipCard::element`] with a map from
//! the picture's texels to the output's physical pixels.
//!
//! Cards are drawn far smaller than their pictures. On a GLES 3 context
//! the card samples its picture's mipmaps (trilinear, through a sampler
//! object, which the renderer's own filter settings do not reach); the
//! host builds them with [`mipmaps`] whenever it draws into a picture.
//! Without GLES 3 a card samples the picture alone.

use std::cell::Cell;

use reprisekit::flip::{self, Homography};
use smithay::backend::renderer::Texture;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement, UnderlyingStorage};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, ffi,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Rectangle, Scale, Transform};

use crate::shaders;

/// How much of its picture's translucent art a card keeps (a frame's
/// glass, which the picture draws without the blur behind it): a texel
/// of alpha `a` keeps `GLASS + (1 - GLASS) * a` of itself, so the glass
/// reads almost clear, its sheen and outline still show and opaque
/// texels stay as they are.
const GLASS: f32 = 0.25;

/// The card's outline: black at this alpha over the picture's outermost
/// screen pixel, all round.
const RIM: f32 = 0.5;

/// The tilted-card program and its trilinear sampler (0 for none), for
/// one renderer.
#[derive(Clone, Debug)]
pub struct FlipFx {
    program: GlesTexProgram,
    sampler: u32,
}

impl FlipFx {
    pub fn new(renderer: &mut GlesRenderer) -> Result<FlipFx, GlesError> {
        let program = shaders::compile_flip(renderer)?;
        let sampler = renderer.with_context(|gl| {
            if !gles3(gl) {
                return 0;
            }
            let mut id = 0;
            // SAFETY: the context is current inside `with_context`; the
            // sampler is new and only its own parameters are set.
            unsafe {
                gl.GenSamplers(1, &mut id);
                let min = ffi::LINEAR_MIPMAP_LINEAR as i32;
                gl.SamplerParameteri(id, ffi::TEXTURE_MIN_FILTER, min);
                gl.SamplerParameteri(id, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                let clamp = ffi::CLAMP_TO_EDGE as i32;
                gl.SamplerParameteri(id, ffi::TEXTURE_WRAP_S, clamp);
                gl.SamplerParameteri(id, ffi::TEXTURE_WRAP_T, clamp);
            }
            id
        })?;
        Ok(FlipFx { program, sampler })
    }

    pub fn trilinear(&self) -> bool {
        self.sampler != 0
    }
}

/// Whether the context runs GLES 3 or later, with sampler objects and
/// mipmaps of any size.
pub(crate) fn gles3(gl: &ffi::Gles2) -> bool {
    // SAFETY: the context is current; GL_VERSION is a static string.
    let v = unsafe { gl.GetString(ffi::VERSION) };
    if v.is_null() || !gl.GenSamplers.is_loaded() || !gl.BindSampler.is_loaded() {
        return false;
    }
    // SAFETY: GL returns a NUL-terminated string it owns.
    let v = unsafe { std::ffi::CStr::from_ptr(v.cast()) }.to_string_lossy();
    // "OpenGL ES 3.2 Mesa ..."
    (v.split_whitespace().nth(2))
        .and_then(|n| n.split('.').next()?.parse::<u32>().ok())
        .is_some_and(|major| major >= 3)
}

/// Build `texture`'s mipmaps from what it holds now, for cards that draw
/// it smaller. No-op without GLES 3, where cards sample it alone.
pub fn mipmaps(renderer: &mut GlesRenderer, texture: &GlesTexture) -> Result<(), GlesError> {
    let id = texture.tex_id();
    renderer.with_context(|gl| {
        if !gles3(gl) {
            return;
        }
        // SAFETY: the context is current; `id` is a live 2D texture of
        // it, and the binding goes back to none, as the renderer leaves it.
        unsafe {
            gl.BindTexture(ffi::TEXTURE_2D, id);
            gl.GenerateMipmap(ffi::TEXTURE_2D);
            gl.BindTexture(ffi::TEXTURE_2D, 0);
        }
    })
}

#[derive(Debug)]
pub struct FlipCard {
    id: Id,
    commit: Cell<CommitCounter>,
}

impl Default for FlipCard {
    fn default() -> FlipCard {
        FlipCard::new()
    }
}

impl FlipCard {
    pub fn new() -> FlipCard {
        FlipCard {
            id: Id::new(),
            commit: Cell::new(CommitCounter::default()),
        }
    }

    /// The card showing `texture` mapped to the output by `to_screen`
    /// (texels to physical pixels) at `alpha`; None when no part of it
    /// lies in front of the eye or the map collapses. With `mipmapped`
    /// the texture's mipmaps are built ([`mipmaps`]) and the card samples
    /// them.
    pub fn element(
        &self,
        fx: &FlipFx,
        texture: &GlesTexture,
        to_screen: Homography,
        alpha: f32,
        mipmapped: bool,
    ) -> Option<FlipCardElement> {
        let size = texture.size();
        let b = flip::bounds(&to_screen, (size.w, size.h));
        if b.width <= 0 || b.height <= 0 {
            return None;
        }
        let back = to_screen.inverse()?;
        let mut commit = self.commit.get();
        commit.increment();
        self.commit.set(commit);
        Some(FlipCardElement {
            id: self.id.clone(),
            commit,
            program: fx.program.clone(),
            sampler: if mipmapped { fx.sampler } else { 0 },
            texture: texture.clone(),
            geometry: Rectangle::new((b.x, b.y).into(), (b.width, b.height).into()),
            back: back.0.map(|r| r.map(|x| x as f32)),
            alpha,
        })
    }
}

/// A card for one frame; see [`FlipCard::element`].
#[derive(Debug)]
pub struct FlipCardElement {
    id: Id,
    commit: CommitCounter,
    program: GlesTexProgram,
    /// The trilinear sampler, 0 for the texture's own filter.
    sampler: u32,
    texture: GlesTexture,
    geometry: Rectangle<i32, Physical>,
    /// From output pixels, where the card was made, to texels.
    back: [[f32; 3]; 3],
    alpha: f32,
}

impl Element for FlipCardElement {
    fn id(&self) -> &Id {
        &self.id
    }

    fn current_commit(&self) -> CommitCounter {
        self.commit
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        let s = self.texture.size();
        Rectangle::from_size((f64::from(s.w), f64::from(s.h)).into())
    }

    fn geometry(&self, _scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.geometry
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        _commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        DamageSet::from_slice(&[Rectangle::from_size(self.geometry(scale).size)])
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

impl RenderElement<GlesRenderer> for FlipCardElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        _opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        // The element may be drawn away from where it was made (a region
        // capture moves everything by its corner); the map runs from
        // where it was made.
        let shift = dst.loc - self.geometry.loc;
        let s = self.texture.size();
        let uniforms = [
            Uniform::new(
                "dst",
                [
                    (dst.loc.x - shift.x) as f32,
                    (dst.loc.y - shift.y) as f32,
                    dst.size.w as f32,
                    dst.size.h as f32,
                ],
            ),
            Uniform::new("hx", self.back[0]),
            Uniform::new("hy", self.back[1]),
            Uniform::new("hw", self.back[2]),
            Uniform::new("tex_size", [s.w as f32, s.h as f32]),
            Uniform::new("glass", GLASS),
            Uniform::new("rim", RIM),
        ];
        let sampler = self.sampler;
        if sampler != 0 {
            // SAFETY: the frame's context is current; the sampler is this
            // context's, bound to the unit the program samples and
            // unbound again below.
            frame.with_context(|gl| unsafe { gl.BindSampler(0, sampler) })?;
        }
        let drawn = frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            &[],
            Transform::Normal,
            self.alpha,
            Some(&self.program),
            &uniforms,
        );
        if sampler != 0 {
            // SAFETY: as above.
            frame.with_context(|gl| unsafe { gl.BindSampler(0, 0) })?;
        }
        drawn
    }

    fn underlying_storage(&self, _renderer: &mut GlesRenderer) -> Option<UnderlyingStorage<'_>> {
        None
    }
}
