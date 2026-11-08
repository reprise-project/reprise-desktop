use super::*;

/// A test that deletes without a [`DataHome`] gets the scratch Recycle Bin: its home trash in the
/// scratch home, no drives' trashes and default settings, whatever the developer's own are.
#[test]
fn a_test_without_a_data_home_gets_the_scratch_bin() {
    let _guard = lock(&TRASH_ENV);
    test_ctx();
    let bins = reprisefs::trash::Bins::system().expect("a bin");
    let scratch =
        std::env::temp_dir().join(format!("reprise-explorer-tests-{}", std::process::id()));
    assert_eq!(bins.home, scratch.join("data/Trash"));
    assert_eq!(bins.mounts, Vec::<PathBuf>::new());
    assert_eq!(bins.policy, reprisefs::bincap::Policy::default());
    let home = tempfile::tempdir().expect("a temp dir");
    drop(DataHome::set(home.path()));
    let again = reprisefs::trash::Bins::system().expect("a bin");
    assert_eq!(
        again.home, bins.home,
        "a data home puts the scratch one back"
    );
}

/// Restore in the Recycle Bin puts the selected item back (with nothing selected, the rest). Undo no
/// longer offers to restore what Restore already put back.
#[test]
fn restore_puts_the_selection_back_then_everything() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    for n in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(n), n).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.delete_now(&[dir.join("a.txt")], false, &mut ctx);
    ex.delete_now(&[dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);

    let b = ex
        .items
        .iter()
        .position(|it| it.name == "b.txt")
        .expect("b");
    ex.sv.selection.set_only(Some(b));
    assert!(ex.run(Command::Open, &mut ctx).spawn.is_empty());
    assert!(
        ex.props.take().is_some(),
        "Open showed the Properties sheet"
    );
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("b.txt")).expect("b"),
        "b.txt"
    );
    assert!(!dir.join("a.txt").exists(), "only the selection came back");
    assert_eq!(ex.items.len(), 1, "the bin re-listed without it");
    assert_eq!(ex.undo, Some("Delete"), "a's delete is still undoable");

    ex.sv.selection.set_only(None);
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("a"),
        "a.txt"
    );
    assert!(ex.items.is_empty());
    assert_eq!(ex.undo, None);
    assert!(ex.dialog.is_none(), "no error box");
}

/// A file on another drive goes to that drive's own trash. The Recycle Bin lists it beside the home
/// trash's items, Restore puts it back, Empty clears both.
#[test]
fn the_recycle_bin_holds_every_drives_trash() {
    use std::os::unix::fs::MetadataExt;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let (dir, drive) = (home.path().join("folder"), home.path().join("drive"));
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::create_dir(&drive).expect("mkdir");
    std::fs::write(dir.join("h.txt"), "h").expect("write");
    std::fs::write(drive.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let rows = format!(
        "/dev/a {} ext4 rw 0 0\n/dev/b {} vfat rw 0 0\n",
        home.path().display(),
        drive.display()
    );
    std::fs::write(&table, rows).expect("write");
    reprisefs::trash::set_mount_table(Some(table));
    let uid = std::fs::metadata(home.path()).expect("stat").uid();
    let drive_files = drive.join(format!(".Trash-{uid}/files"));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.delete_now(&[dir.join("h.txt"), drive.join("d.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(drive_files.join("d.txt").exists(), "on the drive's trash");
    assert!(home.path().join("Trash/files/h.txt").exists());

    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);
    let d = ex
        .items
        .iter()
        .position(|it| it.name == "d.txt")
        .expect("d");
    assert_eq!(ex.sv.entries[d].folder, drive.display().to_string());
    ex.sv.selection.set_only(Some(d));
    ex.run(Command::Restore, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(drive.join("d.txt")).expect("d"),
        "d"
    );
    assert_eq!(ex.items.len(), 1);

    ex.delete_now(&[drive.join("d.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.items.len(), 2, "the bin re-listed with it");
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.items.is_empty());
    assert_eq!(std::fs::read_dir(&drive_files).expect("files").count(), 0);
    assert!(ex.dialog.is_none(), "no error box");
}

#[test]
fn an_open_recycle_bin_lists_a_drives_trash_once_udisks_reports_it_mounted() {
    use reprisefs::udisks::{Snapshot, Volume};
    use std::os::unix::fs::MetadataExt;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let drive = home.path().join("stick");
    let uid = std::fs::metadata(home.path()).expect("stat").uid();
    let files = drive.join(format!(".Trash-{uid}/files"));
    std::fs::create_dir_all(&files).expect("mkdir");
    std::fs::write(files.join("d.txt"), "d").expect("write");
    let table = home.path().join("mounts");
    let home_row = format!("/dev/a {} ext4 rw 0 0\n", home.path().display());
    std::fs::write(&table, &home_row).expect("write");
    reprisefs::trash::set_mount_table(Some(table.clone()));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::RecycleBin, &mut ctx);
    assert!(ex.items.is_empty());
    let listed = |ex: &Explorer| ex.items.iter().any(|it| it.name == "d.txt");
    let stick = |mounts: Vec<PathBuf>, label: &str| Snapshot {
        volumes: vec![Volume {
            path: "/block/sdx1".into(),
            device: "/dev/sdx1".into(),
            label: Some(label.into()),
            filesystem: true,
            mounts,
            ..Default::default()
        }],
        ..Default::default()
    };

    ex.volumes_changed(stick(Vec::new(), "STICK"), &mut ctx);
    std::fs::write(
        &table,
        format!("{home_row}/dev/sdx1 {} vfat rw 0 0\n", drive.display()),
    )
    .expect("write");
    ex.volumes_changed(stick(vec![drive.clone()], "STICK"), &mut ctx);
    assert!(
        listed(&ex),
        "the stick's trashed file is listed once it mounts"
    );

    std::fs::write(&table, &home_row).expect("write");
    let reply = ex.volumes_changed(stick(vec![drive.clone()], "RENAMED"), &mut ctx);
    assert_eq!(
        reply,
        Reply::none(),
        "a change that moves no mount re-lists nothing"
    );
    assert!(listed(&ex));

    ex.volumes_changed(stick(Vec::new(), "RENAMED"), &mut ctx);
    assert!(!listed(&ex), "its file goes once the stick unmounts");
}

/// The Recycle Bin lists where each item came from and when it went, in its columns, status bar and
/// sheet, whose Restore puts the item back. Delete there asks, then deletes the item and its record for
/// good, and Undo no longer offers it; Copy copies nothing.
#[test]
fn the_bin_shows_origins_and_deletes_for_good() {
    use reprisekit::controls::{form, propsheet, shellview};
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    for n in ["a.txt", "b.txt"] {
        std::fs::write(dir.join(n), n).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.delete_now(&[dir.join("a.txt"), dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    assert_eq!(ex.items.len(), 2);
    assert_eq!(ex.sv.fields, shellview::bin_fields());
    let shown = dir.display().to_string();
    assert!(ex.sv.entries.iter().all(|e| e.folder == shown));
    assert!(ex.sv.entries.iter().all(|e| !e.deleted.is_empty()));
    let at = |ex: &Explorer, n: &str| ex.items.iter().position(|it| it.name == n).expect(n);

    let a = at(&ex, "a.txt");
    ex.sv.selection.set_only(Some(a));
    ex.refresh_chrome(&mut ctx);
    let status = &ex.status.panels[0].text;
    assert!(
        status.starts_with(&format!("Original Location: {shown} Type: ")),
        "{status}"
    );
    assert!(ex.run(Command::Copy, &mut ctx).copy_files.is_none());

    ex.run(Command::Properties, &mut ctx);
    let p = ex.props.as_ref().expect("the item's sheet");
    assert_eq!(p.restore, ex.bin_item(a));
    assert!(matches!(
        p.sheet.open().item("v_origin").map(|i| &i.control),
        Some(form::Control::Lines(lines)) if lines == &["folder"]
    ));
    let pressed = propsheet::Event::Page(0, form::Event::Pressed(crate::props_dialog::RESTORE));
    ex.props_outcome(pressed, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.props.is_none(), "Restore closed the sheet");
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).expect("a"),
        "a.txt"
    );
    assert_eq!(ex.items.len(), 1);

    let b = at(&ex, "b.txt");
    let files = ex.items[b].path.clone();
    let info = ex.items[b]
        .trashed
        .as_ref()
        .expect("b's record")
        .info
        .clone();
    ex.sv.selection.set_only(Some(b));
    ex.run(Command::Delete { erase: false }, &mut ctx);
    let Some(Dialog::Confirm(c)) = ex.dialog.take() else {
        panic!("Delete asks first");
    };
    assert!(c.erase, "the bin's Delete is for good");
    assert_eq!(c.title, "Confirm File Delete");
    assert!(c.text.contains("delete 'b.txt'?"), "{}", c.text);
    let items = c
        .bin
        .expect("raised in the bin, it keeps the items as listed");
    assert_eq!(items, vec![ex.bin_item(b).expect("b")]);
    ex.erase_from_bin(items, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(std::fs::symlink_metadata(&files).is_err() && !info.exists());
    assert!(!dir.join("b.txt").exists());
    assert!(ex.items.is_empty());
    assert_eq!(ex.undo, None, "nothing is left to put back");
    assert!(ex.dialog.is_none(), "no error box");
}

/// An item replaced in the bin under the same name since it was listed is left alone: Yes on the prompt
/// over the old one refuses, Restore on its sheet refuses, and the sheet closes on the next re-listing.
#[test]
fn the_bin_acts_only_on_the_item_it_listed() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let (one, two) = (home.path().join("one"), home.path().join("two"));
    for d in [&one, &two] {
        std::fs::create_dir(d).expect("mkdir");
        std::fs::write(d.join("a.txt"), d.display().to_string()).expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(one.clone()), &mut ctx);
    ex.delete_now(&[one.join("a.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    let listed = ex.items[0].trashed.as_deref().cloned().expect("a record");
    ex.run(Command::Properties, &mut ctx);
    ex.run(Command::Delete { erase: false }, &mut ctx);

    reprisefs::trash::restore_free(&listed).expect("put back");
    reprisefs::trash::trash(&two.join("a.txt"), None).expect("trash two");
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(listed.files.exists(), "the new item stays");
    assert!(
        ex.answer_failed(reprisefs::progress::Decision::Abort, &mut ctx)
            .is_some(),
        "the failure box says why"
    );

    assert!(ex.props.is_none(), "the sheet went with its item");
    let stale = crate::fileops::BinItem {
        path: listed.files.clone(),
        record: Some(listed.clone()),
    };
    ex.restore_items(vec![stale], &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(listed.files.exists() && !two.join("a.txt").exists());
    assert!(
        ex.answer_failed(reprisefs::progress::Decision::Abort, &mut ctx)
            .is_some()
    );
}

/// Resizing a column or sorting in the Recycle Bin leaves the folder columns and sort settings alone; a folder still saves both.
#[test]
fn the_bins_columns_and_sort_are_not_saved_as_the_folders() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    let (columns, sort) = (ex.settings.columns.clone(), ex.settings.sort);
    let release = |ex: &mut Explorer, ctx: &mut Ctx| {
        ex.pointer(
            repriseapp::PointerInput::Release {
                x: 0,
                y: 0,
                button: crate::BTN_LEFT,
                mods: repriseapp::Mods::default(),
            },
            ctx,
        );
    };

    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    ex.resizing = Some((1, 100, 180));
    ex.sv.columns[1].width = 240;
    release(&mut ex, &mut ctx);
    ex.run(Command::Sort(4), &mut ctx);
    assert_eq!(ex.sv.sort, Some((4, true)), "the bin sorts by Type");
    assert_eq!(ex.settings.columns, columns, "folder widths kept");
    assert_eq!(ex.settings.sort, sort, "folder sort kept");

    ex.run(Command::GoTo(Place::Folder(dir)), &mut ctx);
    assert_eq!(ex.sv.columns.len(), 4);
    ex.resizing = Some((1, 100, 94));
    ex.sv.columns[1].width = 120;
    release(&mut ex, &mut ctx);
    ex.run(Command::Sort(2), &mut ctx);
    assert_eq!(ex.settings.columns[1], 120, "a folder saves its widths");
    assert_eq!(ex.settings.sort, (2, true), "a folder saves its sort");
}

/// Emptying the Recycle Bin drops the undos that would have restored a trashed file and keeps every
/// other one; a failed empty leaves the stack alone.
#[test]
fn emptying_the_bin_drops_only_the_trash_undos() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "1").expect("write");
    std::fs::rename(dir.join("a.txt"), dir.join("moved.txt")).expect("rename");
    std::fs::write(dir.join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.undos.push(Undoable::Move {
        from: dir.join("a.txt"),
        to: dir.join("moved.txt"),
        listed: false,
    });
    ex.delete_now(&[dir.join("b.txt")], false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("Delete"));

    let files = home.path().join("Trash/files");
    let _writable_again = Mode0700(files.clone());
    let mode = |m: u32| {
        std::fs::set_permissions(&files, std::fs::Permissions::from_mode(m)).expect("chmod")
    };
    mode(0o500);
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        matches!(ex.dialog, Some(crate::dialogs::Dialog::Message { .. })),
        "the failure is reported"
    );
    assert_eq!(std::fs::read_dir(&files).expect("the trash").count(), 1);
    assert_eq!(ex.undo, Some("Delete"), "nothing was forgotten");

    mode(0o700);
    ex.dialog = None;
    ex.empty_bin_now(&mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(std::fs::read_dir(&files).expect("the trash").count(), 0);
    assert_eq!(ex.undo, Some("Move"));
}

#[test]
fn the_empty_bin_prompt_counts_the_bin() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "1").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.confirm_empty_bin(&mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::Confirm(c)) => assert_eq!(
            c.text.replace('\n', " "),
            "Are you sure you want to delete these 3 items?"
        ),
        other => panic!("no confirm box: {:?}", other.is_some()),
    }
}
