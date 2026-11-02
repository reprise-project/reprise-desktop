use super::*;
use crate::harness::Harness;
use std::collections::BTreeMap;

const CELL: desktop::Cell = desktop::Cell::square(desktop::CELL);

#[test]
fn the_store_reads_back_what_it_wrote_and_starts_over_on_a_bad_file() {
    let mut s = Store {
        face: Face::Digital,
        seconds: true,
        meter: MeterStyle::Graph,
        ..Store::default()
    };
    s.homes.insert(Kind::Clock, "Reprise|Test|0".into());
    let cells = BTreeMap::from([
        ("Reprise|Test|0".to_string(), (9, 0)),
        ("DP-2".into(), (0, 3)),
    ]);
    s.cells.insert(Kind::Clock, cells);
    s.cells
        .insert(Kind::Meter, BTreeMap::from([("DP-2".to_string(), (2, 2))]));
    let text = s.render();
    assert!(text.contains("face = \"digital\""), "{text}");
    assert_eq!(Store::parse(&text), s);
    assert_eq!(Store::parse("clock = 3\n"), Store::default());
    assert_eq!(
        Store::parse("[clock]\nface = \"sundial\"\n").face,
        Face::Analog,
        "an unknown face reads as the default"
    );
}

#[test]
fn a_new_widget_takes_the_top_right_clear_of_icons_and_a_saved_one_keeps_its_cell() {
    // 10 columns, 8 rows.
    let area = Rect::new(0, 0, 750, 600);
    let icons = HashSet::from([(9, 0), (9, 1)]);
    let got = arrange(
        CELL,
        area,
        &[
            Placing {
                kind: Kind::Clock,
                size: (129, 129),
                saved: None,
            },
            Placing {
                kind: Kind::Meter,
                size: (210, 54),
                saved: None,
            },
            Placing {
                kind: Kind::Calendar,
                size: (178, 166),
                saved: Some((0, 0)),
            },
        ],
        &icons,
    );
    let spots: Vec<_> = got.iter().map(|(s, new)| (s.cell, s.span, *new)).collect();
    assert_eq!(
        spots,
        [
            ((7, 0), (2, 2), true),
            ((7, 2), (3, 1), true),
            ((0, 0), (3, 3), false),
        ]
    );
}

#[test]
fn a_saved_spot_another_widget_holds_moves_to_the_nearest_free_one() {
    let area = Rect::new(0, 0, 750, 600);
    let got = arrange(
        CELL,
        area,
        &[
            Placing {
                kind: Kind::Clock,
                size: (129, 129),
                saved: Some((4, 4)),
            },
            Placing {
                kind: Kind::Meter,
                size: (210, 54),
                saved: Some((4, 4)),
            },
        ],
        &HashSet::new(),
    );
    assert_eq!(got[0].0.cell, (4, 4));
    assert_eq!(got[1].0.cell, (4, 3), "just above the clock");
}

#[test]
fn a_dropped_widget_lands_on_the_cell_its_cells_corner_is_nearest() {
    // A 129 px panel spans two 75 px cells, centred 10 px in.
    assert_eq!(
        drop_corner(CELL, (2, 2), (129, 129), (160.0, 85.0)),
        (150 + 37, 75 + 37)
    );
    let at = |p: (i32, i32)| (p.0.div_euclid(75), p.1.div_euclid(75));
    assert_eq!(
        at(drop_corner(CELL, (2, 2), (129, 129), (160.0, 85.0))),
        (2, 1)
    );
    assert_eq!(
        at(drop_corner(CELL, (2, 2), (129, 129), (200.0, 85.0))),
        (3, 1)
    );
}

#[test]
fn the_widgets_menu_ticks_what_stands_and_a_widget_menu_ends_in_remove() {
    let (items, commands) = widgets_menu(&[Kind::Meter]);
    let ticks: Vec<bool> = (items.iter())
        .map(|i| matches!(i, MenuItem::Entry { checked: true, .. }))
        .collect();
    assert_eq!(ticks, [false, false, true]);
    assert_eq!(commands[0], Some(DesktopCommand::ToggleWidget(Kind::Clock)));
    let store = Store {
        face: Face::Digital,
        ..Store::default()
    };
    let (items, commands) = widget_menu(Kind::Clock, &store);
    assert!(
        matches!(&items[1], MenuItem::Entry { label, checked: true, radio: true, .. } if label == "Digital")
    );
    assert_eq!(
        commands.last(),
        Some(&Some(DesktopCommand::RemoveWidget(Kind::Clock)))
    );
    let (items, _) = widget_menu(Kind::Calendar, &store);
    assert_eq!(items.len(), 1, "the calendar has only Remove");
}

/// Widgets stand on the grid, the icons flow round them, a drag moves one, and a fresh session
/// reading the store puts them back.
#[test]
fn widgets_stand_clear_of_icons_move_by_drag_and_come_back_after_a_restart() {
    let mut h = Harness::new();
    let out = h.output("TEST-1", 1024, 768);
    h.state.setup_widgets();
    let desk = std::env::temp_dir().join(format!("reprise-desk-widgets-{}", std::process::id()));
    crate::desktopview::create(&mut h.state, desk, Default::default(), None);
    let names = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let view = h.state.desktop.as_mut().unwrap();
    for n in names {
        view.entries.push(desktop::DesktopEntry {
            name: n.into(),
            label: n.into(),
            icon: "text-x-generic".into(),
            target: desktop::Target::Open(n.into()),
            path: n.into(),
            shortcut: false,
            kind: desktop::Kind::File,
            is_dir: false,
            allow_launching: false,
        });
    }
    h.state.config.desktop_widgets = vec![Kind::Clock, Kind::Calendar, Kind::Meter];
    h.state.relayout_desktop();
    assert_eq!(h.state.widgets.shown.len(), 3);
    let grid = h.state.desktop_grid();
    let area = h.state.usable_area(Some(&out));
    let blocked = h.state.icon_blocked();
    let view = h.state.desktop.as_ref().unwrap();
    for r in &view.cells {
        let c = desktop::cell_at(grid, area, r.x, r.y);
        assert!(!blocked.contains(&c), "an icon at {c:?} under a widget");
    }
    let spots: Vec<Spot> = h.state.widgets.shown.iter().map(|s| s.spot).collect();
    for (i, a) in spots.iter().enumerate() {
        for b in &spots[i + 1..] {
            assert!(!a.overlaps(*b), "{a:?} and {b:?} overlap");
        }
    }

    let clock = h.state.widget_panel(Kind::Clock).unwrap();
    let grab = (f64::from(clock.x + 20), f64::from(clock.y + 20));
    h.pointer_to(grab.0, grab.1);
    h.button(crate::harness::LEFT, true);
    h.pointer_to(grab.0 - 30.0, grab.1);
    h.pointer_to(f64::from(area.x + 20), f64::from(area.y + 20));
    h.button(crate::harness::LEFT, false);
    let moved = h
        .state
        .widgets
        .shown
        .iter()
        .find(|s| s.kind == Kind::Clock)
        .unwrap()
        .spot;
    assert_eq!(moved.cell, (0, 0), "the clock dropped at the top-left cell");
    let view = h.state.desktop.as_ref().unwrap();
    let first = desktop::cell_at(grid, area, view.cells[0].x, view.cells[0].y);
    assert!(
        !moved.cells().any(|c| c == first),
        "the first icon made way: {first:?}"
    );
    let before: Vec<(Kind, Spot)> = h
        .state
        .widgets
        .shown
        .iter()
        .map(|s| (s.kind, s.spot))
        .collect();
    crate::state::flush();

    let mut again = Harness::new();
    again.output("TEST-1", 1024, 768);
    again.state.setup_widgets();
    again.state.config.desktop_widgets = vec![Kind::Clock, Kind::Calendar, Kind::Meter];
    again.state.relayout_desktop();
    let after: Vec<(Kind, Spot)> = again
        .state
        .widgets
        .shown
        .iter()
        .map(|s| (s.kind, s.spot))
        .collect();
    assert_eq!(after, before, "every widget back where it stood");
    let _ = std::fs::remove_file(store::path());
}

#[test]
fn the_meter_samples_only_while_its_desktop_shows() {
    let mut h = Harness::new();
    h.output("TEST-1", 1024, 768);
    h.state.config.desktop_widgets = vec![Kind::Meter];
    h.state.relayout_desktop();
    h.state.sample_meter();
    assert_eq!(
        h.state.widgets.meter.memory.len(),
        1,
        "a sample on a showing desktop"
    );
    h.state.widgets.cpu_before = None;
    crate::lockview::lock(&mut h.state);
    h.state.sample_meter();
    assert_eq!(h.state.widgets.meter.memory.len(), 1, "none while locked");
}
