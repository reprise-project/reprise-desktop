use super::*;

#[test]
fn access_names_the_buttons_and_presses_the_active_task() {
    let l = TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        800,
        true,
        counts(1, 2, 1),
        40,
    );
    let names = TaskbarNames {
        start_open: true,
        quick: vec!["Show Desktop".into()],
        tasks: vec![("One".into(), true), ("A & B".into(), false)],
        tray: vec!["Volume".into()],
        clock: "12:00".into(),
        date: "Friday".into(),
        ..TaskbarNames::default()
    };
    let mut t = Tree::window("Taskbar", (l.width, l.height));
    access(&l, &names, &mut t, access::ROOT);
    let start = t.find(Role::Button, "Start").unwrap();
    assert_eq!(t.get(start).unwrap().is_expanded(), Some(true));
    let one = t.find(Role::Button, "One").unwrap();
    assert_eq!(
        t.get(one).unwrap().toggled(),
        Some(Toggled::True),
        "the active window's button is pressed"
    );
    assert!(
        t.find(Role::Button, "A & B").is_some(),
        "an & in a title stays"
    );
    assert!(t.find(Role::Button, "Show Desktop").is_some());
    assert!(t.find(Role::Button, "Volume").is_some());
    let clock = t.find(Role::Button, "12:00").unwrap();
    assert_eq!(t.get(clock).unwrap().description(), Some("Friday"));
    assert_eq!(t.focus(), access::ROOT, "the bar has no keyboard");
}

#[test]
fn a_reader_meets_the_bands_in_the_order_they_are_drawn() {
    let c = TaskbarCounts {
        quick: 2,
        pager: 2,
        tasks: 2,
        tray: 1,
        ..TaskbarCounts::default()
    };
    let names = TaskbarNames {
        quick: vec!["A".into(), "B".into()],
        tasks: vec![("T0".into(), false), ("T1".into(), false)],
        tray: vec!["Volume".into()],
        ..TaskbarNames::default()
    };
    for (tasks_first, want) in [
        (false, ["Desktops", "Quick Launch", "Running Applications"]),
        (true, ["Running Applications", "Desktops", "Quick Launch"]),
    ] {
        let bands = Bands {
            tasks_first,
            first_width: None,
        };
        let l = TaskbarLayout::compute_with(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            c,
            40,
            bands,
        );
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        let bars: Vec<(f64, &str)> = (t.get(access::ROOT).unwrap().children().iter())
            .map(|&id| t.get(id).unwrap())
            .filter(|n| n.role() == Role::Toolbar)
            .map(|n| (n.bounds().unwrap().x0, n.label().unwrap()))
            .collect();
        let names: Vec<&str> = bars.iter().map(|b| b.1).take(3).collect();
        assert_eq!(names, want, "tasks first: {tasks_first}");
        assert!(
            bars.iter().map(|b| b.0).is_sorted(),
            "left to right as drawn: {bars:?}"
        );
    }
}

#[test]
fn access_focuses_the_keyboards_item_and_says_who_wants_attention() {
    let l = TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        800,
        true,
        counts(0, 2, 1),
        40,
    );
    let names = TaskbarNames {
        tasks: vec![("One".into(), true), ("Two".into(), false)],
        attention: vec![false, true],
        tray: vec!["Volume".into()],
        focus: Some(TaskbarHit::Task(1)),
        ..TaskbarNames::default()
    };
    let mut t = Tree::window("Taskbar", (l.width, l.height));
    access(&l, &names, &mut t, access::ROOT);
    let two = t.find(Role::Button, "Two").unwrap();
    assert_eq!(t.focus(), two, "the keyboard is on the second task");
    assert_eq!(t.get(two).unwrap().description(), Some(ATTENTION));
    let one = t.find(Role::Button, "One").unwrap();
    assert_eq!(t.get(one).unwrap().description(), None);
    assert!(
        t.get(two)
            .unwrap()
            .supports_action(crate::access::accesskit::Action::Click),
        "a reader can click it"
    );
    let names = TaskbarNames {
        tray: vec!["Volume".into()],
        focus: Some(TaskbarHit::Tray(0)),
        ..names
    };
    let mut t = Tree::window("Taskbar", (l.width, l.height));
    access(&l, &names, &mut t, access::ROOT);
    assert_eq!(Some(t.focus()), t.find(Role::Button, "Volume"));
}
