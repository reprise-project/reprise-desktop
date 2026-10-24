use super::*;

#[test]
fn the_advanced_page_geometry() {
    let s = built();
    let p = Page::Advanced;
    assert!(
        s.pages[p.index()].item("terminal").is_none(),
        "the terminal is picked on Default Programs"
    );
    assert_eq!(rect(&s, p, "task_manager"), Rect::new(20, 60, 358, 20));
    assert_eq!(rect(&s, p, "adaptive_sync"), Rect::new(20, 88, 13, 13));
    assert_eq!(rect(&s, p, "clipboard_history"), Rect::new(20, 110, 13, 13));
    assert_eq!(rect(&s, p, "screenshot_save"), Rect::new(20, 132, 13, 13));
    assert_eq!(rect(&s, p, "recent_files"), Rect::new(20, 154, 13, 13));
    assert_eq!(rect(&s, p, "capture_border"), Rect::new(20, 176, 13, 13));
    assert_eq!(rect(&s, p, PRIVACY_LOCATION), Rect::new(20, 198, 13, 13));
    assert_eq!(locations(Area::MIN), Rect::new(20, 220, 275, 68));
    assert_eq!(rect(&s, p, WATER), Rect::new(20, 344, 13, 13));
    assert_eq!(
        rect(&s, p, WATER_STYLE),
        Rect::new(132, 340, 100, combo::COMBO_H)
    );
    assert_eq!(rect(&s, p, FORGET), Rect::new(303, 220, 75, 23));
    assert!(
        s.pages[p.index()].item("desktop_icons").is_none(),
        "desktop_icons is on Background"
    );
    assert_eq!(rect(&s, p, DEBUG_OVERLAY), Rect::new(20, 300, 13, 13));
    assert_eq!(rect(&s, p, DEBUG_IN_CAPTURES), Rect::new(38, 322, 13, 13));
    assert_eq!(rect(&s, p, THUMBS), Rect::new(20, 392, 13, 13));
    assert_eq!(rect(&s, p, THUMBS_MB), Rect::new(112, 414, 50, 20));
    assert_eq!(rect(&s, p, THUMBS_CLEAR), Rect::new(303, 412, 75, 23));
    assert_eq!(rect(&s, p, "note"), Rect::new(20, 444, 275, 26));
    assert_eq!(rect(&s, p, EDIT_FILE), Rect::new(303, 444, 75, 23));
    for id in ["banner_label", "startmenu_banner", "start_icon"] {
        assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Taskbar");
    }
    inside_page(&s, p);
}

#[test]
fn water_styles_list_the_fluids() {
    let keys: Vec<_> = WATER_STYLES.iter().map(|c| c.0).collect();
    assert_eq!(
        keys,
        ["calm", "lively", "rain", "ink", "glow", "sparks", "neon"]
    );
    let labels: Vec<_> = WATER_STYLES.iter().skip(3).map(|c| c.1).collect();
    assert_eq!(labels, ["Ink", "Glow", "Sparks", "Neon"]);
    for (key, _) in WATER_STYLES {
        assert!(reprisekit::water::Water::parse(key).is_some(), "{key}");
    }
}

#[test]
fn water_note_sits_under_the_style_combo_and_greys_when_off() {
    let mut s = built();
    let p = Page::Advanced;
    let combo = rect(&s, p, WATER_STYLE);
    let (a, b) = (rect(&s, p, "water_note"), rect(&s, p, "water_note_2"));
    assert!(
        a.y >= combo.bottom() && b.y >= a.bottom(),
        "{a:?} {b:?} under {combo:?}"
    );
    let page = &s.pages[p.index()];
    let Control::Label { text, .. } = &page.item("water_note").unwrap().control else {
        panic!("the note is a label");
    };
    assert!(text.starts_with("With Ink, Glow, Sparks or Neon"));
    inside_page(&s, p);
    let mut want = Settings {
        wallpaper_water: "off".into(),
        ..Settings::default()
    };
    follow(&mut s, &want);
    assert!(!enabled_on(&s, p, "water_note") && !enabled_on(&s, p, "water_note_2"));
    want.wallpaper_water = "ink".into();
    follow(&mut s, &want);
    assert!(enabled_on(&s, p, "water_note") && enabled_on(&s, p, "water_note_2"));
}

#[test]
fn the_water_style_greys_while_water_is_off() {
    let mut s = built();
    let mut want = Settings {
        wallpaper_water: "off".into(),
        ..Settings::default()
    };
    follow(&mut s, &want);
    assert!(
        !enabled_on(&s, Page::Advanced, WATER_STYLE),
        "greyed while off"
    );
    want.wallpaper_water = "rain".into();
    follow(&mut s, &want);
    assert!(enabled_on(&s, Page::Advanced, WATER_STYLE));
    write_forms(&mut s, &want);
    let page = &s.pages[Page::Advanced.index()];
    assert_eq!(
        page.combo_selected(WATER_STYLE),
        Some(2),
        "Rain is the third row"
    );
    assert!(matches!(
        page.item(WATER).map(|i| &i.control),
        Some(Control::Check { checked: true, .. })
    ));
    let mut got = Settings::default();
    tick(&mut s, &mut got, Page::Advanced, WATER, false);
    assert_eq!(got.wallpaper_water, "off");
    tick(&mut s, &mut got, Page::Advanced, WATER, true);
    assert_eq!(got.wallpaper_water, "rain");
}

fn thumbs_mb(s: &PropSheet) -> &UpDown {
    match &s.pages[Page::Advanced.index()]
        .item(THUMBS_MB)
        .unwrap()
        .control
    {
        Control::UpDown(u) => u,
        _ => panic!("{THUMBS_MB} is an up-down"),
    }
}

#[test]
fn the_thumbnail_cache_row_reads_and_writes_its_keys() {
    let mut got = crate::model::load("[thumbnail_cache]\nenabled = false\nmax_mb = 64\n");
    let mut s = with(&got);
    let p = Page::Advanced;
    assert_eq!(thumbs_mb(&s).value(), 64);
    for id in THUMBS_SIZE {
        assert!(!enabled_on(&s, p, id), "{id} greyed with the cache off");
    }
    let base = got.clone();
    tick(&mut s, &mut got, p, THUMBS, true);
    for id in THUMBS_SIZE {
        assert!(enabled_on(&s, p, id), "{id} lit with the cache on");
    }
    if let Some(Control::UpDown(u)) = s.pages[p.index()]
        .item_mut(THUMBS_MB)
        .map(|i| &mut i.control)
    {
        u.set(128);
    }
    read_one(&s, THUMBS_MB, &mut got);
    let text = "[thumbnail_cache]\nenabled = false\nmax_mb = 64\n";
    let out = crate::model::save(text, &got, &base).expect("parses");
    assert_eq!(
        out, "[thumbnail_cache]\nmax_mb = 128\n",
        "on is the default, so its key goes"
    );
    let back = crate::model::load(&out);
    assert_eq!((back.thumbnail_cache, back.thumbnail_max_mb), (true, 128));
}

#[test]
fn the_thumbnail_cache_size_keeps_to_the_range_the_cache_reads() {
    let mut got = crate::model::load("[thumbnail_cache]\nmax_mb = 9000\n");
    let mut s = with(&got);
    let u = thumbs_mb(&s);
    assert_eq!((u.min, u.max), (1, 4096));
    assert_eq!(u.value(), 4096, "shown clamped");
    assert_eq!(got.thumbnail_max_mb, 9000, "left in the file until touched");
    if let Some(Control::UpDown(u)) = s.pages[Page::Advanced.index()]
        .item_mut(THUMBS_MB)
        .map(|i| &mut i.control)
    {
        u.set(0);
    }
    read_one(&s, THUMBS_MB, &mut got);
    assert_eq!(got.thumbnail_max_mb, 1);
    s = with(&Settings::default());
    assert_eq!(thumbs_mb(&s).value(), 512, "the cache's default");
}

#[test]
fn clear_greys_while_it_runs_then_says_what_went() {
    let mut s = built();
    let page = &mut s.pages[Page::Advanced.index()];
    let clear_lit = |page: &Form| {
        matches!(
            page.item(THUMBS_CLEAR).unwrap().control,
            Control::Button { enabled: true, .. }
        )
    };
    let line = |page: &Form| match &page.item(THUMBS_CLEARED).unwrap().control {
        Control::Label { text, .. } => text.clone(),
        _ => panic!("a label"),
    };
    assert!(clear_lit(page));
    assert_eq!(line(page), "");
    show_clearing(page, Clearing::Running);
    assert!(!clear_lit(page));
    assert_eq!(line(page), "Clearing...");
    let done = |files, bytes| Clearing::Done(Cleared { files, bytes });
    show_clearing(page, done(214, 1_384_448));
    assert!(clear_lit(page));
    assert_eq!(line(page), "Removed 214 files, 1.32 MB.");
    show_clearing(page, done(1, 900));
    assert_eq!(line(page), "Removed 1 file, 900 bytes.");
    show_clearing(page, done(0, 0));
    assert_eq!(line(page), "Nothing to remove.");
    show_clearing(page, Clearing::Failed);
    assert!(clear_lit(page));
    assert_eq!(line(page), "Couldn't clear the thumbnails.");
}

/// The Clear row idle, clearing and done, Classic at 100%; every result line fits.
#[test]
fn the_clear_row_pixels() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut page = advanced(Area::MIN);
    let line = page.item(THUMBS_CLEARED).unwrap().rect;
    let widest = Clearing::Done(Cleared {
        files: 99_999,
        bytes: 999 << 30,
    });
    for state in [widest, Clearing::Failed] {
        let line_text = cleared_text(state);
        assert!(
            text.measure(&line_text, false, px) <= line.width,
            "{line_text}"
        );
    }
    let clear = page.item(THUMBS_CLEAR).unwrap().rect;
    let row = Rect::new(line.x, line.y, line.width, clear.bottom() - line.y);
    let hash = |page: &Form| {
        let mut c = Canvas::new(row.right() as u32 + 8, row.bottom() as u32 + 8);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(&mut c, &text, &theme.paint(), page, false, px);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for y in row.y..row.bottom() {
            for x in row.x..row.right() {
                h = (h ^ u64::from(c.get(x, y))).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    };
    let idle = hash(&page);
    show_clearing(&mut page, Clearing::Running);
    let clearing = hash(&page);
    show_clearing(
        &mut page,
        Clearing::Done(Cleared {
            files: 214,
            bytes: 1_384_448,
        }),
    );
    let done = hash(&page);
    assert_eq!(
        [idle, clearing, done],
        [
            7_897_640_068_597_891_350,
            2_251_566_719_493_853_932,
            14_478_241_142_069_230_046
        ],
        "idle, clearing, done"
    );
}

#[test]
fn the_debug_overlay_checks() {
    let mut got = Settings::default();
    let mut s = with(&got);
    let p = Page::Advanced;
    assert!(
        !enabled_on(&s, p, DEBUG_IN_CAPTURES),
        "greyed with the overlay off"
    );
    tick(&mut s, &mut got, p, DEBUG_OVERLAY, true);
    assert!(got.debug_overlay);
    assert!(enabled_on(&s, p, DEBUG_IN_CAPTURES));
    tick(&mut s, &mut got, p, DEBUG_IN_CAPTURES, true);
    assert!(got.debug_overlay_in_captures);
}

#[test]
fn the_location_check_and_list() {
    let mut s = built();
    let mut got = Settings::default();
    assert!(got.location, "programs may ask by default");
    tick(&mut s, &mut got, Page::Advanced, PRIVACY_LOCATION, false);
    assert!(!got.location);
    let mut lv = new_location_list();
    fit_location_columns(
        &mut lv,
        &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
        locations(Area::MIN),
    );
    let widths: Vec<i32> = lv.columns.iter().map(|c| c.width).collect();
    assert_eq!(widths, [185, 70]);
    fill_locations(
        &mut lv,
        &[("Maps".into(), true), ("org.example.Weather".into(), false)],
    );
    let cells: Vec<&[String]> = lv.rows.iter().map(|r| r.cells.as_slice()).collect();
    assert_eq!(
        cells,
        [
            ["Maps", "Allowed"].map(String::from).as_slice(),
            ["org.example.Weather", "Denied"]
                .map(String::from)
                .as_slice(),
        ]
    );
}

#[test]
fn the_power_page_geometry() {
    let s = built();
    let p = Page::Power;
    assert_eq!(rect(&s, p, "turnoff_group"), Rect::new(20, 44, 358, 126));
    assert_eq!(rect(&s, p, "standby_label"), Rect::new(32, 66, 70, 13));
    assert_eq!(rect(&s, p, "standby"), Rect::new(106, 62, 260, 20));
    assert_eq!(rect(&s, p, "poweroff"), Rect::new(106, 88, 260, 20));
    assert_eq!(rect(&s, p, "reboot"), Rect::new(106, 114, 260, 20));
    assert_eq!(rect(&s, p, "hibernate"), Rect::new(106, 140, 260, 20));
    assert_eq!(rect(&s, p, "idle_group"), Rect::new(20, 180, 358, 196));
    assert_eq!(rect(&s, p, "lock_after"), Rect::new(150, 200, 50, 20));
    assert_eq!(rect(&s, p, "blank_after"), Rect::new(150, 252, 50, 20));
    assert_eq!(rect(&s, p, "lock_fade"), Rect::new(150, 278, 50, 20));
    assert_eq!(rect(&s, p, "idle_note"), Rect::new(32, 308, 334, 13));
    assert_eq!(rect(&s, p, "lock_on_sleep"), Rect::new(32, 330, 13, 13));
    assert_eq!(rect(&s, p, "lock_wallpaper"), Rect::new(32, 352, 13, 13));
    inside_page(&s, p);
}

#[test]
fn the_startup_page_geometry() {
    let s = built();
    let p = Page::Startup;
    assert_eq!(rect(&s, p, ADD), Rect::new(222, 260, 75, 23));
    assert_eq!(rect(&s, p, REMOVE), Rect::new(303, 260, 75, 23));
    assert_eq!(rect(&s, p, "session_group"), Rect::new(20, 292, 358, 66));
    assert_eq!(rect(&s, p, "session_restore"), Rect::new(32, 314, 13, 13));
    assert_eq!(
        rect(&s, p, "session_end_processes"),
        Rect::new(32, 336, 13, 13)
    );
    inside_page(&s, p);

    assert_eq!(list(Area::MIN), Rect::new(20, 44, 358, 208));
    let lv = new_list();
    let lay = listview::layout(
        &lv,
        list(Area::MIN),
        &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
    );
    assert_eq!(lay.header, Rect::new(22, 46, 354, listview::HEADER_H));
    assert_eq!(lay.rows, 11, "11 rows of 17 px under the header");
    assert_eq!(
        COLUMNS.iter().map(|c| c.1).sum::<i32>(),
        lay.header.width - 16,
        "the columns leave the 16 px bar its room"
    );
}

#[test]
fn a_startup_list_that_scrolls_draws_only_the_vertical_bar() {
    use reprisekit::interface::Interface;
    let text = TextRenderer::load(&FontConfig::default());
    let entries: Vec<Entry> = (0..30)
        .map(|i| Entry {
            id: format!("item{i}"),
            name: format!("Item {i}"),
            exec: format!("/usr/bin/item{i} --tray"),
            exec_line: format!("/usr/bin/item{i} --tray"),
            hidden: false,
            only_show_in: Vec::new(),
            not_show_in: Vec::new(),
            try_exec: None,
            path: None,
            terminal: false,
            under_systemd: false,
            user: true,
        })
        .collect();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for pct in [100, 150, 200] {
            let theme = reprisekit::interfaces::theme_for(i).with_text_size(pct);
            let cm = theme.controls();
            let r = theme.text.rect(list(Area::MIN));
            let mut lv = new_list();
            fill_list(&mut lv, &entries, &[true; 30]);
            fit_list_columns(&mut lv, &cm, r);
            let lay = listview::layout(&lv, r, &cm);
            let at = format!("{i:?} {pct}%");
            assert!(lay.hscroll.is_none(), "{at}: no horizontal bar");
            let bar = lay.scrollbar.expect("a vertical bar");
            assert_eq!(bar.right(), r.right() - 2, "{at}");
            let mut c = Canvas::new((r.right() + 2) as u32, (r.bottom() + 2) as u32);
            draw_list(&mut c, &text, &theme, &lv, r, theme.fonts.size_px, 1);
            let window = theme.colors.window.argb();
            assert_eq!(
                c.get(lay.body.x + lay.body.width / 2, r.bottom() - 3),
                window,
                "{at}: the last row runs down to the frame"
            );
            assert_ne!(
                c.get(bar.x + bar.width / 2, bar.bottom() - 3),
                window,
                "{at}: the bar"
            );
        }
    }
}
