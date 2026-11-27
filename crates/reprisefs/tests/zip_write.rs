//! Writing into a zip browsed as a compressed folder. Its own binary: it points `$XDG_CACHE_HOME`
//! at a scratch home before anything reads it, and re-runs itself to be killed mid-rewrite.

use reprisefs::archive::{self, Kind};
use reprisefs::extract::{ArchiveEvent, ArchiveJob};
use reprisefs::progress::Meter;
use reprisefs::zipwrite::{self, Adding, Bound};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn home() -> PathBuf {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let home = std::env::temp_dir().join(format!("reprisefs-zip-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("a scratch home");
        // SAFETY: set once, before any test reads the environment; the others wait on the lock.
        unsafe { std::env::set_var("XDG_CACHE_HOME", home.join("cache")) };
        home
    })
    .clone()
}

/// A fresh folder of the scratch home holding `files` (paths and contents) under `src`.
fn fresh(files: &[(&str, &[u8])]) -> PathBuf {
    let dir = tempfile::tempdir_in(home()).expect("a folder").keep();
    for (name, bytes) in files {
        let path = dir.join("src").join(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(&path, bytes).expect("write");
    }
    dir
}

/// `dir/a.zip` made by bsdtar from `names` under `dir/src`, in that order.
fn zip_of(dir: &Path, names: &[&str]) -> PathBuf {
    let zip = dir.join("a.zip");
    let st = Command::new("bsdtar")
        .args(["-c", "-n", "--format", "zip", "-f"])
        .arg(&zip)
        .arg("-C")
        .arg(dir.join("src"))
        .args(names)
        .status()
        .expect("bsdtar");
    assert!(st.success(), "bsdtar made the zip");
    zip
}

/// The zip's entries as (path, kind, size), in listing order.
fn listing(zip: &Path) -> Vec<(String, Kind, u64)> {
    let entries = archive::list(zip, &Meter::default()).expect("the zip lists");
    (entries.iter())
        .map(|e| (e.path.display().to_string(), e.kind, e.size))
        .collect()
}

fn sorted(mut v: Vec<(String, Kind, u64)>) -> Vec<(String, Kind, u64)> {
    v.sort();
    v
}

/// The bytes of the first entry `inner` of `zip`.
fn read_entry(zip: &Path, inner: &str) -> Vec<u8> {
    let argv = archive::open_argv(zip, Path::new(inner));
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .expect("bsdtar");
    assert!(out.status.success(), "{inner} is in the zip");
    out.stdout
}

/// The temp files a rewrite left beside `zip`.
fn temps(zip: &Path) -> Vec<PathBuf> {
    let name = zip
        .file_name()
        .expect("a name")
        .to_string_lossy()
        .into_owned();
    std::fs::read_dir(zip.parent().expect("a folder"))
        .expect("list")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.starts_with(&format!(".{name}.")) && n.ends_with(".tmp")
        })
        .collect()
}

fn add(dir: &Path, names: &[&str]) -> Vec<Adding> {
    vec![Adding {
        dir: dir.to_path_buf(),
        names: names.iter().map(PathBuf::from).collect(),
    }]
}

fn rewrite(zip: &Path, adds: &[Adding]) -> Result<(), String> {
    zipwrite::rewrite(zip, adds, &Arc::new(Meter::default()), Bound::default())
}

#[test]
fn a_saved_file_replaces_its_entry_and_keeps_the_rest_and_the_mode() {
    let dir = fresh(&[
        ("notes.txt", b"top"),
        ("sub/notes.txt", b"old text"),
        ("sub/other.txt", b"other"),
    ]);
    let zip = zip_of(
        &dir,
        &["notes.txt", "sub", "sub/notes.txt", "sub/other.txt"],
    );
    std::fs::set_permissions(&zip, std::fs::Permissions::from_mode(0o640)).expect("chmod");
    zipwrite::update_entry(&zip, Path::new("sub/notes.txt"), b"new and longer text")
        .expect("updated");
    assert_eq!(read_entry(&zip, "sub/notes.txt"), b"new and longer text");
    assert_eq!(read_entry(&zip, "notes.txt"), b"top");
    assert_eq!(
        sorted(listing(&zip)),
        sorted(vec![
            ("notes.txt".into(), Kind::File, 3),
            ("sub".into(), Kind::Folder, 0),
            ("sub/notes.txt".into(), Kind::File, 19),
            ("sub/other.txt".into(), Kind::File, 5),
        ])
    );
    let mode = std::fs::metadata(&zip).expect("stat").permissions().mode();
    assert_eq!(mode & 0o777, 0o640, "the zip keeps its mode");
    assert!(temps(&zip).is_empty(), "no temp file is left");
}

#[test]
fn added_files_land_by_their_names_even_ones_bsdtar_reads_as_options() {
    let dir = fresh(&[("kept.txt", b"kept")]);
    let zip = zip_of(&dir, &["kept.txt"]);
    let drop = fresh(&[("@kept.txt", b"at"), ("-C", b"dash"), ("a\nb.txt", b"nl")]);
    rewrite(
        &zip,
        &add(&drop.join("src"), &["@kept.txt", "-C", "a\nb.txt"]),
    )
    .expect("added");
    assert_eq!(
        sorted(listing(&zip)),
        sorted(vec![
            ("kept.txt".into(), Kind::File, 4),
            ("@kept.txt".into(), Kind::File, 2),
            ("-C".into(), Kind::File, 4),
            ("a\nb.txt".into(), Kind::File, 2),
        ])
    );
    assert_eq!(read_entry(&zip, "kept.txt"), b"kept");
}

#[test]
fn a_name_with_a_line_break_replaces_only_its_own_entry() {
    let dir = fresh(&[("a\nb.txt", b"old"), ("a", b"keep"), ("b.txt", b"keep too")]);
    let zip = zip_of(&dir, &["a\nb.txt", "a", "b.txt"]);
    let drop = fresh(&[("a\nb.txt", b"newer")]);
    rewrite(&zip, &add(&drop.join("src"), &["a\nb.txt"])).expect("added");
    assert_eq!(read_entry(&zip, "a\nb.txt"), b"newer");
    assert_eq!(listing(&zip).len(), 3);
}

#[test]
fn an_added_folder_merges_into_the_zips_folder_of_that_name() {
    let dir = fresh(&[("docs/a.txt", b"old a"), ("docs/b.txt", b"b")]);
    let zip = zip_of(&dir, &["docs", "docs/a.txt", "docs/b.txt"]);
    let drop = fresh(&[("docs/a.txt", b"new a!"), ("docs/c.txt", b"c")]);
    rewrite(&zip, &add(&drop.join("src"), &["docs"])).expect("merged");
    assert_eq!(
        sorted(listing(&zip)),
        sorted(vec![
            ("docs".into(), Kind::Folder, 0),
            ("docs/a.txt".into(), Kind::File, 6),
            ("docs/b.txt".into(), Kind::File, 1),
            ("docs/c.txt".into(), Kind::File, 1),
        ])
    );
    assert_eq!(read_entry(&zip, "docs/a.txt"), b"new a!");
}

#[test]
fn an_added_file_replaces_a_folder_of_its_name_and_all_below_it() {
    let dir = fresh(&[("docs/a.txt", b"a"), ("docsx.txt", b"x")]);
    let zip = zip_of(&dir, &["docs", "docs/a.txt", "docsx.txt"]);
    let drop = fresh(&[("docs", b"a file now")]);
    rewrite(&zip, &add(&drop.join("src"), &["docs"])).expect("replaced");
    assert_eq!(
        sorted(listing(&zip)),
        sorted(vec![
            ("docs".into(), Kind::File, 10),
            ("docsx.txt".into(), Kind::File, 1),
        ])
    );
}

#[test]
fn files_go_into_an_empty_zip() {
    let dir = fresh(&[("one.txt", b"1")]);
    let zip = dir.join("New.zip");
    std::fs::write(&zip, zipwrite::EMPTY_ZIP).expect("write");
    assert!(zipwrite::is_zip(&zip));
    assert!(listing(&zip).is_empty(), "the empty zip lists");
    rewrite(&zip, &add(&dir.join("src"), &["one.txt"])).expect("added");
    assert_eq!(listing(&zip), vec![("one.txt".into(), Kind::File, 1)]);
}

#[test]
fn a_link_a_tar_and_too_much_are_refused_and_left_alone() {
    let dir = fresh(&[("a.txt", b"a")]);
    let zip = zip_of(&dir, &["a.txt"]);
    let before = std::fs::read(&zip).expect("read");
    let link = dir.join("link.zip");
    std::os::unix::fs::symlink(&zip, &link).expect("link");
    let why = rewrite(&link, &add(&dir.join("src"), &["a.txt"])).expect_err("a link");
    assert_eq!(why, "it is a link, or not a file of yours.");
    let tar = dir.join("a.tar");
    let st = Command::new("bsdtar")
        .arg("-cf")
        .arg(&tar)
        .arg("-C")
        .arg(dir.join("src"))
        .arg("a.txt")
        .status()
        .expect("bsdtar");
    assert!(st.success());
    assert!(!zipwrite::is_zip(&tar));
    let why = rewrite(&tar, &add(&dir.join("src"), &["a.txt"])).expect_err("a tar");
    assert_eq!(why, "only a zip compressed folder can be written into.");
    let small = Bound {
        most: Some(0),
        deadline: None,
    };
    let why = zipwrite::rewrite(
        &zip,
        &add(&dir.join("src"), &["a.txt"]),
        &Arc::default(),
        small,
    )
    .expect_err("over the bound");
    assert!(why.starts_with("it holds "), "{why}");
    assert_eq!(std::fs::read(&zip).expect("read"), before);
    assert!(temps(&zip).is_empty());
}

#[test]
fn an_encrypted_zip_is_refused() {
    let dir = fresh(&[("secret.txt", b"secret")]);
    let zip = dir.join("locked.zip");
    let made = Command::new("bsdtar")
        .args([
            "-c",
            "--format",
            "zip",
            "--options",
            "zip:encryption=zipcrypt",
        ])
        .args(["--passphrase", "pw", "-f"])
        .arg(&zip)
        .arg("-C")
        .arg(dir.join("src"))
        .arg("secret.txt")
        .status()
        .expect("bsdtar");
    if !made.success() {
        return;
    }
    let why = zipwrite::update_entry(&zip, Path::new("secret.txt"), b"x").expect_err("locked");
    assert_eq!(why, "it is password-protected.");
}

/// A zip holding `mb` MiB that does not compress, so rewriting it takes a while.
fn big_zip(mb: usize) -> (PathBuf, PathBuf) {
    let mut bytes = vec![0u8; mb << 20];
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    for b in &mut bytes {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    let dir = fresh(&[("noise.bin", &bytes), ("small.txt", b"small")]);
    let zip = zip_of(&dir, &["noise.bin", "small.txt"]);
    (dir, zip)
}

/// Polls for a temp file beside `zip` for up to a minute.
fn wait_for_temp(zip: &Path) -> bool {
    let until = Instant::now() + Duration::from_secs(60);
    while Instant::now() < until {
        if !temps(zip).is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

#[test]
fn a_cancelled_add_leaves_the_zip_as_it_was_and_no_temp_file() {
    let (dir, zip) = big_zip(96);
    let before = std::fs::read(&zip).expect("read");
    let mut job = ArchiveJob::add_to_zip(zip.clone(), add(&dir.join("src"), &["small.txt"]), || {});
    assert!(wait_for_temp(&zip), "the rewrite started");
    job.meter().cancel();
    let until = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    while !job.is_finished() && Instant::now() < until {
        events.extend(job.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(events.as_slice(), [ArchiveEvent::Finished]),
        "{events:?}"
    );
    assert_eq!(std::fs::read(&zip).expect("read"), before);
    assert!(temps(&zip).is_empty(), "Cancel removes the temp file");
}

#[test]
fn a_crash_mid_rewrite_leaves_the_zip_whole_and_its_temp_file_beside_it() {
    if let Some(zip) = std::env::var_os("ZIP_WRITE_CRASH") {
        home();
        let zip = PathBuf::from(zip);
        let src = zip.parent().expect("a folder").join("src");
        let _ = rewrite(&zip, &add(&src, &["small.txt"]));
        return;
    }
    let (_dir, zip) = big_zip(96);
    let before = std::fs::read(&zip).expect("read");
    let mut child = Command::new(std::env::current_exe().expect("this test"))
        .args([
            "--exact",
            "a_crash_mid_rewrite_leaves_the_zip_whole_and_its_temp_file_beside_it",
        ])
        .env("ZIP_WRITE_CRASH", &zip)
        .spawn()
        .expect("run the rewrite");
    let started = wait_for_temp(&zip);
    let _ = child.kill();
    let _ = child.wait();
    assert!(started, "the rewrite started");
    assert_eq!(
        std::fs::read(&zip).expect("read"),
        before,
        "the zip is whole"
    );
    assert_eq!(temps(&zip).len(), 1, "the temp file stays");
}
