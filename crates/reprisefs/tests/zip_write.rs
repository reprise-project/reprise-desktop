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
            n.starts_with(&format!(".{name}.")) && n.ends_with(".zipwrite")
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
    zipwrite::rewrite(zip, adds, &Arc::new(Meter::default()), Bound::default()).map_err(|e| e.why())
}

/// The events of `job` until it finishes, for up to a minute.
fn wait(job: &mut ArchiveJob) -> Vec<ArchiveEvent> {
    let until = Instant::now() + Duration::from_secs(60);
    let mut events = Vec::new();
    while !job.is_finished() {
        assert!(Instant::now() < until, "the job ends");
        events.extend(job.poll());
        std::thread::sleep(Duration::from_millis(5));
    }
    events
}

/// A save of `bytes` over `inner` of `zip` on its job: the error box's text when it failed.
fn update(zip: &Path, inner: &str, bytes: &[u8]) -> Result<(), String> {
    let mut job = ArchiveJob::update_entry(zip.into(), inner.into(), bytes.to_vec(), None, || {});
    match wait(&mut job).as_slice() {
        [ArchiveEvent::Landed { .. }, ArchiveEvent::Finished] => Ok(()),
        [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished] => Err(text.clone()),
        other => panic!("{other:?}"),
    }
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
    update(&zip, "sub/notes.txt", b"new and longer text").expect("updated");
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
fn a_file_over_a_folder_or_a_folder_over_a_file_is_refused_and_the_zip_left_alone() {
    let dir = fresh(&[
        ("docs/a.txt", b"a"),
        ("implied/b.txt", b"b"),
        ("notes", b"n"),
    ]);
    let zip = zip_of(&dir, &["docs", "docs/a.txt", "implied/b.txt", "notes"]);
    let before = std::fs::read(&zip).expect("read");
    let files = fresh(&[("docs", b"a file now"), ("implied", b"a file too")]);
    let folders = fresh(&[("notes/inside.txt", b"a folder now")]);
    let occupied = "There is already a file with the same name in this location.";
    for (src, name) in [(&files, "docs"), (&files, "implied"), (&folders, "notes")] {
        let why = rewrite(&zip, &add(&src.join("src"), &[name])).expect_err(name);
        assert_eq!(why, occupied, "{name}");
    }
    let why = update(&zip, "docs", b"saved over a folder").expect_err("a save over a folder");
    assert_eq!(why, format!("Cannot update a.zip: {occupied}"));
    assert_eq!(std::fs::read(&zip).expect("read"), before);
    assert!(temps(&zip).is_empty());
}

#[test]
fn two_added_items_of_one_path_are_refused() {
    let dir = fresh(&[("kept.txt", b"kept")]);
    let zip = zip_of(&dir, &["kept.txt"]);
    let a = fresh(&[("report.pdf", b"a")]);
    let b = fresh(&[("report.pdf", b"b")]);
    let adds = [
        add(&a.join("src"), &["report.pdf"]),
        add(&b.join("src"), &["report.pdf"]),
    ]
    .concat();
    let why = rewrite(&zip, &adds).expect_err("two of one path");
    assert_eq!(
        why,
        "There is already a file with the same name in this location."
    );
    assert_eq!(listing(&zip), vec![("kept.txt".into(), Kind::File, 4)]);
}

#[test]
fn a_saved_file_keeps_the_mode_of_the_entry_it_replaces() {
    let dir = fresh(&[("run.sh", b"echo old")]);
    let src = dir.join("src/run.sh");
    std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o750)).expect("chmod");
    let zip = zip_of(&dir, &["run.sh"]);
    update(&zip, "run.sh", b"echo new").expect("updated");
    assert_eq!(read_entry(&zip, "run.sh"), b"echo new");
    let out = Command::new("bsdtar")
        .arg("-tvf")
        .arg(&zip)
        .output()
        .expect("bsdtar");
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(listed.starts_with("-rwxr-x--- "), "{listed}");
}

#[test]
fn a_zip_past_the_save_bound_is_refused_before_it_is_read() {
    let dir = fresh(&[]);
    let zip = dir.join("huge.zip");
    let f = std::fs::File::create(&zip).expect("create");
    std::io::Write::write_all(&mut &f, b"PK\x03\x04").expect("write");
    f.set_len(zipwrite::UPDATE_MOST + 1).expect("a sparse zip");
    let why = update(&zip, "notes.txt", b"x").expect_err("too large");
    assert!(
        why.starts_with("Cannot update huge.zip: it holds "),
        "{why}"
    );
}

#[test]
fn a_rewrite_keeps_the_zips_xattrs() {
    let dir = fresh(&[("a.txt", b"a")]);
    let zip = zip_of(&dir, &["a.txt"]);
    let set = Command::new("setfattr")
        .args(["-n", "user.kept", "-v", "yes"])
        .arg(&zip)
        .status();
    if !set.is_ok_and(|s| s.success()) {
        return;
    }
    update(&zip, "a.txt", b"b").expect("updated");
    let got = Command::new("getfattr")
        .args(["--only-values", "-n", "user.kept"])
        .arg(&zip)
        .output()
        .expect("getfattr");
    assert_eq!(got.stdout, b"yes");
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
    .expect_err("over the bound")
    .why();
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
    let why = update(&zip, "secret.txt", b"x").expect_err("locked");
    assert_eq!(why, "Cannot update locked.zip: it is password-protected.");
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
    let events = wait(&mut job);
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
    let left = temps(&zip);
    assert_eq!(left.len(), 1, "the temp file stays");
    let young = rewrite(
        &zip,
        &add(&zip.parent().expect("a folder").join("src"), &["small.txt"]),
    );
    young.expect("a rewrite past a young temp file");
    assert_eq!(
        temps(&zip),
        left,
        "a temp file written to within the deadline stays"
    );
    let stale = std::time::SystemTime::now() - Duration::from_secs(86_400);
    let f = std::fs::File::options()
        .write(true)
        .open(&left[0])
        .expect("open");
    f.set_modified(stale).expect("age it");
    drop(f);
    rewrite(
        &zip,
        &add(&zip.parent().expect("a folder").join("src"), &["small.txt"]),
    )
    .expect("a rewrite past a stale temp file");
    assert!(
        temps(&zip).is_empty(),
        "the next rewrite removes a stale temp file"
    );
}

#[test]
fn a_job_stopped_for_a_quit_removes_its_temp_file_before_it_returns() {
    let (dir, zip) = big_zip(96);
    let mut job = ArchiveJob::add_to_zip(zip.clone(), add(&dir.join("src"), &["small.txt"]), || {});
    assert!(wait_for_temp(&zip), "the rewrite started");
    job.stop_and_wait(Duration::from_secs(30));
    assert!(temps(&zip).is_empty(), "the wait outlasts the temp file");
}

#[test]
fn a_read_only_zip_is_refused_and_left_alone() {
    let dir = fresh(&[("a.txt", b"a")]);
    let zip = zip_of(&dir, &["a.txt"]);
    std::fs::set_permissions(&zip, std::fs::Permissions::from_mode(0o444)).expect("chmod");
    let before = std::fs::read(&zip).expect("read");
    let why = update(&zip, "a.txt", b"b").expect_err("read-only");
    assert_eq!(why, "Cannot update a.zip: it is read-only.");
    assert_eq!(std::fs::read(&zip).expect("read"), before);
}

#[test]
fn an_entry_stored_without_owner_read_still_saves() {
    let dir = fresh(&[("locked.txt", b"old")]);
    let src = dir.join("src/locked.txt");
    std::fs::set_permissions(&src, std::fs::Permissions::from_mode(0o440)).expect("chmod");
    let zip = zip_of(&dir, &["locked.txt"]);
    let out = Command::new("bsdtar")
        .arg("-tvf")
        .arg(&zip)
        .output()
        .expect("bsdtar");
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("-r--r----- "));
    // Stored as 0040: the group may read it and the owner may not.
    let patched = std::fs::read(&zip).expect("read");
    let patched = patch_modes(patched, 0o100040);
    std::fs::write(&zip, patched).expect("write");
    update(&zip, "locked.txt", b"new").expect("saved");
    assert_eq!(read_entry(&zip, "locked.txt"), b"new");
}

/// `zip` with every central directory entry's external attributes giving unix `mode`.
fn patch_modes(mut zip: Vec<u8>, mode: u32) -> Vec<u8> {
    let mut at = 0;
    while let Some(i) = zip[at..].windows(4).position(|w| w == b"PK\x01\x02") {
        let e = at + i;
        zip[e + 38..e + 42].copy_from_slice(&(mode << 16).to_le_bytes());
        at = e + 4;
    }
    zip
}

#[test]
fn a_rewrite_sweeps_stale_temp_files_of_any_zip_in_its_folder() {
    let dir = fresh(&[("a.txt", b"a")]);
    let zip = zip_of(&dir, &["a.txt"]);
    let stale = std::time::SystemTime::now() - Duration::from_secs(86_400);
    let made = |name: &str| {
        let p = dir.join(name);
        let f = std::fs::File::create(&p).expect("create");
        f.set_modified(stale).expect("age it");
        p
    };
    let other = made(".Other.ZIP.0123456789abcdef.zipwrite");
    let text = made(".notes.txt.0123456789abcdef.zipwrite");
    let (f, saved) = reprisefs::ops::create_temp_beside(&zip, None).expect("a save's temp file");
    f.set_modified(stale).expect("age it");
    drop(f);
    let fresh_one = dir.join(".Third.zip.fedcba9876543210.zipwrite");
    std::fs::write(&fresh_one, b"").expect("create");
    rewrite(&zip, &add(&dir.join("src"), &["a.txt"])).expect("rewritten");
    assert!(!other.exists(), "another zip's stale temp file goes");
    assert!(
        text.exists(),
        "a rewrite's temp file of a file not named as a zip stays"
    );
    assert!(
        saved.exists(),
        "a document save's temp file beside the zip stays"
    );
    assert!(
        fresh_one.exists(),
        "a temp file written to within the deadline stays"
    );
}
