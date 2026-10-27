//! Extracting archives with `bsdtar`, which reads zip, 7z and tar plain or compressed. `-P` is
//! never passed, so bsdtar strips a leading `/` and will not write through a symlink.

use crate::names;
use crate::progress::{Decision, Meter, Phase, Totals};
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// The types offered Extract Here and Extract to. A lone compressed file (`.gz`, `.xz`, `.zst`)
/// is no archive: bsdtar reads it only with `--format raw`.
pub const MIMES: [&str; 7] = [
    "application/zip",
    "application/x-tar",
    "application/x-7z-compressed",
    "application/x-compressed-tar",
    "application/x-xz-compressed-tar",
    "application/x-zstd-compressed-tar",
    "application/x-bzip2-compressed-tar",
];

pub const FAILURE_TITLE: &str = "Error Extracting File";

/// Suffixes a folder named after its archive drops, longest first.
const SUFFIXES: [&str; 10] = [
    ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tar", ".zip", ".7z",
];

const SLICE_MS: i32 = 50;

pub fn is_archive(mime: &str) -> bool {
    MIMES.contains(&mime)
}

/// The folder Extract to names after `archive`: its name less the archive suffix.
pub fn folder_name(archive: &OsStr) -> OsString {
    let bytes = archive.as_bytes();
    let lower = bytes.to_ascii_lowercase();
    let cut = SUFFIXES
        .iter()
        .find(|s| lower.ends_with(s.as_bytes()) && lower.len() > s.len())
        .map_or(bytes.len(), |s| bytes.len() - s.len());
    OsStr::from_bytes(&bytes[..cut]).to_owned()
}

pub fn list_argv(archive: &Path) -> Vec<OsString> {
    vec!["bsdtar".into(), "-tf".into(), archive.into()]
}

/// `keep_old` leaves a file already in `into` as it is instead of writing over it.
pub fn extract_argv(archive: &Path, into: &Path, keep_old: bool) -> Vec<OsString> {
    let mut v: Vec<OsString> = vec!["bsdtar".into(), "-xv".into()];
    if keep_old {
        v.push("-k".into());
    }
    v.extend(["-f".into(), archive.into(), "-C".into(), into.into()]);
    v
}

/// Whether a listed entry name reaches above the folder it is extracted into.
pub fn climbs_out(entry: &[u8]) -> bool {
    Path::new(OsStr::from_bytes(entry))
        .components()
        .any(|c| c == Component::ParentDir)
}

/// One archive of a batch: into its own folder, or `here` beside it.
#[derive(Clone, Debug)]
pub struct Archive {
    pub src: PathBuf,
    pub here: bool,
}

#[derive(Debug)]
pub enum ExtractEvent {
    /// Archive `index` is out, into `into`.
    Landed {
        index: usize,
        into: PathBuf,
    },
    /// Archive `index` failed; `text` says why, for the error box.
    Failed {
        index: usize,
        text: String,
    },
    Finished,
}

pub struct ExtractJob {
    events: Receiver<ExtractEvent>,
    meter: Arc<Meter>,
    finished: bool,
}

/// Why an archive did not come out.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Failure {
    Climbs(String),
    Tool(String),
    Cancelled,
}

impl Failure {
    fn text(&self, name: &str) -> String {
        match self {
            Failure::Climbs(entry) => format!(
                "Cannot extract {name}: it holds {entry}, which would land outside the folder."
            ),
            Failure::Tool(why) => format!("Cannot extract {name}: {why}"),
            Failure::Cancelled => format!("Cannot extract {name}: the operation was cancelled."),
        }
    }
}

impl ExtractJob {
    /// Lists every archive first (the totals, and a `..` entry refuses its archive), then extracts
    /// each in order on a thread. A failed archive holds the job for a [`Decision`].
    pub fn start(items: Vec<Archive>, wake: impl Fn() + Send + 'static) -> ExtractJob {
        let meter = Arc::new(Meter::default());
        meter.ask_on_failure();
        let (tx, events) = mpsc::channel();
        let m = meter.clone();
        let spawned = std::thread::Builder::new()
            .name("reprisefs-extract".into())
            .spawn(move || {
                run(&items, &m, |ev| {
                    let sent = tx.send(ev).is_ok();
                    wake();
                    sent
                })
            });
        if let Err(e) = spawned {
            log::error!("reprisefs: cannot spawn the extract thread: {e}");
        }
        ExtractJob {
            events,
            meter,
            finished: false,
        }
    }

    pub fn meter(&self) -> &Arc<Meter> {
        &self.meter
    }

    pub fn poll(&mut self) -> Vec<ExtractEvent> {
        let mut out = Vec::new();
        while !self.finished {
            match self.events.try_recv() {
                Ok(ExtractEvent::Finished) | Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    out.push(ExtractEvent::Finished);
                }
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => break,
            }
        }
        out
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

fn run(items: &[Archive], m: &Meter, send: impl Fn(ExtractEvent) -> bool) {
    let name = |a: &Archive| a.src.file_name().unwrap_or_default().to_owned();
    let mut lists: Vec<Result<u64, Failure>> = Vec::new();
    for a in items {
        m.set_current(&name(a));
        let listed = list(&a.src, m);
        if listed == Err(Failure::Cancelled) {
            send(ExtractEvent::Finished);
            return;
        }
        if let Ok(n) = listed {
            m.add_totals(Totals { bytes: 0, items: n });
        }
        lists.push(listed);
    }
    m.set_phase(Phase::Copying);
    let mut done = 0;
    let mut index = 0;
    let mut again = false;
    while let Some(a) = items.get(index) {
        if m.hold().is_err() {
            break;
        }
        m.set_at(index);
        m.set_current(&name(a));
        if std::mem::take(&mut again) && lists[index].is_err() {
            lists[index] = list(&a.src, m);
            if let Ok(n) = lists[index] {
                m.add_totals(Totals { bytes: 0, items: n });
            }
        }
        let r = match &lists[index] {
            Ok(_) => extract(a, m, done),
            Err(e) => Err(e.clone()),
        };
        let ev = match r {
            Ok(into) => ExtractEvent::Landed { index, into },
            Err(_) if m.cancelled() => break,
            Err(e) => ExtractEvent::Failed {
                index,
                text: e.text(&name(a).to_string_lossy()),
            },
        };
        let failed = matches!(ev, ExtractEvent::Failed { .. });
        if failed {
            m.ask();
        }
        if !send(ev) {
            return;
        }
        if failed {
            match m.decision() {
                Decision::Retry => {
                    again = true;
                    continue;
                }
                Decision::Skip => {}
                Decision::Pending | Decision::Abort => break,
            }
        }
        if let Ok(n) = lists[index] {
            done += n;
            m.settle(Totals {
                bytes: 0,
                items: done,
            });
        }
        index += 1;
    }
    send(ExtractEvent::Finished);
}

/// How many entries `archive` lists, refusing one that climbs out.
fn list(archive: &Path, m: &Meter) -> Result<u64, Failure> {
    count(&list_argv(archive), m)
}

/// Counts the lines `argv` prints, draining its stderr on a thread so a chatty listing cannot
/// fill that pipe and stall.
fn count(argv: &[OsString], m: &Meter) -> Result<u64, Failure> {
    let mut child = spawn(argv, true)?;
    let out = child.stdout.take().ok_or_else(no_pipe)?;
    let mut err = child.stderr.take().ok_or_else(no_pipe)?;
    let drain = std::thread::spawn(move || tail(&mut err, STDERR_TAIL));
    let mut n = 0;
    let mut climbs = None;
    let read = lines(&mut child, out, m, |entry| {
        if climbs_out(entry) {
            climbs = Some(String::from_utf8_lossy(entry).into_owned());
            return false;
        }
        n += 1;
        true
    });
    let status = finish(child, &read);
    let stderr = drain.join().unwrap_or_default();
    if let Some(entry) = climbs {
        return Err(Failure::Climbs(entry));
    }
    match status? {
        true => Ok(n),
        false => Err(Failure::Tool(said(&stderr))),
    }
}

/// Extracts `a`, the batch having counted `before` items: into a new folder named after it,
/// which a failure removes again, or beside it, writing over nothing already there.
fn extract(a: &Archive, m: &Meter, before: u64) -> Result<PathBuf, Failure> {
    let dir = a
        .src
        .parent()
        .ok_or_else(|| Failure::Tool("it has no folder".into()))?;
    let into = match a.here {
        true => dir.to_path_buf(),
        false => make_folder(dir, &folder_name(a.src.file_name().unwrap_or_default()))?,
    };
    let undo = |e: Failure| {
        if !a.here
            && let Err(e) = crate::ops::remove_tree(&into)
        {
            log::warn!("reprisefs: cannot remove {}: {e}", into.display());
        }
        e
    };
    let argv = extract_argv(&a.src, &into, a.here);
    let mut child = spawn(&argv, false).map_err(undo)?;
    let err = child.stderr.take().ok_or_else(no_pipe).map_err(undo)?;
    let mut n = 0;
    let mut last = String::new();
    let read = lines(&mut child, err, m, |line| {
        match line.strip_prefix(b"x ") {
            Some(entry) => {
                n += 1;
                m.set_current(OsStr::from_bytes(entry));
                m.settle(Totals {
                    bytes: 0,
                    items: before + n,
                });
            }
            None => last = String::from_utf8_lossy(line).into_owned(),
        }
        true
    });
    let ok = finish(child, &read).map_err(undo)?;
    if !ok {
        return Err(undo(Failure::Tool(said(last.as_bytes()))));
    }
    Ok(into)
}

/// Makes the first free folder `name`, `name (2)`... in `dir`.
fn make_folder(dir: &Path, name: &OsStr) -> Result<PathBuf, Failure> {
    loop {
        let free = names::first_free(name, |c| dir.join(c).symlink_metadata().is_ok());
        let path = dir.join(free);
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Failure::Tool(e.to_string())),
        }
    }
}

fn no_pipe() -> Failure {
    Failure::Tool("bsdtar gave no output".into())
}

/// The bytes of a tool's stderr kept for [`said`], which reads only its last line.
const STDERR_TAIL: usize = 4096;

/// Reads `r` to its end, keeping only its last `cap` bytes.
fn tail(r: &mut impl Read, cap: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut buf = [0; 8192];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        kept.extend_from_slice(&buf[..n]);
        if kept.len() > 2 * cap {
            kept.drain(..kept.len() - cap);
        }
    }
    kept.drain(..kept.len().saturating_sub(cap));
    kept
}

/// bsdtar's message without its own name, or a stand-in when it said nothing.
fn said(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let line = line.strip_prefix("bsdtar: ").unwrap_or(line).trim();
    match line.is_empty() {
        true => "bsdtar could not read it.".into(),
        false => line.to_string(),
    }
}

/// Starts `argv` with stderr piped, and stdout too when `stdout`.
fn spawn(argv: &[OsString], stdout: bool) -> Result<Child, Failure> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..])
        .stdin(Stdio::null())
        .stderr(Stdio::piped());
    cmd.stdout(match stdout {
        true => Stdio::piped(),
        false => Stdio::null(),
    });
    cmd.spawn().map_err(|e| {
        Failure::Tool(format!(
            "bsdtar cannot start ({e}); is libarchive installed?"
        ))
    })
}

/// Feeds each line of `out` to `each` until it ends, `each` says stop or `m` is cancelled
/// (`Ok(false)`). The child is stopped while `m` is paused.
fn lines(
    child: &mut Child,
    mut out: impl Read + AsRawFd,
    m: &Meter,
    mut each: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<bool> {
    let fd = out.as_raw_fd();
    let pid = child.id() as libc::pid_t;
    let mut stopped = false;
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    let ended = loop {
        if m.cancelled() {
            break false;
        }
        if m.paused() != stopped {
            stopped = !stopped;
            let sig = if stopped {
                libc::SIGSTOP
            } else {
                libc::SIGCONT
            };
            // SAFETY: `pid` is our own unreaped child, so the id names no other process.
            unsafe { libc::kill(pid, sig) };
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, SLICE_MS) };
        if ready == 0 {
            continue;
        }
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        let n = match out.read(&mut buf) {
            Ok(0) => break true,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=nl).collect();
            if !each(&line[..nl]) {
                return Ok(false);
            }
        }
    };
    if ended && !pending.is_empty() && !each(&pending) {
        return Ok(false);
    }
    if stopped {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGCONT) };
    }
    Ok(ended)
}

/// Reaps `child` after [`lines`] read it to `read`: whether it exited cleanly. A child cut
/// short is killed first and counts as cancelled.
fn finish(mut child: Child, read: &std::io::Result<bool>) -> Result<bool, Failure> {
    if !matches!(read, Ok(true)) {
        let _ = child.kill();
        let _ = child.wait();
        return match read {
            Err(e) => Err(Failure::Tool(e.to_string())),
            _ => Err(Failure::Cancelled),
        };
    }
    let status = child.wait().map_err(|e| Failure::Tool(e.to_string()))?;
    Ok(status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bsdtar(dir: &Path, args: &[&str]) {
        let st = Command::new("bsdtar")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("bsdtar runs");
        assert!(st.success(), "bsdtar {args:?}");
    }

    fn run_job(items: Vec<Archive>, pick: Decision) -> Vec<ExtractEvent> {
        let (wtx, woke) = mpsc::channel();
        let mut job = ExtractJob::start(items, move || {
            let _ = wtx.send(());
        });
        let mut all = Vec::new();
        while !job.is_finished() {
            woke.recv_timeout(std::time::Duration::from_secs(30))
                .expect("a wake");
            for ev in job.poll() {
                if matches!(ev, ExtractEvent::Failed { .. }) {
                    job.meter().decide(pick);
                }
                all.push(ev);
            }
        }
        all
    }

    #[test]
    fn the_argv_lists_and_extracts_with_bsdtar_and_never_passes_p() {
        let (a, d) = (Path::new("/d/-x.zip"), Path::new("/d/-x"));
        assert_eq!(list_argv(a), ["bsdtar", "-tf", "/d/-x.zip"]);
        assert_eq!(
            extract_argv(a, d, false),
            ["bsdtar", "-xv", "-f", "/d/-x.zip", "-C", "/d/-x"],
            "a name like an option is an option's argument"
        );
        assert_eq!(
            extract_argv(a, Path::new("/d"), true),
            ["bsdtar", "-xv", "-k", "-f", "/d/-x.zip", "-C", "/d"],
            "Extract Here writes over nothing"
        );
    }

    #[test]
    fn archives_are_extractable_and_a_lone_compressed_file_is_not() {
        for mime in [
            "application/zip",
            "application/x-7z-compressed",
            "application/x-compressed-tar",
            "application/x-zstd-compressed-tar",
        ] {
            assert!(is_archive(mime), "{mime}");
        }
        for mime in [
            "application/gzip",
            "application/x-xz",
            "application/zstd",
            "text/plain",
        ] {
            assert!(!is_archive(mime), "{mime}");
        }
    }

    #[test]
    fn the_folder_drops_the_archive_suffix() {
        for (archive, folder) in [
            ("photos.zip", "photos"),
            ("src-1.2.tar.gz", "src-1.2"),
            ("Backup.TAR.XZ", "Backup"),
            ("a.tgz", "a"),
            ("set.7z", "set"),
            ("dump.tar.zst", "dump"),
            (".zip", ".zip"),
            ("plain", "plain"),
        ] {
            assert_eq!(folder_name(OsStr::new(archive)), folder, "{archive}");
        }
    }

    #[test]
    fn a_listing_that_floods_stderr_still_ends_with_its_count() {
        let script = "head -c 300000 /dev/zero | tr '\\0' w >&2; echo a; echo b; exit 1";
        let argv: Vec<OsString> = ["sh", "-c", script].map(OsString::from).to_vec();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(count(&argv, &Meter::default()));
        });
        let res = rx
            .recv_timeout(std::time::Duration::from_secs(20))
            .expect("the listing ends while stderr holds more than a pipe buffer");
        match res {
            Err(Failure::Tool(said)) => {
                assert_eq!(said.len(), STDERR_TAIL, "only the tail is kept")
            }
            other => panic!("expected the tool's own message, got {other:?}"),
        }
    }

    #[test]
    fn an_entry_with_a_parent_step_climbs_out() {
        for (entry, climbs) in [
            ("a/b.txt", false),
            ("../b.txt", true),
            ("a/../../b", true),
            ("a/..b/c", false),
            ("/etc/passwd", false),
            ("..", true),
        ] {
            assert_eq!(climbs_out(entry.as_bytes()), climbs, "{entry}");
        }
    }

    #[test]
    fn an_archive_that_climbs_out_is_refused_and_extracts_nothing() {
        let t = tempfile::tempdir().unwrap();
        let work = t.path().join("work");
        std::fs::create_dir(&work).unwrap();
        std::fs::write(work.join("evil"), "x").unwrap();
        bsdtar(&work, &["-cf", "../evil.tar", "-s", ",^,../,", "evil"]);
        let got = run_job(
            vec![Archive {
                src: t.path().join("evil.tar"),
                here: false,
            }],
            Decision::Abort,
        );
        assert!(
            matches!(&got[..], [ExtractEvent::Failed { index: 0, text }, ExtractEvent::Finished]
                if text.contains("../evil")),
            "{got:?}"
        );
        assert!(!t.path().join("evil").exists());
        assert!(!t.path().join("evil (2)").exists(), "no folder is made");
    }

    #[test]
    fn a_batch_extracts_into_free_folders_and_skips_a_broken_archive() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("src/sub")).unwrap();
        std::fs::write(d.join("src/a.txt"), "a").unwrap();
        std::fs::write(d.join("src/sub/b.txt"), "b").unwrap();
        bsdtar(
            &d.join("src"),
            &["-a", "-cf", "../pack.zip", "a.txt", "sub"],
        );
        std::fs::write(d.join("broken.zip"), "not an archive").unwrap();
        std::fs::create_dir(d.join("pack")).unwrap();
        let got = run_job(
            vec![
                Archive {
                    src: d.join("broken.zip"),
                    here: false,
                },
                Archive {
                    src: d.join("pack.zip"),
                    here: false,
                },
            ],
            Decision::Skip,
        );
        assert!(
            matches!(
                &got[..],
                [
                    ExtractEvent::Failed { index: 0, .. },
                    ExtractEvent::Landed { index: 1, into },
                    ExtractEvent::Finished,
                ] if *into == d.join("pack (2)")
            ),
            "{got:?}"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("pack (2)/sub/b.txt")).unwrap(),
            "b"
        );
        assert!(!d.join("broken").exists(), "a failure leaves no folder");
    }

    #[test]
    fn a_failed_extract_of_a_read_only_folder_leaves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        for dir in ["src/d", "more/e"] {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        std::fs::write(d.join("src/d/a"), "a").unwrap();
        std::fs::write(d.join("src/e"), "e").unwrap();
        std::fs::write(d.join("more/e/f"), "f").unwrap();
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(d.join("src/d"), mode(0o555)).unwrap();
        bsdtar(&d.join("src"), &["-cf", "../pack.tar", "d", "e"]);
        bsdtar(&d.join("more"), &["-rf", "../pack.tar", "e/f"]);
        std::fs::set_permissions(d.join("src/d"), mode(0o755)).unwrap();
        for dir in ["src", "more"] {
            std::fs::remove_dir_all(d.join(dir)).unwrap();
        }
        let got = run_job(
            vec![Archive {
                src: d.join("pack.tar"),
                here: false,
            }],
            Decision::Skip,
        );
        assert!(
            matches!(
                &got[..],
                [
                    ExtractEvent::Failed { index: 0, .. },
                    ExtractEvent::Finished
                ]
            ),
            "{got:?}"
        );
        let left: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["pack.tar"], "the half folder is removed");
    }

    #[test]
    fn extract_here_keeps_a_file_already_there() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.txt"), "new").unwrap();
        std::fs::write(d.join("src/b.txt"), "b").unwrap();
        bsdtar(&d.join("src"), &["-cf", "../pack.tar", "a.txt", "b.txt"]);
        std::fs::write(d.join("a.txt"), "old").unwrap();
        let got = run_job(
            vec![Archive {
                src: d.join("pack.tar"),
                here: true,
            }],
            Decision::Abort,
        );
        assert!(
            matches!(&got[..], [ExtractEvent::Landed { into, .. }, ExtractEvent::Finished] if into == d),
            "{got:?}"
        );
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b");
    }
}
