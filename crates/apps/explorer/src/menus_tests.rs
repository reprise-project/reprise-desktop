//! Tests for `menus.rs`, kept apart so that file stays under 800 lines.

use super::*;
fn enabled(items: &[MenuItem], i: usize) -> bool {
    matches!(items[i], MenuItem::Entry { enabled: true, .. })
}

fn label(items: &[MenuItem], i: usize) -> &str {
    match &items[i] {
        MenuItem::Entry { label, .. } => label,
        MenuItem::Separator => "",
    }
}

fn bar_rows(state: &MenuState, t: usize) -> Vec<MenuItem> {
    bar_menus(state)[t].1.items().to_vec()
}

#[test]
fn every_row_with_no_linux_counterpart_is_greyed() {
    let s = MenuState::default();
    let edit = bar_rows(&s, 1);
    for i in [7, 8] {
        assert!(!enabled(&edit, i), "edit row {i}");
    }
    assert!(!enabled(&bar_rows(&s, 2), 12));
    for title in [3, 4] {
        let rows = bar_rows(&s, title);
        for (i, it) in rows.iter().enumerate() {
            if matches!(it, MenuItem::Entry { .. }) {
                assert!(!enabled(&rows, i), "title {title} row {i}");
            }
        }
    }
}

#[test]
fn the_tools_rows() {
    let tools = bar_rows(&MenuState::default(), 4);
    let labels: Vec<&str> = (0..tools.len()).map(|i| label(&tools, i)).collect();
    assert_eq!(labels, ["Synchronize...", "", "Folder Options..."]);
}

#[test]
fn favorites_and_help_rows() {
    let s = MenuState::default();
    assert_eq!(label(&bar_rows(&s, 3), 4), "Web Sites");
    let help = bar_rows(&s, 5);
    assert_eq!(help.len(), 3);
    assert_eq!(label(&help, 0), "Help and Support");
    assert_eq!(label(&help, 2), "About Explorer");
}

#[test]
fn the_mode_rows_carry_access_keys() {
    let view = bar_rows(&MenuState::default(), 2);
    // "Choose Details..." is another h and "Tiles" another l if a marker slips.
    for (key, want) in [('h', 4), ('s', 5), ('n', 6), ('l', 7), ('d', 8)] {
        assert_eq!(
            reprisekit::controls::menu::rows_for_key(&view, key).next(),
            Some(want),
            "View + {key}"
        );
    }
}

#[test]
fn the_current_mode_is_the_bulleted_row() {
    for (mode, want) in [
        (Mode::Thumbnails, 4),
        (Mode::Tiles, 5),
        (Mode::Icons, 6),
        (Mode::List, 7),
        (Mode::Details, 8),
    ] {
        let view = bar_rows(
            &MenuState {
                mode,
                ..Default::default()
            },
            2,
        );
        for (i, item) in view.iter().enumerate().take(9).skip(4) {
            let MenuItem::Entry { checked, radio, .. } = item else {
                panic!("view row {i} is a separator");
            };
            assert!(radio, "view row {i} is not a radio row");
            assert_eq!(*checked, i == want, "view row {i} for {mode:?}");
        }
    }
}

#[test]
fn the_views_drop_bullets_the_same_row() {
    let m = views_menu(Mode::List);
    assert_eq!(m.items().len(), 5);
    assert_eq!(m.command(&[3]), Command::Mode(Mode::List));
    let MenuItem::Entry { checked, .. } = &m.items()[3] else {
        panic!("not an entry");
    };
    assert!(checked);
}

#[test]
fn edits_first_row_is_undo_and_names_the_action() {
    let state = MenuState::default();
    assert_eq!(command_for(1, &[0], &state), Command::Undo);
    let edit = bar_rows(&state, 1);
    assert!(!enabled(&edit, 0));
    assert_eq!(label(&edit, 0), "Undo");

    let state = MenuState {
        can_undo: Some("Delete"),
        ..Default::default()
    };
    let edit = bar_rows(&state, 1);
    assert!(enabled(&edit, 0));
    assert_eq!(label(&edit, 0), "Undo Delete");
}

#[test]
fn files_item_rows_grey_with_no_selection() {
    let none = bar_rows(&MenuState::default(), 0);
    for i in [2, 3, 4] {
        assert!(!enabled(&none, i), "file row {i}");
    }
    // Properties opens the folder's own sheet; it greys only where no folder
    // is behind the place (Computer, the Recycle Bin).
    assert!(enabled(&none, 5), "Properties is the folder's own sheet");
    let nowhere = bar_rows(
        &MenuState {
            read_only: true,
            ..Default::default()
        },
        0,
    );
    assert!(!enabled(&nowhere, 5), "no sheet where there is no folder");
    let some = bar_rows(
        &MenuState {
            selection: 1,
            ..Default::default()
        },
        0,
    );
    for i in [2, 3, 4, 5] {
        assert!(enabled(&some, i), "file row {i}");
    }
    assert!(enabled(&none, 7));
}

#[test]
fn paste_greys_without_a_clipboard_and_in_a_read_only_place() {
    assert!(!enabled(&bar_rows(&MenuState::default(), 1), 4));
    let s = MenuState {
        can_paste: true,
        ..Default::default()
    };
    assert!(enabled(&bar_rows(&s, 1), 4));
    let s = MenuState {
        can_paste: true,
        read_only: true,
        ..Default::default()
    };
    assert!(!enabled(&bar_rows(&s, 1), 4));
}

#[test]
fn the_status_bar_row_checks_itself() {
    let on = bar_rows(&MenuState::default(), 2);
    let MenuItem::Entry { checked, .. } = &on[1] else {
        panic!("not an entry");
    };
    assert!(checked);
    let off = bar_rows(
        &MenuState {
            status_bar: false,
            ..Default::default()
        },
        2,
    );
    let MenuItem::Entry { checked, .. } = &off[1] else {
        panic!("not an entry");
    };
    assert!(!checked);
}

#[test]
fn command_for_maps_the_menu_rows() {
    let s = MenuState::default();
    assert_eq!(command_for(0, &[7], &s), Command::Close);
    assert_eq!(command_for(2, &[1], &s), Command::ToggleStatusBar);
    assert_eq!(command_for(2, &[6], &s), Command::Mode(Mode::Icons));
    assert_eq!(command_for(2, &[14], &s), Command::Refresh);
    assert_eq!(command_for(5, &[2], &s), Command::About);
    assert_eq!(command_for(0, &[0], &s), Command::Nothing);
    assert_eq!(command_for(0, &[1], &s), Command::Nothing);
    assert_eq!(command_for(9, &[0], &s), Command::Nothing);
    assert_eq!(command_for(0, &[99], &s), Command::Nothing);
}

#[test]
fn the_bars_submenu_rows_dispatch() {
    let s = MenuState::default();
    assert_eq!(command_for(0, &[0, 0], &s), Command::NewFolder);
    assert_eq!(command_for(0, &[0, 3], &s), Command::NewTextDocument);
    assert_eq!(command_for(2, &[0, 0], &s), Command::ToggleToolbar(0));
    assert_eq!(command_for(2, &[0, 1], &s), Command::ToggleToolbar(1));
    assert_eq!(command_for(2, &[2, 2], &s), Command::ToggleFolders);
    for i in 0..4 {
        assert_eq!(command_for(2, &[10, i], &s), Command::Sort(i));
    }
    assert_eq!(command_for(2, &[13, 0], &s), Command::Back);
    assert_eq!(command_for(2, &[13, 1], &s), Command::Forward);
    assert_eq!(command_for(2, &[13, 2], &s), Command::Up);
    assert_eq!(
        command_for(2, &[13, 6], &s),
        Command::GoTo(Place::Documents)
    );
    assert_eq!(command_for(2, &[13, 7], &s), Command::GoTo(Place::Home));
    assert_eq!(command_for(2, &[13, 8], &s), Command::GoTo(Place::Computer));
}

#[test]
fn the_toolbars_submenu_checks_what_is_shown() {
    let s = MenuState {
        toolbars: [true, false, false],
        ..Default::default()
    };
    let sub = bar_menus(&s)[2].1.at(&[0]).expect("Toolbars").clone();
    let rows = sub.items();
    assert_eq!(label(rows, 0), "Standard Buttons");
    let MenuItem::Entry { checked, .. } = &rows[0] else {
        panic!()
    };
    assert!(checked);
    let MenuItem::Entry { checked, .. } = &rows[1] else {
        panic!()
    };
    assert!(!checked, "the Address Bar is off");
    assert!(!enabled(rows, 2));
}

#[test]
fn the_arrange_submenu_bullets_the_sort_column() {
    let s = MenuState {
        sort: (2, true),
        ..Default::default()
    };
    let sub = bar_menus(&s)[2].1.at(&[10]).expect("Arrange").clone();
    for i in 0..4 {
        let MenuItem::Entry { checked, radio, .. } = &sub.items()[i] else {
            panic!("row {i}")
        };
        assert!(radio);
        assert_eq!(*checked, i == 2, "row {i}");
    }
}

#[test]
fn the_context_menus_answer_for_their_submenu_rows() {
    let s = MenuState::default();
    let apps = [("Notepad".to_string(), None), ("Browser".to_string(), None)];
    let m = item_menu(
        &s,
        ItemKind::File,
        false,
        &apps,
        &[("Desktop".to_string(), true, None)],
    );
    assert_eq!(label(m.items(), 3), "Open With");
    assert_eq!(m.command(&[3, 0]), Command::OpenWith(0));
    assert_eq!(m.command(&[3, 1]), Command::OpenWith(1));
    assert_eq!(m.command(&[3, 3]), Command::ChooseProgram);
    assert_eq!(label(m.items(), 4), "Send To");
    assert_eq!(m.command(&[4, 0]), Command::SendTo(0));
    let m = item_menu(&s, ItemKind::File, false, &[], &[]);
    assert_eq!(m.command(&[3, 0]), Command::ChooseProgram);
    assert!(!enabled(m.items(), 4), "Send To with no targets");

    let m = background_menu(&s);
    assert_eq!(m.command(&[0, 4]), Command::Mode(Mode::Details));
    assert_eq!(m.command(&[2, 1]), Command::Sort(1));
    assert_eq!(m.command(&[11, 0]), Command::NewFolder);
    assert_eq!(m.command(&[11, 3]), Command::NewTextDocument);
}

#[test]
fn the_tree_menus_first_row_follows_the_node() {
    let shut = tree_menu(false, true, true);
    assert_eq!(label(shut.items(), 0), "Expand");
    assert!(enabled(shut.items(), 0));
    assert_eq!(shut.command(&[0]), Command::ToggleNode);
    let open = tree_menu(true, true, true);
    assert_eq!(label(open.items(), 0), "Collapse");
    let leaf = tree_menu(false, false, true);
    assert!(!enabled(leaf.items(), 0));
}

/// Tree menu rows act on the node, not the view's selection. A node that is
/// not a plain folder greys the file rows; Rename stays greyed (no tree editor).
#[test]
fn the_tree_menu_never_acts_on_the_views_selection() {
    let m = tree_menu(false, true, true);
    assert_eq!(label(m.items(), 1), "Explore");
    assert_eq!(m.command(&[1]), Command::OpenNode);
    assert_eq!(m.command(&[2]), Command::OpenNode);
    let live = |m: &Menu| -> Vec<(String, Command)> {
        (3..m.items().len())
            .filter(|&i| enabled(m.items(), i))
            .map(|i| (label(m.items(), i).to_string(), m.command(&[i])))
            .collect()
    };
    assert_eq!(
        live(&m),
        [
            ("Cut".to_string(), Command::NodeCut),
            ("Copy".to_string(), Command::NodeCopy),
            ("Delete".to_string(), Command::NodeDelete),
            ("Properties".to_string(), Command::NodeProperties),
        ]
    );
    let place = tree_menu(false, true, false);
    assert!(live(&place).is_empty());
}

#[test]
fn a_folders_menu_explores_and_a_files_opens_with() {
    let s = MenuState::default();
    let m = item_menu(&s, ItemKind::Folder, false, &[], &[]);
    assert_eq!(label(m.items(), 1), "Explore");
    assert_eq!(m.command(&[1]), Command::Explore);
    assert_eq!(label(m.items(), 2), "Sharing and Security...");
    let m = item_menu(&s, ItemKind::File, false, &[("Notepad".into(), None)], &[]);
    assert_eq!(label(m.items(), 1), "Run as...");
    assert!(!enabled(m.items(), 1));
    assert_eq!(label(m.items(), 3), "Open With");
    assert!(enabled(m.items(), 3));
}

#[test]
fn a_launcher_has_paste_under_copy_and_no_open_with() {
    let s = MenuState {
        selection: 1,
        can_paste: true,
        read_only: true,
        ..Default::default()
    };
    let m = item_menu(
        &s,
        ItemKind::Launcher,
        false,
        &[("Notepad".into(), None)],
        &[],
    );
    let labels: Vec<&str> = (0..m.items().len()).map(|i| label(m.items(), i)).collect();
    assert_eq!(
        labels,
        [
            "Open",
            "Run as...",
            "",
            "Send To",
            "",
            "Cut",
            "Copy",
            "Paste",
            "",
            "Create Shortcut",
            "Delete",
            "Rename",
            "",
            "Properties"
        ]
    );
    assert_eq!(m.command(&[7]), Command::PasteOnItem);
    assert!(
        enabled(m.items(), 7),
        "Paste opens files, so a read-only place keeps it"
    );
    let file = item_menu(&s, ItemKind::File, false, &[], &[]);
    assert!(
        (0..file.items().len()).all(|i| file.command(&[i]) != Command::PasteOnItem),
        "a plain file has no Paste"
    );
}

#[test]
fn a_folder_has_paste_into_it_under_copy() {
    let s = MenuState {
        selection: 1,
        can_paste: true,
        ..Default::default()
    };
    let m = item_menu(&s, ItemKind::Folder, false, &[], &[]);
    let copy = (0..m.items().len())
        .position(|i| label(m.items(), i) == "Copy")
        .expect("Copy");
    assert_eq!(label(m.items(), copy + 1), "Paste");
    assert_eq!(m.command(&[copy + 1]), Command::PasteOnItem);
    assert!(enabled(m.items(), copy + 1));
    let read_only = MenuState {
        read_only: true,
        ..s
    };
    let m = item_menu(&read_only, ItemKind::Folder, false, &[], &[]);
    assert!(
        !enabled(m.items(), copy + 1),
        "a drive under Computer takes no paste"
    );
}

#[test]
fn a_read_only_place_greys_every_writing_row() {
    let s = MenuState {
        read_only: true,
        selection: 1,
        can_paste: true,
        ..Default::default()
    };
    let m = item_menu(&s, ItemKind::File, false, &[], &[]);
    for (i, _) in m.items().iter().enumerate() {
        let c = m.command(&[i]);
        if matches!(
            c,
            Command::Cut | Command::Delete { .. } | Command::Rename | Command::CreateShortcut
        ) {
            assert!(!enabled(m.items(), i), "row {i} ({c:?})");
        }
    }
    assert!(!enabled(background_menu(&s).items(), 7), "Paste");
}

#[test]
fn send_to_rows_carry_their_icons() {
    let s = MenuState::default();
    let send_to = [
        (
            "Mail Recipient".to_string(),
            true,
            Some(Canvas::new(16, 16)),
        ),
        ("Desktop (create shortcut)".to_string(), false, None),
    ];
    let m = item_menu(&s, ItemKind::File, false, &[], &send_to);
    let rows = m.at(&[4]).expect("the Send To submenu").items();
    assert!(
        matches!(&rows[0], MenuItem::Entry { icon: Some(_), .. }),
        "Mail Recipient wears its icon"
    );
    assert!(!enabled(rows, 1), "Desktop (create shortcut) is greyed");
}

#[test]
fn an_ampersand_in_a_program_or_target_name_shows_and_is_no_access_key() {
    let s = MenuState {
        selection: 1,
        ..Default::default()
    };
    let open_with = [("R&D Viewer".to_string(), None)];
    let send_to = [("R&D share".to_string(), true, None)];
    let m = item_menu(&s, ItemKind::File, false, &open_with, &send_to);
    let sub = |name: &str| {
        let at = (0..m.items().len())
            .find(|&i| label(m.items(), i) == name)
            .unwrap_or_else(|| panic!("a {name} row"));
        m.at(&[at]).unwrap().items()
    };
    for (rows, want) in [
        (sub("Open With"), "R&D Viewer"),
        (sub("Send To"), "R&D share"),
    ] {
        assert_eq!(label(rows, 0), want);
        assert_eq!(
            rows[0].access_key(),
            Some('r'),
            "{want}: the first letter, not D"
        );
    }
}

/// Eject sits alone between Sharing and Security... and Copy, on access key j.
#[test]
fn only_a_removable_drives_menu_has_eject() {
    let s = MenuState {
        read_only: true,
        selection: 1,
        ..Default::default()
    };
    let has_eject = |m: &Menu| (0..m.items().len()).any(|i| m.command(&[i]) == Command::Eject);
    assert!(!has_eject(&item_menu(
        &s,
        ItemKind::Folder,
        false,
        &[],
        &[]
    )));
    let m = item_menu(&s, ItemKind::Folder, true, &[], &[]);
    let rows = m.items();
    let at = (0..rows.len())
        .find(|&i| m.command(&[i]) == Command::Eject)
        .expect("an Eject row");
    assert_eq!(label(rows, at), "Eject");
    assert!(enabled(rows, at));
    assert!(matches!(
        rows[at],
        MenuItem::Entry {
            mnemonic: Some(1),
            ..
        }
    ));
    assert!(matches!(rows[at - 1], MenuItem::Separator));
    assert!(matches!(rows[at + 1], MenuItem::Separator));
    assert_eq!(label(rows, 2), "Sharing and Security...");
    assert_eq!(label(rows, at + 3), "Copy", "Cut then Copy follow Eject");
}

/// A Recycle Bin item's menu: Restore, greyed Cut, Delete (for good), then
/// Properties as the bold default.
#[test]
fn a_recycle_bin_items_menu_is_restore_cut_delete_properties() {
    let s = MenuState {
        read_only: true,
        selection: 1,
        bin: Some(2),
        ..Default::default()
    };
    let m = item_menu(&s, ItemKind::File, false, &[("Notepad".into(), None)], &[]);
    let rows = m.items();
    let labels: Vec<&str> = (0..rows.len()).map(|i| label(rows, i)).collect();
    assert_eq!(
        labels,
        ["Restore", "", "Cut", "", "Delete", "", "Properties"]
    );
    assert_eq!(m.command(&[0]), Command::Restore);
    assert!(enabled(rows, 0));
    assert!(!enabled(rows, 2), "Cut greys");
    assert!(enabled(rows, 4));
    assert_eq!(m.command(&[4]), Command::Delete { erase: true });
    assert_eq!(m.command(&[6]), Command::Properties);
    assert!(matches!(rows[6], MenuItem::Entry { bold: true, .. }));
    for (key, want) in [('e', 0), ('r', 6)] {
        assert_eq!(
            reprisekit::controls::menu::rows_for_key(rows, key).next(),
            Some(want),
            "bin item menu + {key}"
        );
    }
}

/// File leads with Restore over a selection, else Empty Recycle Bin (greyed when empty).
#[test]
fn files_first_row_in_the_recycle_bin_is_restore_or_empty() {
    let bin = |selection, n| MenuState {
        read_only: true,
        selection,
        bin: Some(n),
        ..Default::default()
    };
    let file = |s: &MenuState| (bar_rows(s, 0), command_for(0, &[0], s));

    let (rows, cmd) = file(&bin(1, 3));
    assert_eq!(label(&rows, 0), "Restore");
    assert!(enabled(&rows, 0));
    assert_eq!(cmd, Command::Restore);
    assert!(enabled(&rows, 5), "Properties of the selection");

    let (rows, cmd) = file(&bin(0, 3));
    assert_eq!(label(&rows, 0), "Empty Recycle Bin");
    assert!(enabled(&rows, 0));
    assert_eq!(cmd, Command::EmptyRecycleBin);

    assert!(!enabled(&file(&bin(0, 0)).0, 0), "an empty bin greys it");
}

#[test]
fn the_recycle_bin_deletes_for_good_and_copies_nothing() {
    let s = MenuState {
        read_only: true,
        selection: 1,
        bin: Some(2),
        ..Default::default()
    };
    let file = bar_rows(&s, 0);
    assert_eq!(label(&file, 3), "Delete");
    assert!(enabled(&file, 3));
    assert_eq!(command_for(0, &[3], &s), Command::Delete { erase: true });
    assert!(
        !enabled(&file, 2) && !enabled(&file, 4),
        "Create Shortcut and Rename grey"
    );
    let none = MenuState { selection: 0, ..s };
    assert!(!enabled(&bar_rows(&none, 0), 3), "nothing to delete");

    let edit = bar_rows(&s, 1);
    assert_eq!(label(&edit, 3), "Copy");
    assert!(!enabled(&edit, 3));
    assert!(!enabled(&edit, 2), "Cut greys");

    let arrange = arrange_menu(&s);
    let items = arrange.items();
    let sorts: Vec<&str> = (0..6).map(|i| label(items, i)).collect();
    assert_eq!(
        sorts,
        [
            "Name",
            "Original Location",
            "Date Deleted",
            "Size",
            "Type",
            "Modified"
        ]
    );
}

#[test]
fn a_scripts_menu_runs_it_in_place_of_open_with_the_default_bold() {
    use reprisefs::program::ScriptOpen::{Run, Terminal};
    let bold =
        |items: &[MenuItem], i: usize| matches!(&items[i], MenuItem::Entry { bold: true, .. });
    let s = MenuState::default();
    let m = item_menu(&s, ItemKind::Script(Terminal), false, &[], &[]);
    assert_eq!(label(m.items(), 0), "Run");
    assert_eq!(label(m.items(), 1), "Run in terminal");
    assert_eq!(m.command(&[0]), Command::Run(Run));
    assert_eq!(m.command(&[1]), Command::Run(Terminal));
    assert!(!bold(m.items(), 0) && bold(m.items(), 1));
    assert_eq!(label(m.items(), 4), "Open With", "a script is still a file");
    let m = item_menu(&s, ItemKind::Script(Run), false, &[], &[]);
    assert!(bold(m.items(), 0) && !bold(m.items(), 1));
    let m = item_menu(&s, ItemKind::File, false, &[], &[]);
    assert_eq!(
        label(m.items(), 0),
        "Open",
        "a binary or document keeps Open"
    );
}
