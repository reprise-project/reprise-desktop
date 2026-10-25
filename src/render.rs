//! What an output shows, as Smithay render elements front to back: the
//! pointer, then the scene from its top. The same list renders to the
//! screen (`backend`) and into capture buffers (`capture`). The clear
//! colour shows only where no wallpaper buffer covers the output.

use crate::mutex::lock;
use std::sync::Mutex;

use smithay::{
    backend::renderer::{
        Color32F,
        element::{
            Kind,
            memory::MemoryRenderBufferRenderElement,
            render_elements,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            texture::TextureRenderElement,
            utils::{Relocate, RelocateRenderElement},
        },
        gles::{GlesRenderer, GlesTexture},
    },
    input::pointer::{CursorImageAttributes, CursorImageStatus},
    output::Output,
    utils::{Logical, Physical, Point, Scale},
    wayland::compositor::with_states,
};

use crate::{compositor::DeskScene, cursor::Cursors};

render_elements! {
    pub OutputElement<=GlesRenderer>;
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Scene=RelocateRenderElement<reprisescene::SceneElement<GlesRenderer>>,
    Tint=crate::nightlight::tint::TintElement,
    Zoom=TextureRenderElement<GlesTexture>,
    Filtered=reprisefx::ColorElement,
    Border=smithay::backend::renderer::element::solid::SolidColorRenderElement,
}

pub struct Parts<'a> {
    pub scene: &'a DeskScene,
    pub cursor: &'a CursorImageStatus,
    pub cursors: &'a mut Cursors,
    pub pointer: Point<f64, Logical>,
}

pub fn clear_color(colors: &reprisekit::theme::ColorScheme) -> Color32F {
    let c = colors.background;
    Color32F::new(
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
        1.0,
    )
}

/// `output`'s elements, front to back, placed so that `origin` (physical
/// pixels from the output's top-left) is at 0,0: the origin is zero for the
/// whole output and a region's corner for a region capture. The pointer is
/// left out unless `with_cursor`.
pub fn output_elements(
    renderer: &mut GlesRenderer,
    mut scene: Parts<'_>,
    output: &Output,
    origin: Point<i32, Physical>,
    with_cursor: bool,
) -> Vec<OutputElement> {
    let Some(geo) = reprisescene::output_rect(output) else {
        return Vec::new();
    };
    let scale = Scale::from(output.current_scale().fractional_scale());
    let at = |p: Point<f64, Logical>| (p - geo.loc.to_f64()).to_physical(scale) - origin.to_f64();
    let mut out = Vec::new();
    out.extend(crate::nightlight::tint::element(output, origin).map(OutputElement::Tint));

    if with_cursor && geo.to_f64().contains(scene.pointer) {
        let pointer = at(scene.pointer);
        cursor_elements(renderer, &mut scene, scale, pointer, &mut out);
    }

    let shift = Point::default() - origin;
    out.extend(
        scene
            .scene
            .render_elements_for(renderer, output)
            .into_iter()
            .map(|e| {
                OutputElement::Scene(RelocateRenderElement::from_element(
                    e,
                    shift,
                    Relocate::Relative,
                ))
            }),
    );
    out
}

pub fn cursor_image(
    renderer: &mut GlesRenderer,
    mut parts: Parts<'_>,
    scale: Scale<f64>,
) -> (Vec<OutputElement>, Point<i32, Physical>) {
    let mut out = Vec::new();
    let hotspot = cursor_elements(renderer, &mut parts, scale, Point::default(), &mut out);
    let mut placed = Vec::new();
    cursor_elements(renderer, &mut parts, scale, hotspot.to_f64(), &mut placed);
    (placed, hotspot)
}

pub fn pointer_elements(
    renderer: &mut GlesRenderer,
    mut parts: Parts<'_>,
    output: &Output,
    at: Point<f64, Logical>,
) -> Vec<OutputElement> {
    let scale = Scale::from(output.current_scale().fractional_scale());
    let mut out = Vec::new();
    cursor_elements(renderer, &mut parts, scale, at.to_physical(scale), &mut out);
    out
}

fn cursor_elements(
    renderer: &mut GlesRenderer,
    scene: &mut Parts<'_>,
    scale: Scale<f64>,
    pointer: Point<f64, Physical>,
    out: &mut Vec<OutputElement>,
) -> Point<i32, Physical> {
    let status: &CursorImageStatus = scene.cursor;
    match status {
        CursorImageStatus::Hidden => Point::default(),
        CursorImageStatus::Surface(surface) => {
            let hotspot = with_states(surface, |s| {
                s.data_map
                    .get::<Mutex<CursorImageAttributes>>()
                    .map(|a| lock(a).hotspot)
                    .unwrap_or_default()
            });
            let hotspot = hotspot.to_f64().to_physical(scale);
            out.extend(render_elements_from_surface_tree(
                renderer,
                surface,
                (pointer - hotspot).to_i32_round(),
                scale,
                1.0,
                Kind::Cursor,
            ));
            hotspot.to_i32_round()
        }
        CursorImageStatus::Named(icon) => {
            // The theme image is drawn at the output's scale rounded up and
            // shrunk to it, so a fractional output gets a sharp pointer.
            let int_scale = scale.x.ceil() as i32;
            let name = scene.cursors.drag.unwrap_or(icon.name());
            let image = scene.cursors.get(name, int_scale).clone();
            let hotspot = image.hotspot.to_f64().to_physical(scale);
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                pointer - hotspot,
                &image.buffer,
                None,
                None,
                None,
                Kind::Cursor,
            ) {
                Ok(e) => out.push(OutputElement::Memory(e)),
                Err(e) => log::warn!("cursor upload: {e}"),
            }
            hotspot.to_i32_round()
        }
    }
}
