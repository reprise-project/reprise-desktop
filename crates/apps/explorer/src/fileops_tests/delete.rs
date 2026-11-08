use super::*;

/// The tree menu's Delete, Cut and Copy take the node's folder, not the file selected in the view.
#[test]
fn the_tree_menus_file_rows_take_the_node() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::Builder::new()
        .prefix("treemenu")
        .tempdir()
        .expect("a temp dir");
    let dir = home.path().join("folder");
    let sub = dir.join("Reports");
    std::fs::create_dir_all(&sub).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.settings.show_hidden = true;
    ex.sv
        .selection
        .set_only(ex.items.iter().position(|it| it.name == "a.txt"));
    ex.build_tree(&mut ctx);
    ex.tree_follow(&Place::Folder(sub.clone()), &mut ctx);
    assert_eq!(ex.node_folder(), Some((sub.clone(), "Reports".to_string())));

    let reply = ex.run(Command::NodeCopy, &mut ctx);
    assert_eq!(reply.copy_files.map(|c| c.paths), Some(vec![sub.clone()]));

    ex.run(Command::NodeDelete, &mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::Confirm(c)) => assert_eq!(c.paths, vec![sub.clone()]),
        other => panic!("no confirm box: {:?}", other.is_some()),
    }
    assert!(sub.exists());
    assert!(dir.join("a.txt").exists());
}

/// A delete prompt wears a prompt's caption (no icon, close box alone); Properties the ? box; the window its own.
#[test]
fn a_delete_prompt_has_the_prompt_caption() {
    use repriseapp::Caption;
    let _guard = lock(&TRASH_ENV);
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(matches!(ex.dialog, Some(Dialog::Confirm(_))), "Delete asks");
    assert_eq!(ex.caption(crate::dialogs::DIALOG), Caption::Prompt);
    assert_eq!(ex.caption(crate::props_dialog::PROPS), Caption::Help);
    assert_eq!(ex.caption(repriseapp::MAIN), Caption::Window);
}

/// Right moves the delete prompt's focus from Yes to No, and Enter presses No: the prompt closes, the file stays.
#[test]
fn an_arrow_moves_the_delete_prompt_s_focus_and_enter_presses_it() {
    let _guard = lock(&TRASH_ENV);
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(matches!(ex.dialog, Some(Dialog::Confirm(_))), "Delete asks");
    ex.dialog_event(right(), &mut ctx);
    assert!(
        matches!(ex.dialog, Some(Dialog::Confirm(_))),
        "Right leaves the prompt up"
    );
    ex.dialog_event(enter(), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog.is_none(), "Enter on No closes the prompt");
    assert!(dir.path().join("a.txt").exists(), "Enter on No keeps a.txt");
}

/// Shift+Delete's prompt opens on No, so Enter keeps the file; Delete's opens on Yes.
#[test]
fn enter_on_the_shift_delete_prompt_keeps_the_file() {
    use reprisekit::controls::dlgtemplate::BUTTON_IDS;
    let _guard = lock(&TRASH_ENV);
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    let focused = |ex: &Explorer| {
        let f = ex.box_form().expect("the prompt's form");
        f.focus().map(|i| f.items[i].id)
    };
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert_eq!(focused(&ex), Some(BUTTON_IDS[0]), "Delete opens on Yes");
    ex.dialog = None;
    ex.run(Command::Delete { erase: true }, &mut ctx);
    assert_eq!(
        focused(&ex),
        Some(BUTTON_IDS[1]),
        "Shift+Delete opens on No"
    );
    ex.dialog_event(enter(), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog.is_none(), "Enter closes the prompt");
    assert!(dir.path().join("a.txt").exists(), "Enter keeps a.txt");
}

/// A drive's node offers no file rows: Cut, Copy and Delete do nothing.
#[test]
fn the_tree_menus_file_rows_skip_a_drive() {
    let home = tempfile::Builder::new()
        .prefix("treedrive")
        .tempdir()
        .expect("a temp dir");
    let dir = home.path().join("folder");
    let stick = dir.join("Stick");
    std::fs::create_dir_all(&stick).expect("mkdir");
    std::fs::write(stick.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.tree_follow(&Place::Folder(stick.clone()), &mut ctx);
    assert!(ex.node_folder().is_some(), "a plain folder has file rows");

    ex.drive_mounts = vec![stick.clone()];
    assert_eq!(ex.node_folder(), None);
    assert!(ex.run(Command::NodeCut, &mut ctx).copy_files.is_none());
    ex.run(Command::NodeDelete, &mut ctx);
    assert!(ex.dialog.is_none(), "Delete opens no confirm box");
    assert!(stick.join("a.txt").exists());
}

#[test]
fn delete_trashes_the_selection_and_undo_restores_it() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(ex.dialog.is_some());
    assert!(dir.join("a.txt").exists());

    ex.delete_now(&[dir.join("a.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(!dir.join("a.txt").exists());
    assert_eq!(
        std::fs::read_dir(home.path().join("Trash/files"))
            .expect("the trash")
            .count(),
        1
    );
    assert_eq!(ex.undo, Some("Delete"));
    assert!(ex.items.is_empty(), "the view re-listed without it");

    ex.run(Command::Undo, &mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("read"),
        "1"
    );
    assert_eq!(ex.items.len(), 1, "the view re-listed with it back");
    assert_eq!(ex.undo, None);
}

/// A delete that makes room in a full Recycle Bin (1% of the drive here) erases its oldest item, and
/// that item's Undo leaves the stack with it instead of failing on the erased one.
#[test]
fn a_delete_that_purges_the_bin_drops_the_purged_items_undo() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("max_percent = 1\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let cap = reprisefs::bincap::drive_size(home.path()) / 100;
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    for name in ["a.iso", "b.iso", "c.iso"] {
        let path = dir.join(name);
        std::fs::File::create(&path)
            .and_then(|f| f.set_len(cap * 2 / 5))
            .expect("a sparse file");
        ex.delete_now(&[path], false, &mut ctx);
        ex.finish_puts(&mut ctx);
    }
    let trashed: Vec<_> = std::fs::read_dir(home.path().join("Trash/files"))
        .expect("the trash")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(trashed.len(), 2, "the oldest was erased: {trashed:?}");
    assert!(!trashed.iter().any(|n| n == "a.iso"));

    ex.run(Command::Undo, &mut ctx);
    ex.run(Command::Undo, &mut ctx);

    assert!(dir.join("b.iso").exists() && dir.join("c.iso").exists());
    assert_eq!(ex.undo, None, "a.iso's undo went with it");
    assert!(ex.dialog.is_none(), "no error box");
}

/// An item bigger than its whole Recycle Bin (1% of the drive here) raises "too big for the Recycle
/// Bin" before anything is deleted: No leaves it and trashes the rest of the batch, Yes erases it.
#[test]
fn an_item_too_big_for_the_bin_asks_to_be_deleted_for_good() {
    use reprisekit::controls::form;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("max_percent = 1\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let cap = reprisefs::bincap::drive_size(home.path()) / 100;
    let (big, small) = (dir.join("big.iso"), dir.join("small.txt"));
    std::fs::File::create(&big)
        .and_then(|f| f.set_len(cap + 1))
        .expect("a sparse file");
    std::fs::write(&small, "s").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(&[big.clone(), small.clone()], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(
        c.text,
        "'big.iso' is too big for the Recycle Bin. Do you want to delete it permanently?"
    );
    assert_eq!(c.title, "Confirm File Delete");
    assert_eq!(c.buttons(), ["&Yes", "Yes to &All", "&No", "Cancel"]);
    assert!(
        big.exists() && small.exists(),
        "nothing goes before the answer"
    );

    ex.dialog_outcome(form::Event::Pressed("btn2"), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog.is_none());
    assert!(big.exists(), "No leaves it");
    assert!(!small.exists(), "the rest of the batch is trashed");

    ex.delete_now(std::slice::from_ref(&big), false, &mut ctx);
    ex.finish_puts(&mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(c.buttons(), ["&Yes", "&No"], "one item: Yes or No");
    ex.dialog_outcome(form::Event::Default, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(!big.exists(), "Yes erases it");
    let trashed: Vec<_> = std::fs::read_dir(home.path().join("Trash/files"))
        .expect("the trash")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(trashed, ["small.txt"]);
}

/// A drive set to remove files immediately asks the plain delete question and erases; with confirmation off nothing asks.
#[test]
fn remove_immediately_erases_and_the_prompt_can_be_turned_off() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("remove_immediately = true\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the prompt is up")
    };
    assert!(c.erase);
    assert_eq!(c.text, "Are you sure you want to delete 'a.txt'?");
    ex.dialog = None;

    data_home.recycle_bin("remove_immediately = true\nconfirm = false\n");
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(ex.dialog.is_none(), "no prompt");
    ex.finish_puts(&mut ctx);
    assert!(!dir.join("a.txt").exists());
    assert!(!home.path().join("Trash/files").exists(), "nothing trashed");
}

/// Undo of a copy onto a drive that removes files immediately asks before erasing the copy: No
/// keeps it and the Undo row, Yes erases it with nothing put in the bin.
#[test]
fn undo_of_a_copy_on_a_remove_immediately_drive_asks_then_erases() {
    use reprisekit::controls::form;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let data_home = DataHome::set(home.path());
    data_home.recycle_bin("remove_immediately = true\n");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let copy = dir.join("copy.txt");
    std::fs::write(&copy, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.undos
        .push(reprisefs::undo::Undoable::Copied(copy.clone()));
    ex.sync_undo();

    ex.run(Command::Undo, &mut ctx);
    let Some(Dialog::Confirm(c)) = &ex.dialog else {
        panic!("the question is up")
    };
    assert_eq!(
        c.text,
        "'copy.txt' is on a drive that removes files immediately. Do you want to delete it permanently?"
    );
    assert_eq!(c.buttons(), ["&Yes", "&No"]);
    let no = reprisekit::controls::msgbox::button_id(1);
    ex.dialog_outcome(form::Event::Pressed(no), &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(copy.exists(), "No keeps the copy");
    assert_eq!(ex.undo, Some("Copy"), "and the Undo row");

    ex.run(Command::Undo, &mut ctx);
    ex.dialog_outcome(form::Event::Default, &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(!copy.exists(), "Yes erases it");
    assert!(!home.path().join("Trash/files").exists(), "nothing trashed");
    assert_eq!(ex.undo, None);
}

/// Delete on a drive that cannot hold a trash asks to delete for good, and says why.
#[test]
fn delete_on_a_drive_with_no_recycle_bin_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let drive = home.path().join("drive");
    std::fs::create_dir(&drive).expect("mkdir");
    std::fs::write(drive.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let rows = format!(
        "/dev/a {} ext4 rw 0 0\n/dev/b {} vfat ro 0 0\n",
        home.path().display(),
        drive.display()
    );
    std::fs::write(&table, rows).expect("write");
    reprisefs::trash::set_mount_table(Some(table));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(drive.clone()), &mut ctx);
    std::fs::set_permissions(&drive, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Delete { erase: false }, &mut ctx);

    std::fs::set_permissions(&drive, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let Some(Dialog::Confirm(c)) = ex.dialog.as_ref() else {
        panic!("a confirm box");
    };
    assert!(c.erase);
    assert_eq!(
        c.text,
        "This drive has no Recycle Bin. Are you sure you want to delete 'd.txt'?"
    );
}

#[test]
fn erasing_a_symlink_to_a_folder_removes_only_the_link() {
    let home = tempfile::tempdir().expect("a temp dir");
    let target = home.path().join("target");
    let dir = home.path().join("dir");
    std::fs::create_dir_all(&target).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(target.join("x.txt"), "x").expect("write");
    let link = dir.join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(std::slice::from_ref(&link), true, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(!link.exists() && std::fs::symlink_metadata(&link).is_err());
    assert!(target.join("x.txt").is_file(), "the target is untouched");
    assert!(ex.dialog.is_none());
}
