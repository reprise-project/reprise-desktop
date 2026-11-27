//! Keysyms by their xkb names, a key as its cap reads, and a chord's English spelling in the
//! user's language.

use std::ffi::{CString, c_char, c_int};
use std::sync::OnceLock;

const XKB_KEYSYM_CASE_INSENSITIVE: c_int = 1;

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_keysym_from_name(name: *const c_char, flags: c_int) -> u32;
    fn xkb_keysym_get_name(keysym: u32, buffer: *mut c_char, size: usize) -> c_int;
    fn xkb_keysym_to_lower(keysym: u32) -> u32;
    fn xkb_keysym_to_utf32(keysym: u32) -> u32;
}

pub fn key_known(name: &str) -> bool {
    let Ok(c) = CString::new(name) else {
        return false;
    };
    // SAFETY: `c` is a NUL-terminated string that outlives the call.
    unsafe { xkb_keysym_from_name(c.as_ptr(), XKB_KEYSYM_CASE_INSENSITIVE) != 0 }
}

pub fn key_name(sym: u32) -> Option<String> {
    let mut buf = [0 as c_char; 64];
    // SAFETY: xkb writes at most `buf.len()` bytes, NUL included, and
    // returns how many it wanted; -1 for an invalid keysym.
    let n = unsafe { xkb_keysym_get_name(xkb_keysym_to_lower(sym), buf.as_mut_ptr(), buf.len()) };
    if n <= 0 || n as usize >= buf.len() {
        return None;
    }
    // SAFETY: xkb NUL-terminated the name inside `buf`.
    let name = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) };
    name.to_str().ok().map(str::to_string)
}

pub fn keysym(name: &str) -> Option<u32> {
    let c = CString::new(name).ok()?;
    // SAFETY: `c` is a NUL-terminated string that outlives the call;
    // `xkb_keysym_to_lower` is a pure function on a keysym value.
    let sym = unsafe {
        xkb_keysym_to_lower(xkb_keysym_from_name(
            c.as_ptr(),
            XKB_KEYSYM_CASE_INSENSITIVE,
        ))
    };
    (sym != 0).then_some(sym)
}

/// Keysym `sym` as a key cap names it: a letter in capitals, a typed sign as
/// itself, a named key in words ("Page Down", "Print").
pub fn key_text(sym: u32) -> String {
    if let Some(c) = char::from_u32(sym).filter(|_| (0x21..=0x7e).contains(&sym)) {
        return c.to_ascii_uppercase().to_string();
    }
    let name = key_name(sym).unwrap_or_else(|| format!("{sym:#x}"));
    let fixed = match name.as_str() {
        "Return" | "return" => "Enter",
        "space" => "Space",
        "Escape" | "escape" => "Esc",
        "Sys_Req" | "sys_req" => "SysRq",
        "Prior" => "Page Up",
        "Next" => "Page Down",
        "Super_L" | "super_l" => "Left Super",
        "Super_R" | "super_r" => "Right Super",
        "Alt_L" | "alt_l" => "Left Alt",
        "Alt_R" | "alt_r" => "Right Alt",
        "Shift_L" | "shift_l" => "Left Shift",
        "Shift_R" | "shift_r" => "Right Shift",
        "Control_L" | "control_l" => "Left Ctrl",
        "Control_R" | "control_r" => "Right Ctrl",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let name = name.strip_prefix("XF86").unwrap_or(&name);
    // Underscores and case steps split words: "Page_Down", "AudioRaiseVolume".
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c == '_' {
            out.push(' ');
        } else {
            if c.is_uppercase() && prev.is_some_and(char::is_lowercase) {
                out.push(' ');
            }
            out.push(c);
        }
        prev = Some(c);
    }
    let mut chars = out.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The modifiers a chord's spelling may start with, each followed by "+".
const MODIFIERS: [&str; 4] = ["Ctrl", "Shift", "Alt", "Super"];

/// The shown names of a chord's words, the modifiers and the named keys, and the separator
/// between them. A word outside the table (a letter, a sign, F5) shows as written.
pub struct KeyNames {
    separator: String,
    words: Vec<(&'static str, String)>,
}

/// A [`KeyNames`] read through `$tr!("id")`: each id stays a literal scripts/i18n-check finds,
/// and a test can read a fixture catalog instead of reprisekit's strings.
macro_rules! key_names {
    ($tr:ident) => {
        KeyNames {
            separator: $tr!("key-separator"),
            words: vec![
                ("Ctrl", $tr!("key-ctrl")),
                ("Shift", $tr!("key-shift")),
                ("Alt", $tr!("key-alt")),
                ("Super", $tr!("key-super")),
                ("Del", $tr!("key-del")),
                ("Delete", $tr!("key-delete")),
                ("Ins", $tr!("key-ins")),
                ("Insert", $tr!("key-insert")),
                ("Enter", $tr!("key-enter")),
                ("Esc", $tr!("key-esc")),
                ("Escape", $tr!("key-escape")),
                ("Tab", $tr!("key-tab")),
                ("Space", $tr!("key-space")),
                ("Backspace", $tr!("key-backspace")),
                ("Home", $tr!("key-home")),
                ("End", $tr!("key-end")),
                ("PgUp", $tr!("key-pgup")),
                ("PgDn", $tr!("key-pgdn")),
                ("Page Up", $tr!("key-page-up")),
                ("Page Down", $tr!("key-page-down")),
                ("Left", $tr!("key-left")),
                ("Right", $tr!("key-right")),
                ("Up", $tr!("key-up")),
                ("Down", $tr!("key-down")),
                ("Plus", $tr!("key-plus")),
                ("Minus", $tr!("key-minus")),
                ("Back Space", $tr!("key-back-space")),
                ("Print", $tr!("key-print")),
                ("Caps Lock", $tr!("key-caps-lock")),
                ("Num Lock", $tr!("key-num-lock")),
                ("Scroll Lock", $tr!("key-scroll-lock")),
                ("SysRq", $tr!("key-sysrq")),
                ("Pause", $tr!("key-pause")),
                ("Menu", $tr!("key-menu")),
                ("Left Ctrl", $tr!("key-left-ctrl")),
                ("Right Ctrl", $tr!("key-right-ctrl")),
                ("Left Alt", $tr!("key-left-alt")),
                ("Right Alt", $tr!("key-right-alt")),
                ("Left Shift", $tr!("key-left-shift")),
                ("Right Shift", $tr!("key-right-shift")),
                ("Left Super", $tr!("key-left-super")),
                ("Right Super", $tr!("key-right-super")),
                ("Audio Mute", $tr!("key-audio-mute")),
                ("Audio Raise Volume", $tr!("key-audio-raise-volume")),
                ("Audio Lower Volume", $tr!("key-audio-lower-volume")),
                ("Audio Mic Mute", $tr!("key-audio-mic-mute")),
                ("Audio Play", $tr!("key-audio-play")),
                ("Audio Pause", $tr!("key-audio-pause")),
                ("Audio Stop", $tr!("key-audio-stop")),
                ("Audio Prev", $tr!("key-audio-prev")),
                ("Audio Next", $tr!("key-audio-next")),
                ("Mon Brightness Up", $tr!("key-mon-brightness-up")),
                ("Mon Brightness Down", $tr!("key-mon-brightness-down")),
                ("Kbd Brightness Up", $tr!("key-kbd-brightness-up")),
                ("Kbd Brightness Down", $tr!("key-kbd-brightness-down")),
                ("Power Off", $tr!("key-power-off")),
                ("Sleep", $tr!("key-sleep")),
            ],
        }
    };
}

impl KeyNames {
    /// `english`, a chord as Reprise spells it ("Ctrl+Shift+N", "Ctrl++", "Del", or the held
    /// "Ctrl+" with no key yet), with each word and separator in these names.
    pub fn shown(&self, english: &str) -> String {
        let mut out = String::new();
        let mut rest = english;
        while let Some((m, after)) = MODIFIERS
            .iter()
            .find_map(|m| Some((*m, rest.strip_prefix(m)?.strip_prefix('+')?)))
        {
            out += self.word(m);
            out += &self.separator;
            rest = after;
        }
        out + self.word(rest)
    }

    fn word<'a>(&'a self, english: &'a str) -> &'a str {
        self.words
            .iter()
            .find(|(e, _)| *e == english)
            .map_or(english, |(_, shown)| shown)
    }
}

/// [`KeyNames::shown`] in the user's language. Menus' accelerators, the shortcut box and the key
/// sheet show chords through it; config and files keep the English spelling.
pub fn shown_chord(english: &str) -> String {
    static NAMES: OnceLock<KeyNames> = OnceLock::new();
    NAMES.get_or_init(|| key_names!(tr)).shown(english)
}

/// Whether keysym `sym` types a character.
pub fn types_char(sym: u32) -> bool {
    // SAFETY: a pure function on a keysym value.
    unsafe { xkb_keysym_to_utf32(sym) != 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_are_the_lower_case_keysyms() {
        assert_eq!(key_name(0x42).as_deref(), Some("b"));
        assert_eq!(key_name(0x21).as_deref(), Some("exclam"));
        assert_eq!(key_name(0xff0d).as_deref(), Some("Return"));
        assert_eq!(key_name(0xffe9).as_deref(), Some("Alt_L"));
        assert_eq!(
            key_name(0x1008ff13).as_deref(),
            Some("XF86AudioRaiseVolume")
        );
    }

    #[test]
    fn caps_name_letters_signs_and_named_keys() {
        assert_eq!(key_text(0x62), "B");
        assert_eq!(key_text(0x3d), "=");
        assert_eq!(key_text(0xff0d), "Enter");
        assert_eq!(key_text(0xff55), "Page Up");
        assert_eq!(key_text(0xffe9), "Left Alt");
        assert_eq!(key_text(0x1008ff13), "Audio Raise Volume");
    }

    #[test]
    fn english_chords_show_as_spelled() {
        for c in [
            "Ctrl+Z",
            "Ctrl+Shift+N",
            "Ctrl++",
            "Ctrl+-",
            "Ctrl+Plus",
            "Alt+F4",
            "Alt+Enter",
            "Del",
            "F5",
            "Super",
            "Ctrl+",
            "Super+Shift+Page Up",
            "Shift+Left Alt",
            "Ctrl+Back Space",
            "Print",
            "Caps Lock",
            "Right Ctrl",
            "Audio Raise Volume",
            "Super+Shift+Escape",
        ] {
            assert_eq!(shown_chord(c), c);
        }
    }

    #[test]
    fn a_translation_names_modifiers_keys_and_the_separator() {
        use crate::i18n::Catalog;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i18n-keys");
        let de = "de".parse().unwrap();
        let cat = Catalog::load(
            "reprisekit",
            include_str!("../locale/en.ftl"),
            &[de],
            &[dir],
        );
        macro_rules! fixture {
            ($id:literal) => {
                cat.tr_args($id, &[])
            };
        }
        let names = key_names!(fixture);
        assert_eq!(names.shown("Ctrl+Z"), "Strg + Z");
        assert_eq!(names.shown("Ctrl+Shift+N"), "Strg + Umschalt + N");
        assert_eq!(names.shown("Ctrl++"), "Strg + +");
        assert_eq!(names.shown("Ctrl+Plus"), "Strg + Plus");
        assert_eq!(names.shown("Alt+Enter"), "Alt + Eingabe");
        assert_eq!(names.shown("Del"), "Entf");
        assert_eq!(
            names.shown("Super+Shift+Page Up"),
            "Super + Umschalt + Bild auf"
        );
        assert_eq!(names.shown("Ctrl+"), "Strg + ");
        assert_eq!(names.shown("Left Alt"), "Alt links");
        assert_eq!(names.shown("Print"), "Druck");
        assert_eq!(names.shown("F5"), "F5");
        assert_eq!(
            names.shown("Esc"),
            "Esc",
            "a word the translation lacks is English"
        );
    }
}
