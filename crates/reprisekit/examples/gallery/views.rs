use super::{blurred_stripes, cued};
use reprisekit::classic::indicator;
use reprisekit::controls::form::{self, Control, Item};
use reprisekit::controls::shellview;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};
use reprisekit::wallpaper::{self, Position};

/// One picture composed Center, Tile and Stretch on the monitor, then the Appearance sample of `sample_theme` per interface.
pub fn render_backgrounds() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(Theme::default());
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(1090, 396);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    // Red left half, white right half, a black band along the top.
    let mut pic = Canvas::new(64, 48);
    pic.fill_rect(pic.bounds(), Color::rgb(255, 255, 255));
    pic.fill_rect(Rect::new(0, 0, 32, 48), Color::rgb(200, 30, 30));
    pic.fill_rect(Rect::new(0, 0, 64, 4), Color::rgb(0, 0, 0));
    let teal = Color::rgb(0, 128, 128);
    let (_, _, sw, sh) = reprisekit::classic::assets::MONITOR_SCREEN;
    let mut items = Vec::new();
    let monitors = [
        ("center", Position::Center),
        ("tile", Position::Tile),
        ("stretch", Position::Stretch),
    ];
    for (i, (id, pos)) in monitors.into_iter().enumerate() {
        let screen = wallpaper::compose(&pic, sw as u32, sh as u32, pos, teal, 0.5);
        items.push(Item::new(
            id,
            Rect::new(10 + 211 * i as i32, 10, 191, 168),
            Control::Monitor(form::Screen::Picture(screen)),
        ));
    }
    let ruby = reprisekit::glass::Settings {
        color: reprisekit::glass::swatch("ruby").expect("ruby is a swatch"),
        ..reprisekit::glass::Settings::default()
    };
    let samples = [
        (
            "classic",
            cued(interfaces::sample_theme(
                Interface::Classic,
                "Brick",
                &ruby,
                None,
            )),
        ),
        (
            "blue",
            cued(interfaces::sample_theme(
                Interface::Blue,
                "Olive Green",
                &ruby,
                None,
            )),
        ),
        (
            "glass",
            cued(interfaces::sample_theme(
                Interface::Glass,
                "",
                &ruby,
                Some(teal),
            )),
        ),
    ];
    for (i, (id, t)) in samples.into_iter().enumerate() {
        items.push(Item::new(
            id,
            Rect::new(10 + 360 * i as i32, 198, 350, 188),
            Control::Sample {
                themes: false,
                theme: t,
                icon: None,
            },
        ));
    }
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form::Form::new(items),
        false,
        px,
    );
    c
}

/// The Screen Saver & Lock page's monitor with each saver at seed 7 after a second.
pub fn render_savers() -> Canvas {
    use reprisekit::saver::{Kind, Options, Saver};
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(Theme::default());
    let (_, _, sw, sh) = reprisekit::classic::assets::MONITOR_SCREEN;
    let mut c = Canvas::new(10 + 211 * 4, 188);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    let mut pic = Canvas::new(64, 48);
    pic.fill_rect(pic.bounds(), Color::rgb(255, 255, 255));
    pic.fill_rect(Rect::new(0, 0, 32, 48), Color::rgb(200, 30, 30));
    let mut items = Vec::new();
    let kinds = [Kind::Starfield, Kind::Mystify, Kind::Logo, Kind::Photos];
    for (i, kind) in kinds.into_iter().enumerate() {
        let mut saver = Saver::new(kind, &Options::default(), sw as u32, sh as u32, 7, &text);
        if saver.take_photo_request() {
            saver.push_photo(reprisekit::saver::fit(&pic, sw as u32, sh as u32));
        }
        for _ in 0..15 {
            saver.step();
        }
        let mut screen = Canvas::new(sw as u32, sh as u32);
        saver.draw(&mut screen);
        items.push(Item::new(
            kind.name(),
            Rect::new(10 + 211 * i as i32, 10, 191, 168),
            Control::Monitor(form::Screen::Picture(screen)),
        ));
    }
    form::draw(
        &mut c,
        &text,
        &theme.paint(),
        &form::Form::new(items),
        false,
        theme.fonts.size_px,
    );
    c
}

/// The right end of a 320 px bar with the Show desktop strip in each look, at rest and under the pointer.
pub fn render_show_desktop() -> Canvas {
    use reprisekit::taskbar::{TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(670, 140);
    let mut y = 10;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = cued(interfaces::theme_for(i));
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let shape = theme.taskbar_shape();
        let mut layout = TaskbarLayout::compute(
            &theme.metrics,
            &shape,
            320,
            true,
            counts,
            text.measure("3:45 PM", false, theme.fonts.size_px),
        );
        // Glass combines its buttons by default: an icon-only row.
        if i == Interface::Glass {
            layout.lay_tasks(&theme.metrics, &shape, &[], TaskRow::Icons);
        }
        for (x, hot) in [(10, false), (340, true)] {
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
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                task_progress: Vec::new(),
                task_badge: Vec::new(),
                show_desktop_hot: hot,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                meter: None,
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(320, layout.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
            c.blit_canvas(x, y, &bar);
        }
        y += layout.height + 8;
    }
    c
}

/// A terminal screen: a prompt, the 16 colours, attributes, box frames, blocks and wide chars.
/// The file preview's picture, text and card, a row per interface.
pub fn render_file_preview(scale: i32) -> Canvas {
    use reprisekit::controls::filepreview::{self, Body, Facts, FilePreview};
    use reprisekit::controls::multiedit::{Metrics, MultiEdit};
    let text = TextRenderer::load(&FontConfig::default());
    let mono = TextRenderer::monospace(13);
    let (cell_w, cell_h) = (380, 220);
    let mut out = Canvas::with_scale(3 * cell_w as u32, 3 * cell_h as u32, scale);
    out.fill_rect(out.bounds(), Color::rgb(0x40, 0x40, 0x40));
    for (row, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let mut theme = interfaces::theme_for(i);
        theme.look = theme.look.at_scale(scale);
        let p = theme.paint();
        let mut m = Metrics {
            bars_always: false,
            ..Metrics::default()
        };
        m.follow_look(&theme.look.controls());
        let (pw, ph) = (180 * scale as u32, 120 * scale as u32);
        let pixels = (0..pw * ph)
            .map(|k| 0xFF00_0000 | ((k % pw) * 255 / pw) << 16 | ((k / pw) * 255 / ph) << 8 | 0x60)
            .collect();
        let picture = Canvas::from_pixels(pw, ph, pixels).at_scale(scale);
        let doc = "[appearance]\ninterface = \"glass\"\n\n# Space shows a file;\n# the arrows walk the folder.";
        let bodies = [
            Body::Picture(picture),
            Body::Text(MultiEdit::new(doc)),
            Body::Card,
        ];
        for (col, body) in bodies.into_iter().enumerate() {
            let facts = Facts {
                name: "Holiday report.odt".into(),
                kind: "OpenDocument Text".into(),
                size: Some("48.2 KB".into()),
                modified: "10/3/2026 4:05 PM".into(),
            };
            let mut qv = FilePreview::new(facts, None);
            qv.set_body(body);
            let (w, h) = qv.size(&mono, &m, (cell_w - 20, cell_h - 20));
            let mut c = Canvas::with_scale(w as u32, h as u32, scale);
            let r = Rect::new(0, 0, w, h);
            filepreview::draw(&mut c, &p, &text, 11, &mono, &m, &qv, r);
            out.blit_canvas(col as i32 * cell_w + 10, row as i32 * cell_h + 10, &c);
        }
    }
    out
}

pub fn render_image_view(scale: i32) -> Canvas {
    use reprisekit::controls::imageview::{self, ImageView, Source, Zoom};
    let (cell_w, cell_h) = (260, 200);
    let mut out = Canvas::with_scale(3 * cell_w as u32, 3 * cell_h as u32, scale);
    out.fill_rect(out.bounds(), Color::rgb(0x40, 0x40, 0x40));
    let (pw, ph) = (480u32, 320u32);
    let pixels = (0..pw * ph)
        .map(|k| {
            let (x, y) = (k % pw, k / pw);
            let ring = ((x as i32 - 240).pow(2) + (y as i32 - 160).pow(2)) / 900 % 2;
            0xFF00_0000 | (x * 255 / pw) << 16 | (y * 255 / ph) << 8 | (0x40 + 0x80 * ring as u32)
        })
        .collect();
    let picture = Canvas::from_pixels(pw, ph, pixels);
    for (row, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
        .into_iter()
        .enumerate()
    {
        let mut theme = interfaces::theme_for(i);
        theme.look = theme.look.at_scale(scale);
        let p = theme.paint();
        // Fitted, at 100% and panned, turned a quarter turn at 200%.
        for col in 0..3 {
            let mut v = ImageView::new((pw, ph));
            let r = Rect::new(0, 0, cell_w - 20, cell_h - 20);
            match col {
                0 => {}
                1 => v.zoom = Zoom::Scale(1.0),
                _ => {
                    v.zoom = Zoom::Scale(2.0);
                    v.turn(true);
                }
            }
            let lay = imageview::layout(&v, r, scale, &theme.controls());
            v.pan(120, 90, &lay);
            let rendered = (lay.scale < 1.0).then(|| {
                imageview::render(
                    &picture,
                    v.turns,
                    (lay.content.0 as u32, lay.content.1 as u32),
                )
            });
            let mut c = Canvas::with_scale(r.width as u32, r.height as u32, scale);
            let src = Source {
                picture: &picture,
                rendered: rendered.as_ref(),
            };
            imageview::draw(&mut c, &p, &v, src, &lay, theme.colors.window);
            out.blit_canvas(col * cell_w + 10, row as i32 * cell_h + 10, &c);
        }
    }
    out
}

pub fn render_termgrid(scale: i32) -> Canvas {
    use reprisekit::controls::termgrid::{self, CursorShape, Flags, Snapshot};
    let text = TextRenderer::load(&FontConfig {
        family: vec![
            "Lucida Console".into(),
            "Liberation Mono".into(),
            "DejaVu Sans Mono".into(),
        ],
        size_px: 13,
        antialias: true,
        ..FontConfig::default()
    });
    let m = termgrid::metrics(&text, 13, scale);
    let (cols, rows) = (44, 12);
    let fg = Color::rgb(0xC0, 0xC0, 0xC0);
    let mut snap = Snapshot::new(cols, rows, fg, Color::rgb(0, 0, 0));
    let ansi: [u32; 16] = [
        0x000000, 0xCD3131, 0x0DBC79, 0xE5E510, 0x2472C8, 0xBC3FBC, 0x11A8CD, 0xE5E5E5, 0x666666,
        0xF14C4C, 0x23D18B, 0xF5F543, 0x3B8EEA, 0xD670D6, 0x29B8DB, 0xFFFFFF,
    ];
    for (i, v) in ansi.into_iter().enumerate() {
        let bg = Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
        snap.cells[cols + 2 * i].bg = bg;
        snap.cells[cols + 2 * i + 1].bg = bg;
    }
    let mut put = |row: usize, col: usize, s: &str, fg: Color, flags: Flags| {
        let mut c = col;
        for ch in s.chars() {
            let wide = flags.contains(Flags::WIDE);
            let cell = &mut snap.cells[row * cols + c];
            (cell.ch, cell.fg, cell.flags) = (ch, fg, flags);
            if wide {
                let spacer = &mut snap.cells[row * cols + c + 1];
                spacer.flags = Flags::WIDE_SPACER;
                c += 1;
            }
            c += 1;
        }
    };
    let green = Color::rgb(0x4E, 0xC0, 0x4E);
    let blue = Color::rgb(0x60, 0x90, 0xF0);
    put(0, 0, "user@reprise", green, Flags::BOLD);
    put(0, 12, ":", fg, Flags::default());
    put(0, 13, "~/src", blue, Flags::BOLD);
    put(0, 18, "$ ls --color", fg, Flags::default());
    for (col, (word, flags)) in [
        ("bold", Flags::BOLD),
        ("italic", Flags::ITALIC),
        ("under", Flags::UNDERLINE),
        ("strike", Flags::STRIKE),
        ("dim", Flags::DIM),
        ("inverse", Flags::INVERSE),
    ]
    .into_iter()
    .scan(0, |col, w| {
        let at = *col;
        *col += w.0.len() + 1;
        Some((at, w))
    }) {
        put(2, col, word, fg, flags);
    }
    put(3, 0, "selected", fg, Flags::SELECTED);
    put(3, 9, "match", fg, Flags::MATCH);
    put(3, 15, "current", fg, Flags::MATCH | Flags::MATCH_CURRENT);
    put(
        4,
        0,
        "┌──────┬───┐ ╔══════╦═══╗ ╭────╮",
        fg,
        Flags::default(),
    );
    put(
        5,
        0,
        "│ box  │ ├─┤ ║ dbl  ║ ╬ ║ │ ▲▼ │",
        fg,
        Flags::default(),
    );
    put(
        6,
        0,
        "├──────┼───┤ ╠══════╬═══╣ ╰────╯",
        fg,
        Flags::default(),
    );
    put(
        7,
        0,
        "└──────┴───┘ ╚══════╩═══╝ ╱╲╳ ┃━╋",
        fg,
        Flags::default(),
    );
    put(
        8,
        0,
        "▀▄█▌▐░▒▓ ▁▂▃▄▅▆▇█ ▖▗▘▙▚▛▜▝▞▟ ┄┅┆┈╌╍",
        fg,
        Flags::default(),
    );
    put(9, 0, "日本語", fg, Flags::WIDE);
    put(9, 7, "wide chars", fg, Flags::default());
    put(10, 0, "$ ", fg, Flags::default());
    snap.cursor = (10, 2, CursorShape::Block);
    let w = (cols as i32 * m.cell_w + scale - 1) / scale;
    let h = (rows as i32 * m.cell_h + scale - 1) / scale;
    let mut c = Canvas::with_scale(w as u32, h as u32, scale);
    termgrid::draw(&mut c, &text, &m, Rect::new(0, 0, w, h), &snap, 13);
    c
}

/// A 640 px bar along the top in Classic, Blue, Glass and Glass's small bar over the stripes; the orb's buffer goes over each Glass bar.
/// A taskbar per look with a copy running, one paused and one without progress.
/// A Tiles and a Thumbnails drag of three entries pressed near the second's left edge, each image
/// over the view's window colour.
pub fn render_grid_drag(scale: i32) -> Canvas {
    let theme = cued(Theme::default()).scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new((600 * scale) as u32, (330 * scale) as u32);
    c.fill_rect(c.bounds(), theme.colors.window);
    let mut x = 10;
    for (mode, px) in [
        (shellview::Mode::Tiles, 48),
        (shellview::Mode::Thumbnails, 80),
    ] {
        let sv = shellview::sample(mode, false);
        let lay = shellview::layout(&sv, Rect::new(0, 0, 500, 400), &theme.controls());
        let icons: Vec<(usize, Canvas)> = [0, 1, 2]
            .into_iter()
            .map(|i| {
                let argb = sv.entries[i].icon.as_ref().map_or(0, |c| c.pixels[0]);
                let n = (px * scale) as u32;
                (i, Canvas::from_pixels(n, n, vec![argb; (n * n) as usize]))
            })
            .collect();
        let rows: Vec<shellview::DragRow> = icons
            .iter()
            .map(|(index, icon)| shellview::DragRow {
                index: *index,
                icon: Some(icon),
            })
            .collect();
        let cell = lay.cells[1];
        let at = (cell.x + 8, cell.y + cell.height / 2);
        if let Some((img, _)) = shellview::drag_image(&theme, &text, &sv, &lay, &rows, at) {
            c.blit_over(x * scale, 10 * scale, &img);
            x += img.width as i32 / scale + 20;
        }
    }
    c
}

/// The pointer's rings at five steps of their animation, a row per interface, each step over a
/// light and a dark tile.
pub fn render_locate_rings(scale: i32) -> Canvas {
    use reprisekit::locate::{SIZE, rings};
    let themes = [
        interfaces::theme_for(Interface::Classic),
        interfaces::theme_for(Interface::Blue),
        interfaces::theme_for(Interface::Glass),
    ];
    let steps = [0.0, 0.2, 0.4, 0.6, 0.8];
    let tiles = [Color::rgb(0xF0, 0xF0, 0xF0), Color::rgb(0x20, 0x28, 0x30)];
    let cell = SIZE * scale;
    let cols = (steps.len() * tiles.len()) as i32;
    let mut c = Canvas::new((cols * cell) as u32, (themes.len() as i32 * cell) as u32);
    for (row, theme) in themes.iter().enumerate() {
        for (i, &t) in steps.iter().enumerate() {
            for (j, &tile) in tiles.iter().enumerate() {
                let x = (i * tiles.len() + j) as i32 * cell;
                let y = row as i32 * cell;
                c.fill_rect(Rect::new(x, y, cell, cell), tile);
                c.blit_over(x, y, &rings(t, theme.colors.hilight, scale));
            }
        }
    }
    c
}
