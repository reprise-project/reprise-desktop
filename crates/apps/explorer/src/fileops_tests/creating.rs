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
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("New"));
}

#[test]
fn new_from_a_template_copies_it_under_its_name_and_starts_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let templates = tempfile::tempdir().expect("a temp dir");
    let template = templates.path().join("Report.odt");
    std::fs::write(&template, "body").expect("write");
    std::fs::write(dir.path().join("Report.odt"), "mine").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::NewFrom(template), &mut ctx);

    let made = dir.path().join("Report (2).odt");
    assert_eq!(std::fs::read_to_string(&made).expect("read"), "body");
    let i = ex.index_of(&made).expect("the copy is listed");
    assert_eq!(ex.sv.edit.as_ref().map(|(j, _)| *j), Some(i));
    ex.finish_puts(&mut ctx);
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
        matches!(ex.dialog(), Some(Dialog::Message { .. })),
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
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("Rename"));

    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert!(dir.path().join("a.txt").is_file());
    assert!(!dir.path().join("b.txt").exists());
    let a = ex.index_of(&dir.path().join("a.txt"));
    assert_eq!(ex.sv.selection.focus, a, "a.txt has the focus");
    assert_eq!(
        ex.sv.selection.set,
        a.into_iter().collect(),
        "a.txt is the selection"
    );
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

/// A picture alone on the clipboard pastes as a new file in `[screenshot] format` (PNG by
/// default), selected with its rename begun and undone as a New.
#[test]
fn a_pasted_picture_saves_as_a_new_file_and_starts_a_rename() {
    use reprisefs::stillfile::{PngLevel, Still};
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    repriseapp::fake_clipboard_files(false);
    repriseapp::fake_clipboard_image(true);

    let reply = ex.begin_paste(false);
    assert!(
        reply.paste_image && !reply.paste_files,
        "the picture is read"
    );
    let png = reprisefs::stillfile::encode(&[255, 0, 0], 1, 1, &Still::Png(PngLevel::Fast))
        .expect("a PNG");
    ex.pasted_image(png, &mut ctx);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ex.reads.is_empty() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(5));
        ex.take_reads(&mut ctx);
    }

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("list")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    let [name] = &names[..] else {
        panic!("one file: {names:?}")
    };
    assert!(
        name.starts_with("Pasted image ") && name.ends_with(".png"),
        "{name}"
    );
    let i = ex.index_of(&dir.path().join(name)).expect("listed");
    assert_eq!(ex.sv.edit.as_ref().map(|(j, _)| *j), Some(i));
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("New"));
    repriseapp::fake_clipboard_image(false);
}
