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
    assert!(ex.dialog.is_none(), "and no error box follows");
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
    assert!(ex.dialog.is_none(), "nothing to ask");
    ex.finish_puts(&mut ctx);
    assert!(dir.join("b.txt").is_dir());
    assert_eq!(
        std::fs::read_to_string(dir.join("b (2).txt")).expect("read"),
        "b"
    );
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
    match ex.dialog.as_ref().expect("the failure is reported") {
        Dialog::Message { title, text } => {
            assert_eq!(title, "Error");
            assert!(text.starts_with("Cannot undo: "), "{text}");
        }
        _ => panic!("a message box"),
    }
    assert_eq!(ex.undo, Some("Move"), "the batch stays for a retry");
    assert!(dir.join("a.txt").is_file() && dir.join("b.txt").is_file());

    ex.dialog = None;
    std::fs::create_dir_all(&from).expect("mkdir");
    ex.run(Command::Undo, &mut ctx);
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
    assert!(from.join("a.txt").is_file() && from.join("b.txt").is_file());
    assert_eq!(list(), "a.txt\n");
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
    assert_eq!(reply.spawn_app.as_deref(), Some("editor.desktop"));
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
    };
    pending.apply(&[Verdict::Skip, Verdict::Put]);
    let ops = pending.ops();
    assert_eq!(ops.len(), 1);
    assert_eq!(
        (ops[0].0.as_path(), ops[0].3),
        (Path::new("/srv/b.txt"), false)
    );
}
