//! Tests for `menus.rs`, kept apart so that file stays under 800 lines.

use super::*;
fn enabled(items: &[MenuItem], i: usize) -> bool {
    matches!(items[i], MenuItem::Entry { enabled: true, .. })
}

fn label(items: &[MenuItem], i: usize) -> &str {
    match &items[i] {
        MenuItem::Entry { label, .. } => label,
        MenuItem::Separator | MenuItem::Heading(_) => "",
    }
}

fn bar_rows(state: &MenuState, t: usize) -> Vec<MenuItem> {
    bar_menus(state)[t].1.items().to_vec()
}

#[test]
fn every_row_with_no_linux_counterpart_is_greyed() {
    let s = MenuState::default();
    assert!(!enabled(&bar_rows(&s, 2), 12));
}

#[test]
fn copy_and_move_to_folder_take_the_selected_files_to_the_folder_picker() {
    let rows = |s: &MenuState| {
        let edit = bar_rows(s, 1);
        [enabled(&edit, 7), enabled(&edit, 8)]
    };
    let file = MenuState {
        selection: 1,
        ..Default::default()
    };
    assert_eq!(label(&bar_rows(&file, 1), 7), "Copy To Folder...");
    assert_eq!(label(&bar_rows(&file, 1), 8), "Move To Folder...");
    assert_eq!(command_for(1, &[7], &file), Command::CopyTo);
    assert_eq!(command_for(1, &[8], &file), Command::MoveTo);
    assert_eq!(rows(&file), [true, true]);
    assert_eq!(
        rows(&MenuState::default()),
        [false, false],
        "nothing selected"
    );
    let read_only = MenuState {
        read_only: true,
        ..file.clone()
    };
    assert_eq!(
        rows(&read_only),
        [true, false],
        "a read-only place copies out"
    );
    let shell = MenuState {
        shell_rows: 1,
        ..file.clone()
    };
    assert_eq!(rows(&shell), [false, false], "only desktop rows selected");
    let bin = MenuState {
        bin: Some(1),
        ..read_only
    };
    assert_eq!(
        rows(&bin),
        [false, false],
        "the Recycle Bin restores instead"
    );
}

#[test]
fn the_tools_rows() {
    let tools = bar_rows(&MenuState::default(), 4);
    let labels: Vec<&str> = (0..tools.len()).map(|i| label(&tools, i)).collect();
    assert_eq!(labels, ["Synchronize...", "", "Folder Options..."]);
    assert!(!enabled(&tools, 0), "Synchronize has nothing behind it");
    assert!(enabled(&tools, 2));
    assert_eq!(
        command_for(4, &[2], &MenuState::default()),
        Command::FolderOptions
    );
}

#[test]
fn favorites_rows_follow_the_bookmarks() {
    let mut s = MenuState::default();
    let rows = bar_rows(&s, 3);
    let labels: Vec<&str> = (0..rows.len()).map(|i| label(&rows, i)).collect();
    assert_eq!(labels, ["Add to Favorites...", "Organize Favorites..."]);
    assert!(!enabled(&rows, 0), "no folder shown");
    assert!(enabled(&rows, 1));
    let go = Command::GoTo(Place::Home);
    s.can_add_favorite = true;
    s.favorites = vec![
        crate::favorites::MenuRow {
            label: "R&D".into(),
            go: Some(go.clone()),
            icon: crate::favorites::RowIcon::Folder,
        },
        crate::favorites::MenuRow {
            label: "Recent".into(),
            go: None,
            icon: crate::favorites::RowIcon::Server,
        },
    ];
    let (folder, server) = (Canvas::new(16, 16), Canvas::new(8, 8));
    s.favorite_icons = [Some(folder.clone()), Some(server.clone())];
    let rows = bar_rows(&s, 3);
    let icon = |i: usize| match &rows[i] {
        MenuItem::Entry { icon, .. } => icon.as_ref().map(|c| c.width),
        _ => None,
    };
    assert_eq!(
        (icon(3), icon(4)),
        (Some(folder.width), Some(server.width)),
        "a folder wears the folder icon, a server address the server's"
    );
    assert!(enabled(&rows, 0));
    assert_eq!(label(&rows, 3), "R&D", "an ampersand is no access key");
    assert!(enabled(&rows, 3) && !enabled(&rows, 4));
    assert_eq!(command_for(3, &[0], &s), Command::AddFavorite);
    assert_eq!(command_for(3, &[1], &s), Command::OrganizeFavorites);
    assert_eq!(command_for(3, &[3], &s), go);
}

#[test]
fn help_rows() {
    let s = MenuState::default();
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
fn a_picture_alone_lights_paste_but_not_paste_shortcut() {
    let s = MenuState {
        can_paste_image: true,
        ..Default::default()
    };
    let edit = bar_rows(&s, 1);
    assert!(enabled(&edit, 4), "Paste");
    assert!(!enabled(&edit, 5), "Paste Shortcut links files only");
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
fn hidden_items_is_ticked_with_hidden_items_shown_and_takes_ctrl_h_from_history() {
    let row = |hidden| {
        let rows = bar_rows(
            &MenuState {
                hidden,
                ..Default::default()
            },
            2,
        );
        match &rows[11] {
            MenuItem::Entry {
                label,
                checked,
                shortcut,
                ..
            } => (label.clone(), *checked, shortcut.clone()),
            MenuItem::Separator | MenuItem::Heading(_) => panic!("not an entry"),
        }
    };
    assert_eq!(
        row(false),
        ("Hidden Items".into(), false, Some("Ctrl+H".into()))
    );
    assert!(row(true).1);
    let bar = bar_menus(&MenuState::default())[2]
        .1
        .at(&[2])
        .expect("Explorer Bar")
        .clone();
    let history = &bar.items()[1];
    assert!(
        matches!(history, MenuItem::Entry { label, shortcut: None, .. } if label == "History"),
        "{history:?}"
    );
}

#[test]
fn command_for_maps_the_menu_rows() {
    let s = MenuState::default();
    assert_eq!(command_for(0, &[7], &s), Command::ConnectToServer);
    assert_eq!(command_for(0, &[9], &s), Command::Close);
    assert_eq!(command_for(2, &[1], &s), Command::ToggleStatusBar);
    assert_eq!(command_for(2, &[6], &s), Command::Mode(Mode::Icons));
    assert_eq!(command_for(2, &[11], &s), Command::ToggleHidden);
    assert_eq!(command_for(2, &[15], &s), Command::Refresh);
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
    let s = MenuState {
        templates: vec![reprisefs::templates::Template {
            path: "/home/u/Templates/Report.odt".into(),
            label: "Report".into(),
        }],
        ..MenuState::default()
    };
    assert_eq!(
        command_for(0, &[0, 4], &s),
        Command::NewFrom("/home/u/Templates/Report.odt".into()),
        "a template's row under Text Document"
    );
    assert_eq!(command_for(2, &[0, 0], &s), Command::ToggleToolbar(0));
    assert_eq!(command_for(2, &[0, 1], &s), Command::ToggleToolbar(1));
    assert_eq!(command_for(2, &[2, 2], &s), Command::ToggleFolders);
    for i in 0..4 {
        assert_eq!(command_for(2, &[10, i], &s), Command::Sort(i));
    }
    assert_eq!(command_for(2, &[14, 0], &s), Command::Back);
    assert_eq!(command_for(2, &[14, 1], &s), Command::Forward);
    assert_eq!(command_for(2, &[14, 2], &s), Command::Up);
    assert_eq!(
        command_for(2, &[14, 6], &s),
        Command::GoTo(Place::Documents)
    );
    assert_eq!(command_for(2, &[14, 7], &s), Command::GoTo(Place::Home));
    assert_eq!(command_for(2, &[14, 8], &s), Command::GoTo(Place::Computer));
    assert_eq!(
        command_for(2, &[14, 9], &s),
        Command::GoTo(Place::network())
    );
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
        &apps,
        &[("Desktop".to_string(), true, None)],
    );
    assert_eq!(label(m.items(), 3), "Open With");
    assert_eq!(m.command(&[3, 0]), Command::OpenWith(0));
    assert_eq!(m.command(&[3, 1]), Command::OpenWith(1));
    assert_eq!(m.command(&[3, 3]), Command::ChooseProgram);
    assert_eq!(label(m.items(), 4), "Send To");
    assert_eq!(m.command(&[4, 0]), Command::SendTo(0));
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    assert_eq!(m.command(&[3, 0]), Command::ChooseProgram);
    assert!(!enabled(m.items(), 4), "Send To with no targets");

    let m = background_menu(&s);
    assert_eq!(m.command(&[0, 4]), Command::Mode(Mode::Details));
    assert_eq!(m.command(&[2, 1]), Command::Sort(1));
    assert_eq!(m.command(&[12, 0]), Command::NewFolder);
    assert_eq!(m.command(&[12, 3]), Command::NewTextDocument);
}

#[test]
fn the_tree_menus_first_row_follows_the_node() {
    let shut = tree_menu(false, true, true, false, None);
    assert_eq!(label(shut.items(), 0), "Expand");
    assert!(enabled(shut.items(), 0));
    assert_eq!(shut.command(&[0]), Command::ToggleNode);
    let open = tree_menu(true, true, true, false, None);
    assert_eq!(label(open.items(), 0), "Collapse");
    let leaf = tree_menu(false, false, true, false, None);
    assert!(!enabled(leaf.items(), 0));
}

/// Tree menu rows act on the node, not the view's selection. A node that is
/// not a plain folder greys the file rows; Rename stays greyed (no tree editor).
#[test]
fn the_tree_menu_never_acts_on_the_views_selection() {
    let m = tree_menu(false, true, true, false, None);
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
    let place = tree_menu(false, true, false, false, None);
    assert!(live(&place).is_empty());
    let bin = tree_menu(false, true, false, true, None);
    assert_eq!(
        live(&bin),
        [("Properties".to_string(), Command::NodeProperties)],
        "the Recycle Bin's node opens its own sheet"
    );
}

#[test]
fn a_folders_menu_explores_and_a_files_opens_with() {
    let s = MenuState::default();
    let m = item_menu(&s, ItemKind::Folder, &[], &[]);
    assert_eq!(label(m.items(), 1), "Open in New Tab");
    assert_eq!(m.command(&[1]), Command::OpenInNewTab);
    assert_eq!(label(m.items(), 2), "Explore");
    assert_eq!(m.command(&[2]), Command::Explore);
    assert_eq!(label(m.items(), 4), "Disk Usage");
    assert_eq!(m.command(&[4]), Command::DiskUsage);
    assert_eq!(label(m.items(), 5), "Sharing and Security...");
    let m = item_menu(&s, ItemKind::File, &[("Notepad".into(), None)], &[]);
    assert_eq!(label(m.items(), 1), "Run as...");
    assert!(!enabled(m.items(), 1));
    assert_eq!(label(m.items(), 3), "Open With");
    assert!(enabled(m.items(), 3));
}

#[test]
fn an_archives_menu_extracts_all_here_or_into_its_folder() {
    let s = MenuState::default();
    let plain = item_menu(&s, ItemKind::File, &[], &[]);
    let rows = |m: &Menu| {
        (0..m.items().len())
            .map(|i| label(m.items(), i).to_string())
            .collect::<Vec<_>>()
    };
    assert!(!rows(&plain).iter().any(|r| r.starts_with("Extract")));
    let s = MenuState {
        extract: Some("pack/".into()),
        ..MenuState::default()
    };
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    let at = rows(&m)
        .iter()
        .position(|r| r == "Extract Here")
        .expect("Extract Here");
    assert_eq!(label(m.items(), at - 2), "Open With", "under Open With");
    assert_eq!(label(m.items(), at - 1), "Extract All...");
    assert_eq!(m.command(&[at - 1]), Command::ExtractAll);
    assert_eq!(m.command(&[at]), Command::Extract { here: true });
    assert_eq!(label(m.items(), at + 1), "Extract to pack/");
    assert_eq!(m.command(&[at + 1]), Command::Extract { here: false });
}

#[test]
fn a_compressed_folders_entries_open_copy_or_extract_and_nothing_else() {
    for extracts in [true, false] {
        let s = MenuState {
            archive: true,
            extract_entries: extracts,
            read_only: true,
            ..MenuState::default()
        };
        for kind in [ItemKind::File, ItemKind::Folder] {
            let m = item_menu(&s, kind, &[], &[]);
            let rows: Vec<&str> = (0..m.items().len()).map(|i| label(m.items(), i)).collect();
            let want = ["Open", "", "Copy", "", "Extract", "Extract All..."];
            assert_eq!(rows, want, "{kind:?}");
            assert_eq!(m.command(&[0]), Command::Open);
            assert_eq!(m.command(&[2]), Command::Copy);
            assert_eq!(m.command(&[4]), Command::ExtractEntries);
            assert_eq!(
                enabled(m.items(), 4),
                extracts,
                "Extract is live with a folder to go to"
            );
            assert_eq!(m.command(&[5]), Command::ExtractAll);
        }
    }
}

#[test]
fn compress_to_names_its_zip_above_send_to_on_files_and_folders() {
    let rows = |m: &Menu| {
        (0..m.items().len())
            .map(|i| label(m.items(), i).to_string())
            .collect::<Vec<_>>()
    };
    let plain = item_menu(&MenuState::default(), ItemKind::File, &[], &[]);
    assert!(!rows(&plain).iter().any(|r| r.starts_with("Compress")));
    let s = MenuState {
        extract: Some("pack/".into()),
        compress: Some("R&D.zip".into()),
        ..MenuState::default()
    };
    for kind in [ItemKind::File, ItemKind::Folder] {
        let m = item_menu(&s, kind, &[], &[]);
        let at = rows(&m)
            .iter()
            .position(|r| r == "Compress to 'R&D.zip'")
            .expect("Compress to");
        assert_eq!(m.command(&[at]), Command::Compress);
        assert_eq!(label(m.items(), at + 1), "Send To", "{kind:?}");
    }
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    let at = rows(&m).iter().position(|r| r == "Extract to pack/");
    assert_eq!(
        at.map(|i| label(m.items(), i + 1)),
        Some("Compress to 'R&D.zip'"),
        "under Extract"
    );
}

#[test]
fn a_disk_images_menu_mounts_by_default_and_still_opens() {
    let bold = |m: &Menu, i: usize| matches!(&m.items()[i], MenuItem::Entry { bold: true, .. });
    let s = MenuState {
        disk_image: true,
        ..MenuState::default()
    };
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    assert_eq!(label(m.items(), 0), "Mount");
    assert!(bold(&m, 0), "the default row");
    assert_eq!(m.command(&[0]), Command::MountImage);
    assert_eq!(label(m.items(), 1), "Open");
    assert!(!bold(&m, 1));
    assert_eq!(m.command(&[1]), Command::Open);
    let plain = item_menu(&MenuState::default(), ItemKind::File, &[], &[]);
    assert_eq!(label(plain.items(), 0), "Open");
    assert!(bold(&plain, 0));
    assert_eq!(label(plain.items(), 1), "Run as...", "no Mount row");
}

#[test]
fn a_launcher_has_paste_under_copy_and_no_open_with() {
    let s = MenuState {
        selection: 1,
        can_paste: true,
        read_only: true,
        ..Default::default()
    };
    let m = item_menu(&s, ItemKind::Launcher, &[("Notepad".into(), None)], &[]);
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
            "Copy as Path",
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
    let file = item_menu(&s, ItemKind::File, &[], &[]);
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
    let m = item_menu(&s, ItemKind::Folder, &[], &[]);
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
    let m = item_menu(&read_only, ItemKind::Folder, &[], &[]);
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
    let m = item_menu(&s, ItemKind::File, &[], &[]);
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
    let m = item_menu(&s, ItemKind::File, &[], &send_to);
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
    let m = item_menu(&s, ItemKind::File, &open_with, &send_to);
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
    let s = MenuState {
        extract: Some("R&D/".into()),
        ..s
    };
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    let row = (m.items().iter())
        .find(|r| matches!(r, MenuItem::Entry { label, .. } if label.starts_with("Extract to")))
        .expect("an Extract to row");
    assert!(matches!(row, MenuItem::Entry { label, .. } if label == "Extract to R&D/"));
    assert_eq!(row.access_key(), Some('a'), "Extract to's own key");
}

/// Eject sits alone between Sharing and Security... and Copy, on access key j.
#[test]
fn only_a_removable_drives_menu_has_eject() {
    let fixed = MenuState {
        read_only: true,
        selection: 1,
        ..Default::default()
    };
    let has_eject = |m: &Menu| (0..m.items().len()).any(|i| m.command(&[i]) == Command::Eject);
    assert!(!has_eject(&item_menu(&fixed, ItemKind::Folder, &[], &[])));
    let s = MenuState {
        eject: true,
        ..fixed
    };
    let m = item_menu(&s, ItemKind::Folder, &[], &[]);
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
    assert_eq!(label(rows, 3), "Disk Usage", "on a drive too");
    assert_eq!(label(rows, 4), "Sharing and Security...");
    assert_eq!(label(rows, at + 3), "Copy", "Cut then Copy follow Eject");
}

/// File's Eject is its own group under New, there only for a removable drive.
#[test]
fn the_file_menu_ejects_a_removable_drive() {
    let fixed = MenuState {
        read_only: true,
        selection: 1,
        ..Default::default()
    };
    let file = bar_rows(&fixed, 0);
    assert_eq!(
        label(&file, 2),
        "Create Shortcut",
        "no Eject on a fixed drive"
    );
    let removable = MenuState {
        eject: true,
        ..fixed
    };
    let file = bar_rows(&removable, 0);
    assert_eq!(label(&file, 2), "Eject");
    assert!(enabled(&file, 2));
    assert!(matches!(file[3], MenuItem::Separator));
    assert_eq!(command_for(0, &[2], &removable), Command::Eject);
    let share = MenuState {
        share: true,
        ..removable
    };
    assert_eq!(label(&bar_rows(&share, 0), 2), "Disconnect");
}

/// A Recycle Bin item's menu: Restore, greyed Cut and Copy as Path, Delete (for good), then
/// Properties as the bold default.
#[test]
fn a_recycle_bin_items_menu_is_restore_cut_delete_properties() {
    let s = MenuState {
        read_only: true,
        selection: 1,
        bin: Some(2),
        ..Default::default()
    };
    let m = item_menu(&s, ItemKind::File, &[("Notepad".into(), None)], &[]);
    let rows = m.items();
    let labels: Vec<&str> = (0..rows.len()).map(|i| label(rows, i)).collect();
    assert_eq!(
        labels,
        [
            "Restore",
            "",
            "Cut",
            "Copy as Path",
            "",
            "Delete",
            "",
            "Properties"
        ]
    );
    assert_eq!(m.command(&[0]), Command::Restore);
    assert!(enabled(rows, 0));
    assert!(!enabled(rows, 2), "Cut greys");
    assert!(
        !enabled(rows, 3),
        "Copy as Path greys: the item has no path of its own"
    );
    assert!(enabled(rows, 5));
    assert_eq!(m.command(&[5]), Command::Delete { erase: true });
    assert_eq!(m.command(&[7]), Command::Properties);
    assert!(matches!(rows[7], MenuItem::Entry { bold: true, .. }));
    for (key, want) in [('e', 0), ('r', 7)] {
        assert_eq!(
            reprisekit::controls::menu::rows_for_key(rows, key).next(),
            Some(want),
            "bin item menu + {key}"
        );
    }
}

/// Copy as Path sits under a file's Copy (under Paste for a folder), live only when the selection has local paths.
#[test]
fn copy_as_path_follows_copy_and_greys_without_local_paths() {
    for local in [true, false] {
        let s = MenuState {
            selection: 1,
            local_paths: local,
            ..Default::default()
        };
        let m = item_menu(&s, ItemKind::File, &[], &[]);
        let rows = m.items();
        let copy = (0..rows.len())
            .find(|&i| label(rows, i) == "Copy")
            .expect("Copy");
        assert_eq!(label(rows, copy + 1), "Copy as Path");
        assert_eq!(m.command(&[copy + 1]), Command::CopyAsPath);
        assert_eq!(enabled(rows, copy + 1), local);
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
    let none = MenuState {
        selection: 0,
        ..s.clone()
    };
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
    let m = item_menu(&s, ItemKind::Script(Terminal), &[], &[]);
    assert_eq!(label(m.items(), 0), "Run");
    assert_eq!(label(m.items(), 1), "Run in terminal");
    assert_eq!(m.command(&[0]), Command::Run(Run));
    assert_eq!(m.command(&[1]), Command::Run(Terminal));
    assert!(!bold(m.items(), 0) && bold(m.items(), 1));
    assert_eq!(label(m.items(), 4), "Open With", "a script is still a file");
    let m = item_menu(&s, ItemKind::Script(Run), &[], &[]);
    assert!(bold(m.items(), 0) && !bold(m.items(), 1));
    let m = item_menu(&s, ItemKind::File, &[], &[]);
    assert_eq!(
        label(m.items(), 0),
        "Open",
        "a binary or document keeps Open"
    );
}

#[test]
fn open_in_terminal_shows_on_a_folder_and_the_background_of_a_place_on_disk() {
    let rows = |m: &Menu| -> Vec<(String, Command)> {
        (0..m.items().len())
            .map(|i| (label(m.items(), i).to_string(), m.command(&[i])))
            .collect()
    };
    let has = |m: &Menu, cmd: Command| rows(m).contains(&("Open in Terminal".into(), cmd));
    let s = MenuState::default();
    let folder = item_menu(&s, ItemKind::Folder, &[], &[]);
    assert_eq!(
        rows(&folder)[3],
        ("Open in Terminal".into(), Command::OpenTerminal)
    );
    assert!(enabled(folder.items(), 3));
    let back = background_menu(&s);
    assert_eq!(
        rows(&back)[4],
        ("Open in Terminal".into(), Command::OpenTerminalHere)
    );
    for kind in [
        ItemKind::File,
        ItemKind::Launcher,
        ItemKind::Script(ScriptOpen::Run),
    ] {
        assert!(
            !has(&item_menu(&s, kind, &[], &[]), Command::OpenTerminal),
            "{kind:?}"
        );
    }

    let computer = MenuState {
        read_only: true,
        ..MenuState::default()
    };
    assert!(!has(&background_menu(&computer), Command::OpenTerminalHere));
    let drive = item_menu(&computer, ItemKind::Folder, &[], &[]);
    assert!(!has(&drive, Command::OpenTerminal));
    let bin = MenuState {
        read_only: true,
        bin: Some(1),
        ..MenuState::default()
    };
    assert!(!has(&background_menu(&bin), Command::OpenTerminalHere));
    assert!(!has(
        &item_menu(&bin, ItemKind::Folder, &[], &[]),
        Command::OpenTerminal
    ));
}

#[test]
fn a_lone_folders_menu_pins_or_unpins_it() {
    let pinned = |p| MenuState {
        pinned: p,
        ..MenuState::default()
    };
    let m = item_menu(&pinned(Some(false)), ItemKind::Folder, &[], &[]);
    assert_eq!(label(m.items(), 4), "Pin to Places");
    assert_eq!(m.command(&[4]), Command::Pin(true));
    let m = item_menu(&pinned(Some(true)), ItemKind::Folder, &[], &[]);
    assert_eq!(label(m.items(), 4), "Unpin from Places");
    assert_eq!(m.command(&[4]), Command::Pin(false));
    assert_eq!(label(m.items(), 5), "Disk Usage", "under the pin row");
    let m = item_menu(&pinned(None), ItemKind::Folder, &[], &[]);
    assert_eq!(label(m.items(), 4), "Disk Usage", "no pin row");
}

#[test]
fn a_tree_node_with_a_folder_pins_or_unpins_it() {
    let m = tree_menu(false, true, false, false, Some(false));
    assert_eq!(label(m.items(), 3), "Pin to Places");
    assert_eq!(m.command(&[3]), Command::NodePin(true));
    let m = tree_menu(false, true, true, false, Some(true));
    assert_eq!(label(m.items(), 3), "Unpin from Places");
    assert_eq!(m.command(&[3]), Command::NodePin(false));
    let m = tree_menu(false, true, false, false, None);
    assert!(!(0..m.items().len()).any(|i| matches!(m.command(&[i]), Command::NodePin(_))));
}

#[test]
fn the_recycle_bin_with_nothing_selected_offers_properties() {
    let bin = MenuState {
        read_only: true,
        bin: Some(0),
        ..MenuState::default()
    };
    let live = |m: &Menu| {
        (0..m.items().len())
            .any(|i| m.command(&[i]) == Command::Properties && enabled(m.items(), i))
    };
    assert!(live(&bar_menus(&bin)[0].1), "File > Properties");
    assert!(live(&background_menu(&bin)), "the background's Properties");
    let computer = MenuState {
        read_only: true,
        ..MenuState::default()
    };
    assert!(!live(&background_menu(&computer)), "Computer has none");
}

fn enabled_by_label(items: &[MenuItem], want: &str) -> bool {
    let i = (0..items.len()).find(|&i| label(items, i) == want);
    enabled(items, i.unwrap_or_else(|| panic!("a {want} row")))
}

#[test]
fn a_shell_rows_menu_opens_it_and_has_properties_where_its_place_has_a_sheet() {
    for (place, sheet) in [
        (Place::Documents, true),
        (Place::Home, true),
        (Place::Computer, false),
        (Place::network(), false),
        (Place::RecycleBin, true),
    ] {
        let m = shell_row_menu(&place);
        let rows = m.items();
        let labels: Vec<&str> = (0..rows.len()).map(|i| label(rows, i)).collect();
        assert_eq!(
            labels,
            ["Open", "Open in New Tab", "Explore", "", "Properties"],
            "{place:?}"
        );
        assert!(matches!(rows[0], MenuItem::Entry { bold: true, .. }));
        assert_eq!(enabled(rows, 4), sheet, "{place:?}'s Properties");
        assert_eq!(m.command(&[0]), Command::Open);
        assert_eq!(m.command(&[4]), Command::Properties);
    }
}

#[test]
fn the_file_rows_grey_while_only_desktop_rows_are_selected() {
    let only_rows = MenuState {
        selection: 2,
        shell_rows: 2,
        ..Default::default()
    };
    let with_a_file = MenuState {
        selection: 3,
        ..only_rows.clone()
    };
    for (title, want) in [
        (0, "Create Shortcut"),
        (0, "Delete"),
        (0, "Rename"),
        (1, "Cut"),
        (1, "Copy"),
    ] {
        assert!(
            !enabled_by_label(&bar_rows(&only_rows, title), want),
            "{want} greys"
        );
        assert!(
            enabled_by_label(&bar_rows(&with_a_file, title), want),
            "{want} takes the file"
        );
    }
    assert!(enabled_by_label(&bar_rows(&only_rows, 0), "Properties"));
}

#[test]
fn properties_greys_while_the_selected_rows_have_no_sheet() {
    let sheetless = MenuState {
        selection: 1,
        shell_rows: 1,
        sheetless: true,
        ..Default::default()
    };
    assert!(!enabled_by_label(&bar_rows(&sheetless, 0), "Properties"));
    assert!(!enabled_by_label(
        background_menu(&sheetless).items(),
        "Properties"
    ));
    let with_sheet = MenuState {
        sheetless: false,
        ..sheetless.clone()
    };
    assert!(enabled_by_label(&bar_rows(&with_sheet, 0), "Properties"));
    assert!(enabled_by_label(
        background_menu(&with_sheet).items(),
        "Properties"
    ));
}

/// Adds to `out` each access key rows of one level of `tree` share, under `path`.
fn shared_keys(tree: &reprisekit::controls::menu::MenuTree, path: &str, out: &mut Vec<String>) {
    let mut keys: std::collections::BTreeMap<char, Vec<&str>> = Default::default();
    for (i, item) in tree.items().iter().enumerate() {
        let MenuItem::Entry { label, .. } = item else {
            continue;
        };
        if let Some(key) = item.access_key() {
            let same = keys.entry(key).or_default();
            if !same.contains(&label.as_str()) {
                same.push(label);
            }
        }
        if let Some(child) = tree.child(i) {
            shared_keys(child, &format!("{path} > {label}"), out);
        }
    }
    for (key, same) in keys.into_iter().filter(|(_, s)| s.len() > 1) {
        out.push(format!("{path}: '{key}' on {same:?}"));
    }
}

#[test]
fn no_two_rows_of_a_menu_share_an_access_key() {
    let states = [
        MenuState::default(),
        MenuState {
            selection: 1,
            can_paste: true,
            can_undo: Some("Delete"),
            extract: Some("photos".into()),
            compress: Some("photos.zip".into()),
            local_paths: true,
            disk_image: true,
            relabel: true,
            pinned: Some(false),
            eject: true,
            can_connect: true,
            can_add_favorite: true,
            ..MenuState::default()
        },
        MenuState {
            selection: 1,
            eject: true,
            share: true,
            pinned: Some(true),
            ..MenuState::default()
        },
        MenuState {
            bin: Some(2),
            selection: 1,
            ..MenuState::default()
        },
    ];
    let apps = [("Notepad".to_string(), None)];
    let targets = [("Desktop".to_string(), true, None)];
    let mut found = Vec::new();
    for (n, s) in states.iter().enumerate() {
        for (title, menu) in bar_menus(s) {
            shared_keys(&menu.tree(), &format!("{n} {title}"), &mut found);
        }
        shared_keys(
            &background_menu(s).tree(),
            &format!("{n} background"),
            &mut found,
        );
        let kinds = [
            ItemKind::Folder,
            ItemKind::File,
            ItemKind::Script(ScriptOpen::Run),
            ItemKind::Script(ScriptOpen::Terminal),
            ItemKind::Launcher,
        ];
        for kind in kinds {
            let m = item_menu(s, kind, &apps, &targets);
            shared_keys(&m.tree(), &format!("{n} {kind:?}"), &mut found);
        }
    }
    for bin in [false, true] {
        for pinned in [None, Some(false), Some(true)] {
            let m = tree_menu(true, true, true, bin, pinned);
            shared_keys(&m.tree(), &format!("tree {bin} {pinned:?}"), &mut found);
        }
    }
    shared_keys(&views_menu(Mode::Icons).tree(), "views", &mut found);
    let drop = drop_menu(DropOp::Move, repriseapp::Actions::ALL);
    shared_keys(&drop.tree(), "drop", &mut found);
    assert!(
        found.is_empty(),
        "rows sharing an access key:\n{}",
        found.join("\n")
    );
}

#[test]
fn a_compressed_folder_of_a_zip_that_takes_files_pastes_and_makes_new_items() {
    let s = MenuState {
        archive: true,
        read_only: true,
        into_zip: true,
        can_paste: true,
        ..MenuState::default()
    };
    let back = background_menu(&s);
    assert!(enabled_by_label(back.items(), "Paste"));
    assert!(!enabled_by_label(back.items(), "Paste Shortcut"));
    assert!(enabled_by_label(back.items(), "New"));
    assert!(enabled_by_label(&bar_rows(&s, 1), "Paste"));
    for (row, live) in [
        ("Folder", true),
        ("Shortcut...", false),
        ("Text Document", true),
        ("Compressed (zipped) Folder", true),
    ] {
        assert_eq!(enabled_by_label(new_menu(&s).items(), row), live, "{row}");
    }
    let shut = MenuState {
        into_zip: false,
        ..s
    };
    assert!(!enabled_by_label(background_menu(&shut).items(), "Paste"));
    assert!(!enabled_by_label(background_menu(&shut).items(), "New"));
}
