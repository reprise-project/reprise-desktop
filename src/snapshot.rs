use reprisescene::View;
use smithay::backend::renderer::gles::GlesTexture;
use smithay::output::Output;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::offscreen::{Target, physical_size};
use crate::raster::{Canvas, Rect};

impl State {
    fn render_area(&mut self, area: Rect, scale: f64) -> Option<Target> {
        if area.is_empty() {
            return None;
        }
        // The glass draws for the screen the area is mostly on.
        let output = self.output_at(
            f64::from(area.x + area.width / 2),
            f64::from(area.y + area.height / 2),
        );
        let quality = crate::glass::quality(&self.interaction, &self.config.glass);
        let whole = output.as_ref().filter(|o| {
            self.layout_box(Some(o)) == area && o.current_scale().fractional_scale() == scale
        });
        let whole = whole.cloned();
        let wanted = crate::glass::wanted(&self.config);
        let State {
            backend,
            scene,
            glass,
            config,
            ..
        } = self;
        let renderer = &mut backend.as_mut()?.renderer;
        (crate::gpufence::ready(renderer))
            .map_err(|e| log::warn!("snapshot: {e}"))
            .ok()?;
        let clear = crate::render::clear_color(&config.theme.colors);
        if let Some(o) = &output {
            glass.begin(o, config.glass, quality, crate::glass::Hold::No);
        }
        if let Some(o) = &whole {
            glass.update_backdrop(renderer, scene, o, clear, wanted);
        }
        let view = View::new(
            Rectangle::new((area.x, area.y).into(), (area.width, area.height).into()),
            scale,
        );
        let elements = scene.render_elements(renderer, &view);
        let size = physical_size((area.width, area.height).into(), scale);
        let rendered = Target::new(renderer, size, scale).and_then(|t| {
            let mut t = t.clear_to(clear);
            t.render(renderer, &elements).map(|_| t)
        });
        rendered.map_err(|e| log::warn!("snapshot: {e}")).ok()
    }

    pub fn output_still(&mut self, output: &Output) -> Option<Canvas> {
        let area = self.layout_box(Some(output));
        let scale = output.current_scale().fractional_scale();
        let mut target = self.render_area(area, scale)?;
        let renderer = &mut self.backend.as_mut()?.renderer;
        target
            .read(renderer)
            .map_err(|e| log::warn!("snapshot: could not read the still back: {e}"))
            .ok()
    }

    pub fn render_region(
        &mut self,
        at: (i32, i32),
        w: i32,
        h: i32,
        scale: f64,
    ) -> Option<GlesTexture> {
        self.render_area(Rect::new(at.0, at.1, w, h), scale)
            .map(Target::into_texture)
    }
}

pub fn greyscale(c: &mut Canvas) {
    for p in &mut c.pixels {
        let (r, g, b) = ((*p >> 16) & 0xff, (*p >> 8) & 0xff, *p & 0xff);
        let y = (r * 299 + g * 587 + b * 114) / 1000;
        *p = 0xff00_0000 | (y << 16) | (y << 8) | y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greyscale_takes_the_luma_and_makes_pixels_opaque() {
        let mut c = Canvas::new(2, 1);
        c.pixels[0] = 0xff3a_6ea5;
        c.pixels[1] = 0x00ff_0000;
        greyscale(&mut c);
        assert_eq!(c.pixels[0], 0xff64_6464, "luma 100");
        assert_eq!(c.pixels[1], 0xff4c_4c4c);
    }
}
