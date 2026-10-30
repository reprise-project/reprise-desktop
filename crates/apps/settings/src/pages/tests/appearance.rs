use super::*;

#[test]
fn the_appearance_page_geometry() {
    let s = built();
    let p = Page::Appearance;
    assert_eq!(
        rect(&s, p, appearancepage::SAMPLE),
        Rect::new(24, 44, 350, 188)
    );
    assert_eq!(rect(&s, p, "interface_label"), Rect::new(20, 242, 196, 13));
    assert_eq!(rect(&s, p, "interface"), Rect::new(20, 258, 196, 21));
    assert_eq!(rect(&s, p, "scheme_label"), Rect::new(20, 288, 196, 13));
    assert_eq!(
        rect(&s, p, appearancepage::CLASSIC_SCHEME),
        Rect::new(20, 304, 196, 21)
    );
    inside_page(&s, p);
    let g = under("glass");
    assert_eq!(
        rect(&g, p, appearancepage::GLASS_COLOR),
        Rect::new(20, 304, 196, 21)
    );
    assert_eq!(rect(&g, p, "material_label"), Rect::new(20, 334, 90, 13));
    assert_eq!(rect(&g, p, "glass_material"), Rect::new(20, 350, 90, 21));
    assert_eq!(rect(&g, p, "frame_label"), Rect::new(126, 334, 90, 13));
    assert_eq!(rect(&g, p, "glass_frame"), Rect::new(126, 350, 90, 21));
    assert_eq!(
        rect(&g, p, "glass_transparency"),
        Rect::new(20, 384, 13, 13)
    );
    assert_eq!(rect(&g, p, "glass_fast_drag"), Rect::new(20, 406, 13, 13));
    assert_eq!(rect(&g, p, "glass_blur_frames"), Rect::new(20, 428, 13, 13));
    assert_eq!(rect(&g, p, "glass_streaks"), Rect::new(232, 384, 13, 13));
    assert_eq!(rect(&g, p, "intensity_label"), Rect::new(232, 288, 146, 13));
    assert_eq!(rect(&g, p, "glass_intensity"), Rect::new(232, 304, 146, 26));
    assert_eq!(rect(&g, p, "blur_label"), Rect::new(232, 334, 146, 13));
    assert_eq!(rect(&g, p, "glass_blur"), Rect::new(232, 350, 146, 26));
    assert_eq!(rect(&g, p, "orb_glow_label"), Rect::new(232, 406, 146, 13));
    assert_eq!(rect(&g, p, "glass_orb_glow"), Rect::new(232, 422, 146, 26));
    assert_eq!(rect(&g, p, "notify_label"), Rect::new(20, 452, 90, 13));
    assert_eq!(
        rect(&g, p, "glass_notification_style"),
        Rect::new(20, 468, 90, 21)
    );
    inside_page(&g, p);
}

#[test]
fn glass_shows_a_mode_combo_that_writes_glass_mode() {
    let g = under("glass");
    let p = Page::Appearance;
    assert_eq!(
        rect(&g, p, "glass_mode").x,
        232,
        "the Mode combo stands right of the interface combo"
    );
    assert!(
        under("classic").pages[p.index()]
            .item("glass_mode")
            .is_none(),
        "only Glass has it"
    );
}

#[test]
fn glass_material_and_frame_combos_write_their_keys() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    assert_eq!(
        rows(&s, p, "glass_material"),
        (vec!["Frosted".into(), "Smoked".into(), "Molten".into()], 0)
    );
    assert_eq!(
        rows(&s, p, "glass_frame"),
        (vec!["Rounded".into(), "Square".into()], 0)
    );
    pick(&mut s, &mut got, p, "glass_material", 1);
    pick(&mut s, &mut got, p, "glass_frame", 1);
    assert_eq!(
        (got.glass_material.as_str(), got.glass_frame.as_str()),
        ("smoked", "square")
    );
    let g = appearancepage::glass(&got);
    assert_eq!(
        (g.material, g.frame),
        (
            reprisekit::glass::Material::Smoked,
            reprisekit::glass::Frame::Square
        ),
        "the sample draws with them"
    );
}

#[test]
fn glass_notification_combo_writes_its_key() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        glass_notification_style: "unlisted".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    assert_eq!(
        rows(&s, p, "glass_notification_style").0[..3],
        ["Lozenge", "Halo", "Droplet"]
    );
    pick(&mut s, &mut got, p, "glass_notification_style", 0);
    assert_eq!(got.glass_notification_style, "lozenge");
    pick(&mut s, &mut got, p, "glass_notification_style", 1);
    assert_eq!(got.glass_notification_style, "halo");
    pick(&mut s, &mut got, p, "glass_notification_style", 2);
    assert_eq!(got.glass_notification_style, "droplet");
    assert!(
        under("classic").pages[p.index()]
            .item("glass_notification_style")
            .is_none()
    );
}

#[test]
fn the_glass_rows_show_under_glass_only() {
    let glass_rows = [
        "glass_mode",
        "glass_material",
        "glass_frame",
        "glass_transparency",
        "glass_fast_drag",
        "glass_blur_frames",
        "glass_intensity",
        "glass_blur",
        "glass_streaks",
        "glass_orb_glow",
        "glass_notification_style",
    ];
    let p = Page::Appearance;
    for i in ["classic", "blue"] {
        let s = under(i);
        for id in glass_rows {
            assert!(s.pages[p.index()].item(id).is_none(), "{id} under {i}");
        }
    }
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    assert!(
        glass_rows
            .iter()
            .all(|id| s.pages[p.index()].item(id).is_some())
    );
    // The interface moves: the page is rebuilt for Blue, the focus staying on the combo.
    s.pages[p.index()].focus_id("interface");
    pick(&mut s, &mut got, p, "interface", 1);
    assert_eq!(got.interface, "blue");
    assert!(
        s.pages[p.index()]
            .item(appearancepage::GLASS_COLOR)
            .is_none()
    );
    assert_eq!(
        rows(&s, p, appearancepage::BLUE_SCHEME).0,
        [
            "Blue",
            "Olive Green",
            "Silver",
            "Slate",
            "Sapphire",
            "Obsidian",
            "Ember",
            "Cobalt",
            "Carbon",
            "Steel"
        ]
    );
    assert!(
        glass_rows
            .iter()
            .all(|id| s.pages[p.index()].item(id).is_none())
    );
    let focused = |s: &PropSheet| {
        let form = &s.pages[p.index()];
        form.focus().map(|f| form.items[f].id)
    };
    assert_eq!(
        focused(&s),
        Some("interface"),
        "the rebuild keeps the focus"
    );
    // A revert moves the interface under the scheme combo: the focus goes to the new interface's.
    s.pages[p.index()].focus_id(appearancepage::BLUE_SCHEME);
    got.interface = "glass".into();
    follow(&mut s, &got);
    assert_eq!(focused(&s), Some(appearancepage::GLASS_COLOR));
}

#[test]
fn the_scheme_list_is_the_interfaces_own() {
    let p = Page::Appearance;
    let (classic, at) = rows(&built(), p, appearancepage::CLASSIC_SCHEME);
    assert_eq!(classic, reprisekit::schemes::names().collect::<Vec<_>>());
    assert_eq!(
        classic[at],
        appearancepage::classic_default(),
        "no key: the default row"
    );
    let mut got = Settings::default();
    let mut s = with(&got);
    let brick = classic
        .iter()
        .position(|n| n == "Brick")
        .expect("Brick is a Classic scheme");
    pick(&mut s, &mut got, p, appearancepage::CLASSIC_SCHEME, brick);
    assert_eq!(got.color_scheme, "Brick");
    let standard = classic
        .iter()
        .position(|n| n == appearancepage::classic_default())
        .unwrap();
    pick(
        &mut s,
        &mut got,
        p,
        appearancepage::CLASSIC_SCHEME,
        standard,
    );
    assert_eq!(got.color_scheme, "", "the default row writes no key");
    let (glass, at) = rows(&under("glass"), p, appearancepage::GLASS_COLOR);
    assert_eq!(
        glass.len(),
        reprisekit::glass::swatch_names().count(),
        "no Custom row"
    );
    assert_eq!((glass[0].as_str(), at), ("Sky", 0));
}

#[test]
fn a_custom_glass_colour_keeps_its_row() {
    let p = Page::Appearance;
    let mut got =
        crate::model::load("[appearance]\ninterface = \"glass\"\n[glass]\ncolor = \"#102030\"\n");
    let mut s = with(&got);
    let (list, at) = rows(&s, p, appearancepage::GLASS_COLOR);
    assert_eq!(
        (list.last().map(String::as_str), at),
        (Some(appearancepage::CUSTOM_ROW), list.len() - 1)
    );
    let untouched = got.clone();
    read_one(&s, "interface", &mut got);
    assert_eq!(got, untouched, "an untouched sheet writes nothing");
    pick(&mut s, &mut got, p, appearancepage::GLASS_COLOR, 3);
    assert_eq!(got.glass_color, "leaf");
    assert_eq!(
        rows(&s, p, appearancepage::GLASS_COLOR).0.len(),
        list.len(),
        "Custom stays"
    );
    pick(
        &mut s,
        &mut got,
        p,
        appearancepage::GLASS_COLOR,
        list.len() - 1,
    );
    assert_eq!(got.glass_color, "#102030");
}

#[test]
fn the_backdrop_rows_grey_without_transparency() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    let rows = [
        "glass_fast_drag",
        "glass_blur_frames",
        "glass_streaks",
        "blur_label",
        "glass_blur",
    ];
    for id in rows {
        assert!(enabled_on(&s, p, id), "{id}");
    }
    tick(&mut s, &mut got, p, "glass_transparency", false);
    assert!(!got.glass_transparency);
    for id in rows {
        assert!(
            !enabled_on(&s, p, id),
            "opaque glass has no backdrop for {id}"
        );
    }
}

#[test]
fn the_reflections_grey_on_smoked_and_molten_glass() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    for (row, on) in [(1, false), (2, false), (0, true)] {
        pick(&mut s, &mut got, p, "glass_material", row);
        assert_eq!(
            enabled_on(&s, p, "glass_streaks"),
            on,
            "{} glass",
            got.glass_material
        );
        assert!(enabled_on(&s, p, "glass_blur"), "{}", got.glass_material);
    }
}

#[test]
fn the_blur_trackbar_starts_at_blur_1() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    let Some(Control::TrackBar(t)) = s.pages[p.index()].item("glass_blur").map(|i| &i.control)
    else {
        panic!("a blur trackbar");
    };
    assert_eq!((t.pos, t.count), (1, 5), "the default blur 2 of 1..5");
    if let Some(Control::TrackBar(t)) = s.pages[p.index()]
        .item_mut("glass_blur")
        .map(|i| &mut i.control)
    {
        t.set(0);
    }
    read_one(&s, "glass_blur", &mut got);
    assert_eq!(got.glass_blur, 1, "the leftmost position is blur 1");
}

#[test]
fn the_start_button_glow_trackbar_writes_orb_glow() {
    let p = Page::Appearance;
    let mut got = Settings {
        interface: "glass".into(),
        glass_orb_glow: 70,
        ..Settings::default()
    };
    let mut s = with(&got);
    let Some(Control::TrackBar(t)) = s.pages[p.index()]
        .item("glass_orb_glow")
        .map(|i| &i.control)
    else {
        panic!("a Start button glow trackbar");
    };
    assert_eq!((t.pos, t.count), (70, 101), "70 of 0..100");
    for v in [0, 50, 100] {
        if let Some(Control::TrackBar(t)) = s.pages[p.index()]
            .item_mut("glass_orb_glow")
            .map(|i| &mut i.control)
        {
            t.set(v);
        }
        read_one(&s, "glass_orb_glow", &mut got);
        assert_eq!(got.glass_orb_glow, v as i64);
        let base = "[appearance]\ninterface = \"glass\"\n";
        let text = crate::model::save(base, &got, &crate::model::load(base)).unwrap();
        assert_eq!(crate::model::load(&text).glass_orb_glow, v as i64, "{text}");
    }
}

/// The sample follows the glass colour, intensity and transparency: its look
/// is the pending `[glass]`'s style, and a redraw shows the new colour.
#[test]
fn the_sample_follows_the_glass_settings() {
    use reprisekit::glass;
    use reprisekit::look::Look;
    use std::sync::Arc;
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let p = Page::Appearance;
    let draw = |s: &PropSheet| {
        let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
        form::draw(
            &mut c,
            &text,
            &Theme::default().paint(),
            &s.pages[p.index()],
            false,
            px,
        );
        c
    };
    let drawn_in_pending = |s: &PropSheet, got: &Settings| match &s.pages[p.index()]
        .item(appearancepage::SAMPLE)
        .unwrap()
        .control
    {
        Control::Sample {
            theme: Theme {
                look: Look::Styled(a),
                ..
            },
            ..
        } => Arc::ptr_eq(a, &glass::style(&appearancepage::glass(got))),
        _ => panic!("a Glass sample"),
    };
    let mut got = Settings {
        interface: "glass".into(),
        ..Settings::default()
    };
    let mut s = with(&got);
    let before = draw(&s);
    pick(&mut s, &mut got, p, appearancepage::GLASS_COLOR, 3);
    assert_eq!(got.glass_color, "leaf");
    assert!(drawn_in_pending(&s, &got), "the colour");
    assert_ne!(
        draw(&s).pixels,
        before.pixels,
        "a redraw shows the leaf glass"
    );
    if let Some(Control::TrackBar(t)) = s.pages[p.index()]
        .item_mut("glass_intensity")
        .map(|i| &mut i.control)
    {
        t.set(80);
    }
    read_one(&s, "glass_intensity", &mut got);
    follow(&mut s, &got);
    assert_eq!(got.glass_intensity, 80);
    assert!(drawn_in_pending(&s, &got), "the intensity");
    tick(&mut s, &mut got, p, "glass_transparency", false);
    assert!(drawn_in_pending(&s, &got), "the transparency");
}

#[test]
fn the_sample_draws_the_pending_scheme() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let p = Page::Appearance;
    let draw = |s: &PropSheet| {
        let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
        form::draw(
            &mut c,
            &text,
            &Theme::default().paint(),
            &s.pages[p.index()],
            false,
            px,
        );
        c
    };
    let s = with(&Settings {
        color_scheme: "Brick".into(),
        ..Settings::default()
    });
    let r = s.on_page(appearancepage::sample_rect(Area::MIN));
    let brick = reprisekit::schemes::scheme("Brick").expect("Brick is a Classic scheme");
    assert_eq!(draw(&s).get(r.x + 3, r.y + 3), brick.background.argb());
    let g = with(&Settings {
        interface: "glass".into(),
        background: "#102030".into(),
        ..Settings::default()
    });
    assert_eq!(
        draw(&g).get(r.x + 3, r.y + 3),
        Color::rgb(0x10, 0x20, 0x30).argb()
    );
}

#[test]
fn the_background_page_geometry() {
    let s = built();
    let p = Page::Background;
    assert_eq!(
        rect(&s, p, backgroundpage::MONITOR),
        Rect::new(20, 44, 191, 168)
    );
    assert_eq!(rect(&s, p, "wallpaper_label"), Rect::new(219, 44, 66, 13));
    assert_eq!(
        rect(&s, p, backgroundpage::MODE),
        Rect::new(289, 40, 89, 21)
    );
    assert_eq!(
        rect(&s, p, backgroundpage::WALLPAPER),
        Rect::new(219, 66, 159, 191)
    );
    assert_eq!(
        rect(&s, p, backgroundpage::BROWSE),
        Rect::new(303, 264, 75, 23)
    );
    assert_eq!(rect(&s, p, "slideshow_group"), Rect::new(20, 393, 358, 70));
    assert_eq!(
        rect(&s, p, backgroundpage::INTERVAL),
        Rect::new(166, 413, 120, 21)
    );
    assert_eq!(rect(&s, p, "slideshow_shuffle"), Rect::new(32, 439, 13, 13));
    assert_eq!(rect(&s, p, "position_label"), Rect::new(219, 310, 75, 13));
    assert_eq!(
        rect(&s, p, "wallpaper_position"),
        Rect::new(219, 326, 75, 21)
    );
    assert_eq!(rect(&s, p, "color_label"), Rect::new(303, 310, 75, 13));
    assert_eq!(rect(&s, p, "background"), Rect::new(303, 326, 75, 21));
    assert_eq!(rect(&s, p, "icons_group"), Rect::new(20, 222, 191, 160));
    assert_eq!(rect(&s, p, "extensions_label"), Rect::new(32, 334, 167, 13));
    assert_eq!(
        rect(&s, p, backgroundpage::EXTENSIONS),
        Rect::new(32, 350, 167, 21)
    );
    for (i, id) in ["documents", "computer", "home", "desktop_icons"]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(32, 244 + 22 * i as i32, 13, 13),
            "{id}"
        );
    }
    inside_page(&s, p);
}

#[test]
fn unticking_an_icon_keeps_the_recycle_bin() {
    let p = Page::Background;
    let mut got = Settings::default();
    let mut s = with(&got);
    tick(&mut s, &mut got, p, "home", false);
    assert_eq!(got.system_icons, ["documents", "computer", "recycle-bin"]);
    tick(&mut s, &mut got, p, "documents", false);
    tick(&mut s, &mut got, p, "home", true);
    assert_eq!(
        got.system_icons,
        ["computer", "recycle-bin", "home"],
        "a ticked icon goes at the end"
    );
}

#[test]
fn the_position_greys_without_a_wallpaper() {
    let p = Page::Background;
    let s = built();
    assert!(!enabled_on(&s, p, "wallpaper_position") && !enabled_on(&s, p, "position_label"));
    let s = with(&Settings {
        wallpaper: "/w/a.png".into(),
        ..Settings::default()
    });
    assert!(enabled_on(&s, p, "wallpaper_position") && enabled_on(&s, p, "position_label"));
}

#[test]
fn the_extensions_combo_writes_its_key_and_greys_without_the_files() {
    let p = Page::Background;
    let mut got = Settings::default();
    let mut s = with(&got);
    assert!(enabled_on(&s, p, backgroundpage::EXTENSIONS));
    pick(&mut s, &mut got, p, backgroundpage::EXTENSIONS, 1);
    assert_eq!(got.desktop_extensions, "hide_launchers");
    pick(&mut s, &mut got, p, backgroundpage::EXTENSIONS, 2);
    assert_eq!(got.desktop_extensions, "show");
    tick(&mut s, &mut got, p, "desktop_icons", false);
    assert!(
        !enabled_on(&s, p, backgroundpage::EXTENSIONS) && !enabled_on(&s, p, "extensions_label")
    );
    tick(&mut s, &mut got, p, "desktop_icons", true);
    assert!(enabled_on(&s, p, backgroundpage::EXTENSIONS));
}

/// The Color button shows the desktop colour the interface and scheme give until one is picked, and follows a scheme picked meanwhile.
#[test]
fn the_colour_button_shows_the_desktop_the_look_gives() {
    let colour = |s: &PropSheet| match &s.pages[Page::Background.index()]
        .item("background")
        .unwrap()
        .control
    {
        Control::Color(b) => b.color,
        _ => panic!("background is a colour button"),
    };
    let mut got = Settings::default();
    let mut s = with(&got);
    assert_eq!(colour(&s), Theme::default().colors.background);
    let brick = rows(&s, Page::Appearance, appearancepage::CLASSIC_SCHEME)
        .0
        .iter()
        .position(|n| n == "Brick")
        .unwrap();
    pick(
        &mut s,
        &mut got,
        Page::Appearance,
        appearancepage::CLASSIC_SCHEME,
        brick,
    );
    assert_eq!(
        colour(&s),
        reprisekit::schemes::scheme("Brick").unwrap().background
    );
    let s = with(&Settings {
        background: "#336699".into(),
        ..Settings::default()
    });
    assert_eq!(colour(&s), Color::rgb(0x33, 0x66, 0x99));
}

#[test]
fn the_monitor_shows_the_colour_then_the_picture_and_keeps_it_while_the_next_is_read() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let at = Page::Background.index();
    let mut want = Settings {
        background: "#102030".into(),
        ..Settings::default()
    };
    let mut s = with(&want);
    let r = s.on_page(backgroundpage::MONITOR_RECT);
    let (sx, sy, _, _) = reprisekit::classic::assets::MONITOR_SCREEN;
    let at_screen = |s: &PropSheet, x: i32, y: i32| {
        let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
        form::draw(
            &mut c,
            &text,
            &Theme::default().paint(),
            &s.pages[at],
            false,
            px,
        );
        c.get(r.x + sx + x, r.y + sy + y)
    };
    let bg = 0xFF10_2030;
    backgroundpage::show(&mut s.pages[at], &want, "", None, true);
    assert_eq!(
        at_screen(&s, 76, 56),
        bg,
        "no picture shown yet: the colour, even with a picture on its way"
    );
    // An 8 px red picture from a 960 px wide file on a 1920 px screen is centred at half the screen's width: 76 px of the 152.
    want.wallpaper = "/w/red.png".into();
    want.wallpaper_position = "center".into();
    let mut red = Canvas::new(8, 8);
    red.fill_rect(red.bounds(), Color::rgb(255, 0, 0));
    let pic = backgroundpage::Picture {
        path: "/w/red.png".into(),
        canvas: Some(red),
        source_w: 960,
    };
    backgroundpage::show(&mut s.pages[at], &want, "/w/red.png", Some(&pic), false);
    assert_eq!(
        at_screen(&s, 76, 56),
        0xFFFF_0000,
        "the middle of the screen"
    );
    assert_eq!(
        at_screen(&s, 2, 2),
        bg,
        "the colour round a centred picture"
    );
    want.wallpaper = "/w/blue.png".into();
    backgroundpage::show(&mut s.pages[at], &want, "/w/blue.png", Some(&pic), true);
    assert_eq!(
        at_screen(&s, 76, 56),
        0xFFFF_0000,
        "red stays while blue is decoded"
    );
    backgroundpage::show(&mut s.pages[at], &want, "", None, true);
    assert_eq!(
        at_screen(&s, 76, 56),
        0xFFFF_0000,
        "red stays while a slideshow folder is listed"
    );
    backgroundpage::show(&mut s.pages[at], &want, "/w/blue.png", Some(&pic), false);
    assert_eq!(at_screen(&s, 76, 56), bg, "blue did not decode: the colour");
}

#[test]
fn the_fonts_and_icons_page_geometry() {
    let s = built();
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "fonts_group"),
        Rect::new(20, 44, 358, 84)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "font_family"),
        Rect::new(32, 78, 163, 20)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "font_shell_dlg"),
        Rect::new(203, 78, 163, 20)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "font_size_px"),
        Rect::new(80, 104, 50, 20)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "font_antialias"),
        Rect::new(192, 108, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "icons_group"),
        Rect::new(20, 138, 358, 56)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "icon_theme"),
        Rect::new(92, 158, 274, 21)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "windows_group"),
        Rect::new(20, 204, 358, 92)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "taskbar_height"),
        Rect::new(146, 274, 50, 20)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "toolkits_group"),
        Rect::new(20, 306, 358, 114)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "gtk_theme"),
        Rect::new(120, 326, 246, 21)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "qt_platform_theme"),
        Rect::new(120, 352, 246, 21)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "smoothing_label"),
        Rect::new(32, 382, 84, 13)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "font_smoothing"),
        Rect::new(120, 378, 246, 21)
    );
    assert_eq!(
        rect(&s, Page::FontsAndIcons, "gtk4_colors"),
        Rect::new(32, 404, 13, 13)
    );
    let fi = &s.pages[Page::FontsAndIcons.index()];
    assert!(
        fi.item("menus_group").is_none(),
        "the Menus group is on Effects"
    );
    inside_page(&s, Page::FontsAndIcons);
}

#[test]
fn the_effects_page_geometry() {
    let s = built();
    let p = Page::Effects;
    assert_eq!(p.label(), "Effects");
    assert_eq!(rect(&s, p, "window_animation"), Rect::new(GX, 66, 13, 13));
    assert_eq!(rect(&s, p, "open_label"), Rect::new(50, 92, 70, 13));
    assert_eq!(rect(&s, p, "window_open_ms"), Rect::new(124, 88, 50, 20));
    assert_eq!(rect(&s, p, "close_label"), Rect::new(190, 92, 70, 13));
    assert_eq!(rect(&s, p, "window_close_ms"), Rect::new(264, 88, 50, 20));
    assert_eq!(rect(&s, p, "hover_fade"), Rect::new(GX, 122, 13, 13));
    assert_eq!(rect(&s, p, "in_label"), Rect::new(50, 148, 70, 13));
    assert_eq!(rect(&s, p, "hover_in_ms"), Rect::new(124, 144, 50, 20));
    assert_eq!(rect(&s, p, "out_label"), Rect::new(190, 148, 70, 13));
    assert_eq!(rect(&s, p, "hover_out_ms"), Rect::new(264, 144, 50, 20));
    assert_eq!(rect(&s, p, "session_fade"), Rect::new(GX, 178, 13, 13));
    assert_eq!(rect(&s, p, "menus_group"), Rect::new(20, 212, 358, 74));
    assert_eq!(rect(&s, p, "menu_shadow"), Rect::new(96, 230, 50, 20));
    assert_eq!(rect(&s, p, "menu_hover_ms"), Rect::new(246, 230, 50, 20));
    assert_eq!(rect(&s, p, "menu_fade_ms"), Rect::new(96, 256, 50, 20));
    assert_eq!(rect(&s, p, "menu_animation"), Rect::new(246, 256, 110, 21));
    assert_eq!(rect(&s, p, "taskbar_group"), Rect::new(20, 296, 358, 48));
    assert_eq!(rect(&s, p, "taskbar_slide"), Rect::new(GX, 318, 13, 13));
    for id in ["notification_sound", "sound_theme"] {
        assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Sounds");
    }
    inside_page(&s, p);
}

#[test]
fn the_menu_timings_go_in_and_come_back_out_on_effects() {
    let want = Settings {
        menu_shadow: 4,
        menu_hover_ms: 250,
        menu_fade_ms: 0,
        menu_animation: "slide".into(),
        ..Settings::default()
    };
    let s = with(&want);
    let fx = &s.pages[Page::Effects.index()];
    let value = |id: &str| match &fx
        .item(id)
        .unwrap_or_else(|| panic!("{id} is on Effects"))
        .control
    {
        Control::UpDown(u) => u.value(),
        _ => panic!("{id} is an up-down"),
    };
    assert_eq!(
        (
            value("menu_shadow"),
            value("menu_hover_ms"),
            value("menu_fade_ms")
        ),
        (4, 250, 0)
    );
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

fn set_check(s: &mut PropSheet, got: &mut Settings, id: &str, on: bool) {
    if let Some(Control::Check { checked, .. }) = s.pages[Page::Effects.index()]
        .item_mut(id)
        .map(|i| &mut i.control)
    {
        *checked = on;
    }
    read_one(s, id, got);
    follow(s, got);
}

#[test]
fn the_effect_spins_grey_with_their_check() {
    let mut got = Settings::for_interface("glass");
    let mut s = with(&got);
    let p = Page::Effects;
    assert!(enabled_on(&s, p, "window_open_ms") && enabled_on(&s, p, "hover_out_ms"));
    set_check(&mut s, &mut got, "window_animation", false);
    assert!(!enabled_on(&s, p, "window_open_ms") && !enabled_on(&s, p, "window_close_ms"));
    assert!(
        enabled_on(&s, p, "hover_in_ms"),
        "the other group keeps going"
    );
    set_check(&mut s, &mut got, "hover_fade", false);
    assert!(!enabled_on(&s, p, "hover_in_ms") && !enabled_on(&s, p, "hover_out_ms"));
}

#[test]
fn the_taskbar_slide_check_goes_in_and_comes_back_out() {
    let mut got = Settings::for_interface("glass");
    let mut s = with(&got);
    let checked = |s: &PropSheet| match &s.pages[Page::Effects.index()]
        .item("taskbar_slide")
        .unwrap()
        .control
    {
        Control::Check { checked, .. } => *checked,
        _ => panic!("taskbar_slide is a check"),
    };
    assert!(checked(&s), "Glass slides its task buttons");
    set_check(&mut s, &mut got, "taskbar_slide", false);
    assert!(!got.taskbar_slide);
    assert!(!checked(&with(&Settings::for_interface("classic"))));
}

#[test]
fn the_night_light_page_geometry() {
    let s = built();
    let p = Page::NightLight;
    assert_eq!(rect(&s, p, "nl_enabled"), Rect::new(20, 44, 13, 13));
    assert_eq!(rect(&s, p, "nl_color_group"), Rect::new(20, 70, 358, 76));
    assert_eq!(rect(&s, p, "nl_temp_value"), Rect::new(286, 90, 80, 13));
    assert_eq!(
        rect(&s, p, "nl_temperature"),
        Rect::new(78, 111, 238, trackbar::HEIGHT)
    );
    assert_eq!(rect(&s, p, "nl_cool"), Rect::new(322, 117, 44, 13));
    assert_eq!(
        rect(&s, p, "nl_schedule_group"),
        Rect::new(20, 156, 358, 104)
    );
    assert_eq!(rect(&s, p, "nl_sun"), Rect::new(32, 178, 12, 12));
    assert_eq!(rect(&s, p, "nl_fixed"), Rect::new(32, 200, 12, 12));
    assert_eq!(rect(&s, p, "nl_from"), Rect::new(90, 226, 70, 20));
    assert_eq!(rect(&s, p, "nl_to"), Rect::new(210, 226, 70, 20));
    inside_page(&s, p);
}

#[test]
fn night_light_values_go_in_and_come_back_out() {
    let want = Settings {
        nl_enabled: true,
        nl_temperature: 3400,
        nl_schedule: "fixed".into(),
        nl_from: "20:30".into(),
        nl_to: "06:05".into(),
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::NightLight.index()];
    assert_eq!(page.track_pos("nl_temperature"), Some(9));
    assert_eq!(page.checked("nl_fixed"), Some(true));
    assert_eq!(page.checked("nl_sun"), Some(false));
    match &page.item("nl_from").unwrap().control {
        Control::Time(tf) => assert_eq!(tf.text().0, "20:30"),
        _ => panic!("nl_from is a time field"),
    }
    match &page.item("nl_temp_value").unwrap().control {
        Control::Label { text, .. } => assert_eq!(text, "3400 K"),
        _ => panic!("a label"),
    }
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_night_light_controls_follow_the_check_box_and_the_schedule() {
    let enabled = |s: &PropSheet, id: &str| match &s.pages[Page::NightLight.index()]
        .item(id)
        .unwrap()
        .control
    {
        Control::Label { enabled, .. } | Control::Radio { enabled, .. } => *enabled,
        Control::TrackBar(t) => t.enabled,
        Control::Time(t) => t.enabled,
        _ => panic!("{id} has no greyed state here"),
    };
    let off = with(&Settings::default());
    for id in ["nl_temperature", "nl_warm", "nl_sun", "nl_from"] {
        assert!(!enabled(&off, id), "{id} greys while off");
    }
    let sun = Settings {
        nl_enabled: true,
        ..Settings::default()
    };
    let s = with(&sun);
    assert!(enabled(&s, "nl_temperature"));
    assert!(enabled(&s, "nl_fixed"));
    assert!(!enabled(&s, "nl_from"), "sunset needs no times");
    assert!(!enabled(&s, "nl_to_label"));
    let fixed = Settings {
        nl_schedule: "fixed".into(),
        ..sun
    };
    let s = with(&fixed);
    assert!(enabled(&s, "nl_from"));
    assert!(enabled(&s, "nl_to_label"));
}

#[test]
fn automatic_toolkits_are_the_first_rows() {
    let sheet = with(&Settings::default());
    let page = &sheet.pages[Page::FontsAndIcons.index()];
    assert_eq!(page.combo_selected("gtk_theme"), Some(0));
    assert_eq!(page.combo_selected("qt_platform_theme"), Some(0));
    let mut got = Settings {
        gtk_theme: "stale".into(),
        qt_platform_theme: "stale".into(),
        ..Settings::default()
    };
    read_all(&sheet, &mut got);
    assert_eq!(
        (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
        ("auto", "auto")
    );
    let odd = Settings {
        gtk_theme: "Gone".into(),
        qt_platform_theme: "lxqt".into(),
        ..Settings::default()
    };
    let mut got = Settings::default();
    read_all(&with(&odd), &mut got);
    assert_eq!(
        (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
        ("Gone", "lxqt")
    );
}

/// "Reprise (follows the look)" is each toolkit combo's second row and reads
/// back as "reprise"; the Qt one is greyed without qt6ct or qt5ct.
#[test]
fn reprise_toolkits_are_the_second_rows_and_qt_needs_a_tool() {
    let want = Settings {
        gtk_theme: TOOLKIT_REPRISE.into(),
        qt_platform_theme: TOOLKIT_REPRISE.into(),
        ..Settings::default()
    };
    let sheet = with(&want);
    let page = &sheet.pages[Page::FontsAndIcons.index()];
    assert_eq!(page.combo_selected("gtk_theme"), Some(1));
    assert_eq!(page.combo_selected("qt_platform_theme"), Some(1));
    let mut got = Settings::default();
    read_all(&sheet, &mut got);
    assert_eq!(
        (got.gtk_theme.as_str(), got.qt_platform_theme.as_str()),
        ("reprise", "reprise")
    );
    let qt_row = |tools| {
        let page = fonts_and_icons(Area::MIN, &[], &[], tools);
        match &page.item("qt_platform_theme").unwrap().control {
            Control::Combo(c) => (c.items[1].text.clone(), c.items[1].greyed),
            _ => unreachable!("the Qt style is a combo"),
        }
    };
    assert_eq!(qt_row(true), (REPRISE_TOOLKIT_ROW.to_string(), false));
    assert_eq!(qt_row(false), (REPRISE_TOOLKIT_ROW.to_string(), true));
}

#[test]
fn font_smoothing_starts_on_grayscale_and_reads_back_its_pick() {
    let sheet = with(&Settings::default());
    let page = &sheet.pages[Page::FontsAndIcons.index()];
    assert_eq!(page.combo_selected("font_smoothing"), Some(0));
    let none = Settings {
        font_smoothing: "none".into(),
        ..Settings::default()
    };
    let sheet = with(&none);
    assert_eq!(
        sheet.pages[Page::FontsAndIcons.index()].combo_selected("font_smoothing"),
        Some(2)
    );
    let mut got = Settings::default();
    read_all(&sheet, &mut got);
    assert_eq!(got.font_smoothing, "none");
}

#[test]
fn an_unlisted_icon_theme_still_shows() {
    let want = Settings {
        icon_theme: "gone".into(),
        ..Settings::default()
    };
    let sheet = with(&want);
    let mut got = Settings::default();
    read_all(&sheet, &mut got);
    assert_eq!(got.icon_theme, "gone");
}
