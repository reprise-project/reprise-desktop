use super::*;

fn key_event_escape() -> repriseapp::Event {
    repriseapp::Event::Key(repriseapp::KeyInput {
        sym: reprisekit::textedit::keysym::ESCAPE,
        utf8: None,
        mods: repriseapp::Mods {
            ctrl: false,
            shift: false,
            alt: false,
        },
        edit: None,
    })
}

/// Two files and a folder pasted into `dest` where all three names are taken: the replace prompt comes up before anything is written.
fn paste_onto_taken_names(
    ctx: &mut repriseapp::Ctx,
) -> (tempfile::TempDir, std::path::PathBuf, Explorer) {
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d.join("Reports")).expect("mkdir");
        std::fs::write(d.join("Reports/r.txt"), d.to_string_lossy().as_bytes()).expect("write");
    }
    for (name, body) in [("a.txt", "new a"), ("b.txt", "new b")] {
        std::fs::write(src.join(name), body).expect("write");
        std::fs::write(dest.join(name), "old").expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), ctx);
    ex.run(Command::Paste, ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: ["a.txt", "b.txt", "Reports"]
                .iter()
                .map(|n| src.join(n))
                .collect(),
        },
        ctx,
    );
    assert!(ex.put.is_none(), "nothing runs before the answer");
    (t, dest, ex)
}

/// A paste's clashes, a merged folder's files among them, are checked on a listing thread: on a
/// folder that does not answer (a dead mount) the prompt comes up on the wake after the check
/// lands, and a paste made meanwhile asks after it.
#[test]
fn a_paste_whose_clash_check_answers_late_asks_on_a_later_wake() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d.join("Reports")).expect("mkdir");
        std::fs::write(d.join("Reports/r.txt"), "x").expect("write");
        std::fs::write(d.join("a.txt"), "x").expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    for name in ["Reports", "a.txt"] {
        ex.put_all(
            &[src.join(name)],
            &dest,
            &|_| Some(DropOp::Copy),
            false,
            &mut ctx,
        );
    }
    assert_eq!(asked(&ex), None, "no prompt before the check answers");
    assert!(ex.busy(), "a close waits for the check");
    drop(closed);
    ex.held = None;
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    assert_eq!(
        asked(&ex),
        Some(("Confirm Folder Replace", "Reports".to_string()))
    );
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.clashing.is_empty());
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string()))
    );
}

#[test]
fn a_paste_whose_clash_check_passes_the_deadline_gives_up_and_copies_nothing() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    std::fs::write(src.join("a.txt"), "x").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    ex.action_deadline = std::time::Duration::from_millis(20);
    let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.put_all(
        &[src.join("a.txt")],
        &dest,
        &|_| Some(DropOp::Copy),
        false,
        &mut ctx,
    );
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.clashing.is_empty());
    match &ex.dialog {
        Some(Dialog::Message { title, text }) => assert_eq!(
            (title.as_str(), text.as_str()),
            (
                "Error Copying File or Folder",
                "The location is not responding."
            )
        ),
        _ => panic!("the give-up box is up"),
    }
    assert!(!ex.busy());
    drop(closed);
    ex.held = None;
    std::thread::sleep(std::time::Duration::from_millis(100));
    ex.wake(&mut ctx);
    assert!(
        !dest.join("a.txt").exists(),
        "the late check starts no copy"
    );
}

/// The replace prompt's size and date lines stat both files on a listing thread: on a file that
/// does not answer (a dead mount) the prompt comes up on the wake after the read lands, and a
/// paste made meanwhile asks after it.
#[test]
fn a_replace_prompt_whose_files_answer_late_comes_up_on_a_later_wake() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for (d, text) in [(&src, "x"), (&dest, "old")] {
        std::fs::create_dir_all(d).expect("mkdir");
        std::fs::write(d.join("a.txt"), text).expect("write");
        std::fs::write(d.join("b.txt"), text).expect("write");
    }
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let check = std::sync::Arc::new(std::sync::RwLock::new(()));
    let checking = check.write().expect("lock");
    ex.held = Some(check.clone());
    for name in ["a.txt", "b.txt"] {
        ex.put_all(
            &[src.join(name)],
            &dest,
            &|_| Some(DropOp::Copy),
            false,
            &mut ctx,
        );
    }
    let stat = std::sync::Arc::new(std::sync::RwLock::new(()));
    let statting = stat.write().expect("lock");
    ex.held = Some(stat.clone());
    drop(checking);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.clashing.is_empty());
    assert_eq!(asked(&ex), None, "no prompt before its files are read");
    assert!(ex.asking.is_some());
    drop(statting);
    ex.held = None;
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string()))
    );
    match &ex.dialog {
        Some(Dialog::Replace(r)) => match &r.fields.subject {
            replacebox::Subject::File { existing, incoming } => {
                assert_eq!(
                    (existing.size.as_str(), incoming.size.as_str()),
                    ("3 bytes", "1 bytes")
                );
            }
            replacebox::Subject::Folder => panic!("a file clash shows both files"),
        },
        _ => panic!("the replace prompt is up"),
    }
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string()))
    );
}

/// After Close the window is hidden, so a prompt could never be answered.
#[test]
fn a_paste_whose_clash_check_answers_after_close_keeps_the_clashes_puts_the_rest_and_quits() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    std::fs::write(src.join("a.txt"), "new").expect("write");
    std::fs::write(dest.join("a.txt"), "old").expect("write");
    std::fs::write(src.join("b.txt"), "b").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.put_all(
        &[src.join("a.txt"), src.join("b.txt")],
        &dest,
        &|_| Some(DropOp::Copy),
        false,
        &mut ctx,
    );
    assert!(!ex.run(Command::Close, &mut ctx).quit, "the check is out");
    drop(closed);
    ex.held = None;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let quit = loop {
        if ex.wake(&mut ctx).quit {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(asked(&ex), None, "no prompt on the hidden window");
    assert!(quit, "the closed window quits once the paste is done");
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).ok().as_deref(),
        Some("old")
    );
    assert!(
        dest.join("b.txt").exists(),
        "b.txt, which did not clash, is copied"
    );
}

/// Close lands while the first prompt's files are still being read and a second paste waits
/// behind it: neither prompt can be answered on the hidden window.
#[test]
fn a_paste_whose_prompt_is_still_reading_at_close_keeps_the_clashes_puts_the_rest_and_quits() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    for name in ["a.txt", "b.txt"] {
        std::fs::write(src.join(name), "new").expect("write");
        std::fs::write(dest.join(name), "old").expect("write");
    }
    std::fs::write(src.join("c.txt"), "c").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    let check = std::sync::Arc::new(std::sync::RwLock::new(()));
    let checking = check.write().expect("lock");
    ex.held = Some(check.clone());
    for batch in [
        &[src.join("a.txt"), src.join("c.txt")][..],
        &[src.join("b.txt")],
    ] {
        ex.put_all(batch, &dest, &|_| Some(DropOp::Copy), false, &mut ctx);
    }
    let stat = std::sync::Arc::new(std::sync::RwLock::new(()));
    let statting = stat.write().expect("lock");
    ex.held = Some(stat.clone());
    drop(checking);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.clashing.is_empty());
    assert!(ex.asking.is_some() && ex.replace_waiting.len() == 1);
    assert!(
        !ex.run(Command::Close, &mut ctx).quit,
        "the prompt's read is out"
    );
    drop(statting);
    ex.held = None;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let quit = loop {
        if ex.wake(&mut ctx).quit {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(asked(&ex), None, "no prompt on the hidden window");
    assert!(quit, "the closed window quits once the pastes are done");
    for name in ["a.txt", "b.txt"] {
        assert_eq!(
            std::fs::read_to_string(dest.join(name)).ok().as_deref(),
            Some("old"),
            "{name} is kept"
        );
    }
    assert!(
        dest.join("c.txt").exists(),
        "c.txt, which did not clash, is copied"
    );
}

/// Close from the taskbar, which the modal prompt does not block, while one paste's Replace
/// prompt is up and a second paste waits behind it.
#[test]
fn closing_with_the_replace_prompt_up_answers_no_puts_the_rest_and_quits() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    for d in [&src, &dest] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    for name in ["a.txt", "b.txt"] {
        std::fs::write(src.join(name), "new").expect("write");
        std::fs::write(dest.join(name), "old").expect("write");
    }
    std::fs::write(src.join("c.txt"), "c").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    for batch in [
        &[src.join("a.txt"), src.join("c.txt")][..],
        &[src.join("b.txt")],
    ] {
        ex.put_all(batch, &dest, &|_| Some(DropOp::Copy), false, &mut ctx);
    }
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| {
        ex.clashing.is_empty() && ex.asking.is_none()
    });
    assert!(asked(&ex).is_some() && ex.replace_waiting.len() == 1);
    assert!(
        !ex.run(Command::Close, &mut ctx).quit,
        "the window waits for the rest of the paste"
    );
    assert_eq!(asked(&ex), None, "no prompt on the hidden window");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let quit = loop {
        if ex.wake(&mut ctx).quit {
            break true;
        }
        if std::time::Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert!(quit, "the closed window quits once the paste is done");
    for name in ["a.txt", "b.txt"] {
        assert_eq!(
            std::fs::read_to_string(dest.join(name)).ok().as_deref(),
            Some("old"),
            "{name} is kept"
        );
    }
    assert!(
        dest.join("c.txt").exists(),
        "c.txt, which did not clash, is copied"
    );
}

/// Yes replaces the first file, No keeps the second, and the folder prompt's Yes merges, asking
/// about the file inside it the merge would replace.
#[test]
fn the_replace_prompt_asks_per_clash_and_carries_out_each_answer() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string()))
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string()))
    );
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm Folder Replace", "Reports".to_string()))
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "r.txt".to_string()))
    );
    assert!(
        matches!(&ex.dialog, Some(Dialog::Replace(r)) if r.fields.many),
        "a file inside a merge offers Yes to All"
    );
    assert!(
        matches!(&ex.dialog, Some(Dialog::Replace(r)) if r.fields.folder.as_deref() == Some("Reports")),
        "a file inside a merge names the folder 'Reports'"
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert!(ex.dialog.is_none());
    ex.finish_puts(&mut ctx);

    let read = |n: &str| std::fs::read_to_string(dest.join(n)).expect("read");
    assert_eq!(read("a.txt"), "new a");
    assert_eq!(read("b.txt"), "old");
    assert!(
        read("Reports/r.txt").ends_with("src"),
        "the merge replaced r.txt"
    );
    assert!(!dest.join("a (2).txt").exists() && !dest.join("Copy of a.txt").exists());
    assert_eq!(ex.undo, None, "a replacement cannot be undone");
}

/// No on a file inside a merged folder keeps that file; the rest of the folder still merges.
#[test]
fn no_on_a_file_inside_a_merge_keeps_it() {
    let mut ctx = test_ctx();
    let (t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    std::fs::write(t.path().join("src/Reports/mine.txt"), "mine").expect("write");
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "r.txt".to_string()))
    );
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    assert!(ex.dialog.is_none());
    ex.finish_puts(&mut ctx);
    let read = |n: &str| std::fs::read_to_string(dest.join(n)).expect("read");
    assert!(read("Reports/r.txt").ends_with("dest"), "No kept r.txt");
    assert_eq!(read("Reports/mine.txt"), "mine", "the rest merged");
}

/// Right twice moves the replace prompt's focus from Yes past All to No and Enter presses No; the next
/// question's focus is back on Yes.
#[test]
fn arrows_move_the_replace_prompt_s_focus_and_enter_presses_it() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(right(), &mut ctx);
    ex.dialog_event(right(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "the arrows leave a.txt's question up"
    );
    ex.dialog_event(enter(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "b.txt".to_string())),
        "Enter on No answers a.txt"
    );
    ex.dialog_event(enter(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm Folder Replace", "Reports".to_string())),
        "Enter on Yes answers b.txt"
    );
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    assert!(
        ex.dialog.is_none(),
        "the folder and r.txt inside it answered"
    );
    ex.finish_puts(&mut ctx);
    let read = |n: &str| std::fs::read_to_string(dest.join(n)).expect("read");
    assert_eq!(read("a.txt"), "old", "No kept a.txt");
    assert_eq!(read("b.txt"), "new b", "Yes replaced b.txt");
}

#[test]
fn yes_to_all_replaces_the_rest_and_cancel_puts_nothing() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(key_event_alt('a'), &mut ctx);
    assert!(ex.dialog.is_none(), "no more questions");
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("b.txt")).expect("read"),
        "new b"
    );

    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.dialog_event(key_event_escape(), &mut ctx);
    assert!(ex.dialog.is_none() && ex.put.is_none());
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "old"
    );
}

/// An error box raised while a replace prompt is up waits for it, and a second batch's prompt comes
/// after the first's: neither is lost.
#[test]
fn a_box_or_a_second_prompt_waits_for_the_prompt_up() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    ex.message_box(
        "Error Copying File or Folder",
        "Cannot copy x".into(),
        &mut ctx,
    );
    assert!(asked(&ex).is_some(), "the prompt stays up");

    let other = tempfile::tempdir().expect("a temp dir");
    std::fs::write(other.path().join("a.txt"), "other a").expect("write");
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![other.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "still the first batch's question"
    );
    ex.dialog_event(key_event_escape(), &mut ctx);
    assert_eq!(
        asked(&ex),
        Some(("Confirm File Replace", "a.txt".to_string())),
        "the second batch asks next"
    );
    assert!(matches!(&ex.dialog, Some(Dialog::Replace(r)) if !r.fields.many));
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    match &ex.dialog {
        Some(Dialog::Message { text, .. }) => assert_eq!(text, "Cannot copy x"),
        _ => panic!("the waiting box comes last"),
    }
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "other a"
    );
}

/// A Yes on a prompt whose file changed since it was asked numbers instead of replacing, and records its undo.
#[test]
fn a_stale_yes_lands_numbered_with_its_undo() {
    let mut ctx = test_ctx();
    let (_t, dest, mut ex) = paste_onto_taken_names(&mut ctx);
    std::fs::remove_file(dest.join("a.txt")).expect("remove");
    std::fs::write(dest.join("a.txt"), "another batch's").expect("write");
    ex.dialog_event(key_event_alt('y'), &mut ctx);
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.dialog_event(key_event_alt('n'), &mut ctx);
    ex.finish_puts(&mut ctx);
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).expect("read"),
        "another batch's"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("a (2).txt")).expect("read"),
        "new a"
    );
    assert_eq!(ex.undo, Some("Copy"), "the numbered copy can be undone");
}
