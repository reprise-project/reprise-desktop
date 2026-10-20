//! Rendering into a texture instead of onto an output: a still of a window or of a piece of the
//! desktop, a preview kept up to date, a picture read back for a PNG.
//!
//! A [`Target`] is a texture cleared to transparent on its first render, so what the elements leave
//! uncovered stays see-through and a translucent shadow or frame keeps its alpha. It keeps its own
//! damage tracker: render it again with the same kind of elements and only what changed is drawn.
//! [`Target::read`] gives the pixels back.
//!
//! The elements are placed in physical pixels from the texture's top-left at the target's scale: what
//! `Scene::render_elements` (or `render_elements_of`, for one subtree) gives for a `View` of the area the texture covers.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Color32F, ExportMem, Offscreen,
            damage::OutputDamageTracker,
            element::RenderElement,
            gles::{GlesRenderer, GlesTexture, ffi},
        },
    },
    utils::{Buffer, Logical, Physical, Rectangle, Size, Transform},
};

use crate::raster::Canvas;

pub struct Target {
    texture: GlesTexture,
    tracker: OutputDamageTracker,
    /// What the elements leave uncovered shows: transparent unless `clear_to` says otherwise.
    clear: Color32F,
    drawn: bool,
}

impl Target {
    /// A texture `size` physical pixels large, for elements drawn at `scale`.
    pub fn new(
        renderer: &mut GlesRenderer,
        size: Size<i32, Physical>,
        scale: f64,
    ) -> Result<Target, String> {
        if size.w <= 0 || size.h <= 0 || !scale.is_finite() || scale <= 0.0 {
            return Err(format!("no picture of {}x{} at {scale}", size.w, size.h));
        }
        let texture: GlesTexture = renderer
            .create_buffer(Fourcc::Argb8888, (size.w, size.h).into())
            .map_err(|e| e.to_string())?;
        clamp_to_edge(renderer, &texture)?;
        Ok(Target {
            texture,
            tracker: OutputDamageTracker::new(size, scale, Transform::Normal),
            clear: Color32F::TRANSPARENT,
            drawn: false,
        })
    }

    /// Show `colour` where no element covers the texture: a picture of the screen, over the desktop's background.
    pub fn clear_to(mut self, colour: Color32F) -> Target {
        self.clear = colour;
        self
    }

    /// The texture as it stands: a scene buffer shows a clone of it (`Scene::buffer_set_texture`), which a later render updates in place.
    pub fn texture(&self) -> &GlesTexture {
        &self.texture
    }

    /// Draw `elements`, front to back, into the texture. The first render clears it whole; a later one
    /// draws only what changed. Returns the damage drawn, in texture pixels (empty when nothing
    /// changed), as `Scene::buffer_set_texture_with_damage` takes.
    pub fn render<E: RenderElement<GlesRenderer>>(
        &mut self,
        renderer: &mut GlesRenderer,
        elements: &[E],
    ) -> Result<Vec<Rectangle<i32, Buffer>>, String> {
        let age = usize::from(self.drawn);
        let mut fb = renderer
            .bind(&mut self.texture)
            .map_err(|e| e.to_string())?;
        let result = self
            .tracker
            .render_output(renderer, &mut fb, age, elements, self.clear)
            .map_err(|e| format!("{e:?}"))?;
        self.drawn = true;
        // The texture is upright from its top-left, as the elements are placed: physical and buffer pixels are the same.
        Ok(result
            .damage
            .map(|d| {
                d.iter()
                    .map(|r| Rectangle::new((r.loc.x, r.loc.y).into(), (r.size.w, r.size.h).into()))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub fn read(&mut self, renderer: &mut GlesRenderer) -> Result<Canvas, String> {
        let size = self.texture_size();
        let fb = renderer
            .bind(&mut self.texture)
            .map_err(|e| e.to_string())?;
        let mapping = renderer
            .copy_framebuffer(
                &fb,
                Rectangle::from_size((size.w, size.h).into()),
                Fourcc::Argb8888,
            )
            .map_err(|e| e.to_string())?;
        drop(fb);
        // Mapping a read the GPU has not finished waits in the driver for as long as it takes.
        let fence = crate::gpufence::fence_now(renderer)?;
        let deadline = crate::gpufence::READ_DEADLINE;
        if !crate::gpufence::reached_within(&fence, deadline) {
            return Err(format!("the render did not finish in {deadline:?}"));
        }
        let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
        let count = (size.w * size.h) as usize;
        if bytes.len() < count * 4 {
            return Err("short texture read".into());
        }
        // Argb8888 lays out B, G, R, A bytes: 0xAARRGGBB as a little-endian u32.
        let pixels = bytes[..count * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&p| u32::from_le_bytes(p))
            .collect();
        Ok(Canvas::from_pixels(size.w as u32, size.h as u32, pixels))
    }

    fn texture_size(&self) -> Size<i32, Physical> {
        use smithay::backend::renderer::Texture;
        let s = self.texture.size();
        (s.w, s.h).into()
    }

    pub fn into_texture(self) -> GlesTexture {
        self.texture
    }
}

/// Sample `texture` past its edges as its edge pixels. A texture made to render into repeats by
/// default, so a scaled draw of it (the magnifier, a shrinking still) would blend each edge with the
/// opposite one; imported textures already clamp.
fn clamp_to_edge(renderer: &mut GlesRenderer, texture: &GlesTexture) -> Result<(), String> {
    let id = texture.tex_id();
    renderer
        .with_context(|gl| {
            // SAFETY: the renderer's context is current inside `with_context`; `id` is a live texture of
            // that context, and the 2D binding goes back to none, as the renderer leaves it.
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
        .map_err(|e| e.to_string())
}

/// `size` logical pixels at `scale` in whole physical pixels, at least one each way.
pub fn physical_size(size: Size<i32, Logical>, scale: f64) -> Size<i32, Physical> {
    let side = |l: i32| ((f64::from(l) * scale).round() as i32).max(1);
    (side(size.w), side(size.h)).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::backend::renderer::{
        element::{Id, Kind, solid::SolidColorRenderElement},
        utils::CommitCounter,
    };

    fn solid(
        id: &Id,
        commit: CommitCounter,
        at: (i32, i32),
        size: (i32, i32),
        colour: [f32; 4],
    ) -> SolidColorRenderElement {
        SolidColorRenderElement::new(
            id.clone(),
            Rectangle::new(at.into(), size.into()),
            commit,
            colour,
            Kind::Unspecified,
        )
    }

    fn renderer() -> Option<GlesRenderer> {
        match reprisefx::headless::renderer() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("skipped: {e}");
                None
            }
        }
    }

    #[test]
    fn a_translucent_element_reads_back_premultiplied_over_transparency() {
        let Some(mut r) = renderer() else {
            return;
        };
        let mut t = Target::new(&mut r, (8, 4).into(), 1.0).unwrap();
        let id = Id::new();
        let half_red = [0.5, 0.0, 0.0, 0.5];
        let e = solid(&id, CommitCounter::default(), (2, 0), (4, 1), half_red);
        assert_eq!(
            t.render(&mut r, &[e]).unwrap(),
            vec![Rectangle::from_size((8, 4).into())],
            "the first render draws it whole"
        );
        let c = t.read(&mut r).unwrap();
        assert_eq!((c.width, c.height), (8, 4));
        let at = |x: usize, y: usize| c.pixels[y * 8 + x];
        assert_eq!(at(0, 0), 0, "uncovered stays transparent");
        assert_eq!(at(7, 3), 0);
        let p = at(2, 0);
        let ch = |s: u32| (p >> s) & 0xFF;
        assert!(ch(24).abs_diff(128) <= 1, "alpha half: {p:08x}");
        assert!(ch(16).abs_diff(128) <= 1, "red premultiplied: {p:08x}");
        assert_eq!((ch(8), ch(0)), (0, 0), "{p:08x}");
        assert_eq!(at(1, 0), 0, "left of the element");
        assert_eq!(at(2, 1), 0, "below it: the top row comes first");
    }

    #[test]
    fn a_kept_target_redraws_only_what_changed() {
        let Some(mut r) = renderer() else {
            return;
        };
        let mut t = Target::new(&mut r, (8, 2).into(), 1.0).unwrap();
        let (a, b) = (Id::new(), Id::new());
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let mut commit = CommitCounter::default();
        let frame = |commit: CommitCounter, colour| {
            [
                solid(&a, CommitCounter::default(), (0, 0), (4, 2), red),
                solid(&b, commit, (4, 0), (4, 2), colour),
            ]
        };
        assert!(!t.render(&mut r, &frame(commit, red)).unwrap().is_empty());
        assert!(
            t.render(&mut r, &frame(commit, red)).unwrap().is_empty(),
            "nothing changed"
        );
        commit.increment();
        assert_eq!(
            t.render(&mut r, &frame(commit, blue)).unwrap(),
            vec![Rectangle::new((4, 0).into(), (4, 2).into())],
            "the changed half alone"
        );
        let c = t.read(&mut r).unwrap();
        assert_eq!(c.pixels[0], 0xFFFF_0000, "kept from the first render");
        assert_eq!(c.pixels[4], 0xFF00_00FF, "the changed half");
    }

    #[test]
    fn a_logical_size_rounds_to_whole_physical_pixels() {
        assert_eq!(physical_size((10, 3).into(), 1.5), (15, 5).into());
        assert_eq!(physical_size((0, 3).into(), 2.0), (1, 6).into());
    }
}
