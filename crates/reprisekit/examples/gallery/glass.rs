use super::glass_controls::{
    render_glass_balloons, render_glass_choices, render_glass_fields, render_glass_lists,
    render_glass_marquees, render_glass_menus, render_glass_open_lists, render_glass_ranges,
    render_glass_scrollbars, render_glass_toolbars, render_glass_wells,
};
use super::glass_shell::{render_glass_frames, render_glass_spots, render_glass_task_rows};
use super::popups::{
    render_glass_flyouts, render_glass_items, render_glass_notify_sheets, render_glass_popup_parts,
};
use super::sheets::render_glass_sheet;
use super::{blurred_stripes, cued};
use reprisekit::classic::frame::{CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::classic::indicator;
use reprisekit::controls::{switcher, taskpreview};
use reprisekit::glass::shadow;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::pointer::Interaction;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};

/// Glass's frames, taskbars, flyouts and control sheets (each in its own `render_glass_*`), over a box-blurred stripe backdrop standing in for the blurred desktop. Drawn once: Glass has one scheme.
pub fn render_glass() -> Canvas {
    let theme = cued(interfaces::theme_for(Interface::Glass));
    let frames = render_glass_frames();
    let parts = [
        render_glass_task_rows(),
        render_glass_spots(),
        render_glass_choices(&theme),
        render_glass_sheet(&theme),
        render_glass_fields(&theme),
        render_glass_open_lists(&theme),
        render_glass_scrollbars(&theme),
        render_glass_ranges(&theme),
        render_glass_lists(&theme),
        render_glass_wells(&theme),
        render_glass_popup_parts(&theme),
        render_glass_menus(&theme),
        render_glass_marquees(&theme),
        render_glass_balloons(&theme),
        render_glass_notify_sheets(&theme),
        render_glass_flyouts(&theme),
        render_glass_items(&theme),
        render_glass_toolbars(&theme),
        render_glass_materials(reprisekit::glass::Mode::Light),
        render_glass_square_frames(),
        render_glass_client_glass(&theme),
        render_glass_toolbar_on_glass(&theme),
        render_glass_peek_sheet(&theme),
    ];
    let height = frames.height + parts.iter().map(|p| p.height).sum::<u32>();
    let mut all = Canvas::new(950, height);
    all.blit_canvas(0, 0, &frames);
    let mut y = frames.height as i32;
    for p in &parts {
        all.blit_canvas(0, y, p);
        y += p.height as i32;
    }
    all
}

/// Dark Glass's captions, then the same control rows as [`render_glass`] under the dark scheme, then its Smoked and Molten captions.
pub fn render_glass_dark() -> Canvas {
    let theme = cued(interfaces::theme_with(
        Interface::Glass,
        reprisekit::blue::Variant::default(),
        &reprisekit::glass::Settings {
            mode: reprisekit::glass::Mode::Dark,
            ..reprisekit::glass::Settings::default()
        },
    ));
    let parts = [
        render_glass_dark_captions(&theme),
        render_glass_client_glass(&theme),
        render_glass_toolbar_on_glass(&theme),
        render_glass_dark_bar(&theme),
        render_glass_choices(&theme),
        render_glass_sheet(&theme),
        render_glass_fields(&theme),
        render_glass_open_lists(&theme),
        render_glass_scrollbars(&theme),
        render_glass_ranges(&theme),
        render_glass_lists(&theme),
        render_glass_wells(&theme),
        render_glass_popup_parts(&theme),
        render_glass_menus(&theme),
        render_glass_marquees(&theme),
        render_glass_balloons(&theme),
        render_glass_notify_sheets(&theme),
        render_glass_flyouts(&theme),
        render_glass_items(&theme),
        render_glass_toolbars(&theme),
        render_glass_materials(reprisekit::glass::Mode::Dark),
        render_glass_peek_sheet(&theme),
    ];
    let mut all = Canvas::new(950, parts.iter().map(|p| p.height).sum::<u32>());
    let mut y = 0;
    for p in &parts {
        all.blit_canvas(0, y, p);
        y += p.height as i32;
    }
    all
}

/// Glass windows at scale 2 over the stripes, 1300x700 device px, then the Start panel and popups ([`glass_popups_at_2`]) and lit and held caption buttons ([`render_glass_scale2_buttons`]).
pub fn render_glass_scale2() -> Canvas {
    use reprisekit::glass::{self, Frame, Material, Settings};
    let rows = [
        Settings::default(),
        Settings {
            frame: Frame::Square,
            ..Settings::default()
        },
        Settings {
            color: glass::swatch("frost").expect("frost is a swatch"),
            intensity: 100,
            ..Settings::default()
        },
        Settings {
            color: glass::swatch("blush").expect("blush is a swatch"),
            intensity: 100,
            material: Material::Molten,
            ..Settings::default()
        },
    ];
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(1300, 700 + GLASS_SCALE2_BARS + POPUPS_AT_2);
    glass_scale2_bars(&mut c, &rows, 700);
    glass_popups_at_2(&mut c, 700 + GLASS_SCALE2_BARS, &text);
    for (row, s) in rows.iter().enumerate() {
        let theme = cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            s,
        ));
        let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let two = theme.scaled(2);
        for (i, active) in [true, false].into_iter().enumerate() {
            let g = FrameGeometry::layout(&theme.metrics, 290, 30, false, Chrome::WINDOW).scaled(2);
            let st = FrameState {
                title: "before.txt - Notepad",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &two, &text, &caption_text, &g, &st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
            c.blit_over(20 + 640 * i as i32, 20 + 170 * row as i32, &f);
        }
    }
    let buttons = render_glass_scale2_buttons(&rows);
    let mut all = Canvas::new(c.width, c.height + buttons.height);
    all.blit_canvas(0, 0, &c);
    all.blit_canvas(0, c.height as i32, &buttons);
    all
}

/// Caption buttons at scale 2 over the stripes, 1300x480 device px, a row per setting of [`render_glass_scale2`].
fn render_glass_scale2_buttons(rows: &[reprisekit::glass::Settings]) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(1300, 120 * rows.len() as i32);
    for (row, s) in rows.iter().enumerate() {
        let theme = cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            s,
        ));
        let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
        let two = theme.scaled(2);
        let state = |glow, pressed| FrameState {
            title: "Notes",
            active: true,
            maximized: false,
            pressed,
            glow: Glow::lit(glow),
            icon: None,
            tabbed: false,
        };
        let y = 12 + 120 * row as i32;
        for (i, st) in [
            state(Some(CaptionButton::Maximize), None),
            state(None, Some(CaptionButton::Close)),
        ]
        .iter()
        .enumerate()
        {
            let g = FrameGeometry::layout(&theme.metrics, 180, 10, false, Chrome::WINDOW).scaled(2);
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &two, &text, &caption_text, &g, st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
            c.blit_over(20 + 420 * i as i32, y, &f);
        }
        let g = FrameGeometry::layout(&theme.metrics, 180, 10, false, Chrome::DIALOG).scaled(2);
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_dialog_frame(
            &mut f,
            &two,
            &text,
            &g,
            &state(None, None),
            Some(Interaction::Hot),
        );
        f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        c.blit_over(860, y, &f);
    }
    c
}

/// How many rows [`glass_popups_at_2`] draws, 1300 px wide.
const POPUPS_AT_2: i32 = 1960;

/// Glass's Start panel, Alt+Tab panel and task preview at scale 2 in Sky, drawn into `c` from row `y0` down.
fn glass_popups_at_2(c: &mut Canvas, y0: i32, text: &TextRenderer) {
    use reprisekit::startpanel::{self as sp, PlaceRow, ProgramRow, Spot, StartPanel};
    let theme = cued(interfaces::theme_for(Interface::Glass));
    let two = theme.scaled(2);
    let glass = reprisekit::glass::Settings::default();
    let face = interfaces::start_panel_text(&theme);
    let prog = |name: &str| ProgramRow {
        detail: None,
        name: name.into(),
        icon: None,
        jump: false,
        new_pin: false,
    };
    let start = |lit: Spot| {
        let mut p = StartPanel::new();
        p.pinned = vec![prog("Files"), prog("Terminal")];
        p.frequent = ["Notepad", "Calculator", "Paint"].map(prog).to_vec();
        p.places = ["user", "Documents", "Pictures", "Music", "Computer"]
            .iter()
            .map(|l| PlaceRow {
                label: l.to_string(),
                picture: None,
                rule_before: *l == "Computer",
                submenu: false,
            })
            .collect();
        p.pointer(Some(lit));
        p
    };
    for (k, lit) in [Spot::Program(1), Spot::Shutdown].into_iter().enumerate() {
        let p = start(lit);
        let l = sp::layout::layout(&p).scaled(2);
        let mut panel = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        interfaces::draw_start_panel(&mut panel, &two, &glass, &face, &p, &l, None, true);
        if k == 0 {
            c.blit_over(20, y0 + 20, &panel);
        } else {
            let o = l.canvas;
            let b = l.rect(Spot::Shutdown).expect("Shut down");
            let a = l.rect(Spot::ShutdownArrow).expect("its arrow half");
            let both = Rect::new(
                b.x - o.x - 20,
                b.y - o.y - 20,
                a.right() - b.x + 40,
                b.height + 40,
            );
            c.blit_over(900, y0 + 20, &panel.copy_rect(both));
        }
    }
    let grid = interfaces::switcher_grid(&theme);
    let l = switcher::SwitcherLayout::thumbnails(3, 1, Rect::new(0, 0, 640, 300), grid).scaled(2);
    let pictures: Vec<Rect> = l
        .wells
        .iter()
        .map(|&w| taskpreview::fit(400, 300, w, 2))
        .collect();
    let stills: Vec<Canvas> = pictures
        .iter()
        .map(|r| {
            let mut s = Canvas::new(r.width as u32, r.height as u32);
            s.fill_rect(s.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
            s.fill_rect(Rect::new(0, 0, r.width, 12), Color::rgb(0xDD, 0xE6, 0xF2));
            s
        })
        .collect();
    let cells: Vec<switcher::ThumbCell> = pictures
        .iter()
        .zip(&stills)
        .map(|(&r, s)| switcher::ThumbCell {
            icon: None,
            picture: Some(r),
            still: Some(s),
        })
        .collect();
    let a = l.canvas();
    let mut panel = Canvas::new(a.width as u32, a.height as u32);
    interfaces::draw_switcher(
        &mut panel,
        &two,
        text,
        &l,
        "Untitled - Notepad",
        &cells,
        1,
        Some(2),
    );
    c.blit_over(20, y0 + 1100, &panel);
    let l = taskpreview::TaskPreviewLayout::compute(
        interfaces::task_preview_grid(&theme),
        2,
        Rect::new(300, 400, 60, 40),
        Rect::new(0, 0, 640, 400),
        reprisekit::taskbar::Edge::Bottom,
    )
    .scaled(2);
    let cells: Vec<taskpreview::Cell> = ["Untitled - Notepad", "notes.txt - Notepad"]
        .iter()
        .map(|&title| taskpreview::Cell {
            title,
            icon: None,
            active: false,
            picture: None,
        })
        .collect();
    let a = l.canvas();
    let mut panel = Canvas::new(a.width as u32, a.height as u32);
    interfaces::draw_task_preview(&mut panel, &two, text, &l, &cells, Some(1));
    c.blit_over(20, y0 + 1500, &panel);
}

/// How tall [`glass_scale2_bars`] is.
const GLASS_SCALE2_BARS: i32 = 4 * 92 + 90 + 580;

/// Glass taskbars at scale 2 into `c` from row `top`: a 640 px bar per setting in `rows` along the bottom, then 40 px and small bars as 70 px columns up the left of a 280 px output.
fn glass_scale2_bars(c: &mut Canvas, rows: &[reprisekit::glass::Settings], top: i32) {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PagerButton, TaskEntry, TaskRow, TaskbarCounts,
        TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let bar = |c: &mut Canvas, theme: &Theme, edge: Edge, t: i32, length: i32, at: (i32, i32)| {
        let style = theme.look.style().unwrap();
        let shape = theme.taskbar_shape();
        let start_text = TextRenderer::load(&style.start_text.font());
        let clock = ClockText {
            time: text.measure("3:45 PM", false, theme.fonts.size_px),
            date: text.measure("10/04/2026", false, theme.fonts.size_px),
        };
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 4,
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length,
            thickness: t,
        };
        let mut l = TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            counts,
            clock,
            Bands::default(),
        );
        l.lay_tasks(&theme.metrics, &shape, &[0, 2, 1, 3], TaskRow::Shared);
        let px = shape.task_icon * 2;
        let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
            title: title.into(),
            active,
            attention: reprisekit::taskbar::Attention::None,
            glow: Some(Color::rgb(0xD0, 0x40, 0x30)),
            icon: Some(icon.into()),
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: Vec::new(),
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: vec![
                PagerButton::default(),
                PagerButton {
                    current: true,
                    attention: false,
                },
            ],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: Some(3),
            task_pressed: Some(1),
            task_hot_x: Some(12),
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            show_desktop_hot: true,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                task("", false, indicator::draw_record(px)),
                task("Command Prompt (2)", false, indicator::draw_microphone(px)),
                task("Untitled - Notepad", true, indicator::draw_record(px)),
                task("Pictures (3)", false, indicator::draw_microphone(px)),
            ],
            tray_icons: vec![Some(indicator::draw_record(32).into())],
            meter: None,
            clock: "3:45 PM".into(),
        };
        let two = theme.scaled(2);
        let l = l.scaled(2);
        let mut canvas = Canvas::new(l.width as u32, l.height as u32);
        look::draw_taskbar(&mut canvas, &two, &text, &start_text, &l, &state);
        c.blit_over(at.0, at.1, &canvas);
    };
    let themes: Vec<Theme> = rows
        .iter()
        .map(|s| {
            cued(interfaces::theme_with(
                Interface::Glass,
                reprisekit::blue::Variant::default(),
                s,
            ))
        })
        .collect();
    for (i, theme) in themes.iter().enumerate() {
        bar(
            c,
            theme,
            Edge::Bottom,
            40,
            640,
            (10, top + 10 + 92 * i as i32),
        );
    }
    let mut small = themes[0].clone();
    interfaces::small_bar(&mut small, &rows[0]);
    let below = top + 10 + 92 * rows.len() as i32;
    bar(c, &small, Edge::Bottom, 30, 640, (10, below));
    let side = below + 80;
    bar(c, &themes[0], Edge::Left, 70, 280, (10, side));
    bar(c, &small, Edge::Left, 70, 280, (170, side));
}

/// Dark Glass's taskbar over the stripes with two windows lacking icons: each shows the light window glyph.
fn render_glass_dark_bar(theme: &Theme) -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let start_text = TextRenderer::load(&style.start_text.font());
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &style.taskbar,
        930,
        true,
        TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        },
        text.measure("3:45 PM", false, theme.fonts.size_px),
    );
    let entry = |title: &str, active: bool| TaskEntry {
        title: title.into(),
        active,
        attention: reprisekit::taskbar::Attention::None,
        icon: None,
        glow: None,
    };
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
        tasks: vec![entry("xclock", false), entry("xeyes", true)],
        tray_icons: vec![],
        meter: None,
        clock: "3:45 PM".into(),
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
    };
    let mut c = blurred_stripes(950, 60);
    let mut bar = Canvas::new(930, layout.height as u32);
    look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
    c.blit_over(10, 10, &bar);
    c
}

/// Two restored dark Glass windows over the stripes, 950x80, the focused one first.
fn render_glass_dark_captions(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 80);
    for (i, active) in [true, false].into_iter().enumerate() {
        let g = FrameGeometry::layout(&theme.metrics, 424, 20, false, Chrome::WINDOW);
        let st = FrameState {
            title: "before.txt - Notepad",
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
        f.fill_rect(g.client, theme.colors.button_face);
        c.blit_canvas(15 + 470 * i as i32, 10, &f);
    }
    c
}

/// Two Glass windows over the stripes and a white window, the focused one first, 950x110, each
/// with a menu bar across its client's top: the bar clear over the frame's glass, its titles
/// glowing, the client below.
fn render_glass_client_glass(theme: &Theme) -> Canvas {
    use reprisekit::controls::menubar::Bar;
    let text = TextRenderer::load(&FontConfig::default());
    let style = theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 110);
    for x in [115, 585] {
        c.fill_rect(Rect::new(x, 0, 220, 110), Color::rgb(0xFF, 0xFF, 0xFF));
    }
    let bar = Bar::new(vec![
        ("&File", vec![]),
        ("&Edit", vec![]),
        ("&View", vec![]),
        ("F&avorites", vec![]),
    ]);
    for (i, active) in [true, false].into_iter().enumerate() {
        let g = FrameGeometry::layout(&theme.metrics, 424, 50, false, Chrome::WINDOW);
        let themed = Theme {
            active,
            access_keys: false,
            ..theme.clone()
        };
        let mut client = Canvas::new(g.client.width as u32, g.client.height as u32);
        client.fill_rect(client.bounds(), theme.colors.button_face);
        let lay = bar.layout(
            &text,
            theme.fonts.size_px,
            client.bounds(),
            &theme.controls(),
        );
        bar.draw(&mut client, &text, &themed, &lay);
        let g = g.with_glass_top(client.take_glass_top());
        let st = FrameState {
            title: "Documents",
            active,
            maximized: false,
            pressed: None,
            glow: Glow::lit(None),
            icon: None,
            tabbed: false,
        };
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
        let band = Rect::new(g.client.x, g.client.y, g.client.width, g.glass_top);
        look::draw_client_glass(&mut f, &themed, band, g.client.height, false);
        f.blit_over(g.client.x, g.client.y, &client);
        c.blit_over(15 + 470 * i as i32, 10, &f);
    }
    c
}

/// Explorer's toolbar strip clear over a client's glass, 950x50: over the stripes, then over a
/// bright backdrop, Back and Forward greyed, Up and Folders live, the label glow under them.
fn render_glass_toolbar_on_glass(theme: &Theme) -> Canvas {
    use reprisekit::controls::toolbar::{self, Item, ToolBar};
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let ink = look::ink_on_glass(theme).unwrap_or(theme.colors.button_text);
    let icon = || Some(reprisekit::art::icons::COMPUTER.canvas());
    let tb = ToolBar {
        items: vec![
            Item::DropButton {
                id: toolbar::BACK,
                icon: icon(),
                label: Some("Back".into()),
                enabled: false,
            },
            Item::DropButton {
                id: toolbar::FORWARD,
                icon: icon(),
                label: Some(String::new()),
                enabled: false,
            },
            Item::Button {
                id: toolbar::UP,
                icon: icon(),
                label: None,
                enabled: true,
            },
            Item::Separator,
            Item::Button {
                id: toolbar::FOLDERS,
                icon: icon(),
                label: Some("Folders".into()),
                enabled: true,
            },
        ],
        hot: None,
        pressed: None,
        latched: None,
        big: true,
    };
    let mut c = blurred_stripes(950, 50);
    c.fill_rect(Rect::new(475, 0, 475, 50), Color::rgb(0xF4, 0xF6, 0xF8));
    for x in [15, 490] {
        let r = Rect::new(0, 0, 440, 40);
        let rects = toolbar::layout(&tb, &text, theme.fonts.size_px, r, &theme.controls());
        let mut strip = Canvas::new(r.width as u32, r.height as u32);
        toolbar::draw_over_glass(
            &mut strip,
            &text,
            &p,
            &tb,
            r,
            &rects,
            ink,
            theme.fonts.size_px,
        );
        look::glow_on_glass(&mut strip, theme, r);
        toolbar::draw_glass_separators(&mut strip, &p, &tb, &rects);
        c.blit_over(x, 5, &strip);
    }
    c
}

/// An inactive window, then the sheet a peek draws in its place, over the stripes, 950x150.
fn render_glass_peek_sheet(theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let caption_text = TextRenderer::load(&theme.look.style().unwrap().caption_text.font());
    let mut c = blurred_stripes(950, 150);
    let g = FrameGeometry::layout(&theme.metrics, 400, 90, false, Chrome::WINDOW);
    let st = FrameState {
        title: "before.txt - Notepad",
        active: false,
        maximized: false,
        pressed: None,
        glow: Glow::lit(None),
        icon: None,
        tabbed: false,
    };
    let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    look::draw_frame(&mut f, theme, &text, &caption_text, &g, &st);
    f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
    c.blit_over(15, 10, &f);
    let mut sheet = Canvas::new(g.total.0 as u32, g.total.1 as u32);
    let r = sheet.bounds();
    look::draw_sheet(&mut sheet, theme, r);
    c.blit_over(485, 10, &sheet);
    c
}

/// Smoked then Molten windows in `mode` over the stripes, 950x150.
fn render_glass_materials(mode: reprisekit::glass::Mode) -> Canvas {
    use reprisekit::glass::{Material, Settings};
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = blurred_stripes(950, 150);
    for (row, material) in [Material::Smoked, Material::Molten].into_iter().enumerate() {
        let theme = cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &Settings {
                mode,
                material,
                ..Settings::default()
            },
        ));
        let style = theme.look.style().unwrap();
        let caption_text = TextRenderer::load(&style.caption_text.font());
        for (i, active) in [true, false].into_iter().enumerate() {
            let g = FrameGeometry::layout(&theme.metrics, 424, 20, false, Chrome::WINDOW);
            let st = FrameState {
                title: "before.txt - Notepad",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            };
            let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
            look::draw_frame(&mut f, &theme, &text, &caption_text, &g, &st);
            f.fill_rect(g.client, theme.colors.button_face);
            c.blit_canvas(15 + 470 * i as i32, 10 + 70 * row as i32, &f);
        }
    }
    c
}

/// Glass's Square frame (`[glass] frame = "square"`) over the stripes, 950x980: windows, caption button states, dialogs, dark Glass and an overlong title.
fn render_glass_square_frames() -> Canvas {
    use reprisekit::glass::{self, square};
    let square_settings = |mode| glass::Settings {
        frame: glass::Frame::Square,
        mode,
        ..glass::Settings::default()
    };
    let themes = [glass::Mode::Light, glass::Mode::Dark].map(|m| {
        cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &square_settings(m),
        ))
    });
    let text = TextRenderer::load(&FontConfig::default());
    let caption_text = TextRenderer::load(&themes[0].look.style().unwrap().caption_text.font());
    let mut c = blurred_stripes(950, 980);
    let mut frame = |theme: &Theme,
                     (x, y): (i32, i32),
                     (cw, ch): (i32, i32),
                     chrome: Chrome,
                     help: Option<Interaction>,
                     st: &FrameState| {
        let g = FrameGeometry::layout(&theme.metrics, cw, ch, st.maximized, chrome);
        let r = Rect::new(x, y, g.total.0, g.total.1);
        for (i, p) in shadow::place(r, st.active, st.maximized).iter().enumerate() {
            let piece = square::shadow_piece(st.active, i, 1);
            c.blit_canvas(p.dest.x, p.dest.y, &stretched(&piece, p));
        }
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        if chrome == Chrome::WINDOW {
            look::draw_frame(&mut f, theme, &text, &caption_text, &g, st);
            f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        } else {
            look::draw_dialog_frame(&mut f, theme, &text, &g, st, help);
        }
        c.blit_over(x, y, &f);
    };
    let st = |title: &'static str,
              active: bool,
              maximized: bool,
              hot: Option<CaptionButton>,
              pressed: Option<CaptionButton>| FrameState {
        title,
        active,
        maximized,
        pressed,
        glow: Glow::lit(hot),
        icon: None,
        tabbed: false,
    };
    let [light, dark] = &themes;
    let win = Chrome::WINDOW;
    use CaptionButton::{Close, Maximize, Minimize};
    frame(
        light,
        (15, 15),
        (424, 100),
        win,
        None,
        &st("Notes", true, false, None, None),
    );
    frame(
        light,
        (485, 15),
        (424, 100),
        win,
        None,
        &st("Notes", false, false, None, None),
    );
    frame(
        light,
        (15, 190),
        (910, 60),
        win,
        None,
        &st("Notes", true, true, None, None),
    );
    for (i, b) in [Minimize, Maximize, Close].into_iter().enumerate() {
        let x = 15 + 310 * i as i32;
        let lit = st("Notes", true, false, Some(b), None);
        frame(light, (x, 295), (284, 32), win, None, &lit);
        let held = st("Notes", true, false, None, Some(b));
        frame(light, (x, 395), (284, 32), win, None, &held);
    }
    let unfocused_lit = st("Notes", false, false, Some(Close), None);
    frame(light, (15, 495), (284, 32), win, None, &unfocused_lit);
    let dialog = Chrome::DIALOG;
    for (i, active) in [true, false].into_iter().enumerate() {
        let x = 325 + 310 * i as i32;
        frame(
            light,
            (x, 495),
            (284, 32),
            dialog,
            None,
            &st("Run", active, false, None, None),
        );
    }
    let (idle, hot, held) = (Interaction::Idle, Interaction::Hot, Interaction::Pressed);
    for (i, (active, ptr, close_lit)) in [
        (true, idle, None),
        (true, hot, None),
        (true, held, None),
        (true, idle, Some(Close)),
        (false, idle, None),
    ]
    .into_iter()
    .enumerate()
    {
        let at = (15 + 186 * i as i32, 595);
        let s = st("Run", active, false, close_lit, None);
        frame(light, at, (160, 2), Chrome::HELP_DIALOG, Some(ptr), &s);
    }
    let restore_lit = st("Notes", true, true, Some(Maximize), None);
    frame(light, (15, 670), (450, 40), win, None, &restore_lit);
    let close_held = st("Notes", true, true, None, Some(Close));
    frame(light, (485, 670), (450, 40), win, None, &close_held);
    frame(
        dark,
        (15, 750),
        (424, 60),
        win,
        None,
        &st("Notes", true, false, None, None),
    );
    frame(
        dark,
        (485, 750),
        (424, 60),
        win,
        None,
        &st("Notes", false, false, None, None),
    );
    let long = "A title too long to sit centred in so narrow a window";
    frame(
        light,
        (15, 890),
        (220, 20),
        win,
        None,
        &st(long, true, false, None, None),
    );
    c
}

/// `p.src` of `piece` stretched to `p.dest`'s size, as the scene shows a shadow piece.
pub fn stretched(piece: &Canvas, p: &shadow::Placed) -> Canvas {
    let (d, s) = (p.dest, p.src);
    let mut out = Canvas::new(d.width.max(0) as u32, d.height.max(0) as u32);
    for y in 0..d.height {
        for x in 0..d.width {
            let px = piece.get(s.x + x * s.width / d.width, s.y + y * s.height / d.height);
            out.put_raw(x, y, px);
        }
    }
    out
}
