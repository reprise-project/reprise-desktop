//! The emoji picker on the scene: one `reprisekit::controls::emojipicker` panel on the menu layer,
//! at the text caret when the focused client reports one, else at the pointer. A pick is typed
//! through a keymap with one key per code point, then the seat's own keymap and modifiers come
//! back; the clipboard is left alone.

use reprisekit::controls::emojipicker::{self, EmojiPicker, Event, Hit, Key, RECENT};
use reprisekit::emoji::{EmojiFont, Entry};
use reprisescene::{BufferId, TreeId};
use smithay::backend::input::{InputTime, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode};
use smithay::utils::SERIAL_COUNTER;

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::{self, Backdrop};
use crate::keys;
use crate::output::upload_scaled;

/// What outlives the panel: the font and catalog once loaded, and the recent picks.
#[derive(Default)]
pub struct Emoji {
    font: Option<Option<EmojiFont>>,
    catalog: Vec<Entry>,
    recent: Vec<&'static str>,
}

impl Emoji {
    fn font(&self) -> Option<&EmojiFont> {
        self.font.as_ref().and_then(Option::as_ref)
    }

    fn load(&mut self) {
        if self.font.is_none() {
            let font = EmojiFont::load();
            if font.is_none() {
                log::warn!("no colour emoji font found; the picker draws emoji as text");
            }
            self.catalog = reprisekit::emoji::catalog(font.as_ref());
            self.font = Some(font);
        }
    }

    fn used(&mut self, emoji: &'static str) {
        self.recent.retain(|&e| e != emoji);
        self.recent.insert(0, emoji);
        self.recent.truncate(RECENT);
    }
}

pub struct EmojiView {
    pub model: EmojiPicker,
    /// The panel's top-left in layout coordinates.
    pub origin: (i32, i32),
    /// The scale of the output it opened on.
    pub scale: i32,
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
}

/// The picker's key for a press the filter caught: Enter, Escape, the arrows and pages, or an edit
/// key for the filter; None for a key it does not use.
fn key_of(k: &DialogKey) -> Option<Key> {
    let special = k.syms.iter().find_map(|&sym| match sym {
        keys::KEY_Return | keys::KEY_KP_Enter => Some(Key::Enter),
        keys::KEY_Escape => Some(Key::Escape),
        keys::KEY_Left | keys::KEY_KP_Left => Some(Key::Left),
        keys::KEY_Right | keys::KEY_KP_Right => Some(Key::Right),
        keys::KEY_Up | keys::KEY_KP_Up => Some(Key::Up),
        keys::KEY_Down | keys::KEY_KP_Down => Some(Key::Down),
        keys::KEY_Page_Up | keys::KEY_KP_Page_Up => Some(Key::PageUp),
        keys::KEY_Page_Down | keys::KEY_KP_Page_Down => Some(Key::PageDown),
        _ => None,
    });
    special.or_else(|| k.edit.clone().map(Key::Char))
}

/// The xkb keymap source whose key `i` (xkb keycode `i + 9`, evdev `i + 1`) types `chars[i]`, each
/// as its Unicode keysym.
pub fn keymap_source(chars: &[char]) -> String {
    use std::fmt::Write;
    let mut s = String::from("xkb_keymap {\nxkb_keycodes \"reprise-emoji\" {\n");
    let _ = writeln!(s, "minimum = 8;\nmaximum = {};", chars.len() + 9);
    for i in 0..chars.len() {
        let _ = writeln!(s, "<K{i}> = {};", i + 9);
    }
    s.push_str("};\nxkb_types \"reprise-emoji\" { include \"complete\" };\n");
    s.push_str("xkb_compatibility \"reprise-emoji\" { include \"complete\" };\n");
    s.push_str("xkb_symbols \"reprise-emoji\" {\n");
    for (i, c) in chars.iter().enumerate() {
        let _ = writeln!(s, "key <K{i}> {{ [ U{:04X} ] }};", u32::from(*c));
    }
    s.push_str("};\n};\n");
    s
}

/// `text`'s distinct chars in order of first use, and the evdev code of each char of `text` on
/// `keymap_source` of them.
pub fn keys_for(text: &str) -> (Vec<char>, Vec<u32>) {
    let mut chars: Vec<char> = Vec::new();
    let codes = text
        .chars()
        .map(|c| {
            let i = chars.iter().position(|&k| k == c).unwrap_or_else(|| {
                chars.push(c);
                chars.len() - 1
            });
            i as u32 + 1
        })
        .collect();
    (chars, codes)
}

impl State {
    /// The bind: open, or close if open. Does nothing under the lock.
    pub fn toggle_emoji(&mut self) {
        if self.panels.emojiview.is_some() {
            self.close_emoji();
            return;
        }
        if self.is_locked() {
            return;
        }
        self.panels.emoji.load();
        let anchor = self.panel_anchor();
        let out = self.output_at(
            f64::from(anchor.x + anchor.width / 2),
            f64::from(anchor.y + anchor.height / 2),
        );
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let emoji = &self.panels.emoji;
        let recent = emoji.recent.iter().filter_map(|e| Entry::get(e)).collect();
        let model = EmojiPicker::new(recent, emoji.catalog.clone());
        let cm = self.config.theme.look.controls();
        let size = (emojipicker::width(&cm), emojipicker::layout(&cm).height);
        let origin = crate::textinput::popup_origin(anchor, size, self.usable_area(out.as_ref()));
        let tree = self.scene.tree_create(self.layers.menu);
        crate::glass::leave_out(&mut self.scene, tree);
        self.scene.set_data(tree, Some(SceneOwner::EmojiView));
        self.scene.set_position(tree, origin.0, origin.1);
        // Before the buffer, so it lies beneath it.
        let glass_tree = self.scene.tree_create(tree);
        let buffer = self.scene.buffer_create(tree);
        self.panels.emojiview = Some(EmojiView {
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
        self.redraw_emoji();
    }

    pub fn close_emoji(&mut self) {
        let Some(v) = self.panels.emojiview.take() else {
            return;
        };
        self.destroy_redrawn(v.tree);
        self.end_panel_caret();
        crate::access::touch(self);
    }

    /// A press the router sent here: Enter, Escape, the arrows and pages, or an edit key for the
    /// filter.
    pub fn emoji_key(&mut self, k: &DialogKey) {
        let Some(key) = key_of(k) else {
            return;
        };
        if let Key::Char(e) = &key {
            self.clip_edit_key(e);
        }
        let Some(v) = self.panels.emojiview.as_mut() else {
            return;
        };
        let event = v.model.key(key);
        self.restart_caret();
        self.apply_emoji(event);
    }

    /// Text from the clipboard into the filter.
    pub fn emoji_paste(&mut self, text: &str) {
        if let Some(v) = self.panels.emojiview.as_mut() {
            v.model.paste(text);
            self.redraw_emoji();
        }
    }

    /// What the layout point (`lx`, `ly`) lands on.
    pub fn emoji_hit(&self, lx: f64, ly: f64) -> Hit {
        self.panels.emojiview.as_ref().map_or(Hit::Panel, |v| {
            emojipicker::hit(
                &v.model,
                lx.floor() as i32 - v.origin.0,
                ly.floor() as i32 - v.origin.1,
                &self.config.theme.look.controls(),
            )
        })
    }

    /// A left click on the panel.
    pub fn emoji_clicked(&mut self, hit: Hit) {
        let Some(v) = self.panels.emojiview.as_mut() else {
            return;
        };
        let event = v.model.click(hit);
        self.restart_caret();
        self.apply_emoji(event);
    }

    /// The wheel over the panel: `discrete` in 120ths of a notch, down positive, or a `continuous`
    /// amount from a source without notches.
    pub fn emoji_wheel(&mut self, discrete: i32, continuous: f64) {
        let (steps, residual) =
            crate::pointers::wheel_steps(discrete, continuous, self.panels.wheel);
        self.panels.wheel = residual;
        if steps == 0 {
            return;
        }
        if let Some(v) = self.panels.emojiview.as_mut() {
            // `wheel_steps` counts up as positive.
            v.model.scroll(-i64::from(steps));
            self.redraw_emoji();
        }
    }

    fn apply_emoji(&mut self, event: Event) {
        match event {
            Event::None => self.redraw_emoji(),
            Event::Pick(emoji) => {
                self.close_emoji();
                self.panels.emoji.used(emoji);
                self.type_text(emoji);
            }
            Event::Close => self.close_emoji(),
        }
    }

    /// Nothing is sent when no client has the keyboard.
    fn type_text(&mut self, text: &str) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if keyboard.current_focus().is_none() {
            return;
        }
        let Some((mask, _)) = self.xkb_mask() else {
            return;
        };
        let Some(own) = self.keymap_text() else {
            return;
        };
        let (chars, codes) = keys_for(text);
        if let Err(e) = keyboard.set_keymap_from_string(self, keymap_source(&chars)) {
            log::warn!(
                "emoji picker: could not compile a keymap for {} characters: {e:?}",
                chars.len()
            );
            return;
        }
        for code in codes {
            for state in [KeyState::Pressed, KeyState::Released] {
                keyboard.input::<(), _>(
                    self,
                    Keycode::new(code + 8),
                    state,
                    SERIAL_COUNTER.next_serial(),
                    InputTime::now(),
                    |_, _, _| FilterResult::Forward,
                );
            }
        }
        if let Err(e) = keyboard.set_keymap_from_string(self, own) {
            log::warn!("emoji picker: could not restore the keymap: {e:?}");
        }
        self.set_xkb_mask(mask);
    }

    /// Redraw the panel. The caret blink lands here too.
    pub fn redraw_emoji(&mut self) {
        let was = (self.panels.emojiview.as_ref())
            .map(|v| self.scene.outputs_of(v.tree))
            .unwrap_or_default();
        let preedit = self.ime_field_preedit(crate::caret::Editor::EmojiView);
        let State {
            panels,
            config,
            scene,
            text,
            caret,
            glass: glass_ctx,
            ..
        } = self;
        let Some(v) = panels.emojiview.as_ref() else {
            return;
        };
        let theme = config.theme.scaled(v.scale);
        let font = panels.emoji.font();
        let mut canvas = emojipicker::draw(
            &theme,
            text.for_output(theme.scale),
            font,
            &v.model,
            caret.on,
            preedit.as_ref(),
        );
        let cm = config.theme.look.controls();
        let size = (emojipicker::width(&cm), emojipicker::layout(&cm).height);
        let pane = glass::flyout_glass(config, size);
        let Some(v) = panels.emojiview.as_mut() else {
            return;
        };
        upload_scaled(scene, v.buffer, &canvas, v.scale);
        glass::place_flyout(scene, v.buffer, &config.theme, size);
        v.glass.set(scene, glass_ctx, v.glass_tree, pane);
        let m = reprisekit::flyout::margin(&config.theme);
        let (origin, scale) = ((v.origin.0 - m.left, v.origin.1 - m.top), v.scale);
        let spot = canvas.take_spot();
        self.ime_field_drawn(crate::caret::Editor::EmojiView, spot, origin, scale);
        if let Some(tree) = self.panels.emojiview.as_ref().map(|v| v.tree) {
            self.redraw_nodes(was, &[tree.into()]);
        }
        crate::access::touch(self);
    }

    /// The picker's tree for a screen reader: the filter field and the grid, the highlighted emoji
    /// focused.
    pub fn emoji_access(&self) -> Option<reprisekit::access::Tree> {
        use reprisekit::access::{ROOT, accesskit::Role};
        let v = self.panels.emojiview.as_ref()?;
        let (x, y) = v.origin;
        let cm = self.config.theme.look.controls();
        let rect = crate::raster::Rect::new(
            x,
            y,
            emojipicker::width(&cm),
            emojipicker::layout(&cm).height,
        );
        let mut tree = crate::access::root(Role::Window, "Emoji", rect);
        let px = self.config.theme.fonts.size_px;
        let focus = emojipicker::access(&v.model, &self.text, px, &mut tree, ROOT, &cm);
        tree.set_focus(focus);
        Some(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::EditKey;
    use smithay::input::keyboard::xkb;

    #[test]
    fn each_char_gets_one_key_and_repeats_reuse_it() {
        let (chars, codes) = keys_for("👨\u{200d}👩\u{200d}👧");
        assert_eq!(chars, ['👨', '\u{200d}', '👩', '👧']);
        assert_eq!(codes, [1, 2, 3, 2, 4]);
    }

    #[test]
    fn the_keymap_maps_each_key_to_its_unicode_keysym_and_compiles() {
        let src = keymap_source(&['😀', '\u{fe0f}']);
        assert!(src.contains("<K0> = 9;"));
        assert!(src.contains("<K1> = 10;"));
        assert!(src.contains("key <K0> { [ U1F600 ] };"));
        assert!(src.contains("key <K1> { [ UFE0F ] };"));
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let km = xkb::Keymap::new_from_string(
            &ctx,
            src,
            xkb::KEYMAP_FORMAT_TEXT_V1,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .expect("the keymap compiles");
        let syms = km.key_get_syms_by_level(Keycode::new(9), 0, 0);
        assert_eq!(syms.len(), 1);
        assert_eq!(xkb::keysym_to_utf32(syms[0]), 0x1F600);
    }

    #[test]
    fn a_pick_moves_to_the_front_of_the_recent_ones() {
        let mut e = Emoji::default();
        e.used("😀");
        e.used("👍");
        e.used("😀");
        assert_eq!(e.recent, ["😀", "👍"]);
        let many: Vec<&'static str> = reprisekit::emoji::catalog(None)
            .iter()
            .take(RECENT + 5)
            .map(|e| e.emoji)
            .collect();
        for m in &many {
            e.used(m);
        }
        assert_eq!(e.recent.len(), RECENT);
        assert_eq!(e.recent[0], many[RECENT + 4]);
    }

    #[test]
    fn the_arrows_and_pages_win_over_the_filters_edit_key() {
        let press = |sym: u32| DialogKey {
            syms: vec![sym],
            mods: 0,
            edit: Some(EditKey::Insert("x".into())),
        };
        assert_eq!(key_of(&press(keys::KEY_KP_Enter)), Some(Key::Enter));
        assert_eq!(key_of(&press(keys::KEY_Left)), Some(Key::Left));
        assert_eq!(key_of(&press(keys::KEY_KP_Page_Down)), Some(Key::PageDown));
        assert_eq!(
            key_of(&press(keys::KEY_x)),
            Some(Key::Char(EditKey::Insert("x".into())))
        );
    }
}
