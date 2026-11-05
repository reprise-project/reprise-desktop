use super::*;

#[test]
fn the_sound_page_geometry() {
    let s = built();
    let p = Page::Sound;
    assert_eq!(rect(&s, p, "sound_theme_group"), Rect::new(20, 44, 358, 52));
    assert_eq!(rect(&s, p, "sound_theme_label"), Rect::new(GX, 70, 80, 13));
    assert_eq!(rect(&s, p, "sound_theme"), Rect::new(116, 66, 200, 21));
    assert_eq!(
        rect(&s, p, "sound_events_group"),
        Rect::new(20, 106, 358, 74)
    );
    assert_eq!(
        rect(&s, p, "notification_sound"),
        Rect::new(GX, 128, 13, 13)
    );
    assert_eq!(rect(&s, p, "bell"), Rect::new(GX, 154, 13, 13));
    assert_eq!(rect(&s, p, "volume_group"), Rect::new(20, 190, 358, 74));
    assert_eq!(rect(&s, p, "allow_boost"), Rect::new(GX, 212, 13, 13));
    assert_eq!(rect(&s, p, "notify_group"), Rect::new(20, 274, 358, 144));
    assert_eq!(
        rect(&s, p, notifypage::PROGRAMS),
        Rect::new(GX, 294, 334, 89)
    );
    assert_eq!(
        rect(&s, p, notifypage::BALLOONS),
        Rect::new(GX, 391, 13, 13)
    );
    assert_eq!(
        rect(&s, p, notifypage::SOUNDS),
        Rect::new(GX + 140, 391, 13, 13)
    );
    inside_page(&s, p);
}

#[test]
fn the_program_switches_pin_their_pixels_in_every_look() {
    let s = Settings {
        notify_no_sounds: vec!["chat".into()],
        ..Settings::default()
    };
    let mut f = sound(Area::MIN, &[]);
    let programs = notifypage::programs(
        &[
            ("mail".into(), "Mail".into()),
            ("chat".into(), "Chat".into()),
        ],
        &s,
        |_| None,
    );
    notifypage::fill(&mut f, &programs, &s);
    let row = Rect::new(20, 274, 358, 144);
    let Some(none) = row_pins(&f, row) else {
        return;
    };
    if let Some(form::Control::ListBox { list, .. }) =
        f.item_mut(notifypage::PROGRAMS).map(|i| &mut i.control)
    {
        list.selected = Some(1);
    }
    notifypage::fill(&mut f, &programs, &s);
    let chat = row_pins(&f, row).unwrap();
    assert_eq!(
        [none, chat],
        [
            [0x7271412745a72c79, 0xfd6a7e8ff001b3f9, 0xb31b2628785111f8],
            [0xff6d03cb3a6174f0, 0xa243e9831ad6b73a, 0xc5e8178a57516dfb],
        ],
        "nothing picked, Chat picked; Classic, Blue, Glass"
    );
}

/// The checks write `[notifications] sound` and `[sounds] bell` and grey the theme combo while
/// both are off; the combo lists the installed sound themes, keeps a configured one that is
/// missing, and writes the pick.
#[test]
fn the_sound_theme_and_the_sound_checks_write_their_keys() {
    let root = std::env::temp_dir().join(format!("reprise-settings-sounds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    for f in ["ocean/index.theme", "deep/stereo/bell.oga"] {
        let path = root.join(f);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    let themes = repriseaudio::sounds::themes(std::slice::from_ref(&root));
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(themes, ["deep", "ocean"]);

    let want = Settings {
        sound_theme: "gone".into(),
        ..Settings::default()
    };
    let mut s = registry::sheet(
        &want,
        &registry::Sources {
            icon_themes: &[],
            gtk_themes: &[],
            qt_tools: false,
            cursor_themes: &[],
            sound_themes: &themes,
            layouts: &[],
            heads: &[],
        },
    );
    let p = Page::Sound;
    let id = "sound_theme";
    assert_eq!(
        rows(&s, p, id),
        (vec!["deep".into(), "ocean".into(), "gone".into()], 2)
    );
    assert_eq!(s.pages[p.index()].checked("notification_sound"), Some(true));
    assert_eq!(s.pages[p.index()].checked("bell"), Some(true));

    let mut got = want.clone();
    pick(&mut s, &mut got, p, id, 1);
    assert_eq!(got.sound_theme, "ocean");

    for id in ["sound_theme_label", id] {
        assert!(enabled_on(&s, p, id), "{id} is lit with the sound on");
    }
    tick(&mut s, &mut got, p, "notification_sound", false);
    assert!(!got.notification_sound);
    for id in ["sound_theme_label", id] {
        assert!(enabled_on(&s, p, id), "{id} stays lit for the bell");
    }
    tick(&mut s, &mut got, p, "bell", false);
    assert!(!got.bell);
    for id in ["sound_theme_label", id] {
        assert!(!enabled_on(&s, p, id), "{id} greys with both sounds off");
    }
    assert_eq!(
        got,
        Settings {
            notification_sound: false,
            sound_theme: "ocean".into(),
            bell: false,
            ..want
        }
    );
}

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
    assert_eq!(locations(Area::MIN), Rect::new(20, 220, 275, 46));
    assert_eq!(rect(&s, p, FORGET), Rect::new(303, 220, 75, 23));
    assert!(
        s.pages[p.index()].item("desktop_icons").is_none(),
        "desktop_icons is on Background"
    );
    assert_eq!(rect(&s, p, DEBUG_OVERLAY), Rect::new(20, 278, 13, 13));
    assert_eq!(rect(&s, p, DEBUG_IN_CAPTURES), Rect::new(38, 300, 13, 13));
    assert_eq!(rect(&s, p, THUMBS), Rect::new(20, 370, 13, 13));
    assert_eq!(rect(&s, p, THUMBS_MB), Rect::new(82, 392, 50, 20));
    assert_eq!(rect(&s, p, THUMBS_SIZE[2]), Rect::new(136, 396, 20, 13));
    assert_eq!(rect(&s, p, THUMBS_CLEAR), Rect::new(303, 390, 75, 23));
    assert_eq!(rect(&s, p, THUMBNAILERS), Rect::new(20, 418, 13, 13));
    assert_eq!(rect(&s, p, "note"), Rect::new(20, 440, 275, 26));
    assert_eq!(rect(&s, p, EDIT_FILE), Rect::new(303, 440, 75, 23));
    for id in ["banner_label", "startmenu_banner", "start_icon"] {
        assert!(s.pages[p.index()].item(id).is_none(), "{id} is on Taskbar");
    }
    inside_page(&s, p);
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
fn the_thumbnailers_check_writes_external_thumbnailers() {
    let mut got = crate::model::load("");
    let mut s = with(&got);
    let p = Page::Advanced;
    assert!(got.external_thumbnailers, "on unless set");
    let base = got.clone();
    tick(&mut s, &mut got, p, THUMBNAILERS, false);
    let out = crate::model::save("", &got, &base).expect("parses");
    assert_eq!(out, "[thumbnail_cache]\nexternal_thumbnailers = false\n");
    assert!(!crate::model::load(&out).external_thumbnailers);
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
    assert_eq!(rect(&s, p, "storage_group"), Rect::new(20, 388, 358, 74));
    assert_eq!(
        rect(&s, p, "low_space_percent"),
        Rect::new(150, 408, 50, 20)
    );
    assert_eq!(rect(&s, p, "low_space_note"), Rect::new(32, 438, 334, 13));
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
    assert_eq!(rect(&s, p, "updates_group"), Rect::new(20, 368, 358, 44));
    assert_eq!(rect(&s, p, "restart_notice"), Rect::new(32, 390, 13, 13));
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

#[test]
fn the_lock_when_away_group_sits_under_the_screen_saver() {
    let s = built();
    let p = Page::ScreenSaverLock;
    assert_eq!(rect(&s, p, "away_group"), Rect::new(20, 338, 358, 118));
    assert_eq!(rect(&s, p, "away_label"), Rect::new(32, 358, 334, 13));
    assert_eq!(rect(&s, p, "away_device"), Rect::new(32, 374, 200, 21));
    assert_eq!(rect(&s, p, "away_after_s"), Rect::new(76, 404, 50, 20));
    assert_eq!(rect(&s, p, "away_units"), Rect::new(132, 408, 60, 13));
    assert_eq!(rect(&s, p, "away_note"), Rect::new(32, 432, 334, 13));
    inside_page(&s, p);
}

#[test]
fn the_backup_page_geometry() {
    let s = built();
    let p = Page::Backup;
    assert_eq!(rect(&s, p, "backup_intro"), Rect::new(20, 44, 358, 13));
    assert_eq!(rect(&s, p, "backup_list"), Rect::new(GX, 66, 346, 91));
    assert_eq!(rect(&s, p, "backup_left_out"), Rect::new(20, 167, 358, 39));
    assert_eq!(rect(&s, p, BACKUP_EXPORT), Rect::new(20, 216, 75, 23));
    assert_eq!(rect(&s, p, BACKUP_IMPORT), Rect::new(101, 216, 75, 23));
    assert_eq!(rect(&s, p, BACKUP_STATUS), Rect::new(20, 250, 358, 26));
    inside_page(&s, p);
}

/// Every line of the Backup page fits its rect in the dialog font.
#[test]
fn the_backup_page_lines_fit() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let page = backup(Area::MIN);
    for it in &page.items {
        let lines = match &it.control {
            Control::Lines(l) => l.clone(),
            Control::Label { text, .. } => vec![text.clone()],
            _ => continue,
        };
        for line in lines {
            assert!(
                text.measure(&line, false, px) <= it.rect.width,
                "{}: {line}",
                it.id
            );
        }
    }
}

fn serving(clients: Vec<reprisebus::remote::Client>) -> reprisebus::remote::Status {
    reprisebus::remote::Status {
        listening: "0.0.0.0:3389".into(),
        clients,
        encoder: "VA-API H.264".into(),
        encoders: vec!["vaapi".into(), "software".into(), "none".into()],
        fingerprint: "3F:9A:0C:51:E2:7B:44:D8:19:A6:5E:C0:2D:93:7F:B1:08:6C:E4:5A:21:DF:90:3B:C7:12:6E:A8:F5:04:9D:B3".into(),
    }
}

fn client() -> reprisebus::remote::Client {
    reprisebus::remote::Client {
        address: "192.168.1.30:51234".into(),
        user: "user".into(),
        since: 1_762_000_000,
    }
}

/// Whether `id` on `page` is lit; panics for a control with no greyed state.
fn lit(page: &Form, id: &str) -> bool {
    match &page.item(id).unwrap_or_else(|| panic!("{id}")).control {
        Control::Check { enabled, .. }
        | Control::Label { enabled, .. }
        | Control::Edit { enabled, .. }
        | Control::Button { enabled, .. } => *enabled,
        Control::Combo(c) => c.enabled,
        Control::UpDown(u) => u.enabled,
        Control::TrackBar(t) => t.enabled,
        _ => panic!("{id} has no greyed state"),
    }
}

/// Everything under the check box that greys with it.
const REMOTE_GREYED: [&str; 18] = [
    "status",
    "encoder",
    "port_label",
    "port",
    "quality_label",
    "quality_speed",
    "quality",
    "quality_sharp",
    "quality_value",
    "frame_rate_label",
    "frame_rate",
    "video_encoder_label",
    "video_encoder",
    "remote_sound",
    "fingerprint_label",
    "fingerprint",
    "regenerate",
    "disconnect",
];

#[test]
fn the_remote_desktop_page_geometry() {
    let s = built();
    let p = Page::RemoteDesktop;
    assert_eq!(rect(&s, p, "allow"), Rect::new(20, 44, 13, 13));
    assert_eq!(rect(&s, p, "status"), Rect::new(38, 80, 340, 13));
    assert_eq!(rect(&s, p, "encoder"), Rect::new(38, 96, 340, 13));
    assert_eq!(
        rect(&s, p, "connection_group"),
        Rect::new(20, 120, 358, 183)
    );
    assert_eq!(rect(&s, p, "port_label"), Rect::new(GX, 144, 114, 13));
    assert_eq!(rect(&s, p, "port"), Rect::new(150, 140, 50, 20));
    assert_eq!(rect(&s, p, "quality_label"), Rect::new(GX, 170, 200, 13));
    assert_eq!(rect(&s, p, "quality_speed"), Rect::new(GX, 197, 44, 13));
    assert_eq!(rect(&s, p, "quality"), Rect::new(78, 191, 238, 26));
    assert_eq!(rect(&s, p, "quality_sharp"), Rect::new(322, 197, 44, 13));
    assert_eq!(rect(&s, p, "quality_value"), Rect::new(286, 170, 80, 13));
    assert_eq!(rect(&s, p, "frame_rate_label"), Rect::new(GX, 230, 114, 13));
    assert_eq!(
        rect(&s, p, "frame_rate"),
        Rect::new(150, 226, 160, combo::COMBO_H)
    );
    assert_eq!(
        rect(&s, p, "video_encoder_label"),
        Rect::new(GX, 256, 114, 13)
    );
    assert_eq!(
        rect(&s, p, "video_encoder"),
        Rect::new(150, 252, 160, combo::COMBO_H)
    );
    assert_eq!(rect(&s, p, "remote_sound"), Rect::new(GX, 282, 13, 13));
    assert_eq!(
        rect(&s, p, "certificate_group"),
        Rect::new(20, 313, 358, 66)
    );
    assert_eq!(
        rect(&s, p, "fingerprint_label"),
        Rect::new(GX, 333, 334, 13)
    );
    assert_eq!(rect(&s, p, "fingerprint"), Rect::new(GX, 349, 218, 20));
    assert_eq!(rect(&s, p, "regenerate"), Rect::new(256, 347, 110, 23));
    assert_eq!(rect(&s, p, "client_group"), Rect::new(20, 389, 358, 85));
    assert_eq!(rect(&s, p, "clients"), Rect::new(GX, 409, 253, 55));
    assert_eq!(rect(&s, p, "disconnect"), Rect::new(291, 409, 75, 23));
    inside_page(&s, p);
    let page = &s.pages[p.index()];
    assert_eq!(
        rows(&s, p, "frame_rate"),
        (
            vec!["30 frames per second".into(), "60 frames per second".into()],
            1
        ),
        "60 unless set"
    );
    assert_eq!(page.track_pos("quality"), Some(1), "balanced unless set");
    assert_eq!(
        rows(&s, p, "video_encoder"),
        (
            vec![
                "Automatic".into(),
                "Hardware, VA-API".into(),
                "Hardware, NVENC".into(),
                "Hardware, Vulkan".into(),
                "Software (x264)".into(),
                "None (RemoteFX)".into(),
            ],
            0
        ),
        "automatic unless set"
    );
}

#[test]
fn the_remote_switch_says_it_acts_at_once() {
    let s = built();
    let p = Page::RemoteDesktop;
    assert_eq!(
        lines(&s, p, "allow_note"),
        ["Turning this on or off takes effect at once."]
    );
    assert_eq!(rect(&s, p, "allow_note"), Rect::new(38, 64, 340, 13));
    assert_eq!(rect(&s, p, "connection_group").y, 120);
    assert_eq!(rect(&s, p, "client_group"), Rect::new(20, 389, 358, 85));
    inside_page(&s, p);
}

/// The encoder rows lit, top to bottom.
fn encoders_lit(page: &Form) -> Vec<bool> {
    match &page.item("video_encoder").unwrap().control {
        Control::Combo(c) => c.items.iter().map(|i| !i.greyed).collect(),
        _ => panic!("the encoder is a drop-down"),
    }
}

#[test]
fn unavailable_encoders_are_greyed() {
    let software_only = reprisebus::remote::Status {
        encoders: vec!["software".into(), "none".into()],
        ..serving(Vec::new())
    };
    let mut page = remote_desktop(Area::MIN, Some(&software_only), true);
    assert_eq!(
        encoders_lit(&page),
        [true, false, false, false, true, true],
        "Automatic, Software and None"
    );
    show_remote(&mut page, None, true, None, None);
    assert_eq!(
        encoders_lit(&page),
        [true, false, false, false, false, false],
        "only Automatic while the server does not answer"
    );
    assert_eq!(
        encoders_lit(&remote_desktop(Area::MIN, Some(&software_only), false)),
        [true, false, false, false, false, false],
        "only Automatic while it is off"
    );
}

/// Alt+A stays Apply's: an access key takes the page's first match, and the sheet's buttons
/// come after the page's controls.
#[test]
fn no_remote_desktop_control_shares_an_access_key_with_the_sheet_buttons() {
    let s = built();
    let items = &s.pages[Page::RemoteDesktop.index()].items;
    let key = |c: &Control| match c {
        Control::Label { text, .. } => reprisekit::text::access_key(text),
        Control::Check { label, .. }
        | Control::Radio { label, .. }
        | Control::Button { label, .. } => reprisekit::text::access_key(label),
        _ => None,
    };
    let sheet = [propsheet::OK, propsheet::CANCEL, propsheet::APPLY];
    let buttons: Vec<char> = items
        .iter()
        .filter(|it| sheet.contains(&it.id))
        .filter_map(|it| key(&it.control))
        .collect();
    assert_eq!(buttons, ['a'], "Apply carries the sheet's one access key");
    for it in items.iter().filter(|it| !sheet.contains(&it.id)) {
        if let Some(k) = key(&it.control) {
            assert!(!buttons.contains(&k), "{} takes Alt+{k}", it.id);
        }
    }
}

#[test]
fn an_unknown_quality_shows_balanced() {
    let got = crate::model::load("[remote_desktop]\nquality = \"ultra\"\n");
    let s = with(&got);
    let page = &s.pages[Page::RemoteDesktop.index()];
    assert_eq!(page.track_pos("quality"), Some(1));
    assert!(
        matches!(
            &page.item("quality_value").unwrap().control,
            Control::Label { text, .. } if text == "Balanced"
        ),
        "the name above the trackbar is Balanced"
    );
}

#[test]
fn allow_reflects_the_unit() {
    let off = remote_desktop(Area::MIN, None, false);
    assert_eq!(off.checked("allow"), Some(false));
    assert!(lit(&off, "allow"), "the check box itself stays lit");
    assert!(
        lit(&off, "allow_note"),
        "the note under the check box stays lit"
    );
    for id in REMOTE_GREYED {
        assert!(
            !lit(&off, id),
            "{id} greys while remote connections are off"
        );
    }
    let Control::Label { text, .. } = &off.item("status").unwrap().control else {
        panic!("the status is a label");
    };
    assert_eq!(text, "Off");

    let on = remote_desktop(Area::MIN, Some(&serving(vec![client()])), true);
    assert_eq!(on.checked("allow"), Some(true));
    for id in REMOTE_GREYED {
        assert!(lit(&on, id), "{id} is lit while the server runs");
    }
}

#[test]
fn the_status_line_says_where_the_server_listens() {
    let s = serving(Vec::new());
    assert_eq!(
        crate::pages::system::remote_status(None, false, None),
        "Off"
    );
    assert_eq!(
        crate::pages::system::remote_status(Some(&s), false, None),
        "Off",
        "a disabled unit is off whatever still answers"
    );
    assert_eq!(
        crate::pages::system::remote_status(None, true, None),
        "Not answering"
    );
    assert_eq!(
        crate::pages::system::remote_status(Some(&s), true, Some("192.168.1.20")),
        "Listening on port 3389 at 192.168.1.20"
    );
    assert_eq!(
        crate::pages::system::remote_status(Some(&s), true, None),
        "Listening on port 3389"
    );
    let failed = reprisebus::remote::Status {
        listening: "error: Address already in use (os error 98)".into(),
        ..s
    };
    assert_eq!(
        crate::pages::system::remote_status(Some(&failed), true, Some("192.168.1.20")),
        "Not listening: Address already in use (os error 98)"
    );
}

#[test]
fn disconnect_greys_without_a_client() {
    let idle = remote_desktop(Area::MIN, Some(&serving(Vec::new())), true);
    assert!(!lit(&idle, "disconnect"));
    assert!(lit(&idle, "regenerate"));
    let busy = remote_desktop(Area::MIN, Some(&serving(vec![client()])), true);
    assert!(lit(&busy, "disconnect"));
    let Control::ListBox { list, .. } = &busy.item("clients").unwrap().control else {
        panic!("the clients are a list box");
    };
    assert_eq!(list.items, ["192.168.1.30"], "the address without its port");
    assert!(
        list.detail[0].starts_with("user, since "),
        "{:?}",
        list.detail
    );
    let silent = remote_desktop(Area::MIN, None, true);
    for id in ["disconnect", "regenerate"] {
        assert!(
            !lit(&silent, id),
            "{id} greys while the server does not answer"
        );
    }
    assert!(lit(&silent, "port"), "the settings stay lit");
}

fn viewer(address: &str) -> reprisebus::remote::Client {
    reprisebus::remote::Client {
        address: address.into(),
        ..client()
    }
}

#[test]
fn two_viewers_from_one_ip_are_listed_with_their_ports() {
    let page = remote_desktop(
        Area::MIN,
        Some(&serving(vec![
            viewer("192.168.1.30:51234"),
            viewer("192.168.1.30:51240"),
            viewer("192.168.1.31:40100"),
        ])),
        true,
    );
    let Control::ListBox { list, .. } = &page.item("clients").unwrap().control else {
        panic!("the clients are a list box");
    };
    assert_eq!(
        list.items,
        ["192.168.1.30:51234", "192.168.1.30:51240", "192.168.1.31"]
    );
}

fn pick_client(page: &mut Form, row: usize) {
    let Some(Control::ListBox { list, .. }) = page.item_mut("clients").map(|i| &mut i.control)
    else {
        panic!("the clients are a list box");
    };
    assert!(list.select(row, 2));
}

fn client_row(page: &Form) -> Option<usize> {
    match &page.item("clients").unwrap().control {
        Control::ListBox { list, .. } => list.selected,
        _ => panic!("the clients are a list box"),
    }
}

#[test]
fn the_selection_follows_its_address_when_a_row_above_goes() {
    let three = serving(vec![
        viewer("192.168.1.30:51234"),
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    let mut page = remote_desktop(Area::MIN, Some(&three), true);
    assert_eq!(
        client_row(&page),
        Some(0),
        "the first row unless one is picked"
    );
    pick_client(&mut page, 2);
    let keep = selected_client(&page, Some(&three)).map(str::to_owned);
    assert_eq!(keep.as_deref(), Some("192.168.1.31:40100"));
    let left = serving(vec![
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    show_remote(&mut page, Some(&left), true, None, keep.as_deref());
    assert_eq!(client_row(&page), Some(1), "the picked client is row 1");
    assert_eq!(
        selected_client(&page, Some(&left)),
        Some("192.168.1.31:40100")
    );
    assert!(lit(&page, "disconnect"));
    show_remote(
        &mut page,
        Some(&serving(Vec::new())),
        true,
        None,
        keep.as_deref(),
    );
    assert_eq!(client_row(&page), None);
    assert!(!lit(&page, "disconnect"));
}

#[test]
fn the_selection_follows_its_address_when_a_row_comes_above() {
    let two = serving(vec![
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    let mut page = remote_desktop(Area::MIN, Some(&two), true);
    pick_client(&mut page, 1);
    let keep = selected_client(&page, Some(&two)).map(str::to_owned);
    let three = serving(vec![
        viewer("192.168.1.30:51234"),
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    show_remote(&mut page, Some(&three), true, None, keep.as_deref());
    assert_eq!(client_row(&page), Some(2), "the picked client is row 2");
    assert_eq!(
        selected_client(&page, Some(&three)),
        Some("192.168.1.31:40100")
    );
}

#[test]
fn disconnect_names_the_selected_viewer() {
    let two = serving(vec![
        viewer("192.168.1.30:51234"),
        viewer("192.168.1.31:40100"),
    ]);
    let mut page = remote_desktop(Area::MIN, Some(&two), true);
    assert_eq!(
        crate::remotehost::disconnect(&page, Some(&two)),
        Some(crate::remotehost::Act::Disconnect(
            "192.168.1.30:51234".into()
        ))
    );
    pick_client(&mut page, 1);
    assert_eq!(
        crate::remotehost::disconnect(&page, Some(&two)),
        Some(crate::remotehost::Act::Disconnect(
            "192.168.1.31:40100".into()
        ))
    );
}

#[test]
fn port_quality_frame_rate_and_encoder_round_trip_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    let page = &mut s.pages[p.index()];
    if let Some(Control::UpDown(u)) = page.item_mut("port").map(|i| &mut i.control) {
        assert_eq!((u.min, u.max, u.value()), (1024, 65535, 3389));
        u.set(3390);
    }
    page.set_track("quality", 2);
    read_one(&s, "port", &mut got);
    read_one(&s, "quality", &mut got);
    pick(&mut s, &mut got, p, "frame_rate", 0);
    pick(&mut s, &mut got, p, "video_encoder", 4);
    assert!(
        matches!(
            &s.pages[p.index()].item("quality_value").unwrap().control,
            Control::Label { text, .. } if text == "Sharp"
        ),
        "the trackbar's stop is named above it"
    );
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"]
        .as_table()
        .expect("the section")
        .to_string();
    let rd = repriseconf::remote::from_table(&table.parse().unwrap());
    assert_eq!(
        (rd.port, rd.quality, rd.frame_rate, rd.encoder),
        (
            3390,
            repriseconf::remote::Quality::Sharp,
            30,
            repriseconf::remote::EncoderChoice::Software
        )
    );
    let back = crate::model::load(&out);
    let shown = with(&back);
    let page = &shown.pages[p.index()];
    assert_eq!(page.track_pos("quality"), Some(2));
    assert_eq!(page.combo_selected("frame_rate"), Some(0));
    assert_eq!(page.combo_selected("video_encoder"), Some(4));
}

#[test]
fn sound_on_the_remote_computer_round_trips_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    let page = &mut s.pages[p.index()];
    assert_eq!(page.checked("remote_sound"), Some(true), "on unless set");
    page.set_check("remote_sound", false);
    read_one(&s, "remote_sound", &mut got);
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert!(!rd.sound, "sound = false is written");
    let back = with(&crate::model::load(&out));
    assert_eq!(back.pages[p.index()].checked("remote_sound"), Some(false));
}

/// The page with one client, selected, and the defaults in its settings, Classic at 100%.
#[test]
fn the_remote_desktop_page_pixels() {
    if !reprisekit::text::test_tahoma() {
        return;
    }
    let theme = Theme {
        access_keys: true,
        ..Theme::default()
    };
    let text = TextRenderer::load(&FontConfig::default());
    let px = theme.fonts.size_px;
    let mut s = built();
    let (dx, dy) = s.page_offset();
    let page = &mut s.pages[Page::RemoteDesktop.index()];
    show_remote(
        page,
        Some(&serving(vec![client()])),
        true,
        Some("192.168.1.20"),
        None,
    );
    // The local time zone writes the time; the pins hold one.
    if let Some(Control::ListBox { list, .. }) = page.item_mut("clients").map(|i| &mut i.control) {
        list.detail = vec!["user, since 11/1/2026 12:26".into()];
    }
    let area = Rect::new(dx + 20, dy + 44, 358, 431);
    let mut c = Canvas::new(area.right() as u32, area.bottom() as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    form::draw(&mut c, &text, &theme.paint(), page, false, px);
    let h = c
        .copy_rect(area)
        .pixels
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, p| {
            (h ^ u64::from(*p)).wrapping_mul(0x100_0000_01b3)
        });
    assert_eq!(
        h, 13_272_056_036_377_627_569,
        "the page with its client selected"
    );
}
