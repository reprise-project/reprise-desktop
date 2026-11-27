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
