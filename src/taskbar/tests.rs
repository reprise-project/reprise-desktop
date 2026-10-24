use super::*;

#[test]
fn date_text_is_the_long_date() {
    let s = date_text();
    let (_, rest) = s.split_once(", ").expect("weekday");
    let (_, year) = rest.rsplit_once(", ").expect("year");
    assert_eq!(year.len(), 4);
    assert!(year.chars().all(|c| c.is_ascii_digit()));
}
use reprisekit::taskbar::TASK_GAP;

#[test]
fn spot_redraw_waits_for_the_tick() {
    assert_eq!(
        spot_due(Some(10), Some(10), false),
        SpotStep::Nothing,
        "same x"
    );
    assert_eq!(
        spot_due(Some(10), Some(11), false),
        SpotStep::Arm,
        "first move arms"
    );
    assert_eq!(
        spot_due(Some(10), Some(12), true),
        SpotStep::Wait,
        "armed: the tick draws it"
    );
    assert_eq!(
        spot_due(None, Some(12), false),
        SpotStep::Nothing,
        "a new hot button redraws anyway"
    );
}

#[test]
#[ignore]
fn render_taskbar_menu() {
    use crate::raster::Canvas;
    use crate::startmenu::{MenuLayout, MenuStyle, draw_menu};
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let (items, _) = taskbar_menu(true, false);
    let style =
        MenuStyle::context_for(&theme, &items).fit(&text, &items, theme.fonts.size_px, i32::MAX);
    let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(layout.width as u32, layout.height as u32);
    draw_menu(&mut c, &theme, &text, &layout, &items, None, "", false);
    crate::raster::write_preview(&c, "taskbarmenu-preview.png");
}

/// Writes target/windowlist-preview.png for a visual check.
#[test]
#[ignore]
fn render_window_list() {
    use crate::raster::Canvas;
    use crate::startmenu::{MenuLayout, MenuStyle, draw_menu};
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let titles = ["~/projects - foot", "~/notes - foot", "htop - foot"].map(String::from);
    let (items, _) = window_list(&titles, 4);
    let style = MenuStyle::submenu(&theme).fit(&text, &items, theme.fonts.size_px, i32::MAX);
    let layout = MenuLayout::compute_styled(&style, &items, usize::MAX);
    let mut c = Canvas::new(layout.width as u32, layout.height as u32);
    draw_menu(&mut c, &theme, &text, &layout, &items, None, "", false);
    crate::raster::write_preview(&c, "windowlist-preview.png");
}

#[test]
fn taskbar_menu_lists_its_rows() {
    let (items, commands) = taskbar_menu(true, true);
    assert_eq!(items.len(), 11);
    assert_eq!(commands[0], Some(TaskbarCommand::Toolbars));
    assert!(matches!(items[0], MenuItem::Entry { submenu: true, .. }));
    assert!(matches!(items[1], MenuItem::Separator));
    assert_eq!(commands[2], Some(TaskbarCommand::Cascade));
    assert_eq!(commands[5], Some(TaskbarCommand::ShowDesktop));
    assert!(matches!(items[6], MenuItem::Separator));
    assert_eq!(commands[7], Some(TaskbarCommand::TaskManager));
    assert!(matches!(items[8], MenuItem::Separator));
    assert_eq!(commands[9], Some(TaskbarCommand::LockTaskbar));
    assert!(matches!(items[9], MenuItem::Entry { checked: true, .. }));
    assert_eq!(commands[10], Some(TaskbarCommand::Properties));
    for it in &items[2..6] {
        assert!(matches!(it, MenuItem::Entry { enabled: true, .. }));
    }
    let (items, _) = taskbar_menu(false, false);
    assert!(matches!(items[9], MenuItem::Entry { checked: false, .. }));
    for it in &items[2..5] {
        assert!(matches!(it, MenuItem::Entry { enabled: false, .. }));
    }
    assert!(matches!(items[5], MenuItem::Entry { enabled: true, .. }));
    let (rows, cmds) = toolbars_menu(true, false);
    assert_eq!(cmds, vec![Some(TaskbarCommand::QuickLaunch)]);
    assert!(matches!(rows[0], MenuItem::Entry { checked: true, .. }));
    assert!(matches!(
        toolbars_menu(false, false).0[0],
        MenuItem::Entry {
            checked: false,
            enabled: true,
            ..
        }
    ));
    assert!(
        matches!(
            toolbars_menu(true, true).0[0],
            MenuItem::Entry { enabled: false, .. }
        ),
        "greyed with the programs pinned to the task row"
    );
}

#[test]
fn clock_format() {
    // Locale-dependent: a colon between two runs of digits and no stray padding.
    let s = clock_text();
    assert_eq!(s, s.trim());
    let (h, rest) = s.split_once(':').expect("HH:MM");
    assert!(h.chars().all(|c| c.is_ascii_digit()) && !h.is_empty());
    assert!(rest.as_bytes()[..2].iter().all(u8::is_ascii_digit));
}

fn lay_buttons(
    layout: &mut TaskbarLayout,
    m: &reprisekit::theme::Metrics,
    shape: &TaskbarShape,
    apps: &[String],
    mode: TaskbarButtons,
) -> Vec<Vec<usize>> {
    lay_slots(layout, m, shape, apps, &ends(apps.len()), &[], 0, &[], mode)
        .into_iter()
        .map(|s| s.windows)
        .collect()
}

fn keys(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_grip_drags_along_its_bar() {
    use reprisekit::taskbar::Edge;
    assert_eq!(grip_cursor(Edge::Bottom), "ew-resize");
    assert_eq!(grip_cursor(Edge::Top), "ew-resize");
    assert_eq!(grip_cursor(Edge::Left), "ns-resize");
    assert_eq!(grip_cursor(Edge::Right), "ns-resize");
}

/// Up a side, when_full groups an app's windows once the column holds no more full buttons, not at
/// `GROUP_BELOW`; along the bottom the same windows stay apart.
#[test]
fn a_side_bar_groups_when_its_column_is_full() {
    use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge};
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let apps = keys(&["a", "a", "b", "c", "d", "e", "f", "g", "h", "i"]);
    let counts = TaskbarCounts {
        tasks: apps.len(),
        ..TaskbarCounts::default()
    };
    let column = |length: i32| {
        let span = BarSpan {
            edge: Edge::Left,
            length,
            thickness: 60,
        };
        TaskbarLayout::compute_on(
            &m,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        )
    };
    let mut side = column(200);
    let plan = lay_buttons(&mut side, &m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(plan.len(), 9, "the two a windows share a button");
    // 27..370 down a 400 px column: 31 px a button, under GROUP_BELOW, yet room for ten full buttons.
    let mut tall = column(400);
    let plan = lay_buttons(&mut tall, &m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(
        plan.len(),
        10,
        "ten full buttons fit: each window keeps its own"
    );
    let mut row = TaskbarLayout::compute(&m, &shape, 1920, true, counts, 30);
    let plan = lay_buttons(&mut row, &m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(plan.len(), 10, "a 1920 px row has room for each");
}

/// Glass's 60 px column is icons only, buttons 2 px apart: an app's windows share a button once full
/// buttons no longer fit, so none waits behind the » cell.
#[test]
fn an_icon_only_glass_column_groups_by_its_icon_buttons_gap() {
    use reprisekit::interface::Interface;
    use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge};
    let glass = reprisekit::interfaces::theme_for(Interface::Glass);
    let (m, shape) = (&glass.metrics, glass.taskbar_shape());
    let column = |tasks: usize| {
        let span = BarSpan {
            edge: Edge::Left,
            length: 800,
            thickness: 60,
        };
        let counts = TaskbarCounts {
            tasks,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute_on(
            m,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        )
    };
    let l = column(1);
    assert!(l.icons_only, "Glass's 60 px column: icons only");
    let room = l.tasks_limit() - l.tasks_x();
    let (button, pitch) = l.icon_step(&shape);
    assert_eq!(pitch - button, 2, "the icon buttons' gap");
    let n = room / button;
    assert!(n * pitch - 2 > room, "{n} buttons in {room} px");
    let mut apps = keys(&["a", "a"]);
    apps.extend((2..n).map(|i| i.to_string()));
    let mut l = column(apps.len());
    let plan = lay_buttons(&mut l, m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(
        plan.len(),
        apps.len() - 1,
        "the two a windows share a button"
    );
    assert_eq!(l.tasks.len(), plan.len(), "every button shows");
    assert_eq!(l.more, None);
}

#[test]
fn a_two_line_clock_puts_the_date_under_the_time() {
    assert_eq!(bar_clock_text(false), clock_text(), "one line: the time");
    let two = bar_clock_text(true);
    let (time, date) = two.split_once('\n').expect("two lines");
    assert_eq!(time, clock_text());
    assert!(
        date.chars().any(|c| c.is_ascii_digit()) && !date.contains('\n'),
        "the date is one line of digits and separators, not {date:?}"
    );
}

#[test]
fn plan_buttons_groups_only_when_the_bar_is_full() {
    let apps = keys(&["a", "b", "a", "c"]);
    let when_full = TaskbarButtons::WhenFull;
    let wide = plan_buttons(&apps, 1000, GROUP_BELOW, TASK_GAP, when_full);
    assert_eq!(wide, vec![vec![0], vec![1], vec![2], vec![3]]);
    // Tight: the two-window app "a" collapses at its first slot, the singles stay put.
    let tight = plan_buttons(&apps, 120, GROUP_BELOW, TASK_GAP, when_full);
    assert_eq!(tight, vec![vec![0, 2], vec![1], vec![3]]);
}

#[test]
fn plan_buttons_handles_empty_and_all_grouped() {
    let when_full = TaskbarButtons::WhenFull;
    assert!(plan_buttons(&[], 100, GROUP_BELOW, TASK_GAP, when_full).is_empty());
    let same: Vec<String> = vec!["x".into(); 5];
    let tight = plan_buttons(&same, 100, GROUP_BELOW, TASK_GAP, when_full);
    assert_eq!(tight, vec![vec![0, 1, 2, 3, 4]]);
}

#[test]
fn combine_groups_every_app_with_two_or_more_windows_however_roomy() {
    let apps = keys(&["a", "b", "a", "c", "b", "a"]);
    let want = vec![vec![0, 2, 5], vec![1, 4], vec![3]];
    for avail in [120, 1000, 10_000] {
        assert_eq!(
            plan_buttons(&apps, avail, GROUP_BELOW, TASK_GAP, TaskbarButtons::Combine),
            want,
            "one button per app in first-window order at {avail} px"
        );
    }
    assert!(plan_buttons(&[], 1000, GROUP_BELOW, TASK_GAP, TaskbarButtons::Combine).is_empty());
}

#[test]
fn never_gives_every_window_its_own_button_however_tight() {
    let apps = keys(&["a", "b", "a", "a"]);
    for avail in [0, 120, 1000] {
        assert_eq!(
            plan_buttons(&apps, avail, GROUP_BELOW, TASK_GAP, TaskbarButtons::Never),
            vec![vec![0], vec![1], vec![2], vec![3]],
            "one button per window at {avail} px"
        );
    }
}

#[test]
fn each_buttons_mode_lays_its_own_row() {
    use crate::config::Config;
    let apps = keys(&["a", "b", "a", "c"]);
    let row = |text: &str| {
        let cfg = Config::parse(text).unwrap();
        let (m, shape) = (&cfg.theme.metrics, cfg.theme.taskbar_shape());
        let counts = TaskbarCounts {
            tasks: apps.len(),
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(m, &shape, 800, true, counts, 30);
        let plan = lay_buttons(&mut l, m, &shape, &apps, cfg.taskbar.buttons);
        (plan, l.tasks, l.stacks)
    };
    let (plan, tasks, stacks) = row("");
    assert_eq!(plan.len(), 4, "when_full has room for every window");
    assert_eq!(tasks[1], Rect::new(222, 4, 160, 22));
    assert_eq!(stacks, vec![1; 4]);
    let (plan, tasks, stacks) = row("[taskbar]\nbuttons = \"combine\"\n");
    assert_eq!(plan, vec![vec![0, 2], vec![1], vec![3]]);
    assert_eq!(
        tasks,
        vec![
            Rect::new(59, 4, 22, 22),
            Rect::new(84, 4, 22, 22),
            Rect::new(109, 4, 22, 22),
        ],
        "square icon buttons"
    );
    assert_eq!(stacks, vec![2, 1, 1]);
    let (plan, tasks, _) = row("[taskbar]\nbuttons = \"never\"\n");
    assert_eq!(plan.len(), 4);
    assert_eq!(tasks[1].width, 160);
}

#[test]
fn never_stops_shrinking_where_when_full_would_group() {
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let apps: Vec<String> = vec!["x".into(); 12];
    let counts = TaskbarCounts {
        tasks: apps.len(),
        ..TaskbarCounts::default()
    };
    let mut l = TaskbarLayout::compute(&m, &shape, 600, true, counts, 30);
    let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::Never);
    assert_eq!(plan.len(), 12, "a button per window");
    assert!(l.tasks.iter().all(|r| r.width == GROUP_BELOW));
    assert!(l.tasks.len() < 12, "the rest are left out");
    let mut l = TaskbarLayout::compute(&m, &shape, 600, true, counts, 30);
    let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(
        plan,
        vec![(0..12).collect::<Vec<_>>()],
        "one grouped button"
    );
    assert_eq!(l.stacks, vec![12]);
}

/// A top bar lays its task row after mirroring, so the buttons keep the screen-edge gap against the top.
#[test]
fn a_top_bar_lays_its_task_row_against_the_screen_edge() {
    use reprisekit::taskbar::Edge;
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let apps = keys(&["a", "b", "a", "c"]);
    let counts = TaskbarCounts {
        tasks: apps.len(),
        ..TaskbarCounts::default()
    };
    let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30).on_edge(Edge::Top);
    let plan = lay_buttons(&mut l, &m, &shape, &apps, TaskbarButtons::WhenFull);
    assert_eq!(plan.len(), 4);
    assert_eq!(l.tasks[1], Rect::new(222, 2, 160, 22));
    assert_eq!(l.edge, Edge::Top);
}

fn pin(file: &str, class: Option<&str>) -> QuickItem {
    let class = class.map_or(String::new(), |c| format!("StartupWMClass={c}\n"));
    let text = format!("[Desktop Entry]\nType=Application\nName={file}\nExec=x\n{class}");
    QuickItem {
        file: file.into(),
        entry: crate::quicklaunch::parse(&text, file, "foot").unwrap(),
        app: reprisefs::apps::parse_desktop(&text, file),
    }
}

#[test]
fn a_window_finds_its_pin_by_desktop_id_then_class_then_file_name() {
    let pins = [
        pin("foot.desktop", None),
        pin("footclient.desktop", None),
        pin("code.desktop", None),
        pin("vscode.desktop", Some("Code")),
        pin("org.gnome.Nautilus.desktop", None),
    ];
    assert_eq!(
        pin_for(
            &pins,
            "org.gnome.Nautilus",
            Some("org.gnome.Nautilus.desktop")
        ),
        Some(4)
    );
    assert_eq!(pin_for(&pins, "FOOT", None), Some(0), "by file name");
    assert_eq!(
        pin_for(&pins, "foot", Some("footclient.desktop")),
        Some(1),
        "the desktop id wins over a file name"
    );
    assert_eq!(
        pin_for(&pins, "code", None),
        Some(3),
        "the class wins over a file name"
    );
    assert_eq!(pin_for(&pins, "alacritty", None), None);
    assert_eq!(pin_for(&pins, "", Some("foot.desktop")), None);
}

fn ends(n: usize) -> Vec<Place> {
    (0..n).map(|i| Place::end(i as u64)).collect()
}
fn order(slots: &[Slot]) -> Vec<String> {
    slots
        .iter()
        .map(|s| match (s.pin, s.dialog) {
            (Some(k), _) => format!("p{k}"),
            (_, Some(d)) => format!("d{d}"),
            _ => format!("w{}", s.windows[0]),
        })
        .collect()
}

#[test]
fn an_anchored_button_sits_after_that_many_pins() {
    let places = [Place { anchor: 0, seq: 5 }, Place { anchor: 1, seq: 6 }];
    let slots = merge_row(vec![vec![0], vec![1]], 2, &[None, None], &places, &[]);
    assert_eq!(order(&slots), ["w0", "p0", "w1", "p1"]);
}

#[test]
fn a_new_window_lands_after_a_moved_one() {
    let places = [Place { anchor: 0, seq: 9 }, Place::end(3), Place::end(10)];
    let slots = merge_row(vec![vec![0], vec![1], vec![2]], 1, &[None; 3], &places, &[]);
    assert_eq!(order(&slots), ["w0", "p0", "w1", "w2"]);
}

#[test]
fn an_anchor_past_the_pins_sits_at_the_end() {
    let places = [Place { anchor: 5, seq: 1 }, Place::end(2)];
    let slots = merge_row(vec![vec![0], vec![1]], 1, &[None, None], &places, &[]);
    assert_eq!(order(&slots), ["p0", "w0", "w1"]);
}

#[test]
fn a_group_sits_at_its_earliest_window() {
    let places = [Place::end(4), Place::end(5), Place { anchor: 0, seq: 1 }];
    let slots = merge_row(vec![vec![0, 2], vec![1]], 1, &[None; 3], &places, &[]);
    assert_eq!(order(&slots), ["w0", "p0", "w1"]);
}

#[test]
fn a_dialog_takes_its_own_place() {
    let places = [Place::end(1), Place::end(3)];
    let dialogs = [DialogTask {
        id: 7,
        place: Place::end(2),
    }];
    let slots = merge_row(vec![vec![0], vec![1]], 0, &[None, None], &places, &dialogs);
    assert_eq!(order(&slots), ["w0", "d7", "w1"]);
}

#[test]
fn pinned_slots_lead_in_pin_order_and_unmatched_apps_follow() {
    let pin_of = [None, Some(0), None, Some(0), Some(0)];
    let slot = |windows: &[usize], pin: Option<usize>| Slot {
        windows: windows.to_vec(),
        pin,
        dialog: None,
    };
    let combined = merge_row(
        vec![vec![0], vec![1, 3, 4], vec![2]],
        2,
        &pin_of,
        &ends(5),
        &[],
    );
    assert_eq!(
        combined,
        [
            slot(&[1, 3, 4], Some(0)),
            slot(&[], Some(1)),
            slot(&[0], None),
            slot(&[2], None),
        ]
    );
    let plan = (0..5).map(|i| vec![i]).collect();
    assert_eq!(
        merge_row(plan, 2, &pin_of, &ends(5), &[]),
        [
            slot(&[1], Some(0)),
            slot(&[3], Some(0)),
            slot(&[4], Some(0)),
            slot(&[], Some(1)),
            slot(&[0], None),
            slot(&[2], None),
        ]
    );
    assert_eq!(
        merge_row(vec![vec![0]], 2, &[Some(0)], &ends(1), &[]),
        [slot(&[0], Some(0)), slot(&[], Some(1))]
    );
    assert_eq!(
        merge_row(Vec::new(), 2, &[], &[], &[]),
        [slot(&[], Some(0)), slot(&[], Some(1))]
    );
    assert_eq!(
        merge_row(
            vec![vec![1], vec![0]],
            0,
            &[],
            &[Place::end(1), Place::end(0)],
            &[]
        ),
        [slot(&[1], None), slot(&[0], None)]
    );
}

#[test]
fn lay_slots_puts_idle_pins_in_icon_slots() {
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let apps = keys(&["foot", "kitty", "foot"]);
    let pin_of = [Some(1), None, Some(1)];
    let counts = TaskbarCounts {
        tasks: 4,
        ..TaskbarCounts::default()
    };
    let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
    let slots = lay_slots(
        &mut l,
        &m,
        &shape,
        &apps,
        &ends(3),
        &pin_of,
        2,
        &[],
        TaskbarButtons::WhenFull,
    );
    let windows: Vec<Vec<usize>> = slots.iter().map(|s| s.windows.clone()).collect();
    assert_eq!(windows, [vec![], vec![0], vec![2], vec![1]]);
    assert_eq!(l.stacks, [0, 1, 1, 1]);
    assert_eq!(l.tasks[0], Rect::new(59, 4, 22, 22), "the idle pin");
    assert_eq!(l.tasks[1], Rect::new(84, 4, 160, 22));
    let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
    lay_slots(
        &mut l,
        &m,
        &shape,
        &apps,
        &ends(3),
        &pin_of,
        2,
        &[],
        TaskbarButtons::Combine,
    );
    assert_eq!(l.stacks, [0, 2, 1]);
    assert!(l.tasks.iter().all(|r| r.width == 22));
}

#[test]
fn a_dragged_button_drops_among_the_row_units() {
    let tasks = [
        Rect::new(0, 0, 60, 40),
        Rect::new(62, 0, 60, 40),
        Rect::new(124, 0, 60, 40),
        Rect::new(186, 0, 60, 40),
    ];
    let pins = [Some(0), Some(0), Some(1), None];
    assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 10), 0);
    assert_eq!(
        row_drop_slot(&tasks, &pins, &[], 3, 60),
        0,
        "pin 0 spans 0..122"
    );
    assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 61), 1);
    assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 150), 1);
    assert_eq!(row_drop_slot(&tasks, &pins, &[], 3, 160), 2);
    assert_eq!(
        row_drop_slot(&tasks, &pins, &[], 3, 400),
        2,
        "the unpinned button stays last"
    );
    let mut order = vec!["foot", "files"];
    assert!(move_button(
        &mut order,
        0,
        row_drop_slot(&tasks, &pins, &[], 3, 160)
    ));
    assert_eq!(order, ["files", "foot"]);
}

#[test]
fn a_reorder_slides_the_buttons_whose_x_changed_from_their_old_x() {
    let k = TaskKey::<u8>::Window;
    let old = [(k(1), 0), (k(2), 62), (TaskKey::Pin(0), 124), (k(3), 186)];
    let new = [(k(2), 0), (k(1), 62), (TaskKey::Pin(0), 124), (k(4), 186)];
    assert_eq!(
        moved_buttons(&old, &new),
        [(k(2), 62, 0), (k(1), 0, 62)],
        "the pin stays put; a button new to the row or gone from it slides nowhere"
    );
}

#[test]
fn a_reorder_names_a_pin_by_its_new_place() {
    let order = [2, 0, 1];
    assert_eq!(
        pin_renamed(TaskKey::<u8>::Pin(0), &order),
        Some(TaskKey::Pin(1))
    );
    assert_eq!(
        pin_renamed(TaskKey::<u8>::Pin(2), &order),
        Some(TaskKey::Pin(0))
    );
    assert_eq!(pin_renamed(TaskKey::<u8>::Pin(5), &order), None);
    assert_eq!(
        pin_renamed(TaskKey::Window(7u8), &order),
        Some(TaskKey::Window(7))
    );
    assert_eq!(
        pin_renamed(TaskKey::<u8>::Dialog(3), &order),
        Some(TaskKey::Dialog(3))
    );
}

#[test]
fn a_reorder_keeps_the_lit_floated_and_sliding_pins() {
    let order = [1, 0];
    let mut hot = Some(TaskKey::<u8>::Pin(0));
    let mut float = Some((TaskKey::<u8>::Pin(0), 40));
    let mut slides = vec![(TaskKey::<u8>::Pin(1), 62), (TaskKey::Window(7u8), 9)];
    pins_renamed(&mut hot, &mut float, &mut slides, &order);
    assert_eq!(hot, Some(TaskKey::Pin(1)), "the lit pin is now pin 1");
    assert_eq!(float, Some((TaskKey::Pin(1), 40)), "the float is now pin 1");
    assert_eq!(
        slides,
        [(TaskKey::Pin(0), 62), (TaskKey::Window(7), 9)],
        "the neighbour slides as pin 0, a window keeps its key"
    );

    let mut hot = Some(TaskKey::<u8>::Pin(2));
    let mut float = Some((TaskKey::<u8>::Pin(2), 0));
    let mut slides = vec![(TaskKey::<u8>::Pin(2), 0)];
    pins_renamed(&mut hot, &mut float, &mut slides, &order);
    assert_eq!(
        (hot, float, slides.len()),
        (None, None, 0),
        "a pin gone from the order drops out"
    );
}

#[test]
fn a_dragged_button_floats_where_it_was_grabbed_inside_the_row() {
    let tasks = [
        Rect::new(100, 0, 60, 40),
        Rect::new(162, 0, 60, 40),
        Rect::new(224, 0, 30, 40),
    ];
    assert_eq!(float_x(&tasks, 0..1, 0, 150, 20), Some(130));
    assert_eq!(
        float_x(&tasks, 0..1, 0, 50, 20),
        Some(100),
        "not before the row"
    );
    assert_eq!(
        float_x(&tasks, 0..1, 0, 400, 20),
        Some(194),
        "its right edge stops at the row's end"
    );
    assert_eq!(
        float_x(&tasks, 2..3, 2, 400, 5),
        Some(224),
        "a narrow one too"
    );
    assert_eq!(float_x(&tasks, 3..4, 3, 150, 20), None);
}

#[test]
fn a_pins_buttons_move_as_one_unit() {
    let pins = [Some(0), Some(0), None, Some(1)];
    assert_eq!(row_units(&pins, &[]), [0..2, 2..3, 3..4]);
}

#[test]
fn an_unpinned_button_dropped_before_the_pins_anchors_at_0() {
    let pins = [Some(0), Some(1), None];
    let (pin_order, anchors) = reorder_row(&pins, &[], 2, 0).unwrap();
    assert_eq!(pin_order, [0, 1]);
    assert_eq!(anchors, [(2..3, 0)]);
}

#[test]
fn a_pin_dragged_past_an_unpinned_button_reorders_both() {
    let pins = [Some(0), None, Some(1)];
    let (pin_order, anchors) = reorder_row(&pins, &[], 0, 3).unwrap();
    assert_eq!(pin_order, [1, 0], "p1 first, then p0");
    assert_eq!(anchors, [(1..2, 0)], "w now precedes both pins");
}

#[test]
fn a_pin_added_after_a_drag_joins_the_pins() {
    let pins = [Some(0), Some(1), None, None];
    let (pin_order, anchors) = reorder_row(&pins, &[], 2, 1).unwrap();
    assert_eq!(pin_order, [0, 1]);
    assert_eq!(anchors, [(2..3, 1), (3..4, usize::MAX)]);
    let places = [
        Place { anchor: 1, seq: 10 },
        Place {
            anchor: anchors[1].1,
            seq: 11,
        },
    ];
    let slots = merge_row(vec![vec![0], vec![1]], 3, &[None, None], &places, &[]);
    assert_eq!(order(&slots), ["p0", "w0", "p1", "p2", "w1"]);
}

#[test]
fn a_drop_in_place_moves_nothing() {
    let pins = [Some(0), None];
    assert_eq!(reorder_row(&pins, &[], 1, 1), None);
    assert_eq!(reorder_row(&pins, &[], 1, 2), None);
}

#[test]
fn with_pins_every_task_button_acts_on_release_and_without_on_press() {
    assert_eq!(press_timing(true), PressTiming::OnRelease);
    assert_eq!(press_timing(false), PressTiming::OnPress);
}

/// A held button is found again by its window or pin after the row is laid out anew.
#[test]
fn a_held_task_button_is_found_again_after_the_row_changes() {
    let buttons = vec![vec![10], vec![20], vec![]];
    let pins = vec![None, None, Some(3)];
    let second = task_key(&buttons, &pins, &[], 1).unwrap();
    let pinned = task_key(&buttons, &pins, &[], 2).unwrap();
    assert_eq!((second, pinned), (TaskKey::Window(20), TaskKey::Pin(3)));
    assert_eq!(task_key(&buttons, &pins, &[], 3), None, "stale index");
    let buttons = vec![vec![20], vec![]];
    let pins = vec![None, Some(3)];
    assert_eq!(find_task(&buttons, &pins, &[], &second), Some(0));
    assert_eq!(find_task(&buttons, &pins, &[], &pinned), Some(1));
    let buttons = vec![vec![20], vec![30, 31]];
    assert_eq!(
        find_task(&buttons, &pins, &[], &pinned),
        Some(1),
        "same slot"
    );
    assert_eq!(
        find_task(&buttons, &pins, &[], &TaskKey::Window(31)),
        Some(1)
    );
    assert_eq!(
        find_task(&buttons, &pins, &[], &TaskKey::Window(10)),
        None,
        "gone"
    );
    // A pinned slot is held by its pin even with windows, so it outlives them.
    assert_eq!(task_key(&buttons, &pins, &[], 1), Some(TaskKey::Pin(3)));
    let buttons = vec![vec![20], vec![]];
    assert_eq!(
        find_task(&buttons, &pins, &[], &pinned),
        Some(1),
        "windows gone"
    );
    let buttons = vec![vec![20], vec![30], vec![31]];
    let pins = vec![None, Some(3), Some(3)];
    assert_eq!(task_key(&buttons, &pins, &[], 2), Some(TaskKey::Window(31)));
}

/// The hot button is keyed like a held one: when its window closes, the neighbour sliding into its
/// place stays dark until the pointer moves.
#[test]
fn a_hot_task_button_that_closes_lights_no_neighbour() {
    let buttons = vec![vec![10], vec![20], vec![30]];
    let pins = vec![None; 3];
    let hot = task_key(&buttons, &pins, &[], 1).unwrap();
    assert_eq!(find_task(&buttons, &pins, &[], &hot), Some(1));
    let buttons = vec![vec![10], vec![30]];
    assert_eq!(find_task(&buttons, &pins[..2], &[], &hot), None);
    let buttons = vec![vec![20], vec![30]];
    assert_eq!(find_task(&buttons, &pins[..2], &[], &hot), Some(0));
}

/// A dialog's button follows the pinned buttons and earlier windows and goes before later ones, as
/// wide as theirs. It never groups, and its room counts when windows decide whether to.
#[test]
fn group_puts_a_new_window_by_its_apps_buttons() {
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let counts = TaskbarCounts {
        tasks: 6,
        ..TaskbarCounts::default()
    };
    let apps = keys(&["notepad", "explorer", "notepad", "", "", "notepad"]);
    let seqs = [0, 2, 1, 3, 4, 5].map(Place::end);
    let row = |mode| {
        let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
        order(&lay_slots(
            &mut l,
            &m,
            &shape,
            &apps,
            &seqs,
            &[],
            0,
            &[],
            mode,
        ))
    };
    assert_eq!(
        row(TaskbarButtons::Group),
        ["w0", "w2", "w5", "w1", "w3", "w4"],
        "the notepads in creation order, then explorer; no app id groups nothing"
    );
    assert_eq!(
        row(TaskbarButtons::Never),
        ["w0", "w2", "w1", "w3", "w4", "w5"],
        "never keeps creation order"
    );
}

#[test]
fn a_dragged_unit_passes_a_neighbour_by_its_leading_end() {
    let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
    let pins = [None; 4];
    let run = Some("notepad".to_string());
    let runs = [run.clone(), run.clone(), run, None];
    assert_eq!(row_drop_slot(&tasks, &pins, &runs, 1, 90), 0, "in place");
    assert_eq!(
        row_drop_slot(&tasks, &pins, &runs, 1, 89),
        0,
        "the lead stays short of the lone button's centre"
    );
    assert_eq!(
        row_drop_slot(&tasks, &pins, &runs, 1, 150),
        2,
        "the pointer carried to the run's last button (210) reaches the lone button's centre"
    );
    let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
    let runs = [None, runs[0].clone(), runs[1].clone(), runs[2].clone()];
    assert_eq!(
        row_drop_slot(&tasks, &pins, &runs, 3, 150),
        1,
        "grabbed in its tail, the pointer carried to the run's first button (30) reaches the centre"
    );
    assert_eq!(row_drop_slot(&tasks, &pins, &runs, 3, 149), 0);
}

#[test]
fn a_floated_unit_stays_inside_the_row() {
    let tasks = [0, 60, 120, 180].map(|x| Rect::new(x, 0, 60, 40));
    assert_eq!(
        float_x(&tasks, 1..4, 2, 900, 30),
        Some(120),
        "the run's tail stops at the row's end"
    );
    assert_eq!(
        float_x(&tasks, 1..4, 2, -50, 30),
        Some(60),
        "its head stops at the row's start"
    );
}

#[test]
fn a_dragged_group_run_moves_as_one_unit() {
    let pins = [Some(0), None, None, None];
    let runs = [
        None,
        Some("notepad".into()),
        Some("notepad".into()),
        Some("explorer".into()),
    ];
    assert_eq!(row_units(&pins, &runs), [0..1, 1..3, 3..4]);
    let (_, anchors) = reorder_row(&pins, &runs, 2, 3).unwrap();
    assert_eq!(
        anchors,
        [(3..4, usize::MAX), (1..3, usize::MAX)],
        "explorer, then the whole notepad run, after the pin"
    );
}

#[test]
fn a_dialogs_button_goes_in_creation_order() {
    let m = reprisekit::theme::Metrics::default();
    let shape = TaskbarShape::CLASSIC;
    let counts = TaskbarCounts {
        tasks: 4,
        ..TaskbarCounts::default()
    };
    let mode = TaskbarButtons::WhenFull;
    let row = |slots: Vec<Slot>| -> Vec<(Vec<usize>, Option<usize>, Option<u64>)> {
        slots
            .into_iter()
            .map(|s| (s.windows, s.pin, s.dialog))
            .collect()
    };
    let apps = keys(&["foot", "kitty", "xterm"]);
    let seqs = [Place::end(9), Place::end(2), Place::end(6)];
    let pin_of = [Some(0), None, None];
    let dialog = [DialogTask {
        id: 7,
        place: Place::end(4),
    }];
    let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
    let slots = lay_slots(&mut l, &m, &shape, &apps, &seqs, &pin_of, 1, &dialog, mode);
    assert_eq!(
        row(slots),
        [
            (vec![0], Some(0), None),
            (vec![1], None, None),
            (vec![], None, Some(7)),
            (vec![2], None, None),
        ]
    );
    assert_eq!(l.stacks, [1, 1, 1, 1]);
    assert_eq!(l.tasks[2].width, l.tasks[1].width);
    assert!(l.tasks[2].x > l.tasks[1].right());
    let mut l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
    let last = [DialogTask {
        id: 7,
        place: Place::end(10),
    }];
    let slots = lay_slots(&mut l, &m, &shape, &apps, &seqs, &pin_of, 1, &last, mode);
    assert_eq!(slots[3].dialog, Some(7), "made last, it ends the row");

    let gap = shape.labelled.gap;
    let l0 = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
    let n = ((l0.tasks_limit() - l0.tasks_x() + gap) / (GROUP_BELOW + gap)) as usize;
    let same = vec![String::from("foot"); n];
    let seqs: Vec<Place> = (0..n as u64).map(|i| Place::end(i * 2)).collect();
    let mut l = l0.clone();
    let alone = lay_slots(&mut l, &m, &shape, &same, &seqs, &[], 0, &[], mode);
    assert_eq!(alone.len(), n);
    let mut l = l0.clone();
    let between = [DialogTask {
        id: 7,
        place: Place::end(1),
    }];
    let grouped = lay_slots(&mut l, &m, &shape, &same, &seqs, &[], 0, &between, mode);
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].windows, (0..n).collect::<Vec<_>>());
    assert_eq!(grouped[1].dialog, Some(7));
}

/// End Program's button beside its window on a 28 px bar: 160x22 at x 222, three past the window's.
/// While active it is pushed in over the dither, 16 px glyph at (5, 4), bold title from x 25 with an
/// ellipsis inside the button; otherwise raised.
#[test]
fn a_dialogs_button_draws_as_a_task_button() {
    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};
    let mut theme = Theme::default();
    theme.metrics.taskbar_height = 28;
    let cs = theme.colors.clone();
    let text = TextRenderer::load(&FontConfig::default());
    let m = &theme.metrics;
    let shape = theme.taskbar_shape();
    let counts = TaskbarCounts {
        tasks: 2,
        ..TaskbarCounts::default()
    };
    let mut l = TaskbarLayout::compute(m, &shape, 800, true, counts, 30);
    let apps = keys(&["hang"]);
    let slots = lay_slots(
        &mut l,
        m,
        &shape,
        &apps,
        &[Place::end(0)],
        &[],
        0,
        &[DialogTask {
            id: 7,
            place: Place::end(1),
        }],
        TaskbarButtons::WhenFull,
    );
    assert_eq!(slots[1].dialog, Some(7));
    assert_eq!(l.tasks[0], Rect::new(59, 4, 160, 22));
    let r = l.tasks[1];
    assert_eq!(r, Rect::new(222, 4, 160, 22));
    let window = || TaskEntry {
        title: "Hang".into(),
        active: false,
        attention: false,
        icon: None,
        glow: None,
    };
    let draw = |active: bool| {
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                window(),
                dialog_entry("End Program - signal-desktop", active),
            ],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 28);
        reprisekit::look::draw_taskbar(&mut c, &theme, &text, &text, &l, &state);
        c
    };
    let px = |c: &Canvas, x: i32, y: i32| c.get(r.x + x, r.y + y);
    let c = draw(true);
    assert_eq!(
        px(&c, 0, 10),
        cs.button_dk_shadow.argb(),
        "pushed: dark left"
    );
    assert_eq!(px(&c, 1, 10), cs.button_shadow.argb());
    assert_eq!(
        px(&c, 10, 0),
        cs.button_dk_shadow.argb(),
        "pushed: dark top"
    );
    assert_eq!(px(&c, 2, 2), cs.button_hilight.argb(), "the dither");
    assert_eq!(px(&c, 3, 2), cs.button_face.argb());
    assert_eq!(
        px(&c, 5, 4),
        cs.button_dk_shadow.argb(),
        "the glyph's corner"
    );
    assert_eq!(px(&c, 20, 19), cs.button_dk_shadow.argb());
    assert_eq!(px(&c, 21, 12), cs.button_face.argb(), "past the glyph");
    let ink: Vec<i32> = (24..r.width)
        .filter(|&x| (5..18).any(|y| px(&c, x, y) == cs.button_text.argb()))
        .collect();
    if reprisekit::text::test_tahoma() {
        assert_eq!(ink.first(), Some(&25), "the title from x 25");
    }
    assert!(ink.last().is_some_and(|&x| x < r.width - 3), "cut inside");
    let c = draw(false);
    assert_eq!(px(&c, 0, 10), cs.button_hilight.argb(), "raised");
    assert_eq!(
        px(&c, 4, 3),
        cs.button_dk_shadow.argb(),
        "the glyph at (4, 3)"
    );
}

/// A held dialog button is found again by its dialog, wherever the row moved it.
#[test]
fn a_held_dialog_button_is_found_by_its_dialog() {
    let buttons = vec![vec![10], vec![]];
    let pins = vec![None, None];
    let dialogs = vec![None, Some(4)];
    let key = task_key(&buttons, &pins, &dialogs, 1).unwrap();
    assert_eq!(key, TaskKey::Dialog(4));
    assert_eq!(
        task_key(&buttons, &pins, &dialogs, 0),
        Some(TaskKey::Window(10))
    );
    let buttons = vec![vec![10], vec![20], vec![]];
    let dialogs = vec![None, None, Some(4)];
    assert_eq!(find_task(&buttons, &[], &dialogs, &key), Some(2));
    assert_eq!(
        find_task(&buttons, &[], &[None, None], &key),
        None,
        "closed"
    );
}

#[test]
fn any_task_button_drags_once_moved() {
    assert!(press_drags(false, true));
    assert!(!press_drags(false, false), "not moved yet");
    assert!(press_drags(true, false), "a drag stays one");
}

#[test]
fn jump_lists_follow_what_the_button_stands_for() {
    use std::path::PathBuf;
    let labels = |items: &[MenuItem]| -> Vec<String> {
        items
            .iter()
            .map(|i| match i {
                MenuItem::Entry { label, .. } => label.clone(),
                _ => "-".into(),
            })
            .collect()
    };
    let recent = [PathBuf::from("/h/b.txt"), PathBuf::from("/h/a.txt")];
    let titles = |n: usize| (1..=n).map(|k| format!("w{k}")).collect::<Vec<_>>();
    let (items, rows) = jump_list("Terminal", &recent, true, true, &titles(3), 1);
    assert_eq!(
        labels(&items),
        [
            "b.txt",
            "a.txt",
            "-",
            "w1",
            "w2",
            "w3",
            "-",
            "Terminal",
            "Unpin this program from taskbar",
            "Close all windows"
        ]
    );
    assert_eq!(rows[0], Some(JumpRow::Recent(recent[0].clone())));
    assert_eq!(rows[2], None);
    assert_eq!(
        rows[3..],
        [
            Some(JumpRow::Window(0)),
            Some(JumpRow::Window(1)),
            Some(JumpRow::Window(2)),
            None,
            Some(JumpRow::Launch),
            Some(JumpRow::Unpin),
            Some(JumpRow::Close)
        ]
    );
    let (items, rows) = jump_list("Terminal", &[], true, true, &[], 4);
    assert_eq!(
        labels(&items),
        ["Terminal", "Unpin this program from taskbar"]
    );
    assert_eq!(rows, [Some(JumpRow::Launch), Some(JumpRow::Unpin)]);
    let (items, rows) = jump_list("Editor", &[], false, true, &titles(1), 1);
    assert_eq!(
        labels(&items),
        ["Editor", "Pin this program to taskbar", "Close window"]
    );
    assert_eq!(rows[1], Some(JumpRow::Pin));
    assert!(matches!(items[1], MenuItem::Entry { enabled: true, .. }));
    let (items, _) = jump_list("Editor", &[], false, false, &titles(1), 1);
    assert!(matches!(items[1], MenuItem::Entry { enabled: false, .. }));
    let (items, rows) = jump_list("Editor", &[], false, true, &titles(2), 4);
    assert_eq!(
        labels(&items)[4..],
        [
            "Pin this program to taskbar",
            "Move to desktop",
            "Close all windows"
        ]
    );
    assert_eq!(rows[5], Some(JumpRow::MoveToMenu));
    assert!(matches!(items[5], MenuItem::Entry { submenu: true, .. }));
}

#[test]
fn a_window_list_moves_the_group_only_with_several_desktops() {
    use WindowListRow::*;
    let titles = ["a", "b"].map(String::from);
    let (items, rows) = window_list(&titles, 4);
    assert_eq!(
        rows,
        [
            Some(Window(0)),
            Some(Window(1)),
            None,
            Some(Minimize),
            Some(MoveToMenu),
            Some(Close)
        ]
    );
    assert!(
        matches!(&items[4], MenuItem::Entry { submenu: true, label, .. } if label == "Move to desktop")
    );
    let (_, rows) = window_list(&titles, 1);
    assert_eq!(rows[3..], [Some(Minimize), Some(Close)]);
}

#[test]
fn a_group_click_brings_up_the_latest_window_then_steps_through() {
    let group = [10, 20, 30];
    let recency = [30, 10, 20, 99];
    assert_eq!(group_click_pick(&group, &recency, Some(99)), Some(20));
    assert_eq!(group_click_pick(&group, &recency, None), Some(20));
    assert_eq!(group_click_pick(&group, &recency, Some(20)), Some(30));
    assert_eq!(group_click_pick(&group, &recency, Some(30)), Some(10));
    assert_eq!(group_click_pick(&group, &[], None), Some(10));
    assert_eq!(group_click_pick::<i32>(&[], &recency, None), None);
}

#[test]
fn group_name_takes_the_last_dotted_segment() {
    assert_eq!(group_name("org.gnome.Nautilus"), "Nautilus");
    assert_eq!(group_name("Alacritty"), "Alacritty");
    assert_eq!(group_name(""), "Window");
}

#[test]
fn move_button_lands_in_the_slot_and_ignores_no_ops() {
    let mut v = vec!['a', 'b', 'c', 'd'];
    assert!(!move_button(&mut v, 1, 1));
    assert!(!move_button(&mut v, 1, 2));
    assert!(move_button(&mut v, 0, 3));
    assert_eq!(v, ['b', 'c', 'a', 'd']);
    assert!(move_button(&mut v, 3, 0));
    assert_eq!(v, ['d', 'b', 'c', 'a']);
    assert!(!move_button(&mut v, 9, 0));
    assert!(!move_button(&mut v, 0, 9));
}
