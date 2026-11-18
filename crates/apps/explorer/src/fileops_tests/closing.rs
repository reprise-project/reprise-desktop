use super::*;

/// Close during a copy hides the window and quits once the copy has landed and recorded its undo.
#[test]
fn close_waits_for_a_copy_in_flight() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.run(Command::Paste, &mut ctx);
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![dir.path().join("a.txt")],
        },
        &mut ctx,
    );
    assert!(ex.put.is_some());

    let reply = ex.run(Command::Close, &mut ctx);
    assert!(!reply.quit, "the copy is still out");
    assert!(ex.closing);

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
    assert!(quit, "the wake after the copy quits");
    assert!(dir.path().join("Copy of a.txt").exists());
    assert_eq!(ex.undo, Some("Copy"));
}

#[test]
fn log_off_is_held_off_while_a_copy_runs_and_let_go_once_it_lands() {
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("a.txt"), "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert_eq!(ex.logout_reason(), None);
    ex.run(Command::Paste, &mut ctx);
    let clip = FileClip {
        cut: false,
        paths: vec![dir.path().join("a.txt")],
    };
    ex.pasted(clip, &mut ctx);
    ex.hold_logout();
    assert_eq!(ex.logout_held, Some("Copying files"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while ex.put.is_some() {
        assert!(std::time::Instant::now() < deadline, "the copy lands");
        ex.wake(&mut ctx);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    ex.hold_logout();
    assert_eq!(ex.logout_held, None);
}

#[test]
fn close_when_idle_quits_at_once() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    assert!(ex.run(Command::Close, &mut ctx).quit);
    assert!(!ex.closing);
}

#[test]
fn a_close_after_a_put_gave_up_waits_for_its_stuck_call_to_return() {
    let mut ctx = test_ctx();
    let t = tempfile::tempdir().expect("a temp dir");
    let dest = t.path().join("dest");
    std::fs::create_dir_all(&dest).expect("mkdir");
    std::fs::write(t.path().join("a.txt"), "a").expect("write");
    let mut ex = crate::tests::test_explorer(Place::Folder(dest.clone()), &mut ctx);
    ex.action_deadline = std::time::Duration::from_millis(20);
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = std::sync::Mutex::new(gate);
    let pending = PendingPut {
        items: vec![PutItem {
            src: t.path().join("a.txt"),
            display_name: "a.txt".into(),
            is_app: false,
            op: DropOp::Shortcut,
            shortcut: Box::new(move |_| {
                let _ = gate.lock().expect("gate").recv();
                Some(String::new())
            }),
            replace: None,
        }],
        folders: vec![false],
        dir: dest.clone(),
        select_landed: false,
    };
    let ops = pending.ops();
    ex.start_put(pending, ops, &ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    match ex.dialog() {
        Some(Dialog::Message { text, .. }) => {
            assert!(text.ends_with("The location is not responding."), "{text}")
        }
        _ => panic!("the give-up box is up"),
    }
    ex.take_up();
    assert!(!ex.close(&mut ctx).quit, "the stuck call is still out");
    release.send(()).expect("release");
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.busy());
    assert!(
        ex.wake(&mut ctx).quit,
        "the window quits once the call returns"
    );
}

/// A failure that lands after the folder window closed comes up on its own, with a task button,
/// and the process ends only once it is answered.
#[test]
fn a_failure_landing_after_the_window_closed_comes_up_and_the_process_waits_for_it() {
    use repriseapp::{App, Caption, Extended};
    let dir = tempfile::tempdir().expect("a temp dir");
    let one = dir.path().join("one.txt");
    std::fs::write(&one, "1").expect("write");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let gate = std::sync::Arc::new(std::sync::RwLock::new(()));
    let renaming = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    ex.sv.selection.set_only(Some(0));
    ex.sv.edit = Some((0, reprisekit::textedit::TextEdit::new("two.txt", false)));
    ex.commit_rename(&mut ctx);
    ex.held = None;
    let mut app = crate::Lazy {
        start: crate::parse_args([dir.path().as_os_str().to_os_string()]),
        inner: Some(ex),
        units: reprisekit::theme::TextUnits::ONE,
    };
    let ex = app.inner.as_mut().expect("the window");
    assert!(!ex.close(&mut ctx).quit, "the rename is still out");
    std::fs::remove_file(&one).expect("rm");
    drop(renaming);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.inner.as_ref().is_some_and(|ex| ex.dialog().is_none()) {
        assert!(std::time::Instant::now() < end, "the rename lands");
        let woke = app.event(crate::MAIN, repriseapp::Event::Wake, &mut ctx);
        assert!(!woke.quit, "no quit with the failure owed");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let win = app.inner.as_ref().expect("the window").dialog_win();
    assert!(app.children().iter().any(|c| c.id == win), "the box shows");
    assert_eq!(app.caption(win), Caption::Fixed, "with a task button");
    assert!(
        !app.event(crate::MAIN, repriseapp::Event::Wake, &mut ctx)
            .quit
    );
    assert!(app.event(win, key_event_escape(), &mut ctx).quit);
}
