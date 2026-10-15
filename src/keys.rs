//! Key symbols and modifier bits for binds, the key route and the config.

use smithay::input::keyboard::ModifiersState;
pub use smithay::input::keyboard::keysyms::*;

pub const MOD_SHIFT: u32 = 1 << 0;
pub const MOD_CAPS: u32 = 1 << 1;
pub const MOD_CTRL: u32 = 1 << 2;
pub const MOD_ALT: u32 = 1 << 3;
/// Num Lock in the default keymaps.
pub const MOD_MOD2: u32 = 1 << 4;
pub const MOD_LOGO: u32 = 1 << 6;
pub const MOD_MOD5: u32 = 1 << 7;

/// The `MOD_*` mask of a keyboard's modifier state.
pub fn mask(m: &ModifiersState) -> u32 {
    [
        (m.shift, MOD_SHIFT),
        (m.caps_lock, MOD_CAPS),
        (m.ctrl, MOD_CTRL),
        (m.alt, MOD_ALT),
        (m.num_lock, MOD_MOD2),
        (m.logo, MOD_LOGO),
        (m.iso_level3_shift, MOD_MOD5),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .fold(0, |acc, (_, bit)| acc | bit)
}

// A pure function on a value; xkbcommon is linked through Smithay.
unsafe extern "C" {
    safe fn xkb_keysym_to_lower(ks: u32) -> u32;
}

/// The lowercase form of a keysym, as binds are matched.
pub fn to_lower(sym: u32) -> u32 {
    xkb_keysym_to_lower(sym)
}

/// The character a keysym types; None for a function key.
pub fn key_char(sym: u32) -> Option<char> {
    smithay::input::keyboard::Keysym::new(sym).key_char()
}

/// F13 to F24 by the keycode of the key, for the keys a keymap gives other keysyms (evdev's F22 is
/// XF86TouchpadOn): a bind on "F22" still fires from the key named F22.
pub fn function_key_for(xkb_code: u32) -> Option<u32> {
    // evdev KEY_F13..=KEY_F24 is 183..=194; xkb codes are evdev + 8.
    (191..=202)
        .contains(&xkb_code)
        .then(|| KEY_F13 + (xkb_code - 191))
}

/// The keysym a name like "Return" or "a" spells, matched without case; None for a name xkb does
/// not know.
pub fn from_name(name: &str) -> Option<u32> {
    use smithay::input::keyboard::xkb;
    let sym = xkb::keysym_from_name(name, xkb::KEYSYM_CASE_INSENSITIVE).raw();
    (sym != KEY_NoSymbol).then_some(sym)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_f13_to_f24_keycodes_name_their_function_keys() {
        assert_eq!(function_key_for(191), Some(KEY_F13));
        assert_eq!(function_key_for(200), Some(KEY_F22));
        assert_eq!(function_key_for(202), Some(KEY_F24));
        assert_eq!(function_key_for(190), None);
        assert_eq!(function_key_for(203), None);
    }

    #[test]
    fn lowercase_maps_letters_and_leaves_the_rest() {
        assert_eq!(to_lower(KEY_A), KEY_a);
        assert_eq!(to_lower(KEY_a), KEY_a);
        assert_eq!(to_lower(KEY_Return), KEY_Return);
    }

    #[test]
    fn names_resolve_without_case() {
        assert_eq!(from_name("return"), Some(KEY_Return));
        assert_eq!(from_name("F2"), Some(KEY_F2));
        assert_eq!(from_name("no-such-key"), None);
    }

    #[test]
    fn mask_sets_one_bit_per_modifier() {
        let m = ModifiersState {
            ctrl: true,
            alt: true,
            logo: true,
            ..Default::default()
        };
        assert_eq!(mask(&m), MOD_CTRL | MOD_ALT | MOD_LOGO);
        assert_eq!(mask(&ModifiersState::default()), 0);
    }
}
