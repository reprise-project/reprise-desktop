use super::*;

fn glass_top_row(stacks: &[u8], hot: Option<usize>) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let shape = s.taskbar;
    let counts = TaskbarCounts {
        tasks: stacks.len(),
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let mut l =
        TaskbarLayout::compute(&theme.metrics, &shape, 1920, true, counts, 60).on_edge(Edge::Top);
    l.lay_tasks(
        &theme.metrics,
        &shape,
        stacks,
        crate::taskbar::TaskRow::Icons,
    );
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&vec![""; stacks.len()]);
    st.start_icon = Some(clear_start_logo());
    let white = solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), 32);
    for t in &mut st.tasks {
        t.icon = Some(white.clone().into());
    }
    st.task_hot = hot;
    let mut c = Canvas::new(1920, 40);
    draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
    (c, l)
}

#[test]
fn a_top_glass_bar_turns_its_bar_well_and_strip_over_under_upright_buttons() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, l) = glass_top_row(&[1, 1, 1], None);
    assert_eq!(l.start, Rect::new(0, 0, 54, 40), "the orb fills the bar");
    let full = Rect::new(0, 0, 1920, 40);
    let strip = l.show_desktop.unwrap();
    let mut want = Canvas::new(1920, 40);
    want.with_rows_flipped(full, |w| {
        s.draw(w, full, Part::Taskbar, State::Normal, 1);
        s.draw(w, l.notification, Part::TrayArea, State::Normal, 1);
    });
    s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
    let white = solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), 32);
    for r in &l.tasks {
        s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
        want.blit_canvas(r.x + 14, r.y + 4, &white);
    }
    want.clear_rect(strip);
    want.with_rows_flipped(strip, |w| {
        s.draw(w, strip, Part::ShowDesktop, State::Normal, 1)
    });
    assert_eq!(
        same_columns(&c, &want, 0..1920),
        Ok(()),
        "the bar, the well and the strip turned over; the orb and the buttons upright"
    );
}

#[test]
fn a_top_glass_bar_turns_the_labelled_rows_veil_over_with_the_bar() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let st = state_with_tasks(&["One", "Two", "Three"]);
    let (bottom, _) = draw_glass(s, 40, &st);
    let counts = TaskbarCounts {
        tasks: 3,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60)
        .on_edge(Edge::Top);
    assert!(
        !l.icons_only && l.tasks_limit() > 1000,
        "a labelled row, its veil past x 1000"
    );
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut top = Canvas::new(1920, 40);
    draw_taskbar(&mut top, s, &theme, &text, &start, &l, &st);
    for y in 0..40 {
        assert_eq!(
            top.get(1000, y),
            bottom.get(1000, 39 - y),
            "row {y} is the Bottom bar's row {}",
            39 - y
        );
    }
}

#[test]
fn a_top_blue_bar_turns_its_bar_and_tray_over_under_upright_buttons() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tasks: 2,
        tray: 1,
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40)
        .on_edge(Edge::Top);
    assert_eq!(
        (l.tasks[0].y, l.tasks[0].height),
        (2, 25),
        "the buttons 2 px from the top, 3 from the foot"
    );
    let px = s.taskbar.task_icon;
    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
    let mut st = state_with_tasks(&["", ""]);
    for t in &mut st.tasks {
        t.icon = Some(icon.clone().into());
    }
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut c = Canvas::new(800, 30);
    draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
    let full = Rect::new(0, 0, 800, 30);
    let mut want = Canvas::new(800, 30);
    want.with_rows_flipped(full, |w| {
        s.draw(w, full, Part::Taskbar, State::Normal, 1);
        s.draw(w, l.notification, Part::TrayArea, State::Normal, 1);
    });
    s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
    draw_start_pill(&mut want, s, &start, l.start, &st, 1);
    for r in &l.tasks {
        s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
        want.blit_canvas(r.x + 10 + (16 - px) / 2, r.y + (r.height - px) / 2, &icon);
    }
    s.draw(
        &mut want,
        l.show_desktop.unwrap(),
        Part::TaskButton,
        State::Normal,
        1,
    );
    assert_eq!(
        same_columns(&c, &want, 0..800),
        Ok(()),
        "the bar and the tray turned over; Start, the buttons and the strip upright"
    );
}

#[test]
fn a_blue_bar_draws_the_round_arrow_and_a_glass_bar_keeps_its_caret() {
    for (interface, round) in [(Interface::Blue, true), (Interface::Glass, false)] {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        assert_eq!(l.tray_arrow, round, "{interface:?}");
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(800, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
        let r = l.chevron.unwrap();
        let mut bare = l.clone();
        bare.chevron = None;
        let mut want = Canvas::new(800, l.height as u32);
        draw_taskbar(
            &mut want,
            s,
            &theme,
            &text,
            &start,
            &bare,
            &state_with_tasks(&[]),
        );
        if round {
            let art = s.tray_arrow.as_deref().unwrap();
            draw_blue_tray_arrow(&mut want, r, art, PopupCell::default(), 1);
            assert_eq!(
                pixels(&c, r),
                pixels(&want, r),
                "the cell holds the round button"
            );
        } else {
            crate::taskbar::draw_chevron(&mut want, r, s.bar_text, 1);
            assert_eq!(pixels(&c, r), pixels(&want, r), "the cell holds the ^");
        }
    }
}

#[test]
fn a_held_blue_arrow_draws_its_pressed_disc() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tray: 1,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let r = l.chevron.unwrap();
    for open in [false, true] {
        let cell = PopupCell {
            hot: true,
            open,
            held: true,
        };
        let mut st = state_with_tasks(&[]);
        st.chevron = cell;
        let mut c = Canvas::new(800, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
        let mut bare = l.clone();
        bare.chevron = None;
        let mut want = Canvas::new(800, l.height as u32);
        draw_taskbar(
            &mut want,
            s,
            &theme,
            &text,
            &start,
            &bare,
            &state_with_tasks(&[]),
        );
        let art = s.tray_arrow.as_deref().unwrap();
        draw_blue_tray_arrow(&mut want, r, art, cell, 1);
        assert_eq!(
            pixels(&c, r),
            pixels(&want, r),
            "open {open}: the pressed disc"
        );
    }
}

#[test]
fn a_top_bar_points_its_caret_down_in_glass() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tray: 1,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let draw = |edge: Edge| {
        let l =
            TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40).on_edge(edge);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(800, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
        (c, l.chevron.unwrap())
    };
    let ((top, t), (bottom, b)) = (draw(Edge::Top), draw(Edge::Bottom));
    assert_eq!(t, b, "the cell is centred down the bar and keeps its rect");
    for y in 0..16 {
        assert_eq!(
            pixels(&top, Rect::new(t.x, t.y + y, 16, 1)),
            pixels(&bottom, Rect::new(b.x, b.y + 15 - y, 16, 1)),
            "the cell's row {y} is the Bottom cell's row {}",
            15 - y
        );
    }
}

#[test]
fn on_a_top_bar_a_lit_idle_glass_pin_has_its_light_turned_over_at_its_top() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, l) = glass_top_row(&[0, 1, 1], Some(0));
    let r = l.tasks[0];
    let full = Rect::new(0, 0, 1920, 40);
    let mut want = Canvas::new(1920, 40);
    want.with_rows_flipped(full, |w| s.draw(w, full, Part::Taskbar, State::Normal, 1));
    for x in [r.x, r.right() - 2] {
        s.draw(
            &mut want,
            Rect::new(x, r.y, 2, r.height),
            Part::SlotEdge,
            State::Hot,
            1,
        );
    }
    let light = Rect::new(r.x, r.y, 60, 7);
    want.with_rows_flipped(light, |w| s.draw(w, light, Part::SlotLight, State::Hot, 1));
    want.fill_rect(
        Rect::new(r.x + 14, r.y + 4, 32, 32),
        crate::theme::Color::rgb(0xFF, 0xFF, 0xFF),
    );
    assert_eq!(
        same_columns(&c, &want, r.x..r.right()),
        Ok(()),
        "the bar turned over, the edges upright, the 60x7 light turned over on the top rows, the icon"
    );
    let rest = glass_top_row(&[0, 1, 1], None).0;
    let at = (r.x + r.width / 2, r.y);
    assert!(
        lum(c.get(at.0, at.1)) > lum(rest.get(at.0, at.1)) + 20,
        "the slot's top row is lit"
    );
}

#[test]
fn on_a_top_bar_the_glass_pager_lights_the_current_desktop_at_its_top() {
    use crate::taskbar::{PagerButton, PagerMark, TaskRow};
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let shape = s.taskbar;
    let counts = TaskbarCounts {
        pager: 4,
        ..TaskbarCounts::default()
    };
    let mut l =
        TaskbarLayout::compute(&theme.metrics, &shape, 800, true, counts, 60).on_edge(Edge::Top);
    l.lay_tasks(&theme.metrics, &shape, &[], TaskRow::Icons);
    let text = TextRenderer::load(&theme.fonts);
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&[]);
    st.pager = (0..4)
        .map(|i| PagerButton {
            current: i == 1,
            attention: false,
        })
        .collect();
    st.pager_mark = PagerMark::Light;
    let mut top = Canvas::new(800, 40);
    draw_taskbar(&mut top, s, &theme, &text, &start, &l, &st);
    let (bottom, cells, _) = glass_pager(PagerMark::Light, None, false);
    let r1 = l.pager[1];
    assert_eq!(r1, cells[1], "a full-height cell keeps its rect");
    for y in 0..7 {
        for x in r1.x..r1.right() {
            assert_eq!(
                top.get(x, y),
                bottom.get(x, 39 - y),
                "({x}, {y}) is the Bottom cell's foot row {} turned over",
                39 - y
            );
        }
    }
    assert!(
        lum(top.get(r1.x + 11, 0)) > lum(top.get(l.pager[0].x + 11, 0)) + 20,
        "the current cell's top row is lit"
    );
}

fn hidden_line(interface: Interface, edge: Edge) -> (Canvas, Canvas, std::ops::Range<i32>) {
    let theme = theme_for(interface);
    let s = theme.look.style().unwrap();
    let counts = TaskbarCounts {
        tray: 1,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40).on_edge(edge);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let h = l.height;
    let mut c = Canvas::new(800, h as u32);
    draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
    let full = Rect::new(0, 0, 800, h);
    let mut art = Canvas::new(800, h as u32);
    if edge == Edge::Bottom {
        s.draw(&mut art, full, Part::Taskbar, State::Normal, 1)
    } else {
        art.with_rows_flipped(full, |a| s.draw(a, full, Part::Taskbar, State::Normal, 1))
    }
    let line = l.reveal_line();
    let inner = if edge == Edge::Bottom {
        Rect::new(0, 0, 800, 2)
    } else {
        Rect::new(0, h - 2, 800, 2)
    };
    assert_eq!(
        line, inner,
        "{interface:?} {edge:?}: the reveal line's rows"
    );
    (
        c.copy_rect(line),
        art.copy_rect(inner),
        l.start.right() + 2..l.notification.x - 2,
    )
}

#[test]
fn a_hidden_blue_bar_leaves_its_two_inner_bar_rows_on_screen() {
    for edge in [Edge::Bottom, Edge::Top] {
        let (got, want, xs) = hidden_line(Interface::Blue, edge);
        assert_eq!((got.width, got.height), (800, 2), "{edge:?}");
        assert_eq!(
            same_columns(&got, &want, xs.clone()),
            Ok(()),
            "{edge:?}: the line is the bar's own gradient rows"
        );
        assert_ne!(
            got.get(xs.start, 0),
            got.get(xs.start, 1),
            "{edge:?}: two rows of the gradient"
        );
    }
}

#[test]
fn a_hidden_glass_bar_leaves_its_dark_and_light_lines_on_screen() {
    for edge in [Edge::Bottom, Edge::Top] {
        let (got, want, xs) = hidden_line(Interface::Glass, edge);
        assert_eq!((got.width, got.height), (800, 2), "{edge:?}");
        assert_eq!(
            same_columns(&got, &want, xs.clone()),
            Ok(()),
            "{edge:?}: the line is the bar's own two edge lines"
        );
        assert_ne!(got.get(xs.start, 0) >> 24, 0, "{edge:?}: the line shows");
        assert_ne!(
            got.get(xs.start, 0),
            got.get(xs.start, 1),
            "{edge:?}: a dark line and a light one"
        );
    }
}
