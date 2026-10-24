use super::*;

#[test]
fn a_shared_row_is_the_labelled_layout_with_its_stacks() {
    let mut l = classic_row(3);
    let before = l.tasks.clone();
    assert_eq!(l.stacks, vec![1, 1, 1], "compute lays single windows");
    l.lay_tasks(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        &[1, 4, 1],
        TaskRow::Shared,
    );
    assert_eq!(l.tasks, before);
    assert_eq!(l.stacks, vec![1, 4, 1]);
}

#[test]
fn classic_icon_buttons_are_square_three_px_apart() {
    let mut l = classic_row(3);
    l.lay_tasks(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        &[1, 2, 3],
        TaskRow::Icons,
    );
    assert_eq!(
        l.tasks,
        vec![
            Rect::new(59, 4, 22, 22),
            Rect::new(84, 4, 22, 22),
            Rect::new(109, 4, 22, 22),
        ]
    );
    assert_eq!(l.stacks, vec![1, 2, 3]);
    assert_eq!(l.hit(90, 10), TaskbarHit::Task(1));
}

#[test]
fn glass_icon_buttons_are_60_by_40_at_a_62_px_pitch() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let (m, shape) = (&theme.metrics, theme.taskbar_shape());
    let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 2, 0), 30);
    l.lay_tasks(m, &shape, &[5, 2], TaskRow::Icons);
    let x = l.tasks_x();
    assert_eq!(
        l.tasks,
        vec![Rect::new(x, 0, 60, 40), Rect::new(x + 62, 0, 60, 40)]
    );
}

#[test]
fn small_icons_shrink_glass_to_a_30_px_bar_at_a_46_px_pitch() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let shape = crate::glass::small_bar_style(&Default::default()).taskbar;

    let mut l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
    l.lay_tasks(&m, &shape, &[3, 1], TaskRow::Icons);
    let x = l.tasks_x();
    assert_eq!(
        l.tasks,
        vec![Rect::new(x, 0, 44, 30), Rect::new(x + 46, 0, 44, 30)]
    );

    let mut l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
    l.lay_tasks(&m, &shape, &[1, 1], TaskRow::Shared);
    assert!(l.tasks.iter().all(|r| r.height == 30 && r.width <= 160));

    let mut l = TaskbarLayout::compute(&m, &shape, 200, true, counts(0, 6, 0), 30);
    l.lay_tasks(&m, &shape, &[1; 6], TaskRow::AtLeast(62));
    assert!(l.tasks.iter().all(|r| r.height == 30 && r.width == 62));
}

#[test]
fn the_small_glass_orb_stands_5_px_above_the_bar_and_its_disc_is_start() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
    let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
    assert_eq!(l.start, Rect::new(0, 0, 54, 30), "Start is 54x30 at (0, 0)");
    assert_eq!(
        l.start_above,
        Some(Rect::new(0, -13, 54, 13)),
        "the rows above the bar are 54x13 at (0, -13)"
    );
    assert_eq!(
        (l.start_orb(false), l.start_orb(true)),
        (
            Some(Rect::new(0, -13, 54, 13)),
            Some(Rect::new(0, -13, 54, 43))
        ),
        "the orb's own buffer: the rows above, held with Start's 30 rows too"
    );
    assert_eq!(
        l.start_disc,
        Some(Rect::new(9, -5, 36, 36)),
        "the disc is 36x36 at (9, -5)"
    );
    let across = |y: i32| {
        (0..54)
            .filter(|&x| l.hit(x, y) == TaskbarHit::Start)
            .count()
    };
    assert_eq!(across(-5), 8, "the disc's top row is 8 px of Start");
    assert_eq!(across(-1), 24, "the row against the bar is 24 px of Start");
    assert_eq!(across(-6), 0, "no Start above the disc, 0 px");
    assert_eq!(l.hit(26, -5), TaskbarHit::Start, "(26, -5) is Start");
    assert_eq!(
        l.hit(10, -1),
        TaskbarHit::Nothing,
        "(10, -1), beside the disc, is Nothing"
    );
    assert_eq!(
        l.hit(26, -8),
        TaskbarHit::Nothing,
        "(26, -8), on the lit glow's rows, is Nothing"
    );
    assert_eq!(
        l.hit(100, -1),
        TaskbarHit::Nothing,
        "(100, -1), above the bar past Start, is Nothing"
    );
    let s2 = l.scaled(2);
    assert_eq!(
        s2.start_above,
        Some(Rect::new(0, -26, 108, 26)),
        "at scale 2 the rows above the bar are 108x26 at (0, -26)"
    );
    assert_eq!(
        s2.start_disc,
        Some(Rect::new(18, -10, 72, 72)),
        "at scale 2 the disc is 72x72 at (18, -10)"
    );
    assert!(
        s2.on_disc(53, -10),
        "at scale 2 the disc's top row holds (53, -10)"
    );
    assert!(
        !s2.on_disc(53, -11),
        "at scale 2 (53, -11) lies above the disc"
    );

    let full = TaskbarLayout::compute(
        &theme.metrics,
        &theme.taskbar_shape(),
        1920,
        true,
        counts(0, 2, 0),
        30,
    );
    assert_eq!(
        (full.start_above, full.start_disc),
        (Some(Rect::new(0, -6, 54, 6)), None),
        "the 40 px bar has 54x6 rows above it and no disc above it"
    );
    assert_eq!(
        full.start_orb(true),
        Some(Rect::new(0, -6, 54, 46)),
        "held, the 40 px bar's orb buffer takes in Start: 54x46 at (0, -6)"
    );
    assert_eq!(
        full.hit(26, -1),
        TaskbarHit::Nothing,
        "(26, -1) above the 40 px bar is Nothing"
    );
    let classic = TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        1920,
        true,
        counts(0, 2, 0),
        30,
    );
    assert_eq!(classic.start_above, None, "Classic has no rows above: None");
}

#[test]
fn a_small_glass_orbs_access_bounds_take_in_its_disc_above_the_bar() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let m = Metrics {
        taskbar_height: 30,
        ..theme.metrics.clone()
    };
    let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
    let bounds = |l: &TaskbarLayout| {
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(l, &TaskbarNames::default(), &mut t, access::ROOT);
        let b = t
            .get(t.find(Role::Button, "Start").unwrap())
            .unwrap()
            .bounds()
            .unwrap();
        (b.x0, b.y0, b.x1, b.y1)
    };
    let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
    assert_eq!(
        bounds(&l),
        (0.0, -5.0, 54.0, 30.0),
        "Start reaches from (0, -5) to (54, 30)"
    );
    let full = TaskbarLayout::compute(
        &theme.metrics,
        &theme.taskbar_shape(),
        1920,
        true,
        counts(0, 2, 0),
        30,
    );
    assert_eq!(
        bounds(&full),
        (0.0, 0.0, 54.0, 40.0),
        "on the 40 px bar Start is its button, (0, 0) to (54, 40)"
    );
}

#[test]
fn icon_buttons_past_the_row_are_left_out() {
    let mut l = TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        200,
        true,
        counts(0, 10, 0),
        30,
    );
    l.lay_tasks(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        &[1; 10],
        TaskRow::Icons,
    );
    assert_eq!(l.tasks_limit(), 146);
    assert_eq!(l.tasks.len(), 3);
    assert_eq!(l.stacks.len(), 3, "a stack for every laid-out button");
}

#[test]
fn an_at_least_row_stops_shrinking_and_leaves_the_rest_out() {
    let m = Metrics::default();
    let mut l = classic_row(20);
    let avail = l.tasks_limit() - l.tasks_x();
    assert!(l.tasks[0].width < 62, "20 shared buttons are narrower");
    l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1; 20], TaskRow::AtLeast(62));
    assert!(l.tasks.iter().all(|r| r.width == 62));
    assert_eq!(l.tasks.len() as i32, (avail + TASK_GAP) / (62 + TASK_GAP));
    assert!(l.tasks.last().unwrap().right() <= l.tasks_limit());
    let mut roomy = classic_row(2);
    roomy.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::AtLeast(62));
    assert_eq!(roomy.tasks, classic_row(2).tasks);
}

#[test]
fn an_at_least_row_never_passes_the_widest_button() {
    let m = Metrics {
        task_button_max_width: 40,
        ..Metrics::default()
    };
    let mut l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
    l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::AtLeast(62));
    assert!(l.tasks.iter().all(|r| r.width == 40), "{:?}", l.tasks);
}

#[test]
fn a_pinned_app_without_windows_takes_an_icon_slot_and_labels_share_the_rest() {
    let m = Metrics::default();
    let mut l = classic_row(3);
    l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[0, 1, 1], TaskRow::Shared);
    assert_eq!(
        l.tasks,
        vec![
            Rect::new(59, 4, 22, 22),
            Rect::new(84, 4, 160, 22),
            Rect::new(247, 4, 160, 22),
        ]
    );
    assert!(l.pinned_idle(0) && !l.pinned_idle(1) && !l.pinned_idle(2));
    let mut l = classic_row(8);
    let avail = l.tasks_limit() - l.tasks_x();
    l.lay_tasks(
        &m,
        &TaskbarShape::CLASSIC,
        &[1, 0, 1, 1, 1, 1, 1, 1],
        TaskRow::Shared,
    );
    assert_eq!(l.tasks[1].width, 22);
    assert_eq!(l.tasks[0].width, (avail - 25 - TASK_GAP * 6) / 7);
    assert_eq!(l.tasks[2].x, l.tasks[1].x + 25);
}

#[test]
fn a_glass_pinned_slot_is_an_icon_button() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let (m, shape) = (&theme.metrics, theme.taskbar_shape());
    let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 3, 0), 30);
    l.lay_tasks(m, &shape, &[0, 1, 0], TaskRow::Shared);
    let x = l.tasks_x();
    assert_eq!(l.tasks[0], Rect::new(x, 0, 60, 40));
    assert_eq!(l.tasks[1].x, x + 62);
    // Glass's labelled buttons abut each other, but a pinned icon
    // after one keeps the icon buttons' 2 px gap.
    assert_eq!(l.tasks[2], Rect::new(l.tasks[1].right() + 2, 0, 60, 40));
    assert_eq!(l.icon_step(&shape), (60, 62));
}

#[test]
fn glass_labelled_buttons_abut_160_px_wide_after_the_pinned_slots() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let (m, shape) = (&theme.metrics, theme.taskbar_shape());
    let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 6, 0), 30);
    l.lay_tasks(m, &shape, &[0, 0, 0, 1, 1, 1], TaskRow::AtLeast(62));
    let xs: Vec<i32> = l.tasks.iter().map(|r| r.x).collect();
    assert_eq!(xs, [59, 121, 183, 245, 405, 565]);
    for r in &l.tasks[3..] {
        assert_eq!((r.width, r.height), (160, 40), "a labelled slot at {}", r.x);
    }
}

#[test]
fn a_labelled_button_keeps_the_icon_gap_before_an_idle_pin() {
    use crate::interface::Interface;
    for (i, icon_gap) in [(Interface::Glass, 2), (Interface::Classic, 3)] {
        let theme = crate::interfaces::theme_for(i);
        let (m, shape) = (&theme.metrics, theme.taskbar_shape());
        for (width, row) in [(1920, TaskRow::AtLeast(62)), (700, TaskRow::Shared)] {
            let mut l = TaskbarLayout::compute(m, &shape, width, true, counts(0, 5, 0), 30);
            l.lay_tasks(m, &shape, &[1, 0, 1, 1, 0], row);
            assert_eq!(l.tasks.len(), 5, "{i:?} at {width}: every slot fits");
            let gaps: Vec<i32> = l.tasks.windows(2).map(|p| p[1].x - p[0].right()).collect();
            let labelled = shape.labelled.gap;
            assert_eq!(
                gaps,
                [icon_gap, icon_gap, labelled, icon_gap],
                "{i:?} at {width}: a pinned icon is {icon_gap} px from its neighbours"
            );
            assert!(l.tasks[4].right() <= l.tasks_limit(), "{i:?} at {width}");
        }
    }
}

#[test]
fn scaled_layout_keeps_the_stacks() {
    let mut l = classic_row(2);
    l.lay_tasks(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        &[3, 1],
        TaskRow::Icons,
    );
    let s = l.scaled(2);
    assert_eq!(s.tasks[0], l.tasks[0].scaled(2));
    assert_eq!(s.stacks, vec![3, 1]);
}

fn bar_1920(i: crate::interface::Interface, strip: bool) -> TaskbarLayout {
    let theme = crate::interfaces::theme_for(i);
    let c = TaskbarCounts {
        tasks: 2,
        tray: 2,
        show_desktop: strip,
        ..TaskbarCounts::default()
    };
    TaskbarLayout::compute(&theme.metrics, &theme.taskbar_shape(), 1920, true, c, 40)
}

#[test]
fn each_look_ends_its_bar_in_its_own_show_desktop_strip() {
    use crate::interface::Interface;
    for (i, strip, gap) in [
        (Interface::Classic, Rect::new(1896, 4, 22, 22), 2),
        (Interface::Blue, Rect::new(1895, 3, 25, 25), 12),
        (Interface::Glass, Rect::new(1905, 0, 15, 40), 2),
    ] {
        let (off, on) = (bar_1920(i, false), bar_1920(i, true));
        assert_eq!(off.show_desktop, None, "{i:?}: no strip with the key off");
        assert_eq!(
            off.clock.right(),
            1920 - gap,
            "{i:?}: the clock ends {gap} px in"
        );
        assert_eq!(on.show_desktop, Some(strip), "{i:?}: the strip's rect");
        assert_eq!(
            on.clock.right(),
            strip.x - gap,
            "{i:?}: the clock {gap} px before it"
        );
        let dx = off.clock.x - on.clock.x;
        assert_eq!(dx, 1920 - strip.x, "{i:?}: the clock moves by the strip");
        for (a, b) in off.tray.iter().zip(&on.tray) {
            assert_eq!(a.x - b.x, dx, "{i:?}: the tray moves with the clock");
        }
        assert_eq!(off.tasks_limit() - on.tasks_limit(), dx, "{i:?}: the row");
        assert!(
            on.notification.right() <= strip.x,
            "{i:?}: the well stops short"
        );
        assert_eq!(on.width, 1920, "{i:?}: the bar keeps its width");
        assert_eq!(on.scaled(2).show_desktop, Some(strip.scaled(2)));
    }
    let glass = bar_1920(Interface::Glass, true);
    assert_eq!(
        glass.notification.right(),
        1905,
        "Glass's well runs to the strip"
    );
}

#[test]
fn the_show_desktop_strip_takes_its_own_clicks() {
    use crate::interface::Interface;
    let l = bar_1920(Interface::Glass, true);
    assert_eq!(l.hit(1905, 0), TaskbarHit::ShowDesktop);
    assert_eq!(l.hit(1919, 39), TaskbarHit::ShowDesktop);
    assert_eq!(
        l.hit(1903, 20),
        TaskbarHit::Nothing,
        "the gap after the clock"
    );
    assert_eq!(l.hit(l.clock.x + 2, 20), TaskbarHit::Clock);
    let off = bar_1920(Interface::Glass, false);
    assert_eq!(off.hit(1919, 20), TaskbarHit::Nothing, "no strip, the well");
    let classic = bar_1920(Interface::Classic, true);
    assert_eq!(classic.hit(1900, 10), TaskbarHit::ShowDesktop);
    assert_eq!(
        classic.hit(1918, 10),
        TaskbarHit::Nothing,
        "the 2 px margin"
    );
}

#[test]
fn access_names_the_show_desktop_strip() {
    use crate::interface::Interface;
    let tree_of = |l: &TaskbarLayout| {
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(l, &TaskbarNames::default(), &mut t, access::ROOT);
        t
    };
    let l = bar_1920(Interface::Glass, true);
    let t = tree_of(&l);
    let id = t
        .find(Role::Button, "Show desktop")
        .expect("a Show desktop button");
    assert_eq!(
        t.get(id).unwrap().bounds().map(|b| b.x0),
        Some(1905.0),
        "at the strip"
    );
    let t = tree_of(&bar_1920(Interface::Glass, false));
    assert!(
        t.find(Role::Button, "Show desktop").is_none(),
        "none without it"
    );
}

#[test]
fn scaled_layout_scales_tray_too() {
    let m = Metrics::default();
    let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
    let s = l.scaled(2);
    assert_eq!(s.tray[0], l.tray[0].scaled(2));
    assert_eq!(s.tray.len(), 2);
}
