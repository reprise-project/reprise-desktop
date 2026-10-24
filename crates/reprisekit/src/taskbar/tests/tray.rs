use super::*;

#[test]
fn the_chevron_leads_the_tray_one_cell_left_of_its_icons() {
    let m = Metrics::default();
    let shape = TaskbarShape {
        tray_arrow: None,
        ..TaskbarShape::CLASSIC
    };
    let bare = TaskbarLayout::compute(&m, &shape, 800, true, counts(0, 1, 2), 30);
    let with = TaskbarLayout::compute(
        &m,
        &shape,
        800,
        true,
        TaskbarCounts {
            chevron: true,
            ..counts(0, 1, 2)
        },
        30,
    );
    assert_eq!(bare.chevron, None);
    assert_eq!(with.tray, bare.tray);
    let c = with.chevron.unwrap();
    assert_eq!(c, Rect::new(bare.tray[0].x - 20, bare.tray[0].y, 16, 16));
    assert_eq!(with.notification.x, c.x - 4);
    assert_eq!(bare.tasks_limit() - with.tasks_limit(), 20);
    assert_eq!(with.hit(c.x + 8, c.y + 8), TaskbarHit::Chevron);
    assert_eq!(with.hit(with.tray[0].x + 3, 15), TaskbarHit::Tray(0));
    assert_eq!(with.scaled(2).chevron, Some(c.scaled(2)));
}

#[test]
fn with_every_item_hidden_the_chevron_sits_before_the_clock() {
    let m = Metrics::default();
    let counts = TaskbarCounts {
        chevron: true,
        ..counts(0, 1, 0)
    };
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts, 30);
    let c = l.chevron.unwrap();
    assert!(l.tray.is_empty());
    assert_eq!(c.right(), l.clock.x - 6);
    assert_eq!(l.tasks_limit(), c.x - 4);
}

#[test]
fn the_chevron_is_a_centred_up_triangle() {
    let mut c = Canvas::new(16, 16);
    let ink = Color::rgb(255, 255, 255);
    draw_chevron(&mut c, Rect::new(0, 0, 16, 16), ink, 1);
    let row = |y: i32| {
        (0..16)
            .filter(|&x| c.get(x, y) == ink.argb())
            .collect::<Vec<_>>()
    };
    assert!(row(5).is_empty());
    assert_eq!(row(6), vec![8]);
    assert_eq!(row(7), vec![7, 8, 9]);
    assert_eq!(row(8), (6..=10).collect::<Vec<_>>());
    assert_eq!(row(9), (5..=11).collect::<Vec<_>>());
    assert!(row(10).is_empty());
}

#[test]
fn the_chevron_is_a_button_only_when_laid_out() {
    let m = Metrics::default();
    let names = TaskbarNames {
        hidden_open: true,
        ..TaskbarNames::default()
    };
    let tree = |chevron| {
        let counts = TaskbarCounts {
            chevron,
            ..counts(0, 0, 0)
        };
        let shape = TaskbarShape {
            tray_arrow: None,
            ..TaskbarShape::CLASSIC
        };
        let l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        t
    };
    let t = tree(true);
    let n = t.find(Role::Button, "Show hidden icons").unwrap();
    assert_eq!(t.get(n).unwrap().is_expanded(), Some(true));
    assert_eq!(t.get(n).unwrap().has_popup(), Some(HasPopup::Dialog));
    assert!(
        tree(false)
            .find(Role::Button, "Show hidden icons")
            .is_none()
    );
}

#[test]
fn an_open_run_lays_the_hidden_cells_between_the_arrow_and_the_icons() {
    let m = Metrics::default();
    let lay = |reveal| {
        TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            arrow_counts(2, 3, reveal),
            30,
        )
    };
    let shut = lay(0);
    let arrow = shut.chevron.unwrap();
    assert_eq!((shut.tray.len(), shut.tray_hidden), (2, 0));
    assert_eq!(reveal_width(&TaskbarShape::CLASSIC, 3), 60);
    let open = lay(60);
    assert_eq!(
        open.chevron.unwrap().x,
        arrow.x - 60,
        "the arrow leads the run"
    );
    assert_eq!(open.tray_hidden, 3);
    let first = arrow.right() + 1;
    assert_eq!(first, shut.tray[0].x);
    let xs: Vec<i32> = open.tray.iter().map(|r| r.x).collect();
    assert_eq!(
        xs,
        vec![first - 60, first - 40, first - 20, first, first + 20]
    );
    assert_eq!(
        open.notification.x,
        shut.notification.x - 60,
        "the well widens"
    );
    assert_eq!(
        open.tasks_end,
        shut.tasks_end - 60,
        "the task row gives way"
    );
}

#[test]
fn a_part_open_run_shows_only_whole_cells_and_moves_the_arrow_by_the_pixel() {
    let m = Metrics::default();
    let shut = TaskbarLayout::compute(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        true,
        arrow_counts(2, 3, 0),
        30,
    );
    let half = TaskbarLayout::compute(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        true,
        arrow_counts(2, 3, 30),
        30,
    );
    let arrow = shut.chevron.unwrap().x;
    assert_eq!(half.chevron.unwrap().x, arrow - 30);
    assert_eq!(half.tray_hidden, 1, "one whole cell in 30 px");
    assert_eq!(
        half.tray[0].x,
        shut.tray[0].x - 20,
        "the hidden cell nearest the icons comes out first"
    );
}

#[test]
fn an_open_run_never_reaches_the_clock_on_a_narrow_bar() {
    let m = Metrics::default();
    let lay = |reveal| {
        TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            240,
            true,
            arrow_counts(6, 8, reveal),
            30,
        )
    };
    let l = lay(160);
    assert!(l.chevron.unwrap().x >= l.band_x);
    for r in &l.tray {
        assert!(
            r.right() <= l.clock.x - 6,
            "cell at {} ends 6 px before the clock",
            r.x
        );
    }
    // Six shown cells already fill the row: the run of eight opens
    // none, and the row stays as it is shut.
    assert_eq!(l.tray_hidden, 0, "no room for a hidden cell");
    let shut = lay(0);
    assert_eq!((l.chevron, &l.tray), (shut.chevron, &shut.tray));
}

#[test]
fn only_classic_and_blue_have_a_tray_arrow() {
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    assert!(TaskbarShape::CLASSIC.tray_arrow.is_some());
    assert!(
        theme_for(Interface::Blue)
            .taskbar_shape()
            .tray_arrow
            .is_some()
    );
    assert!(
        theme_for(Interface::Glass)
            .taskbar_shape()
            .tray_arrow
            .is_none()
    );
}

#[test]
fn a_shape_without_an_arrow_ignores_the_reveal() {
    let m = Metrics::default();
    let mut shape = TaskbarShape::CLASSIC;
    shape.tray_arrow = None;
    let a = TaskbarLayout::compute(&m, &shape, 800, true, arrow_counts(2, 3, 0), 30);
    let b = TaskbarLayout::compute(&m, &shape, 800, true, arrow_counts(2, 3, 60), 30);
    assert_eq!(
        (a.chevron, a.tray.clone(), b.tray_hidden),
        (b.chevron, b.tray.clone(), 0)
    );
}

#[test]
fn a_classic_tray_arrow_is_a_cell_flush_in_the_well_1_px_before_the_first_icon() {
    let m = Metrics::default();
    let lay = |reveal| {
        TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            arrow_counts(2, 3, reveal),
            30,
        )
    };
    let (shut, open) = (lay(0), lay(60));
    let icon = shut.tray[0];
    let a = shut.chevron.unwrap();
    assert_eq!(a, Rect::new(icon.x - 17, icon.y - 1, 16, 18));
    assert_eq!(
        (shut.notification.x, shut.notification.y),
        (a.x - 1, a.y - 2),
        "the cell inside the well's frame: flush left, 1 px clear above"
    );
    assert_eq!(shut.notification.bottom(), a.bottom() + 2, "and below");
    let b = open.chevron.unwrap();
    assert_eq!(b, Rect::new(a.x - 60, a.y, 16, 18), "out by the whole run");
    assert_eq!(
        b.right() + 1,
        open.tray[0].x,
        "the first hidden icon 1 px after it"
    );
    assert_eq!(open.tray[3..], shut.tray[..], "the shown icons stay");
    assert_eq!(open.notification.x, b.x - 1);
}

#[test]
fn a_blue_tray_arrow_is_a_19_px_button_across_the_wells_left_edge() {
    use crate::interface::Interface;
    let blue = crate::interfaces::theme_for(Interface::Blue);
    let shape = blue.taskbar_shape();
    assert_eq!(reveal_width(&shape, 3), 54, "18 px a cell");
    let lay = |reveal| {
        TaskbarLayout::compute(
            &blue.metrics,
            &shape,
            800,
            true,
            arrow_counts(2, 3, reveal),
            30,
        )
    };
    let (shut, open) = (lay(0), lay(54));
    let icon = shut.tray[0];
    let a = shut.chevron.unwrap();
    assert_eq!(a, Rect::new(icon.x - 20, icon.y - 2, 19, 19));
    assert_eq!(
        shut.notification.x,
        a.x + 9,
        "the well's left edge on the button's middle column"
    );
    assert_eq!(shut.notification.x, icon.x - 11, "11 px before the icon");
    let b = open.chevron.unwrap();
    assert_eq!(b, Rect::new(a.x - 54, a.y, 19, 19));
    assert_eq!(b.right() + 1, open.tray[0].x);
    assert_eq!(open.tray[3..], shut.tray[..], "the shown icons stay");
    assert_eq!(open.notification.x, b.x + 9);
}

#[test]
fn a_classic_side_tray_arrow_spans_the_wells_top() {
    let shut = classic_column(Edge::Left, 60, arrow_counts(2, 3, 0), true);
    let open = classic_column(Edge::Left, 60, arrow_counts(2, 3, 1), true);
    for l in [&shut, &open] {
        let (a, well) = (l.chevron.unwrap(), l.notification);
        assert_eq!(
            a,
            Rect::new(well.x + 2, well.y + 1, well.width - 4, 16),
            "16 tall, flush under the well's frame, 1 px clear of its sides"
        );
        assert_eq!(l.tray[0].y, a.y + 17, "the first icon row 1 px under it");
    }
    assert_eq!(open.tray_hidden, 3);
    assert_eq!(
        open.tray[0].x, shut.tray[0].x,
        "the hidden cells lead the grid"
    );
}

#[test]
fn a_blue_side_tray_arrow_is_centred_on_the_wells_top_edge() {
    use crate::interface::Interface;
    let blue = crate::interfaces::theme_for(Interface::Blue);
    let lay = |edge, thickness| {
        let span = BarSpan {
            edge,
            length: 700,
            thickness,
        };
        TaskbarLayout::compute_on(
            &blue.metrics,
            &blue.taskbar_shape(),
            span,
            true,
            arrow_counts(2, 3, 0),
            ClockText::default(),
            Bands::default(),
        )
    };
    for (edge, t, x) in [
        (Edge::Left, 104, 41),
        (Edge::Left, 80, 29),
        (Edge::Right, 104, 44),
    ] {
        let l = lay(edge, t);
        let a = l.chevron.unwrap();
        assert_eq!(
            (a.x, a.width, a.height),
            (x, 19, 19),
            "{edge:?} at {t} px: centred across"
        );
        assert_eq!(
            l.notification.y,
            a.y + 8,
            "its middle row 1 px under the well's top edge"
        );
        assert_eq!(l.tray[0].y, a.y + 21, "the first icon row 2 px under it");
    }
}

#[test]
fn blue_row_tray_icons_sit_8_px_down_a_30_px_bar_and_the_arrow_6() {
    use crate::interface::Interface;
    let blue = crate::interfaces::theme_for(Interface::Blue);
    assert_eq!(blue.metrics.taskbar_height, 30);
    let l = TaskbarLayout::compute(
        &blue.metrics,
        &blue.taskbar_shape(),
        800,
        true,
        arrow_counts(2, 3, 0),
        30,
    );
    assert!(l.tray.iter().all(|r| r.y == 8), "{:?}", l.tray);
    assert_eq!(l.chevron.unwrap().y, 6);
}

#[test]
fn classic_side_tray_icons_start_4_px_inside_the_wells_frame() {
    let c = |hidden, reveal| TaskbarCounts {
        tray: 6,
        ..arrow_counts(6, hidden, reveal)
    };
    for (edge, frame) in [(Edge::Left, 2), (Edge::Right, 4)] {
        for counts in [c(0, 0), c(3, 1)] {
            let l = classic_column(edge, 104, counts, true);
            let well = l.notification;
            assert_eq!(well.x, frame, "{edge:?}: the well's frame column");
            assert_eq!(l.tray[0].x, frame + 5, "{edge:?}: 4 px inside the frame");
            let xs: Vec<i32> = l.tray.iter().map(|r| r.x).collect();
            assert_eq!(xs[..4], [0, 20, 40, 60].map(|d| frame + 5 + d), "{edge:?}");
            assert_eq!(xs[4], frame + 5, "{edge:?}: the fifth wraps");
            assert!(l.tray.iter().all(|r| r.right() <= well.right() - 5));
        }
    }
}

#[test]
fn with_no_shown_icon_an_open_run_ends_where_a_shown_icon_would() {
    use crate::interface::Interface;
    let blue = crate::interfaces::theme_for(Interface::Blue);
    let looks = [
        (Metrics::default(), TaskbarShape::CLASSIC),
        (blue.metrics.clone(), blue.taskbar_shape()),
    ];
    for (m, shape) in looks {
        let lay = |tray, hidden, reveal| {
            TaskbarLayout::compute(
                &m,
                &shape,
                800,
                true,
                arrow_counts(tray, hidden, reveal),
                30,
            )
        };
        let full = reveal_width(&shape, 2);
        let (shut, open) = (lay(0, 2, 0), lay(0, 2, full));
        let one = lay(1, 0, 0);
        assert_eq!(
            shut.chevron.unwrap().right(),
            shut.clock.x - 6,
            "shut, the arrow ends 6 px before the clock"
        );
        assert_eq!(
            open.tray.last(),
            one.tray.first(),
            "the nearest hidden cell where a lone shown icon sits"
        );
        let (a, w) = (open.chevron.unwrap(), shape.tray_arrow.unwrap());
        assert_eq!(a.right() + w.gap, open.tray[0].x, "the arrow leads the run");
    }
}

fn ink_rows(c: &Canvas, r: Rect, ink: Color) -> Vec<String> {
    (r.y..r.bottom())
        .map(|y| {
            (r.x..r.right())
                .map(|x| if c.get(x, y) == ink.argb() { '#' } else { '.' })
                .collect()
        })
        .collect()
}

fn glyph_at(rows: &[&str], x: usize, y: usize, w: usize, h: usize) -> Vec<String> {
    (0..h)
        .map(|j| {
            let mut line = vec!['.'; w];
            if let Some(row) = j.checked_sub(y).and_then(|k| rows.get(k)) {
                for (i, ch) in row.chars().enumerate() {
                    line[x + i] = ch;
                }
            }
            line.into_iter().collect()
        })
        .collect()
}

#[test]
fn the_tray_arrow_points_back_while_shut_and_on_while_open() {
    let ink = Color::rgb(0, 0, 0);
    let r = Rect::new(0, 0, 16, 18);
    for (open, rows) in [
        (
            false,
            ["..##..##", ".##..##.", "##..##..", ".##..##.", "..##..##"],
        ),
        (
            true,
            ["##..##..", ".##..##.", "..##..##", ".##..##.", "##..##.."],
        ),
    ] {
        let mut c = Canvas::new(16, 18);
        draw_tray_arrow(&mut c, r, ink, 1, open);
        assert_eq!(
            ink_rows(&c, r, ink),
            glyph_at(&rows, 4, 7, 16, 18),
            "open {open}: 8x5 at (4, 7)"
        );
    }
}

#[test]
fn the_side_tray_arrow_points_up_while_shut_and_down_while_open() {
    let ink = Color::rgb(0, 0, 0);
    let r = Rect::new(0, 0, 95, 16);
    let up = ["..#..", ".###.", "##.##", "#...#"];
    let down = ["#...#", "##.##", ".###.", "..#.."];
    for (open, rows, y) in [(false, up, 2), (true, down, 6)] {
        let mut c = Canvas::new(95, 16);
        draw_tray_arrow_side(&mut c, r, ink, 1, open);
        let twice: Vec<&str> = rows.iter().chain(&rows).copied().collect();
        assert_eq!(
            ink_rows(&c, r, ink),
            glyph_at(&twice, 45, y, 95, 16),
            "open {open}: 5x8 centred across, {y} px down"
        );
    }
}

#[test]
fn the_tray_arrows_double_at_scale_two() {
    let ink = Color::rgb(0, 0, 0);
    type Draw = fn(&mut Canvas, Rect, Color, i32, bool);
    let glyphs: [(Draw, i32, i32); 2] = [(draw_tray_arrow, 16, 18), (draw_tray_arrow_side, 95, 16)];
    for (draw, w, h) in glyphs {
        for open in [false, true] {
            let mut one = Canvas::new(w as u32, h as u32);
            draw(&mut one, Rect::new(0, 0, w, h), ink, 1, open);
            let mut two = Canvas::new(2 * w as u32, 2 * h as u32);
            draw(&mut two, Rect::new(0, 0, 2 * w, 2 * h), ink, 2, open);
            for y in 0..2 * h {
                for x in 0..2 * w {
                    assert_eq!(two.get(x, y), one.get(x / 2, y / 2), "({x}, {y})");
                }
            }
        }
    }
}

#[test]
fn the_tray_arrow_is_a_button_reading_expanded_while_open() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        true,
        arrow_counts(1, 2, 0),
        30,
    );
    for (open, name) in [(false, "Show hidden icons"), (true, "Hide")] {
        let names = TaskbarNames {
            hidden_open: open,
            ..TaskbarNames::default()
        };
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        let n = t.find(Role::Button, name).expect(name);
        let node = t.get(n).unwrap();
        assert_eq!(node.is_expanded(), Some(open), "{name}");
        assert_eq!(node.has_popup(), None, "{name}: it opens no popup");
    }
}

#[test]
fn a_column_too_short_for_its_tray_keeps_the_arrow_and_cells_off_the_clock() {
    use crate::interface::Interface;
    for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for len in (40..240).step_by(4) {
            for (tray, hidden, reveal) in [(12, 3, 0), (12, 3, 1), (2, 3, 1), (0, 3, 1)] {
                let c = arrow_counts(tray, hidden, reveal);
                let l = look_column(
                    interface,
                    Edge::Left,
                    (60, len),
                    c,
                    ClockText::default(),
                    Bands::default(),
                );
                for r in l.tray.iter().chain(&l.chevron) {
                    assert!(
                        r.bottom() <= l.clock.y - 6,
                        "{interface:?} {len} px long, {tray} shown, {hidden} hidden, \
                         reveal {reveal}: {r:?} ends 6 px or more above the clock {:?}",
                        l.clock
                    );
                }
            }
        }
    }
    let l = look_column(
        Interface::Classic,
        Edge::Left,
        (60, 60),
        arrow_counts(2, 3, 0),
        ClockText::default(),
        Bands::default(),
    );
    assert_eq!(l.chevron, None, "no room for the arrow: it is left out");
}

#[test]
fn an_open_run_never_pushes_the_shown_cells_off_a_row() {
    let m = Metrics::default();
    let s = TaskbarShape::CLASSIC;
    let lay = |reveal| TaskbarLayout::compute(&m, &s, 400, true, arrow_counts(4, 30, reveal), 30);
    let l = lay(reveal_width(&s, 30));
    let shut = lay(0);
    assert_eq!(
        l.tray.len() - l.tray_hidden,
        4,
        "every shown cell stays on the bar"
    );
    assert_eq!(
        l.tray[l.tray_hidden..],
        shut.tray[..],
        "the shown cells stay put"
    );
    let arrow = l.chevron.unwrap();
    assert!(
        arrow.x >= l.band_x,
        "the arrow {arrow:?} stops at the bands' start {}",
        l.band_x
    );
    assert!(
        arrow.x < l.band_x + 20,
        "the run fills the room up to the bands' start: {arrow:?}"
    );
    assert!(l.tray_hidden < 30);
}

#[test]
fn a_capped_run_opens_fully_at_its_run_width() {
    let m = Metrics::default();
    let s = TaskbarShape::CLASSIC;
    let lay = |reveal| TaskbarLayout::compute(&m, &s, 400, true, arrow_counts(4, 30, reveal), 30);
    let full = lay(reveal_width(&s, 30));
    let run = lay(0).run_width;
    assert!(run > 0 && run < reveal_width(&s, 30), "capped: {run}");
    assert_eq!(full.run_width, run, "the cap does not move with the run");
    assert_eq!(run, reveal_width(&s, full.tray_hidden));
    let at = lay(run);
    assert_eq!((at.chevron, &at.tray), (full.chevron, &full.tray));
    assert_eq!(lay(run - 1).tray_hidden, full.tray_hidden - 1);
}

fn tools_and_arrow(hidden: usize, reveal: i32) -> TaskbarCounts {
    TaskbarCounts {
        quick: 3,
        pager: 4,
        tasks: 2,
        ..arrow_counts(4, hidden, reveal)
    }
}

/// The tools band's rects end before `arrow` on the bar's long axis.
fn clear_of_tools(l: &TaskbarLayout, arrow: Rect, side: bool) -> bool {
    (l.quick.iter().chain(&l.pager)).all(|r| {
        if side {
            r.bottom() <= arrow.y
        } else {
            r.right() <= arrow.x
        }
    })
}

#[test]
fn a_capped_run_leaves_its_arrow_clear_of_the_tools_band() {
    let m = Metrics::default();
    let s = TaskbarShape::CLASSIC;
    for locked in [true, false] {
        for tasks_first in [false, true] {
            let bands = Bands {
                tasks_first,
                first_width: None,
            };
            let lay = |reveal| {
                let span = BarSpan {
                    edge: Edge::Bottom,
                    length: 800,
                    thickness: m.taskbar_height,
                };
                let clock = ClockText { time: 30, date: 30 };
                TaskbarLayout::compute_on(
                    &m,
                    &s,
                    span,
                    locked,
                    tools_and_arrow(60, reveal),
                    clock,
                    bands,
                )
            };
            let open = lay(reveal_width(&s, 60));
            let arrow = open.chevron.unwrap();
            assert!(open.tray_hidden > 0, "the run still opens");
            assert!(
                clear_of_tools(&open, arrow, false),
                "locked {locked}, tasks first {tasks_first}: arrow {arrow:?} under {:?} {:?}",
                open.quick,
                open.pager
            );
            assert_eq!(open.run_width, reveal_width(&s, open.tray_hidden));
        }
    }
}

#[test]
fn a_capped_column_run_leaves_its_arrow_clear_of_the_tools_band() {
    let open = classic_column(Edge::Left, 60, tools_and_arrow(60, 1), true);
    let arrow = open.chevron.unwrap();
    assert!(open.tray_hidden > 0, "the run still opens");
    assert!(
        clear_of_tools(&open, arrow, true),
        "arrow {arrow:?} under {:?} {:?}",
        open.quick,
        open.pager
    );
}

#[test]
fn a_shut_columns_run_width_is_what_its_open_run_shows() {
    let shut = classic_column(Edge::Left, 60, arrow_counts(4, 60, 0), true);
    let open = classic_column(Edge::Left, 60, arrow_counts(4, 60, 1), true);
    assert_eq!(shut.tray_hidden, 0);
    assert_eq!(
        shut.run_width,
        reveal_width(&TaskbarShape::CLASSIC, open.tray_hidden)
    );
    assert!(open.tray_hidden > 0 && open.tray_hidden < 60);
}

#[test]
fn an_open_run_never_pushes_the_shown_cells_off_a_column() {
    let l = classic_column(Edge::Left, 60, arrow_counts(4, 60, 1), true);
    let shut = classic_column(Edge::Left, 60, arrow_counts(4, 60, 0), true);
    assert!(l.tray_hidden > 0 && l.tray_hidden < 60);
    assert_eq!(
        l.tray.len() - l.tray_hidden,
        4,
        "every shown cell stays on the bar"
    );
    let last = |l: &TaskbarLayout| *l.tray.last().unwrap();
    assert!(last(&l).bottom() <= l.clock.y - 6);
    assert_eq!(last(&l).y, last(&shut).y, "the grid still ends at the foot");
}

#[test]
fn revealed_counts_the_whole_cells_inside_the_run() {
    let s = TaskbarShape::CLASSIC;
    assert_eq!(revealed(&s, 3, -5), 0, "a negative run shows none");
    assert_eq!(revealed(&s, 3, 19), 0, "a cell shows once it is whole");
    assert_eq!(revealed(&s, 3, 20), 1);
    assert_eq!(revealed(&s, 3, 59), 2);
    assert_eq!(revealed(&s, 3, 60), 3);
    assert_eq!(revealed(&s, 3, 500), 3, "never more than are hidden");
    assert_eq!(revealed(&s, 0, 60), 0);
}
