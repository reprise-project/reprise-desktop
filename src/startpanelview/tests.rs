use std::path::Path;
use std::rc::Rc;

use reprisefs::apps::AppEntry;
use reprisekit::startpanel::{Key, Mode, ProgramRow, TreeRow, layout};
use reprisekit::taskbar::{Edge, StartMenuDock};
use reprisekit::textedit::EditKey;
use smithay::utils::{Point, Rectangle};

use crate::barplace::BarPlacement;
use crate::dialogview::DialogKey;
use crate::keys;
use crate::raster::Canvas;

use super::draw::{Drawn, RasterKey, Rasters, panel_origin, panel_pane, panel_room, takes_pointer};
use super::input::{key_of, spot_app};
use super::lists::{
    app_icon, frequent_rows, jump_file, one_per_name, refill_tree, replace_tree, tree_rows,
};
use super::menus::shutdown_options_at;
use super::picture::{UserPictures, picture_paths, read_pictures};
use super::search::{ResultKind, panel_matches, result_groups};
use super::*;

#[test]
fn a_blink_back_to_the_canvas_before_compares_only_the_box_they_differ_in() {
    let (on, off) = (Rc::new(Canvas::new(8, 4)), Rc::new(Canvas::new(8, 4)));
    let caret = Rectangle::new((3, 1).into(), (1, 2).into());
    let drawn = Drawn {
        canvas: off.clone(),
        mode: Mode::Home,
        picture: None,
        before: Some((on.clone(), caret)),
    };
    assert_eq!(drawn.differs_within(&on), Some(caret));
    assert_eq!(drawn.differs_within(&Rc::new(Canvas::new(8, 4))), None);
    assert_eq!(drawn.differs_within(&off), None);
}

fn press(sym: u32, mods: u32) -> DialogKey {
    DialogKey {
        syms: vec![sym],
        mods,
        edit: Some(EditKey::Insert("x".into())),
    }
}

fn app(name: &str, category: &str) -> AppEntry {
    AppEntry {
        name: name.into(),
        exec_line: name.into(),
        terminal: false,
        id: format!("{name}.desktop"),
        icon: None,
        startup_wm_class: None,
        categories: vec![category.into()],
        mime_types: Vec::new(),
        shown: true,
    }
}

#[test]
fn a_tree_refill_gives_only_app_rows_their_icons() {
    let mut rows = vec![
        TreeRow::Folder {
            name: "Games".into(),
            open: true,
            icon: None,
        },
        TreeRow::App {
            name: "Chess".into(),
            icon: None,
            depth: 1,
        },
    ];
    refill_tree(&mut rows, &[None, Some(3)], |i| {
        Some(Canvas::new(i as _, 1))
    });
    let widths: Vec<Option<usize>> = (rows.iter())
        .map(|r| match r {
            TreeRow::Folder { icon, .. } | TreeRow::App { icon, .. } => {
                icon.as_ref().map(|c| c.width as usize)
            }
        })
        .collect();
    assert_eq!(widths, [None, Some(3)]);
}

#[test]
fn a_program_icon_file_is_never_read_on_the_loop() {
    use reprisekit::icontheme::{IconCache, IconTheme};
    let file = "/srv/icons/chess.png";
    let mut chess = app("Chess", "Game");
    chess.icon = Some(file.into());
    let mut icons = IconCache::new(IconTheme::load("none", &[]));
    assert_eq!(app_icon(&mut icons, &chess, 16), None);
    assert_eq!(icons.take_wanted(), [(file.to_string(), 16)]);
}

#[test]
fn a_caret_blink_reuses_the_canvas_drawn_for_that_phase_until_the_panel_changes() {
    let key = |caret: bool, model: &StartPanel| RasterKey {
        model: model.clone(),
        pressed: None,
        caret,
        preedit: None,
        scale: 1,
    };
    let model = StartPanel::new();
    let (on, off) = (Rc::new(Canvas::new(2, 2)), Rc::new(Canvas::new(2, 2)));
    let mut r = Rasters::default();
    r.keep(key(true, &model), on.clone(), None);
    r.keep(key(false, &model), off.clone(), None);
    let found = |r: &Rasters, k: &RasterKey| r.find(k).map(|(c, _)| c);
    assert!(found(&r, &key(true, &model)).is_some_and(|c| Rc::ptr_eq(&c, &on)));
    assert!(found(&r, &key(false, &model)).is_some_and(|c| Rc::ptr_eq(&c, &off)));
    let mut typed = model.clone();
    typed.paste("a");
    assert!(
        found(&r, &key(true, &typed)).is_none(),
        "a changed panel draws again"
    );
    let pressed = RasterKey {
        pressed: Some(Spot::Search),
        ..key(true, &model)
    };
    assert!(found(&r, &pressed).is_none(), "a press draws again");
    r.keep(key(true, &typed), on.clone(), None);
    assert!(
        found(&r, &key(true, &model)).is_none(),
        "two canvases are kept"
    );
    assert!(found(&r, &key(false, &model)).is_some());
}

#[test]
fn the_frequent_list_tops_up_with_the_starters_not_shown() {
    assert_eq!(
        frequent_rows(vec![7], &[1, 7, 2, 3], &[2]),
        (vec![7, 1, 3], 1)
    );
    let full: Vec<usize> = (10..20).collect();
    assert_eq!(
        frequent_rows(full.clone(), &[1], &[]),
        (full, 10),
        "ten rows are full"
    );
}

#[test]
fn a_starter_row_offers_no_remove_from_this_list() {
    let programs = [5, 7, 1, 3];
    let at = |s| {
        spot_app(
            s,
            &programs,
            (1, 1),
            &[None, Some(9)],
            (&[4], &[ResultKind::Programs]),
        )
    };
    assert_eq!(at(Spot::Program(0)), Some((5, false)), "pinned");
    assert_eq!(at(Spot::Program(1)), Some((7, true)), "launched");
    assert_eq!(at(Spot::Program(2)), Some((1, false)), "starter");
    assert_eq!(at(Spot::Program(3)), Some((3, false)), "starter");
}

#[test]
fn only_programs_tree_apps_and_program_results_get_a_program_menu() {
    let kinds = [ResultKind::Calc, ResultKind::Programs, ResultKind::Files];
    let at = |s| spot_app(s, &[5], (0, 1), &[None, Some(9)], (&[4], &kinds));
    assert_eq!(at(Spot::Tree(1)), Some((9, false)));
    assert_eq!(at(Spot::Tree(0)), None, "a folder");
    assert_eq!(at(Spot::Result(1, 0)), Some((4, false)));
    assert_eq!(at(Spot::Result(0, 0)), None, "the answer");
    assert_eq!(at(Spot::Result(2, 0)), None, "a file result");
    assert_eq!(at(Spot::Place(0)), None);
    assert_eq!(at(Spot::Program(1)), None, "past the rows");
    let files = [ResultKind::Files];
    let no_programs = spot_app(Spot::Result(0, 0), &[], (0, 0), &[], (&[], &files));
    assert_eq!(no_programs, None, "group 0 is Files");
}

#[test]
fn search_groups_the_answer_programs_settings_then_files_with_counts() {
    let (groups, kinds) = result_groups(
        Some("= 14"),
        &["Notepad".into()],
        &["Mouse".into(), "Pointers".into()],
        &[PathBuf::from("/home/u/notes.txt")],
    );
    let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Calculator", "Programs (1)", "Settings (2)", "Files (1)"]
    );
    assert_eq!(
        kinds,
        [
            ResultKind::Calc,
            ResultKind::Programs,
            ResultKind::Settings,
            ResultKind::Files
        ]
    );
    assert_eq!(groups[0].rows[0].name, "= 14");
    assert_eq!(groups[3].rows[0].name, "notes.txt");
    let (groups, kinds) = result_groups(None, &[], &["Mouse".into()], &[]);
    assert_eq!((groups.len(), kinds), (1, vec![ResultKind::Settings]));
    assert!(result_groups(None, &[], &[], &[]).0.is_empty());
}

#[test]
fn a_group_shows_at_most_eight_rows_and_counts_them() {
    let names: Vec<String> = (0..12).map(|i| format!("App {i}")).collect();
    let (groups, _) = result_groups(None, &names, &[], &[]);
    assert_eq!(groups[0].title, "Programs (8)");
    assert_eq!(groups[0].rows.len(), 8);
}

#[test]
fn the_tree_lists_folders_with_their_apps_under_them() {
    let apps = [
        app("Paint", "Graphics"),
        app("Calculator", "Utility"),
        app("Gimp", "Graphics"),
    ];
    let (rows, map) = tree_rows(&apps, &reprisefs::apps::group(&apps), &[true; 3]);
    let names: Vec<&str> = rows
        .iter()
        .map(|r| match r {
            TreeRow::Folder { name, .. } | TreeRow::App { name, .. } => name.as_str(),
        })
        .collect();
    assert_eq!(
        names,
        ["Accessories", "Calculator", "Graphics", "Paint", "Gimp"]
    );
    assert_eq!(map, [None, Some(1), None, Some(0), Some(2)]);
    assert!(
        rows.iter()
            .all(|r| !matches!(r, TreeRow::Folder { open: true, .. }))
    );
}

#[test]
fn two_entries_of_one_name_list_once_by_the_lower_id() {
    let mut apps = vec![
        app("Paint", "Graphics"),
        app("Notepad", "Utility"),
        app("Paint", "Utility"),
        app("Notes", "Utility"),
    ];
    apps[0].id = "z-paint.desktop".into();
    apps[2].id = "a-paint.desktop".into();
    apps[3].shown = false;
    let kept = one_per_name(&apps);
    assert_eq!(kept, [false, true, true, false]);
    let (rows, map) = tree_rows(&apps, &reprisefs::apps::group(&apps), &kept);
    assert_eq!(map, [None, Some(1), Some(2)], "Graphics is left empty");
    assert!(matches!(&rows[0], TreeRow::Folder { name, .. } if name == "Accessories"));
    assert_eq!(panel_matches("pa", &apps, &kept, 8), [2]);
}

#[test]
fn the_program_search_lists_names_with_a_word_starting_with_the_text() {
    let apps = [
        app("Network Connections", "Utility"),
        app("Notepad", "Utility"),
        app("Print Editor", "Utility"),
        app("Sticky Notes", "Utility"),
    ];
    let kept = one_per_name(&apps);
    assert_eq!(panel_matches("no", &apps, &kept, 8), [1, 3]);
    assert_eq!(panel_matches("no", &apps, &kept, 1), [1], "capped");
}

#[test]
fn a_rebuilt_tree_drops_the_scroll_and_the_lit_row() {
    let apps: Vec<AppEntry> = (0..30)
        .map(|k| app(&format!("App {k}"), "Utility"))
        .collect();
    let (rows, _) = tree_rows(&apps, &reprisefs::apps::group(&apps), &one_per_name(&apps));
    let mut p = StartPanel::new();
    replace_tree(&mut p, rows.clone());
    p.set_mode(Mode::Programs);
    p.toggle_folder(0);
    p.scroll_tree(5);
    p.key(Key::Up);
    p.key(Key::Up);
    assert!(matches!(p.lit(), Some(Spot::Tree(_))), "{:?}", p.lit());
    assert!(p.tree_top > 0);
    replace_tree(&mut p, rows);
    assert_eq!((p.tree_top, p.lit()), (0, None));
    assert_eq!(p.mode(), Mode::Programs);
}

fn bar_on(screen: Rect, edge: Edge) -> (BarPlacement, Rect) {
    let p = crate::barplace::place(screen, edge, 40);
    (p, crate::barplace::usable(screen, &p))
}

#[test]
fn panel_origin_follows_the_dock() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let (bottom, area) = bar_on(screen, Edge::Bottom);
    let h = layout::layout(&StartPanel::new()).panel.height;
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &bottom, (0, 0), area, (layout::W, h)),
        (0, 1040 - h)
    );
    let mut p = StartPanel::new();
    let row = ProgramRow {
        name: "Notepad".into(),
        icon: None,
        jump: false,
        new_pin: false,
    };
    p.pinned = vec![row.clone()];
    p.frequent = vec![row];
    let tall = layout::layout(&p).panel.height;
    assert_eq!(tall, h + 4);
    assert_eq!(
        panel_origin(
            StartMenuDock::OnTop,
            &bottom,
            (0, 0),
            area,
            (layout::W, tall)
        )
        .1,
        1040 - h - 4
    );
    assert_eq!(
        panel_origin(
            StartMenuDock::Into(4),
            &bottom,
            (0, 0),
            area,
            (layout::W, h)
        )
        .1,
        1044 - h
    );
    let (top, area) = bar_on(screen, Edge::Top);
    for height in [h, tall] {
        let y = panel_origin(
            StartMenuDock::OnTop,
            &top,
            (0, 0),
            area,
            (layout::W, height),
        )
        .1;
        assert_eq!(y, 40, "a {height} px panel hangs from the bar");
        assert_eq!(y - layout::OVERHANG, 14, "the picture is on the screen");
    }
    assert_eq!(
        panel_origin(StartMenuDock::Into(4), &top, (0, 0), area, (layout::W, h)).1,
        36,
        "a dock into the bar reaches up into it"
    );
    let thin = crate::barplace::place(screen, Edge::Top, 20);
    let thin_area = crate::barplace::usable(screen, &thin);
    let y = panel_origin(
        StartMenuDock::OnTop,
        &thin,
        (0, 0),
        thin_area,
        (layout::W, h),
    )
    .1;
    assert_eq!((y, y - layout::OVERHANG), (26, 0));
}

/// On a 768-line screen a panel as tall as its room stands whole by every bar, its picture
/// on the screen.
#[test]
fn a_panel_of_its_room_stands_whole_with_its_picture_on_the_screen() {
    let screen = Rect::new(0, 0, 1024, 768);
    for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
        let (p, area) = bar_on(screen, edge);
        let start = (p.shown.x, p.shown.y);
        for dock in [StartMenuDock::OnTop, StartMenuDock::Into(4)] {
            let h = panel_room(dock, &p, area);
            let (_, y) = panel_origin(dock, &p, start, area, (layout::W, h));
            assert!(
                y - layout::OVERHANG >= 0,
                "{edge:?} {dock:?}: picture at {y}"
            );
            assert!(y + h <= screen.bottom(), "{edge:?} {dock:?}");
        }
    }
    let (bottom, area) = bar_on(screen, Edge::Bottom);
    assert_eq!(
        panel_room(StartMenuDock::OnTop, &bottom, area),
        728 - layout::OVERHANG
    );
    let (top, area) = bar_on(screen, Edge::Top);
    assert_eq!(panel_room(StartMenuDock::OnTop, &top, area), 768 - 40);
}

#[test]
fn a_hidden_bars_panel_stands_on_its_reveal_line() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let h = layout::layout(&StartPanel::new()).panel.height;
    let at = |edge, dock| {
        let p = crate::barplace::place_hiding(screen, edge, 40, Some(1.0));
        let area = crate::barplace::usable(screen, &p);
        panel_origin(dock, &p, (0, 0), area, (layout::W, h)).1
    };
    for dock in [StartMenuDock::Into(4), StartMenuDock::OnTop] {
        assert_eq!(
            at(Edge::Bottom, dock) + h,
            1078,
            "{dock:?}: over a bottom bar the panel's foot is the line's top"
        );
        let y = at(Edge::Top, dock);
        assert_eq!(
            (y, y - layout::OVERHANG),
            (26, 0),
            "{dock:?}: under a top bar the panel hangs off the line, its picture's top the output's"
        );
    }
    let shown = crate::barplace::place_hiding(screen, Edge::Bottom, 40, Some(0.0));
    let area = crate::barplace::usable(screen, &shown);
    assert_eq!(
        panel_origin(StartMenuDock::Into(4), &shown, (0, 0), area, (layout::W, h)).1,
        1044 - h
    );
}

#[test]
fn the_panel_stays_inside_the_usable_area() {
    let screen = Rect::new(1920, 0, 1280, 1024);
    let h = layout::layout(&StartPanel::new()).panel.height;
    for edge in [Edge::Bottom, Edge::Top] {
        let (p, area) = bar_on(screen, edge);
        assert_eq!(
            panel_origin(
                StartMenuDock::OnTop,
                &p,
                (3150, 0),
                area,
                (layout::JUMP_W, h)
            )
            .0,
            1920 + 1280 - layout::JUMP_W,
            "{edge:?}"
        );
        let (x, y) = panel_origin(StartMenuDock::OnTop, &p, (1900, 0), area, (layout::W, h));
        assert_eq!(x, 1920, "{edge:?}");
        assert!(
            y >= area.y && y + h <= area.bottom(),
            "{edge:?}: {y} in {area:?}"
        );
    }
}

#[test]
fn a_jump_rows_tip_is_its_files_path_with_the_home_folder_as_a_tilde() {
    let jumps = vec![
        Vec::new(),
        vec![
            PathBuf::from("/home/u/Docs/notes.txt"),
            PathBuf::from("/srv/share/plan.odt"),
        ],
    ];
    let home = Some(Path::new("/home/u"));
    let tip = |mode, k| jump_file(&jumps, mode, k).map(|p| crate::search::tilde(p, home));
    assert_eq!(tip(Mode::Jump(1), 0).as_deref(), Some("~/Docs/notes.txt"));
    assert_eq!(
        tip(Mode::Jump(1), 1).as_deref(),
        Some("/srv/share/plan.odt")
    );
    assert_eq!(tip(Mode::Jump(1), 2), None, "past the rows");
    assert_eq!(tip(Mode::Jump(0), 0), None);
    assert_eq!(tip(Mode::Programs, 0), None, "no jump list shown");
}

#[test]
fn a_short_area_keeps_the_panels_bottom_inside() {
    let h = layout::layout(&StartPanel::new()).panel.height;
    let at = |screen: Rect, edge| {
        let (p, area) = bar_on(screen, edge);
        panel_origin(StartMenuDock::OnTop, &p, (0, 0), area, (layout::W, h)).1
    };
    // 493 tall over a 40 px bottom bar: the panel ends on the area's bottom
    // and the picture rises 9 px past its top.
    let y = at(Rect::new(0, 0, 1920, 533), Edge::Bottom);
    assert_eq!(y + h, 493, "the panel's bottom is the area's");
    assert_eq!(y - layout::OVERHANG, -9, "the picture rises past the top");
    // 470 tall under a 40 px top bar: the panel rises 6 px over the bar.
    let y = at(Rect::new(0, 0, 1920, 510), Edge::Top);
    assert_eq!((y, y + h), (34, 510));
    assert_eq!(at(Rect::new(0, 0, 1920, 340), Edge::Bottom), 0);
    assert_eq!(at(Rect::new(0, 0, 1920, 340), Edge::Top), 0);
    let screen = Rect::new(0, 0, 1920, 1080);
    let (bottom, _) = bar_on(screen, Edge::Bottom);
    let above = Rect::new(0, 0, 1920, 1000);
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &bottom, (0, 0), above, (layout::W, h)).1,
        1000 - h
    );
    let (top, _) = bar_on(screen, Edge::Top);
    let below = Rect::new(0, 80, 1920, 1000);
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &top, (0, 0), below, (layout::W, h)).1,
        80
    );
}

#[test]
fn the_shutdown_options_open_on_the_screen_side_of_the_bar() {
    let usable = Rect::new(0, 40, 1920, 1040);
    let arrow = Rect::new(339, 474, 22, 22);
    let size = (120, 130);
    assert_eq!(
        shutdown_options_at(Edge::Bottom, arrow, size, usable),
        (361, 496 - 130),
        "over a bottom bar: level with the arrow's foot"
    );
    assert_eq!(
        shutdown_options_at(Edge::Top, arrow, size, usable),
        (361, 474),
        "under a top bar: level with the arrow's top"
    );
    let low = Rect::new(339, 1000, 22, 22);
    assert_eq!(
        shutdown_options_at(Edge::Top, low, size, usable),
        (361, 1022 - 130)
    );
    for edge in [Edge::Left, Edge::Right] {
        assert_eq!(
            shutdown_options_at(edge, arrow, size, usable),
            (361, 474),
            "{edge:?}: Start tops a side bar, so they drop as under a top bar"
        );
    }
}

#[test]
fn the_walking_keys_win_over_the_fields_edit_key() {
    assert_eq!(key_of(&press(keys::KEY_Return, 0)), Some(Key::Enter));
    assert_eq!(key_of(&press(keys::KEY_Escape, 0)), Some(Key::Escape));
    assert_eq!(key_of(&press(keys::KEY_KP_Up, 0)), Some(Key::Up));
    assert_eq!(key_of(&press(keys::KEY_Left, 0)), Some(Key::Left));
    assert_eq!(
        key_of(&press(keys::KEY_Right, keys::MOD_MOD2)),
        Some(Key::Right),
        "Num Lock makes no chord"
    );
    assert_eq!(key_of(&press(keys::KEY_Tab, 0)), Some(Key::Tab));
    assert_eq!(key_of(&press(keys::KEY_Menu, 0)), Some(Key::Menu));
    assert_eq!(
        key_of(&press(keys::KEY_F10, keys::MOD_SHIFT)),
        Some(Key::Menu)
    );
    assert_eq!(
        key_of(&press(keys::KEY_x, 0)),
        Some(Key::Char(EditKey::Insert("x".into())))
    );
}

#[test]
fn a_chorded_caret_key_edits_the_field_and_shift_tab_does_nothing() {
    let shift_left = DialogKey {
        syms: vec![keys::KEY_Left],
        mods: keys::MOD_SHIFT,
        edit: Some(EditKey::Left {
            select: true,
            word: false,
        }),
    };
    assert_eq!(
        key_of(&shift_left),
        Some(Key::Char(EditKey::Left {
            select: true,
            word: false
        }))
    );
    assert_eq!(
        key_of(&press(keys::KEY_ISO_Left_Tab, keys::MOD_SHIFT)),
        None
    );
    let shift = DialogKey {
        syms: vec![keys::KEY_Shift_L],
        mods: keys::MOD_SHIFT,
        edit: None,
    };
    assert_eq!(key_of(&shift), None, "a modifier alone does nothing");
}

#[test]
fn the_user_picture_is_looked_for_in_face_then_accounts_service() {
    assert_eq!(
        picture_paths(Some(Path::new("/home/jan")), "jan"),
        [
            PathBuf::from("/home/jan/.face"),
            PathBuf::from("/var/lib/AccountsService/icons/jan")
        ]
    );
    assert_eq!(
        picture_paths(None, "../x"),
        Vec::<PathBuf>::new(),
        "a user name is never a path"
    );
}

#[test]
fn the_user_picture_is_the_first_path_that_decodes_at_each_size() {
    let dir = std::env::temp_dir().join(format!("reprise-face-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut c = Canvas::new(8, 8);
    c.fill_rect(Rect::new(0, 0, 8, 8), crate::theme::Color::rgb(0, 128, 255));
    std::fs::write(dir.join("face.png"), reprisekit::raster::to_png(&c)).unwrap();
    std::fs::write(dir.join("broken"), "not a png").unwrap();
    let paths = [dir.join("gone"), dir.join("broken"), dir.join("face.png")];
    let found = read_pictures(&paths, &[48, 96]);
    let sizes: Vec<_> = (found.iter())
        .map(|(px, c)| (*px, c.as_ref().map(|c| (c.width, c.height))))
        .collect();
    assert_eq!(sizes, [(48, Some((48, 48))), (96, Some((96, 96)))]);
    assert_eq!(read_pictures(&paths[..2], &[48]), [(48, None)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_user_picture_read_runs_at_a_time_and_opens_show_the_last_read() {
    let t0 = Instant::now();
    let mut p = UserPictures::default();
    assert_eq!(p.at(48), None, "nothing read yet");
    let (a, sizes) = p.start(vec![96, 48, 96], t0).unwrap();
    assert_eq!(sizes, [48, 96]);
    assert_eq!(p.start(vec![48], t0), None, "a read is under way");
    assert!(p.land(a, vec![(48, Some(Canvas::new(48, 48))), (96, None)]));
    assert_eq!(p.at(48).map(|c| c.map(|c| c.width)), Some(Some(48)));
    assert_eq!(p.at(96), Some(None), "read, no picture");
    let (b, _) = p.start(vec![48], t0).unwrap();
    assert!(p.land(b, vec![(48, None)]));
    assert_eq!(p.at(48), Some(None), "the picture was removed");
    assert_eq!(p.at(96), Some(None));
}

#[test]
fn a_size_not_read_yet_shows_the_largest_picture_read_resampled() {
    let mut big = Canvas::new(96, 96);
    big.pixels.fill(0xFF20_4060);
    let mut p = UserPictures::default();
    assert_eq!(p.shown(48), None, "nothing read: avatar-default");
    let (a, _) = p.start(vec![48, 96], Instant::now()).unwrap();
    assert!(p.land(a, vec![(48, Some(Canvas::new(48, 48))), (96, Some(big))]));
    let shown = p.shown(144).expect("the 96 px picture resampled to 144");
    assert_eq!((shown.width, shown.height), (144, 144));
    assert!(shown.pixels.iter().all(|&px| px == 0xFF20_4060));
    let (b, _) = p.start(vec![48], Instant::now()).unwrap();
    assert!(p.land(b, vec![(48, None)]));
    assert_eq!(
        p.shown(48),
        None,
        "a read that found no picture shows avatar-default"
    );
}

#[test]
fn a_user_picture_read_stuck_on_a_dead_home_lets_a_later_open_read_again() {
    let stuck = crate::fsjobs::STUCK;
    let t0 = Instant::now();
    let mut p = UserPictures::default();
    let (held, _) = p.start(vec![48], t0).unwrap();
    assert_eq!(
        p.start(vec![48], t0 + stuck / 2),
        None,
        "the held read is young"
    );
    let (b, _) = (p.start(vec![48], t0 + stuck))
        .expect("an open after the held read turned stuck reads again");
    assert!(p.land(b, vec![(48, Some(Canvas::new(48, 48)))]));
    assert!(
        !p.land(held, vec![(48, None)]),
        "the held read landing late is stale"
    );
    assert_eq!(p.at(48).map(|c| c.map(|c| c.width)), Some(Some(48)));
}

#[test]
fn the_panel_outlives_its_output_once_the_scene_drops_it() {
    let out = |name: &str| {
        Output::new(
            name.into(),
            smithay::output::PhysicalProperties {
                size: (0, 0).into(),
                subpixel: smithay::output::Subpixel::Unknown,
                make: "Reprise".into(),
                model: "Test".into(),
                serial_number: "0".into(),
            },
        )
    };
    let (a, b) = (out("A-1"), out("B-1"));
    assert!(!outlived(&a, &[a.clone(), b.clone()]));
    assert!(outlived(&a, &[b]), "A-1 was removed");
    assert!(outlived(&a, &[]));
}

#[test]
fn the_pointer_takes_the_panel_and_the_picture_only() {
    let l = layout::layout(&StartPanel::new());
    let t = takes_pointer(&l);
    let at = |x: i32, y: i32| Point::from(((x - l.canvas.x) as f64, (y - l.canvas.y) as f64));
    assert!(t(at(0, 0)));
    assert!(t(at(l.panel.width - 1, l.panel.height - 1)));
    let pic = l.picture.expect("Home shows the picture");
    assert!(t(at(pic.x + 1, pic.y + 1)), "the picture over the top edge");
    assert!(!t(at(1, -1)), "above the panel beside the picture");
    assert!(!t(at(pic.right(), -1)), "right of the picture");
}

#[test]
fn the_buffer_is_the_panel_and_the_picture_above_it() {
    let l = layout::layout(&StartPanel::new());
    let pic = l.picture.expect("Home shows the picture");
    assert_eq!(l.canvas, l.panel.union(&pic));
    assert_eq!((l.canvas.x, l.canvas.y), (0, pic.y), "the buffer's offset");
    assert_eq!(l.canvas.bottom(), l.panel.bottom());
}

fn panels() -> Vec<StartPanel> {
    let row = |jump| ProgramRow {
        name: "Notepad".into(),
        icon: None,
        jump,
        new_pin: false,
    };
    let home = StartPanel::new();
    let mut both = StartPanel::new();
    both.pinned = vec![row(true)];
    both.frequent = vec![row(false)];
    let mut search = both.clone();
    search.set_mode(Mode::Search);
    let mut jump = both.clone();
    jump.set_mode(Mode::Jump(0));
    vec![home, both, search, jump]
}

#[test]
fn the_glass_lies_under_every_pixel_the_panel_draws_and_no_other() {
    use reprisekit::glass::{Material, Settings};
    let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
    let text = reprisekit::interfaces::start_panel_text(&theme);
    for p in panels() {
        let l = layout::layout(&p);
        let mut c = Canvas::new(l.canvas.width as u32, l.canvas.height as u32);
        reprisekit::interfaces::draw_start_panel(
            &mut c,
            &theme,
            &Settings::default(),
            &text,
            &p,
            &l,
            None,
            true,
        );
        let pane = panel_pane(&l);
        assert_eq!(
            pane.holes,
            [l.pane],
            "{:?}: the white pane is clear",
            p.mode()
        );
        let shapes = crate::glass::drawn_shapes(&pane, Material::Frosted, 1.0);
        let alpha = |x: i32, y: i32| c.get(x - l.canvas.x, y - l.canvas.y) >> 24;
        let r = l.panel;
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let glass = shapes.iter().any(|(s, _)| s.contains(x, y));
                assert_eq!(
                    glass,
                    alpha(x, y) > 0,
                    "{:?} {}x{}: glass at ({x}, {y})",
                    p.mode(),
                    r.width,
                    r.height
                );
            }
        }
        let h = l.pane;
        let corner = |x: i32, y: i32| {
            (x < h.x + 5 || x >= h.right() - 5) && (y < h.y + 5 || y >= h.bottom() - 5)
        };
        for y in h.y..h.bottom() {
            for x in h.x..h.right() {
                if !corner(x, y) {
                    assert_eq!(alpha(x, y), 255, "{:?}: pane at ({x}, {y})", p.mode());
                }
            }
        }
    }
}

#[test]
fn the_panels_glass_rounds_its_top_corners_above_scale_1_as_its_chrome_does() {
    use reprisekit::glass::Material;
    let l = layout::layout(&StartPanel::new());
    let pane = panel_pane(&l);
    for scale in [1.5, 2.0] {
        assert_eq!(
            crate::glass::drawn_shapes(&pane, Material::Frosted, scale),
            vec![(l.panel, [6.0, 6.0, 0.0, 0.0])],
            "at {scale}"
        );
    }
    assert!(crate::glass::drawn_shapes(&pane, Material::Frosted, 1.0).len() > 1);
}

#[test]
fn beside_a_side_bar_the_panel_hangs_level_with_start_its_picture_on_the_screen() {
    let screen = Rect::new(0, 0, 1920, 1080);
    let h = layout::layout(&StartPanel::new()).panel.height;
    let (left, area) = bar_on(screen, Edge::Left);
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &left, (3, 0), area, (layout::W, h)),
        (40, layout::OVERHANG),
        "on the bar's right edge, the picture's top the output's"
    );
    assert_eq!(
        panel_origin(StartMenuDock::Into(4), &left, (3, 0), area, (layout::W, h)).0,
        36
    );
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &left, (3, 1000), area, (layout::W, h)).1,
        1080 - h,
        "kept above the output's foot"
    );
    let (right, area) = bar_on(screen, Edge::Right);
    assert_eq!(
        panel_origin(
            StartMenuDock::OnTop,
            &right,
            (1883, 0),
            area,
            (layout::W, h)
        )
        .0,
        1880 - layout::W,
        "its right edge on the bar's left edge"
    );
    let narrow = Rect::new(1920, 0, 400, 1080);
    let (right, area) = bar_on(narrow, Edge::Right);
    assert_eq!(
        panel_origin(
            StartMenuDock::OnTop,
            &right,
            (2283, 0),
            area,
            (layout::W, h)
        )
        .0,
        1920,
        "a right bar's panel stops at the output's left edge"
    );
    let (left, area) = bar_on(narrow, Edge::Left);
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &left, (1923, 0), area, (layout::W, h)).0,
        1920,
        "a left bar's panel stops at the output's left edge"
    );
    let wide = Rect::new(0, 0, 440, 1080);
    let (left, area) = bar_on(wide, Edge::Left);
    assert_eq!(
        panel_origin(StartMenuDock::OnTop, &left, (3, 0), area, (layout::W, h)).0,
        440 - layout::W,
        "a left bar's panel stops at the output's right edge"
    );
    let hidden = crate::barplace::place_hiding(screen, Edge::Left, 40, Some(1.0));
    let area = crate::barplace::usable(screen, &hidden);
    assert_eq!(
        panel_origin(
            StartMenuDock::Into(4),
            &hidden,
            (3, 0),
            area,
            (layout::W, h)
        )
        .0,
        2,
        "a hidden bar's panel stands on its reveal line"
    );
}

#[test]
fn opening_and_closing_start_redraws_only_its_own_bar() {
    let mut config = crate::config::Config::default();
    config.startmenu_style = crate::config::StartMenuStyle::Glass;
    let mut h = crate::harness::Harness::with_config(config);
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1024, 768);
    let (own, other) = (h.state.bars[0].tree, h.state.bars[1].tree);
    let draws = |h: &crate::harness::Harness| {
        (
            h.state.bar(own).unwrap().draws,
            h.state.bar(other).unwrap().draws,
        )
    };
    let (own_before, other_before) = draws(&h);
    h.state.toggle_start(own);
    assert!(h.state.start_panel.is_some());
    assert!(h.state.bar(own).unwrap().start_pressed);
    h.state.toggle_start(own);
    assert!(h.state.start_panel.is_none());
    assert!(!h.state.bar(own).unwrap().start_pressed);
    let (own_after, other_after) = draws(&h);
    assert_eq!(
        own_after - own_before,
        2,
        "Start's bar is drawn on open and on close"
    );
    assert_eq!(
        other_after, other_before,
        "the other output's bar is not drawn"
    );
}

#[test]
fn opening_and_closing_start_redraws_only_starts_part_of_the_bar() {
    for interface in ["classic", "glass"] {
        let toml = format!("[appearance]\ninterface = \"{interface}\"\n");
        let mut config = crate::config::Config::parse(&toml).unwrap();
        config.startmenu_style = crate::config::StartMenuStyle::Glass;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("TEST-1", 1024, 768);
        let bar = h.state.bars[0].tree;
        for open in [true, false] {
            let before = h.state.bar(bar).unwrap().start_only_draws;
            h.state.toggle_start(bar);
            assert_eq!(h.state.start_panel.is_some(), open);
            let b = h.state.bar(bar).unwrap();
            assert_eq!(b.start_only_draws, before + 1, "{interface} open {open}");
            let d = b.drawn.as_ref().unwrap();
            assert_eq!(d.state.start_pressed, open);
            let theme = h.state.config.theme.scaled(d.scale);
            let mut want = Canvas::new(d.canvas.width, d.canvas.height);
            let s = &h.state;
            reprisekit::look::draw_taskbar(
                &mut want,
                &theme,
                s.text.for_output(theme.scale),
                s.start_text.for_output(theme.scale),
                &d.layout,
                &d.state,
            );
            assert!(
                d.canvas.pixels == want.pixels,
                "{interface} open {open}: the bar as a full draw gives it"
            );
        }
    }
}

#[test]
fn opening_and_closing_start_asks_only_its_output_for_frames() {
    let mut config = crate::config::Config::default();
    config.startmenu_style = crate::config::StartMenuStyle::Glass;
    let mut h = crate::harness::Harness::with_config(config);
    h.output("TEST-1", 1024, 768);
    h.output("TEST-2", 1024, 768);
    let own = h.state.bars[0].tree;
    for open in [true, false] {
        crate::backend::take_asks();
        h.state.toggle_start(own);
        assert_eq!(h.state.start_panel.is_some(), open);
        let asked = crate::backend::take_asks();
        assert!(!asked.is_empty(), "open {open}: Start asks for a frame");
        assert!(
            asked.iter().all(|(o, _)| o == "TEST-1"),
            "open {open}: only TEST-1, Start's output: {asked:?}"
        );
    }
}
