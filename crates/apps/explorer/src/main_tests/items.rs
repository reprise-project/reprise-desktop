use super::*;

#[test]
fn ascending_puts_folders_first_and_descending_reverses_it_all() {
    let items = [
        item("zeta", true, 0, "File Folder", Some(10)),
        item("b.txt", false, 300, "Text Document", Some(30)),
        item("A.txt", false, 100, "Text Document", Some(20)),
        item("c.bin", false, 200, "BIN File", Some(40)),
    ];
    let order = |col: usize, asc: bool| {
        crate::place::sort_order(&items, shellview::details_fields()[col], asc)
            .into_iter()
            .map(|i| items[i].name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(0, true), ["zeta", "A.txt", "b.txt", "c.bin"]);
    assert_eq!(order(0, false), ["c.bin", "b.txt", "A.txt", "zeta"]);
    assert_eq!(order(1, true), ["zeta", "A.txt", "c.bin", "b.txt"]);
    assert_eq!(order(1, false), ["b.txt", "c.bin", "A.txt", "zeta"]);
    assert_eq!(order(2, true), ["zeta", "c.bin", "A.txt", "b.txt"]);
    assert_eq!(order(3, true), ["zeta", "A.txt", "b.txt", "c.bin"]);
    assert_eq!(order(3, false), ["c.bin", "b.txt", "A.txt", "zeta"]);
}

#[test]
fn a_symlink_to_a_folder_sorts_among_the_files() {
    let link = Item {
        is_link: true,
        ..item("b link", true, 7, "Shortcut", Some(5))
    };
    let items = [
        item("c", false, 1, "Text Document", Some(30)),
        link,
        item("a", false, 1, "Text Document", Some(20)),
        item("mfolder", true, 0, "File Folder", Some(10)),
    ];
    let order = |col: usize| {
        crate::place::sort_order(&items, shellview::details_fields()[col], true)
            .into_iter()
            .map(|i| items[i].name.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(order(0), ["mfolder", "a", "b link", "c"], "Name");
    assert_eq!(order(1), ["mfolder", "a", "c", "b link"], "Size");
    assert_eq!(order(2), ["mfolder", "b link", "a", "c"], "Type");
    assert_eq!(order(3), ["mfolder", "b link", "a", "c"], "Date Modified");
}

#[test]
fn a_symlink_to_a_folder_lists_as_a_file_and_opens_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("mfolder")).expect("mkdir");
    std::fs::write(dir.path().join("a.txt"), "x").expect("write");
    std::os::unix::fs::symlink("mfolder", dir.path().join("b link")).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["mfolder", "a.txt", "b link"]);
    use reprisekit::controls::listview::Rows;
    let fields = shellview::details_fields();
    let rows = shellview::DetailsRows(&ex.sv.entries, &fields);
    assert_eq!(rows.cell(0, 1), "", "a folder is not sized");
    assert_eq!(rows.cell(2, 1), "1 KB", "the link is");

    ex.sv.selection.set_only(Some(2));
    ex.open_selected(&mut ctx);
    let target = std::fs::canonicalize(dir.path().join("mfolder")).expect("canonical");
    assert_eq!(ex.place, Place::Folder(target));
}

#[test]
fn a_shortcut_whose_read_answers_late_puts_its_location_up_on_a_later_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    std::os::unix::fs::symlink("/etc", real.join("c etc")).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(real.clone()), &mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.sv.selection.set_only(Some(0));
    ex.refresh_chrome(&mut ctx);
    assert_eq!(ex.status.panels[0].text, "1 object(s) selected");

    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| {
        ex.status.panels[0].text == "Location: /"
    });
}

#[test]
fn one_selected_shortcut_shows_its_location_in_the_status_bar() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    std::fs::create_dir(real.join("mfolder")).expect("mkdir");
    std::fs::write(real.join("a.txt"), "x").expect("write");
    std::os::unix::fs::symlink("mfolder/a.txt", real.join("b link")).expect("symlink");
    std::os::unix::fs::symlink("/etc", real.join("c etc")).expect("symlink");
    reprisefs::apps::write_launcher(
        &real.join("d.desktop"),
        "[Desktop Entry]\nType=Application\nName=d shell\nExec=/bin/sh -l\n",
    )
    .expect("write");
    reprisefs::apps::write_launcher(
        &real.join("e.desktop"),
        "[Desktop Entry]\nType=Link\nName=e web\nURL=https://example.org/\n",
    )
    .expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(real.clone()), &mut ctx);
    let mut status = |ex: &mut Explorer, sel: &[&str]| {
        ex.sv.selection.set.clear();
        for name in sel {
            let i = ex.items.iter().position(|it| it.name == *name).expect(name);
            ex.sv.selection.set.insert(i);
        }
        ex.refresh_chrome(&mut ctx);
        ex.status.panels[0].text.clone()
    };
    let mfolder = real.join("mfolder");
    assert_eq!(
        status(&mut ex, &["b link"]),
        format!("Location: {}", mfolder.display())
    );
    assert_eq!(status(&mut ex, &["c etc"]), "Location: /");
    assert_eq!(status(&mut ex, &["d.desktop"]), "Location: /bin");
    assert_eq!(status(&mut ex, &["e.desktop"]), "https://example.org/");
    assert_eq!(status(&mut ex, &["a.txt"]), "1 object(s) selected");
    assert_eq!(
        status(&mut ex, &["b link", "c etc"]),
        "2 object(s) selected"
    );
}

#[test]
fn a_sort_keeps_the_selection_when_its_index_moves() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let a = ex
        .items
        .iter()
        .position(|it| it.name == "a.txt")
        .expect("a.txt");
    ex.sv.selection.set_only(Some(a));

    ex.sv.sort = Some((0, false));
    ex.resort();

    let names: Vec<&str> = ex
        .sv
        .selection
        .set
        .iter()
        .map(|&i| ex.items[i].name.as_str())
        .collect();
    assert_eq!(names, ["a.txt"]);
    assert_eq!(
        ex.sv.selection.focus,
        Some(2),
        "a.txt sorts last descending"
    );
}

#[test]
fn the_details_section_follows_the_selection() {
    let file = item("notes.txt", false, 1260, "Text Document", None);
    let folder = item("Reports", true, 0, "File Folder", None);

    let none: Vec<&Item> = vec![];
    assert_eq!(
        window::details_text("Documents", true, &none, None, true),
        [
            ("Documents".to_string(), true),
            ("System Folder".to_string(), false)
        ]
    );
    assert_eq!(
        window::details_text("Reports", false, &none, None, true)[1],
        ("File Folder".to_string(), false)
    );

    let one = window::details_text("Documents", false, &[&file], None, true);
    let gap = (String::new(), false);
    assert_eq!(one[0], gap);
    assert_eq!(one[1], ("notes.txt".to_string(), true));
    assert_eq!(one[2], ("Text Document".to_string(), false));
    assert_eq!(one[3], gap);
    assert_eq!(one[4], ("Size: 1.23 KB".to_string(), false));
    let one = window::details_text("Documents", false, &[&folder], None, true);
    assert_eq!(one.len(), 3);
    let dated = item("a.txt", false, 0, "Text Document", Some(1_757_795_100));
    let one = window::details_text("Documents", false, &[&dated], None, true);
    assert!(one[4].0.starts_with("Date Modified: "), "{:?}", one[4]);

    let pic = item("a.png", false, 1260, "PNG Image", Some(1_757_795_100));
    let one = window::details_text("Documents", false, &[&pic], Some((800, 600)), true);
    assert_eq!(
        one[3..5],
        [gap.clone(), ("Dimensions: 800 x 600".to_string(), false)]
    );
    assert_eq!(one[6], ("Size: 1.23 KB".to_string(), false));
    assert!(one[8].0.starts_with("Date Modified: "), "{:?}", one[8]);

    let many = window::details_text("Documents", false, &[&file, &folder, &file], None, true);
    assert_eq!(many[0], ("3 items selected".to_string(), true));
    assert_eq!(many[2], ("Size: 2.46 KB".to_string(), false));
}

#[test]
fn a_shortcut_picked_while_anothers_read_is_out_is_read_once_that_one_lands() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let real = std::fs::canonicalize(dir.path()).expect("canonical");
    for d in ["m1", "m2"] {
        std::fs::create_dir(real.join(d)).expect("mkdir");
        std::fs::write(real.join(d).join("a.txt"), "x").expect("write");
    }
    std::os::unix::fs::symlink("m1/a.txt", real.join("c one")).expect("symlink");
    std::os::unix::fs::symlink("m2/a.txt", real.join("d two")).expect("symlink");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(real.clone()), &mut ctx);
    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    let pick = |ex: &mut Explorer, ctx: &mut Ctx, name: &str| {
        let i = ex.items.iter().position(|it| it.name == name).expect(name);
        ex.sv.selection.set_only(Some(i));
        ex.refresh_chrome(ctx);
    };
    pick(&mut ex, &mut ctx, "c one");
    pick(&mut ex, &mut ctx, "d two");
    let out = ex.link_status.as_ref().map(|(at, _)| at.clone());
    assert_eq!(
        out,
        Some(real.join("c one")),
        "d two waits for c one's read"
    );

    drop(closed);
    ex.held = None;
    let want = format!("Location: {}", real.join("m2").display());
    wake_until(&mut ex, &mut ctx, |ex| ex.status.panels[0].text == want);
}
