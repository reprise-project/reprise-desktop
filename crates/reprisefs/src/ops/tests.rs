use super::mounts::mount_under;
use super::put::{FAIL_SWAP_BACK, Swap, preflight_merge, refuse_stuck_original, replace_file};
use super::*;
use crate::replace::{Replacing, Seen};
use crate::testfs::mkfifo;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::AtomicBool;

/// A Yes on the prompt for what `seen` saw, with no file inside a merge answered.
fn yes(seen: Option<Seen>) -> Option<Replacing> {
    seen.map(|seen| Replacing {
        seen,
        files: BTreeMap::new(),
    })
}

/// A Yes to All on the prompt for folder `dest` with `src` merging into it.
fn yes_all(src: &Path, dest: &Path) -> Option<Replacing> {
    Some(Replacing {
        seen: Seen::of(dest)?,
        files: merge_clashes(src, dest, false).into_iter().collect(),
    })
}

#[test]
fn read_regular_refuses_a_fifo_a_folder_and_a_file_over_the_cap() {
    let t = tempfile::tempdir().unwrap();
    let fifo = t.path().join("fifo");
    mkfifo(&fifo);
    let kind = |p: &Path, max| read_regular(p, max).unwrap_err().kind();
    assert_eq!(kind(&fifo, 10), io::ErrorKind::InvalidInput);
    assert_eq!(kind(t.path(), 10), io::ErrorKind::InvalidInput);
    let file = t.path().join("f");
    fs::write(&file, "0123456789").unwrap();
    assert_eq!(kind(&file, 9), io::ErrorKind::FileTooLarge);
    assert_eq!(read_regular(&file, 10).unwrap(), b"0123456789");
}

#[test]
fn create_new_numbered_skips_a_planted_symlink_and_a_taken_name() {
    let t = tempfile::tempdir().unwrap();
    let victim = t.path().join("victim");
    fs::write(&victim, "keep").unwrap();
    std::os::unix::fs::symlink(&victim, t.path().join("Shot.png")).unwrap();
    fs::write(t.path().join("Shot (2).png"), "older").unwrap();
    let (mut f, path) = create_new_numbered(t.path(), "Shot.png").unwrap();
    io::Write::write_all(&mut f, b"new").unwrap();
    assert_eq!(path, t.path().join("Shot (3).png"));
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    assert_eq!(
        fs::read_to_string(t.path().join("Shot (2).png")).unwrap(),
        "older"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
}

/// A dot followed by a space does not start an extension, on disk as
/// in a listing: "Mr. Smith" numbers as a whole.
#[test]
fn unique_path_numbers_a_name_whose_last_dot_is_not_an_extension() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("Mr. Smith"), "").unwrap();
    fs::write(t.path().join("a.txt"), "").unwrap();
    assert_eq!(
        unique_path(t.path(), "Mr. Smith"),
        t.path().join("Mr. Smith (2)")
    );
    assert_eq!(unique_path(t.path(), "a.txt"), t.path().join("a (2).txt"));
    assert_eq!(unique_path(t.path(), "b.txt"), t.path().join("b.txt"));
}

#[test]
fn shell_quote_wraps_in_single_quotes_and_escapes_them() {
    assert_eq!(shell_quote("plain"), "'plain'");
    assert_eq!(shell_quote("it's"), "'it'\\''s'");
    assert_eq!(shell_quote(""), "''");
}

#[test]
fn zip_argv_runs_bsdtar() {
    assert_eq!(
        zip_argv(
            &["/home/u/Desktop/Reports".into()],
            Path::new("/home/u/Desktop/Reports.zip")
        ),
        [
            "bsdtar",
            "-a",
            "-cf",
            "/home/u/Desktop/Reports.zip",
            "-C",
            "/home/u/Desktop",
            "--",
            "Reports"
        ]
    );
}

#[test]
fn zip_argv_keeps_a_name_that_is_not_utf8() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(OsStr::from_bytes(b"/d/\xff dir/a\xff.txt"));
    let zip = Path::new(OsStr::from_bytes(b"/d/\xff dir/a\xff.zip"));
    let argv = zip_argv(&[path.to_path_buf()], zip);
    assert_eq!(argv[3], zip.as_os_str());
    assert_eq!(argv[5], OsStr::from_bytes(b"/d/\xff dir"));
    assert_eq!(argv[7], OsStr::from_bytes(b"a\xff.txt"));
}

/// A moved item leaves its old folder's `.hidden`, and the new
/// folder's list gains nothing; a copy leaves the list alone.
#[test]
fn a_move_takes_a_listed_name_off_the_old_list_only() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "1").unwrap();
    fs::write(a.join("y.txt"), "2").unwrap();
    fs::write(a.join(".hidden"), "x.txt\ny.txt\n").unwrap();
    let sc = |_: &str| Some(String::new());
    put_in(&a.join("y.txt"), "y.txt", false, &b, DropOp::Copy, &[], &sc).unwrap();
    put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap();
    assert_eq!(fs::read_to_string(a.join(".hidden")).unwrap(), "y.txt\n");
    assert!(!b.join(".hidden").exists());
}

#[test]
fn put_in_moves_copies_and_names_a_copy_in_the_same_folder() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "1").unwrap();
    let sc = |_: &str| Some(String::new());
    assert_eq!(
        put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap(),
        "x.txt"
    );
    assert!(b.join("x.txt").exists() && !a.join("x.txt").exists());
    let name = put_in(
        &b.join("x.txt"),
        "x.txt",
        false,
        &b,
        DropOp::Copy,
        &["x.txt".into()],
        &sc,
    )
    .unwrap();
    assert_eq!(name, "Copy of x.txt");
    assert!(b.join("Copy of x.txt").exists());
}

/// A name that is not UTF-8 keeps its bytes through a move and a
/// copy, and `put_in` returns them, so an undo finds the file.
#[test]
fn put_in_keeps_a_non_utf8_name() {
    use std::os::unix::ffi::OsStrExt;
    let t = tempfile::tempdir().unwrap();
    let (a, b, c) = (t.path().join("a"), t.path().join("b"), t.path().join("c"));
    for d in [&a, &b, &c] {
        fs::create_dir(d).unwrap();
    }
    let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    fs::write(a.join(name), "hi").unwrap();
    let sc = |_: &str| None;

    let moved = put_in(
        &a.join(name),
        "caf\u{fffd}.txt",
        false,
        &b,
        DropOp::Move,
        &[],
        &sc,
    );
    assert_eq!(moved.unwrap(), name);
    assert!(b.join(name).is_file());

    let copied = put_in(
        &b.join(name),
        "caf\u{fffd}.txt",
        false,
        &c,
        DropOp::Copy,
        &[],
        &sc,
    );
    assert_eq!(copied.unwrap(), name);
    assert!(c.join(name).is_file());

    // Taken in the destination: replaced when told to, bytes kept.
    fs::write(b.join(name), "again").unwrap();
    let again = put_in_with(
        &b.join(name),
        "caf\u{fffd}.txt",
        false,
        &c,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&c.join(name))).as_ref(),
    );
    assert_eq!(again.unwrap(), name);
    assert_eq!(fs::read_to_string(c.join(name)).unwrap(), "again");
}

#[test]
fn put_in_shortcut_writes_the_callers_text() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("doc.txt"), "").unwrap();
    let name = put_in(
        &t.path().join("doc.txt"),
        "doc.txt",
        false,
        t.path(),
        DropOp::Shortcut,
        &[],
        &|n| Some(format!("[Desktop Entry]\nName={n}\n")),
    )
    .unwrap();
    assert_eq!(name, "Shortcut to doc.txt.desktop");
    assert_eq!(
        fs::read_to_string(t.path().join("Shortcut to doc.txt.desktop")).unwrap(),
        "[Desktop Entry]\nName=Shortcut to doc.txt\n"
    );
}

/// Pasting a folder into itself, into a folder under it, or through
/// a path with `..` in it: all refused, and nothing is written.
#[test]
fn put_in_refuses_a_destination_inside_the_source() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Reports");
    let deep = src.join("2005").join("Q1");
    fs::create_dir_all(&deep).unwrap();
    fs::write(src.join("x.txt"), "1").unwrap();
    let sc = |_: &str| Some(String::new());
    let before = fs::read_dir(&src).unwrap().count();

    for dir in [
        src.clone(),
        deep.clone(),
        src.join("2005").join("..").join("2005"),
    ] {
        for op in [DropOp::Copy, DropOp::Move] {
            let err = put_in(&src, "Reports", false, &dir, op, &[], &sc).unwrap_err();
            assert!(matches!(err, PutError::Subfolder), "{dir:?} {op:?}");
            assert_eq!(
                err.to_string(),
                "The destination folder is a subfolder of the source folder."
            );
        }
    }
    assert_eq!(fs::read_dir(&src).unwrap().count(), before);
    assert!(fs::read_dir(&deep).unwrap().next().is_none());

    // A folder copied beside itself is not its own subfolder.
    let away = t.path().join("elsewhere");
    fs::create_dir(&away).unwrap();
    assert_eq!(
        put_in(&src, "Reports", false, &away, DropOp::Copy, &[], &sc).unwrap(),
        "Reports"
    );
    assert!(away.join("Reports").join("x.txt").exists());
    // And a shortcut inside the folder itself is still allowed.
    assert!(
        put_in(&src, "Reports", false, &src, DropOp::Shortcut, &[], &|n| {
            Some(format!("[Desktop Entry]\nName={n}\n"))
        })
        .is_ok()
    );
}

/// A move into the folder that already holds the file is refused and
/// makes no copy.
#[test]
fn put_in_refuses_a_move_into_the_same_folder() {
    let t = tempfile::tempdir().unwrap();
    fs::write(t.path().join("x.txt"), "1").unwrap();
    let sc = |_: &str| None;
    let e = put_in(
        &t.path().join("x.txt"),
        "x.txt",
        false,
        t.path(),
        DropOp::Move,
        &["x.txt".into()],
        &sc,
    )
    .unwrap_err();
    assert!(matches!(e, PutError::SameFolder));
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}

/// A move or a copy onto a name the destination folder already holds
/// lands numbered unless told to replace, and the file already there
/// is left alone.
#[test]
fn put_in_numbers_past_an_existing_name() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    let b = t.path().join("b");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "source").unwrap();
    fs::write(b.join("x.txt"), "destination").unwrap();
    let sc = |_: &str| Some(String::new());

    let put = |op| put_in(&a.join("x.txt"), "x.txt", false, &b, op, &[], &sc).unwrap();
    assert_eq!(put(DropOp::Copy), "x (2).txt");
    assert_eq!(put(DropOp::Move), "x (3).txt");
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "destination");
    assert_eq!(fs::read_to_string(b.join("x (3).txt")).unwrap(), "source");
    assert!(!a.join("x.txt").exists(), "the move took its source");
}

/// Told to replace, a copy overwrites the file of the same name and a
/// move takes its place, leaving no temporary file behind.
#[test]
fn put_in_with_replace_overwrites_a_file_by_copy_and_by_move() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "source").unwrap();
    fs::write(b.join("x.txt"), "destination").unwrap();
    let sc = |_: &str| None;
    let put = |op| {
        put_in_with(
            &a.join("x.txt"),
            "x.txt",
            false,
            &b,
            op,
            &[],
            &sc,
            yes(Seen::of(&b.join("x.txt"))).as_ref(),
        )
    };

    assert_eq!(put(DropOp::Copy).unwrap(), "x.txt");
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "source");
    assert!(a.join("x.txt").exists(), "a copy keeps its source");

    fs::write(b.join("x.txt"), "destination").unwrap();
    assert_eq!(put(DropOp::Move).unwrap(), "x.txt");
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "source");
    assert!(!a.join("x.txt").exists(), "a move takes its source");
    assert_eq!(fs::read_dir(&b).unwrap().count(), 1, "no .part file left");
}

/// Told to replace, a folder merges into the folder of the same name:
/// its files replace theirs, the others on both sides stay, and a
/// moved folder is gone afterwards.
#[test]
fn put_in_with_replace_merges_a_folder() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
    fs::create_dir_all(src.join("q1")).unwrap();
    fs::create_dir_all(dest.join("Reports/q1")).unwrap();
    fs::write(src.join("same.txt"), "new").unwrap();
    fs::write(src.join("q1/deep.txt"), "new deep").unwrap();
    fs::write(src.join("mine.txt"), "mine").unwrap();
    fs::write(dest.join("Reports/same.txt"), "old").unwrap();
    fs::write(dest.join("Reports/q1/deep.txt"), "old deep").unwrap();
    fs::write(dest.join("Reports/theirs.txt"), "theirs").unwrap();
    let sc = |_: &str| None;
    let put = |op| {
        put_in_with(
            &src,
            "Reports",
            false,
            &dest,
            op,
            &[],
            &sc,
            yes_all(&src, &dest.join("Reports")).as_ref(),
        )
    };

    assert_eq!(put(DropOp::Copy).unwrap(), "Reports");
    let read = |p: &str| fs::read_to_string(dest.join("Reports").join(p)).unwrap();
    assert_eq!(read("same.txt"), "new");
    assert_eq!(read("q1/deep.txt"), "new deep");
    assert_eq!(read("mine.txt"), "mine");
    assert_eq!(read("theirs.txt"), "theirs");
    assert!(src.join("same.txt").exists(), "a copy keeps its source");

    fs::write(dest.join("Reports/same.txt"), "old").unwrap();
    assert_eq!(put(DropOp::Move).unwrap(), "Reports");
    assert_eq!(read("same.txt"), "new");
    assert_eq!(read("theirs.txt"), "theirs");
    assert!(!src.exists(), "a moved folder is removed once merged");
}

/// A merge puts a file over one inside only when the prompt's Yes covered it and it is still
/// what the prompt saw: a file answered No, or changed since, keeps its contents, and a move
/// leaves the incoming one in its folder.
#[test]
fn a_merge_replaces_only_the_files_answered_yes() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
    for op in [DropOp::Copy, DropOp::Move] {
        fs::create_dir_all(src.join("q1")).unwrap();
        fs::create_dir_all(dest.join("R/q1")).unwrap();
        for (name, new, old) in [
            ("yes.txt", "new", "old"),
            ("q1/no.txt", "new", "old"),
            ("changed.txt", "new", "old"),
        ] {
            fs::write(src.join(name), new).unwrap();
            fs::write(dest.join("R").join(name), old).unwrap();
        }
        fs::write(src.join("mine.txt"), "mine").unwrap();
        let seen = |n: &str| Seen::of(&dest.join("R").join(n)).unwrap();
        let answered = Replacing {
            seen: Seen::of(&dest.join("R")).unwrap(),
            files: [
                (PathBuf::from("yes.txt"), seen("yes.txt")),
                (PathBuf::from("changed.txt"), seen("changed.txt")),
            ]
            .into_iter()
            .collect(),
        };
        fs::write(dest.join("R/changed.txt"), "changed since").unwrap();
        let sc = |_: &str| None;
        let landed = put_in_with(&src, "R", false, &dest, op, &[], &sc, Some(&answered));
        assert_eq!(landed.unwrap(), "R", "{op:?}");
        let read = |p: &str| fs::read_to_string(dest.join("R").join(p)).unwrap();
        assert_eq!(read("yes.txt"), "new", "{op:?}");
        assert_eq!(read("q1/no.txt"), "old", "{op:?}");
        assert_eq!(read("changed.txt"), "changed since", "{op:?}");
        assert_eq!(read("mine.txt"), "mine", "{op:?}");
        let kept = |p: &str| src.join(p).exists();
        assert!(kept("q1/no.txt"), "{op:?}");
        assert!(kept("changed.txt"), "{op:?}");
        assert_eq!(kept("yes.txt"), op == DropOp::Copy, "{op:?}");
        fs::remove_dir_all(&dest).unwrap();
    }
}

/// A file onto a folder, or a folder onto a file, never replaces it,
/// even when told to: it lands numbered and the other stays.
#[test]
fn put_in_with_replace_numbers_a_file_onto_a_folder() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(a.join("Reports")).unwrap();
    fs::create_dir_all(b.join("x.txt")).unwrap();
    fs::write(a.join("x.txt"), "file").unwrap();
    fs::write(b.join("Reports"), "file").unwrap();
    let sc = |_: &str| None;
    let put = |name: &str, op| {
        put_in_with(
            &a.join(name),
            name,
            false,
            &b,
            op,
            &[],
            &sc,
            yes(Seen::of(&b.join(name))).as_ref(),
        )
    };
    assert_eq!(put("x.txt", DropOp::Copy).unwrap(), "x (2).txt");
    assert_eq!(put("Reports", DropOp::Move).unwrap(), "Reports (2)");
    assert!(b.join("x.txt").is_dir() && b.join("Reports").is_file());
    assert!(b.join("Reports (2)").is_dir() && !a.join("Reports").exists());
}

/// A merge that would put a file onto a folder, or a link onto a
/// folder, anywhere in the tree is refused before anything changes.
#[test]
fn a_merge_with_a_kind_mismatch_inside_changes_nothing() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
    fs::create_dir_all(src.join("q1")).unwrap();
    fs::create_dir_all(dest.join("Reports/q1/deep")).unwrap();
    fs::write(src.join("same.txt"), "new").unwrap();
    fs::write(dest.join("Reports/same.txt"), "old").unwrap();
    fs::write(src.join("q1/deep"), "a file onto a folder").unwrap();
    let sc = |_: &str| None;
    for op in [DropOp::Copy, DropOp::Move] {
        let err = put_in_with(
            &src,
            "Reports",
            false,
            &dest,
            op,
            &[],
            &sc,
            yes(Seen::of(&dest.join("Reports"))).as_ref(),
        )
        .unwrap_err();
        assert!(matches!(err, PutError::Occupied), "{op:?}");
        assert_eq!(
            fs::read_to_string(dest.join("Reports/same.txt")).unwrap(),
            "old"
        );
        assert!(src.join("same.txt").exists());
    }
    fs::remove_file(src.join("q1/deep")).unwrap();
    std::os::unix::fs::symlink(t.path(), src.join("q1/deep")).unwrap();
    let err = put_in_with(
        &src,
        "Reports",
        false,
        &dest,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&dest.join("Reports"))).as_ref(),
    )
    .unwrap_err();
    assert!(matches!(err, PutError::Occupied), "a link onto a folder");
}

/// A merge deeper than the copy's depth limit is refused before it
/// changes anything.
#[test]
fn a_merge_past_the_depth_limit_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let deep = "d/".repeat(MAX_DEPTH + 1);
    let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
    fs::create_dir_all(src.join(&deep)).unwrap();
    fs::create_dir_all(dest.join("R").join(&deep)).unwrap();
    fs::write(src.join("top.txt"), "new").unwrap();
    let sc = |_: &str| None;
    let err = put_in_with(
        &src,
        "R",
        false,
        &dest,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&dest.join("R"))).as_ref(),
    )
    .unwrap_err();
    assert!(matches!(&err, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
    assert!(!dest.join("R/top.txt").exists());
}

/// A link inside a merged folder that points at the file it would
/// replace is refused before anything changes: replacing would leave
/// a link to itself and lose the file.
#[test]
fn a_merge_never_replaces_a_file_with_a_link_to_it() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(dest.join("R")).unwrap();
    fs::write(dest.join("R/x.txt"), "keep").unwrap();
    fs::write(src.join("a.txt"), "a").unwrap();
    std::os::unix::fs::symlink(dest.join("R/x.txt"), src.join("x.txt")).unwrap();
    let sc = |_: &str| None;
    for op in [DropOp::Copy, DropOp::Move] {
        let seen = Seen::of(&dest.join("R"));
        let err =
            put_in_with(&src, "R", false, &dest, op, &[], &sc, yes(seen).as_ref()).unwrap_err();
        assert!(matches!(err, PutError::SameFile), "{op:?}");
        assert_eq!(fs::read_to_string(dest.join("R/x.txt")).unwrap(), "keep");
        assert!(!dest.join("R/a.txt").exists(), "nothing was merged");
    }
}

/// A Yes replaces only what the prompt saw: a file put at the name
/// since (by another batch) stays, and the put lands numbered.
#[test]
fn a_stale_answer_numbers_instead_of_replacing() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "mine").unwrap();
    fs::write(b.join("x.txt"), "asked about").unwrap();
    let seen = Seen::of(&b.join("x.txt"));
    fs::remove_file(b.join("x.txt")).unwrap();
    fs::write(b.join("x.txt"), "another batch's").unwrap();
    let sc = |_: &str| None;
    for (op, want) in [(DropOp::Copy, "x (2).txt"), (DropOp::Move, "x (3).txt")] {
        let Landing { name, replaced, .. } = put_in_until(
            &a.join("x.txt"),
            "x.txt",
            false,
            &b,
            op,
            &[],
            &sc,
            yes(seen).as_ref(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!((name.to_str().unwrap(), replaced), (want, false));
    }
    assert_eq!(
        fs::read_to_string(b.join("x.txt")).unwrap(),
        "another batch's"
    );
}

/// A folder under the source that cannot be read refuses the merge
/// before anything changes, rather than stopping it halfway.
#[test]
fn a_merge_with_an_unreadable_folder_changes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/R"), t.path().join("dest"));
    fs::create_dir_all(src.join("locked")).unwrap();
    fs::create_dir_all(dest.join("R")).unwrap();
    fs::write(src.join("a.txt"), "a").unwrap();
    fs::write(src.join("locked/b.txt"), "b").unwrap();
    fs::set_permissions(src.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(src.join("locked")).is_ok() {
        // Running as root: nothing is unreadable.
        return;
    }
    let sc = |_: &str| None;
    let seen = Seen::of(&dest.join("R"));
    let err = put_in_with(
        &src,
        "R",
        false,
        &dest,
        DropOp::Copy,
        &[],
        &sc,
        yes(seen).as_ref(),
    );
    fs::set_permissions(src.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(err, Err(PutError::Io(_))));
    assert!(!dest.join("R/a.txt").exists(), "nothing was merged");
}

/// A dangling link where the temporary file or a numbered name would
/// go is passed over, not retried until the put fails.
#[test]
fn dangling_links_at_the_next_names_are_passed_over() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "new").unwrap();
    fs::write(b.join("x.txt"), "old").unwrap();
    std::os::unix::fs::symlink(t.path().join("nowhere"), b.join(".reprise.part")).unwrap();
    std::os::unix::fs::symlink(t.path().join("nowhere"), b.join("x (2).txt")).unwrap();
    let sc = |_: &str| None;
    let seen = Seen::of(&b.join("x.txt"));
    put_in_with(
        &a.join("x.txt"),
        "x.txt",
        false,
        &b,
        DropOp::Copy,
        &[],
        &sc,
        yes(seen).as_ref(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
    let name = put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Move, &[], &sc).unwrap();
    assert_eq!(name, "x (3).txt");
}

/// A merge moving to another drive copies, and a copy cannot carry a
/// FIFO: the preflight refuses it, even deep in a folder the other
/// side does not have, before anything moves. On the same drive the
/// move renames it like anything else.
#[test]
fn a_move_to_another_drive_refuses_a_fifo_up_front() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    fs::create_dir_all(src.join("only/here")).unwrap();
    fs::create_dir_all(&dest).unwrap();
    mkfifo(&src.join("only/here/pipe"));
    let err = preflight_merge(&src, Some(&dest), true, true, 0).unwrap_err();
    assert!(matches!(&err, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
    assert!(preflight_merge(&src, Some(&dest), true, false, 0).is_ok());
}

/// A file that took the name after the last check (another job's) is
/// swapped back out and kept: the put lands numbered beside it, for a
/// copy and for a move, and no temporary file is left.
#[test]
fn a_changed_target_is_swapped_back_and_numbered() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "new").unwrap();
    fs::write(b.join("x.txt"), "another job's").unwrap();
    let asked_about = (0, 0);
    let from = fs::metadata(a.join("x.txt")).unwrap();
    for (moving, want) in [(false, "x (2).txt"), (true, "x (3).txt")] {
        match replace_file(
            &a.join("x.txt"),
            &b.join("x.txt"),
            &from,
            moving,
            asked_about,
        ) {
            Ok(Swap::Numbered(name)) => assert_eq!(name, want),
            Ok(_) => panic!("expected to land numbered beside another job's file"),
            Err(e) => panic!("{e}"),
        }
        assert_eq!(
            fs::read_to_string(b.join("x.txt")).unwrap(),
            "another job's"
        );
        assert_eq!(fs::read_to_string(b.join(want)).unwrap(), "new");
    }
    assert!(!a.join("x.txt").exists(), "the move took its source");
    let names: Vec<_> = fs::read_dir(&b)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 3, "no temporary file left: {names:?}");
}

/// A swap that finds another job's file and cannot swap it back
/// keeps both: the new file holds the name, the other is moved aside,
/// and no temporary file is left.
#[test]
fn a_failed_swap_back_keeps_both_files() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "new").unwrap();
    fs::write(b.join("x.txt"), "another job's").unwrap();
    let from = fs::metadata(a.join("x.txt")).unwrap();
    FAIL_SWAP_BACK.with(|f| f.set(true));
    let swap = replace_file(&a.join("x.txt"), &b.join("x.txt"), &from, false, (0, 0));
    FAIL_SWAP_BACK.with(|f| f.set(false));
    match swap {
        Ok(Swap::Displaced(aside)) => {
            assert_eq!(aside, "x (2).txt");
            assert_eq!(fs::read_to_string(b.join(&aside)).unwrap(), "another job's");
        }
        Ok(_) => panic!("expected the other file moved aside"),
        Err(e) => panic!("{e}"),
    }
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
    assert_eq!(
        fs::read_dir(&b).unwrap().count(),
        2,
        "no temporary file left"
    );
}

/// The file asked about is swapped out and removed, the new one in its
/// place.
#[test]
fn the_target_asked_about_is_swapped_out() {
    use std::os::unix::fs::MetadataExt;
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "new").unwrap();
    fs::write(b.join("x.txt"), "old").unwrap();
    let m = fs::symlink_metadata(b.join("x.txt")).unwrap();
    let from = fs::metadata(a.join("x.txt")).unwrap();
    let swap = replace_file(
        &a.join("x.txt"),
        &b.join("x.txt"),
        &from,
        false,
        (m.dev(), m.ino()),
    );
    assert!(matches!(swap, Ok(Swap::Replaced)));
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "new");
    assert_eq!(fs::read_dir(&b).unwrap().count(), 1);
}

/// A move to another drive whose original sits in a read-only folder
/// is refused before the copy: the original could not be removed.
#[test]
fn a_stuck_original_refuses_the_move_to_another_drive() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src"), t.path().join("dest"));
    fs::create_dir_all(src.join("R/locked")).unwrap();
    fs::create_dir_all(dest.join("R")).unwrap();
    fs::write(src.join("R/locked/a.txt"), "a").unwrap();
    fs::set_permissions(src.join("R/locked"), fs::Permissions::from_mode(0o555)).unwrap();
    let writable = can_write(&src.join("R/locked"));
    let merge = preflight_merge(&src.join("R"), Some(&dest.join("R")), true, true, 0);
    let file = refuse_stuck_original(&src.join("R/locked/a.txt"));
    fs::set_permissions(src.join("R/locked"), fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        // Running as root: nothing is read-only.
        return;
    }
    assert!(matches!(&merge, Err(PutError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied));
    assert!(matches!(&file, Err(PutError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied));
    assert!(refuse_stuck_original(&src.join("R")).is_ok());
}

/// A numbered copy passes over a name a dangling link holds.
#[test]
fn a_numbered_copy_passes_over_a_dangling_link() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "new").unwrap();
    fs::write(b.join("x.txt"), "old").unwrap();
    std::os::unix::fs::symlink(t.path().join("nowhere"), b.join("x (2).txt")).unwrap();
    let sc = |_: &str| None;
    let name = put_in(&a.join("x.txt"), "x.txt", false, &b, DropOp::Copy, &[], &sc).unwrap();
    assert_eq!(name, "x (3).txt");
}

/// A copied merge skips a FIFO as a plain folder copy does.
#[test]
fn a_copied_merge_skips_a_fifo() {
    let t = tempfile::tempdir().unwrap();
    let (src, dest) = (t.path().join("src/Reports"), t.path().join("dest"));
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(dest.join("Reports")).unwrap();
    mkfifo(&src.join("pipe"));
    fs::write(src.join("a.txt"), "a").unwrap();
    let sc = |_: &str| None;
    put_in_with(
        &src,
        "Reports",
        false,
        &dest,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&dest.join("Reports"))).as_ref(),
    )
    .unwrap();
    assert!(dest.join("Reports/a.txt").exists());
    assert!(fs::symlink_metadata(dest.join("Reports/pipe")).is_err());
}

/// A link as the source of a copy is followed when it replaces, as it
/// is when it lands on a free name: the file it points at is copied.
#[test]
fn a_copied_link_replaces_with_what_it_points_at() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(t.path().join("target.txt"), "target").unwrap();
    std::os::unix::fs::symlink(t.path().join("target.txt"), a.join("x.txt")).unwrap();
    fs::write(b.join("x.txt"), "old").unwrap();
    let sc = |_: &str| None;
    put_in_with(
        &a.join("x.txt"),
        "x.txt",
        false,
        &b,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&b.join("x.txt"))).as_ref(),
    )
    .unwrap();
    let meta = fs::symlink_metadata(b.join("x.txt")).unwrap();
    assert!(meta.is_file(), "a file, not a link");
    assert_eq!(fs::read_to_string(b.join("x.txt")).unwrap(), "target");
}

/// Replacing a file with another link to the same file is refused,
/// and the source stays where it is.
#[test]
fn replacing_a_hard_link_to_itself_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("x.txt"), "one").unwrap();
    fs::hard_link(a.join("x.txt"), b.join("x.txt")).unwrap();
    let sc = |_: &str| None;
    let err = put_in_with(
        &a.join("x.txt"),
        "x.txt",
        false,
        &b,
        DropOp::Move,
        &[],
        &sc,
        yes(Seen::of(&b.join("x.txt"))).as_ref(),
    )
    .unwrap_err();
    assert!(matches!(err, PutError::SameFile));
    assert_eq!(
        err.to_string(),
        "The source and destination are the same file."
    );
    assert!(a.join("x.txt").exists() && b.join("x.txt").exists());
}

/// The temporary file beside the one being replaced has a short name
/// of its own, so a name near the file system's limit still works.
#[test]
fn a_long_name_is_replaced_without_running_past_the_limit() {
    let t = tempfile::tempdir().unwrap();
    let (a, b) = (t.path().join("a"), t.path().join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let name = format!("{}.txt", "n".repeat(250));
    fs::write(a.join(&name), "new").unwrap();
    fs::write(b.join(&name), "old").unwrap();
    let sc = |_: &str| None;
    put_in_with(
        &a.join(&name),
        &name,
        false,
        &b,
        DropOp::Copy,
        &[],
        &sc,
        yes(Seen::of(&b.join(&name))).as_ref(),
    )
    .unwrap();
    assert_eq!(fs::read_to_string(b.join(&name)).unwrap(), "new");
    assert_eq!(fs::read_dir(&b).unwrap().count(), 1);
}

/// A copy into the folder it is in, reached through a link or `..`,
/// is still a copy beside itself: "Copy of x".
#[test]
fn a_copy_into_its_own_folder_through_a_link_is_a_copy_of() {
    let t = tempfile::tempdir().unwrap();
    let a = t.path().join("a");
    fs::create_dir_all(&a).unwrap();
    fs::write(a.join("x.txt"), "x").unwrap();
    std::os::unix::fs::symlink(&a, t.path().join("via")).unwrap();
    let sc = |_: &str| None;
    let name = put_in_with(
        &a.join("x.txt"),
        "x.txt",
        false,
        &t.path().join("via"),
        DropOp::Copy,
        &["x.txt".into()],
        &sc,
        yes(Seen::of(&a.join("x.txt"))).as_ref(),
    )
    .unwrap();
    assert_eq!(name, "Copy of x.txt");
    assert_eq!(fs::read_to_string(a.join("x.txt")).unwrap(), "x");
}

/// A copy never lands on a file that appeared after the name was
/// picked: the open itself refuses.
#[test]
fn copy_new_refuses_an_existing_destination() {
    let t = tempfile::tempdir().unwrap();
    let from = t.path().join("a");
    let to = t.path().join("b");
    fs::write(&from, "new").unwrap();
    fs::write(&to, "old").unwrap();
    let e = copy_new(&from, &to).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&to).unwrap(), "old");
    fs::remove_file(&to).unwrap();
    copy_new(&from, &to).unwrap();
    assert_eq!(fs::read_to_string(&to).unwrap(), "new");
}

/// A read that fails part way leaves no partial destination.
/// `/proc/self/mem` is a regular file whose read at offset 0 is `EIO`.
#[test]
fn copy_new_removes_the_destination_when_the_read_fails() {
    let t = tempfile::tempdir().unwrap();
    let to = t.path().join("b");
    assert!(copy_new(Path::new("/proc/self/mem"), &to).is_err());
    assert!(!to.exists(), "partial copy left behind");
}

/// A FIFO handed over directly is refused at once instead of blocking
/// the open, and nothing lands in the destination folder.
#[test]
fn put_in_refuses_a_fifo_as_the_source() {
    let t = tempfile::tempdir().unwrap();
    let pipe = t.path().join("pipe");
    mkfifo(&pipe);
    let dest = t.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let sc = |_: &str| None;

    let e = put_in(&pipe, "pipe", false, &dest, DropOp::Copy, &[], &sc).unwrap_err();

    assert!(matches!(e, PutError::Io(e) if e.kind() == io::ErrorKind::InvalidInput));
    assert!(fs::read_dir(&dest).unwrap().next().is_none());
}

/// A symlink is copied as a symlink, so a link pointing back up the
/// tree is one entry rather than a walk that never ends.
#[test]
fn copy_dir_all_copies_a_link_as_a_link_instead_of_following_it() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Reports");
    fs::create_dir_all(src.join("2005")).unwrap();
    fs::write(src.join("x.txt"), "1").unwrap();
    std::os::unix::fs::symlink("..", src.join("loop")).unwrap();
    let dest = t.path().join("Copy");

    copy_dir_all(&src, &dest).unwrap();

    let link = dest.join("loop");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_link(&link).unwrap(), Path::new(".."));
    assert!(dest.join("2005").is_dir());
    assert_eq!(fs::read_to_string(dest.join("x.txt")).unwrap(), "1");
}

/// A copy that fails leaves no half-folder behind, and a FIFO in the
/// tree is skipped rather than opened.
#[test]
fn a_failed_copy_removes_what_it_made() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Reports");
    fs::create_dir_all(src.join("open")).unwrap();
    fs::write(src.join("open/a.txt"), "1").unwrap();
    mkfifo(&src.join("open/pipe"));
    fs::create_dir(src.join("shut")).unwrap();
    fs::write(src.join("shut/b.txt"), "2").unwrap();
    fs::set_permissions(src.join("shut"), fs::Permissions::from_mode(0o000)).unwrap();
    let dest = t.path().join("Copy");

    let r = copy_dir_all(&src, &dest);
    fs::set_permissions(src.join("shut"), fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: geteuid takes no arguments and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        // root reads anything; only the FIFO part is checkable here.
        r.unwrap();
        assert!(!dest.join("open/pipe").exists());
        return;
    }
    assert!(r.is_err());
    assert!(!dest.exists(), "partial copy left behind");
}

#[test]
fn can_write_follows_the_folders_write_and_search_bits() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    assert!(can_write(t.path()), "a fresh temp dir is writable");
    fs::set_permissions(t.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let shut = can_write(t.path());
    fs::set_permissions(t.path(), fs::Permissions::from_mode(0o755)).unwrap();
    // SAFETY: geteuid takes no arguments and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        // root writes anywhere.
        return;
    }
    assert!(!shut, "a 0555 folder is not writable");
}

#[test]
fn a_stopped_copy_fails_interrupted_and_removes_what_it_made() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Reports");
    fs::create_dir_all(src.join("2005")).unwrap();
    fs::write(src.join("2005/a.txt"), "1").unwrap();
    let dest = t.path().join("Copy");
    let e = copy_dir_until(&src, &dest, &AtomicBool::new(true)).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::Interrupted);
    assert!(!dest.exists(), "no partial copy left behind");
    assert!(src.join("2005/a.txt").exists(), "the source is untouched");
}

/// A destination that already exists is refused and left as it was.
#[test]
fn copy_dir_all_leaves_an_existing_destination_alone() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("Reports");
    fs::create_dir(&src).unwrap();
    fs::write(src.join("new.txt"), "new").unwrap();
    let dest = t.path().join("Taken");
    fs::create_dir(&dest).unwrap();
    fs::write(dest.join("old.txt"), "old").unwrap();

    let e = copy_dir_all(&src, &dest).unwrap_err();

    assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(dest.join("old.txt")).unwrap(), "old");
    assert!(!dest.join("new.txt").exists());
}

/// A name that looks like an option or an archive to include is an
/// operand: bsdtar reads `-C` and `@` even past the double dash.
#[test]
fn zip_argv_takes_a_name_like_an_option_as_a_name() {
    let z = zip_argv(
        &[
            "/d/-x".into(),
            "/d/-Cy".into(),
            "/d/@z".into(),
            "/d/a-b".into(),
        ],
        Path::new("/d/-x.zip"),
    );
    assert_eq!(z[6..], ["--", "./-x", "./-Cy", "./@z", "a-b"]);
}

#[test]
fn zip_argv_puts_every_item_into_one_archive() {
    let z = zip_argv(&["/d/a.txt".into(), "/d/b".into()], Path::new("/d/a.zip"));
    assert_eq!(
        z,
        [
            "bsdtar", "-a", "-cf", "/d/a.zip", "-C", "/d", "--", "a.txt", "b"
        ]
    );
}

#[test]
fn mount_root_is_a_device_change_from_the_parent() {
    assert!(
        mount_root(0x803, 0x801),
        "a directory on another device is a mount root"
    );
    assert!(
        !mount_root(0x801, 0x801),
        "a directory on its parent's device is not"
    );
}

#[test]
fn is_mount_point_takes_the_root_and_not_a_plain_folder_or_a_link() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("Plain");
    fs::create_dir(&dir).unwrap();
    let link = t.path().join("Root link");
    std::os::unix::fs::symlink("/", &link).unwrap();

    assert!(is_mount_point(Path::new("/")), "/ is a mount point");
    assert!(!is_mount_point(&dir), "a plain folder is not");
    assert!(!is_mount_point(&link), "a link to / is not");
    assert!(
        !is_mount_point(&t.path().join("gone")),
        "a missing path is not"
    );
    assert!(refuse_mount_point(&dir).is_ok());
    assert_eq!(
        refuse_mount_point(Path::new("/")).unwrap_err().kind(),
        io::ErrorKind::ResourceBusy
    );
}

/// `Mounted` stands in for a mount: the lookup puts it and what is
/// under it on device 2, the rest of the tree on 1. The link to it is
/// not followed, so a tree holding only the link passes.
#[test]
fn mount_under_finds_a_folder_on_another_device_and_skips_links() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Stuff");
    let mounted = root.join("Docs").join("Mounted");
    fs::create_dir_all(mounted.join("deep")).unwrap();
    fs::write(root.join("a.txt"), "a").unwrap();
    let linked = t.path().join("Linked");
    fs::create_dir(&linked).unwrap();
    std::os::unix::fs::symlink(&mounted, linked.join("to mount")).unwrap();
    let dev_of = |p: &Path| Ok(if p.starts_with(&mounted) { 2 } else { 1 });

    assert_eq!(mount_under(&root, &dev_of).unwrap(), Some(mounted.clone()));
    assert_eq!(mount_under(&linked, &dev_of).unwrap(), None);
    assert_eq!(mount_under(&root.join("a.txt"), &dev_of).unwrap(), None);
    let one = |_: &Path| Ok(1);
    assert_eq!(mount_under(&root, &one).unwrap(), None);
    assert!(
        refuse_holding_mount(&root).is_ok(),
        "a real tree holds no mount"
    );
}

#[test]
fn erase_removes_a_plain_folder_and_only_the_link_of_a_link() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("Folder");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("sub/a.txt"), "a").unwrap();
    let link = t.path().join("Link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();

    erase(&link).unwrap();
    assert!(!link.exists() && dir.join("sub/a.txt").exists());
    erase(&dir).unwrap();
    assert!(!dir.exists());
}

#[test]
fn trash_in_keeps_a_record_and_takes_a_gone_path_as_done() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Trash");
    let file = t.path().join("notes.txt");
    fs::write(&file, "n").unwrap();

    trash_in(&root, &file).unwrap();

    assert!(!file.exists(), "the file left its folder");
    assert_eq!(
        fs::read_to_string(root.join("files/notes.txt")).unwrap(),
        "n"
    );
    let info = fs::read_to_string(root.join("info/notes.txt.trashinfo")).unwrap();
    assert_eq!(
        crate::trash::original_path(&info, None).as_deref(),
        Some(file.as_path())
    );
    trash_in(&root, &file).expect("a gone path is already deleted");
}

/// A name that is not UTF-8 is erased by its own bytes; the name it
/// would read as with U+FFFD in place of the byte stays.
#[test]
fn delete_erases_a_name_that_is_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let t = tempfile::tempdir().unwrap();
    let raw = t.path().join(std::ffi::OsStr::from_bytes(b"a\xffb.txt"));
    let lossy = t.path().join("a\u{fffd}b.txt");
    fs::write(&raw, "raw").unwrap();
    fs::write(&lossy, "lossy").unwrap();

    delete(&raw, crate::bincap::Plan::Erase).unwrap();

    assert!(!raw.exists(), "the named file is gone");
    assert_eq!(fs::read_to_string(&lossy).unwrap(), "lossy");
}

#[test]
fn modifier_op_takes_ctrl_shift_and_alt() {
    assert_eq!(modifier_op(false, false, false), None);
    assert_eq!(modifier_op(true, false, false), Some(DropOp::Copy));
    assert_eq!(modifier_op(false, true, false), Some(DropOp::Move));
    assert_eq!(modifier_op(false, false, true), Some(DropOp::Shortcut));
    assert_eq!(modifier_op(true, true, false), Some(DropOp::Shortcut));
    assert_eq!(modifier_op(true, false, true), Some(DropOp::Shortcut));
}

/// Set `user.test=1` on `path`; false (said on stderr) where the file
/// system takes no `user.*` xattrs.
fn set_user_test(path: &Path) -> bool {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: both strings are NUL-terminated and the value is one byte.
    let rc = unsafe {
        libc::setxattr(
            c.as_ptr(),
            c"user.test".as_ptr(),
            c"1".as_ptr().cast(),
            1,
            0,
        )
    };
    if rc != 0 {
        eprintln!(
            "SKIPPED xattr check: {} takes no user xattrs ({})",
            path.display(),
            io::Error::last_os_error()
        );
    }
    rc == 0
}

fn user_test(path: &Path) -> Option<Vec<u8>> {
    let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let mut buf = [0u8; 8];
    // SAFETY: both strings are NUL-terminated and `buf` is writable for its length.
    let n = unsafe {
        libc::getxattr(
            c.as_ptr(),
            c"user.test".as_ptr(),
            buf.as_mut_ptr().cast(),
            buf.len(),
        )
    };
    usize::try_from(n).ok().map(|n| buf[..n].to_vec())
}

fn an_hour_ago() -> std::time::SystemTime {
    std::time::SystemTime::now() - std::time::Duration::from_secs(3600)
}

fn set_mtime(path: &Path, t: std::time::SystemTime) {
    fs::File::open(path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(t))
        .unwrap();
}

#[test]
fn copy_new_keeps_the_mtime_and_xattrs() {
    let t = tempfile::tempdir().unwrap();
    let from = t.path().join("photo.jpg");
    fs::write(&from, "jpeg").unwrap();
    let xattr = set_user_test(&from);
    set_mtime(&from, an_hour_ago());
    let to = t.path().join("copy.jpg");

    copy_new(&from, &to).unwrap();

    let modified = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    assert_eq!(modified(&to), modified(&from), "the copy keeps the mtime");
    if xattr {
        assert_eq!(
            user_test(&to).as_deref(),
            Some(&b"1"[..]),
            "user.test=1 kept"
        );
    }
}

#[test]
fn copy_dir_all_keeps_folder_modes_and_mtimes() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("private");
    let inner = src.join("inner");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("a.txt"), "a").unwrap();
    fs::set_permissions(&inner, fs::Permissions::from_mode(0o2750)).unwrap();
    fs::set_permissions(&src, fs::Permissions::from_mode(0o700)).unwrap();
    set_mtime(&inner, an_hour_ago());
    set_mtime(&src, an_hour_ago());
    let dest = t.path().join("copy");

    copy_dir_all(&src, &dest).unwrap();

    let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&dest), 0o700, "the top folder stays 0700");
    assert_eq!(
        mode(&dest.join("inner")),
        0o2750,
        "a subfolder keeps setgid"
    );
    let modified = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    assert_eq!(modified(&dest), modified(&src));
    assert_eq!(modified(&dest.join("inner")), modified(&inner));
}

#[test]
fn copy_new_keeps_a_read_only_launcher_trusted() {
    use std::os::unix::fs::PermissionsExt;
    let t = tempfile::tempdir().unwrap();
    let from = t.path().join("app.desktop");
    fs::write(&from, "[Desktop Entry]\nType=Application\nExec=true\n").unwrap();
    if let Err(e) = crate::apps::set_trusted(&from) {
        eprintln!("SKIPPED: {} takes no user xattrs ({e})", t.path().display());
        return;
    }
    fs::set_permissions(&from, fs::Permissions::from_mode(0o555)).unwrap();
    let to = t.path().join("copy.desktop");

    copy_new(&from, &to).unwrap();

    assert!(crate::apps::is_trusted(&to), "the copy is trusted");
    assert_eq!(
        fs::metadata(&to).unwrap().permissions().mode() & 0o7777,
        0o555
    );
}
