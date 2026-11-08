use super::glass::stretched;
use super::{blurred_stripes, cued};
use reprisekit::classic::frame::{CaptionButton, Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::classic::indicator;
use reprisekit::controls::{switcher, taskpreview};
use reprisekit::glass::shadow;
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};

/// [`render_glass`]'s frames, taskbars and flyout, 950x1120.
pub fn render_glass_frames() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let glass_theme = cued(interfaces::theme_for(Interface::Glass));
    let style = glass_theme.look.style().unwrap();
    let caption_text = TextRenderer::load(&style.caption_text.font());
    let mut c = blurred_stripes(950, 1120);
    let mut glass_frame = |x: i32, y: i32, cw: i32, ch: i32, chrome: Chrome, st: &FrameState| {
        let g = FrameGeometry::layout(&glass_theme.metrics, cw, ch, st.maximized, chrome);
        let frame = Rect::new(x, y, g.total.0, g.total.1);
        for (i, p) in shadow::place(frame, st.active, st.maximized)
            .iter()
            .enumerate()
        {
            c.blit_canvas(
                p.dest.x,
                p.dest.y,
                &stretched(&shadow::piece(st.active, i, 1), p),
            );
        }
        let mut f = Canvas::new(g.total.0 as u32, g.total.1 as u32);
        look::draw_frame(&mut f, &glass_theme, &text, &caption_text, &g, st);
        f.fill_rect(g.client, Color::rgb(0xFF, 0xFF, 0xFF));
        c.blit_canvas(x, y, &f);
    };
    let st = |active: bool,
              maximized: bool,
              hot: Option<CaptionButton>,
              pressed: Option<CaptionButton>| FrameState {
        title: "Notes",
        active,
        maximized,
        pressed,
        glow: Glow::lit(hot),
        icon: None,
        tabbed: false,
    };
    let win = Chrome::WINDOW;
    glass_frame(15, 15, 424, 162, win, &st(true, false, None, None));
    glass_frame(485, 15, 424, 162, win, &st(false, false, None, None));
    glass_frame(15, 260, 910, 150, win, &st(true, true, None, None));
    use CaptionButton::{Close, Maximize, Minimize};
    for (i, b) in [Minimize, Maximize, Close].into_iter().enumerate() {
        let x = 15 + 310 * i as i32;
        glass_frame(x, 435, 284, 32, win, &st(true, false, Some(b), None));
        glass_frame(x, 515, 284, 32, win, &st(true, false, None, Some(b)));
    }
    glass_frame(15, 595, 284, 32, win, &st(false, false, Some(Close), None));
    glass_frame(
        325,
        605,
        300,
        40,
        win,
        &st(true, true, Some(Maximize), None),
    );
    let dialog = Chrome::DIALOG;
    glass_frame(635, 595, 284, 32, dialog, &st(true, false, None, None));
    glass_frame(
        325,
        955,
        300,
        40,
        win,
        &st(false, true, Some(Maximize), None),
    );
    glass_frame(
        635,
        955,
        300,
        40,
        win,
        &st(true, true, None, Some(Maximize)),
    );
    for (i, glow) in [[0.5, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.25, 0.75]]
        .into_iter()
        .enumerate()
    {
        let st = FrameState {
            glow: Glow(glow),
            ..st(true, false, None, None)
        };
        glass_frame(15 + 310 * i as i32, 1040, 284, 32, win, &st);
    }

    {
        use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
        let start_text = TextRenderer::load(&style.start_text.font());
        let clock = "3:45 PM\n9/27/2026";
        let clock_w = clock
            .lines()
            .map(|l| text.measure(l, false, glass_theme.fonts.size_px))
            .max()
            .unwrap_or(0);
        let layout = TaskbarLayout::compute(
            &glass_theme.metrics,
            &style.taskbar,
            930,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 3,
                tray: 2,
                chevron: false,
                show_desktop: false,
                ..TaskbarCounts::default()
            },
            clock_w,
        );
        for (i, y) in [690, 745].into_iter().enumerate() {
            let hot = i == 1;
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
                start_hot: hot,
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
                task_hot: hot.then_some(2),
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                task_progress: Vec::new(),
                task_badge: Vec::new(),
                tasks: vec![
                    entry("Notes", false),
                    entry("Explorer", true),
                    TaskEntry {
                        glow: Some(Color::rgb(0xF0, 0xC8, 0x40)),
                        ..entry("Terminal", false)
                    },
                ],
                tray_icons: vec![
                    Some(indicator::draw_record(16).into()),
                    Some(indicator::draw_microphone(16).into()),
                ],
                meter: None,
                clock: clock.into(),
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
            };
            let mut bar = Canvas::new(930, 40);
            look::draw_taskbar(&mut bar, &glass_theme, &text, &start_text, &layout, &state);
            c.blit_canvas(10, y, &bar);
        }
    }

    {
        use reprisekit::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarState};
        use reprisekit::trayflyout::{self, FlyoutLayout};
        let start_text = TextRenderer::load(&style.start_text.font());
        let layout = TaskbarLayout::compute(
            &glass_theme.metrics,
            &style.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 0,
                tray: 2,
                chevron: true,
                show_desktop: false,
                ..TaskbarCounts::default()
            },
            text.measure("00:00", false, glass_theme.fonts.size_px),
        );
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
            tasks: vec![],
            tray_icons: vec![
                Some(indicator::draw_record(16).into()),
                Some(indicator::draw_microphone(16).into()),
            ],
            meter: None,
            clock: "3:45 PM".into(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(800, 40);
        look::draw_taskbar(&mut bar, &glass_theme, &text, &start_text, &layout, &state);
        c.blit_over(15, 890, &bar);
        let icons: Vec<Option<Canvas>> = (0..6)
            .map(|i| {
                Some(if i % 2 == 0 {
                    indicator::draw_record(16)
                } else {
                    indicator::draw_microphone(16)
                })
            })
            .collect();
        let chevron = layout.chevron.unwrap();
        let fl = FlyoutLayout::compute(&glass_theme, icons.len());
        let fx = 15 + chevron.x + chevron.width / 2 - fl.width / 2;
        let m = reprisekit::flyout::margin(&glass_theme);
        c.blit_over(
            fx - m.left,
            890 - fl.height - m.top,
            &trayflyout::draw(&glass_theme, &text, &fl, &icons, false),
        );
        let classic = cued(Theme::default());
        let fl = FlyoutLayout::compute(&classic, icons.len());
        c.blit_canvas(
            15,
            890 - fl.height,
            &trayflyout::draw(&classic, &text, &fl, &icons, false),
        );
    }
    c
}

/// The Glass task row in each `[taskbar] buttons` mode (combine, when_full, never), on the 40 px and small bars, over the striped backdrop, one 930 px bar each, with groups, lit, held and active buttons, the pager and the orb; the orb's buffer (`start_orb`) goes over each bar.
pub fn render_glass_task_rows() -> Canvas {
    use reprisekit::taskbar::{
        PagerButton, PagerMark, TaskEntry, TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let full = cued(interfaces::theme_for(Interface::Glass));
    let small = reprisekit::glass::small_bar_style(&Default::default());
    type Row<'a> = (
        &'a [u8],
        TaskRow,
        &'a [&'a str],
        bool,
        usize,
        Option<usize>,
        bool,
        bool,
        Option<usize>,
        Option<PagerMark>,
    );
    let rows: [Row; 14] = [
        (
            &[0, 1, 2, 3],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            3,
            None,
            false,
            false,
            None,
            None,
        ),
        (
            &[1, 3, 1],
            TaskRow::Shared,
            &["Notes", "Terminal", "Explorer"],
            false,
            2,
            None,
            true,
            false,
            None,
            None,
        ),
        (
            &[1, 1, 1, 1],
            TaskRow::AtLeast(62),
            &["Notes", "Terminal", "Terminal", "Explorer"],
            false,
            3,
            None,
            false,
            true,
            None,
            None,
        ),
        (
            &[0, 1, 3],
            TaskRow::Icons,
            &["", "", ""],
            true,
            2,
            None,
            false,
            true,
            None,
            None,
        ),
        (
            &[1, 3, 1, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            true,
            3,
            Some(2),
            true,
            false,
            None,
            None,
        ),
        (
            &[1, 3, 1, 3],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            0,
            Some(1),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[1, 1, 1],
            TaskRow::AtLeast(62),
            &["Notes", "Terminal", "Explorer"],
            false,
            2,
            Some(2),
            false,
            false,
            Some(1),
            None,
        ),
        (
            &[0, 1, 1, 1],
            TaskRow::AtLeast(62),
            &["", "Notes", "Terminal", "Explorer"],
            false,
            1,
            Some(2),
            false,
            false,
            None,
            None,
        ),
        (
            &[0, 1, 0, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            false,
            3,
            Some(0),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[0, 1, 0, 1],
            TaskRow::Icons,
            &["", "", "", ""],
            true,
            3,
            Some(0),
            false,
            false,
            Some(2),
            None,
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            false,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Light),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            false,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Tile),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            true,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Light),
        ),
        (
            &[1, 1],
            TaskRow::Icons,
            &["", ""],
            true,
            0,
            None,
            false,
            false,
            None,
            Some(PagerMark::Tile),
        ),
    ];
    let mut c = blurred_stripes(950, 780);
    let mut y = 0;
    for (stacks, row, titles, is_small, active, hot, lit, held, pressed, mark) in rows {
        let mut theme = full.clone();
        if is_small {
            theme.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
            theme.look = look::Look::Styled(small.clone());
        }
        let style = theme.look.style().unwrap();
        let start_text = TextRenderer::load(&style.start_text.font());
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            pager: if mark.is_some() { 4 } else { 0 },
            show_desktop: is_small,
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let clock_w = text.measure(clock, false, theme.fonts.size_px);
        let mut layout =
            TaskbarLayout::compute(&theme.metrics, &style.taskbar, 930, true, counts, clock_w);
        layout.lay_tasks(&theme.metrics, &style.taskbar, stacks, row);
        let tasks = titles
            .iter()
            .enumerate()
            .map(|(i, t)| TaskEntry {
                title: (*t).into(),
                active: i == active,
                attention: reprisekit::taskbar::Attention::None,
                icon: Some(if i % 2 == 0 {
                    indicator::draw_record(16).into()
                } else {
                    indicator::draw_microphone(16).into()
                }),
                glow: None,
            })
            .collect();
        let state = TaskbarState {
            start_pressed: held,
            start_focus: false,
            focus: None,
            start_hot: lit,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: match mark {
                Some(_) => (0..4)
                    .map(|i| PagerButton {
                        current: i == 1,
                        attention: false,
                    })
                    .collect(),
                None => vec![],
            },
            pager_hot: mark.map(|_| 2),
            pager_mark: mark.unwrap_or_default(),
            task_hot: hot,
            task_pressed: pressed,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            tasks,
            tray_icons: vec![],
            meter: None,
            clock: clock.into(),
            show_desktop_hot: lit,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(930, layout.height as u32);
        look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
        y -= layout.start_above.map_or(0, |r| r.y);
        c.blit_over(10, y + 5, &bar);
        if let Some(r) = layout.start_orb(held) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, &theme, &layout, &state);
            c.blit_over(10 + r.x, y + 5 + r.y, &orb);
        }
        y += layout.height + 10;
    }
    c
}

/// Glass's lit task buttons over the striped backdrop with the spot under the pointer at several offsets, one 930 px bar each; each icon is a square in its own colour that the button glows in. The orb's buffer goes over each bar.
pub fn render_glass_spots() -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let full = cued(interfaces::theme_for(Interface::Glass));
    let small = reprisekit::glass::small_bar_style(&Default::default());
    let colours = [
        Color::rgb(0xF0, 0xC8, 0x40),
        Color::rgb(0x30, 0x70, 0xD0),
        Color::rgb(113, 164, 168),
        Color::rgb(0x80, 0x80, 0x80),
    ];
    let icons = [false, true].map(|small| {
        colours.map(|c| {
            let px = if small { 16 } else { 32 };
            let mut icon = Canvas::new(px, px);
            icon.fill_rect(icon.bounds(), c);
            icon
        })
    });
    let (one, labelled) = (TaskRow::Icons, TaskRow::AtLeast(62));
    type Row = (
        &'static [u8],
        TaskRow,
        bool,
        Option<usize>,
        Option<usize>,
        Option<usize>,
        i32,
    );
    let rows: [Row; 8] = [
        (&[1, 1, 3, 1], one, false, None, Some(0), None, 10),
        (&[1, 1, 3, 1], one, false, None, Some(0), None, 50),
        (&[1, 1, 3, 1], one, false, Some(1), Some(1), None, 10),
        (&[1, 1, 3, 1], one, false, None, Some(2), None, 56),
        (&[1, 1, 1, 1], labelled, false, None, Some(1), None, 20),
        (&[1, 1, 1, 1], labelled, false, None, Some(1), None, 140),
        (&[1, 1, 3, 1], one, true, None, Some(1), None, 10),
        (&[1, 1, 3, 1], one, true, None, None, Some(1), 34),
    ];
    let mut c = blurred_stripes(950, 440);
    let mut y = 0;
    for (stacks, row, is_small, active, hot, held, x) in rows {
        let mut theme = full.clone();
        if is_small {
            theme.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
            theme.look = look::Look::Styled(small.clone());
        }
        let style = theme.look.style().unwrap();
        let glow = style.task_glow.expect("Glass buttons glow").colour;
        let start_text = TextRenderer::load(&style.start_text.font());
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let clock_w = text.measure(clock, false, theme.fonts.size_px);
        let mut layout =
            TaskbarLayout::compute(&theme.metrics, &style.taskbar, 930, true, counts, clock_w);
        layout.lay_tasks(&theme.metrics, &style.taskbar, stacks, row);
        let titles = ["Notes", "Terminal", "Explorer", "Paint"];
        let tasks = (0..stacks.len())
            .map(|i| {
                let icon = &icons[usize::from(is_small)][i % colours.len()];
                TaskEntry {
                    title: if row == labelled {
                        titles[i].into()
                    } else {
                        String::new()
                    },
                    active: Some(i) == active,
                    attention: reprisekit::taskbar::Attention::None,
                    icon: Some(icon.clone().into()),
                    glow: glow(icon),
                }
            })
            .collect();
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
            task_hot: hot,
            task_pressed: held,
            task_hot_x: Some(x),
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            tasks,
            tray_icons: vec![],
            meter: None,
            clock: clock.into(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
        };
        let mut bar = Canvas::new(930, layout.height as u32);
        look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
        y -= layout.start_above.map_or(0, |r| r.y);
        c.blit_over(10, y + 5, &bar);
        if let Some(r) = layout.start_orb(false) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, &theme, &layout, &state);
            c.blit_over(10 + r.x, y + 5 + r.y, &orb);
        }
        y += layout.height + 10;
    }
    c
}

/// Glass's task preview popups over the striped backdrop: one window, then a group of three with the middle one hovered. White boxes stand in for the live pictures.
pub fn render_glass_preview() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(interfaces::theme_for(Interface::Glass));
    let mut c = blurred_stripes(1000, 440);
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(Rect::new(2, 1, 12, 14), Color::rgb(80, 193, 255));
    let mut window = Canvas::new(184, 108);
    window.fill_rect(window.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
    window.fill_rect(Rect::new(0, 0, 184, 8), Color::rgb(0xDD, 0xE6, 0xF2));
    let grid = interfaces::task_preview_grid(&theme);
    let mut popup = |n: usize, button: Rect, bounds: Rect, hot: Option<usize>| {
        let l = taskpreview::TaskPreviewLayout::compute(
            grid,
            n,
            button,
            bounds,
            reprisekit::taskbar::Edge::Bottom,
        );
        let a = l.canvas();
        let mut panel = Canvas::new(a.width as u32, a.height as u32);
        let titles = [
            "Untitled - Notepad",
            "notes.txt - Notepad",
            "A title too long for its cell",
        ];
        let cells: Vec<taskpreview::Cell> = (0..n)
            .map(|i| taskpreview::Cell {
                title: titles[i % titles.len()],
                icon: Some(&icon),
                active: i == 0,
                picture: Some(&window),
            })
            .collect();
        interfaces::draw_task_preview(&mut panel, &theme, &text, &l, &cells, hot);
        c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &panel);
    };
    popup(
        1,
        Rect::new(100, 200, 60, 40),
        Rect::new(0, 0, 400, 200),
        None,
    );
    popup(
        3,
        Rect::new(600, 420, 60, 40),
        Rect::new(260, 220, 740, 200),
        Some(1),
    );
    c
}

/// Glass's Alt+Tab thumbnail panels over the striped backdrop, then the icon style. White boxes stand in for the pictures; an entry with no picture shows its icon.
pub fn render_glass_switcher() -> Canvas {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = cued(interfaces::theme_for(Interface::Glass));
    let grid = interfaces::switcher_grid(&theme);
    let mut c = blurred_stripes(1100, 900);
    let mut panel =
        |n: usize, area: Rect, selected: usize, hot: Option<usize>, grid: switcher::Grid| {
            let l = switcher::SwitcherLayout::thumbnails(n, selected, area, grid);
            let pictured = n.min(5);
            let pictures: Vec<Rect> = l.wells[..pictured]
                .iter()
                .map(|&w| taskpreview::fit(400, 300, w, 1))
                .collect();
            let stills: Vec<Canvas> = pictures
                .iter()
                .map(|r| {
                    let mut s = Canvas::new(r.width as u32, r.height as u32);
                    s.fill_rect(s.bounds(), Color::rgb(0xFF, 0xFF, 0xFF));
                    s.fill_rect(Rect::new(0, 0, r.width, 6), Color::rgb(0xDD, 0xE6, 0xF2));
                    s
                })
                .collect();
            let mut cells: Vec<switcher::ThumbCell> = pictures
                .iter()
                .zip(&stills)
                .map(|(&r, s)| switcher::ThumbCell {
                    icon: None,
                    picture: Some(r),
                    still: Some(s),
                })
                .collect();
            cells.resize(
                n,
                switcher::ThumbCell {
                    icon: None,
                    picture: None,
                    still: None,
                },
            );
            let a = l.canvas();
            let mut p = Canvas::new(a.width as u32, a.height as u32);
            interfaces::draw_switcher(
                &mut p,
                &theme,
                &text,
                &l,
                "Untitled - Notepad",
                &cells,
                selected,
                hot,
            );
            interfaces::draw_switcher_badges(&mut p, &theme, &l, &cells);
            c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &p);
        };
    panel(6, Rect::new(0, 0, 1100, 260), 1, Some(3), grid);
    panel(1, Rect::new(0, 240, 500, 220), 0, None, grid);
    panel(2, Rect::new(450, 240, 650, 220), 0, None, grid);
    panel(
        3,
        Rect::new(0, 660, 1100, 240),
        2,
        None,
        grid.sized(switcher::Size::Large),
    );
    let icons = interfaces::switcher_icon_grid(&theme).expect("Glass has an icon grid");
    let icon = |rgb: (u8, u8, u8)| {
        let mut i = Canvas::new(32, 32);
        i.fill_rect(Rect::new(4, 4, 24, 24), Color::rgb(rgb.0, rgb.1, rgb.2));
        i
    };
    let glyphs = [
        icon((40, 120, 220)),
        icon((220, 140, 40)),
        icon((60, 160, 80)),
    ];
    let mut icon_panel = |n: usize, area: Rect, selected: usize, hot: Option<usize>| {
        let l = switcher::SwitcherLayout::thumbnails(n, selected, area, icons);
        let cells: Vec<switcher::ThumbCell> = (0..n)
            .map(|k| switcher::ThumbCell {
                icon: (k % 4 != 3).then(|| &glyphs[k % 4 % 3]),
                picture: None,
                still: None,
            })
            .collect();
        let a = l.canvas();
        let mut p = Canvas::new(a.width as u32, a.height as u32);
        interfaces::draw_switcher(
            &mut p,
            &theme,
            &text,
            &l,
            "Untitled - Notepad",
            &cells,
            selected,
            hot,
        );
        c.blit_over(l.rect.x + a.x, l.rect.y + a.y, &p);
    };
    icon_panel(9, Rect::new(0, 460, 650, 200), 2, Some(4));
    icon_panel(3, Rect::new(600, 460, 500, 200), 1, None);
    c
}
