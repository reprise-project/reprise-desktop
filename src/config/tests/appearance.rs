use super::*;

#[test]
fn color_scheme_picks_classic_colours() {
    let want = reprisekit::schemes::scheme("Brick").expect("Brick is a Classic scheme");
    let c = Config::parse("[appearance]\ncolor_scheme = \"Brick\"\n").unwrap();
    assert_eq!(c.theme.colors, want);
}

#[test]
fn colors_table_applies_over_the_scheme() {
    let c = Config::parse(
        "[appearance]\ncolor_scheme = \"Brick\"\n[colors]\nbackground = \"#102030\"\n",
    )
    .unwrap();
    assert_eq!(c.theme.colors.background, Color::rgb(0x10, 0x20, 0x30));
}

#[test]
fn color_scheme_picks_the_blue_variant() {
    let c = Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Olive Green\"\n")
        .unwrap();
    assert_eq!(c.blue_variant, reprisekit::blue::Variant::Olive);
    let c =
        Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Obsidian\"\n").unwrap();
    assert_eq!(c.blue_variant, reprisekit::blue::Variant::Obsidian);
}

#[test]
fn a_scheme_of_another_interface_falls_back() {
    let blue =
        Config::parse("[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Brick\"\n").unwrap();
    assert_eq!(blue.blue_variant, reprisekit::blue::Variant::Blue);
    let classic = Config::parse("[appearance]\ncolor_scheme = \"Olive Green\"\n").unwrap();
    assert_eq!(classic.theme.colors, Config::default().theme.colors);
}

#[test]
fn interface_defaults_to_classic_and_parses() {
    assert_eq!(Config::parse("").unwrap().interface, Interface::Classic);
    let c = Config::parse("[appearance]\ninterface = \"classic\"\n").unwrap();
    assert_eq!(c.interface, Interface::Classic);
}

#[test]
fn blue_interface_rebases_metrics_and_colors() {
    let c = Config::parse("[appearance]\ninterface = \"blue\"\n").unwrap();
    assert_eq!(c.interface, Interface::Blue);
    assert_eq!(c.theme.metrics.caption_height, 26);
    assert_eq!(
        c.theme.colors.background,
        reprisekit::blue::Variant::Blue.scheme().system.background
    );
    assert!(matches!(c.theme.look, reprisekit::look::Look::Styled(_)));
}

#[test]
fn user_metrics_apply_over_the_blue_base() {
    let c = Config::parse(
        "[appearance]\ninterface = \"blue\"\n[metrics]\nframe_width = 6\n[colors]\nbackground = \"#102030\"\n",
    )
    .unwrap();
    assert_eq!(c.theme.metrics.frame_width, 6);
    assert_eq!(c.theme.metrics.caption_height, 26);
    assert_eq!(c.theme.colors.background, Color::rgb(0x10, 0x20, 0x30));
}

#[test]
fn classic_config_theme_is_unchanged() {
    assert_eq!(Config::parse("").unwrap().theme, Theme::default());
}

#[test]
fn unknown_interface_falls_back_to_classic() {
    let c = Config::parse("[appearance]\ninterface = \"frosted\"\n").unwrap();
    assert_eq!(c.interface, Interface::Classic);
}

#[test]
fn glass_material_and_frame_parse_their_names() {
    use reprisekit::glass::{Frame, Material};
    let c = Config::parse("").unwrap();
    assert_eq!(
        (c.glass.material, c.glass.frame),
        (Material::Frosted, Frame::Rounded)
    );
    let c = Config::parse("[glass]\nmaterial = \"smoked\"\nframe = \"square\"\n").unwrap();
    assert_eq!(
        (c.glass.material, c.glass.frame),
        (Material::Smoked, Frame::Square)
    );
    assert_eq!(
        Config::parse("[glass]\nmaterial = \"molten\"\n")
            .unwrap()
            .glass
            .material,
        Material::Molten
    );
    assert!(Config::parse("[glass]\nmaterial = \"glassy\"\n").is_err());
    assert!(Config::parse("[glass]\nframe = \"round\"\n").is_err());
}

#[test]
fn glass_notification_style_parses_and_rejects_unknown_names() {
    use reprisekit::glass::NotifyStyle;
    assert_eq!(
        Config::parse("").unwrap().glass.notification,
        NotifyStyle::Lozenge
    );
    assert_eq!(
        Config::parse("[glass]\nnotification_style = \"lozenge\"\n")
            .unwrap()
            .glass
            .notification,
        NotifyStyle::Lozenge
    );
    assert_eq!(
        Config::parse("[glass]\nnotification_style = \"halo\"\n")
            .unwrap()
            .glass
            .notification,
        NotifyStyle::Halo
    );
    assert_eq!(
        Config::parse("[glass]\nnotification_style = \"droplet\"\n")
            .unwrap()
            .glass
            .notification,
        NotifyStyle::Droplet
    );
    let err = Config::parse("[glass]\nnotification_style = \"bubble\"\n").unwrap_err();
    assert!(
        err.to_string()
            .contains("[glass] notification_style = \"bubble\" is not lozenge, halo or droplet"),
        "{err}"
    );
}

#[test]
fn glass_mode_parses_light_and_dark() {
    use reprisekit::glass::Mode;
    assert_eq!(Config::parse("").unwrap().glass.mode, Mode::Light);
    assert_eq!(
        Config::parse("[glass]\nmode = \"dark\"\n")
            .unwrap()
            .glass
            .mode,
        Mode::Dark
    );
    assert_eq!(
        Config::parse("[glass]\nmode = \"light\"\n")
            .unwrap()
            .glass
            .mode,
        Mode::Light
    );
    assert!(
        Config::parse("[glass]\nmode = \"dim\"\n").is_err(),
        "a mode other than light or dark is an error"
    );
}

#[test]
fn glass_interface_and_settings_parse() {
    let c = Config::parse(
        "[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"#102030\"\nintensity = 80\ntransparency = false\n",
    )
    .unwrap();
    assert_eq!(c.interface, Interface::Glass);
    assert_eq!(c.glass.intensity, 80);
    assert!(!c.glass.transparency);
    assert_eq!(c.theme.metrics.frame_width, 8);
    let d = Config::parse("[glass]\ncolor = \"sky\"\n").unwrap();
    assert_eq!(d.glass, reprisekit::glass::Settings::default());
    assert!(
        d.glass.fast_drag,
        "the faster glass while dragging is on by default"
    );
    let e = Config::parse("[glass]\nfast_drag = false\n").unwrap();
    assert!(!e.glass.fast_drag);
    assert!(d.glass.blur_frames, "frames behind glass show by default");
    let f = Config::parse("[glass]\nblur_frames = false\n").unwrap();
    assert!(!f.glass.blur_frames);
    assert_eq!(
        d.glass.blur_rate, 30,
        "client content re-blurs 30 times a second"
    );
    let r = Config::parse("[glass]\nblur_rate = 0\n").unwrap();
    assert_eq!(r.glass.blur_rate, 0);
    assert!(
        Config::parse("[glass]\nblur_rate = -1\n").is_err(),
        "the rate is not negative"
    );
    assert!(
        Config::parse("[glass]\nintensity = 101\n").is_err(),
        "intensity is 0..100"
    );
    assert_eq!(d.glass.orb_glow, 100, "the orb glows fully by default");
    for o in [0, 50, 100] {
        let c = Config::parse(&format!("[glass]\norb_glow = {o}\n")).unwrap();
        assert_eq!(c.glass.orb_glow, o);
    }
    for bad in [-1, 101] {
        assert!(
            Config::parse(&format!("[glass]\norb_glow = {bad}\n")).is_err(),
            "orb_glow is 0..100, not {bad}"
        );
    }
    assert_eq!((d.glass.blur, d.glass.streaks), (2, true));
    let b = Config::parse("[glass]\nblur = 5\nstreaks = false\n").unwrap();
    assert_eq!((b.glass.blur, b.glass.streaks), (5, false));
    for bad in [0, 6] {
        assert!(
            Config::parse(&format!("[glass]\nblur = {bad}\n")).is_err(),
            "blur is 1..5, not {bad}"
        );
    }
    assert!(
        Config::parse("[glass]\ncolor = \"mauve-ish\"\n").is_err(),
        "unknown colour name"
    );
}

#[test]
fn side_width_takes_its_key_and_refuses_a_negative() {
    for i in ["classic", "blue", "glass"] {
        let head = format!("[appearance]\ninterface = \"{i}\"\n");
        assert_eq!(
            Config::parse(&head).unwrap().taskbar.side_width,
            0,
            "{i}: the look's default"
        );
    }
    let c = Config::parse("[taskbar]\nside_width = 120\n").unwrap();
    assert_eq!(c.taskbar.side_width, 120);
    let c = Config::parse("[taskbar]\nside_width = 5000\n").unwrap();
    assert_eq!(
        c.taskbar.side_width, 5000,
        "clamped where it is used, not here"
    );
    for w in ["-1", "2147483648"] {
        let err = Config::parse(&format!("[taskbar]\nside_width = {w}\n")).unwrap_err();
        assert_eq!(
            err,
            format!("[taskbar] side_width must be between 0 and 2147483647, got {w}")
        );
    }
    assert!(Config::parse("[taskbar]\nside_width = \"wide\"\n").is_err());
}

#[test]
fn tray_overflow_is_on_by_default_in_every_look() {
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        assert!(TaskbarConfig::default_for(i).tray_overflow, "{i:?}");
    }
}

#[test]
fn taskbar_defaults_follow_the_interface() {
    let classic = Config::parse("").unwrap().taskbar;
    assert_eq!(classic, TaskbarConfig::default_for(Interface::Classic));
    assert_eq!(classic.buttons, TaskbarButtons::WhenFull);
    assert!(
        !classic.pinned && !classic.small_icons && !classic.show_desktop_button && !classic.peek,
        "Classic turns every taskbar extra off but the tray arrow"
    );
    assert!(classic.tray_overflow);
    let blue = Config::parse("[appearance]\ninterface = \"blue\"\n").unwrap();
    assert_eq!(blue.taskbar, classic, "Blue keeps Classic's taskbar");
    let glass = Config::parse("[appearance]\ninterface = \"glass\"\n")
        .unwrap()
        .taskbar;
    assert_eq!(
        glass,
        TaskbarConfig {
            position: reprisekit::taskbar::Edge::Bottom,
            buttons: TaskbarButtons::Combine,
            pinned: true,
            small_icons: false,
            show_desktop_button: true,
            peek: true,
            tray_overflow: true,
            auto_hide: false,
            reveal_on_edge: true,
            reveal_on_keys: true,
            reveal_on_attention: true,
            side_width: 0,
        }
    );
    assert_eq!(glass, TaskbarConfig::default_for(Interface::Glass));
}

#[test]
fn taskbar_keys_override_the_interface_defaults() {
    let c = Config::parse(
        "[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\npeek = false\nsmall_icons = true\n",
    )
    .unwrap();
    assert_eq!(c.taskbar.buttons, TaskbarButtons::Never);
    assert!(!c.taskbar.peek);
    assert!(c.taskbar.small_icons);
    assert!(c.taskbar.pinned, "an absent key keeps Glass's default");
    let c = Config::parse("[taskbar]\nbuttons = \"combine\"\ntray_overflow = true\n").unwrap();
    assert_eq!(c.taskbar.buttons, TaskbarButtons::Combine);
    assert!(c.taskbar.tray_overflow);
    assert!(!c.taskbar.pinned && !c.taskbar.peek);
    let w = Config::parse("[taskbar]\nbuttons = \"when_full\"\n").unwrap();
    assert_eq!(w.taskbar.buttons, TaskbarButtons::WhenFull);
    let g = Config::parse("[taskbar]\nbuttons = \"group\"\n").unwrap();
    assert_eq!(g.taskbar.buttons, TaskbarButtons::Group);
}

#[test]
fn the_taskbar_sits_on_any_edge_position_names() {
    use reprisekit::taskbar::Edge;
    for i in ["classic", "blue", "glass"] {
        let head = format!("[appearance]\ninterface = \"{i}\"\n");
        let c = Config::parse(&head).unwrap();
        assert_eq!(
            c.taskbar.position,
            Edge::Bottom,
            "{i} puts the bar at the bottom"
        );
        let c = Config::parse(&format!("{head}[taskbar]\nposition = \"top\"\n")).unwrap();
        assert_eq!(c.taskbar.position, Edge::Top, "{i} takes the key");
    }
    let c = Config::parse("[taskbar]\nposition = \"bottom\"\n").unwrap();
    assert_eq!(c.taskbar.position, Edge::Bottom);
    let err = Config::parse("[taskbar]\nposition = \"middle\"\n").unwrap_err();
    assert!(
        err.contains("\"middle\""),
        "the error names the value: {err}"
    );
    for edge in ["bottom", "top", "left", "right"] {
        assert!(err.contains(edge), "the error names {edge}: {err}");
    }
    assert!(Config::parse("[taskbar]\nposition = 1\n").is_err());
    for (name, edge) in [("left", Edge::Left), ("right", Edge::Right)] {
        let c = Config::parse(&format!("[taskbar]\nposition = \"{name}\"\n")).unwrap();
        assert_eq!(c.taskbar.position, edge, "{name}");
    }
}

#[test]
fn auto_hide_is_off_and_its_triggers_on_under_every_interface() {
    for i in ["classic", "blue", "glass"] {
        let c = Config::parse(&format!("[appearance]\ninterface = \"{i}\"\n")).unwrap();
        let t = &c.taskbar;
        assert!(!t.auto_hide, "{i} keeps the bar up");
        assert!(
            t.reveal_on_edge && t.reveal_on_keys && t.reveal_on_attention,
            "{i} turns every trigger on"
        );
        assert_eq!(c.effects.taskbar_hide_ms, 200, "{i} slides it over 200 ms");
    }
}

#[test]
fn auto_hide_and_its_triggers_take_their_keys() {
    let c = Config::parse(
        "[taskbar]\nauto_hide = true\nreveal_on_edge = false\nreveal_on_keys = false\n\
         reveal_on_attention = false\n[effects]\ntaskbar_hide_ms = 0\n",
    )
    .unwrap();
    assert!(c.taskbar.auto_hide);
    assert!(!c.taskbar.reveal_on_edge && !c.taskbar.reveal_on_keys);
    assert!(!c.taskbar.reveal_on_attention);
    assert_eq!(c.effects.taskbar_hide_ms, 0, "0 hides and shows at once");
    assert!(Config::parse("[taskbar]\nauto_hide = \"yes\"\n").is_err());
    let err = Config::parse("[effects]\ntaskbar_hide_ms = 2001\n").unwrap_err();
    assert!(
        err.contains("taskbar_hide_ms"),
        "the error names the key: {err}"
    );
    assert!(Config::parse("[effects]\ntaskbar_hide_ms = -1\n").is_err());
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

#[test]
fn small_icons_under_glass_take_the_small_bars_style() {
    let glass = |b: bool| {
        Config::parse(&format!(
            "[appearance]\ninterface = \"glass\"\n[taskbar]\nsmall_icons = {b}\n"
        ))
        .unwrap()
    };
    let (full, small) = (glass(false), glass(true));
    let settings = &full.glass;
    let style = |c: &Config| match &c.theme.look {
        reprisekit::look::Look::Styled(s) => s.clone(),
        reprisekit::look::Look::Classic => panic!("Glass is a styled look"),
    };
    // A shared, capped cache may rebuild either style in between, so compare
    // pixels rather than pointers.
    let pixels = |s: &std::sync::Arc<reprisekit::style::Style>| {
        s.part(reprisekit::style::Part::Caption)
            .unwrap()
            .atlas
            .pixels
            .clone()
    };
    assert_eq!(
        pixels(&style(&full)),
        pixels(&reprisekit::glass::style(settings))
    );
    assert_eq!(
        pixels(&style(&small)),
        pixels(&reprisekit::glass::small_bar_style(settings)),
        "small icons take the small bar's style"
    );
}

#[test]
fn taskbar_rejects_unknown_buttons_and_keys() {
    assert!(Config::parse("[taskbar]\nbuttons = \"sometimes\"\n").is_err());
    assert!(Config::parse("[taskbar]\nbuttons = true\n").is_err());
    assert!(
        Config::parse("[taskbar]\nautohide = true\n").is_err(),
        "the key is auto_hide"
    );
}

#[test]
fn appearance_rejects_unknown_keys() {
    assert!(Config::parse("[appearance]\nfoo = 1\n").is_err());
}

#[test]
fn icons_section_defaults_and_parses() {
    assert_eq!(Config::default().icons.theme, "hicolor");
    let c = Config::parse("[icons]\ntheme = \"Adwaita\"\n").unwrap();
    assert_eq!(c.icons.theme, "Adwaita");
    assert!(Config::parse("[icons]\nnope = 1\n").is_err());
}

#[test]
fn toolkits_default_to_auto_and_resolve_against_the_look() {
    assert_eq!(Config::default().toolkits, ToolkitsConfig::default());
    let auto = &Config::default().toolkits;
    assert_eq!(resolve_gtk_theme(&auto.gtk_theme, false, None), "Adwaita");
    assert_eq!(
        resolve_gtk_theme(&auto.gtk_theme, true, None),
        "Adwaita-dark"
    );
    assert_eq!(resolve_qt(&auto.qt_platform_theme, false, None), None);
    assert_eq!(
        resolve_qt(&auto.qt_platform_theme, true, None),
        Some("xdgdesktopportal")
    );
    let c =
        Config::parse("[toolkits]\ngtk_theme = \"Arc\"\nqt_platform_theme = \"qt6ct\"\n").unwrap();
    assert_eq!(resolve_gtk_theme(&c.toolkits.gtk_theme, true, None), "Arc");
    assert_eq!(
        resolve_qt(&c.toolkits.qt_platform_theme, false, None),
        Some("qt6ct")
    );
    assert!(Config::parse("[toolkits]\nnope = 1\n").is_err());
}

#[test]
fn reprise_resolves_to_what_was_generated_else_as_auto() {
    let c = Config::parse("[toolkits]\ngtk_theme = \"reprise\"\nqt_platform_theme = \"reprise\"\n")
        .unwrap();
    let (gtk, qt) = (&c.toolkits.gtk_theme, &c.toolkits.qt_platform_theme);
    assert_eq!(
        resolve_gtk_theme(gtk, true, Some("Reprise-alt")),
        "Reprise-alt"
    );
    assert_eq!(resolve_gtk_theme(gtk, true, None), "Adwaita-dark");
    assert_eq!(resolve_gtk_theme(gtk, false, None), "Adwaita");
    assert_eq!(resolve_qt(qt, false, Some("qt6ct")), Some("qt6ct"));
    assert_eq!(resolve_qt(qt, true, None), Some("xdgdesktopportal"));
    assert_eq!(resolve_qt(qt, false, None), None);
    assert_eq!(
        resolve_gtk_theme("Arc", false, Some("Reprise")),
        "Arc",
        "a named theme ignores the generated one"
    );
}

#[test]
fn font_smoothing_defaults_to_grayscale_and_an_unknown_value_keeps_it() {
    assert_eq!(
        Config::default().toolkits.font_smoothing,
        FontSmoothing::Grayscale
    );
    for (s, f) in [
        ("grayscale", FontSmoothing::Grayscale),
        ("subpixel", FontSmoothing::Subpixel),
        ("none", FontSmoothing::None),
        ("crisp", FontSmoothing::Grayscale),
    ] {
        let c = Config::parse(&format!("[toolkits]\nfont_smoothing = \"{s}\"\n")).unwrap();
        assert_eq!(c.toolkits.font_smoothing, f, "{s}");
    }
}

#[test]
fn cursor_section_parses_and_bounds_the_size() {
    let c = Config::parse("[cursor]\ntheme = \"Adwaita\"\nsize = 32\n").unwrap();
    assert_eq!(
        c.cursor,
        CursorConfig {
            theme: "Adwaita".into(),
            size: 32
        }
    );
    assert!(Config::parse("[cursor]\nsize = 257\n").is_err());
    assert!(Config::parse("[cursor]\nnope = 1\n").is_err());
}

#[test]
fn an_unset_cursor_falls_back_to_the_session_then_to_default_24() {
    let none = |_: &str| None;
    assert_eq!(
        CursorConfig::default().resolve(none),
        ("default".to_string(), 24)
    );
    let session = |k: &str| match k {
        "XCURSOR_THEME" => Some("breeze_cursors".to_string()),
        "XCURSOR_SIZE" => Some("48".to_string()),
        _ => None,
    };
    assert_eq!(
        CursorConfig::default().resolve(session),
        ("breeze_cursors".to_string(), 48)
    );
    let set = CursorConfig {
        theme: "Adwaita".into(),
        size: 32,
    };
    assert_eq!(set.resolve(session), ("Adwaita".to_string(), 32));
    let junk = |k: &str| (k == "XCURSOR_SIZE").then(|| "huge".to_string());
    assert_eq!(CursorConfig::default().resolve(junk).1, 24);
}

#[test]
fn the_start_menu_style_follows_the_interface_unless_set() {
    let glass = Config::parse("[appearance]\ninterface = \"glass\"\n").unwrap();
    assert_eq!(glass.startmenu_style, StartMenuStyle::Glass);
    let classic = Config::parse("").unwrap();
    assert_eq!(classic.startmenu_style, StartMenuStyle::Classic);
    let set = Config::parse("startmenu_style = \"classic\"\n[appearance]\ninterface = \"glass\"\n")
        .unwrap();
    assert_eq!(set.startmenu_style, StartMenuStyle::Classic);
}

#[test]
fn an_unknown_start_menu_style_is_an_error() {
    let err = Config::parse("startmenu_style = \"round\"\n").unwrap_err();
    assert!(err.contains("startmenu_style"), "{err}");
}

#[test]
fn start_icon_defaults_to_the_reprise_mark_and_is_configurable() {
    assert!(!Config::default().start_icon);
    assert!(Config::parse("start_icon = true\n").unwrap().start_icon);
}

#[test]
fn the_second_dialog_font_is_optional() {
    let font = |text: &str| Config::parse(text).unwrap().theme.fonts.shell_dlg;
    assert_eq!(font(""), None);
    assert_eq!(font("[fonts]\nsize_px = 11\n"), None);
    assert_eq!(font("[fonts]\nshell_dlg = \"\"\n"), None);
    assert_eq!(
        font("[fonts]\nshell_dlg = \"Some Sans\"\n").as_deref(),
        Some("Some Sans")
    );
    assert_eq!(
        font("[fonts]\nshell_dlg = \"/opt/fonts/dlg.ttf\"\n").as_deref(),
        Some("/opt/fonts/dlg.ttf")
    );
}

#[test]
fn font_size_is_range_checked_at_parse() {
    let err = |t: &str| Config::parse(t).unwrap_err();
    assert_eq!(
        err("[fonts]\nsize_px = 60000\n"),
        "fonts.size_px must be between 6 and 72, got 60000"
    );
    assert_eq!(
        err("[fonts]\nsize_px = 5\n"),
        "fonts.size_px must be between 6 and 72, got 5"
    );
    for px in [6, 72] {
        let cfg = Config::parse(&format!("[fonts]\nsize_px = {px}\n")).unwrap();
        assert_eq!(cfg.theme.fonts.size_px, px);
    }
}

#[test]
fn metrics_are_range_checked_at_parse() {
    let err = |t: &str| Config::parse(t).unwrap_err();
    assert_eq!(
        err("[metrics]\ndouble_click_ms = -1\n"),
        "metrics.double_click_ms must be between 0 and 10000, got -1"
    );
    assert_eq!(
        err("[metrics]\nframe_width = 10001\n"),
        "metrics.frame_width must be between 0 and 10000, got 10001"
    );
    assert_eq!(
        err("[metrics]\ntaskbar_height = 21\n"),
        "metrics.taskbar_height must be >= 22, got 21"
    );
    assert_eq!(
        err("[metrics]\nbogus = 1\n"),
        "unknown metric name \"bogus\""
    );
    assert_eq!(
        Config::parse("[metrics]\nmenu_shadow = 0\n")
            .unwrap()
            .theme
            .metrics
            .menu_shadow,
        0
    );
}

#[test]
fn wallpaper_reads_image_and_position() {
    let c = Config::parse("[wallpaper]\nimage = \"/w/a.jpg\"\nposition = \"tile\"\n").unwrap();
    assert_eq!(c.wallpaper.image.as_deref(), Some(Path::new("/w/a.jpg")));
    assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Tile);
}

#[test]
fn wallpaper_defaults_to_none_stretched() {
    let c = Config::default();
    assert_eq!(c.wallpaper.image, None);
    assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Stretch);
}

#[test]
fn wallpaper_reads_a_builtin_and_fills_the_screen_with_it() {
    let c = Config::parse("[wallpaper]\nimage = \"reprise:terraces/frost\"\nposition = \"tile\"\n")
        .unwrap();
    assert_eq!(
        c.wallpaper.image.as_deref(),
        Some(Path::new("reprise:terraces/frost"))
    );
    assert_eq!(c.wallpaper.position, crate::wallpaper::Position::Stretch);
    let c = Config::parse("[wallpaper]\nimage = \"reprise:reeded/hue-30-dark\"\n").unwrap();
    assert_eq!(
        c.wallpaper.image.as_deref(),
        Some(Path::new("reprise:reeded/hue-30-dark"))
    );
}

#[test]
fn wallpaper_refuses_a_builtin_it_does_not_have() {
    for (bad, says) in [
        ("reprise:lake", "no built-in wallpaper \"lake\""),
        ("reprise:tarn/noon", "tarn has no variant \"noon\""),
        ("reprise:tarn/hue-20", "tarn has no variant"),
        ("reprise:flow/hue-400", "flow has no variant"),
    ] {
        let e = Config::parse(&format!("[wallpaper]\nimage = \"{bad}\"\n")).unwrap_err();
        assert!(
            e.contains("wallpaper.image") && e.contains(says),
            "{bad}: {e}"
        );
    }
}

#[test]
fn wallpaper_defaults_to_each_interfaces_own() {
    let image = |toml: &str| Config::parse(toml).unwrap().wallpaper.image;
    assert_eq!(image(""), None, "Classic keeps its desktop colour");
    assert_eq!(
        image("[appearance]\ninterface = \"blue\"\n"),
        Some(PathBuf::from("reprise:tarn/blue"))
    );
    let glass = image("[appearance]\ninterface = \"glass\"\n").unwrap();
    assert!(
        glass.to_str().unwrap().starts_with("reprise:flow/hue-"),
        "{glass:?}"
    );
    assert_ne!(
        image("[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"ruby\"\n"),
        Some(glass),
        "another glass colour, another tint"
    );
    assert_eq!(
        image("[appearance]\ninterface = \"glass\"\n[glass]\nmode = \"dark\"\n"),
        Some(PathBuf::from("reprise:flow/night"))
    );
    assert_eq!(
        image("[wallpaper]\nimage = \"reprise:flow/glass\"\n[glass]\nmode = \"dark\"\n"),
        Some(PathBuf::from("reprise:flow/night"))
    );
    assert_eq!(
        image("[appearance]\ninterface = \"blue\"\n[wallpaper]\nimage = \"/w/a.jpg\"\n"),
        Some(PathBuf::from("/w/a.jpg"))
    );
    assert_eq!(
        image("[appearance]\ninterface = \"glass\"\n[wallpaper]\nimage = \"\"\n"),
        None
    );
}

#[test]
fn wallpaper_image_that_is_missing_still_parses() {
    let c = Config::parse("[wallpaper]\nimage = \"/nowhere/none.png\"\n").unwrap();
    assert_eq!(
        c.wallpaper.image.as_deref(),
        Some(Path::new("/nowhere/none.png"))
    );
}

#[test]
fn wallpaper_image_expands_home_and_empty_is_none() {
    let home = Some(std::ffi::OsStr::new("/home/u"));
    assert_eq!(
        wallpaper_image("~/Pictures/a.png", home),
        Ok(Some(PathBuf::from("/home/u/Pictures/a.png")))
    );
    assert!(wallpaper_image("~/a.png", None).is_err());
    assert_eq!(wallpaper_image("", home), Ok(None));
    assert_eq!(
        wallpaper_image("/w/a.png", home),
        Ok(Some(PathBuf::from("/w/a.png")))
    );
    let c = Config::parse("[wallpaper]\nimage = \"\"\n").unwrap();
    assert_eq!(c.wallpaper.image, None);
}

#[test]
fn wallpaper_water_parses_and_defaults_off() {
    use reprisekit::water::Water;
    let w = |toml: &str| Config::parse(toml).map(|c| c.wallpaper.water);
    assert_eq!(w(""), Ok(Water::Off));
    assert_eq!(
        w("[appearance]\ninterface = \"glass\"\n"),
        Ok(Water::Off),
        "off under Glass too"
    );
    assert_eq!(w("[wallpaper]\nwater = \"calm\"\n"), Ok(Water::Calm));
    assert_eq!(w("[wallpaper]\nwater = \"Lively\"\n"), Ok(Water::Lively));
    assert_eq!(w("[wallpaper]\nwater = \"rain\"\n"), Ok(Water::Rain));
    assert_eq!(w("[wallpaper]\nwater = \"glow\"\n"), Ok(Water::Glow));
    assert!(w("[wallpaper]\nwater = \"storm\"\n").is_err());
}

#[test]
fn wallpaper_water_rate_parses_and_defaults_to_refresh() {
    use reprisekit::water::Rate;
    let r = |toml: &str| Config::parse(toml).map(|c| c.wallpaper.water_rate);
    assert_eq!(r(""), Ok(Rate::Refresh));
    assert_eq!(
        r("[appearance]\ninterface = \"glass\"\n"),
        Ok(Rate::Refresh),
        "refresh under Glass too"
    );
    assert_eq!(
        r("[wallpaper]\nwater_rate = \"Refresh\"\n"),
        Ok(Rate::Refresh)
    );
    assert_eq!(r("[wallpaper]\nwater_rate = 144\n"), Ok(Rate::Fixed(144)));
    assert_eq!(r("[wallpaper]\nwater_rate = 30\n"), Ok(Rate::Fixed(30)));
    for bad in ["29", "241", "\"fast\"", "60.5"] {
        let e = r(&format!("[wallpaper]\nwater_rate = {bad}\n")).unwrap_err();
        assert!(e.contains("wallpaper.water_rate"), "{bad}: {e}");
    }
}

#[test]
fn wallpaper_lock_screen_defaults_by_interface() {
    let on = |toml: &str| Config::parse(toml).unwrap().wallpaper.lock_screen;
    assert!(!on(""), "off under Classic");
    assert!(on("[appearance]\ninterface = \"blue\"\n"), "on under Blue");
    assert!(
        on("[appearance]\ninterface = \"glass\"\n"),
        "on under Glass"
    );
    assert!(on("[wallpaper]\nlock_screen = true\n"));
    assert!(!on(
        "[appearance]\ninterface = \"glass\"\n[wallpaper]\nlock_screen = false\n"
    ));
}

#[test]
fn wallpaper_rejects_an_unknown_position() {
    assert!(Config::parse("[wallpaper]\nposition = \"fill\"\n").is_err());
}

#[test]
fn wallpaper_slideshow_keys() {
    let w = Config::default().wallpaper;
    assert_eq!((w.slideshow, w.interval, w.shuffle), (None, 30, false));
    let c = Config::parse("[wallpaper]\nslideshow = \"/srv/pics\"\ninterval = 1\nshuffle = true\n")
        .unwrap();
    assert_eq!(c.wallpaper.slideshow, Some(PathBuf::from("/srv/pics")));
    assert_eq!((c.wallpaper.interval, c.wallpaper.shuffle), (1, true));
    assert_eq!(
        Config::parse("[wallpaper]\nslideshow = \"\"\n")
            .unwrap()
            .wallpaper
            .slideshow,
        None
    );
    assert!(Config::parse("[wallpaper]\ninterval = 0\n").is_err());
}

#[test]
fn screensaver_keys() {
    use reprisekit::saver::Kind;
    let d = Config::default().screensaver;
    assert_eq!(d.kind, Kind::Blank);
    assert_eq!(d.options.speed, 5);
    let c = Config::parse(
        "[screensaver]\nname = \"mystify\"\nspeed = 9\nshapes = 3\ncycle_colors = false\nfolder = \"/srv/pics\"\nphoto_seconds = 20\nstars = 50\n",
    )
    .unwrap()
    .screensaver;
    assert_eq!(c.kind, Kind::Mystify);
    assert_eq!(
        (c.options.speed, c.options.shapes, c.options.stars),
        (9, 3, 50)
    );
    assert_eq!(
        (c.options.cycle_colors, c.options.photo_seconds),
        (false, 20)
    );
    assert_eq!(c.folder, Some(PathBuf::from("/srv/pics")));
    assert!(Config::parse("[screensaver]\nname = \"toasters\"\n").is_err());
    assert!(Config::parse("[screensaver]\nspeed = 11\n").is_err());
}
