use super::*;

fn side_bar_in(
    theme: &Theme,
    s: &Style,
    edge: Edge,
    stacks: &[u8],
    hot: Option<usize>,
) -> (Canvas, TaskbarLayout) {
    let t = s.taskbar.side_default(&theme.metrics);
    side_bar_titled(theme, s, edge, t, stacks, hot, "")
}

fn side_bar_titled(
    theme: &Theme,
    s: &Style,
    edge: Edge,
    t: i32,
    stacks: &[u8],
    hot: Option<usize>,
    title: &str,
) -> (Canvas, TaskbarLayout) {
    side_bar_with(theme, s, edge, t, 0, stacks, title, |st| st.task_hot = hot)
}

#[allow(clippy::too_many_arguments)]
fn side_bar_with(
    theme: &Theme,
    s: &Style,
    edge: Edge,
    t: i32,
    pager: usize,
    stacks: &[u8],
    title: &str,
    tweak: impl FnOnce(&mut TaskbarState),
) -> (Canvas, TaskbarLayout) {
    use crate::taskbar::{Bands, BarSpan, ClockText, TaskRow};
    let shape = s.taskbar;
    let counts = TaskbarCounts {
        pager,
        tasks: stacks.len(),
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
    l.lay_tasks(&theme.metrics, &shape, stacks, TaskRow::Shared);
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut st = state_with_tasks(&vec![title; stacks.len()]);
    st.start_icon = Some(clear_start_logo());
    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), shape.task_icon);
    for t in &mut st.tasks {
        t.icon = Some(icon.clone().into());
    }
    st.pager = vec![crate::taskbar::PagerButton::default(); pager];
    tweak(&mut st);
    let mut c = Canvas::new(l.width as u32, l.height as u32);
    draw_taskbar(&mut c, s, theme, &text, &start, &l, &st);
    (c, l)
}

fn side_bar(
    interface: Interface,
    edge: Edge,
    stacks: &[u8],
    hot: Option<usize>,
) -> (Canvas, TaskbarLayout) {
    let theme = theme_for(interface);
    let s = theme.look.style().unwrap();
    side_bar_in(&theme, s, edge, stacks, hot)
}

#[test]
fn a_side_blue_bar_lays_its_bar_and_well_on_their_side_under_upright_buttons() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let start = TextRenderer::load(&s.start_text.font());
    let px = s.taskbar.task_icon;
    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
    for edge in [Edge::Left, Edge::Right] {
        let (c, l) = side_bar(Interface::Blue, edge, &[1, 1], None);
        let (w, h) = (l.width, l.height);
        assert_eq!(
            l.notification.width, w,
            "{edge:?}: the well a block across the column"
        );
        assert!(!l.icons_only, "{edge:?}: labelled at Start's width");
        let mut want = Canvas::new(w as u32, h as u32);
        want.with_transposed(Rect::new(0, 0, w, h), edge == Edge::Left, |a| {
            s.draw(a, Rect::new(0, 0, h, w), Part::Taskbar, State::Normal, 1);
            s.draw(
                a,
                l.to_along(l.notification),
                Part::TrayArea,
                State::Normal,
                1,
            );
        });
        s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
        let mut st = state_with_tasks(&["", ""]);
        st.start_icon = Some(clear_start_logo());
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
            same_columns(&c, &want, 0..w),
            Ok(()),
            "{edge:?}: the bar and the well on their side; Start, the buttons and the strip upright"
        );
    }
}

#[test]
fn a_side_glass_bar_lays_its_bar_well_and_strip_on_their_side_under_upright_buttons() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), 32);
    for edge in [Edge::Left, Edge::Right] {
        let (c, l) = side_bar(Interface::Glass, edge, &[1, 1, 1], None);
        let (w, h) = (l.width, l.height);
        let left = edge == Edge::Left;
        assert!(l.icons_only, "{edge:?}: one slot, icons only");
        let strip = l.show_desktop.unwrap();
        assert_eq!(
            strip,
            Rect::new(0, 585, w, 15),
            "{edge:?}: 15 px across the column, last"
        );
        let mut want = Canvas::new(w as u32, h as u32);
        want.with_transposed(Rect::new(0, 0, w, h), left, |a| {
            s.draw(a, Rect::new(0, 0, h, w), Part::Taskbar, State::Normal, 1);
            s.draw(
                a,
                l.to_along(l.notification),
                Part::TrayArea,
                State::Normal,
                1,
            );
        });
        s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
        for r in &l.tasks {
            s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
            want.blit_canvas(r.x + 14, r.y + 4, &icon);
        }
        want.clear_rect(strip);
        want.with_transposed(strip, left, |a| {
            let at = Rect::new(0, 0, strip.height, strip.width);
            s.draw(a, at, Part::ShowDesktop, State::Normal, 1)
        });
        assert_eq!(
            same_columns(&c, &want, 0..w),
            Ok(()),
            "{edge:?}: the bar, the well and the strip on their side; the orb and the buttons upright"
        );
    }
}

#[test]
fn a_glass_column_draws_its_buttons_icon_only_below_70_px() {
    let glass = theme_for(Interface::Glass);
    let mut small = theme_for(Interface::Glass);
    crate::interfaces::small_bar(&mut small, &Default::default());
    let title = "Untitled - Notepad";
    for (theme, t) in [(&glass, 60), (&small, 44)] {
        let s = theme.look.style().unwrap();
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar_titled(theme, s, edge, t, &[1, 1], None, title);
            let (bare, _) = side_bar_titled(theme, s, edge, t, &[1, 1], None, "");
            assert!(l.icons_only, "{edge:?} at {t} px: icons only");
            assert_eq!(
                same_columns(&c, &bare, 0..l.width),
                Ok(()),
                "{edge:?} at {t} px: the buttons as if untitled, no title on them"
            );
        }
    }
    let s = glass.look.style().unwrap();
    let (c, l) = side_bar_titled(&glass, s, Edge::Left, 70, &[1, 1], None, title);
    let (bare, _) = side_bar_titled(&glass, s, Edge::Left, 70, &[1, 1], None, "");
    assert!(!l.icons_only, "70 px: labelled");
    assert_ne!(
        same_columns(&c, &bare, 0..l.width),
        Ok(()),
        "70 px: the title on the buttons"
    );
    let counts = TaskbarCounts {
        tasks: 2,
        ..TaskbarCounts::default()
    };
    let mut l = TaskbarLayout::compute(&glass.metrics, &s.taskbar, 800, true, counts, 60);
    l.lay_tasks(
        &glass.metrics,
        &s.taskbar,
        &[1, 1],
        crate::taskbar::TaskRow::Icons,
    );
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let draw = |title: &str| {
        let mut c = Canvas::new(800, l.height as u32);
        let st = state_with_tasks(&[title, title]);
        draw_taskbar(&mut c, s, &glass, &text, &start, &l, &st);
        c
    };
    assert_eq!(
        same_columns(&draw(title), &draw(""), 0..800),
        Ok(()),
        "a combined row: no title on its buttons"
    );
}

#[test]
fn on_a_side_bar_a_lit_idle_glass_pin_has_its_light_down_its_outer_side() {
    for edge in [Edge::Left, Edge::Right] {
        let (lit, l) = side_bar(Interface::Glass, edge, &[0, 1], Some(0));
        let (rest, _) = side_bar(Interface::Glass, edge, &[0, 1], None);
        let r = l.tasks[0];
        let (outer, inner) = if edge == Edge::Left {
            (r.x + 2, r.right() - 5)
        } else {
            (r.right() - 3, r.x + 4)
        };
        let y = r.y + r.height / 2;
        assert!(
            lum(lit.get(outer, y)) > lum(rest.get(outer, y)) + 20,
            "{edge:?}: lit down its outer side"
        );
        assert_eq!(
            lit.get(inner, y),
            rest.get(inner, y),
            "{edge:?}: not on its inner side"
        );
    }
}

fn slot_ends(s: &Style, c: &mut Canvas, slot: Rect, state: State) {
    for x in [slot.x, slot.right() - 2] {
        let at = Rect::new(x, slot.y, 2, slot.height);
        s.draw(c, at, Part::SlotEdge, state, 1);
    }
}

#[test]
fn on_a_side_bar_a_lit_idle_glass_pin_turns_its_edges_and_band_across_its_ends() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let t = s.taskbar.side_default(&theme.metrics);
    let bar = |edge: Edge, hover: Hover| {
        let (hot, held) = hover.hot_held();
        side_bar_with(&theme, s, edge, t, 0, &[0, 1], "", |st| {
            st.task_hot = hot;
            st.task_pressed = held;
        })
    };
    for edge in [Edge::Left, Edge::Right] {
        let (rest, l) = bar(edge, Hover::None);
        let r = l.tasks[0];
        let px = s.taskbar.task_icon;
        let icon = Rect::new(
            r.x + (r.width - px) / 2 - 1,
            r.y + (r.height - px) / 2 - 1,
            px + 3,
            px + 3,
        );
        for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
            let got = bar(edge, hover).0;
            let mut want = rest.clone();
            if state == State::Pressed {
                want = turned(&want, r, edge, |c, p| {
                    let band = Rect::new(2, 0, p.width - 4, p.height);
                    s.draw(c, band, Part::SlotBand, State::Pressed, 1)
                });
            }
            want = turned(&want, r, edge, |c, p| slot_ends(s, c, p, state));
            want = turned(&want, r, edge, |c, p| {
                let foot = Rect::new(p.width / 2 - 30, p.height - 7, 60, 7);
                s.draw(c, foot, Part::SlotLight, state, 1)
            });
            for y in r.y..r.bottom() {
                for x in r.x..r.right() {
                    if icon.contains(x, y) {
                        continue;
                    }
                    assert_eq!(
                        got.get(x, y),
                        want.get(x, y),
                        "{edge:?} {state:?} ({x}, {y}): the band, the edges and the light turned"
                    );
                }
            }
        }
    }
}

#[test]
fn on_a_side_bar_the_hot_glass_pager_cell_turns_its_edges_across_its_ends() {
    use crate::taskbar::PagerMark;
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let t = s.taskbar.side_default(&theme.metrics);
    for edge in [Edge::Left, Edge::Right] {
        let bar = |hot: Option<usize>| {
            side_bar_with(&theme, s, edge, t, 4, &[], "", |st| {
                st.pager_mark = PagerMark::Light;
                st.pager[0].current = true;
                st.pager_hot = hot;
            })
        };
        let (rest, l) = bar(None);
        let (got, _) = bar(Some(1));
        let r = l.pager[1];
        let want = turned(&rest, r, edge, |c, p| slot_ends(s, c, p, State::Hot));
        assert_eq!(
            pixels(&got, r),
            pixels(&want, r),
            "{edge:?}: the cell's edges across its top and foot"
        );
    }
}

#[test]
fn on_a_side_bar_the_glass_pager_light_lies_whole_inside_the_column() {
    use crate::taskbar::PagerMark;
    let glass = theme_for(Interface::Glass);
    let mut small = theme_for(Interface::Glass);
    crate::interfaces::small_bar(&mut small, &Default::default());
    for (theme, t) in [(&glass, 60), (&small, 44), (&glass, 140)] {
        let s = theme.look.style().unwrap();
        for edge in [Edge::Left, Edge::Right] {
            let bar = |current: bool| {
                side_bar_with(theme, s, edge, t, 4, &[], "", |st| {
                    st.pager_mark = PagerMark::Light;
                    st.pager[0].current = current;
                })
            };
            let ((lit, l), (dark, _)) = (bar(true), bar(false));
            let r = l.pager[0];
            let xs: Vec<i32> = (r.x..r.right())
                .filter(|&x| {
                    (r.y..r.bottom()).any(|y| lum(lit.get(x, y)) > lum(dark.get(x, y)) + 20)
                })
                .collect();
            assert!(!xs.is_empty(), "{edge:?} at {t} px: the cell lit");
            let (first, last) = (xs[0], *xs.last().unwrap());
            assert!(
                first >= 2 && last < l.width - 2,
                "{edge:?} at {t} px: the light (x {first}..={last}) clear of the screen edge and the 2 px inner edge"
            );
        }
    }
}

#[test]
fn an_unlocked_side_bar_marks_each_grip_with_a_line_across_the_column() {
    use crate::taskbar::{Bands, BarSpan, ClockText};
    for interface in [Interface::Blue, Interface::Glass] {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        let bar = s.part(Part::Taskbar).unwrap();
        let f = bar.frame(State::Normal);
        let row = |y: i32| bar.atlas.get(f.x, f.y + y);
        let inks = match interface {
            Interface::Glass => vec![row(0), row(1)],
            _ => vec![row(GRIP_ROW)],
        };
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        for edge in [Edge::Left, Edge::Right] {
            let span = BarSpan {
                edge,
                length: 600,
                thickness: s.taskbar.side_default(&theme.metrics),
            };
            let counts = TaskbarCounts {
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
            draw_taskbar(
                &mut c,
                s,
                &theme,
                &text,
                &start,
                &l,
                &state_with_tasks(&[""]),
            );
            for g in &l.grips {
                assert!(
                    g.width > g.height,
                    "{interface:?} {edge:?}: the grip lies across"
                );
                for (k, &ink) in (0..).zip(&inks) {
                    let line: Vec<u32> = (g.x..g.right()).map(|x| c.get(x, g.y + k)).collect();
                    assert_eq!(
                        line,
                        vec![ink; g.width as usize],
                        "{interface:?} {edge:?}: the grip's row {k} in {ink:08x}"
                    );
                }
                let n = inks.len() as i32;
                assert!(
                    !inks.contains(&c.get(g.x + g.width / 2, g.y + n)),
                    "{interface:?} {edge:?}: {n} rows, hairlines"
                );
            }
        }
    }
}

#[test]
fn a_side_bar_points_its_chevron_at_the_screen() {
    use crate::taskbar::{Bands, BarSpan, ClockText};
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let ink = s.bar_text.argb();
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    for edge in [Edge::Left, Edge::Right] {
        let counts = TaskbarCounts {
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length: 600,
            thickness: s.taskbar.side_default(&theme.metrics),
        };
        let l = TaskbarLayout::compute_on(
            &theme.metrics,
            &s.taskbar,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
        let r = l.chevron.unwrap();
        let count = |x: i32| (0..16).filter(|&y| c.get(r.x + x, r.y + y) == ink).count();
        let (tip, base) = if edge == Edge::Left { (9, 6) } else { (6, 9) };
        assert_eq!(
            (count(tip), count(base)),
            (1, 7),
            "{edge:?}: the tip toward the screen"
        );
    }
}

#[test]
fn a_side_bar_draws_its_more_cell_with_the_double_chevron() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let (c, l) = side_bar(Interface::Glass, Edge::Left, &[1; 14], None);
    let r = l
        .more
        .expect("fourteen 40 px buttons overrun a 600 px column");
    let ink = s.bar_text.argb();
    let (x0, mid) = (r.x + (r.width - 10) / 2, r.y + r.height / 2);
    assert_eq!(c.get(x0, mid), ink, "the first >'s back");
    assert_eq!(c.get(x0 + 6, mid), ink, "the second's");
    assert_ne!(c.get(x0 + 5, mid), ink, "the gap between them");
}

#[test]
fn a_side_blue_bars_more_cell_is_a_task_button_at_rest_with_the_double_chevron() {
    let theme = theme_for(Interface::Blue);
    let s = theme.look.style().unwrap();
    let (c, l) = side_bar(Interface::Blue, Edge::Left, &[1; 30], None);
    let r = l
        .more
        .expect("thirty 25 px buttons overrun a 600 px column");
    let mut bare = l.clone();
    bare.more = None;
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let mut want = Canvas::new(l.width as u32, l.height as u32);
    let st = state_with_tasks(&[""; 30]);
    draw_taskbar(&mut want, s, &theme, &text, &start, &bare, &st);
    s.draw(&mut want, r, Part::TaskButton, State::Normal, 1);
    crate::taskbar::draw_more(&mut want, r, s.bar_text, 1);
    let cell = |c: &Canvas| {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)))
            .collect::<Vec<_>>()
    };
    assert_eq!(cell(&c), cell(&want));
    assert_eq!(
        c.get(r.x + (r.width - 10) / 2, r.y + r.height / 2),
        s.bar_text.argb(),
        "the first >'s back in the bar's ink"
    );
}

fn bar_with_popup_cells(
    interface: Interface,
    edge: Edge,
    tasks: usize,
    cells: impl Fn(&mut TaskbarState),
) -> (Canvas, Canvas, TaskbarLayout) {
    use crate::taskbar::{Bands, BarSpan, ClockText};
    let theme = theme_for(interface);
    let s = theme.look.style().unwrap();
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
        tasks,
        tray: 1,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute_on(
        &theme.metrics,
        &s.taskbar,
        span,
        true,
        counts,
        ClockText::default(),
        Bands::default(),
    );
    let text = TextRenderer::load(&FontConfig::default());
    let start = TextRenderer::load(&s.start_text.font());
    let draw = |l: &TaskbarLayout, st: &TaskbarState| {
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, l, st);
        c
    };
    let mut st = state_with_tasks(&vec![""; tasks]);
    let mut bare = l.clone();
    bare.more = None;
    bare.chevron = None;
    let under = draw(&bare, &st);
    cells(&mut st);
    (draw(&l, &st), under, l)
}

#[test]
fn the_more_cell_lights_under_the_pointer_and_is_checked_while_its_menu_is_open() {
    use crate::taskbar::PopupCell;
    let cases = [
        (
            PopupCell {
                hot: true,
                open: false,
                held: false,
            },
            State::Hot,
        ),
        (
            PopupCell {
                hot: false,
                open: true,
                held: false,
            },
            State::Checked,
        ),
        (
            PopupCell {
                hot: true,
                open: true,
                held: false,
            },
            State::Checked,
        ),
    ];
    for interface in [Interface::Blue, Interface::Glass] {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        for edge in [Edge::Left, Edge::Right] {
            for (cell, look) in cases {
                let (c, mut want, l) =
                    bar_with_popup_cells(interface, edge, 30, |st| st.more = cell);
                let r = l.more.expect("thirty buttons overrun a 600 px column");
                s.draw(&mut want, r, Part::TaskButton, look, 1);
                let ink = s.text_color(Part::TaskButton, look).unwrap_or(s.bar_text);
                crate::taskbar::draw_more(&mut want, r, ink, 1);
                assert_eq!(
                    pixels(&c, r),
                    pixels(&want, r),
                    "{interface:?} {edge:?} {cell:?}: a {look:?} task button under the »"
                );
            }
        }
    }
}

#[test]
fn the_chevron_cell_lights_under_the_pointer_and_is_checked_while_its_flyout_is_open() {
    use crate::taskbar::PopupCell;
    let cases = [
        (
            PopupCell {
                hot: true,
                open: false,
                held: false,
            },
            State::Hot,
        ),
        (
            PopupCell {
                hot: false,
                open: true,
                held: false,
            },
            State::Checked,
        ),
        (
            PopupCell {
                hot: true,
                open: true,
                held: false,
            },
            State::Checked,
        ),
    ];
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
        for (cell, look) in cases {
            let (c, mut want, l) =
                bar_with_popup_cells(Interface::Glass, edge, 1, |st| st.chevron = cell);
            let r = l.chevron.unwrap();
            s.draw(&mut want, r, Part::TaskButton, look, 1);
            let ink = s.text_color(Part::TaskButton, look).unwrap_or(s.bar_text);
            edge.draw_art(&mut want, r, |w, at| {
                crate::taskbar::draw_chevron(w, at, ink, 1)
            });
            assert_eq!(
                pixels(&c, r),
                pixels(&want, r),
                "{edge:?} {cell:?}: a {look:?} task button under the ^"
            );
        }
    }
}

#[test]
fn a_side_glass_bars_end_cap_tops_the_column_and_its_lines_run_down_the_inner_edge() {
    let theme = theme_for(Interface::Glass);
    let s = theme.look.style().unwrap();
    let mut bottom = Canvas::new(600, 40);
    s.draw(
        &mut bottom,
        Rect::new(0, 0, 600, 40),
        Part::Taskbar,
        State::Normal,
        1,
    );
    for edge in [Edge::Left, Edge::Right] {
        let (c, l) = side_bar(Interface::Glass, edge, &[1], None);
        let w = l.width;
        let col = |k: i32| if edge == Edge::Left { w - 1 - k } else { k };
        assert!(
            l.start.bottom() < 104,
            "the end cap's rows are among those checked"
        );
        let rows = (l.start.bottom()..l.tasks[0].y).chain(l.tasks[0].bottom()..l.notification.y);
        for y in rows {
            for k in 0..2 {
                assert_eq!(
                    c.get(col(k), y),
                    bottom.get(y, k),
                    "{edge:?}: row {y}, the inner edge's column {k} is the bottom bar's row {k} at {y} px along"
                );
            }
        }
    }
}

#[test]
fn a_small_glass_side_bar_draws_its_whole_disc_inside_the_column() {
    let mut small = theme_for(Interface::Glass);
    crate::interfaces::small_bar(&mut small, &Default::default());
    let s = small.look.style().unwrap();
    let size = |p: Part| {
        let f = s.part(p).unwrap().frame(State::Normal);
        (f.width, f.height)
    };
    assert_eq!(
        (size(Part::StartButton), size(Part::StartAbove)),
        ((54, 30), (54, 13)),
        "the small orb's button art and the 13 rows over it"
    );
    for edge in [Edge::Left, Edge::Right] {
        let (c, l) = side_bar_in(&small, s, edge, &[1], None);
        let st = l.start;
        assert_eq!(st, Rect::new(0, 0, 44, 35), "{edge:?}");
        let bar = || {
            let mut b = Canvas::new(l.width as u32, l.height as u32);
            b.with_transposed(
                Rect::new(0, 0, l.width, l.height),
                edge == Edge::Left,
                |a| {
                    s.draw(
                        a,
                        Rect::new(0, 0, l.height, l.width),
                        Part::Taskbar,
                        State::Normal,
                        1,
                    )
                },
            );
            b
        };
        let mut want = bar();
        want.with_clip(st, |w| {
            s.draw(
                w,
                Rect::new(-5, -8, 54, 13),
                Part::StartAbove,
                State::Normal,
                1,
            );
            s.draw(
                w,
                Rect::new(-5, 5, 54, 30),
                Part::StartButton,
                State::Normal,
                1,
            );
        });
        assert_eq!(
            pixels(&c, st),
            pixels(&want, st),
            "{edge:?}: the orb at its own size, not stretched over Start"
        );
        let bare = bar();
        let disc = (0..5).filter(|&y| c.get(22, y) != bare.get(22, y)).count();
        assert_eq!(
            disc, 5,
            "{edge:?}: the disc's top five rows stand above the button art"
        );
        assert_eq!(
            c.get(1, 1),
            bare.get(1, 1),
            "{edge:?}: beside the disc, the bar alone"
        );
    }
}

#[test]
fn a_hidden_side_bar_leaves_its_inner_two_columns_on_screen() {
    for i in [Interface::Blue, Interface::Glass] {
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar(i, edge, &[1], None);
            let inner = if edge == Edge::Left {
                Rect::new(l.width - 2, 0, 2, 600)
            } else {
                Rect::new(0, 0, 2, 600)
            };
            assert_eq!(l.reveal_line(), inner, "{i:?} {edge:?}");
            let got = c.copy_rect(inner);
            let y = l.start.bottom() + 1;
            assert_ne!(got.get(0, y) >> 24, 0, "{i:?} {edge:?}: the line shows");
            assert_ne!(got.get(1, y) >> 24, 0, "{i:?} {edge:?}");
            if i == Interface::Glass {
                assert_ne!(
                    got.get(0, y),
                    got.get(1, y),
                    "{edge:?}: a dark line and a light one"
                );
            }
        }
    }
}
