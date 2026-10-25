use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::combo::Combo;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::hotkey::{Chord, HotKey};
use reprisekit::raster::Rect;

use repriseconf::keybind::NONE;

use crate::model::{ACTIONS, Shortcut, check_keys, check_program_keys, default_chord};

pub const CLIENT: (i32, i32) = (320, 160);

const MARGIN: i32 = 12;
const LABEL_W: i32 = 64;
const FIELD_X: i32 = MARGIN + LABEL_W + 4;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const ROW_Y: [i32; 3] = [MARGIN, MARGIN + 28, MARGIN + 56];
const LABEL_DY: i32 = 4;
const NOTE_Y: i32 = ROW_Y[2] + 28;
const CLEAR_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const KEYS_W: i32 = CLEAR_X - 6 - FIELD_X;

const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const OK_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

pub const KEYS: &str = "keys";
pub const CLEAR: &str = "clear";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";

const HINT: [&str; 2] = [
    "Press the keys in the field, as in Super+Shift+B.",
    "Backspace or Clear empties it.",
];

/// The action list: the compositor's actions, then "none", which unbinds a
/// default's chord.
fn choices() -> Vec<&'static str> {
    ACTIONS.iter().copied().chain([NONE]).collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Accept(Shortcut),
    Cancel,
}

pub struct ShortcutDialog {
    pub form: Form,
    pub title: &'static str,
    others: Vec<Shortcut>,
    program: bool,
}

fn label(id: &'static str, y: i32, text: &str) -> Item {
    Item::label(id, Rect::new(MARGIN, y + LABEL_DY, LABEL_W, 13), text)
}

fn edit(id: &'static str, y: i32, text: &str, enabled: bool) -> Item {
    Item::edit(id, Rect::new(FIELD_X, y, FIELD_W, 20), text, enabled)
}

fn button(id: &'static str, x: i32, y: i32, text: &str, default: bool) -> Item {
    Item::button(id, Rect::new(x, y, BUTTON_W, BUTTON_H), text, true, default)
}

impl ShortcutDialog {
    pub fn new(sc: Option<&Shortcut>, others: Vec<Shortcut>) -> ShortcutDialog {
        let (title, sc) = match sc {
            Some(sc) => ("Edit Shortcut", sc.clone()),
            None => ("Add Shortcut", Shortcut::new("", ACTIONS[1], "")),
        };
        let selected = (choices().iter())
            .position(|a| *a == sc.action)
            .unwrap_or(1);
        let items = vec![
            label("keys_l", ROW_Y[0], "&Keys:"),
            Item::new(
                KEYS,
                Rect::new(FIELD_X, ROW_Y[0], KEYS_W, 20),
                Control::HotKey(HotKey::from_keys(&sc.keys)),
            ),
            button(CLEAR, CLEAR_X, ROW_Y[0] - 2, "C&lear", false),
            label("action_l", ROW_Y[1], "&Action:"),
            Item::new(
                "action",
                Rect::new(FIELD_X, ROW_Y[1], FIELD_W, 21),
                Control::Combo(Combo::new(&choices(), selected, 8)),
            ),
            label("command_l", ROW_Y[2], "&Command:"),
            edit("command", ROW_Y[2], &sc.command, sc.action == "spawn"),
            Item::new(
                "note",
                Rect::new(MARGIN, NOTE_Y, CLIENT.0 - 2 * MARGIN, 2 * 13),
                Control::Lines(HINT.map(String::from).to_vec()),
            ),
            button(OK, OK_X, BTN_Y, "OK", true),
            button(CANCEL, CANCEL_X, BTN_Y, "Cancel", false),
        ];
        let mut d = ShortcutDialog {
            form: Form::new(items),
            title,
            others,
            program: false,
        };
        d.sync();
        d
    }

    pub fn for_program(keys: &str, others: Vec<Shortcut>) -> ShortcutDialog {
        let mut d = ShortcutDialog::new(Some(&Shortcut::new(keys, "", "")), others);
        d.program = true;
        d.title = "Change Shortcut";
        if let Some(i) = d.form.item_mut("action") {
            i.control = Control::Combo(Combo::new(&[""], 0, 1));
        }
        for id in ["action_l", "action", "command_l"] {
            d.form.set_enabled(id, false);
        }
        d.sync();
        d
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed(CLEAR) => {
                if let Some(h) = self.hotkey_mut() {
                    h.clear();
                }
                self.form.focus_id(KEYS);
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => {
                if self.valid() {
                    Outcome::Accept(self.value())
                } else {
                    Outcome::None
                }
            }
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// While true, the compositor's own shortcuts reach the field as keys.
    pub fn capturing(&self) -> bool {
        self.form.focus().is_some() && self.form.focus() == self.form.index_of(KEYS)
    }

    pub fn value(&self) -> Shortcut {
        let action = self.action();
        Shortcut {
            keys: self.hotkey().map(HotKey::keys).unwrap_or_default(),
            command: if action == "spawn" {
                self.field("command").trim().to_string()
            } else {
                String::new()
            },
            action,
        }
    }

    pub fn conflict(&self) -> Option<&Shortcut> {
        let chord = self.hotkey()?.value.as_ref()?;
        // A "none" row leaves its chord free.
        (self.others.iter())
            .filter(|o| o.action != NONE)
            .find(|o| Chord::parse(&o.keys).is_some_and(|c| c.same(chord)))
    }

    pub fn valid(&self) -> bool {
        let v = self.value();
        if self.program {
            return check_program_keys(&v.keys, "").is_ok() && self.conflict().is_none();
        }
        check_keys(&v.keys).is_ok()
            && self.conflict().is_none()
            && (v.action != "spawn" || !v.command.is_empty())
            && (v.action != NONE || default_chord(&v.keys))
    }

    /// The note's two lines: the taken keys' owner, keys the compositor
    /// would refuse (or, for a program, never give it), or the hint.
    fn note(&self) -> [String; 2] {
        let shown = self
            .hotkey()
            .and_then(|h| h.value.as_ref())
            .map(Chord::shown)
            .unwrap_or_default();
        if let Some(o) = self.conflict() {
            return [
                format!("{shown} is already the {} shortcut.", o.action),
                "Press other keys, or change that one first.".into(),
            ];
        }
        let keys = self.value().keys;
        let checked = if self.program {
            check_program_keys(&keys, &shown)
        } else {
            check_keys(&keys).map_err(String::from)
        };
        match checked {
            Err(why) if !keys.is_empty() => [why, String::new()],
            Ok(()) if !self.program && self.action() == NONE && !default_chord(&keys) => [
                "Only a default shortcut can be set to none.".into(),
                "Remove a shortcut you added instead.".into(),
            ],
            _ => HINT.map(String::from),
        }
    }

    fn sync(&mut self) {
        let spawn = self.action() == "spawn" && !self.program;
        let valid = self.valid();
        let has_keys = self.hotkey().is_some_and(|h| h.value.is_some());
        let note = self.note();
        self.form.set_enabled("command", spawn);
        self.form.set_lines("note", &note);
        let focus = self.form.focus();
        self.form.set_enabled(OK, valid);
        self.form.set_enabled(CLEAR, has_keys);
        // A greyed button is no tab stop, so the focus cannot stay on it.
        if focus.is_some_and(|i| self.form.items[i].id == OK && !valid) {
            self.form.focus_edge(false);
        }
    }

    fn hotkey(&self) -> Option<&HotKey> {
        match self.form.item(KEYS).map(|i| &i.control) {
            Some(Control::HotKey(h)) => Some(h),
            _ => None,
        }
    }

    fn hotkey_mut(&mut self) -> Option<&mut HotKey> {
        match self.form.item_mut(KEYS).map(|i| &mut i.control) {
            Some(Control::HotKey(h)) => Some(h),
            _ => None,
        }
    }

    fn action(&self) -> String {
        match self.form.item("action").map(|i| &i.control) {
            Some(Control::Combo(c)) => c
                .items
                .get(c.selected)
                .map(|i| i.text.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn field(&self, id: &str) -> &str {
        self.form.edit_text(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::hotkey::Mods;
    use reprisekit::textedit::EditKey;

    const SUPER_L: u32 = 0xffeb;
    const SHIFT_L: u32 = 0xffe1;

    fn typed(d: &mut ShortcutDialog, id: &str, text: &str) -> Outcome {
        d.form.focus_id(id);
        let ev = d.form.key(form::Key::Edit(EditKey::Insert(text.into())));
        d.event(ev)
    }

    /// Keys pressed in the keys field, in order, then let go in reverse:
    /// `(keysym, name)`, the modifiers first.
    fn chord(d: &mut ShortcutDialog, keys: &[(u32, &str)]) -> Outcome {
        d.form.focus_id(KEYS);
        let mut out = Outcome::None;
        for &(sym, name) in keys {
            if let Some(ev) = d.form.hotkey_key(sym, name, Mods::default(), true) {
                out = d.event(ev);
            }
        }
        for &(sym, name) in keys.iter().rev() {
            if let Some(ev) = d.form.hotkey_key(sym, name, Mods::default(), false) {
                d.event(ev);
            }
        }
        out
    }

    fn pick(d: &mut ShortcutDialog, action: &str) -> Outcome {
        if let Some(Control::Combo(c)) = d.form.item_mut("action").map(|i| &mut i.control) {
            c.selected = choices().iter().position(|a| *a == action).unwrap();
        }
        d.event(form::Event::Changed("action"))
    }

    fn enabled(d: &ShortcutDialog, id: &str) -> bool {
        match d.form.item(id).map(|i| &i.control) {
            Some(
                Control::Button { enabled, .. }
                | Control::Edit { enabled, .. }
                | Control::Label { enabled, .. },
            ) => *enabled,
            Some(Control::Combo(c)) => c.enabled,
            _ => false,
        }
    }

    fn note(d: &ShortcutDialog) -> Vec<String> {
        match d.form.item("note").map(|i| &i.control) {
            Some(Control::Lines(l)) => l.clone(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn add_starts_empty_with_ok_greyed_and_the_keys_field_focused() {
        let mut d = ShortcutDialog::new(None, Vec::new());
        assert_eq!(d.title, "Add Shortcut");
        assert_eq!(d.form.focus(), d.form.index_of(KEYS));
        assert!(d.capturing());
        assert!(!enabled(&d, OK));
        assert!(!enabled(&d, CLEAR), "nothing to clear");
        assert!(!enabled(&d, "command"), "terminal is no spawn");
        assert_eq!(d.event(form::Event::Default), Outcome::None);
    }

    #[test]
    fn a_pressed_chord_lights_ok_and_clear() {
        let mut d = ShortcutDialog::new(None, Vec::new());
        assert_eq!(
            chord(
                &mut d,
                &[(SUPER_L, "Super_L"), (SHIFT_L, "Shift_L"), (0x42, "b")]
            ),
            Outcome::Redraw
        );
        assert!(enabled(&d, OK));
        assert!(enabled(&d, CLEAR));
        assert_eq!(
            d.event(form::Event::Pressed(OK)),
            Outcome::Accept(Shortcut::new("Super+Shift+b", "terminal", ""))
        );
        d.form.focus_id(CLEAR);
        assert!(!d.capturing());
        d.event(form::Event::Pressed(CLEAR));
        assert_eq!(d.value().keys, "");
        assert!(d.capturing());
        assert!(!enabled(&d, OK));
        assert!(!enabled(&d, CLEAR));
    }

    #[test]
    fn keys_another_shortcut_has_grey_ok_and_say_whose_they_are() {
        let others = vec![
            Shortcut::new("Super+q", "close", ""),
            Shortcut::new("Alt+F4", "close", ""),
        ];
        let mut d = ShortcutDialog::new(None, others);
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x71, "q")]);
        assert_eq!(d.conflict().map(|o| o.keys.as_str()), Some("Super+q"));
        assert!(!enabled(&d, OK));
        assert_eq!(
            note(&d),
            [
                "Super + Q is already the close shortcut.",
                "Press other keys, or change that one first."
            ]
        );
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x77, "w")]);
        assert!(d.conflict().is_none());
        assert!(enabled(&d, OK));
        assert_eq!(note(&d), HINT);
    }

    #[test]
    fn editing_a_row_is_no_conflict_with_itself() {
        let sc = Shortcut::new("Super+q", "close", "");
        let mut d = ShortcutDialog::new(Some(&sc), Vec::new());
        assert_eq!(d.title, "Edit Shortcut");
        assert_eq!(d.value(), sc);
        assert!(enabled(&d, OK));
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn a_key_the_layout_does_not_name_says_so() {
        let sc = Shortcut::new("Super+NoSuchKey", "close", "");
        let d = ShortcutDialog::new(Some(&sc), Vec::new());
        assert!(!enabled(&d, OK));
        assert_eq!(
            note(&d)[0],
            "The key is not one the keyboard layout names, as in b, F5 or Return."
        );
    }

    #[test]
    fn spawn_enables_the_command_field_and_waits_for_it() {
        let mut d = ShortcutDialog::new(None, Vec::new());
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x74, "t")]);
        pick(&mut d, "spawn");
        assert!(enabled(&d, "command"));
        assert!(!enabled(&d, OK), "a spawn without a command");
        typed(&mut d, "command", "foot");
        assert!(enabled(&d, OK));
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Accept(Shortcut::new("Super+t", "spawn", "foot"))
        );
        pick(&mut d, "close");
        assert!(!enabled(&d, "command"));
        assert_eq!(d.value(), Shortcut::new("Super+t", "close", ""));
    }

    #[test]
    fn none_unbinds_a_default_chord_and_only_a_default_chord() {
        let sc = Shortcut::new("Super+q", "close", "");
        let mut d = ShortcutDialog::new(Some(&sc), Vec::new());
        pick(&mut d, "none");
        assert!(!enabled(&d, "command"));
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Accept(Shortcut::new("Super+q", "none", ""))
        );
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x78, "x")]);
        assert!(!enabled(&d, OK), "Super+x is no default");
        assert_eq!(note(&d)[0], "Only a default shortcut can be set to none.");
        pick(&mut d, "lock");
        assert!(enabled(&d, OK));
        assert_eq!(note(&d), HINT);
    }

    #[test]
    fn a_none_rows_keys_are_free_and_it_opens_on_none() {
        let none = Shortcut::new("Super+q", "none", "");
        let d = ShortcutDialog::new(Some(&none), Vec::new());
        assert_eq!(d.value(), none);
        let mut d = ShortcutDialog::new(None, vec![none]);
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x71, "q")]);
        assert!(d.conflict().is_none());
        assert!(enabled(&d, OK));
    }

    #[test]
    fn a_program_shortcut_edits_only_its_keys_and_may_have_none() {
        let mut d = ShortcutDialog::for_program("Pause", vec![Shortcut::new("Super+r", "run", "")]);
        assert_eq!(d.title, "Change Shortcut");
        assert!(!enabled(&d, "action"), "the action is the program's");
        for id in ["action_l", "command_l", "command"] {
            assert!(!enabled(&d, id), "{id} is greyed");
        }
        assert!(d.valid());
        assert!(enabled(&d, CLEAR));
        d.event(form::Event::Pressed(CLEAR));
        assert_eq!(d.value().keys, "");
        assert!(d.valid());
        assert!(enabled(&d, OK));
        assert!(!enabled(&d, CLEAR), "nothing left to clear");
        chord(&mut d, &[(SUPER_L, "Super_L"), (0x72, "r")]);
        assert!(!d.valid());
    }

    /// A program never gets a chord of modifiers alone or keys the desktop
    /// keeps: the compositor would leave either unassigned.
    #[test]
    fn a_program_shortcut_refuses_modifiers_alone_and_kept_keys() {
        const CTRL_L: u32 = 0xffe3;
        const ALT_L: u32 = 0xffe9;
        let mut d = ShortcutDialog::for_program("", Vec::new());
        chord(&mut d, &[(CTRL_L, "Control_L"), (SHIFT_L, "Shift_L")]);
        assert_eq!(d.value().keys, "Ctrl+Shift_L");
        assert!(!d.valid());
        assert!(!enabled(&d, OK));
        assert_eq!(
            note(&d)[0],
            "A program shortcut ends in a key that is not a modifier."
        );
        chord(
            &mut d,
            &[(CTRL_L, "Control_L"), (ALT_L, "Alt_L"), (0xffbe, "F1")],
        );
        assert_eq!(d.value().keys, "Ctrl+Alt+F1");
        assert!(!d.valid());
        assert_eq!(note(&d)[0], "Ctrl + Alt + F1 is kept by the desktop.");
        let mut d = ShortcutDialog::new(None, Vec::new());
        chord(&mut d, &[(CTRL_L, "Control_L"), (SHIFT_L, "Shift_L")]);
        assert!(d.valid(), "a bind of modifiers alone fires on release");
    }

    #[test]
    #[ignore = "writes a preview PNG"]
    fn preview() {
        use reprisekit::raster::{Canvas, write_preview};
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let others = vec![Shortcut::new("Super+q", "close", "")];
        let mut out = Canvas::new((CLIENT.0 * 2 + 8) as u32, CLIENT.1 as u32);
        out.fill_rect(out.bounds(), theme.colors.background);
        for (k, x) in [0, CLIENT.0 + 8].into_iter().enumerate() {
            let mut d = ShortcutDialog::new(None, others.clone());
            if k == 0 {
                d.form
                    .hotkey_key(0xffe3, "Control_L", Mods::default(), true);
            } else {
                chord(&mut d, &[(SUPER_L, "Super_L"), (0x71, "q")]);
            }
            d.form.caret = true;
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            form::draw(&mut c, &text, &theme.paint(), &d.form, true, 11);
            out.blit_canvas(x, 0, &c);
        }
        write_preview(&out, "shortcut-dialog.png");
    }

    #[test]
    fn everything_sits_inside_the_client() {
        let d = ShortcutDialog::new(None, Vec::new());
        let c = Rect::new(0, 0, CLIENT.0, CLIENT.1);
        for i in &d.form.items {
            let r = i.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= c.right() && r.bottom() <= c.bottom(),
                "{} at {r:?}",
                i.id
            );
        }
        assert_eq!(d.form.item(KEYS).unwrap().rect, Rect::new(80, 12, 147, 20));
        assert_eq!(d.form.item(CLEAR).unwrap().rect, Rect::new(233, 10, 75, 23));
        assert_eq!(d.form.item(OK).unwrap().rect, Rect::new(152, 125, 75, 23));
        assert_eq!(
            d.form.item(CANCEL).unwrap().rect,
            Rect::new(233, 125, 75, 23)
        );
    }
}
