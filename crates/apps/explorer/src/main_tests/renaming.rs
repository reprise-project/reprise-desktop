use super::*;

#[test]
fn a_rename_writes_the_file_and_keeps_it_selected() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("one.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Rename, &mut ctx);
    let (i, ed) = ex.sv.edit.clone().expect("the box is up");
    assert_eq!(i, 0);
    assert_eq!(ed.text, "one.txt");
    assert_eq!(ed.selection(), 0.."one".len(), "the stem, not the suffix");

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("two.txt", false)));
    ex.commit_rename(&mut ctx);

    assert!(ex.sv.edit.is_none());
    assert!(dir.path().join("two.txt").is_file());
    assert!(!dir.path().join("one.txt").exists());
    assert_eq!(ex.items[0].name, "two.txt");
    assert_eq!(ex.sv.selection.set.iter().copied().collect::<Vec<_>>(), [0]);
}

/// A rename whose relist lands late (a read of the folder was already out, on a mount slow to
/// answer) selects the item under its new name once the listing of it lands.
#[test]
fn a_rename_whose_relist_lands_late_keeps_the_renamed_item_selected() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["b.txt", "c.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let place = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(place.clone(), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    let before = ex.reader(place.clone(), &ctx)();
    let (answer, gate) = std::sync::mpsc::channel::<()>();
    ex.reading = Some(crate::Reading {
        key: (place, ex.settings.show_hidden, ctx.icons.scale()),
        job: repriseapp::listjob::Job::spawn(None, move || {
            gate.recv().ok();
            before
        }),
        again: false,
    });

    ex.held = Some(Arc::default());
    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("d.txt", false)));
    ex.commit_rename(&mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.acting.is_empty());
    assert!(dir.path().join("d.txt").is_file());
    answer.send(()).expect("send");
    wake_until(&mut ex, &mut ctx, |ex| ex.reading.is_none());
    let names: Vec<&str> = ex.items.iter().map(|it| it.name.as_str()).collect();
    assert_eq!(names, ["c.txt", "d.txt"]);
    assert_eq!(ex.selected_paths(), [dir.path().join("d.txt")]);
}

#[test]
fn a_colliding_rename_is_refused_with_a_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    for name in ["one.txt", "two.txt"] {
        std::fs::write(dir.path().join(name), "x").expect("write");
    }
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("two.txt", false)));
    ex.commit_rename(&mut ctx);

    assert!(dir.path().join("one.txt").is_file());
    match ex.dialog.as_ref().expect("the box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Rename");
            assert!(text.starts_with("Cannot rename one.txt: "), "{text}");
        }
        _ => panic!("the refused rename raises a message box"),
    }
}

/// The folder watch makes the listing live, so a name free when the
/// rename box opened can be taken by the time Enter lands. The write
/// refuses it instead of replacing whatever took it.
#[test]
fn a_rename_onto_a_name_taken_since_the_listing_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "mine").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("b.txt", false)));

    // Something else lands under the new name while the box is open;
    // the view has not re-listed, so the name still looks free.
    std::fs::write(dir.path().join("b.txt"), "someone else").expect("write");
    ex.commit_rename(&mut ctx);

    match ex.dialog.as_ref().expect("the box is up") {
        crate::dialogs::Dialog::Message { title, text } => {
            assert_eq!(title, "Rename");
            assert_eq!(
                text,
                "Cannot rename a.txt: A file with the name you specified already \
                 exists. Specify a different file name."
            );
        }
        _ => panic!("the refused rename raises a message box"),
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).expect("read"),
        "someone else"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).expect("read"),
        "mine"
    );
}

#[test]
fn renaming_a_folder_relabels_its_row_in_the_tree() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    // `tempdir` names start with a dot, so the tree hides the whole
    // chain until hidden entries are shown.
    ex.settings.show_hidden = true;
    ex.build_tree(&mut ctx);
    ex.show(here.clone(), &mut ctx);

    let path = ex.tree_path_of(&here).expect("this folder's node");
    let labels = |ex: &Explorer, path: &[usize]| -> Vec<String> {
        ex.tree
            .node(path)
            .map(|n| n.children.iter().map(|c| c.label.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(labels(&ex, &path), ["sub"]);

    let i = ex
        .items
        .iter()
        .position(|it| it.name == "sub")
        .expect("sub");
    ex.sv.selection.set_only(Some(i));
    ex.sv.edit = Some((i, reprisekit::textedit::TextEdit::new("renamed", false)));
    ex.commit_rename(&mut ctx);

    assert!(dir.path().join("renamed").is_dir());
    let path = ex.tree_path_of(&here).expect("this folder's node");
    assert_eq!(labels(&ex, &path), ["renamed"], "the row follows the name");
    let mut child = path.clone();
    child.push(0);
    assert_eq!(
        ex.place_of(&child),
        Some(Place::Folder(dir.path().join("renamed")))
    );
    assert_eq!(
        ex.tree_path_of(&Place::Folder(dir.path().join("sub"))),
        None
    );
}
