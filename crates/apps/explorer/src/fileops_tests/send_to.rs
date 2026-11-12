use super::*;

#[test]
fn send_to_zip_packs_the_set_into_one_zip_named_after_the_folder() {
    let tmp = tempfile::tempdir().expect("a temp dir");
    let d = tmp.path().join("Photos");
    std::fs::create_dir(&d).expect("mkdir");
    std::fs::write(d.join("a.txt"), "1").expect("write");
    std::fs::write(d.join("b.txt"), "2").expect("write");
    std::fs::write(d.join("Photos.zip"), "").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(d.clone()), &mut ctx);
    let at = |name: &str| ex.index_of(&d.join(name)).unwrap();
    let (a, b) = (at("a.txt"), at("b.txt"));
    ex.sv.selection.set_only(Some(a));
    ex.sv.selection.set.insert(b);
    ex.send_to = vec![SendToRow {
        label: "Compressed (zipped) Folder".into(),
        icon: "application-zip",
        dest: SendTo::Zip,
        enabled: true,
    }];

    let reply = ex.send_to(0, &mut ctx);
    assert!(reply.spawn.is_empty(), "the zip is made on a job");
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    let zip = d.join("Photos (2).zip");
    let listed = std::process::Command::new("bsdtar")
        .arg("-tf")
        .arg(&zip)
        .output()
        .expect("bsdtar runs");
    let mut names: Vec<String> = String::from_utf8_lossy(&listed.stdout)
        .lines()
        .map(String::from)
        .collect();
    names.sort();
    assert_eq!(names, ["a.txt", "b.txt"]);
    assert_eq!(ex.selected_paths(), [zip], "the new zip is selected");
}

#[test]
fn send_to_a_folder_copies_on_a_job() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = tempfile::tempdir().expect("a temp dir");
    let dest = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.sv.selection.set.insert(1);
    ex.send_to = vec![SendToRow {
        label: "Docs".into(),
        icon: "folder-documents",
        dest: SendTo::Folder(dest.path().to_path_buf()),
        enabled: true,
    }];

    ex.send_to(0, &mut ctx);
    assert!(
        ex.put.is_some(),
        "the copy runs on a job, not in the handler"
    );
    ex.finish_puts(&mut ctx);

    assert!(dest.path().join("a.txt").exists());
    assert!(dest.path().join("b.txt").exists());
    assert!(dir.path().join("a.txt").exists(), "Send To never moves");
    assert_eq!(ex.undo, Some("Copy"));
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(!dest.path().join("a.txt").exists(), "one undo takes both");
    assert!(!dest.path().join("b.txt").exists(), "one undo takes both");
}

/// A drive listed under a mount root in `table`, for Send To.
fn send_drive(t: &Path) -> (reprisefs::sendto::Drives, PathBuf) {
    let (root, table) = (t.join("media"), t.join("mounts"));
    let stick = root.join("STICK");
    std::fs::create_dir_all(&stick).expect("mkdir");
    let line = format!("/dev/sdb1 {} vfat rw 0 0\n", stick.display());
    std::fs::write(&table, line).expect("write");
    let scanner = reprisefs::places::MountedDrives::at(table, vec![root]);
    (reprisefs::sendto::Drives::new(scanner), stick)
}

fn send_row(ex: &mut Explorer, ctx: &mut Ctx) -> (usize, bool) {
    let rows = ex.build_send_to(&mut Default::default(), ctx);
    let i = rows
        .iter()
        .position(|(l, ..)| l == "STICK")
        .expect("the drive's row");
    (i, rows[i].1)
}

#[test]
fn a_send_to_drive_whose_stat_is_out_greys_its_row_and_a_send_past_the_deadline_copies_nothing() {
    let t = tempfile::tempdir().expect("a temp dir");
    let src = t.path().join("src");
    std::fs::create_dir(&src).expect("mkdir");
    std::fs::write(src.join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(src.clone()), &mut ctx);
    let stick;
    (ex.send.drives, stick) = send_drive(t.path());
    ex.sv.selection.set_only(Some(0));
    assert!(send_row(&mut ex, &mut ctx).1, "STICK answered");

    let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert!(!send_row(&mut ex, &mut ctx).1, "STICK's new stat is out");
    drop(closed);
    ex.held = None;
    let (i, enabled) = send_row(&mut ex, &mut ctx);
    assert!(enabled, "STICK answered again");

    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.action_deadline = std::time::Duration::from_millis(20);
    ex.send_to(i, &mut ctx);
    assert!(ex.busy(), "the send waits on STICK's stat");
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    match &ex.dialog {
        Some(Dialog::Message { title, text }) => assert_eq!(
            (title.as_str(), text.as_str()),
            ("Send To", "The location is not responding.")
        ),
        _ => panic!("the give-up box is up"),
    }
    assert!(!ex.busy());
    drop(closed);
    ex.held = None;
    ex.dialog = None;
    // The menu's rows wait on the late stat, so it has landed once they are built.
    let (i, enabled) = send_row(&mut ex, &mut ctx);
    assert!(enabled, "the late stat landed");
    ex.finish_puts(&mut ctx);
    assert!(!stick.join("a.txt").exists(), "the late stat sends nothing");

    // A copy's flush to the drive can sit behind other writes for longer than 20 ms.
    ex.action_deadline = crate::acting::ACTION_DEADLINE;
    ex.send_to(i, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        stick.join("a.txt").exists(),
        "a send to STICK answering copies"
    );
}
