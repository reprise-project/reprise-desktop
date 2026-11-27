use super::*;

#[test]
fn a_paste_moves_a_cut_copies_a_copy_and_ignores_a_cut_into_its_own_folder() {
    let dir = Path::new("/home/u/Documents");
    let here = dir.join("x.txt");
    let away = Path::new("/home/u/Desktop/x.txt");
    assert_eq!(plan_paste(true, false, away, dir), Some(DropOp::Move));
    assert_eq!(plan_paste(false, false, away, dir), Some(DropOp::Copy));
    assert_eq!(plan_paste(false, false, &here, dir), Some(DropOp::Copy));
    assert_eq!(plan_paste(true, false, &here, dir), None);
    assert_eq!(plan_paste(true, true, &here, dir), Some(DropOp::Shortcut));
}

#[test]
fn paste_greys_with_no_files_on_the_clipboard_but_ctrl_v_still_asks() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    repriseapp::fake_clipboard_files(true);
    assert!(
        ex.menu_state().can_paste,
        "files on the clipboard: Paste is live"
    );
    repriseapp::fake_clipboard_files(false);
    assert!(
        !ex.menu_state().can_paste,
        "no files on the clipboard: Paste greys"
    );

    let ctrl_v = repriseapp::KeyInput {
        sym: 'v' as u32,
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: true,
            shift: false,
            alt: false,
        },
        edit: None,
    };
    assert!(
        ex.key_inner(ctrl_v, &mut ctx).paste_files,
        "Ctrl+V asks the clipboard anyway"
    );
}

#[test]
fn a_paste_into_the_same_folder_lands_as_a_copy() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    let reply = ex.run(Command::Paste, &mut ctx);
    assert!(reply.paste_files, "the clipboard was asked for");
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(
        ex.put.is_some(),
        "the copy runs on a job, not in the handler"
    );
    ex.finish_puts(&mut ctx);

    assert!(dir.path().join("Copy of a.txt").exists());
    assert!(ex.items.iter().any(|it| it.name == "Copy of a.txt"));
    assert_eq!(ex.undo, Some("Copy"));
}

#[test]
fn a_paste_selects_what_it_landed_in_place_of_the_source() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    ex.run(Command::Paste, &mut ctx);
    let clip = FileClip {
        cut: false,
        paths: vec![dir.path().join("a.txt")],
    };
    ex.pasted(clip, &mut ctx);
    ex.finish_puts(&mut ctx);

    assert_eq!(
        ex.selected_paths(),
        [dir.path().join("Copy of a.txt")],
        "the copy is selected, the source is not"
    );
}

/// Cut puts the selection on the clipboard as a cut and fades its entries; Copy does neither.
#[test]
fn cut_marks_its_entries_and_copy_does_not() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let reply = ex.run(Command::Cut, &mut ctx);
    assert_eq!(
        reply.copy_files,
        Some(FileClip {
            cut: true,
            paths: vec![dir.path().join("a.txt")],
        })
    );
    assert!(ex.sv.entries[0].cut);

    let reply = ex.run(Command::Copy, &mut ctx);
    assert_eq!(reply.copy_files.map(|c| c.cut), Some(false));
    assert!(!ex.sv.entries[0].cut);
}

/// The clipboard answers after Paste asked, so the window may have walked off in between. The Recycle
/// Bin has a real path, so "has a path" is not enough: the paste drops rather than writing files into the trash.
#[test]
fn a_paste_that_arrives_after_navigating_away_is_dropped() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.run(Command::GoTo(Place::RecycleBin), &mut ctx);
    let reply = ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert_eq!(reply, Reply::none());
    assert!(ex.put.is_none(), "nothing was started");
    assert!(!dir.path().join("Copy of a.txt").exists());
    assert!(!ex.run(Command::Paste, &mut ctx).paste_files);
}

/// A folder copied into itself would recurse for ever; reprisefs refuses and Explorer says so in an error box.
#[test]
fn pasting_a_folder_into_itself_is_refused_with_an_error_box() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let reports = dir.path().join("Reports");
    std::fs::create_dir(&reports).expect("mkdir");
    std::fs::write(reports.join("x.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(reports.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![reports.clone()],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    let (title, text) = ex
        .answer_failed(reprisefs::progress::Decision::Abort, &mut ctx)
        .expect("the refusal raises the failure box");
    assert_eq!(title, "Error Copying File or Folder");
    assert_eq!(
        text,
        "Cannot copy Reports: The destination folder is a subfolder of the source folder."
    );
    assert_eq!(
        std::fs::read_dir(&reports).expect("read").count(),
        1,
        "nothing was created"
    );
}

/// A cut of two files pasted where the second is gone: the first moves, the second fails with the
/// Skip / Retry / Cancel box, and the one undo holds only the move that happened.
#[test]
fn a_paste_whose_second_file_fails_keeps_the_first_and_names_the_second() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("gone.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    assert!(dir.join("a.txt").is_file() && !from.join("a.txt").exists());
    let (title, text) = ex
        .answer_failed(reprisefs::progress::Decision::Abort, &mut ctx)
        .expect("the failure raises the failure box");
    assert_eq!(title, "Error Moving File or Folder");
    assert!(text.starts_with("Cannot move gone.txt: "), "{text}");
    assert_eq!(ex.undo, Some("Move"));
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(from.join("a.txt").is_file(), "the one move is undone");
    assert_eq!(ex.undo, None, "and that was the whole batch");
}

/// A paste whose first file is gone holds on the failure box: Skip goes on and the second lands.
#[test]
fn skipping_a_failed_file_pastes_the_rest() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("gone.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    assert!(!dir.join("b.txt").exists(), "the job holds on the failure");
    let (_, text) = ex
        .answer_failed(reprisefs::progress::Decision::Skip, &mut ctx)
        .expect("the failure box is up");
    assert!(text.starts_with("Cannot copy gone.txt: "), "{text}");
    assert!(dir.join("b.txt").is_file(), "Skip goes on to the next file");
    assert!(ex.dialog().is_none(), "and no error box follows");
}

/// The failure box's close box is its Cancel: the held paste ends at the failed file.
#[test]
fn closing_the_failure_box_ends_its_paste() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("gone.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert!(ex.holds_a_job(), "the paste holds on the failure box");

    ex.dialog_event(repriseapp::Event::CloseRequest, &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.busy());
    assert!(ex.dialog().is_none(), "no box follows");
    assert!(!dir.join("b.txt").exists(), "the paste ended at gone.txt");
}

/// A paste cancelled from its progress window, by its X or its Cancel, while it holds on a failed
/// item ends, and its failure box goes with it, whether up or waiting behind another box.
#[test]
fn a_paste_cancelled_from_its_progress_window_takes_its_failure_box_with_it() {
    let cancels = [
        ("X", repriseapp::Event::CloseRequest),
        ("Cancel", key_event_escape()),
    ];
    for (how, cancel) in cancels {
        for waiting in [false, true] {
            let home = tempfile::tempdir().expect("a temp dir");
            let (from, dir) = (home.path().join("from"), home.path().join("to"));
            std::fs::create_dir_all(&from).expect("mkdir");
            std::fs::create_dir_all(&dir).expect("mkdir");
            std::fs::write(from.join("b.txt"), "b").expect("write");
            let mut ctx = test_ctx();
            let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
            if waiting {
                ex.message_box("Explorer", "Up first".into(), &mut ctx);
            }
            ex.run(Command::Paste, &mut ctx);
            ex.pasted(
                FileClip {
                    cut: false,
                    paths: vec![from.join("gone.txt"), from.join("b.txt")],
                },
                &mut ctx,
            );
            let holds = |ex: &Explorer| {
                (ex.dialogs.iter()).any(|d| matches!(d, Dialog::Confirm(c) if c.job.is_some()))
            };
            crate::tests::wake_until(&mut ex, &mut ctx, holds);
            let p = &mut ex.put.as_mut().expect("the paste").progress;
            let accent = reprisekit::theme::Color::rgb(0, 0, 128);
            let cp = reprisekit::controls::copyprogress::CopyProgress::new(
                p.kind, &p.from, &p.to, accent,
            );
            p.shown = Some(crate::copydlg::Shown { cp, focused: false });

            ex.event(crate::copydlg::COPY, cancel.clone(), &mut ctx);
            crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.busy());
            let case = format!("{how}, waiting: {waiting}");
            assert!(!holds(&ex), "the failure box is gone ({case})");
            assert_eq!(
                ex.dialogs.len(),
                waiting as usize,
                "the other box stays ({case})"
            );
            assert!(
                !dir.join("b.txt").exists(),
                "the paste ended at gone.txt ({case})"
            );
        }
    }
}

/// A failed item's box raised while the Open With chooser is up waits for it, and comes up once
/// Escape closes the chooser.
#[test]
fn a_failure_behind_the_chooser_comes_up_when_it_closes() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    std::fs::write(dir.join("x.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.choose_program_for(&dir.join("x.txt"), None, &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("gone.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.boxes_waiting() > 0);
    assert!(matches!(ex.dialog(), Some(Dialog::ChooseProgram(_))));

    ex.event(crate::DIALOG, key_event_escape(), &mut ctx);
    assert!(ex.holds_a_job(), "the failure box comes up");
    ex.answer_failed(reprisefs::progress::Decision::Skip, &mut ctx);
    assert!(dir.join("b.txt").is_file(), "Skip goes on with the paste");
}

/// A delete question raised while a failed item's box is up waits behind it, and asks once the
/// failure is answered.
#[test]
fn a_delete_question_waits_behind_a_failed_items_box() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    std::fs::write(dir.join("x.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("gone.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert!(ex.holds_a_job(), "the paste holds on the failure box");

    let x = ex.items.iter().position(|it| it.name == "x.txt");
    ex.sv.selection.set_only(x);
    ex.run(Command::Delete { erase: false }, &mut ctx);
    assert!(ex.holds_a_job(), "the paste's question stays up");

    ex.event(crate::DIALOG, key_event_alt('s'), &mut ctx);
    match ex.dialog() {
        Some(Dialog::Confirm(c)) => assert_eq!(c.paths, vec![dir.join("x.txt")]),
        _ => panic!("the delete asks next"),
    }
}

#[test]
fn a_file_pasted_onto_a_folders_name_lands_numbered() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(dir.join("b.txt")).expect("mkdir");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("b.txt")],
        },
        &mut ctx,
    );
    assert!(ex.dialog().is_none(), "nothing to ask");
    ex.finish_puts(&mut ctx);
    assert!(dir.join("b.txt").is_dir());
    assert_eq!(
        std::fs::read_to_string(dir.join("b (2).txt")).expect("read"),
        "b"
    );
}

/// Two windows share one undo history: Undo in the window that copied first reverses the other
/// window's later rename, which its Edit menu names, then its own copy.
#[test]
fn undo_in_one_window_reverses_the_newest_operation_of_any_window_first() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let config = home.path().join("config/reprise/config.toml");
    std::fs::create_dir_all(config.parent().expect("a folder")).expect("mkdir");
    std::fs::write(&config, "desktop_extensions = \"show\"\n").expect("write");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    std::fs::write(dir.join("b.txt"), "b").expect("write");
    let history = home.path().join("reprise-undo");
    let mut ctx = test_ctx();
    let mut copier = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    let mut renamer = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    copier.undos = reprisefs::undo::SharedUndo::at(history.clone());
    renamer.undos = reprisefs::undo::SharedUndo::at(history);

    copier.run(Command::Paste, &mut ctx);
    copier.pasted(
        FileClip {
            cut: false,
            paths: vec![from.join("a.txt")],
        },
        &mut ctx,
    );
    copier.finish_puts(&mut ctx);
    renamer.sv.selection.set_only(Some(0));
    renamer.run(Command::Rename, &mut ctx);
    let (_, edit) = renamer.sv.edit.as_mut().expect("the rename box");
    *edit = reprisekit::textedit::TextEdit::new("c.txt", false);
    renamer.commit_rename(&mut ctx);
    renamer.finish_puts(&mut ctx);
    assert!(dir.join("a.txt").is_file() && dir.join("c.txt").is_file());

    copier.sync_undo();
    assert_eq!(copier.undo, Some("Rename"));
    copier.run(Command::Undo, &mut ctx);
    copier.finish_puts(&mut ctx);
    assert!(dir.join("b.txt").is_file() && !dir.join("c.txt").exists());
    assert_eq!(copier.undo, Some("Copy"));
    copier.run(Command::Undo, &mut ctx);
    copier.finish_puts(&mut ctx);
    assert!(!dir.join("a.txt").exists() && from.join("a.txt").is_file());
    renamer.sync_undo();
    assert_eq!(renamer.undo, None);
}

/// An open menu's Undo row takes the entry it named, not one another window pushed since.
#[test]
fn the_undo_row_takes_only_the_entry_it_named() {
    use reprisefs::undo::{Undoable, file_id};
    let home = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(home.path().to_path_buf()), &mut ctx);
    let [named, later] = ["named", "later"].map(|n| home.path().join(n));
    let push = |ex: &Explorer, p: &std::path::Path| {
        std::fs::create_dir(p).expect("mkdir");
        ex.undos.push(Undoable::New(p.to_path_buf(), file_id(p)));
    };
    push(&ex, &named);
    ex.sync_undo();
    let menu = crate::menus::background_menu(&ex.menu_state());
    ex.popup = Some(crate::window::Popup::new((0, 0), menu, &ctx, false));
    push(&ex, &later);
    ex.sync_undo();
    assert_eq!(ex.undo, Some("New"));

    ex.popup = None;
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        named.exists() && later.exists(),
        "its entry is no longer on top"
    );
    ex.sync_undo();
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(named.exists() && !later.exists());
}

/// While another window's undo runs, the Undo rows grey and Ctrl+Z says why it does nothing.
#[test]
fn undo_greys_and_ctrl_z_says_so_while_another_window_undoes() {
    use reprisefs::undo::{SharedUndo, Undoable};
    let home = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(home.path().to_path_buf()), &mut ctx);
    let history = home.path().join("reprise-undo");
    ex.undos = SharedUndo::at(history.clone());
    let other = SharedUndo::at(history);
    other.push(Undoable::Copied(home.path().join("x"), None));
    other.push(Undoable::Copied(home.path().join("y"), None));
    let (undo, running) = other.take().expect("the other window's undo");

    ex.sync_undo();
    assert_eq!(ex.undo, None, "greyed");
    ex.run(Command::Undo, &mut ctx);
    match ex.dialog() {
        Some(Dialog::Message { text, .. }) => assert_eq!(text, "Another undo is still running."),
        d => panic!("a box: {:?}", d.is_some()),
    }
    ex.take_up();
    other.put_back(undo, running, 1, None);
    ex.sync_undo();
    assert_eq!(ex.undo, Some("Copy"));
}

/// While a clear the history lock kept out waits to land, the Undo rows grey and Ctrl+Z, with no
/// undo running, shows no box.
#[test]
fn undo_greys_and_ctrl_z_says_nothing_while_a_clear_waits_to_land() {
    use reprisefs::undo::{SharedUndo, Undoable};
    let home = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(home.path().to_path_buf()), &mut ctx);
    let history = home.path().join("reprise-undo");
    ex.undos = SharedUndo::at(history.clone());
    let other = SharedUndo::at(history);
    other.push(Undoable::Copied(home.path().join("x"), None));
    let lock = std::fs::File::create(home.path().join("reprise-undo.lock")).expect("lock file");
    lock.lock().expect("lock");
    other.clear();

    ex.sync_undo();
    assert_eq!(ex.undo, None, "greyed");
    ex.run(Command::Undo, &mut ctx);
    assert!(ex.dialog().is_none(), "no undo runs to say so");
    drop(lock);
    other.settle();
    ex.sync_undo();
    assert_eq!(ex.undo, None, "the clear landed");
}

/// Ctrl+Z right after a rename, before the window's edit of the history lands, undoes the rename
/// once it does.
#[test]
fn ctrl_z_before_the_windows_own_edit_lands_undoes_it_once_it_does() {
    let home = tempfile::tempdir().expect("a temp dir");
    let dir = home.path();
    std::fs::create_dir(dir.join("b")).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));
    ex.run(Command::Rename, &mut ctx);
    let (_, edit) = ex.sv.edit.as_mut().expect("the rename box");
    *edit = reprisekit::textedit::TextEdit::new("c", false);
    ex.commit_rename(&mut ctx);
    assert!(dir.join("c").is_dir());
    assert_eq!(ex.undo_edits_pending, 1, "the rename's entry is on its way");

    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(dir.join("b").is_dir() && !dir.join("c").exists());
    assert!(ex.dialog().is_none());
}

/// An edit of the undo history that panics still comes off the window's count, so Undo and a
/// close go on, and the edits queued behind it run.
#[test]
fn a_panicking_undo_history_edit_frees_the_window_and_the_edits_behind_it_run() {
    use reprisefs::undo::Undoable;
    let home = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(home.path().to_path_buf()), &mut ctx);
    let (go, wait) = std::sync::mpsc::channel::<()>();
    ex.record_undos(&ctx, move |_| {
        let _ = wait.recv();
        panic!("an edit that panics");
    });
    let behind = home.path().join("behind");
    ex.record_undos(&ctx, move |undos| undos.push(Undoable::New(behind, None)));
    go.send(()).expect("the first edit waits");

    let start = std::time::Instant::now();
    while ex.busy() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "the window is left busy"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        ex.poll_jobs(&mut ctx);
    }
    assert_eq!((ex.undo_edits_pending, ex.jobs_pending), (0, 0));
    assert_eq!(ex.undo, Some("New"), "the edit behind the panic ran");
}

/// Undo of a move whose source folder vanished fails, says so, and keeps the batch, so Undo again once
/// the folder is back finishes the job.
#[test]
fn undo_after_the_source_folder_vanished_keeps_the_batch_for_a_retry() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert_eq!(ex.undo, Some("Move"));

    std::fs::remove_dir_all(&from).expect("rmdir");
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    match ex.dialog().expect("the failure is reported") {
        Dialog::Message { title, text, .. } => {
            assert_eq!(title, "Error");
            assert!(text.starts_with("Cannot undo: "), "{text}");
        }
        _ => panic!("a message box"),
    }
    assert_eq!(ex.undo, Some("Move"), "the batch stays for a retry");
    assert!(dir.join("a.txt").is_file() && dir.join("b.txt").is_file());

    ex.take_up();
    std::fs::create_dir_all(&from).expect("mkdir");
    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(from.join("a.txt").is_file() && from.join("b.txt").is_file());
    assert_eq!(ex.undo, None);
}

/// A move takes an item off its old folder's `.hidden` and Undo Move puts it back, so it is hidden
/// again where it came from. One the list never named stays off it.
#[test]
fn undo_of_a_move_lists_a_hidden_item_again() {
    let home = tempfile::tempdir().expect("a temp dir");
    let from = home.path().join("from");
    let dir = home.path().join("to");
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    std::fs::write(from.join("b.txt"), "b").expect("write");
    std::fs::write(from.join(".hidden"), "a.txt\n").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt"), from.join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    let list = || std::fs::read_to_string(from.join(".hidden")).unwrap_or_default();
    assert_eq!(list(), "", "the move let go of the line");

    ex.run(Command::Undo, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(from.join("a.txt").is_file() && from.join("b.txt").is_file());
    assert_eq!(list(), "a.txt\n");
}

#[test]
fn undo_of_a_move_runs_on_a_job_after_the_handler_returns() {
    let home = tempfile::tempdir().expect("a temp dir");
    let (from, dir) = (home.path().join("from"), home.path().join("to"));
    std::fs::create_dir_all(&from).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(from.join("a.txt"), "a").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![from.join("a.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);

    ex.run(Command::Undo, &mut ctx);
    assert!(
        dir.join("a.txt").is_file(),
        "the handler moves nothing back"
    );
    assert!(ex.busy(), "the undo is work a close waits for");
    while ex.removing.is_none() {
        std::thread::sleep(std::time::Duration::from_millis(2));
        ex.poll_jobs(&mut ctx);
    }
    assert_eq!(ex.undo, None, "its entry left the stack");
    let progress = &ex.removing.as_ref().expect("the undo's job").progress;
    assert_eq!(
        (progress.kind, &*progress.from, &*progress.to),
        (reprisekit::controls::copyprogress::Kind::Move, "to", "from"),
        "its dialog moves from where the item is back to where it was"
    );
    ex.finish_puts(&mut ctx);
    assert!(from.join("a.txt").is_file() && !dir.join("a.txt").exists());
    assert_eq!(ex.undo, None);
}

#[test]
fn a_close_waits_for_a_put_back_the_history_lock_kept_out() {
    use reprisefs::undo::{SharedUndo, Undoable};
    let home = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(home.path().to_path_buf()), &mut ctx);
    ex.undos = SharedUndo::at(home.path().join("reprise-undo"));
    let [a, b] = ["a.txt", "b.txt"].map(|n| home.path().join(n));
    ex.undos
        .push_all(vec![Undoable::Copied(a, None), Undoable::Copied(b, None)]);

    ex.sync_undo();
    ex.run(Command::Undo, &mut ctx);
    while ex.removing.is_none() {
        std::thread::sleep(std::time::Duration::from_millis(2));
        ex.poll_jobs(&mut ctx);
    }
    let lock = std::fs::File::create(home.path().join("reprise-undo.lock")).expect("lock file");
    lock.lock().expect("lock");
    let held = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        drop(lock);
    });
    ex.finish_puts(&mut ctx);
    held.join().expect("the lock's thread");

    assert!(!ex.undos.retrying(), "the work ended with the retry");
    assert_eq!(ex.undo, Some("Copy"), "the batch's rest is back");
}

#[test]
fn cancel_drops_the_pastes_waiting_behind_the_running_one() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    std::fs::write(dir.path().join("b.txt"), "2").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    for name in ["a.txt", "b.txt"] {
        ex.run(Command::Paste, &mut ctx);
        ex.pasted(
            FileClip {
                cut: false,
                paths: vec![dir.path().join(name)],
            },
            &mut ctx,
        );
    }
    assert_eq!(ex.put_queue.len(), 1, "the second paste waits");

    ex.cancel_puts();
    assert!(ex.put_queue.is_empty());
    ex.finish_puts(&mut ctx);

    assert!(
        !dir.path().join("Copy of b.txt").exists(),
        "the waiting paste never runs"
    );
}

/// Paste on a folder entry copies the clipboard's files into that folder, even after the window
/// has moved on, and a cut moves them.
#[test]
fn paste_on_a_folder_lands_the_clipboards_files_inside_it() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sub = dir.path().join("Sub");
    std::fs::create_dir(&sub).expect("mkdir");
    std::fs::write(dir.path().join("a.txt"), "a").expect("write");
    std::fs::write(dir.path().join("b.txt"), "b").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let folder = ex
        .items
        .iter()
        .position(|it| it.is_dir)
        .expect("Sub is listed");

    ex.entry_menu_at(folder, (0, 0), &mut ctx, false);
    let reply = ex.run(Command::PasteOnItem, &mut ctx);
    assert!(reply.paste_files, "the clipboard was asked for");
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(sub.join("a.txt")).expect("read"),
        "a"
    );
    assert!(
        dir.path().join("a.txt").exists(),
        "a copy leaves the source"
    );

    ex.entry_menu_at(folder, (0, 0), &mut ctx, false);
    ex.run(Command::PasteOnItem, &mut ctx);
    ex.go(Place::Computer, &mut ctx);
    ex.pasted(
        FileClip {
            cut: true,
            paths: vec![dir.path().join("b.txt")],
        },
        &mut ctx,
    );
    ex.finish_puts(&mut ctx);
    assert!(
        sub.join("b.txt").exists(),
        "the folder was named by the menu"
    );
    assert!(!dir.path().join("b.txt").exists(), "a cut moves");
}

/// Paste on a launcher opens each clipboard file in its program, in its app scope, and copies
/// nothing into the folder.
#[test]
fn paste_on_a_launcher_opens_the_clipboards_files_in_its_program() {
    let dir = tempfile::tempdir().expect("a temp dir");
    reprisefs::apps::write_launcher(
        &dir.path().join("editor.desktop"),
        "[Desktop Entry]\nType=Application\nName=Editor\nExec=editor %f\n",
    )
    .expect("write");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let launcher = ex
        .items
        .iter()
        .position(|it| it.shortcut.is_some())
        .expect("the launcher is listed");

    ex.entry_menu_at(launcher, (0, 0), &mut ctx, false);
    let reply = ex.run(Command::PasteOnItem, &mut ctx);
    assert!(reply.paste_files, "the clipboard was asked for");
    let paths = vec![dir.path().join("a.txt"), PathBuf::from("/srv/b.txt")];
    let reply = ex.pasted(FileClip { cut: true, paths }, &mut ctx);
    assert_eq!(
        reply.spawn,
        [
            vec![OsString::from("editor"), dir.path().join("a.txt").into()],
            vec![OsString::from("editor"), OsString::from("/srv/b.txt")],
        ]
    );
    assert_eq!(reply.spawn_app.as_ref().map(|a| a.as_str()), Some("editor"));
    assert!(ex.put.is_none(), "nothing is copied");
    assert!(dir.path().join("a.txt").exists(), "a cut file stays put");

    let reply = ex.run(Command::Paste, &mut ctx);
    assert!(reply.paste_files);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(
        ex.put.is_some(),
        "a later Paste copies into the folder again"
    );
}

/// A mount's job asking for the whole window drawn and a quick one landing with it in one wake.
#[test]
fn jobs_landing_in_one_wake_keep_every_reply() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let mut ex = crate::tests::test_explorer(Place::Folder(t.path().to_path_buf()), &mut ctx);
    let replies: [fn() -> Reply; 2] = [Reply::redraw_all, Reply::redraw];
    for reply in replies {
        ex.jobs_pending += 1;
        let done: JobDone = Box::new(move |_, _| reply());
        ex.jobs_tx.send(done).expect("send");
    }
    let got = ex.poll_jobs(&mut ctx).expect("two jobs landed");
    assert!(got.redraw_all, "the first job's redraw_all is kept");
    assert_eq!(ex.jobs_pending, 0);
}

/// A finished paste decides on refilling the Folders tree from its own items: the landed copy is
/// not read again on the window's thread, where a drive that stopped answering would freeze it.
#[test]
fn a_pasted_folder_refills_the_tree_without_reading_its_landed_copy() {
    let dir = tempfile::Builder::new()
        .prefix("tree")
        .tempdir()
        .expect("a temp dir shown in the tree");
    let src = tempfile::tempdir().expect("a temp dir");
    std::fs::create_dir(src.path().join("Reports")).expect("mkdir");
    std::fs::write(src.path().join("Reports/r.txt"), "x").expect("write");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = crate::tests::test_explorer(here.clone(), &mut ctx);
    let path = ex.tree_path_of(&here).expect("the folder's node");
    let labels = |ex: &Explorer| -> Vec<String> {
        let node = ex.tree.node(&path).expect("node");
        node.children.iter().map(|n| n.label.clone()).collect()
    };
    assert!(labels(&ex).is_empty());

    ex.put_all(
        &[src.path().join("Reports")],
        dir.path(),
        &|_| Some(DropOp::Copy),
        false,
        &mut ctx,
    );
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while ex.put.as_ref().expect("a put").job.progress().phase
        != reprisefs::progress::Phase::Finishing
    {
        assert!(std::time::Instant::now() < end, "the folder lands");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    std::fs::rename(dir.path().join("Reports"), dir.path().join("Moved")).expect("rename");
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| {
        ex.put.is_none() && ex.filling.is_empty()
    });
    assert_eq!(labels(&ex), ["Moved"], "the node was read again");
}

#[test]
fn a_skipped_item_takes_its_folder_flag_with_it() {
    let item = |name: &str| PutItem {
        src: PathBuf::from("/srv").join(name),
        display_name: name.into(),
        is_app: false,
        op: DropOp::Copy,
        shortcut: Box::new(|_| None),
        replace: None,
    };
    let mut pending = PendingPut {
        items: vec![item("a"), item("b.txt")],
        folders: vec![true, false],
        dir: PathBuf::from("/srv/dest"),
        select_landed: false,
        zip: None,
    };
    pending.apply(&[Verdict::Skip, Verdict::Put]);
    let ops = pending.ops();
    assert_eq!(ops.len(), 1);
    assert_eq!(
        (ops[0].0.as_path(), ops[0].3),
        (Path::new("/srv/b.txt"), false)
    );
}

/// A failed item and the end of its job read in one wake: the put ends and says why, and no box
/// asks Skip, Retry or Cancel of a job that is over.
#[test]
fn a_failure_read_with_the_end_of_its_put_ends_the_put_and_says_why() {
    use std::os::unix::fs::PermissionsExt;
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let dest = t.path().join("dest");
    std::fs::create_dir(&dest).expect("mkdir");
    std::fs::write(t.path().join("a.txt"), "a").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(t.path().to_path_buf()), &mut ctx);
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let pending = PendingPut {
        items: vec![PutItem {
            src: t.path().join("a.txt"),
            display_name: "a.txt".into(),
            is_app: false,
            op: DropOp::Copy,
            shortcut: Box::new(|_| None),
            replace: None,
        }],
        folders: vec![false],
        dir: dest.clone(),
        select_landed: false,
        zip: None,
    };
    let ops = pending.ops();
    ex.start_put(pending, ops, &ctx);
    let meter = ex.put.as_ref().expect("the put").progress.meter.clone();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !meter.asking() {
        assert!(std::time::Instant::now() < end, "the copy fails");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // The job goes on and ends before Explorer reads its failure.
    meter.decide(reprisefs::progress::Decision::Skip);
    while std::sync::Arc::strong_count(&meter) > 3 {
        assert!(std::time::Instant::now() < end, "the job's threads return");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    ex.poll_put(&mut ctx);
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(ex.put.is_none(), "the put ended");
    assert!(!ex.holds_a_job(), "nothing asks about a job that ended");
    match ex.dialog() {
        Some(Dialog::Message { text, .. }) => assert!(text.contains("a.txt"), "{text}"),
        _ => panic!("the failure is shown"),
    }
}

/// Ctrl+V onto a taken name, then Enter, typed while the clipboard is slow: the paste lands and
/// asks to replace, and the Enter, typed before the question was up, does not answer Yes.
#[test]
fn enter_typed_ahead_of_a_paste_does_not_answer_the_replace_prompt_it_raises() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for (d, body) in [(&src, "new"), (&dest, "old")] {
        std::fs::create_dir_all(d).expect("mkdir");
        std::fs::write(d.join("a.txt"), body).expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let boxes = |ex: &Explorer| -> Vec<repriseapp::WinId> {
        let kids = ex.children().into_iter();
        kids.filter(|c| c.modal).map(|c| c.id).collect()
    };
    let mut wait = repriseapp::PasteWait::default();
    let mut asked_for = Vec::new();
    let ctrl_v = repriseapp::KeyInput {
        sym: 'v' as u32,
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: true,
            ..Default::default()
        },
        edit: None,
    };
    for ev in [repriseapp::Event::Key(ctrl_v), enter()] {
        if let Some(ev) = wait.hold(repriseapp::MAIN, ev)
            && ex.event(repriseapp::MAIN, ev, &mut ctx).paste_files
        {
            asked_for.push(wait.begin(repriseapp::MAIN));
        }
    }
    assert_eq!(asked_for.len(), 1, "Ctrl+V asked for the clipboard");
    let clip = FileClip {
        cut: false,
        paths: vec![src.join("a.txt")],
    };
    wait.land(asked_for[0], Some(repriseapp::Event::PastedFiles(clip)));
    while let Some((win, held)) = wait.pop() {
        let repriseapp::Held::Event(ev) = held else {
            continue;
        };
        let before = boxes(&ex);
        let gate = before.last().copied().filter(|_| win == repriseapp::MAIN);
        let to = wait.aim(win, &ev, gate);
        let key = matches!(ev, repriseapp::Event::Key(_) | repriseapp::Event::KeyUp(_));
        if !(key && gate.is_some() && to == repriseapp::MAIN) {
            ex.event(to, ev, &mut ctx);
        }
        wait.opened(&before, &boxes(&ex));
    }
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace".to_string(), "a.txt".to_string()))
    );
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "old"
    );
}

#[test]
fn a_start_menu_program_dropped_in_a_folder_lands_as_a_launcher() {
    use std::os::unix::fs::PermissionsExt;
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let menu = home.path().join("applications");
    std::fs::create_dir(&menu).expect("mkdir");
    let entry = "[Desktop Entry]\nType=Application\nName=Calculator\nExec=calc\n";
    std::fs::write(menu.join("calc.desktop"), entry).expect("write");
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);

    ex.drop_into(&[menu.join("calc.desktop")], &dir, DropOp::Copy, &mut ctx);
    ex.finish_puts(&mut ctx);

    let landed = dir.join("calc.desktop");
    let mode = std::fs::metadata(&landed)
        .expect("landed")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "expected executable, mode {mode:o}");
    assert!(reprisefs::apps::is_trusted(&landed), "expected trusted");
}
