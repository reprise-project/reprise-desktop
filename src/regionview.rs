//! Shift+PrtSc's region select on the scene: `reprisekit`'s `regionselect`
//! over every output, on top of the overlay layer. Its dim and border are
//! solid scene rects, its size and mode tips a buffer each. While it is open
//! it takes the pointer (`repoint`, `handle_button`, the wheel) and every key
//! press (`keyroute`), and the pointer shows a crosshair. On release the
//! overlay goes first, so the still `screenshot` renders is clean; in Record
//! mode the pick starts a recording (`screenrecord`) instead.

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
    mode: BufferId,
}

/// `argb` (premultiplied) as the scene's float colour.
fn rgba(argb: u32) -> [f32; 4] {
    let c = |shift: u32| ((argb >> shift) & 0xff) as f32 / 255.0;
    [c(16), c(8), c(0), c(24)]
}

impl State {
    /// The bind: open the overlay, in Record mode for `record`. No-op while locked or already
    /// open.
    pub fn open_region(&mut self, record: bool) {
        if self.panels.region.is_some() || self.desktop_locked() {
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
        let mode = self.scene.buffer_create(tree);
        let p = self.pointer_location();
        self.panels.region = Some(RegionView {
            model: RegionSelect::new((p.x as i32, p.y as i32), record),
            tree,
            dim,
            border,
            readout,
            mode,
        });
        self.clear_pointer_focus();
        self.set_cursor("crosshair");
        self.redraw_region();
        crate::access::touch(self);
    }

    pub fn close_region(&mut self) {
        let Some(v) = self.panels.region.take() else {
            return;
        };
        self.scene.destroy(v.tree);
        self.set_cursor("default");
        self.redraw_all();
        crate::access::touch(self);
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

    /// A key press while open: Escape cancels, V switches between a screenshot and a recording,
    /// the rest do nothing.
    pub fn region_key(&mut self, sym: u32) {
        use crate::keys::{KEY_Escape, KEY_V, KEY_v};
        let Some(v) = self.panels.region.as_mut() else {
            return;
        };
        let e = if sym == KEY_Escape {
            v.model.escape()
        } else if sym == KEY_v || sym == KEY_V {
            v.model.toggle_record()
        } else {
            return;
        };
        self.apply_region(e);
        crate::access::touch(self);
    }

    fn apply_region(&mut self, e: Event) {
        let record = self
            .panels
            .region
            .as_ref()
            .is_some_and(|v| v.model.records());
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
        match area {
            Some(r) if record => self.record_area(r),
            Some(r) => self.screenshot_area(r),
            None => {}
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
        let (theme, text) = (theme.scaled(scale), text.for_output(scale));
        let tip = regionselect::draw_readout(&theme, text, &v.model);
        match (lay.readout, tip) {
            (Some(r), Some(canvas)) => {
                upload_scaled(scene, v.readout, &canvas, scale);
                scene.set_position(v.readout, r.x, r.y);
            }
            _ => scene.buffer_clear(v.readout),
        }
        let mode = regionselect::draw_mode(&theme, text, &v.model);
        upload_scaled(scene, v.mode, &mode, scale);
        scene.set_position(v.mode, lay.mode.x, lay.mode.y);
        self.redraw_all();
    }
}

/// The overlay as a reader sees it: a window over the pointer's screen naming the mode, its tips as
/// labels.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let v = state.panels.region.as_ref()?;
    let p = v.model.pointer();
    let screen = state.layout_box(state.output_at(f64::from(p.0), f64::from(p.1)).as_ref());
    let lay = regionselect::layout(
        &v.model,
        &state.text,
        &state.config.theme,
        state.layout_box(None),
        screen,
    );
    let title = if v.model.records() {
        "Record an area"
    } else {
        "Screenshot of an area"
    };
    let local = |r: Rect| Rect::new(r.x - screen.x, r.y - screen.y, r.width, r.height);
    let lay = regionselect::Layout {
        readout: lay.readout.map(local),
        mode: local(lay.mode),
        ..lay
    };
    let mut tree = crate::access::root(Role::Window, title, screen);
    regionselect::access(&v.model, &lay, &mut tree, ROOT);
    Some(tree)
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
