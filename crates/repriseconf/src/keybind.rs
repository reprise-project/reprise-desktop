//! `[[keybinds]]` as the settings sheet, portal and compositor agree on: the defaults a file's list
//! layers over, chord comparison, and the `keys` check (a key xkb does not know fails the whole
//! config in the compositor, so the sheet refuses it first).

use std::ffi::{CString, c_char, c_int};

pub const MODIFIERS: &[&str] = &[
    "super", "logo", "mod4", "win", "ctrl", "control", "alt", "mod1", "shift",
];

const XKB_KEYSYM_CASE_INSENSITIVE: c_int = 1;

#[link(name = "xkbcommon")]
unsafe extern "C" {
    fn xkb_keysym_from_name(name: *const c_char, flags: c_int) -> u32;
    fn xkb_keysym_get_name(keysym: u32, buffer: *mut c_char, size: usize) -> c_int;
    fn xkb_keysym_to_lower(keysym: u32) -> u32;
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

pub fn check(keys: &str) -> Result<(), &'static str> {
    let parts: Vec<&str> = keys.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last().ok_or(ENDS_IN_A_KEY)?;
    if key.is_empty() {
        return Err(ENDS_IN_A_KEY);
    }
    if mods
        .iter()
        .any(|m| !MODIFIERS.contains(&m.to_ascii_lowercase().as_str()))
    {
        return Err("The modifiers are Super, Ctrl, Alt and Shift.");
    }
    if !key_known(key) {
        return Err("The key is not one the keyboard layout names, as in b, F5 or Return.");
    }
    Ok(())
}

const ENDS_IN_A_KEY: &str = "A shortcut ends in a key, as in Super+Shift+b.";

pub const DEFAULTS: &[(&str, &str)] = &[
    ("Super+Return", "terminal"),
    ("Super+q", "close"),
    ("Alt+F4", "close"),
    ("Alt+Tab", "cycle"),
    ("Super+Tab", "flip"),
    ("Super+Shift+e", "exit"),
    ("Super+l", "lock"),
    ("Ctrl+Escape", "startmenu"),
    ("Super+r", "run"),
    ("Alt+space", "windowmenu"),
    ("Super+Shift+b", "toggleframe"),
    ("Super+space", "launcher"),
    ("Super+c", "center"),
    ("Super+Left", "snap-left"),
    ("Super+Right", "snap-right"),
    ("Super+Up", "snap-up"),
    ("Super+Down", "snap-down"),
    ("Super+Ctrl+Left", "desktop-prev"),
    ("Super+Ctrl+Right", "desktop-next"),
    ("Super+Ctrl+Shift+Left", "desktop-carry-prev"),
    ("Super+Ctrl+Shift+Right", "desktop-carry-next"),
    ("Print", "screenshot"),
    ("Alt+Sys_Req", "screenshot-window"),
    ("Super+Shift+w", "screenshot-window"),
    ("Shift+Print", "screenshot-region"),
    ("Ctrl+Alt+Sys_Req", "screenshot-content"),
    ("Super+v", "clipboard-history"),
    ("Super+period", "emoji-picker"),
    ("Super+equal", "zoom-in"),
    ("Super+minus", "zoom-out"),
    ("Super+0", "zoom-reset"),
    ("Super+Ctrl+c", "color-filter"),
    ("Super+w", "overview"),
    ("Super+z", "snap-layouts"),
    ("Super+t", "focus-taskbar"),
    ("Super+b", "focus-tray"),
    ("Shift+Alt_L", "next-layout"),
    ("Super+grave", "tab-next"),
    ("Super+Page_Down", "tab-next"),
    ("Super+Page_Up", "tab-prev"),
];

pub const MODIFIER_KEYS: &[&str] = &[
    "Shift_L",
    "Shift_R",
    "Control_L",
    "Control_R",
    "Alt_L",
    "Alt_R",
    "Meta_L",
    "Meta_R",
    "Super_L",
    "Super_R",
];

const MACHINE_KEYS: &[&str] = &[
    "XF86PowerOff",
    "XF86Sleep",
    "XF86AudioRaiseVolume",
    "XF86AudioLowerVolume",
    "XF86AudioMute",
    "XF86AudioMicMute",
    "XF86MonBrightnessUp",
    "XF86MonBrightnessDown",
    "XF86KbdBrightnessUp",
    "XF86KbdBrightnessDown",
    "XF86AudioPlay",
    "XF86AudioPause",
    "XF86AudioNext",
    "XF86AudioPrev",
    "XF86AudioStop",
    "XF86WLAN",
];

fn split(keys: &str) -> Option<(Vec<&'static str>, &str)> {
    let parts: Vec<&str> = keys.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    let mut out = Vec::new();
    for m in mods {
        out.push(match m.to_ascii_lowercase().as_str() {
            "super" | "logo" | "mod4" | "win" => "super",
            "ctrl" | "control" => "ctrl",
            "alt" | "mod1" => "alt",
            "shift" => "shift",
            _ => return None,
        });
    }
    out.sort_unstable();
    out.dedup();
    Some((out, key))
}

pub fn reserved(keys: &str) -> bool {
    let Some((mods, key)) = split(keys) else {
        return false;
    };
    let is = |name: &str| key.eq_ignore_ascii_case(name);
    let vt_f_key = || {
        let k = key.to_ascii_lowercase();
        k.strip_prefix('f')
            .and_then(|n| n.parse::<u8>().ok())
            .is_some_and(|n| (1..=12).contains(&n))
    };
    (mods == ["shift", "super"] && is("Escape"))
        || key.to_ascii_lowercase().starts_with("xf86switch_vt_")
        || (mods == ["alt", "ctrl"] && vt_f_key())
        || (mods.is_empty() && MACHINE_KEYS.iter().any(|k| is(k)))
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

/// The modifier a modifier key holds, as `split` names it, and the key it counts as (Meta is Alt).
fn modifier_role(key: &str) -> Option<(&'static str, &'static str)> {
    let k = key.to_ascii_lowercase();
    Some(match k.as_str() {
        "shift_l" => ("shift", "Shift_L"),
        "shift_r" => ("shift", "Shift_R"),
        "control_l" => ("ctrl", "Control_L"),
        "control_r" => ("ctrl", "Control_R"),
        "alt_l" | "meta_l" => ("alt", "Alt_L"),
        "alt_r" | "meta_r" => ("alt", "Alt_R"),
        "super_l" => ("super", "Super_L"),
        "super_r" => ("super", "Super_R"),
        _ => return None,
    })
}

/// Whether two `keys` name one chord: the same modifiers and keysym, whatever the spelling and
/// order. A modifier-only chord counts its key among its modifiers and its key's side must agree,
/// so "Alt+Shift_L" and "Shift+Alt_L" are one chord and "Alt+Shift_R" is another.
pub fn same(a: &str, b: &str) -> bool {
    let (Some((ma, ka)), Some((mb, kb))) = (split(a), split(b)) else {
        return false;
    };
    if let (Some((ra, sa)), Some((rb, sb))) = (modifier_role(ka), modifier_role(kb)) {
        let held = |mut m: Vec<&'static str>, r| {
            m.push(r);
            m.sort_unstable();
            m.dedup();
            m
        };
        let side = |key: &str| key.ends_with("_R");
        return held(ma, ra) == held(mb, rb) && side(sa) == side(sb);
    }
    ma == mb && keysym(ka).is_some_and(|s| keysym(kb) == Some(s))
}

/// The action that unbinds a chord, the defaults' included.
pub const NONE: &str = "none";

/// The binds a `[[keybinds]]` list runs with, layered over [`DEFAULTS`]: the entries on a
/// default's chord take its place (a [`NONE`] entry binds nothing), every other default stays,
/// and the remaining entries follow in file order. `bind` reads an entry's keys and action.
pub fn layered<T>(
    entries: Vec<T>,
    bind: impl Fn(&T) -> (&str, &str),
    default: impl Fn(&'static str, &'static str) -> T,
) -> Vec<T> {
    let mut rest: Vec<Option<T>> = entries.into_iter().map(Some).collect();
    let mut out = Vec::new();
    let binds = |e: &T| bind(e).1 != NONE;
    for &(keys, action) in DEFAULTS {
        let mut replaced = false;
        for slot in &mut rest {
            if slot.as_ref().is_some_and(|e| same(bind(e).0, keys)) {
                replaced = true;
                out.extend(slot.take().filter(|e| binds(e)));
            }
        }
        if !replaced {
            out.push(default(keys, action));
        }
    }
    out.extend(rest.into_iter().flatten().filter(|e| binds(e)));
    out
}

pub fn modifier_only(keys: &str) -> bool {
    split(keys).is_some_and(|(_, key)| MODIFIER_KEYS.iter().any(|m| key.eq_ignore_ascii_case(m)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_rejects_what_the_compositor_rejects() {
        assert_eq!(check("Super+Shift+b"), Ok(()));
        assert_eq!(check("Super + t"), Ok(()));
        assert_eq!(check("Ctrl+Alt+F3"), Ok(()));
        assert!(check("Super+foo").is_err());
        assert!(check("Super+").is_err());
        assert!(check("Bogus+x").is_err());
        assert!(check("").is_err());
    }

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
    fn the_desktop_keeps_its_escape_power_media_and_vt_keys() {
        assert!(reserved("Super+Shift+Escape"));
        assert!(reserved("shift+super+escape"));
        assert!(reserved("XF86PowerOff"));
        assert!(reserved("XF86AudioPlay"));
        assert!(reserved("Ctrl+Alt+XF86Switch_VT_1"));
        assert!(!reserved("Pause"));
        assert!(
            !reserved("Ctrl+XF86AudioPlay"),
            "a media key with a modifier is free"
        );
        assert!(!reserved("Super+Escape"));
        assert!(reserved("Ctrl+Alt+F1"), "Ctrl+Alt+F1 switches the VT");
        assert!(reserved("alt+control+f12"));
        assert!(!reserved("Ctrl+Alt+F13"));
        assert!(!reserved("Ctrl+F1"));
        assert!(!reserved("Ctrl+Alt+Shift+F1"));
    }

    #[test]
    fn chords_compare_by_modifiers_and_keysym() {
        assert_eq!(keysym("Page_Up"), Some(0xff55));
        assert_eq!(keysym("prior"), Some(0xff55));
        assert_eq!(keysym("B"), Some(0x62), "lower-cased");
        assert_eq!(keysym("Nope"), None);
        assert_eq!(keysym(""), None);
        assert!(same("Prior", "Page_Up"), "two names of one keysym");
        assert!(same("Ctrl+Prior", "control+page_up"));
        assert!(same("Super+B", "win+b"));
        assert!(same("Shift+Alt+x", "Alt+Shift+x"));
        assert!(!same("Prior", "Next"));
        assert!(!same("Ctrl+Prior", "Prior"));
        assert!(
            !same("Bogus+x", "Bogus+x"),
            "an unknown modifier binds nothing"
        );
        assert!(!same("Nope", "Nope"), "an unknown key binds nothing");
    }

    #[test]
    fn a_modifier_only_chord_is_one_chord_in_either_order() {
        assert!(same("Alt+Shift_L", "Shift+Alt_L"));
        assert!(same("shift+alt_l", "Alt+Shift_L"));
        assert!(same("Shift+Meta_L", "Shift+Alt_L"), "Meta is Alt");
        assert!(same("Ctrl+Alt+Shift_L", "Shift+Ctrl+Alt_L"));
        assert!(!same("Shift+Alt_L", "Shift+Alt_R"), "the key's side counts");
        assert!(
            !same("Alt+Shift_R", "Shift+Alt_L"),
            "the key's side counts with the roles swapped"
        );
        assert!(!same("Shift+Alt_L", "Ctrl+Shift_L"));
        assert!(!same("Shift+Alt_L", "Alt_L"));
        assert!(!same("Ctrl+Alt+Shift_L", "Alt+Shift_L"));
        assert!(!same("Shift+Alt_L", "Shift+a"));
    }

    #[test]
    fn a_swapped_modifier_only_entry_replaces_the_default() {
        let out = run(&[("Alt+Shift_L", "next-layout")]);
        assert_eq!(out.len(), DEFAULTS.len());
        assert!(out.contains(&("Alt+Shift_L", "next-layout")));
        assert!(!out.contains(&("Shift+Alt_L", "next-layout")));
        let out = run(&[("Alt+Shift_L", NONE)]);
        assert!(!out.iter().any(|&(k, _)| k == "Shift+Alt_L"));
    }

    #[test]
    fn a_swapped_entry_on_the_other_side_keeps_the_default() {
        let out = run(&[("Alt+Shift_R", "next-layout")]);
        assert_eq!(out.len(), DEFAULTS.len() + 1);
        assert!(out.contains(&("Alt+Shift_R", "next-layout")));
        assert!(out.contains(&("Shift+Alt_L", "next-layout")));
    }

    #[test]
    fn a_chord_ending_in_a_modifier_is_modifier_only() {
        assert!(modifier_only("Shift+Alt_L"));
        assert!(modifier_only("Super_L"));
        assert!(!modifier_only("Shift+a"));
        assert!(!modifier_only("Pause"));
    }

    fn run(file: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
        layered(file.to_vec(), |&(k, a)| (k, a), |k, a| (k, a))
    }

    #[test]
    fn no_entries_give_the_defaults() {
        assert_eq!(run(&[]), DEFAULTS);
    }

    #[test]
    fn an_entry_on_a_new_chord_is_added_after_every_default() {
        let out = run(&[("Super+F9", "lock")]);
        assert_eq!(out[..DEFAULTS.len()], *DEFAULTS);
        assert_eq!(out[DEFAULTS.len()..], [("Super+F9", "lock")]);
    }

    #[test]
    fn an_entry_on_a_default_chord_takes_its_place() {
        let out = run(&[("super+Q", "minimize")]);
        assert_eq!(out.len(), DEFAULTS.len());
        assert_eq!(out[1], ("super+Q", "minimize"));
        assert!(!out.contains(&("Super+q", "close")));
    }

    #[test]
    fn none_unbinds_a_default_chord() {
        let out = run(&[("Win+Tab", NONE)]);
        assert_eq!(out.len(), DEFAULTS.len() - 1);
        assert!(!out.iter().any(|&(k, _)| same(k, "Super+Tab")));
        assert!(!out.iter().any(|&(_, a)| a == NONE));
    }

    #[test]
    fn a_rebound_action_keeps_its_default_chord() {
        let out = run(&[("Super+x", "close")]);
        assert!(out.contains(&("Super+q", "close")));
        assert!(out.contains(&("Super+x", "close")));
        let out = run(&[("Super+x", "close"), ("Super+q", NONE)]);
        assert!(
            !out.contains(&("Super+q", "close")),
            "none frees the old chord"
        );
        assert!(out.contains(&("Super+x", "close")));
    }

    #[test]
    fn an_old_full_list_gains_a_default_it_lacks() {
        let old: Vec<_> = (DEFAULTS.iter().copied())
            .filter(|&(k, _)| k != "Super+Tab")
            .collect();
        assert_eq!(run(&old), DEFAULTS);
    }

    #[test]
    fn every_default_bind_checks() {
        for (keys, _) in DEFAULTS {
            check(keys).unwrap_or_else(|e| panic!("{keys}: {e}"));
        }
    }
}
