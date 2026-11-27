//! What the programs do with a file copied out of a compressed folder. Its own binary: it points
//! `$XDG_CACHE_HOME` and `$XDG_DATA_HOME` at a scratch home before anything reads them.

use reprisefs::{archive, archivecopy, recent};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

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
    let again = archivecopy::lasting_background(&copy("dunes.png", b"two")).expect("kept again");
    assert_eq!(again, first);
    assert_eq!(std::fs::read(&again).expect("read"), b"two");
    let own = plain("dunes.png", b"mine");
    assert_eq!(archivecopy::lasting_background(&own).expect("own"), own);
}
