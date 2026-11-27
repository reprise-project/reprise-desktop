use super::super::desktop::{
    CONTENTS, CONTENTS_NOTE, GLOW_CONTROLS, SNAP_PREVIEW, show_snap_preview, snap_preview,
};
use super::*;

#[test]
fn the_start_menu_page_sits_inside_its_page() {
    inside_page(&built(), Page::StartMenu);
}

#[test]
fn start_menu_holds_its_style_banner_and_quick_launch_check() {
    for id in ["startmenu_style", "taskbar_pinned"] {
        assert_moved(id, Page::Taskbar, Page::StartMenu);
    }
    let s = built();
    assert!(
        s.pages[Page::StartMenu.index()]
            .item("startmenu_banner")
            .is_some()
    );
    assert!(
        s.pages[Page::Taskbar.index()]
            .item("startmenu_banner")
            .is_none()
    );
    assert_eq!(
        keys_on(&s, Page::StartMenu),
        [".startmenu_banner", ".startmenu_style", "taskbar.pinned"]
    );
    let p = Page::StartMenu;
    assert_eq!(rect(&s, p, "startmenu_style"), Rect::new(134, 44, 244, 21));
    assert_eq!(rect(&s, p, "startmenu_banner"), Rect::new(134, 70, 244, 20));
    assert_eq!(
        crate::menupage::tree_rect(Area::MIN),
        Rect::new(20, 100, 275, 168)
    );
    assert_eq!(rect(&s, p, "ql_group"), Rect::new(20, 284, 358, 156));
    assert_eq!(rect(&s, p, "taskbar_pinned"), Rect::new(32, 304, 13, 13));
    assert_eq!(
        crate::quickpage::list_rect(Area::MIN),
        Rect::new(32, 326, 251, 106)
    );
    inside_page(&s, p);
}

#[test]
fn the_blue_and_glass_styles_show_their_menus_options_in_the_trees_place() {
    use crate::menupage::{CLEAR_RECENT, PLACES, POWER, PROGRAMS, TREE};
    let s = under("glass");
    let p = Page::StartMenu;
    let page = &s.pages[p.index()];
    assert!(page.item(TREE).is_none() && page.item("startmenu_banner").is_none());
    assert_eq!(
        keys_on(&s, p),
        [
            ".startmenu_style",
            ".startmenu_places",
            ".startmenu_power",
            ".startmenu_programs",
            "taskbar.pinned"
        ]
    );
    assert_eq!(rect(&s, p, "sm_places"), Rect::new(20, 70, 358, 104));
    let tops: Vec<(i32, i32)> = PLACES
        .iter()
        .map(|(id, _)| (rect(&s, p, id).x, rect(&s, p, id).y))
        .collect();
    assert_eq!(
        tops,
        [
            (32, 90),
            (32, 110),
            (32, 130),
            (32, 150),
            (199, 90),
            (199, 110),
            (199, 130)
        ]
    );
    assert_eq!(rect(&s, p, POWER), Rect::new(134, 186, 120, 21));
    assert_eq!(rect(&s, p, PROGRAMS).y, 212);
    assert_eq!(rect(&s, p, CLEAR_RECENT), Rect::new(20, 242, 125, 23));
    assert!(rect(&s, p, CLEAR_RECENT).bottom() < rect(&s, p, "ql_group").y);
    inside_page(&s, p);
}

#[test]
fn the_start_menu_page_swaps_with_the_style_and_keeps_quick_launch() {
    use crate::menupage::{PLACES, POWER, TREE};
    let mut s = built();
    let p = Page::StartMenu;
    let quick = vec![crate::quickpage::Launcher {
        file: "a.desktop".into(),
        name: "Alpha".into(),
        icon: None,
    }];
    crate::quickpage::fill(&mut s.pages[p.index()], &quick, Some(0));
    let mut want = Settings::default();
    pick(&mut s, &mut want, p, "startmenu_style", 0);
    assert_eq!(want.startmenu_style, "glass");
    assert!(s.pages[p.index()].item(TREE).is_none());
    assert!(enabled_on(&s, p, POWER), "Glass has a power button");
    assert!(
        enabled_on(&s, p, crate::quickpage::REMOVE),
        "the pick stays"
    );
    tick(&mut s, &mut want, p, PLACES[1].0, false);
    assert!(!want.startmenu_places.contains(&"pictures".to_string()));
    pick(&mut s, &mut want, p, POWER, 2);
    assert_eq!(want.startmenu_power, "lock");
    pick(&mut s, &mut want, p, "startmenu_style", 1);
    assert!(!enabled_on(&s, p, POWER), "Blue has no power button");
    pick(&mut s, &mut want, p, "startmenu_style", 2);
    assert!(s.pages[p.index()].item(TREE).is_some());
    assert!(crate::menupage::selected(&s.pages[p.index()]).is_none());
    assert!(enabled_on(&s, p, crate::quickpage::REMOVE));
}

#[test]
fn the_switcher_page_geometry() {
    let s = built();
    let p = Page::Switcher;
    assert_eq!(rect(&s, p, "switcher_group"), Rect::new(20, 44, 358, 186));
    assert_eq!(rect(&s, p, "switcher_style"), Rect::new(146, 64, 130, 21));
    assert_eq!(rect(&s, p, "switcher_size"), Rect::new(146, 90, 130, 21));
    assert_eq!(rect(&s, p, "switcher_preview"), Rect::new(32, 118, 13, 13));
    assert_eq!(rect(&s, p, "switcher_desktop"), Rect::new(32, 140, 13, 13));
    assert_eq!(rect(&s, p, "switcher_peek"), Rect::new(32, 162, 13, 13));
    assert_eq!(rect(&s, p, "switcher_flip"), Rect::new(32, 184, 13, 13));
    assert_eq!(rect(&s, p, FLIP_NOTE), Rect::new(GX + 18, 202, 316, 13));
    assert_eq!(rect(&s, p, "overview_group"), Rect::new(20, 240, 358, 76));
    assert_eq!(rect(&s, p, "hot_corner"), Rect::new(146, 284, 130, 21));
    inside_page(&s, p);
}

#[test]
fn search_and_switcher_notes_name_their_keys() {
    let d = Settings::default().shortcuts;
    assert_eq!(
        search_note(&d),
        [
            "Super+Space opens the launcher, which finds installed",
            "programs as you type their names. The Start menu's",
            "search box follows these too.",
        ]
    );
    assert_eq!(
        search_note(&[]),
        [
            "The launcher finds installed programs as you type their",
            "names. The Start menu's search box follows these too.",
        ]
    );
    assert_eq!(flip_note(&d), "Super+Tab shows them; Glass only.");
    assert_eq!(flip_note(&[]), "No key shows them; Glass only.");
    assert_eq!(
        overview_note(&d),
        "Super+W shows this desktop's windows side by side."
    );
    assert_eq!(
        overview_note(&[]),
        "Every window of this desktop side by side."
    );
}

#[test]
fn the_key_notes_follow_rebinding() {
    let mut binds = Settings::default().shortcuts;
    for b in binds.iter_mut().filter(|b| b.action == "launcher") {
        b.keys = "Ctrl+Alt+space".into();
    }
    let mut s = built();
    show_keys(&mut s, &binds);
    assert_eq!(
        lines(&s, Page::Search, SEARCH_NOTE),
        [
            "Ctrl+Alt+Space opens the launcher, which finds",
            "installed programs as you type their names. The Start",
            "menu's search box follows these too.",
        ]
    );
    assert_eq!(
        lines(&s, Page::Switcher, FLIP_NOTE),
        ["Super+Tab shows them; Glass only."]
    );
    assert_eq!(
        lines(&s, Page::Switcher, OVERVIEW_NOTE),
        ["Super+W shows this desktop's windows side by side."]
    );
    assert_eq!(
        lines(&s, Page::Privacy, "clipboard_history"),
        ["Keep a &clipboard history (Super+V opens it)"]
    );
    show_keys(&mut s, &[]);
    assert_eq!(
        lines(&s, Page::Privacy, "clipboard_history"),
        ["Keep a &clipboard history"]
    );
    assert_eq!(
        lines(&s, Page::Search, SEARCH_NOTE),
        [
            "The launcher finds installed programs as you type their",
            "names. The Start menu's search box follows these too.",
        ]
    );
    assert_eq!(
        lines(&s, Page::Switcher, FLIP_NOTE),
        ["No key shows them; Glass only."]
    );
    assert_eq!(
        lines(&s, Page::Switcher, OVERVIEW_NOTE),
        ["Every window of this desktop side by side."]
    );
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
fn the_virtual_desktops_page_geometry() {
    let s = built();
    let p = Page::VirtualDesktops;
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
    assert_eq!(
        rect(&s, p, "desktop_animation_note"),
        Rect::new(20, 296, 358, 13)
    );
    inside_page(&s, p);
}

#[test]
fn the_wheel_on_desktop_box_writes_its_key_and_greys_with_one_desktop() {
    let p = Page::VirtualDesktops;
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
    let p = Page::VirtualDesktops;
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
        let mut f = virtual_desktops(Area::MIN);
        if let Some(Control::Edit { edit, .. }) = f.item_mut(DESKTOP_NAMES).map(|i| &mut i.control)
        {
            *edit = reprisekit::textedit::TextEdit::new(names.join(", "), false);
        }
        let h = row_pins(&f, row);
        pins.push(h);
    }
    let mut f = virtual_desktops(Area::MIN);
    for id in ["desktop_names_label", DESKTOP_NAMES, "desktop_names_note"] {
        f.set_enabled(id, false);
    }
    pins.push(row_pins(&f, row));
    check_rows("desktop_names_rows", &["named", "empty", "greyed"], &pins);
}

#[test]
fn the_wheel_on_desktop_row_pins_its_pixels_in_every_look() {
    let mut f = virtual_desktops(Area::MIN);
    let row = Rect::new(50, 212, 328, 17);
    let off = row_pins(&f, row);
    if let Some(Control::Check { checked, .. }) =
        f.item_mut("wheel_on_desktop").map(|i| &mut i.control)
    {
        *checked = true;
    }
    let on = row_pins(&f, row);
    f.set_enabled("wheel_on_desktop", false);
    let greyed = row_pins(&f, row);
    check_rows(
        "wheel_on_desktop_row",
        &["off", "ticked", "greyed"],
        &[off, on, greyed],
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
    let p = Page::VirtualDesktops;
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
fn the_duration_follows_the_animation() {
    let p = Page::VirtualDesktops;
    let mut s = built();
    assert!(enabled_on(&s, p, "desktop_animation"));
    assert!(!enabled_on(&s, p, "desktop_animation_ms"), "no animation");
    assert!(!enabled_on(&s, p, "duration_units"));
    let fade = Settings {
        desktop_animation: "fade".into(),
        ..Settings::default()
    };
    follow(&mut s, &fade);
    assert!(enabled_on(&s, p, "desktop_animation_ms"));
}

#[test]
fn the_refresh_rate_follows_what_shows_a_live_picture() {
    let p = Page::Taskbar;
    let still = Settings {
        switcher_preview: false,
        preview_taskbar: false,
        ..Settings::default()
    };
    let mut s = with(&still);
    for id in FPS_CONTROLS {
        assert!(!enabled_on(&s, p, id), "{id} greys with no live picture");
    }
    let thumbs = Settings {
        switcher_style: "thumbnails".into(),
        ..still.clone()
    };
    follow(&mut s, &thumbs);
    assert!(enabled_on(&s, p, "previews_fps"), "thumbnails are live");
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
        shown(Page::VirtualDesktops, "desktop_animation"),
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
    let p = Page::VirtualDesktops;
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
    let p = Page::WindowBehavior;
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
fn a_key_note_shows_an_ampersand_key_as_typed() {
    let binds = ["flip", "overview", "next-layout", "clipboard-history"]
        .map(|action| Shortcut::new("Super+ampersand", action, ""));
    let mut s = built();
    show_keys(&mut s, &binds);
    assert_eq!(
        lines(&s, Page::Switcher, FLIP_NOTE),
        ["Super+&& shows them; Glass only."]
    );
    assert_eq!(
        lines(&s, Page::Switcher, OVERVIEW_NOTE),
        ["Super+&& shows this desktop's windows side by side."]
    );
    assert_eq!(
        lines(&s, Page::Keyboard, KB_SWITCH_NOTE),
        ["Super+&& switches them."]
    );
    assert_eq!(
        lines(&s, Page::Privacy, "clipboard_history"),
        ["Keep a &clipboard history (Super+&& opens it)"]
    );
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
    assert!(
        enabled_on(&s, p, FLIP_NOTE),
        "its note says why, so it stays lit"
    );
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
    match &s.pages[Page::VirtualDesktops.index()]
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
    assert_eq!(
        rect(&s, p, "taskbar_height_label"),
        Rect::new(254, 48, 44, 13)
    );
    assert_eq!(rect(&s, p, "taskbar_height"), Rect::new(302, 44, 50, 20));
    assert_eq!(
        rect(&s, p, "taskbar_height_units"),
        Rect::new(358, 48, 20, 13)
    );
    assert_eq!(rect(&s, p, "buttons_label"), Rect::new(20, 76, 100, 13));
    assert_eq!(rect(&s, p, "taskbar_buttons"), Rect::new(124, 72, 230, 21));
    for (i, id) in [
        "taskbar_small_icons",
        "taskbar_show_desktop",
        "taskbar_peek",
        "taskbar_badges",
        "taskbar_per_monitor",
        "taskbar_tray_overflow",
        "taskbar_network_meter",
        "taskbar_slide",
        "taskbar_calendar_events",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(20, 118 + 18 * i as i32, 13, 13),
            "{id} is row {i}"
        );
    }
    assert_eq!(rect(&s, p, "taskbar_note"), Rect::new(20, 100, 358, 13));
    assert_eq!(rect(&s, p, TRAY_CUSTOMIZE), Rect::new(294, 203, 84, 23));
    assert_eq!(
        rect(&s, p, "taskbar_slide_note"),
        Rect::new(282, 244, 96, 13)
    );
    assert_eq!(
        rect(&s, p, "taskbar_events_note"),
        Rect::new(180, 262, 198, 13)
    );
    assert_eq!(rect(&s, p, "taskbar_auto_hide").y, 280);
    for (i, id) in REVEAL_CONTROLS.into_iter().enumerate() {
        assert_eq!(
            rect(&s, p, id),
            Rect::new(38, 298 + 18 * i as i32, 13, 13),
            "{id} sits under the auto-hide label"
        );
    }
    assert_eq!(rect(&s, p, "preview_taskbar"), Rect::new(20, 352, 13, 13));
    assert_eq!(rect(&s, p, "fps_label"), Rect::new(38, 372, 110, 13));
    assert_eq!(rect(&s, p, "previews_fps"), Rect::new(152, 368, 50, 20));
    assert_eq!(rect(&s, p, "fps_units"), Rect::new(208, 372, 170, 13));
    assert_eq!(rect(&s, p, "start_icon"), Rect::new(20, 394, 13, 13));
    assert_eq!(rect(&s, p, "orb_glow_label"), Rect::new(20, 418, 110, 13));
    assert_eq!(rect(&s, p, "glass_orb_glow"), Rect::new(134, 412, 150, 26));
    assert_eq!(rect(&s, p, "orb_glow_note"), Rect::new(292, 418, 86, 13));
    inside_page(&s, p);
}

#[test]
fn taskbar_holds_its_height_slide_pictures_and_glow() {
    assert_moved("taskbar_height", Page::FontsAndIcons, Page::Taskbar);
    assert_moved("taskbar_slide", Page::Effects, Page::Taskbar);
    assert_moved("preview_taskbar", Page::VirtualDesktops, Page::Taskbar);
    assert_moved("glass_orb_glow", Page::Appearance, Page::Taskbar);
    for i in ["classic", "glass"] {
        let k = keys_on(&under(i), Page::Taskbar);
        for want in [
            "metrics.taskbar_height",
            "effects.taskbar_slide",
            "previews.taskbar",
            "previews.fps",
            "glass.orb_glow",
        ] {
            assert!(k.contains(&want.to_string()), "{i}: {want}");
        }
    }
    assert!(Page::Taskbar.keywords().contains(&"previews"));
}

#[test]
fn glass_only_taskbar_controls_grey_with_a_note_elsewhere() {
    let classic = under("classic");
    for id in GLOW_CONTROLS {
        assert!(!enabled_on(&classic, Page::Taskbar, id), "{id}");
    }
    assert_eq!(
        lines(&classic, Page::Taskbar, "orb_glow_note"),
        ["(Glass only)"]
    );
    assert!(
        enabled_on(&classic, Page::Taskbar, "orb_glow_note"),
        "the note says why, so it stays lit"
    );
    assert!(!enabled_on(&classic, Page::Taskbar, "taskbar_small_icons"));
    assert_eq!(
        lines(&classic, Page::Taskbar, "taskbar_note"),
        ["Left unchanged, the settings on this page follow the interface."]
    );
    let glass = under("glass");
    assert!(enabled_on(&glass, Page::Taskbar, "glass_orb_glow"));
    assert!(enabled_on(&glass, Page::Taskbar, "taskbar_small_icons"));
}

#[test]
fn the_taskbar_customize_stays_beside_its_row_on_a_wide_page_and_the_note_takes_one_line() {
    reprisekit::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let wide = super::super::desktop::taskbar(Area { w: 1100, dh: 0 });
    let at = |id: &str| wide.item(id).unwrap().rect;
    assert_eq!(at(TRAY_CUSTOMIZE), Rect::new(294, 203, 84, 23));
    assert_eq!(
        at("taskbar_tray_overflow").y + 18,
        at("taskbar_network_meter").y
    );
    let line = "Left unchanged, the settings on this page follow the interface.";
    assert!(text.measure(line, false, px) <= Area::MIN.w);
}

#[test]
fn taskbar_values_go_in_and_come_back_out() {
    let want = Settings {
        taskbar_position: "top".into(),
        taskbar_buttons: "combine".into(),
        taskbar_height: 40,
        taskbar_slide: false,
        preview_taskbar: false,
        previews_fps: 12,
        glass_orb_glow: 30,
        taskbar_peek: true,
        taskbar_tray_overflow: true,
        taskbar_badges: false,
        taskbar_per_monitor: true,
        taskbar_network_meter: true,
        taskbar_calendar_events: false,
        recent_files: false,
        start_icon: false,
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
    assert_eq!(tb.checked("taskbar_per_monitor"), Some(true));
    assert_eq!(tb.checked("taskbar_network_meter"), Some(true));
    assert_eq!(tb.checked("taskbar_calendar_events"), Some(false));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_taskbar_slide_check_goes_in_and_comes_back_out() {
    let mut got = Settings::for_interface("glass");
    let mut s = with(&got);
    let p = Page::Taskbar;
    assert_eq!(
        s.pages[p.index()].checked("taskbar_slide"),
        Some(true),
        "Glass slides its task buttons"
    );
    tick(&mut s, &mut got, p, "taskbar_slide", false);
    assert!(!got.taskbar_slide);
    assert_eq!(
        with(&Settings::for_interface("classic")).pages[p.index()].checked("taskbar_slide"),
        Some(false)
    );
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
fn the_search_page_geometry() {
    let s = built();
    let p = Page::Search;
    assert_eq!(
        rect(&s, p, "launcher_search_group"),
        Rect::new(20, 44, 358, 174)
    );
    assert_eq!(rect(&s, p, SEARCH_NOTE), Rect::new(32, 66, 334, 39));
    assert_eq!(rect(&s, p, "launcher_files"), Rect::new(32, 117, 13, 13));
    assert_eq!(rect(&s, p, "launcher_windows"), Rect::new(32, 139, 13, 13));
    assert_eq!(rect(&s, p, CONTENTS), Rect::new(32, 161, 13, 13));
    assert_eq!(rect(&s, p, CONTENTS_NOTE), Rect::new(50, 179, 316, 26));
    assert_eq!(
        rect(&s, p, "search_providers_group"),
        Rect::new(20, 228, 358, 168)
    );
    assert_eq!(
        rect(&s, p, "search_providers_note"),
        Rect::new(32, 250, 334, 26)
    );
    assert_eq!(
        rect(&s, p, crate::providerpage::LIST),
        Rect::new(32, 280, 253, 106)
    );
    assert_eq!(
        rect(&s, p, crate::providerpage::UP),
        Rect::new(291, 280, 75, 23)
    );
    assert_eq!(
        rect(&s, p, crate::providerpage::DOWN),
        Rect::new(291, 307, 75, 23)
    );
    inside_page(&s, p);
}

/// Every line of the Search page fits its rect in the dialog font, the list's empty note too.
#[test]
fn the_search_page_lines_fit() {
    reprisekit::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut page = search(Area::MIN);
    crate::providerpage::fill(&mut page, Some(&[]), None);
    let list_w = page.item(crate::providerpage::LIST).unwrap().rect.width;
    for it in &page.items {
        let (lines, width) = match &it.control {
            Control::Lines(l) => (l.clone(), it.rect.width),
            Control::Label { text, .. } => (vec![text.clone()], it.rect.width),
            Control::ListBox { list, .. } => (list.items.clone(), list_w - 8),
            _ => continue,
        };
        for line in lines {
            assert!(text.measure(&line, false, px) <= width, "{}: {line}", it.id);
        }
    }
}

#[test]
fn search_values_go_in_and_come_back_out() {
    assert!(!Settings::default().launcher_contents, "off by default");
    let want = Settings {
        launcher_files: false,
        launcher_contents: true,
        launcher_windows: false,
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::Search.index()];
    assert_eq!(page.checked("launcher_files"), Some(false));
    assert_eq!(page.checked(CONTENTS), Some(true));
    assert_eq!(page.checked("launcher_windows"), Some(false));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn the_hidden_frame_rate_row_pins_its_pixels_in_every_look() {
    let row = Rect::new(20, 404, 358, 34);
    let mut f = window_behavior(Area::MIN);
    let unlimited = row_pins(&f, row);
    if let Some(Control::UpDown(u)) = f.item_mut("hidden_frame_rate").map(|i| &mut i.control) {
        u.set(30);
    }
    let capped = row_pins(&f, row);
    check_rows("hidden_frame_rate_row", &["0", "30"], &[unlimited, capped]);
}

#[test]
fn the_layouts_preview_draws_what_the_field_names_and_nothing_while_it_does_not_parse() {
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Classic);
    reprisekit::pins::fonts();
    let size = |text: &str, scale: i32| {
        snap_preview(&theme, text, scale).map(|c| (c.width, c.height, c.device_size()))
    };
    assert_eq!(
        size("", 1),
        Some((204, 25, (204, 25))),
        "the picker's own six"
    );
    assert_eq!(size("1|1 2-1:2", 1), Some((68, 25, (68, 25))));
    assert_eq!(size("1|1 2-1:2", 2), Some((68, 25, (136, 50))));
    assert_eq!(size("1|1 2|x", 1), None);
    assert_eq!(size("1 1 1 1 1 1 1", 1), None, "seven layouts");
    use reprisekit::interface::Interface;
    let hash = |c: &Canvas| {
        (c.pixels.iter()).fold(0xcbf29ce484222325, |h, p| {
            (h ^ *p as u64).wrapping_mul(0x100000001b3)
        })
    };
    let pins: Vec<(String, u64)> = [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .flat_map(|i| {
            let theme = reprisekit::interfaces::theme_for(i);
            [1, 2].map(|s| {
                let c = snap_preview(&theme.scaled(s), "1|1:2 1-1 2|1|1", s).unwrap();
                (format!("{i:?} x{s}"), hash(&c))
            })
        })
        .collect();
    pin("snap_layouts_preview", &pins);
}

#[test]
fn a_layout_that_does_not_parse_empties_the_preview_and_the_note_says_which_and_why() {
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Classic);
    reprisekit::pins::fonts();
    let mut f = window_behavior(Area::MIN);
    let note = |f: &Form| {
        let it = f.item("snap_layouts_note").unwrap();
        match &it.control {
            Control::Lines(l) => (it.rect, l.clone()),
            c => panic!("the note is lines, got {c:?}"),
        }
    };
    let preview = |f: &Form| match &f.item(SNAP_PREVIEW).unwrap().control {
        Control::Picture { canvas, .. } => canvas.is_some(),
        c => panic!("the preview is a picture, got {c:?}"),
    };
    let (hint_rect, hint) = note(&f);
    show_snap_preview(&mut f, &theme, "1|1 2|x", 1);
    assert!(!preview(&f), "nothing drawn while an entry does not parse");
    let (rect, lines) = note(&f);
    assert_eq!(
        lines,
        [r#"Layout 2 (2|x): "x" is not a size. A band's size is a number above 0."#]
    );
    let shown = f.item(SNAP_PREVIEW).unwrap().rect;
    assert_eq!(
        (rect.x, rect.right()),
        (shown.x, hint_rect.right()),
        "the reason runs from the preview's left to the note's right"
    );
    show_snap_preview(&mut f, &theme, "1|1 2|1", 1);
    assert!(preview(&f));
    assert_eq!(
        note(&f),
        (hint_rect, hint),
        "a fixed entry brings the hint back"
    );
}

#[test]
fn the_window_behavior_page_geometry() {
    let s = built();
    let p = Page::WindowBehavior;
    assert_eq!(rect(&s, p, "snap_group"), Rect::new(20, 44, 358, 172));
    assert_eq!(rect(&s, p, "snap_enabled"), Rect::new(32, 66, 13, 13));
    assert_eq!(rect(&s, p, "snap_picker"), Rect::new(32, 84, 13, 13));
    assert_eq!(rect(&s, p, "snap_bar"), Rect::new(32, 102, 13, 13));
    assert_eq!(
        rect(&s, p, "snap_distance_label"),
        Rect::new(32, 126, 150, 13)
    );
    assert_eq!(rect(&s, p, "snap_distance").y, 122);
    assert_eq!(
        rect(&s, p, "snap_distance_units"),
        Rect::new(242, 126, 30, 13)
    );
    assert_eq!(rect(&s, p, "snap_layouts"), Rect::new(120, 150, 246, 20));
    assert_eq!(rect(&s, p, SNAP_PREVIEW), Rect::new(32, 178, 204, 25));
    assert_eq!(
        rect(&s, p, "snap_layouts_note"),
        Rect::new(244, 178, 122, 26)
    );
    assert_eq!(rect(&s, p, "tabs_group"), Rect::new(20, 228, 358, 210));
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
    assert_eq!(
        rect(&s, p, "hidden_frame_rate_label"),
        Rect::new(32, 412, 170, 13)
    );
    assert_eq!(rect(&s, p, "hidden_frame_rate").y, 408);
    assert_eq!(
        rect(&s, p, "hidden_frame_rate_units"),
        Rect::new(262, 412, 104, 13)
    );
    inside_page(&s, p);
}

#[test]
fn the_window_behavior_page_reads_back_tabs_shaking_the_picker_the_bar_the_distance_the_layouts_and_the_hidden_rate()
 {
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
        hidden_frame_rate: 30,
        ..Settings::default()
    };
    let s = with(&want);
    let p = &s.pages[Page::WindowBehavior.index()];
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

#[test]
fn the_program_results_list_pins_its_pixels_in_every_look() {
    use crate::providerpage::{LIST, Row, fill};
    let row = |name: &str, on, held| Row {
        id: name.to_lowercase(),
        name: name.into(),
        icon: None,
        on,
        held,
    };
    let rows = [
        row("Calculator", true, false),
        row("Characters", false, false),
        row("Files", true, true),
    ];
    let mut page = search(Area::MIN);
    let list = page.item(LIST).unwrap().rect;
    let mut pins = Vec::new();
    for shown in [Some(&rows[..]), None, Some(&[][..])] {
        fill(&mut page, shown, Some(0));
        pins.push(row_pins(&page, list));
    }
    check_rows(
        "search_programs_list",
        &["ticked, unticked and held", "Loading...", "no program"],
        &pins,
    );
}

/// The Window Rules page empty and listing two rules with the second picked, and the rule dialog
/// adding one and editing one with the open windows offered, at scale 1 and 2.
#[test]
fn the_window_rules_page_and_dialog_pin_their_pixels_in_every_look_at_both_scales() {
    use crate::ruledlg::{CLIENT, RuleDialog};
    use crate::rulespage;
    use crate::windowrules::Rule;
    let rules = [
        Rule {
            app_id: "foot".into(),
            size: Some([800, 600]),
            state: Some("maximized".into()),
            ..Rule::default()
        },
        Rule {
            app_id: "reprise.notepad".into(),
            title: Some("*.txt*".into()),
            desktop: Some(2),
            always_on_top: Some(true),
            ..Rule::default()
        },
    ];
    let both = |f: &Form, r: Rect| [row_pins_at(f, r, 1), row_pins_at(f, r, 2)];
    let page_rect = Rect::new(20, 40, 358, 392);
    let mut f = rulespage::page(Area::MIN);
    rulespage::fill(&mut f, &[], None);
    let mut rows = both(&f, page_rect).to_vec();
    rulespage::fill(&mut f, &rules, Some(1));
    rows.extend(both(&f, page_rect));
    let dialog = Rect::new(0, 0, CLIENT.0, CLIENT.1);
    rows.extend(both(&RuleDialog::new(None, Vec::new()).form, dialog));
    let mut d = RuleDialog::new(Some(&rules[1]), vec!["DP-1".into(), "HDMI-A-1".into()]);
    let w = |id: &str, title: &str| (id.to_string(), title.to_string());
    d.offer_windows(vec![
        w("foot", "~"),
        w("reprise.notepad", "Untitled - Notepad"),
    ]);
    rows.extend(both(&d.form, dialog));
    let x1: Vec<_> = rows.iter().step_by(2).collect();
    for (i, r) in x1.iter().enumerate() {
        assert!(
            !x1[..i].contains(r),
            "state {i} draws apart from the ones before"
        );
    }
    check_rows(
        "window_rules",
        &[
            "page empty x1",
            "page empty x2",
            "page listing x1",
            "page listing x2",
            "add dialog x1",
            "add dialog x2",
            "edit dialog x1",
            "edit dialog x2",
        ],
        &rows,
    );
}
