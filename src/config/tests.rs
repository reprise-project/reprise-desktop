use super::*;
use crate::keys::*;
use crate::windowrules::WindowState;
use reprisekit::theme::Color;

mod keybinds;

/// The action the first bind on `keys` runs, as a key press finds it.
fn action_on(c: &Config, keys: &str) -> Option<Action> {
    let (mods, keysym) = parse_keybind(keys).unwrap();
    (c.keybinds.iter())
        .find(|b| b.mods == mods && b.keysym == keysym)
        .map(|b| b.action.clone())
}

#[test]
fn defaults() {
    let c = Config::default();
    assert_eq!(c.terminal, "reprise-terminal");
    assert!(c.retired.is_empty());
    assert_eq!(c.keybinds.len(), 40);
    assert!(matches!(c.keybinds[0].action, Action::Terminal));
    assert_eq!(c.startmenu.len(), 9);
    assert_eq!(c.startmenu_banner, "Reprise Desktop");
    assert!(matches!(
        &c.startmenu[0],
        MenuEntry::Item { label, target: MenuTarget::Programs, .. } if label == "&Programs"
    ));
    assert!(matches!(c.startmenu[6], MenuEntry::Separator));
    assert!(matches!(
        &c.startmenu[7],
        MenuEntry::Item {
            target: MenuTarget::Action(Action::LogOff),
            ..
        }
    ));
    assert_eq!(c.power, PowerConfig::default());
    assert_eq!(c.quicklaunch.len(), 3);
    assert_eq!(c.quicklaunch[1].action, Action::ShowDesktop);
}

#[test]
fn night_light_is_off_by_default_and_every_key_is_checked() {
    let d = Config::default().base.night_light;
    assert!(!d.enabled);
    assert_eq!((d.temperature, d.schedule), (4000, NightSchedule::Sun));
    assert_eq!((d.from, d.to, d.location), (21 * 60, 7 * 60, None));
    let c = Config::parse(
        "[night_light]\nenabled = true\ntemperature = 3400\nschedule = \"fixed\"\n\
         from = \"20:30\"\nto = \"6:05\"\nlatitude = 47.5\nlongitude = 19.08\n",
    )
    .unwrap()
    .base
    .night_light;
    assert!(c.enabled);
    assert_eq!((c.temperature, c.schedule), (3400, NightSchedule::Fixed));
    assert_eq!((c.from, c.to), (20 * 60 + 30, 6 * 60 + 5));
    assert_eq!(c.location, Some((47.5, 19.08)));
    for bad in [
        "temperature = 900",
        "temperature = 7000",
        "schedule = \"moon\"",
        "from = \"25:00\"",
        "to = \"7\"",
        "from = \"07:00\"",
        "latitude = 47.5",
        "latitude = 91.0\nlongitude = 0.0",
        "brightness = 1",
    ] {
        assert!(
            Config::parse(&format!("[night_light]\n{bad}\n")).is_err(),
            "{bad} is refused"
        );
    }
    let bind = Config::parse("[[keybinds]]\nkeys = \"Super+n\"\naction = \"night-light\"\n");
    assert_eq!(
        action_on(&bind.unwrap(), "Super+n"),
        Some(Action::NightLight)
    );
}

#[test]
fn no_colour_filter_by_default_and_each_one_parses() {
    use repriseconf::config::Follow;
    use reprisekit::colorfilter::Filter;
    let d = Config::default().base.accessibility;
    assert_eq!(d.color_filter, Filter::None);
    assert_eq!(d.color_filter_intensity, 100);
    assert_eq!(d.magnifier_follow, Follow::PointerAndFocus);
    let c = Config::parse(
        "[accessibility]\ncolor_filter = \"tritanopia\"\n\
         color_filter_intensity = 40\nmagnifier_follow = \"pointer\"\n",
    )
    .unwrap()
    .base
    .accessibility;
    assert_eq!(c.color_filter, Filter::Tritanopia);
    assert_eq!(c.color_filter_intensity, 40);
    assert_eq!(c.magnifier_follow, Follow::Pointer);
    for bad in [
        "color_filter = \"sepia\"",
        "color_filter_intensity = 101",
        "color_filter_intensity = -1",
        "magnifier_follow = \"caret\"",
    ] {
        assert!(
            Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
            "{bad} is refused"
        );
    }
    let (mods, keysym) = parse_keybind("Super+Ctrl+c").unwrap();
    assert!(
        Config::default()
            .keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::ColorFilter),
        "Super+Ctrl+C toggles the filter"
    );
}

#[test]
fn clipboard_history_is_on_bound_to_super_v_and_configurable() {
    let cfg = Config::default();
    assert!(cfg.clipboard.history);
    assert!(cfg.clipboard.terminal_apps.contains(&"foot".to_string()));
    assert!(
        cfg.clipboard
            .terminal_apps
            .contains(&"reprise.terminal".to_string())
    );
    let (mods, keysym) = parse_keybind("Super+v").unwrap();
    assert!(
        cfg.keybinds
            .iter()
            .any(|b| b.mods == mods && b.keysym == keysym && b.action == Action::ClipboardHistory)
    );
    let c = Config::parse("[clipboard]\nhistory = false\nterminal_apps = [\"st\"]\n").unwrap();
    assert!(!c.clipboard.history);
    assert_eq!(c.clipboard.terminal_apps, vec!["st".to_string()]);
    assert!(cfg.clipboard.trusted_apps.is_empty());
    let c = Config::parse("[clipboard]\ntrusted_apps = [\"me.proton.Pass\"]\n").unwrap();
    assert_eq!(c.clipboard.trusted_apps, vec!["me.proton.Pass".to_string()]);
    assert!(Config::parse("[clipboard]\nsize = 3\n").is_err());
}

#[test]
fn four_desktops_by_default_and_count_is_checked() {
    use crate::desktops::Step;
    let cfg = Config::default();
    assert_eq!(cfg.workspaces, 4);
    for (keys, action) in [
        ("Super+Ctrl+Left", Action::Desktop(Step::Prev)),
        ("Super+Ctrl+Right", Action::Desktop(Step::Next)),
        ("Super+Ctrl+Shift+Left", Action::CarryToDesktop(Step::Prev)),
        ("Super+Ctrl+Shift+Right", Action::CarryToDesktop(Step::Next)),
    ] {
        let (mods, keysym) = parse_keybind(keys).unwrap();
        assert!(
            cfg.keybinds
                .iter()
                .any(|b| b.mods == mods && b.keysym == keysym && b.action == action),
            "{keys} runs {action:?}"
        );
    }
    assert_eq!(
        Config::parse("[workspaces]\ncount = 1\n")
            .unwrap()
            .workspaces,
        1
    );
    assert_eq!(
        Config::parse("[workspaces]\ncount = 16\n")
            .unwrap()
            .workspaces,
        16
    );
    assert!(Config::parse("[workspaces]\ncount = 0\n").is_err());
    assert!(Config::parse("[workspaces]\ncount = 17\n").is_err());
    assert!(Config::parse("[workspaces]\nnames = []\n").is_err());
}

#[test]
fn quicklaunch_entries_parse_and_an_empty_list_hides_the_toolbar() {
    let c = Config::parse(
        "[[quicklaunch]]\nlabel = \"Files\"\nicon = \"folder\"\ncommand = \"nautilus\"\n\n[[quicklaunch]]\nlabel = \"Desk\"\naction = \"showdesktop\"\n",
    )
    .unwrap();
    assert_eq!(
        c.quicklaunch,
        vec![
            QuickLaunchEntry {
                label: "Files".into(),
                icon: Some("folder".into()),
                action: Action::Spawn("nautilus".into()),
            },
            QuickLaunchEntry {
                label: "Desk".into(),
                icon: None,
                action: Action::ShowDesktop,
            },
        ]
    );
    assert!(
        Config::parse("quicklaunch = []\n")
            .unwrap()
            .quicklaunch
            .is_empty()
    );
    assert!(Config::parse("[[quicklaunch]]\nlabel = \"Nothing\"\n").is_err());
}

#[test]
fn full_file() {
    let text = r##"
terminal = "kitty"

[colors]
active_title = "#000000"
background = "1 2 3"

[metrics]
caption_height = 20

[fonts]
family = ["DejaVu Sans"]
size_px = 12
antialias = true

[[keybinds]]
keys = "Super+t"
action = "spawn"
command = "kitty"

[[keybinds]]
keys = "Super+w"
action = "close"

[[startmenu]]
label = "Kitty"
command = "kitty"

[[startmenu]]
separator = true

[[startmenu]]
label = "Quit"
action = "exit"
"##;
    let c = Config::parse(text).unwrap();
    assert_eq!(c.terminal, "kitty");
    assert_eq!(c.theme.colors.active_title, Color::rgb(0, 0, 0));
    assert_eq!(c.theme.colors.background, Color::rgb(1, 2, 3));
    assert_eq!(c.theme.colors.button_face, Color::rgb(212, 208, 200));
    assert_eq!(c.theme.metrics.caption_height, 20);
    assert_eq!(c.theme.metrics.frame_width, 4);
    assert_eq!(c.theme.fonts.family, vec!["DejaVu Sans".to_string()]);
    assert!(c.theme.fonts.antialias);
    assert_eq!(c.keybinds.len(), 40);
    assert_eq!(
        action_on(&c, "Super+t"),
        Some(Action::Spawn("kitty".into()))
    );
    assert_eq!(action_on(&c, "Super+w"), Some(Action::Close));
    assert_eq!(c.startmenu.len(), 3);
    assert!(matches!(
        &c.startmenu[0],
        MenuEntry::Item { target: MenuTarget::Action(Action::Spawn(s)), icon: None, .. } if s == "kitty"
    ));
    assert!(matches!(c.startmenu[1], MenuEntry::Separator));
}

#[test]
fn startmenu_entries_nest_and_name_dynamic_submenus() {
    let text = r##"
startmenu_banner = "Custom Banner"

[[startmenu]]
label = "Programs"
icon = "applications-other"
submenu = "programs"

[[startmenu]]
label = "Settings"
[[startmenu.items]]
label = "Control Panel"
submenu = "settings"
[[startmenu.items]]
label = "Docs"
icon = ""
submenu = "documents"
[[startmenu.items]]
separator = true
[[startmenu.items]]
label = "Editor"
command = "gedit"
"##;
    let c = Config::parse(text).unwrap();
    assert_eq!(c.startmenu_banner, "Custom Banner");
    assert!(matches!(
        &c.startmenu[0],
        MenuEntry::Item { icon: Some(i), target: MenuTarget::Programs, .. } if i == "applications-other"
    ));
    let MenuEntry::Item {
        target: MenuTarget::Submenu(items),
        ..
    } = &c.startmenu[1]
    else {
        panic!("expected a static submenu");
    };
    assert_eq!(items.len(), 4);
    assert!(matches!(
        items[0],
        MenuEntry::Item {
            target: MenuTarget::Settings,
            ..
        }
    ));
    assert!(matches!(
        items[1],
        MenuEntry::Item {
            icon: None,
            target: MenuTarget::Documents,
            ..
        }
    ));
    assert!(matches!(items[2], MenuEntry::Separator));
    assert!(matches!(
        &items[3],
        MenuEntry::Item { target: MenuTarget::Action(Action::Spawn(s)), .. } if s == "gedit"
    ));
    assert!(c.startmenu[0].opens_submenu());
    assert!(!items[3].opens_submenu());
    assert!(!items[2].opens_submenu());
}

#[test]
fn startmenu_entry_rejects_ambiguous_or_unknown_targets() {
    let err = Config::parse("[[startmenu]]\nlabel = \"x\"\nsubmenu = \"bogus\"\n").unwrap_err();
    assert!(err.contains("unknown submenu"), "{err}");
    let err =
        Config::parse("[[startmenu]]\nlabel = \"x\"\ncommand = \"a\"\nsubmenu = \"programs\"\n")
            .unwrap_err();
    assert!(err.contains("mutually exclusive"), "{err}");
}

#[test]
fn a_startmenu_folder_without_items_is_an_empty_folder() {
    for text in [
        "[[startmenu]]\nlabel = \"x\"\n",
        "[[startmenu]]\nlabel = \"x\"\nitems = []\n",
    ] {
        let c = Config::parse(text).unwrap();
        assert!(
            matches!(
                &c.startmenu[..],
                [MenuEntry::Item { target: MenuTarget::Submenu(items), .. }] if items.is_empty()
            ),
            "{text:?} is one empty folder"
        );
    }
}

#[test]
fn partial_file_keeps_defaults() {
    let c = Config::parse("terminal = \"foot\"\n").unwrap();
    assert_eq!(c.terminal, "foot");
    assert_eq!(c.keybinds.len(), 40);
    assert_eq!(c.theme, crate::theme::Theme::default());
}

#[test]
fn errors_are_reported() {
    assert!(Config::parse("terminal = ").is_err());
    assert!(Config::parse("[colors]\nactive_title = \"zzz\"\n").is_err());
    assert!(Config::parse("[colors]\nnope = \"#000000\"\n").is_err());
    assert!(Config::parse("[[keybinds]]\nkeys = \"Super+x\"\naction = \"dance\"\n").is_err());
    assert!(Config::parse("[[keybinds]]\nkeys = \"Super+x\"\naction = \"spawn\"\n").is_err());
}

#[test]
fn load_missing_file_gives_defaults() {
    let (c, notice, _) = Config::load(Some(std::path::Path::new("/nonexistent/reprise.toml")));
    assert_eq!(c.terminal, "reprise-terminal");
    assert_eq!(notice, None, "a missing file is not worth a notice");
}

#[test]
fn load_malformed_file_falls_back() {
    let path = std::env::temp_dir().join(format!("reprise-bad-config-{}.toml", std::process::id()));
    std::fs::write(&path, "terminal = \n[colors]\nactive_title = \"zzz\"\n").unwrap();
    let (c, notice, _) = Config::load(Some(&path));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(c, Config::default());
    assert!(notice.is_some(), "a file that is not TOML gets a notice");
}

#[test]
fn load_unreadable_file_gives_defaults_and_a_notice() {
    let dir = std::env::temp_dir();
    let (c, notice, _) = Config::load(Some(&dir));
    assert_eq!(c, Config::default());
    assert!(notice.is_some(), "a path that cannot be read gets a notice");
}

/// An unknown key or bad value costs only its own top-level key at login; `Config::parse` refuses the same file whole.
#[test]
fn load_drops_only_the_keys_that_fail() {
    let text = "terminal = \"foot\"\n\
                future_key = 1\n\
                [idle]\nlock_after = 0\n\
                [power]\nlid_close = \"standby\"\nnewer_key = true\n\
                [[outputs]]\nname = \"Virtual-1\"\nmode = \"1920x1080@60\"\n\
                [[keybinds]]\nkeys = \"Super+x\"\naction = \"dance\"\n";
    assert!(Config::parse(text).is_err());
    let (c, dropped) = Config::parse_lenient(text).unwrap();
    let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["future_key", "keybinds", "power.newer_key"]);
    assert_eq!(c.terminal, "foot");
    assert_eq!(c.idle.lock_after, 0);
    assert_eq!(c.outputs.len(), 1, "outputs keeps Virtual-1");
    assert_eq!(
        c.power.lid_close,
        PowerAction::Standby,
        "power keeps the keys that parse"
    );
    assert_eq!(
        c.keybinds,
        Config::default().keybinds,
        "keybinds are at their defaults"
    );

    let path =
        std::env::temp_dir().join(format!("reprise-skew-config-{}.toml", std::process::id()));
    std::fs::write(&path, text).unwrap();
    let (loaded, notice, _) = Config::load(Some(&path));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(loaded, c);
    let notice = notice.expect("dropped keys get a notice");
    assert!(
        notice.contains("future_key, keybinds, power.newer_key"),
        "{notice}"
    );
}

#[test]
fn a_reload_drops_only_the_keys_that_fail_and_names_them() {
    let path =
        std::env::temp_dir().join(format!("reprise-reload-config-{}.toml", std::process::id()));
    std::fs::write(&path, "terminal = \"foot\"\nfuture_key = 1\n").unwrap();
    let read = crate::configwatch::read(&path, &toml::Table::new());
    std::fs::write(&path, "terminal = [\n").unwrap();
    let broken = crate::configwatch::read(&path, &toml::Table::new());
    std::fs::remove_file(&path).unwrap();
    let (c, notice, _) = read.expect("the file reloads");
    assert_eq!(c.terminal, "foot", "the keys that parse apply");
    let notice = notice.expect("the dropped key gets a notice");
    assert!(notice.contains("future_key"), "{notice}");
    assert!(
        broken.is_none(),
        "a file that does not parse keeps the old config"
    );
}

#[test]
fn a_reload_keeps_the_running_value_of_a_key_that_fails() {
    let path =
        std::env::temp_dir().join(format!("reprise-reload-keep-{}.toml", std::process::id()));
    std::fs::write(&path, "[[outputs]]\nname = \"A\"\nscale = 2.0\n").unwrap();
    let (_, _, base) = crate::configwatch::read(&path, &toml::Table::new()).unwrap();
    std::fs::write(
        &path,
        "terminal = \"foot\"\n[[outputs]]\nname = \"A\"\nscale = \"1,5\"\n",
    )
    .unwrap();
    let read = crate::configwatch::read(&path, &base);
    std::fs::remove_file(&path).unwrap();
    let (c, notice, _) = read.expect("the file reloads");
    assert_eq!(c.outputs.len(), 1);
    assert_eq!(c.outputs[0].scale, 2.0, "the output keeps scale 2");
    assert_eq!(c.terminal, "foot", "the other keys in the file apply");
    let notice = notice.expect("the dropped key gets a notice");
    assert!(
        notice.contains("outputs") && notice.contains("previous"),
        "{notice}"
    );
}

#[test]
fn keyboard_section_parses_and_defaults() {
    let c = Config::parse("[keyboard]\nlayout = \"hu\"\nrepeat_rate = 30\n").unwrap();
    assert_eq!(c.keyboard.layout, "hu");
    assert_eq!(c.keyboard.repeat_rate, 30);
    assert_eq!(c.keyboard.repeat_delay, 600);
    let xkb = xkb_config(&c.keyboard);
    assert_eq!(xkb.rules, "");
    assert_eq!(xkb.layout, "hu");
    assert_eq!(xkb.options, None, "no options picks the defaults");
    assert_eq!(Config::default().keyboard.layout, "");
}

#[test]
fn example_config_parses() {
    let text = include_str!("../../config.example.toml");
    let c = Config::parse(text).unwrap();
    assert_eq!(
        Config::parse_lenient(text).unwrap(),
        (c.clone(), Vec::new()),
        "the login parse reads the example the same, dropping nothing"
    );
    assert_eq!(c.startmenu, Config::default().startmenu);
    assert_eq!(c.keybinds, Config::default().keybinds);
    assert!(c.outputs.is_empty());
}

#[test]
fn window_rules_parse_in_file_order_with_desktops_counted_from_one() {
    let c = Config::parse(
        "[windows]\nremember = false\n\n\
         [[window_rules]]\napp_id = \"foot\"\nsize = [800, 600]\nposition = [-10, 40]\n\
         output = \"DP-2\"\nstate = \"maximized\"\ndesktop = 2\nalways_on_top = true\n\
         sticky = false\ntab_group = \"shells\"\n\n\
         [[window_rules]]\napp_id = \"firefox\"\ntitle = \"*Mail*\"\n",
    )
    .unwrap();
    assert!(!c.remember_windows);
    assert_eq!(
        c.window_rules,
        [
            WindowRule {
                app_id: "foot".into(),
                title: None,
                size: Some((800, 600)),
                position: Some((-10, 40)),
                output: Some("DP-2".into()),
                state: Some(WindowState::Maximized),
                desktop: Some(1),
                always_on_top: Some(true),
                sticky: Some(false),
                tab_group: Some("shells".into()),
            },
            WindowRule {
                app_id: "firefox".into(),
                title: Some("*Mail*".into()),
                ..WindowRule::default()
            },
        ]
    );
    let d = Config::default();
    assert!(d.remember_windows && d.window_rules.is_empty());
}

#[test]
fn a_bad_window_rule_names_its_number_app_and_key() {
    let err = |text: &str| Config::parse(text).unwrap_err();
    let one = |body: &str| format!("[[window_rules]]\napp_id = \"x\"\n\n[[window_rules]]\n{body}");
    assert_eq!(
        err(&one("app_id = \"foot\"\nstate = \"big\"\n")),
        "window_rules #2 (\"foot\"): state \"big\" is not one of \
         normal, maximized, minimized, fullscreen"
    );
    assert_eq!(
        err(&one("app_id = \"foot\"\nsize = [0, 600]\n")),
        "window_rules #2 (\"foot\"): size must be two numbers from 1 to 32767, got [0, 600]"
    );
    assert_eq!(
        err(&one("app_id = \"foot\"\nposition = [0, 40000]\n")),
        "window_rules #2 (\"foot\"): position must be two numbers from -32768 to 32767, \
         got [0, 40000]"
    );
    assert_eq!(
        err(&one("app_id = \"foot\"\ndesktop = 0\n")),
        "window_rules #2 (\"foot\"): desktop must be between 1 and 16, got 0"
    );
    assert_eq!(
        err(&one("app_id = \"\"\n")),
        "window_rules #2 (\"\"): app_id is empty"
    );
    assert!(Config::parse("[[window_rules]]\ntitle = \"x\"\n").is_err());
    assert!(Config::parse("[[window_rules]]\napp_id = \"x\"\nsize = [1]\n").is_err());
    assert!(Config::parse("[[window_rules]]\napp_id = \"x\"\nwidth = 3\n").is_err());
    assert!(Config::parse("[windows]\nremember = 1\n").is_err());
}

#[test]
fn small_icons_shrinks_the_glass_bar_and_the_usable_area() {
    let full = |b: bool| {
        Config::parse(&format!(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = {b}\n"
        ))
        .unwrap()
        .theme
        .metrics
        .taskbar_height
    };
    assert_eq!(full(false), 40, "Glass's own bar height, unshrunk");
    assert_eq!(full(true), reprisekit::glass::SMALL_TASKBAR_HEIGHT);

    let screen = reprisekit::raster::Rect::new(0, 0, 1920, 1080);
    let usable = |h| {
        let p = crate::barplace::place(screen, reprisekit::taskbar::Edge::Bottom, h);
        crate::barplace::usable(screen, &p)
    };
    let usable_off = usable(full(false));
    let usable_on = usable(full(true));
    assert_eq!(usable_off.height, 1040);
    assert_eq!(usable_on.height, 1050, "10 px taller, the bar's own shrink");

    let classic = |b: bool| {
        Config::parse(&format!("[taskbar]\nsmall_icons = {b}\n"))
            .unwrap()
            .theme
            .metrics
            .taskbar_height
    };
    assert_eq!(classic(false), classic(true), "Classic ignores the key");
    let blue = |b: bool| {
        Config::parse(&format!(
            "[appearance]\ninterface = \"blue\"\n[taskbar]\nsmall_icons = {b}\n"
        ))
        .unwrap()
        .theme
        .metrics
        .taskbar_height
    };
    assert_eq!(blue(false), blue(true), "Blue ignores the key");

    let c = Config::parse(
        "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = true\n[metrics]\ntaskbar_height = 36\n",
    )
    .unwrap();
    assert_eq!(c.theme.metrics.taskbar_height, 36);
}
