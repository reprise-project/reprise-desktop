//! The emoji picker on the scene: one `reprisekit::controls::emojipicker` panel on the menu layer,
//! at the text caret when the focused client reports one, else at the pointer. With no input
//! method running, the bind enters the client's text input until it reports. A pick is typed
//! through a keymap with one key per code point, then the seat's own keymap and modifiers come
//! back; the clipboard is left alone.

use reprisekit::controls::emojipicker::{self, EmojiPicker, Event, Hit, Key, RECENT};
use reprisekit::emoji::{EmojiFont, Entry};
use reprisekit::tr;
use reprisescene::{BufferId, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::rc::Rc;
use std::time::Duration;

use crate::compositor::{SceneOwner, State};
use crate::dialogview::DialogKey;
use crate::glass::{self, Backdrop};
use crate::input::Interaction;
use crate::keys;
use crate::output::upload_scaled;
use crate::raster::Rect;

/// How long Super+. waits for the focused client's caret before opening at the pointer.
const CARET_WAIT: Duration = Duration::from_millis(100);

/// What outlives the panel: the font and catalog once loaded, and the recent picks.
#[derive(Default)]
pub struct Emoji {
    font: Option<Option<Rc<EmojiFont>>>,
    catalog: Vec<Entry>,
    recent: Vec<&'static str>,
}

impl Emoji {
    fn font(&self) -> Option<&EmojiFont> {
        self.font.as_ref().and_then(Option::as_deref)
    }

    fn load(&mut self) {
        if self.font.is_none() {
            let font = EmojiFont::shared();
            if font.is_none() {
                log::warn!("no colour emoji font found; the picker draws emoji as text");
            }
            self.catalog = reprisekit::emoji::catalog(font.as_deref());
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
    /// The output it opened on; None with no output.
    output: Option<Output>,
    pub(crate) tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
}

pub fn open_on(state: &State) -> Option<Output> {
    state.panels.emojiview.as_ref()?.output.clone()
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
        if self.panels.emojiview.is_some() || self.panels.emoji_wait.is_some() {
            self.close_emoji();
            return;
        }
        if self.desktop_locked() {
            return;
        }
        if !self.wait_for_caret() {
            self.open_emoji(self.panel_anchor());
        }
    }

    /// With no input method running, a client reports its caret only once its text input is
    /// entered: enter it and open on the report, or at the pointer after [`CARET_WAIT`].
    fn wait_for_caret(&mut self) -> bool {
        use smithay::reexports::wayland_server::Resource;
        use smithay::wayland::{input_method::InputMethodSeat, text_input::TextInputSeat};
        let ti = self.seat.text_input();
        if self.text_caret().is_some()
            || ti.compositor_input_method()
            || self.seat.input_method().has_instance()
        {
            return false;
        }
        let Some(surface) = ti.focus().filter(|s| s.is_alive()) else {
            return false;
        };
        if !crate::textinput::has_text_input(self, &surface) {
            return false;
        }
        let timer = self
            .handle
            .insert_source(Timer::from_duration(CARET_WAIT), |_, _, state| {
                state.panels.emoji_wait = None;
                state.sync_caret_reports();
                state.open_emoji(state.panel_anchor());
                TimeoutAction::Drop
            });
        let Ok(token) = timer else {
            return false;
        };
        self.panels.emoji_wait = Some(token);
        self.sync_caret_reports();
        true
    }

    /// A text input committed: the caret the picker waits for, if it now has one.
    pub fn emoji_caret_heard(&mut self) {
        if self.panels.emoji_wait.is_none() {
            return;
        }
        let Some(caret) = self.text_caret() else {
            return;
        };
        self.end_caret_wait();
        self.open_emoji(caret);
    }

    fn end_caret_wait(&mut self) -> bool {
        let Some(token) = self.panels.emoji_wait.take() else {
            return false;
        };
        self.handle.remove(token);
        self.sync_caret_reports();
        true
    }

    fn open_emoji(&mut self, anchor: Rect) {
        if self.desktop_locked() {
            return;
        }
        self.panels.emoji.load();
        let out = self.output_at(
            f64::from(anchor.x + anchor.width / 2),
            f64::from(anchor.y + anchor.height / 2),
        );
        let scale = self.draw_scale_at(out.as_ref()).max(1);
        let emoji = &self.panels.emoji;
        let recent = emoji.recent.iter().filter_map(|e| Entry::get(e)).collect();
        let model = EmojiPicker::new(recent, emoji.catalog.clone());
        let cm = self.config.theme.controls();
        let size = (emojipicker::width(&cm), emojipicker::layout(&cm).height);
        let origin = crate::textinput::popup_origin(anchor, size, self.usable_area(out.as_ref()));
        let tree = self.scene.tree_create(self.layers.menu);
        crate::glass::leave_out(&mut self.scene, tree);
        if let Some(o) = &out {
            crate::output::keep_on(&mut self.scene, tree, o);
        }
        self.scene.set_data(tree, Some(SceneOwner::EmojiView));
        self.scene.set_position(tree, origin.0, origin.1);
        // Before the buffer, so it lies beneath it.
        let glass_tree = self.scene.tree_create(tree);
        let buffer = self.scene.buffer_create(tree);
        self.panels.emojiview = Some(EmojiView {
            model,
            origin,
            scale,
            output: out,
            tree,
            glass_tree,
            glass: Backdrop::new(&self.glass),
            buffer,
        });
        self.panels.wheel = Default::default();
        self.restart_caret();
        self.redraw_emoji();
    }

    pub fn close_emoji(&mut self) {
        if self.end_caret_wait() {
            return;
        }
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
                &self.config.theme.controls(),
            )
        })
    }

    /// A left click on the panel.
    pub fn emoji_clicked(&mut self, hit: Hit) {
        let Some(v) = self.panels.emojiview.as_mut() else {
            return;
        };
        let event = v.model.click(hit);
        if v.model.holding() {
            self.interaction = Interaction::PanelBar;
            self.repeat_panel_bar(|state, now| {
                let Some(v) = state.panels.emojiview.as_mut() else {
                    return false;
                };
                if v.model.tick(now) {
                    state.redraw_emoji();
                }
                state
                    .panels
                    .emojiview
                    .as_ref()
                    .is_some_and(|v| v.model.holding())
            });
        }
        self.restart_caret();
        self.apply_emoji(event);
    }

    /// The pointer at layout (`lx`, `ly`) with the panel's scrollbar held.
    pub fn emoji_drag(&mut self, lx: f64, ly: f64) {
        let cm = self.config.theme.controls();
        let Some(v) = self.panels.emojiview.as_mut() else {
            return;
        };
        let (x, y) = (
            lx.floor() as i32 - v.origin.0,
            ly.floor() as i32 - v.origin.1,
        );
        if v.model.drag(x, y, &cm) {
            self.redraw_emoji();
        }
    }

    pub fn emoji_release(&mut self) {
        if let Some(v) = self.panels.emojiview.as_mut() {
            v.model.release();
        }
    }

    /// The wheel over the panel: `discrete` in 120ths of a notch, down positive, or a `continuous`
    /// amount from a source without notches.
    pub fn emoji_wheel(&mut self, discrete: i32, continuous: f64) {
        let notches = self.panels.wheel.notches(discrete, continuous);
        if notches == 0 {
            return;
        }
        if let Some(v) = self.panels.emojiview.as_mut() {
            v.model.scroll(i64::from(notches));
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

    /// Type `text` to the keyboard's client through a keymap made for it. Nothing is sent when no
    /// client has the keyboard.
    pub(crate) fn type_text(&mut self, text: &str) {
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
        self.tap_past_ime(codes.into_iter().map(|c| c + 8));
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
        let cm = config.theme.controls();
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
        let cm = self.config.theme.controls();
        let rect = crate::raster::Rect::new(
            x,
            y,
            emojipicker::width(&cm),
            emojipicker::layout(&cm).height,
        );
        let mut tree = crate::access::root(Role::Window, &tr!("dlg-emoji-title"), rect);
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
    use smithay::input::keyboard::{Keycode, xkb};

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
    fn an_emoji_typed_under_an_input_methods_grab_reaches_the_client_alone() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        h.map_toplevel(&mut c, "test.app", 300, 200);
        let mut m = h.grabbing_input_method();
        h.roundtrip(&mut c);
        c.side.keys.clear();
        h.state.type_text("😀");
        h.roundtrip(&mut c);
        h.roundtrip(&mut m);
        assert_eq!(c.side.keys, [(1, true), (1, false)]);
        let im = &m.side.ims[0];
        assert!(im.grab_keys.is_empty(), "the grab heard {:?}", im.grab_keys);
    }

    /// A focused window whose client made a text input, and the pointer parked away from it.
    fn window_with_text_input() -> (crate::harness::Harness, crate::harness::TestClient, usize) {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let ti = c.text_input();
        h.map_toplevel(&mut c, "test.app", 300, 200);
        h.pointer_to(1000.0, 740.0);
        (h, c, ti)
    }

    fn reports_caret(h: &crate::harness::Harness) -> bool {
        use smithay::wayland::text_input::TextInputSeat;
        h.state.seat.text_input().compositor_input_method()
    }

    #[test]
    fn with_no_input_method_the_picker_opens_at_the_caret_the_client_reports() {
        let (mut h, mut c, ti) = window_with_text_input();
        h.state.toggle_emoji();
        assert!(h.state.panels.emojiview.is_none(), "it waits for the caret");
        assert!(reports_caret(&h), "the client's text input is entered");
        let text_input = &c.side.text_inputs[ti];
        text_input.enable();
        text_input.set_cursor_rectangle(40, 30, 0, 18);
        text_input.commit();
        h.roundtrip(&mut c);
        let window = h.state.windows[0].wl_surface().clone();
        let scene = &h.state.scene;
        let (at, _) = scene
            .coords(scene.surface_node(&window).expect("the window's node"))
            .expect("the window is placed");
        let caret = Rect::new(at.x + 40, at.y + 30, 0, 18);
        let cm = h.state.config.theme.controls();
        let size = (emojipicker::width(&cm), emojipicker::layout(&cm).height);
        let usable = h
            .state
            .usable_area(h.state.output_at(500.0, 400.0).as_ref());
        let v = h.state.panels.emojiview.as_ref().expect("the picker opens");
        assert_eq!(
            v.origin,
            crate::textinput::popup_origin(caret, size, usable),
            "the picker sits at the caret"
        );
        assert!(!reports_caret(&h), "the text input is left once heard");
    }

    #[test]
    fn a_client_that_reports_no_caret_gets_the_picker_at_the_pointer() {
        let (mut h, _c, _) = window_with_text_input();
        h.state.toggle_emoji();
        h.until("the picker opens", |s| s.panels.emojiview.is_some());
        let cm = h.state.config.theme.controls();
        let size = (emojipicker::width(&cm), emojipicker::layout(&cm).height);
        let usable = h
            .state
            .usable_area(h.state.output_at(500.0, 400.0).as_ref());
        let pointer = Rect::new(1000, 740, 1, 1);
        assert_eq!(
            h.state.panels.emojiview.as_ref().map(|v| v.origin),
            Some(crate::textinput::popup_origin(pointer, size, usable)),
            "the picker sits at the pointer"
        );
        assert!(!reports_caret(&h), "the text input is left");
    }

    /// An input method that binds while the picker waits keeps the client's text input entered
    /// once the wait ends, so the text input it activated on stays the one it types into.
    #[test]
    fn an_input_method_bound_during_the_wait_keeps_the_text_input_entered() {
        let (mut h, mut c, ti) = window_with_text_input();
        h.state.toggle_emoji();
        h.roundtrip(&mut c);
        assert!(c.side.text_input_entered[ti], "entered for the wait");
        let mut m = h.connect();
        m.input_method();
        h.roundtrip(&mut m);
        let text_input = &c.side.text_inputs[ti];
        text_input.enable();
        text_input.commit();
        h.roundtrip(&mut c);
        h.roundtrip(&mut m);
        h.until("the picker opens", |s| s.panels.emojiview.is_some());
        h.roundtrip(&mut c);
        assert!(!reports_caret(&h), "the compositor stops reporting");
        assert!(
            c.side.text_input_entered[ti],
            "the text input stays entered under the input method"
        );
    }

    #[test]
    fn a_second_press_while_waiting_opens_nothing() {
        let (mut h, _c, _) = window_with_text_input();
        h.state.toggle_emoji();
        h.state.toggle_emoji();
        assert!(!reports_caret(&h), "the text input is left");
        h.run_for(2 * CARET_WAIT.as_millis() as u64);
        assert!(h.state.panels.emojiview.is_none(), "no picker opens");
    }

    #[test]
    fn typing_an_emoji_gives_the_client_back_its_keymap_and_layout() {
        let (mut h, mut c) = crate::harness::Harness::on_second_layout();
        let seat = h.state.keymap_text().expect("the seat's keymap");
        h.state.type_text("😀");
        h.roundtrip(&mut c);
        assert_eq!(c.side.keys, [(1, true), (1, false)]);
        assert_eq!(c.side.keymap_text(), seat);
        assert_eq!(
            h.layout_heard(&mut c),
            (1, 1, 1),
            "de, and the client hears it"
        );
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
