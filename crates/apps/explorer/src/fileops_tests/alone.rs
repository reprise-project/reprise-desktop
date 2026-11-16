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
    match &app.inner.as_ref()?.dialog {
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
        .filter(|n| n.ends_with(".desktop"))
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
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog.is_some()));
    let text = confirm_text(&app).expect("Confirm File Delete");
    assert!(text.contains("a.txt"), "names the file: {text}");
    assert!(a.exists(), "nothing goes before the answer");
    yes(&mut app, &mut ctx);
    run_out(&mut app, &mut ctx);
    assert!(!a.exists());
    assert!(home.path().join("Trash/files/a.txt").exists(), "trashed");

    let mut app = started(&["--delete".as_ref(), "--erase".as_ref(), b.as_os_str()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog.is_some()));
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
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog.is_some()));
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
        app.inner.as_ref().is_some_and(|ex| ex.dialog.is_none()),
        "an empty bin asks nothing"
    );

    let a = home.path().join("Quarterly report.txt");
    std::fs::write(&a, "x").expect("write");
    reprisefs::trash::trash(&a, None).expect("trash");
    let mut app = started(&["--empty-bin".as_ref()]);
    assert!(!run(&mut app, &mut ctx, |ex| ex.dialog.is_some()));
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
fn zip_lands_one_archive_named_past_the_folders_names_and_ends() {
    let t = tempfile::tempdir().expect("a temp dir");
    let dir = t.path();
    std::fs::write(dir.join("a.txt"), "a").expect("write");
    std::fs::write(dir.join("a.zip"), "taken").expect("write");
    std::fs::create_dir(dir.join("b")).expect("mkdir");
    let (a, b) = (dir.join("a.txt"), dir.join("b"));
    let mut ctx = test_ctx();
    let mut app = started(&["--zip".as_ref(), a.as_os_str(), b.as_os_str()]);
    run_out(&mut app, &mut ctx);
    assert_eq!(std::fs::read(dir.join("a.zip")).unwrap(), b"taken");
    let zips: Vec<_> = (std::fs::read_dir(dir).unwrap().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zip") && n != "a.zip")
        .collect();
    assert_eq!(zips.len(), 1, "one new zip: {zips:?}");
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
