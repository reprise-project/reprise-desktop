use super::*;
use crate::pages::system::NOTIFICATION_SOUND;

#[test]
fn the_sound_page_geometry() {
    let s = built();
    let p = Page::Sound;
    assert_eq!(rect(&s, p, AUDIO_DEVICES), Rect::new(268, 44, 110, 23));
    assert_eq!(rect(&s, p, "sound_theme_group"), Rect::new(20, 80, 358, 52));
    assert_eq!(rect(&s, p, "sound_theme_label"), Rect::new(GX, 106, 80, 13));
    assert_eq!(rect(&s, p, "sound_theme"), Rect::new(116, 102, 200, 21));
    assert_eq!(
        rect(&s, p, "sound_events_group"),
        Rect::new(20, 142, 358, 48)
    );
    assert_eq!(rect(&s, p, "bell"), Rect::new(GX, 164, 13, 13));
    assert_eq!(rect(&s, p, "volume_group"), Rect::new(20, 200, 358, 74));
    assert_eq!(rect(&s, p, "allow_boost"), Rect::new(GX, 222, 13, 13));
    assert_eq!(rect(&s, p, "allow_boost_note").y, 242);
    inside_page(&s, p);
}

#[test]
fn sound_opens_sounds_and_audio_devices() {
    let s = built();
    let p = Page::Sound;
    assert_eq!(
        lines(&s, p, "audio_devices_note"),
        [
            "To pick speakers and microphones or",
            "set each program's volume, open"
        ]
    );
    assert_eq!(program_for(AUDIO_DEVICES), Some("reprise-sound.desktop"));
    assert_eq!(program_for(EDIT_FILE), None);
}

#[test]
fn notifications_gather_on_their_own_page() {
    for id in [NOTIFICATION_SOUND, notifypage::PROGRAMS] {
        assert_moved(id, Page::Sound, Page::Notifications);
    }
    assert_moved(
        "glass_notification_style",
        Page::Appearance,
        Page::Notifications,
    );
    for i in ["classic", "glass"] {
        assert_eq!(
            keys_on(&under(i), Page::Notifications),
            [
                "glass.notification_style",
                "notifications.sound",
                "notifications.no_balloons",
                "notifications.no_sounds",
                "notifications.quiet_hours",
                "notifications.auto_dnd"
            ],
            "{i}"
        );
    }
    assert_eq!(
        keys_on(&built(), Page::Sound),
        ["sounds.theme", "sounds.bell", "sounds.allow_boost"]
    );
    for k in ["balloons", "do not disturb", "dnd", "quiet hours", "popups"] {
        assert!(Page::Notifications.keywords().contains(&k), "{k}");
    }
    assert_eq!(Page::named("notifications"), Some(Page::Notifications));
}

#[test]
fn the_notifications_page_geometry() {
    let s = built();
    let p = Page::Notifications;
    assert_eq!(rect(&s, p, "balloons_group"), Rect::new(20, 44, 358, 92));
    assert_eq!(rect(&s, p, NOTIFICATION_SOUND), Rect::new(GX, 66, 13, 13));
    assert_eq!(
        rect(&s, p, "glass_notification_style"),
        Rect::new(GX + 88, 88, 120, 21)
    );
    assert_eq!(
        rect(&s, p, notifypage::STYLE_NOTE),
        Rect::new(GX, 114, 334, 13)
    );
    assert_eq!(rect(&s, p, "dnd_group"), Rect::new(20, 146, 358, 116));
    assert_eq!(
        rect(&s, p, notifypage::AUTO_DND),
        Rect::new(GX, 168, 13, 13)
    );
    assert_eq!(rect(&s, p, notifypage::QUIET), Rect::new(GX, 192, 13, 13));
    assert_eq!(rect(&s, p, "quiet_from").y, 212);
    assert_eq!(rect(&s, p, "notify_group"), Rect::new(20, 272, 358, 144));
    assert_eq!(
        rect(&s, p, notifypage::PROGRAMS),
        Rect::new(GX, 292, 334, 89)
    );
    assert_eq!(
        rect(&s, p, notifypage::BALLOONS),
        Rect::new(GX, 389, 13, 13)
    );
    assert_eq!(
        rect(&s, p, notifypage::SOUNDS),
        Rect::new(GX + 140, 389, 13, 13)
    );
    inside_page(&s, p);
}

#[test]
fn quiet_hours_tick_to_both_times_and_untick_to_none() {
    let mut s = built();
    let mut got = Settings::default();
    assert!(got.quiet_hours.is_empty() && got.auto_dnd);
    follow(&mut s, &got);
    for id in notifypage::QUIET_TIMES {
        assert!(
            !enabled_on(&s, Page::Notifications, id),
            "{id} greyed while off"
        );
    }
    tick(
        &mut s,
        &mut got,
        Page::Notifications,
        notifypage::QUIET,
        true,
    );
    assert_eq!(got.quiet_hours, ["22:00", "07:00"]);
    for id in notifypage::QUIET_TIMES {
        assert!(enabled_on(&s, Page::Notifications, id), "{id} lit while on");
    }
    tick(
        &mut s,
        &mut got,
        Page::Notifications,
        notifypage::QUIET,
        false,
    );
    assert_eq!(got.quiet_hours, Vec::<String>::new());
    tick(
        &mut s,
        &mut got,
        Page::Notifications,
        notifypage::AUTO_DND,
        false,
    );
    assert!(!got.auto_dnd);
}

#[test]
fn quiet_hours_show_the_file_and_a_time_change_writes_its_end() {
    let mut got = Settings {
        quiet_hours: vec!["23:15".into(), "06:30".into()],
        ..Settings::default()
    };
    let mut s = with(&got);
    let p = Page::Notifications;
    assert_eq!(s.pages[p.index()].checked(notifypage::QUIET), Some(true));
    let to = notifypage::QUIET_TIMES[3];
    if let Some(Control::Time(t)) = s.pages[p.index()].item_mut(to).map(|i| &mut i.control) {
        assert_eq!((t.h, t.m), (6, 30));
        t.set(8, 0, 0);
    }
    read_one(&s, to, &mut got);
    assert_eq!(got.quiet_hours, ["23:15", "08:00"]);
}

#[test]
fn the_notification_style_greys_with_a_note_outside_glass() {
    let classic = under("classic");
    let p = Page::Notifications;
    assert!(!enabled_on(&classic, p, "glass_notification_style"));
    assert_eq!(
        lines(&classic, p, notifypage::STYLE_NOTE),
        ["Only Glass draws balloons in a style."]
    );
    assert!(enabled_on(&under("glass"), p, "glass_notification_style"));
}

#[test]
fn the_program_switches_pin_their_pixels_in_every_look() {
    let s = Settings {
        notify_no_sounds: vec!["chat".into()],
        ..Settings::default()
    };
    let mut f = notifications(Area::MIN);
    let programs = notifypage::programs(
        &[
            ("mail".into(), "Mail".into()),
            ("chat".into(), "Chat".into()),
        ],
        &s,
        |_| None,
    );
    notifypage::fill(&mut f, &programs, &s);
    let row = Rect::new(20, 272, 358, 144);
    let none = row_pins(&f, row);
    if let Some(form::Control::ListBox { list, .. }) =
        f.item_mut(notifypage::PROGRAMS).map(|i| &mut i.control)
    {
        list.selected = Some(1);
    }
    notifypage::fill(&mut f, &programs, &s);
    let chat = row_pins(&f, row);
    assert_eq!(
        [none, chat],
        [
            [0x7271412745a72c79, 0xfd6a7e8ff001b3f9, 0xb31b2628785111f8],
            [0xff6d03cb3a6174f0, 0xa243e9831ad6b73a, 0xc5e8178a57516dfb],
        ],
        "nothing picked, Chat picked; Classic, Blue, Glass"
    );
}

/// The theme combo keeps a configured theme that is missing and stays lit with the bell off:
/// notifications play from it too.
#[test]
fn the_bell_and_the_sound_theme_write_their_keys() {
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
            system_layout: "us",
            heads: &[],
        },
    );
    let p = Page::Sound;
    let id = "sound_theme";
    assert_eq!(
        rows(&s, p, id),
        (vec!["deep".into(), "ocean".into(), "gone".into()], 2)
    );
    assert_eq!(s.pages[p.index()].checked("bell"), Some(true));

    let mut got = want.clone();
    pick(&mut s, &mut got, p, id, 1);
    assert_eq!(got.sound_theme, "ocean");
    tick(&mut s, &mut got, p, "bell", false);
    assert!(!got.bell);
    tick(
        &mut s,
        &mut got,
        Page::Notifications,
        NOTIFICATION_SOUND,
        false,
    );
    assert!(!got.notification_sound);
    for id in ["sound_theme_label", id] {
        assert!(enabled_on(&s, p, id), "{id} stays lit");
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
fn advanced_groups_what_has_no_other_page() {
    assert_moved("restart_notice", Page::Startup, Page::Advanced);
    let s = built();
    let mut k = keys_on(&s, Page::Advanced);
    k.sort();
    assert_eq!(
        k,
        [
            ".task_manager",
            "debug.overlay",
            "debug.overlay_in_captures",
            "power.hibernate",
            "power.poweroff",
            "power.reboot",
            "power.standby",
            "screenshot.save",
            "storage.low_space_percent",
            "thumbnail_cache.enabled",
            "thumbnail_cache.external_thumbnailers",
            "thumbnail_cache.max_mb",
            "updates.restart_notice",
        ]
    );
    let p = Page::Advanced;
    assert_eq!(rect(&s, p, "task_manager"), Rect::new(20, 60, 358, 20));
    assert_eq!(rect(&s, p, "screenshot_save"), Rect::new(20, 90, 13, 13));
    assert_eq!(rect(&s, p, "restart_notice"), Rect::new(20, 108, 13, 13));
    assert_eq!(rect(&s, p, "turnoff_group"), Rect::new(20, 130, 358, 74));
    assert_eq!(rect(&s, p, "standby"), Rect::new(94, 148, 101, 20));
    assert_eq!(rect(&s, p, "poweroff"), Rect::new(265, 148, 101, 20));
    assert_eq!(rect(&s, p, "reboot"), Rect::new(94, 174, 101, 20));
    assert_eq!(rect(&s, p, "hibernate"), Rect::new(265, 174, 101, 20));
    let d = Settings::default();
    use reprisekit::interface::Interface;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = reprisekit::interfaces::theme_for(i);
        let text = TextRenderer::load(&theme.fonts);
        for (id, cmd) in [
            ("standby", &d.standby),
            ("poweroff", &d.poweroff),
            ("reboot", &d.reboot),
            ("hibernate", &d.hibernate),
        ] {
            let r = rect(&s, p, id);
            let mut c = Canvas::new(r.right() as u32, r.bottom() as u32);
            let inner = reprisekit::look::widgets::draw_edit_field(
                &mut c,
                &theme.paint(),
                r,
                theme.colors.window,
                false,
                reprisekit::look::widgets::Interaction::Idle,
            );
            assert!(
                text.measure(cmd, false, theme.fonts.size_px) <= inner.width,
                "{cmd} shows whole in {id} under {i:?}"
            );
        }
    }
    assert_eq!(rect(&s, p, "storage_group"), Rect::new(20, 212, 358, 66));
    assert_eq!(
        rect(&s, p, "low_space_percent"),
        Rect::new(150, 230, 50, 20)
    );
    assert_eq!(rect(&s, p, "low_space_note").y, 254);
    assert_eq!(rect(&s, p, "thumbs_group"), Rect::new(20, 286, 358, 92));
    assert_eq!(rect(&s, p, THUMBS), Rect::new(GX, 308, 13, 13));
    assert_eq!(rect(&s, p, THUMBS_CLEARED).y, 308);
    assert_eq!(rect(&s, p, THUMBS_CLEAR), Rect::new(291, 326, 75, 23));
    let mb = rect(&s, p, THUMBS_MB);
    assert_eq!(mb.y, 328);
    let units = rect(&s, p, THUMBS_SIZE[2]);
    assert_eq!((units.x, units.y), (mb.right() + 4, 332));
    assert_eq!(rect(&s, p, THUMBNAILERS), Rect::new(GX, 356, 13, 13));
    assert_eq!(rect(&s, p, DEBUG_OVERLAY), Rect::new(20, 386, 13, 13));
    assert_eq!(rect(&s, p, DEBUG_IN_CAPTURES), Rect::new(38, 404, 13, 13));
    assert_eq!(rect(&s, p, "note"), Rect::new(20, 422, 275, 26));
    assert_eq!(rect(&s, p, EDIT_FILE), Rect::new(303, 422, 75, 23));
    inside_page(&s, p);
}

#[test]
fn privacy_holds_location_history_and_the_sharing_border() {
    for id in [
        PRIVACY_LOCATION,
        FORGET,
        "recent_files",
        "clipboard_history",
        "capture_border",
    ] {
        assert_moved(id, Page::Advanced, Page::Privacy);
    }
    assert_eq!(
        keys_on(&built(), Page::Privacy),
        [
            "clipboard.history",
            "privacy.recent_files",
            "privacy.location",
            "privacy.capture_border"
        ]
    );
    for k in [
        "location",
        "clipboard",
        "recent files",
        "history",
        "screen sharing",
    ] {
        assert!(Page::Privacy.keywords().contains(&k), "{k}");
        assert!(!Page::Advanced.keywords().contains(&k), "{k}");
    }
    let s = built();
    let p = Page::Privacy;
    assert_eq!(rect(&s, p, "location_group"), Rect::new(20, 44, 358, 168));
    assert_eq!(rect(&s, p, PRIVACY_LOCATION), Rect::new(GX, 66, 13, 13));
    assert_eq!(locations(Area::MIN), Rect::new(GX, 88, 251, 114));
    assert_eq!(rect(&s, p, FORGET), Rect::new(291, 88, 75, 23));
    assert_eq!(rect(&s, p, "history_group"), Rect::new(20, 222, 358, 70));
    assert_eq!(rect(&s, p, "clipboard_history"), Rect::new(GX, 266, 13, 13));
    assert_eq!(rect(&s, p, "sharing_group"), Rect::new(20, 302, 358, 48));
    assert_eq!(rect(&s, p, "capture_border"), Rect::new(GX, 324, 13, 13));
    assert_eq!(LOCATION_COLUMNS[1].0, "Answer");
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
    reprisekit::pins::fonts();
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
            4_963_357_358_802_389_574,
            13_921_270_549_905_946_812,
            18_015_333_014_336_005_774
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
    tick(&mut s, &mut got, Page::Privacy, PRIVACY_LOCATION, false);
    assert!(!got.location);
    let mut lv = new_location_list();
    fit_location_columns(
        &mut lv,
        &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
        locations(Area::MIN),
    );
    let widths: Vec<i32> = lv.columns.iter().map(|c| c.width).collect();
    assert_eq!(widths, [161, 70]);
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
fn the_startup_page_geometry() {
    let s = built();
    let p = Page::Startup;
    assert_eq!(rect(&s, p, ADD), Rect::new(222, 311, 75, 23));
    assert_eq!(rect(&s, p, REMOVE), Rect::new(303, 311, 75, 23));
    assert_eq!(rect(&s, p, "session_group"), Rect::new(20, 343, 358, 66));
    assert_eq!(rect(&s, p, "session_restore"), Rect::new(32, 365, 13, 13));
    assert_eq!(
        rect(&s, p, "session_end_processes"),
        Rect::new(32, 387, 13, 13)
    );
    inside_page(&s, p);

    assert_eq!(list(Area::MIN), Rect::new(20, 44, 358, 259));
    let lv = new_list();
    let lay = listview::layout(
        &lv,
        list(Area::MIN),
        &reprisekit::controls::metrics::ControlMetrics::CLASSIC,
    );
    assert_eq!(lay.header, Rect::new(22, 46, 354, listview::HEADER_H));
    assert_eq!(lay.rows, 14, "14 rows of 17 px under the header");
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
fn the_saver_stands_beside_the_monitor_and_the_lock_group_under_them() {
    let s = built();
    let p = Page::ScreenSaverLock;
    assert_eq!(
        rect(&s, p, crate::saverpage::MONITOR),
        Rect::new(20, 44, 191, 168)
    );
    assert_eq!(rect(&s, p, "saver_label"), Rect::new(219, 44, 159, 13));
    assert_eq!(
        rect(&s, p, crate::saverpage::SAVER),
        Rect::new(219, 60, 159, 21)
    );
    assert_eq!(
        rect(&s, p, crate::saverpage::SETTINGS),
        Rect::new(219, 89, 75, 23)
    );
    assert_eq!(
        rect(&s, p, crate::saverpage::PREVIEW),
        Rect::new(303, 89, 75, 23)
    );
    assert_eq!(
        rect(&s, p, crate::saverpage::WAIT),
        Rect::new(263, 122, 50, 20)
    );
    assert_eq!(rect(&s, p, "wait_units"), Rect::new(319, 126, 59, 13));
    assert_eq!(rect(&s, p, "lock_group"), Rect::new(20, 222, 358, 160));
    assert_eq!(rect(&s, p, "lock_after"), Rect::new(150, 240, 50, 20));
    assert_eq!(rect(&s, p, "lock_units"), Rect::new(206, 244, 160, 13));
    assert_eq!(rect(&s, p, "lock_fade"), Rect::new(150, 264, 50, 20));
    assert_eq!(rect(&s, p, "lock_on_sleep"), Rect::new(32, 290, 13, 13));
    assert_eq!(rect(&s, p, "lock_wallpaper"), Rect::new(32, 312, 13, 13));
    assert_eq!(rect(&s, p, "away_label"), Rect::new(32, 334, 334, 13));
    assert_eq!(rect(&s, p, "away_device"), Rect::new(32, 350, 150, 21));
    assert_eq!(rect(&s, p, "away_after_s"), Rect::new(236, 350, 50, 20));
    assert_eq!(rect(&s, p, "away_units"), Rect::new(292, 354, 60, 13));
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
    reprisekit::pins::fonts();
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

/// Everything under the check box that greys with it.
const REMOTE_GREYED: [&str; 23] = [
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
    "remote_lock",
    "remote_border",
    "remote_screen_label",
    "remote_screen_show",
    "remote_screen_lock",
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
    assert_eq!(rect(&s, p, "port_label"), Rect::new(292, 44, 32, 13));
    assert_eq!(rect(&s, p, "port"), Rect::new(328, 40, 50, 20));
    assert_eq!(rect(&s, p, "allow_note"), Rect::new(38, 64, 340, 13));
    assert_eq!(rect(&s, p, "status"), Rect::new(38, 80, 340, 13));
    assert_eq!(
        rect(&s, p, "fingerprint_label"),
        Rect::new(20, 104, 126, 13)
    );
    assert_eq!(rect(&s, p, "fingerprint"), Rect::new(150, 100, 112, 20));
    assert_eq!(rect(&s, p, "regenerate"), Rect::new(268, 98, 110, 23));
    assert_eq!(rect(&s, p, "quality_label"), Rect::new(20, 130, 200, 13));
    assert_eq!(rect(&s, p, "quality_speed"), Rect::new(20, 157, 44, 13));
    assert_eq!(rect(&s, p, "quality"), Rect::new(66, 151, 262, 26));
    assert_eq!(rect(&s, p, "quality_sharp"), Rect::new(334, 157, 44, 13));
    assert_eq!(rect(&s, p, "quality_value"), Rect::new(298, 130, 80, 13));
    assert_eq!(rect(&s, p, "frame_rate_label"), Rect::new(20, 184, 160, 13));
    assert_eq!(
        rect(&s, p, "frame_rate"),
        Rect::new(20, 200, 160, combo::COMBO_H)
    );
    assert_eq!(
        rect(&s, p, "video_encoder_label"),
        Rect::new(244, 184, 134, 13)
    );
    assert_eq!(
        rect(&s, p, "video_encoder"),
        Rect::new(244, 200, 134, combo::COMBO_H)
    );
    assert_eq!(rect(&s, p, "remote_sound"), Rect::new(20, 230, 13, 13));
    assert_eq!(
        rect(&s, p, "encoder"),
        Rect::new(244, 230, 134, 13),
        "the encoder in use shows under the choice"
    );
    assert_eq!(rect(&s, p, "clients_label"), Rect::new(20, 252, 200, 13));
    assert_eq!(
        rect(&s, p, "clients"),
        Rect::new(20, 268, 277, 38),
        "two rows"
    );
    assert_eq!(rect(&s, p, "disconnect"), Rect::new(303, 268, 75, 23));
    assert_eq!(rect(&s, p, "remote_lock"), Rect::new(20, 312, 13, 13));
    assert_eq!(rect(&s, p, "lock_grace_label"), Rect::new(38, 333, 238, 13));
    assert_eq!(
        rect(&s, p, "lock_grace"),
        Rect::new(282, 329, 96, combo::COMBO_H)
    );
    assert_eq!(rect(&s, p, "remote_border"), Rect::new(20, 355, 13, 13));
    assert_eq!(
        rect(&s, p, "remote_screen_label"),
        Rect::new(20, 377, 358, 13)
    );
    assert_eq!(
        rect(&s, p, "remote_screen_show"),
        Rect::new(20, 395, 12, 12)
    );
    assert_eq!(
        rect(&s, p, "remote_screen_lock"),
        Rect::new(100, 395, 12, 12)
    );
    assert_eq!(
        rect(&s, p, "remote_screen_note"),
        Rect::new(38, 413, 340, 13)
    );
    assert_eq!(
        rect(&s, p, "second_client_label"),
        Rect::new(20, 431, 176, 13)
    );
    assert_eq!(
        rect(&s, p, "second_client_join"),
        Rect::new(202, 431, 12, 12)
    );
    assert_eq!(
        rect(&s, p, "second_client_replace"),
        Rect::new(272, 431, 12, 12),
        "the page ends by 450"
    );
    inside_page(&s, p);
    let page = &s.pages[p.index()];
    assert_eq!(
        rows(&s, p, "frame_rate"),
        (
            vec![
                "30 frames per second".into(),
                "60 frames per second".into(),
                "120 frames per second".into(),
                "144 frames per second".into(),
            ],
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
    assert_eq!(
        rows(&s, p, "lock_grace"),
        (
            vec![
                "Don't wait".into(),
                "5 seconds".into(),
                "10 seconds".into(),
                "30 seconds".into(),
            ],
            1
        ),
        "5 seconds unless set"
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
    let mut page = remote_desktop(Area::MIN, Some(&software_only), true, None);
    assert_eq!(
        encoders_lit(&page),
        [true, false, false, false, true, true],
        "Automatic, Software and None"
    );
    show_remote(&mut page, None, true, None, None, None);
    assert_eq!(
        encoders_lit(&page),
        [true, false, false, false, false, false],
        "only Automatic while the server does not answer"
    );
    assert_eq!(
        encoders_lit(&remote_desktop(
            Area::MIN,
            Some(&software_only),
            false,
            None
        )),
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
    let off = remote_desktop(Area::MIN, None, false, None);
    assert_eq!(off.checked("allow"), Some(false));
    assert!(enabled(&off, "allow"), "the check box itself stays lit");
    assert!(
        enabled(&off, "allow_note"),
        "the note under the check box stays lit"
    );
    for id in REMOTE_GREYED {
        assert!(
            !enabled(&off, id),
            "{id} greys while remote connections are off"
        );
    }
    let Control::Label { text, .. } = &off.item("status").unwrap().control else {
        panic!("the status is a label");
    };
    assert_eq!(text, "Off");

    let on = remote_desktop(Area::MIN, Some(&serving(vec![client()])), true, None);
    assert_eq!(on.checked("allow"), Some(true));
    for id in REMOTE_GREYED {
        assert!(enabled(&on, id), "{id} is lit while the server runs");
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
    let idle = remote_desktop(Area::MIN, Some(&serving(Vec::new())), true, None);
    assert!(!enabled(&idle, "disconnect"));
    assert!(enabled(&idle, "regenerate"));
    let busy = remote_desktop(Area::MIN, Some(&serving(vec![client()])), true, None);
    assert!(enabled(&busy, "disconnect"));
    let Control::ListBox { list, .. } = &busy.item("clients").unwrap().control else {
        panic!("the clients are a list box");
    };
    assert_eq!(list.items, ["192.168.1.30"], "the address without its port");
    assert!(
        list.detail[0].starts_with("user, since "),
        "{:?}",
        list.detail
    );
    let silent = remote_desktop(Area::MIN, None, true, None);
    for id in ["disconnect", "regenerate"] {
        assert!(
            !enabled(&silent, id),
            "{id} greys while the server does not answer"
        );
    }
    assert!(enabled(&silent, "port"), "the settings stay lit");
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
        None,
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
    let mut page = remote_desktop(Area::MIN, Some(&three), true, None);
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
    show_remote(&mut page, Some(&left), true, None, keep.as_deref(), None);
    assert_eq!(client_row(&page), Some(1), "the picked client is row 1");
    assert_eq!(
        selected_client(&page, Some(&left)),
        Some("192.168.1.31:40100")
    );
    assert!(enabled(&page, "disconnect"));
    show_remote(
        &mut page,
        Some(&serving(Vec::new())),
        true,
        None,
        keep.as_deref(),
        None,
    );
    assert_eq!(client_row(&page), None);
    assert!(!enabled(&page, "disconnect"));
}

#[test]
fn the_selection_follows_its_address_when_a_row_comes_above() {
    let two = serving(vec![
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    let mut page = remote_desktop(Area::MIN, Some(&two), true, None);
    pick_client(&mut page, 1);
    let keep = selected_client(&page, Some(&two)).map(str::to_owned);
    let three = serving(vec![
        viewer("192.168.1.30:51234"),
        viewer("192.168.1.30:51240"),
        viewer("192.168.1.31:40100"),
    ]);
    show_remote(&mut page, Some(&three), true, None, keep.as_deref(), None);
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
    let mut page = remote_desktop(Area::MIN, Some(&two), true, None);
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

#[test]
fn locking_when_the_last_remote_computer_leaves_round_trips_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    let page = &mut s.pages[p.index()];
    assert_eq!(page.checked("remote_lock"), Some(true), "on unless set");
    page.set_check("remote_lock", false);
    read_one(&s, "remote_lock", &mut got);
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert!(!rd.lock_on_leave, "lock_on_leave = false is written");
    let back = with(&crate::model::load(&out));
    assert_eq!(back.pages[p.index()].checked("remote_lock"), Some(false));
}

#[test]
fn the_wait_before_locking_round_trips_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    s.pages[p.index()].set_combo("lock_grace", 0);
    read_one(&s, "lock_grace", &mut got);
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert_eq!(rd.lock_grace_s, 0, "Don't wait writes lock_grace_s = 0");
    let back = with(&crate::model::load(&out));
    assert_eq!(back.pages[p.index()].combo_selected("lock_grace"), Some(0));
    let other = with(&crate::model::load("[remote_desktop]\nlock_grace_s = 15\n"));
    assert_eq!(
        rows(&other, p, "lock_grace"),
        (
            vec![
                "Don't wait".into(),
                "5 seconds".into(),
                "10 seconds".into(),
                "30 seconds".into(),
                "15".into(),
            ],
            4
        ),
        "a value the list lacks shows after it"
    );
}

#[test]
fn the_wait_before_locking_greys_with_the_lock_check_box_off() {
    let mut s = with(&crate::model::load(""));
    let p = Page::RemoteDesktop;
    let mut want = crate::model::load("");
    let status = serving(Vec::new());
    show_remote(
        &mut s.pages[p.index()],
        Some(&status),
        true,
        None,
        None,
        None,
    );
    for id in ["lock_grace_label", "lock_grace"] {
        assert!(enabled_on(&s, p, id), "{id} is lit with the lock on");
    }
    s.pages[p.index()].set_check("remote_lock", false);
    read_one(&s, "remote_lock", &mut want);
    crate::pages::follow(&mut s, &want);
    for id in ["lock_grace_label", "lock_grace"] {
        assert!(!enabled_on(&s, p, id), "{id} greys with the lock off");
    }
    show_remote(
        &mut s.pages[p.index()],
        Some(&status),
        true,
        None,
        None,
        None,
    );
    for id in ["lock_grace_label", "lock_grace"] {
        assert!(
            !enabled_on(&s, p, id),
            "{id} stays grey as the server's state is shown"
        );
    }
    s.pages[p.index()].set_check("remote_lock", true);
    show_remote(&mut s.pages[p.index()], None, false, None, None, None);
    for id in ["lock_grace_label", "lock_grace"] {
        assert!(
            !enabled_on(&s, p, id),
            "{id} greys with remote connections off"
        );
    }
}

#[test]
fn the_remote_desktop_border_round_trips_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    let page = &mut s.pages[p.index()];
    assert_eq!(page.checked("remote_border"), Some(false), "off unless set");
    page.set_check("remote_border", true);
    read_one(&s, "remote_border", &mut got);
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert!(rd.capture_border, "capture_border = true is written");
    let back = with(&crate::model::load(&out));
    assert_eq!(back.pages[p.index()].checked("remote_border"), Some(true));
}

#[test]
fn the_laptop_screen_radios_round_trip_through_apply() {
    let mut got = crate::model::load("");
    let base = got.clone();
    let mut s = with(&got);
    let p = Page::RemoteDesktop;
    let page = &mut s.pages[p.index()];
    assert_eq!(
        page.checked("remote_screen_lock"),
        Some(true),
        "lock unless set"
    );
    assert_eq!(page.checked("remote_screen_show"), Some(false));
    page.set_check("remote_screen_show", true);
    page.set_check("remote_screen_lock", false);
    read_one(&s, "remote_screen_show", &mut got);
    read_one(&s, "remote_screen_lock", &mut got);
    let out = crate::model::save("", &got, &base).expect("parses");
    let doc: toml_edit::DocumentMut = out.parse().unwrap();
    let table = doc["remote_desktop"].as_table().expect("the section");
    let rd = repriseconf::remote::from_table(&table.to_string().parse().unwrap());
    assert_eq!(
        rd.laptop_screen,
        repriseconf::remote::LaptopScreen::Show,
        "laptop_screen = \"show\" is written"
    );
    let back = with(&crate::model::load(&out));
    assert_eq!(
        back.pages[p.index()].checked("remote_screen_show"),
        Some(true)
    );
    assert_eq!(
        back.pages[p.index()].checked("remote_screen_lock"),
        Some(false)
    );
}

#[test]
fn the_lock_radio_greys_with_a_note_while_a_locker_is_set() {
    let note = "Lock mode is off while an external screen locker is set.";
    let locker = crate::model::load("[power]\nlocker = \"swaylock\"\n");
    let mut s = with(&locker);
    let p = Page::RemoteDesktop;
    let status = serving(Vec::new());
    show_remote(
        &mut s.pages[p.index()],
        Some(&status),
        true,
        None,
        None,
        None,
    );
    assert!(enabled_on(&s, p, "remote_screen_show"), "Stays on is lit");
    assert!(
        !enabled_on(&s, p, "remote_screen_lock"),
        "the Lock radio greys while a locker is set"
    );
    assert_eq!(lines(&s, p, "remote_screen_note"), [note]);
    assert!(
        enabled_on(&s, p, "remote_screen_note"),
        "its note says why, so it stays lit"
    );
    let none = crate::model::load("");
    crate::pages::follow(&mut s, &none);
    assert!(
        enabled_on(&s, p, "remote_screen_lock"),
        "lit with no other locker"
    );
    assert_eq!(lines(&s, p, "remote_screen_note"), [""], "no note then");
    crate::pages::follow(&mut s, &locker);
    show_remote(
        &mut s.pages[p.index()],
        Some(&status),
        true,
        None,
        None,
        None,
    );
    assert!(
        !enabled_on(&s, p, "remote_screen_lock"),
        "it stays grey as the server's state is shown"
    );
}

#[test]
fn a_blank_locker_leaves_the_lock_radio_lit() {
    let blank = crate::model::load("[power]\nlocker = \"  \"\n");
    let mut s = with(&blank);
    let p = Page::RemoteDesktop;
    show_remote(
        &mut s.pages[p.index()],
        Some(&serving(Vec::new())),
        true,
        None,
        None,
        None,
    );
    assert!(
        enabled_on(&s, p, "remote_screen_lock"),
        "a whitespace-only locker leaves the Lock radio lit"
    );
    assert_eq!(lines(&s, p, "remote_screen_note"), [""], "no note then");
}

/// The page with one client, selected, and the defaults in its settings, Classic at 100%.
#[test]
fn the_remote_desktop_page_pixels() {
    reprisekit::pins::fonts();
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
        h, 15_354_460_693_671_608_155,
        "the page with its client selected"
    );
}

#[test]
fn variable_refresh_rate_words_find_display() {
    for word in ["vrr", "adaptive sync", "freesync"] {
        assert!(Page::Display.find(word).is_some(), "{word}");
    }
}

/// The Remote Logins group: off with the front not running, then on with its fingerprint.
#[test]
fn the_remote_logins_group_pins_its_pixels_in_every_look() {
    let row = Rect::new(20, 152, 358, 158);
    let mut f = login_manager(Area::MIN);
    let off = row_pins(&f, row);
    let mut c = repriselogin::machine::MachineConfig::default();
    c.remote.enabled = true;
    show_login(&mut f, &c, &[], Some("3F:9A:01"));
    let on = row_pins(&f, row);
    assert_eq!(
        [off, on],
        [
            [0xacb5b86e525def62, 0xa43cab3129b6a3ed, 0xc74f9e48b0f27025],
            [0x2a0864c10b5a7905, 0x4b2cfd128d199ca1, 0x8610ba006abc8dfc],
        ],
        "off, on; Classic, Blue, Glass"
    );
}

/// The Login Screen group with the defaults, then Blue, no user list and an automatic logon.
#[test]
fn the_login_screen_group_pins_its_pixels_in_every_look() {
    let row = Rect::new(20, 44, 358, 98);
    let mut f = login_manager(Area::MIN);
    let unset = row_pins(&f, row);
    let mut c = repriselogin::machine::MachineConfig::default();
    c.login.interface = Some(repriselogin::Interface::Blue);
    c.login.show_users = false;
    c.login.autologin_user = Some("anna".into());
    show_login(&mut f, &c, &["anna".into()], None);
    let set = row_pins(&f, row);
    assert_eq!(
        [unset, set],
        [
            [0x66906fad015804f2, 0x1c36c36a279a0411, 0x9e3567829c5158a2],
            [0xd562090bc7a5d904, 0x5d593a36c3b4dce7, 0x47e809cee3925083],
        ],
        "defaults, set; Classic, Blue, Glass"
    );
}

/// The Remote Desktop page while the login manager runs remote logins, with its fingerprint,
/// before your Remote Desktop password's status is known, then not set and set.
#[test]
fn the_remote_desktop_page_under_the_login_manager_pins_its_pixels_in_every_look() {
    let machine = Some("3F:9A:01");
    let mut page = remote_desktop(Area::MIN, None, false, machine);
    show_remote(&mut page, None, false, None, None, machine);
    let row = Rect::new(20, 44, 358, 435);
    let unknown = row_pins(&page, row);
    let mut got = vec![unknown];
    for set in [false, true] {
        show_rd_password(&mut page, Some(set));
        got.push(row_pins(&page, row));
    }
    assert_eq!(
        got,
        [
            [0x825c80c18980d191, 0xe848d7d8954cb2ab, 0xc1d740305e6ff6d6],
            [0xef84a824d49cbf04, 0xa7a3385216bd0031, 0x29bf690dae6c3219],
            [0x262ba88709416289, 0x8e04bbad38017266, 0x9b06a2addc35c407],
        ],
        "unknown, not set, set; Classic, Blue, Glass"
    );
}
