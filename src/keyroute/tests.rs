use super::*;

#[test]
fn every_media_key_is_reserved_for_the_desktop() {
    // The XF86 keysym block.
    for sym in 0x1008_ff00u32..=0x1008_ffff {
        if media_action_for(sym).is_some() {
            let name = repriseconf::keybind::key_name(sym).expect("an XF86 key has a name");
            assert!(repriseconf::keybind::reserved(&name), "{name} is reserved");
        }
    }
}

#[test]
fn every_modifier_key_name_is_a_modifier_here() {
    for name in repriseconf::keybind::MODIFIER_KEYS {
        let (_, sym) = crate::config::parse_keybind(name).expect(name);
        assert!(modifier_key(sym).is_some(), "{name} is a modifier key");
    }
}

fn press(sym: u32, mods: u32) -> KeyPress {
    let lower = to_lower(sym);
    KeyPress {
        syms: vec![sym],
        lower: vec![lower],
        mods,
        pressed: true,
    }
}

fn release(sym: u32) -> KeyPress {
    KeyPress {
        pressed: false,
        ..press(sym, 0)
    }
}

fn binds() -> Vec<Keybind> {
    vec![
        Keybind {
            mods: MOD_ALT,
            keysym: KEY_Tab,
            action: Action::Cycle,
        },
        Keybind {
            mods: MOD_LOGO,
            keysym: KEY_Tab,
            action: Action::Flip,
        },
        Keybind {
            mods: MOD_LOGO,
            keysym: KEY_r,
            action: Action::Run,
        },
        Keybind {
            mods: MOD_LOGO,
            keysym: KEY_space,
            action: Action::Launcher,
        },
        Keybind {
            mods: MOD_LOGO,
            keysym: KEY_v,
            action: Action::ClipboardHistory,
        },
        Keybind {
            mods: MOD_LOGO,
            keysym: KEY_period,
            action: Action::EmojiPicker,
        },
    ]
}

fn r(modes: &Modes, k: &KeyPress) -> KeyRoute {
    route(modes, k, &binds(), &[]).route
}

#[test]
fn the_taskbar_with_the_keyboard_takes_keys_but_not_binds() {
    let m = Modes {
        taskbar_keys: true,
        dialog_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Right, 0)), KeyRoute::Taskbar(KEY_Right));
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Taskbar(KEY_Escape));
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle]),
        "Alt+Tab still switches"
    );
    assert_eq!(r(&m, &release(KEY_Right)), KeyRoute::Client);
    let menu = Modes {
        menus_open: true,
        ..m.clone()
    };
    assert_eq!(
        r(&menu, &press(KEY_Escape, 0)),
        KeyRoute::Menu(KEY_Escape),
        "a menu the bar opened takes its keys first"
    );
    let calendar = Modes {
        calendar_open: true,
        ..m
    };
    assert_eq!(
        r(&calendar, &press(KEY_Escape, 0)),
        KeyRoute::CalendarClose,
        "Escape shuts the clock's popup before it leaves the bar"
    );
}

#[test]
fn power_keys_route_before_everything() {
    let all = Modes {
        locked: true,
        captured: true,
        menus_open: true,
        dialog_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(r(&all, &press(KEY_XF86PowerOff, 0)), KeyRoute::PowerKey);
    assert_eq!(r(&all, &press(KEY_XF86Sleep, 0)), KeyRoute::SleepKey);
    // A release is nobody's: the lock takes it, as it takes every key.
    assert_eq!(r(&all, &release(KEY_XF86PowerOff)), KeyRoute::Lock);
}

#[test]
fn a_preview_saver_keeps_every_press_from_the_program() {
    let m = Modes {
        saver_preview: true,
        desktop_has_keyboard: false,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_a, 0)), KeyRoute::Swallow);
    assert_eq!(
        r(&m, &press(KEY_r, MOD_LOGO)),
        KeyRoute::Swallow,
        "binds wait too"
    );
    let super_down = route(&m, &press(KEY_Super_L, 0), &binds(), &[]);
    assert_eq!(super_down.route, KeyRoute::Swallow);
    assert_eq!(
        super_down.arm_super,
        Some(false),
        "its release opens no Start menu"
    );
    assert_eq!(
        r(&m, &press(KEY_XF86AudioMute, 0)),
        KeyRoute::Media(Action::Mute)
    );
    assert_eq!(
        r(&m, &release(KEY_a)),
        KeyRoute::Client,
        "held back by the taken list"
    );
}

#[test]
fn precedence_runs_vt_lock_interaction_switcher_menu_dialog_rename_icons_binds_client() {
    let all = Modes {
        can_switch_vt: true,
        locked: true,
        keyboard_interaction: true,
        switcher_open: true,
        menus_open: true,
        dialog_has_keyboard: true,
        renaming: true,
        desktop_has_keyboard: true,
        has_icons: true,
        selected_icon: Some(0),
        ..Modes::default()
    };
    let enter = press(KEY_Return, 0);
    // VT switching wins over everything, the lock included.
    assert_eq!(
        r(&all, &press(KEY_XF86Switch_VT_3, 0)),
        KeyRoute::VtSwitch(3)
    );
    assert_eq!(r(&all, &release(KEY_XF86Switch_VT_3)), KeyRoute::Lock);
    let mut m = all.clone();
    assert_eq!(r(&m, &enter), KeyRoute::Lock);
    m.locked = false;
    assert_eq!(r(&m, &enter), KeyRoute::Interaction(KEY_Return));
    m.keyboard_interaction = false;
    assert_eq!(r(&m, &enter), KeyRoute::Swallow);
    m.switcher_open = false;
    assert_eq!(r(&m, &enter), KeyRoute::Menu(KEY_Return));
    m.menus_open = false;
    assert_eq!(r(&m, &enter), KeyRoute::Dialog);
    m.dialog_has_keyboard = false;
    assert_eq!(r(&m, &enter), KeyRoute::Rename);
    m.renaming = false;
    assert_eq!(r(&m, &enter), KeyRoute::IconOpen(0));
    m.selected_icon = None;
    assert_eq!(
        r(&m, &press(KEY_Down, 0)),
        KeyRoute::IconMove(IconMove::Down)
    );
    m.desktop_has_keyboard = false;
    assert_eq!(
        r(&m, &press(KEY_r, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Run])
    );
    assert_eq!(r(&m, &enter), KeyRoute::Client);
}

#[test]
fn the_switcher_bind_passes_a_dialog_and_show_desktop_a_windowed_one_but_not_a_welcome_panel() {
    let mut m = Modes {
        dialog_has_keyboard: true,
        ..Modes::default()
    };
    let alt_tab = press(KEY_Tab, MOD_ALT);
    let super_tab = press(KEY_Tab, MOD_LOGO);
    assert_eq!(r(&m, &alt_tab), KeyRoute::Binds(vec![Action::Cycle]));
    assert_eq!(r(&m, &super_tab), KeyRoute::Binds(vec![Action::Flip]));
    // Other binds still go to the dialog.
    assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Dialog);
    let mut with_sd = binds();
    with_sd.push(Keybind {
        mods: MOD_LOGO,
        keysym: KEY_d,
        action: Action::ShowDesktop,
    });
    let super_d = |m: &Modes| route(m, &press(KEY_d, MOD_LOGO), &with_sd, &[]).route;
    assert_eq!(super_d(&m), KeyRoute::Dialog);
    m.dialog_windowed = true;
    assert_eq!(
        super_d(&m),
        KeyRoute::Binds(vec![Action::ShowDesktop]),
        "a windowed dialog lets Show Desktop through"
    );
    assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Dialog);
    m.dialog_windowed = false;
    m.dialog_modal = true;
    assert_eq!(r(&m, &alt_tab), KeyRoute::Dialog);
    assert_eq!(r(&m, &super_tab), KeyRoute::Dialog);
}

#[test]
fn zoom_lock_screenshot_and_volume_binds_pass_a_dialog_but_window_binds_do_not() {
    let mut b = binds();
    for (mods, keysym, action) in [
        (
            MOD_LOGO,
            KEY_equal,
            Action::Zoom(crate::magnifier::ZoomStep::In),
        ),
        (MOD_LOGO, KEY_l, Action::Lock),
        (0, KEY_Print, Action::Screenshot),
        (MOD_LOGO, KEY_Up, Action::VolumeUp),
        (MOD_LOGO, KEY_Down, Action::Minimize),
    ] {
        b.push(Keybind {
            mods,
            keysym,
            action,
        });
    }
    let mut m = Modes {
        dialog_has_keyboard: true,
        ..Modes::default()
    };
    for modal in [false, true] {
        m.dialog_modal = modal;
        let r = |k: &KeyPress| route(&m, k, &b, &[]).route;
        assert_eq!(
            r(&press(KEY_equal, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Zoom(crate::magnifier::ZoomStep::In)])
        );
        assert_eq!(
            r(&press(KEY_l, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Lock])
        );
        assert_eq!(
            r(&press(KEY_Print, 0)),
            KeyRoute::Binds(vec![Action::Screenshot])
        );
        assert_eq!(
            r(&press(KEY_Up, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::VolumeUp])
        );
        assert_eq!(
            r(&press(KEY_XF86AudioRaiseVolume, 0)),
            KeyRoute::Media(Action::VolumeUp)
        );
        assert_eq!(r(&press(KEY_Down, MOD_LOGO)), KeyRoute::Dialog);
        assert_eq!(r(&press(KEY_r, MOD_LOGO)), KeyRoute::Dialog);
        assert_eq!(r(&press(KEY_Escape, 0)), KeyRoute::Dialog);
        assert_eq!(r(&press(KEY_Return, 0)), KeyRoute::Dialog);
        assert_eq!(r(&press(KEY_equal, 0)), KeyRoute::Dialog);
    }
}

#[test]
fn the_switcher_binds_pass_an_open_start_menu_menu_or_popup() {
    let open = [
        Modes {
            menus_open: true,
            ..Modes::default()
        },
        Modes {
            start_panel_open: true,
            ..Modes::default()
        },
        Modes {
            start_panel_open: true,
            menus_open: true,
            ..Modes::default()
        },
        Modes {
            launcher_open: true,
            ..Modes::default()
        },
        Modes {
            clipview_open: true,
            ..Modes::default()
        },
        Modes {
            emojiview_open: true,
            ..Modes::default()
        },
        Modes {
            menus_open: true,
            dragging: true,
            ..Modes::default()
        },
    ];
    for m in open {
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_ALT)),
            KeyRoute::Binds(vec![Action::Cycle]),
            "{m:?}"
        );
        assert_eq!(
            r(&m, &press(KEY_Tab, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Flip]),
            "{m:?}"
        );
        assert_eq!(
            matches!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Binds(_)),
            m.menus_open,
            "other binds pass a menu but stay with a popup: {m:?}"
        );
    }
    let welcome = Modes {
        menus_open: true,
        dialog_has_keyboard: true,
        dialog_modal: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&welcome, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Menu(KEY_Tab)
    );
}

#[test]
fn a_capture_takes_every_key_after_the_lock_and_before_everything_else() {
    let mut m = Modes {
        can_switch_vt: true,
        captured: true,
        keyboard_interaction: true,
        menus_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Captured);
    assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Captured);
    assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Captured);
    m.locked = true;
    assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Lock);
}

#[test]
fn super_shift_escape_and_vt_switches_are_never_captured() {
    let m = Modes {
        can_switch_vt: true,
        captured: true,
        ..Modes::default()
    };
    let chord = press(KEY_Escape, MOD_LOGO | MOD_SHIFT);
    let routed = route(&m, &chord, &binds(), &[]);
    assert_eq!(routed.route, KeyRoute::CaptureEscape);
    assert_eq!(routed.arm_super, Some(false));
    assert_eq!(r(&m, &press(KEY_XF86Switch_VT_2, 0)), KeyRoute::VtSwitch(2));
    // Escape alone, or with one of the two modifiers, is the receiver's.
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Captured);
    assert_eq!(r(&m, &press(KEY_Escape, MOD_LOGO)), KeyRoute::Captured);
    assert_eq!(r(&Modes::default(), &chord), KeyRoute::Client);
    let locked = Modes { locked: true, ..m };
    assert_eq!(r(&locked, &chord), KeyRoute::Lock);
}

#[test]
fn releases_pass_through_except_under_the_lock_and_the_switcher() {
    let mut m = Modes {
        keyboard_interaction: true,
        menus_open: true,
        dialog_has_keyboard: true,
        renaming: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Client);
    m.switcher_open = true;
    assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Swallow);
    m.locked = true;
    assert_eq!(r(&m, &release(KEY_Return)), KeyRoute::Lock);
}

#[test]
fn switcher_keys() {
    let m = Modes {
        switcher_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::SwitcherCancel);
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::SwitcherStep { backwards: false }
    );
    assert_eq!(
        r(&m, &press(KEY_ISO_Left_Tab, MOD_ALT | MOD_SHIFT)),
        KeyRoute::SwitcherStep { backwards: true }
    );
    assert_eq!(r(&m, &press(KEY_a, 0)), KeyRoute::Swallow);
    assert_eq!(r(&m, &press(KEY_Right, MOD_ALT)), KeyRoute::Swallow);
}

#[test]
fn the_3d_switcher_steps_with_its_bind_and_not_the_arrows() {
    let m = Modes {
        switcher_open: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&m, &press(KEY_Escape, MOD_LOGO)),
        KeyRoute::SwitcherCancel
    );
    let step = |backwards| KeyRoute::SwitcherStep { backwards };
    assert_eq!(r(&m, &press(KEY_Tab, MOD_LOGO)), step(false));
    assert_eq!(
        r(&m, &press(KEY_ISO_Left_Tab, MOD_LOGO | MOD_SHIFT)),
        step(true)
    );
    for arrow in [KEY_Right, KEY_Down, KEY_Left, KEY_Up] {
        assert_eq!(r(&m, &press(arrow, MOD_LOGO)), KeyRoute::Swallow);
    }
    assert_eq!(r(&m, &press(KEY_a, MOD_LOGO)), KeyRoute::Swallow);
}

#[test]
fn super_tap_arms_on_a_bare_press_and_fires_on_the_release() {
    let m = Modes::default();
    let bare = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
    assert_eq!(bare.arm_super, Some(true));
    assert!(!bare.fire_super);
    // Any other press disarms, and a Super+x chord never arms.
    assert_eq!(
        route(&m, &press(KEY_a, 0), &binds(), &[]).arm_super,
        Some(false)
    );
    assert_eq!(
        route(&m, &press(KEY_r, MOD_LOGO), &binds(), &[]).arm_super,
        Some(false)
    );
    let armed = Modes {
        super_tap_armed: true,
        ..Modes::default()
    };
    let up = route(&armed, &release(KEY_Super_L), &binds(), &[]);
    assert!(up.fire_super);
    assert_eq!(up.route, KeyRoute::Client);
    assert!(!route(&m, &release(KEY_Super_L), &binds(), &[]).fire_super);
    // The switcher swallows the press before the tap logic sees it.
    let sw = Modes {
        switcher_open: true,
        ..Modes::default()
    };
    assert_eq!(
        route(&sw, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]).arm_super,
        None
    );
}

#[test]
fn a_menu_skips_a_super_keysym_for_its_own_key() {
    let m = Modes {
        menus_open: true,
        ..Modes::default()
    };
    let k = KeyPress {
        syms: vec![KEY_Super_L, KEY_Down],
        lower: vec![KEY_Super_L, KEY_Down],
        mods: MOD_LOGO,
        pressed: true,
    };
    assert_eq!(r(&m, &k), KeyRoute::Menu(KEY_Down));
    assert_eq!(r(&m, &press(KEY_Super_L, MOD_LOGO)), KeyRoute::Swallow);
}

fn menu_binds() -> Vec<Keybind> {
    let bind = |mods, keysym, action| Keybind {
        mods,
        keysym,
        action,
    };
    vec![
        bind(MOD_LOGO, KEY_l, Action::Lock),
        bind(MOD_LOGO, KEY_d, Action::ShowDesktop),
        bind(MOD_LOGO, KEY_Left, Action::Snap(crate::snap::Dir::Left)),
        bind(MOD_CTRL, KEY_Escape, Action::StartMenu),
        bind(MOD_ALT, KEY_F4, Action::Close),
        bind(0, KEY_Print, Action::Screenshot),
    ]
}

#[test]
fn the_close_bind_on_the_desktop_opens_turn_off() {
    let alt_f4 = press(KEY_F4, MOD_ALT);
    let desktop = Modes {
        desktop_has_keyboard: true,
        has_icons: true,
        selected_icon: Some(0),
        ..Modes::default()
    };
    let routed = route(&desktop, &alt_f4, &menu_binds(), &[]);
    assert_eq!(routed.route, KeyRoute::Binds(vec![Action::TurnOff]));
    assert!(routed.route.opens_surface());
    assert_eq!(
        route(&Modes::default(), &alt_f4, &menu_binds(), &[]).route,
        KeyRoute::Binds(vec![Action::Close]),
        "a focused window keeps the close bind"
    );
    let bar = Modes {
        taskbar_keys: true,
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(
        route(&bar, &alt_f4, &menu_binds(), &[]).route,
        KeyRoute::Binds(vec![Action::TurnOff]),
        "the taskbar with the keyboard holds no window to close"
    );
}

#[test]
fn a_modified_bind_runs_over_an_open_menu() {
    let m = Modes {
        menus_open: true,
        ..Modes::default()
    };
    let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
    for (sym, mods, a) in [
        (KEY_l, MOD_LOGO, Action::Lock),
        (KEY_d, MOD_LOGO, Action::ShowDesktop),
        (KEY_Left, MOD_LOGO, Action::Snap(crate::snap::Dir::Left)),
        (KEY_Escape, MOD_CTRL, Action::StartMenu),
        (KEY_F4, MOD_ALT, Action::Close),
    ] {
        assert_eq!(rm(&press(sym, mods)), KeyRoute::Binds(vec![a]));
    }
    assert_eq!(
        rm(&press(KEY_Print, 0)),
        KeyRoute::Menu(KEY_Print),
        "a bare bind stays the menu's"
    );
}

#[test]
fn a_letter_under_ctrl_or_super_is_no_access_key() {
    let m = Modes {
        menus_open: true,
        ..Modes::default()
    };
    let rm = |m: &Modes, k: &KeyPress| route(m, k, &menu_binds(), &[]).route;
    assert_eq!(rm(&m, &press(KEY_r, MOD_CTRL)), KeyRoute::Swallow);
    assert_eq!(rm(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Swallow);
    assert_eq!(rm(&m, &press(KEY_1, MOD_CTRL)), KeyRoute::Swallow);
    assert_eq!(rm(&m, &press(KEY_r, 0)), KeyRoute::Menu(KEY_r));
    assert_eq!(rm(&m, &press(KEY_R, MOD_SHIFT)), KeyRoute::Menu(KEY_R));
    assert_eq!(rm(&m, &press(KEY_r, MOD_ALT)), KeyRoute::Menu(KEY_r));
    assert_eq!(rm(&m, &press(KEY_Down, MOD_CTRL)), KeyRoute::Menu(KEY_Down));
    let drag = Modes {
        dragging: true,
        ..m.clone()
    };
    assert_eq!(
        rm(&drag, &press(KEY_r, MOD_CTRL)),
        KeyRoute::Menu(KEY_r),
        "Ctrl held for a copy keeps the access keys"
    );
    assert_eq!(rm(&drag, &press(KEY_r, MOD_LOGO)), KeyRoute::Swallow);
}

#[test]
fn during_a_drag_only_drag_binds_pass_a_menu() {
    let m = Modes {
        menus_open: true,
        dragging: true,
        ..Modes::default()
    };
    let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
    assert_eq!(
        rm(&press(KEY_d, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::ShowDesktop])
    );
    assert_eq!(rm(&press(KEY_l, MOD_LOGO)), KeyRoute::Swallow);
    assert_eq!(rm(&press(KEY_F4, MOD_ALT)), KeyRoute::Menu(KEY_F4));
}

#[test]
fn a_welcome_panel_keeps_the_binds_from_a_menu() {
    let m = Modes {
        menus_open: true,
        dialog_has_keyboard: true,
        dialog_modal: true,
        ..Modes::default()
    };
    let rm = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
    assert_eq!(rm(&press(KEY_l, MOD_LOGO)), KeyRoute::Swallow);
    assert_eq!(rm(&press(KEY_Left, MOD_LOGO)), KeyRoute::Menu(KEY_Left));
}

#[test]
fn media_keys_route_before_everything_and_only_on_press() {
    let all = Modes {
        locked: true,
        captured: true,
        menus_open: true,
        dialog_has_keyboard: true,
        switcher_open: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&all, &press(KEY_XF86AudioRaiseVolume, 0)),
        KeyRoute::Media(Action::VolumeUp)
    );
    assert_eq!(
        r(&all, &press(KEY_XF86MonBrightnessDown, 0)),
        KeyRoute::Media(Action::BrightnessDown)
    );
    assert_eq!(
        r(&all, &press(KEY_XF86AudioPause, 0)),
        KeyRoute::Media(Action::PlayPause)
    );
    assert_eq!(r(&all, &release(KEY_XF86AudioMute)), KeyRoute::Lock);
}

#[test]
fn the_wifi_key_goes_to_the_lock_while_locked() {
    let locked = Modes {
        locked: true,
        ..Modes::default()
    };
    assert_eq!(r(&locked, &press(KEY_XF86WLAN, 0)), KeyRoute::Lock);
    assert_eq!(
        r(&Modes::default(), &press(KEY_XF86WLAN, 0)),
        KeyRoute::Media(Action::Wifi)
    );
}

#[test]
fn a_config_bind_on_a_media_key_wins_over_the_default() {
    let binds = vec![Keybind {
        mods: 0,
        keysym: KEY_XF86AudioMute,
        action: Action::Lock,
    }];
    let m = Modes::default();
    assert_eq!(
        route(&m, &press(KEY_XF86AudioMute, 0), &binds, &[]).route,
        KeyRoute::Binds(vec![Action::Lock])
    );
    // With a modifier held the default no longer applies either: the
    // key is a chord now, and chords are the binds' business.
    assert_eq!(
        route(&m, &press(KEY_XF86AudioMute, MOD_LOGO), &binds, &[]).route,
        KeyRoute::Client
    );
}

#[test]
fn an_open_start_panel_takes_every_press_but_a_menu_over_it_comes_first() {
    let m = Modes {
        start_panel_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::StartPanel);
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::StartPanel);
    let m = Modes {
        start_panel_open: true,
        menus_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
}

#[test]
fn the_super_tap_still_fires_over_an_open_start_panel() {
    let m = Modes {
        start_panel_open: true,
        ..Modes::default()
    };
    let down = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
    assert_eq!(down.route, KeyRoute::StartPanel);
    assert_eq!(
        down.arm_super,
        Some(true),
        "a bare Super press arms the tap"
    );
    let armed = Modes {
        super_tap_armed: true,
        ..m
    };
    assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
}

#[test]
fn an_open_launcher_takes_every_press_except_its_own_bind() {
    let m = Modes {
        launcher_open: true,
        dialog_has_keyboard: true,
        renaming: true,
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::Launcher);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Launcher);
    assert_eq!(
        r(&m, &press(KEY_space, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Launcher])
    );
    let menus = Modes {
        menus_open: true,
        ..m.clone()
    };
    assert_eq!(r(&menus, &press(KEY_Return, 0)), KeyRoute::Menu(KEY_Return));
    assert_eq!(r(&m, &release(KEY_n)), KeyRoute::Client);
}

#[test]
fn the_open_clipboard_history_takes_every_press_except_its_own_bind() {
    let m = Modes {
        clipview_open: true,
        dialog_has_keyboard: true,
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::ClipView);
    assert_eq!(r(&m, &press(KEY_Delete, 0)), KeyRoute::ClipView);
    assert_eq!(r(&m, &press(KEY_space, MOD_LOGO)), KeyRoute::ClipView);
    assert_eq!(
        r(&m, &press(KEY_v, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::ClipboardHistory])
    );
    assert_eq!(r(&m, &release(KEY_v)), KeyRoute::Client);
    assert_eq!(
        r(&Modes::default(), &press(KEY_v, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::ClipboardHistory])
    );
}

#[test]
fn binds_and_global_shortcuts_run_over_the_displays_panel() {
    let m = Modes {
        displays_open: true,
        ..Modes::default()
    };
    let rd = |k: &KeyPress| route(&m, k, &menu_binds(), &talk()).route;
    assert_eq!(
        rd(&press(KEY_l, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Lock])
    );
    assert_eq!(
        rd(&press(KEY_Print, 0)),
        KeyRoute::Binds(vec![Action::Screenshot])
    );
    assert_eq!(rd(&press(KEY_Pause, 0)), hit());
    assert_eq!(rd(&press(KEY_Right, 0)), KeyRoute::Displays);
}

#[test]
fn the_displays_panel_takes_the_keys_but_not_the_switcher() {
    let m = Modes {
        displays_open: true,
        ..Modes::default()
    };
    for sym in [KEY_Return, KEY_Tab, KEY_Escape, KEY_Left] {
        assert_eq!(r(&m, &press(sym, 0)), KeyRoute::Displays, "{sym:#x}");
    }
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle]),
        "Alt+Tab still switches"
    );
    assert_eq!(r(&m, &release(KEY_Left)), KeyRoute::Client);
}

#[test]
fn the_open_emoji_picker_takes_every_press_except_its_own_bind() {
    let m = Modes {
        emojiview_open: true,
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Return, 0)), KeyRoute::EmojiView);
    assert_eq!(r(&m, &press(KEY_Left, 0)), KeyRoute::EmojiView);
    assert_eq!(r(&m, &press(KEY_v, MOD_LOGO)), KeyRoute::EmojiView);
    assert_eq!(
        r(&m, &press(KEY_period, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::EmojiPicker])
    );
    assert_eq!(r(&m, &release(KEY_period)), KeyRoute::Client);
    assert_eq!(
        r(&Modes::default(), &press(KEY_period, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::EmojiPicker])
    );
}

/// A balloon's reply field, once clicked, takes every press, binds
/// and Escape included, except Alt+Tab; unclicked, it takes none.
#[test]
fn a_clicked_reply_field_takes_every_press_but_the_switcher() {
    let m = Modes {
        notify_reply_focused: true,
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    for (sym, mods) in [
        (KEY_a, 0),
        (KEY_Return, 0),
        (KEY_Escape, 0),
        (KEY_r, MOD_LOGO),
    ] {
        assert_eq!(r(&m, &press(sym, mods)), KeyRoute::NotifyReply, "{sym:#x}");
    }
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle])
    );
    let unclicked = Modes {
        desktop_has_keyboard: true,
        ..Modes::default()
    };
    assert_ne!(r(&unclicked, &press(KEY_a, 0)), KeyRoute::NotifyReply);
}

#[test]
fn the_notification_history_takes_every_press_but_a_bind() {
    let m = Modes {
        notify_center_open: true,
        desktop_has_keyboard: true,
        has_icons: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Down, 0)), KeyRoute::NotifyCenter(KEY_Down));
    assert_eq!(
        r(&m, &press(KEY_Escape, 0)),
        KeyRoute::NotifyCenter(KEY_Escape)
    );
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle])
    );
    let menus = Modes {
        menus_open: true,
        ..m.clone()
    };
    assert_eq!(r(&menus, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
    assert_eq!(r(&m, &release(KEY_Down)), KeyRoute::Client);
}

#[test]
fn the_region_overlay_takes_every_press_after_the_lock_and_a_capture() {
    let m = Modes {
        region_open: true,
        keyboard_interaction: true,
        menus_open: true,
        launcher_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Region(KEY_Escape));
    assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Region(KEY_r));
    assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
    let locked = Modes {
        locked: true,
        ..m.clone()
    };
    assert_eq!(r(&locked, &press(KEY_Escape, 0)), KeyRoute::Lock);
    let captured = Modes {
        captured: true,
        ..m
    };
    assert_eq!(r(&captured, &press(KEY_Escape, 0)), KeyRoute::Captured);
}

#[test]
fn escape_cancels_a_bar_drag_and_other_keys_pass() {
    let m = Modes {
        bar_drag: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::BarDragCancel);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
    assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
    // An open menu takes the Escape first.
    let menu = Modes {
        bar_drag: true,
        menus_open: true,
        ..Modes::default()
    };
    assert_ne!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::BarDragCancel);
}

#[test]
fn escape_cancels_a_pointer_move_and_other_keys_pass() {
    let m = Modes {
        pointer_move: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::MoveCancel);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
    assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
    let menu = Modes {
        pointer_move: true,
        menus_open: true,
        ..Modes::default()
    };
    assert_ne!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::MoveCancel);
}

#[test]
fn escape_closes_the_clock_popup_and_other_keys_pass() {
    let m = Modes {
        calendar_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::CalendarClose);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
    assert_eq!(r(&m, &release(KEY_Escape)), KeyRoute::Client);
}

#[test]
fn escape_cancels_a_drag_and_no_client_hears_a_press() {
    let m = Modes {
        dragging: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::DragCancel);
    // Held with the copy modifier too, and ahead of Ctrl+Escape's
    // Start menu.
    let mut b = binds();
    b.push(Keybind {
        mods: MOD_CTRL,
        keysym: KEY_Escape,
        action: Action::StartMenu,
    });
    let ctrl_escape = press(KEY_Escape, MOD_CTRL);
    assert_eq!(route(&m, &ctrl_escape, &b, &[]).route, KeyRoute::DragCancel);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Swallow);
    assert_eq!(r(&m, &release(KEY_n)), KeyRoute::Client);
    // Binds with nothing to drop on wait for the drop; the launcher's
    // and Run's fields take it.
    let routed = route(&m, &press(KEY_v, MOD_LOGO), &binds(), &[]);
    assert_eq!(routed.route, KeyRoute::Swallow);
    assert_eq!(routed.arm_super, Some(false));
    assert_eq!(
        r(&m, &press(KEY_space, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Launcher])
    );
    assert_eq!(
        r(&m, &press(KEY_r, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Run])
    );
}

#[test]
fn alt_tab_and_the_super_tap_work_during_a_drag() {
    let m = Modes {
        dragging: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&m, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle])
    );
    let down = route(&m, &press(KEY_Super_L, 0), &binds(), &[]);
    assert_eq!(down.route, KeyRoute::Swallow);
    assert_eq!(down.arm_super, Some(true));
    let armed = Modes {
        super_tap_armed: true,
        ..m.clone()
    };
    assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
    // A client inhibiting shortcuts hears no key during a drag, so the
    // binds are Reprise's then too.
    let inhibited = Modes {
        shortcuts_inhibited: true,
        ..m
    };
    assert_eq!(
        r(&inhibited, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::Binds(vec![Action::Cycle])
    );
    let down = route(&inhibited, &press(KEY_Super_L, 0), &binds(), &[]);
    assert_eq!(down.arm_super, Some(true));
    let armed = Modes {
        super_tap_armed: true,
        ..inhibited
    };
    assert!(route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
}

#[test]
fn the_switcher_and_the_menus_keep_their_keys_during_a_drag() {
    let switcher = Modes {
        dragging: true,
        switcher_open: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&switcher, &press(KEY_Tab, MOD_ALT)),
        KeyRoute::SwitcherStep { backwards: false }
    );
    assert_eq!(
        r(&switcher, &press(KEY_Escape, MOD_ALT)),
        KeyRoute::SwitcherCancel
    );
    assert_eq!(r(&switcher, &press(KEY_n, 0)), KeyRoute::Swallow);
    let menu = Modes {
        dragging: true,
        menus_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&menu, &press(KEY_Escape, 0)), KeyRoute::Menu(KEY_Escape));
    assert_eq!(r(&menu, &press(KEY_Down, 0)), KeyRoute::Menu(KEY_Down));
}

/// Mid-drag with Start open, Shift pressed over a held Ctrl and the
/// menu's arrows leave Ctrl picking the drop; a bind's own modifier
/// and the switcher's are spent.
#[test]
fn a_menu_key_mid_drag_spends_no_held_modifier() {
    let menu = Modes {
        dragging: true,
        menus_open: true,
        ..Modes::default()
    };
    let spends = |m: &Modes, k: &KeyPress| drag_spends(&r(m, k), k.mods);
    let ctrl = MOD_CTRL;
    assert_eq!(spends(&menu, &press(KEY_Shift_L, ctrl)), 0);
    assert_eq!(spends(&menu, &press(KEY_Down, ctrl)), 0);
    assert_eq!(spends(&menu, &press(KEY_Return, ctrl)), 0);
    let switcher = Modes {
        dragging: true,
        switcher_open: true,
        ..Modes::default()
    };
    let alt = MOD_ALT;
    assert_eq!(spends(&switcher, &press(KEY_Tab, alt)), alt);
    assert_eq!(drag_spends(&KeyRoute::Binds(vec![Action::Cycle]), alt), alt);
    assert_eq!(drag_spends(&KeyRoute::Swallow, ctrl), 0);
}

#[test]
fn only_the_binds_that_bring_a_target_within_reach_run_during_a_drag() {
    use crate::desktops::Step;
    use crate::magnifier::ZoomStep;
    use crate::snap::Dir;
    for a in [
        Action::Cycle,
        Action::StartMenu,
        Action::Launcher,
        Action::Run,
        Action::ShowDesktop,
        Action::Minimize,
        Action::Maximize,
        Action::Snap(Dir::Left),
        Action::Desktop(Step::Next),
        Action::Zoom(ZoomStep::In),
    ] {
        assert!(runs_during_drag(&a), "{a:?} runs during a drag");
    }
    for a in [
        Action::Close,
        Action::Exit,
        Action::Lock,
        Action::WindowMenu,
        Action::CarryToDesktop(Step::Next),
        Action::Screenshot,
        Action::ScreenshotRegion,
        Action::Terminal,
    ] {
        assert!(!runs_during_drag(&a), "{a:?} waits for the drop");
    }
}

#[test]
fn a_bind_that_opens_a_surface_ends_a_held_gesture() {
    use crate::desktops::Step;
    use crate::magnifier::ZoomStep;
    use crate::snap::Dir;
    let menus = Modes {
        menus_open: true,
        ..Modes::default()
    };
    for m in [Modes::default(), menus] {
        for sym in [KEY_space, KEY_v, KEY_period, KEY_r] {
            assert!(
                r(&m, &press(sym, MOD_LOGO)).opens_surface(),
                "Super+{sym:#x} opens a surface"
            );
        }
    }
    assert!(!r(&Modes::default(), &press(KEY_a, 0)).opens_surface());
    for a in [
        Action::StartMenu,
        Action::WindowMenu,
        Action::FocusTray,
        Action::SnapLayouts,
    ] {
        assert!(opens_surface(&a), "{a:?} opens a surface");
    }
    for a in [
        Action::VolumeUp,
        Action::Zoom(ZoomStep::In),
        Action::Snap(Dir::Left),
        Action::Desktop(Step::Next),
        Action::Screenshot,
    ] {
        assert!(!opens_surface(&a), "{a:?} keeps the gesture");
    }
}

#[test]
fn no_chord_fires_during_a_drag() {
    let m = Modes {
        dragging: true,
        ..Modes::default()
    };
    assert!(!chord_allowed(&m, &Action::NextLayout));
    assert!(chord_allowed(&Modes::default(), &Action::NextLayout));
}

#[test]
fn modifiers_pressed_during_a_drag_fire_no_chord_after_it() {
    // Shift and Alt pressed while the drag runs, each press spoiling the
    // chord, then released once it has ended.
    let mut st = ChordState::default();
    let mut fired = feed(&mut st, &[(SHIFT, KEY_Shift_L, true)]);
    st.spoil();
    fired.extend(feed(&mut st, &[(ALT, KEY_Alt_L, true)]));
    st.spoil();
    fired.extend(feed(
        &mut st,
        &[(ALT, KEY_Alt_L, false), (SHIFT, KEY_Shift_L, false)],
    ));
    assert_eq!(fired, vec![], "no layout switch");
    let fired = feed(
        &mut st,
        &[
            (SHIFT, KEY_Shift_L, true),
            (ALT, KEY_Alt_L, true),
            (ALT, KEY_Alt_L, false),
            (SHIFT, KEY_Shift_L, false),
        ],
    );
    assert_eq!(fired, vec![NEXT]);
}

#[test]
fn snap_assist_lets_the_snap_binds_through_and_the_overview_does_not() {
    use crate::snap::Dir;
    let bind = vec![Keybind {
        mods: MOD_LOGO,
        keysym: KEY_Left,
        action: Action::Snap(Dir::Left),
    }];
    let overview = Modes {
        overview_open: true,
        ..Modes::default()
    };
    let assist = Modes {
        snap_assist_open: true,
        ..overview.clone()
    };
    let snap = press(KEY_Left, MOD_LOGO);
    assert_eq!(
        route(&assist, &snap, &bind, &[]).route,
        KeyRoute::Binds(vec![Action::Snap(Dir::Left)])
    );
    assert_eq!(
        route(&assist, &press(KEY_Left, 0), &bind, &[]).route,
        KeyRoute::Overview,
        "a bare arrow walks the cards"
    );
    assert_eq!(
        route(&overview, &snap, &bind, &[]).route,
        KeyRoute::Overview
    );
}

#[test]
fn alt_tab_reaches_the_switcher_past_the_overview_and_the_snap_picker() {
    let overview = Modes {
        overview_open: true,
        ..Modes::default()
    };
    let assist = Modes {
        snap_assist_open: true,
        ..overview.clone()
    };
    let picker = Modes {
        snap_picker_open: true,
        snap_picker_keys: true,
        ..Modes::default()
    };
    let cycle = KeyRoute::Binds(vec![Action::Cycle]);
    for m in [&overview, &assist, &picker] {
        assert_eq!(r(m, &press(KEY_Tab, MOD_ALT)), cycle);
        assert_eq!(
            r(m, &press(KEY_Tab, MOD_LOGO)),
            KeyRoute::Binds(vec![Action::Flip])
        );
    }
    assert_eq!(
        r(&overview, &press(KEY_Tab, 0)),
        KeyRoute::Overview,
        "plain Tab walks the cards"
    );
    assert_eq!(
        r(&picker, &press(KEY_Tab, 0)),
        KeyRoute::SnapPicker(KEY_Tab)
    );
}

#[test]
fn the_snap_picker_takes_every_key_from_its_bind_and_only_escape_from_the_pointer() {
    let keys = Modes {
        snap_picker_open: true,
        snap_picker_keys: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&keys, &press(KEY_Left, 0)),
        KeyRoute::SnapPicker(KEY_Left)
    );
    assert_eq!(r(&keys, &press(KEY_n, 0)), KeyRoute::SnapPicker(KEY_n));
    let bind = vec![Keybind {
        mods: MOD_LOGO,
        keysym: KEY_z,
        action: Action::SnapLayouts,
    }];
    let again = route(&keys, &press(KEY_z, MOD_LOGO), &bind, &[]).route;
    assert_eq!(again, KeyRoute::Binds(vec![Action::SnapLayouts]));
    let hover = Modes {
        snap_picker_open: true,
        ..Modes::default()
    };
    assert_eq!(
        r(&hover, &press(KEY_Escape, 0)),
        KeyRoute::SnapPicker(KEY_Escape)
    );
    assert_eq!(r(&hover, &press(KEY_Left, 0)), KeyRoute::Client);
}

#[test]
fn escape_closes_the_hidden_icons_flyout_and_other_keys_pass() {
    let m = Modes {
        tray_flyout_open: true,
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::TrayFlyoutClose);
    assert_eq!(r(&m, &press(KEY_n, 0)), KeyRoute::Client);
}

#[test]
fn an_inhibiting_client_gets_the_binds_and_super_but_nothing_ahead_of_them() {
    let m = Modes {
        can_switch_vt: true,
        shortcuts_inhibited: true,
        ..Modes::default()
    };
    let alt_tab = press(KEY_Tab, MOD_ALT);
    assert_eq!(r(&m, &alt_tab), KeyRoute::Client);
    assert_eq!(r(&m, &press(KEY_r, MOD_LOGO)), KeyRoute::Client);
    let sup = route(&m, &press(KEY_Super_L, MOD_LOGO), &binds(), &[]);
    assert_eq!(sup.route, KeyRoute::Client);
    assert_eq!(sup.arm_super, Some(false));
    let armed = Modes {
        super_tap_armed: true,
        ..m.clone()
    };
    assert!(!route(&armed, &release(KEY_Super_L), &binds(), &[]).fire_super);
    // VT switches, the machine's keys, the lock and an open menu still
    // come first.
    assert_eq!(r(&m, &press(KEY_XF86Switch_VT_2, 0)), KeyRoute::VtSwitch(2));
    assert_eq!(
        r(&m, &press(KEY_XF86AudioMute, 0)),
        KeyRoute::Media(Action::Mute)
    );
    let locked = Modes {
        locked: true,
        ..m.clone()
    };
    assert_eq!(r(&locked, &alt_tab), KeyRoute::Lock);
    let menus = Modes {
        menus_open: true,
        ..m.clone()
    };
    assert_eq!(r(&menus, &press(KEY_Return, 0)), KeyRoute::Menu(KEY_Return));
}

#[test]
fn super_shift_escape_suspends_a_shortcuts_inhibitor() {
    let m = Modes {
        shortcuts_inhibited: true,
        ..Modes::default()
    };
    let chord = press(KEY_Escape, MOD_LOGO | MOD_SHIFT);
    let routed = route(&m, &chord, &binds(), &[]);
    assert_eq!(routed.route, KeyRoute::InhibitEscape);
    assert_eq!(routed.arm_super, Some(false));
    assert_eq!(r(&m, &press(KEY_Escape, 0)), KeyRoute::Client);
    // A capture takes the chord first.
    let captured = Modes {
        captured: true,
        ..m
    };
    assert_eq!(r(&captured, &chord), KeyRoute::CaptureEscape);
}

#[test]
fn an_inhibiting_client_leaves_the_lock_the_magnifier_and_screenshots_to_reprise() {
    let m = Modes {
        shortcuts_inhibited: true,
        ..Modes::default()
    };
    let ri = |k: &KeyPress| route(&m, k, &menu_binds(), &[]).route;
    assert_eq!(
        ri(&press(KEY_l, MOD_LOGO)),
        KeyRoute::Binds(vec![Action::Lock])
    );
    assert_eq!(
        ri(&press(KEY_Print, 0)),
        KeyRoute::Binds(vec![Action::Screenshot])
    );
    assert_eq!(ri(&press(KEY_d, MOD_LOGO)), KeyRoute::Client);
}

/// Keycodes for the chord tests: the left and right Alt and Shift,
/// Ctrl, Tab and a letter.
const ALT: u32 = 64;
const RALT: u32 = 108;
const SHIFT: u32 = 50;
const RSHIFT: u32 = 62;
const CTRL: u32 = 37;
const TAB: u32 = 23;
const F: u32 = 41;

fn chord_binds() -> Vec<Keybind> {
    let mut b = binds();
    b.push(Keybind {
        mods: MOD_SHIFT,
        keysym: KEY_Alt_L,
        action: Action::NextLayout,
    });
    b
}

/// Feed `(keycode, keysym, pressed)` events; the actions fired, in order.
fn feed(st: &mut ChordState, events: &[(u32, u32, bool)]) -> Vec<Action> {
    let binds = chord_binds();
    events
        .iter()
        .filter_map(|&(code, sym, down)| {
            let k = if down { press(sym, 0) } else { release(sym) };
            st.key(code, &k, &binds)
        })
        .collect()
}

const NEXT: Action = Action::NextLayout;

#[test]
fn left_alt_and_shift_fire_on_the_first_release_in_either_order() {
    let mut st = ChordState::default();
    let fired = feed(
        &mut st,
        &[
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
            (ALT, KEY_Alt_L, false),
        ],
    );
    assert_eq!(fired, vec![NEXT], "once, on the Shift release");
    // Shift first: the Alt key then gives Meta_L, and the Alt release fires.
    let fired = feed(
        &mut st,
        &[
            (RSHIFT, KEY_Shift_R, true),
            (ALT, KEY_Meta_L, true),
            (ALT, KEY_Meta_L, false),
            (RSHIFT, KEY_Shift_R, false),
        ],
    );
    assert_eq!(fired, vec![NEXT]);
}

#[test]
fn each_shift_tap_under_a_held_alt_fires_again() {
    let mut st = ChordState::default();
    let fired = feed(
        &mut st,
        &[
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
            (ALT, KEY_Alt_L, false),
        ],
    );
    assert_eq!(fired, vec![NEXT, NEXT]);
}

#[test]
fn another_key_or_a_button_in_between_cancels_the_chord() {
    let mut st = ChordState::default();
    // Alt+Shift+Tab stays the switcher's.
    let fired = feed(
        &mut st,
        &[
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (TAB, KEY_ISO_Left_Tab, true),
            (TAB, KEY_ISO_Left_Tab, false),
            (SHIFT, KEY_Shift_L, false),
            (ALT, KEY_Alt_L, false),
        ],
    );
    assert!(fired.is_empty());
    feed(
        &mut st,
        &[(ALT, KEY_Alt_L, true), (SHIFT, KEY_Shift_L, true)],
    );
    st.spoil();
    let fired = feed(
        &mut st,
        &[(SHIFT, KEY_Shift_L, false), (ALT, KEY_Alt_L, false)],
    );
    assert!(fired.is_empty(), "a click in between");
    let fired = feed(
        &mut st,
        &[
            (F, KEY_f, true),
            (F, KEY_f, false),
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
        ],
    );
    assert_eq!(fired, vec![NEXT]);
}

#[test]
fn a_third_modifier_or_the_right_alt_is_not_the_chord() {
    let mut st = ChordState::default();
    let fired = feed(
        &mut st,
        &[
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (CTRL, KEY_Control_L, true),
            (CTRL, KEY_Control_L, false),
            (SHIFT, KEY_Shift_L, false),
            (ALT, KEY_Alt_L, false),
        ],
    );
    assert!(fired.is_empty(), "Ctrl joined an armed chord");
    let fired = feed(
        &mut st,
        &[
            (CTRL, KEY_Control_L, true),
            (ALT, KEY_Alt_L, true),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
            (ALT, KEY_Alt_L, false),
            (CTRL, KEY_Control_L, false),
        ],
    );
    assert!(fired.is_empty(), "Ctrl+Alt+Shift");
    let fired = feed(
        &mut st,
        &[
            (RALT, KEY_Alt_R, true),
            (SHIFT, KEY_Shift_L, true),
            (SHIFT, KEY_Shift_L, false),
            (RALT, KEY_Alt_R, false),
        ],
    );
    assert!(fired.is_empty(), "the bind names the left Alt");
}

#[test]
fn a_chord_bind_never_matches_as_a_press() {
    let m = Modes::default();
    let k = press(KEY_Alt_L, MOD_SHIFT);
    assert_eq!(route(&m, &k, &chord_binds(), &[]).route, KeyRoute::Client);
}

#[test]
fn a_chord_runs_where_binds_do_and_the_layout_switch_at_the_lock() {
    let m = Modes::default();
    assert!(chord_allowed(&m, &NEXT));
    let locked = Modes {
        locked: true,
        ..Modes::default()
    };
    assert!(chord_allowed(&locked, &NEXT));
    assert!(!chord_allowed(&locked, &Action::Terminal));
    for m in [
        Modes {
            captured: true,
            ..Modes::default()
        },
        Modes {
            shortcuts_inhibited: true,
            ..Modes::default()
        },
    ] {
        assert!(!chord_allowed(&m, &NEXT));
    }
}

#[test]
fn a_dialog_with_the_keyboard_passes_a_chord_only_what_it_passes_a_bind() {
    let dialog = Modes {
        dialog_has_keyboard: true,
        ..Modes::default()
    };
    assert!(chord_allowed(&dialog, &NEXT));
    assert!(chord_allowed(&dialog, &Action::Lock));
    assert!(!chord_allowed(&dialog, &Action::Terminal));
    assert!(!chord_allowed(&dialog, &Action::ShowDesktop));
    let windowed = Modes {
        dialog_windowed: true,
        ..dialog
    };
    assert!(chord_allowed(&windowed, &Action::ShowDesktop));
    assert!(!chord_allowed(&windowed, &Action::Terminal));
}

#[test]
fn alt_enter_on_a_selected_icon_is_its_properties_and_enter_opens() {
    let m = Modes {
        desktop_has_keyboard: true,
        has_icons: true,
        selected_icon: Some(2),
        ..Modes::default()
    };
    for enter in [KEY_Return, KEY_KP_Enter] {
        assert_eq!(r(&m, &press(enter, 0)), KeyRoute::IconOpen(2));
        assert_eq!(
            r(&m, &press(enter, MOD_ALT)),
            KeyRoute::IconProperties(2),
            "Alt+Enter shows the icon's Properties and opens nothing"
        );
    }
}

#[test]
fn icons_need_the_desktop_to_hold_the_keyboard() {
    let m = Modes {
        has_icons: true,
        selected_icon: Some(2),
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_F2, 0)), KeyRoute::Client);
    let m = Modes {
        desktop_has_keyboard: true,
        ..m
    };
    assert_eq!(r(&m, &press(KEY_F2, 0)), KeyRoute::IconRename(2));
    assert_eq!(r(&m, &press(KEY_Delete, 0)), KeyRoute::IconTrash(2));
    assert_eq!(
        r(&m, &press(KEY_Home, 0)),
        KeyRoute::IconMove(IconMove::Home)
    );
}

#[test]
fn the_desktop_takes_its_menu_clipboard_and_refresh_keys() {
    let m = Modes {
        desktop_has_keyboard: true,
        has_icons: true,
        selected_icon: Some(2),
        ..Modes::default()
    };
    assert_eq!(r(&m, &press(KEY_Menu, 0)), KeyRoute::IconMenu(Some(2)));
    assert_eq!(
        r(&m, &press(KEY_F10, MOD_SHIFT)),
        KeyRoute::IconMenu(Some(2))
    );
    assert_eq!(r(&m, &press(KEY_F10, 0)), KeyRoute::Client);
    assert_eq!(r(&m, &press(KEY_a, MOD_CTRL)), KeyRoute::IconSelectAll);
    // Caps Lock gives the capital keysym; the route reads the lower.
    assert_eq!(r(&m, &press(KEY_C, MOD_CTRL)), KeyRoute::IconCopy(2));
    assert_eq!(r(&m, &press(KEY_x, MOD_CTRL)), KeyRoute::IconCut(2));
    assert_eq!(r(&m, &press(KEY_v, MOD_CTRL)), KeyRoute::IconPaste);
    assert_eq!(
        r(&m, &press(KEY_Delete, MOD_SHIFT)),
        KeyRoute::IconErase(2),
        "Shift+Delete deletes for good"
    );
    assert_eq!(r(&m, &press(KEY_v, 0)), KeyRoute::Client);
    // With nothing selected the menu is the desktop's, and Paste still
    // lands there.
    let none = Modes {
        selected_icon: None,
        ..m
    };
    assert_eq!(r(&none, &press(KEY_Menu, 0)), KeyRoute::IconMenu(None));
    assert_eq!(r(&none, &press(KEY_c, MOD_CTRL)), KeyRoute::Client);
    assert_eq!(r(&none, &press(KEY_v, MOD_CTRL)), KeyRoute::IconPaste);
    assert_eq!(r(&none, &press(KEY_F5, 0)), KeyRoute::IconRefresh);
}

#[test]
fn a_bound_arrow_runs_its_bind_over_the_desktop_icons() {
    let m = Modes {
        desktop_has_keyboard: true,
        has_icons: true,
        ..Modes::default()
    };
    let mut binds = binds();
    binds.push(Keybind {
        mods: MOD_LOGO | MOD_CTRL,
        keysym: KEY_Right,
        action: Action::Desktop(crate::desktops::Step::Next),
    });
    let super_ctrl_right = press(KEY_Right, MOD_LOGO | MOD_CTRL);
    assert_eq!(
        route(&m, &super_ctrl_right, &binds, &[]).route,
        KeyRoute::Binds(vec![Action::Desktop(crate::desktops::Step::Next)])
    );
    assert_eq!(
        route(&m, &press(KEY_Right, 0), &binds, &[]).route,
        KeyRoute::IconMove(IconMove::Right)
    );
}

fn talk() -> Vec<GlobalBind> {
    vec![GlobalBind {
        mods: 0,
        keysym: KEY_Pause,
        app: "t".into(),
        id: "talk".into(),
    }]
}
fn hit() -> KeyRoute {
    KeyRoute::Global(vec![GlobalHit {
        app: "t".into(),
        id: "talk".into(),
    }])
}

#[test]
fn a_live_global_chord_goes_to_its_program() {
    let m = Modes::default();
    assert_eq!(
        route(&m, &press(KEY_Pause, 0), &binds(), &talk()).route,
        hit()
    );
    assert_eq!(
        route(&m, &press(KEY_Pause, MOD_CTRL), &binds(), &talk()).route,
        KeyRoute::Client
    );
    assert_eq!(
        route(&m, &release(KEY_Pause), &binds(), &talk()).route,
        KeyRoute::Client,
        "releases are the held list's"
    );
}

#[test]
fn a_keybind_beats_a_global_chord() {
    let g = vec![GlobalBind {
        mods: MOD_ALT,
        keysym: KEY_Tab,
        app: "t".into(),
        id: "x".into(),
    }];
    assert_eq!(
        route(&Modes::default(), &press(KEY_Tab, MOD_ALT), &binds(), &g).route,
        KeyRoute::Binds(vec![Action::Cycle])
    );
}

#[test]
fn an_inhibitor_and_the_lock_come_before_a_global_chord() {
    let inhibited = Modes {
        shortcuts_inhibited: true,
        ..Modes::default()
    };
    assert_eq!(
        route(&inhibited, &press(KEY_Pause, 0), &binds(), &talk()).route,
        KeyRoute::Client
    );
    let locked = Modes {
        locked: true,
        ..Modes::default()
    };
    assert_eq!(
        route(&locked, &press(KEY_Pause, 0), &binds(), &talk()).route,
        KeyRoute::Lock
    );
}

#[test]
fn a_global_chord_beats_the_icon_keys_and_the_notification_history() {
    let g = vec![GlobalBind {
        mods: MOD_CTRL,
        keysym: KEY_Delete,
        app: "t".into(),
        id: "x".into(),
    }];
    let x = KeyRoute::Global(vec![GlobalHit {
        app: "t".into(),
        id: "x".into(),
    }]);
    let ctrl_delete = press(KEY_Delete, MOD_CTRL);
    let desktop = Modes {
        desktop_has_keyboard: true,
        has_icons: true,
        selected_icon: Some(0),
        ..Modes::default()
    };
    assert_eq!(route(&desktop, &ctrl_delete, &binds(), &g).route, x);
    assert_eq!(
        route(&desktop, &ctrl_delete, &binds(), &[]).route,
        KeyRoute::IconTrash(0),
        "unbound, Ctrl+Delete trashes the icon"
    );
    let center = Modes {
        notify_center_open: true,
        ..Modes::default()
    };
    assert_eq!(route(&center, &ctrl_delete, &binds(), &g).route, x);
    assert_eq!(
        route(&center, &ctrl_delete, &binds(), &[]).route,
        KeyRoute::NotifyCenter(KEY_Delete)
    );
}
