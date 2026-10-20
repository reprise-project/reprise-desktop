//! A still of one window alone over transparency for the open and close animations; the frame's glass is left out
//! (`windowanim` lays a live backdrop under it).

use reprisescene::{TreeId, View};
use smithay::backend::renderer::gles::GlesTexture;
use smithay::utils::Rectangle;

use crate::compositor::State;
use crate::offscreen::Target;
use crate::raster::Rect;
use crate::window::WindowId;

/// A picture of one window alone, covering `rect` (layout coordinates,
/// the frame plus room for its shadow) at `scale`.
pub struct Still {
    pub texture: GlesTexture,
    pub rect: Rect,
}

pub fn still_rect(f: Rect) -> Rect {
    let m = reprisekit::glass::shadow::shadow(true).margins;
    Rect::new(
        f.x - m.left,
        f.y - m.top,
        f.width + m.left + m.right,
        f.height + m.top + m.bottom,
    )
}

impl State {
    /// Render `w` (shadow, frame, client, subsurfaces, popups) alone, shown
    /// or hidden; at an unmap the client's last buffers are still there, so a
    /// closing window is pictured as it was. At the sharpest scale of the
    /// screens it stands on (1 on none). None when the window has no size or
    /// the render fails.
    pub fn window_still(&mut self, w: WindowId) -> Option<Still> {
        let (tree, f) = self.win(w).map(|x| (x.tree, x.frame_rect()))?;
        if f.is_empty() {
            return None;
        }
        let rect = still_rect(f);
        let outputs: Vec<_> = self
            .scene
            .outputs()
            .iter()
            .filter(|o| !self.layout_box(Some(o)).intersect(&rect).is_empty())
            .cloned()
            .collect();
        let scale = outputs
            .iter()
            .map(|o| o.current_scale().fractional_scale())
            .fold(1.0f64, f64::max);
        let texture = self.tree_still(tree, rect, scale, false)?;
        Some(Still { texture, rect })
    }

    /// Render `tree` alone over `rect` (layout coordinates) at `scale`,
    /// shown or hidden, into a texture cleared to transparent, or with
    /// `desktop` to the desktop's background colour; the glass is left out.
    /// None when the render fails.
    pub fn tree_still(
        &mut self,
        tree: TreeId,
        rect: Rect,
        scale: f64,
        desktop: bool,
    ) -> Option<GlesTexture> {
        let mut target = self.tree_target(rect, scale, desktop)?;
        self.tree_render(tree, rect, scale, &mut target)?;
        Some(target.into_texture())
    }

    /// A texture for [`State::tree_render`] over `rect` (layout
    /// coordinates) at `scale`, cleared to transparent, or with `desktop`
    /// to the desktop's background colour. None when it cannot be made.
    pub fn tree_target(&mut self, rect: Rect, scale: f64, desktop: bool) -> Option<Target> {
        let clear = desktop.then(|| crate::render::clear_color(&self.config.theme.colors));
        let size = crate::offscreen::physical_size((rect.width, rect.height).into(), scale);
        let renderer = &mut self.backend.as_mut()?.renderer;
        match Target::new(renderer, size, scale) {
            Ok(t) => Some(match clear {
                Some(c) => t.clear_to(c),
                None => t,
            }),
            Err(e) => {
                log::warn!("window still: {e}");
                None
            }
        }
    }

    /// Render `tree` alone over `rect` (layout coordinates) at `scale`,
    /// shown or hidden, into `target`, made by [`State::tree_target`] for
    /// the same `rect` and `scale`; the glass is left out. Only what changed
    /// since the target's last render is drawn: whether anything was. None
    /// when the render fails.
    pub fn tree_render(
        &mut self,
        tree: TreeId,
        rect: Rect,
        scale: f64,
        target: &mut Target,
    ) -> Option<bool> {
        let output = (self.scene.outputs().iter())
            .find(|o| !self.layout_box(Some(o)).intersect(&rect).is_empty())
            .or_else(|| self.scene.outputs().first())
            .cloned();
        let State {
            backend,
            scene,
            glass,
            config,
            ..
        } = self;
        let renderer = &mut backend.as_mut()?.renderer;
        (crate::gpufence::ready(renderer))
            .map_err(|e| log::warn!("window still: {e}"))
            .ok()?;
        // No glass in the picture: it would blur the transparency it is
        // drawn over.
        if let Some(o) = &output {
            let settings = reprisekit::glass::Settings {
                transparency: false,
                ..config.glass
            };
            glass.begin(o, settings, reprisefx::Quality::default(), false);
        }
        let view = View::new(
            Rectangle::new((rect.x, rect.y).into(), (rect.width, rect.height).into()),
            scale,
        );
        let elements = scene.render_elements_of(renderer, tree, &view);
        match target.render(renderer, &elements) {
            Ok(damage) => Some(!damage.is_empty()),
            Err(e) => {
                log::warn!("window still: {e}");
                None
            }
        }
    }
}
