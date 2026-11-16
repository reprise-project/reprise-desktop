//! `[[keybinds]]` as the settings sheet, portal and compositor agree on: the defaults a file's list
//! layers over, chord comparison, and the `keys` check (a key xkb does not know fails the whole
//! config in the compositor, so the sheet refuses it first).

pub use reprisekit::keycap::{key_known, key_name, key_text, keysym};

pub const MODIFIERS: &[&str] = &[
    "super", "logo", "mod4", "win", "ctrl", "control", "alt", "mod1", "shift",
];

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
    ("Ctrl+Shift+Escape", "task-manager"),
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
    ("Ctrl+Shift+Alt+r", "record-screen"),
    ("Super+Shift+c", "color-picker"),
    ("Super+v", "clipboard-history"),
    ("Super+period", "emoji-picker"),
    ("Super+equal", "zoom-in"),
    ("Super+minus", "zoom-out"),
    ("Super+0", "zoom-reset"),
    ("Super+1", "taskbar-item-1"),
    ("Super+2", "taskbar-item-2"),
    ("Super+3", "taskbar-item-3"),
    ("Super+4", "taskbar-item-4"),
    ("Super+5", "taskbar-item-5"),
    ("Super+6", "taskbar-item-6"),
    ("Super+7", "taskbar-item-7"),
    ("Super+8", "taskbar-item-8"),
    ("Super+9", "taskbar-item-9"),
    ("Super+d", "showdesktop"),
    ("Super+e", "explorer"),
    ("Super+Ctrl+c", "color-filter"),
    ("Super+Alt+s", "screen-reader"),
    ("Super+w", "overview"),
    ("Super+z", "snap-layouts"),
    ("Super+p", "display-switcher"),
    ("Super+t", "focus-taskbar"),
    ("Super+b", "focus-tray"),
    ("Super+Shift+n", "notifications"),
    ("Shift+Alt_L", "next-layout"),
    ("Super+grave", "tab-next"),
    ("Super+Page_Down", "tab-next"),
    ("Super+Page_Up", "tab-prev"),
    ("Alt+grave", "cycle-app"),
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
    "XF86TouchpadToggle",
    "XF86TouchpadOn",
    "XF86TouchpadOff",
    "XF86RFKill",
    "XF86RotationLockToggle",
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

const CARET_KEYS: &[&str] = &[
    "Left",
    "Right",
    "Up",
    "Down",
    "Home",
    "End",
    "Page_Up",
    "Page_Down",
    "KP_Left",
    "KP_Right",
    "KP_Up",
    "KP_Down",
    "KP_Home",
    "KP_End",
    "KP_Page_Up",
    "KP_Page_Down",
];

/// Whether `keys` types or edits text in a focused field: no Ctrl, Alt or
/// Super, and a key that types a character (Return, Tab, BackSpace, Delete
/// and Escape included) or moves the caret.
pub fn types_text(keys: &str) -> bool {
    let Some((mods, key)) = split(keys) else {
        return false;
    };
    if mods.iter().any(|m| *m != "shift") {
        return false;
    }
    let Some(sym) = keysym(key) else {
        return false;
    };
    reprisekit::keycap::types_char(sym) || CARET_KEYS.iter().any(|k| keysym(k) == Some(sym))
}

pub fn modifier_only(keys: &str) -> bool {
    split(keys).is_some_and(|(_, key)| MODIFIER_KEYS.iter().any(|m| key.eq_ignore_ascii_case(m)))
}

pub const WINDOWS: &str = "Windows";
pub const DESKTOPS: &str = "Desktops";
pub const LAUNCH: &str = "Launch and search";
pub const SCREENSHOTS: &str = "Screenshots";
pub const MEDIA: &str = "Media";
pub const ACCESSIBILITY: &str = "Accessibility";
pub const SYSTEM: &str = "System";
pub const PROGRAMS: &str = "Programs";

/// `taskbar-item-N`: task button N of the row, pinned slots first.
pub const TASKBAR_ITEMS: [&str; 10] = [
    "taskbar-item-1",
    "taskbar-item-2",
    "taskbar-item-3",
    "taskbar-item-4",
    "taskbar-item-5",
    "taskbar-item-6",
    "taskbar-item-7",
    "taskbar-item-8",
    "taskbar-item-9",
    "taskbar-item-10",
];

/// `desktop-N`: switch to desktop N.
pub const DESKTOP_ITEMS: [&str; 9] = [
    "desktop-1",
    "desktop-2",
    "desktop-3",
    "desktop-4",
    "desktop-5",
    "desktop-6",
    "desktop-7",
    "desktop-8",
    "desktop-9",
];

/// `desktop-carry-N`: switch to desktop N, taking the focused window along.
pub const DESKTOP_CARRY_ITEMS: [&str; 9] = [
    "desktop-carry-1",
    "desktop-carry-2",
    "desktop-carry-3",
    "desktop-carry-4",
    "desktop-carry-5",
    "desktop-carry-6",
    "desktop-carry-7",
    "desktop-carry-8",
    "desktop-carry-9",
];

/// Every `[[keybinds]]` action but `spawn`: its id, the key sheet heading it is listed under and
/// what it does in words.
pub const ACTION_NAMES: &[(&str, &str, &str)] = &[
    ("close", WINDOWS, "Close the window"),
    ("cycle", WINDOWS, "Switch windows"),
    ("cycle-app", WINDOWS, "Switch the app's windows"),
    ("flip", WINDOWS, "Switch windows in 3D"),
    ("maximize", WINDOWS, "Maximize"),
    ("minimize", WINDOWS, "Minimize"),
    ("windowmenu", WINDOWS, "Window menu"),
    ("showdesktop", WINDOWS, "Show the desktop"),
    ("toggleframe", WINDOWS, "Hide or show the frame"),
    ("alwaysontop", WINDOWS, "Always on top"),
    ("mini-view", WINDOWS, "Mini view"),
    ("center", WINDOWS, "Centre the window"),
    ("tab-next", WINDOWS, "Next tab"),
    ("tab-prev", WINDOWS, "Previous tab"),
    ("snap-left", WINDOWS, "Snap left"),
    ("snap-right", WINDOWS, "Snap right"),
    ("snap-up", WINDOWS, "Snap up"),
    ("snap-down", WINDOWS, "Snap down"),
    ("snap-layouts", WINDOWS, "Snap layouts"),
    ("overview", WINDOWS, "Window overview"),
    ("desktop-prev", DESKTOPS, "Previous desktop"),
    ("desktop-next", DESKTOPS, "Next desktop"),
    (
        "desktop-carry-prev",
        DESKTOPS,
        "Take the window to the previous one",
    ),
    (
        "desktop-carry-next",
        DESKTOPS,
        "Take the window to the next one",
    ),
    ("all-desktops", DESKTOPS, "Show on all desktops"),
    ("desktop-1", DESKTOPS, "Desktop 1"),
    ("desktop-2", DESKTOPS, "Desktop 2"),
    ("desktop-3", DESKTOPS, "Desktop 3"),
    ("desktop-4", DESKTOPS, "Desktop 4"),
    ("desktop-5", DESKTOPS, "Desktop 5"),
    ("desktop-6", DESKTOPS, "Desktop 6"),
    ("desktop-7", DESKTOPS, "Desktop 7"),
    ("desktop-8", DESKTOPS, "Desktop 8"),
    ("desktop-9", DESKTOPS, "Desktop 9"),
    ("desktop-carry-1", DESKTOPS, "Take the window to desktop 1"),
    ("desktop-carry-2", DESKTOPS, "Take the window to desktop 2"),
    ("desktop-carry-3", DESKTOPS, "Take the window to desktop 3"),
    ("desktop-carry-4", DESKTOPS, "Take the window to desktop 4"),
    ("desktop-carry-5", DESKTOPS, "Take the window to desktop 5"),
    ("desktop-carry-6", DESKTOPS, "Take the window to desktop 6"),
    ("desktop-carry-7", DESKTOPS, "Take the window to desktop 7"),
    ("desktop-carry-8", DESKTOPS, "Take the window to desktop 8"),
    ("desktop-carry-9", DESKTOPS, "Take the window to desktop 9"),
    ("launcher", LAUNCH, "Launcher"),
    ("startmenu", LAUNCH, "Start menu"),
    ("run", LAUNCH, "Run"),
    ("task-manager", LAUNCH, "Task Manager"),
    ("terminal", LAUNCH, "Terminal"),
    ("explorer", LAUNCH, "Explorer"),
    ("taskbar-item-1", LAUNCH, "Taskbar button 1"),
    ("taskbar-item-2", LAUNCH, "Taskbar button 2"),
    ("taskbar-item-3", LAUNCH, "Taskbar button 3"),
    ("taskbar-item-4", LAUNCH, "Taskbar button 4"),
    ("taskbar-item-5", LAUNCH, "Taskbar button 5"),
    ("taskbar-item-6", LAUNCH, "Taskbar button 6"),
    ("taskbar-item-7", LAUNCH, "Taskbar button 7"),
    ("taskbar-item-8", LAUNCH, "Taskbar button 8"),
    ("taskbar-item-9", LAUNCH, "Taskbar button 9"),
    ("taskbar-item-10", LAUNCH, "Taskbar button 10"),
    ("clipboard-history", LAUNCH, "Clipboard history"),
    ("emoji-picker", LAUNCH, "Emoji picker"),
    ("color-picker", LAUNCH, "Colour picker"),
    ("screenshot", SCREENSHOTS, "Whole screen"),
    ("screenshot-window", SCREENSHOTS, "Window"),
    ("screenshot-region", SCREENSHOTS, "Area"),
    (
        "screenshot-content",
        SCREENSHOTS,
        "Window without its frame",
    ),
    ("record-screen", SCREENSHOTS, "Record the screen"),
    ("record-region", SCREENSHOTS, "Record an area"),
    ("volume-up", MEDIA, "Volume up"),
    ("volume-down", MEDIA, "Volume down"),
    ("mute", MEDIA, "Mute"),
    ("mic-mute", MEDIA, "Mute the microphone"),
    ("brightness-up", MEDIA, "Brighter"),
    ("brightness-down", MEDIA, "Dimmer"),
    ("kbd-brightness-up", MEDIA, "Keyboard backlight up"),
    ("kbd-brightness-down", MEDIA, "Keyboard backlight down"),
    ("play-pause", MEDIA, "Play or pause"),
    ("next", MEDIA, "Next track"),
    ("prev", MEDIA, "Previous track"),
    ("stop", MEDIA, "Stop"),
    ("wifi", MEDIA, "Wireless on or off"),
    ("touchpad", MEDIA, "Touchpad on or off"),
    ("touchpad-on", MEDIA, "Touchpad on"),
    ("touchpad-off", MEDIA, "Touchpad off"),
    ("airplane-mode", MEDIA, "Airplane mode on or off"),
    ("rotation-lock", MEDIA, "Rotation lock on or off"),
    ("zoom-in", ACCESSIBILITY, "Zoom in"),
    ("zoom-out", ACCESSIBILITY, "Zoom out"),
    ("zoom-reset", ACCESSIBILITY, "Zoom back"),
    ("color-filter", ACCESSIBILITY, "Colour filter"),
    ("screen-reader", ACCESSIBILITY, "Screen reader on or off"),
    ("lock", SYSTEM, "Lock"),
    ("logoff", SYSTEM, "Log off"),
    ("turnoff", SYSTEM, "Turn off"),
    ("exit", SYSTEM, "Quit Reprise"),
    ("do-not-disturb", SYSTEM, "Do not disturb"),
    ("notifications", SYSTEM, "Answer the newest notification"),
    ("night-light", SYSTEM, "Night light"),
    (
        "display-switcher",
        SYSTEM,
        "Extend, duplicate or pick a display",
    ),
    ("focus-taskbar", SYSTEM, "Go to the taskbar"),
    ("focus-tray", SYSTEM, "Go to the notification area"),
    ("next-layout", SYSTEM, "Next keyboard layout"),
];

/// The heading and plain name of action `action`; None for `spawn` and an unknown id.
pub fn describe(action: &str) -> Option<(&'static str, &'static str)> {
    (ACTION_NAMES.iter())
        .find(|(id, _, _)| *id == action)
        .map(|&(_, group, what)| (group, what))
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
    fn bare_typing_and_caret_keys_type_text() {
        for keys in [
            "a",
            "Shift+a",
            "1",
            "space",
            "Return",
            "KP_Enter",
            "Tab",
            "BackSpace",
            "Delete",
            "Escape",
            "Left",
            "Page_Down",
            "KP_Home",
            "slash",
            "eacute",
        ] {
            assert!(types_text(keys), "{keys} types text");
        }
        for keys in [
            "Pause",
            "F13",
            "F24",
            "XF86AudioPlay",
            "Ctrl+a",
            "Alt+Return",
            "Super+space",
            "Ctrl+Shift+Left",
        ] {
            assert!(!types_text(keys), "{keys} types no text");
        }
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
    fn every_action_has_a_heading_and_a_plain_name() {
        assert_eq!(
            describe("toggleframe"),
            Some((WINDOWS, "Hide or show the frame"))
        );
        assert_eq!(describe("exit"), Some((SYSTEM, "Quit Reprise")));
        assert_eq!(
            describe("desktop-carry-prev"),
            Some((DESKTOPS, "Take the window to the previous one"))
        );
        assert_eq!(describe("spawn"), None);
        let headings = [
            WINDOWS,
            DESKTOPS,
            LAUNCH,
            SCREENSHOTS,
            MEDIA,
            ACCESSIBILITY,
            SYSTEM,
            PROGRAMS,
        ];
        let mut ids = std::collections::HashSet::new();
        for (id, heading, plain) in ACTION_NAMES {
            assert!(headings.contains(heading), "{id}: {heading}");
            assert!(!plain.is_empty(), "{id}");
            assert!(ids.insert(id), "{id} is listed twice");
        }
    }

    #[test]
    fn the_example_config_lists_every_action_under_its_heading() {
        let text = include_str!("../../../config.example.toml");
        assert!(text.contains("#action = \"spawn\"\n#command = "));
        let list = text
            .split_once("# Every other action is one of these names:\n")
            .expect("the [[keybinds]] preamble lists the action names")
            .1;
        let mut listed: Vec<(String, Vec<String>)> = Vec::new();
        for line in list.lines() {
            if let Some(more) = line.strip_prefix("#     ") {
                let (_, ids) = listed.last_mut().expect("a heading line comes first");
                ids.extend(more.split_whitespace().map(String::from));
            } else if let Some((heading, ids)) =
                line.strip_prefix("#   ").and_then(|l| l.split_once(": "))
            {
                let ids = ids.split_whitespace().map(String::from).collect();
                listed.push((heading.to_string(), ids));
            } else {
                break;
            }
        }
        let mut expected: Vec<(String, Vec<String>)> = Vec::new();
        for (id, heading, _) in ACTION_NAMES {
            match expected.last_mut() {
                Some((h, ids)) if h == heading => ids.push(id.to_string()),
                _ => expected.push((heading.to_string(), vec![id.to_string()])),
            }
        }
        assert_eq!(listed, expected);
    }

    #[test]
    fn every_default_bind_checks() {
        for (keys, _) in DEFAULTS {
            check(keys).unwrap_or_else(|e| panic!("{keys}: {e}"));
        }
    }
}
