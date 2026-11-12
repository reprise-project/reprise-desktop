//! The main page's shell and desk sections.

use super::cued;
use super::page::swatch16;
use super::popups::{display_lines, display_panel};
use reprisekit::art::signalbars;
use reprisekit::classic::frame::{
    CaptionButton, Chrome, FrameGeometry, FrameState, Glow, draw_frame,
};
// Classic helpers with no look::widgets twin.
use reprisekit::classic::widgets::{ROW_H, draw_edit};
use reprisekit::classic::{indicator, snapframe};
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::menu::MenuItem;
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::treeview::{self, Node, TreeView};
use reprisekit::controls::{
    browsefolder, colorbutton, colorpick, displaypanel, filesheet, grouplist, launcher, multiedit,
    notifyhistory, osd, regionselect, replacebox, switcher, taskpreview, tooltip, updown, wizard,
};
use reprisekit::interface::Interface;
use reprisekit::look::widgets::draw_sunken_field;
use reprisekit::look::{self, Paint, sample};
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::textedit::{Caret, EditKey, Preedit, TextEdit};
use reprisekit::theme::{Color, Theme};
use reprisekit::{interfaces, schemes};

/// Network icons, OSD panels, launcher, pickers, notifications, frames, menu bar and status bar.
pub fn page_shell(mut c: Canvas, theme: &Theme, text: &TextRenderer, with_blue: bool) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    {
        use reprisekit::art::neticon::{NetIcon, draw, draw32};
        let icons = [
            NetIcon::Wired,
            NetIcon::WiredUnplugged,
            NetIcon::Wireless(3),
            NetIcon::WirelessOff,
            NetIcon::Connecting,
        ];
        c.fill_rect(Rect::new(10, 3005, 410, 62), cs.button_face);
        for (i, icon) in icons.iter().enumerate() {
            let x16 = 16 + 40 * i as i32;
            c.blit_canvas(x16, 3011, &draw(*icon, false));
            c.blit_canvas(x16 + 20, 3011, &draw(*icon, true));
            let x32 = 16 + 76 * i as i32;
            c.blit_canvas(x32, 3031, &draw32(*icon, false));
            c.blit_canvas(x32 + 38, 3031, &draw32(*icon, true));
        }
        for (i, (icon, vpn)) in [(NetIcon::Wired, false), (NetIcon::Wireless(3), true)]
            .into_iter()
            .enumerate()
        {
            let mut badged = draw(icon, vpn);
            reprisekit::art::neticon::badge_limited(&mut badged);
            c.blit_canvas(216 + 20 * i as i32, 3011, &badged);
        }
    }

    {
        c.fill_rect(Rect::new(10, 3075, 410, 62), cs.button_face);
        for n in 0..=5u8 {
            signalbars::draw(&mut c, 16 + 28 * n as i32, 3081, n, true);
        }
        signalbars::draw(&mut c, 16 + 28 * 6, 3081, 5, false);
        let mut secret = TextEdit::secret();
        secret.paste("correcthorse", &[]);
        let mut kept = TextEdit::secret();
        kept.set_cue("Leave empty to keep the saved key");
        let key = form::Form::new(vec![
            Item::edit(
                "fingerprint",
                Rect::new(216, 3079, 198, 20),
                "3F:9A:0C:51:E2:7B:44:D8:19:A6",
                true,
            )
            .read_only(),
            Item::new(
                "key",
                Rect::new(16, 3105, 150, 20),
                Control::Edit {
                    edit: secret,
                    first: std::cell::Cell::new(0),
                    enabled: true,
                },
            ),
            Item::new(
                "kept",
                Rect::new(176, 3105, 238, 20),
                Control::Edit {
                    edit: kept,
                    first: std::cell::Cell::new(0),
                    enabled: true,
                },
            ),
        ]);
        form::draw(&mut c, text, &Paint { s, ..theme.paint() }, &key, true, px);
    }

    let osds = [
        ("Volume", Some(osd::Meter::new(0.45, true))),
        ("Muted", Some(osd::Meter::new(0.45, false))),
        ("Caps Lock On", None),
        ("Music Player: Next", None),
    ];
    for (i, (title, meter)) in osds.iter().enumerate() {
        let panel = match *title {
            "Caps Lock On" => osd::draw_lock_key(theme, text, osd::LockKey::Caps, true),
            _ => osd::draw(
                theme,
                text,
                &osd::Osd {
                    title: title.to_string(),
                    meter: *meter,
                },
                None,
            ),
        };
        c.blit_canvas(
            20 + (i as i32 % 2) * 280,
            3145 + (i as i32 / 2) * 80,
            &panel,
        );
    }

    c.blit_canvas(20, 6190, &osd::draw_lines(theme, text, &display_lines()));
    c.blit_canvas(
        20,
        6320,
        &displaypanel::draw(theme, text, &display_panel(0)),
    );

    {
        let l = switcher::SwitcherLayout::compute(3, 1, Rect::new(580, 3150, 330, 107), false);
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(2, 4, 28, 24), Color::rgb(80, 193, 255));
        switcher::draw(
            &mut panel,
            theme,
            text,
            &l,
            "Documents",
            &[Some(icon.clone()), Some(icon), None],
            1,
            None,
            None,
        );
        c.blit_canvas(l.rect.x, l.rect.y, &panel);
    }

    {
        let mut l = launcher::Launcher::new();
        l.key(launcher::Key::Char(EditKey::Insert("no".into())));
        l.set_rows(vec![launcher::Row {
            name: "Notepad".to_string(),
            icon: None,
        }]);
        l.set_files(vec![launcher::FileRow {
            name: "notes.txt".to_string(),
            folder: "~/Documents".to_string(),
            icon: None,
        }]);
        let panel = launcher::draw(theme, text, &l, None, true, None);
        c.blit_canvas(20, 3305, &panel);
    }

    {
        use reprisekit::controls::cliphistory::{self, ClipHistory, Row};
        let mut thumb = Canvas::new(16, 16);
        thumb.fill_rect(Rect::new(0, 3, 16, 10), Color::rgb(58, 110, 165));
        let rows = [
            ("cargo build --release", None),
            ("Image, 1920x1080", Some(thumb)),
            ("hello world", None),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (label, icon))| Row {
            id: i as u64,
            label: label.into(),
            icon,
        })
        .collect();
        let m = ClipHistory::new(rows);
        c.blit_canvas(540, 5800, &cliphistory::draw(theme, text, &m, true, None));
    }

    {
        use reprisekit::controls::emojipicker::{self, EmojiPicker, Key};
        use reprisekit::emoji::{EmojiFont, Entry, catalog};
        let font = EmojiFont::load();
        let recent = ["👍", "🎉"].into_iter().filter_map(Entry::get).collect();
        let mut m = EmojiPicker::new(recent, catalog(font.as_ref()));
        m.key(Key::Down);
        let panel = emojipicker::draw(theme, text, font.as_ref(), &m, true, None);
        c.blit_canvas(560, 5962, &panel);
    }

    {
        c.fill_rect(Rect::new(10, 3425, 410, 120), cs.button_face);
        c.blit_canvas(20, 3435, &snapframe::draw(200, 100, snapframe::BAND, 1));
    }

    {
        let lay = tooltip::notify_layout(
            theme,
            text,
            "Updates are ready",
            "Click here to install them.",
            &["Install", "Later"],
            &[],
            None,
        );
        let pushed = Some(tooltip::NotifyHit::Button(0));
        let reply = tooltip::notify_layout(
            theme,
            text,
            "Ana",
            "Lunch on Friday?",
            &["Reply"],
            &[],
            Some(tooltip::NotifyReply {
                placeholder: "Write a reply",
                send: "Send",
            }),
        );
        let typed = reprisekit::textedit::TextEdit::new("Sounds good", false);
        let caret = reprisekit::textedit::Caret::Shown;
        c.fill_rect(Rect::new(0, 3545, 330, 235), cs.button_face);
        c.blit_over(
            10,
            3550,
            &tooltip::draw_notify_balloon(theme, text, &lay, None, &[], pushed, None, None),
        );
        let open = tooltip::draw_notify_balloon(
            theme,
            text,
            &reply,
            None,
            &[],
            None,
            None,
            Some((&typed, caret, None)),
        );
        c.blit_over(10, 3556 + lay.height, &open);
    }

    {
        use reprisekit::controls::grouplist::{GroupList, Row};
        c.fill_rect(Rect::new(340, 3545, 360, 230), cs.button_face);
        let item = |name: &str| Row::Item {
            name: name.into(),
            data: (),
        };
        let mut rows = vec![
            Row::Group("Recommended Programs:".into()),
            item("Notepad"),
            item("Text Editor"),
            Row::Group("Other Programs:".into()),
        ];
        rows.extend((1..=10).map(|i| item(&format!("Program {i}"))));
        let mut list = GroupList::new(rows);
        list.select(2, 11);
        let lay = grouplist::layout(
            &list,
            Rect::new(350, 3555, 338, 210),
            &theme.look.controls(),
        );
        let icon = swatch16(Color::rgb(80, 193, 255));
        let icons: Vec<Option<&Canvas>> = (0..list.rows.len()).map(|_| Some(&icon)).collect();
        grouplist::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &list,
            &lay,
            &icons,
            grouplist::Shown::Focused,
            px,
        );
    }

    {
        use reprisekit::controls::openwith::{OpenWith, Program};
        let p = |id: &str, name: &str| Program {
            id: id.into(),
            name: name.into(),
            icon: None,
        };
        let mut ow = OpenWith::new(
            "New Text Document.txt",
            vec![p("notepad", "Notepad"), p("texteditor", "Text Editor")],
            vec![
                p("imageeditor", "Image Editor"),
                p("imageviewer", "Image Viewer"),
            ],
        );
        ow.select_id("texteditor");
        let (x, y) = (350, 8596);
        let lay = ow.layout(text, px, (x, y), &theme.look.controls());
        ow.draw(
            &mut c,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &lay,
            None,
            &[],
            reprisekit::cues::Cues::NONE,
            px,
        );
    }

    c.fill_rect(Rect::new(0, 3780, 950, 150), cs.background);
    for (x, chrome, title, active, pressed) in [
        (
            10,
            Chrome::WINDOW,
            "Untitled - Notepad",
            true,
            Some(CaptionButton::Close),
        ),
        (360, Chrome::DIALOG, "Run", false, None),
    ] {
        let g =
            FrameGeometry::layout(&theme.metrics, 300, 100, false, chrome).translated(-x, -3790);
        draw_frame(
            &mut c,
            theme,
            text,
            &g,
            &FrameState {
                title,
                active,
                maximized: false,
                pressed,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        c.fill_rect(g.client, cs.window);
    }

    if with_blue {
        let blue_theme = cued(interfaces::theme_for(Interface::Blue));
        let style = blue_theme.look.style().unwrap();
        let caption_text = TextRenderer::load(&style.caption_text.font());
        let bcs = &blue_theme.colors;
        let band = Rect::new(0, 9050, 950, 240);
        c.fill_rect(band, bcs.background);
        for (i, (active, pressed)) in [
            (true, None),
            (false, None),
            (true, Some(CaptionButton::Close)),
        ]
        .into_iter()
        .enumerate()
        {
            let x = 10 + i as i32 * 240;
            let y = band.y + 10;
            let g = FrameGeometry::layout(&blue_theme.metrics, 220, 80, false, Chrome::WINDOW)
                .translated(-x, -y);
            look::draw_frame(
                &mut c,
                &blue_theme,
                text,
                &caption_text,
                &g,
                &FrameState {
                    title: "Notes",
                    active,
                    maximized: false,
                    pressed,
                    glow: Glow::lit(None),
                    icon: None,
                    tabbed: false,
                },
            );
            c.fill_rect(g.client, bcs.window);
        }
        let g = FrameGeometry::layout(&blue_theme.metrics, 320, 60, true, Chrome::WINDOW)
            .translated(-10, -(band.y + 134));
        look::draw_frame(
            &mut c,
            &blue_theme,
            text,
            &caption_text,
            &g,
            &FrameState {
                title: "Notes",
                active: true,
                maximized: true,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        c.fill_rect(g.client, bcs.window);
        for (i, active) in [true, false].into_iter().enumerate() {
            let x = 350 + i as i32 * 300;
            let dialog = Chrome::DIALOG;
            let g = FrameGeometry::layout(&blue_theme.metrics, 280, 50, false, dialog)
                .translated(-x, -(band.y + 134));
            look::draw_dialog_frame(
                &mut c,
                &blue_theme,
                text,
                &g,
                &FrameState {
                    title: "Run",
                    active,
                    maximized: false,
                    pressed: None,
                    glow: Glow::lit(active.then_some(CaptionButton::Close)),
                    icon: None,
                    tabbed: false,
                },
                active.then_some(Interaction::Idle),
            );
        }
    }

    c.fill_rect(Rect::new(0, 3930, 950, 678), cs.background);
    {
        use reprisekit::controls::menubar::Bar;
        let mut bar = Bar::new(vec![
            (
                "&File",
                vec![
                    MenuItem::item("&New"),
                    MenuItem::item("&Open..."),
                    MenuItem::item("&Save"),
                    MenuItem::Separator,
                    MenuItem::item("E&xit"),
                ],
            ),
            ("&Edit", vec![MenuItem::item("&Undo")]),
            ("F&ormat", vec![MenuItem::item("&Word Wrap")]),
            ("&View", vec![MenuItem::item("&Status Bar")]),
            ("&Help", vec![MenuItem::item("&About Notepad")]),
        ]);
        let client = Rect::new(10, 3940, 400, 170);
        c.fill_rect(client, cs.window);
        let lay = bar.layout(text, px, client, &theme.look.controls());
        bar.open_title(0, &lay, text, theme, px, true);
        bar.underline = true;
        bar.draw(&mut c, text, theme, &lay);
    }
    {
        let mut b = colorbutton::ColorButton::new(cs.background, schemes::PALETTE.to_vec());
        b.toggle();
        let f = form::Form::new(vec![Item::new(
            "color",
            Rect::new(440, 3940, 75, 21),
            Control::Color(b),
        )]);
        form::draw(&mut c, text, &Paint { s, ..theme.paint() }, &f, true, px);
    }
    c.blit_canvas(
        580,
        3940,
        &tooltip::draw_info_tip(
            theme,
            text,
            "Displays the properties of the selected items, such as their size and type.",
        ),
    );

    {
        let facts = filesheet::Facts {
            is_dir: false,
            type_name: "Text Document".into(),
            opens_with: Some("Notepad".into()),
            description: None,
            location: "/home/user/Documents".into(),
            size: "1.00 KB (1,024 bytes)".into(),
            on_disk: "4.00 KB (4,096 bytes)".into(),
            contains: None,
            created: Some("Today, September 23, 2026, 9:41:07 AM".into()),
            modified: "Today, September 23, 2026, 9:41:07 AM".into(),
            accessed: "Today, September 23, 2026, 10:02:15 AM".into(),
            read_only: true,
            executable: false,
            can_set_executable: true,
            hidden: false,
        };
        let (page, height) =
            filesheet::general_page(&filesheet::rows(&facts), "readme.txt", None, None);
        let general = PropSheet::new(&["General"], vec![page], (filesheet::SHEET_W, height));
        let (gw, gh) = general.size;
        let mut general_c = Canvas::new(gw as u32, gh as u32);
        general_c.fill_rect(general_c.bounds(), cs.button_face);
        propsheet::draw(
            &mut general_c,
            text,
            &Paint { s, ..theme.paint() },
            &general,
            true,
            px,
        );
        c.blit_canvas(10, 4120, &general_c);

        let (aw, ah) = filesheet::ATTR_SIZE;
        let mut attr_c = Canvas::new(aw as u32, ah as u32);
        attr_c.fill_rect(attr_c.bounds(), cs.button_face);
        let attr = filesheet::attr_form(
            "set read-only",
            filesheet::Subject::Folder,
            text,
            px,
            reprisekit::theme::TextUnits::ONE,
        );
        form::draw(
            &mut attr_c,
            text,
            &Paint { s, ..theme.paint() },
            &attr,
            true,
            px,
        );
        c.blit_canvas(400, 4120, &attr_c);

        for (x, is_dir) in [(10, true), (400, false)] {
            let subject = filesheet::Subject::single(is_dir);
            let (w, h) = filesheet::advanced_size(subject);
            let mut adv_c = Canvas::new(w as u32, h as u32);
            adv_c.fill_rect(adv_c.bounds(), cs.button_face);
            let robots = is_dir.then_some([
                reprisekit::classic::widgets::CheckState::Mixed,
                reprisekit::classic::widgets::CheckState::On,
            ]);
            let adv = filesheet::advanced_form(
                subject,
                None,
                robots,
                text,
                px,
                reprisekit::theme::TextUnits::ONE,
            );
            form::draw(
                &mut adv_c,
                text,
                &Paint { s, ..theme.paint() },
                &adv,
                true,
                px,
            );
            c.blit_canvas(x, 4630, &adv_c);
        }
    }
    c.blit_canvas(400, 4346, &sample::draw_themes(theme, text, None, None));

    let bar = StatusBar {
        panels: vec![
            Panel {
                text: "4 objects".into(),
                width: 0,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
            Panel {
                text: String::new(),
                width: 76,
                pad: statusbar::TEXT_PAD,
                icon: None,
            },
            Panel {
                text: "Computer".into(),
                width: 153,
                pad: statusbar::TEXT_PAD,
                icon: Some(swatch16(Color::rgb(80, 193, 255))),
            },
        ],
        grip: true,
    };
    let bar_r = Rect::new(
        0,
        c.height as i32 - statusbar::HEIGHT,
        c.width as i32,
        statusbar::HEIGHT,
    );
    statusbar::draw(&mut c, text, &Paint { s, ..theme.paint() }, &bar, bar_r, px);

    c
}

/// Tile list, device art, wizard, clock and calendar, hot key, indicators, switcher and IME.
pub fn page_desk(
    mut c: Canvas,
    theme: &Theme,
    text: &TextRenderer,
    mono: &TextRenderer,
    with_blue: bool,
) -> Canvas {
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = 1;

    {
        use reprisekit::controls::tilelist::{self, Entry, TILE_H, Tile, TileList};
        let rows = [
            ("Headphones", "Connected, battery 80%", "Connected", false),
            ("Speaker", "Not connected", "", false),
            ("Game controller", "Not connected", "", true),
        ];
        let mut list = TileList::new(TILE_H);
        list.entries = vec![
            Entry::Header("Audio".into()),
            Entry::Row(0),
            Entry::Row(1),
            Entry::Header("Input".into()),
            Entry::Row(2),
        ];
        list.select_row(Some(1));
        let r = Rect::new(10, 4940, 360, 3 * TILE_H + 2 * 31 + 4);
        let lay = tilelist::layout(&list, r, &theme.look.controls());
        tilelist::draw(
            &mut c,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &list,
            &lay,
            px,
            |c, rr, i, sel| {
                let (title, caption, tag, greyed) = rows[i];
                let tile = Tile {
                    icon: None,
                    title,
                    tag,
                    caption,
                    greyed,
                };
                tilelist::draw_tile(c, text, cs, rr, &tile, sel, px, lay.text);
            },
        );
    }

    {
        use reprisekit::art::bticon::{draw, draw32};
        c.fill_rect(Rect::new(380, 4940, 140, 50), cs.button_face);
        c.blit_canvas(386, 4946, &draw(true));
        c.blit_canvas(406, 4946, &draw(false));
        c.blit_canvas(430, 4946, &draw32(true));
        c.blit_canvas(466, 4946, &draw32(false));
    }

    {
        use reprisekit::art::devicon::{ALL, draw};
        c.fill_rect(Rect::new(536, 4940, 364, 40), cs.button_face);
        for (i, d) in ALL.iter().enumerate() {
            c.blit_over(540 + 40 * i as i32, 4944, &draw(*d));
        }
    }

    {
        use reprisekit::art::userpic::{ALL, draw};
        c.fill_rect(Rect::new(380, 5000, 456, 64), cs.button_face);
        for (i, p) in ALL.iter().enumerate() {
            c.blit_canvas(388 + 56 * i as i32, 5008, &draw(*p, 48));
        }
    }

    {
        use reprisekit::art::printbadge::{ALL, Badge, apply, draw};
        c.fill_rect(Rect::new(380, 5000, 112, 30), cs.button_face);
        for (i, b) in ALL.iter().enumerate() {
            c.blit_over(384 + 12 * i as i32, 5004, &draw(*b));
        }
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(1, 3, 14, 10), cs.button_shadow);
        for (i, set) in [
            [Badge::Default, Badge::Paused],
            [Badge::Default, Badge::Error],
        ]
        .iter()
        .enumerate()
        {
            c.blit_over(424 + 20 * i as i32, 5004, &apply(&icon, set));
        }
    }

    // Stacked: side by side would clip the second page.
    {
        let exterior = wizard::Wizard::new(
            wizard::Style::Exterior {
                title: "Welcome to the Add Bluetooth Device Wizard".into(),
            },
            vec![],
            true,
            false,
        );
        let interior = wizard::Wizard::new(
            wizard::Style::Interior {
                title: "Searching for devices".into(),
                subtitle: "This may take a moment.".into(),
                icon: None,
            },
            vec![],
            false,
            false,
        );
        let (cw, ch) = wizard::CLIENT;
        let mut ce = Canvas::new(cw as u32, ch as u32);
        wizard::draw(
            &mut ce,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &exterior,
            true,
            px,
        );
        let mut ci = Canvas::new(cw as u32, ch as u32);
        wizard::draw(
            &mut ci,
            text,
            &Paint {
                s: 1,
                ..theme.paint()
            },
            &interior,
            true,
            px,
        );
        c.blit_over(10, 5180, &ce);
        c.blit_over(10, 5180 + ch + 10, &ci);
    }

    {
        let area = Rect::new(0, 0, 420, 240);
        let mut under = Canvas::new(420, 240);
        under.fill_rect(area, cs.background);
        under.fill_rect(Rect::new(40, 30, 300, 170), cs.button_face);
        under.fill_rect(Rect::new(40, 30, 300, 18), cs.active_title);
        let mut m = regionselect::RegionSelect::new((0, 0));
        m.press((80, 60));
        m.motion((280, 160));
        c.blit_canvas(
            520,
            5180,
            &regionselect::draw(theme, text, &m, area, &under),
        );
    }

    {
        use reprisekit::art::clockface;
        use reprisekit::controls::{calendar, timefield};
        c.fill_rect(Rect::new(520, 5430, 420, 360), cs.button_face);
        let year = updown::UpDown::new(2026, 1970, 2099).with_buddy();
        let yl = updown::layout_for(
            &year,
            Rect::new(620, 5440, 79, 21),
            &theme.look.controls(),
            reprisekit::theme::TextUnits::ONE,
        );
        updown::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &year,
            &yl,
            false,
            None,
            px,
        );
        let mut wait = updown::UpDown::new(10, 1, 9999);
        wait.enter();
        let wl = updown::layout(720, 5440);
        updown::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &wait,
            &wl,
            true,
            None,
            px,
        );
        let mut cal = calendar::Calendar::new(2026, 9, Some(24), 0);
        cal.today = Some((2026, 9, 3));
        let cl = calendar::layout(
            Rect::new(530, 5470, calendar::W, calendar::H),
            &theme.controls(),
        );
        calendar::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &cal,
            &cl,
            true,
            px,
        );
        let face = clockface::Face {
            h: 23,
            m: 41,
            s: 24,
        };
        clockface::draw(
            &mut c,
            &Paint { s, ..theme.paint() },
            Rect::new(710, 5440, 132, 100),
            face,
        );
        let mut tf = timefield::TimeField::new(23, 41, 24, true);
        tf.select(1);
        let tl = timefield::layout(Rect::new(734, 5570, 103, 20), &theme.look.controls());
        timefield::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &tf,
            &tl,
            true,
            None,
            px,
        );
        let hm = timefield::TimeField::hm(21, 0, false);
        let hl = timefield::layout(Rect::new(734, 5596, 70, 20), &theme.look.controls());
        timefield::draw(
            &mut c,
            text,
            &Paint { s, ..theme.paint() },
            &hm,
            &hl,
            false,
            None,
            px,
        );
        cal.day = None;
        c.blit_canvas(
            530,
            5610,
            &calendar::draw_popup(theme, text, &cal, calendar::PopupHit::Panel),
        );
    }

    {
        use reprisekit::controls::hotkey::{self, HotKey, Mods};
        let mut held = HotKey::new(None);
        held.key(0xffe3, "Control_L", Mods::default(), true);
        held.key(0xffe9, "Alt_L", Mods::default(), true);
        let mut greyed = HotKey::from_keys("Alt+F4");
        greyed.enabled = false;
        let fields = [
            (HotKey::new(None), false),
            (held, true),
            (HotKey::from_keys("Super+Shift+b"), false),
            (greyed, false),
        ];
        for (k, (hk, focused)) in fields.iter().enumerate() {
            let r = Rect::new(734, 5630 + 26 * k as i32, 180, 20);
            hotkey::draw(
                &mut c,
                text,
                &Paint { s, ..theme.paint() },
                hk,
                hotkey::layout(r),
                *focused,
                true,
                Interaction::Idle,
                px,
            );
        }

        use reprisekit::controls::searchfield::{self, SearchField};
        let mut typed = SearchField::default();
        typed.key(reprisekit::textedit::EditKey::Insert("report".into()));
        for (k, (sf, focused)) in [(SearchField::default(), false), (typed, true)]
            .iter()
            .enumerate()
        {
            let field = Rect::new(734, 5734 + 26 * k as i32, 180, 22);
            searchfield::draw(
                &mut c,
                text,
                &Paint { s, ..theme.paint() },
                sf,
                field,
                *focused,
                Caret::new(*focused, true),
                px,
            );
        }
    }

    c.blit_canvas(
        10,
        5930,
        &notifyhistory::draw(theme, text, &notifyhistory::sample()),
    );
    c.fill_rect(Rect::new(400, 5930, 136, 20), cs.button_face);
    for (i, (unseen, dnd)) in [(0, false), (3, false), (12, false), (0, true)]
        .into_iter()
        .enumerate()
    {
        let bell = indicator::draw_bell(16, unseen, dnd);
        c.blit_canvas(402 + 22 * i as i32, 5932, &bell);
    }
    for (i, label) in ["EN", "HU"].into_iter().enumerate() {
        let sq = indicator::draw_layout(text, cs, label, 16, theme.fonts.size_px);
        c.blit_canvas(492 + 22 * i as i32, 5932, &sq);
    }
    c.fill_rect(Rect::new(540, 5930, 70, 20), cs.button_face);
    for (i, (pick, held)) in [
        (indicator::MouseButton::Left, false),
        (indicator::MouseButton::Middle, false),
        (indicator::MouseButton::Right, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mouse = indicator::draw_mouse_keys(16, pick, held);
        c.blit_canvas(542 + 22 * i as i32, 5932, &mouse);
    }

    {
        let side = |size: &str| replacebox::Side {
            size: size.into(),
            modified: "modified: Monday, September 21, 2026, 4:05:12 PM".into(),
            icon: None,
        };
        let file = replacebox::Fields {
            name: "notes.txt".into(),
            folder: None,
            subject: replacebox::Subject::File {
                existing: Box::new(side("512 bytes")),
                incoming: Box::new(side("1.23 KB")),
            },
            icon: None,
            many: false,
        };
        let folder = replacebox::Fields {
            name: "Reports".into(),
            folder: None,
            subject: replacebox::Subject::Folder,
            icon: None,
            many: true,
        };
        let mut y = 6220;
        for fields in [file, folder] {
            let (w, h) = replacebox::client(&fields, text, px, reprisekit::theme::TextUnits::ONE);
            let mut box_c = Canvas::new(w as u32, h as u32);
            let f = replacebox::form(&fields, text, px, reprisekit::theme::TextUnits::ONE);
            replacebox::draw(&mut box_c, text, &Paint { s, ..theme.paint() }, &f, px);
            c.blit_canvas(424, y, &box_c);
            y += h + 10;
        }
    }

    {
        let l = switcher::SwitcherLayout::compute(3, 1, Rect::new(20, 6680, 428, 357), true);
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let mut icon = Canvas::new(32, 32);
        icon.fill_rect(Rect::new(2, 4, 28, 24), Color::rgb(80, 193, 255));
        switcher::draw(
            &mut panel,
            theme,
            text,
            &l,
            "Terminal",
            &[Some(icon.clone()), Some(icon.clone()), None],
            1,
            None,
            Some(&icon),
        );
        c.blit_canvas(l.rect.x, l.rect.y, &panel);

        let l = taskpreview::TaskPreviewLayout::compute(
            taskpreview::CLASSIC,
            2,
            Rect::new(560, 6900, 160, 22),
            Rect::new(480, 6680, 450, 220),
            reprisekit::taskbar::Edge::Bottom,
        );
        let mut panel = Canvas::new(l.rect.width as u32, l.rect.height as u32);
        let cells = [
            taskpreview::Cell {
                title: "user@reprise: ~",
                icon: None,
                active: true,
                picture: None,
            },
            taskpreview::Cell {
                title: "A window with a title too long for its cell",
                icon: None,
                active: false,
                picture: Some(&icon),
            },
        ];
        taskpreview::draw(&mut panel, theme, text, &l, &cells, Some(1));
        c.blit_canvas(l.rect.x, l.rect.y, &panel);
    }

    {
        c.set_preedit(Preedit::new("nihao".into(), 2, 2));
        let field = Rect::new(20, 7090, 200, ROW_H);
        let inner = draw_sunken_field(
            &mut c,
            &Paint { s, ..theme.paint() },
            field,
            theme.colors.window,
        );
        let mut edit = TextEdit::new("say  to all", false);
        edit.place(4, false);
        draw_edit(
            &mut c,
            text,
            &edit,
            inner,
            px,
            &Paint { s, ..theme.paint() },
            Caret::Shown,
            0,
        );
        let m = multiedit::Metrics {
            bars_always: false,
            bar: theme.look.controls().scroll,
            thumb_min: theme.look.controls().thumb_min,
            ..Default::default()
        };
        let mut edit = multiedit::MultiEdit::new("first line\nsay  to all");
        edit.place(multiedit::Pos { line: 1, col: 4 }, false);
        let r = Rect::new(240, 7086, 240, 44);
        let lay = multiedit::layout(mono, &m, &edit, r, s);
        let view = multiedit::View::default();
        multiedit::draw(
            &mut c,
            mono,
            &Paint { s, ..theme.paint() },
            &m,
            &edit,
            view,
            &lay,
            Caret::Shown,
            None,
        );
        c.set_preedit(None);
    }

    {
        let mut desk = Node::new("Desktop", true);
        desk.expanded = true;
        desk.children = vec![
            Node::new("Documents", true),
            Node::new("Stuff", false),
            Node::new("New Folder", false),
        ];
        let mut tv = TreeView::new(vec![desk], treeview::Style::Picker);
        tv.selected = Some(vec![0, 1]);
        tv.begin_edit(vec![0, 2]);
        let d = browsefolder::BrowseFolder::new(
            "Pick the folder to move 'a.txt' to, then click Move.".into(),
            "Move".into(),
            tv,
        );
        let lay = d.layout(text, px, &theme.look.controls());
        let (w, h) = browsefolder::CLIENT;
        let mut dc = Canvas::new(w as u32, h as u32);
        d.draw(&mut dc, text, &Paint { s, ..theme.paint() }, &lay, px);
        c.blit_canvas(10, 8596, &dc);
    }

    {
        let (w, h) = (200, 190);
        let mut under = Canvas::new(w as u32, h as u32);
        under.fill_rect(Rect::new(0, 0, w, h), cs.background);
        under.fill_rect(Rect::new(20, 20, 160, 100), cs.button_face);
        under.fill_rect(Rect::new(20, 20, 160, 18), cs.active_title);
        let still = colorpick::Still {
            logical: (w, h),
            width: w as u32,
            height: h as u32,
            pixels: under.pixels.clone(),
        };
        let at = (22, 36);
        let mut loupe = Canvas::new(colorpick::W as u32, colorpick::H as u32);
        colorpick::draw(&mut loupe, text, px, &still, at);
        let r = colorpick::layout(at, (w, h)).loupe;
        under.blit_canvas(r.x, r.y, &loupe);
        c.blit_canvas(745, 8596, &under);
    }

    if with_blue {
        use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
        let blue_theme = cued(interfaces::theme_for(Interface::Blue));
        let style = blue_theme.look.style().unwrap();
        let bcs = &blue_theme.colors;
        let band = Rect::new(0, 9312, 950, 40);
        c.fill_rect(band, bcs.background);
        let start_text = TextRenderer::load(&style.start_text.font());
        let layout = TaskbarLayout::compute(
            &blue_theme.metrics,
            &style.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 3,
                tray: 2,
                show_desktop: false,
                chevron: false,
                ..TaskbarCounts::default()
            },
            text.measure("00:00", false, blue_theme.fonts.size_px),
        );
        let tasks = vec![
            TaskEntry {
                title: "Notepad".into(),
                active: true,
                attention: reprisekit::taskbar::Attention::None,
                icon: None,
                glow: None,
            },
            TaskEntry {
                title: "A window with a title too long for its button".into(),
                active: false,
                attention: reprisekit::taskbar::Attention::None,
                icon: None,
                glow: None,
            },
            TaskEntry {
                title: "Explorer".into(),
                active: false,
                attention: reprisekit::taskbar::Attention::None,
                icon: None,
                glow: None,
            },
        ];
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: Some(1),
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks,
            tray_icons: vec![
                Some(indicator::draw_record(16).into()),
                Some(indicator::draw_microphone(16).into()),
            ],
            meter: None,
            clock: "3:45 PM".into(),
        };
        let mut bar = Canvas::new(800, 30);
        look::draw_taskbar(&mut bar, &blue_theme, text, &start_text, &layout, &state);
        c.blit_canvas(10, band.y + 5, &bar);
    }

    c
}
