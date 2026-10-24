use super::*;
use crate::keys::{KEY_KP_Enter, KEY_Return};

#[test]
fn the_edit_menu_over_the_start_panel_unrolls_facing_the_press() {
    let mut config = crate::config::Config::default();
    config.startmenu_style = crate::config::StartMenuStyle::Glass;
    config.effects.menu_animation = crate::config::MenuAnimation::Slide;
    let mut h = crate::harness::Harness::with_config(config);
    h.output("TEST-1", 1024, 768);
    h.pointer_to(512.0, 400.0);
    h.state.toggle_start_from_keys();
    h.dispatch();
    assert!(h.state.start_panel.is_some(), "the Start panel is open");
    h.pointer_to(600.0, 100.0);
    assert!(h.state.open_edit_menu(Some((600, 100))));
    let menu = h.state.menus.first().expect("the edit menu is open");
    assert_eq!(
        menu.opening,
        Some(Opening::Unroll(UnrollFrom::Top)),
        "hanging below the press, it unrolls down from it, not up from the bottom bar"
    );
    assert!(menu.from.holds_bar(), "it keeps an auto-hidden bar shown");
}

#[test]
fn an_open_start_menu_row_takes_an_icon_installed_while_it_is_open() {
    let config = crate::config::Config {
        startmenu: vec![MenuEntry::item(
            "Probe",
            "reprise-probe",
            MenuTarget::Programs,
        )],
        ..Default::default()
    };
    let mut h = crate::harness::Harness::with_config(config);
    let root = crate::harness::scratch().join("menu-icons");
    let apps = root.join("hicolor/scalable/apps");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(
        root.join("hicolor/index.theme"),
        "[Icon Theme]\nName=Hicolor\nDirectories=scalable/apps\n\n\
         [scalable/apps]\nSize=48\nMinSize=8\nMaxSize=512\nType=Scalable\n",
    )
    .unwrap();
    h.state
        .icons
        .set_theme(reprisekit::icontheme::IconTheme::load("hicolor", &[root]));
    h.output("TEST-1", 1024, 768);
    let bar = h.state.bars[0].tree;
    h.state.open_start_menu(bar);
    let icon = |h: &crate::harness::Harness| match &h.state.menus[0].items[0] {
        MenuItem::Entry { icon, .. } => icon
            .as_ref()
            .map(|c| c.get(c.width as i32 / 2, c.height as i32 / 2)),
        MenuItem::Separator => None,
    };
    assert_eq!(icon(&h), None, "no file has the name yet");
    std::fs::write(
        apps.join("reprise-probe.svg"),
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#ff0000"/></svg>"##,
    )
    .unwrap();
    h.state.icons.reload();
    h.state.relook_menu_icons();
    assert_eq!(
        icon(&h),
        Some(0xFFFF_0000),
        "the open row wears the new file"
    );
}

#[test]
fn an_icon_file_lands_on_its_waiting_rows_only() {
    let (file, other) = ("/srv/icons/app.png", "/srv/icons/other.png");
    let mut items = vec![
        MenuItem::entry("Aaa"),
        MenuItem::entry("Bbb"),
        MenuItem::entry("Ccc"),
    ];
    let mut waits = vec![(0, file.to_string(), 16), (2, other.to_string(), 16)];
    let art = Some(Canvas::new(16, 16));
    assert!(
        !land_icon(&mut items, &mut waits, file, 32, &art),
        "another size is its own decode"
    );
    assert!(land_icon(&mut items, &mut waits, file, 16, &art));
    let icon_width = |it: &MenuItem| match it {
        MenuItem::Entry { icon, .. } => icon.as_ref().map(|c| c.width),
        MenuItem::Separator => None,
    };
    assert_eq!(
        items.iter().map(icon_width).collect::<Vec<_>>(),
        [Some(16), None, None]
    );
    assert_eq!(waits, [(2, other.to_string(), 16)]);
}

#[test]
fn show_desktop_restores_bottom_first_as_they_were_stacked() {
    let mut scene = crate::compositor::DeskScene::new();
    let layer = scene.tree_create(scene.root());
    let other = scene.tree_create(scene.root());
    let [a, dialog, b] = [0; 3].map(|_| scene.tree_create(layer));
    let on_top = scene.tree_create(other);
    scene.raise_to_top(a);
    let items = vec![(a, 'a'), (b, 'b'), (on_top, 't'), (dialog, 'd')];
    let order: Vec<char> = super::restore_order(scene.children(layer), items)
        .into_iter()
        .map(|(_, c)| c)
        .collect();
    assert_eq!(order, ['t', 'd', 'b', 'a']);
}

#[test]
fn an_overflow_rows_pick_names_its_bar_and_button() {
    use crate::taskbar::TaskKey;
    let mut scene = crate::compositor::DeskScene::new();
    let bar = scene.tree_create(scene.root());
    let w = WindowId::next();
    let kind = MenuKind::TaskOverflow {
        bar,
        keys: vec![TaskKey::Pin(2), TaskKey::Window(w), TaskKey::Dialog(9)],
    };
    assert_eq!(
        kind.pick(1),
        MenuPick::TaskOverflow {
            bar,
            key: TaskKey::Window(w)
        }
    );
    assert_eq!(kind.pick(3), MenuPick::Nothing);
}

#[test]
fn an_overflow_row_finds_its_button_after_the_row_changes() {
    use crate::taskbar::{find_task, task_key};
    let mut scene = crate::compositor::DeskScene::new();
    let bar = scene.tree_create(scene.root());
    let [a, b, c] = [WindowId::next(), WindowId::next(), WindowId::next()];
    let opened = [vec![a], vec![b], vec![c]];
    let kind = MenuKind::TaskOverflow {
        bar,
        keys: (1..3)
            .filter_map(|i| task_key(&opened, &[], &[], i))
            .collect(),
    };
    let MenuPick::TaskOverflow { key, .. } = kind.pick(1) else {
        panic!("row 1 is c's button");
    };
    let a_closed = [vec![b], vec![c]];
    assert_eq!(
        find_task(&a_closed, &[], &[], &key),
        Some(1),
        "one place up"
    );
    let c_closed = [vec![a], vec![b]];
    assert_eq!(find_task(&c_closed, &[], &[], &key), None, "gone");
}

#[test]
fn a_quick_overflow_row_launches_its_own_file() {
    let mut scene = crate::compositor::DeskScene::new();
    let bar = scene.tree_create(scene.root());
    let kind = MenuKind::QuickOverflow {
        bar,
        files: vec!["c.desktop".into(), "d.desktop".into()],
    };
    assert_eq!(kind.pick(1), MenuPick::QuickOverflow("d.desktop".into()));
    assert_eq!(kind.pick(2), MenuPick::Nothing);
}

#[test]
fn a_window_list_hangs_off_its_button_or_the_more_cell() {
    use reprisekit::taskbar::{Bands, BarSpan, ClockText, Edge, TaskbarCounts, TaskbarLayout};
    let glass = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let span = BarSpan {
        edge: Edge::Left,
        length: 400,
        thickness: 60,
    };
    let counts = TaskbarCounts {
        tasks: 8,
        ..TaskbarCounts::default()
    };
    let shape = glass.taskbar_shape();
    let l = TaskbarLayout::compute_on(
        &glass.metrics,
        &shape,
        span,
        true,
        counts,
        ClockText::default(),
        Bands::default(),
    );
    let more = l.more.expect("eight 40 px buttons overrun 400 px");
    assert_eq!(l.tasks.len(), 6);
    assert_eq!(
        list_anchor(&l, 1),
        l.tasks[1],
        "a button on the column: itself"
    );
    assert_eq!(
        list_anchor(&l, 7),
        more,
        "past the column's end: the » cell"
    );
    let p = crate::barplace::place(Rect::new(0, 0, 1920, 1080), Edge::Left, 60);
    assert_eq!(
        p.anchor_point(list_anchor(&l, 7)),
        (60, 314),
        "right of the bar, level with »"
    );
}

#[test]
fn escape_in_a_start_panel_popup_closes_the_popup_only() {
    use crate::keyroute::{KeyPress, KeyRoute, Modes, route};
    let modes = Modes {
        start_panel_open: true,
        menus_open: true,
        ..Modes::default()
    };
    let escape = KeyPress {
        syms: vec![keys::KEY_Escape],
        lower: vec![keys::KEY_Escape],
        mods: 0,
        pressed: true,
    };
    assert_eq!(
        route(&modes, &escape, &[], &[]).route,
        KeyRoute::Menu(keys::KEY_Escape)
    );
    let (items, rows) = crate::startmenu::shutdown_menu(false);
    assert_eq!(
        key_act(&items, Some(0), keys::KEY_Escape, KeyStops::Enabled),
        Some(KeyAct::CloseTopmost)
    );
    let kind = MenuKind::StartPanel { rows };
    assert!(
        !pick_closes_start_panel(&kind.pick(0)),
        "Log off is the panel's"
    );
    assert!(pick_closes_start_panel(&MenuPick::Taskbar(
        TaskbarCommand::Cascade
    )));
}

#[test]
fn the_start_menu_docks_on_the_screen_side_of_the_bar() {
    use reprisekit::taskbar::{Edge, StartMenuDock};
    let screen = Rect::new(0, 0, 1920, 1080);
    let at = |dock: StartMenuDock, edge: Edge, height: i32| {
        let p = crate::barplace::place(screen, edge, 30);
        let start = Rect::new(2, p.shown.y + 4, 54, 22);
        start_menu_at(
            dock,
            &p,
            start,
            (300, height),
            crate::barplace::usable(screen, &p),
        )
        .1
    };
    assert_eq!(
        at(StartMenuDock::Into(4), Edge::Bottom, 400),
        1050 + 4 - 400
    );
    assert_eq!(at(StartMenuDock::OnTop, Edge::Bottom, 400), 1050 - 400);
    assert_eq!(at(StartMenuDock::Into(4), Edge::Bottom, 2000), 0);
    assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 400), 30 - 4);
    assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 400), 30);
    assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 1054), 30 - 4);
    assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 1060), 1080 - 1060);
    assert_eq!(at(StartMenuDock::Into(4), Edge::Top, 1060), 1080 - 1060);
    assert_eq!(at(StartMenuDock::OnTop, Edge::Top, 2000), 0);
}

#[test]
fn a_hidden_bars_start_menu_stands_on_its_reveal_line() {
    use reprisekit::taskbar::{Edge, StartMenuDock};
    let screen = Rect::new(0, 0, 1920, 1080);
    let at = |dock: StartMenuDock, edge: Edge, height: i32| {
        let p = crate::barplace::place_hiding(screen, edge, 30, Some(1.0));
        let start = Rect::new(2, p.shown.y + 4, 54, 22);
        start_menu_at(
            dock,
            &p,
            start,
            (300, height),
            crate::barplace::usable(screen, &p),
        )
        .1
    };
    for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
        assert_eq!(
            at(dock, Edge::Bottom, 400) + 400,
            1078,
            "{dock:?}: over a bottom bar the menu's foot is the line's top"
        );
        assert_eq!(
            at(dock, Edge::Top, 400),
            2,
            "{dock:?}: under a top bar the menu's top is the line's foot"
        );
        assert_eq!(
            at(dock, Edge::Bottom, 2000),
            0,
            "{dock:?}: a tall menu stops at the top"
        );
        assert_eq!(
            at(dock, Edge::Top, 2000),
            0,
            "{dock:?}: and starts at the top"
        );
    }
    for edge in [Edge::Bottom, Edge::Right] {
        let p = crate::barplace::place_hiding(screen, edge, 30, Some(1.0));
        assert_eq!(
            start_menu_shadow_floor(StartMenuDock::Into(4), &p),
            Some(0),
            "{edge:?}"
        );
    }
    let shown = crate::barplace::place_hiding(screen, Edge::Bottom, 30, Some(0.0));
    assert_eq!(
        start_menu_shadow_floor(StartMenuDock::Into(4), &shown),
        None,
        "its shadow falls on the bar it reaches into"
    );
    let hidden = crate::barplace::place_hiding(screen, Edge::Bottom, 30, Some(1.0));
    assert_eq!(
        start_submenu_shadow_floor(StartMenuDock::Into(4), &hidden, 700),
        Some(1078 - 700)
    );
    assert_eq!(
        start_submenu_shadow_floor(StartMenuDock::Into(4), &shown, 700),
        None
    );
    let area = crate::barplace::usable(screen, &shown);
    assert_eq!(
        start_menu_at(
            StartMenuDock::Into(4),
            &shown,
            Rect::new(2, shown.shown.y + 4, 54, 22),
            (300, 400),
            area
        )
        .1,
        1050 + 4 - 400
    );
}

#[test]
fn the_tray_cells_menus_pick_their_rows() {
    use crate::nightlight::Row;
    assert_eq!(MenuKind::Layouts.pick(2), MenuPick::Layout(2));
    let night = MenuKind::NightLight(vec![Some(Row::Pause), None, Some(Row::Settings)]);
    assert_eq!(night.pick(0), MenuPick::NightLight(Row::Pause));
    assert_eq!(night.pick(1), MenuPick::Nothing);
    assert_eq!(night.pick(2), MenuPick::NightLight(Row::Settings));
}

#[test]
fn documents_folders_have_their_infotips() {
    let docs = MenuKind::Documents {
        paths: vec![
            Some("/home/u/Documents".into()),
            None,
            Some("/tmp/a.txt".into()),
        ],
        tips: vec![
            crate::desktop::document_tip("folder-documents").map(String::from),
            None,
            None,
        ],
        dirs: true,
        icon_px: 16,
    };
    assert_eq!(
        docs.tip(0).as_deref(),
        Some("Contains letters, reports, and other documents and files.")
    );
    assert_eq!(docs.tip(1), None);
    assert_eq!(docs.tip(2), None);
}

#[test]
fn enter_runs_the_default_row_when_nothing_is_highlighted() {
    let rows = |enabled: bool| {
        vec![
            MenuItem::item("&Copy Here"),
            MenuItem::item("&Move Here").bold(true).enabled(enabled),
            MenuItem::item("Create &Shortcuts Here"),
            MenuItem::Separator,
            MenuItem::item("Cancel"),
        ]
    };
    for sym in [KEY_Return, KEY_KP_Enter] {
        assert_eq!(
            key_act(&rows(true), None, sym, KeyStops::Enabled),
            Some(KeyAct::SelectAnd(1, Box::new(KeyAct::Activate(1))))
        );
        assert_eq!(
            key_act(&rows(true), Some(0), sym, KeyStops::Enabled),
            Some(KeyAct::Activate(0))
        );
        assert_eq!(
            key_act(&rows(false), None, sym, KeyStops::Enabled),
            Some(KeyAct::Nothing)
        );
    }
    let plain = [MenuItem::item("&Open"), MenuItem::item("&Delete")];
    assert_eq!(
        key_act(&plain, None, KEY_Return, KeyStops::Enabled),
        Some(KeyAct::Nothing)
    );
    let sub = [MenuItem::item("Send &To").bold(true).submenu(true)];
    assert_eq!(
        key_act(&sub, None, KEY_Return, KeyStops::Enabled),
        Some(KeyAct::SelectAnd(0, Box::new(KeyAct::OpenSubmenu(0))))
    );
}

#[test]
fn access_keys_pick_or_cycle_and_escape_closes() {
    use crate::keys::{KEY_Escape, KEY_d, KEY_o};
    let items = [
        MenuItem::item("&Open"),
        MenuItem::item("&Delete"),
        MenuItem::item("&Details"),
    ];
    assert_eq!(
        key_act(&items, None, KEY_o, KeyStops::Enabled),
        Some(KeyAct::SelectAnd(0, Box::new(KeyAct::Activate(0))))
    );
    assert_eq!(
        key_act(&items, None, KEY_d, KeyStops::Enabled),
        Some(KeyAct::Select(Some(1)))
    );
    assert_eq!(
        key_act(&items, Some(1), KEY_d, KeyStops::Enabled),
        Some(KeyAct::Select(Some(2)))
    );
    assert_eq!(
        key_act(&items, None, KEY_Escape, KeyStops::Enabled),
        Some(KeyAct::CloseTopmost)
    );
}

#[test]
fn beside_a_side_bar_the_start_menu_hangs_level_with_start() {
    use reprisekit::taskbar::{Edge, StartMenuDock};
    let screen = Rect::new(0, 0, 1920, 1080);
    let at = |dock, edge, start: Rect, height| {
        let p = crate::barplace::place(screen, edge, 60);
        start_menu_at(
            dock,
            &p,
            start,
            (300, height),
            crate::barplace::usable(screen, &p),
        )
    };
    let start = Rect::new(2, 2, 54, 22);
    assert_eq!(
        at(StartMenuDock::Into(4), Edge::Left, start, 400),
        (60 - 4, 2),
        "4 px into a left bar, level with Start's top"
    );
    assert_eq!(at(StartMenuDock::OnTop, Edge::Left, start, 400), (60, 2));
    let start = Rect::new(1864, 2, 54, 22);
    assert_eq!(
        at(StartMenuDock::Into(4), Edge::Right, start, 400),
        (1860 + 4 - 300, 2)
    );
    assert_eq!(
        at(StartMenuDock::OnTop, Edge::Right, start, 400),
        (1860 - 300, 2)
    );
    let low = Rect::new(2, 1000, 54, 22);
    assert_eq!(at(StartMenuDock::OnTop, Edge::Left, low, 400).1, 1080 - 400);
    assert_eq!(at(StartMenuDock::OnTop, Edge::Left, start, 2000).1, 0);
    let narrow = Rect::new(1920, 0, 320, 1080);
    let side = |dock, edge| {
        let p = crate::barplace::place(narrow, edge, 60);
        let start = Rect::new(p.shown.x + 2, 2, 54, 22);
        start_menu_at(
            dock,
            &p,
            start,
            (300, 400),
            crate::barplace::usable(narrow, &p),
        )
        .0
    };
    for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
        assert_eq!(
            side(dock, Edge::Right),
            1920,
            "{dock:?}: a right bar's menu stops at the output's left edge"
        );
        assert_eq!(
            side(dock, Edge::Left),
            2240 - 300,
            "{dock:?}: a left bar's menu stops at the output's right edge"
        );
    }
    let p = crate::barplace::place_hiding(screen, Edge::Left, 60, Some(1.0));
    let area = crate::barplace::usable(screen, &p);
    assert_eq!(
        start_menu_at(StartMenuDock::Into(4), &p, start, (300, 400), area).0,
        2
    );
}
