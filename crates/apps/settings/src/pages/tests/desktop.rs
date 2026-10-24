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
    assert_eq!(rect(&s, p, "desktops_group"), Rect::new(20, 44, 358, 168));
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
    assert_eq!(rect(&s, p, "previews_group"), Rect::new(20, 222, 358, 76));
    assert_eq!(rect(&s, p, "preview_taskbar"), Rect::new(32, 246, 13, 13));
    assert_eq!(rect(&s, p, "previews_fps"), Rect::new(146, 268, 50, 20));
    assert_eq!(rect(&s, p, "fps_units"), Rect::new(202, 272, 100, 13));
    for id in ["session_restore", "session_end_processes"] {
        assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Startup");
    }
    inside_page(&s, p);
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
        "taskbar_tray_overflow",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(20, 108 + 22 * i as i32, 13, 13),
            "{id} is row {i}"
        );
    }
    assert_eq!(rect(&s, p, "taskbar_note"), Rect::new(20, 216, 358, 26));
    assert_eq!(rect(&s, p, "taskbar_auto_hide"), Rect::new(20, 250, 13, 13));
    for (i, id) in REVEAL_CONTROLS.into_iter().enumerate() {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(38, 272 + 22 * i as i32, 13, 13),
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
        Rect::new(20, 44, 358, 86)
    );
    assert_eq!(
        rect(&s, p, "launcher_search_note"),
        Rect::new(32, 66, 334, 26)
    );
    assert_eq!(rect(&s, p, "launcher_files"), Rect::new(32, 104, 13, 13));
    inside_page(&s, p);
}

#[test]
fn launcher_values_go_in_and_come_back_out() {
    let want = Settings {
        launcher_files: false,
        ..Settings::default()
    };
    let s = with(&want);
    assert_eq!(
        s.pages[Page::Launcher.index()].checked("launcher_files"),
        Some(false)
    );
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
    assert_eq!(rect(&s, p, "tabs_group"), Rect::new(20, 228, 358, 96));
    assert_eq!(rect(&s, p, "window_tabs"), Rect::new(32, 250, 13, 13));
    assert_eq!(rect(&s, p, "remember_windows"), Rect::new(32, 274, 13, 13));
    assert_eq!(rect(&s, p, "window_shake"), Rect::new(32, 298, 13, 13));
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
        ..Settings::default()
    };
    let s = with(&want);
    let p = &s.pages[Page::Windows.index()];
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
