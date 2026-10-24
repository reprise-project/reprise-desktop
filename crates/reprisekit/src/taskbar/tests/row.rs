use super::*;

#[test]
fn classic_shape_reproduces_the_default_layout() {
    let m = Metrics::default();
    let a = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 1), 40);
    assert_eq!(a.start, Rect::new(2, 4, 54, 22));
    assert_eq!(a.tasks[0].y, 4);
    assert_eq!(a.tasks[0].height, 22);
}

#[test]
fn a_full_height_shape_lays_start_and_the_well_edge_to_edge() {
    let m = Metrics {
        taskbar_height: 30,
        start_button_width: 97,
        ..Metrics::default()
    };
    let shape = TaskbarShape {
        start: Rect::new(0, 0, 0, 30),
        button_top: 3,
        button_bottom: 2,
        start_gap: 7,
        well_full_height: true,
        well_lead: 4,
        tray_gap: 4,
        tray_low: false,
        side_tray_inset: 0,
        tray_arrow: None,
        clock: ClockBox::CLASSIC,
        start_icon: 18,
        task_icon: ICON_SIZE,
        icon_button: None,
        show_desktop_width: None,
        orb: false,
        orb_rise: None,
        two_line_clock: None,
        start_menu: StartMenuDock::Into(4),
        labelled: LabelledButton::CLASSIC,
    };
    let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 1, 2), 40);
    assert_eq!(l.start, Rect::new(0, 0, 97, 30));
    assert_eq!(
        (l.tasks[0].x, l.tasks[0].y, l.tasks[0].height),
        (104, 3, 25)
    );
    assert_eq!(l.notification.y, 0);
    assert_eq!(l.notification.height, 30);
    assert_eq!(l.notification.right(), 1920);
}

#[test]
fn layout_with_default_metrics() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
    assert_eq!(l.height, 28);
    assert_eq!(l.start, Rect::new(2, 4, 54, 22));
    assert_eq!(l.clock, Rect::new(800 - 2 - 46, 4, 46, 22));
    assert_eq!(l.tasks.len(), 2);
    assert_eq!(l.tasks[0], Rect::new(59, 4, 160, 22));
    assert_eq!(l.tasks[1], Rect::new(222, 4, 160, 22));
    assert!(l.grips.is_empty());
}

#[test]
fn tasks_shrink_to_fit() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 5, 0), 30);
    let avail = l.clock.x - 6 - l.tasks_x();
    let w = (avail - 2 * 4) / 5;
    assert_eq!(l.tasks[0].width, w);
    assert!(l.tasks[4].right() <= l.clock.x - 6);
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 0, 0), 30);
    assert!(l.tasks.is_empty());
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 40, 0), 30);
    assert!(l.tasks[0].width >= 1);
}

#[test]
fn tasks_never_overrun_clock() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 200, true, counts(0, 50, 0), 30);
    for r in &l.tasks {
        assert!(r.right() <= l.clock.x - 6);
    }
    assert!(l.tasks.len() < 50);
}

#[test]
fn narrow_width_clamps_clock_and_drops_tasks() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 10, true, counts(0, 3, 0), 30);
    assert!(l.clock.x >= l.tasks_x());
    assert!(l.tasks.is_empty());
}

#[test]
fn hit_testing() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
    assert_eq!(l.hit(10, 10), TaskbarHit::Start);
    assert_eq!(l.hit(70, 10), TaskbarHit::Task(0));
    assert_eq!(l.hit(230, 10), TaskbarHit::Task(1));
    assert_eq!(l.hit(780, 10), TaskbarHit::Clock);
    assert_eq!(l.hit(500, 10), TaskbarHit::Nothing);
    assert_eq!(l.hit(10, 40), TaskbarHit::Nothing);
}

fn every_cell(l: &TaskbarLayout) -> Vec<(TaskbarHit, Rect)> {
    let mut cells = vec![(TaskbarHit::Start, l.start), (TaskbarHit::Clock, l.clock)];
    let each = |hit: fn(usize) -> TaskbarHit, v: &[Rect]| {
        v.iter()
            .enumerate()
            .map(move |(i, &r)| (hit(i), r))
            .collect::<Vec<_>>()
    };
    cells.extend(each(TaskbarHit::Grip, &l.grips));
    cells.extend(each(TaskbarHit::Quick, &l.quick));
    cells.extend(each(TaskbarHit::Pager, &l.pager));
    cells.extend(each(TaskbarHit::Task, &l.tasks));
    cells.extend(each(TaskbarHit::Tray, &l.tray));
    cells.extend(l.quick_more.map(|r| (TaskbarHit::QuickMore, r)));
    cells.extend(l.chevron.map(|r| (TaskbarHit::Chevron, r)));
    cells.extend(l.show_desktop.map(|r| (TaskbarHit::ShowDesktop, r)));
    cells.extend(l.more.map(|r| (TaskbarHit::More, r)));
    cells
}

/// The screen-edge pixel across from the middle of `r`, and the strip between it and `r`.
fn edge_pixel(l: &TaskbarLayout, r: Rect) -> ((i32, i32), Rect) {
    let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
    match l.edge {
        Edge::Bottom => (
            (cx, l.height - 1),
            Rect::new(cx, r.bottom(), 1, l.height - r.bottom()),
        ),
        Edge::Top => ((cx, 0), Rect::new(cx, 0, 1, r.y)),
        Edge::Left => ((0, cy), Rect::new(0, cy, r.x, 1)),
        Edge::Right => (
            (l.width - 1, cy),
            Rect::new(r.right(), cy, l.width - r.right(), 1),
        ),
    }
}

#[test]
fn every_cell_takes_the_screen_edge_pixel_across_from_it() {
    use crate::interface::Interface;
    let counts = [
        TaskbarCounts {
            quick: 6,
            pager: 2,
            tasks: 3,
            tray: 3,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        },
        TaskbarCounts {
            quick: 2,
            tasks: 60,
            tray: 1,
            ..TaskbarCounts::default()
        },
    ];
    let bands = Bands {
        tasks_first: false,
        first_width: Some(60),
    };
    let mut seen = std::collections::HashSet::new();
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(look);
        let shape = theme.taskbar_shape();
        for edge in Edge::ALL {
            let thickness = if edge.is_side() {
                100
            } else {
                theme.metrics.taskbar_height
            };
            let span = BarSpan {
                edge,
                length: if edge.is_side() { 700 } else { 1000 },
                thickness,
            };
            for (c, locked) in counts.iter().zip([false, true]) {
                let l = TaskbarLayout::compute_on(
                    &theme.metrics,
                    &shape,
                    span,
                    locked,
                    *c,
                    ClockText { time: 30, date: 30 },
                    bands,
                );
                let cells = every_cell(&l);
                for &(hit, r) in &cells {
                    let ((x, y), between) = edge_pixel(&l, r);
                    let shielded = cells
                        .iter()
                        .any(|&(h, o)| h != hit && !o.intersect(&between).is_empty());
                    if shielded {
                        continue;
                    }
                    assert_eq!(
                        l.hit(x, y),
                        hit,
                        "{look:?} {edge:?}: {hit:?} at {r:?} takes ({x}, {y})"
                    );
                    seen.insert(std::mem::discriminant(&hit));
                }
            }
        }
    }
    assert_eq!(seen.len(), 11, "every kind of cell checked");
}

#[test]
fn scaled_layout_doubles_every_rect() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
    let s = l.scaled(2);
    assert_eq!(s.width, 1600);
    assert_eq!(s.height, 56);
    assert_eq!(s.start, l.start.scaled(2));
    assert_eq!(s.clock, l.clock.scaled(2));
    assert_eq!(s.tasks, vec![l.tasks[0].scaled(2), l.tasks[1].scaled(2)]);
    // hit-testing stays on the logical layout; the scaled one is for drawing only
    assert_eq!(l.hit(10, 10), TaskbarHit::Start);
}

#[test]
fn tray_sits_left_of_clock_and_narrows_tasks() {
    let m = Metrics::default();
    let none = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
    let two = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
    assert_eq!(two.tray.len(), 2);
    assert_eq!(two.tray[1].right(), two.clock.x - 6);
    assert_eq!(two.tray[1].x - two.tray[0].right(), 4);
    assert_eq!(two.tray[0].width, crate::icon::ICON_SIZE);
    assert_eq!(two.tray[0].height, crate::icon::ICON_SIZE);
    assert_eq!(
        two.tray[0].y,
        4 + (m.taskbar_height - 6 - crate::icon::ICON_SIZE) / 2
    );
    // A single task button is capped at task_button_max_width either way,
    // so compare the available span instead: two icons cost 40 px.
    let avail = |l: &TaskbarLayout| l.tasks_limit() - l.tasks_x();
    assert_eq!(avail(&none) - avail(&two), 40);
    assert_eq!(two.hit(two.tray[1].x + 3, 15), TaskbarHit::Tray(1));
    assert_eq!(two.hit(two.tray[0].x + 3, 15), TaskbarHit::Tray(0));
    assert_eq!(none.tasks_limit(), none.clock.x - 6);
}

#[test]
fn tray_cells_that_reach_the_clock_are_dropped() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 1, 16), 30);
    assert!(!l.tray.is_empty() && l.tray.len() < 16);
    assert_eq!(l.tray[0].x, l.band_x);
    for r in &l.tray {
        assert!(
            r.right() <= l.clock.x - 6,
            "cell at {} ends 6 px before the clock",
            r.x
        );
    }
}

#[test]
fn notification_well_wraps_the_tray_and_clock() {
    let m = Metrics::default();
    let bare = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
    assert_eq!(bare.notification, bare.clock);
    let two = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
    assert_eq!(two.notification.x, two.tray[0].x - 4);
    assert_eq!(two.notification.right(), two.clock.right());
    assert!(two.notification.x < two.tray[0].x);
    assert_eq!(two.notification.y, two.clock.y);
    assert_eq!(two.notification.height, two.clock.height);
}

#[test]
fn quick_launch_sits_between_start_and_the_tasks() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(2, 1, 0), 30);
    assert_eq!(
        l.quick,
        vec![Rect::new(59, 4, 23, 22), Rect::new(82, 4, 23, 22)]
    );
    assert_eq!(l.tasks_x(), 109);
    assert_eq!(l.tasks[0].x, 109);
    assert_eq!(l.hit(70, 10), TaskbarHit::Quick(0));
    assert_eq!(l.hit(100, 10), TaskbarHit::Quick(1));
    assert_eq!(l.hit(120, 10), TaskbarHit::Task(0));
    let none = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
    assert!(none.quick.is_empty());
    assert_eq!(none.tasks_x(), 59);
    assert_eq!(l.scaled(2).quick[1], l.quick[1].scaled(2));
    assert_eq!(l.scaled(2).tasks_x(), 218);
}

#[test]
fn pager_leads_quick_launch_in_its_band() {
    let m = Metrics::default();
    let c = |quick, pager| TaskbarCounts {
        quick,
        pager,
        tasks: 1,
        tray: 0,
        show_desktop: false,
        chevron: false,
        ..TaskbarCounts::default()
    };
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, c(2, 4), 30);
    assert_eq!(l.pager[0], Rect::new(59, 4, 23, 22));
    assert_eq!(l.pager[3], Rect::new(128, 4, 23, 22));
    assert_eq!(l.quick[0].x, 155);
    assert_eq!(l.quick[1].x, 178);
    assert_eq!(l.tasks_x(), 205);
    assert_eq!(l.hit(60, 10), TaskbarHit::Pager(0));
    assert_eq!(l.hit(150, 10), TaskbarHit::Pager(3));
    assert_eq!(l.hit(152, 10), TaskbarHit::Nothing);
    assert!(l.on_quick_band(152, 10) && !l.on_quick_band(150, 10));
    let alone = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, c(0, 2), 30);
    assert_eq!(alone.pager[0], Rect::new(59, 4, 23, 22));
    assert_eq!(alone.tasks_x(), 109);
    assert!(alone.on_quick_band(106, 10) && !alone.on_quick_band(104, 10));
    assert!(!alone.on_quick_band(57, 10) && !alone.on_quick_band(109, 10));
    let un = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, c(2, 4), 30);
    assert_eq!(un.pager[0].x, 68);
    assert_eq!(un.quick[0].x, 164);
    assert_eq!(un.bands.first_width, Some(142));
    assert_eq!(un.grips[1].x, 216);
    assert_eq!(l.scaled(2).pager[1], l.pager[1].scaled(2));
}

#[test]
fn unlocked_taskbar_puts_a_grip_before_each_band() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 1, 0), 30);
    assert_eq!(
        l.grips,
        vec![Rect::new(61, 6, 3, 18), Rect::new(120, 6, 3, 18)]
    );
    assert_eq!(l.quick[0], Rect::new(68, 4, 23, 22));
    assert_eq!(l.tasks[0].x, 127);
    assert_eq!(l.hit(62, 10), TaskbarHit::Grip(0));
    assert_eq!(l.hit(119, 10), TaskbarHit::Grip(1));
    assert_eq!(l.hit(126, 10), TaskbarHit::Nothing);
    assert_eq!(
        l.bands,
        Bands {
            tasks_first: false,
            first_width: Some(46)
        }
    );
    assert!(l.on_quick_band(66, 10) && l.on_quick_band(118, 10));
    assert!(!l.on_quick_band(127, 10));
    let idle = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 0, 0), 30);
    assert_eq!(idle.grips, vec![Rect::new(61, 6, 3, 18)]);
    assert_eq!(idle.tasks_x(), 127);
    let bare = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(0, 1, 0), 30);
    assert_eq!(bare.grips, vec![Rect::new(61, 6, 3, 18)]);
    assert_eq!(bare.tasks[0].x, 68);
    assert_eq!(l.scaled(2).grips[0], l.grips[0].scaled(2));
}

#[test]
fn first_band_width_moves_the_second_band() {
    let m = Metrics::default();
    let wide = |w| Bands {
        tasks_first: false,
        first_width: Some(w),
    };
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        wide(247),
    );
    assert_eq!(l.grips[1], Rect::new(321, 6, 3, 18));
    assert_eq!(l.tasks[0].x, 328);
    assert_eq!(l.quick[1].x, 91);
    assert!(l.on_quick_band(300, 10) && !l.on_quick_band(328, 10));
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        wide(10),
    );
    assert_eq!(
        l.grips[1].x,
        68 + 14 + 6,
        "the Quick Launch band shrinks to its »"
    );
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        true,
        counts(2, 1, 0),
        30,
        wide(247),
    );
    assert!(l.grips.is_empty());
    assert_eq!(l.tasks_x(), 59 + 247 + 4);
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        400,
        false,
        counts(2, 1, 2),
        30,
        wide(1000),
    );
    assert_eq!(l.tasks_x(), l.tasks_limit());
}

#[test]
fn tasks_first_puts_quick_launch_after_the_task_band() {
    let m = Metrics::default();
    let b = Bands {
        tasks_first: true,
        first_width: Some(425),
    };
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        b,
    );
    assert_eq!(
        l.grips,
        vec![Rect::new(61, 6, 3, 18), Rect::new(499, 6, 3, 18)]
    );
    assert_eq!(l.tasks[0], Rect::new(68, 4, 160, 22));
    assert_eq!(l.tasks_limit(), 493);
    assert_eq!(l.quick[0].x, 506);
    assert_eq!(l.band_at_grip(0), Band::Tasks);
    assert_eq!(l.band_at_grip(1), Band::Quick);
    assert!(l.on_quick_band(500, 10) && l.on_quick_band(560, 10) && !l.on_quick_band(300, 10));
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        Bands {
            tasks_first: true,
            first_width: None,
        },
    );
    assert_eq!(l.grips[1].x, 68 + 160 + 6);
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        Bands {
            tasks_first: true,
            first_width: Some(0),
        },
    );
    assert_eq!(l.grips[1].x, 68 + 66 + 6);
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(0, 1, 0),
        30,
        b,
    );
    assert_eq!(l.grips.len(), 1);
    assert_eq!(l.tasks[0].x, 68);
    assert_eq!(l.bands, Bands::default());
}

#[test]
fn an_open_tray_run_reaches_a_quick_band_dragged_short() {
    let m = Metrics::default();
    let lay = |bands: Bands| {
        let span = BarSpan {
            edge: Edge::Bottom,
            length: 800,
            thickness: m.taskbar_height,
        };
        let counts = TaskbarCounts {
            quick: 12,
            reveal: i32::MAX,
            ..arrow_counts(2, 30, 0)
        };
        let clock = ClockText { time: 30, date: 30 };
        TaskbarLayout::compute_on(
            &m,
            &TaskbarShape::CLASSIC,
            span,
            false,
            counts,
            clock,
            bands,
        )
    };
    let whole = lay(Bands::default());
    let short = lay(Bands {
        tasks_first: false,
        first_width: Some(60),
    });
    let more = short.quick_more.expect("a » cell");
    let arrow = short.chevron.expect("the arrow");
    assert_eq!(
        (whole.tray_hidden, short.tray_hidden),
        (16, 27),
        "the run reaches the shortened band"
    );
    assert!(arrow.x > more.right(), "the run stops short of the band");
}

#[test]
fn a_quick_band_shorter_than_its_buttons_ends_in_a_more_cell() {
    let m = Metrics::default();
    let lay = |bands: Bands, edge: Edge| {
        let span = BarSpan {
            edge,
            length: 800,
            thickness: m.taskbar_height,
        };
        let clock = ClockText { time: 30, date: 30 };
        TaskbarLayout::compute_on(
            &m,
            &TaskbarShape::CLASSIC,
            span,
            false,
            counts(5, 1, 0),
            clock,
            bands,
        )
    };
    let wide = |w| Bands {
        tasks_first: false,
        first_width: Some(w),
    };
    let whole = lay(Bands::default(), Edge::Bottom);
    assert_eq!((whole.quick.len(), whole.quick_more), (5, None));
    let x0 = whole.quick[0].x;
    let three = lay(wide(3 * 23 + 14), Edge::Bottom);
    assert_eq!(three.quick, whole.quick[..3].to_vec());
    let r = three.quick_more.expect("a band 83 px long shows a »");
    assert_eq!(r, Rect::new(x0 + 69, 4, 14, 22));
    assert_eq!(three.hit(r.x + 7, r.y + 11), TaskbarHit::QuickMore);
    assert!(three.on_quick_band(r.right() - 1, 10));
    let two = lay(wide(3 * 23 + 13), Edge::Bottom);
    assert_eq!(two.quick.len(), 2, "a px short of three buttons and the »");
    assert_eq!(two.quick_more.map(|r| r.x), Some(x0 + 46));
    let least = lay(wide(0), Edge::Bottom);
    assert_eq!(least.quick, vec![]);
    assert_eq!(least.quick_more, Some(Rect::new(x0, 4, 14, 22)));
    assert_eq!(
        least.grips[1].x,
        x0 + 14 + 6,
        "the band keeps room for the » alone"
    );

    let top = lay(wide(83), Edge::Top);
    assert_eq!(top.quick_more, Some(Rect::new(x0 + 69, 2, 14, 22)));
    assert_eq!(three.scaled(2).quick_more, Some(r.scaled(2)));

    let after = lay(
        Bands {
            tasks_first: true,
            first_width: Some(2000),
        },
        Edge::Bottom,
    );
    let r = after
        .quick_more
        .expect("a task band dragged over the tools leaves the »");
    assert_eq!(after.quick, vec![]);
    assert_eq!(r.right(), after.tasks_limit() + 13 + 14);
}

#[test]
fn a_short_column_quick_band_gives_its_last_cell_to_the_more_cell() {
    let span = BarSpan {
        edge: Edge::Left,
        length: 600,
        thickness: 60,
    };
    let lay = |first_width| {
        TaskbarLayout::compute_on(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            span,
            false,
            counts(7, 1, 0),
            ClockText { time: 30, date: 0 },
            Bands {
                tasks_first: false,
                first_width,
            },
        )
    };
    let whole = lay(None);
    assert_eq!((whole.quick.len(), whole.quick_more), (7, None));
    let two_rows = lay(Some(2 * 23));
    assert_eq!(two_rows.quick, whole.quick[..3].to_vec());
    assert_eq!(two_rows.quick_more, Some(whole.quick[3]));
    assert_eq!(
        two_rows.hit(30, whole.quick[3].y + 5),
        TaskbarHit::QuickMore
    );
    let least = lay(Some(0));
    assert_eq!(least.quick, vec![whole.quick[0]]);
    assert_eq!(least.quick_more, Some(whole.quick[1]));
}

#[test]
fn dragging_a_grip_resizes_or_swaps_the_bands() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 1, 0), 30);
    let d = Bands::default();
    assert_eq!(
        l.drag_band(d, Band::Tasks, 315),
        Bands {
            tasks_first: false,
            first_width: Some(241)
        }
    );
    let back = l.drag_band(d, Band::Tasks, 50);
    assert!(!back.tasks_first);
    let l2 = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        back,
    );
    assert_eq!(l2.grips[1].x, 68 + 14 + 6);
    assert_eq!(l.drag_band(d, Band::Quick, 100), d);
    assert_eq!(
        l.drag_band(d, Band::Quick, 499),
        Bands {
            tasks_first: true,
            first_width: Some(425)
        }
    );
    let t = Bands {
        tasks_first: true,
        first_width: Some(425),
    };
    let l = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        t,
    );
    assert_eq!(
        l.drag_band(t, Band::Quick, 200),
        Bands {
            tasks_first: true,
            first_width: Some(126)
        }
    );
    let back = l.drag_band(t, Band::Quick, 40);
    assert!(back.tasks_first);
    let l2 = TaskbarLayout::compute_with(
        &m,
        &TaskbarShape::CLASSIC,
        800,
        false,
        counts(2, 1, 0),
        30,
        back,
    );
    assert_eq!(l2.grips[1].x, 140);
    assert_eq!(l2.tasks[0].width, 66);
    assert_eq!(l.drag_band(t, Band::Tasks, 300), t);
    let one = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(0, 1, 0), 30);
    assert_eq!(one.drag_band(d, Band::Tasks, 300), d);
}

#[test]
fn drop_slot_counts_the_buttons_left_of_the_pointer() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(3, 0, 0), 30);
    assert_eq!(l.drop_slot(0, 10), 0);
    assert_eq!(l.drop_slot(l.quick[0].x + 11, 10), 1);
    assert_eq!(l.drop_slot(l.quick[1].x + 5, 10), 1);
    assert_eq!(l.drop_slot(l.quick[2].x + 20, 10), 3);
    assert_eq!(l.drop_slot(700, 10), 3);
}
