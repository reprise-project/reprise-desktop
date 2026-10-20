//! The screen's colour filter (`reprisekit::colorfilter`): a texture of
//! the whole frame drawn through one affine colour map per pixel.
//!
//! The host compiles the program once per renderer ([`ColorFx::new`]) and
//! wraps the element that shows its frame texture in a [`ColorElement`]:
//! the same element, id, damage and all, drawn through the map. A change
//! of map is the host's to announce, by giving the element a new id.

use reprisekit::colorfilter::Matrix;
use smithay::backend::renderer::element::texture::TextureRenderElement;
use smithay::backend::renderer::element::{Element, Id, Kind, RenderElement};
use smithay::backend::renderer::gles::{
    GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
};
use smithay::backend::renderer::utils::{CommitCounter, DamageSet, OpaqueRegions};
use smithay::utils::user_data::UserDataMap;
use smithay::utils::{Buffer, Physical, Point, Rectangle, Scale, Transform};

use crate::shaders;

#[derive(Clone, Debug)]
pub struct ColorFx {
    program: GlesTexProgram,
}

impl ColorFx {
    pub fn new(renderer: &mut GlesRenderer) -> Result<ColorFx, GlesError> {
        Ok(ColorFx {
            program: shaders::compile_color(renderer)?,
        })
    }

    pub fn element(
        &self,
        inner: TextureRenderElement<GlesTexture>,
        texture: GlesTexture,
        m: &Matrix,
    ) -> ColorElement {
        ColorElement {
            inner,
            texture,
            program: self.program.clone(),
            rows: m.rows,
            linear_light: m.linear,
        }
    }
}

pub struct ColorElement {
    inner: TextureRenderElement<GlesTexture>,
    texture: GlesTexture,
    program: GlesTexProgram,
    rows: [[f32; 4]; 3],
    linear_light: bool,
}

impl Element for ColorElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }

    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }

    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }

    fn transform(&self) -> Transform {
        self.inner.transform()
    }

    fn src(&self) -> Rectangle<f64, Buffer> {
        self.inner.src()
    }

    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }

    fn opaque_regions(&self, scale: Scale<f64>) -> OpaqueRegions<i32, Physical> {
        self.inner.opaque_regions(scale)
    }

    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }

    fn kind(&self) -> Kind {
        self.inner.kind()
    }

    fn location(&self, scale: Scale<f64>) -> Point<i32, Physical> {
        self.inner.location(scale)
    }
}

impl RenderElement<GlesRenderer> for ColorElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, Buffer>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
        _cache: Option<&UserDataMap>,
    ) -> Result<(), GlesError> {
        let uniforms = [
            Uniform::new("row_r", self.rows[0]),
            Uniform::new("row_g", self.rows[1]),
            Uniform::new("row_b", self.rows[2]),
            Uniform::new("linear_light", if self.linear_light { 1.0f32 } else { 0.0 }),
        ];
        frame.render_texture_from_to(
            &self.texture,
            src,
            dst,
            damage,
            opaque_regions,
            self.transform(),
            self.alpha(),
            Some(&self.program),
            &uniforms,
        )
    }
}
