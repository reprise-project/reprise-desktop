//! The colour picker on the scene: `reprisekit`'s `colorpick` loupe over
//! every output, sampling stills taken as it opens, before the loupe exists.
//! While open it takes the pointer and every key press (`keyroute`'s region
//! route), and the pointer shows a crosshair. The bind copies a pick as
//! `#rrggbb` and shows it in a notice; a portal's PickColor hears it on its
//! `reprise_color_pick_v1`.

use reprisekit::controls::colorpick::{self, ColorPick, Event, Still};
use reprisescene::{BufferId, TreeId};
use smithay::backend::input::InputTime;
use smithay::reexports::wayland_server::Resource;

use crate::compositor::State;
use crate::inputproto::proto::reprise_color_pick_v1::RepriseColorPickV1;
use crate::output::upload_scaled;
use crate::raster::{Canvas, Rect};
use crate::theme::Color;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

/// Who opened the picker, and so hears the pick.
pub enum Asker {
    Bind,
    Portal(RepriseColorPickV1),
}

impl Asker {
    fn cancel(self) {
        if let Asker::Portal(r) = self {
            r.cancelled();
        }
    }
}

pub struct ColorPickView {
    pick: Pick,
    tree: TreeId,
    loupe: BufferId,
    asker: Asker,
}

/// The picker's point over the stills.
struct Pick {
    /// The point, relative to `area`'s origin.
    model: ColorPick,
    /// The layout box of every output.
    area: Rect,
    /// Each output's still, with its place in the layout.
    stills: Vec<(Rect, Still)>,
}

/// The still holding layout point `p`.
fn still_at(stills: &[(Rect, Still)], p: (i32, i32)) -> Option<&(Rect, Still)> {
    stills.iter().find(|(r, _)| r.contains(p.0, p.1))
}

/// The pixel under layout point `p` as opaque ARGB.
fn sample(stills: &[(Rect, Still)], p: (i32, i32)) -> Option<u32> {
    still_at(stills, p).map(|(r, s)| s.sample((p.0 - r.x, p.1 - r.y)))
}

/// A pick as the clipboard takes it.
fn copied(argb: u32) -> String {
    colorpick::hex(argb).to_lowercase()
}

/// The notice's picture of the pick: a square of it in a grey frame, so white shows.
fn swatch(argb: u32) -> Canvas {
    const SIZE: i32 = 32;
    let [_, r, g, b] = argb.to_be_bytes();
    let mut c = Canvas::new(SIZE as u32, SIZE as u32);
    c.fill_rect(Rect::new(0, 0, SIZE, SIZE), Color::rgb(0x80, 0x80, 0x80));
    c.fill_rect(Rect::new(1, 1, SIZE - 2, SIZE - 2), Color::rgb(r, g, b));
    c
}

impl Pick {
    fn point(&self) -> Option<(i32, i32)> {
        let (x, y) = self.model.point()?;
        Some((x + self.area.x, y + self.area.y))
    }

    fn motion(&mut self, p: (i32, i32)) -> Event {
        self.model.motion((p.0 - self.area.x, p.1 - self.area.y))
    }

    /// A key: an arrow that would leave every output leaves the point where it is.
    fn key(&mut self, sym: u32) -> Event {
        let before = self.model;
        let e = self.model.key(sym, (self.area.width, self.area.height));
        if e == Event::Redraw
            && self
                .point()
                .is_none_or(|p| still_at(&self.stills, p).is_none())
        {
            self.model = before;
            return Event::None;
        }
        match e {
            Event::Pick((x, y)) => Event::Pick((x + self.area.x, y + self.area.y)),
            e => e,
        }
    }
}

/// The loupe as a reader sees it: a colour well on the output under the point, valued by the
/// pixel there.
pub fn access(state: &State) -> Option<reprisekit::access::Tree> {
    pick_access(&state.panels.colorpick.as_ref()?.pick)
}

fn pick_access(pick: &Pick) -> Option<reprisekit::access::Tree> {
    use reprisekit::access::{ROOT, accesskit::Role};
    let p = pick.point()?;
    let (r, still) = still_at(&pick.stills, p)?;
    let mut local = ColorPick::default();
    local.motion((p.0 - r.x, p.1 - r.y));
    let mut tree = crate::access::root(Role::Window, "Colour picker", *r);
    colorpick::access(&local, still, (r.width, r.height), &mut tree, ROOT)?;
    Some(tree)
}

impl State {
    /// The bind, or a portal's PickColor: open the picker. One already open,
    /// the region overlay or the desktop's lock turn the asker away.
    pub fn open_color_pick(&mut self, asker: Asker) {
        let open = self.panels.colorpick.is_some() || self.panels.region.is_some();
        if open || self.desktop_locked() {
            return asker.cancel();
        }
        let outputs = self.scene.outputs().to_vec();
        let mut stills = Vec::new();
        for o in &outputs {
            let shown = self.config.debug.overlay_in_captures;
            let Some(c) = self.without_debug_panels(shown, |s| s.output_still(o)) else {
                log::warn!("colour picker: could not take a still of {}", o.name());
                return asker.cancel();
            };
            let r = self.layout_box(Some(o));
            let (width, height) = c.device_size();
            let still = Still {
                logical: (r.width, r.height),
                width,
                height,
                pixels: c.pixels,
            };
            stills.push((r, still));
        }
        if stills.is_empty() {
            return asker.cancel();
        }
        // The picker takes the button, so whatever it held ends here.
        self.cancel_interaction();
        let tree = self.scene.tree_create(self.layers.overlay);
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.raise_to_top(tree);
        let loupe = self.scene.buffer_create(tree);
        let mut pick = Pick {
            model: ColorPick::default(),
            area: self.layout_box(None),
            stills,
        };
        let p = self.pointer_location();
        pick.motion((p.x as i32, p.y as i32));
        let v = ColorPickView {
            pick,
            tree,
            loupe,
            asker,
        };
        self.panels.colorpick = Some(v);
        self.clear_pointer_focus();
        self.set_cursor("crosshair");
        self.redraw_color_pick();
    }

    /// Close the picker as Escape would.
    pub fn cancel_color_pick(&mut self) {
        self.apply_color_pick(Event::Cancel);
    }

    fn close_color_pick(&mut self) -> Option<ColorPickView> {
        let v = self.panels.colorpick.take()?;
        crate::access::touch(self);
        self.scene.destroy(v.tree);
        self.set_cursor("default");
        self.redraw_all();
        self.repoint(self.pointer_location(), InputTime::now());
        Some(v)
    }

    /// The portal's object went before its answer: the picker closes unheard.
    pub fn color_pick_gone(&mut self, r: &RepriseColorPickV1) {
        let asked = self.panels.colorpick.as_ref().map(|v| &v.asker);
        if matches!(asked, Some(Asker::Portal(a)) if a.id() == r.id()) {
            self.close_color_pick();
        }
    }

    pub fn color_pick_motion(&mut self, x: f64, y: f64) {
        if let Some(v) = self.panels.colorpick.as_mut() {
            let e = v.pick.motion((x as i32, y as i32));
            self.apply_color_pick(e);
        }
    }

    /// A button came up: the left picks the point under it, the right
    /// cancels. Presses do nothing, so the release is the picker's too.
    pub fn color_pick_button(&mut self, button: u32, pressed: bool) {
        let p = self.pointer_location();
        let Some(v) = self.panels.colorpick.as_mut() else {
            return;
        };
        if pressed || !matches!(button, BTN_LEFT | BTN_RIGHT) {
            return;
        }
        v.pick.motion((p.x as i32, p.y as i32));
        let e = match (button, v.pick.point()) {
            (BTN_LEFT, Some(p)) => Event::Pick(p),
            (BTN_LEFT, None) => Event::None,
            _ => Event::Cancel,
        };
        self.apply_color_pick(e);
    }

    pub fn color_pick_key(&mut self, sym: u32) {
        if let Some(v) = self.panels.colorpick.as_mut() {
            let e = v.pick.key(sym);
            self.apply_color_pick(e);
        }
    }

    fn apply_color_pick(&mut self, e: Event) {
        match e {
            Event::None => {}
            Event::Redraw => self.redraw_color_pick(),
            Event::Cancel => {
                if let Some(v) = self.close_color_pick() {
                    v.asker.cancel();
                }
            }
            Event::Pick(p) => {
                let Some(v) = self.close_color_pick() else {
                    return;
                };
                let Some(argb) = sample(&v.pick.stills, p) else {
                    return v.asker.cancel();
                };
                match v.asker {
                    Asker::Portal(r) => r.picked(argb & 0x00ff_ffff),
                    Asker::Bind => {
                        let hex = copied(argb);
                        self.clip_copy(&hex);
                        crate::notifyview::notice_with_image(
                            self,
                            "Colour copied",
                            &format!("{hex} is on the clipboard."),
                            swatch(argb),
                        );
                    }
                }
            }
        }
    }

    fn redraw_color_pick(&mut self) {
        crate::access::touch(self);
        let Some(v) = self.panels.colorpick.as_ref() else {
            return;
        };
        let Some(p) = v.pick.point() else {
            self.scene.buffer_clear(v.loupe);
            return self.redraw_all();
        };
        let Some((r, still)) = still_at(&v.pick.stills, p) else {
            return;
        };
        let local = (p.0 - r.x, p.1 - r.y);
        let place = colorpick::layout(local, (r.width, r.height)).loupe;
        let out = self.output_at(f64::from(p.0), f64::from(p.1));
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let mut c = Canvas::with_scale(colorpick::W as u32, colorpick::H as u32, scale);
        colorpick::draw(
            &mut c,
            &self.text,
            self.config.theme.fonts.size_px,
            still,
            local,
        );
        let (dw, dh) = c.device_size();
        let mut flat = Canvas::new(dw, dh);
        flat.pixels = c.pixels;
        upload_scaled(&mut self.scene, v.loupe, &flat, scale);
        self.scene
            .set_position(v.loupe, r.x + place.x, r.y + place.y);
        self.redraw_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::keysym;

    fn flat(logical: (i32, i32), argb: u32) -> Still {
        Still {
            logical,
            width: logical.0 as u32,
            height: logical.1 as u32,
            pixels: vec![argb; (logical.0 * logical.1) as usize],
        }
    }

    /// Two outputs side by side, the right one higher up, under a picker at `p`.
    fn view(p: (i32, i32)) -> Pick {
        let stills = vec![
            (Rect::new(0, 100, 100, 50), flat((100, 50), 0xff11_2233)),
            (Rect::new(100, 80, 80, 60), flat((80, 60), 0xffaa_bbcc)),
        ];
        let mut v = Pick {
            model: ColorPick::default(),
            area: Rect::new(0, 0, 180, 150),
            stills,
        };
        v.motion(p);
        v
    }

    #[test]
    fn the_loupe_reads_as_a_colour_well_valued_by_the_pixel_on_the_output_under_the_point() {
        use reprisekit::access::accesskit::Role;
        let t = pick_access(&view((120, 100))).expect("a tree");
        let (_, well) = (t.nodes())
            .find(|(_, n)| n.role() == Role::ColorWell)
            .expect("the loupe");
        assert_eq!(well.value(), Some("#aabbcc"));
        assert!(
            pick_access(&view((5, 5))).is_none(),
            "no output under the point"
        );
    }

    #[test]
    fn a_point_samples_the_output_it_is_on() {
        let v = view((0, 0));
        assert_eq!(sample(&v.stills, (5, 120)), Some(0xff11_2233));
        assert_eq!(sample(&v.stills, (100, 80)), Some(0xffaa_bbcc));
        assert_eq!(sample(&v.stills, (5, 5)), None, "no output there");
    }

    #[test]
    fn arrows_nudge_a_pixel_across_outputs_but_not_off_them() {
        let mut v = view((99, 100));
        assert_eq!(v.key(keysym::RIGHT), Event::Redraw);
        assert_eq!(v.point(), Some((100, 100)), "onto the right output");
        let mut v = view((99, 100));
        assert_eq!(v.key(keysym::UP), Event::None, "no output above");
        assert_eq!(v.point(), Some((99, 100)));
        assert_eq!(v.key(keysym::RETURN), Event::Pick((99, 100)));
        assert_eq!(v.key(keysym::ESCAPE), Event::Cancel);
    }

    #[test]
    fn in_lock_mode_a_click_with_the_color_picker_open_goes_to_the_lock() {
        use crate::console::tests::{click_lock_cancel, in_lock_mode, lock_input};
        let (mut h, _portal, _lock) = in_lock_mode();
        let s = &mut h.state;
        let tree = s.scene.tree_create(s.layers.overlay);
        let loupe = s.scene.buffer_create(tree);
        s.panels.colorpick = Some(ColorPickView {
            pick: view((0, 0)),
            tree,
            loupe,
            asker: Asker::Bind,
        });
        click_lock_cancel(&mut h);
        assert_eq!(lock_input(&h), "", "Cancel cleared the field");
    }

    #[test]
    fn in_lock_mode_a_remote_right_click_cancels_the_color_picker() {
        use crate::console::tests::{in_lock_mode, lock_input};
        use smithay::backend::input::ButtonState;
        let (mut h, _portal, _lock) = in_lock_mode();
        let s = &mut h.state;
        let tree = s.scene.tree_create(s.layers.overlay);
        let loupe = s.scene.buffer_create(tree);
        s.panels.colorpick = Some(ColorPickView {
            pick: view((0, 0)),
            tree,
            loupe,
            asker: Asker::Bind,
        });
        s.lock.console.as_mut().unwrap().state.input = "secret".into();
        h.remote_pointer_to(2200.0, 300.0);
        for b in [ButtonState::Pressed, ButtonState::Released] {
            h.from(crate::inputproto::InputSource::Remote, |s| {
                s.handle_button(BTN_RIGHT, b, InputTime::now())
            });
        }
        assert!(
            h.state.panels.colorpick.is_none(),
            "the picker is cancelled"
        );
        assert_eq!(lock_input(&h), "secret", "the lock heard nothing");
    }

    #[test]
    fn a_pick_is_copied_as_lowercase_hex() {
        assert_eq!(copied(0xff3a_6ea5), "#3a6ea5");
        let s = swatch(0xffff_ffff);
        assert_eq!(s.get(0, 0), 0xff80_8080, "a grey frame");
        assert_eq!(s.get(16, 16), 0xffff_ffff);
    }
}
