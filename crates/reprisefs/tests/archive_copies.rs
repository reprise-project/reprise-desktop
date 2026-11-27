//! What the programs do with a file copied out of a compressed folder. Its own binary: it points
//! `$XDG_CACHE_HOME` and `$XDG_DATA_HOME` at a scratch home before anything reads them.

use reprisefs::{archive, archivecopy, recent};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn home() -> PathBuf {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home =
            std::env::temp_dir().join(format!("reprisefs-archive-copies-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("a scratch home");
        // SAFETY: set once, before any test reads the environment; the others wait on the lock.
        unsafe {
            std::env::set_var("XDG_CACHE_HOME", home.join("cache"));
            std::env::set_var("XDG_DATA_HOME", home.join("data"));
        }
        home
    })
    .clone()
}

/// A read-only copy named `name` in a fresh scratch folder, as Explorer leaves one.
fn copy(name: &str, bytes: &[u8]) -> PathBuf {
    home();
    let copy = archive::scratch().expect("a scratch folder").join(name);
    std::fs::write(&copy, bytes).expect("write the copy");
    std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o400)).expect("chmod");
    copy
}

fn plain(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = home().join("docs");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    path
}

#[test]
fn a_copy_out_of_a_compressed_folder_is_not_recorded() {
    let (copy, own) = (copy("notes.txt", b"inside"), plain("notes.txt", b"mine"));
    let xbel = home().join("recently-used.xbel");
    recent::record_at(&xbel, &[(&copy, None, None), (&own, None, None)]).expect("record");
    let got = recent::RecentFiles::at(Some(xbel)).current(10);
    assert_eq!(got, vec![own]);
}

#[test]
fn a_background_from_a_compressed_folder_is_kept_in_the_data_dir() {
    let kept_dir = home().join("data/reprise/backgrounds");
    let first = archivecopy::lasting_background(&copy("dunes.png", b"one")).expect("kept");
    assert_eq!(first, kept_dir.join("dunes.png"));
    let again = archivecopy::lasting_background(&copy("dunes.png", b"one")).expect("kept again");
    assert_eq!(again, first, "the same picture is used again");
    let other = archivecopy::lasting_background(&copy("dunes.png", b"two")).expect("another");
    assert_eq!(other, kept_dir.join("dunes (2).png"));
    assert_eq!(std::fs::read(&first).expect("read"), b"one");
    assert_eq!(std::fs::read(&other).expect("read"), b"two");
    let own = plain("dunes.png", b"mine");
    assert_eq!(archivecopy::lasting_background(&own).expect("own"), own);
    let around = home().join("cache/reprise/archives/../../docs/dunes.png");
    assert_eq!(
        archivecopy::lasting_background(&around).expect("own"),
        around
    );
}

#[test]
fn a_kept_background_never_writes_through_a_link_in_its_place() {
    let kept_dir = home().join("data/reprise/backgrounds");
    std::fs::create_dir_all(&kept_dir).expect("mkdir");
    let own = plain("sun.png", b"mine");
    std::os::unix::fs::symlink(&own, kept_dir.join("sun.png")).expect("symlink");
    std::fs::hard_link(&own, kept_dir.join("sun (2).png")).expect("hard link");
    let kept = archivecopy::lasting_background(&copy("sun.png", b"mine")).expect("kept");
    assert_eq!(kept, kept_dir.join("sun (3).png"));
    let kept = archivecopy::lasting_background(&copy("sun.png", b"new")).expect("kept");
    assert_eq!(kept, kept_dir.join("sun (4).png"));
    assert_eq!(std::fs::read(&own).expect("read"), b"mine");
}
#[test]
fn a_copy_names_the_compressed_folder_it_came_from() {
    let copy = copy("report.txt", b"inside");
    let zip = Path::new("/home/user/Downloads/test.zip");
    match archivecopy::set_origin(&copy, zip) {
        Err(e) if e.raw_os_error() == Some(libc::EOPNOTSUPP) => return,
        made => made.expect("mark the scratch folder"),
    }
    assert_eq!(archivecopy::origin(&copy).as_deref(), Some(zip));
    let own = plain("report.txt", b"mine");
    assert_eq!(archivecopy::origin(&own), None);
}

/// Marks `copy`'s folder as come from `archive` without [`archivecopy::set_origin`]'s checks;
/// false where the file system takes no user xattrs.
fn mark_raw(copy: &Path, archive: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let dir = std::ffi::CString::new(copy.parent().expect("a folder").as_os_str().as_bytes())
        .expect("no NUL");
    let value = archive.as_os_str().as_bytes();
    // SAFETY: `dir` and the name are NUL-terminated, and `value` is valid for its length.
    let set = unsafe {
        libc::setxattr(
            dir.as_ptr(),
            c"user.reprise.archive".as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    set == 0
}

#[test]
fn a_copy_out_of_an_archive_inside_another_names_the_outer_one() {
    let inner = copy("inner.zip", b"PK");
    let outer = Path::new("/home/user/Downloads/outer.zip");
    if !mark_raw(&inner, outer) {
        return;
    }
    let notes = copy("notes.txt", b"inside");
    archivecopy::set_origin(&notes, &inner).expect("mark");
    assert_eq!(archivecopy::origin(&notes).as_deref(), Some(outer));
    assert_eq!(
        archivecopy::dialog_folder(&notes).as_deref(),
        Some(Path::new("/home/user/Downloads"))
    );
}

#[test]
fn no_dialog_starts_in_a_scratch_folder() {
    let inner = copy("unknown.zip", b"PK");
    let notes = copy("notes.txt", b"inside");
    archivecopy::set_origin(&notes, &inner).expect("nothing to note");
    assert_eq!(archivecopy::origin(&notes), None);
    assert_eq!(archivecopy::dialog_folder(&notes), None);
    let marked = copy("marked.txt", b"inside");
    if mark_raw(&marked, &inner) {
        assert_eq!(archivecopy::dialog_folder(&marked), None);
    }
}
