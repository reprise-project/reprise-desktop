use super::*;

#[test]
fn the_start_menu_page_sits_inside_its_page() {
    inside_page(&built(), Page::StartMenu);
}

#[test]
fn the_switcher_page_geometry() {
    let s = built();
    let p = Page::Switcher;
    assert_eq!(rect(&s, p, "switcher_group"), Rect::new(20, 44, 358, 168));
    assert_eq!(rect(&s, p, "switcher_style"), Rect::new(146, 64, 130, 21));
    assert_eq!(rect(&s, p, "switcher_size"), Rect::new(146, 90, 130, 21));
    assert_eq!(rect(&s, p, "switcher_preview"), Rect::new(32, 118, 13, 13));
    assert_eq!(rect(&s, p, "switcher_desktop"), Rect::new(32, 140, 13, 13));
    assert_eq!(rect(&s, p, "switcher_peek"), Rect::new(32, 162, 13, 13));
    assert_eq!(rect(&s, p, "switcher_flip"), Rect::new(32, 184, 13, 13));
    assert_eq!(rect(&s, p, "overview_group"), Rect::new(20, 224, 358, 76));
    assert_eq!(rect(&s, p, "hot_corner"), Rect::new(146, 268, 130, 21));
    inside_page(&s, p);
}

#[test]
fn switcher_values_go_in_and_come_back_out() {
    let want = Settings {
        switcher_style: "thumbnails".into(),
        switcher_size: "large".into(),
        switcher_preview: false,
        switcher_desktop: true,
        switcher_peek: true,
        switcher_flip: true,
        hot_corner: "bottom-right".into(),
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::Switcher.index()];
    assert_eq!(page.checked("switcher_desktop"), Some(true));
    assert_eq!(page.checked("switcher_peek"), Some(true));
    assert_eq!(page.checked("switcher_flip"), Some(true));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_desktops_page_geometry() {
    let s = built();
    let p = Page::Desktops;
    assert_eq!(rect(&s, p, "desktops_group"), Rect::new(20, 44, 358, 244));
    assert_eq!(rect(&s, p, "use_desktops"), Rect::new(32, 64, 13, 13));
    assert_eq!(rect(&s, p, "desktops_label"), Rect::new(50, 88, 206, 13));
    assert_eq!(rect(&s, p, "desktops"), Rect::new(260, 84, 50, 20));
    assert_eq!(
        rect(&s, p, "desktop_animation"),
        Rect::new(164, 110, 130, 21)
    );
    assert_eq!(
        rect(&s, p, "desktop_animation_ms"),
        Rect::new(164, 136, 50, 20)
    );
    assert_eq!(rect(&s, p, "duration_units"), Rect::new(220, 140, 60, 13));
    assert_eq!(rect(&s, p, "pager_mark_label"), Rect::new(50, 166, 190, 13));
    assert_eq!(rect(&s, p, "pager_mark"), Rect::new(50, 184, 134, 21));
    assert_eq!(rect(&s, p, "wheel_on_desktop"), Rect::new(50, 214, 13, 13));
    assert_eq!(
        rect(&s, p, "desktop_names_label"),
        Rect::new(50, 242, 90, 13)
    );
    assert_eq!(rect(&s, p, DESKTOP_NAMES), Rect::new(144, 238, 222, 20));
    assert_eq!(
        rect(&s, p, "desktop_names_note"),
        Rect::new(50, 264, 316, 13)
    );
    assert_eq!(rect(&s, p, "previews_group"), Rect::new(20, 298, 358, 76));
    assert_eq!(rect(&s, p, "preview_taskbar"), Rect::new(32, 322, 13, 13));
    assert_eq!(rect(&s, p, "previews_fps"), Rect::new(146, 344, 50, 20));
    assert_eq!(rect(&s, p, "fps_units"), Rect::new(202, 348, 100, 13));
    assert_eq!(rect(&s, p, "widgets_group"), Rect::new(20, 384, 358, 88));
    assert_eq!(rect(&s, p, "widget_clock"), Rect::new(32, 406, 13, 13));
    assert_eq!(rect(&s, p, "widget_meter"), Rect::new(32, 450, 13, 13));
    for id in ["session_restore", "session_end_processes"] {
        assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Startup");
    }
    inside_page(&s, p);
}

#[test]
fn the_wheel_on_desktop_box_writes_its_key_and_greys_with_one_desktop() {
    let p = Page::Desktops;
    let mut got = Settings::default();
    let s = with(&got);
    assert_eq!(s.pages[p.index()].checked("wheel_on_desktop"), Some(false));
    assert!(enabled_on(&s, p, "wheel_on_desktop"));
    let mut s = with(&Settings {
        wheel_on_desktop: true,
        ..Settings::default()
    });
    assert_eq!(s.pages[p.index()].checked("wheel_on_desktop"), Some(true));
    if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
        .item_mut("wheel_on_desktop")
        .map(|i| &mut i.control)
    {
        *checked = true;
    }
    read_one(&s, "wheel_on_desktop", &mut got);
    assert!(got.wheel_on_desktop);
    let one = with(&Settings {
        desktops: 1,
        ..Settings::default()
    });
    assert!(!enabled_on(&one, p, "wheel_on_desktop"));
}

#[test]
fn desktop_names_go_in_apart_by_commas_and_come_back_entry_for_entry() {
    let p = Page::Desktops;
    let mut s = with(&Settings {
        desktop_names: vec!["Work".into(), "".into(), "Chat".into()],
        ..Settings::default()
    });
    assert_eq!(s.pages[p.index()].edit_text(DESKTOP_NAMES), "Work, , Chat");
    if let Some(Control::Edit { edit, .. }) = s.pages[p.index()]
        .item_mut(DESKTOP_NAMES)
        .map(|i| &mut i.control)
    {
        *edit = reprisekit::textedit::TextEdit::new(" Mail ,,Music, , ".to_string(), false);
    }
    let mut got = Settings::default();
    read_one(&s, DESKTOP_NAMES, &mut got);
    assert_eq!(got.desktop_names, ["Mail", "", "Music"]);
    let one = with(&Settings {
        desktops: 1,
        ..Settings::default()
    });
    assert!(!enabled_on(&one, p, DESKTOP_NAMES));
}

/// The names row and its note, filled, empty and greyed.
#[test]
fn the_desktop_names_rows_pin_their_pixels_in_every_look() {
    let row = Rect::new(46, 236, 332, 42);
    let mut pins = Vec::new();
    for names in [vec!["Work".to_string(), "Chat".into()], Vec::new()] {
        let mut f = desktops(Area::MIN);
        if let Some(Control::Edit { edit, .. }) = f.item_mut(DESKTOP_NAMES).map(|i| &mut i.control)
        {
            *edit = reprisekit::textedit::TextEdit::new(names.join(", "), false);
        }
        let Some(h) = row_pins(&f, row) else {
            return;
        };
        pins.push(h);
    }
    let mut f = desktops(Area::MIN);
    for id in ["desktop_names_label", DESKTOP_NAMES, "desktop_names_note"] {
        f.set_enabled(id, false);
    }
    pins.push(row_pins(&f, row).unwrap());
    assert_eq!(
        pins,
        [
            [0x175f6ce3e5c88b48, 0x8384de64ca166860, 0xf350274c9b867d96],
            [0x667aef5cce7654be, 0xf23a3bb9df410796, 0x8319f5382979e4ac],
            [0xef8f79758ce43c65, 0x761da2a50ec9ebd8, 0x620c1f5c0d189658],
        ],
        "named, empty, greyed; Classic, Blue, Glass"
    );
}

#[test]
fn the_widget_boxes_tick_what_stands_and_write_desktop_widgets() {
    let p = Page::Desktops;
    let mut got = Settings {
        desktop_widgets: vec!["meter".into(), "clock".into()],
        ..Settings::default()
    };
    let mut s = with(&got);
    let page = &s.pages[p.index()];
    assert_eq!(page.checked("widget_clock"), Some(true));
    assert_eq!(page.checked("widget_calendar"), Some(false));
    assert_eq!(page.checked("widget_meter"), Some(true));
    let mut tick = |id: &str, on: bool| {
        if let Some(Control::Check { checked, .. }) =
            s.pages[p.index()].item_mut(id).map(|i| &mut i.control)
        {
            *checked = on;
        }
    };
    tick("widget_calendar", true);
    tick("widget_meter", false);
    for id in ["widget_clock", "widget_calendar", "widget_meter"] {
        read_one(&s, id, &mut got);
    }
    assert_eq!(got.desktop_widgets, ["clock", "calendar"]);
}

#[test]
fn the_wheel_on_desktop_row_pins_its_pixels_in_every_look() {
    let mut f = desktops(Area::MIN);
    let row = Rect::new(50, 212, 328, 17);
    let Some(off) = row_pins(&f, row) else {
        return;
    };
    if let Some(Control::Check { checked, .. }) =
        f.item_mut("wheel_on_desktop").map(|i| &mut i.control)
    {
        *checked = true;
    }
    let on = row_pins(&f, row).unwrap();
    f.set_enabled("wheel_on_desktop", false);
    let greyed = row_pins(&f, row).unwrap();
    assert_eq!(
        [off, on, greyed],
        [
            [0x444d64fcbc043bf4, 0x8613c8d92bda6a23, 0x36ce02d14726ecc7],
            [0x6008f30a911bdf3b, 0xb211584d4f8c711f, 0x78eb9868337e9a80],
            [0xf69aabd6f21cade1, 0x356b03616b75d2fe, 0x6d40df75f8c1acfa],
        ],
        "off, ticked, greyed; Classic, Blue, Glass"
    );
}

/// One desktop unticks "Use virtual desktops" and greys the count, animation
/// and duration; the count shows the default so a tick starts from it.
/// Ticking writes the count back, unticking writes 1.
#[test]
fn use_virtual_desktops_sets_one_and_greys_the_rest() {
    let one = Settings {
        desktops: 1,
        desktop_animation: "slide".into(),
        ..Settings::default()
    };
    let mut s = with(&one);
    let p = Page::Desktops;
    assert_eq!(s.pages[p.index()].checked("use_desktops"), Some(false));
    for id in [
        "desktops",
        "desktops_label",
        "desktop_animation",
        "desktop_animation_ms",
    ] {
        assert!(!enabled_on(&s, p, id), "{id} greys with one desktop");
    }
    match &s.pages[p.index()].item("desktops").unwrap().control {
        Control::UpDown(u) => assert_eq!((u.value(), u.min, u.max), (4, 2, 16)),
        _ => panic!("an up-down"),
    }

    let mut got = one.clone();
    if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
        .item_mut("use_desktops")
        .map(|i| &mut i.control)
    {
        *checked = true;
    }
    read_one(&s, "use_desktops", &mut got);
    assert_eq!(got.desktops, 4, "ticked: the count the up-down shows");
    follow(&mut s, &got);
    for id in ["desktops", "desktop_animation", "desktop_animation_ms"] {
        assert!(enabled_on(&s, p, id), "{id} comes back");
    }

    if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
        .item_mut("use_desktops")
        .map(|i| &mut i.control)
    {
        *checked = false;
    }
    read_one(&s, "use_desktops", &mut got);
    assert_eq!(got.desktops, 1, "unticked: one desktop");
}

#[test]
fn the_duration_and_the_refresh_rate_follow_what_uses_them() {
    let p = Page::Desktops;
    let still = Settings {
        switcher_preview: false,
        preview_taskbar: false,
        ..Settings::default()
    };
    let mut s = with(&still);
    assert!(enabled_on(&s, p, "desktop_animation"));
    assert!(!enabled_on(&s, p, "desktop_animation_ms"), "no animation");
    assert!(!enabled_on(&s, p, "duration_units"));
    for id in FPS_CONTROLS {
        assert!(!enabled_on(&s, p, id), "{id} greys with no live picture");
    }
    let thumbs = Settings {
        switcher_style: "thumbnails".into(),
        desktop_animation: "fade".into(),
        ..still.clone()
    };
    follow(&mut s, &thumbs);
    assert!(enabled_on(&s, p, "previews_fps"), "thumbnails are live");
    assert!(enabled_on(&s, p, "desktop_animation_ms"));
    let taskbar = Settings {
        preview_taskbar: true,
        ..still
    };
    follow(&mut s, &taskbar);
    assert!(enabled_on(&s, p, "previews_fps"));
}

/// The combos show words, and the config's values go in and come back out through them.
#[test]
fn named_combos_show_labels_and_carry_values() {
    let want = Settings {
        switcher_style: "thumbnails".into(),
        desktop_animation: "slide-vertical".into(),
        ..Settings::default()
    };
    let s = with(&want);
    let shown = |page: Page, id: &str| match &s.pages[page.index()].item(id).unwrap().control {
        Control::Combo(c) => c.items[c.selected].text.clone(),
        _ => panic!("{id} is a combo"),
    };
    assert_eq!(shown(Page::Switcher, "switcher_style"), "Thumbnails");
    assert_eq!(
        shown(Page::Desktops, "desktop_animation"),
        "Slide vertically"
    );
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);

    let odd = Settings {
        desktop_animation: "spin".into(),
        ..Settings::default()
    };
    let s = with(&odd);
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got.desktop_animation, "spin", "an unknown value survives");
}

#[test]
fn the_pager_mark_greys_outside_glass() {
    let p = Page::Desktops;
    let with_interface = |interface: &str, desktops: i64| Settings {
        interface: interface.into(),
        desktops,
        ..Settings::default()
    };
    let mut s = with(&with_interface("classic", 4));
    for id in ["pager_mark_label", "pager_mark"] {
        assert!(!enabled_on(&s, p, id), "{id} greys outside Glass");
    }
    follow(&mut s, &with_interface("glass", 4));
    for id in ["pager_mark_label", "pager_mark"] {
        assert!(enabled_on(&s, p, id), "{id} enables on Glass");
    }
    follow(&mut s, &with_interface("glass", 1));
    for id in ["pager_mark_label", "pager_mark"] {
        assert!(!enabled_on(&s, p, id), "{id} greys with one desktop");
    }
}

#[test]
fn the_snap_bar_and_distance_grey_with_drag_snapping_off() {
    let p = Page::Windows;
    let off = Settings {
        snap_enabled: false,
        ..Settings::default()
    };
    let mut s = with(&off);
    for id in DRAG_SNAP_CONTROLS {
        assert!(!enabled_on(&s, p, id), "{id} greys: it needs drags");
    }
    assert_eq!(s.pages[p.index()].checked("snap_bar"), Some(true));
    follow(&mut s, &Settings::default());
    for id in DRAG_SNAP_CONTROLS {
        assert!(enabled_on(&s, p, id), "{id} follows drag snapping back on");
    }
}

#[test]
fn the_flip_check_greys_outside_glass() {
    let p = Page::Switcher;
    let on = |interface: &str| Settings {
        interface: interface.into(),
        switcher_flip: true,
        ..Settings::default()
    };
    let mut s = with(&on("classic"));
    assert!(!enabled_on(&s, p, "switcher_flip"), "greyed under Classic");
    assert_eq!(s.pages[p.index()].checked("switcher_flip"), Some(true));
    follow(&mut s, &on("blue"));
    assert!(!enabled_on(&s, p, "switcher_flip"), "greyed under Blue");
    follow(&mut s, &on("glass"));
    assert!(enabled_on(&s, p, "switcher_flip"), "enabled under Glass");
}

#[test]
fn the_pager_mark_combo_carries_its_value() {
    let want = Settings {
        pager_mark: "tile".into(),
        ..Settings::default()
    };
    let s = with(&want);
    match &s.pages[Page::Desktops.index()]
        .item("pager_mark")
        .unwrap()
        .control
    {
        Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Glass tile"),
        _ => panic!("a combo"),
    }
    let odd = Settings {
        pager_mark: "box".into(),
        ..Settings::default()
    };
    let s = with(&odd);
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got.pager_mark, "box", "an unknown value survives");
}

#[test]
fn the_taskbar_page_geometry() {
    let s = built();
    let p = Page::Taskbar;
    assert_eq!(rect(&s, p, "location_label"), Rect::new(20, 48, 100, 13));
    assert_eq!(rect(&s, p, "taskbar_position"), Rect::new(124, 44, 120, 21));
    assert_eq!(rect(&s, p, "buttons_label"), Rect::new(20, 76, 100, 13));
    assert_eq!(rect(&s, p, "taskbar_buttons"), Rect::new(124, 72, 230, 21));
    for (i, id) in [
        "taskbar_pinned",
        "taskbar_small_icons",
        "taskbar_show_desktop",
        "taskbar_peek",
        "taskbar_badges",
        "taskbar_tray_overflow",
        "taskbar_network_meter",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(20, 100 + 18 * i as i32, 13, 13),
            "{id} is row {i}"
        );
    }
    assert_eq!(rect(&s, p, "taskbar_note"), Rect::new(20, 228, 358, 26));
    assert_eq!(rect(&s, p, "taskbar_auto_hide"), Rect::new(20, 258, 13, 13));
    for (i, id) in REVEAL_CONTROLS.into_iter().enumerate() {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(38, 278 + 20 * i as i32, 13, 13),
            "{id} sits under the auto-hide label"
        );
    }
    assert!(
        s.pages[p.index()].item("recent_files").is_none(),
        "recent_files is on Advanced"
    );
    assert_eq!(
        rect(&s, p, "start_group"),
        Rect::new(20, 338, 358, 94),
        "the Start Button group ends 4 px above the page's foot"
    );
    assert_eq!(rect(&s, p, "start_icon"), Rect::new(32, 360, 13, 13));
    assert_eq!(rect(&s, p, "banner_label"), Rect::new(32, 384, 110, 13));
    assert_eq!(
        rect(&s, p, "startmenu_banner"),
        Rect::new(146, 380, 220, 20)
    );
    assert_eq!(
        rect(&s, p, "startmenu_style_label"),
        Rect::new(32, 412, 110, 13)
    );
    assert_eq!(rect(&s, p, "startmenu_style"), Rect::new(146, 408, 220, 21));
    inside_page(&s, p);
}

#[test]
fn taskbar_values_go_in_and_come_back_out() {
    let want = Settings {
        taskbar_position: "top".into(),
        taskbar_buttons: "combine".into(),
        taskbar_pinned: true,
        taskbar_peek: true,
        taskbar_tray_overflow: true,
        taskbar_badges: false,
        taskbar_network_meter: true,
        recent_files: false,
        start_icon: false,
        startmenu_banner: "Reprise".into(),
        startmenu_style: "classic".into(),
        taskbar_auto_hide: true,
        taskbar_reveal_on_edge: false,
        taskbar_reveal_on_attention: false,
        ..Settings::default()
    };
    let s = with(&want);
    match &s.pages[Page::Taskbar.index()]
        .item("taskbar_buttons")
        .unwrap()
        .control
    {
        Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Combine, icons only"),
        _ => panic!("taskbar_buttons is a combo"),
    }
    match &s.pages[Page::Taskbar.index()]
        .item("taskbar_position")
        .unwrap()
        .control
    {
        Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Top"),
        _ => panic!("taskbar_position is a combo"),
    }
    let tb = &s.pages[Page::Taskbar.index()];
    assert_eq!(tb.checked("start_icon"), Some(false));
    assert_eq!(tb.checked("taskbar_auto_hide"), Some(true));
    assert_eq!(tb.checked("taskbar_reveal_on_edge"), Some(false));
    assert_eq!(tb.checked("taskbar_reveal_on_keys"), Some(true));
    assert_eq!(tb.checked("taskbar_badges"), Some(false));
    assert_eq!(tb.checked("taskbar_network_meter"), Some(true));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_location_combo_lists_bottom_top_left_and_right() {
    let want = Settings {
        taskbar_position: "right".into(),
        ..Settings::default()
    };
    let s = with(&want);
    match &s.pages[Page::Taskbar.index()]
        .item("taskbar_position")
        .unwrap()
        .control
    {
        Control::Combo(c) => {
            let texts: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
            assert_eq!(texts, ["Bottom", "Top", "Left", "Right"]);
            assert_eq!(c.items[c.selected].text, "Right");
        }
        _ => panic!("taskbar_position is a combo"),
    }
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(
        got.taskbar_position, "right",
        "read back as the config's name"
    );
}

#[test]
fn the_reveal_check_boxes_grey_while_auto_hide_is_off() {
    let mut got = Settings::default();
    let mut s = with(&got);
    let p = Page::Taskbar;
    assert!(enabled_on(&s, p, "taskbar_auto_hide"));
    for id in REVEAL_CONTROLS {
        assert!(!enabled_on(&s, p, id), "{id} greys with auto-hide off");
    }
    if let Some(Control::Check { checked, .. }) = s.pages[p.index()]
        .item_mut("taskbar_auto_hide")
        .map(|i| &mut i.control)
    {
        *checked = true;
    }
    read_one(&s, "taskbar_auto_hide", &mut got);
    follow(&mut s, &got);
    assert!(got.taskbar_auto_hide);
    for id in REVEAL_CONTROLS {
        assert!(enabled_on(&s, p, id), "{id} comes back with auto-hide on");
    }
}

#[test]
fn the_launcher_page_geometry() {
    let s = built();
    let p = Page::Launcher;
    assert_eq!(
        rect(&s, p, "launcher_search_group"),
        Rect::new(20, 44, 358, 108)
    );
    assert_eq!(
        rect(&s, p, "launcher_search_note"),
        Rect::new(32, 66, 334, 26)
    );
    assert_eq!(rect(&s, p, "launcher_files"), Rect::new(32, 104, 13, 13));
    assert_eq!(rect(&s, p, "launcher_windows"), Rect::new(32, 126, 13, 13));
    inside_page(&s, p);
}

#[test]
fn launcher_values_go_in_and_come_back_out() {
    let want = Settings {
        launcher_files: false,
        launcher_windows: false,
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::Launcher.index()];
    assert_eq!(page.checked("launcher_files"), Some(false));
    assert_eq!(page.checked("launcher_windows"), Some(false));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_windows_page_geometry() {
    let s = built();
    let p = Page::Windows;
    assert_eq!(rect(&s, p, "snap_group"), Rect::new(20, 44, 358, 172));
    assert_eq!(rect(&s, p, "snap_enabled"), Rect::new(32, 66, 13, 13));
    assert_eq!(rect(&s, p, "snap_picker"), Rect::new(32, 90, 13, 13));
    assert_eq!(rect(&s, p, "snap_bar"), Rect::new(32, 114, 13, 13));
    assert_eq!(
        rect(&s, p, "snap_distance_label"),
        Rect::new(32, 142, 150, 13)
    );
    assert_eq!(rect(&s, p, "snap_distance").y, 138);
    assert_eq!(
        rect(&s, p, "snap_distance_units"),
        Rect::new(242, 142, 30, 13)
    );
    assert_eq!(rect(&s, p, "snap_layouts"), Rect::new(120, 166, 246, 20));
    assert_eq!(rect(&s, p, "tabs_group"), Rect::new(20, 228, 358, 182));
    assert_eq!(rect(&s, p, "window_tabs"), Rect::new(32, 250, 13, 13));
    assert_eq!(rect(&s, p, "remember_windows"), Rect::new(32, 274, 13, 13));
    assert_eq!(rect(&s, p, FORGET_FRAMES), Rect::new(50, 296, 110, 23));
    assert_eq!(rect(&s, p, "window_shake"), Rect::new(32, 328, 13, 13));
    assert_eq!(
        rect(&s, p, "title_double_click_label"),
        Rect::new(32, 356, 150, 13)
    );
    assert_eq!(
        rect(&s, p, "title_double_click"),
        Rect::new(186, 352, 120, 21)
    );
    assert_eq!(
        rect(&s, p, "dim_inactive_label"),
        Rect::new(32, 384, 230, 13)
    );
    assert_eq!(rect(&s, p, "dim_inactive").y, 380);
    assert_eq!(
        rect(&s, p, "dim_inactive_units"),
        Rect::new(322, 384, 20, 13)
    );
    inside_page(&s, p);
}

#[test]
fn the_windows_page_reads_back_tabs_shaking_the_picker_the_bar_the_distance_and_the_layouts() {
    let want = Settings {
        window_tabs: false,
        snap_picker: false,
        snap_bar: false,
        snap_layouts: "1|1 2-1:2".into(),
        snap_distance: 15,
        remember_windows: false,
        window_shake: true,
        title_double_click: "shade".into(),
        dim_inactive: 20,
        ..Settings::default()
    };
    let s = with(&want);
    let p = &s.pages[Page::Windows.index()];
    match &p.item("title_double_click").unwrap().control {
        Control::Combo(c) => assert_eq!(c.items[c.selected].text, "Roll up"),
        _ => panic!("title_double_click is a combo"),
    }
    assert_eq!(p.checked("window_tabs"), Some(false));
    assert_eq!(p.checked("window_shake"), Some(true));
    assert_eq!(p.checked("snap_picker"), Some(false));
    assert_eq!(p.checked("snap_bar"), Some(false));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_switcher_preview_greys_with_thumbnails() {
    let enabled = |s: &PropSheet| match &s.pages[Page::Switcher.index()]
        .item("switcher_preview")
        .unwrap()
        .control
    {
        Control::Check { enabled, .. } => *enabled,
        _ => panic!("a check box"),
    };
    let mut want = Settings {
        switcher_style: "thumbnails".into(),
        ..Settings::default()
    };
    let mut s = with(&want);
    assert!(!enabled(&s), "thumbnails grey the preview check box");
    assert_eq!(
        s.pages[Page::Switcher.index()].checked("switcher_preview"),
        Some(true)
    );
    want.switcher_style = "icons".into();
    follow(&mut s, &want);
    assert!(enabled(&s), "icons bring the preview check box back");
}
