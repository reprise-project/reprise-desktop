use super::*;

#[test]
fn a_side_bars_reveal_line_is_its_inner_two_columns() {
    let row = classic_row(1);
    let left = TaskbarLayout {
        edge: Edge::Left,
        width: 60,
        height: 600,
        ..row.clone()
    };
    assert_eq!(
        left.reveal_line(),
        Rect::new(58, 0, 2, 600),
        "a Left bar's right columns"
    );
    let right = TaskbarLayout {
        edge: Edge::Right,
        ..left.clone()
    };
    assert_eq!(
        right.reveal_line(),
        Rect::new(0, 0, 2, 600),
        "a Right bar's left columns"
    );
    assert_eq!((left.length(), left.thickness()), (600, 60));
}

#[test]
fn a_side_bar_is_starts_width_or_one_slot_by_default_and_clamped() {
    use crate::interface::Interface;
    let m = Metrics::default();
    let c = TaskbarShape::CLASSIC;
    assert_eq!((c.side_slot(&m), c.side_default(&m)), (28, 60), "Classic");
    let blue = crate::interfaces::theme_for(Interface::Blue);
    let b = blue.taskbar_shape();
    assert_eq!(
        (b.side_slot(&blue.metrics), b.side_default(&blue.metrics)),
        (30, blue.metrics.start_button_width),
        "Blue: Start fills its bar's height, so its width is the column's"
    );
    let glass = crate::interfaces::theme_for(Interface::Glass);
    let g = glass.taskbar_shape();
    assert_eq!(
        (g.side_slot(&glass.metrics), g.side_default(&glass.metrics)),
        (60, 60),
        "Glass: one 60 px slot"
    );
    let small_m = Metrics {
        taskbar_height: 30,
        ..glass.metrics.clone()
    };
    let small = crate::glass::small_bar_style(&Default::default()).taskbar;
    assert_eq!(
        (small.side_slot(&small_m), small.side_default(&small_m)),
        (44, 44),
        "Glass's small bar: one 44 px slot"
    );
    assert_eq!(c.side_width(&m, 0, 1920), 60, "0: the default");
    assert_eq!(c.side_width(&m, 10, 1920), 28, "never under a slot");
    assert_eq!(c.side_width(&m, 200, 1920), 200);
    assert_eq!(
        c.side_width(&m, 2000, 1920),
        640,
        "never past a third of the output"
    );
    assert_eq!(
        c.side_width(&m, 0, 60),
        28,
        "an output too narrow for the default: a slot"
    );
}

#[test]
fn a_left_classic_column_stacks_start_its_bands_tray_clock_and_strip() {
    let c = TaskbarCounts {
        quick: 3,
        pager: 2,
        tasks: 2,
        tray: 2,
        show_desktop: true,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let l = classic_column(Edge::Left, 60, c, true);
    assert_eq!((l.width, l.height), (60, 600), "a column 60 wide");
    assert_eq!(
        l.start,
        Rect::new(2, 2, 54, 22),
        "Start 2 px from the screen's edge and its top, 4 from the inner edge"
    );
    assert_eq!(
        l.pager,
        vec![Rect::new(2, 27, 23, 23), Rect::new(25, 27, 23, 23)],
        "the pager's own row of 23 px cells, 3 px under Start"
    );
    assert_eq!(
        l.quick,
        vec![
            Rect::new(2, 54, 23, 23),
            Rect::new(25, 54, 23, 23),
            Rect::new(2, 77, 23, 23),
        ],
        "two cells a row, 4 px under the pager"
    );
    assert_eq!(
        l.tasks,
        vec![Rect::new(2, 104, 54, 22), Rect::new(2, 129, 54, 22)],
        "one 22 px button a row, 3 px apart, labelled"
    );
    assert!(!l.icons_only);
    assert_eq!(
        l.chevron,
        Some(Rect::new(4, 513, 50, 16)),
        "the arrow across the well's top"
    );
    assert_eq!(
        l.tray,
        vec![Rect::new(7, 530, 16, 16), Rect::new(27, 530, 16, 16)],
        "the icons under it, two a row, 4 px inside the well's frame"
    );
    assert_eq!(l.clock, Rect::new(2, 552, 54, 22));
    assert_eq!(l.clock_lines, None, "no room across for the date");
    assert_eq!(l.notification, Rect::new(2, 512, 54, 62));
    assert_eq!(
        l.show_desktop,
        Some(Rect::new(2, 576, 54, 22)),
        "last, 2 px off the foot"
    );
    assert_eq!((l.tasks_x(), l.tasks_limit()), (104, 509));
    assert_eq!(l.more, None);
    let r = classic_column(Edge::Right, 60, c, true);
    assert_eq!(r.start, Rect::new(4, 2, 54, 22), "against the right edge");
    assert_eq!(r.quick[1], Rect::new(27, 54, 23, 23));
    assert_eq!(r.tasks[0], Rect::new(4, 104, 54, 22));
    assert_eq!(r.chevron, Some(Rect::new(6, 513, 50, 16)));
}

#[test]
fn along_down_a_side_bar_is_along_y() {
    let l = classic_column(Edge::Left, 60, counts(0, 2, 0), true);
    let r = l.tasks[1];
    assert_eq!((l.length(), l.thickness()), (600, 60));
    assert_eq!(l.along(r.x + 5, r.y + 3), r.y + 3);
    assert_eq!(l.along_f(12.0, 300.5), 300.5);
    assert_eq!(l.quick_drag(5.0, -2.0), 5.0, "either way over the grid");
    assert_eq!(
        l.moved_along(r, 7),
        Rect::new(r.x, r.y + 7, r.width, r.height)
    );
    assert_eq!(
        l.placed_along(r, 300),
        Rect::new(r.x, 300, r.width, r.height)
    );
    assert_eq!(
        l.along_rects(&l.tasks)[1],
        Rect::new(r.y, r.x, r.height, r.width)
    );
}

#[test]
fn a_column_labels_its_buttons_once_it_is_wide_enough() {
    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let shape = glass.taskbar_shape();
    let lay = |m: &Metrics, s: &TaskbarShape, t: i32| {
        let span = BarSpan {
            edge: Edge::Left,
            length: 800,
            thickness: t,
        };
        TaskbarLayout::compute_on(
            m,
            s,
            span,
            true,
            counts(0, 2, 0),
            ClockText::default(),
            Bands::default(),
        )
    };
    assert!(
        lay(&glass.metrics, &shape, 60).icons_only,
        "Glass's 60 px slot: icons only"
    );
    assert!(lay(&glass.metrics, &shape, 69).icons_only);
    assert!(
        !lay(&glass.metrics, &shape, 70).icons_only,
        "70 px: 24 px of title after the icon's 46"
    );
    let m = Metrics::default();
    assert!(
        lay(&m, &TaskbarShape::CLASSIC, 59).icons_only,
        "a 53 px button row"
    );
    assert!(
        !lay(&m, &TaskbarShape::CLASSIC, 60).icons_only,
        "Classic's default: labelled"
    );
    let mut l = lay(&m, &TaskbarShape::CLASSIC, 60);
    l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::Icons);
    assert!(l.icons_only, "Combine stays icons only however wide");
    assert_eq!(l.tasks[0].height, 22, "still one button a row");
}

#[test]
fn an_open_column_adds_every_hidden_cell_to_its_grid() {
    let shut = classic_column(Edge::Left, 60, arrow_counts(5, 3, 0), true);
    let open = classic_column(Edge::Left, 60, arrow_counts(5, 3, 1), true);
    assert_eq!((shut.tray.len(), shut.tray_hidden), (5, 0));
    assert_eq!(
        (open.tray.len(), open.tray_hidden),
        (8, 3),
        "any open px shows the whole run"
    );
    // The grid grows up from the clock: its last row stays where it was.
    // Cells are numbered from the top-left, so the shown ones may change
    // column.
    assert_eq!(open.tray.last().map(|r| r.y), shut.tray.last().map(|r| r.y));
    assert!(open.chevron.unwrap().y < shut.chevron.unwrap().y);
}

#[test]
fn a_column_wraps_its_tray_into_rows_above_the_clock() {
    let c = TaskbarCounts {
        tray: 5,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let l = classic_column(Edge::Left, 60, c, true);
    assert_eq!(
        l.clock,
        Rect::new(2, 576, 54, 22),
        "no strip: 2 px off the foot"
    );
    assert_eq!(l.chevron, Some(Rect::new(4, 497, 50, 16)));
    assert_eq!(
        l.tray,
        vec![
            Rect::new(7, 514, 16, 16),
            Rect::new(27, 514, 16, 16),
            Rect::new(7, 534, 16, 16),
            Rect::new(27, 534, 16, 16),
            Rect::new(7, 554, 16, 16),
        ],
        "two a row, the last row 6 px over the clock"
    );
    assert_eq!(
        l.notification,
        Rect::new(2, 496, 54, 102),
        "from 1 px over the arrow to the clock's foot"
    );
    assert_eq!(l.tasks_limit(), 493);
}

fn glass_column(edge: Edge, t: i32, small: bool, counts: TaskbarCounts) -> TaskbarLayout {
    let mut glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    if small {
        crate::interfaces::small_bar(&mut glass, &Default::default());
    }
    let span = BarSpan {
        edge,
        length: 700,
        thickness: t,
    };
    TaskbarLayout::compute_on(
        &glass.metrics,
        &glass.taskbar_shape(),
        span,
        true,
        counts,
        ClockText::default(),
        Bands::default(),
    )
}

#[test]
fn an_orb_column_keeps_its_tray_grid_2_px_off_both_sides() {
    let c = TaskbarCounts {
        tray: 7,
        chevron: true,
        ..TaskbarCounts::default()
    };
    for (t, small, per_row) in [(60, false, 3), (44, true, 2), (140, false, 7)] {
        for edge in [Edge::Left, Edge::Right] {
            let l = glass_column(edge, t, small, c);
            let cells: Vec<Rect> = l.chevron.into_iter().chain(l.tray.clone()).collect();
            assert_eq!(cells.len(), 8, "{edge:?} at {t} px: every cell laid");
            for r in &cells {
                assert!(
                    r.x >= 2 && r.right() <= t - 2,
                    "{edge:?} at {t} px: {r:?} 2 px clear of the screen edge and the inner edge"
                );
            }
            let (x0, x1) = (cells[0].x, cells[per_row - 1].right());
            assert_eq!(cells[per_row].x, x0, "{edge:?} at {t} px: {per_row} a row");
            assert_eq!(
                x0,
                t - x1,
                "{edge:?} at {t} px: the grid centred across the column"
            );
        }
    }
}

#[test]
fn an_orb_column_centres_its_quick_launch_and_pager_grids() {
    let c = TaskbarCounts {
        quick: 1,
        pager: 4,
        ..TaskbarCounts::default()
    };
    for edge in [Edge::Left, Edge::Right] {
        let l = glass_column(edge, 60, false, c);
        assert_eq!(l.quick[0].x, 18, "{edge:?}: one 23 px cell, centred");
        let xs: Vec<i32> = l.pager.iter().map(|r| r.x).collect();
        assert_eq!(xs, [7, 30, 7, 30], "{edge:?}: two cells a row, centred");
        let wide = glass_column(edge, 140, false, c);
        let xs: Vec<i32> = wide.pager.iter().map(|r| r.x).collect();
        assert_eq!(
            xs,
            [24, 47, 70, 93],
            "{edge:?} at 140 px: the four cells on one row, centred"
        );
    }
}

#[test]
fn a_column_wraps_the_pager_and_quick_launch_into_rows_of_its_width() {
    let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let shape = blue.taskbar_shape();
    let t = shape.side_default(&blue.metrics);
    let span = BarSpan {
        edge: Edge::Left,
        length: 700,
        thickness: t,
    };
    let c = TaskbarCounts {
        quick: 5,
        pager: 6,
        tasks: 1,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute_on(
        &blue.metrics,
        &shape,
        span,
        true,
        c,
        ClockText::default(),
        Bands::default(),
    );
    assert_eq!(
        l.start,
        Rect::new(0, 0, t, 30),
        "Start across the whole column"
    );
    let xs: Vec<i32> = l.pager.iter().map(|r| r.x).collect();
    assert_eq!(
        xs,
        [2, 25, 48, 71, 2, 25],
        "four 23 px cells a row, from the left"
    );
    assert_eq!(l.pager[0].y, 37, "7 px under Start");
    assert_eq!(l.pager[4], Rect::new(2, l.pager[0].y + 23, 23, 23));
    assert_eq!(
        l.quick[0].y,
        l.pager[5].bottom() + 4,
        "Quick Launch 4 px under the pager"
    );
    assert_eq!(l.quick[4].y, l.quick[0].y + 23);
    assert_eq!(l.tasks_x(), l.quick[4].bottom() + 4);
}

#[test]
fn a_column_sets_the_date_under_the_time_once_it_fits_across() {
    let lay = |t: i32| {
        let span = BarSpan {
            edge: Edge::Right,
            length: 600,
            thickness: t,
        };
        let clock = ClockText { time: 40, date: 60 };
        TaskbarLayout::compute_on(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            span,
            true,
            counts(0, 1, 0),
            clock,
            Bands::default(),
        )
    };
    let narrow = lay(60);
    assert_eq!(
        narrow.clock_lines, None,
        "the date needs 76 px, the row has 54"
    );
    assert_eq!(narrow.clock, Rect::new(4, 576, 54, 22));
    let wide = lay(82);
    assert_eq!(wide.clock_lines, Some([17, 34]));
    assert_eq!(
        wide.clock,
        Rect::new(4, 558, 76, 40),
        "two lines take 40 rows"
    );
    assert_eq!(
        classic_row(1).clock_lines,
        None,
        "a 28 px bottom bar keeps one line"
    );
}

#[test]
fn a_blue_or_glass_column_runs_its_well_and_strip_across_the_column() {
    use crate::interface::Interface;
    let c = TaskbarCounts {
        tasks: 1,
        tray: 2,
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let clock = ClockText { time: 40, date: 0 };
    let lay = |i, edge, t| look_column(i, edge, (t, 600), c, clock, Bands::default());
    let blue = lay(Interface::Blue, Edge::Left, 100);
    assert_eq!(
        blue.show_desktop,
        Some(Rect::new(0, 575, 100, 25)),
        "Blue: a button's height across the column, last"
    );
    assert_eq!(
        blue.clock,
        Rect::new(2, 538, 95, 25),
        "12 px over the strip"
    );
    assert_eq!(
        blue.tray,
        vec![Rect::new(2, 516, 16, 16), Rect::new(20, 516, 16, 16)],
        "2 px apart, 6 px over the clock"
    );
    assert_eq!(
        blue.notification,
        Rect::new(0, 505, 100, 70),
        "from 11 px over the tray to the strip, across the column"
    );
    let right = lay(Interface::Blue, Edge::Right, 100);
    assert_eq!(right.notification, blue.notification);
    assert_eq!(right.tray[0], Rect::new(3, 516, 16, 16));
    let glass = lay(Interface::Glass, Edge::Left, 60);
    assert_eq!(
        glass.show_desktop,
        Some(Rect::new(0, 585, 60, 15)),
        "Glass: 15 px across the column, last"
    );
    assert_eq!(glass.clock, Rect::new(0, 543, 60, 40));
    assert_eq!(
        glass.tray,
        vec![Rect::new(12, 521, 16, 16), Rect::new(32, 521, 16, 16)],
        "4 px apart, centred across the column"
    );
    assert_eq!(
        glass.notification,
        Rect::new(0, 513, 60, 72),
        "from 8 px over the tray to the strip"
    );
}

#[test]
fn a_blue_or_glass_column_sets_the_date_under_the_time_once_it_fits_across() {
    use crate::interface::Interface;
    let clock = ClockText { time: 40, date: 60 };
    let lay = |i, t| {
        look_column(
            i,
            Edge::Left,
            (t, 600),
            counts(0, 1, 0),
            clock,
            Bands::default(),
        )
    };
    let blue = lay(Interface::Blue, 100);
    assert_eq!(blue.clock_lines, Some([17, 34]), "Blue: 62 px in 95");
    assert_eq!(
        blue.clock,
        Rect::new(2, 548, 95, 40),
        "two lines take 40 rows"
    );
    assert_eq!(
        lay(Interface::Blue, 60).clock_lines,
        None,
        "62 px in 55: one line"
    );
    let glass = lay(Interface::Glass, 120);
    assert_eq!(glass.clock_lines, Some([17, 34]), "Glass: 76 px in 120");
    assert_eq!(glass.clock, Rect::new(0, 558, 120, 40));
    assert_eq!(lay(Interface::Glass, 60).clock_lines, None, "76 px in 60");
}

#[test]
fn a_column_with_the_tasks_first_lays_them_above_quick_launch() {
    use crate::interface::Interface;
    let lay = |tasks, first_width| {
        let bands = Bands {
            tasks_first: true,
            first_width,
        };
        let clock = ClockText { time: 30, date: 0 };
        look_column(
            Interface::Classic,
            Edge::Left,
            (60, 600),
            counts(3, tasks, 0),
            clock,
            bands,
        )
    };
    let l = lay(2, Some(100));
    assert_eq!(l.tasks_x(), 27, "the task row first, 3 px under Start");
    assert_eq!(
        l.tasks,
        vec![Rect::new(2, 27, 54, 22), Rect::new(2, 52, 54, 22)]
    );
    assert_eq!(l.tasks_limit(), 127, "the band 100 px long");
    assert_eq!(
        l.quick,
        vec![
            Rect::new(2, 131, 23, 23),
            Rect::new(25, 131, 23, 23),
            Rect::new(2, 154, 23, 23),
        ],
        "Quick Launch 4 px after it, two cells a row"
    );
    let natural = lay(1, None);
    assert_eq!(
        (natural.tasks_x(), natural.tasks_limit()),
        (27, 49),
        "unsized, the band is one full button long"
    );
    assert_eq!(natural.quick[0].y, 53);
}

#[test]
fn a_column_a_third_of_a_1920_output_wide_labels_its_buttons_across_it() {
    use crate::interface::Interface;
    let m = Metrics::default();
    let t = TaskbarShape::CLASSIC.side_width(&m, 2000, 1920);
    assert_eq!(t, 640);
    let l = look_column(
        Interface::Classic,
        Edge::Left,
        (t, 1080),
        counts(0, 2, 3),
        ClockText { time: 40, date: 60 },
        Bands::default(),
    );
    assert_eq!((l.width, l.height), (640, 1080));
    assert_eq!(l.start, Rect::new(2, 2, 54, 22), "Start keeps its width");
    assert!(!l.icons_only);
    assert_eq!(l.tasks[0], Rect::new(2, 27, 634, 22), "across the row");
    assert_eq!(l.clock, Rect::new(2, 1038, 634, 40));
    assert!(l.clock_lines.is_some(), "the date fits across");
    assert!(
        l.tray.iter().all(|r| r.y == l.tray[0].y),
        "the tray on one row: {:?}",
        l.tray
    );
}

#[test]
fn a_side_bars_grips_lie_across_the_column() {
    let l = classic_column(Edge::Left, 60, counts(2, 1, 0), false);
    assert_eq!(
        l.grips,
        vec![Rect::new(20, 29, 18, 3), Rect::new(20, 65, 18, 3)],
        "3 px bars across the button row, 2 px under Start and 6 px after the band"
    );
    assert_eq!(l.quick[0].y, 36);
    assert_eq!(l.tasks_x(), 72);
    assert_eq!(l.hit(25, 30), TaskbarHit::Grip(0));
    assert_eq!(l.hit(25, 67), TaskbarHit::Grip(1));
    assert_eq!(
        l.drag_band(Bands::default(), Band::Tasks, 300),
        Bands {
            tasks_first: false,
            first_width: Some(300 - 6 - 36)
        },
        "dragged down the column"
    );
}

#[test]
fn a_column_drops_a_quick_launch_button_in_reading_order() {
    let l = classic_column(Edge::Left, 60, counts(3, 0, 0), true);
    assert_eq!(
        l.quick,
        vec![
            Rect::new(2, 27, 23, 23),
            Rect::new(25, 27, 23, 23),
            Rect::new(2, 50, 23, 23),
        ]
    );
    assert_eq!(l.drop_slot(10, 30), 0, "left of the first cell's middle");
    assert_eq!(l.drop_slot(20, 30), 1);
    assert_eq!(l.drop_slot(40, 30), 2, "past the second");
    assert_eq!(
        l.drop_slot(5, 55),
        2,
        "the next row, before its first cell's middle"
    );
    assert_eq!(l.drop_slot(30, 60), 3);
    assert_eq!(l.drop_slot(5, 300), 3, "under the band");
    assert!(l.on_quick_band(30, 60) && !l.on_quick_band(30, l.tasks_x()));
}

#[test]
fn a_side_glass_bar_keeps_its_orb_inside_the_column() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let small_m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let small = crate::glass::small_bar_style(&Default::default()).taskbar;
    let full = theme.taskbar_shape();
    let lay = |m: &Metrics, s: &TaskbarShape, edge| {
        let span = BarSpan {
            edge,
            length: 600,
            thickness: s.side_default(m),
        };
        TaskbarLayout::compute_on(
            m,
            s,
            span,
            true,
            counts(0, 1, 0),
            ClockText::default(),
            Bands::default(),
        )
    };
    for edge in [Edge::Left, Edge::Right] {
        let l = lay(&theme.metrics, &full, edge);
        assert_eq!(
            l.start,
            Rect::new(3, 0, 54, 40),
            "{edge:?}: the orb centred in 60 px"
        );
        assert_eq!(
            (l.start_above, l.start_disc, l.disc_past_bar()),
            (None, None, None)
        );
        let s = lay(&small_m, &small, edge);
        assert_eq!(
            s.start,
            Rect::new(0, 0, 44, 35),
            "{edge:?}: the small orb's 5 px rise inside its 44 px column"
        );
        assert_eq!(s.start_disc, Some(Rect::new(4, 0, 36, 36)));
        assert_eq!(
            s.disc_past_bar(),
            None,
            "{edge:?}: the disc stays inside the column"
        );
        assert_eq!(s.start_orb(true), None, "{edge:?}: no buffer of its own");
        assert_eq!(s.hit(22, 2), TaskbarHit::Start);
        assert_eq!(s.hit(22, -1), TaskbarHit::Nothing);
    }
}

#[test]
fn every_edges_rects_take_their_own_hits() {
    let c = TaskbarCounts {
        quick: 2,
        pager: 2,
        tasks: 2,
        tray: 2,
        show_desktop: true,
        chevron: true,
        ..TaskbarCounts::default()
    };
    for edge in Edge::ALL {
        let l = if edge.is_side() {
            classic_column(edge, 60, c, false)
        } else {
            classic_full(edge)
        };
        let mut want = vec![(l.start, TaskbarHit::Start), (l.clock, TaskbarHit::Clock)];
        want.extend(
            l.grips
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, TaskbarHit::Grip(i))),
        );
        want.extend(
            l.quick
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, TaskbarHit::Quick(i))),
        );
        want.extend(
            l.pager
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, TaskbarHit::Pager(i))),
        );
        want.extend(
            l.tasks
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, TaskbarHit::Task(i))),
        );
        want.extend(
            l.tray
                .iter()
                .enumerate()
                .map(|(i, r)| (*r, TaskbarHit::Tray(i))),
        );
        want.extend(l.chevron.map(|r| (r, TaskbarHit::Chevron)));
        want.extend(l.show_desktop.map(|r| (r, TaskbarHit::ShowDesktop)));
        for (r, hit) in want {
            let corners = [
                (r.x, r.y),
                (r.x + r.width / 2, r.y + r.height / 2),
                (r.right() - 1, r.bottom() - 1),
            ];
            for (x, y) in corners {
                assert_eq!(l.hit(x, y), hit, "{edge:?}: ({x}, {y}) in {r:?}");
            }
        }
        assert_eq!(l.hit(-1, 5), TaskbarHit::Nothing, "{edge:?}: off the bar");
    }
}

#[test]
fn past_what_fits_a_column_shrinks_its_buttons_then_hides_the_rest_behind_more() {
    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let (m, shape) = (&glass.metrics, glass.taskbar_shape());
    let lay = |n: usize| {
        let span = BarSpan {
            edge: Edge::Left,
            length: 400,
            thickness: 60,
        };
        TaskbarLayout::compute_on(
            m,
            &shape,
            span,
            true,
            counts(0, n, 0),
            ClockText::default(),
            Bands::default(),
        )
    };
    let seven = lay(7);
    assert_eq!((seven.tasks_x(), seven.tasks_limit()), (45, 352));
    assert_eq!(seven.tasks.len(), 7);
    assert!(seven.tasks.iter().all(|r| r.width == 60 && r.height == 40));
    assert_eq!(seven.tasks[1].y, 87);
    assert_eq!(seven.more, None);
    let eight = lay(8);
    assert_eq!(eight.more, Some(Rect::new(0, 314, 60, 38)));
    assert_eq!((eight.tasks.len(), eight.stacks.len()), (6, 6));
    assert!(eight.tasks.iter().all(|r| r.height == 38));
    assert_eq!(eight.tasks[5], Rect::new(0, 245, 60, 38));
    assert_eq!(eight.hit(30, 330), TaskbarHit::More);
    assert_eq!(eight.scaled(2).more, Some(Rect::new(0, 628, 120, 76)));
    assert_eq!(
        eight.least_button(62),
        40,
        "a full button decides grouping on a side bar"
    );
    assert_eq!(
        classic_row(1).least_button(62),
        62,
        "the row's own least on a bottom bar"
    );
    assert_eq!(
        eight.icon_step(&shape),
        (40, 42),
        "an icon button is a full row"
    );
}

#[test]
fn a_column_too_short_for_its_tray_lays_no_buttons_and_no_more() {
    let span = BarSpan {
        edge: Edge::Left,
        length: 120,
        thickness: 60,
    };
    let c = TaskbarCounts {
        tray: 12,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute_on(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        span,
        true,
        c,
        ClockText::default(),
        Bands::default(),
    );
    assert!(
        l.tasks_limit() < l.tasks_x(),
        "the tray reaches past the bands"
    );
    assert_eq!((l.tasks.len(), l.more), (0, None));
    let mut l = l;
    l.lay_tasks(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        &[1, 1],
        TaskRow::Shared,
    );
    assert_eq!((l.tasks.len(), l.more), (0, None), "no room for a » either");
}

#[test]
fn a_column_too_short_for_its_clock_has_an_empty_well() {
    let c = TaskbarCounts {
        show_desktop: true,
        ..TaskbarCounts::default()
    };
    let l = look_column(
        crate::interface::Interface::Glass,
        Edge::Left,
        (60, 50),
        c,
        ClockText::default(),
        Bands::default(),
    );
    assert!(
        l.clock.y > l.show_desktop.unwrap().y,
        "the clock pushed past the strip"
    );
    assert_eq!(
        l.notification,
        Rect::new(0, l.clock.y, 60, 0),
        "no rows, not a negative height"
    );
}

#[test]
fn the_more_cell_is_two_centred_right_chevrons() {
    let mut c = Canvas::new(60, 22);
    let ink = Color::rgb(255, 255, 255);
    draw_more(&mut c, Rect::new(0, 0, 60, 22), ink, 1);
    let col = |x: i32| {
        (0..22)
            .filter(|&y| c.get(x, y) == ink.argb())
            .collect::<Vec<_>>()
    };
    assert_eq!(col(25), (7..14).collect::<Vec<_>>());
    assert_eq!(col(28), vec![10]);
    assert!(
        col(29).is_empty() && col(30).is_empty(),
        "2 px between them"
    );
    assert_eq!(col(31), (7..14).collect::<Vec<_>>());
    assert_eq!(col(34), vec![10]);
    assert!(col(35).is_empty());
}

fn left_side_bar() -> TaskbarLayout {
    let span = BarSpan {
        edge: Edge::Left,
        length: 600,
        thickness: 60,
    };
    TaskbarLayout::compute_on(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        span,
        true,
        TaskbarCounts::default(),
        ClockText::default(),
        Bands::default(),
    )
}

#[test]
#[should_panic(expected = "compute_on lays a side bar out")]
fn turning_a_side_bar_to_the_other_side_panics() {
    left_side_bar().on_edge(Edge::Right);
}

#[test]
#[should_panic(expected = "compute_on lays a side bar out")]
fn turning_a_side_bar_to_the_bottom_panics() {
    left_side_bar().on_edge(Edge::Bottom);
}

#[test]
fn beside_a_right_bar_a_start_menu_on_its_edge_casts_no_shadow() {
    assert_eq!(
        StartMenuDock::OnTop.shadow_floor(Edge::Right),
        Some(0),
        "its shadow would fall on the bar"
    );
    assert_eq!(
        StartMenuDock::OnTop.shadow_floor(Edge::Left),
        None,
        "it falls away from a left bar"
    );
    assert_eq!(
        StartMenuDock::Into(4).shadow_floor(Edge::Right),
        None,
        "a menu into the bar casts its shadow on it"
    );
}

#[test]
fn a_side_bars_toolbars_are_vertical_and_its_more_cell_opens_a_menu() {
    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let shape = glass.taskbar_shape();
    let span = BarSpan {
        edge: Edge::Left,
        length: 400,
        thickness: 60,
    };
    let c = TaskbarCounts {
        quick: 2,
        tasks: 9,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute_on(
        &glass.metrics,
        &shape,
        span,
        true,
        c,
        ClockText::default(),
        Bands::default(),
    );
    assert!(l.more.is_some(), "nine 40 px buttons overrun the column");
    let names = TaskbarNames {
        quick: vec!["A".into(), "B".into()],
        tasks: (0..9).map(|i| (format!("T{i}"), false)).collect(),
        more_open: true,
        ..TaskbarNames::default()
    };
    let mut t = Tree::window("Taskbar", (l.width, l.height));
    access(&l, &names, &mut t, access::ROOT);
    let quick = t.find(Role::Toolbar, "Quick Launch").unwrap();
    assert_eq!(
        t.get(quick).unwrap().orientation(),
        Some(Orientation::Vertical)
    );
    let tasks = t.find(Role::Toolbar, "Running Applications").unwrap();
    assert_eq!(
        t.get(tasks).unwrap().orientation(),
        Some(Orientation::Vertical)
    );
    let more = t.find(Role::Button, "Show more windows").unwrap();
    assert_eq!(t.get(more).unwrap().has_popup(), Some(HasPopup::Menu));
    assert_eq!(
        t.get(more).unwrap().is_expanded(),
        Some(true),
        "its menu is open"
    );
    let row = classic_full(Edge::Bottom);
    let mut b = Tree::window("Taskbar", (row.width, row.height));
    let names = TaskbarNames {
        quick: vec!["A".into(), "B".into()],
        ..TaskbarNames::default()
    };
    access(&row, &names, &mut b, access::ROOT);
    let q = b.find(Role::Toolbar, "Quick Launch").unwrap();
    assert_eq!(
        b.get(q).unwrap().orientation(),
        None,
        "a bottom bar's toolbars stay as they were"
    );
    assert!(
        b.find(Role::Button, "Show more windows").is_none(),
        "a bottom bar has no » cell"
    );
}

#[test]
fn an_unlocked_side_bar_takes_width_drags_on_its_inner_edge_over_its_buttons() {
    let c = TaskbarCounts {
        quick: 2,
        pager: 0,
        tasks: 3,
        tray: 2,
        show_desktop: true,
        chevron: false,
        ..TaskbarCounts::default()
    };
    let left = classic_column(Edge::Left, 60, c, false);
    assert_eq!(
        left.resize,
        Some(Rect::new(57, 0, RESIZE_W, 600)),
        "down the right edge"
    );
    let right = classic_column(Edge::Right, 60, c, false);
    assert_eq!(
        right.resize,
        Some(Rect::new(0, 0, RESIZE_W, 600)),
        "down the left edge"
    );
    let t = left.tasks[0];
    assert_eq!(left.hit(59, t.y + 2), TaskbarHit::Resize);
    assert_eq!(left.hit(t.x + 1, t.y + 2), TaskbarHit::Task(0));
    assert_eq!(right.hit(0, right.tasks[0].y + 2), TaskbarHit::Resize);
    assert_eq!(left.scaled(2).resize, Some(Rect::new(114, 0, 6, 1200)));
    // Glass's strip spans the whole column: the edge wins over it.
    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let (m, shape) = (&glass.metrics, glass.taskbar_shape());
    let span = BarSpan {
        edge: Edge::Left,
        length: 600,
        thickness: 60,
    };
    let clock = ClockText { time: 30, date: 0 };
    let g = TaskbarLayout::compute_on(m, &shape, span, false, c, clock, Bands::default());
    let sd = g.show_desktop.expect("a strip");
    assert_eq!(sd.right(), 60, "Glass's strip runs to the inner edge");
    assert_eq!(g.hit(59, sd.y + 2), TaskbarHit::Resize);
    assert_eq!(g.hit(56, sd.y + 2), TaskbarHit::ShowDesktop);
    let locked = classic_column(Edge::Left, 60, c, true);
    assert_eq!(locked.resize, None);
    assert_ne!(locked.hit(59, t.y + 2), TaskbarHit::Resize);
    for edge in [Edge::Bottom, Edge::Top] {
        let row = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            false,
            c,
            30,
        )
        .on_edge(edge);
        assert_eq!(row.resize, None, "{edge:?}: no height drag");
    }
}

#[test]
fn a_turn_between_a_row_and_a_column_drops_the_first_bands_width() {
    let b = Bands {
        tasks_first: true,
        first_width: Some(300),
    };
    assert_eq!(
        b.turned(Edge::Bottom, Edge::Top),
        b,
        "the same axis keeps it"
    );
    assert_eq!(b.turned(Edge::Left, Edge::Right), b);
    assert_eq!(b.turned(Edge::Left, Edge::Left), b);
    let natural = Bands {
        tasks_first: true,
        first_width: None,
    };
    assert_eq!(b.turned(Edge::Bottom, Edge::Left), natural);
    assert_eq!(b.turned(Edge::Right, Edge::Top), natural);
}
