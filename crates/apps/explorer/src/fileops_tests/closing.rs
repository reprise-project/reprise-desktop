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
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog.is_some());
    match &ex.dialog {
        Some(Dialog::Message { text, .. }) => {
            assert!(text.ends_with("The location is not responding."), "{text}")
        }
        _ => panic!("the give-up box is up"),
    }
    ex.dialog = None;
    assert!(!ex.close(&mut ctx).quit, "the stuck call is still out");
    release.send(()).expect("release");
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| !ex.busy());
    assert!(
        ex.wake(&mut ctx).quit,
        "the window quits once the call returns"
    );
}
