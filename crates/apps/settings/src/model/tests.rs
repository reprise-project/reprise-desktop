use super::shortcuts::default_shortcuts;
use super::*;

#[test]
fn the_layout_list_adds_moves_and_removes_through_layout_and_variant() {
    let mut s = Settings::default();
    assert!(s.layouts().is_empty(), "the system default");
    assert!(s.add_layout("us", ""));
    assert!(s.add_layout("hu", "qwerty"));
    assert_eq!(
        (s.kb_layout.as_str(), s.kb_variant.as_str()),
        ("us,hu", ",qwerty")
    );
    assert!(!s.move_layout_up(0));
    assert!(s.move_layout_up(1));
    assert_eq!(
        (s.kb_layout.as_str(), s.kb_variant.as_str()),
        ("hu,us", "qwerty,")
    );
    s.remove_layout(0);
    assert_eq!((s.kb_layout.as_str(), s.kb_variant.as_str()), ("us", ""));
    assert!(s.add_layout("de", ""));
    assert!(s.add_layout("fr", ""));
    assert!(s.add_layout("it", ""));
    assert!(!s.add_layout("es", ""), "four at most");
    assert_eq!(s.kb_layout, "us,de,fr,it");
    for _ in 0..4 {
        s.remove_layout(0);
    }
    assert_eq!((s.kb_layout.as_str(), s.kb_variant.as_str()), ("", ""));
}

#[test]
fn banner_default_matches_the_compositor() {
    assert_eq!(Settings::default().startmenu_banner, "Reprise Desktop");
}

/// A commented file with a default key and a non-default one: what `save` must leave alone.
const SAMPLE: &str = "\
# Reprise configuration.

terminal = \"foot\"           # what Super+Return runs
desktop_icons = true          # the default, spelled out

[fonts]
family = [\"Tahoma\", \"DejaVu Sans\"]
size_px = 12

[idle]                        # seconds, 0 disables
lock_after = 0
";

/// Comments above the keys they document, as `config.example.toml` writes them.
const COMMENTED: &str = "\
# Reprise configuration. Every key is optional.

terminal = \"foot\"
# task_manager = \"gnome-system-monitor\"   # default: top in the terminal
desktop_icons = false         # the desktop shows no icons

[metrics]
frame_width = 5
# how tall the taskbar is
taskbar_height = 32

[fonts]
size_px = 12
";

fn saved(text: &str, want: &Settings) -> String {
    save(text, want, &load(text)).expect("the sample parses")
}

#[test]
fn effects_defaults_follow_the_files_interface() {
    let classic = load("");
    assert!(!classic.window_animation && !classic.hover_fade);
    assert_eq!(
        (classic.window_open_ms, classic.window_close_ms),
        (250, 200)
    );
    assert_eq!((classic.hover_in_ms, classic.hover_out_ms), (120, 250));
    assert!(!classic.taskbar_slide);
    let glass = load("[appearance]\ninterface = \"glass\"\n");
    assert!(glass.window_animation && glass.hover_fade && glass.taskbar_slide);
}

#[test]
fn the_session_fade_is_on_under_every_interface_and_written_only_off() {
    for text in ["", "[appearance]\ninterface = \"glass\"\n"] {
        let mut want = load(text);
        assert!(want.session_fade, "{text:?}");
        want.session_fade = false;
        let out = saved(text, &want);
        assert!(out.contains("session_fade = false"), "{out}");
        assert_eq!(load(&out), want);
        let mut back = want.clone();
        back.session_fade = true;
        assert!(!saved(&out, &back).contains("session_fade"));
    }
}

#[test]
fn effects_are_written_against_the_interfaces_defaults() {
    let text = "[appearance]\ninterface = \"glass\"\n";
    let mut want = load(text);
    want.hover_fade = false;
    want.window_open_ms = 400;
    let out = saved(text, &want);
    assert!(out.contains("[effects]"), "{out}");
    assert!(out.contains("hover_fade = false"), "{out}");
    assert!(out.contains("window_open_ms = 400"), "{out}");
    assert!(
        !out.contains("window_animation"),
        "Glass's default stays unwritten: {out}"
    );
    assert_eq!(load(&out), want);
    let base = want.clone();
    want.hover_fade = true;
    want.window_open_ms = 250;
    let out = save(&out, &want, &base).expect("parses");
    assert!(
        !out.contains("hover_fade") && !out.contains("window_open_ms"),
        "{out}"
    );
}

#[test]
fn a_table_whose_last_key_goes_back_to_its_default_goes_too() {
    let text = "[appearance]\ninterface = \"glass\"\n";
    let mut want = load(text);
    want.hover_fade = false;
    let out = saved(text, &want);
    assert!(out.contains("[effects]"), "{out}");
    let base = want.clone();
    want.hover_fade = true;
    let out = save(&out, &want, &base).expect("parses");
    assert_eq!(out, text, "no bare [effects] is left");
}

#[test]
fn save_carries_a_removed_keys_comments_on_to_the_next_key() {
    let mut s = load(COMMENTED);
    s.desktop_icons = true;
    let out = saved(COMMENTED, &s);

    assert_eq!(
        out,
        COMMENTED.replace(
            "desktop_icons = false         # the desktop shows no icons\n",
            ""
        ),
        "byte-identical but for the removed key's own line"
    );
    assert!(out.starts_with("# Reprise configuration. Every key is optional.\n"));
    assert!(out.contains("# task_manager = \"gnome-system-monitor\""));
}

#[test]
fn save_carries_the_comments_of_a_tables_last_key_past_the_table() {
    let mut s = load(COMMENTED);
    s.taskbar_height = 28;
    let out = saved(COMMENTED, &s);

    assert_eq!(
        out,
        COMMENTED.replace("taskbar_height = 32\n", ""),
        "the comment lands above the next table, not in the bin"
    );
}

#[test]
fn save_carries_the_comments_of_the_last_key_in_the_file() {
    let text =
        "[fonts]\nsize_px = 12\n# the font size above is the only key here\nantialias = true\n";
    let mut s = load(text);
    s.font_antialias = false;
    let out = save(text, &s, &load(text)).expect("parses");

    assert_eq!(out, text.replace("antialias = true\n", ""));
}

#[test]
fn rebase_takes_the_files_values_and_keeps_the_ones_the_sheet_moved() {
    let base = load("");
    let mut want = base.clone();
    want.sticky_keys = true;
    let edited = "[accessibility]\nmouse_keys = true\nsticky_keys = false\n";
    let fresh = load(edited);
    rebase(&mut want, &base, &fresh);
    assert!(want.mouse_keys, "an untouched value follows the file");
    assert!(want.sticky_keys, "a moved value stays pending");
    let out = save(edited, &want, &fresh).expect("parses");
    assert_eq!(
        out, "[accessibility]\nmouse_keys = true\nsticky_keys = true\n",
        "Apply writes only the moved value"
    );
}

#[test]
fn save_leaves_a_key_an_editor_changed_meanwhile() {
    let opened = "terminal = \"foot\"\n[fonts]\nsize_px = 12\n";
    let base = load(opened);
    let mut want = base.clone();
    want.font_size_px = 14;
    let edited = "terminal = \"kitty\"\n[fonts]\nsize_px = 12\n";

    let out = save(edited, &want, &base).expect("parses");
    assert!(out.contains("terminal = \"kitty\""), "{out}");
    assert!(out.contains("size_px = 14"), "{out}");
}

#[test]
fn save_keeps_glass_blur_rate_when_it_writes_the_glass_table() {
    let text = "[appearance]\ninterface = \"glass\"\n\n[glass]\nblur_rate = 0\nintensity = 30\n";
    let mut want = load(text);
    want.glass_intensity = 75;
    want.glass_fast_drag = false;
    want.glass_blur_frames = false;
    want.glass_orb_glow = 50;

    let out = saved(text, &want);
    assert!(out.contains("blur_rate = 0\n"), "{out}");
    assert!(out.contains("orb_glow = 50"), "{out}");
    assert_eq!(load(&out).glass_orb_glow, 50, "the glow reads back");
    assert!(out.contains("intensity = 75"), "{out}");
    assert!(out.contains("fast_drag = false"), "{out}");
    assert!(out.contains("blur_frames = false"), "{out}");
    assert!(!load(&out).glass_blur_frames, "it reads back off");
}

#[test]
fn save_refuses_a_file_that_does_not_parse() {
    let broken = "terminal = \"foot\"\n[metrics\nframe_width = 4\n";
    let mut s = load(broken);
    s.terminal = "kitty".into();

    let err =
        save(broken, &s, &Settings::default()).expect_err("a broken file is not ours to rewrite");
    assert!(!err.is_empty());
}

#[test]
fn wallpaper_water_round_trips() {
    for v in ["calm", "lively", "rain", "off"] {
        let mut s = load("");
        s.wallpaper_water = v.into();
        let out = saved("", &s);
        if v != "off" {
            assert!(out.contains(&format!("water = \"{v}\"")), "{out}");
        }
        assert_eq!(load(&out).wallpaper_water, v);
    }
    assert_eq!(load("").wallpaper_water, "off");
}

#[test]
fn load_reads_the_keys_it_knows() {
    let s = load(SAMPLE);
    assert_eq!(s.terminal, "foot");
    assert_eq!(s.font_family, "Tahoma, DejaVu Sans");
    assert_eq!(s.font_size_px, 12);
    assert_eq!(s.lock_after, 0);
}

#[test]
fn load_falls_back_to_the_compositors_defaults() {
    let s = load("");
    assert_eq!(s, Settings::default());
    assert_eq!(s.terminal, "reprise-terminal");
    assert_eq!(s.taskbar_height, 28);
    assert_eq!(s.screen_off_after, 900);
    assert_eq!(s.task_manager, "", "an absent key is the empty string");
}

#[test]
fn load_ignores_a_file_that_does_not_parse() {
    assert_eq!(load("terminal = "), Settings::default());
}

#[test]
fn save_leaves_an_unchanged_file_byte_identical() {
    let s = load(SAMPLE);
    assert_eq!(saved(SAMPLE, &s), SAMPLE);
}

#[test]
fn save_rewrites_only_the_key_that_changed() {
    let mut s = load(SAMPLE);
    s.terminal = "kitty".into();
    let out = saved(SAMPLE, &s);
    assert_eq!(out, SAMPLE.replace("\"foot\"", "\"kitty\""));
    assert!(
        out.contains("# what Super+Return runs"),
        "the trailing comment survives: {out}"
    );
}

#[test]
fn save_removes_a_key_that_is_back_to_its_default() {
    let mut s = load(SAMPLE);
    s.font_size_px = 11;
    let out = saved(SAMPLE, &s);
    assert!(!out.contains("size_px"), "{out}");
    assert!(out.contains("family = [\"Tahoma\", \"DejaVu Sans\"]"));
}

#[test]
fn save_creates_a_table_only_when_a_key_needs_one() {
    let mut s = load(SAMPLE);
    let untouched = saved(SAMPLE, &s);
    assert!(!untouched.contains("[power]"), "nothing needed it");

    s.poweroff = "loginctl poweroff".into();
    let out = saved(SAMPLE, &s);
    assert!(out.contains("[power]"));
    assert!(out.contains("poweroff = \"loginctl poweroff\""));
    assert!(!out.contains("standby"), "the defaults stay out: {out}");
}

#[test]
fn save_writes_the_font_family_back_as_an_array() {
    let mut s = load(SAMPLE);
    s.font_family = "Verdana, Tahoma".into();
    let out = saved(SAMPLE, &s);
    assert!(out.contains("family = [\"Verdana\", \"Tahoma\"]"), "{out}");
    assert_eq!(load(&out).font_family, "Verdana, Tahoma");
}

#[test]
fn save_starts_a_file_from_nothing() {
    let s = Settings {
        taskbar_height: 32,
        ..Settings::default()
    };
    let out = saved("", &s);
    assert_eq!(out, "[metrics]\ntaskbar_height = 32\n");
    assert_eq!(load(&out).taskbar_height, 32);
}

#[test]
fn glass_mode_saves_dark_and_drops_light() {
    let base = Settings::default();
    let want = Settings {
        glass_mode: "dark".into(),
        ..base.clone()
    };
    let text = save("", &want, &base).unwrap();
    assert!(text.contains("mode = \"dark\""), "dark is written: {text}");
    assert_eq!(load(&text).glass_mode, "dark");
    let back = save(&text, &base, &want).unwrap();
    assert!(
        !back.contains("mode ="),
        "light, the default, drops the key: {back}"
    );
}

#[test]
fn glass_material_and_frame_save_their_names_and_drop_the_defaults() {
    let base = Settings::default();
    let want = Settings {
        glass_material: "smoked".into(),
        glass_frame: "square".into(),
        ..base.clone()
    };
    let text = save("", &want, &base).unwrap();
    assert_eq!(text, "[glass]\nmaterial = \"smoked\"\nframe = \"square\"\n");
    let back = load(&text);
    assert_eq!(
        (back.glass_material.as_str(), back.glass_frame.as_str()),
        ("smoked", "square")
    );
    assert_eq!(
        save(&text, &base, &want).unwrap(),
        "",
        "frosted and rounded, the defaults, drop both keys"
    );
}

#[test]
fn glass_notification_style_drops_its_default() {
    let base = Settings::default();
    let want = Settings {
        glass_notification_style: "halo".into(),
        ..base.clone()
    };
    let text = save("", &want, &base).unwrap();
    assert_eq!(text, "[glass]\nnotification_style = \"halo\"\n");
    assert_eq!(
        save(&text, &base, &want).unwrap(),
        "",
        "lozenge, the default, drops the key"
    );
}

#[test]
fn appearance_defaults_match_the_compositor() {
    let d = Settings::default();
    assert_eq!(
        (d.interface.as_str(), d.color_scheme.as_str()),
        ("classic", "")
    );
    let g = reprisekit::glass::Settings::default();
    assert_eq!(reprisekit::glass::swatch(&d.glass_color), Some(g.color));
    assert_eq!(d.glass_intensity, i64::from(g.intensity));
    assert_eq!(d.glass_orb_glow, i64::from(g.orb_glow));
    assert_eq!(
        (d.glass_transparency, d.glass_fast_drag, d.glass_blur_frames),
        (g.transparency, g.fast_drag, g.blur_frames)
    );
    assert_eq!(
        (d.glass_blur, d.glass_streaks),
        (i64::from(g.blur), g.streaks)
    );
    assert_eq!(reprisekit::glass::Mode::parse(&d.glass_mode), Some(g.mode));
    assert_eq!(
        reprisekit::glass::Material::parse(&d.glass_material),
        Some(g.material)
    );
    assert_eq!(
        reprisekit::glass::NotifyStyle::parse(&d.glass_notification_style),
        Some(g.notification)
    );
    assert_eq!(
        reprisekit::glass::Frame::parse(&d.glass_frame),
        Some(g.frame)
    );
    assert_eq!(d.glass_custom, "");
    assert_eq!((d.wallpaper.as_str(), d.background.as_str()), ("", ""));
    assert_eq!(
        reprisekit::wallpaper::Position::parse(&d.wallpaper_position),
        Some(reprisekit::wallpaper::Position::default())
    );
    assert_eq!(
        d.system_icons,
        ["documents", "computer", "home", "recycle-bin"]
    );
}

#[test]
fn a_glass_colour_no_swatch_names_is_the_custom_one() {
    assert_eq!(
        load("[glass]\ncolor = \"#102030\"\n").glass_custom,
        "#102030"
    );
    assert_eq!(load("[glass]\ncolor = \"ruby\"\n").glass_custom, "");
    // Sky's own colour spelled out is Sky, not a custom colour.
    assert_eq!(load("[glass]\ncolor = \"#70AFFC\"\n").glass_custom, "");
    assert_eq!(glass_swatch("#70AFFC"), Some("sky"));
    assert_eq!(glass_swatch("leaf"), Some("leaf"));
    assert_eq!(glass_swatch("#102030"), None);
}

#[test]
fn system_icons_keep_their_order_and_the_bin() {
    let have: Vec<String> = vec!["computer".into(), "recycle-bin".into()];
    assert_eq!(
        system_icons_with(&have, "documents", true),
        ["computer", "recycle-bin", "documents"]
    );
    assert_eq!(system_icons_with(&have, "computer", false), ["recycle-bin"]);
    assert_eq!(system_icons_with(&have, "computer", true), have);
    // A list without the bin gets it back at the end; a name the
    // desktop does not know stays where it was.
    let odd: Vec<String> = vec!["home".into(), "nowhere".into()];
    assert_eq!(
        system_icons_with(&odd, "computer", true),
        ["home", "nowhere", "computer", "recycle-bin"]
    );
}

#[test]
fn a_hand_ordered_icon_list_keeps_its_order() {
    let hand: Vec<String> = ["computer", "recycle-bin", "documents", "custom-x"]
        .map(String::from)
        .to_vec();
    let off = system_icons_with(&hand, "computer", false);
    assert_eq!(off, ["recycle-bin", "documents", "custom-x"]);
    assert_eq!(
        system_icons_with(&off, "computer", true),
        ["recycle-bin", "documents", "custom-x", "computer"]
    );
    let off = system_icons_with(&hand, "documents", false);
    assert_eq!(
        system_icons_with(&off, "documents", true),
        ["computer", "recycle-bin", "custom-x", "documents"]
    );
}

#[test]
fn older_icon_names_load_as_the_current_ones() {
    let s = load("system_icons = [\"my-computer\", \"my-network-places\", \"recycle-bin\"]\n");
    assert_eq!(s.system_icons, ["computer", "recycle-bin"]);
}

#[test]
fn the_start_menu_style_defaults_follow_the_interface() {
    assert_eq!(Settings::for_interface("glass").startmenu_style, "glass");
    assert_eq!(
        Settings::for_interface("classic").startmenu_style,
        "classic"
    );
    assert_eq!(Settings::for_interface("blue").startmenu_style, "classic");
}

#[test]
fn the_lock_screen_wallpaper_defaults_on_under_blue_and_glass() {
    assert!(!Settings::for_interface("").lock_wallpaper);
    assert!(!Settings::for_interface("classic").lock_wallpaper);
    assert!(Settings::for_interface("blue").lock_wallpaper);
    assert!(Settings::for_interface("glass").lock_wallpaper);
}

#[test]
fn a_new_interface_resets_the_scheme_and_the_unset_defaults() {
    let text = "[appearance]\ncolor_scheme = \"Brick\"\n[taskbar]\npinned = false\n";
    let loaded = load(text);
    let mut s = loaded.clone();
    s.background = "#102030".into();
    s.interface = "glass".into();
    switch_interface(&mut s, "classic", text, &loaded);
    assert_eq!(s.color_scheme, "", "the scheme goes back to Glass's own");
    assert_eq!(s.background, "#102030", "the desktop colour is its own key");
    assert_eq!(s.taskbar_buttons, "combine", "unset: Glass's default");
    assert!(s.taskbar_peek && s.window_animation && s.hover_fade && s.taskbar_slide);
    assert!(!s.taskbar_pinned, "the file sets it");
    s.interface = "classic".into();
    switch_interface(&mut s, "glass", text, &loaded);
    assert_eq!(s.taskbar_buttons, "when_full");
    assert!(!s.taskbar_peek && !s.window_animation && !s.hover_fade && !s.taskbar_slide);
}

#[test]
fn picking_the_interface_back_keeps_the_colour_scheme() {
    let text = "[appearance]\ninterface = \"blue\"\ncolor_scheme = \"Olive Green\"\n\
                [glass]\ncolor = \"ruby\"\n";
    let loaded = load(text);
    let mut want = loaded.clone();
    want.interface = "classic".into();
    switch_interface(&mut want, "blue", text, &loaded);
    assert_eq!(want.color_scheme, "", "Classic's own scheme");
    want.interface = "blue".into();
    switch_interface(&mut want, "classic", text, &loaded);
    assert_eq!(want.color_scheme, "Olive Green");
    assert_eq!(want.glass_color, "ruby");
    assert_eq!(want, loaded, "OK has nothing to write");
    assert_eq!(save(text, &want, &loaded).expect("parses"), text);
}

#[test]
fn a_switched_interface_is_written_against_its_own_defaults() {
    let base = load("");
    let mut want = base.clone();
    want.interface = "glass".into();
    switch_interface(&mut want, "classic", "", &base);
    let out = save("", &want, &base).expect("parses");
    assert_eq!(
        out, "[appearance]\ninterface = \"glass\"\n",
        "no Glass default is pinned"
    );
    assert_eq!(load(&out), want);
    let base = want.clone();
    want.interface = "classic".into();
    switch_interface(&mut want, "glass", &out, &base);
    let out = save(&out, &want, &base).expect("parses");
    assert_eq!(
        out, "",
        "Classic is the default, so the key and its table go"
    );
}

#[test]
fn switcher_defaults_follow_the_interface() {
    let g = Settings::for_interface("glass");
    assert_eq!(g.switcher_style, "thumbnails");
    assert!(g.switcher_desktop && g.switcher_peek && g.switcher_preview && g.switcher_flip);
    let c = Settings::for_interface("classic");
    assert_eq!(c.switcher_style, "icons");
    assert!(!c.switcher_desktop && !c.switcher_peek && c.switcher_preview && !c.switcher_flip);
}

#[test]
fn window_shake_follows_the_interface_and_auto_reads_as_its_default() {
    assert!(Settings::for_interface("glass").window_shake);
    assert!(!Settings::for_interface("classic").window_shake);
    assert!(!Settings::for_interface("blue").window_shake);
    let glass = "[appearance]\ninterface = \"glass\"\n";
    assert!(load(&format!("{glass}[windows]\nshake = \"auto\"\n")).window_shake);
    assert!(!load(&format!("{glass}[windows]\nshake = false\n")).window_shake);
    let base = load(glass);
    let off = Settings {
        window_shake: false,
        ..base.clone()
    };
    let text = save(glass, &off, &base).unwrap();
    assert!(text.contains("shake = false"), "{text}");
    assert!(!save(&text, &base, &off).unwrap().contains("shake"));
}

#[test]
fn save_round_trips_every_field() {
    let s = Settings {
        terminal: "kitty".into(),
        script_terminal: "close".into(),
        script_open: "terminal".into(),
        task_manager: "htop".into(),
        startmenu_banner: "Reprise".into(),
        startmenu_style: "classic".into(),
        desktop_icons: false,
        desktop_extensions: "hide_launchers".into(),
        start_icon: false,
        adaptive_sync: true,
        switcher_preview: false,
        switcher_desktop: true,
        switcher_peek: true,
        switcher_flip: true,
        preview_taskbar: false,
        previews_fps: 30,
        switcher_style: "thumbnails".into(),
        switcher_size: "medium".into(),
        hot_corner: "top-right".into(),
        screenshot_save: false,
        clipboard_history: false,
        interface: "blue".into(),
        color_scheme: "Olive Green".into(),
        glass_color: "#102030".into(),
        glass_mode: "dark".into(),
        glass_material: "smoked".into(),
        glass_frame: "square".into(),
        glass_notification_style: "lozenge".into(),
        glass_custom: "#102030".into(),
        glass_intensity: 70,
        glass_transparency: false,
        glass_blur: 4,
        glass_streaks: false,
        glass_fast_drag: false,
        glass_blur_frames: false,
        glass_orb_glow: 35,
        wallpaper: "/usr/share/backgrounds/a.png".into(),
        wallpaper_position: "tile".into(),
        wallpaper_water: "rain".into(),
        drive_insert: "open".into(),
        slideshow: "~/Pictures/Slides".into(),
        slideshow_interval: 5,
        slideshow_shuffle: true,
        saver: "photos".into(),
        saver_speed: 8,
        saver_stars: 400,
        saver_shapes: 3,
        saver_cycle: false,
        saver_folder: "/srv/photos".into(),
        saver_photo_seconds: 20,
        background: "#336699".into(),
        system_icons: vec!["computer".into(), "recycle-bin".into()],
        taskbar_buttons: "never".into(),
        taskbar_pinned: true,
        taskbar_small_icons: true,
        taskbar_show_desktop: true,
        taskbar_peek: true,
        taskbar_tray_overflow: true,
        taskbar_position: "top".into(),
        taskbar_auto_hide: true,
        taskbar_reveal_on_edge: false,
        taskbar_reveal_on_keys: false,
        taskbar_reveal_on_attention: false,
        recent_files: false,
        location: false,
        capture_border: false,
        debug_overlay: true,
        debug_overlay_in_captures: true,
        launcher_files: false,
        thumbnail_cache: false,
        thumbnail_max_mb: 64,
        desktops: 1,
        desktop_animation: "slide-vertical".into(),
        desktop_animation_ms: 350,
        pager_mark: "tile".into(),
        remember_windows: false,
        window_tabs: false,
        window_shake: true,
        session_restore: true,
        session_end_processes: false,
        font_family: "Verdana".into(),
        font_size_px: 13,
        font_antialias: true,
        font_shell_dlg: "/opt/fonts/dlg.ttf".into(),
        icon_theme: "hicolor".into(),
        gtk_theme: "Arc-Dark".into(),
        qt_platform_theme: "qt6ct".into(),
        font_smoothing: "subpixel".into(),
        gtk4_colors: true,
        cursor_theme: "Adwaita".into(),
        cursor_size: 48,
        frame_width: 5,
        caption_height: 20,
        taskbar_height: 32,
        menu_shadow: 0,
        menu_hover_ms: 300,
        menu_fade_ms: 0,
        menu_animation: "slide".into(),
        window_animation: true,
        window_open_ms: 300,
        window_close_ms: 180,
        hover_fade: true,
        hover_in_ms: 90,
        hover_out_ms: 400,
        taskbar_slide: true,
        session_fade: false,
        double_click_ms: 400,
        kb_layout: "hu".into(),
        kb_variant: "nodeadkeys".into(),
        kb_model: "pc105".into(),
        kb_options: "ctrl:nocaps".into(),
        repeat_rate: 30,
        repeat_delay: 500,
        standby: "loginctl suspend".into(),
        poweroff: "loginctl poweroff".into(),
        reboot: "loginctl reboot".into(),
        hibernate: "loginctl hibernate".into(),
        lock_on_sleep: false,
        lock_wallpaper: true,
        lock_after: 300,
        screen_off_after: 0,
        blank_after: 120,
        lock_fade: 4,
        mouse_speed: -5,
        mouse_accel: false,
        mouse_natural: true,
        touchpad_speed: 10,
        touchpad_accel: false,
        touchpad_natural: true,
        tap_to_click: false,
        two_finger_scroll: false,
        two_finger_tap_right_click: false,
        disable_while_typing: false,
        snap_enabled: false,
        snap_picker: false,
        snap_bar: false,
        snap_layouts: "1|1 2-1:2".into(),
        snap_distance: 15,
        nl_enabled: true,
        nl_temperature: 3400,
        nl_schedule: "fixed".into(),
        nl_from: "20:30".into(),
        nl_to: "06:00".into(),
        sticky_keys: true,
        sticky_lock: false,
        sticky_two_keys_off: false,
        slow_keys: true,
        slow_keys_delay: 3,
        key_shortcuts: false,
        color_filter: "protanopia".into(),
        color_filter_intensity: 60,
        magnifier_follow: "pointer".into(),
        mouse_keys: true,
        mouse_keys_num_lock: false,
        mouse_keys_speed: 900,
        mouse_keys_acceleration: 25,
        text_size: 150,
        shortcuts: (default_shortcuts().into_iter())
            .map(|b| match b.keys.as_str() {
                "Super+t" => Shortcut::new("Super+t", "spawn", "foot"),
                _ => b,
            })
            .collect(),
        // The portal adds these; the sheet only edits what is there.
        programs: Vec::new(),
        denied: Vec::new(),
        startmenu: vec![crate::startmenu::Entry::item(
            "&Web",
            "web-browser",
            crate::startmenu::Target::Command("firefox".into()),
        )],
        tablet_pads: vec![crate::tabletpage::PadEntry {
            name: "Wacom*".into(),
            buttons: [(1, "zoom-in".to_string())].into(),
            ring_up: "ctrl+plus".into(),
            ..Default::default()
        }],
    };
    let out = saved(SAMPLE, &s);
    assert_eq!(load(&out), s);
}

#[test]
fn snap_defaults_match_the_compositor() {
    let d = Settings::default();
    assert!(d.snap_enabled, "drag snapping is on");
    assert!(d.snap_picker);
    assert!(d.snap_bar);
    assert_eq!(d.snap_layouts, "", "the built-in six");
    assert_eq!(d.snap_distance, 30);
}

#[test]
fn snap_layouts_write_as_an_array_and_read_back() {
    let want = Settings {
        snap_picker: false,
        snap_layouts: "1|1,2|1  1:2|1:2".into(),
        ..Settings::default()
    };
    let out = saved("", &want);
    assert!(
        out.contains(r#"layouts = ["1|1", "2|1", "1:2|1:2"]"#),
        "{out}"
    );
    assert!(out.contains("picker = false"), "{out}");
    let got = load(&out);
    assert!(!got.snap_picker);
    assert_eq!(got.snap_layouts, "1|1 2|1 1:2|1:2");
    let cleared = Settings {
        snap_layouts: String::new(),
        ..got.clone()
    };
    let out = save(&out, &cleared, &got).unwrap();
    assert!(!out.contains("layouts"), "empty is the default: {out}");
}

#[test]
fn validate_refuses_a_snap_layout_the_compositor_would() {
    let base = Settings::default();
    let with = |l: &str| Settings {
        snap_layouts: l.into(),
        ..Settings::default()
    };
    assert_eq!(validate(&with("1|1 2-1:2"), &base), Ok(()));
    assert_eq!(
        validate(&with("1|1 1|0"), &base),
        Err(r#"Snap layouts: "1|0": "0" is not a weight > 0."#.into())
    );
    assert_eq!(
        validate(&with("1|1-1"), &base),
        Err(r#"Snap layouts: "1|1-1" mixes | and -."#.into())
    );
    assert_eq!(
        validate(&with("1 1 1 1 1 1 1"), &base),
        Err("Snap layouts: at most 6 layouts, got 7.".into())
    );
}

#[test]
fn shortcuts_default_to_the_compositors_list() {
    let s = load("");
    assert_eq!(s.shortcuts.len(), 40);
    assert_eq!(
        s.shortcuts[0],
        Shortcut::new("Super+Return", "terminal", "")
    );
    assert_eq!(
        s.shortcuts[10],
        Shortcut::new("Super+Shift+b", "toggleframe", "")
    );
    assert_eq!(
        s.shortcuts[11],
        Shortcut::new("Super+space", "launcher", "")
    );
    assert_eq!(s.shortcuts[12], Shortcut::new("Super+c", "center", ""));
    assert_eq!(
        s.shortcuts[16],
        Shortcut::new("Super+Down", "snap-down", "")
    );
    assert_eq!(
        s.shortcuts[20],
        Shortcut::new("Super+Ctrl+Shift+Right", "desktop-carry-next", "")
    );
    assert_eq!(
        s.shortcuts[22],
        Shortcut::new("Alt+Sys_Req", "screenshot-window", "")
    );
    assert_eq!(validate(&s, &Settings::default()), Ok(()));
}

#[test]
fn a_keybinds_table_in_the_file_layers_over_the_defaults() {
    let text = "[[keybinds]]\nkeys = \"Super+t\"\naction = \"spawn\"\ncommand = \"foot\"\n\n\
                [[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"\n\n\
                [[keybinds]]\nkeys = \"Super+F9\"\naction = \"lock\"\n";
    let s = load(text);
    let mut want = default_shortcuts();
    let t = want.iter().position(|b| b.keys == "Super+t").unwrap();
    want[t] = Shortcut::new("Super+t", "spawn", "foot");
    let q = want.iter().position(|b| b.keys == "Super+q").unwrap();
    want[q] = Shortcut::new("Super+q", "none", "");
    want.push(Shortcut::new("Super+F9", "lock", ""));
    assert_eq!(s.shortcuts, want);
}

#[test]
fn a_none_row_stays_only_on_a_free_default_chord() {
    let text = "[[keybinds]]\nkeys = \"Super+F9\"\naction = \"none\"\n\n\
                [[keybinds]]\nkeys = \"Super+w\"\naction = \"none\"\n\n\
                [[keybinds]]\nkeys = \"Super+w\"\naction = \"lock\"\n";
    let s = load(text);
    assert!(
        !s.shortcuts.iter().any(|b| b.action == "none"),
        "{:?}",
        s.shortcuts
    );
    assert!(s.shortcuts.contains(&Shortcut::new("Super+w", "lock", "")));
}

#[test]
fn remove_on_a_default_sets_it_to_none_and_an_action_brings_it_back() {
    let mut s = load(SAMPLE);
    let q = s
        .shortcuts
        .iter()
        .position(|b| b.keys == "Super+q")
        .unwrap();
    s.shortcuts.push(Shortcut::new("Super+F9", "lock", ""));
    let mut removed = s.clone();
    forget(&mut removed, Row::Bind(q));
    let last = removed.shortcuts.len() - 1;
    forget(&mut removed, Row::Bind(last));
    assert_eq!(removed.shortcuts[q], Shortcut::new("Super+q", "none", ""));
    assert_eq!(removed.shortcuts.len(), s.shortcuts.len() - 1);
    assert_eq!(validate(&removed, &s), Ok(()));
    let out = save(SAMPLE, &removed, &load(SAMPLE)).unwrap();
    assert_eq!(
        out[SAMPLE.len()..].trim(),
        "[[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"",
    );
    assert_eq!(load(&out), removed);

    let mut back = removed.clone();
    back.shortcuts[q] = Shortcut::new("Super+q", "close", "");
    let out = save(&out, &back, &removed).unwrap();
    assert!(!out.contains("keybinds"), "{out}");
    assert_eq!(load(&out).shortcuts, default_shortcuts());
}

#[test]
fn validate_takes_none_only_on_a_default_chord() {
    let mut s = Settings::default();
    s.shortcuts[0].action = "none".into();
    assert_eq!(validate(&s, &Settings::default()), Ok(()));
    s.shortcuts.push(Shortcut::new("Super+F9", "none", ""));
    assert!(validate(&s, &Settings::default()).is_err());
}

#[test]
fn shortcuts_save_what_differs_from_the_defaults_and_go_when_back_to_default() {
    let mut s = load(SAMPLE);
    s.shortcuts.push(Shortcut::new("Super+F9", "spawn", "foot"));
    let out = saved(SAMPLE, &s);
    assert!(
        out.starts_with(SAMPLE),
        "the rest of the file is untouched: {out}"
    );
    assert_eq!(
        out[SAMPLE.len()..].trim(),
        "[[keybinds]]\nkeys = \"Super+F9\"\naction = \"spawn\"\ncommand = \"foot\"",
        "no default is copied into the file"
    );
    assert_eq!(load(&out), s);

    let mut removed = s.clone();
    let q = removed
        .shortcuts
        .iter()
        .position(|b| b.keys == "Super+q")
        .unwrap();
    removed.shortcuts[q] = Shortcut::new("Super+q", "none", "");
    removed.shortcuts[2] = Shortcut::new("Alt+Tab", "flip", "");
    let out = save(&out, &removed, &s).unwrap();
    assert!(
        out.contains("[[keybinds]]\nkeys = \"Super+q\"\naction = \"none\"\n"),
        "a default set to none is written as none: {out}"
    );
    assert!(out.contains("[[keybinds]]\nkeys = \"Alt+Tab\"\naction = \"flip\"\n"));
    let mut reread = removed.clone();
    reread
        .shortcuts
        .insert(2, Shortcut::new("Alt+F4", "none", ""));
    assert_eq!(
        load(&out),
        reread,
        "a default moved off its chord reads back as none"
    );

    let out = save(&out, &load(SAMPLE), &removed).unwrap();
    assert!(!out.contains("keybinds"), "{out}");
}

#[test]
fn a_files_none_survives_a_rewrite_of_its_keybinds() {
    let text = "[[keybinds]]\nkeys = \"Super+l\"\naction = \"none\"\n";
    let base = load(text);
    let mut want = base.clone();
    want.shortcuts.push(Shortcut::new("Super+F9", "lock", ""));
    let out = save(text, &want, &base).unwrap();
    assert!(
        out.contains("[[keybinds]]\nkeys = \"Super+l\"\naction = \"none\"\n"),
        "{out}"
    );
    assert_eq!(load(&out), want);
}

#[test]
fn validate_refuses_a_shortcut_it_cannot_bind() {
    let mut s = Settings::default();
    s.shortcuts.push(Shortcut::new("Hyper+x", "close", ""));
    assert!(validate(&s, &Settings::default()).is_err());
    s.shortcuts.pop();
    s.shortcuts.push(Shortcut::new("Super+", "close", ""));
    assert!(validate(&s, &Settings::default()).is_err());
    s.shortcuts.pop();
    s.shortcuts.push(Shortcut::new("Super+t", "spawn", ""));
    assert!(validate(&s, &Settings::default()).is_err());
    s.shortcuts.pop();
    s.shortcuts.push(Shortcut::new("Super+t", "nosuch", ""));
    assert!(validate(&s, &Settings::default()).is_err());
}

#[test]
fn desktop_and_preview_defaults_match_the_compositor() {
    let d = Settings::default();
    assert_eq!(d.desktops, 4);
    assert_eq!(d.desktop_animation, "none");
    assert_eq!(d.desktop_animation_ms, 200);
    assert_eq!(d.previews_fps, 10);
    assert!(d.screenshot_save && d.clipboard_history && d.remember_windows);
    assert!(d.window_tabs);
    assert!(!d.session_restore);
    assert!(d.session_end_processes);
    assert_eq!(d.hibernate, "systemctl hibernate");
    assert!(d.lock_on_sleep);
}

#[test]
fn workspaces_and_previews_round_trip_and_drop_the_defaults() {
    let text = "[workspaces]\ncount = 6\nanimation = \"wipe\"\n[previews]\nfps = 24\n";
    let s = load(text);
    assert_eq!(s.desktops, 6);
    assert_eq!(s.desktop_animation, "wipe");
    assert_eq!(s.desktop_animation_ms, 200, "absent: the default");
    assert_eq!(s.previews_fps, 24);

    let mut want = s.clone();
    want.desktops = 1;
    want.desktop_animation_ms = 500;
    want.previews_fps = 10;
    let out = saved(text, &want);
    assert_eq!(
        out, "[workspaces]\ncount = 1\nanimation = \"wipe\"\nanimation_ms = 500\n",
        "the emptied [previews] goes"
    );
    assert_eq!(load(&out), want);

    let base = want.clone();
    want.desktops = 4;
    want.desktop_animation = "none".into();
    want.desktop_animation_ms = 200;
    let out = save(&out, &want, &base).expect("parses");
    assert_eq!(out, "", "the emptied [workspaces] goes too");
}

#[test]
fn the_desktop_extensions_round_trip_and_the_default_leaves_the_file() {
    assert_eq!(load("").desktop_extensions, "hide");
    let base = load("");
    let mut show = base.clone();
    show.desktop_extensions = "show".into();
    let text = saved("", &show);
    assert_eq!(text, "desktop_extensions = \"show\"\n");
    assert_eq!(load(&text).desktop_extensions, "show");
    let mut launchers = load(&text);
    launchers.desktop_extensions = "hide_launchers".into();
    assert_eq!(
        saved(&text, &launchers),
        "desktop_extensions = \"hide_launchers\"\n"
    );
    let mut hide = load(&text);
    hide.desktop_extensions = "hide".into();
    assert_eq!(saved(&text, &hide), "", "the default removes the key");
}

#[test]
fn the_script_open_round_trips_and_the_default_leaves_the_file() {
    assert_eq!(load("").script_open, "run");
    let foot = "terminal = \"foot\"\n";
    let mut terminal = load(foot);
    terminal.script_open = "terminal".into();
    let text = saved(foot, &terminal);
    assert_eq!(text, "terminal = \"foot\"\nscript_open = \"terminal\"\n");
    assert_eq!(load(&text).script_open, "terminal");
    let mut run = load(&text);
    run.script_open = "run".into();
    assert_eq!(
        saved(&text, &run),
        "terminal = \"foot\"\n",
        "the default removes the key"
    );
}

#[test]
fn the_script_end_round_trips_and_the_default_leaves_the_file() {
    assert_eq!(load("").script_terminal, "stay");
    let foot = "terminal = \"foot\"\n";
    let mut close = load(foot);
    close.script_terminal = "close".into();
    let text = saved(foot, &close);
    assert_eq!(text, "terminal = \"foot\"\nscript_terminal = \"close\"\n");
    assert_eq!(load(&text).script_terminal, "close");
    let mut stay = load(&text);
    stay.script_terminal = "stay".into();
    assert_eq!(
        saved(&text, &stay),
        "terminal = \"foot\"\n",
        "the default removes the key"
    );
}

#[test]
fn the_drive_insert_mode_round_trips_and_the_default_leaves_the_file() {
    assert_eq!(load("").drive_insert, "ask");
    let text = "[drives]\non_insert = \"nothing\"\n";
    let s = load(text);
    assert_eq!(s.drive_insert, "nothing");
    assert_eq!(saved(text, &s), text);
    let mut open = s.clone();
    open.drive_insert = "open".into();
    assert_eq!(saved(text, &open), "[drives]\non_insert = \"open\"\n");
    let mut ask = s;
    ask.drive_insert = "ask".into();
    assert_eq!(
        saved(text, &ask),
        "",
        "the default removes the key and its table"
    );
}

#[test]
fn the_pager_mark_round_trips() {
    assert_eq!(load("").pager_mark, "light");
    let text = "[workspaces]\npager = \"tile\"\n";
    let s = load(text);
    assert_eq!(s.pager_mark, "tile");
    assert_eq!(saved(text, &s), text);
    let mut light = s.clone();
    light.pager_mark = "light".into();
    assert_eq!(saved(text, &light), "", "the default leaves the file");
    let odd = "[workspaces]\npager = \"box\"\n";
    let s = load(odd);
    assert_eq!(s.pager_mark, "box");
    assert_eq!(saved(odd, &s), odd, "an unknown value survives");
}

#[test]
fn taskbar_defaults_follow_the_files_interface() {
    for text in ["", "[appearance]\ninterface = \"blue\"\n"] {
        let s = load(text);
        assert_eq!(s.taskbar_buttons, "when_full", "{text:?}");
        assert!(
            !s.taskbar_pinned
                && !s.taskbar_small_icons
                && !s.taskbar_show_desktop
                && !s.taskbar_peek,
            "{text:?} turns every extra off but the tray arrow"
        );
        assert!(s.taskbar_tray_overflow, "{text:?}");
    }
    let g = load("[appearance]\ninterface = \"glass\"\n");
    assert_eq!(g.taskbar_buttons, "combine");
    assert!(g.taskbar_pinned && g.taskbar_show_desktop && g.taskbar_peek);
    assert!(g.taskbar_tray_overflow && !g.taskbar_small_icons);
    let set =
        load("[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\npeek = false\n");
    assert_eq!(set.taskbar_buttons, "never");
    assert!(!set.taskbar_peek && set.taskbar_pinned);
}

#[test]
fn taskbar_keys_are_written_against_the_interfaces_defaults() {
    let glass = "[appearance]\ninterface = \"glass\"\n";
    let mut want = load(glass);
    want.taskbar_buttons = "when_full".into();
    want.taskbar_small_icons = true;
    let out = saved(glass, &want);
    assert_eq!(
        out,
        "[appearance]\ninterface = \"glass\"\n\n[taskbar]\nbuttons = \"when_full\"\nsmall_icons = true\n"
    );
    assert_eq!(load(&out), want);
    let base = want.clone();
    want.taskbar_buttons = "combine".into();
    let out = save(&out, &want, &base).expect("parses");
    assert!(!out.contains("buttons"), "back to Glass's default: {out}");

    // The same value under Classic is not its default, so it is written.
    let mut classic = load("");
    classic.taskbar_buttons = "combine".into();
    classic.taskbar_peek = true;
    let out = saved("", &classic);
    assert!(out.contains("buttons = \"combine\""), "{out}");
    assert!(out.contains("peek = true"), "{out}");
}

/// Keys are written against the defaults of the interface in the file at Apply, which another writer may have switched.
#[test]
fn taskbar_keys_are_written_against_the_interface_on_disk_at_apply() {
    let glass = "[appearance]\ninterface = \"glass\"\n[taskbar]\nbuttons = \"never\"\n";
    let base = load(glass);
    let mut want = base.clone();
    want.taskbar_buttons = "combine".into();
    // Switched to Classic meanwhile: combine is not Classic's default,
    // so it is written rather than dropped.
    let classic = "[appearance]\ninterface = \"classic\"\n[taskbar]\nbuttons = \"never\"\n";
    let out = save(classic, &want, &base).expect("parses");
    assert!(out.contains("buttons = \"combine\""), "{out}");
    assert_eq!(load(&out).taskbar_buttons, "combine");
    // Still Glass: combine is its default, so the key goes.
    let out = save(glass, &want, &base).expect("parses");
    assert!(!out.contains("buttons"), "{out}");
}

#[test]
fn the_taskbar_position_is_written_only_off_the_bottom() {
    for i in ["classic", "blue", "glass"] {
        for edge in ["top", "left", "right"] {
            let text = format!("[appearance]\ninterface = \"{i}\"\n");
            let s = load(&text);
            assert_eq!(s.taskbar_position, "bottom", "{i}");
            let mut moved = s.clone();
            moved.taskbar_position = edge.into();
            let out = saved(&text, &moved);
            assert_eq!(
                out,
                format!("{text}\n[taskbar]\nposition = \"{edge}\"\n"),
                "{i} {edge}"
            );
            assert_eq!(load(&out), moved);
            let back = save(&out, &s, &moved).expect("parses");
            assert!(!back.contains("position"), "back at the bottom: {back}");
            assert_eq!(load(&back), s);
        }
    }
}

#[test]
fn the_taskbar_positions_are_the_edge_names() {
    use reprisekit::taskbar::Edge;
    let values: Vec<&str> = TASKBAR_POSITIONS.iter().map(|c| c.0).collect();
    let names: Vec<&str> = Edge::ALL.iter().map(|e| e.name()).collect();
    assert_eq!(values, names);
    assert_eq!(Settings::default().taskbar_position, Edge::default().name());
}

#[test]
fn the_auto_hide_keys_are_written_only_off_their_defaults() {
    for i in ["classic", "blue", "glass"] {
        let text = format!("[appearance]\ninterface = \"{i}\"\n");
        let s = load(&text);
        assert!(!s.taskbar_auto_hide, "{i}");
        assert!(
            s.taskbar_reveal_on_edge && s.taskbar_reveal_on_keys && s.taskbar_reveal_on_attention,
            "{i}"
        );
        let mut want = s.clone();
        want.taskbar_auto_hide = true;
        want.taskbar_reveal_on_keys = false;
        let out = saved(&text, &want);
        assert_eq!(
            out,
            format!("{text}\n[taskbar]\nauto_hide = true\nreveal_on_keys = false\n"),
            "{i}"
        );
        assert_eq!(load(&out), want);
        let back = save(&out, &s, &want).expect("parses");
        assert!(
            !back.contains("auto_hide") && !back.contains("reveal_on"),
            "back at the defaults: {back}"
        );
    }
}

#[test]
fn session_and_power_flags_write_their_own_tables() {
    let mut s = load("");
    s.session_restore = true;
    s.lock_on_sleep = false;
    s.hibernate = "zzz".into();
    s.clipboard_history = false;
    let out = saved("", &s);
    assert!(out.contains("[session]\nrestore = true\n"), "{out}");
    assert!(out.contains("[clipboard]\nhistory = false\n"), "{out}");
    assert!(out.contains("hibernate = \"zzz\"\n"), "{out}");
    assert!(out.contains("lock_on_sleep = false\n"), "{out}");
    assert_eq!(load(&out), s);
}

#[test]
fn pointer_speed_is_tenths_of_libinputs_float() {
    let text = "[mouse]\nspeed = -0.5\n[touchpad]\nspeed = 1\n";
    let s = load(text);
    assert_eq!(s.mouse_speed, -5);
    assert_eq!(s.touchpad_speed, 10);
    let mut s = load("");
    s.mouse_speed = 3;
    let out = saved("", &s);
    assert_eq!(out, "[mouse]\nspeed = 0.3\n");
    s.mouse_speed = 0;
    assert!(
        !saved(&out, &s).contains("speed"),
        "the default leaves the file"
    );
}

const PROGRAMS: &str = "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"Pause\"\n\
    [[global_shortcuts]]\napp = \"t\"\nid = \"mute\"\nkeys = \"\"\n\
    [portal]\nshortcuts_denied = [\"n\"]\n";

fn p(app: &str, id: &str, desc: &str, keys: &str) -> ProgramShortcut {
    ProgramShortcut {
        app: app.into(),
        id: id.into(),
        description: desc.into(),
        keys: keys.into(),
    }
}

#[test]
fn program_shortcuts_and_denials_load() {
    let s = load(PROGRAMS);
    assert_eq!(
        s.programs,
        [
            p("t", "talk", "Hold to talk", "Pause"),
            p("t", "mute", "", "")
        ]
    );
    assert_eq!(s.denied, ["n"]);
}

#[test]
fn rows_run_binds_then_programs_then_denials() {
    let s = load(PROGRAMS);
    let n = s.shortcuts.len();
    assert_eq!(row(&s, 0), Some(Row::Bind(0)));
    assert_eq!(row(&s, n), Some(Row::Program(0)));
    assert_eq!(row(&s, n + 2), Some(Row::Denied(0)));
    assert_eq!(row(&s, n + 3), None);
}

#[test]
fn forgetting_a_program_drops_all_its_shortcuts_or_its_denial() {
    let mut s = load(PROGRAMS);
    forget(&mut s, Row::Program(1));
    assert!(s.programs.is_empty(), "both of t's go");
    forget(&mut s, Row::Denied(0));
    assert!(s.denied.is_empty());
    let out = saved(PROGRAMS, &s);
    assert!(!out.contains("global_shortcuts"), "{out}");
    assert!(!out.contains("shortcuts_denied"), "{out}");
    assert!(!out.contains("[portal]"), "no bare header is left: {out}");
}

#[test]
fn a_changed_program_key_saves_in_place() {
    let mut s = load(PROGRAMS);
    s.programs[0].keys = "F9".into();
    let out = saved(PROGRAMS, &s);
    assert_eq!(load(&out).programs[0].keys, "F9");
    assert_eq!(load(&out).programs[1], p("t", "mute", "", ""));
}

#[test]
fn a_program_bound_while_the_sheet_was_open_survives_a_save() {
    let base = load(PROGRAMS);
    let mut want = base.clone();
    want.programs[0].keys = "F9".into();
    let now =
        format!("{PROGRAMS}[[global_shortcuts]]\napp = \"v\"\nid = \"ptt\"\nkeys = \"F10\"\n");
    let out = save(&now, &want, &base).unwrap();
    let after = load(&out);
    assert_eq!(after.programs[0].keys, "F9");
    assert!(
        after
            .programs
            .iter()
            .any(|e| e.app == "v" && e.keys == "F10"),
        "{out}"
    );
}

#[test]
fn a_program_key_may_be_empty_but_not_wrong() {
    let base = load(PROGRAMS);
    let mut s = base.clone();
    assert!(validate(&s, &base).is_ok());
    s.programs[0].keys = "Ctrl+Nope".into();
    assert!(validate(&s, &base).is_err());
    s.programs[0].keys = "Ctrl+Shift_L".into();
    assert_eq!(
        validate(&s, &base),
        Err("Hold to talk: A program shortcut ends in a key that is not a modifier.".into())
    );
    s.programs[0].keys = "Super+Shift+Escape".into();
    assert_eq!(
        validate(&s, &base),
        Err("Hold to talk: Super+Shift+Escape is kept by the desktop.".into())
    );
}

#[test]
fn a_bad_program_key_from_the_file_does_not_block_apply() {
    let base = load(&PROGRAMS.replace("keys = \"Pause\"", "keys = \"Ctrl+Nope\""));
    let mut s = base.clone();
    s.desktops = 2;
    assert_eq!(validate(&s, &base), Ok(()), "the entry is as loaded");
    s.programs[0].keys = "Ctrl+Shift_L".into();
    assert!(validate(&s, &base).is_err(), "an edited entry is checked");
    s.programs[0].keys = "F9".into();
    assert_eq!(validate(&s, &base), Ok(()));
}
