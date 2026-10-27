use crate::access::{Tree, accesskit};
use crate::classic::widgets::access_edit;
use crate::controls::form::greyed_field_scheme;
use crate::look::Paint;
use crate::look::widgets::{draw_disabled_field, draw_edit_field, draw_edit_text};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{Caret, TextEdit, keysym};
use accesskit::NodeId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub logo: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub fn any(self) -> bool {
        self.logo || self.ctrl || self.alt || self.shift
    }

    fn union(self, o: Mods) -> Mods {
        Mods {
            logo: self.logo || o.logo,
            ctrl: self.ctrl || o.ctrl,
            alt: self.alt || o.alt,
            shift: self.shift || o.shift,
        }
    }

    fn names(self) -> impl Iterator<Item = &'static str> {
        [
            (self.logo, "Super"),
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
        ]
        .into_iter()
        .filter_map(|(on, n)| on.then_some(n))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Logo,
    Ctrl,
    Alt,
    Shift,
}

impl Modifier {
    fn of(sym: u32) -> Option<(Modifier, &'static str)> {
        Some(match sym {
            0xffe1 => (Modifier::Shift, "Shift_L"),
            0xffe2 => (Modifier::Shift, "Shift_R"),
            0xffe3 => (Modifier::Ctrl, "Control_L"),
            0xffe4 => (Modifier::Ctrl, "Control_R"),
            // Meta is Alt: xkb gives the left Alt key Meta_L under Shift.
            0xffe7 | 0xffe9 => (Modifier::Alt, "Alt_L"),
            0xffe8 | 0xffea => (Modifier::Alt, "Alt_R"),
            0xffeb => (Modifier::Logo, "Super_L"),
            0xffec => (Modifier::Logo, "Super_R"),
            _ => return None,
        })
    }

    fn mods(self) -> Mods {
        let mut m = Mods::default();
        match self {
            Modifier::Logo => m.logo = true,
            Modifier::Ctrl => m.ctrl = true,
            Modifier::Alt => m.alt = true,
            Modifier::Shift => m.shift = true,
        }
        m
    }
}

fn ignored(sym: u32) -> bool {
    matches!(
        sym,
        0xffe5 | 0xffe6 | 0xffed | 0xffee | 0xff7f | 0xff14 | 0xfe03 | 0xfe11
    )
}

fn is_character(sym: u32) -> bool {
    (0x21..=0x7e).contains(&sym) || (0xa0..=0xff).contains(&sym) || sym & 0xff00_0000 == 0x0100_0000
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub mods: Mods,
    pub key: String,
}

impl Chord {
    /// A config `keys` string ("Super+Shift+b") as a chord, the
    /// modifiers' aliases (logo, mod4, win, control, mod1) folded in.
    pub fn parse(keys: &str) -> Option<Chord> {
        let parts: Vec<&str> = keys.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last()?;
        if key.is_empty() {
            return None;
        }
        let mut m = Mods::default();
        for p in mods {
            match p.to_ascii_lowercase().as_str() {
                "super" | "logo" | "mod4" | "win" => m.logo = true,
                "ctrl" | "control" => m.ctrl = true,
                "alt" | "mod1" => m.alt = true,
                "shift" => m.shift = true,
                _ => return None,
            }
        }
        Some(Chord {
            mods: m,
            key: key.to_string(),
        })
    }

    pub fn keys(&self) -> String {
        let mut s: Vec<&str> = self.mods.names().collect();
        s.push(&self.key);
        s.join("+")
    }

    pub fn shown(&self) -> String {
        let key = match self.key.as_bytes() {
            [c] if c.is_ascii_alphabetic() => self.key.to_ascii_uppercase(),
            _ => self.key.clone(),
        };
        let mut s: Vec<&str> = self.mods.names().collect();
        s.push(&key);
        s.join(" + ")
    }

    pub fn same(&self, other: &Chord) -> bool {
        self.mods == other.mods && self.key.eq_ignore_ascii_case(&other.key)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    Pass,
    Redraw,
    Changed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HotKey {
    pub value: Option<Chord>,
    pub enabled: bool,
    down: Vec<(Modifier, &'static str)>,
    keyed: bool,
}

impl HotKey {
    pub fn new(value: Option<Chord>) -> HotKey {
        HotKey {
            value,
            enabled: true,
            ..HotKey::default()
        }
    }

    pub fn from_keys(keys: &str) -> HotKey {
        HotKey::new(Chord::parse(keys))
    }

    pub fn clear(&mut self) -> bool {
        self.down.clear();
        self.value.take().is_some()
    }

    pub fn blur(&mut self) {
        self.down.clear();
        self.keyed = false;
    }

    fn pending(&self) -> Option<Mods> {
        (!self.down.is_empty() && !self.keyed).then(|| {
            self.down
                .iter()
                .fold(Mods::default(), |m, (k, _)| m.union(k.mods()))
        })
    }

    /// `name` is xkb's name for the lower-case keysym. `held` may miss Super, so
    /// the field also counts the modifier keys it saw go down itself.
    pub fn key(&mut self, sym: u32, name: &str, held: Mods, down: bool) -> Took {
        if !self.enabled {
            return Took::Pass;
        }
        if let Some((m, mname)) = Modifier::of(sym) {
            return if down {
                self.modifier_down(m, mname)
            } else {
                self.modifier_up(m)
            };
        }
        if !down {
            return Took::Pass;
        }
        if ignored(sym) {
            return Took::Redraw;
        }
        let mods = self.down.iter().fold(held, |m, (k, _)| m.union(k.mods()));
        let only_shift = !(mods.logo || mods.ctrl || mods.alt);
        match sym {
            keysym::TAB | keysym::ISO_LEFT_TAB if only_shift => {
                self.blur();
                return Took::Pass;
            }
            keysym::RETURN | keysym::KP_ENTER | keysym::ESCAPE if !mods.any() => {
                return Took::Pass;
            }
            keysym::BACKSPACE | keysym::DELETE | keysym::KP_DELETE | 0x20 if !mods.any() => {
                self.keyed = true;
                return if self.value.take().is_some() {
                    Took::Changed
                } else {
                    Took::Redraw
                };
            }
            _ => {}
        }
        self.keyed = true;
        // A character key with no modifier but Shift would bind a typed key, so it
        // gets Super.
        let mods = Mods {
            logo: mods.logo || (only_shift && is_character(sym)),
            ..mods
        };
        let chord = Chord {
            mods,
            key: name.to_string(),
        };
        if self.value.as_ref() == Some(&chord) {
            return Took::Redraw;
        }
        self.value = Some(chord);
        Took::Changed
    }

    fn modifier_down(&mut self, m: Modifier, name: &'static str) -> Took {
        if self.down.is_empty() {
            self.keyed = false;
        }
        if !self.down.iter().any(|(k, n)| *k == m && *n == name) {
            self.down.push((m, name));
        }
        Took::Redraw
    }

    fn modifier_up(&mut self, m: Modifier) -> Took {
        let Some(at) = self.down.iter().position(|(k, _)| *k == m) else {
            // Held since before the field had the focus (the Alt of the access key
            // that focused it).
            return Took::Pass;
        };
        let mut took = Took::Redraw;
        if !self.keyed {
            self.keyed = true;
            let value = match self.down.split_last() {
                Some((last, rest)) if !rest.is_empty() => Some(Chord {
                    mods: rest
                        .iter()
                        .fold(Mods::default(), |a, (k, _)| a.union(k.mods())),
                    key: last.1.to_string(),
                }),
                _ => None,
            };
            if value != self.value {
                self.value = value;
                took = Took::Changed;
            }
        }
        self.down.remove(at);
        took
    }

    pub fn shown(&self, focused: bool) -> String {
        if focused && let Some(m) = self.pending() {
            return m.names().map(|n| format!("{n} + ")).collect();
        }
        match &self.value {
            Some(c) => c.shown(),
            None => "None".into(),
        }
    }

    pub fn keys(&self) -> String {
        self.value.as_ref().map(Chord::keys).unwrap_or_default()
    }
}

pub fn layout(r: Rect) -> Rect {
    r
}

pub fn hit(field: Rect, x: i32, y: i32) -> bool {
    field.contains(x, y)
}

fn as_edit(hk: &HotKey, focused: bool) -> TextEdit {
    TextEdit::new(hk.shown(focused), false)
}

#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    hk: &HotKey,
    field: Rect,
    focused: bool,
    caret: bool,
    ptr: Interaction,
    px: u32,
) {
    let r = field.scaled(p.s);
    let inner = if hk.enabled {
        let inner = draw_edit_field(c, p, r, p.cs.window, focused, ptr);
        c.fill_rect(inner, p.cs.window);
        inner
    } else {
        draw_disabled_field(c, p, r)
    };
    let focused = focused && hk.enabled;
    let greyed = greyed_field_scheme(p, hk.enabled);
    let ep = match &greyed {
        Some(cs) => Paint { cs, ..*p },
        None => *p,
    };
    draw_edit_text(
        c,
        text,
        &as_edit(hk, focused),
        inner,
        px,
        &ep,
        Caret::new(focused, caret),
        0,
    );
}

#[expect(clippy::too_many_arguments)]
pub fn access(
    hk: &HotKey,
    field: Rect,
    focused: bool,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    access_edit(
        &as_edit(hk, focused),
        0,
        false,
        field,
        hk.enabled,
        text,
        px,
        tree,
        parent,
        key,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    const SUPER_L: u32 = 0xffeb;
    const CTRL_L: u32 = 0xffe3;
    const ALT_L: u32 = 0xffe9;
    const SHIFT_L: u32 = 0xffe1;
    const NONE: Mods = Mods {
        logo: false,
        ctrl: false,
        alt: false,
        shift: false,
    };

    fn press(hk: &mut HotKey, sym: u32, name: &str, held: Mods) -> Took {
        hk.key(sym, name, held, true)
    }

    fn release(hk: &mut HotKey, sym: u32) -> Took {
        hk.key(sym, "", NONE, false)
    }

    #[test]
    fn keys_strings_round_trip_in_chord_order() {
        let c = Chord::parse("Shift + super+B").unwrap();
        assert_eq!(c.keys(), "Super+Shift+B");
        assert_eq!(c.shown(), "Super + Shift + B");
        assert!(c.same(&Chord::parse("Super+Shift+b").unwrap()));
        assert!(!c.same(&Chord::parse("Super+b").unwrap()));
        assert_eq!(
            Chord::parse("Mod4+Control+Left").unwrap().keys(),
            "Super+Ctrl+Left"
        );
        assert_eq!(Chord::parse("Print").unwrap().shown(), "Print");
        assert_eq!(Chord::parse("Hyper+x"), None);
        assert_eq!(Chord::parse("Super+"), None);
        assert_eq!(HotKey::from_keys("").value, None);
    }

    #[test]
    fn held_modifiers_show_until_the_key_then_the_chord() {
        let mut hk = HotKey::new(None);
        assert_eq!(hk.shown(true), "None");
        assert_eq!(press(&mut hk, SUPER_L, "Super_L", NONE), Took::Redraw);
        assert_eq!(hk.shown(true), "Super + ");
        let shift = Mods {
            shift: true,
            ..NONE
        };
        press(&mut hk, SHIFT_L, "Shift_L", NONE);
        assert_eq!(hk.shown(true), "Super + Shift + ");
        assert_eq!(hk.shown(false), "None", "unfocused, the value shows");
        assert_eq!(press(&mut hk, 0x42, "b", shift), Took::Changed);
        assert_eq!(hk.keys(), "Super+Shift+b");
        assert_eq!(hk.shown(true), "Super + Shift + B");
        assert_eq!(release(&mut hk, SHIFT_L), Took::Redraw);
        assert_eq!(release(&mut hk, SUPER_L), Took::Redraw);
        assert_eq!(hk.keys(), "Super+Shift+b");
    }

    #[test]
    fn a_lone_modifier_let_go_empties_the_field_and_two_make_a_chord() {
        let mut hk = HotKey::from_keys("Super+t");
        press(&mut hk, CTRL_L, "Control_L", NONE);
        assert_eq!(release(&mut hk, CTRL_L), Took::Changed);
        assert_eq!(hk.value, None);
        press(&mut hk, SHIFT_L, "Shift_L", NONE);
        press(&mut hk, ALT_L, "Alt_L", NONE);
        assert_eq!(release(&mut hk, ALT_L), Took::Changed);
        assert_eq!(hk.keys(), "Shift+Alt_L");
        assert_eq!(release(&mut hk, SHIFT_L), Took::Redraw);
        assert_eq!(
            hk.keys(),
            "Shift+Alt_L",
            "the second release changes nothing"
        );
        press(&mut hk, SHIFT_L, "Shift_L", NONE);
        press(&mut hk, 0xffe7, "Meta_L", NONE);
        release(&mut hk, SHIFT_L);
        assert_eq!(hk.keys(), "Shift+Alt_L");
    }

    #[test]
    fn a_modifier_held_from_before_the_focus_is_not_the_fields() {
        let mut hk = HotKey::from_keys("Super+t");
        let alt = Mods { alt: true, ..NONE };
        assert_eq!(release(&mut hk, ALT_L), Took::Pass);
        assert_eq!(hk.keys(), "Super+t");
        press(&mut hk, 0xffc2, "F5", alt);
        assert_eq!(hk.keys(), "Alt+F5");
    }

    #[test]
    fn clearing_keys_and_the_keys_the_dialog_keeps() {
        let mut hk = HotKey::from_keys("Super+t");
        assert_eq!(
            press(&mut hk, keysym::BACKSPACE, "BackSpace", NONE),
            Took::Changed
        );
        assert_eq!(hk.value, None);
        assert_eq!(press(&mut hk, keysym::DELETE, "Delete", NONE), Took::Redraw);
        hk.value = Chord::parse("Super+t");
        assert_eq!(press(&mut hk, 0x20, "space", NONE), Took::Changed);
        assert_eq!(hk.value, None);
        let shift = Mods {
            shift: true,
            ..NONE
        };
        for (sym, held) in [
            (keysym::TAB, NONE),
            (keysym::ISO_LEFT_TAB, shift),
            (keysym::RETURN, NONE),
            (keysym::ESCAPE, NONE),
        ] {
            assert_eq!(press(&mut hk, sym, "", held), Took::Pass, "{sym:#x}");
        }
        assert_eq!(release(&mut hk, 0x74), Took::Pass);
        let alt = Mods { alt: true, ..NONE };
        let ctrl = Mods { ctrl: true, ..NONE };
        press(&mut hk, keysym::TAB, "Tab", alt);
        assert_eq!(hk.keys(), "Alt+Tab");
        press(&mut hk, keysym::ESCAPE, "Escape", ctrl);
        assert_eq!(hk.keys(), "Ctrl+Escape");
        press(&mut hk, 0x20, "space", alt);
        assert_eq!(hk.keys(), "Alt+space");
        assert!(hk.clear());
        assert!(!hk.clear());
    }

    #[test]
    fn a_character_gets_super_and_other_keys_stand_alone() {
        let mut hk = HotKey::new(None);
        press(&mut hk, 0x71, "q", NONE);
        assert_eq!(hk.keys(), "Super+q");
        let shift = Mods {
            shift: true,
            ..NONE
        };
        press(&mut hk, 0x21, "exclam", shift);
        assert_eq!(hk.keys(), "Super+Shift+exclam");
        press(&mut hk, 0xff61, "Print", NONE);
        assert_eq!(hk.keys(), "Print");
        press(&mut hk, 0xffc2, "F5", shift);
        assert_eq!(hk.keys(), "Shift+F5");
        assert_eq!(press(&mut hk, 0xffe5, "Caps_Lock", NONE), Took::Redraw);
        assert_eq!(hk.keys(), "Shift+F5");
    }

    #[test]
    fn tab_forgets_the_held_shift() {
        let mut hk = HotKey::from_keys("Super+t");
        press(&mut hk, SHIFT_L, "Shift_L", NONE);
        let shift = Mods {
            shift: true,
            ..NONE
        };
        assert_eq!(press(&mut hk, keysym::ISO_LEFT_TAB, "", shift), Took::Pass);
        assert_eq!(hk.shown(true), "Super + T");
        assert_eq!(release(&mut hk, SHIFT_L), Took::Pass);
        assert_eq!(hk.keys(), "Super+t");
    }

    #[test]
    fn a_form_routes_keys_to_its_focused_field() {
        use crate::controls::form::{self, Control, Event, Form, Item};
        let mut f = Form::new(vec![
            Item::new(
                "keys",
                Rect::new(0, 0, 100, 20),
                Control::HotKey(HotKey::new(None)),
            ),
            Item::button("ok", Rect::new(0, 30, 75, 23), "OK", true, true),
        ]);
        assert_eq!(f.focus(), Some(0));
        assert_eq!(
            f.hotkey_key(SUPER_L, "Super_L", NONE, true),
            Some(Event::Redraw)
        );
        assert_eq!(
            f.hotkey_key(0x74, "t", NONE, true),
            Some(Event::Changed("keys"))
        );
        assert_eq!(
            f.hotkey_key(keysym::TAB, "Tab", NONE, true),
            Some(Event::Changed("keys"))
        );
        assert_eq!(
            f.hotkey_key(SUPER_L, "Super_L", NONE, false),
            Some(Event::Redraw)
        );
        let keys = |f: &Form| match &f.item("keys").unwrap().control {
            Control::HotKey(h) => h.keys(),
            _ => unreachable!(),
        };
        assert_eq!(keys(&f), "Super+Tab");
        assert_eq!(
            f.hotkey_key(keysym::TAB, "Tab", NONE, true),
            None,
            "a bare Tab"
        );
        assert_eq!(f.key(form::Key::Tab { back: false }), Event::Redraw);
        assert_eq!(f.focus(), Some(1));
        assert_eq!(
            f.hotkey_key(0x71, "q", NONE, true),
            None,
            "no field has the focus"
        );
        f.set_focus(Some(0));
        f.hotkey_key(CTRL_L, "Control_L", NONE, true);
        f.set_focus(Some(1));
        f.set_focus(Some(0));
        assert_eq!(f.hotkey_key(CTRL_L, "Control_L", NONE, false), None);
        assert_eq!(keys(&f), "Super+Tab");
    }

    #[test]
    fn a_greyed_field_takes_nothing() {
        let mut hk = HotKey::from_keys("Super+t");
        hk.enabled = false;
        assert_eq!(press(&mut hk, 0x71, "q", NONE), Took::Pass);
        assert_eq!(hk.keys(), "Super+t");
    }

    #[test]
    fn pixels_match_an_edit_field_with_the_caret_at_the_end() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let p = classic_paint(cs, 1);
        let r = Rect::new(4, 4, 140, 20);
        let mut hk = HotKey::new(None);
        let paint = |hk: &HotKey, focused: bool, caret: bool| {
            let mut c = Canvas::new(150, 30);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(
                &mut c,
                &text,
                &p,
                hk,
                layout(r),
                focused,
                caret,
                Interaction::Idle,
                11,
            );
            c
        };
        let c = paint(&hk, false, false);
        assert_eq!(c.get(4, 4), cs.button_shadow.argb(), "the outer top-left");
        assert_eq!(
            c.get(5, 5),
            cs.button_dk_shadow.argb(),
            "the inner top-left"
        );
        assert_eq!(
            c.get(143, 23),
            cs.button_hilight.argb(),
            "the outer bottom-right"
        );
        assert_eq!(c.get(7, 7), cs.window.argb(), "the well");
        let ink = |c: &Canvas| {
            (6..22)
                .flat_map(|y| (6..142).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
                .map(|p| p.0)
                .collect::<Vec<_>>()
        };
        let first = ink(&c).into_iter().min().unwrap();
        assert_eq!(first, 7, "\"None\" starts on the well's inner edge");

        press(&mut hk, CTRL_L, "Control_L", NONE);
        let shown = paint(&hk, true, true);
        let hidden = paint(&hk, true, false);
        let caret_x = 7 + text.measure("Ctrl + ", false, 11);
        let diff: Vec<(i32, i32)> = (0..30)
            .flat_map(|y| (0..150).map(move |x| (x, y)))
            .filter(|&(x, y)| shown.get(x, y) != hidden.get(x, y))
            .collect();
        assert_eq!(
            diff,
            (7..20).map(|y| (caret_x, y)).collect::<Vec<_>>(),
            "the caret column"
        );
        assert_eq!(shown.get(caret_x, 8), cs.button_text.argb());
        assert!(ink(&hidden).into_iter().max().unwrap() < caret_x);

        hk.enabled = false;
        let grey = paint(&hk, true, true);
        assert_eq!(grey.get(7, 7), cs.button_face.argb());
    }
}
