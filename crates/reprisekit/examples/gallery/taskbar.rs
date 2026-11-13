use super::{blurred_stripes, cued};
use reprisekit::classic::{indicator, snapframe};
use reprisekit::interface::Interface;
use reprisekit::interfaces;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, FontConfig, Theme};

pub fn render_task_progress(scale: i32) -> Canvas {
    use reprisekit::taskbar::{
        TaskEntry, TaskProgress, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let glass_dark = cued(interfaces::theme_with(
        Interface::Glass,
        reprisekit::blue::Variant::default(),
        &reprisekit::glass::Settings {
            mode: reprisekit::glass::Mode::Dark,
            ..reprisekit::glass::Settings::default()
        },
    ));
    let themes = [
        cued(interfaces::theme_for(Interface::Classic)),
        cued(interfaces::blue_theme(reprisekit::blue::Variant::Blue)),
        cued(interfaces::blue_theme(reprisekit::blue::Variant::Olive)),
        cued(interfaces::blue_theme(reprisekit::blue::Variant::Silver)),
        cued(interfaces::theme_for(Interface::Glass)),
        glass_dark,
    ];
    let (w, gap) = (560, 10);
    let rows: Vec<Canvas> = themes
        .iter()
        .map(|theme| {
            let start_text = match theme.look.style() {
                Some(s) => TextRenderer::load(&s.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let counts = TaskbarCounts {
                tasks: 3,
                ..TaskbarCounts::default()
            };
            let layout = TaskbarLayout::compute(
                &theme.metrics,
                &theme.taskbar_shape(),
                w,
                true,
                counts,
                text.measure("3:45 PM", false, theme.fonts.size_px),
            );
            let task = |title: &str| TaskEntry {
                title: title.into(),
                active: false,
                attention: reprisekit::taskbar::Attention::None,
                icon: None,
                glow: None,
            };
            let state = TaskbarState {
                tasks: vec![task("Copying..."), task("Moving..."), task("Notepad")],
                task_progress: vec![
                    Some(TaskProgress {
                        fraction: 0.6,
                        paused: false,
                    }),
                    Some(TaskProgress {
                        fraction: 0.35,
                        paused: true,
                    }),
                    None,
                ],
                task_badge: vec![None, Some(7), Some(128)],
                clock: "3:45 PM".into(),
                ..TaskbarState::default()
            };
            let scaled = theme.scaled(scale);
            let mut bar = Canvas::new((w * scale) as u32, (layout.height * scale) as u32);
            look::draw_taskbar(
                &mut bar,
                &scaled,
                &text,
                &start_text,
                &layout.scaled(scale),
                &state,
            );
            bar
        })
        .collect();
    let height = rows
        .iter()
        .map(|r| r.height as i32 + gap * scale)
        .sum::<i32>()
        + gap * scale;
    let mut c = Canvas::new(((w + 2 * gap) * scale) as u32, height as u32);
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    let mut y = gap * scale;
    for r in &rows {
        c.blit_over(gap * scale, y, r);
        y += r.height as i32 + gap * scale;
    }
    c
}

/// Task buttons wanting attention on each interface's bar: at rest, lit (a flash's lit half, or
/// steady), a flash's dark half, and active; Glass also as icon buttons.
pub fn render_task_attention(scale: i32) -> Canvas {
    use reprisekit::taskbar::{
        Attention, TaskEntry, TaskRow, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let glass_with = |s: reprisekit::glass::Settings| {
        cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &s,
        ))
    };
    let mut themes: Vec<_> = vec![cued(interfaces::theme_for(Interface::Classic))];
    themes.extend(
        reprisekit::blue::Variant::ALL
            .into_iter()
            .map(|v| cued(interfaces::blue_theme(v))),
    );
    themes.extend([
        glass_with(Default::default()),
        glass_with(reprisekit::glass::Settings {
            mode: reprisekit::glass::Mode::Dark,
            ..Default::default()
        }),
        glass_with(reprisekit::glass::Settings {
            material: reprisekit::glass::Material::Smoked,
            ..Default::default()
        }),
        glass_with(reprisekit::glass::Settings {
            material: reprisekit::glass::Material::Molten,
            ..Default::default()
        }),
    ]);
    let (w, gap) = (640, 10);
    let mut rows: Vec<Canvas> = Vec::new();
    for theme in &themes {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let glass = theme.look.style().is_some_and(|s| s.glass.is_some());
        for icons in [false, true].into_iter().filter(|&i| !i || glass) {
            let counts = TaskbarCounts {
                tasks: 4,
                ..TaskbarCounts::default()
            };
            let shape = theme.taskbar_shape();
            let mut layout = TaskbarLayout::compute(
                &theme.metrics,
                &shape,
                w,
                true,
                counts,
                text.measure("3:45 PM", false, theme.fonts.size_px),
            );
            if icons {
                layout.lay_tasks(&theme.metrics, &shape, &[1; 4], TaskRow::Icons);
            }
            let task = |title: &str, active: bool, attention| TaskEntry {
                title: title.into(),
                active,
                attention,
                icon: Some(indicator::draw_record(theme.icon_size()).into()),
                glow: None,
            };
            let state = TaskbarState {
                tasks: vec![
                    task("Notepad", false, Attention::None),
                    task("Chat", false, Attention::Steady),
                    task("Mail", false, Attention::FlashOff),
                    task("Paint", true, Attention::None),
                ],
                clock: "3:45 PM".into(),
                ..TaskbarState::default()
            };
            let scaled = theme.scaled(scale);
            let mut bar = Canvas::new((w * scale) as u32, (layout.height * scale) as u32);
            look::draw_taskbar(
                &mut bar,
                &scaled,
                &text,
                &start_text,
                &layout.scaled(scale),
                &state,
            );
            rows.push(bar);
        }
    }
    let height = rows
        .iter()
        .map(|r| r.height as i32 + gap * scale)
        .sum::<i32>()
        + gap * scale;
    let mut c = Canvas::new(((w + 2 * gap) * scale) as u32, height as u32);
    c.fill_rect(c.bounds(), Color::rgb(58, 110, 165));
    let mut y = gap * scale;
    for r in &rows {
        c.blit_over(gap * scale, y, r);
        y += r.height as i32 + gap * scale;
    }
    c
}

pub fn render_taskbar_top() -> Canvas {
    use reprisekit::taskbar::{
        Edge, PagerButton, TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = cued(interfaces::theme_for(Interface::Glass));
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        cued(interfaces::theme_for(Interface::Classic)),
        cued(interfaces::theme_for(Interface::Blue)),
        cued(interfaces::theme_for(Interface::Glass)),
        small,
    ];
    let mut c = blurred_stripes(660, 200);
    let mut y = 10;
    for (i, theme) in themes.iter().enumerate() {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            meter: Some(reprisekit::netmeter::size(
                &text,
                reprisekit::netmeter::font_px(theme.fonts.size_px, 1),
            )),
            ..TaskbarCounts::default()
        };
        let clock = "3:45 PM";
        let shape = theme.taskbar_shape();
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &shape,
            640,
            true,
            counts,
            text.measure(clock, false, theme.fonts.size_px),
        )
        .on_edge(Edge::Top);
        let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
            title: title.into(),
            active,
            attention: reprisekit::taskbar::Attention::None,
            icon: Some(icon.into()),
            glow: None,
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: i == 2,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
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
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            task_badge: Vec::new(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                task("Untitled - Notepad", true, indicator::draw_record(16)),
                task("Command Prompt", false, indicator::draw_microphone(16)),
            ],
            tray_icons: vec![
                Some(indicator::draw_record(16).into()),
                Some(indicator::draw_recording(16).into()),
            ],
            meter: Some(["\u{2193} 2.4 MB/s".into(), "\u{2191} 120 KB/s".into()]),
            clock: clock.into(),
        };
        let mut bar = Canvas::new(640, layout.height as u32);
        look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
        c.blit_over(10, y, &bar);
        if let Some(r) = layout.start_orb(false) {
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            look::draw_start_above(&mut orb, theme, &layout, &state);
            c.blit_over(10 + r.x, y + r.y, &orb);
        }
        y += layout.start_above.map_or(layout.height, |r| r.bottom()) + 8;
    }
    c
}

/// Each interface's bar along every edge over the stripes: 640 px along the bottom and top, then up the left and right of a 360 px output, each followed by its hidden columns.
pub fn render_taskbar_edges() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PagerButton, TaskEntry, TaskbarCounts, TaskbarLayout,
        TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = cued(interfaces::theme_for(Interface::Glass));
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        cued(interfaces::theme_for(Interface::Classic)),
        cued(interfaces::theme_for(Interface::Blue)),
        cued(interfaces::theme_for(Interface::Glass)),
        small,
    ];
    let (time, date) = ("3:45 PM", "10/04/2026");
    let task = |title: &str, active: bool, icon: Canvas| TaskEntry {
        title: title.into(),
        active,
        attention: reprisekit::taskbar::Attention::None,
        icon: Some(icon.into()),
        glow: None,
    };
    let state = |quick: usize, two_lines: bool, icon_px: i32| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![None; quick],
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
        task_hot: None,
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        task_progress: Vec::new(),
        task_badge: Vec::new(),
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
        tasks: vec![
            task("Untitled - Notepad", true, indicator::draw_record(icon_px)),
            task("Command Prompt", false, indicator::draw_microphone(icon_px)),
        ],
        tray_icons: vec![Some(indicator::draw_record(16).into())],
        meter: None,
        clock: if two_lines {
            format!("{time}\n{date}")
        } else {
            time.into()
        },
    };
    // Draws `l` in `theme` at `at` of `c` with the orb's buffer over it; returns the bar's canvas.
    let draw =
        |c: &mut Canvas, theme: &Theme, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let start_text = match theme.look.style() {
                Some(s) => TextRenderer::load(&s.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, l, st);
            c.blit_over(at.0, at.1, &bar);
            if let Some(r) = l.start_orb(false) {
                let mut orb = Canvas::new(r.width as u32, r.height as u32);
                look::draw_start_above(&mut orb, theme, l, st);
                c.blit_over(at.0 + r.x, at.1 + r.y, &orb);
            }
            bar
        };
    let clock_of = |theme: &Theme| ClockText {
        time: text.measure(time, false, theme.fonts.size_px),
        date: text.measure(date, false, theme.fonts.size_px),
    };
    let mut c = blurred_stripes(740, 860);
    let mut y = 24;
    for theme in &themes {
        let shape = theme.taskbar_shape();
        let counts = TaskbarCounts {
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for edge in [Edge::Bottom, Edge::Top] {
            let span = BarSpan {
                edge,
                length: 640,
                thickness: theme.metrics.taskbar_height,
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                clock_of(theme),
                Bands::default(),
            );
            draw(
                &mut c,
                theme,
                &l,
                &state(0, l.clock_lines.is_some(), shape.task_icon),
                (10, y),
            );
            y += l.height + 22;
        }
    }
    let top = y + 10;
    let mut x = 10;
    for theme in &themes {
        let shape = theme.taskbar_shape();
        let t = shape.side_default(&theme.metrics);
        let counts = TaskbarCounts {
            quick: 3,
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for edge in [Edge::Left, Edge::Right] {
            let span = BarSpan {
                edge,
                length: 360,
                thickness: t,
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                clock_of(theme),
                Bands::default(),
            );
            let bar = draw(
                &mut c,
                theme,
                &l,
                &state(3, l.clock_lines.is_some(), shape.task_icon),
                (x, top),
            );
            c.blit_over(x + t + 4, top, &bar.copy_rect(l.reveal_line()));
            x += t + 16;
        }
        x += 14;
    }
    c
}

/// The » and ^ cells at rest, lit and with their popup open, on unlocked bars in each look: 520 px bars along the bottom, then columns up the left of a 360 px output.
pub fn render_taskbar_cells() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PopupCell, TaskEntry, TaskbarCounts, TaskbarLayout,
        TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = cued(interfaces::theme_for(Interface::Glass));
    interfaces::small_bar(&mut small, &Default::default());
    let themes = [
        cued(interfaces::theme_for(Interface::Classic)),
        cued(interfaces::theme_for(Interface::Blue)),
        cued(interfaces::theme_for(Interface::Glass)),
        small,
    ];
    let cells = [
        PopupCell::default(),
        PopupCell {
            hot: true,
            open: false,
            held: false,
        },
        PopupCell {
            hot: false,
            open: true,
            held: false,
        },
    ];
    let state = |tasks: usize, icon_px: i32, cell: PopupCell| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![None; 2],
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
        show_desktop_hot: false,
        more: cell,
        chevron: cell,
        tasks: (0..tasks)
            .map(|i| TaskEntry {
                title: "Untitled - Notepad".into(),
                active: i == 0,
                attention: reprisekit::taskbar::Attention::None,
                icon: Some(indicator::draw_record(icon_px).into()),
                glow: None,
            })
            .collect(),
        tray_icons: vec![Some(indicator::draw_record(16).into())],
        meter: None,
        clock: "3:45 PM".into(),
    };
    // Draws `l` in `theme` at `at` of `c` with the orb's buffer over it.
    let draw =
        |c: &mut Canvas, theme: &Theme, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let start_text = match theme.look.style() {
                Some(s) => TextRenderer::load(&s.start_text.font()),
                None => TextRenderer::load(&FontConfig::default()),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, l, st);
            c.blit_over(at.0, at.1, &bar);
            if let Some(r) = l.start_orb(false) {
                let mut orb = Canvas::new(r.width as u32, r.height as u32);
                look::draw_start_above(&mut orb, theme, l, st);
                c.blit_over(at.0 + r.x, at.1 + r.y, &orb);
            }
        };
    let lay = |theme: &Theme, span: BarSpan, tasks: usize| {
        let counts = TaskbarCounts {
            quick: 2,
            tasks,
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute_on(
            &theme.metrics,
            &theme.taskbar_shape(),
            span,
            false,
            counts,
            ClockText {
                time: text.measure("3:45 PM", false, theme.fonts.size_px),
                date: 0,
            },
            Bands::default(),
        )
    };
    let mut c = blurred_stripes(960, 1056);
    let mut y = 24;
    for theme in &themes {
        let icon_px = theme.taskbar_shape().task_icon;
        for cell in cells {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 520,
                thickness: theme.metrics.taskbar_height,
            };
            let l = lay(theme, span, 2);
            draw(&mut c, theme, &l, &state(2, icon_px, cell), (10, y));
            y += l.height + 22;
        }
    }
    let top = y + 10;
    let mut x = 10;
    for theme in &themes {
        let icon_px = theme.taskbar_shape().task_icon;
        let t = theme.taskbar_shape().side_default(&theme.metrics);
        for cell in cells {
            let span = BarSpan {
                edge: Edge::Left,
                length: 360,
                thickness: t,
            };
            let l = lay(theme, span, 12);
            draw(&mut c, theme, &l, &state(12, icon_px, cell), (x, top));
            x += t + 10;
        }
        x += 14;
    }
    c
}

/// A Quick Launch band dragged shorter than its five buttons, ending in its » at rest, lit and with its menu open, in each look: along the bottom and the top, along the bottom at scale 2, then down left and right columns.
pub fn render_quick_more() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PopupCell, TaskbarCounts, TaskbarLayout, TaskbarState,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let looks = [Interface::Classic, Interface::Blue, Interface::Glass];
    let cells = [
        PopupCell::default(),
        PopupCell {
            hot: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            ..PopupCell::default()
        },
    ];
    let state = |cell: PopupCell| TaskbarState {
        start_pressed: false,
        start_focus: false,
        focus: None,
        start_hot: false,
        start_icon: None,
        orb_glow: 100,
        quick_icons: vec![Some(indicator::draw_record(16).into()); 5],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        quick_more: cell,
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
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
        tasks: vec![],
        tray_icons: vec![],
        meter: None,
        clock: "3:45 PM".into(),
    };
    let lay = |theme: &Theme, span: BarSpan| {
        let counts = TaskbarCounts {
            quick: 5,
            ..TaskbarCounts::default()
        };
        let first_width = if span.edge.is_side() { 30 } else { 2 * 23 + 14 };
        TaskbarLayout::compute_on(
            &theme.metrics,
            &theme.taskbar_shape(),
            span,
            false,
            counts,
            ClockText {
                time: text.measure("3:45 PM", false, theme.fonts.size_px),
                date: 0,
            },
            Bands {
                tasks_first: false,
                first_width: Some(first_width),
            },
        )
    };
    let draw = |c: &mut Canvas, theme: &Theme, l: &TaskbarLayout, cell, at: (i32, i32)| {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let mut bar = Canvas::new(l.width as u32, l.height as u32);
        look::draw_taskbar(&mut bar, theme, &text, &start_text, l, &state(cell));
        c.blit_over(at.0, at.1, &bar);
    };
    let mut c = blurred_stripes(1060, 1570);
    let mut y = 10;
    for interface in looks {
        let theme = cued(interfaces::theme_for(interface));
        let h = theme.metrics.taskbar_height;
        for edge in [Edge::Bottom, Edge::Top] {
            let mut x = 10;
            for cell in cells {
                let span = BarSpan {
                    edge,
                    length: 340,
                    thickness: h,
                };
                draw(&mut c, &theme, &lay(&theme, span), cell, (x, y));
                x += 350;
            }
            y += h + 8;
        }
        let scaled = theme.scaled(2);
        for cell in cells {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 340,
                thickness: h,
            };
            let l = lay(&theme, span).scaled(2);
            draw(&mut c, &scaled, &l, cell, (10, y));
            y += l.height + 8;
        }
        y += 8;
    }
    for interface in looks {
        let theme = cued(interfaces::theme_for(interface));
        let t = theme.taskbar_shape().side_default(&theme.metrics);
        let mut x = 10;
        for edge in [Edge::Left, Edge::Right] {
            for cell in cells {
                let span = BarSpan {
                    edge,
                    length: 200,
                    thickness: t,
                };
                draw(&mut c, &theme, &lay(&theme, span), cell, (x, y));
                x += t + 6;
            }
            x += 10;
        }
        y += 210;
    }
    c
}

/// The tray arrow on Classic and Blue bars with one shown and two hidden icons: along the bottom in each state, then down a left bar.
pub fn render_tray_arrow() -> Canvas {
    use reprisekit::taskbar::{
        Bands, BarSpan, ClockText, Edge, PopupCell, TaskbarCounts, TaskbarLayout, TaskbarState,
        reveal_width,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let state = |cell: PopupCell, icons: usize| TaskbarState {
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
        show_desktop_hot: false,
        more: Default::default(),
        chevron: cell,
        tasks: vec![],
        tray_icons: vec![Some(indicator::draw_record(16).into()); icons],
        meter: None,
        clock: "3:45 PM".into(),
    };
    let cell = |hot, open, held| PopupCell { hot, open, held };
    let mut c = blurred_stripes(1060, 262);
    let mut x0 = 10;
    for interface in [Interface::Classic, Interface::Blue] {
        let theme = cued(interfaces::theme_for(interface));
        let shape = theme.taskbar_shape();
        let full = reveal_width(&shape, 2);
        let lay = |span: BarSpan, reveal: i32| {
            let counts = TaskbarCounts {
                tray: 1,
                hidden: 2,
                reveal,
                chevron: true,
                ..TaskbarCounts::default()
            };
            TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                counts,
                ClockText {
                    time: text.measure("3:45 PM", false, theme.fonts.size_px),
                    date: 0,
                },
                Bands::default(),
            )
        };
        let draw = |c: &mut Canvas, l: &TaskbarLayout, st: &TaskbarState, at: (i32, i32)| {
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &text, l, st);
            c.blit_over(at.0, at.1, &bar);
        };
        let mut y = 16;
        for (reveal, cell) in [
            (0, cell(false, false, false)),
            (full / 2, cell(false, false, false)),
            (full, cell(false, true, false)),
            (0, cell(true, false, false)),
            (full, cell(true, true, false)),
            (0, cell(true, false, true)),
        ] {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 360,
                thickness: theme.metrics.taskbar_height,
            };
            let l = lay(span, reveal);
            let st = state(cell, l.tray.len());
            draw(&mut c, &l, &st, (x0, y));
            y += l.height + 10;
        }
        let mut x = x0 + 370;
        for (reveal, cell) in [
            (0, cell(false, false, false)),
            (1, cell(false, true, false)),
        ] {
            let span = BarSpan {
                edge: Edge::Left,
                length: 200,
                thickness: 60,
            };
            let l = lay(span, reveal);
            let st = state(cell, l.tray.len());
            draw(&mut c, &l, &st, (x, 16));
            x += 70;
        }
        x0 += 540;
    }
    c
}

/// The Blue tray arrow in every colour scheme, a row each, at `n` device px per design px.
pub fn render_tray_arrow_schemes(n: i32) -> Canvas {
    use reprisekit::taskbar::{
        PopupCell, TaskbarCounts, TaskbarLayout, TaskbarState, reveal_width,
    };
    let text = TextRenderer::load(&FontConfig::default());
    let cells = [
        PopupCell::default(),
        PopupCell {
            hot: true,
            ..PopupCell::default()
        },
        PopupCell {
            hot: true,
            held: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            hot: true,
            ..PopupCell::default()
        },
        PopupCell {
            open: true,
            hot: true,
            held: true,
        },
    ];
    let (w, gap) = (200, 10);
    let cols = cells.len() as i32;
    let mut c = blurred_stripes((cols * (w + gap) + gap) * n, (7 * 40 + gap) * n);
    for (row, v) in reprisekit::blue::Variant::ALL.into_iter().enumerate() {
        let one = cued(interfaces::blue_theme(v));
        let theme = one.scaled(n);
        let shape = theme.taskbar_shape();
        for (col, cell) in cells.into_iter().enumerate() {
            let counts = TaskbarCounts {
                tray: 1,
                hidden: 2,
                reveal: if cell.open {
                    reveal_width(&shape, 2)
                } else {
                    0
                },
                chevron: true,
                ..TaskbarCounts::default()
            };
            let l = TaskbarLayout::compute(
                &one.metrics,
                &shape,
                w,
                true,
                counts,
                text.measure("3:45 PM", false, one.fonts.size_px),
            )
            .scaled(n);
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
                show_desktop_hot: false,
                more: Default::default(),
                chevron: cell,
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16 * n).into()); l.tray.len()],
                meter: None,
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(l.width as u32, l.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &text, &l, &state);
            let x = gap + col as i32 * (w + gap);
            c.blit_over(x * n, (gap + row as i32 * 40) * n, &bar);
        }
    }
    c
}

/// Glass's Start orb at rest, lit and held on a 120 px bar at scale `n`, on Sky and dark glass.
pub fn render_orb_glow(n: i32) -> Canvas {
    use reprisekit::taskbar::{TaskbarCounts, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    // Rows are 80 px apart so a lit glow's rows above the bar land on the stripes.
    let (w, gap, row_h) = (120, 10, 80);
    let quads = four_colour_icon(24 * n);
    let looks = [
        reprisekit::glass::Mode::Light,
        reprisekit::glass::Mode::Dark,
    ];
    let mut c = blurred_stripes((3 * (w + gap) + gap) * n, 4 * row_h * n);
    for (li, mode) in looks.into_iter().enumerate() {
        let one = cued(interfaces::theme_with(
            Interface::Glass,
            reprisekit::blue::Variant::default(),
            &reprisekit::glass::Settings {
                mode,
                ..reprisekit::glass::Settings::default()
            },
        ));
        let theme = one.scaled(n);
        let shape = theme.taskbar_shape();
        for (ii, icon) in [None, Some(quads.clone())].into_iter().enumerate() {
            for (col, (hot, held)) in [(false, false), (true, false), (true, true)]
                .into_iter()
                .enumerate()
            {
                let l = TaskbarLayout::compute(
                    &one.metrics,
                    &shape,
                    w,
                    true,
                    TaskbarCounts::default(),
                    text.measure("3:45 PM", false, one.fonts.size_px),
                )
                .scaled(n);
                let state = TaskbarState {
                    start_pressed: held,
                    start_focus: false,
                    focus: None,
                    start_hot: hot,
                    start_icon: icon.clone().map(Into::into),
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
                    show_desktop_hot: false,
                    more: Default::default(),
                    chevron: Default::default(),
                    tasks: vec![],
                    tray_icons: vec![],
                    meter: None,
                    clock: "3:45 PM".into(),
                };
                let mut bar = Canvas::new(l.width as u32, l.height as u32);
                look::draw_taskbar(&mut bar, &theme, &text, &text, &l, &state);
                let x = (gap + col as i32 * (w + gap)) * n;
                let y = (40 + (2 * li + ii) as i32 * row_h) * n;
                c.blit_over(x, y, &bar);
                if let Some(r) = l.start_orb(held) {
                    let mut orb = Canvas::new(r.width as u32, r.height as u32);
                    look::draw_start_above(&mut orb, &theme, &l, &state);
                    c.blit_over(x + r.x, y + r.y, &orb);
                }
            }
        }
    }
    c
}

/// A `px` square test icon of four rounded panes: red, green, blue, yellow clockwise from the top left.
fn four_colour_icon(px: i32) -> Canvas {
    let mut c = Canvas::new(px as u32, px as u32);
    let inks = [0xFFE0_3020_u32, 0xFF30_B030, 0xFF20_70E0, 0xFFF0_C020];
    let h = px / 2;
    let gap = (px / 12).max(1);
    for y in 0..px {
        for x in 0..px {
            let (right, down) = (x >= h, y >= h);
            let (lx, ly) = (x % h, y % h);
            if lx < gap / 2 + 1 && right || ly < gap / 2 + 1 && down {
                continue;
            }
            if !right && lx >= h - gap / 2 || !down && ly >= h - gap / 2 {
                continue;
            }
            let i = match (right, down) {
                (false, false) => 0,
                (true, false) => 1,
                (true, true) => 3,
                (false, true) => 2,
            };
            c.put_raw(x, y, inks[i]);
        }
    }
    c
}

/// The taskbar with keyboard focus rings (Super+T) in Classic, Blue and Glass.
pub fn render_taskbar_keys() -> Canvas {
    use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarHit, TaskbarLayout, TaskbarState};
    let text = TextRenderer::load(&FontConfig::default());
    let spots = [
        TaskbarHit::Start,
        TaskbarHit::Quick(1),
        TaskbarHit::Task(0),
        TaskbarHit::Chevron,
        TaskbarHit::Tray(0),
        TaskbarHit::Clock,
    ];
    let mut c = blurred_stripes(1080, 580);
    let mut y = 20;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = cued(interfaces::theme_for(i));
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        let counts = TaskbarCounts {
            quick: 2,
            tasks: 2,
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &theme.taskbar_shape(),
            520,
            true,
            counts,
            text.measure("3:45 PM", false, theme.fonts.size_px),
        );
        let icon_px = theme.taskbar_shape().task_icon;
        for (n, spot) in spots.into_iter().enumerate() {
            let state = TaskbarState {
                start_pressed: false,
                start_focus: false,
                focus: Some(spot),
                start_hot: false,
                start_icon: None,
                orb_glow: 100,
                quick_icons: vec![None; 2],
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
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: (0..2)
                    .map(|t| TaskEntry {
                        title: "Untitled - Notepad".into(),
                        active: t == 0,
                        attention: reprisekit::taskbar::Attention::None,
                        icon: Some(indicator::draw_record(icon_px).into()),
                        glow: None,
                    })
                    .collect(),
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                meter: None,
                clock: "3:45 PM".into(),
            };
            let mut bar = Canvas::new(layout.width as u32, layout.height as u32);
            look::draw_taskbar(&mut bar, &theme, &text, &start_text, &layout, &state);
            let x = 10 + (n as i32 % 2) * 540;
            c.blit_over(x, y + (n as i32 / 2) * (layout.height + 12), &bar);
        }
        y += 3 * (layout.height + 12) + 20;
    }
    c
}

/// The line an auto-hidden bar leaves on screen in each look, along the bottom then the top.
pub fn render_taskbar_hidden() -> Canvas {
    use reprisekit::taskbar::{Edge, TaskbarCounts, TaskbarLayout, TaskbarState};
    const BAND: i32 = 16;
    let text = TextRenderer::load(&FontConfig::default());
    let mut small = cued(interfaces::theme_for(Interface::Glass));
    small.metrics.taskbar_height = reprisekit::glass::SMALL_TASKBAR_HEIGHT;
    small.look = look::Look::Styled(reprisekit::glass::small_bar_style(&Default::default()));
    let themes = [
        cued(interfaces::theme_for(Interface::Classic)),
        cued(interfaces::theme_for(Interface::Blue)),
        cued(interfaces::theme_for(Interface::Glass)),
        small,
    ];
    let mut c = blurred_stripes(660, 20 + 8 * (BAND + 6));
    let mut y = 10;
    for theme in &themes {
        let start_text = match theme.look.style() {
            Some(s) => TextRenderer::load(&s.start_text.font()),
            None => TextRenderer::load(&FontConfig::default()),
        };
        for edge in [Edge::Bottom, Edge::Top] {
            let counts = TaskbarCounts {
                tray: 1,
                ..TaskbarCounts::default()
            };
            let clock = "3:45 PM";
            let shape = theme.taskbar_shape();
            let layout = TaskbarLayout::compute(
                &theme.metrics,
                &shape,
                640,
                true,
                counts,
                text.measure(clock, false, theme.fonts.size_px),
            )
            .on_edge(edge);
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
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![],
                tray_icons: vec![Some(indicator::draw_record(16).into())],
                meter: None,
                clock: clock.into(),
            };
            let mut bar = Canvas::new(640, layout.height as u32);
            look::draw_taskbar(&mut bar, theme, &text, &start_text, &layout, &state);
            let line = bar.copy_rect(layout.reveal_line());
            let at = if edge == Edge::Bottom {
                y + BAND - line.height as i32
            } else {
                y
            };
            c.blit_over(10, at, &line);
            y += BAND + 6;
        }
    }
    c
}

/// The 2 px outline an edge drag shows where the bar would land, on each edge of a small output.
pub fn render_taskbar_drag() -> Canvas {
    use reprisekit::taskbar::Edge;
    let (w, h, gap) = (200, 130, 10);
    let mut c = Canvas::new((gap + 4 * (w + gap)) as u32, (h + 2 * gap) as u32);
    c.fill_rect(
        Rect::new(0, 0, c.width as i32, c.height as i32),
        Color::rgb(255, 255, 255),
    );
    for (i, edge) in Edge::ALL.into_iter().enumerate() {
        let (x, y) = (gap + i as i32 * (w + gap), gap);
        c.fill_rect(Rect::new(x, y, w, h), Color::rgb(58, 110, 165));
        let bar = match edge {
            Edge::Bottom => Rect::new(0, h - 28, w, 28),
            Edge::Top => Rect::new(0, 0, w, 28),
            Edge::Left => Rect::new(0, 0, 60, h),
            Edge::Right => Rect::new(w - 60, 0, 60, h),
        };
        c.blit_canvas(
            x + bar.x,
            y + bar.y,
            &snapframe::draw(bar.width, bar.height, snapframe::BAR_BAND, 1),
        );
    }
    c
}
