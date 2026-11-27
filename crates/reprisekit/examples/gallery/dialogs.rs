use super::{blurred_stripes, cued};
use crate::interfaces;
use reprisekit::classic::frame::{CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::controls::form::{self};
use reprisekit::controls::{
    aboutbox, endsession, favorites, filedlg, fontdlg, importsheet, installsheet, locationprompt,
    namebox, pagesetup, printdlg, printpreview, shellview,
};
use reprisekit::interface::Interface;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};

/// Three outputs in the monitor well, then greyed with output 3 copying 1, plus Identify's
/// badge; Classic then Blue and Glass.
pub fn render_monitors(with_blue: bool) -> Canvas {
    use reprisekit::controls::arrange::{self, Arrange, BADGE, Tile};
    let tile = |number, pos, size| Tile {
        number,
        name: format!("OUT-{number}"),
        pos,
        size,
    };
    let tiles = vec![
        tile(1, (0, 0), (1920, 1080)),
        tile(2, (1920, 0), (1280, 1024)),
        tile(3, (0, 1080), (1080, 1920)),
    ];
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (ww, wh) = (358, 108);
    let col = ww + BADGE + 30;
    let m = reprisekit::look::panel::margin(&themes[themes.len() - 1]);
    let mut c = Canvas::new(
        (themes.len() as i32 * col + 10 + m.left + m.right) as u32,
        (2 * wh + 30) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let (p, px) = (theme.paint(), theme.fonts.size_px);
        let x = 10 + i as i32 * col;
        let mut face = Canvas::new((ww + 20) as u32, (2 * wh + 30) as u32);
        face.fill_rect(face.bounds(), theme.colors.button_face);
        let mut a = Arrange::new(tiles.clone(), 1);
        let well = Rect::new(10, 10, ww, wh);
        arrange::draw(
            &mut face,
            &text,
            &p,
            &a,
            &arrange::layout(well, &a),
            true,
            px,
        );
        a.enabled = false;
        a.tiles[2].pos = (0, 0);
        let greyed = Rect::new(10, 20 + wh, ww, wh);
        arrange::draw(
            &mut face,
            &text,
            &p,
            &a,
            &arrange::layout(greyed, &a),
            false,
            px,
        );
        c.blit_canvas(x, 0, &face);
        c.blit_canvas(x + ww + 25, 10, &arrange::draw_badge(theme, &text, &[1, 3]));
    }
    c
}

/// The session-end prompt with a program that has a window and one that gave a reason.
pub fn render_end_session(with_blue: bool) -> Canvas {
    let fields = endsession::Fields {
        goal: endsession::Goal::LogOff,
        rows: vec![
            endsession::Row {
                name: "Untitled - Notepad".into(),
                reason: String::new(),
            },
            endsession::Row {
                name: "Text Editor".into(),
                reason: "There are unsaved documents".into(),
            },
        ],
        icon: None,
    };
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let base = TextRenderer::load(&Theme::default().fonts);
    let (w, h) = endsession::client(
        &fields,
        &base,
        Theme::default().fonts.size_px,
        reprisekit::theme::TextUnits::ONE,
    );
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let (bw, bh) = endsession::client(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let f = endsession::form(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let mut box_c = Canvas::new(bw as u32, bh as u32);
        endsession::draw(&mut box_c, &text, &theme.paint(), &f, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// The location prompt for a program called Maps, focus on Deny.
/// The install sheet asking and then running, in each look.
pub fn render_install_sheet(with_blue: bool) -> Canvas {
    use installsheet::{Sheet, Stage, Verb};
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 4, 24, 24), Color::rgb(52, 120, 200));
    let sheet = |stage| Sheet {
        verb: Verb::Uninstall,
        name: "Foot".into(),
        icon: Some(icon.clone()),
        facts: vec![
            ("Package:".into(), "foot".into()),
            ("Version:".into(), "1.28.0-2".into()),
            ("Size:".into(), "812 KB".into()),
            ("Source:".into(), "Arch Linux packages".into()),
        ],
        note: "Uninstalling it also removes 1 package that needs it: foot-terminfo.".into(),
        stage,
    };
    let stages = [
        sheet(Stage::Asking { ready: true }),
        sheet(Stage::Running {
            fraction: Some(0.4),
            doing: "Removing foot".into(),
            cancellable: true,
        }),
        Sheet {
            note: "Uninstalling it would remove glibc (needed by the system).".into(),
            ..sheet(Stage::Refused)
        },
    ];
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = installsheet::client(&stages[0]);
    let mut c = Canvas::new(
        (stages.len() as i32 * (w + 10) + 10) as u32,
        (themes.len() as i32 * (h + 10) + 10) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        for (col, s) in stages.iter().enumerate() {
            let u = reprisekit::theme::TextUnits::ONE;
            let f = installsheet::form(s, &text, px, u);
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            installsheet::draw(&mut sheet_c, &text, &theme.paint(), &f, px);
            let at = (10 + col as i32 * (w + 10), 10 + row as i32 * (h + 10));
            c.blit_canvas(at.0, at.1, &sheet_c);
        }
    }
    c
}

/// Recycle Bin Properties on Global, then on a drive's tab with the drives set apart and files
/// erased there, in each look.
pub fn render_bin_sheet(with_blue: bool) -> Canvas {
    use reprisekit::controls::binsheet::{BinSheet, Drive, Rule, Settings};
    use reprisekit::controls::propsheet;
    let drives = vec![
        Drive {
            key: "home".into(),
            label: "Local Disk (/home)".into(),
            size: 21_365_735_424,
        },
        Drive {
            key: "5A1F".into(),
            label: "STICK".into(),
            size: 7_999_999_999,
        },
    ];
    let global = Rule {
        remove_immediately: false,
        max_percent: 10,
    };
    let settings = Settings {
        confirm: true,
        independent: false,
        global,
        drives: Default::default(),
        older_than_days: 0,
    };
    let mut apart = Settings {
        independent: true,
        ..settings.clone()
    };
    apart.drives.insert(
        "5A1F".into(),
        Rule {
            remove_immediately: true,
            ..global
        },
    );
    let mut on_drive = BinSheet::new(apart, drives.clone());
    on_drive.sheet.open_tab(2);
    let sheets = [BinSheet::new(settings, drives), on_drive];
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = sheets[0].sheet.size;
    let mut c = Canvas::new(
        (sheets.len() as i32 * (w + 10) + 10) as u32,
        (themes.len() as i32 * (h + 10) + 10) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        for (col, s) in sheets.iter().enumerate() {
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), p.cs.button_face);
            propsheet::draw(&mut sheet_c, &text, &p, &s.sheet, true, theme.fonts.size_px);
            let at = (10 + col as i32 * (w + 10), 10 + row as i32 * (h + 10));
            c.blit_canvas(at.0, at.1, &sheet_c);
        }
    }
    c
}

/// Folder Options' General, View with every box flipped, and File Types, in each look.
pub fn render_folder_sheet(with_blue: bool) -> Canvas {
    use reprisekit::controls::folderopts::{FolderSheet, Settings};
    use reprisekit::controls::propsheet;
    let flipped = Settings {
        new_window: true,
        single_click: true,
        show_hidden: true,
        hide_extensions: false,
        tips: false,
        cache_thumbnails: false,
    };
    let sheets: Vec<FolderSheet> = [
        (Settings::default(), 0),
        (flipped, 1),
        (Settings::default(), 2),
    ]
    .into_iter()
    .map(|(settings, tab)| {
        let mut s = FolderSheet::new(settings);
        s.sheet.open_tab(tab);
        s
    })
    .collect();
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = sheets[0].sheet.size;
    let mut c = Canvas::new(
        (sheets.len() as i32 * (w + 10) + 10) as u32,
        (themes.len() as i32 * (h + 10) + 10) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        for (col, s) in sheets.iter().enumerate() {
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            sheet_c.fill_rect(sheet_c.bounds(), p.cs.button_face);
            propsheet::draw(&mut sheet_c, &text, &p, &s.sheet, true, theme.fonts.size_px);
            let at = (10 + col as i32 * (w + 10), 10 + row as i32 * (h + 10));
            c.blit_canvas(at.0, at.1, &sheet_c);
        }
    }
    c
}

/// The import sheet over GNOME and Plasma settings, the second change unchecked, then done.
pub fn render_import_sheet(with_blue: bool) -> Canvas {
    use importsheet::{Line, Sheet, Source};
    let line = |what: &str, now: &str, new: &str, greyed| Line {
        what: what.into(),
        now: now.into(),
        new: new.into(),
        greyed,
    };
    let sources = || {
        vec![
            Source {
                name: "GNOME".into(),
                lines: vec![
                    line(
                        "Wallpaper",
                        "None",
                        "/home/sam/Pictures/Fjord.jpg (Stretch)",
                        false,
                    ),
                    line("Dark style", "Off", "On", false),
                    line(
                        "Keyboard layouts",
                        "System default",
                        "us, de (nodeadkeys)",
                        false,
                    ),
                    line("Touchpad natural scrolling", "Off", "On", false),
                    line("Text size", "100%", "125%", false),
                    line("Quick Launch", "", "Firefox", false),
                    line(
                        "Clock",
                        "the clock follows the formats locale",
                        "12-hour",
                        true,
                    ),
                    line(
                        "Input method",
                        "set up in the input method",
                        "ibus: anthy",
                        true,
                    ),
                ],
            },
            Source {
                name: "Plasma".into(),
                lines: vec![line("Tap to click", "On", "Off", false)],
            },
        ]
    };
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = importsheet::CLIENT;
    let mut c = Canvas::new(
        (2 * (w + 10) + 10) as u32,
        (themes.len() as i32 * (h + 10) + 10) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let mut asking = Sheet::new(sources(), &text, theme);
        let space = form::Key::Space;
        asking.list_key(&form::Key::Down, &text, theme);
        asking.list_key(&space, &text, theme);
        let mut done = Sheet::new(sources(), &text, theme);
        done.done("Imported 6 GNOME settings.".into(), true, &text, theme);
        for (col, s) in [asking, done].iter().enumerate() {
            let mut sheet_c = Canvas::new(w as u32, h as u32);
            importsheet::draw(&mut sheet_c, &text, &theme.paint(), s, px);
            let at = (10 + col as i32 * (w + 10), 10 + row as i32 * (h + 10));
            c.blit_canvas(at.0, at.1, &sheet_c);
        }
    }
    c
}

pub fn render_location_prompt(with_blue: bool) -> Canvas {
    let mut icon = Canvas::new(32, 32);
    icon.fill_rect(Rect::new(4, 4, 24, 24), Color::rgb(52, 160, 80));
    let fields = locationprompt::Fields {
        program: "Maps".into(),
        icon: Some(icon),
    };
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let base = TextRenderer::load(&Theme::default().fonts);
    let (w, h) = locationprompt::client(
        &fields,
        &base,
        Theme::default().fonts.size_px,
        reprisekit::theme::TextUnits::ONE,
    );
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let (bw, bh) =
            locationprompt::client(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let f = locationprompt::form(&fields, &text, px, reprisekit::theme::TextUnits::ONE);
        let mut box_c = Canvas::new(bw as u32, bh as u32);
        locationprompt::draw(&mut box_c, &text, &theme.paint(), &f, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// Organize Favorites with an address Explorer cannot open picked, and Add Favorite under it;
/// Classic then Blue and Glass.
pub fn render_favorites(with_blue: bool) -> Canvas {
    let row = |label: &str, address: &str, state| favorites::Row {
        label: label.into(),
        address: address.into(),
        state,
    };
    let rows = [
        row("Projects", "/home/user/Projects", favorites::State::Opens),
        row("Server", "sftp://server/srv/share", favorites::State::Opens),
        row(
            "Backup",
            "/run/media/user/Backup",
            favorites::State::Missing,
        ),
        row("Recent", "recent:///", favorites::State::Unsupported),
    ];
    let organize = favorites::organize(&rows, Some(3));
    let add = namebox::NameBox::new("Projects").form;
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = favorites::CLIENT;
    let add_h = namebox::CLIENT.1;
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + add_h + 30) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let x = 10 + i as i32 * (w + 10);
        for (f, (bw, bh), y) in [
            (&organize, favorites::CLIENT, 10),
            (&add, namebox::CLIENT, h + 20),
        ] {
            let mut box_c = Canvas::new(bw as u32, bh as u32);
            box_c.fill_rect(box_c.bounds(), p.cs.button_face);
            form::draw(&mut box_c, &text, &p, f, true, theme.fonts.size_px);
            c.blit_canvas(x, y, &box_c);
        }
    }
    c
}

/// A form too tall for its client in each look: its buttons kept in the corner, the rest
/// scrolled to the top, then to the end.
pub fn render_form_scroll(with_blue: bool) -> Canvas {
    use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
    use reprisekit::controls::form::{Control, Form, Item};
    use reprisekit::controls::formscroll::FormScroll;
    fn as_built(_: &mut Form, _: (i32, i32)) -> (i32, i32) {
        (220, 260)
    }
    let built = || {
        let button = |id, x, label: &str| {
            let control = Control::Button {
                label: label.into(),
                enabled: true,
                default: id == "cancel",
            };
            Item::new(id, Rect::new(x, 228, BUTTON_W, BUTTON_H), control)
        };
        let check = Control::Check {
            label: "&Remember this choice".into(),
            checked: false,
            enabled: true,
            mixed: None,
        };
        Form::new(vec![
            Item::label("note", Rect::new(11, 11, 184, 13), "Choose one of these."),
            Item::new(
                "group",
                Rect::new(9, 32, 186, 160),
                Control::Group("Sources".into()),
            ),
            Item::new("check", Rect::new(9, 200, 180, 13), check),
            button("ok", 56, "OK"),
            button("cancel", 136, "Cancel"),
        ])
    };
    let client = (220, 160);
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let mut c = Canvas::new(
        (2 * (client.0 + 10) + 10) as u32,
        (themes.len() as i32 * (client.1 + 10) + 10) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut f = built();
        f.drawn_with(&p);
        let mut scroll = FormScroll::new(&["ok", "cancel"], as_built);
        scroll.fit(&mut f, client, &text, theme.fonts.size_px);
        for col in 0..2 {
            if col == 1 {
                scroll.wheel(&mut f, 100);
            }
            let mut box_c = Canvas::new(client.0 as u32, client.1 as u32);
            box_c.fill_rect(box_c.bounds(), p.cs.button_face);
            form::draw(&mut box_c, &text, &p, &f, true, theme.fonts.size_px);
            scroll.draw(&mut box_c, &p, &f);
            let at = (
                10 + col * (client.0 + 10),
                10 + row as i32 * (client.1 + 10),
            );
            c.blit_canvas(at.0, at.1, &box_c);
        }
    }
    c
}

/// The copy progress dialog in each look at `scale`: Classic preparing, copying and finishing a
/// move; Blue, Olive and Silver copying; Glass light and dark with the details, and with fewer.
pub fn render_copy_progress(with_blue: bool, scale: i32) -> Canvas {
    use reprisekit::controls::copyprogress::{CopyProgress, Kind, Phase, Status};
    let status = |phase, bytes: u64| Status {
        phase,
        bytes,
        total_bytes: 1_200_000_000,
        items: bytes / 100_000_000,
        total_items: 12,
        current: "holiday-2026.mkv".into(),
        waiting: 1,
        ..Status::default()
    };
    let glass_dark = cued(interfaces::theme_with(
        Interface::Glass,
        reprisekit::blue::Variant::default(),
        &reprisekit::glass::Settings {
            mode: reprisekit::glass::Mode::Dark,
            ..reprisekit::glass::Settings::default()
        },
    ));
    let mut boxes: Vec<(Theme, Kind, Phase, bool)> = vec![
        (
            cued(interfaces::classic()),
            Kind::Move,
            Phase::Preparing,
            true,
        ),
        (
            cued(interfaces::classic()),
            Kind::Move,
            Phase::Copying,
            true,
        ),
        (
            cued(interfaces::classic()),
            Kind::Move,
            Phase::Finishing,
            true,
        ),
        (
            cued(interfaces::classic()),
            Kind::Delete,
            Phase::Copying,
            true,
        ),
        (
            cued(interfaces::classic()),
            Kind::Empty,
            Phase::Copying,
            true,
        ),
        (
            cued(interfaces::classic()),
            Kind::Restore,
            Phase::Copying,
            true,
        ),
    ];
    if with_blue {
        for v in [
            reprisekit::blue::Variant::Blue,
            reprisekit::blue::Variant::Olive,
            reprisekit::blue::Variant::Silver,
        ] {
            boxes.push((
                cued(interfaces::blue_theme(v)),
                Kind::Move,
                Phase::Copying,
                true,
            ));
        }
        let blue = cued(interfaces::blue_theme(reprisekit::blue::Variant::Blue));
        for kind in [Kind::Delete, Kind::Empty, Kind::Restore] {
            boxes.push((blue.clone(), kind, Phase::Copying, true));
        }
        let olive = cued(interfaces::blue_theme(reprisekit::blue::Variant::Olive));
        boxes.push((olive, Kind::Delete, Phase::Finishing, true));
        let glass = cued(interfaces::theme_for(Interface::Glass));
        boxes.push((glass.clone(), Kind::Move, Phase::Copying, true));
        boxes.push((glass_dark, Kind::Move, Phase::Copying, true));
        boxes.push((glass.clone(), Kind::Move, Phase::Copying, false));
        boxes.push((glass, Kind::Delete, Phase::Copying, true));
    }
    let drawn: Vec<Canvas> = boxes
        .iter()
        .map(|(theme, kind, phase, details)| {
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            let mut cp =
                CopyProgress::new(*kind, "Downloads", "KINGSTON", Color::rgb(0x2E, 0x8C, 0xFF));
            cp.follow(&p);
            if !details {
                cp.toggle_details();
            }
            for k in 0..30u64 {
                let b = k * 15_000_000 + (k % 5) * 3_000_000;
                cp.update(status(Phase::Copying, b), k * 300 + 40);
            }
            cp.update(status(*phase, 450_000_000), 30 * 300 + 40);
            let (w, h) = cp.client();
            let mut c = Canvas::with_scale(w as u32, h as u32, scale);
            cp.draw(&mut c, &text, &p, theme.fonts.size_px);
            c
        })
        .collect();
    let cols = 3;
    let cell_w = drawn.iter().map(|c| c.width as i32).max().unwrap_or(0) + 10;
    let cell_h = drawn.iter().map(|c| c.height as i32).max().unwrap_or(0) + 10;
    let rows = drawn.len().div_ceil(cols) as i32;
    let mut c = Canvas::with_scale(
        (cols as i32 * cell_w + 10) as u32,
        (rows * cell_h + 10) as u32,
        scale,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, b) in drawn.iter().enumerate() {
        let (col, row) = ((i % cols) as i32, (i / cols) as i32);
        c.blit_canvas(10 + col * cell_w, 10 + row * cell_h, b);
    }
    c
}

/// A windowed dialog's frame at `scale`, one row per look (Classic, Blue, Olive, Silver, Glass
/// light and dark, the Square frame): at rest, minimize hot, minimize held, inactive.
/// Labels too long for their rects, cut to an ellipsis, with the tip of the one under the
/// pointer, in Classic, Blue and Glass.
pub fn render_label_ellipsis(scale: i32) -> Canvas {
    use reprisekit::controls::form::{self, Form, Item};
    let text = TextRenderer::load(&FontConfig::default());
    let looks = [
        cued(interfaces::classic()),
        cued(interfaces::blue_theme(reprisekit::blue::Variant::Blue)),
        cued(interfaces::theme_for(Interface::Glass)),
    ];
    let (w, h) = (230, 110);
    let mut c = Canvas::new((w * looks.len() as i32 * scale) as u32, (h * scale) as u32);
    for (i, theme) in looks.iter().enumerate() {
        let theme = theme.scaled(scale);
        let mut cell = Canvas::new((w * scale) as u32, (h * scale) as u32);
        cell.fill_rect(cell.bounds(), theme.colors.button_face);
        let model = "HP LaserJet 4250 PostScript Printer Driver";
        let mut f = Form::new(vec![
            Item::label_at("model_label", 10, 10, 50, "Model:"),
            Item::label_at("model", 64, 10, 150, model),
            Item::label_at(
                "status",
                10,
                30,
                204,
                "Error unlocking /dev/sdc: Failed to activate device: Operation not permitted",
            ),
            Item::label_at("grey", 10, 50, 204, model).enabled(false),
        ]);
        let px = theme.fonts.size_px;
        f.motion(120, 55, None);
        f.tip_tick();
        form::draw(&mut cell, &text, &theme.paint(), &f, false, px);
        form::draw_tip(&mut cell, &text, &theme);
        c.blit_canvas(w * scale * i as i32, 0, &cell);
    }
    c
}

pub fn render_windowed_dialog(scale: i32) -> Canvas {
    use reprisekit::blue::Variant;
    use reprisekit::glass::{Frame, Mode, Settings};
    let glass = |mode, frame| {
        let s = Settings {
            mode,
            frame,
            ..Settings::default()
        };
        cued(interfaces::theme_with(
            Interface::Glass,
            Variant::default(),
            &s,
        ))
    };
    let looks = [
        cued(interfaces::classic()),
        cued(interfaces::blue_theme(Variant::Blue)),
        cued(interfaces::blue_theme(Variant::Olive)),
        cued(interfaces::blue_theme(Variant::Silver)),
        glass(Mode::Light, Frame::Rounded),
        glass(Mode::Dark, Frame::Rounded),
        glass(Mode::Light, Frame::Square),
    ];
    let min = Some(CaptionButton::Minimize);
    let states = [
        (true, None, None),
        (true, min, None),
        (true, None, min),
        (false, None, None),
    ];
    let (cw, ch) = (200, 40);
    let totals = looks
        .iter()
        .map(|t| FrameGeometry::layout(&t.metrics, cw, ch, false, Chrome::WINDOWED_DIALOG).total);
    let cell = totals.fold((0, 0), |c, t| (c.0.max(t.0 + 20), c.1.max(t.1 + 20)));
    let mut c = Canvas::new(
        ((states.len() as i32 * cell.0 + 20) * scale) as u32,
        ((looks.len() as i32 * cell.1 + 20) * scale) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (row, theme) in looks.iter().enumerate() {
        let g = FrameGeometry::layout(&theme.metrics, cw, ch, false, Chrome::WINDOWED_DIALOG)
            .scaled(scale);
        let theme = theme.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        for (col, &(active, hot, pressed)) in states.iter().enumerate() {
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            let st = FrameState {
                title: "Copying...",
                active,
                maximized: false,
                pressed,
                glow: Glow::lit(hot),
                icon: None,
                tabbed: false,
            };
            look::draw_dialog_frame(&mut f, &theme, &text, &g, &st, None);
            let (x, y) = (10 + col as i32 * cell.0, 10 + row as i32 * cell.1);
            c.blit_over(x * scale, y * scale, &f);
        }
    }
    c
}

/// The debug overlay's panel with every row, in each look.
pub fn render_stats_panel(with_blue: bool) -> Canvas {
    use reprisekit::controls::statspanel::{self, Row, StatsPanel};
    let p = StatsPanel {
        title: "DP-1 (1920x1080 at 60 Hz)".into(),
        rows: vec![
            Row::new("Renders/s", "58"),
            Row::new("CPU/render", "1.4 ms avg, 3.9 max"),
            Row::new("Flip interval", "16.7 ms avg, 33.3 max"),
            Row::new("Damage", "3.2 rects, 12.5% area"),
            Row::new("Clients", "9"),
            Row::new("Windows", "4"),
            Row::new("Surfaces", "37"),
            Row::new("Memory", "84.2 MB"),
        ],
    };
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let (w, h) = statspanel::size(p.rows.len(), themes[0].text);
    let step = w + 40;
    let mut c = blurred_stripes(themes.len() as i32 * step + 20, h + 60);
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let m = reprisekit::look::panel::margin(theme);
        let panel = statspanel::draw(theme, &text, &p);
        c.blit_over(30 + i as i32 * step - m.left, 30 - m.top, &panel);
    }
    c
}

/// About Reprise's system view with a stand-in logo, in each look, at `pct` text size.
pub fn render_about_system(with_blue: bool, pct: u32) -> Canvas {
    let g = |label: &str, lines: &[&str]| aboutbox::Group {
        label: label.into(),
        lines: lines.iter().map(|s| s.to_string()).collect(),
    };
    let mut logo = Canvas::new(aboutbox::LOGO as u32, aboutbox::LOGO as u32);
    logo.fill_rect(Rect::new(8, 8, 48, 48), Color::rgb(23, 147, 209));
    let system = aboutbox::System {
        logo: Some(logo),
        groups: vec![
            g(
                "Reprise:",
                &["Version 0.1.0 (build a1b2c3d)", "Classic interface"],
            ),
            g(
                "Operating system:",
                &["Arch Linux", "Linux 6.9.0-1-generic (x86_64)"],
            ),
            g(
                "Computer:",
                &["reprise-vm-1", "QEMU Standard PC (Q35 + ICH9, 2009)"],
            ),
            g(
                "Processor:",
                &[
                    "Example CPU @ 2.00GHz",
                    "4 cores, 8 threads, up to 3.40 GHz",
                ],
            ),
            g("Graphics:", &["Red Hat Virtio 1.0 GPU"]),
            g("Memory:", &["16,777,216 KB", "9,542,041 KB available"]),
            g("Disk (/):", &["500.0 GB (120.0 GB free)"]),
            g("Display:", &["1920x1080 at 60 Hz"]),
            g("Session:", &["Jo Doe (jo)", "Up 3 hours, 12 minutes"]),
        ],
    };
    let mut themes = vec![cued(interfaces::classic()).with_text_size(pct)];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)).with_text_size(pct));
        themes.push(cued(interfaces::theme_for(Interface::Glass)).with_text_size(pct));
    }
    let (w, h) = themes[0].text.size(aboutbox::system_size(&system));
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let built = aboutbox::system_form(&system, &text, theme.text.base_px(px), theme.text);
        let mut box_c = Canvas::new(w as u32, h as u32);
        box_c.fill_rect(box_c.bounds(), theme.colors.button_face);
        form::draw(&mut box_c, &text, &theme.paint(), &built, true, px);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// The Open dialog enlarged, in Details, sorted by Size twice: files first, largest first, then folders Z to A.
pub fn render_file_dialog() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(interfaces::classic());
    let mut d = filedlg::FileDlg::new(
        filedlg::Fields {
            title: "Open".into(),
            accept_label: "Open".into(),
            mode: filedlg::DlgMode::Open,
            multiple: false,
            filters: vec!["Text Documents (*.txt)".into(), "All Files (*.*)".into()],
        },
        vec![],
    );
    let entry = |id: u64, name: &str, is_dir: bool, size: u64| shellview::Entry {
        id,
        name: name.into(),
        is_dir,
        size,
        kind: if is_dir {
            "File Folder"
        } else {
            "Text Document"
        }
        .into(),
        modified: "9/12/2026 10:13 AM".into(),
        ..Default::default()
    };
    d.set_listing(filedlg::Folder {
        id: 1,
        parent: None,
        crumbs: vec![filedlg::Crumb {
            id: 1,
            label: "Documents".into(),
            depth: 0,
            icon: None,
        }],
        entries: vec![
            entry(2, "Letters", true, 0),
            entry(3, "notes.txt", false, 1_024),
            entry(4, "Reports", true, 0),
            entry(5, "setup.log", false, 114_688),
            entry(6, "todo.txt", false, 32_768),
        ],
    });
    d.set_size((filedlg::CLIENT.0 + 168, filedlg::CLIENT.1 + 55));
    let (w, h) = d.client();
    let r = Rect::new(0, 0, w, h);
    d.click(r, Some(filedlg::Hit::ToolBar(3)), Default::default());
    d.click(r, Some(filedlg::Hit::ViewsItem(4)), Default::default());
    let size = Some(filedlg::Hit::Shell(shellview::Hit::Header(1)));
    for _ in 0..2 {
        d.click(r, size, Default::default());
        d.release();
    }
    let mut dc = Canvas::new(w as u32, h as u32);
    d.draw(&mut dc, &text, &theme.paint(), r, theme.fonts.size_px);
    let mut c = Canvas::new(w as u32 + 20, h as u32 + 20);
    c.fill_rect(c.bounds(), theme.colors.background);
    c.blit_canvas(10, 10, &dc);
    c
}

/// The Font dialog on Tahoma Bold 14 pt at `scale`; Classic then Blue and Glass.
pub fn render_font_dialog(with_blue: bool, scale: i32) -> Canvas {
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let families = [
        "Arial",
        "DejaVu Sans",
        "DejaVu Sans Mono",
        "Lucida Console",
        "Tahoma",
    ]
    .map(String::from)
    .to_vec();
    let pick = fontdlg::Pick {
        family: "Tahoma".into(),
        style: fontdlg::Style::Bold,
        size: 14,
    };
    let sample = TextRenderer::load(&FontConfig {
        family: vec![pick.family.clone()],
        antialias: true,
        ..FontConfig::default()
    });
    let (w, h) = (fontdlg::CLIENT.0 * scale, fontdlg::CLIENT.1 * scale);
    let mut c = Canvas::new(
        (themes.len() as i32 * (w + 10) + 10) as u32,
        (h + 20) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let theme = theme.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let f = fontdlg::form(&families, &pick);
        let mut box_c = Canvas::new(w as u32, h as u32);
        box_c.fill_rect(box_c.bounds(), p.cs.button_face);
        let s = fontdlg::Sample {
            text: &sample,
            bold: pick.style.bold(),
            px: fontdlg::px(pick.size) * scale as u32,
        };
        fontdlg::draw(&mut box_c, &text, &p, &f, theme.fonts.size_px, &s);
        c.blit_canvas(10 + i as i32 * (w + 10), 10, &box_c);
    }
    c
}

/// Page Setup on Letter landscape with a header and footer over Print with two printers, at `scale`; Classic then Blue and
/// Glass.
pub fn render_print_dialogs(with_blue: bool, scale: i32) -> Canvas {
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let setup = pagesetup::PageSetup {
        landscape: true,
        header: "&f".into(),
        footer: "Page &p".into(),
        ..pagesetup::PageSetup::on(&pagesetup::PAPERS[0])
    };
    let dests = [
        printdlg::Dest {
            name: "Laser".into(),
            info: "Office laser".into(),
            location: "Second floor".into(),
            state: printdlg::DestState::Ready,
        },
        printdlg::Dest {
            name: "PDF".into(),
            info: "Virtual PDF Printer".into(),
            ..Default::default()
        },
    ];
    let (sw, sh) = pagesetup::CLIENT;
    let (sw, sh) = (sw * scale, sh * scale);
    let (pw, ph) = (printdlg::CLIENT.0 * scale, printdlg::CLIENT.1 * scale);
    let col = sw.max(pw) + 10;
    let mut c = Canvas::new(
        (themes.len() as i32 * col + 10) as u32,
        (sh + ph + 30) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    for (i, theme) in themes.iter().enumerate() {
        let theme = theme.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let px = theme.fonts.size_px;
        let mut setup_c = Canvas::new(sw as u32, sh as u32);
        setup_c.fill_rect(setup_c.bounds(), p.cs.button_face);
        pagesetup::draw(
            &mut setup_c,
            &text,
            &p,
            &pagesetup::form(&setup, pagesetup::Kind::Text),
            px,
        );
        c.blit_canvas(10 + i as i32 * col, 10, &setup_c);
        let mut print_c = Canvas::new(pw as u32, ph as u32);
        print_c.fill_rect(print_c.bounds(), p.cs.button_face);
        let f = printdlg::form(Ok(&dests), 0, 4);
        form::draw(&mut print_c, &text, &p, &f, true, px);
        c.blit_canvas(10 + i as i32 * col, sh + 20, &print_c);
    }
    c
}

/// Print Preview on page 2 of 3 of a text with a header and footer, the whole page; Classic then
/// Blue and Glass.
pub fn render_print_preview(with_blue: bool, scale: i32) -> Canvas {
    let mut themes = vec![cued(interfaces::classic())];
    if with_blue {
        themes.push(cued(interfaces::theme_for(Interface::Blue)));
        themes.push(cued(interfaces::theme_for(Interface::Glass)));
    }
    let a4 = (21000, 29700);
    let (w, h) = printpreview::CLIENT;
    let col = w + 10;
    let mut c = Canvas::with_scale(
        (themes.len() as i32 * col + 10) as u32,
        (h + 20) as u32,
        scale,
    );
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    let mut page = Canvas::new(2480, 3508);
    page.fill_rect(page.bounds(), Color::rgb(255, 255, 255));
    let ink = Color::rgb(0, 0, 0);
    page.fill_rect(Rect::new(1040, 110, 400, 40), ink);
    for row in 0..44 {
        let len = 1890 * (4 + row % 5) / 8;
        page.fill_rect(Rect::new(236, 236 + row * 68, len, 40), ink);
    }
    page.fill_rect(Rect::new(1140, 3360, 200, 40), ink);
    for (i, theme) in themes.iter().enumerate() {
        let mut theme = theme.clone();
        theme.look = theme.look.at_scale(scale);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut pv = printpreview::Preview::new(a4);
        pv.laid(3);
        pv.go(1);
        let r = Rect::new(0, 0, w, h);
        let units = theme.text;
        let lay = printpreview::layout(&pv, r, scale, units, &theme.controls());
        if let Some((n, size)) = pv.wanted(&lay) {
            let at = Rect::new(0, 0, a4.0, a4.1);
            pv.show(n, printpreview::sheet(&page, at, a4, size));
        }
        let lay = printpreview::layout(&pv, r, scale, units, &theme.controls());
        let mut win = Canvas::with_scale(w as u32, h as u32, scale);
        win.fill_rect(win.bounds(), p.cs.button_face);
        printpreview::draw(&mut win, &text, &p, &pv, &lay, theme.fonts.size_px);
        c.blit_canvas(10 + i as i32 * col, 10, &win);
    }
    c
}
