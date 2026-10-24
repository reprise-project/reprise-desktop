use super::parse::{home_path, wallpaper_image};
use super::*;
use crate::keys::*;
use crate::theme::Color;
use crate::windowrules::WindowState;
use std::path::{Path, PathBuf};

mod appearance;
mod keybinds;
mod outputs;

#[test]
fn the_portal_table_belongs_to_the_portal() {
    assert!(Config::parse("[portal]\nshortcuts_denied = [\"com.example.App\"]\n").is_ok());
}

#[test]
fn the_thumbnail_cache_table_parses() {
    assert!(Config::parse("[thumbnail_cache]\nenabled = false\nmax_mb = 64\n").is_ok());
}

#[test]
fn idle_section_parses_and_defaults() {
    let c = Config::parse("[idle]\nlock_after = 10\n").unwrap();
    assert_eq!(c.idle.lock_after, 10);
    assert_eq!(c.idle.screen_off_after, 900);
    let c = Config::parse("[idle]\nlock_after = 0\nscreen_off_after = 0\nlock_fade = 3\n").unwrap();
    assert_eq!(
        c.idle,
        IdleConfig {
            lock_after: 0,
            screen_off_after: 0,
            blank_after: 0,
            suspend_after: 0,
            lock_fade: 3,
        }
    );
    assert!(Config::parse("[idle]\nlock_after = -1\n").is_err());
    assert_eq!(
        Config::parse("[idle]\nlock_after = 4294967296\n").unwrap_err(),
        "idle.lock_after must be 0..=4294967295, got 4294967296"
    );
    assert!(Config::parse("[idle]\nbogus = 1\n").is_err());
    assert_eq!(Config::default().idle, IdleConfig::default());
}

#[test]
fn idle_battery_overrides_fall_back_to_idle() {
    let c = Config::parse("[idle]\nscreen_off_after = 100\n[idle.battery]\nsuspend_after = 5\n")
        .unwrap();
    assert_eq!(
        c.idle_for(true),
        IdleConfig {
            lock_after: 600,
            screen_off_after: 100,
            blank_after: 0,
            suspend_after: 5,
            lock_fade: 0,
        }
    );
    assert_eq!(c.idle_for(false).suspend_after, 0);
    assert_eq!(c.idle_for(false).screen_off_after, 100);
    assert!(Config::parse("[idle.battery]\nsuspend_after = -1\n").is_err());
    assert!(Config::parse("[idle.battery]\nbogus = 1\n").is_err());
    assert_eq!(Config::default().idle_for(true), IdleConfig::default());
}

#[test]
fn power_actions_parse_by_name() {
    for (name, action) in PowerAction::NAMES {
        assert_eq!(PowerAction::parse(name), Ok(action));
        assert_eq!(action.name(), name);
    }
    assert!(PowerAction::parse("bogus").is_err());
}

#[test]
fn power_keys_parse() {
    let c = Config::parse(
        "[power]\nhibernate = \"h\"\nlid_close = \"screen_off\"\npower_button = \"nothing\"\n\
         sleep_button = \"hibernate\"\nbattery_low = 20\nbattery_critical = 5\n\
         critical_action = \"turnoff\"\nalways_show_icon = true\n",
    )
    .unwrap();
    assert_eq!(c.power.hibernate, "h");
    assert_eq!(c.power.lid_close, PowerAction::ScreenOff);
    assert_eq!(c.power.power_button, PowerAction::Nothing);
    assert_eq!(c.power.sleep_button, PowerAction::Hibernate);
    assert_eq!(c.power.battery_low, 20);
    assert_eq!(c.power.battery_critical, 5);
    assert_eq!(c.power.critical_action, PowerAction::TurnOff);
    assert!(c.power.always_show_icon);
    assert!(Config::parse("[power]\nlid_close = \"bogus\"\n").is_err());
    assert!(Config::parse("[power]\ncritical_action = \"ask\"\n").is_err());
    assert!(Config::parse("[power]\nbattery_low = 101\n").is_err());
    let d = Config::default().power;
    assert_eq!(d.power_button, PowerAction::Ask);
    assert_eq!(d.lid_close, PowerAction::Standby);
    assert!(d.lock_on_sleep);
    let c = Config::parse("[power]\nlock_on_sleep = false\n").unwrap();
    assert!(!c.power.lock_on_sleep);
    assert_eq!(d.locker, "");
    let c = Config::parse("[power]\nlocker = \"swaylock -f\"\n").unwrap();
    assert_eq!(c.power.locker, "swaylock -f");
}

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
fn previews_default_on_and_each_turns_off() {
    assert_eq!(
        Config::default().previews,
        PreviewsConfig {
            taskbar: true,
            fps: 10
        }
    );
    let c = Config::parse("[previews]\ntaskbar = false\n").unwrap();
    assert!(!c.previews.taskbar);
    assert_eq!(
        Config::parse("[previews]\nfps = 30\n")
            .unwrap()
            .previews
            .fps,
        30
    );
    assert!(Config::parse("[previews]\nfps = 0\n").is_err());
    assert!(Config::parse("[previews]\nfps = 61\n").is_err());
    assert!(Config::parse("[previews]\nfps = 12.5\n").is_err());
    assert!(Config::parse("[previews]\ncolour = true\n").is_err());
    assert!(
        Config::parse("[previews]\nswitcher = false\n").is_err(),
        "the Alt+Tab preview lives in [switcher] now"
    );
}

#[test]
fn switcher_keys_follow_the_interface() {
    let classic = Config::default().switcher;
    assert_eq!(
        classic,
        SwitcherConfig {
            style: SwitcherStyle::Icons,
            preview: true,
            desktop: false,
            peek: false,
            flip: false,
            size: reprisekit::switcher::Size::Small,
        }
    );
    let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
        .unwrap()
        .switcher;
    assert_eq!(
        glass,
        SwitcherConfig {
            style: SwitcherStyle::Thumbnails,
            preview: true,
            desktop: true,
            peek: true,
            flip: true,
            size: reprisekit::switcher::Size::Small,
        }
    );
    let blue = Config::parse("[appearance]\ninterface = \"blue\"\n")
        .unwrap()
        .switcher;
    assert_eq!(blue, classic);
}

#[test]
fn switcher_keys_override_the_interface() {
    let c = Config::parse(
        "[appearance]\ninterface = \"glass\"\n[switcher]\nstyle = \"icons\"\n\
         preview = false\ndesktop = false\npeek = false\nflip = false\n",
    )
    .unwrap();
    assert_eq!(
        c.switcher,
        SwitcherConfig {
            style: SwitcherStyle::Icons,
            preview: false,
            desktop: false,
            peek: false,
            flip: false,
            size: reprisekit::switcher::Size::Small,
        }
    );
    let c = Config::parse("[switcher]\nstyle = \"thumbnails\"\ndesktop = true\n").unwrap();
    assert_eq!(
        (c.switcher.style, c.switcher.desktop),
        (SwitcherStyle::Thumbnails, true)
    );
    assert!(Config::parse("[switcher]\nstyle = \"cards\"\n").is_err());
    assert_eq!(
        Config::parse("[switcher]\nsize = \"large\"\n")
            .unwrap()
            .switcher
            .size,
        reprisekit::switcher::Size::Large
    );
    assert_eq!(
        Config::parse("[switcher]\nsize = \"huge\"\n")
            .unwrap()
            .switcher
            .size,
        reprisekit::switcher::Size::Huge
    );
    assert!(Config::parse("[switcher]\nsize = \"giant\"\n").is_err());
}

#[test]
fn the_hot_corner_is_off_until_a_corner_is_named() {
    assert_eq!(Config::default().hot_corner, HotCorner::None);
    let c = Config::parse("[overview]\nhot_corner = \"top-left\"\n").unwrap();
    assert_eq!(c.hot_corner, HotCorner::TopLeft);
    assert!(Config::parse("[overview]\nhot_corner = \"middle\"\n").is_err());
    for h in HotCorner::ALL {
        assert_eq!(HotCorner::from_name(h.name()), Some(h));
    }
}

#[test]
fn gestures_take_three_fingers_until_told_four() {
    let d = Config::default().gestures;
    assert_eq!((d.enabled, d.fingers), (true, 3));
    let c = Config::parse("[gestures]\nenabled = false\nfingers = 4\n").unwrap();
    assert_eq!((c.gestures.enabled, c.gestures.fingers), (false, 4));
    assert!(Config::parse("[gestures]\nfingers = 2\n").is_err());
}

#[test]
fn gesture_tuning_defaults_and_ranges() {
    let d = Config::default().gestures;
    assert_eq!((d.distance, d.commit_at, d.flick_speed), (400, 30, 300));
    let c = Config::parse("[gestures]\ndistance = 600\ncommit_at = 50\nflick_speed = 800\n")
        .unwrap()
        .gestures;
    assert_eq!((c.distance, c.commit_at, c.flick_speed), (600, 50, 800));
    for bad in [
        "distance = 99",
        "distance = 2001",
        "commit_at = 4",
        "commit_at = 96",
        "flick_speed = 49",
        "flick_speed = 5001",
        "flick_speed = -1",
    ] {
        assert!(
            Config::parse(&format!("[gestures]\n{bad}\n")).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn screenshot_save_defaults_on_and_is_configurable() {
    assert!(Config::default().screenshot.save);
    let c = Config::parse("[screenshot]\nsave = false\n").unwrap();
    assert!(!c.screenshot.save);
    assert!(Config::parse("[screenshot]\nfolder = \"x\"\n").is_err());
}

#[test]
fn night_light_is_off_by_default_and_every_key_is_checked() {
    let d = Config::default().night_light;
    assert!(!d.enabled);
    assert_eq!((d.temperature, d.schedule), (4000, NightSchedule::Sun));
    assert_eq!((d.from, d.to, d.location), (21 * 60, 7 * 60, None));
    let c = Config::parse(
        "[night_light]\nenabled = true\ntemperature = 3400\nschedule = \"fixed\"\n\
         from = \"20:30\"\nto = \"6:05\"\nlatitude = 47.5\nlongitude = 19.08\n",
    )
    .unwrap()
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
    use crate::magnifier::Follow;
    use reprisekit::colorfilter::Filter;
    let d = Config::default().accessibility;
    assert_eq!(d.color_filter, Filter::None);
    assert_eq!(d.color_filter_intensity, 100);
    assert_eq!(d.magnifier_follow, Follow::PointerAndFocus);
    let c = Config::parse(
        "[accessibility]\ncolor_filter = \"tritanopia\"\n\
         color_filter_intensity = 40\nmagnifier_follow = \"pointer\"\n",
    )
    .unwrap()
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
fn the_key_aids_are_off_with_their_shortcuts_on_by_default() {
    let d = Config::default().accessibility;
    assert!(!d.sticky_keys && !d.slow_keys);
    assert!(d.sticky_lock && d.sticky_two_keys_off && d.shortcuts);
    assert_eq!(d.slow_keys_delay, 1000);
    let c = Config::parse(
        "[accessibility]\nsticky_keys = true\nsticky_lock = false\n\
         sticky_two_keys_off = false\nslow_keys = true\nslow_keys_delay = 0.3\n\
         shortcuts = false\n",
    )
    .unwrap()
    .accessibility;
    assert!(c.sticky_keys && c.slow_keys);
    assert!(!c.sticky_lock && !c.sticky_two_keys_off && !c.shortcuts);
    assert_eq!(c.slow_keys_delay, 300);
    for bad in [
        "slow_keys_delay = 0.2",
        "slow_keys_delay = 2.5",
        "sticky = true",
    ] {
        assert!(
            Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
            "{bad} is refused"
        );
    }
}

#[test]
fn mouse_keys_and_the_text_size_parse_within_their_ranges() {
    let d = Config::default().accessibility;
    assert!(!d.mouse_keys && d.mouse_keys_num_lock);
    assert_eq!((d.mouse_keys_speed, d.mouse_keys_acceleration), (400, 1000));
    assert_eq!(d.text_size, 100);
    let c = Config::parse(
        "[accessibility]\nmouse_keys = true\nmouse_keys_speed = 800\n\
         mouse_keys_acceleration = 0.5\nmouse_keys_num_lock = false\ntext_size = 150\n",
    )
    .unwrap()
    .accessibility;
    assert!(c.mouse_keys && !c.mouse_keys_num_lock);
    assert_eq!((c.mouse_keys_speed, c.mouse_keys_acceleration), (800, 500));
    assert_eq!(c.text_size, 150);
    for bad in [
        "mouse_keys_speed = 10",
        "mouse_keys_speed = 5000",
        "mouse_keys_acceleration = -1.0",
        "mouse_keys_acceleration = 6.0",
        "text_size = 75",
        "text_size = 140",
        "text_size = 225",
    ] {
        assert!(
            Config::parse(&format!("[accessibility]\n{bad}\n")).is_err(),
            "{bad} is refused"
        );
    }
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
fn menus_fade_unless_told_to_slide_or_keep_still() {
    let opening = |text: &str| Config::parse(text).unwrap().menu_opening();
    assert_eq!(opening(""), MenuAnimation::Fade);
    assert_eq!(
        opening("[appearance]\ninterface = \"glass\"\n"),
        MenuAnimation::Fade
    );
    assert_eq!(
        opening("[effects]\nmenu_animation = \"slide\"\n"),
        MenuAnimation::Slide
    );
    assert_eq!(
        opening("[effects]\nmenu_animation = \"none\"\n"),
        MenuAnimation::None
    );
    assert_eq!(
        opening("[metrics]\nmenu_fade_ms = 0\n[effects]\nmenu_animation = \"slide\"\n"),
        MenuAnimation::None,
        "a 0 ms opening is no animation"
    );
    assert!(Config::parse("[effects]\nmenu_animation = \"roll\"\n").is_err());
}

#[test]
fn effects_follow_the_interface_and_the_keys_override_them() {
    let classic = Config::parse("").unwrap().effects;
    assert_eq!(classic, EffectsConfig::default_for(Interface::Classic));
    assert!(
        !classic.window_animation && !classic.hover_fade,
        "Classic stays still"
    );
    assert_eq!(
        (classic.window_open_ms, classic.window_close_ms),
        (250, 200)
    );
    assert_eq!(classic.window_scale, 0.95);
    assert_eq!((classic.hover_in_ms, classic.hover_out_ms), (120, 250));
    let blue = Config::parse("[appearance]\ninterface = \"blue\"\n")
        .unwrap()
        .effects;
    assert_eq!(blue, classic, "Blue is still by default");
    let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
        .unwrap()
        .effects;
    assert!(
        glass.window_animation && glass.hover_fade,
        "Glass moves by default"
    );
    assert!(glass.opens() && glass.closes());
    assert_eq!(glass.hover_ms(), (120, 250));
    let on_blue = Config::parse(
        "[appearance]\ninterface = \"blue\"\n[effects]\nwindow_animation = true\nhover_fade = true\nwindow_open_ms = 400\nwindow_scale = 0.9\n",
    )
    .unwrap()
    .effects;
    assert!(on_blue.window_animation && on_blue.hover_fade);
    assert_eq!(on_blue.window_open_ms, 400);
    assert_eq!(on_blue.window_scale, 0.9);
    let off = Config::parse(
        "[appearance]\ninterface = \"glass\"\n[effects]\nhover_fade = false\nwindow_close_ms = 0\n",
    )
    .unwrap()
    .effects;
    assert_eq!(off.hover_ms(), (0, 0), "no fade: both instant");
    assert!(off.opens() && !off.closes(), "0 ms is no close animation");
}

#[test]
fn the_session_fades_under_every_interface_and_the_keys_override_it() {
    let parse = |s: &str| Config::parse(s).unwrap().effects.session_fade_ms();
    assert_eq!(parse(""), (600, 400));
    assert_eq!(parse("[appearance]\ninterface = \"blue\"\n"), (600, 400));
    assert_eq!(parse("[appearance]\ninterface = \"glass\"\n"), (600, 400));
    assert_eq!(
        parse("[effects]\nsession_fade_in_ms = 0\nsession_fade_out_ms = 2000\n"),
        (0, 2000)
    );
    assert_eq!(
        parse("[effects]\nsession_fade = false\nsession_fade_in_ms = 900\n"),
        (0, 0),
        "off: both instant"
    );
    for bad in ["session_fade_in_ms = 2001", "session_fade_out_ms = -1"] {
        assert!(
            Config::parse(&format!("[effects]\n{bad}\n")).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn task_buttons_slide_on_glass_by_default_and_the_key_overrides_it() {
    let parse = |s: &str| Config::parse(s).unwrap().effects.taskbar_slide;
    assert!(!parse(""), "Classic stays still");
    assert!(
        !parse("[appearance]\ninterface = \"blue\"\n"),
        "so does Blue"
    );
    assert!(parse("[appearance]\ninterface = \"glass\"\n"));
    assert!(parse("[effects]\ntaskbar_slide = true\n"));
    assert!(!parse(
        "[appearance]\ninterface = \"glass\"\n[effects]\ntaskbar_slide = false\n"
    ));
}

#[test]
fn effects_values_out_of_range_are_errors() {
    for bad in [
        "window_open_ms = -1",
        "window_open_ms = 2001",
        "window_close_ms = 2001",
        "hover_in_ms = 2001",
        "hover_out_ms = -5",
        "window_scale = 0.79",
        "window_scale = 1.01",
        "window_scale = nan",
        "wobble = true",
    ] {
        assert!(
            Config::parse(&format!("[effects]\n{bad}\n")).is_err(),
            "{bad}"
        );
    }
    assert!(
        Config::parse("[effects]\nwindow_scale = 0.8\nhover_in_ms = 0\nhover_out_ms = 2000\n")
            .is_ok()
    );
}

#[test]
fn desktop_switches_are_instant_by_default_and_the_animation_is_checked() {
    let cfg = Config::default();
    assert_eq!(cfg.workspace_animation.style, SwitchStyle::None);
    assert_eq!(cfg.workspace_animation.ms, 200);
    assert!(!cfg.workspace_animation.enabled(), "none is no animation");
    for style in SwitchStyle::ALL {
        let c = Config::parse(&format!("[workspaces]\nanimation = {:?}\n", style.name())).unwrap();
        assert_eq!(c.workspace_animation.style, style);
    }
    let c = Config::parse("[workspaces]\nanimation = \"fade\"\nanimation_ms = 0\n").unwrap();
    assert_eq!(c.workspace_animation.ms, 0);
    assert!(!c.workspace_animation.enabled(), "0 ms is no animation");
    let c = Config::parse("[workspaces]\nanimation = \"none\"\n").unwrap();
    assert!(!c.workspace_animation.enabled());
    let c = Config::parse("[workspaces]\nanimation_ms = 350\n").unwrap();
    assert_eq!(c.workspaces, 4);
    assert_eq!(c.workspace_animation.style, SwitchStyle::None);
    assert_eq!(c.workspace_animation.ms, 350);
    assert!(Config::parse("[workspaces]\nanimation = \"spin\"\n").is_err());
    assert!(Config::parse("[workspaces]\nanimation_ms = -1\n").is_err());
    assert!(Config::parse("[workspaces]\nanimation_ms = 2001\n").is_err());
}

#[test]
fn the_pager_mark_defaults_to_light_and_rejects_others() {
    use reprisekit::taskbar::PagerMark;
    assert_eq!(Config::default().pager_mark, PagerMark::Light);
    let c = Config::parse("[workspaces]\npager = \"tile\"\n").unwrap();
    assert_eq!(c.pager_mark, PagerMark::Tile);
    let c = Config::parse("[workspaces]\npager = \"light\"\n").unwrap();
    assert_eq!(c.pager_mark, PagerMark::Light);
    let err = Config::parse("[workspaces]\npager = \"box\"\n").unwrap_err();
    assert!(
        err.contains("\"light\"") && err.contains("\"tile\""),
        "the error names both choices: {err}"
    );
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
    let (c, notice) = Config::load(Some(std::path::Path::new("/nonexistent/reprise.toml")));
    assert_eq!(c.terminal, "reprise-terminal");
    assert_eq!(notice, None, "a missing file is not worth a notice");
}

#[test]
fn load_malformed_file_falls_back() {
    let path = std::env::temp_dir().join(format!("reprise-bad-config-{}.toml", std::process::id()));
    std::fs::write(&path, "terminal = \n[colors]\nactive_title = \"zzz\"\n").unwrap();
    let (c, notice) = Config::load(Some(&path));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(c, Config::default());
    assert!(notice.is_some(), "a file that is not TOML gets a notice");
}

#[test]
fn load_unreadable_file_gives_defaults_and_a_notice() {
    let dir = std::env::temp_dir();
    let (c, notice) = Config::load(Some(&dir));
    assert_eq!(c, Config::default());
    assert!(notice.is_some(), "a path that cannot be read gets a notice");
}

/// An unknown key or bad value costs only its own top-level key at login; `Config::parse` refuses the same file whole.
#[test]
fn load_drops_only_the_sections_that_fail() {
    let text = "terminal = \"foot\"\n\
                future_key = 1\n\
                [idle]\nlock_after = 0\n\
                [power]\nlid_close = \"standby\"\nnewer_key = true\n\
                [[outputs]]\nname = \"Virtual-1\"\nmode = \"1920x1080@60\"\n\
                [[keybinds]]\nkeys = \"Super+x\"\naction = \"dance\"\n";
    assert!(Config::parse(text).is_err());
    let (c, dropped) = Config::parse_lenient(text).unwrap();
    let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["future_key", "keybinds", "power"]);
    assert_eq!(c.terminal, "foot");
    assert_eq!(c.idle.lock_after, 0);
    assert_eq!(c.outputs.len(), 1, "outputs keeps Virtual-1");
    assert_eq!(c.power, Config::default().power, "power is at its defaults");
    assert_eq!(
        c.keybinds,
        Config::default().keybinds,
        "keybinds are at their defaults"
    );

    let path =
        std::env::temp_dir().join(format!("reprise-skew-config-{}.toml", std::process::id()));
    std::fs::write(&path, text).unwrap();
    let (loaded, notice) = Config::load(Some(&path));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(loaded, c);
    let notice = notice.expect("dropped keys get a notice");
    assert!(notice.contains("future_key, keybinds, power"), "{notice}");
}

#[test]
fn keyboard_section_parses_and_defaults() {
    let c = Config::parse("[keyboard]\nlayout = \"hu\"\nrepeat_rate = 30\n").unwrap();
    assert_eq!(c.keyboard.layout, "hu");
    assert_eq!(c.keyboard.repeat_rate, 30);
    assert_eq!(c.keyboard.repeat_delay, 600);
    let xkb = c.keyboard.xkb_config();
    assert_eq!(xkb.rules, "");
    assert_eq!(xkb.layout, "hu");
    assert_eq!(xkb.options, None, "no options picks the defaults");
    assert_eq!(Config::default().keyboard.layout, "");
}

#[test]
fn recent_files_default_on_and_privacy_turns_them_off() {
    assert!(Config::default().recent_files);
    assert!(
        !Config::parse("[privacy]\nrecent_files = false\n")
            .unwrap()
            .recent_files
    );
}

#[test]
fn location_default_on_and_privacy_turns_it_off() {
    assert!(Config::default().location);
    assert!(
        !Config::parse("[privacy]\nlocation = false\n")
            .unwrap()
            .location
    );
}

#[test]
fn capture_border_default_on_and_privacy_turns_it_off() {
    assert!(Config::default().capture_border);
    assert!(
        !Config::parse("[privacy]\ncapture_border = false\n")
            .unwrap()
            .capture_border
    );
}

#[test]
fn the_debug_overlay_is_off_and_left_out_of_captures_by_default() {
    assert_eq!(Config::default().debug, DebugConfig::default());
    assert!(!Config::default().debug.overlay);
    let d = Config::parse("[debug]\noverlay = true\noverlay_in_captures = true\n")
        .unwrap()
        .debug;
    assert!(d.overlay && d.overlay_in_captures);
}

#[test]
fn the_launcher_searches_files_unless_turned_off() {
    assert!(Config::default().launcher_files);
    assert!(
        !Config::parse("[launcher]\nsearch_files = false\n")
            .unwrap()
            .launcher_files
    );
}

#[test]
fn script_terminal_defaults_to_stay_and_reads_close() {
    use reprisefs::program::ScriptEnd;
    assert_eq!(Config::default().script_terminal, ScriptEnd::Stay);
    let c = Config::parse("script_terminal = \"close\"\n").unwrap();
    assert_eq!(c.script_terminal, ScriptEnd::Close);
    let c = Config::parse("script_terminal = \"stay\"\n").unwrap();
    assert_eq!(c.script_terminal, ScriptEnd::Stay);
    assert!(Config::parse("script_terminal = \"open\"\n").is_err());
}

#[test]
fn desktop_extensions_default_to_hide_and_read_each_mode() {
    use reprisefs::names::Extensions;
    assert_eq!(Config::default().desktop_extensions, Extensions::Hide);
    for (text, want) in [
        ("hide", Extensions::Hide),
        ("hide_launchers", Extensions::HideLaunchers),
        ("show", Extensions::Show),
    ] {
        let c = Config::parse(&format!("desktop_extensions = \"{text}\"\n")).unwrap();
        assert_eq!(c.desktop_extensions, want, "{text}");
    }
    let err = Config::parse("desktop_extensions = \"all\"\n").unwrap_err();
    assert!(err.contains("hide, hide_launchers or show"), "{err}");
}

#[test]
fn desktop_icons_defaults_on_and_is_configurable() {
    assert!(Config::default().desktop_icons);
    assert_eq!(Config::default().system_icons.len(), 4);
    assert_eq!(
        Config::parse("system_icons = [\"recycle-bin\"]\n")
            .unwrap()
            .system_icons,
        ["recycle-bin"]
    );
    assert!(
        Config::parse("system_icons = []\n")
            .unwrap()
            .system_icons
            .is_empty()
    );
    assert_eq!(
        Config::parse(
            "system_icons = [\"my-computer\", \"my-network-places\", \"my-documents\"]\n"
        )
        .unwrap()
        .system_icons,
        ["computer", "documents"],
        "an older file's names read as the current ones"
    );
    assert!(
        !Config::parse("desktop_icons = false\n")
            .unwrap()
            .desktop_icons
    );
}

#[test]
fn keyboard_repeat_rejects_negative() {
    assert!(Config::parse("[keyboard]\nrepeat_rate = -1\n").is_err());
    assert!(Config::parse("[keyboard]\nrepeat_delay = -1\n").is_err());
    assert_eq!(
        Config::parse("[keyboard]\nrepeat_rate = 0\n")
            .unwrap()
            .keyboard
            .repeat_rate,
        0
    );
}

#[test]
fn pointer_sections_parse_and_default() {
    let d = Config::default();
    assert_eq!(
        d.mouse,
        PointerConfig {
            speed: 0.0,
            acceleration: true,
            natural_scroll: false,
        }
    );
    assert_eq!(
        d.touchpad,
        TouchpadConfig {
            pointer: PointerConfig {
                speed: 0.0,
                acceleration: true,
                natural_scroll: false,
            },
            tap_to_click: true,
            two_finger_scroll: true,
            two_finger_tap_right_click: true,
            disable_while_typing: true,
        }
    );
    let c = Config::parse(
        "[mouse]\nspeed = -0.5\nacceleration = false\nnatural_scroll = true\n\
         [touchpad]\nspeed = 1\ntap_to_click = false\ntwo_finger_scroll = false\n\
         disable_while_typing = false\n",
    )
    .unwrap();
    assert_eq!(c.mouse.speed, -0.5);
    assert!(!c.mouse.acceleration);
    assert!(c.mouse.natural_scroll);
    assert_eq!(c.touchpad.pointer.speed, 1.0);
    assert!(
        !c.touchpad.pointer.natural_scroll,
        "each class keeps its own direction"
    );
    assert!(c.touchpad.pointer.acceleration);
    assert!(!c.touchpad.tap_to_click);
    assert!(!c.touchpad.two_finger_scroll);
    assert!(c.touchpad.two_finger_tap_right_click);
    assert!(!c.touchpad.disable_while_typing);
    let c = Config::parse("[touchpad]\nnatural_scroll = true\n").unwrap();
    assert!(c.touchpad.pointer.natural_scroll);
    assert!(!c.mouse.natural_scroll);
}

#[test]
fn pointer_speed_rejects_out_of_range() {
    assert!(Config::parse("[mouse]\nspeed = 1.5\n").is_err());
    assert!(Config::parse("[touchpad]\nspeed = -2\n").is_err());
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
fn session_restore_is_off_by_default_and_reads_its_exclusions() {
    let d = Config::default();
    assert!(!d.session.restore && d.session.exclude.is_empty());
    let c = Config::parse("[session]\nrestore = true\nexclude = [\"foot\", \"org.*\"]\n").unwrap();
    assert_eq!(
        c.session,
        SessionConfig {
            restore: true,
            exclude: vec!["foot".into(), "org.*".into()],
            end_processes: true,
        }
    );
    assert!(Config::parse("[session]\nrestor = true\n").is_err());
}

#[test]
fn log_off_ends_leftover_processes_unless_turned_off() {
    assert!(Config::default().session.end_processes);
    let c = Config::parse("[session]\nend_processes = false\n").unwrap();
    assert!(!c.session.end_processes);
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
fn window_tabs_are_on_unless_the_file_turns_them_off() {
    assert!(Config::default().window_tabs);
    let c = Config::parse("[windows]\ntabs = false\n").unwrap();
    assert!(!c.window_tabs, "tabs = false turns tab groups off");
    assert!(c.remember_windows, "remember keeps its default");
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
fn retired_keys_still_load_and_are_named() {
    let c = Config::parse(
        "renderer = \"vulkan\"\n\n[[outputs]]\nname = \"DP-1\"\ncolor_profile = \"dell.icc\"\nhdr = \"yes\"\n\n[[outputs]]\nname = \"eDP-1\"\nhdr = true\n",
    )
    .expect("retired keys parse whatever their value");
    assert_eq!(
        c.retired,
        [
            "renderer",
            "outputs.DP-1.color_profile",
            "outputs.DP-1.hdr",
            "outputs.eDP-1.hdr"
        ]
    );
    assert!(
        Config::parse("terminal = \"foot\"\n")
            .unwrap()
            .retired
            .is_empty()
    );
}

#[test]
fn a_home_path_is_absolute_or_under_home() {
    let home = Some(std::ffi::OsStr::new("/home/ann"));
    assert_eq!(
        home_path("~/Pictures/a.png", home),
        Ok(PathBuf::from("/home/ann/Pictures/a.png"))
    );
    assert_eq!(home_path("/a.png", None), Ok(PathBuf::from("/a.png")));
    assert!(home_path("~/a.png", None).is_err());
    assert!(home_path("pics/a.png", home).is_err());
    assert!(home_path("~user/a.png", home).is_err());
}

#[test]
fn window_snapping_is_on_by_default_and_the_old_grid_keys_are_ignored() {
    let c = Config::parse("").unwrap();
    assert!(c.window_snapping.enabled, "drag snapping is on");
    assert!(c.window_snapping.picker, "the picker is on");
    assert!(c.window_snapping.layouts.is_empty(), "built-in layouts");
    let old = Config::parse(
        "[window_snapping]\nthreshold = 20\ncolumns = 2\ncolumn_weights = [1, 1]\n\
         rows = 2\nrow_weights = [1]\n\n[[outputs]]\nname = \"DP-2\"\n\
         [outputs.window_snapping]\ncolumn_weights = [1, 2, 1]\nbogus = 1\n",
    )
    .expect("the grid's keys parse and are ignored");
    assert!(old.window_snapping.enabled);
    assert!(
        Config::parse("[window_snapping]\nbogus = 1\n").is_err(),
        "a typo still fails"
    );
}

#[test]
fn window_snapping_distance_defaults_to_the_band_and_is_bounded() {
    assert_eq!(Config::parse("").unwrap().window_snapping.distance, 30);
    let c = Config::parse("[window_snapping]\ndistance = 15\n").unwrap();
    assert_eq!(c.window_snapping.distance, 15);
    for bad in [0, 101] {
        let text = format!("[window_snapping]\ndistance = {bad}\n");
        assert!(Config::parse(&text).is_err(), "distance {bad} is refused");
    }
}

#[test]
fn the_snap_bar_is_on_by_default_and_can_be_turned_off() {
    assert!(Config::parse("").unwrap().window_snapping.bar);
    let c = Config::parse("[window_snapping]\nbar = false\n").unwrap();
    assert!(!c.window_snapping.bar);
}

#[test]
fn window_shake_is_on_under_glass_alone_unless_set() {
    let shake = |text: &str| Config::parse(text).unwrap().window_shake;
    let glass = "[appearance]\ninterface = \"glass\"\n";
    assert!(!shake(""));
    assert!(!shake("[appearance]\ninterface = \"blue\"\n"));
    assert!(shake(glass));
    assert!(shake(&format!("{glass}[windows]\nshake = \"auto\"\n")));
    assert!(!shake(&format!("{glass}[windows]\nshake = false\n")));
    assert!(shake("[windows]\nshake = true\n"));
    let bad = Config::parse("[windows]\nshake = \"yes\"\n").unwrap_err();
    assert!(bad.contains("[windows] shake"), "{bad}");
}

#[test]
fn the_snap_layout_picker_reads_its_switch_and_layouts() {
    let c = Config::parse("[window_snapping]\npicker = false\nlayouts = [\"1|1\", \"3|1:2\"]\n")
        .unwrap();
    assert!(!c.window_snapping.picker);
    assert_eq!(c.window_snapping.layouts.len(), 2);
    assert_eq!(c.window_snapping.layouts[1].bands, vec![(3.0, 1), (1.0, 2)]);
    let bad = Config::parse("[window_snapping]\nlayouts = [\"1|x\"]\n").unwrap_err();
    assert!(bad.contains("window_snapping: layouts"), "{bad}");
    let seven = ["\"1\""; 7].join(", ");
    let many = format!("[window_snapping]\nlayouts = [{seven}]\n");
    assert!(Config::parse(&many).is_err());
}
