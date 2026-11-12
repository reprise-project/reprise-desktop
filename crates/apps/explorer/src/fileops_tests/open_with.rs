use super::*;

/// Open With on one entry of a multi-selection runs the program in its app scope and raises the
/// chooser on the entry the menu was built for.
#[test]
fn open_with_runs_the_program_as_its_entry_on_the_entry_the_menu_was_built_for() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = vec![app("Editor", "editor", &["text/plain"])].into();
    for it in &mut ex.items {
        it.mime = Some("text/plain".into());
    }
    let b = ex
        .items
        .iter()
        .position(|it| it.name == "b.txt")
        .expect("b.txt is listed");
    ex.sv.selection.set = (0..ex.items.len()).collect();
    assert_eq!(
        ex.build_open_with(b, &mut Default::default(), &mut ctx)
            .len(),
        1
    );

    let reply = ex.run(Command::OpenWith(0), &mut ctx);
    let argv = reply.spawn.first().expect("the program runs");
    assert_eq!(
        argv.last(),
        Some(&dir.path().join("b.txt").into_os_string())
    );
    assert_eq!(
        reply.spawn_app.as_deref(),
        Some("editor.desktop"),
        "in the program's app scope"
    );

    ex.run(Command::ChooseProgram, &mut ctx);
    match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => assert_eq!(c.path, dir.path().join("b.txt")),
        other => panic!("no chooser: {:?}", other.is_some()),
    }
    match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => {
            assert_eq!(
                c.row_icons.len(),
                c.ow.list.rows.len(),
                "one icon slot per row"
            );
            assert!(c.ow.always.enabled, "Open With leaves the box live");
        }
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    }
}

/// A chooser opened from the keyboard shows its cues from the start; one opened with the pointer waits for a key.
#[test]
fn a_chooser_opened_by_key_shows_its_cues() {
    use reprisekit::cues::Cues;
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let cues = |ex: &crate::Explorer| match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => c.input.cues(),
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    };
    ex.choose_program_for(&path, false, &mut ctx);
    assert_eq!(cues(&ex), Cues::NONE, "opened with the pointer");
    ex.keyed = true;
    ex.choose_program_for(&path, false, &mut ctx);
    assert_eq!(cues(&ex), Cues::ALL, "opened from the keyboard");
}

/// OK in the chooser acts on release: a press only arms it, the release over it runs the program.
#[test]
fn chooser_ok_runs_the_program_on_release() {
    use repriseapp::{Event, Mods, PointerInput};
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = vec![app("Editor", "editor", &["text/plain"])].into();
    ex.choose_program_for(&path, false, &mut ctx);
    let ok = match ex.dialog.as_ref() {
        Some(Dialog::ChooseProgram(c)) => {
            c.ow.layout(&ex.dialog_text, ex.dialog_px, (0, 0), &ctx.theme.controls())
                .ok
        }
        other => panic!("expected the chooser, got {:?}", other.is_some()),
    };
    let (x, y) = (ok.x + ok.width / 2, ok.y + ok.height / 2);
    let press = Event::Pointer(PointerInput::Press {
        x,
        y,
        button: crate::BTN_LEFT,
        time: 1000,
        mods: Mods::default(),
    });
    let reply = ex.dialog_event(press, &mut ctx);
    assert!(reply.spawn.is_empty(), "the press only arms OK");
    let release = Event::Pointer(PointerInput::Release {
        x,
        y,
        button: crate::BTN_LEFT,
        mods: Mods::default(),
    });
    let reply = ex.dialog_event(release, &mut ctx);
    let argv = reply.spawn.first().expect("the release runs the program");
    assert_eq!(argv.last(), Some(&path.clone().into_os_string()));
    assert_eq!(reply.spawn_app.as_deref(), Some("editor.desktop"));
    assert!(ex.dialog.is_none(), "the chooser closes");
}

#[test]
fn ok_on_a_browsed_program_runs_it_on_the_file() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&dir.path().join("b.txt"), false, &mut ctx);
    let c = match ex.dialog.take() {
        Some(Dialog::ChooseProgram(c)) => c,
        _ => panic!("expected the chooser"),
    };
    let reply = ex.choose_program_ok(*c, "/usr/bin/nano".into(), true, &mut ctx);
    let argv = reply.spawn.first().expect("one command");
    assert_eq!(argv[0], std::ffi::OsString::from("/usr/bin/nano"));
    assert_eq!(argv[1], dir.path().join("b.txt").into_os_string());
}

#[test]
fn browse_opens_the_program_picker_over_the_chooser() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&dir.path().join("b.txt"), false, &mut ctx);
    ex.chooser_event(key_event_alt('b'), &mut ctx);
    let kids = ex.children();
    assert!(
        kids.iter()
            .any(|k| k.id == crate::dialogs::PROGRAM_BROWSE && k.modal),
        "the picker is a modal child"
    );
    assert!(
        kids.iter().any(|k| k.id == crate::dialogs::DIALOG),
        "the chooser stays up under it"
    );
}

#[test]
fn the_program_picker_has_the_plain_caption_of_a_file_dialog() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.choose_program_for(&dir.path().join("b.txt"), false, &mut ctx);
    ex.chooser_event(key_event_alt('b'), &mut ctx);
    assert_eq!(
        ex.caption(crate::dialogs::DIALOG),
        repriseapp::Caption::Help
    );
    assert_eq!(
        ex.caption(crate::dialogs::PROGRAM_BROWSE),
        repriseapp::Caption::Window
    );
}
