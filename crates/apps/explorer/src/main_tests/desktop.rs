use super::*;

fn row_of(ex: &Explorer, place: &Place) -> usize {
    (0..ex.items.len())
        .find(|&i| ex.shell_row(i).as_ref() == Some(place))
        .unwrap_or_else(|| panic!("a row for {place:?}"))
}

#[test]
fn the_desktop_view_lists_what_its_tree_node_holds() {
    let mut ctx = test_ctx();
    let ex = test_explorer(Place::Desktop, &mut ctx);
    let rows: Vec<Place> = (0..ex.items.len()).map_while(|i| ex.shell_row(i)).collect();
    assert_eq!(
        rows,
        place::desktop_places(),
        "the rows lead, in the tree's order"
    );
    let names: Vec<&str> = ex.sv.entries[..5].iter().map(|e| e.name.as_str()).collect();
    let home = reprisefs::places::home_label();
    assert_eq!(
        names,
        [
            "Documents",
            home.as_str(),
            "Computer",
            "Network",
            "Recycle Bin"
        ]
    );
    for node in &ex.tree.roots[0].children {
        let place = &ex.tree_places[node.id as usize];
        assert!(
            (ex.items.iter())
                .any(|it| Place::from_path(&it.path) == *place && it.label() == node.label),
            "the view lists the tree's {}",
            node.label
        );
    }
}

#[test]
fn a_desktop_row_opens_its_place_and_up_selects_it_again() {
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Desktop, &mut ctx);
    for place in [Place::Computer, Place::RecycleBin] {
        ex.sv.selection.set_only(Some(row_of(&ex, &place)));
        ex.open_selected(&mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| {
            ex.place == place && ex.reading.is_none()
        });
        ex.run(Command::Up, &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| {
            ex.place == Place::Desktop && ex.reading.is_none()
        });
        let selected = ex.sv.selection.single().and_then(|i| ex.shell_row(i));
        assert_eq!(
            selected,
            Some(place.clone()),
            "Up from {place:?} selects its row"
        );
    }
}

#[test]
fn a_desktop_row_takes_no_file_operation() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = rows_over(dir.path(), &mut ctx);
    for place in place::desktop_places() {
        let i = row_of(&ex, &place);
        ex.sv.selection.set_only(Some(i));
        for cmd in [
            Command::Cut,
            Command::Copy,
            Command::CopyAsPath,
            Command::Delete { erase: false },
            Command::Delete { erase: true },
            Command::Rename,
            Command::CreateShortcut,
            Command::MoveTo,
            Command::CopyTo,
            Command::Compress,
        ] {
            let reply = ex.run(cmd.clone(), &mut ctx);
            let what = format!("{cmd:?} on {place:?}");
            assert!(
                reply.copy_files.is_none() && reply.copy.is_none(),
                "{what} copies"
            );
            assert!(ex.dialog().is_none(), "{what} puts nothing up");
            assert!(ex.sv.edit.is_none(), "{what} opens no rename box");
            assert!(!ex.sv.entries[i].cut, "{what} greys no row");
        }
        ex.arm_drag(i, BTN_LEFT, 0, 0);
        assert!(ex.drag.is_none(), "{place:?} is not dragged");
        assert_eq!(ex.selected_paths(), Vec::<PathBuf>::new());
    }
    let computer = row_of(&ex, &Place::Computer);
    ex.sv.selection.set_only(Some(computer));
    ex.run(Command::Properties, &mut ctx);
    assert!(
        ex.dialog().is_none() && ex.props.is_none(),
        "Computer has no sheet"
    );
    assert_eq!(std::fs::read_dir(dir.path()).expect("read").count(), 1);
}

#[test]
fn a_drop_on_a_desktop_row_lands_in_its_places_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let ex = rows_over(dir.path(), &mut ctx);
    for place in place::desktop_places() {
        let want = (!place.read_only()).then(|| place.path()).flatten();
        assert_eq!(ex.entry_dir(row_of(&ex, &place)), want, "{place:?}");
    }
    assert!(
        ex.entry_dir(row_of(&ex, &Place::Home)).is_some(),
        "Home takes drops"
    );
}

#[test]
fn properties_greys_where_the_selected_rows_open_no_sheet() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = rows_over(dir.path(), &mut ctx);
    let file = ex.index_of(&dir.path().join("a.txt")).expect("a.txt");
    let row = |p: Place| row_of(&ex, &p);
    let cases = [
        (vec![row(Place::Computer)], true),
        (vec![row(Place::network())], true),
        (vec![row(Place::Documents)], false),
        (vec![row(Place::RecycleBin)], false),
        (vec![row(Place::Documents), row(Place::Home)], true),
        (vec![row(Place::Computer), file], false),
        (vec![], false),
    ];
    for (picked, sheetless) in cases {
        ex.sv.selection.clear();
        for i in &picked {
            ex.sv.selection.set.insert(*i);
        }
        assert_eq!(ex.menu_state().sheetless, sheetless, "{picked:?}");
    }
}
