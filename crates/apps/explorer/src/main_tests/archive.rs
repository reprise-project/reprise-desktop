use super::*;

/// `pack.zip` in a fresh folder, holding `top.txt` and `docs/a.txt`.
fn pack() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::create_dir_all(d.join("src/docs")).expect("mkdir");
    std::fs::write(d.join("src/docs/a.txt"), "inside").expect("write");
    std::fs::write(d.join("src/top.txt"), "top").expect("write");
    let made = std::process::Command::new("bsdtar")
        .current_dir(d.join("src"))
        .args(["-a", "-cf", "../pack.zip", "top.txt", "docs"])
        .status()
        .expect("bsdtar runs");
    assert!(made.success());
    std::fs::remove_dir_all(d.join("src")).expect("rm");
    let zip = d.join("pack.zip");
    (dir, zip)
}

fn names(ex: &Explorer) -> Vec<&str> {
    ex.sv.entries.iter().map(|e| e.name.as_str()).collect()
}

#[test]
fn a_zip_opens_as_a_read_only_folder_that_up_leaves() {
    let (dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.select_paths(std::slice::from_ref(&zip));
    ex.open_selected(&mut ctx);
    assert_eq!(ex.place, Place::Archive(zip.clone(), PathBuf::new()));
    assert!(ex.place.read_only());
    assert_eq!(names(&ex), ["docs", "top.txt"]);

    ex.select_paths(&[zip.join("top.txt")]);
    assert!(
        ex.selected_paths().is_empty(),
        "no entry is a file to copy, cut or delete"
    );
    ex.select_paths(&[zip.join("docs")]);
    ex.open_selected(&mut ctx);
    assert_eq!(ex.place, Place::Archive(zip.clone(), "docs".into()));
    assert_eq!(names(&ex), ["a.txt"]);

    ex.run(Command::Up, &mut ctx);
    assert_eq!(ex.place, Place::Archive(zip.clone(), PathBuf::new()));
    ex.run(Command::Up, &mut ctx);
    assert_eq!(ex.place, Place::Folder(dir.path().to_path_buf()));
    assert_eq!(
        ex.selected_paths(),
        [zip],
        "the archive is selected on the way out"
    );
}

#[test]
fn a_file_in_a_compressed_folder_opens_as_a_read_only_copy() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), "docs".into()), &mut ctx);
    ex.app_index = opener_index();
    ex.select_paths(&[zip.join("docs/a.txt")]);
    let reply = ex.open_selected(&mut ctx);
    let copy = (reply.spawn.iter().flatten())
        .map(PathBuf::from)
        .find(|p| p.file_name() == Some("a.txt".as_ref()))
        .expect("the opener gets the copy");
    assert!(opens(&reply, &copy));
    assert!(copy.starts_with(scratch().join("cache/reprise/archives")));
    assert_eq!(std::fs::read_to_string(&copy).expect("copied"), "inside");
    let mode = std::fs::metadata(&copy).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o400);
}

#[test]
fn a_damaged_archive_says_why_it_does_not_open() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let zip = dir.path().join("broken.zip");
    std::fs::write(&zip, "not a zip").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.go(Place::Archive(zip, PathBuf::new()), &mut ctx);
    assert!(names(&ex).is_empty());
    assert!(
        matches!(ex.dialog(), Some(crate::dialogs::Dialog::Message { text, .. })
            if text.starts_with("Cannot open broken.zip: ")),
        "an error box names the archive"
    );
}

#[test]
fn extract_all_picks_a_folder_and_shows_the_archives_folder_made_in_it() {
    let (dir, zip) = pack();
    let to = dir.path().join("to");
    std::fs::create_dir(&to).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), "docs".into()), &mut ctx);
    ex.run(Command::ExtractAll, &mut ctx);
    assert!(
        matches!(ex.dialog(), Some(crate::dialogs::Dialog::Browse(b))
            if b.pick == crate::browse::Pick::Extract && b.items == [zip.clone()]),
        "the picker asks where the whole archive goes"
    );
    ex.extract_into(vec![zip], to.clone(), &ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    assert_eq!(
        std::fs::read_to_string(to.join("pack/docs/a.txt")).expect("extracted"),
        "inside"
    );
    assert_eq!(ex.place, Place::Folder(to.join("pack")));
}

#[test]
fn a_program_in_a_compressed_folder_is_not_opened_and_says_to_extract_first() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::create_dir(d.join("src")).expect("mkdir");
    std::fs::write(d.join("src/setup.sh"), "#!/bin/sh\ntouch ran\n").expect("write");
    let made = std::process::Command::new("bsdtar")
        .current_dir(d.join("src"))
        .args(["-a", "-cf", "../tools.zip", "setup.sh"])
        .status()
        .expect("bsdtar runs");
    assert!(made.success());
    let zip = d.join("tools.zip");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.app_index = opener_index();
    ex.select_paths(&[zip.join("setup.sh")]);
    let reply = ex.open_selected(&mut ctx);
    assert!(reply.spawn.is_empty(), "nothing starts");
    assert!(
        matches!(ex.dialog(), Some(crate::dialogs::Dialog::Message { text, .. })
            if text.starts_with("setup.sh is a program")),
        "a box says to extract it first"
    );
}

#[test]
fn a_typed_address_inside_an_archive_opens_that_folder() {
    let (dir, zip) = pack();
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    let mut ex = test_explorer(here.clone(), &mut ctx);
    ex.navigate(
        &format!("{}/docs/", zip.display()),
        |d| d.is_dir(),
        &mut ctx,
    );
    wake_until(&mut ex, &mut ctx, |ex| ex.place != here);
    assert_eq!(ex.place, Place::Archive(zip.clone(), "docs".into()));
    assert_eq!(names(&ex), ["a.txt"]);
    ex.navigate(&zip.display().to_string(), |d| d.is_dir(), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.place.title() == "pack.zip");
    assert_eq!(ex.place, Place::Archive(zip, PathBuf::new()));
}

#[test]
fn a_typed_address_of_no_folder_in_an_archive_cannot_be_found() {
    let (dir, zip) = pack();
    let mut ctx = test_ctx();
    let here = Place::Folder(dir.path().to_path_buf());
    for missing in ["nope", "top.txt", "docs/a.txt/deeper"] {
        let mut ex = test_explorer(here.clone(), &mut ctx);
        let typed = format!("{}/{missing}", zip.display());
        ex.navigate(&typed, |d| d.is_dir(), &mut ctx);
        wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
        assert!(
            matches!(ex.dialog(), Some(crate::dialogs::Dialog::Message { text, .. })
                if *text == format!("Cannot find '{typed}'. Check the spelling and try again.")),
            "{missing}"
        );
        assert_eq!(ex.place, here, "{missing}");
    }
}

#[test]
fn a_start_address_inside_an_archive_opens_that_folder() {
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let start = parse_args([zip.join("docs").into_os_string()]);
    let mut ex = Explorer::open(start, |d| d.is_dir(), test_explorer, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        matches!(ex.place, Place::Archive(..))
    });
    assert_eq!(ex.place, Place::Archive(zip, "docs".into()));
    assert_eq!(names(&ex), ["a.txt"]);
    assert!(!ex.history.can_back());
}

#[test]
fn a_start_address_of_no_folder_in_an_archive_opens_home_and_says_it_cannot_be_found() {
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let missing = zip.join("nope");
    let start = parse_args([missing.clone().into_os_string()]);
    let mut ex = Explorer::open(start, |d| d.is_dir(), test_explorer, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    let text = format!(
        "Cannot find '{}'. Check the spelling and try again.",
        missing.display()
    );
    assert!(
        matches!(ex.dialog(), Some(Dialog::Message { text: t, .. }) if *t == text),
        "a box says {text}"
    );
    assert_eq!(ex.place, Place::Home);
}

/// Polls the archive jobs to the end of the queue, keeping every reply.
fn finish_copies(ex: &mut Explorer, ctx: &mut Ctx) -> Reply {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut got = Reply::none();
    while ex.extracting.is_some() {
        assert!(std::time::Instant::now() < end, "the copy ends");
        std::thread::sleep(std::time::Duration::from_millis(10));
        if let Some(r) = ex.poll_extract(ctx) {
            got = got.and(r);
        }
    }
    got
}

#[test]
fn copy_in_a_compressed_folder_puts_copies_of_the_picked_entries_on_the_clipboard() {
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.select_paths(&[zip.join("docs"), zip.join("top.txt")]);
    let cut = ex.run(Command::Cut, &mut ctx);
    assert!(
        cut.copy_files.is_none(),
        "nothing is cut from a read-only place"
    );
    assert!(ex.extracting.is_none());
    ex.run(Command::Copy, &mut ctx);
    let clip = finish_copies(&mut ex, &mut ctx)
        .copy_files
        .expect("the copies go on the clipboard");
    assert!(!clip.cut);
    let [docs, top] = &clip.paths[..] else {
        panic!("one copy per picked entry: {:?}", clip.paths);
    };
    assert!(docs.starts_with(scratch().join("cache/reprise/archives")));
    assert_eq!(
        (docs.file_name(), top.file_name()),
        (Some("docs".as_ref()), Some("top.txt".as_ref()))
    );
    assert_eq!(
        std::fs::read_to_string(docs.join("a.txt")).expect("copied"),
        "inside"
    );
    assert_eq!(std::fs::read_to_string(top).expect("copied"), "top");
}

#[test]
fn copy_to_folder_out_of_a_compressed_folder_asks_before_replacing_as_a_copy_does() {
    let (dir, zip) = pack();
    let to = dir.path().join("to");
    std::fs::create_dir(&to).expect("mkdir");
    std::fs::write(to.join("top.txt"), "old").expect("write");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    // The temp folders' names start with a dot.
    ex.folders.show_hidden = true;
    ex.select_paths(&[zip.join("docs"), zip.join("top.txt")]);
    ex.open_browse(crate::browse::Pick::Copy, &mut ctx);
    let mounts = ex.drive_mounts.clone();
    let Some(Dialog::Browse(b)) = ex.dialogs.up_mut() else {
        panic!("the picker is up");
    };
    assert_eq!(b.items, [zip.join("docs"), zip.join("top.txt")]);
    let path = foldertree::path_of(&b.bf.tree, &b.places, &Place::Folder(to.clone()), &mounts);
    assert!(path.is_some(), "the picker walked to the archive's folder");
    b.bf.tree.selected = path;
    let (accept, new_folder) = b.judge().expect("a new pick")();
    b.bf.set_enabled(accept, new_folder);
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), &mut ctx);
    assert!(ex.extracting.is_some(), "the entries copy out first");

    finish_copies(&mut ex, &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    assert!(
        matches!(ex.dialog(), Some(Dialog::Replace(_))),
        "top.txt asks before it replaces what is there"
    );
    assert_eq!(
        std::fs::read_to_string(to.join("top.txt")).expect("kept"),
        "old"
    );
}

/// The scratch folder the copy out running fills.
fn copying_into(ex: &Explorer) -> PathBuf {
    let ex = ex.extracting.as_ref().expect("the copy runs");
    let scratch = ex.scratch.as_ref().expect("a scratch folder");
    scratch.path().to_path_buf()
}

#[test]
fn a_cancelled_copy_out_of_a_compressed_folder_puts_nothing_on_the_clipboard() {
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.select_paths(&[zip.join("docs"), zip.join("top.txt")]);
    ex.run(Command::Copy, &mut ctx);
    let scratch = copying_into(&ex);
    ex.extracting.as_ref().unwrap().job.meter().cancel();
    let got = finish_copies(&mut ex, &mut ctx);
    assert!(got.copy_files.is_none(), "nothing goes on the clipboard");
    wake_until(&mut ex, &mut ctx, |_| !scratch.exists());
}

#[test]
fn a_second_copy_out_onto_the_clipboard_lets_go_of_the_first_ones_copies() {
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.select_paths(&[zip.join("top.txt")]);
    ex.run(Command::Copy, &mut ctx);
    let first = copying_into(&ex);
    finish_copies(&mut ex, &mut ctx);
    assert!(first.join("top.txt").exists());
    ex.run(Command::Copy, &mut ctx);
    finish_copies(&mut ex, &mut ctx);
    assert!(!first.exists(), "nothing else held the first copies");
    let second = ex.clip_scratch.0.as_ref().expect("the clipboard's copies");
    assert!(second.path().join("top.txt").exists());
}

#[test]
fn a_copy_onto_the_clipboard_during_a_paste_of_the_last_ones_leaves_the_paste_its_copies() {
    let (dir, zip) = pack();
    let to = dir.path().join("to");
    std::fs::create_dir(&to).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.select_paths(&[zip.join("docs")]);
    ex.run(Command::Copy, &mut ctx);
    let first = copying_into(&ex);
    let clip = finish_copies(&mut ex, &mut ctx)
        .copy_files
        .expect("the copies go on the clipboard");
    ex.put_all(&clip.paths, &to, &|_| Some(DropOp::Copy), true, &mut ctx);
    ex.select_paths(&[zip.join("top.txt")]);
    ex.run(Command::Copy, &mut ctx);
    finish_copies(&mut ex, &mut ctx);
    assert!(
        first.join("docs/a.txt").exists(),
        "the paste still holds what it reads"
    );
    wake_until(&mut ex, &mut ctx, |ex| ex.puts_idle() && !first.exists());
    assert_eq!(
        std::fs::read_to_string(to.join("docs/a.txt")).expect("pasted"),
        "inside"
    );
}

#[test]
fn copy_to_folder_out_of_a_compressed_folder_lets_go_of_its_copies_once_put() {
    let (dir, zip) = pack();
    let to = dir.path().join("to");
    std::fs::create_dir(&to).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    let picks = ex.archive_picks(&[zip.join("docs")]).expect("picked");
    ex.copy_out(picks, crate::extracting::Then::Into(to.clone()), &mut ctx);
    let scratch = copying_into(&ex);
    finish_copies(&mut ex, &mut ctx);
    assert!(scratch.exists(), "the put holds the copies");
    wake_until(&mut ex, &mut ctx, |_| {
        to.join("docs/a.txt").exists() && !scratch.exists()
    });
    assert_eq!(
        std::fs::read_to_string(to.join("docs/a.txt")).expect("put"),
        "inside"
    );
}

#[test]
fn extract_in_a_compressed_folder_puts_the_selection_beside_the_zip_with_its_dates() {
    use std::os::unix::fs::MetadataExt;
    let (dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::from("docs")), &mut ctx);
    assert_eq!(ex.extract_folder().as_deref(), Some(dir.path()));
    ex.select_paths(&[zip.join("docs/a.txt")]);
    ex.run(Command::ExtractEntries, &mut ctx);
    finish_copies(&mut ex, &mut ctx);
    let out = dir.path().join("a.txt");
    wake_until(&mut ex, &mut ctx, |ex| out.exists() && ex.puts_idle());
    assert_eq!(std::fs::read_to_string(&out).expect("put"), "inside");
    use std::os::unix::fs::PermissionsExt;
    let shut = std::fs::Permissions::from_mode(0o555);
    std::fs::set_permissions(dir.path(), shut).expect("chmod");
    // Root writes past the mode.
    if !reprisefs::ops::can_write(dir.path()) {
        let shut = test_explorer(Place::Archive(zip.clone(), PathBuf::from("docs")), &mut ctx);
        assert_eq!(shut.extract_folder(), None, "a folder that takes no writes");
    }
    let open = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(dir.path(), open).expect("chmod");
    let listed = ex.archives.kept(&zip).expect("listed");
    let entry = listed.iter().find(|e| e.path == Path::new("docs/a.txt"));
    assert_eq!(
        std::fs::metadata(&out).expect("put").mtime(),
        entry.and_then(|e| e.modified).expect("a date"),
        "the file keeps the entry's date"
    );
}

#[test]
fn closing_explorer_hands_the_copies_on_its_clipboard_to_the_day_old_sweep() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, zip) = pack();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    ex.select_paths(&[zip.join("top.txt")]);
    ex.run(Command::Copy, &mut ctx);
    let copies = copying_into(&ex);
    finish_copies(&mut ex, &mut ctx);
    drop(ex);
    assert!(
        copies.join("top.txt").exists(),
        "the clipboard history may still paste them"
    );
    let mode = std::fs::metadata(&copies)
        .expect("kept")
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o1700, "marked handed over");
}

/// `locked.zip` in a fresh folder, holding `a.txt` encrypted with the password `pw`.
fn locked() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let d = dir.path();
    std::fs::write(d.join("a.txt"), "secret").expect("write");
    let made = std::process::Command::new("bsdtar")
        .current_dir(d)
        .args(["--options", "zip:encryption=zipcrypt", "--passphrase", "pw"])
        .args(["-a", "-cf", "locked.zip", "a.txt"])
        .status()
        .expect("bsdtar runs");
    assert!(made.success());
    std::fs::remove_file(d.join("a.txt")).expect("rm");
    let zip = d.join("locked.zip");
    (dir, zip)
}

/// Types `typed` into the password box up and presses Enter.
fn give_password(ex: &mut Explorer, typed: &str, ctx: &mut Ctx) {
    for ch in typed.chars() {
        let utf8 = ch.to_string();
        let edit = reprisekit::textedit::key_for(ch as u32, false, false, Some(&utf8));
        let key = repriseapp::KeyInput {
            sym: ch as u32,
            utf8: Some(utf8),
            mods: repriseapp::Mods::default(),
            edit,
        };
        ex.dialog_event(Event::Key(key), ctx);
    }
    ex.dialog_event(Event::Key(key_in(keysym::RETURN)), ctx);
}

/// The status line of the password box up, and what it opens.
fn password_box(ex: &Explorer) -> Option<(String, &crate::zipkey::Unlocks)> {
    let Some(Dialog::Unlock(u, unlocks)) = ex.dialog() else {
        return None;
    };
    let status = match u.form.item("status").map(|i| &i.control) {
        Some(reprisekit::controls::form::Control::Label { text, .. }) => text.clone(),
        _ => String::new(),
    };
    Some((status, unlocks))
}

#[test]
fn an_encrypted_zip_asks_its_password_until_one_opens_it_then_not_again_in_the_window() {
    use crate::zipkey::Unlocks;
    let (dir, zip) = locked();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.event(crate::MAIN, Event::Focus(true), &mut ctx);
    ex.go(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    assert!(names(&ex).is_empty());
    assert!(
        matches!(password_box(&ex), Some((s, Unlocks::Open(z))) if s.is_empty() && *z == zip),
        "the password box is up"
    );
    give_password(&mut ex, "nope", &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        password_box(ex).is_some_and(|(s, _)| s == "The password is not correct.")
    });
    give_password(&mut ex, "pw", &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| names(ex) == ["a.txt"]);
    assert!(ex.dialog().is_none(), "the box went");

    ex.app_index = opener_index();
    ex.select_paths(&[zip.join("a.txt")]);
    let reply = ex.open_selected(&mut ctx);
    let copy = (reply.spawn.iter().flatten())
        .map(PathBuf::from)
        .find(|p| p.file_name() == Some("a.txt".as_ref()))
        .expect("the opener gets the copy");
    assert_eq!(std::fs::read_to_string(&copy).expect("copied"), "secret");

    ex.run(Command::Up, &mut ctx);
    ex.go(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    assert_eq!(names(&ex), ["a.txt"]);
    assert!(ex.dialog().is_none(), "the window asks once");
}

#[test]
fn the_password_box_waits_for_the_window_to_first_hold_the_keyboard() {
    let (dir, zip) = locked();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.go(Place::Archive(zip, PathBuf::new()), &mut ctx);
    assert!(
        ex.dialog().is_none(),
        "no box while another window has the keyboard"
    );
    ex.event(crate::MAIN, Event::Focus(true), &mut ctx);
    assert!(
        password_box(&ex).is_some(),
        "the box comes up with the keyboard"
    );
}

#[test]
fn a_cancel_during_the_check_lets_its_answer_go() {
    let (dir, zip) = locked();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.event(crate::MAIN, Event::Focus(true), &mut ctx);
    ex.go(Place::Archive(zip.clone(), PathBuf::new()), &mut ctx);
    give_password(&mut ex, "pw", &mut ctx);
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| ex.jobs_pending == 0);
    assert!(names(&ex).is_empty(), "the zip stays unlisted");
    assert!(ex.archives.wants_key(&zip), "the window keeps no password");
}

#[test]
fn a_cancelled_password_box_says_the_zip_is_password_protected() {
    let (dir, zip) = locked();
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.event(crate::MAIN, Event::Focus(true), &mut ctx);
    ex.go(Place::Archive(zip, PathBuf::new()), &mut ctx);
    assert!(password_box(&ex).is_some());
    ex.dialog_event(Event::Key(key_in(keysym::ESCAPE)), &mut ctx);
    assert!(
        matches!(ex.dialog(), Some(Dialog::Message { text, .. })
            if text == "Cannot open locked.zip: it is password-protected."),
        "the box says the zip is password-protected"
    );
}

#[test]
fn extract_all_of_an_encrypted_zip_asks_its_password_until_one_opens_it() {
    use crate::zipkey::Unlocks;
    let (dir, zip) = locked();
    let to = dir.path().join("to");
    std::fs::create_dir(&to).expect("mkdir");
    let mut ctx = test_ctx();
    let mut ex = test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);
    ex.extract_into(vec![zip], to.clone(), &ctx);
    let asked = |ex: &Explorer, status: &str| matches!(password_box(ex), Some((s, Unlocks::Extract(_))) if s == status);
    wake_until(&mut ex, &mut ctx, |ex| asked(ex, ""));
    give_password(&mut ex, "nope", &mut ctx);
    wake_until(&mut ex, &mut ctx, |ex| {
        asked(ex, "The password is not correct.")
    });
    give_password(&mut ex, "pw", &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    assert_eq!(
        std::fs::read_to_string(to.join("locked/a.txt")).expect("extracted"),
        "secret"
    );
}
