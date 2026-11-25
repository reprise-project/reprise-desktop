use super::*;

/// Opening an executable script runs it in its own folder once the token it asks for is in,
/// handed that token; a +x picture and a script without the execute bit open in their programs,
/// and a script whose interpreter is missing gets a box.
#[test]
fn opening_an_executable_script_runs_it_in_its_folder_with_its_token() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let put = |name: &str, bytes: &[u8], mode: u32| {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).expect("write");
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).expect("chmod");
        p
    };
    put(
        "a.sh",
        b"#!/bin/sh\nprintf %s \"$XDG_ACTIVATION_TOKEN\" >part && mv part ran\n",
        0o755,
    );
    put("b.sh", b"#!/no/such/shell\n", 0o755);
    let photo = put("c.png", b"\x89PNG\r\n\x1a\n", 0o755);
    let plain = put("d.sh", b"#!/bin/sh\ntouch ran\n", 0o644);
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = opener_index();
    let names: Vec<&str> = ex.sv.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["a.sh", "b.sh", "c.png", "d.sh"]);
    let open_as = |i: usize, how, ex: &mut Explorer, ctx: &mut Ctx| {
        ex.sv.selection.set_only(Some(i));
        ex.open_selected_as(how, ctx)
    };
    let marker = dir.path().join("ran");
    let ran = || {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < end,
                "the script ran in its folder"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let token = std::fs::read_to_string(&marker).unwrap();
        std::fs::remove_file(&marker).unwrap();
        token
    };

    let reply = open_as(0, ScriptOpen::Run, &mut ex, &mut ctx);
    assert!(reply.spawn.is_empty(), "run, not opened");
    assert_eq!(reply.handoffs, 1, "a token is asked for");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!marker.exists(), "nothing runs before the token");
    hand(&mut ex, Some("run"), &mut ctx);
    assert_eq!(ran(), "run");
    assert!(ex.dialog().is_none());
    let open =
        |i: usize, ex: &mut Explorer, ctx: &mut Ctx| open_as(i, ScriptOpen::Terminal, ex, ctx);
    assert!(
        open(0, &mut ex, &mut ctx).spawn.is_empty(),
        "run, not opened"
    );
    hand(&mut ex, Some("in a terminal"), &mut ctx);
    assert_eq!(ran(), "in a terminal");
    assert!(ex.dialog().is_none());
    let state = super::scratch();
    let term_argv = std::fs::read_to_string(state.join("term-argv")).expect("the terminal ran");
    let a_sh = dir.path().join("a.sh");
    let term_argv: Vec<&str> = term_argv.lines().collect();
    assert_eq!(
        term_argv[..3],
        ["-e", "sh", "-c"],
        "the config's terminal keeps it open"
    );
    assert_eq!(
        term_argv.last(),
        a_sh.to_str().as_ref(),
        "the script is the last argument"
    );

    open(1, &mut ex, &mut ctx);
    hand(&mut ex, Some("b"), &mut ctx);
    match ex.take_up().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text, .. } => {
            assert_eq!(title, "b.sh");
            assert_eq!(
                text,
                "Cannot run b.sh: The program that runs it was not found."
            );
        }
        _ => panic!("a failed run raises a message box"),
    }

    for (i, path) in [(2, photo), (3, plain)] {
        let reply = open(i, &mut ex, &mut ctx);
        assert_eq!(reply.spawn.len(), 1);
        assert_eq!(
            reply.spawn[0].last(),
            Some(&path.into_os_string()),
            "opened"
        );
        assert_eq!(reply.handoffs, 0, "the host asks for the opener's token");
    }
}

/// The script question's buttons hold their labels inside the focus ring under every interface
/// and text size.
#[test]
fn the_script_question_fits_its_button_labels() {
    use reprisekit::controls::dlgtemplate::{self, BUTTON_IDS};
    const FOCUS_INSET: i32 = 4;
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(std::env::temp_dir()), &mut ctx);
    ex.ask_run(
        std::path::Path::new("/tmp/a.sh"),
        ScriptOpen::Run,
        reprisefs::program::Asked::Script,
        &mut ctx,
    );
    let Some(crate::dialogs::Dialog::Confirm(c)) = ex.dialog() else {
        panic!("the script question is up");
    };
    let over = reprisekit::interfaces::overflows(|_, theme, text| {
        let px = theme.fonts.size_px;
        let lay = c.layout(text, px, theme.text);
        let form = dlgtemplate::form(&lay, None, &c.buttons(), 0);
        form.drawn_with(&theme.paint());
        c.buttons()
            .iter()
            .zip(BUTTON_IDS)
            .filter_map(|(label, id)| {
                let ink = text.ink_width(&reprisekit::text::split_mnemonic(label).0, false, px);
                let by = ink + 2 * FOCUS_INSET - form.rect(id)?.width;
                (by > 0).then(|| ("script question".to_string(), id, by))
            })
            .collect()
    });
    assert!(over.is_empty(), "{over:#?}");
}

/// Opening an executable script asks Run / Open in Notepad / Cancel before anything runs; Run
/// runs it and is remembered, Open in Notepad opens it there, and a binary never asks.
#[test]
fn opening_an_executable_script_asks_before_it_runs() {
    use reprisekit::controls::{dlgtemplate::BUTTON_IDS, form::Event};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let script = dir.path().join("a.sh");
    std::fs::write(&script, b"#!/bin/sh\ntouch ran\n").expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    let marker = dir.path().join("ran");
    let asked = |ex: &crate::Explorer| match ex.dialog() {
        Some(crate::dialogs::Dialog::Confirm(c)) => {
            assert_eq!(c.title, "Open File");
            assert_eq!(c.buttons(), ["&Run", "Open in &Notepad", "Cancel"]);
            c.script.is_some()
        }
        _ => false,
    };

    ex.sv.selection.set_only(Some(0));
    let reply = ex.open_selected(&mut ctx);
    assert!(reply.spawn.is_empty(), "nothing spawns before the answer");
    assert!(asked(&ex), "the question is up");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(!marker.exists(), "the script did not run");

    let reply = ex.dialog_outcome(Event::Pressed(BUTTON_IDS[1]), &mut ctx);
    assert!(ex.dialog().is_none());
    assert_eq!(
        reply.spawn,
        [vec![
            std::ffi::OsString::from("reprise-notepad"),
            script.clone().into_os_string()
        ]],
        "Open in Notepad"
    );
    assert!(!marker.exists());

    ex.open_selected(&mut ctx);
    assert!(asked(&ex), "Notepad is not remembered");
    ex.dialog_outcome(Event::Pressed(BUTTON_IDS[2]), &mut ctx);
    assert!(ex.dialog().is_none(), "Cancel closes it");

    ex.open_selected(&mut ctx);
    let reply = ex.dialog_outcome(Event::Default, &mut ctx);
    assert!(reply.spawn.is_empty(), "Run runs it, not an opener");
    assert_eq!(reply.handoffs, 1, "Run asks for the token");
    hand(&mut ex, Some("run"), &mut ctx);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !marker.exists() {
        assert!(std::time::Instant::now() < end, "Run ran the script");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::fs::remove_file(&marker).unwrap();

    if reprisefs::apps::is_trusted(&script) {
        ex.open_selected(&mut ctx);
        assert!(ex.dialog().is_none(), "Run is remembered for the file");
    }
}

/// The question up in `ex`, as its buttons.
fn question(ex: &crate::Explorer) -> Option<Vec<String>> {
    match ex.dialog() {
        Some(crate::dialogs::Dialog::Confirm(c)) if c.script.is_some() => {
            assert_eq!(c.title, "Open File");
            Some(c.buttons())
        }
        _ => None,
    }
}

/// Opening an executable ELF file asks Run / Cancel, whatever its name; Run runs it.
#[test]
fn opening_an_elf_file_asks_run_or_cancel() {
    use reprisekit::controls::{dlgtemplate::BUTTON_IDS, form::Event};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let elf = dir.path().join("invoice.pdf");
    std::fs::write(&elf, b"\x7fELF\x02\x01\x01\0").expect("write");
    std::fs::set_permissions(&elf, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let reply = ex.open_selected(&mut ctx);
    assert!(reply.spawn.is_empty(), "nothing spawns before the answer");
    assert_eq!(
        question(&ex).expect("the question is up"),
        ["&Run", "Cancel"]
    );
    ex.dialog_outcome(Event::Default, &mut ctx);
    assert!(ex.dialog().is_none(), "Cancel, the default, closes it");

    ex.open_selected(&mut ctx);
    ex.dialog_outcome(Event::Pressed(BUTTON_IDS[0]), &mut ctx);
    hand(&mut ex, Some("run"), &mut ctx);
    match ex.take_up() {
        Some(crate::dialogs::Dialog::Message { text, .. }) => assert_eq!(
            text,
            "Cannot run invoice.pdf: It is not a program this computer can run."
        ),
        _ => panic!("Run tried to run it"),
    }
}

/// Opening a type its program runs (a `.exe` under Wine), or a file with no type whose first
/// bytes are `MZ`, asks Run / Cancel before the program opens it, with or without the execute bit.
#[test]
fn opening_a_runnable_type_asks_before_its_program_opens_it() {
    use reprisekit::controls::{dlgtemplate::BUTTON_IDS, form::Event};
    for name in ["setup.exe", "invoice"] {
        let dir = tempfile::tempdir().expect("a temp dir");
        let exe = dir.path().join(name);
        std::fs::write(&exe, b"MZ\x90\0").expect("write");
        let mut ctx = test_ctx();
        let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
        ex.globs = Arc::new(reprisefs::apps::MimeDb::parse([
            "50:application/x-msdownload:*.exe\n",
        ]));
        ex.app_index = opener_index();
        ex.sv.selection.set_only(Some(0));

        let reply = ex.open_selected(&mut ctx);
        assert!(
            reply.spawn.is_empty(),
            "{name}: nothing opens before the answer"
        );
        assert_eq!(
            question(&ex).expect("the question is up"),
            ["&Run", "Cancel"]
        );
        let reply = ex.dialog_outcome(Event::Default, &mut ctx);
        assert!(
            reply.spawn.is_empty() && ex.dialog().is_none(),
            "Cancel, the default, opens nothing"
        );

        ex.open_selected(&mut ctx);
        let reply = ex.dialog_outcome(Event::Pressed(BUTTON_IDS[0]), &mut ctx);
        if name == "invoice" {
            assert!(reply.spawn.is_empty(), "a file with no type has no program");
            assert!(
                matches!(ex.dialog(), Some(crate::dialogs::Dialog::ChooseProgram(_))),
                "Run asks which program opens it"
            );
        } else {
            assert!(
                opens(&reply, &exe),
                "Run opens it in its program: {reply:?}"
            );
        }
    }
}

/// A file whose type no program opens puts up Open With for it, with Always use unticked.
#[test]
fn opening_a_file_no_program_takes_asks_open_with() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let data = dir.path().join("data.qqz");
    std::fs::write(&data, [0u8, 0xff, 7, 0x80]).expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = opener_index();
    ex.sv.selection.set_only(Some(0));

    let reply = ex.open_selected(&mut ctx);
    assert!(reply.spawn.is_empty(), "nothing runs: {reply:?}");
    match ex.dialog() {
        Some(crate::dialogs::Dialog::ChooseProgram(c)) => {
            assert_eq!(c.path, data);
            assert!(!c.ow.always.on, "Always use starts unticked");
        }
        _ => panic!("Open With is up"),
    }
}

/// Opening a program file reads it and runs it on a listing thread: on a dead mount the window
/// goes on at once, a late read asks for the token on its wake, and a run that fails raises its
/// box on the wake after the answer.
#[test]
fn a_program_that_answers_late_raises_its_box_on_a_later_wake() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("b.sh");
    std::fs::write(
        &p,
        b"#!/no/such/shell
",
    )
    .expect("write");
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.sv.selection.set_only(Some(0));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(
        ex.open_selected_as(ScriptOpen::Run, &mut ctx),
        Reply::none()
    );
    drop(closed);
    ex.held = None;
    let end = std::time::Instant::now() + LIST_WAIT;
    while ex.wake(&mut ctx).handoffs == 0 {
        assert!(
            std::time::Instant::now() < end,
            "the late read asks for the token"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(hand(&mut ex, Some("late"), &mut ctx), Reply::none());
    assert!(ex.dialog().is_none(), "no box before the answer");
    drop(closed);
    ex.held = None;
    wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    match ex.take_up().expect("the error box is up") {
        crate::dialogs::Dialog::Message { title, text, .. } => {
            assert_eq!(title, "b.sh");
            assert_eq!(
                text,
                "Cannot run b.sh: The program that runs it was not found."
            );
        }
        _ => panic!("a failed run raises a message box"),
    }
}

#[test]
fn a_file_opened_late_still_opens_when_the_window_closes_first() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("a.txt");
    std::fs::write(&p, "text").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.app_index = opener_index();
    ex.sv.selection.set_only(Some(0));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.open_selected(&mut ctx), Reply::none());
    assert!(!ex.close(&mut ctx).quit, "the window waits for the opening");

    drop(closed);
    ex.held = None;
    let mut spawned = false;
    for _ in 0..500 {
        let r = ex.wake(&mut ctx);
        spawned |= opens(&r, &p);
        if r.quit {
            assert!(spawned, "the file opens before the window quits");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the window quits once the opening answers");
}

#[test]
fn a_file_opened_late_opens_when_another_answer_lands_in_the_same_wake() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let p = dir.path().join("a.txt");
    std::fs::write(&p, "text").expect("write");
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).expect("mkdir");
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.app_index = opener_index();
    let at = ex
        .items
        .iter()
        .position(|it| it.path == p)
        .expect("a.txt listed");
    ex.sv.selection.set_only(Some(at));

    let gate = Arc::new(std::sync::RwLock::new(()));
    let closed = gate.write().expect("lock");
    ex.held = Some(gate.clone());
    assert_eq!(ex.open_selected(&mut ctx), Reply::none());
    ex.held = None;
    let typed = gate.clone();
    let reply = ex.navigate(
        &sub.display().to_string(),
        move |d| {
            drop(typed.read());
            d.is_dir()
        },
        &mut ctx,
    );
    assert_eq!(reply, Reply::none());

    drop(closed);
    // Both answers land before the wake that takes them.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let mut spawned = false;
    for _ in 0..500 {
        spawned |= opens(&ex.wake(&mut ctx), &p);
        if ex.place != here && spawned {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(ex.place, Place::Folder(sub));
    assert!(spawned, "the file opens in its program");
}

/// Opening a disk image asks before it is mounted; the menu's Mount row is the answer itself.
#[test]
fn opening_a_disk_image_asks_before_mounting_it() {
    use reprisekit::controls::{dlgtemplate::BUTTON_IDS, form::Event};
    let dir = tempfile::tempdir().expect("a temp dir");
    std::fs::write(dir.path().join("stick.img"), b"image").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.items[0].mime = Some("application/x-raw-disk-image".into());
    ex.sv.selection.set_only(Some(0));
    let mounting = |ex: &crate::Explorer| match ex.dialog() {
        Some(crate::dialogs::Dialog::Message { title, .. }) => title == "Could not mount stick.img",
        _ => false,
    };

    ex.open_selected(&mut ctx);
    match ex.dialog() {
        Some(crate::dialogs::Dialog::Confirm(c)) => {
            assert_eq!(c.title, "Mount Disk Image");
            assert_eq!(c.buttons(), ["&Mount", "Cancel"]);
        }
        _ => panic!("the question is up before any mount"),
    }
    ex.dialog_outcome(Event::Default, &mut ctx);
    assert!(ex.dialog().is_none(), "Cancel, the default, mounts nothing");

    ex.open_selected(&mut ctx);
    ex.dialog_outcome(Event::Pressed(BUTTON_IDS[0]), &mut ctx);
    assert!(mounting(&ex), "Mount mounts it (here with no UDisks2)");
    ex.take_up();

    ex.run(crate::commands::Command::MountImage, &mut ctx);
    assert!(mounting(&ex), "the Mount row mounts unasked");
}
