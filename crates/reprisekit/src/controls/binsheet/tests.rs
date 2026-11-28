use super::*;
use crate::controls::form::Key;
use crate::interface::Interface;
use crate::raster::Canvas;
use crate::text::TextRenderer;
use crate::theme::Theme;

fn settings() -> Settings {
    Settings {
        confirm: true,
        independent: false,
        global: Rule {
            remove_immediately: false,
            max_percent: 10,
        },
        drives: BTreeMap::new(),
        older_than_days: 0,
    }
}

fn drive(key: &str, label: &str) -> Drive {
    Drive {
        key: key.into(),
        label: label.into(),
        size: 21_365_735_424,
    }
}

fn sheet() -> BinSheet {
    BinSheet::new(settings(), vec![drive("home", "Local Disk (/home)")])
}

/// Focuses `id` on the open page and presses `key` there.
fn press(s: &mut BinSheet, id: &'static str, key: Key) -> Outcome {
    s.sheet.open_mut().focus_id(id);
    let ev = s.sheet.key(key, None);
    s.event(ev)
}

fn label(f: &Form, id: &str) -> String {
    match &f.item(id).unwrap().control {
        Control::Label { text, .. } => text.clone(),
        _ => panic!("{id} is no label"),
    }
}

#[test]
fn a_landing_drive_tab_keeps_the_open_tab_and_the_boxes() {
    let home = drive("home", "Local Disk (/home)");
    let mut s = sheet();
    assert!(s.show_drives(vec![home.clone(), drive("b", "B")]));
    s.sheet.open_tab(2);
    s.pending.confirm = false;
    let all = vec![home.clone(), drive("a", "A"), drive("b", "B")];
    assert!(s.show_drives(all.clone()));
    assert!(!s.show_drives(all));
    assert_eq!(
        s.sheet.tabs.labels,
        ["Global", "Local Disk (/home)", "A", "B"]
    );
    assert_eq!(s.sheet.tabs.open, 3, "B's tab stays open");
    assert!(!s.pending.confirm, "what the boxes said");
    s.sheet.open_tab(2);
    assert!(s.show_drives(vec![home, drive("b", "B")]));
    assert_eq!(s.sheet.tabs.open, 0, "A's tab went: Global opens");
}

#[test]
fn every_page_fits_its_text_in_every_look_and_size() {
    let over = crate::interfaces::form_overflows(|_, _, _| {
        let s = sheet();
        let mut pages = vec![("global".to_string(), s.page_global())];
        pages.extend(s.drives.iter().map(|d| (d.key.clone(), s.page_drive(d))));
        pages
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

/// "Do not move files"'s second line starts a pixel right of the first in
/// Classic and under it in Blue and Glass, 16 px past the box (Glass's R may
/// ink a column later). Its "D" tops out 3 px below the box's top in Classic
/// and 2 px in Blue and Glass, and the second line's "R" 13 px below the "D".
/// The group's top line runs to x 29 and on past its label: from x 186 in
/// Classic and Blue, x 195 in Glass's wider face.
#[test]
fn the_do_not_move_second_line_follows_the_label_in_every_look() {
    crate::pins::fonts();
    let page = sheet().page_global();
    let ink = |c: &Canvas, r: Rect| {
        (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) & 0xFF < 0xA0))
    };
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let text = TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new(340, 260);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &page,
            false,
            theme.fonts.size_px,
        );
        let first = ink(&c, Rect::new(44, 93, 30, 13)).expect("the first line");
        let second = ink(&c, Rect::new(44, 106, 30, 13)).expect("the second line");
        let step = match i {
            Interface::Classic => 1,
            _ => 0,
        };
        assert_eq!(second, first + step, "{i:?}: the second line {step} px in");
        if i != Interface::Classic {
            assert!(
                (30 + 16..=30 + 17).contains(&second),
                "{i:?}: the second line 16 px past the box, its R's ink past its side bearing: {second}"
            );
        }
        let top = |x: i32, y: std::ops::Range<i32>| {
            y.into_iter()
                .find(|&y| (x..x + 6).any(|x| c.get(x, y) & 0xFF < 0xA0))
        };
        let (d_top, r_top) = match i {
            Interface::Classic => (96, 109),
            _ => (95, 108),
        };
        assert_eq!(top(first, 88..104), Some(d_top), "{i:?}: the D's top row");
        assert_eq!(top(second, 104..120), Some(r_top), "{i:?}: the R's top row");
        let line = c.get(250, 70);
        assert_eq!(c.get(29, 70), line, "{i:?}: the line up to x 29");
        let resume = match i {
            Interface::Glass => 195,
            _ => 186,
        };
        let found = (100..250).find(|&x| (x..250).all(|x| c.get(x, 70) == line));
        assert_eq!(found, Some(resume), "{i:?}: the line on from x {resume}");
    }
}

/// The live trackbar's thumb sits at the setting's percentage (10% on the
/// second tick, 50% on the sixth) with the percentage in the text colour;
/// greyed with its group, in the shadow colour.
#[test]
fn the_trackbar_and_its_percentage_follow_the_setting() {
    crate::pins::fonts();
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let draw = |s: &BinSheet| {
        let mut c = Canvas::new(340, 260);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let page = s.page_global();
        crate::controls::form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &page,
            false,
            theme.fonts.size_px,
        );
        c
    };
    let page = sheet().page_global();
    let bar = page.item("size").unwrap();
    let lay = match &bar.control {
        Control::TrackBar(t) => trackbar::layout(t, bar.rect.x, bar.rect.y, bar.rect.width),
        _ => panic!(),
    };
    assert_eq!(
        lay.thumb.x + lay.thumb.width / 2,
        lay.ticks[10],
        "10% on the second tick"
    );
    // The thumb covers the channel where it sits: 10% and 50% differ on both ticks' columns.
    let mut s = sheet();
    let ten = draw(&s);
    s.pending.global.max_percent = 50;
    let c = draw(&s);
    let row = lay.channel.y + 1;
    for x in [lay.ticks[10], lay.ticks[50]] {
        assert_ne!(ten.get(x, row), c.get(x, row), "x {x}");
    }
    assert_eq!(ten.get(lay.ticks[90], row), c.get(lay.ticks[90], row));
    let ink =
        |c: &Canvas, colour: u32| (176..189).any(|y| (139..180).any(|x| c.get(x, y) == colour));
    assert!(
        ink(&c, theme.colors.button_text.argb()),
        "the live percentage"
    );
    s.pending.independent = true;
    let c = draw(&s);
    assert!(
        !ink(&c, theme.colors.button_text.argb()),
        "greyed with its group"
    );
    assert!(ink(&c, theme.colors.button_shadow.argb()));
}

/// Four drives' tabs wrap onto a second row: the sheet grows 18 px, the page
/// and every control sit 18 px lower, and a rebuild keeps them there.
#[test]
fn a_second_row_of_tabs_grows_the_sheet_and_survives_a_rebuild() {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    let drives = ["/", "/boot", "/home", "/var"]
        .iter()
        .map(|m| drive(m, &format!("Local Disk ({m})")))
        .collect();
    let mut s = BinSheet::new(settings(), drives);
    s.sheet.fit_tabs(&text, px);
    s.sheet.fit_tabs(&text, px);
    assert_eq!(s.sheet.size, (338, 394));
    let page = Rect::new(6, 46, 326, 312);
    assert_eq!(s.sheet.layout(&text, px).page, page);
    let ok = |s: &BinSheet| s.sheet.pages[0].rect("ok").unwrap();
    assert_eq!(ok(&s), Rect::new(95, 364, 75, 23));
    s.rebuild();
    assert_eq!(ok(&s), Rect::new(95, 364, 75, 23));
    assert_eq!(
        s.sheet.pages[0].rect("confirm").unwrap(),
        Rect::new(30, 260, 13, 13)
    );
    let l = s.sheet.layout(&text, px);
    assert!(
        l.tabs
            .iter()
            .all(|r| r.x >= page.x && r.right() <= page.right())
    );
    assert_eq!(l.tabs[0].bottom(), page.y, "the open tab meets the page");
}

#[test]
fn pages_sit_on_their_pixels() {
    let s = sheet();
    let pages = &s.sheet.pages;
    assert_eq!(pages.len(), 2);
    let g = &pages[0];
    let rect = |id: &str| g.rect(id).unwrap();
    assert_eq!(rect("independent"), Rect::new(31, 42, 12, 12));
    assert_eq!(rect("all"), Rect::new(31, 64, 12, 12));
    assert_eq!(rect("group"), Rect::new(21, 70, 296, 161));
    assert_eq!(rect("no_bin"), Rect::new(30, 93, 13, 13));
    assert_eq!(rect("confirm"), Rect::new(30, 242, 13, 13));
    // The trackbar's channel runs x 54 to 264 at y 150, 7 rows under the control's top, ticks 59 to 259.
    let bar = g.item("size").unwrap();
    let r = g.rect("size").unwrap();
    let lay = match &bar.control {
        Control::TrackBar(t) => trackbar::layout(t, r.x, r.y, r.width),
        _ => panic!(),
    };
    assert_eq!(lay.channel, Rect::new(54, 150, 211, 4));
    assert_eq!(lay.ticks.first(), Some(&59));
    assert_eq!(lay.ticks.last(), Some(&259));
    assert!(matches!(
        bar.control,
        Control::TrackBar(TrackBar {
            pos: 10,
            count: 101,
            enabled: true,
            ..
        })
    ));
    assert!(matches!(
        g.item("all").unwrap().control,
        Control::Radio {
            checked: true,
            enabled: true,
            ..
        }
    ));
    assert!(matches!(
        g.item("confirm").unwrap().control,
        Control::Check {
            checked: true,
            enabled: true,
            ..
        }
    ));
    assert!(matches!(
        g.item("apply").unwrap().control,
        Control::Button { enabled: false, .. }
    ));
    assert_eq!(rect("ok"), Rect::new(95, 346, 75, 23));
    assert_eq!(rect("cancel"), Rect::new(176, 346, 75, 23));
    assert_eq!(rect("apply"), Rect::new(257, 346, 75, 23));
    // The drive's page: sizes at x 153, every control greyed while one setting serves every drive.
    let d = &pages[1];
    assert_eq!(label(d, "size_of"), "19.8 GB");
    assert_eq!(label(d, "reserved"), "1.98 GB");
    assert!(matches!(
        d.item("no_bin").unwrap().control,
        Control::Check { enabled: false, .. }
    ));
    assert_eq!(d.rect("no_bin").unwrap().x, 21);
    assert_eq!(d.rect("size_of").unwrap(), Rect::new(153, 41, 160, 13));
    assert!(d.item("confirm").is_none());
}

#[test]
fn boxes_feed_the_settings_and_ok_applies_them() {
    let mut s = sheet();
    assert_eq!(press(&mut s, "no_bin", Key::Space), Outcome::Redraw);
    assert_eq!(press(&mut s, "confirm", Key::Space), Outcome::Redraw);
    let mut want = Settings {
        confirm: false,
        ..settings()
    };
    want.global.remove_immediately = true;
    assert_eq!(s.pending, want);
    let page = s.sheet.open();
    assert!(matches!(
        page.item("apply").unwrap().control,
        Control::Button { enabled: true, .. }
    ));
    assert!(
        matches!(
            page.item("size").unwrap().control,
            Control::TrackBar(TrackBar { enabled: false, .. })
        ),
        "no size while files are not moved"
    );
    assert_eq!(
        s.event(propsheet::Event::Apply),
        Outcome::Save {
            settings: want.clone(),
            close: false
        }
    );
    assert!(matches!(
        s.sheet.open().item("apply").unwrap().control,
        Control::Button { enabled: false, .. }
    ));
    assert_eq!(
        s.event(propsheet::Event::Ok),
        Outcome::Save {
            settings: want,
            close: true
        }
    );
    assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
}

/// "Delete items older than" turns the days box on at 30; its arrows step the days, which the
/// box keeps while unticked.
#[test]
fn the_age_box_sets_the_days_old_items_are_kept() {
    let days = |s: &BinSheet| match &s.sheet.pages[0].item("days").unwrap().control {
        Control::UpDown(u) => (u.value(), u.enabled),
        _ => panic!(),
    };
    let mut s = sheet();
    assert_eq!(days(&s), (30, false), "greyed at 30 while off");
    press(&mut s, "aging", Key::Space);
    assert_eq!(s.pending.older_than_days, 30);
    assert_eq!(days(&s), (30, true));
    assert_eq!(press(&mut s, "days", Key::Up), Outcome::Redraw);
    assert_eq!(s.pending.older_than_days, 31);
    press(&mut s, "aging", Key::Space);
    assert_eq!(s.pending.older_than_days, 0);
    assert_eq!(days(&s), (31, false));
    let Outcome::Save {
        settings: saved, ..
    } = s.event(propsheet::Event::Ok)
    else {
        panic!()
    };
    assert_eq!(saved.older_than_days, 0);

    let set = Settings {
        older_than_days: 7,
        ..settings()
    };
    assert_eq!(days(&BinSheet::new(set, vec![])), (7, true));
}

/// The trackbar's keys move the cap a percent at a time, ten with Page Down; the labels follow.
#[test]
fn the_trackbar_sets_the_percentage() {
    let mut s = sheet();
    press(&mut s, "size", Key::Left);
    assert_eq!(s.pending.global.max_percent, 9);
    press(&mut s, "size", Key::PageDown);
    assert_eq!(s.pending.global.max_percent, 19);
    assert_eq!(label(&s.sheet.pages[0], "pct"), "19%");
    assert_eq!(label(&s.sheet.pages[1], "reserved"), "3.78 GB");
}

/// "Configure drives independently" greys Global's group and makes each
/// drive's tab live; a drive's setting starts from the global one.
#[test]
fn independent_drives_take_their_own_settings() {
    let drives = vec![drive("home", "Local Disk (/)"), drive("5A1F", "STICK")];
    let mut s = BinSheet::new(settings(), drives);
    s.sheet.open_mut().focus_id("all");
    let ev = s.sheet.key(Key::Up, None);
    assert_eq!(
        ev,
        propsheet::Event::Page(0, FormEvent::Changed("independent"))
    );
    s.event(ev);
    assert!(s.pending.independent);
    assert!(matches!(
        s.sheet.pages[0].item("no_bin").unwrap().control,
        Control::Check { enabled: false, .. }
    ));
    assert!(matches!(
        s.sheet.pages[2].item("no_bin").unwrap().control,
        Control::Check { enabled: true, .. }
    ));

    s.sheet.open_tab(2);
    press(&mut s, "no_bin", Key::Space);
    let p = &s.pending;
    assert!(p.rule("5A1F").remove_immediately);
    assert!(!p.rule("home").remove_immediately);
    assert_eq!(p.drives.len(), 1, "only the drive that changed");
    assert_eq!(label(&s.sheet.pages[2], "reserved"), "0 bytes");
}

#[test]
fn the_sheet_is_338x376_with_a_tab_per_drive() {
    let theme = Theme::default();
    let text = TextRenderer::load(&theme.fonts);
    let s = sheet();
    assert_eq!(s.sheet.size, (338, 376));
    let l = s.sheet.layout(&text, theme.fonts.size_px);
    assert_eq!(l.strip, (6, 8));
    assert_eq!(l.page, Rect::new(6, 28, 326, 312));
    assert_eq!(l.tabs.len(), 2);
}

/// Global as it opens and a drive's tab with the drives set apart, in each look at scale 1 and 2.
#[test]
fn the_sheet_draws_its_pins() {
    let drives = vec![drive("home", "Local Disk (/home)"), drive("5A1F", "STICK")];
    let independent = Settings {
        independent: true,
        ..settings()
    };
    let states = [
        ("global", BinSheet::new(settings(), drives.clone()), 0),
        ("drive", BinSheet::new(independent, drives), 2),
    ];
    let mut got = Vec::new();
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for (name, s, tab) in &states {
            for scale in [1, 2] {
                let theme = Theme {
                    access_keys: true,
                    ..crate::interfaces::theme_for(look).scaled(scale)
                };
                let text = TextRenderer::load(&theme.fonts);
                let p = theme.paint();
                let mut s = s.clone();
                s.sheet.open_tab(*tab);
                let (w, h) = s.sheet.size;
                let mut c = Canvas::new((w * scale) as u32, (h * scale) as u32);
                c.fill_rect(c.bounds(), p.cs.button_face);
                propsheet::draw(&mut c, &text, &p, &s.sheet, true, theme.fonts.size_px);
                got.push((format!("{look:?} {name} {scale}"), crate::pins::fnv(&c)));
            }
        }
    }
    crate::pins::check("binsheet", &got);
}
