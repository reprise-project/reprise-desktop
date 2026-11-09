//! Compressing items into a zip with `bsdtar`. The zip is written under a temp name beside it and
//! renamed once whole, so a cancelled or failed run leaves no half zip.

use crate::extract::{self, ArchiveEvent, ArchiveJob, Failure};
use crate::names;
use crate::ops;
use crate::progress::{Meter, Phase, Totals};
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const FAILURE_TITLE: &str = "Error Compressing File";

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
    /// Counts the entries under `c`'s items, then zips them on a thread: one
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
                Failure::Tool(why) | Failure::Climbs(why) | Failure::Refused(why) => why,
                Failure::Cancelled => "the operation was cancelled.".into(),
            };
            Some(ArchiveEvent::Failed {
                index: 0,
                text: format!("Cannot compress {name}: {why}"),
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
    let mut total = 0;
    for p in &c.paths {
        total += count(p, m).ok_or(Failure::Cancelled)?;
    }
    m.add_totals(Totals {
        bytes: 0,
        items: total,
    });
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
    let mut n = 0;
    let mut last = Vec::new();
    let read = extract::lines(&mut child, err, m, |line| {
        // An entry bsdtar fails to write ends its `a name` line with the error.
        let (line, said) = match line.windows(8).rposition(|w| w == b"bsdtar: ") {
            Some(at) if at > 0 => line.split_at(at),
            _ => (line, &b""[..]),
        };
        if !said.is_empty() {
            last = said.to_vec();
        }
        match line.strip_prefix(b"a ") {
            Some(entry) => {
                n += 1;
                m.set_current(OsStr::from_bytes(entry));
                m.settle(Totals {
                    bytes: 0,
                    items: n.min(total),
                });
            }
            None if !String::from_utf8_lossy(line).contains(extract::DELAYED) => {
                last = line.to_vec()
            }
            None => {}
        }
        true
    });
    let ok = extract::finish(child, &read).map_err(undo)?;
    if !ok {
        return Err(undo(Failure::Tool(extract::said(&last, "bsdtar stopped."))));
    }
    land(&tmp, &c.zip).map_err(|e| undo(io_err(e)))
}

/// The entries bsdtar writes for `path`: it, and everything under it when a folder. Links are not
/// followed. `None` once `m` is cancelled.
fn count(path: &Path, m: &Meter) -> Option<u64> {
    let Ok(meta) = path.symlink_metadata() else {
        return Some(0);
    };
    let mut n = 1;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            if m.cancelled() {
                return None;
            }
            n += count(&entry.path(), m)?;
        }
    }
    Some(n)
}

/// Renames `tmp` to the first free of `zip`, `zip (2)`..., never over a file that took the name
/// while the zip was written.
fn land(tmp: &Path, zip: &Path) -> io::Result<PathBuf> {
    let dir = zip.parent().unwrap_or(Path::new("."));
    let name = zip.file_name().unwrap_or_default();
    for candidate in names::numbered(name, names::paren).take(1000) {
        let to = dir.join(candidate);
        match ops::rename2(tmp, &to, libc::RENAME_NOREPLACE) {
            Ok(()) => return Ok(to),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) if ops::unsupported(&e) => {
                if to.symlink_metadata().is_ok() {
                    continue;
                }
                std::fs::rename(tmp, &to)?;
                return Ok(to);
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "every numbered name is taken",
    ))
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
                "echo \"bsdtar: Can't open 'a.txt': Permission denied\" >&2",
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
