use crate::trash::{self, Bins};
use std::fs;
use std::io;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

fn uid() -> u32 {
    // SAFETY: getuid has no failure mode.
    unsafe { libc::getuid() }
}

/// A temp dir holding a drive `STICK`, its trash `.Trash-$uid` with `files/` and `info/` as the
/// test makes them, and a folder `victim` off the drive holding `keep.txt` and
/// `keep.txt.trashinfo`, both dated long ago.
struct Drive {
    t: tempfile::TempDir,
    top: PathBuf,
    root: PathBuf,
    victim: PathBuf,
}

fn drive() -> Drive {
    let t = tempfile::tempdir().unwrap();
    let top = t.path().join("STICK");
    let root = top.join(format!(".Trash-{}", uid()));
    let victim = t.path().join("victim");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&victim).unwrap();
    fs::write(victim.join("keep.txt"), "mine").unwrap();
    fs::create_dir(victim.join("dir")).unwrap();
    fs::write(victim.join("dir/deep.txt"), "mine").unwrap();
    let old = trash::trashinfo(Path::new("keep.txt"), 1_000_000);
    fs::write(victim.join("keep.txt.trashinfo"), &old).unwrap();
    fs::write(victim.join("dir.trashinfo"), &old).unwrap();
    Drive {
        t,
        top,
        root,
        victim,
    }
}

fn victim_whole(d: &Drive) {
    for f in [
        "keep.txt",
        "dir/deep.txt",
        "keep.txt.trashinfo",
        "dir.trashinfo",
    ] {
        assert!(d.victim.join(f).exists(), "{f} off the drive stays");
    }
}

#[test]
fn empty_never_walks_a_linked_files_or_info_and_removes_the_links() {
    let d = drive();
    symlink(&d.victim, d.root.join("files")).unwrap();
    symlink(&d.victim, d.root.join("info")).unwrap();
    let _ = trash::empty_in(&d.root);
    victim_whole(&d);
    assert!(
        fs::symlink_metadata(d.root.join("files")).is_err(),
        "the files link goes"
    );
    assert!(
        fs::symlink_metadata(d.root.join("info")).is_err(),
        "the info link goes"
    );
}

#[test]
fn empty_removes_a_linked_entry_and_not_its_target() {
    let d = drive();
    fs::create_dir_all(d.root.join("files")).unwrap();
    fs::create_dir_all(d.root.join("info")).unwrap();
    symlink(d.victim.join("dir"), d.root.join("files/dir")).unwrap();
    symlink(
        d.victim.join("keep.txt.trashinfo"),
        d.root.join("info/x.trashinfo"),
    )
    .unwrap();
    assert_eq!(trash::empty_in(&d.root).unwrap(), 1);
    victim_whole(&d);
    assert_eq!(fs::read_dir(d.root.join("files")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(d.root.join("info")).unwrap().count(), 0);
}

#[test]
fn empty_refuses_a_drive_trash_that_is_a_link() {
    let d = drive();
    let linked = d.top.join(format!(".Trash-{}", uid()));
    fs::remove_dir(&linked).unwrap();
    fs::create_dir(d.victim.join("files")).unwrap();
    fs::write(d.victim.join("files/keep.txt"), "mine").unwrap();
    symlink(&d.victim, &linked).unwrap();
    let e = trash::empty_in(&linked).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(d.victim.join("files/keep.txt").exists());
}

#[test]
fn empty_refuses_a_shared_trash_without_the_sticky_bit() {
    let d = drive();
    let shared = d.top.join(".Trash");
    let own = shared.join(uid().to_string());
    fs::create_dir_all(own.join("files")).unwrap();
    fs::write(own.join("files/keep.txt"), "mine").unwrap();
    fs::set_permissions(&shared, fs::Permissions::from_mode(0o777)).unwrap();
    let e = trash::empty_in(&own).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(own.join("files/keep.txt").exists());

    fs::set_permissions(&shared, fs::Permissions::from_mode(0o1777)).unwrap();
    assert_eq!(trash::empty_in(&own).unwrap(), 1, "a sticky one empties");
}

#[test]
fn the_daily_purge_never_walks_a_linked_files_or_info() {
    let d = drive();
    symlink(&d.victim, d.root.join("files")).unwrap();
    fs::create_dir(d.root.join("info")).unwrap();
    let old = trash::trashinfo(Path::new("keep.txt"), 1_000_000);
    fs::write(d.root.join("info/keep.txt.trashinfo"), &old).unwrap();
    fs::write(d.root.join("info/dir.trashinfo"), &old).unwrap();
    crate::binage::purge_in(&d.root, 1, 1_800_000_000);
    victim_whole(&d);

    fs::remove_file(d.root.join("files")).unwrap();
    fs::remove_dir_all(d.root.join("info")).unwrap();
    fs::create_dir(d.root.join("files")).unwrap();
    symlink(&d.victim, d.root.join("info")).unwrap();
    crate::binage::purge_in(&d.root, 1, 1_800_000_000);
    victim_whole(&d);
}

#[test]
fn making_room_never_walks_a_linked_files() {
    let d = drive();
    symlink(&d.victim, d.root.join("files")).unwrap();
    symlink(&d.victim, d.root.join("info")).unwrap();
    let used = crate::bincap::used(&d.root);
    let _ = crate::bincap::make_room(&d.root, used, 1, 1);
    victim_whole(&d);
}

#[test]
fn a_drive_trash_with_a_linked_files_takes_nothing_and_lists_nothing() {
    let d = drive();
    symlink(&d.victim, d.root.join("files")).unwrap();
    let bins = Bins::new(
        d.t.path().join("home/Trash"),
        vec![d.t.path().to_path_buf(), d.top.clone()],
    );
    let src = d.top.join("x.txt");
    fs::write(&src, "x").unwrap();
    let e = bins.trash(&src).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::CrossesDevices);
    assert!(src.exists());
    assert!(!d.victim.join("x.txt").exists());
    assert!(!bins.files_dirs().contains(&d.root.join("files")));
}

fn record(d: &Drive, name: &str, path: &str) -> PathBuf {
    fs::create_dir_all(d.root.join("files")).unwrap();
    fs::create_dir_all(d.root.join("info")).unwrap();
    let files = d.root.join("files").join(name);
    fs::write(&files, "from the drive").unwrap();
    let body = format!("[Trash Info]\nPath={path}\nDeletionDate=2026-01-01T00:00:00\n");
    fs::write(d.root.join("info").join(format!("{name}.trashinfo")), body).unwrap();
    files
}

#[test]
fn a_drive_record_naming_a_path_off_the_drive_is_refused() {
    let d = drive();
    let away = d.victim.join("planted.desktop");
    let files = record(&d, "planted.desktop", &trash::percent_encode(&away));
    let e = trash::trashed(&files).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    assert!(e.to_string().contains("outside its drive"), "{e}");

    let climbs = format!("{}/../victim/x", trash::percent_encode(&d.top));
    let files = record(&d, "x", &climbs);
    assert_eq!(
        trash::trashed(&files).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert!(!away.exists());
}

#[test]
fn a_drive_record_with_an_absolute_path_on_the_drive_restores() {
    let d = drive();
    let back = d.top.join("docs/a.txt");
    let files = record(&d, "a.txt", &trash::percent_encode(&back));
    let t = trash::trashed(&files).unwrap();
    assert_eq!(t.original, back);
    assert_eq!(trash::restore_free(&t).unwrap(), back);
    assert_eq!(fs::read_to_string(&back).unwrap(), "from the drive");
    assert!(!t.info.exists());
}

#[test]
fn a_drive_restore_never_follows_a_link_on_the_drive() {
    let d = drive();
    symlink(&d.victim, d.top.join("docs")).unwrap();
    let files = record(&d, "a.txt", "docs/a.txt");
    let t = trash::trashed(&files).unwrap();
    let e = trash::restore_free(&t).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    assert!(!d.victim.join("a.txt").exists());
    assert!(files.exists(), "the item stays in the bin");
    assert!(t.info.exists());
}

#[test]
fn a_drive_restore_makes_its_folders_and_numbers_a_taken_name() {
    let d = drive();
    let files = record(&d, "a.txt", "new/deep/a.txt");
    let t = trash::trashed(&files).unwrap();
    assert_eq!(
        trash::restore_free(&t).unwrap(),
        d.top.join("new/deep/a.txt")
    );
    let files = record(&d, "a.txt", "new/deep/a.txt");
    let t = trash::trashed(&files).unwrap();
    assert_eq!(
        trash::restore_free(&t).unwrap(),
        d.top.join("new/deep/a (2).txt")
    );
}

#[test]
fn the_size_cache_is_never_read_or_written_through_a_link() {
    let d = drive();
    fs::create_dir_all(d.root.join("files/dir")).unwrap();
    fs::write(d.root.join("files/dir/a"), "abc").unwrap();
    fs::create_dir(d.root.join("info")).unwrap();
    let old = trash::trashinfo(Path::new("dir"), 1_000_000);
    fs::write(d.root.join("info/dir.trashinfo"), old).unwrap();
    let sizes = d.root.join("directorysizes");
    symlink(d.victim.join("keep.txt"), &sizes).unwrap();
    crate::bincap::used(&d.root);
    assert_eq!(
        fs::read_to_string(d.victim.join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(
        fs::symlink_metadata(&sizes).unwrap().is_file(),
        "the cache replaces the link"
    );

    fs::remove_file(&sizes).unwrap();
    let c = std::ffi::CString::new(sizes.as_os_str().as_encoded_bytes()).unwrap();
    // SAFETY: `c` is a NUL-terminated path that outlives the call.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    crate::bincap::gone(&d.root.join("files/dir"));
}

/// Folder `top/name` holding up to `levels` folders each inside the last, made by descriptor
/// since the path outgrows `PATH_MAX`; returns how many levels the file system allowed.
fn deep_tree(top: &Path, name: &str, levels: usize) -> usize {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    fs::create_dir(top.join(name)).unwrap();
    let mut at = OwnedFd::from(fs::File::open(top.join(name)).unwrap());
    for level in 0..levels {
        // SAFETY: the name is NUL-terminated and `at` is an open folder.
        if unsafe { libc::mkdirat(at.as_raw_fd(), c"d".as_ptr(), 0o755) } != 0 {
            let e = io::Error::last_os_error();
            // A path-based security module refuses a path past PATH_MAX.
            assert_eq!(
                e.raw_os_error(),
                Some(libc::ENAMETOOLONG),
                "level {level}: {e}"
            );
            return level;
        }
        // SAFETY: as above.
        let fd = unsafe {
            libc::openat(
                at.as_raw_fd(),
                c"d".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY,
            )
        };
        assert!(fd >= 0, "level {level}: {}", io::Error::last_os_error());
        // SAFETY: `fd` was just opened and is owned by nothing else.
        at = unsafe { OwnedFd::from_raw_fd(fd) };
    }
    levels
}

#[test]
fn empty_removes_a_tree_thousands_of_folders_deep() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path().join("Trash");
    fs::create_dir_all(root.join("files")).unwrap();
    fs::create_dir_all(root.join("info")).unwrap();
    let levels = deep_tree(&root.join("files"), "deep", 10_000);
    assert!(levels >= 4_000, "only {levels} levels");
    let info = trash::trashinfo(Path::new("deep"), 1_000_000);
    fs::write(root.join("info/deep.trashinfo"), info).unwrap();
    // Depth must cost heap, not stack: a small stack overflows on a walk that recurses.
    let emptied = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || trash::empty_in(&root).map(|n| (n, root)))
        .unwrap()
        .join()
        .expect("the walk returns");
    let (n, root) = emptied.unwrap();
    assert_eq!(n, 1);
    assert_eq!(fs::read_dir(root.join("files")).unwrap().count(), 0);
}
