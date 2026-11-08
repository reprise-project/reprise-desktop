use super::*;

/// New Folder creates the default name, selects it and opens the rename box on it.
#[test]
fn new_folder_creates_the_folder_and_starts_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::NewFolder, &mut ctx);

    assert!(dir.path().join("New Folder").is_dir());
    let i = ex
        .items
        .iter()
        .position(|it| it.name == "New Folder")
        .expect("the new folder is listed");
    assert_eq!(ex.sv.edit.as_ref().map(|(j, _)| *j), Some(i));
    assert_eq!(ex.undo, Some("New"));
}

/// A New Text Document whose name appeared after the last listing leaves that file as it was and reports the clash.
#[test]
fn new_text_document_never_truncates_an_unlisted_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let taken = dir.path().join("New Text Document.txt");
    std::fs::write(&taken, "keep me").expect("write");

    ex.new_item(false, &mut ctx);

    assert_eq!(std::fs::read_to_string(&taken).expect("read"), "keep me");
    assert!(
        matches!(ex.dialog, Some(Dialog::Message { .. })),
        "the clash is reported"
    );
    assert_eq!(ex.undo, None);
}

#[test]
fn create_shortcut_writes_a_launcher_beside_the_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::CreateShortcut, &mut ctx);
    ex.finish_puts(&mut ctx);

    let text = std::fs::read_to_string(dir.path().join("Shortcut to a.txt.desktop"))
        .expect("the shortcut");
    assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
    assert!(text.contains("Name=Shortcut to a.txt\n"));
    let target = dir.path().join("a.txt");
    let exec = reprisefs::apps::exec_arg(&target.to_string_lossy());
    assert!(text.contains(&format!("Exec=xdg-open {exec}\n")), "{text}");
}

/// A rename joins the undo stack: Edit's first row offers it and Undo restores the old name.
#[test]
fn undo_reverses_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("b.txt", false)));
    ex.commit_rename(&mut ctx);
    assert_eq!(ex.undo, Some("Rename"));

    ex.run(Command::Undo, &mut ctx);

    assert!(dir.path().join("a.txt").is_file());
    assert!(!dir.path().join("b.txt").exists());
}

/// The folder watch fires as New > Folder writes its folder; that re-listing must not take the rename box away.
#[test]
fn a_refresh_keeps_an_open_rename_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(Command::NewFolder, &mut ctx);
    assert!(ex.sv.edit.is_some(), "the rename box opened");

    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    ex.refresh(&mut ctx);

    let (i, _) = ex.sv.edit.as_ref().expect("the rename box is still up");
    assert_eq!(ex.items[*i].name, "New Folder");
}
