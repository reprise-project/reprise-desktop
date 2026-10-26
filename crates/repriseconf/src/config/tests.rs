use super::parse::{home_path, wallpaper_image};
use super::*;
use reprisekit::theme::Color;
use std::path::{Path, PathBuf};

mod appearance;
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
fn script_open_defaults_to_run_and_reads_terminal() {
    use reprisefs::program::ScriptOpen;
    assert_eq!(Config::default().script_open, ScriptOpen::Run);
    let c = Config::parse("script_open = \"terminal\"\n").unwrap();
    assert_eq!(c.script_open, ScriptOpen::Terminal);
    assert!(Config::parse("script_open = \"stay\"\n").is_err());
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
fn window_tabs_are_on_unless_the_file_turns_them_off() {
    assert!(Config::default().window_tabs);
    let c = Config::parse("[windows]\ntabs = false\n").unwrap();
    assert!(!c.window_tabs, "tabs = false turns tab groups off");
    assert!(c.remember_windows, "remember keeps its default");
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

#[test]
fn follow_names_round_trip() {
    for f in [Follow::Pointer, Follow::PointerAndFocus] {
        assert_eq!(Follow::parse(f.name()), Some(f));
    }
    assert_eq!(Follow::parse("caret"), None);
    assert!(!Follow::Pointer.focus() && Follow::PointerAndFocus.focus());
}

#[test]
fn a_bad_key_in_a_table_drops_that_key_alone() {
    let text = "[taskbar]\nposition = \"botom\"\nauto_hide = true\nside_width = 300\n";
    assert!(Config::parse(text).is_err());
    let (c, dropped) = Config::parse_lenient(text).unwrap();
    let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["taskbar.position"]);
    assert!(
        c.taskbar.auto_hide,
        "auto_hide survives its neighbour's typo"
    );
    assert_eq!(c.taskbar.side_width, 300);
    assert_eq!(c.taskbar.position, reprisekit::taskbar::Edge::Bottom);
}

#[test]
fn a_table_whose_keys_fail_only_together_is_dropped_whole() {
    let text = "[night_light]\nenabled = true\nfrom = \"08:00\"\nto = \"08:00\"\ntemperature = 1\n";
    let (c, dropped) = Config::parse_lenient(text).unwrap();
    let keys: Vec<&str> = dropped.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["night_light"]);
    assert_eq!(c.night_light, NightLightConfig::default());
}
