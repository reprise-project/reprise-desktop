use super::*;

#[test]
fn the_text_and_keys_page_geometry() {
    let s = built();
    let p = Page::TextKeys;
    assert_eq!(rect(&s, p, "text_size"), Rect::new(94, 44, 80, 21));
    assert_eq!(rect(&s, p, "sticky_group"), Rect::new(20, 70, 358, 103));
    assert_eq!(rect(&s, p, "sticky_keys"), Rect::new(32, 109, 13, 13));
    assert_eq!(rect(&s, p, "sticky_lock"), Rect::new(32, 131, 13, 13));
    assert_eq!(
        rect(&s, p, "sticky_two_keys_off"),
        Rect::new(32, 153, 13, 13)
    );
    assert_eq!(rect(&s, p, "slow_group"), Rect::new(20, 179, 358, 115));
    assert_eq!(rect(&s, p, "slow_keys"), Rect::new(32, 218, 13, 13));
    assert_eq!(rect(&s, p, "slow_delay_value"), Rect::new(286, 240, 80, 13));
    assert_eq!(
        rect(&s, p, "slow_keys_delay"),
        Rect::new(78, 261, 238, trackbar::HEIGHT)
    );
    assert_eq!(rect(&s, p, "mouse_keys_group"), Rect::new(20, 300, 358, 98));
    assert_eq!(rect(&s, p, "mouse_keys"), Rect::new(32, 318, 13, 13));
    assert_eq!(
        rect(&s, p, "mouse_keys_num_lock"),
        Rect::new(199, 318, 13, 13)
    );
    assert_eq!(
        rect(&s, p, "mouse_keys_speed"),
        Rect::new(148, 338, 184, trackbar::HEIGHT)
    );
    assert_eq!(
        rect(&s, p, "mouse_keys_acceleration"),
        Rect::new(148, 366, 184, trackbar::HEIGHT)
    );
    assert_eq!(rect(&s, p, "key_shortcuts"), Rect::new(20, 404, 13, 13));
    assert_eq!(rect(&s, p, "shortcuts_note"), Rect::new(38, 422, 340, 26));
    inside_page(&s, p);
}

#[test]
fn text_and_keys_values_go_in_and_come_back_out() {
    let want = Settings {
        sticky_keys: true,
        sticky_lock: false,
        sticky_two_keys_off: true,
        slow_keys: false,
        slow_keys_delay: 15,
        key_shortcuts: false,
        mouse_keys: false,
        mouse_keys_num_lock: false,
        mouse_keys_speed: 700,
        mouse_keys_acceleration: 20,
        text_size: 175,
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::TextKeys.index()];
    assert_eq!(page.track_pos("slow_keys_delay"), Some(12));
    assert_eq!(page.track_pos("mouse_keys_speed"), Some(6));
    assert_eq!(page.track_pos("mouse_keys_acceleration"), Some(6));
    assert_eq!(page.combo_selected("text_size"), Some(3));
    assert_eq!(page.checked("mouse_keys_num_lock"), Some(false));
    assert_eq!(page.checked("sticky_lock"), Some(false));
    assert_eq!(page.checked("key_shortcuts"), Some(false));
    match &page.item("slow_delay_value").unwrap().control {
        Control::Label { text, .. } => assert_eq!(text, "1.5 seconds"),
        _ => panic!("a label"),
    }
    let enabled = |id: &str| match &page.item(id).unwrap().control {
        Control::Check { enabled, .. } | Control::Label { enabled, .. } => *enabled,
        Control::TrackBar(t) => t.enabled,
        _ => panic!("{id} greys"),
    };
    assert!(enabled("sticky_lock") && enabled("sticky_two_keys_off"));
    assert!(!enabled("slow_keys_delay") && !enabled("slow_delay_label"));
    assert!(!enabled("mouse_keys_speed") && !enabled("mouse_keys_num_lock"));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn double_click_speed_reads_slow_to_fast() {
    let mut s = built();
    let p = Page::MouseTouchpad;
    assert_eq!(lines(&s, p, "double_click_slow"), ["Slow"]);
    assert_eq!(lines(&s, p, "double_click_fast"), ["Fast"]);
    assert_eq!(lines(&s, p, "double_click_value"), ["500 ms"]);
    assert_eq!(s.pages[p.index()].track_pos("double_click_ms"), Some(8));
    let mut got = Settings::default();
    s.pages[p.index()].set_track("double_click_ms", 14);
    read_one(&s, "double_click_ms", &mut got);
    assert_eq!(got.double_click_ms, 200);
    follow(&mut s, &got);
    assert_eq!(lines(&s, p, "double_click_value"), ["200 ms"]);
    s.pages[p.index()].set_track("double_click_ms", 0);
    read_one(&s, "double_click_ms", &mut got);
    assert_eq!(got.double_click_ms, 900);
}

#[test]
fn mousekeys_trackbars_have_slow_and_fast_ends() {
    let s = built();
    let p = Page::TextKeys;
    for id in ["mk_speed_slow", "mk_accel_slow"] {
        assert_eq!(lines(&s, p, id), ["Slow"]);
    }
    for id in ["mk_speed_fast", "mk_accel_fast"] {
        assert_eq!(lines(&s, p, id), ["Fast"]);
    }
    let off = under("classic");
    assert!(
        !enabled_on(&off, p, "mk_accel_fast"),
        "the ends grey with MouseKeys off"
    );
}

#[test]
fn the_magnifier_filters_page_geometry() {
    let s = built();
    let p = Page::MagnifierFilters;
    assert_eq!(rect(&s, p, "cf_group"), Rect::new(20, 44, 358, 110));
    assert_eq!(rect(&s, p, "color_filter"), Rect::new(106, 62, 150, 21));
    assert_eq!(rect(&s, p, "cf_preview"), Rect::new(266, 60, 100, 28));
    assert_eq!(
        rect(&s, p, "color_filter_intensity"),
        Rect::new(96, 96, 222, trackbar::HEIGHT)
    );
    assert_eq!(rect(&s, p, "cf_note"), Rect::new(32, 132, 334, 13));
    assert_eq!(rect(&s, p, "mag_group"), Rect::new(20, 164, 358, 93));
    assert_eq!(rect(&s, p, "magnifier_follow"), Rect::new(32, 184, 13, 13));
    assert_eq!(rect(&s, p, "mag_note"), Rect::new(32, 206, 334, 39));
    inside_page(&s, p);
    reprisekit::pins::fonts();
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let page = &s.pages[p.index()];
    let Control::Lines(lines) = &page.item("mag_note").unwrap().control else {
        panic!("the note is lines");
    };
    let check = "When zoomed in, follow the text cursor and focus";
    for line in lines.iter().map(String::as_str).chain([check]) {
        let w = text.measure(line, false, px);
        assert!(w <= 334 - 18, "{line:?} is {w} px, past the group");
    }
}

/// The filter combo stands for its name, the trackbar for the percent, the
/// magnifier's check box for "pointer+focus"; the intensity greys with no correction.
#[test]
fn color_filter_values_go_in_and_come_back_out() {
    let want = Settings {
        color_filter: "tritanopia".into(),
        color_filter_intensity: 40,
        magnifier_follow: "pointer".into(),
        ..Settings::default()
    };
    let mut s = with(&want);
    let page = &s.pages[Page::MagnifierFilters.index()];
    assert_eq!(page.checked("magnifier_follow"), Some(false));
    assert_eq!(page.combo_selected("color_filter"), Some(5));
    assert_eq!(page.track_pos("color_filter_intensity"), Some(40));
    match &page.item("cf_intensity_value").unwrap().control {
        Control::Label { text, enabled, .. } => {
            assert_eq!(text, "40%");
            assert!(*enabled);
        }
        _ => panic!("a label"),
    }
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
    let gray = Settings {
        color_filter: "grayscale".into(),
        ..want
    };
    follow(&mut s, &gray);
    match &s.pages[Page::MagnifierFilters.index()]
        .item("color_filter_intensity")
        .unwrap()
        .control
    {
        Control::TrackBar(t) => assert!(!t.enabled, "grayscale takes no intensity"),
        _ => panic!("a trackbar"),
    }
}

#[test]
fn the_color_filter_preview_draws_the_pending_filter() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let at = Page::MagnifierFilters.index();
    let mut want = Settings {
        color_filter: "grayscale".into(),
        ..Settings::default()
    };
    let mut s = with(&want);
    let r = s.on_page(crate::magnifierpage::preview_rect(Area::MIN));
    let pixel = |s: &PropSheet, x: i32, y: i32| {
        let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
        form::draw(
            &mut c,
            &text,
            &Theme::default().paint(),
            &s.pages[at],
            false,
            px,
        );
        c.get(r.x + x, r.y + y)
    };
    assert_eq!(pixel(&s, 0, 0), 0xff80_8080, "the frame's outer shadow");
    assert_eq!(pixel(&s, 2, 2), 0xffd0_3030, "red as it is");
    assert_eq!(pixel(&s, 2, 16), 0xff52_5252, "red in grayscale");
    assert_eq!(pixel(&s, 2 + 4 * 16, 16), 0xff5e_5e5e, "blue in grayscale");
    want.color_filter = "invert".into();
    follow(&mut s, &want);
    assert_eq!(pixel(&s, 2, 16), 0xff2f_cfcf, "red inverted");
    want.color_filter = "none".into();
    follow(&mut s, &want);
    assert_eq!(pixel(&s, 2, 16), 0xffd0_3030, "no filter: red again");
}

#[test]
fn the_keyboard_page_geometry() {
    let s = built();
    assert_eq!(
        rect(&s, Page::Keyboard, "keyboard_group"),
        Rect::new(20, 44, 358, 210)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, KB_LAYOUTS),
        Rect::new(116, 62, 169, 55)
    );
    assert_eq!(rect(&s, Page::Keyboard, KB_ADD), Rect::new(291, 62, 75, 23));
    assert_eq!(
        rect(&s, Page::Keyboard, KB_REMOVE),
        Rect::new(291, 89, 75, 23)
    );
    assert_eq!(rect(&s, Page::Keyboard, KB_UP), Rect::new(291, 116, 75, 23));
    assert_eq!(
        rect(&s, Page::Keyboard, KB_SWITCH_NOTE),
        Rect::new(116, 122, 169, 13)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "layout_per_window").y,
        146,
        "the check sits under the Layouts list"
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "kb_model"),
        Rect::new(116, 166, 250, 20)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "kb_options"),
        Rect::new(116, 192, 250, 20)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "repeat_rate"),
        Rect::new(116, 218, 50, 20)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "repeat_delay"),
        Rect::new(272, 218, 50, 20)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "rate_units"),
        Rect::new(172, 222, 50, 13)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "delay_units"),
        Rect::new(328, 222, 20, 13)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "shortcuts_group"),
        Rect::new(20, 262, 358, 138)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "shortcut_overlay"),
        Rect::new(32, 280, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "shortcut_overlay_ms"),
        Rect::new(250, 276, 50, 20)
    );
    assert_eq!(shortcuts(Area::MIN), Rect::new(32, 298, 334, 74));
    assert_eq!(
        rect(&s, Page::Keyboard, SC_ADD),
        Rect::new(129, 376, 75, 23)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, SC_EDIT),
        Rect::new(210, 376, 75, 23)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, SC_REMOVE),
        Rect::new(291, 376, 75, 23)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "osd_group"),
        Rect::new(20, 408, 358, 40)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "osd_caps_lock"),
        Rect::new(32, 426, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::Keyboard, "osd_num_lock"),
        Rect::new(199, 426, 13, 13)
    );
    inside_page(&s, Page::Keyboard);

    let lv = new_shortcut_list();
    let lay = listview::layout(
        &lv,
        shortcuts(Area::MIN),
        &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
    );
    assert_eq!(lay.rows, 3, "three rows of 17 px under the header");
    // The default list is longer than four rows, so the vertical bar is up; at 16 or 17 px the columns leave it room and no horizontal one appears.
    for (scroll, action) in [(16, 194), (17, 193)] {
        let cm = reprisekit::controls::metrics::ControlMetrics {
            scroll,
            ..reprisekit::controls::metrics::ControlMetrics::CLASSIC
        };
        let mut full = new_shortcut_list();
        fill_shortcuts(&mut full, &Settings::default(), &|a| a.into());
        fit_shortcut_columns(&mut full, &cm, shortcuts(Area::MIN));
        assert_eq!(full.columns[1].width, action);
        let lay = listview::layout(&full, shortcuts(Area::MIN), &cm);
        assert!(lay.scrollbar.is_some());
        assert!(lay.hscroll.is_none(), "no horizontal bar at {scroll}");
        assert_eq!(lay.rows, 3);
    }
}

#[test]
fn the_layouts_list_names_each_layout_and_its_buttons_follow_it() {
    let two = Settings {
        kb_layout: "us,hu".into(),
        ..Settings::default()
    };
    let mut s = with(&two);
    let rows = |s: &PropSheet| match &s.pages[Page::Keyboard.index()]
        .item(KB_LAYOUTS)
        .unwrap()
        .control
    {
        Control::ListBox { list, .. } => list.items.clone(),
        _ => panic!("a list box"),
    };
    let enabled = |s: &PropSheet, id: &str| match &s.pages[Page::Keyboard.index()]
        .item(id)
        .unwrap()
        .control
    {
        Control::Button { enabled, .. } => *enabled,
        _ => panic!("{id} is a button"),
    };
    assert_eq!(rows(&s), vec!["English (US)", "Hungarian"]);
    assert!(enabled(&s, KB_ADD));
    assert!(!enabled(&s, KB_REMOVE), "nothing picked");
    assert!(!enabled(&s, KB_UP));
    fill_layouts(&mut s, &two, &reg(), "us", Some(1));
    assert!(enabled(&s, KB_REMOVE));
    assert!(enabled(&s, KB_UP));
    s.pages[Page::Keyboard.index()].focus_id(KB_UP);
    fill_layouts(&mut s, &two, &reg(), "us", Some(0));
    assert!(!enabled(&s, KB_UP), "the first cannot move up");
    assert_eq!(
        s.pages[Page::Keyboard.index()].focus(),
        s.pages[Page::Keyboard.index()].index_of(KB_LAYOUTS),
        "the focus leaves the greyed button for the list"
    );
    let four = Settings {
        kb_layout: "us,hu,de,fr".into(),
        ..Settings::default()
    };
    fill_layouts(&mut s, &four, &reg(), "us", None);
    assert!(!enabled(&s, KB_ADD), "four is the most");
    assert_eq!(rows(&s)[2], "de", "an unknown code shows as itself");
}

#[test]
fn an_empty_layout_list_shows_the_layout_in_use_greyed() {
    assert_eq!(system_layout(None), "us");
    assert_eq!(system_layout(Some("de,us")), "de");
    let mut s = with(&Settings::default());
    let list = |s: &mut PropSheet| match s.pages[Page::Keyboard.index()]
        .item_mut(KB_LAYOUTS)
        .map(|i| &mut i.control)
    {
        Some(Control::ListBox { list, .. }) => list.clone(),
        _ => panic!("a list box"),
    };
    let l = list(&mut s);
    assert_eq!(
        (l.items.as_slice(), l.greyed.as_slice(), l.selected),
        (
            &["English (US) (system default)".to_string()][..],
            &[true][..],
            None
        )
    );
    if let Some(Control::ListBox { list, .. }) = s.pages[Page::Keyboard.index()]
        .item_mut(KB_LAYOUTS)
        .map(|i| &mut i.control)
    {
        list.selected = Some(0);
    }
    follow(&mut s, &Settings::default());
    assert!(
        !enabled_on(&s, Page::Keyboard, KB_REMOVE),
        "nothing to remove"
    );
    assert!(!enabled_on(&s, Page::Keyboard, KB_UP), "nothing to move");
    fill_layouts(&mut s, &Settings::default(), &reg(), "hu", None);
    assert_eq!(list(&mut s).items, ["Hungarian (system default)"]);
}

#[test]
fn the_keyboard_page_names_its_switch_keys_and_units() {
    let s = built();
    let p = Page::Keyboard;
    assert_eq!(
        lines(&s, p, KB_SWITCH_NOTE),
        ["Shift+Left Alt switches them."]
    );
    assert_eq!(lines(&s, p, "rate_units"), ["a second"]);
}

#[test]
fn the_switch_note_follows_the_next_layout_keys() {
    let mut s = built();
    let rebound = [
        Shortcut::new("Super+space", "next-layout", ""),
        Shortcut::new("Alt+Shift_R", "next-layout", ""),
    ];
    show_keys(&mut s, &rebound);
    assert_eq!(
        lines(&s, Page::Keyboard, KB_SWITCH_NOTE),
        ["Super+Space or Alt+Right Shift switches them."]
    );
    show_keys(&mut s, &[]);
    assert_eq!(
        lines(&s, Page::Keyboard, KB_SWITCH_NOTE),
        ["No key switches them."]
    );
}

#[test]
fn the_shortcut_list_shows_key_caps_and_the_actions_plain_name() {
    let mut lv = new_shortcut_list();
    let s = Settings {
        shortcuts: vec![
            Shortcut::new("Super+Return", "terminal", ""),
            Shortcut::new("Super+t", "spawn", "foot"),
            Shortcut::new("Shift+Alt_L", "next-layout", ""),
        ],
        ..Settings::default()
    };
    fill_shortcuts(&mut lv, &s, &|a| a.into());
    assert_eq!(lv.rows.len(), 3);
    assert_eq!(lv.rows[0].cells, vec!["Super+Enter", "Terminal"]);
    assert_eq!(lv.rows[1].cells, vec!["Super+T", "Run foot"]);
    assert_eq!(
        lv.rows[2].cells,
        vec!["Shift+Left Alt", "Next keyboard layout"]
    );
    assert!(lv.checks.is_none());
}

#[test]
fn program_rows_follow_the_binds() {
    let s = crate::model::load(
        "[[global_shortcuts]]\napp = \"t\"\nid = \"talk\"\ndescription = \"Hold to talk\"\nkeys = \"\"\n[portal]\nshortcuts_denied = [\"n\"]\n",
    );
    let mut l = new_shortcut_list();
    fill_shortcuts(&mut l, &s, &|a| {
        if a == "t" { "Taltos".into() } else { a.into() }
    });
    let n = s.shortcuts.len();
    assert_eq!(l.rows[n].cells, ["None", "Taltos: Hold to talk"]);
    assert_eq!(l.rows[n + 1].cells, ["", "n: not allowed"]);
}

/// The Motion group, twice: a "Select a pointer speed:" label, Slow and Fast
/// around the trackbar, and "Enhance pointer precision" under it (label 8 px
/// under the title, trackbar row 15 px under that, check 28 px under the
/// trackbar's top), and each group's "Reverse scrolling direction" check 22 px
/// under the last row.
#[test]
fn the_mouse_touchpad_page_geometry() {
    let s = built();
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_group"),
        Rect::new(20, 44, 358, 178)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "double_click_ms"),
        Rect::new(78, 89, 238, trackbar::HEIGHT)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "double_click_value"),
        Rect::new(286, 68, 80, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_speed_label"),
        Rect::new(32, 120, 334, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_slow"),
        Rect::new(32, 141, 26, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_speed"),
        Rect::new(60, 135, 160, trackbar::HEIGHT)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_fast"),
        Rect::new(224, 141, 26, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_accel"),
        Rect::new(32, 169, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "mouse_natural"),
        Rect::new(32, 191, 13, 13)
    );

    assert_eq!(
        rect(&s, Page::MouseTouchpad, "touchpad_group"),
        Rect::new(20, 232, 358, 218)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "touchpad_speed"),
        Rect::new(60, 271, 160, trackbar::HEIGHT)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "touchpad_accel"),
        Rect::new(32, 305, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "tap_to_click"),
        Rect::new(32, 331, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "two_finger_scroll"),
        Rect::new(32, 353, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "two_finger_tap_right_click"),
        Rect::new(32, 375, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "disable_while_typing"),
        Rect::new(32, 397, 13, 13)
    );
    assert_eq!(
        rect(&s, Page::MouseTouchpad, "touchpad_natural"),
        Rect::new(32, 419, 13, 13)
    );
    inside_page(&s, Page::MouseTouchpad);
}

#[test]
fn the_speed_trackbars_run_from_minus_one_to_one_in_tenths() {
    let want = Settings {
        mouse_speed: -10,
        touchpad_speed: 3,
        ..Settings::default()
    };
    let s = with(&want);
    let pos = |id: &str| match &s.pages[Page::MouseTouchpad.index()]
        .item(id)
        .unwrap()
        .control
    {
        Control::TrackBar(t) => (t.pos, t.count),
        _ => panic!("{id} is a trackbar"),
    };
    assert_eq!(pos("mouse_speed"), (0, 21));
    assert_eq!(pos("touchpad_speed"), (13, 21));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(got, want);
}

#[test]
fn two_finger_tap_follows_tap_to_click() {
    let mut s = with(&Settings::default());
    let enabled = |s: &PropSheet| match &s.pages[Page::MouseTouchpad.index()]
        .item("two_finger_tap_right_click")
        .unwrap()
        .control
    {
        Control::Check { enabled, .. } => *enabled,
        _ => panic!("a check"),
    };
    assert!(enabled(&s));
    let off = Settings {
        tap_to_click: false,
        ..Settings::default()
    };
    write(&mut s, &off, &reg(), "us");
    assert!(!enabled(&s));
}

#[test]
fn the_cursor_page_geometry() {
    let s = built();
    let p = Page::Cursor;
    assert_eq!(rect(&s, p, "cursor_group"), Rect::new(20, 44, 358, 84));
    assert_eq!(rect(&s, p, "cursor_theme"), Rect::new(92, 62, 274, 21));
    assert_eq!(rect(&s, p, "cursor_size"), Rect::new(92, 92, 110, 21));
    assert_eq!(rect(&s, p, CURSOR_PREVIEW), Rect::new(32, 160, 334, 208));
    assert_eq!(preview_size(Area::MIN), (330, 204));
}

#[test]
fn an_empty_cursor_theme_is_the_default_row() {
    let sheet = with(&Settings::default());
    let page = &sheet.pages[Page::Cursor.index()];
    assert_eq!(page.combo_selected("cursor_theme"), Some(0));
    assert_eq!(page.combo_selected("cursor_size"), Some(0));
    let mut got = Settings {
        cursor_theme: "stale".into(),
        cursor_size: 7,
        ..Settings::default()
    };
    read_all(&sheet, &mut got);
    assert_eq!(got.cursor_theme, "", "(Default) reads back as no theme");
    assert_eq!(got.cursor_size, 0, "Default reads back as size 0");
}

#[test]
fn a_cursor_theme_and_size_the_lists_lack_still_show() {
    let want = Settings {
        cursor_theme: "gone".into(),
        cursor_size: 40,
        ..Settings::default()
    };
    let sheet = with(&want);
    let mut got = Settings::default();
    read_all(&sheet, &mut got);
    assert_eq!((got.cursor_theme.as_str(), got.cursor_size), ("gone", 40));
}

#[test]
fn shortcut_sheet_values_go_in_and_come_back_out() {
    let want = Settings {
        shortcut_overlay: false,
        shortcut_overlay_ms: 600,
        ..Settings::default()
    };
    let s = with(&want);
    let page = &s.pages[Page::Keyboard.index()];
    assert_eq!(page.checked("shortcut_overlay"), Some(false));
    let mut got = Settings::default();
    read_all(&s, &mut got);
    assert_eq!(
        (got.shortcut_overlay, got.shortcut_overlay_ms),
        (false, 600)
    );
}
