use super::*;

/// A task's progress lands inside its own button, under the label, in Blue and Glass.
#[test]
fn a_tasks_progress_draws_inside_its_button() {
    use crate::taskbar::{TaskProgress, TaskRow};
    for iface in [Interface::Blue, Interface::Glass] {
        let theme = theme_for(iface);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        layout.lay_tasks(&theme.metrics, &s.taskbar, &[0, 1], TaskRow::Shared);
        let h = layout.height as u32;
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |st: &TaskbarState| {
            let mut c = Canvas::new(800, h);
            draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
            c
        };
        let mut st = state_with_tasks(&["One", "Two"]);
        let plain = draw(&st);
        st.task_progress = vec![
            None,
            Some(TaskProgress {
                fraction: 0.5,
                paused: false,
            }),
        ];
        let shown = draw(&st);
        let changed: Vec<(i32, i32)> = (0..h as i32)
            .flat_map(|y| (0..800).map(move |x| (x, y)))
            .filter(|&(x, y)| plain.get(x, y) != shown.get(x, y))
            .collect();
        let b = layout.tasks[1];
        assert!(!changed.is_empty(), "{iface:?} draws the progress");
        assert!(
            changed.iter().all(|&(x, y)| b.contains(x, y)),
            "{iface:?} keeps it inside the second button"
        );
        if iface == Interface::Glass {
            assert!(
                changed.iter().all(|&(x, _)| x < b.x + b.width * 3 / 4),
                "Glass fills only the done half"
            );
        }
    }
}

#[test]
fn a_pinned_app_without_windows_draws_no_button_until_hot() {
    use crate::taskbar::TaskRow;
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tasks: 2,
        ..TaskbarCounts::default()
    };
    let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
    layout.lay_tasks(&theme.metrics, &s.taskbar, &[0, 1], TaskRow::Shared);
    let h = layout.height as u32;
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let draw = |st: &TaskbarState| {
        let mut c = Canvas::new(800, h);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
        c
    };
    let part_px = |state, r: Rect, x, y| {
        let mut w = Canvas::new(800, h);
        s.draw(
            &mut w,
            Rect::new(0, 0, 800, h as i32),
            Part::Taskbar,
            State::Normal,
            1,
        );
        s.draw(&mut w, r, Part::TaskButton, state, 1);
        w.get(x, y)
    };
    let p = layout.tasks[0];
    let mut st = state_with_tasks(&["", "Two"]);
    let (x, y) = (p.x + 1, p.y + 2);
    let mut bar = Canvas::new(800, h);
    s.draw(
        &mut bar,
        Rect::new(0, 0, 800, h as i32),
        Part::Taskbar,
        State::Normal,
        1,
    );
    assert_eq!(draw(&st).get(x, y), bar.get(x, y), "the bar shows through");
    st.task_hot = Some(0);
    let hot = part_px(State::Hot, p, x, y);
    assert_ne!(hot, bar.get(x, y), "a Hot button shows");
    assert_eq!(draw(&st).get(x, y), hot);
    st.task_hot = None;
    st.task_pressed = Some(0);
    let held = part_px(State::Pressed, p, x, y);
    assert_ne!(held, hot, "a Pressed button differs from the Hot one");
    assert_eq!(draw(&st).get(x, y), held, "a Pressed button shows held");
}

#[test]
fn blue_bar_paints_start_tasks_and_the_tray_area() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts {
            quick: 0,
            pager: 0,
            tasks: 2,
            tray: 1,
            show_desktop: false,
            chevron: false,
            ..TaskbarCounts::default()
        },
        40,
    );
    let mut st = state_with_tasks(&["One", "Two"]);
    st.tasks[0].active = true;
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(800, 30);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    let part_px = |part, state, r: Rect, x, y| {
        let mut w = Canvas::new(800, 30);
        s.draw(&mut w, r, part, state, 1);
        w.get(x, y)
    };
    let t0 = layout.tasks[0];
    let t1 = layout.tasks[1];
    assert_eq!(
        c.get(t0.x + 2, t0.y + 2),
        part_px(Part::TaskButton, State::Checked, t0, t0.x + 2, t0.y + 2)
    );
    assert_eq!(
        c.get(t1.x + 2, t1.y + 2),
        part_px(Part::TaskButton, State::Normal, t1, t1.x + 2, t1.y + 2)
    );
    let n = layout.notification;
    assert_eq!(
        c.get(n.x + 1, 1),
        part_px(Part::TrayArea, State::Normal, n, n.x + 1, 1)
    );
    assert_eq!(
        c.get(3, 15),
        part_px(Part::StartButton, State::Normal, layout.start, 3, 15)
    );
}

#[test]
fn the_start_label_draws_at_its_own_font_size() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let start = TextRenderer::load(&s.start_text.font());
    if !start.has_face("Franklin Gothic Medium") && !start.has_face("Trebuchet MS") {
        return;
    }
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts::default(),
        40,
    );
    let text = TextRenderer::load(&FontConfig::default());
    let mut c = Canvas::new(800, 30);
    draw_taskbar(
        &mut c,
        s,
        &theme,
        &text,
        &start,
        &layout,
        &state_with_tasks(&[]),
    );

    let scale = theme.scale.max(1);
    let st = layout.start;
    let spec = &s.start_text;
    let label = Rect::new(st.x + 28 * scale, st.y, st.right() - 28 * scale, st.height);
    let drawn = ink_span(&c, label, spec.color.argb());

    let mut alone = Canvas::new(100, 30);
    start.draw(
        &mut alone,
        Rect::new(0, 0, 100, 30),
        "start",
        spec.bold,
        spec.color,
        Align::Left,
        spec.size_in(&start),
    );
    let want = ink_span(&alone, Rect::new(0, 0, 100, 30), spec.color.argb());
    assert_eq!(
        drawn, want,
        "the label's ink spans {drawn} px, not the {want} px its own font draws it at"
    );
}

fn ink_span(c: &Canvas, r: Rect, ink: u32) -> i32 {
    let xs = (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| c.get(x, y) == ink)
        .map(|(x, _)| x);
    let (lo, hi) = xs.fold((i32::MAX, i32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
    if lo > hi { 0 } else { hi - lo + 1 }
}

#[test]
fn a_task_buttons_icon_and_title_sit_10_and_30_px_in_a_pixel_further_when_pushed() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts {
            quick: 0,
            pager: 0,
            tasks: 2,
            tray: 0,
            show_desktop: false,
            chevron: false,
            ..TaskbarCounts::default()
        },
        40,
    );
    let mut icon = Canvas::new(16, 16);
    icon.fill_rect(
        Rect::new(0, 0, 16, 16),
        crate::theme::Color::rgb(0xFF, 0, 0xFF),
    );
    let magenta = icon.get(0, 0);
    let mut st = state_with_tasks(&["Notes", "Notes"]);
    st.tasks[1].active = true;
    for t in &mut st.tasks {
        t.icon = Some(icon.clone().into());
    }
    let mut c = Canvas::new(800, 30);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);

    let white = s.bar_text.argb();
    let mut alone = Canvas::new(60, 25);
    text.draw(
        &mut alone,
        Rect::new(0, 0, 60, 25),
        "Notes",
        false,
        s.bar_text,
        Align::Left,
        theme.fonts.size_px,
    );
    let lead = |c: &Canvas, r: Rect| {
        (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) == white))
    };
    let bearing = lead(&alone, Rect::new(0, 0, 60, 25));

    for (i, o) in [(0, 0), (1, 1)] {
        let r = layout.tasks[i];
        assert_eq!(
            c.get(r.x + 10 + o, r.y + 4 + o),
            magenta,
            "task {i} icon top-left"
        );
        assert_ne!(
            c.get(r.x + 9 + o, r.y + 4 + o),
            magenta,
            "task {i} left of its icon"
        );
        let title = Rect::new(r.x + 27, r.y, r.width - 27, r.height);
        assert_eq!(
            lead(&c, title),
            bearing.map(|b| r.x + 30 + o + b),
            "task {i} title's first ink"
        );
    }
}

#[test]
fn the_start_flag_sits_10_px_in_and_5_down_and_the_label_35_px_in() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts::default(),
        40,
    );
    assert_eq!(s.taskbar.start_icon, 18);
    let mut flag = Canvas::new(18, 18);
    flag.fill_rect(
        Rect::new(0, 0, 18, 18),
        crate::theme::Color::rgb(0xFF, 0, 0xFF),
    );
    let magenta = flag.get(0, 0);
    let mut st = state_with_tasks(&[]);
    st.start_icon = Some(flag.into());
    let mut c = Canvas::new(800, 30);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    let b = layout.start;
    assert_eq!(c.get(b.x + 10, b.y + 5), magenta, "the flag's top-left");
    assert_eq!(
        c.get(b.x + 27, b.y + 22),
        magenta,
        "the flag's bottom-right"
    );
    assert_ne!(c.get(b.x + 9, b.y + 5), magenta);
    assert_ne!(c.get(b.x + 10, b.y + 4), magenta);

    let spec = &s.start_text;
    let white = spec.color.argb();
    let lead = |c: &Canvas, r: Rect| {
        (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) == white))
    };
    let mut alone = Canvas::new(100, 30);
    start.draw(
        &mut alone,
        Rect::new(0, 0, 100, 30),
        "start",
        spec.bold,
        spec.color,
        Align::Left,
        spec.size_in(&start),
    );
    let bearing = lead(&alone, Rect::new(0, 0, 100, 30));
    let label = Rect::new(b.x + 28, b.y, b.width - 28, b.height);
    assert_eq!(lead(&c, label), bearing.map(|x| b.x + 35 + x));
}

#[test]
fn the_start_label_is_inked_heavier_over_a_soft_shadow_from_1_px_up_left_to_2_down_right() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let start = TextRenderer::load(&s.start_text.font());
    if !start.has_face("Franklin Gothic Medium") && !start.has_face("Trebuchet MS") {
        return;
    }
    let text = TextRenderer::load(&FontConfig::default());
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts::default(),
        40,
    );
    let mut c = Canvas::new(800, 30);
    draw_taskbar(
        &mut c,
        s,
        &theme,
        &text,
        &start,
        &layout,
        &state_with_tasks(&[]),
    );
    let b = layout.start;
    let mut bare = Canvas::new(800, 30);
    s.draw(
        &mut bare,
        Rect::new(0, 0, 800, 30),
        Part::Taskbar,
        State::Normal,
        1,
    );
    s.draw(&mut bare, b, Part::StartButton, State::Normal, 1);

    let spec = &s.start_text;
    let mut cov = Canvas::new(800, 30);
    let label = Rect::new(b.x + 35, b.y, b.width - 35, b.height);
    let white = crate::theme::Color::rgb(0xFF, 0xFF, 0xFF);
    let size = spec.size_in(&start);
    start.draw(
        &mut cov,
        label,
        "start",
        spec.bold,
        white,
        Align::Left,
        size,
    );
    let at = |x: i32, y: i32| (cov.get(x, y) >> 16) & 0xFF;
    let inked: Vec<(i32, i32)> = (0..30)
        .flat_map(|y| (0..800).map(move |x| (x, y)))
        .filter(|&(x, y)| at(x, y) != 0)
        .collect();
    let x0 = inked.iter().map(|p| p.0).min().unwrap();
    let x1 = inked.iter().map(|p| p.0).max().unwrap();
    let y0 = inked.iter().map(|p| p.1).min().unwrap();
    let y1 = inked.iter().map(|p| p.1).max().unwrap();

    let plain = {
        let mut p = bare.clone();
        start.draw(&mut p, label, "start", spec.bold, white, Align::Left, size);
        p
    };
    let luma = |px: u32| (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF);
    let &(hx, hy) = inked
        .iter()
        .find(|&&(x, y)| (96..160).contains(&at(x, y)))
        .expect("a half-covered pixel");
    assert!(
        luma(c.get(hx, hy)) > luma(plain.get(hx, hy)),
        "({hx}, {hy}) at coverage {} inks heavier than a plain draw",
        at(hx, hy)
    );

    let differs = |x: i32, y: i32| c.get(x, y) != bare.get(x, y);
    let within = |x: i32, y: i32| (x0 - 1..=x1 + 2).contains(&x) && (y0 - 1..=y1 + 2).contains(&y);
    for y in 0..30 {
        for x in b.x + 28..b.right() {
            if !within(x, y) {
                assert!(!differs(x, y), "({x}, {y}) is the bare pill");
            }
        }
    }
    assert!(
        (y0..=y1).any(|y| differs(x1 + 2, y)),
        "2 px right of the ink"
    );
    assert!((x0..=x1).any(|x| differs(x, y1 + 2)), "2 px below the ink");
    assert!(
        (y0..=y1).any(|y| differs(x0 - 1, y)),
        "1 px left of the ink"
    );
    assert!((x0..=x1).any(|x| differs(x, y0 - 1)), "1 px above the ink");
    assert_eq!(
        spec.shadow.map(|sh| sh.color),
        Some(crate::blue::Variant::Blue.scheme().inks.start_shadow),
        "a dark olive shadow"
    );
}
