use super::*;

#[test]
fn a_top_bar_mirrors_every_rect_across_its_height() {
    let bottom = classic_full(Edge::Bottom);
    let top = classic_full(Edge::Top);
    let flip = |r: Rect| Rect::new(r.x, 28 - r.y - r.height, r.width, r.height);
    let all = |v: &[Rect]| v.iter().copied().map(flip).collect::<Vec<_>>();
    assert_eq!(top.edge, Edge::Top);
    assert_eq!(
        top.start,
        Rect::new(2, 2, 54, 22),
        "Start 2 px from the top"
    );
    assert_eq!(top.start, flip(bottom.start));
    assert_eq!(top.grips, all(&bottom.grips));
    assert_eq!(top.quick, all(&bottom.quick));
    assert_eq!(top.pager, all(&bottom.pager));
    assert_eq!(top.tasks, all(&bottom.tasks));
    assert_eq!(top.tray, all(&bottom.tray));
    assert_eq!(top.chevron, bottom.chevron.map(flip));
    assert_eq!(top.clock, flip(bottom.clock));
    assert_eq!(top.notification, flip(bottom.notification));
    assert_eq!(top.show_desktop, bottom.show_desktop.map(flip));
    assert_eq!(
        (
            top.width,
            top.height,
            top.tasks_x(),
            top.tasks_limit(),
            top.bands
        ),
        (
            bottom.width,
            bottom.height,
            bottom.tasks_x(),
            bottom.tasks_limit(),
            bottom.bands
        ),
        "along the bar nothing moves"
    );
    assert_eq!(
        top.to_along(top.start),
        bottom.start,
        "to_along maps a Top rect back"
    );
    assert_eq!(
        bottom.to_along(bottom.start),
        bottom.start,
        "a Bottom bar keeps its rects"
    );
    assert_eq!(
        top.on_edge(Edge::Bottom),
        bottom,
        "back on the foot it is the Bottom bar"
    );
}

#[test]
fn a_top_bar_hits_what_the_bottom_bar_hits_on_the_mirrored_row() {
    let bottom = classic_full(Edge::Bottom);
    let top = classic_full(Edge::Top);
    for y in 0..28 {
        for x in 0..800 {
            assert_eq!(top.hit(x, 27 - y), bottom.hit(x, y), "({x}, {y})");
        }
    }
    assert_eq!(top.hit(10, -1), TaskbarHit::Nothing, "above the bar");
    assert_eq!(top.hit(10, 28), TaskbarHit::Nothing, "below the bar");
}

#[test]
fn lay_tasks_on_a_top_bar_keeps_the_row_mirrored() {
    let m = Metrics::default();
    let mut top = classic_row(3).on_edge(Edge::Top);
    top.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 0, 2], TaskRow::Icons);
    assert_eq!(
        top.tasks,
        vec![
            Rect::new(59, 2, 22, 22),
            Rect::new(84, 2, 22, 22),
            Rect::new(109, 2, 22, 22),
        ],
        "22 px squares 2 px from the top"
    );
    assert_eq!(top.stacks, vec![1, 0, 2]);
    top.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1, 1], TaskRow::Shared);
    assert_eq!(
        top.tasks,
        classic_row(3).on_edge(Edge::Top).tasks,
        "a shared row lays out again the same"
    );
    assert!(top.tasks.iter().all(|r| r.y == 2));
}

#[test]
fn a_scaled_top_bar_stays_on_the_top() {
    let top = classic_row(2).on_edge(Edge::Top);
    let s = top.scaled(2);
    assert_eq!(s.edge, Edge::Top);
    assert_eq!(
        s.start,
        Rect::new(4, 4, 108, 44),
        "Start 4 device px from the top at scale 2"
    );
    assert_eq!(s.tasks[1], top.tasks[1].scaled(2));
    assert_eq!(s.to_along(s.start), classic_row(2).scaled(2).start);
}

#[test]
fn an_edge_goes_by_its_config_name() {
    assert_eq!(
        Edge::default(),
        Edge::Bottom,
        "a bar runs along the foot by default"
    );
    assert_eq!(
        Edge::ALL.map(Edge::name),
        ["bottom", "top", "left", "right"]
    );
    assert!(Edge::Left.is_side() && Edge::Right.is_side());
    assert!(!Edge::Bottom.is_side() && !Edge::Top.is_side());
    for e in Edge::ALL {
        assert_eq!(Edge::from_name(e.name()), Some(e), "{e:?} reads back");
    }
    assert_eq!(Edge::from_name("sideways"), None);
}

#[test]
fn a_top_glass_bar_hangs_its_orb_below_the_bar() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let small = crate::glass::small_bar_style(&Default::default()).taskbar;
    let bottom = TaskbarLayout::compute(&m, &small, 1920, true, counts(0, 2, 0), 30);
    let l = bottom.clone().on_edge(Edge::Top);
    assert_eq!(l.start, Rect::new(0, 0, 54, 30), "Start is 54x30 at (0, 0)");
    assert_eq!(
        l.start_above,
        Some(Rect::new(0, 30, 54, 13)),
        "the rows below the bar are 54x13 at (0, 30)"
    );
    assert_eq!(
        l.start_disc,
        Some(Rect::new(9, 0, 36, 36)),
        "the disc is 36x36 at (9, 0), lowered by its 5-row hang"
    );
    assert_eq!(
        l.disc_past_bar(),
        Some(Rect::new(9, 30, 36, 5)),
        "the disc's art hangs 36x5 at (9, 30)"
    );
    assert_eq!(
        (l.start_orb(false), l.start_orb(true)),
        (
            Some(Rect::new(0, 30, 54, 13)),
            Some(Rect::new(0, 0, 54, 43))
        ),
        "the orb's own buffer: the rows below, held with Start's 30 rows too"
    );
    // The art is upright and lowered by 5 rows, so each row below the
    // bar is the disc's row 5 rows higher over a Bottom bar: 26 px of
    // Start against the bar, 14 in the art's last row. The box's 36th
    // row, which runs off a Bottom bar's foot, has no art and no Start.
    let across = |y: i32| {
        (0..54)
            .filter(|&x| l.hit(x, y) == TaskbarHit::Start)
            .count()
    };
    for y in 30..35 {
        let upright = (0..54).filter(|&x| bottom.on_disc(x, y - 5)).count();
        assert_eq!(across(y), upright, "row {y} is as wide as row {}", y - 5);
    }
    assert_eq!(across(30), 26, "the row against the bar is 26 px of Start");
    assert_eq!(across(34), 14, "the art's last row is 14 px of Start");
    assert_eq!(across(35), 0, "no Start below the art, 0 px");
    assert_eq!(
        l.hit(10, 30),
        TaskbarHit::Nothing,
        "(10, 30), beside the disc, is Nothing"
    );
    assert_eq!(
        l.hit(26, 38),
        TaskbarHit::Nothing,
        "(26, 38), on the lit glow's rows, is Nothing"
    );
    assert_eq!(
        l.hit(26, -1),
        TaskbarHit::Nothing,
        "(26, -1), past the screen's top edge, is Nothing"
    );
    assert_eq!(l.on_edge(Edge::Bottom), bottom, "moved back it rises");

    let full = TaskbarLayout::compute(
        &theme.metrics,
        &theme.taskbar_shape(),
        1920,
        true,
        counts(0, 2, 0),
        30,
    )
    .on_edge(Edge::Top);
    assert_eq!(
        (full.start_above, full.start_disc),
        (Some(Rect::new(0, 40, 54, 6)), None),
        "the 40 px bar has 54x6 rows below it and no disc below it"
    );
    assert_eq!(
        full.hit(26, 40),
        TaskbarHit::Nothing,
        "(26, 40) below the 40 px bar is Nothing"
    );
}

#[test]
fn a_top_small_glass_orbs_access_bounds_take_in_its_disc_below_the_bar() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
    let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30).on_edge(Edge::Top);
    let mut t = Tree::window("Taskbar", (l.width, l.height));
    access(&l, &TaskbarNames::default(), &mut t, access::ROOT);
    let b = t
        .get(t.find(Role::Button, "Start").unwrap())
        .unwrap()
        .bounds()
        .unwrap();
    assert_eq!(
        (b.x0, b.y0, b.x1, b.y1),
        (0.0, 0.0, 54.0, 35.0),
        "Start reaches from (0, 0) to (54, 35)"
    );
}

#[test]
fn the_reveal_line_is_the_bars_inner_two_rows() {
    let l = TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        800,
        true,
        TaskbarCounts::default(),
        40,
    );
    let h = l.height;
    assert_eq!(
        l.reveal_line(),
        Rect::new(0, 0, 800, REVEAL_LINE),
        "a Bottom bar's top rows"
    );
    let top = l.on_edge(Edge::Top);
    assert_eq!(
        top.reveal_line(),
        Rect::new(0, h - REVEAL_LINE, 800, REVEAL_LINE),
        "a Top bar's foot rows"
    );
    let thin = TaskbarLayout { height: 1, ..top };
    assert_eq!(
        thin.reveal_line(),
        Rect::new(0, 0, 800, 1),
        "a 1 px bar is all line"
    );
}

#[test]
fn an_edge_turns_a_rect_between_along_across_and_x_y_and_back() {
    let r = Rect::new(59, 4, 160, 22);
    assert_eq!(
        Edge::Bottom.to_xy(r, 28),
        r,
        "a Bottom bar is its own along/across form"
    );
    assert_eq!(
        Edge::Top.to_xy(r, 28),
        Rect::new(59, 2, 160, 22),
        "a Top bar mirrors it across its 28 rows"
    );
    for e in Edge::ALL {
        assert_eq!(
            e.to_xy(e.to_xy(r, 28), 28),
            r,
            "{e:?}: twice is where it started"
        );
    }
    assert_eq!(
        Edge::Left.to_xy(r, 60),
        Rect::new(4, 59, 22, 160),
        "a side bar swaps x and y: along runs down the column"
    );
    assert_eq!(Edge::Right.to_xy(r, 60), Edge::Left.to_xy(r, 60));
}

#[test]
fn along_a_top_bar_is_along_x_as_on_a_bottom_one() {
    for edge in [Edge::Bottom, Edge::Top] {
        let l = classic_row(3).on_edge(edge);
        let r = l.tasks[1];
        assert_eq!((l.length(), l.thickness()), (800, 28), "{edge:?}");
        assert_eq!(l.along(r.x + 5, r.y + 3), r.x + 5, "{edge:?}");
        assert_eq!(l.along_f(12.5, 3.0), 12.5, "{edge:?}");
        assert_eq!(
            l.quick_drag(5.0, -40.0),
            5.0,
            "{edge:?}: only along the bar"
        );
        assert_eq!(
            l.moved_along(r, 7),
            Rect::new(r.x + 7, r.y, r.width, r.height),
            "{edge:?}"
        );
        assert_eq!(
            l.placed_along(r, 300),
            Rect::new(300, r.y, r.width, r.height),
            "{edge:?}"
        );
        assert_eq!(l.along_rects(&l.tasks)[1].x, r.x, "{edge:?}");
        assert_eq!(l.to_xy(l.to_along(r)), r, "{edge:?}: there and back");
    }
}

#[test]
fn a_top_span_lays_the_bottom_bar_turned_over() {
    let m = Metrics::default();
    let c = TaskbarCounts {
        quick: 2,
        pager: 2,
        tasks: 3,
        tray: 2,
        show_desktop: true,
        chevron: true,
        ..TaskbarCounts::default()
    };
    let span = BarSpan {
        edge: Edge::Top,
        length: 800,
        thickness: 28,
    };
    let clock = ClockText { time: 30, date: 30 };
    let top = TaskbarLayout::compute_on(
        &m,
        &TaskbarShape::CLASSIC,
        span,
        false,
        c,
        clock,
        Bands::default(),
    );
    let bottom = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, c, 30);
    assert_eq!(top, bottom.on_edge(Edge::Top));
}

#[test]
fn a_two_line_clock_is_as_wide_as_its_wider_line() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let shape = theme.taskbar_shape();
    let lay = |thickness| {
        let span = BarSpan {
            edge: Edge::Bottom,
            length: 1920,
            thickness,
        };
        let clock = ClockText { time: 40, date: 70 };
        TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            TaskbarCounts::default(),
            clock,
            Bands::default(),
        )
    };
    assert_eq!(
        lay(40).clock.width,
        70 + shape.clock.pad,
        "two lines on the 40 px bar: the date's width"
    );
    assert_eq!(
        lay(30).clock.width,
        40 + shape.clock.pad,
        "one line on a 30 px bar: the time's"
    );
}

#[test]
fn a_top_bar_drops_a_quick_launch_button_where_a_bottom_one_does() {
    let bottom = classic_full(Edge::Bottom);
    let top = classic_full(Edge::Top);
    for x in [0, 66, 70, 80, 95, 118, 300] {
        assert_eq!(top.drop_slot(x, 20), bottom.drop_slot(x, 7), "x {x}");
        assert_eq!(
            top.on_quick_band(x, 20),
            bottom.on_quick_band(x, 7),
            "x {x}"
        );
    }
}

#[test]
fn draw_art_turns_a_bottom_picture_to_each_edge() {
    let ink = Color::rgb(255, 0, 0);
    let px = ink.argb();
    let art = |c: &mut Canvas, at: Rect| c.fill_rect(Rect::new(at.x, at.y, 3, 1), ink);
    let mut b = Canvas::new(6, 4);
    Edge::Bottom.draw_art(&mut b, Rect::new(0, 0, 6, 4), art);
    assert_eq!(
        (b.get(2, 0), b.get(3, 0), b.get(0, 1)),
        (px, 0, 0),
        "Bottom: as drawn"
    );
    let mut t = Canvas::new(6, 4);
    Edge::Top.draw_art(&mut t, Rect::new(0, 0, 6, 4), art);
    assert_eq!((t.get(0, 3), t.get(0, 0)), (px, 0), "Top: on the foot row");
    let mut l = Canvas::new(4, 6);
    Edge::Left.draw_art(&mut l, Rect::new(0, 0, 4, 6), art);
    assert_eq!(
        (l.get(3, 0), l.get(3, 2), l.get(3, 3), l.get(0, 0)),
        (px, px, 0, 0),
        "Left: down the right column, from the top"
    );
    let mut r = Canvas::new(4, 6);
    Edge::Right.draw_art(&mut r, Rect::new(0, 0, 4, 6), art);
    assert_eq!(
        (r.get(0, 0), r.get(0, 2), r.get(0, 3), r.get(3, 0)),
        (px, px, 0, 0),
        "Right: down the left column"
    );
}
