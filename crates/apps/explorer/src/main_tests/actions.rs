use super::*;

/// A search through `find`, which every system has, in place of plocate or fd.
fn find_search(
    q: &reprisefs::search::Query,
    cancel: &std::sync::atomic::AtomicBool,
) -> Option<Vec<PathBuf>> {
    let mut find = std::process::Command::new("find");
    find.arg(&q.root);
    Some(reprisefs::search::collect(find, q, cancel))
}

#[test]
fn a_search_with_no_tool_says_what_it_needs_in_the_status_bar() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("report.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.search_run = |_, _| None;
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert!(ex.items.is_empty());
    assert_eq!(ex.status.panels[0].text, "File search needs plocate or fd.");
    press(&mut ex, keysym::ESCAPE, &mut ctx);
    assert_eq!(ex.status.panels[0].text, "1 object");
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
    assert_eq!(ex.search.caption(&[]), Some(title));
    let deep_entry = ex.sv.entries.iter().find(|e| e.name == "report.txt");
    assert_eq!(
        deep_entry.map(|e| e.folder.clone()),
        Some(deep.display().to_string()),
        "In Folder names where it was found"
    );
    assert_eq!(ex.sv.fields, shellview::search_fields());

    press(&mut ex, keysym::ESCAPE, &mut ctx);
    assert_eq!(ex.search.caption(&[]), None);
    assert_eq!(names(&ex), ["Report-old.md", "a", "other.txt"]);
    assert_eq!(ex.sv.fields, shellview::details_fields());
    assert_eq!(ex.search_field.text(), "", "leaving the search clears it");

    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(names(&ex).len(), 2);
    ex.run(Command::GoTo(Place::Folder(dir.path().join("a"))), &mut ctx);
    assert_eq!(ex.search.caption(&[]), None, "a navigation ends the search");
    assert_eq!(names(&ex), ["b"]);
}

fn back_on(ex: &Explorer) -> bool {
    match &ex.toolbar.items[0] {
        reprisekit::controls::toolbar::Item::DropButton { enabled, .. }
        | reprisekit::controls::toolbar::Item::Button { enabled, .. } => *enabled,
        reprisekit::controls::toolbar::Item::Separator => panic!("Back is the first item"),
    }
}

#[test]
fn escape_in_the_search_field_clears_the_query_then_leaves_the_search() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("report.txt"), "x").expect("write");
    std::fs::write(dir.path().join("other.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.search_run = find_search;
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(ex.items.len(), 1);
    ex.run(Command::SearchFocus, &mut ctx);

    press(&mut ex, keysym::ESCAPE, &mut ctx);
    assert_eq!(ex.search_field.text(), "");
    assert_eq!(
        ex.focus,
        Focus::Search,
        "the first Escape stays in the field"
    );
    assert!(ex.search.caption(&[]).is_some(), "the results stay");

    press(&mut ex, keysym::ESCAPE, &mut ctx);
    assert_eq!(ex.focus, Focus::View);
    assert_eq!(ex.search.caption(&[]), None);
    assert_eq!(ex.items.len(), 2, "the folder's listing is back");
}

#[test]
fn back_over_search_results_returns_to_the_folder_searched() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(sub.join("report.txt"), "x").expect("write");
    std::fs::write(sub.join("other.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.search_run = find_search;
    assert!(!back_on(&ex));
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(ex.items.len(), 1);
    assert!(back_on(&ex), "Back leaves the results");

    ex.run(Command::Back, &mut ctx);
    assert_eq!(ex.search.caption(&[]), None);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    assert_eq!(ex.items.len(), 1, "the folder's own listing: sub");
    assert_eq!(ex.search_field.text(), "");
    assert!(!back_on(&ex), "no history before the folder");

    ex.run(Command::GoTo(Place::Folder(sub.clone())), &mut ctx);
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    ex.run(Command::Back, &mut ctx);
    assert_eq!(
        ex.place,
        Place::Folder(sub),
        "the folder searched, not the one before"
    );
    assert_eq!(ex.items.len(), 2);
    assert!(back_on(&ex));
}

#[test]
fn backs_drop_over_search_results_with_no_history_offers_the_folder_searched() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("report.txt"), "x").expect("write");
    std::fs::write(dir.path().join("other.txt"), "x").expect("write");
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ctx = test_ctx();
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.search_run = find_search;
    assert_eq!(ex.back_rows(), [], "no results and no history");
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert_eq!(ex.items.len(), 1);

    let rows = ex.back_rows();
    assert_eq!(rows, [(ex.title_of(&here), Command::Back)]);
    ex.run(rows[0].1.clone(), &mut ctx);
    assert_eq!(ex.search.caption(&[]), None, "the row leaves the results");
    assert_eq!(ex.place, here);
    assert_eq!(ex.items.len(), 2, "the folder's own listing");
    assert_eq!(ex.back_rows(), []);
}

#[test]
fn backs_drop_over_search_results_puts_the_folder_searched_above_the_history() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (dir.path().join("a"), dir.path().join("a/b"));
    std::fs::create_dir_all(&b).expect("mkdir");
    std::fs::write(b.join("report.txt"), "x").expect("write");
    let places = [dir.path(), &a, &b].map(|p| Place::Folder(p.to_path_buf()));
    let mut ctx = test_ctx();
    let mut ex = test_explorer(places[0].clone(), &mut ctx);
    ex.search_run = find_search;
    ex.run(Command::GoTo(places[1].clone()), &mut ctx);
    ex.run(Command::GoTo(places[2].clone()), &mut ctx);
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert("report".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    assert!(ex.search.caption(&[]).is_some());

    let rows = ex.back_rows();
    assert_eq!(
        rows,
        [
            (ex.title_of(&places[2]), Command::Back),
            (ex.title_of(&places[1]), Command::Go(1)),
            (ex.title_of(&places[0]), Command::Go(2)),
        ]
    );
    ex.run(rows[1].1.clone(), &mut ctx);
    assert_eq!(ex.search.caption(&[]), None);
    assert_eq!(ex.place, places[1], "the row's folder, one step back");
}

fn search_for(ex: &mut Explorer, text: &str, ctx: &mut Ctx) {
    ex.run(Command::SearchFocus, ctx);
    ex.search_field.clear();
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert(text.into()));
    press(ex, keysym::RETURN, ctx);
    assert!(ex.search.caption(&[]).is_some(), "results for {text} shown");
}

#[test]
fn every_navigation_from_search_results_empties_the_search_field() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (dir.path().join("a"), dir.path().join("a/b"));
    std::fs::create_dir_all(b.join("reports")).expect("mkdir");
    std::fs::write(b.join("report.txt"), "x").expect("write");
    let places = [dir.path(), &a, &b].map(|p| Place::Folder(p.to_path_buf()));
    let mut ctx = test_ctx();
    let mut ex = test_explorer(places[0].clone(), &mut ctx);
    ex.search_run = find_search;
    ex.run(Command::GoTo(places[1].clone()), &mut ctx);
    ex.run(Command::GoTo(places[2].clone()), &mut ctx);
    let ended = |ex: &Explorer, place: &Place, how: &str| {
        assert_eq!(ex.search.caption(&[]), None, "{how} ends the search");
        assert_eq!(&ex.place, place, "{how} lands in its folder");
        assert_eq!(ex.search_field.text(), "", "{how} empties the field");
    };

    search_for(&mut ex, "report", &mut ctx);
    let go = ex.back_rows()[1].1.clone();
    assert_eq!(go, Command::Go(1));
    ex.run(go, &mut ctx);
    ended(&ex, &places[1], "Back's drop");

    search_for(&mut ex, "report", &mut ctx);
    ex.run(Command::Forward, &mut ctx);
    ended(&ex, &places[2], "Forward");

    search_for(&mut ex, "report", &mut ctx);
    ex.run(Command::Up, &mut ctx);
    ended(&ex, &places[1], "Up");

    ex.run(Command::GoTo(places[2].clone()), &mut ctx);
    search_for(&mut ex, "report", &mut ctx);
    let folder = ex.items.iter().position(|it| it.name == "reports");
    ex.sv.selection.set_only(folder);
    ex.open_selected(&mut ctx);
    ended(
        &ex,
        &Place::Folder(b.join("reports")),
        "opening a folder hit",
    );
}

#[test]
fn a_refresh_over_search_results_searches_again_and_keeps_the_field() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("report.txt"), "x").expect("write");
    std::fs::write(dir.path().join("other.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.search_run = find_search;
    search_for(&mut ex, "report", &mut ctx);
    std::fs::write(dir.path().join("report-2.txt"), "x").expect("write");

    ex.run(Command::Refresh, &mut ctx);
    assert!(ex.search.caption(&[]).is_some(), "the results stay");
    assert_eq!(ex.items.len(), 2, "the new match is found");
    assert_eq!(ex.search_field.text(), "report");
}

/// Search results list several folders: renaming a hit to a name another folder's hit wears is
/// no clash, inline or in the Rename dialog.
#[test]
fn a_hit_renamed_to_another_folders_hits_name_is_not_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for d in ["a", "b"] {
        std::fs::create_dir(dir.path().join(d)).expect("mkdir");
    }
    std::fs::write(dir.path().join("a/x.txt"), "x").expect("write");
    std::fs::write(dir.path().join("b/y.txt"), "y").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.search_run = find_search;
    ex.run(Command::SearchFocus, &mut ctx);
    ex.search_field
        .key(reprisekit::textedit::EditKey::Insert(".txt".into()));
    press(&mut ex, keysym::RETURN, &mut ctx);
    let x = ex
        .items
        .iter()
        .position(|it| it.name == "x.txt")
        .expect("x.txt");

    ex.sv.selection.set_only(Some(x));
    ex.run(Command::SelectAll, &mut ctx);
    ex.run(Command::Rename, &mut ctx);
    use reprisekit::controls::batchrename::{FIND, OK, WITH};
    use reprisekit::controls::form::Control;
    for (id, text) in [(FIND, "x"), (WITH, "y")] {
        let Some(crate::dialogs::Dialog::BatchRename(b)) = ex.dialogs.up_mut() else {
            panic!("the Rename dialog is up");
        };
        if let Some(Control::Edit { edit, .. }) = b.form.item_mut(id).map(|i| &mut i.control) {
            *edit = reprisekit::textedit::TextEdit::new(text, false);
        }
        ex.dialog_outcome(reprisekit::controls::form::Event::Changed(id), &mut ctx);
    }
    let Some(crate::dialogs::Dialog::BatchRename(b)) = ex.dialog() else {
        panic!("the Rename dialog is up");
    };
    let ok = b.form.item(OK);
    assert!(
        matches!(
            ok.map(|i| &i.control),
            Some(reprisekit::controls::form::Control::Button { enabled: true, .. })
        ),
        "y.txt is free in a"
    );
    ex.take_up();

    ex.sv.selection.set_only(Some(x));
    ex.run(Command::Rename, &mut ctx);
    if let Some((_, ed)) = ex.sv.edit.as_mut() {
        *ed = reprisekit::textedit::TextEdit::new("y.txt", false);
    }
    ex.commit_rename(&mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(ex.dialog().is_none(), "no clash box");
    assert!(
        dir.path().join("a/y.txt").exists(),
        "the rename was written"
    );
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
    match ex.dialog() {
        Some(crate::dialogs::Dialog::Message { title, text, .. }) => {
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

    let Some(Dialog::About(form, _)) = ex.dialog() else {
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
    assert!(ex.dialog().is_none());
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

/// The token the test terminal ([`test_env`]) was handed in `folder`, once it has started there.
fn terminal_token_in(folder: &Path) -> String {
    let marker = folder.join("opened-here");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < end,
            "the terminal opened in {folder:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::read_to_string(&marker).expect("read")
}

/// Open in Terminal asks for a token and starts the config's terminal on its own in the folder
/// picked or shown once it is in, handed that token; one that came tokenless hands none.
#[test]
fn open_in_terminal_starts_the_terminal_in_the_folder_with_its_token() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    assert_eq!(ex.run(Command::OpenTerminal, &mut ctx).handoffs, 1);
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        !dir.path().join("sub/opened-here").exists(),
        "nothing starts before the token"
    );
    hand(&mut ex, Some("first"), &mut ctx);
    assert_eq!(terminal_token_in(&dir.path().join("sub")), "first");
    assert_eq!(ex.run(Command::OpenTerminalHere, &mut ctx).handoffs, 1);
    hand(&mut ex, None, &mut ctx);
    assert_eq!(terminal_token_in(dir.path()), "none");
    assert_eq!(
        hand(&mut ex, Some("stray"), &mut ctx),
        Reply::none(),
        "a token with no start waiting starts nothing"
    );
}

#[test]
fn two_launches_asked_before_their_tokens_start_in_order_with_their_own() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::OpenTerminal, &mut ctx);
    ex.run(Command::OpenTerminalHere, &mut ctx);
    hand(&mut ex, Some("first"), &mut ctx);
    hand(&mut ex, Some("second"), &mut ctx);
    assert_eq!(terminal_token_in(&dir.path().join("sub")), "first");
    assert_eq!(terminal_token_in(dir.path()), "second");
}

/// A window closed while Open in Terminal waits for its token, or while the terminal is still
/// starting, quits only once the terminal has started.
#[test]
fn closing_waits_for_a_pending_terminal_to_start() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(Command::OpenTerminalHere, &mut ctx);
    assert!(!ex.close(&mut ctx).quit, "the launch waits for its token");

    let gate = Arc::new(std::sync::RwLock::new(()));
    let held = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    hand(&mut ex, Some("t"), &mut ctx);
    assert!(ex.busy(), "the terminal is still starting");
    assert!(!ex.wake(&mut ctx).quit);
    drop(held);
    ex.held = None;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ex.wake(&mut ctx).quit {
        assert!(std::time::Instant::now() < end, "the closed window quits");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(terminal_token_in(dir.path()), "t");
}

#[test]
fn a_file_dialog_shows_the_pins_explorer_reads() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let file = dir.path().join("bookmarks");
    let work = dir.path().join("Work");
    std::fs::create_dir(&work).expect("mkdir");
    reprisefs::pins::edit_at(&file, |b| b.pin(&work)).expect("pin");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Computer, &mut ctx);
    let pins = reprisefs::pins::Pins::start_at(file, || {});
    pins.wait_first(std::time::Duration::from_secs(5));
    ex.pins = Some(pins);
    let mut host = ex.dialog_host();
    let fields = repriseapp::openwith::browse_fields();
    let dlg = host.open(dir.path(), fields, reprisekit::theme::TextUnits::ONE);
    assert_eq!(dlg.places().last().map(|p| p.label.as_str()), Some("Work"));
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
    ex.folders.show_hidden = true;
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
    let written = |ex: &Explorer| {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while ex.favorites_busy() {
            assert!(
                std::time::Instant::now() < end,
                "the pins thread writes the pin"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
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
        written(&ex);
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
        written(&ex);
        assert_eq!(marks(), Vec::<PathBuf>::new(), "{place:?} unpinned");
        while pin_row(&ex) != Some(true) {
            assert!(std::time::Instant::now() < end, "{place:?}: unpinned");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// Waits until only `held` handles to `meter` are left: the job's threads have returned.
fn wait_threads_gone(meter: &Arc<reprisefs::progress::Meter>, held: usize) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while Arc::strong_count(meter) > held {
        assert!(std::time::Instant::now() < end, "the job's threads return");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Cancel pressed after an archive failed and before the failure was read: the failure and the
/// job's end arrive together, and the job ends instead of asking Skip, Retry or Cancel.
#[test]
fn an_archive_failure_read_with_the_end_of_its_cancelled_job_ends_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::write(d.join("broken.zip"), "not a zip").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(d.to_path_buf()), &mut ctx);
    ex.select_paths(&[d.join("broken.zip")]);
    ex.run(Command::Extract { here: false }, &mut ctx);
    let meter = ex.extracting.as_ref().expect("the job").job.meter().clone();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !meter.asking() {
        assert!(std::time::Instant::now() < end, "the archive fails");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    ex.cancel_extracts();
    wait_threads_gone(&meter, 3);

    let _ = ex.poll_extract(&mut ctx);
    assert!(ex.extracting.is_none(), "the job ended");
    assert!(!ex.holds_a_job(), "nothing asks about a job that ended");
    match ex.dialog() {
        Some(Dialog::Message { title, .. }) => assert_eq!(title, "Error Extracting File"),
        _ => panic!("the failure is shown"),
    }
}
