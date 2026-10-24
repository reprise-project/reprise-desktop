use super::*;

fn clock_rows(c: &Canvas, bare: &Canvas, k: Rect) -> Vec<i32> {
    (k.y..k.bottom())
        .filter(|&y| (k.x..k.right()).any(|x| c.get(x, y) != bare.get(x, y)))
        .collect()
}

#[test]
fn a_taller_bar_centres_the_two_clock_lines() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let clock = "12:34\n27.09.2026";
    let at = |h: i32| {
        let (bare, l) = glass_bar_in(s, h, "", None);
        let (c, _) = glass_bar_in(s, h, clock, None);
        clock_rows(&c, &bare, l.clock)
    };
    let (forty, sixty) = (at(40), at(60));
    assert!(!forty.is_empty(), "the clock inks");
    let moved: Vec<i32> = forty.iter().map(|y| y + 10).collect();
    assert_eq!(sixty, moved, "on a 60 px bar the pair sits 10 px lower");
}

#[test]
fn a_glass_start_is_the_orb_in_any_colour_and_intensity() {
    let other = crate::glass::Settings {
        color: crate::theme::Color::rgb(206, 15, 15),
        intensity: 80,
        ..crate::glass::Settings::default()
    };
    let s = crate::glass::style(&other);
    assert!(s.taskbar.orb, "an orb at intensity 80");
    let (c, _) = glass_bar_in(&s, 40, "", None);
    let (d, _) = glass_bar("", None);
    assert_ne!(
        c.get(26, 25),
        d.get(26, 25),
        "the orb takes the glass colour"
    );
    assert_ne!(c.get(200, 20), d.get(200, 20), "the bar's glass does not");
}

#[test]
fn a_glass_bar_draws_its_bar_orb_and_buttons_from_the_art() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, l) = glass_bar("", Some(Canvas::new(24, 24)));
    let part = |part, state, r: Rect, x, y| {
        let mut w = Canvas::new(1920, 40);
        s.draw(
            &mut w,
            Rect::new(0, 0, 1920, 40),
            Part::Taskbar,
            State::Normal,
            1,
        );
        s.draw(&mut w, r, part, state, 1);
        w.get(x, y)
    };
    let mut alone = Canvas::new(1920, 40);
    s.draw(
        &mut alone,
        Rect::new(0, 0, 1920, 40),
        Part::Taskbar,
        State::Normal,
        1,
    );
    let veiled = over_veil(s, &l);
    assert_eq!(c.get(1000, 20), veiled.get(1000, 20), "the bar's glass");
    assert_ne!(c.get(1000, 20), alone.get(1000, 20), "veiled under the row");
    assert_eq!(
        c.get(26, 19),
        part(Part::StartButton, State::Normal, l.start, 26, 19),
        "the orb, with no label on it"
    );
    for (i, state) in [State::Normal, State::Checked, State::Hot]
        .into_iter()
        .enumerate()
    {
        let r = l.tasks[i];
        let mut want = over_veil(s, &l);
        label_in(&mut want, s, label_frame(r, 1), state, 1);
        assert_eq!(
            c.get(r.x + 60, r.y + 1),
            want.get(r.x + 60, r.y + 1),
            "button {i} is {state:?}"
        );
    }
}

#[test]
fn the_orbs_icon_sits_centred_on_it() {
    let mut icon = Canvas::new(24, 24);
    icon.fill_rect(
        Rect::new(0, 0, 24, 24),
        crate::theme::Color::rgb(0xFF, 0, 0xFF),
    );
    let magenta = icon.get(0, 0);
    let (c, _) = glass_bar("", Some(icon));
    assert_eq!(c.get(15, 8), magenta, "top-left at (15, 8)");
    assert_eq!(c.get(38, 31), magenta, "bottom-right at (38, 31)");
    assert_ne!(c.get(14, 8), magenta);
    assert_ne!(c.get(15, 7), magenta);
}

#[test]
fn a_two_line_clock_sets_the_time_over_the_date() {
    let (bare, l) = glass_bar("", None);
    let (c, _) = glass_bar("12:34\n27.09.2026", None);
    let k = l.clock;
    let ink: Vec<(i32, i32)> = (k.y..k.bottom())
        .flat_map(|y| (k.x..k.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != bare.get(x, y))
        .collect();
    let rows = |lo: i32, hi: i32| -> Vec<(i32, i32)> {
        ink.iter()
            .copied()
            .filter(|&(_, y)| y >= lo && y < hi)
            .collect()
    };
    let (time, date) = (rows(0, 20), rows(20, 40));
    assert!(!time.is_empty() && !date.is_empty(), "both lines ink");
    let bottom = |v: &[(i32, i32)]| v.iter().map(|&(_, y)| y).max().unwrap();
    assert!(
        (15..=17).contains(&bottom(&time)),
        "the time sits on row 17"
    );
    assert!(
        (32..=34).contains(&bottom(&date)),
        "the date sits on row 34"
    );
    let centre = |v: &[(i32, i32)]| {
        let lo = v.iter().map(|&(x, _)| x).min().unwrap();
        let hi = v.iter().map(|&(x, _)| x).max().unwrap();
        lo + hi
    };
    let mid = 2 * k.x + k.width;
    assert!((centre(&time) - mid).abs() <= 3, "the time is centred");
    assert!((centre(&date) - mid).abs() <= 3, "the date is centred");
}

#[test]
fn an_unlocked_glass_bars_grips_stand_out_from_the_bar_beside_them() {
    use crate::taskbar::{Bands, BarSpan, ClockText};
    let text = TextRenderer::load(&FontConfig::default());
    for small in [false, true] {
        let mut theme = theme_for(Interface::Glass);
        if small {
            crate::interfaces::small_bar(&mut theme, &Default::default());
        }
        let s = theme.look.style().unwrap();
        let start = TextRenderer::load(&s.start_text.font());
        for edge in [Edge::Bottom, Edge::Left] {
            let span = BarSpan {
                edge,
                length: if edge.is_side() { 600 } else { 800 },
                thickness: if edge.is_side() {
                    s.taskbar.side_default(&theme.metrics)
                } else {
                    theme.metrics.taskbar_height
                },
            };
            let counts = TaskbarCounts {
                quick: 2,
                tasks: 1,
                ..TaskbarCounts::default()
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &s.taskbar,
                span,
                false,
                counts,
                ClockText::default(),
                Bands::default(),
            );
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            let st = state_with_tasks(&[""]);
            draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
            assert_eq!(l.grips.len(), 2, "small {small} {edge:?}: two bands");
            for (i, g) in l.grips.iter().enumerate() {
                let shows = |a: u32, b: u32| {
                    let (a, b) = (over_grey(a), over_grey(b));
                    (0..3).any(|ch| (a[ch] - b[ch]).abs() >= 24)
                };
                for k in 0..2 {
                    let (line, beside): (Vec<_>, Vec<_>) = if edge.is_side() {
                        (g.x..g.right())
                            .map(|x| (c.get(x, g.y + k), c.get(x, g.y - 1)))
                            .unzip()
                    } else {
                        (g.y..g.bottom())
                            .map(|y| (c.get(g.x + k, y), c.get(g.x - 1, y)))
                            .unzip()
                    };
                    assert!(
                        line.iter().zip(&beside).all(|(&a, &b)| shows(a, b)),
                        "small {small} {edge:?}: grip {i}'s line {k} {line:08x?} \
                         stands out from the bar beside it {beside:08x?}"
                    );
                }
            }
        }
    }
}

#[test]
fn an_unlocked_bar_marks_each_grip_with_the_taskbar_atlas_row_27_colour() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        false,
        TaskbarCounts::default(),
        40,
    );
    let g = layout.grips[0];
    let bar = s.part(Part::Taskbar).unwrap();
    let f = bar.frame(State::Normal);
    let want = bar.atlas.get(f.x, f.y + GRIP_ROW);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
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
    assert_eq!(c.get(g.x, g.y + 1), want);
}

fn bar_with_strip(i: Interface, hot: bool) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(i);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tasks: 1,
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 40);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&["One"]);
    st.show_desktop_hot = hot;
    let mut c = Canvas::new(1920, layout.height as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    (c, layout)
}

#[test]
fn the_glass_strip_replaces_the_bar_under_it() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, layout) = bar_with_strip(Interface::Glass, false);
    let r = layout.show_desktop.unwrap();
    let mut alone = Canvas::new(1920, 40);
    s.draw(&mut alone, r, Part::ShowDesktop, State::Normal, 1);
    for (x, y) in [(0, 0), (0, 1), (7, 1), (7, 20), (14, 39)] {
        assert_eq!(
            c.get(r.x + x, y),
            alone.get(r.x + x, y),
            "({x}, {y}) is the strip's own pixel, no bar under it"
        );
    }
}

#[test]
fn blue_draws_its_strip_as_a_task_button_lit_under_the_pointer() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    for (hot, state) in [(false, State::Normal), (true, State::Hot)] {
        let (c, layout) = bar_with_strip(Interface::Blue, hot);
        let r = layout.show_desktop.unwrap();
        assert_eq!(r, Rect::new(1895, 3, 25, 25));
        let mut base = Canvas::new(1920, 30);
        s.draw(
            &mut base,
            Rect::new(0, 0, 1920, 30),
            Part::Taskbar,
            State::Normal,
            1,
        );
        s.draw(&mut base, r, Part::TaskButton, state, 1);
        for (x, y) in [(1, 1), (2, 12), (12, 12), (23, 23)] {
            assert_eq!(
                c.get(r.x + x, r.y + y),
                base.get(r.x + x, r.y + y),
                "hot {hot}: ({x}, {y}) is the {state:?} task button over the bar"
            );
        }
    }
    let (idle, l) = bar_with_strip(Interface::Blue, false);
    let (lit, _) = bar_with_strip(Interface::Blue, true);
    let r = l.show_desktop.unwrap();
    assert_ne!(idle.get(r.x + 12, r.y + 12), lit.get(r.x + 12, r.y + 12));
}

fn doubles_outside(one: &Canvas, two: &Canvas, skip: std::ops::Range<i32>) -> bool {
    (0..two.height as i32).all(|y| {
        (0..two.width as i32)
            .filter(|x| !skip.contains(&(x / 2)))
            .all(|x| two.get(x, y) == one.get(x / 2, y / 2))
    })
}

fn blue_bar_at_1_and_2(flag: bool) -> (Canvas, Canvas, std::ops::Range<i32>) {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tray: 1,
        ..TaskbarCounts::default()
    };
    let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
    let draw = |theme: &Theme, layout: &TaskbarLayout| {
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let px = s.taskbar.start_icon * theme.scale;
        let mut ic = Canvas::new(px as u32, px as u32);
        ic.fill_rect(Rect::new(0, 0, px, px), crate::theme::Color::rgb(9, 99, 9));
        let st = TaskbarState {
            start_icon: flag.then_some(ic.into()),
            orb_glow: 100,
            ..state_with_tasks(&[])
        };
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_taskbar(&mut c, s, theme, &text, &start, layout, &st);
        c
    };
    let one = draw(&theme, &layout);
    let two = draw(&theme.scaled(2), &layout.scaled(2));
    let label = layout.start.x + 34..layout.start.right() + 2;
    (one, two, label)
}

#[test]
fn a_blue_bar_at_scale_2_doubles_the_scale_1_bar_outside_the_start_label() {
    let (one, two, label) = blue_bar_at_1_and_2(true);
    assert!(
        doubles_outside(&one, &two, label),
        "the bar, the Start pill and flag and the tray area at scale 2 are the scale-1 pixels doubled"
    );
}

#[test]
fn blue_s_start_mark_is_the_16_px_drawing_and_the_32_px_one_at_scale_2() {
    let (one, two, label) = blue_bar_at_1_and_2(false);
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tray: 1,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
    let flag = l.start.x + 10..label.end;
    assert!(
        doubles_outside(&one, &two, flag),
        "the bar around the mark doubles"
    );
    let ink = s.start_text.color;
    let (x, y) = (l.start.x + 11, l.start.y + 6);
    crate::art::logo::assert_drawn(&one, x, y, 16, ink);
    crate::art::logo::assert_drawn(&two, 2 * x, 2 * y, 32, ink);
}

#[test]
fn the_glass_orb_carries_the_mark_in_white_without_a_theme_logo() {
    let (c, l) = glass_bar("", None);
    let s = theme_for(Interface::Glass)
        .look
        .style()
        .unwrap()
        .taskbar
        .start_icon
        * ORB_MARK_SHARE.0
        / ORB_MARK_SHARE.1;
    let d = l.start_disc.unwrap_or(l.start);
    crate::art::logo::assert_path_drawn(
        &c,
        d.x + (d.width - s) / 2,
        d.y + (d.height - s) / 2,
        s,
        crate::theme::Color::rgb(0xff, 0xff, 0xff),
    );
}

fn ink_size(c: &Canvas, r: Rect, ink: u32) -> (i32, i32) {
    let hits: Vec<(i32, i32)> = (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .filter(|&(x, y)| c.get(x, y) == ink)
        .collect();
    let xs = || hits.iter().map(|p| p.0);
    let ys = || hits.iter().map(|p| p.1);
    (
        xs().max().unwrap() - xs().min().unwrap() + 1,
        ys().max().unwrap() - ys().min().unwrap() + 1,
    )
}

fn blue_clock_bar(clock: &str) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let counts = TaskbarCounts {
        tray: 2,
        ..TaskbarCounts::default()
    };
    let w = text.measure(clock, false, theme.fonts.size_px);
    let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, w);
    let st = TaskbarState {
        clock: clock.into(),
        tray_icons: vec![None, None],
        ..state_with_tasks(&[])
    };
    let mut c = Canvas::new(1920, 30);
    draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
    (c, layout)
}

#[test]
fn blue_sets_the_clock_in_its_48_px_box_12_px_from_the_end() {
    if !crate::text::test_tahoma() {
        return;
    }
    for (clock, x0, x1) in [
        ("11:00 PM", 1862, 1904),
        ("12:19 PM", 1862, 1904),
        ("10:28 AM", 1862, 1905),
        ("7:46 AM", 1864, 1902),
        ("8:29 PM", 1864, 1901),
        ("9:12 PM", 1864, 1901),
        ("1:35 PM", 1865, 1901),
    ] {
        let (c, layout) = blue_clock_bar(clock);
        assert_eq!(layout.clock, Rect::new(1860, 3, 48, 25), "{clock}: the box");
        let white = crate::theme::Color::rgb(0xFF, 0xFF, 0xFF).argb();
        let ink: Vec<(i32, i32)> = (0..30)
            .flat_map(|y| (1840..1920).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == white)
            .collect();
        let xs = || ink.iter().map(|p| p.0);
        let ys = || ink.iter().map(|p| p.1);
        assert_eq!(
            (xs().min(), xs().max()),
            (Some(x0), Some(x1)),
            "{clock}: its first and last inked columns"
        );
        assert_eq!(
            (ys().min(), ys().max()),
            (Some(11), Some(18)),
            "{clock}: its rows"
        );
    }
}

#[test]
fn every_variant_sets_the_start_label_and_the_clock_as_blue_does() {
    use crate::blue::{Variant, style_for};
    use crate::theme::Color;
    let blue = style_for(Variant::Blue);
    for v in Variant::ALL {
        let s = style_for(v);
        let mut want = blue.start_text.clone();
        if let Some(sh) = &mut want.shadow {
            sh.color = v.scheme().inks.start_shadow;
        }
        assert_eq!(s.start_text, want, "{v:?}: the Start label");
        assert_eq!(s.taskbar, blue.taskbar, "{v:?}: the clock box and tray");
    }
    assert_eq!(
        style_for(Variant::Ember)
            .start_text
            .shadow
            .map(|sh| sh.color),
        Some(Color::rgb(0x00, 0x00, 0x00)),
        "ember's label casts a black shadow"
    );
}

#[test]
fn blues_tray_icons_sit_2_px_apart_6_px_before_the_clock_in_a_well_from_11_px_before_them() {
    // The box holds any 12-hour time, so the well stays put.
    for w in [39, 45, 46] {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 2,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, w);
        assert_eq!(l.clock, Rect::new(1860, 3, 48, 25), "{w} px of text");
        assert_eq!(
            l.tray,
            [Rect::new(1820, 8, 16, 16), Rect::new(1838, 8, 16, 16)]
        );
        assert_eq!(l.notification, Rect::new(1809, 0, 111, 30));
    }
}

#[test]
fn a_styled_clock_at_scale_2_is_set_at_twice_the_scale_1_font_size() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &s.taskbar,
        800,
        true,
        TaskbarCounts::default(),
        40,
    );
    let draw = |theme: &Theme, layout: &TaskbarLayout| {
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let st = TaskbarState {
            clock: "12:34".into(),
            ..state_with_tasks(&[])
        };
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_taskbar(&mut c, s, theme, &text, &start, layout, &st);
        ink_size(&c, layout.clock, s.bar_text.argb())
    };
    let (w1, h1) = draw(&theme, &layout);
    let (w2, h2) = draw(&theme.scaled(2), &layout.scaled(2));
    assert!(
        (w2 - 2 * w1).abs() <= 2 && (h2 - 2 * h1).abs() <= 2,
        "the scale-2 clock inks {w2}x{h2}, expected {}x{} +- 2",
        2 * w1,
        2 * h1
    );
}

fn glass_bar_at(
    theme: &Theme,
    s: &Style,
    (edge, t): (Edge, i32),
    strip_hot: bool,
    sc: i32,
) -> (Canvas, TaskbarLayout) {
    use crate::taskbar::{Bands, BarSpan, ClockText, TaskRow};
    let shape = s.taskbar;
    let counts = TaskbarCounts {
        tasks: 3,
        tray: 1,
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let span = BarSpan {
        edge,
        length: 600,
        thickness: t,
    };
    let mut l = TaskbarLayout::compute_on(
        &theme.metrics,
        &shape,
        span,
        true,
        counts,
        ClockText::default(),
        Bands::default(),
    );
    l.lay_tasks(&theme.metrics, &shape, &[1, 2, 3], TaskRow::Icons);
    let (theme, l) = (theme.scaled(sc), l.scaled(sc));
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&["", "", ""]);
    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), shape.task_icon * sc);
    for t in &mut st.tasks {
        t.icon = Some(icon.clone().into());
    }
    st.tasks[1].active = true;
    st.show_desktop_hot = strip_hot;
    st.tray_icons = vec![None];
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
    (c, l)
}

#[test]
fn a_glass_bar_at_scale_2_draws_its_parts_painted_at_2_where_scale_1_puts_them() {
    let glass = theme_for(Interface::Glass);
    let mut small = theme_for(Interface::Glass);
    crate::interfaces::small_bar(&mut small, &Default::default());
    for (theme, h) in [(&glass, 40), (&small, 30)] {
        let s2 = theme.scaled(2);
        let s = s2.look.style().unwrap();
        assert_eq!(s.part(Part::TaskButton).unwrap().scale, 2);
        let side = s.taskbar.side_default(&theme.metrics);
        for (edge, t) in [(Edge::Bottom, h), (Edge::Left, side)] {
            for hot in [false, true] {
                let (c, l) = glass_bar_at(theme, s, (edge, t), hot, 2);
                let (w, ht) = (l.width, l.height);
                let mut want = Canvas::new(w as u32, ht as u32);
                l.edge
                    .draw_art(&mut want, Rect::new(0, 0, w, ht), |a, along| {
                        s.draw(a, along, Part::Taskbar, State::Normal, 2);
                        let well = l.to_along(l.notification);
                        s.draw(a, well, Part::TrayArea, State::Normal, 2);
                    });
                let px = s.taskbar.task_icon * 2;
                let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
                for (i, &slot) in l.tasks.iter().enumerate() {
                    let st = if i == 1 {
                        State::Checked
                    } else {
                        State::Normal
                    };
                    let (front, edges) = stacked(s, slot, l.stacks.get(i).copied(), 2);
                    for (k, e) in edges.iter().enumerate() {
                        s.draw_nth(&mut want, *e, Part::TaskStack, st, k, 2);
                    }
                    s.draw(&mut want, front, Part::TaskButton, st, 2);
                    let (x, y) = (
                        front.x + (front.width - px) / 2,
                        front.y + (front.height - px) / 2,
                    );
                    want.blit_canvas(x, y, &icon);
                }
                let strip = l.show_desktop.unwrap();
                want.clear_rect(strip);
                let lit = if hot { State::Hot } else { State::Normal };
                l.edge.draw_art(&mut want, strip, |a, at| {
                    s.draw(a, at, Part::ShowDesktop, lit, 2)
                });
                let what = format!("{h} px bar along {edge:?}, strip lit {hot}");
                let first = l.tasks[0];
                let rects = l.tasks.iter().copied().chain([strip, l.notification]);
                for r in rects {
                    for y in r.y..r.bottom() {
                        for x in r.x..r.right() {
                            assert_eq!(c.get(x, y), want.get(x, y), "{what}: ({x}, {y})");
                        }
                    }
                }
                let gap = if edge.is_side() {
                    Rect::new(0, l.start.bottom(), w, first.y - l.start.bottom())
                } else {
                    Rect::new(l.start.right(), 0, first.x - l.start.right(), ht)
                };
                for y in gap.y..gap.bottom() {
                    for x in gap.x..gap.right() {
                        assert_eq!(c.get(x, y), want.get(x, y), "{what}: bar ({x}, {y})");
                    }
                }
            }
        }
    }
}
