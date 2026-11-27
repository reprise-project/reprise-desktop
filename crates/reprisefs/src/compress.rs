//! Compressing items into a zip with `bsdtar`. The zip is written under a temp name beside it and
//! renamed once whole, so a cancelled or failed run leaves no half zip.

use crate::extract::{self, ArchiveEvent, ArchiveJob, Failure};
use crate::ops;
use crate::progress::{Meter, Phase, Totals};
use reprisekit::tr;
use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The caption of a compress error box.
pub fn failure_title() -> String {
    tr!("compress-failed-title")
}
const SAMPLE_EVERY: Duration = Duration::from_millis(100);

/// Items of one folder going into the zip `zip`, numbered past a name taken by the time it lands.
#[derive(Clone, Debug)]
pub struct Compress {
    pub paths: Vec<PathBuf>,
    pub zip: PathBuf,
}

/// The zip's name for `paths`, items of one folder: one item's name (a file's less its
/// extension), or for several the folder's name.
pub fn zip_name(paths: &[PathBuf], one_folder: bool) -> OsString {
    let base = match paths {
        [p] if one_folder => p.file_name(),
        [p] => p.file_stem(),
        _ => paths.first().and_then(|p| p.parent()?.file_name()),
    };
    let mut name = base
        .filter(|n| !n.is_empty())
        .unwrap_or(OsStr::new("Archive"))
        .to_os_string();
    name.push(".zip");
    name
}

/// Compress to's and Send To's job for `paths`: the [`zip_name`] beside them, numbered past a
/// name `taken` refuses. None for an empty selection, the root, or items of several folders,
/// which one zip beside them cannot hold by their names.
pub fn target(
    paths: &[PathBuf],
    one_folder: bool,
    taken: impl Fn(&OsStr) -> bool,
) -> Option<Compress> {
    let dir = paths.first()?.parent()?;
    if paths.iter().any(|p| p.parent() != Some(dir)) {
        return None;
    }
    let name = crate::names::first_free(&zip_name(paths, one_folder), taken);
    Some(Compress {
        zip: dir.join(name),
        paths: paths.to_vec(),
    })
}

/// Whether `paths` is one folder as a listing reads it ([`Item::is_folder`]): a symlink to a
/// folder is none, and the zip holds the link.
///
/// [`Item::is_folder`]: crate::listing::Item::is_folder
pub fn one_folder(paths: &[PathBuf]) -> bool {
    matches!(paths, [p] if std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()))
}

/// bsdtar writing `paths`, items of one folder, as a zip into `to`, one `a name` line on stderr
/// per entry. The format is named because `to` is a temp name without the `.zip` suffix.
pub fn compress_argv(paths: &[PathBuf], to: &Path) -> Vec<OsString> {
    let (dir, names) = ops::members(paths);
    let mut argv: Vec<OsString> = ["bsdtar", "-c", "-v", "--format", "zip", "-f"]
        .map(OsString::from)
        .into();
    argv.extend([to.into(), "-C".into(), dir.into(), "--".into()]);
    argv.extend(names);
    argv
}

impl ArchiveJob {
    /// Counts the entries and bytes under `c`'s items, then zips them on a thread: one
    /// [`ArchiveEvent::Landed`] with the zip, or [`ArchiveEvent::Failed`]. A cancelled job sends
    /// neither.
    pub fn compress(c: Compress, wake: impl Fn() + Send + 'static) -> ArchiveJob {
        let meter = Arc::new(Meter::default());
        ArchiveJob::on_thread("reprisefs-compress", meter, wake, move |m, send| {
            run(&c, m, None, send)
        })
    }
}

/// The job's body; `path` replaces the `PATH` bsdtar is looked up in.
fn run(c: &Compress, m: &Meter, path: Option<&OsStr>, send: &dyn Fn(ArchiveEvent) -> bool) {
    let ev = match zip(c, m, path) {
        Ok(into) => Some(ArchiveEvent::Landed { index: 0, into }),
        Err(_) if m.cancelled() => None,
        Err(e) => {
            let name = c.zip.file_name().unwrap_or_default().to_string_lossy();
            let why = match e {
                Failure::Tool(why)
                | Failure::Climbs(why)
                | Failure::Refused(why)
                | Failure::Kept(why) => why,
                Failure::Cancelled => tr!("compress-cancelled"),
            };
            Some(ArchiveEvent::Failed {
                index: 0,
                text: tr!("compress-failed", name = &*name, why = &*why),
            })
        }
    };
    if let Some(ev) = ev
        && !send(ev)
    {
        return;
    }
    send(ArchiveEvent::Finished);
}

fn zip(c: &Compress, m: &Meter, path: Option<&OsStr>) -> Result<PathBuf, Failure> {
    let io_err = |e: io::Error| Failure::Tool(e.to_string());
    m.set_current(c.zip.file_name().unwrap_or_default());
    let mut total = Totals::default();
    for p in &c.paths {
        total = total + count(p, m).ok_or(Failure::Cancelled)?;
    }
    m.add_totals(total);
    m.set_phase(Phase::Copying);
    let (file, tmp) = ops::create_temp_beside(&c.zip, None).map_err(io_err)?;
    drop(file);
    let undo = |e: Failure| {
        if let Err(err) = std::fs::remove_file(&tmp) {
            log::warn!("reprisefs: cannot remove {}: {err}", tmp.display());
        }
        e
    };
    let argv = compress_argv(&c.paths, &tmp);
    let mut child = extract::spawn(&argv, false, path).map_err(undo)?;
    let err = (child.stderr.take())
        .ok_or_else(extract::no_pipe)
        .map_err(undo)?;
    let pid = child.id();
    let done = Cell::new(Totals::default());
    let settle = |t: Totals| {
        done.set(t);
        m.settle(t);
    };
    let mut first = None;
    let each = |line: &[u8]| {
        // An entry bsdtar fails to write ends its `a name` line with the error.
        let (line, said) = match line.windows(8).rposition(|w| w == b"bsdtar: ") {
            Some(at) if at > 0 => line.split_at(at),
            _ => (line, &b""[..]),
        };
        if !said.is_empty() {
            first.get_or_insert_with(|| said.to_vec());
        }
        match line.strip_prefix(b"a ") {
            Some(entry) => {
                m.set_current(OsStr::from_bytes(entry));
                let d = done.get();
                settle(Totals {
                    items: (d.items + 1).min(total.items),
                    ..d
                });
            }
            None if !String::from_utf8_lossy(line).contains(extract::DELAYED) => {
                first.get_or_insert_with(|| line.to_vec());
            }
            None => {}
        }
        true
    };
    // bsdtar ends an entry's line only once its data is written, so the bar follows the bytes it
    // has read instead.
    let mut sampled = Instant::now();
    let tick = || {
        if sampled.elapsed() < SAMPLE_EVERY {
            return;
        }
        sampled = Instant::now();
        if let Some(read) = bytes_read(pid) {
            let d = done.get();
            settle(Totals {
                bytes: read.min(total.bytes),
                ..d
            });
        }
    };
    let read = extract::lines_ticking(&mut child, err, m, each, tick);
    let ok = extract::finish(child, &read).map_err(undo)?;
    if !ok {
        let why = reason(&first.unwrap_or_default(), &c.paths, m);
        return Err(undo(Failure::Tool(why)));
    }
    ops::rename_numbered(&tmp, &c.zip).map_err(|e| undo(io_err(e)))
}

/// The failure box's reason from bsdtar's first error line `said`: its entry [`locate`]d, then
/// how many other entries under `paths` do not open. bsdtar gives up on an item at its first
/// such entry, so the others are found by looking.
fn reason(said: &[u8], paths: &[PathBuf], m: &Meter) -> String {
    let mut shut = Vec::new();
    for p in paths {
        unreadable(p, m, &mut shut);
    }
    let why = locate(extract::said(said, "bsdtar stopped."), paths, &shut);
    let dir = ops::members(paths).0;
    let others = (shut.iter())
        .map(|p| p.strip_prefix(dir).unwrap_or(p).display().to_string())
        .filter(|s| !why.starts_with(&format!("{s}: ")) && !why.contains(&format!("'{s}'")))
        .count();
    extract::and_others(why, others, "read")
}

/// bsdtar's reason with the entry it quotes by its file name alone (`` Can't open `b.txt' ``)
/// quoted by its path under the zipped items' folder instead: the first entry of that name in
/// `shut`, the entries that do not open, else the first under `paths`. Unchanged when none has
/// the name.
fn locate(why: String, paths: &[PathBuf], shut: &[PathBuf]) -> String {
    let Some((head, rest)) = why.split_once('`') else {
        return why;
    };
    let Some((name, tail)) = rest.split_once('\'').filter(|(name, _)| !name.is_empty()) else {
        return why;
    };
    let name = Path::new(name);
    let found = (shut.iter().find(|p| p.ends_with(name)).cloned())
        .or_else(|| paths.iter().find_map(|p| named(p, name)));
    let Some(found) = found else {
        return why;
    };
    let dir = ops::members(paths).0;
    let shown = found.strip_prefix(dir).unwrap_or(&found);
    format!("{head}'{}'{tail}", shown.display())
}

/// The first entry at or under `path` whose path ends with `name`. Links are not followed.
fn named(path: &Path, name: &Path) -> Option<PathBuf> {
    if path.ends_with(name) {
        return Some(path.to_path_buf());
    }
    if !path.symlink_metadata().ok()?.is_dir() {
        return None;
    }
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|e| named(&e.path(), name))
}

/// Adds to `shut` the entries at or under `path` that do not open: files that do not open for
/// reading and folders that do not list, whose contents stay unseen. Links are not followed and
/// other kinds are not opened. Stops once `m` is cancelled.
fn unreadable(path: &Path, m: &Meter, shut: &mut Vec<PathBuf>) {
    let Ok(meta) = path.symlink_metadata() else {
        return;
    };
    if meta.is_file() && std::fs::File::open(path).is_err() {
        shut.push(path.to_path_buf());
    }
    if !meta.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        shut.push(path.to_path_buf());
        return;
    };
    for e in entries.flatten() {
        if m.cancelled() {
            return;
        }
        unreadable(&e.path(), m, shut);
    }
}

/// The entries bsdtar writes for `path`, it and everything under it when a folder, and their
/// files' bytes. Links are not followed. `None` once `m` is cancelled.
fn count(path: &Path, m: &Meter) -> Option<Totals> {
    let Ok(meta) = path.symlink_metadata() else {
        return Some(Totals::default());
    };
    let mut n = Totals {
        bytes: if meta.is_file() { meta.len() } else { 0 },
        items: 1,
    };
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            if m.cancelled() {
                return None;
            }
            n = n + count(&entry.path(), m)?;
        }
    }
    Some(n)
}

/// The bytes process `pid` and its reaped children have read, `rchar` in `/proc/<pid>/io`.
fn bytes_read(pid: u32) -> Option<u64> {
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    io.lines()
        .find_map(|l| l.strip_prefix("rchar:"))?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;

    fn events(c: &Compress, m: &Meter, path: Option<&OsStr>) -> Vec<ArchiveEvent> {
        let (tx, rx) = mpsc::channel();
        run(c, m, path, &|ev| tx.send(ev).is_ok());
        rx.try_iter().collect()
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// A folder holding one executable `bsdtar` running `script`, and a `PATH` finding it first.
    fn fake_bsdtar(script: &str) -> (tempfile::TempDir, OsString) {
        let bin = tempfile::tempdir().unwrap();
        let tool = bin.path().join("bsdtar");
        std::fs::write(&tool, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut path = bin.path().as_os_str().to_owned();
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        (bin, path)
    }

    /// Sets `f` to the argument after `-f`.
    const TARGET: &str = r#"while [ $# -gt 0 ]; do [ "$1" = -f ] && f=$2; shift; done"#;

    #[test]
    fn the_zip_is_named_after_one_item_or_the_folder_of_several() {
        let one = |p: &str, dir| zip_name(&[PathBuf::from(p)], dir);
        assert_eq!(one("/home/u/report.txt", false), "report.zip");
        assert_eq!(one("/home/u/a.tar.gz", false), "a.tar.zip");
        assert_eq!(
            one("/home/u/v1.2", true),
            "v1.2.zip",
            "a folder keeps its dot"
        );
        assert_eq!(one("/home/u/.bashrc", false), ".bashrc.zip");
        let several: Vec<PathBuf> = vec!["/home/u/Photos/a.jpg".into(), "/home/u/Photos/b".into()];
        assert_eq!(zip_name(&several, false), "Photos.zip");
        assert_eq!(
            zip_name(&["/a".into(), "/b".into()], false),
            "Archive.zip",
            "the root has no name"
        );
    }

    #[test]
    fn the_target_is_numbered_past_a_taken_name_beside_the_items() {
        let taken = |names: &'static [&'static str]| move |c: &OsStr| names.iter().any(|n| c == *n);
        let several: Vec<PathBuf> = vec!["/d/P/b2.txt".into(), "/d/P/a1.txt".into()];
        let c = target(&several, false, taken(&["P.zip", "b2.zip"])).unwrap();
        assert_eq!(c.zip, Path::new("/d/P/P (2).zip"));
        assert_eq!(c.paths, several);
        let one = target(&["/d/P/b2.txt".into()], false, taken(&["b2.zip"])).unwrap();
        assert_eq!(one.zip, Path::new("/d/P/b2 (2).zip"));
        assert!(target(&[], false, taken(&[])).is_none());
        assert!(target(&["/".into()], true, taken(&[])).is_none());
        let apart: Vec<PathBuf> = vec!["/a/x".into(), "/b/y".into()];
        assert!(
            target(&apart, false, taken(&[])).is_none(),
            "items of two folders make no zip"
        );
    }

    #[test]
    fn a_link_to_a_folder_is_no_folder_as_the_listing_reads_it() {
        let t = tempfile::tempdir().unwrap();
        let (dir, link) = (t.path().join("my.app"), t.path().join("link.app"));
        std::fs::create_dir(&dir).unwrap();
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        let db = crate::apps::MimeDb::empty();
        for p in [&dir, &link] {
            let listed = crate::listing::item_at(p, &db).is_folder();
            assert_eq!(one_folder(std::slice::from_ref(p)), listed, "{p:?}");
        }
        for (p, zip) in [(link, "link.zip"), (dir, "my.app.zip")] {
            let one = std::slice::from_ref(&p);
            assert_eq!(zip_name(one, one_folder(one)), zip);
        }
    }

    #[test]
    fn a_name_that_is_not_utf8_keeps_its_bytes() {
        let p = PathBuf::from(OsStr::from_bytes(b"/d/a\xffb.txt"));
        let c = target(&[p], false, |c| c == OsStr::from_bytes(b"a\xffb.zip")).unwrap();
        assert_eq!(c.zip, Path::new(OsStr::from_bytes(b"/d/a\xffb (2).zip")));
    }

    #[test]
    fn the_argv_writes_a_verbose_zip_to_the_temp_name() {
        let argv = compress_argv(
            &["/d/-x".into(), "/d/b c".into()],
            Path::new("/d/.b.zip.1.tmp"),
        );
        assert_eq!(
            argv,
            [
                "bsdtar",
                "-c",
                "-v",
                "--format",
                "zip",
                "-f",
                "/d/.b.zip.1.tmp",
                "-C",
                "/d",
                "--",
                "./-x",
                "b c"
            ]
        );
    }

    #[test]
    fn a_folder_and_a_file_land_in_one_zip_numbered_past_a_taken_name() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("Docs/sub")).unwrap();
        std::fs::write(d.join("Docs/sub/b.txt"), "b").unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::write(d.join("Docs.zip"), "taken").unwrap();
        let c = Compress {
            paths: vec![d.join("Docs"), d.join("a.txt")],
            zip: d.join("Docs.zip"),
        };
        let m = Meter::default();
        let got = events(&c, &m, None);
        let zip = d.join("Docs (2).zip");
        assert!(
            matches!(&got[..], [ArchiveEvent::Landed { into, .. }, ArchiveEvent::Finished] if *into == zip),
            "{got:?}"
        );
        assert_eq!(names_in(d), ["Docs", "Docs (2).zip", "Docs.zip", "a.txt"]);
        let s = m.snapshot();
        assert_eq!((s.items, s.total_items), (4, 4), "Docs, sub, b.txt, a.txt");
        assert_eq!(s.total_bytes, 2, "the files' bytes");
        let listed = std::process::Command::new("bsdtar")
            .arg("-tf")
            .arg(&zip)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&listed.stdout),
            "Docs/\nDocs/sub/\nDocs/sub/b.txt\na.txt\n"
        );
    }

    #[test]
    fn a_failing_bsdtar_leaves_no_zip_and_says_why() {
        let delayed = "echo 'bsdtar: Error exit delayed from previous errors.' >&2";
        for (said, why) in [
            (
                "printf \"bsdtar: Can't open \\140a.txt': Permission denied\\n\" >&2",
                "Can't open 'a.txt': Permission denied",
            ),
            ("printf 'a a.txtbsdtar: Write error\\n' >&2", "Write error"),
            ("", "bsdtar stopped."),
        ] {
            let t = tempfile::tempdir().unwrap();
            let d = t.path();
            std::fs::write(d.join("a.txt"), "a").unwrap();
            let (_bin, path) = fake_bsdtar(&format!(
                "{TARGET}\necho half > \"$f\"\n{said}\n{delayed}\nexit 1"
            ));
            let c = Compress {
                paths: vec![d.join("a.txt")],
                zip: d.join("a.zip"),
            };
            let got = events(&c, &Meter::default(), Some(&path));
            let want = format!("Cannot compress a.zip: {why}");
            assert!(
                matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                    if *text == want),
                "{got:?}"
            );
            assert_eq!(names_in(d), ["a.txt"], "the temp file is removed");
        }
    }

    #[test]
    fn an_unreadable_entry_is_named_by_its_path_in_the_folder() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("Docs/sub")).unwrap();
        std::fs::write(d.join("Docs/secret.txt"), "kept").unwrap();
        let secret = d.join("Docs/sub/secret.txt");
        std::fs::write(&secret, "shut").unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o000)).unwrap();
        let c = Compress {
            paths: vec![d.join("Docs"), d.join("a.txt")],
            zip: d.join("Docs.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o644)).unwrap();
        let want = "Cannot compress Docs.zip: Can't open 'Docs/sub/secret.txt': Permission denied";
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text == want),
            "{got:?}"
        );
        assert_eq!(names_in(d), ["Docs", "a.txt"]);
    }

    #[test]
    fn a_folder_that_does_not_open_fails_the_zip() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("Docs/locked")).unwrap();
        std::fs::write(d.join("Docs/locked/f.txt"), "shut").unwrap();
        std::fs::write(d.join("Docs/open.txt"), "open").unwrap();
        let locked = d.join("Docs/locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let c = Compress {
            paths: vec![d.join("Docs")],
            zip: d.join("Docs.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if is_root() {
            return;
        }
        let want =
            "Cannot compress Docs.zip: Docs/locked: Couldn't visit directory: Permission denied";
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text == want),
            "{got:?}"
        );
        assert_eq!(names_in(d), ["Docs"], "no zip or temp file is left");
    }

    fn is_root() -> bool {
        // SAFETY: geteuid takes no arguments and cannot fail.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn a_folder_with_two_that_do_not_open_names_one_and_counts_the_other() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for sub in ["a", "b"] {
            std::fs::create_dir_all(d.join("Docs").join(sub)).unwrap();
            std::fs::write(d.join("Docs").join(sub).join("f.txt"), "shut").unwrap();
        }
        let shut = [d.join("Docs/a"), d.join("Docs/b")];
        for p in &shut {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let c = Compress {
            paths: vec![d.join("Docs")],
            zip: d.join("Docs.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        for p in &shut {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        if is_root() {
            return;
        }
        let want = |sub| {
            format!(
                "Cannot compress Docs.zip: Docs/{sub}: Couldn't visit directory: Permission denied, \
                 and 1 other item could not be read"
            )
        };
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if *text == want("a") || *text == want("b")),
            "{got:?}"
        );
        assert_eq!(names_in(d), ["Docs"]);
    }

    #[test]
    fn three_items_that_do_not_open_name_the_first_and_count_the_others() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for sub in ["x", "y", "z"] {
            std::fs::create_dir(d.join(sub)).unwrap();
            std::fs::write(d.join(sub).join("f.txt"), "f").unwrap();
        }
        std::fs::write(d.join("w.txt"), "shut").unwrap();
        let shut = [d.join("x"), d.join("y"), d.join("w.txt")];
        for p in &shut {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o000)).unwrap();
        }
        let c = Compress {
            paths: ["x", "y", "z", "w.txt"].map(|n| d.join(n)).into(),
            zip: d.join("x.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        for p in &shut {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        if is_root() {
            return;
        }
        let want = "Cannot compress x.zip: x: Couldn't visit directory: Permission denied, \
                    and 2 other items could not be read";
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text == want),
            "{got:?}"
        );
        assert_eq!(names_in(d), ["w.txt", "x", "y", "z"]);
    }

    #[test]
    fn a_link_to_a_folder_that_does_not_open_is_zipped_as_the_link() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("Docs")).unwrap();
        std::fs::create_dir(d.join("shut")).unwrap();
        std::fs::write(d.join("shut/f.txt"), "shut").unwrap();
        std::os::unix::fs::symlink("../shut", d.join("Docs/link")).unwrap();
        std::fs::set_permissions(d.join("shut"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let c = Compress {
            paths: vec![d.join("Docs")],
            zip: d.join("Docs.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        std::fs::set_permissions(d.join("shut"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(
                &got[..],
                [ArchiveEvent::Landed { .. }, ArchiveEvent::Finished]
            ),
            "{got:?}"
        );
        let listed = std::process::Command::new("bsdtar")
            .args(["-tvf".as_ref(), d.join("Docs.zip").as_os_str()])
            .output()
            .unwrap();
        let listed = String::from_utf8_lossy(&listed.stdout);
        assert!(listed.contains("Docs/link -> ../shut"), "{listed}");
        assert!(!listed.contains("f.txt"), "{listed}");
    }

    #[test]
    fn a_quoted_name_under_none_of_the_items_is_left_as_bsdtar_said() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.txt");
        std::fs::write(&a, "a").unwrap();
        let why = "Can't open `gone.txt': No such file or directory";
        assert_eq!(locate(why.into(), &[a], &[]), why);
    }

    #[test]
    fn a_cancelled_run_stops_bsdtar_and_removes_the_temp_file() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().to_path_buf();
        std::fs::write(d.join("big"), "x").unwrap();
        let (_bin, path) = fake_bsdtar(&format!(
            "{TARGET}\necho half > \"$f\"\necho 'a big' >&2\nexec sleep 30"
        ));
        let c = Compress {
            paths: vec![d.join("big")],
            zip: d.join("big.zip"),
        };
        let m = Arc::new(Meter::default());
        let (tx, rx) = mpsc::channel();
        let m2 = m.clone();
        let worker = std::thread::spawn(move || {
            run(&c, &m2, Some(&path), &|ev| tx.send(ev).is_ok());
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while m.snapshot().items == 0 {
            assert!(std::time::Instant::now() < deadline, "bsdtar starts");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(names_in(&d).len(), 2, "the temp file is being written");
        m.cancel();
        let ev = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("the job ends without waiting for bsdtar");
        assert!(matches!(ev, ArchiveEvent::Finished), "{ev:?}");
        worker.join().unwrap();
        assert_eq!(names_in(&d), ["big"]);
    }

    #[test]
    fn the_bar_counts_the_bytes_bsdtar_has_read_before_its_entry_line() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().to_path_buf();
        let big = d.join("big");
        std::fs::write(&big, vec![7u8; 4 << 20]).unwrap();
        let (_bin, path) = fake_bsdtar(&format!(
            "{TARGET}\ncat '{}' > \"$f\"\nexec sleep 30",
            big.display()
        ));
        let c = Compress {
            paths: vec![big],
            zip: d.join("big.zip"),
        };
        let m = Arc::new(Meter::default());
        let (tx, rx) = mpsc::channel();
        let m2 = m.clone();
        let worker = std::thread::spawn(move || {
            run(&c, &m2, Some(&path), &|ev| tx.send(ev).is_ok());
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while m.snapshot().bytes < 4 << 20 {
            assert!(
                std::time::Instant::now() < deadline,
                "the bytes read reach 4 MiB, at {:?}",
                m.snapshot()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let s = m.snapshot();
        assert_eq!((s.bytes, s.total_bytes), (4 << 20, 4 << 20));
        assert_eq!((s.items, s.total_items), (0, 1), "no entry line yet");
        m.cancel();
        let ev = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
        assert!(matches!(ev, ArchiveEvent::Finished), "{ev:?}");
        worker.join().unwrap();
    }

    #[test]
    fn a_read_only_folder_fails_before_bsdtar_runs() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path().join("ro");
        std::fs::create_dir(&d).unwrap();
        std::fs::write(d.join("a.txt"), "a").unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let c = Compress {
            paths: vec![d.join("a.txt")],
            zip: d.join("a.zip"),
        };
        let got = events(&c, &Meter::default(), None);
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text.starts_with("Cannot compress a.zip: Permission denied")),
            "{got:?}"
        );
        assert_eq!(names_in(&d), ["a.txt"]);
    }
}
