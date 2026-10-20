//! Shift+PrtSc's region select on the scene: `reprisekit`'s `regionselect`
//! over every output, on top of the overlay layer. Its dim and border are
//! solid scene rects, its size tip one buffer. While it is open it takes
//! the pointer (`repoint`, `handle_button`, the wheel) and every key press
//! (`keyroute`), and the pointer shows a crosshair. On release the overlay
//! goes first, so the still `screenshot` renders is clean.

use reprisekit::controls::regionselect::{self, BORDER, DIM, Event, RegionSelect};
use reprisescene::{BufferId, RectId, TreeId};
use smithay::backend::input::InputTime;

use crate::compositor::State;
use crate::input::Hit;
use crate::output::upload_scaled;
use crate::raster::Rect;

const BTN_LEFT: u32 = 0x110;

pub struct RegionView {
    pub model: RegionSelect,
    tree: TreeId,
    /// Filled with `DIM`, one per rect `layout` can give.
    dim: [RectId; 4],
    /// Filled with `BORDER`, above the dim.
    border: [RectId; 4],
    readout: BufferId,
}

/// `argb` (premultiplied) as the scene's float colour.
fn rgba(argb: u32) -> [f32; 4] {
    let c = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
    [c(16), c(8), c(0), c(24)]
}

impl State {
    /// The bind: open the overlay. No-op while locked or already open.
    pub fn open_region(&mut self) {
        if self.panels.region.is_some() || self.is_locked() {
            return;
        }
        // The overlay takes the button, so whatever it held ends here
        // rather than on a release it never sees.
        self.cancel_interaction();
        let tree = self.scene.tree_create(self.layers.overlay);
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.raise_to_top(tree);
        let mut rects = |argb: u32| {
            let color = rgba(argb);
            [(); 4].map(|_| self.scene.rect_create(tree, (0, 0), color))
        };
        let dim = rects(DIM);
        let border = rects(BORDER);
        let readout = self.scene.buffer_create(tree);
        let p = self.pointer_location();
        self.panels.region = Some(RegionView {
            model: RegionSelect::new((p.x as i32, p.y as i32)),
            tree,
            dim,
            border,
            readout,
        });
        self.clear_pointer_focus();
        self.set_cursor("crosshair");
        self.redraw_region();
    }

    pub fn close_region(&mut self) {
        let Some(v) = self.panels.region.take() else {
            return;
        };
        self.scene.destroy(v.tree);
        self.set_cursor("default");
        self.redraw_all();
    }

    pub fn region_motion(&mut self, x: f64, y: f64) {
        if let Some(v) = self.panels.region.as_mut() {
            let e = v.model.motion((x as i32, y as i32));
            self.apply_region(e);
        }
    }

    /// A button went down or came up: the left one selects, a press of
    /// any other cancels.
    pub fn region_button(&mut self, button: u32, pressed: bool) {
        let p = self.pointer_location();
        let p = (p.x as i32, p.y as i32);
        let Some(v) = self.panels.region.as_mut() else {
            return;
        };
        let e = match (button == BTN_LEFT, pressed) {
            (true, true) => v.model.press(p),
            (true, false) => v.model.release(p),
            (false, true) => v.model.escape(),
            (false, false) => Event::None,
        };
        self.apply_region(e);
    }

    /// A key press while open: Escape cancels, the rest do nothing.
    pub fn region_key(&mut self, sym: u32) {
        if sym == crate::keys::KEY_Escape
            && let Some(v) = self.panels.region.as_mut()
        {
            let e = v.model.escape();
            self.apply_region(e);
        }
    }

    fn apply_region(&mut self, e: Event) {
        match e {
            Event::None => return,
            Event::Redraw => return self.redraw_region(),
            Event::Cancel | Event::Region(_) | Event::Point(..) => self.close_region(),
        }
        let area = match e {
            Event::Region(r) => Some(r),
            Event::Point(x, y) => Some(self.clicked_area(x, y)),
            _ => None,
        };
        if let Some(r) = area {
            self.screenshot_area(r);
        }
        self.repoint(self.pointer_location(), InputTime::now());
    }

    /// What a click at (`x`, `y`) takes: the window there, frame and all,
    /// or else the whole output. The overlay is closed by now.
    fn clicked_area(&self, x: i32, y: i32) -> Rect {
        let (lx, ly) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
        let window = match self.scene_hit((lx, ly).into()) {
            Hit::Surface { window, .. } => window,
            Hit::Frame { window, .. } => Some(window),
            _ => None,
        };
        match window.and_then(|w| self.win(w)) {
            Some(w) => w.frame_rect(),
            None => self.layout_box(self.output_at(lx, ly).as_ref()),
        }
    }

    fn redraw_region(&mut self) {
        let Some(v) = self.panels.region.as_ref() else {
            return;
        };
        let area = self.layout_box(None);
        let p = v.model.pointer();
        let out = self.output_at(f64::from(p.0), f64::from(p.1));
        let screen = self.layout_box(out.as_ref());
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let State {
            panels,
            config,
            scene,
            text,
            ..
        } = self;
        let Some(v) = panels.region.as_ref() else {
            return;
        };
        let theme = &config.theme;
        let lay = regionselect::layout(&v.model, text, theme, area, screen);
        let mut place = |nodes: &[RectId; 4], rects: &[Rect]| {
            for (i, &n) in nodes.iter().enumerate() {
                match rects.get(i) {
                    Some(r) => {
                        scene.rect_set_size(n, (r.width, r.height));
                        scene.set_position(n, r.x, r.y);
                        scene.set_enabled(n, true);
                    }
                    None => scene.set_enabled(n, false),
                }
            }
        };
        place(&v.dim, &lay.dim);
        place(&v.border, &lay.border);
        let tip =
            regionselect::draw_readout(&theme.scaled(scale), text.for_output(scale), &v.model);
        match (lay.readout, tip) {
            (Some(r), Some(canvas)) => {
                upload_scaled(scene, v.readout, &canvas, scale);
                scene.set_position(v.readout, r.x, r.y);
            }
            _ => scene.buffer_clear(v.readout),
        }
        self.redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_premultiplied_colour_keeps_its_channels_in_scene_order() {
        assert_eq!(
            rgba(0x80_40_20_10),
            [
                0x40 as f32 / 255.0,
                0x20 as f32 / 255.0,
                0x10 as f32 / 255.0,
                0x80 as f32 / 255.0
            ]
        );
    }
}
