//! The clipboard history popup on the scene: one
//! `reprisekit::controls::cliphistory` panel on the menu layer, at the
//! text caret when the focused client reports one, else at the pointer.
//! While open it takes the keyboard from `keyroute`. A pick puts the entry
//! back on the clipboard and sends the focused window its paste key:
//! Ctrl+V, or Ctrl+Shift+V for an app id in `[clipboard] terminal_apps`.

use reprisekit::controls::cliphistory::{self, ClipHistory, Event, Hit, Key, Row};
use reprisescene::{BufferId, TreeId};
use smithay::backend::input::{InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode, xkb};
use smithay::utils::SERIAL_COUNTER;

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::{self, Backdrop};
use crate::keys;
use crate::output::upload_scaled;
use crate::raster::Canvas;

pub struct ClipView {
    pub model: ClipHistory,
    pub origin: (i32, i32),
    /// The scale of the output it opened on; rows' thumbnails are sized
    /// for it.
    pub scale: i32,
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
}

fn panel_size(model: &ClipHistory, theme: &reprisekit::theme::Theme) -> (i32, i32) {
    let cm = theme.look.controls();
    (
        cliphistory::width(&cm),
        cliphistory::layout(model, &cm).height,
    )
}

/// A thumbnail centred on a transparent 16 px square at `scale`, so every
/// row's label starts at the same x.
fn row_icon(thumb: &Canvas, scale: i32) -> Canvas {
    let side = (16 * scale) as u32;
    let (w, h) = crate::cliphistory::fit(thumb.width, thumb.height, side);
    let small = thumb.resample_box(w, h);
    let mut c = Canvas::new(side, side);
    c.blit_over(((side - w) / 2) as i32, ((side - h) / 2) as i32, &small);
    c
}

/// The popup's key for a press the filter caught: Enter, Escape, the
/// arrows and pages, or an edit key for the filter; None for a key it
/// does not use.
fn key_of(k: &DialogKey) -> Option<Key> {
    let special = k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_Return | keys::KEY_KP_Enter => Some(Key::Enter),
        keys::KEY_Escape => Some(Key::Escape),
        keys::KEY_Up | keys::KEY_KP_Up => Some(Key::Up),
        keys::KEY_Down | keys::KEY_KP_Down => Some(Key::Down),
        keys::KEY_Page_Up | keys::KEY_KP_Page_Up => Some(Key::PageUp),
        keys::KEY_Page_Down | keys::KEY_KP_Page_Down => Some(Key::PageDown),
        _ => None,
    });
    special.or_else(|| k.edit.clone().map(Key::Char))
}

/// evdev's code for the V key, the paste key's place without a v on the
/// layout.
const EVDEV_V: u32 = 47;

fn mod_bit(i: u32) -> u32 {
    if i < 32 { 1 << i } else { 0 }
}

impl State {
    /// The bind: open, or close if open. Does nothing with the history off
    /// or under the lock.
    pub fn toggle_clipview(&mut self) {
        if self.panels.clipview.is_some() {
            self.close_clipview();
            return;
        }
        if self.is_locked() || !self.config.clipboard.history {
            return;
        }
        let anchor = self.panel_anchor();
        let out = self.output_at(
            f64::from(anchor.x + anchor.width / 2),
            f64::from(anchor.y + anchor.height / 2),
        );
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let rows = self
            .panels
            .history
            .history
            .entries()
            .map(|e| Row {
                id: e.id,
                label: e.kind.label(),
                icon: e.thumb.as_ref().map(|t| row_icon(t, scale)),
            })
            .collect();
        let model = ClipHistory::new(rows);
        let size = panel_size(&model, &self.config.theme);
        let origin = crate::textinput::popup_origin(anchor, size, self.usable_area(out.as_ref()));
        let tree = self.scene.tree_create(self.layers.menu);
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.set_data(tree, Some(SceneOwner::ClipView));
        self.scene.set_position(tree, origin.0, origin.1);
        let glass_tree = self.scene.tree_create(tree);
        let buffer = self.scene.buffer_create(tree);
        self.panels.clipview = Some(ClipView {
            model,
            origin,
            scale,
            tree,
            glass_tree,
            glass: Backdrop::new(&self.glass),
            buffer,
        });
        self.panels.wheel = 0;
        self.restart_caret();
        self.redraw_clipview();
    }

    pub fn close_clipview(&mut self) {
        let Some(v) = self.panels.clipview.take() else {
            return;
        };
        self.scene.destroy(v.tree);
        self.end_panel_caret();
        self.redraw_all();
        crate::access::touch(self);
    }

    pub fn clipview_key(&mut self, k: &DialogKey) {
        let Some(key) = key_of(k) else {
            return;
        };
        // Copy and Cut offer the filter's selection, Paste asks for the
        // clipboard's text; it lands back here through `clipview_paste`.
        if let Key::Char(e) = &key {
            self.clip_edit_key(e);
        }
        let Some(v) = self.panels.clipview.as_mut() else {
            return;
        };
        let event = v.model.key(key);
        self.restart_caret();
        self.apply_clipview(event);
    }

    pub fn clipview_paste(&mut self, text: &str) {
        if let Some(v) = self.panels.clipview.as_mut() {
            v.model.paste(text);
            self.redraw_clipview();
        }
    }

    pub fn clipview_hit(&self, lx: f64, ly: f64) -> Hit {
        self.panels.clipview.as_ref().map_or(Hit::Panel, |v| {
            cliphistory::hit(
                &v.model,
                lx.floor() as i32 - v.origin.0,
                ly.floor() as i32 - v.origin.1,
                &self.config.theme.look.controls(),
            )
        })
    }

    pub fn clipview_clicked(&mut self, hit: Hit) {
        let Some(v) = self.panels.clipview.as_mut() else {
            return;
        };
        let event = v.model.click(hit);
        self.restart_caret();
        self.apply_clipview(event);
    }

    /// The wheel over the panel: `discrete` in 120ths of a notch, down
    /// positive, or a `continuous` amount from a source without notches.
    pub fn clipview_wheel(&mut self, discrete: i32, continuous: f64) {
        let (steps, residual) =
            crate::pointers::wheel_steps(discrete, continuous, self.panels.wheel);
        self.panels.wheel = residual;
        let Some(v) = self.panels.clipview.as_mut() else {
            return;
        };
        // `wheel_steps` counts up as positive.
        if steps != 0 && v.model.wheel(-steps * reprisekit::pointer::WHEEL_ROWS) {
            self.redraw_clipview();
        }
    }

    fn apply_clipview(&mut self, event: Event) {
        match event {
            Event::None => self.redraw_clipview(),
            Event::Remove(id) => {
                self.panels.history.history.remove(id);
                self.redraw_clipview();
            }
            Event::Pick(id) => {
                self.close_clipview();
                self.pick_clip(id);
            }
            Event::Close => self.close_clipview(),
        }
    }

    pub fn clip_thumb_ready(&mut self, id: u64, thumb: &Canvas) {
        if let Some(v) = self.panels.clipview.as_mut() {
            let icon = row_icon(thumb, v.scale);
            v.model.set_icon(id, icon);
            self.redraw_clipview();
        }
    }

    /// Put entry `id` back on the clipboard and paste it into the focused
    /// window. The selection change records it again, which moves it to
    /// the top.
    fn pick_clip(&mut self, id: u64) {
        let Some(offers) = self
            .panels
            .history
            .history
            .get(id)
            .map(|e| e.offers.clone())
        else {
            return;
        };
        self.clip_offer_each(offers);
        self.send_paste();
    }

    fn focused_is_terminal(&self) -> bool {
        if self.layers_shell.focused.is_some() {
            return false;
        }
        self.focused
            .and_then(|w| self.win(w))
            .is_some_and(|w| self.config.clipboard.terminal_apps.contains(&w.app_id()))
    }

    /// Send the focused client Ctrl+V (Ctrl+Shift+V for a terminal) as if
    /// typed, then give it back the keyboard's real modifiers. Nothing is
    /// sent when no client has the keyboard.
    fn send_paste(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if keyboard.current_focus().is_none() {
            return;
        }
        let Some((real, index)) = self.xkb_mask() else {
            return;
        };
        let (shift, ctrl) = (mod_bit(index[0]), mod_bit(index[2]));
        let mut held = real;
        held.depressed = ctrl;
        if self.focused_is_terminal() {
            held.depressed |= shift;
        }
        held.latched = 0;
        let code = keyboard
            .with_xkb_state(self, |ctx| {
                let x = ctx.xkb().lock().unwrap();
                // SAFETY: the state and keymap are only read; no reference
                // outlives the lock.
                let (s, km) = unsafe { (x.state(), x.keymap()) };
                let layout = s.serialize_layout(xkb::STATE_LAYOUT_EFFECTIVE);
                (km.min_keycode().raw().max(8)..=km.max_keycode().raw()).find(|&kc| {
                    km.key_get_syms_by_level(Keycode::new(kc), layout, 0)
                        .iter()
                        .any(|s| s.raw() == keys::KEY_v)
                })
            })
            .unwrap_or(EVDEV_V + 8);
        self.set_xkb_mask(held);
        for state in [KeyState::Pressed, KeyState::Released] {
            keyboard.input::<(), _>(
                self,
                Keycode::new(code),
                state,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_, _, _| FilterResult::Forward,
            );
        }
        self.set_xkb_mask(real);
    }

    pub fn redraw_clipview(&mut self) {
        let preedit = self.ime_field_preedit(crate::caret::Editor::ClipView);
        let State {
            panels,
            config,
            scene,
            text,
            caret,
            glass: glass_ctx,
            ..
        } = self;
        let Some(v) = panels.clipview.as_ref() else {
            return;
        };
        let theme = config.theme.scaled(v.scale);
        let mut canvas = cliphistory::draw(&theme, text, &v.model, caret.on, preedit.as_ref());
        let size = panel_size(&v.model, &config.theme);
        let pane = glass::flyout_glass(config, size);
        let Some(v) = panels.clipview.as_mut() else {
            return;
        };
        upload_scaled(scene, v.buffer, &canvas, v.scale);
        glass::place_flyout(scene, v.buffer, &config.theme, size);
        v.glass.set(scene, glass_ctx, v.glass_tree, pane);
        let m = reprisekit::flyout::margin(&config.theme);
        let (origin, scale) = ((v.origin.0 - m.left, v.origin.1 - m.top), v.scale);
        let spot = canvas.take_spot();
        self.ime_field_drawn(crate::caret::Editor::ClipView, spot, origin, scale);
        self.redraw_all();
        crate::access::touch(self);
    }

    pub fn clipview_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.panels.clipview.as_ref()?;
        let (x, y) = v.origin;
        let (w, h) = panel_size(&v.model, &self.config.theme);
        let rect = crate::raster::Rect::new(x, y, w, h);
        let mut tree = crate::access::root(Role::Window, "Clipboard History", rect);
        let px = self.config.theme.fonts.size_px;
        let cm = self.config.theme.look.controls();
        let focus = cliphistory::access(&v.model, &self.text, px, &mut tree, ROOT, &cm);
        tree.set_focus(focus);
        Some(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rows_thumbnail_is_centred_in_a_sixteen_pixel_square() {
        let mut wide = Canvas::new(32, 16);
        wide.fill_rect(wide.bounds(), reprisekit::theme::Color::rgb(9, 9, 9));
        let icon = row_icon(&wide, 2);
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.pixels[0] >> 24, 0, "the band above is clear");
        assert_eq!(icon.pixels[(16 * 32) as usize] >> 24, 0xff);
    }

    #[test]
    fn a_modifier_the_keymap_lacks_sets_no_bit() {
        assert_eq!(mod_bit(2), 0b100);
        assert_eq!(mod_bit(u32::MAX), 0);
    }
}
