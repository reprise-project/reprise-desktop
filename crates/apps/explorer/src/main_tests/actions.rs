use super::*;

/// A search through `find`, which every system has, in place of plocate or fd.
fn find_search(
    q: &reprisefs::search::Query,
    cancel: &std::sync::atomic::AtomicBool,
) -> Vec<PathBuf> {
    let mut find = std::process::Command::new("find");
    find.arg(&q.root);
    reprisefs::search::collect(find, q, cancel)
}

#[test]
fn a_search_lists_matches_below_the_folder_until_escape_or_a_navigation() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let deep = dir.path().join("a").join("b");
    std::fs::create_dir_all(&deep).expect("mkdir");
    std::fs::write(deep.join("report.txt"), "x").expect("write");
    std::fs::write(dir.path().join("Report-old.md"), "x").expect("write");
    std::fs::write(dir.path().join("other.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.search_run = find_search;
    let names = |ex: &Explorer| {
        let mut n: Vec<String> = ex.items.iter().map(|it| it.name.clone()).collect();
        n.sort();
        n
    };
    assert_eq!(names(&ex), ["Report-old.md", "a", "other.txt"]);

    ex.run(Command::SearchFocus, &mut ctx);
    assert_eq!(ex.focus, Focus::Search);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(names(&ex), ["Report-old.md", "report.txt"]);
    assert_eq!(ex.focus, Focus::View, "the results take the keyboard");
    assert_eq!(ex.place, place, "the window stays in the folder");
    let title = format!("Search results in {}", place.title());
    assert_eq!(ex.search.caption(), Some(title));
    let deep_entry = ex.sv.entries.iter().find(|e| e.name == "report.txt");
    assert_eq!(
        deep_entry.map(|e| e.folder.clone()),
        Some(deep.display().to_string()),
        "In Folder names where it was found"
    );
    assert_eq!(ex.sv.fields, shellview::search_fields());

    press(&mut ex, keysym::ESCAPE, &mut ctx);
    assert_eq!(ex.search.caption(), None);
    assert_eq!(names(&ex), ["Report-old.md", "a", "other.txt"]);
    assert_eq!(ex.sv.fields, shellview::details_fields());

    ex.run(Command::SearchFocus, &mut ctx);
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(names(&ex).len(), 2, "the field kept its text");
    ex.run(Command::GoTo(Place::Folder(dir.path().join("a"))), &mut ctx);
    assert_eq!(ex.search.caption(), None, "a navigation ends the search");
    assert_eq!(names(&ex), ["b"]);
}

#[test]
fn extract_to_lands_a_zip_in_a_folder_named_after_it_and_selects_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::create_dir_all(d.join("src/docs")).expect("mkdir");
    std::fs::write(d.join("src/docs/a.txt"), "a").expect("write");
    let made = std::process::Command::new("bsdtar")
        .current_dir(d.join("src"))
        .args(["-a", "-cf", "../pack.zip", "docs"])
        .status()
        .expect("bsdtar runs");
    assert!(made.success());
    std::fs::remove_dir_all(d.join("src")).expect("rm");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(d.to_path_buf()), &mut ctx);
    let zip = ex.index_of(&d.join("pack.zip")).expect("the zip is listed");
    ex.sv.selection.set_only(Some(zip));
    assert_eq!(ex.extract_label().as_deref(), Some("pack/"));

    ex.run(Command::Extract { here: false }, &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    assert_eq!(
        std::fs::read_to_string(d.join("pack/docs/a.txt")).expect("extracted"),
        "a"
    );
    assert_eq!(
        ex.selected_paths(),
        [d.join("pack")],
        "the new folder is selected"
    );

    ex.select_paths(&[d.join("pack.zip")]);
    ex.run(Command::Extract { here: true }, &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    assert!(
        d.join("docs/a.txt").exists(),
        "Extract Here lands beside the zip"
    );
}

#[test]
fn compress_to_names_the_zip_on_the_menu_and_selects_it() {
    let tmp = tempfile::tempdir().expect("a temp dir");
    let d = tmp.path().join("Work");
    std::fs::create_dir_all(d.join("v1.2/sub")).expect("mkdir");
    std::fs::write(d.join("v1.2/sub/a.txt"), "a").expect("write");
    std::fs::write(d.join("report.txt"), "r").expect("write");
    std::fs::write(d.join("report.zip"), "").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(d.clone()), &mut ctx);
    ex.select_paths(&[d.join("report.txt")]);
    assert_eq!(ex.compress_label().as_deref(), Some("report (2).zip"));
    ex.select_paths(&[d.join("v1.2")]);
    assert_eq!(ex.compress_label().as_deref(), Some("v1.2.zip"));
    ex.select_paths(&[d.join("v1.2"), d.join("report.txt")]);
    assert_eq!(ex.compress_label().as_deref(), Some("Work.zip"));

    ex.select_paths(&[d.join("v1.2")]);
    ex.run(Command::Compress, &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    let zip = d.join("v1.2.zip");
    let listed = std::process::Command::new("bsdtar")
        .arg("-tf")
        .arg(&zip)
        .output()
        .expect("bsdtar runs");
    assert_eq!(
        String::from_utf8_lossy(&listed.stdout),
        "v1.2/\nv1.2/sub/\nv1.2/sub/a.txt\n"
    );
    assert_eq!(ex.selected_paths(), [zip], "the new zip is selected");
    assert_eq!(
        Explorer::compress_label(&test_explorer(Place::Computer, &mut ctx)),
        None,
        "a read-only place compresses nothing"
    );
}

#[test]
fn a_zip_that_cannot_be_written_shows_an_error_box_and_leaves_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().expect("a temp dir");
    let d = tmp.path().join("ro");
    std::fs::create_dir(&d).expect("mkdir");
    std::fs::write(d.join("a.txt"), "a").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(d.clone()), &mut ctx);
    ex.select_paths(&[d.join("a.txt")]);
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    ex.run(Command::Compress, &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    match &ex.dialog {
        Some(crate::dialogs::Dialog::Message { title, text }) => {
            assert_eq!(title, "Error Compressing File");
            assert!(text.starts_with("Cannot compress a.zip: "), "{text}");
        }
        _ => panic!("an error box is up"),
    }
    let left: Vec<_> = std::fs::read_dir(&d)
        .expect("ls")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(left, ["a.txt"]);
}

#[test]
fn about_opens_the_shared_box_with_the_explorer_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(commands::Command::About, &mut ctx);

    let children = ex.children();
    let about = children.iter().find(|c| c.id == DIALOG).expect("the box");
    assert_eq!(about.title, "About Explorer");
    assert_eq!(about.size, reprisekit::controls::aboutbox::CLIENT);
    assert!(about.fixed && about.modal);

    let Some(Dialog::About(form)) = ex.dialog.as_ref() else {
        panic!("the About box is what is up");
    };
    let product = form.item("product").expect("the product row");
    match &product.control {
        reprisekit::controls::form::Control::Label { text, .. } => assert_eq!(text, "Explorer"),
        other => panic!("expected a label, got {other:?}"),
    }
    assert_eq!(
        form.default_button().map(|i| form.items[i].id),
        Some(reprisekit::controls::aboutbox::OK)
    );

    let esc = repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods::default(),
        edit: None,
    };
    ex.dialog_event(Event::Key(esc), &mut ctx);
    assert!(ex.dialog.is_none());
    assert!(ex.children().iter().all(|c| c.id != DIALOG));
}

#[test]
fn a_launcher_lists_runs_and_renames_by_its_name() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.foot.desktop");
    reprisefs::apps::write_launcher(
        &path,
        "[Desktop Entry]\nType=Application\nName=Terminal\nExec=foot\nIcon=utilities-terminal\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("notes.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["notes.txt", "Terminal"], "sorted by the shown name");
    assert!(ex.sv.entries[1].link, "the shortcut arrow");
    assert_eq!(
        reprisefs::icons::item_icon(&ex.items[1], &[], &|_| true),
        "utilities-terminal"
    );

    ex.sv.selection.set_only(Some(1));
    let reply = ex.open_selected(&mut ctx);
    assert_eq!(reply.spawn, [vec![std::ffi::OsString::from("foot")]]);

    ex.properties(&mut ctx);
    let p = ex.props.as_ref().expect("the sheet");
    assert_eq!(p.title(), "Terminal Properties");
    assert_eq!(
        reprisekit::controls::filesheet::typed_name(p.general()),
        "a.foot.desktop",
        "the field holds the file name"
    );
    ex.props = None;

    ex.run(Command::Rename, &mut ctx);
    let (_, ed) = ex.sv.edit.clone().expect("the box is up");
    assert_eq!(ed.text, "Terminal");
    assert!(ed.all_selected());
    ex.sv.edit = Some((1, reprisekit::textedit::TextEdit::new("Shell", false)));
    ex.commit_rename(&mut ctx);
    assert!(path.is_file(), "the file keeps its name");
    let text = std::fs::read_to_string(&path).expect("read");
    assert!(text.contains("Name=Shell\n"), "{text}");
    assert_eq!(ex.sv.entries[1].name, "Shell");
}

/// Open in Terminal starts the config's terminal on its own in the folder picked or shown.
#[test]
fn open_in_terminal_starts_the_terminal_in_the_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let opened = |folder: &Path| {
        let marker = folder.join("opened-here");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < end,
                "the terminal opened in {folder:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::OpenTerminal, &mut ctx);
    opened(&dir.path().join("sub"));
    ex.run(Command::OpenTerminalHere, &mut ctx);
    opened(dir.path());
}

/// Documents, Home and a plain folder's nodes pin from the tree menu; Computer and the Recycle
/// Bin are not folders and have no pin row.
#[test]
fn the_tree_menu_pins_and_unpins_a_nodes_folder() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("bookmarks");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.settings.show_hidden = true;
    let pins = reprisefs::pins::Pins::start_at(file.clone(), || {});
    pins.wait_first(std::time::Duration::from_secs(5));
    ex.pins = Some(pins);
    ex.build_tree(&mut ctx);
    ex.tree_follow(&here, &mut ctx);
    let pin_row = |ex: &Explorer| {
        let m = ex.node_menu();
        (0..m.items().len()).find_map(|i| match m.command(&[i]) {
            Command::NodePin(pin) => Some(pin),
            _ => None,
        })
    };
    let marks = || -> Vec<PathBuf> {
        let b = reprisefs::pins::load(&file);
        b.folders().map(|(p, _)| p.to_path_buf()).collect()
    };
    let select = |ex: &mut Explorer, place: &Place| {
        ex.tree.selected = Some(ex.tree_path_of(place).expect("the place's node"));
    };
    for place in [Place::Computer, Place::RecycleBin] {
        select(&mut ex, &place);
        assert_eq!(pin_row(&ex), None, "{place:?} has no pin row");
    }
    for place in [Place::Documents, Place::Home, here] {
        let Some(folder) = place.path() else {
            continue;
        };
        select(&mut ex, &place);
        assert_eq!(pin_row(&ex), Some(true), "{place:?}: Pin to Places");
        ex.run(Command::NodePin(true), &mut ctx);
        assert_eq!(marks(), std::slice::from_ref(&folder), "{place:?} pinned");
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while pin_row(&ex) != Some(false) {
            assert!(
                std::time::Instant::now() < end,
                "{place:?}: Unpin from Places"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        ex.run(Command::NodePin(false), &mut ctx);
        assert_eq!(marks(), Vec::<PathBuf>::new(), "{place:?} unpinned");
        while pin_row(&ex) != Some(true) {
            assert!(std::time::Instant::now() < end, "{place:?}: unpinned");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
