use super::*;
use crate::{Lazy, MAIN, parse_args};
use repriseapp::App;

/// The program started on `args` with no folder window.
fn started(args: &[&std::ffi::OsStr]) -> Lazy {
    let app = Lazy {
        start: parse_args(args.iter().map(|a| a.to_os_string())),
        inner: None,
        units: reprisekit::theme::TextUnits::ONE,
    };
    assert!(app.start.alone.is_some(), "a job with no folder window");
    assert!(repriseapp::Extended::hidden(&app));
    app
}

/// Wakes `app` until `done` or its process ends, at most 10 s; true once it ends.
fn run(app: &mut Lazy, ctx: &mut repriseapp::Ctx, done: impl Fn(&Explorer) -> bool) -> bool {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if app.event(MAIN, repriseapp::Event::Wake, ctx).quit {
            return true;
        }
        if app.inner.as_ref().is_some_and(&done) {
            return false;
        }
        assert!(std::time::Instant::now() < end, "the job ends");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn run_out(app: &mut Lazy, ctx: &mut repriseapp::Ctx) {
    assert!(run(app, ctx, |_| false), "the process ends with its job");
}

fn yes(app: &mut Lazy, ctx: &mut repriseapp::Ctx) {
    app.event(crate::dialogs::DIALOG, key_event_alt('y'), ctx);
}

fn confirm_text(app: &Lazy) -> Option<String> {
    match app.inner.as_ref()?.dialog() {
        Some(Dialog::Confirm(c)) => Some(c.text.clone()),
        _ => None,
    }
}

#[test]
fn put_copies_moves_and_links_into_its_target_and_ends_with_it() {
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    std::fs::create_dir(&src).expect("mkdir");
    std::fs::create_dir(&dest).expect("mkdir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(src.join(name), name).expect("write");
    }
    let mut ctx = test_ctx();
    for (op, name) in [("copy", "a.txt"), ("move", "b.txt"), ("link", "c.txt")] {
        let from = src.join(name);
        let args = [
            "--put".as_ref(),
            op.as_ref(),
            dest.as_os_str(),
            "--".as_ref(),
            from.as_os_str(),
        ];
        let mut app = started(&args);
        run_out(&mut app, &mut ctx);
    }
    assert_eq!(
        std::fs::read_to_string(dest.join("a.txt")).unwrap(),
        "a.txt"
    );
    assert!(src.join("a.txt").exists(), "a copy leaves its source");
    assert!(dest.join("b.txt").exists() && !src.join("b.txt").exists());
    let links: Vec<_> = (std::fs::read_dir(&dest).unwrap().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| reprisefs::apps::DesktopId::from_file_name(n).is_some())
        .collect();
    assert_eq!(links.len(), 1, "one shortcut: {links:?}");
    assert!(src.join("c.txt").exists(), "a shortcut leaves its source");
}

#[test]
fn a_put_onto_a_taken_name_asks_and_the_process_waits_for_the_answer() {
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    std::fs::create_dir(&src).expect("mkdir");
    std::fs::create_dir(&dest).expect("mkdir");
    std::fs::write(src.join("a.txt"), "new").expect("write");
    std::fs::write(dest.join("a.txt"), "old").expect("write");
    let from = src.join("a.txt");
    let args = [
        "--put".as_ref(),
        "copy".as_ref(),
        dest.as_os_str(),
        "--".as_ref(),
        from.as_os_str(),
    ];
    let mut app = started(&args);
    let mut ctx = test_ctx();
    let ended = run(&mut app, &mut ctx, |ex| asked(ex).is_some());
    assert!(!ended, "the process waits on the replace prompt");
    assert!(
        app.children()
            .iter()
            .any(|c| c.id == crate::dialogs::DIALOG),
        "the prompt is up"
    );
    yes(&mut app, &mut ctx);
    run_out(&mut app, &mut ctx);
    assert_eq!(std::fs::read_to_string(dest.join("a.txt")).unwrap(), "new");
}

#[test]
fn delete_asks_then_trashes_and_erase_asks_then_removes() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let dir = home.path().join("folder");
    std::fs::create_dir(&dir).expect("mkdir");
    let (a, b) = (dir.join("a.txt"), dir.join("b.txt"));
    for f in [&a, &b] {
        std::fs::write(f, "x").expect("write");
    }
    let mut ctx = test_ctx();

    let mut app = started(&["--delete".as_ref(), a.as_os_str()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog().is_some()));
    let text = confirm_text(&app).expect("Confirm File Delete");
    assert!(text.contains("a.txt"), "names the file: {text}");
    assert!(a.exists(), "nothing goes before the answer");
    yes(&mut app, &mut ctx);
    run_out(&mut app, &mut ctx);
    assert!(!a.exists());
    assert!(home.path().join("Trash/files/a.txt").exists(), "trashed");

    let mut app = started(&["--delete".as_ref(), "--erase".as_ref(), b.as_os_str()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog().is_some()));
    yes(&mut app, &mut ctx);
    run_out(&mut app, &mut ctx);
    assert!(!b.exists());
    assert!(
        !home.path().join("Trash/files/b.txt").exists(),
        "erased, not trashed"
    );
}

#[test]
fn a_cancelled_delete_question_ends_the_process_and_keeps_the_file() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let a = home.path().join("a.txt");
    std::fs::write(&a, "x").expect("write");
    let mut ctx = test_ctx();
    let mut app = started(&["--delete".as_ref(), a.as_os_str()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog().is_some()));
    let reply = app.event(crate::dialogs::DIALOG, key_event_escape(), &mut ctx);
    assert!(
        reply.quit || run(&mut app, &mut ctx, |_| false),
        "No ends the process"
    );
    assert!(a.exists());
}

#[test]
fn empty_bin_counts_every_bin_asks_and_empties_them() {
    let _guard = lock(&TRASH_ENV);
    let home = tempfile::tempdir().expect("a temp dir");
    let _data_home = DataHome::set(home.path());
    let mut ctx = test_ctx();
    let mut app = started(&["--empty-bin".as_ref()]);
    run_out(&mut app, &mut ctx);
    assert!(
        app.inner.as_ref().is_some_and(|ex| ex.dialog().is_none()),
        "an empty bin asks nothing"
    );

    let a = home.path().join("Quarterly report.txt");
    std::fs::write(&a, "x").expect("write");
    reprisefs::trash::trash(&a, None).expect("trash");
    let mut app = started(&["--empty-bin".as_ref()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog().is_some()));
    let text = confirm_text(&app).expect("Confirm File Delete");
    assert!(
        text.contains("Quarterly report.txt"),
        "names the item: {text}"
    );
    yes(&mut app, &mut ctx);
    run_out(&mut app, &mut ctx);
    let left = std::fs::read_dir(home.path().join("Trash/files"))
        .unwrap()
        .count();
    assert_eq!(left, 0, "the bin is empty");
}

#[test]
fn zip_of_several_lands_one_archive_named_after_their_folder_past_its_names() {
    let t = tempfile::tempdir().expect("a temp dir");
    let dir = t.path().join("P");
    std::fs::create_dir(&dir).expect("mkdir");
    std::fs::write(dir.join("a.txt"), "a").expect("write");
    std::fs::write(dir.join("P.zip"), "taken").expect("write");
    std::fs::create_dir(dir.join("b")).expect("mkdir");
    let (a, b) = (dir.join("a.txt"), dir.join("b"));
    let mut ctx = test_ctx();
    let mut app = started(&["--zip".as_ref(), a.as_os_str(), b.as_os_str()]);
    run_out(&mut app, &mut ctx);
    assert_eq!(std::fs::read(dir.join("P.zip")).unwrap(), b"taken");
    let zips: Vec<_> = (std::fs::read_dir(&dir).unwrap().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zip") && n != "P.zip")
        .collect();
    assert_eq!(zips, ["P (2).zip"]);
}

#[test]
fn zip_of_items_of_two_folders_makes_no_zip() {
    let t = tempfile::tempdir().expect("a temp dir");
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    for (dir, file) in [(&a, "x"), (&b, "y")] {
        std::fs::create_dir(dir).expect("mkdir");
        std::fs::write(dir.join(file), file).expect("write");
    }
    std::fs::write(a.join("y"), "a's own y").expect("write");
    let (x, y) = (a.join("x"), b.join("y"));
    let mut ctx = test_ctx();
    let mut app = started(&["--zip".as_ref(), x.as_os_str(), y.as_os_str()]);
    run_out(&mut app, &mut ctx);
    let zips = [t.path(), &a, &b].map(|d| {
        (std::fs::read_dir(d).unwrap().flatten())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".zip"))
            .count()
    });
    assert_eq!(zips, [0, 0, 0], "no zip, not one of a's own y");
}

#[test]
fn a_failure_box_up_before_the_progress_dialog_keeps_it_from_coming_up() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    std::fs::create_dir(&src).expect("mkdir");
    std::fs::create_dir(&dest).expect("mkdir");
    std::fs::write(src.join("a.txt"), "a").expect("write");
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let from = src.join("a.txt");
    let args = [
        "--put".as_ref(),
        "move".as_ref(),
        dest.as_os_str(),
        "--".as_ref(),
        from.as_os_str(),
    ];
    let mut app = started(&args);
    let mut ctx = test_ctx();
    assert!(!run(&mut app, &mut ctx, Explorer::holds_a_job));
    std::thread::sleep(reprisekit::controls::copyprogress::SHOW_AFTER);
    app.event(MAIN, repriseapp::Event::Wake, &mut ctx);
    let ids: Vec<_> = app.children().iter().map(|c| c.id).collect();
    assert_eq!(ids, [crate::dialogs::DIALOG], "the failure box alone");
    app.event(crate::dialogs::DIALOG, key_event_escape(), &mut ctx);
    run_out(&mut app, &mut ctx);
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    assert!(from.exists(), "the file stays where it was");
}

#[test]
fn a_failure_box_with_no_folder_window_has_a_task_button_and_asks_to_come_up() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    std::fs::create_dir(&src).expect("mkdir");
    std::fs::create_dir(&dest).expect("mkdir");
    std::fs::write(src.join("a.txt"), "a").expect("write");
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let from = src.join("a.txt");
    let args = [
        "--put".as_ref(),
        "copy".as_ref(),
        dest.as_os_str(),
        "--".as_ref(),
        from.as_os_str(),
    ];
    let mut app = started(&args);
    let mut ctx = test_ctx();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let raised = loop {
        let reply = app.event(MAIN, repriseapp::Event::Wake, &mut ctx);
        if app.inner.as_ref().is_some_and(Explorer::holds_a_job) {
            break reply;
        }
        assert!(std::time::Instant::now() < end, "the copy fails");
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let ex = app.inner.as_ref().expect("the program");
    let win = ex.dialog_win();
    assert_eq!(raised.raise, Some(win), "the box asks to come to the front");
    assert_eq!(
        repriseapp::Extended::caption(&app, win),
        repriseapp::Caption::Fixed,
        "the box keeps a task button"
    );
    app.event(win, key_event_escape(), &mut ctx);
    run_out(&mut app, &mut ctx);
    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).expect("chmod");
}

#[test]
fn undo_with_nothing_to_undo_ends_at_once() {
    let mut ctx = test_ctx();
    let mut app = started(&["--undo".as_ref(), "1:1".as_ref()]);
    run_out(&mut app, &mut ctx);
}

#[test]
fn undo_of_a_named_entry_runs_only_while_it_is_on_top() {
    use reprisefs::undo::{Undoable, file_id};
    let bare = parse_args(std::iter::once(std::ffi::OsString::from("--undo")));
    assert!(bare.alone.is_none(), "--undo names its entry");
    let dir = tempfile::tempdir().expect("a temp dir");
    let [first, second] = ["first", "second"].map(|n| dir.path().join(n));
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let push = |ex: &mut Explorer, p: &std::path::Path| {
        std::fs::create_dir(p).expect("mkdir");
        ex.undos.push(Undoable::New(p.to_path_buf(), file_id(p)));
        ex.undos.top(|id, _| id).expect("on top")
    };
    let below = push(&mut ex, &first);
    let top = push(&mut ex, &second);

    ex.undo_with(Some(below), false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(
        first.exists() && second.exists(),
        "an entry under another is not undone"
    );

    ex.undo_with(Some(top), false, &mut ctx);
    ex.finish_puts(&mut ctx);
    assert!(first.exists() && !second.exists());
}
