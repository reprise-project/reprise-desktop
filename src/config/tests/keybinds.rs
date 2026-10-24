use super::*;

#[test]
fn global_shortcuts_parse_with_their_chord() {
    let cfg = Config::parse(
        "[[global_shortcuts]]\napp = \"hu.taltos.Taltos\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"Pause\"\n\
         [[global_shortcuts]]\napp = \"hu.taltos.Taltos\"\nid = \"mute\"\nkeys = \"\"\n",
    )
    .unwrap();
    let g = &cfg.global_shortcuts;
    assert_eq!(g.len(), 2);
    assert_eq!(g[0].chord, Some(parse_keybind("Pause").unwrap()));
    assert_eq!(g[0].description, "Hold to talk");
    assert_eq!(g[1].chord, None, "empty keys are unassigned");
    assert_eq!(g[1].description, "", "description is optional");
}

#[test]
fn a_global_shortcut_with_bad_keys_is_unassigned_and_the_file_still_loads() {
    let cfg = Config::parse(
        "[[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"Ctrl+Nope\"\n\
         [[global_shortcuts]]\napp = \"a\"\nid = \"y\"\nkeys = \"Shift_L\"\n",
    )
    .unwrap();
    assert_eq!(cfg.global_shortcuts[0].chord, None);
    assert_eq!(
        cfg.global_shortcuts[0].keys, "Ctrl+Nope",
        "the text is kept"
    );
    assert_eq!(
        cfg.global_shortcuts[1].chord, None,
        "a modifier-only chord takes no key"
    );
}

#[test]
fn a_repeated_global_shortcut_keeps_the_first() {
    let cfg = Config::parse(
        "[[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"F9\"\n\
         [[global_shortcuts]]\napp = \"a\"\nid = \"x\"\nkeys = \"F10\"\n",
    )
    .unwrap();
    assert_eq!(cfg.global_shortcuts.len(), 1);
    assert_eq!(cfg.global_shortcuts[0].keys, "F9");
}

#[test]
fn a_keybinds_list_layers_over_the_defaults() {
    let defaults = Config::default().keybinds;
    let c = Config::parse("[[keybinds]]\nkeys = \"Super+F9\"\naction = \"lock\"\n").unwrap();
    assert_eq!(c.keybinds[..defaults.len()], defaults[..]);
    assert_eq!(action_on(&c, "Super+F9"), Some(Action::Lock));

    let c = Config::parse("[[keybinds]]\nkeys = \"super+Q\"\naction = \"minimize\"\n").unwrap();
    assert_eq!(c.keybinds.len(), defaults.len());
    assert_eq!(action_on(&c, "Super+q"), Some(Action::Minimize));

    let c = Config::parse("[[keybinds]]\nkeys = \"Super+Tab\"\naction = \"none\"\n").unwrap();
    assert_eq!(c.keybinds.len(), defaults.len() - 1);
    assert_eq!(action_on(&c, "Super+Tab"), None);
    assert!(
        Config::parse("[[keybinds]]\nkeys = \"Super+Nope\"\naction = \"none\"\n").is_err(),
        "a none entry's keys still have to parse"
    );
}

#[test]
fn a_swapped_layout_chord_replaces_the_default_one() {
    let defaults = Config::default().keybinds;
    let c = Config::parse("[[keybinds]]\nkeys = \"Alt+Shift_L\"\naction = \"none\"\n").unwrap();
    assert_eq!(c.keybinds.len(), defaults.len() - 1);
    assert_eq!(action_on(&c, "Shift+Alt_L"), None);

    let c = Config::parse("[[keybinds]]\nkeys = \"Alt+Shift_L\"\naction = \"lock\"\n").unwrap();
    assert_eq!(c.keybinds.len(), defaults.len());
    assert_eq!(action_on(&c, "Shift+Alt_L"), None);
    assert_eq!(action_on(&c, "Alt+Shift_L"), Some(Action::Lock));
}

#[test]
fn an_old_full_keybinds_list_gains_super_tab() {
    let text: String = (repriseconf::keybind::DEFAULTS.iter())
        .filter(|(keys, _)| *keys != "Super+Tab")
        .map(|(keys, action)| format!("[[keybinds]]\nkeys = \"{keys}\"\naction = \"{action}\"\n"))
        .collect();
    let c = Config::parse(&text).unwrap();
    assert_eq!(c.keybinds, Config::default().keybinds);
    assert_eq!(action_on(&c, "Super+Tab"), Some(Action::Flip));
}

#[test]
fn keybind_parsing() {
    let (mods, sym) = parse_keybind("Super+Shift+E").unwrap();
    assert_eq!(mods, MOD_LOGO | MOD_SHIFT);
    assert_eq!(sym, KEY_e);
    let (mods, sym) = parse_keybind("Ctrl+Alt+F3").unwrap();
    assert_eq!(mods, MOD_CTRL | MOD_ALT);
    assert_eq!(sym, KEY_F3);
    let (mods, sym) = parse_keybind("Super+Return").unwrap();
    assert_eq!(mods, MOD_LOGO);
    assert_eq!(sym, KEY_Return);
    assert!(parse_keybind("Bogus+x").is_err());
    assert_eq!(
        parse_keybind("Win+x").unwrap(),
        parse_keybind("Super+x").unwrap(),
        "win is Super, as the settings sheet and its key field take it"
    );
    // The settings sheet's check trims, so this side must too.
    assert_eq!(
        parse_keybind("Super + t").unwrap(),
        parse_keybind("Super+t").unwrap()
    );
    assert!(parse_keybind("Super+NoSuchKey").is_err());
    assert!(parse_keybind("Super+").is_err());
}

#[test]
fn action_names_round_trip() {
    for name in [
        "terminal",
        "close",
        "cycle",
        "flip",
        "exit",
        "maximize",
        "minimize",
        "lock",
        "startmenu",
        "run",
        "logoff",
        "turnoff",
        "windowmenu",
        "showdesktop",
        "toggleframe",
        "alwaysontop",
        "launcher",
        "overview",
        "snap-layouts",
        "focus-taskbar",
        "focus-tray",
        "clipboard-history",
        "emoji-picker",
        "next-layout",
        "center",
        "tab-next",
        "tab-prev",
        "do-not-disturb",
        "night-light",
        "desktop-prev",
        "desktop-next",
        "desktop-carry-prev",
        "desktop-carry-next",
        "all-desktops",
        "screenshot",
        "screenshot-window",
        "screenshot-region",
        "screenshot-content",
        "zoom-in",
        "zoom-out",
        "zoom-reset",
        "color-filter",
    ] {
        let a = Action::from_parts(name, None).unwrap();
        assert_eq!(a.name(), Some(name));
    }
    assert_eq!(Action::Spawn("x".into()).name(), None);
}

#[test]
fn toggleframe_is_bound_by_default() {
    let (mods, keysym) = parse_keybind("Super+Shift+b").unwrap();
    let cfg = Config::default();
    assert!(
        cfg.keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::ToggleFrame)
    );
}

#[test]
fn center_is_bound_to_super_c_by_default() {
    let (mods, keysym) = parse_keybind("Super+c").unwrap();
    let cfg = Config::default();
    assert!(
        cfg.keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::Center)
    );
}

#[test]
fn print_screen_is_bound_by_default() {
    let cfg = Config::default();
    for (keys, action) in [
        ("Print", Action::Screenshot),
        ("Alt+Sys_Req", Action::ScreenshotWindow),
        ("Super+Shift+w", Action::ScreenshotWindow),
        ("Shift+Print", Action::ScreenshotRegion),
        ("Ctrl+Alt+Sys_Req", Action::ScreenshotContent),
    ] {
        let (mods, keysym) = parse_keybind(keys).unwrap();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
            "{keys} copies with {action:?}"
        );
    }
}

#[test]
fn the_emoji_picker_is_bound_to_super_period() {
    let (mods, keysym) = parse_keybind("Super+period").unwrap();
    assert!(
        Config::default()
            .keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::EmojiPicker)
    );
}

#[test]
fn the_tab_keys_are_bound_by_default() {
    let cfg = Config::default();
    for (keys, action) in [
        ("Super+grave", Action::Tab(false)),
        ("Super+Page_Down", Action::Tab(false)),
        ("Super+Page_Up", Action::Tab(true)),
    ] {
        let (mods, keysym) = parse_keybind(keys).unwrap();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
            "{keys} runs {action:?}"
        );
    }
}

#[test]
fn launcher_is_bound_to_super_space_by_default() {
    let (mods, keysym) = parse_keybind("Super+space").unwrap();
    let cfg = Config::default();
    assert!(
        cfg.keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::Launcher)
    );
}

#[test]
fn media_actions_round_trip_through_their_names() {
    for a in [
        Action::VolumeUp,
        Action::VolumeDown,
        Action::Mute,
        Action::MicMute,
        Action::BrightnessUp,
        Action::BrightnessDown,
        Action::KbdBrightnessUp,
        Action::KbdBrightnessDown,
        Action::PlayPause,
        Action::Next,
        Action::Prev,
        Action::Stop,
        Action::Wifi,
    ] {
        assert_eq!(Action::from_parts(a.name().unwrap(), None), Ok(a));
    }
}

#[test]
fn dialog_actions_and_power_commands_parse() {
    let c = Config::parse(
        "[power]\nreboot = \"loginctl reboot\"\n\n[[keybinds]]\nkeys = \"Super+F4\"\naction = \"turnoff\"\n\n[[startmenu]]\nlabel = \"Go\"\naction = \"run\"\n\n[[startmenu]]\nlabel = \"Bye\"\naction = \"logoff\"\n",
    )
    .unwrap();
    assert_eq!(c.power.reboot, "loginctl reboot");
    assert_eq!(c.power.standby, "systemctl suspend");
    assert_eq!(action_on(&c, "Super+F4"), Some(Action::TurnOff));
    assert!(matches!(
        &c.startmenu[0],
        MenuEntry::Item {
            target: MenuTarget::Action(Action::Run),
            ..
        }
    ));
    assert!(matches!(
        &c.startmenu[1],
        MenuEntry::Item {
            target: MenuTarget::Action(Action::LogOff),
            ..
        }
    ));
    assert!(Config::parse("[power]\nhalt = \"x\"\n").is_err());
}

#[test]
fn lock_action_and_default_bind() {
    let c = Config::default();
    assert!(
        c.keybinds
            .iter()
            .any(|b| b.action == Action::Lock && b.mods == MOD_LOGO && b.keysym == KEY_l)
    );
    let c = Config::parse("[[keybinds]]\nkeys = \"Super+F12\"\naction = \"lock\"\n").unwrap();
    assert_eq!(action_on(&c, "Super+F12"), Some(Action::Lock));
}
