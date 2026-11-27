use super::*;

/// A folder holding `a.zip` (made by bsdtar from `in_zip`, name and contents) and the files
/// `outside` beside it, with Explorer showing it.
fn zip_beside(
    in_zip: &[(&str, &str)],
    outside: &[(&str, &str)],
    ctx: &mut repriseapp::Ctx,
) -> (tempfile::TempDir, PathBuf, Explorer) {
    let t = tempfile::tempdir().expect("a temp dir");
    let (src, dir) = (t.path().join("src"), t.path().join("dir"));
    std::fs::create_dir_all(&src).expect("mkdir");
    std::fs::create_dir_all(&dir).expect("mkdir");
    for (name, body) in in_zip {
        let path = src.join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }
    let zip = dir.join("a.zip");
    let made = std::process::Command::new("bsdtar")
        .args(["-c", "--format", "zip", "-f"])
        .arg(&zip)
        .arg("-C")
        .arg(&src)
        .args(in_zip.iter().map(|(n, _)| n))
        .status()
        .expect("bsdtar runs");
    assert!(made.success());
    for (name, body) in outside {
        std::fs::write(dir.join(name), body).expect("write");
    }
    let ex = crate::tests::test_explorer(Place::Folder(dir), ctx);
    (t, zip, ex)
}

/// The zip's entries and their contents, by name.
fn zip_holds(zip: &Path) -> Vec<(String, String)> {
    let entries = reprisefs::archive::list(zip, &Default::default()).expect("the zip lists");
    let mut held: Vec<(String, String)> = (entries.iter())
        .map(|e| {
            let argv = reprisefs::archive::open_argv(zip, &e.path);
            let out = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .output();
            let body = String::from_utf8_lossy(&out.expect("bsdtar runs").stdout).into_owned();
            (e.path.display().to_string(), body)
        })
        .collect();
    held.sort();
    held
}

#[test]
fn new_compressed_folder_writes_an_empty_zip_and_starts_a_rename() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut ctx = test_ctx();
    let mut ex = crate::tests::test_explorer(Place::Folder(dir.path().to_path_buf()), &mut ctx);

    ex.run(Command::NewCompressedFolder, &mut ctx);

    let made = dir.path().join("New Compressed (zipped) Folder.zip");
    assert_eq!(
        std::fs::read(&made).expect("read"),
        reprisefs::zipwrite::EMPTY_ZIP
    );
    let i = ex.index_of(&made).expect("the zip is listed");
    assert_eq!(ex.sv.edit.as_ref().map(|(j, _)| *j), Some(i));
    assert!(zip_holds(&made).is_empty());
}

#[test]
fn a_file_dropped_on_a_zip_goes_into_it_as_a_copy() {
    let mut ctx = test_ctx();
    let (_t, zip, mut ex) = zip_beside(&[("kept.txt", "kept")], &[("new.txt", "new")], &mut ctx);
    let new = zip.with_file_name("new.txt");
    let i = ex.index_of(&zip).expect("the zip is listed");
    assert_eq!(ex.entry_dir(i), Some(zip.clone()), "the zip takes a drop");

    ex.drop_into(std::slice::from_ref(&new), &zip, DropOp::Move, &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);

    assert_eq!(
        zip_holds(&zip),
        [
            ("kept.txt".into(), "kept".into()),
            ("new.txt".into(), "new".into())
        ]
    );
    assert!(new.exists(), "the file dropped stays where it was");
    assert_eq!(ex.selected_paths(), [zip], "the zip is selected");
}

#[test]
fn a_name_the_zip_holds_asks_first_and_yes_replaces_its_entry() {
    let mut ctx = test_ctx();
    let (_t, zip, mut ex) = zip_beside(
        &[("a.txt", "old"), ("b.txt", "b")],
        &[("a.txt", "newer")],
        &mut ctx,
    );
    let a = zip.with_file_name("a.txt");
    ex.run(Command::Copy, &mut ctx);
    ex.paste = Some(PasteTo::Zip(zip.clone()));
    ex.pasted(
        FileClip {
            cut: false,
            paths: vec![a.clone()],
        },
        &mut ctx,
    );
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    match ex.dialog() {
        Some(Dialog::Replace(r)) => match &r.fields.subject {
            replacebox::Subject::File { existing, incoming } => {
                assert_eq!(r.fields.name, "a.txt");
                assert_eq!(
                    (existing.size.as_str(), incoming.size.as_str()),
                    ("3 bytes", "5 bytes"),
                    "the zip's entry and the file pasted"
                );
            }
            replacebox::Subject::Folder => panic!("a file clash shows both files"),
        },
        _ => panic!("the replace prompt is up"),
    }
    ex.event(crate::DIALOG, key_event_alt('y'), &mut ctx);
    ex.finish_extracts(reprisefs::progress::Decision::Abort, &mut ctx);
    assert_eq!(
        zip_holds(&zip),
        [
            ("a.txt".into(), "newer".into()),
            ("b.txt".into(), "b".into())
        ]
    );
}

#[test]
fn no_on_the_prompt_adds_nothing_and_leaves_the_zip_as_it_was() {
    let mut ctx = test_ctx();
    let (_t, zip, mut ex) = zip_beside(&[("a.txt", "old")], &[("a.txt", "newer")], &mut ctx);
    let before = std::fs::read(&zip).expect("read");
    ex.drop_into(&[zip.with_file_name("a.txt")], &zip, DropOp::Copy, &mut ctx);
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    ex.event(crate::DIALOG, key_event_alt('n'), &mut ctx);
    assert!(ex.extracting.is_none(), "no job starts");
    assert_eq!(std::fs::read(&zip).expect("read"), before);
}

#[test]
fn a_zips_item_menu_pastes_into_it() {
    let mut ctx = test_ctx();
    let (_t, zip, mut ex) = zip_beside(&[("a.txt", "a")], &[], &mut ctx);
    let i = ex.index_of(&zip).expect("listed");
    ex.sv.selection.set_only(Some(i));
    ex.entry_menu_at(i, (0, 0), &mut ctx, false);
    assert!(matches!(&ex.menu_paste, Some(PasteTo::Zip(z)) if *z == zip));
}

#[test]
fn a_file_onto_a_folder_of_its_name_in_the_zip_is_refused_as_a_copy_refuses_it() {
    let mut ctx = test_ctx();
    let (_t, zip, mut ex) = zip_beside(
        &[("photos/a.jpg", "a"), ("photos/b.jpg", "b")],
        &[("photos", "a file")],
        &mut ctx,
    );
    let before = std::fs::read(&zip).expect("read");
    ex.drop_into(
        &[zip.with_file_name("photos")],
        &zip,
        DropOp::Copy,
        &mut ctx,
    );
    crate::tests::wake_until(&mut ex, &mut ctx, |ex| ex.dialog().is_some());
    let Some(Dialog::Message { text, .. }) = ex.dialog() else {
        panic!("a box says why");
    };
    assert_eq!(
        text,
        "Cannot copy photos: There is already a file with the same name in this location."
    );
    assert!(ex.extracting.is_none(), "no job starts");
    assert_eq!(std::fs::read(&zip).expect("read"), before);
}
