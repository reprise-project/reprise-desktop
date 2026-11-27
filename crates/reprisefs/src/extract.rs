//! Extracting archives with `bsdtar` (zip, 7z, tar), never with `-P`, each into a fresh folder;
//! docs/security.md "Extracting archives" has the rules.

use crate::names;
use crate::progress::{Decision, Meter, Phase, Totals};
use reprisekit::tr;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::{AsFd, AsRawFd, FromRawFd};
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

/// The caption of an extract error box.
pub fn failure_title() -> String {
    tr!("extract-failed-title")
}

/// Suffixes a folder named after its archive drops, longest first.
const SUFFIXES: [&str; 10] = [
    ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tar", ".zip", ".7z",
];

const SLICE_MS: i32 = 50;

/// How often an extraction's writes are counted.
const WRITES_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Bytes an extraction may write past its listing: its stderr not yet read, with room to spare.
const WRITE_SLACK: u64 = 1 << 20;

/// The share of a folder's free space one extraction may fill, in percent.
pub const FREE_SHARE_PERCENT: u64 = 90;

/// The most entries one archive extracts; fewer where the file system has fewer free inodes.
pub const MAX_ENTRIES: u64 = 1_000_000;

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
    ["bsdtar", "-tv", "--numeric-owner", "-f"]
        .map(OsString::from)
        .into_iter()
        .chain([archive.into()])
        .collect()
}

pub fn extract_argv(archive: &Path, into: &Path) -> Vec<OsString> {
    ["bsdtar", "-xv", "-f"]
        .map(OsString::from)
        .into_iter()
        .chain([archive.into(), "-C".into(), into.into()])
        .collect()
}

/// Whether a listed entry name lands outside the folder it is extracted into: a `..` step, or a
/// path from the root (which bsdtar would strip).
pub fn climbs_out(entry: &[u8]) -> bool {
    Path::new(OsStr::from_bytes(entry))
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::RootDir))
}

/// What an archive's listing holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) entries: u64,
    /// The bytes its files hold, as their headers say.
    pub(crate) bytes: u64,
    /// A top-level entry's name starts with a `.`.
    pub(crate) hidden: bool,
    /// A top-level entry is named `bin`.
    pub(crate) bin: bool,
}

/// How much one extraction may write into a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) bytes: u64,
    pub(crate) entries: u64,
}

impl Limits {
    /// [`FREE_SHARE_PERCENT`] of `free` bytes, and [`MAX_ENTRIES`] or `inodes` free, whichever
    /// is fewer (`None`: the file system does not count them).
    pub(crate) fn from_free(free: u64, inodes: Option<u64>) -> Limits {
        Limits {
            bytes: (u128::from(free) * u128::from(FREE_SHARE_PERCENT) / 100) as u64,
            entries: inodes.map_or(MAX_ENTRIES, |n| n.min(MAX_ENTRIES)),
        }
    }

    /// [`Limits::from_free`] of the file system holding `dir`; none it cannot ask.
    pub(crate) fn of(dir: &Path) -> Limits {
        let Ok(c) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return Limits::from_free(u64::MAX, None);
        };
        // SAFETY: `c` is NUL-terminated and `st` is a writable `statvfs` the call fills in.
        let st = unsafe {
            let mut st: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c.as_ptr(), &mut st) != 0 {
                return Limits::from_free(u64::MAX, None);
            }
            st
        };
        let free = (st.f_bavail as u64).saturating_mul(st.f_frsize as u64);
        Limits::from_free(free, (st.f_files > 0).then_some(st.f_favail as u64))
    }

    pub(crate) fn admit(self, l: Listed) -> Result<(), Failure> {
        if l.entries > self.entries {
            return Err(Failure::Refused(tr!(
                "extract-too-many",
                entries = l.entries,
                most = self.entries
            )));
        }
        if l.bytes > self.bytes {
            return Err(Failure::Refused(tr!(
                "extract-no-room",
                size = crate::props::short_size(l.bytes),
                room = crate::props::short_size(self.bytes)
            )));
        }
        Ok(())
    }
}

/// One archive of a batch: into its own folder, or `here` beside it. Its own folder is made in
/// `to` when set (Extract All), else beside it.
#[derive(Clone, Debug)]
pub struct Archive {
    pub src: PathBuf,
    pub here: bool,
    pub to: Option<PathBuf>,
}

#[derive(Debug)]
pub enum ArchiveEvent {
    /// Archive `index` is out, into `into`; a compression's zip landed at `into`.
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

/// An extraction or a compression on its thread.
pub struct ArchiveJob {
    events: Receiver<ArchiveEvent>,
    meter: Arc<Meter>,
    finished: bool,
}

/// Why an archive did not come out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Climbs(String),
    /// Why Reprise will not extract it.
    Refused(String),
    Tool(String),
    /// Extract Here stopped part way and could not move everything back.
    Kept(String),
    Cancelled,
}

impl Failure {
    fn text(&self, name: &str) -> String {
        tr!("extract-failed", name = name, why = self.why())
    }

    pub(crate) fn why(&self) -> String {
        match self {
            Failure::Climbs(entry) => tr!("extract-climbs", entry = entry.as_str()),
            Failure::Tool(why) | Failure::Refused(why) | Failure::Kept(why) => why.clone(),
            Failure::Cancelled => tr!("extract-cancelled"),
        }
    }
}

impl ArchiveJob {
    /// Lists every archive first (the totals; an entry landing outside refuses its archive), then
    /// extracts each in order on a thread. A failed archive holds the job for a [`Decision`].
    pub fn extract(items: Vec<Archive>, wake: impl Fn() + Send + 'static) -> ArchiveJob {
        ArchiveJob::extract_within(items, Limits::of, wake)
    }

    /// [`ArchiveJob::extract`], `limits` saying how much each folder takes.
    pub(crate) fn extract_within(
        items: Vec<Archive>,
        limits: fn(&Path) -> Limits,
        wake: impl Fn() + Send + 'static,
    ) -> ArchiveJob {
        let meter = Arc::new(Meter::default());
        meter.ask_on_failure();
        ArchiveJob::on_thread("reprisefs-extract", meter, wake, move |m, send| {
            run(&items, limits, m, send)
        })
    }

    /// Runs `body` on a thread named `name`, counting into `meter`; each event it sends wakes
    /// the host. A send returns false once the host dropped the job.
    pub(crate) fn on_thread(
        name: &str,
        meter: Arc<Meter>,
        wake: impl Fn() + Send + 'static,
        body: impl FnOnce(&Meter, &dyn Fn(ArchiveEvent) -> bool) + Send + 'static,
    ) -> ArchiveJob {
        let (tx, events) = mpsc::channel();
        let m = meter.clone();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                body(&m, &|ev| {
                    let sent = tx.send(ev).is_ok();
                    wake();
                    sent
                })
            });
        if let Err(e) = spawned {
            log::error!("reprisefs: cannot spawn {name}: {e}");
        }
        ArchiveJob {
            events,
            meter,
            finished: false,
        }
    }

    pub fn meter(&self) -> &Arc<Meter> {
        &self.meter
    }

    pub fn poll(&mut self) -> Vec<ArchiveEvent> {
        let mut out = Vec::new();
        while !self.finished {
            match self.events.try_recv() {
                Ok(ArchiveEvent::Finished) | Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    out.push(ArchiveEvent::Finished);
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

/// A job let go before it finished is cancelled, so a paused one neither sleeps nor keeps its
/// stopped bsdtar for good.
impl Drop for ArchiveJob {
    fn drop(&mut self) {
        if !self.finished {
            self.meter.cancel();
        }
    }
}

fn run(
    items: &[Archive],
    limits: fn(&Path) -> Limits,
    m: &Meter,
    send: &dyn Fn(ArchiveEvent) -> bool,
) {
    let name = |a: &Archive| a.src.file_name().unwrap_or_default().to_owned();
    let mut lists: Vec<Result<Listed, Failure>> = Vec::new();
    for a in items {
        m.set_current(&name(a));
        let listed = list(&a.src, m);
        if listed == Err(Failure::Cancelled) {
            send(ArchiveEvent::Finished);
            return;
        }
        if let Ok(l) = listed {
            m.add_totals(Totals {
                bytes: 0,
                items: l.entries,
            });
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
            if let Ok(l) = lists[index] {
                m.add_totals(Totals {
                    bytes: 0,
                    items: l.entries,
                });
            }
        }
        let r = match &lists[index] {
            Ok(l) => extract(a, m, done, *l, limits),
            Err(e) => Err(e.clone()),
        };
        let ev = match r {
            Ok(into) => ArchiveEvent::Landed { index, into },
            Err(_) if m.cancelled() => break,
            Err(e) => ArchiveEvent::Failed {
                index,
                text: e.text(&name(a).to_string_lossy()),
            },
        };
        let failed = matches!(ev, ArchiveEvent::Failed { .. });
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
        if let Ok(l) = lists[index] {
            done += l.entries;
            m.settle(Totals {
                bytes: 0,
                items: done,
            });
        }
        index += 1;
    }
    send(ArchiveEvent::Finished);
}

fn list(archive: &Path, m: &Meter) -> Result<Listed, Failure> {
    if crate::archive::zip_encrypted(archive) {
        return Err(Failure::Refused(tr!("archive-locked")));
    }
    let budget = crate::archive::cpu_secs(crate::archive::archive_bytes(archive));
    match listing(&list_argv(archive), budget, m) {
        Err(Failure::Tool(why)) if crate::archive::locked(why.as_bytes()) => {
            Err(Failure::Refused(tr!("archive-locked")))
        }
        listed => listed,
    }
}

/// Reads the `bsdtar -tv` listing `argv` prints in at most `cpu_secs` of CPU, refusing an entry
/// that lands outside, and draining its stderr on a thread so a chatty listing cannot fill that
/// pipe and stall.
fn listing(argv: &[OsString], cpu_secs: u64, m: &Meter) -> Result<Listed, Failure> {
    let mut child = spawn_with(argv, true, |cmd| {
        c_dates(cmd);
        crate::archive::limit(cmd, cpu_secs, None);
    })?;
    let out = child.stdout.take().ok_or_else(no_pipe)?;
    let mut err = child.stderr.take().ok_or_else(no_pipe)?;
    let drain = std::thread::spawn(move || tail(&mut err, STDERR_TAIL));
    let mut got = Listed::default();
    let mut climbs = None;
    let mut unread = false;
    let read = lines(&mut child, out, m, |line| {
        let Some((kind, size, rest)) = listed_line(line) else {
            unread = true;
            return true;
        };
        let cut = |sep: &[u8]| {
            (rest.windows(sep.len()))
                .position(|w| w == sep)
                .map_or(rest, |at| &rest[..at])
        };
        let entry = match kind {
            b'l' => cut(b" -> "),
            b'h' => cut(b" link to "),
            _ => rest,
        };
        if climbs_out(entry) {
            climbs = Some(String::from_utf8_lossy(entry).into_owned());
            return false;
        }
        got.entries += 1;
        got.bytes = got.bytes.saturating_add(size);
        let top = top_name(entry);
        got.hidden |= top.is_some_and(|n| n.starts_with(b"."));
        got.bin |= top == Some(b"bin".as_slice());
        true
    });
    let status = finish_status(child, &read);
    let stderr = drain.join().unwrap_or_default();
    if let Some(entry) = climbs {
        return Err(Failure::Climbs(entry));
    }
    match status? {
        s if s.success() && unread => Err(Failure::Tool(tr!("archive-unread"))),
        s if s.success() => Ok(got),
        s if crate::archive::ran_out(&s) => Err(Failure::Refused(tr!("extract-list-too-long"))),
        _ => Err(Failure::Tool(said(&stderr, &tr!("archive-unread")))),
    }
}

/// The listing's dates in the C locale's three words, the rest of the locale kept.
fn c_dates(cmd: &mut Command) {
    if let Some(all) = std::env::var_os("LC_ALL") {
        cmd.env_remove("LC_ALL").env("LC_CTYPE", all);
    }
    cmd.env("LC_TIME", "C");
}

/// Type letter, size and rest (name, then link target) of a `bsdtar -tv --numeric-owner` line:
/// mode, links, uid, gid, size, a three-word date. A device's `major,minor` size holds no bytes.
fn listed_line(line: &[u8]) -> Option<(u8, u64, &[u8])> {
    let mut rest = line;
    let mut words = [&line[..0]; 8];
    for w in &mut words {
        rest = &rest[rest.iter().position(|&b| b != b' ')?..];
        (*w, rest) = rest.split_at(rest.iter().position(|&b| b == b' ')?);
    }
    let kind = *words[0].first()?;
    let size = match kind {
        b'c' | b'b' if words[4].contains(&b',') => 0,
        _ => std::str::from_utf8(words[4]).ok()?.parse().ok()?,
    };
    Some((kind, size, rest.strip_prefix(b" ")?))
}

fn top_name(entry: &[u8]) -> Option<&[u8]> {
    Path::new(OsStr::from_bytes(entry))
        .components()
        .find_map(|c| match c {
            Component::Normal(n) => Some(n.as_bytes()),
            _ => None,
        })
}

/// Whether Extract Here keeps `listed` in a folder of its own in `dir`: one with a hidden top-level
/// entry, or with a top-level `bin` in the home folder. docs/security.md "Extracting archives".
fn kept_apart(listed: Listed, dir: &Path, home: Option<&Path>) -> bool {
    listed.hidden || listed.bin && home.is_some_and(|h| is_same_dir(dir, h))
}

fn is_same_dir(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => (a.dev(), a.ino()) == (b.dev(), b.ino()),
        _ => false,
    }
}

/// Extracts `a`, the batch having counted `before` items, into a fresh folder that Extract to keeps
/// and Extract Here merges into place. docs/security.md "Extracting archives" has the rules.
fn extract(
    a: &Archive,
    m: &Meter,
    before: u64,
    listed: Listed,
    limits: fn(&Path) -> Limits,
) -> Result<PathBuf, Failure> {
    let dir = (a.to.as_deref())
        .or(a.src.parent())
        .ok_or_else(|| Failure::Tool(tr!("extract-no-folder")))?;
    limits(dir).admit(listed)?;
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let here = a.here && !kept_apart(listed, dir, home.as_deref());
    let name = folder_name(a.src.file_name().unwrap_or_default());
    let into = make_folder(dir, &if here { staging(&name) } else { name.clone() })?;
    let remove = || discard(&into);
    let undo = |e: Failure| {
        remove();
        e
    };
    let argv = extract_argv(&a.src, &into);
    let budget = crate::archive::archive_bytes(&a.src).saturating_add(listed.bytes);
    let mut child = spawn_with(&argv, false, |cmd| {
        crate::archive::limit(cmd, crate::archive::cpu_secs(budget), Some(listed.bytes));
    })
    .map_err(undo)?;
    let err = child.stderr.take().ok_or_else(no_pipe).map_err(undo)?;
    let pid = child.id();
    let io = |e: std::io::Error| Failure::Tool(e.to_string());
    let into_fd = crate::ops::at::c_path(&into)
        .and_then(|p| crate::ops::at::open_dir(crate::ops::at::cwd(), &p, true))
        .map_err(io)
        .map_err(undo)?;
    // What bsdtar may write: the listed bytes, its stderr, and what that pipe still holds.
    let said_bytes = std::cell::Cell::new(0u64);
    let past = std::cell::Cell::new(false);
    let stop = || {
        past.set(true);
        // SAFETY: `pid` is our own unreaped child, so the id names no other process.
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    };
    let wrote_past = || {
        let allowed = (listed.bytes)
            .saturating_add(said_bytes.get())
            .saturating_add(WRITE_SLACK);
        io_count(pid, "wchar").map(|w| w > allowed)
    };
    // Without a write count (no /proc, or one that does not show it), each entry bsdtar reports
    // is measured once as it lands instead.
    let counted = io_count(pid, "wchar").is_some();
    let mut landed = Landed::default();
    let mut sampled = std::time::Instant::now();
    let tick = || {
        if !counted || sampled.elapsed() < WRITES_EVERY || past.get() {
            return;
        }
        sampled = std::time::Instant::now();
        if wrote_past() == Some(true) {
            stop();
        }
    };
    let mut n = 0;
    let mut first = None;
    let mut others = 0;
    let mut locked = false;
    let read = lines_ticking(
        &mut child,
        err,
        m,
        |line| {
            said_bytes.set(said_bytes.get() + line.len() as u64 + 1);
            match line.strip_prefix(b"x ") {
                Some(entry) => {
                    n += 1;
                    m.set_current(OsStr::from_bytes(entry));
                    m.settle(Totals {
                        bytes: 0,
                        items: before + n,
                    });
                    // bsdtar ends an entry's line once it is written, with `: why` when that failed.
                    // A name holding `: ` is told apart by having landed.
                    let failed = entry.windows(2).any(|w| w == b": ");
                    let stat = (failed || !counted && !past.get())
                        .then(|| landed_at(into_fd.as_fd(), entry).ok())
                        .flatten();
                    // The entry past the listing has landed whole, and so have those whose lines
                    // the pipe still holds: each under RLIMIT_FSIZE, which stops the one bsdtar
                    // is writing at the listing's total.
                    if let Some(st) = stat.as_ref().filter(|_| !counted && !past.get())
                        && landed.add(st) > listed.bytes
                    {
                        stop();
                    }
                    if failed && stat.is_none() {
                        // Only a failed entry's line: a name that landed says nothing.
                        locked |= crate::archive::locked(entry);
                        match first {
                            None => first = Some(entry.to_vec()),
                            Some(_) => others += 1,
                        }
                    }
                }
                None if !String::from_utf8_lossy(line).contains(DELAYED) => {
                    locked |= crate::archive::locked(line);
                    first.get_or_insert_with(|| line.to_vec());
                }
                None => {}
            }
            true
        },
        tick,
    );
    // The child is unreaped, so its count is still there when it ended between samples.
    if !past.get() && matches!(read, Ok(true)) && wrote_past() == Some(true) {
        past.set(true);
    }
    let status = finish_status(child, &read).map_err(undo)?;
    let wrote_too_much =
        std::os::unix::process::ExitStatusExt::signal(&status) == Some(libc::SIGXFSZ);
    if past.get() || wrote_too_much {
        return Err(undo(Failure::Refused(tr!("archive-more-than-listed"))));
    }
    if crate::archive::ran_out(&status) {
        return Err(undo(Failure::Refused(tr!("extract-too-long"))));
    }
    let ok = status.success();
    if !ok && locked {
        return Err(undo(Failure::Refused(tr!("archive-locked"))));
    }
    if !ok {
        let why = and_others(
            said(&first.unwrap_or_default(), &tr!("archive-unread")),
            others,
            Others::Extracted,
        );
        return Err(undo(Failure::Tool(why)));
    }
    let modes = open_up(&into).map_err(io).map_err(undo)?;
    // What landed holds whatever the write count or the reported entries missed.
    if landed_bytes(&into).map_err(io).map_err(undo)? > listed.bytes {
        return Err(undo(Failure::Refused(tr!("archive-more-than-listed"))));
    }
    let reach = leads_out(&into).map_err(io).map_err(undo)?;
    if let Some(entry) = reach.out {
        return Err(undo(Failure::Refused(tr!(
            "extract-link-out",
            entry = entry
        ))));
    }
    // Merged into `dir`, a link to nothing in the archive could name what `dir` already holds.
    if !here || reach.dangles {
        let into = match here {
            true => set_apart(dir, &into, &name).map_err(undo)?,
            false => into.clone(),
        };
        if let Err(e) = set_modes(&into, &modes, |_| true) {
            discard(&into);
            return Err(io(e));
        }
        return Ok(into);
    }
    let merged = merge_into(&into, dir);
    if let Ok(moved) = &merged {
        let landed = |rel: &Path| moved.iter().any(|m| rel.starts_with(m));
        if let Err(e) = set_modes(dir, &modes, landed) {
            log::warn!(
                "reprisefs: cannot set the folder modes of {}: {e}",
                a.src.display()
            );
        }
    }
    if !matches!(merged, Err(Failure::Kept(_))) {
        remove();
    }
    merged.map(|_| dir.to_path_buf())
}

/// Gives every folder under `root` its owner's read, write and search bits, so the walk and the
/// move into place can enter and rename it, returning each changed one's mode for [`set_modes`].
fn open_up(root: &Path) -> std::io::Result<Vec<(PathBuf, libc::mode_t)>> {
    use crate::ops::at::{c_path, cwd, open_dir, stat_at};
    let mut changed = Vec::new();
    let top = std::rc::Rc::new(open_dir(cwd(), &c_path(root)?, true)?);
    // A folder still to list holds its parent's descriptor, so about one is open per level.
    let mut pending = Vec::new();
    let mut next = Some((top, PathBuf::new()));
    while let Some((fd, rel)) = next.take() {
        for (name, is_dir) in crate::ops::entries_at(fd.as_fd())? {
            if !is_dir {
                continue;
            }
            let mode = stat_at(fd.as_fd(), &name)?.st_mode & 0o7777;
            let sub = rel.join(OsStr::from_bytes(name.to_bytes()));
            if mode & 0o700 != 0o700 {
                // SAFETY: `name` is NUL-terminated and `fd` is an open folder.
                if unsafe { libc::fchmodat(fd.as_raw_fd(), name.as_ptr(), mode | 0o700, 0) } != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                changed.push((sub.clone(), mode));
            }
            pending.push((fd.clone(), name, sub));
        }
        if let Some((parent, name, rel)) = pending.pop() {
            next = Some((open_dir(parent.as_fd(), &name, false)?.into(), rel));
        }
    }
    Ok(changed)
}

/// Gives back the modes [`open_up`] changed to the folders now under `root` that `landed` picks,
/// deepest first, so a folder closed to its owner is closed only once its children are set.
fn set_modes(
    root: &Path,
    modes: &[(PathBuf, libc::mode_t)],
    landed: impl Fn(&Path) -> bool,
) -> std::io::Result<()> {
    use crate::ops::at::{c_path, cwd, open_dir};
    let mut picked: Vec<_> = modes.iter().filter(|(rel, _)| landed(rel)).collect();
    picked.sort_by_key(|(rel, _)| std::cmp::Reverse(rel.components().count()));
    let base = open_dir(cwd(), &c_path(root)?, true)?;
    for (rel, mode) in picked {
        let at = steps(base.as_fd(), rel)?;
        // SAFETY: `at` is an open folder.
        if unsafe { libc::fchmod(at.as_raw_fd(), *mode) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn discard(folder: &Path) {
    if let Err(e) = crate::ops::remove_tree(folder) {
        log::warn!("reprisefs: cannot remove {}: {e}", folder.display());
    }
}

/// Renames the staging folder `from` in `dir` to the first free `name`, `name (2)`... there.
fn set_apart(dir: &Path, from: &Path, name: &OsStr) -> Result<PathBuf, Failure> {
    use crate::ops::at::{c_name, c_path, cwd, open_dir, rename_free};
    let io = |e: std::io::Error| Failure::Tool(e.to_string());
    let at = open_dir(cwd(), &c_path(dir).map_err(io)?, true).map_err(io)?;
    let from = c_name(from.file_name().unwrap_or_default()).map_err(io)?;
    loop {
        let free = names::first_free(name, |c| dir.join(c).symlink_metadata().is_ok());
        match rename_free(at.as_fd(), &from, at.as_fd(), &c_name(&free).map_err(io)?) {
            Ok(()) => return Ok(dir.join(free)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io(e)),
        }
    }
}

/// The hidden folder Extract Here fills before moving its entries into place.
fn staging(name: &OsStr) -> OsString {
    let mut s = OsString::from(".");
    s.push(name);
    s.push(".extracting");
    s
}

/// Where the entries under a folder just extracted into lead.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Reach {
    /// The first that leads outside it: a link that does not resolve beneath it, or a file with
    /// a hard link from elsewhere.
    pub(crate) out: Option<String>,
    /// A link resolves to nothing in it.
    pub(crate) dangles: bool,
}

pub(crate) fn leads_out(root: &Path) -> std::io::Result<Reach> {
    use std::os::unix::fs::MetadataExt;
    let mut reach = Reach::default();
    let base = std::fs::File::open(root)?;
    let mut linked: std::collections::HashMap<(u64, u64), (u64, PathBuf)> = Default::default();
    let mut dirs = vec![PathBuf::new()];
    while let Some(rel) = dirs.pop() {
        for e in std::fs::read_dir(root.join(&rel))? {
            let e = e?;
            let (kind, rel) = (e.file_type()?, rel.join(e.file_name()));
            if kind.is_dir() {
                dirs.push(rel);
            } else if kind.is_symlink() {
                match resolves(&base, &rel)? {
                    Resolves::Beneath => {}
                    Resolves::Nothing => reach.dangles = true,
                    Resolves::Outside => {
                        reach.out = Some(rel.to_string_lossy().into_owned());
                        return Ok(reach);
                    }
                }
            } else if let Ok(meta) = e.metadata()
                && meta.nlink() > 1
            {
                let seen = linked.entry((meta.dev(), meta.ino())).or_insert((0, rel));
                seen.0 += 1;
            }
        }
    }
    let out = linked.into_values().find(|(seen, path)| {
        std::fs::symlink_metadata(root.join(path)).is_ok_and(|m| m.nlink() > *seen)
    });
    reach.out = out.map(|(_, path)| path.to_string_lossy().into_owned());
    Ok(reach)
}

enum Resolves {
    Beneath,
    Nothing,
    Outside,
}

/// Where link `rel` under the folder open as `base` resolves; one through a folder it cannot
/// search counts as outside.
fn resolves(base: &std::fs::File, rel: &Path) -> std::io::Result<Resolves> {
    let path = std::ffi::CString::new(rel.as_os_str().as_bytes())?;
    // SAFETY: an all-zero `open_how` is a valid value of a plain C struct.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_PATH | libc::O_CLOEXEC) as u64;
    how.resolve = libc::RESOLVE_BENEATH;
    // SAFETY: `path` is NUL-terminated, `how` is an `open_how` of the size passed, and `base` is
    // an open folder.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            base.as_raw_fd(),
            path.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd >= 0 {
        // SAFETY: `fd` was just opened and is owned by nothing else.
        drop(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as i32) });
        return Ok(Resolves::Beneath);
    }
    let e = std::io::Error::last_os_error();
    match e.raw_os_error() {
        Some(libc::EXDEV | libc::EACCES) => Ok(Resolves::Outside),
        Some(libc::ENOENT | libc::ELOOP | libc::ENOTDIR) => Ok(Resolves::Nothing),
        _ => Err(e),
    }
}

/// Move `from`'s entries into `to`, merging folders (never through a link) and refusing over a
/// non-folder; returns the moved paths. A failure moves them back, or `Kept` says some stayed.
fn merge_into(from: &Path, to: &Path) -> Result<Vec<PathBuf>, Failure> {
    use crate::ops::at::{c_path, cwd, open_dir, rename_free};
    let io = |e: std::io::Error| Failure::Tool(e.to_string());
    let from_fd = open_dir(cwd(), &c_path(from).map_err(io)?, true).map_err(io)?;
    let to_fd = open_dir(cwd(), &c_path(to).map_err(io)?, true).map_err(io)?;
    let mut moved = Vec::new();
    let Err(why) = merge_at(from_fd.as_fd(), to_fd.as_fd(), Path::new(""), &mut moved) else {
        return Ok(moved);
    };
    let back = |rel: &Path| -> std::io::Result<()> {
        let parent = rel.parent().unwrap_or(Path::new(""));
        let name = crate::ops::at::c_name(rel.file_name().unwrap_or_default())?;
        let there = steps(to_fd.as_fd(), parent)?;
        let here = steps(from_fd.as_fd(), parent)?;
        rename_free(there.as_fd(), &name, here.as_fd(), &name)
    };
    let mut stuck = false;
    for rel in moved.iter().rev() {
        if let Err(e) = back(rel) {
            log::warn!("reprisefs: cannot move {} back: {e}", rel.display());
            stuck = true;
        }
    }
    Err(match stuck {
        false => why,
        true => Failure::Kept(tr!(
            "extract-kept",
            why = why.why(),
            folder = from.display().to_string()
        )),
    })
}

/// Folder `rel` under `base`, each step opened without following a link.
fn steps(base: std::os::fd::BorrowedFd, rel: &Path) -> std::io::Result<std::os::fd::OwnedFd> {
    use crate::ops::at::{c_name, open_dir};
    let mut at = open_dir(base, c".", false)?;
    for part in rel.components() {
        at = open_dir(at.as_fd(), &c_name(part.as_os_str())?, false)?;
    }
    Ok(at)
}

/// [`merge_into`] of the folder open as `from`, `rel` under the archive's top, into `to`; an entry
/// that fails is named by its path under the top ([`crate::ops::in_tree`]).
fn merge_at(
    from: std::os::fd::BorrowedFd,
    to: std::os::fd::BorrowedFd,
    rel: &Path,
    moved: &mut Vec<PathBuf>,
) -> Result<(), Failure> {
    use crate::ops::at::{open_dir, rename_free};
    let io = |rel: &Path, e| Failure::Tool(crate::ops::in_tree(rel, e).to_string());
    for (name, is_dir) in crate::ops::entries_at(from).map_err(|e| io(rel, e))? {
        let rel = rel.join(OsStr::from_bytes(name.to_bytes()));
        match rename_free(from, &name, to, &name) {
            Ok(()) => {
                moved.push(rel);
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(&rel, e)),
        }
        if !is_dir {
            continue;
        }
        let there = open_dir(to, &name, false).map_err(|_| {
            Failure::Refused(tr!(
                "extract-not-a-folder",
                folder = rel.display().to_string()
            ))
        })?;
        let here = open_dir(from, &name, false).map_err(|e| io(&rel, e))?;
        merge_at(here.as_fd(), there.as_fd(), &rel, moved)?;
    }
    Ok(())
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

pub(crate) fn no_pipe() -> Failure {
    Failure::Tool(tr!("extract-no-output"))
}

/// The bytes of a tool's stderr kept for [`said`], which reads only its last line.
pub(crate) const STDERR_TAIL: usize = 4096;

/// Reads `r` to its end, keeping only its last `cap` bytes.
pub(crate) fn tail(r: &mut impl Read, cap: usize) -> Vec<u8> {
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

/// The line bsdtar ends on after an error it went past, which says nothing of the error.
pub(crate) const DELAYED: &str = "Error exit delayed from previous errors";

/// bsdtar's message without its own name, or `fallback` when it said nothing.
pub(crate) fn said(stderr: &[u8], fallback: &str) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty() && !l.contains(DELAYED))
        .unwrap_or("");
    let line = line.strip_prefix("bsdtar: ").unwrap_or(line).trim();
    match line.is_empty() {
        true => fallback.into(),
        false => line.to_string(),
    }
}

/// What the other entries [`and_others`] counts could not be.
#[derive(Clone, Copy)]
pub(crate) enum Others {
    Extracted,
    Read,
}

/// `why`, then how many `others` entries could not be extracted or read too.
pub(crate) fn and_others(why: String, others: usize, done: Others) -> String {
    match (others, done) {
        (0, _) => why,
        (n, Others::Extracted) => tr!("extract-and-others", why = why, others = n),
        (n, Others::Read) => tr!("compress-and-others", why = why, others = n),
    }
}

/// Starts `argv` with stderr piped, and stdout too when `stdout`; `path` replaces the `PATH` it
/// is looked up in.
pub(crate) fn spawn(
    argv: &[OsString],
    stdout: bool,
    path: Option<&OsStr>,
) -> Result<Child, Failure> {
    spawn_with(argv, stdout, |cmd| {
        if let Some(path) = path {
            cmd.env("PATH", path);
        }
    })
}

/// [`spawn`], `env` setting the command's environment. The child has no terminal, a stdin it
/// cannot read, and dies with the thread that started it.
pub(crate) fn spawn_with(
    argv: &[OsString],
    stdout: bool,
    env: impl FnOnce(&mut Command),
) -> Result<Child, Failure> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(&argv[0]);
    env(&mut cmd);
    // bsdtar asks for an encrypted entry's passphrase on the terminal, else on stdin, and asks
    // again forever when stdin answers nothing. With no terminal and a stdin it cannot read,
    // it fails at once.
    let stdin = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .map_or_else(|_| Stdio::null(), Stdio::from);
    // No terminal also means no terminal's signals, so the child dies with the thread that
    // started it instead. SAFETY: setsid and prctl are async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    cmd.args(&argv[1..]).stdin(stdin).stderr(Stdio::piped());
    cmd.stdout(match stdout {
        true => Stdio::piped(),
        false => Stdio::null(),
    });
    cmd.spawn()
        .map_err(|e| Failure::Tool(tr!("extract-no-bsdtar", why = e.to_string())))
}

/// The longest line [`lines`] takes: an output with a longer one is no listing.
const MAX_LINE: usize = 1 << 20;

/// Feeds each line of `out` to `each` until it ends, `each` says stop or `m` is cancelled
/// (`Ok(false)`). The child is stopped while `m` is paused.
pub(crate) fn lines(
    child: &mut Child,
    out: impl Read + AsRawFd,
    m: &Meter,
    each: impl FnMut(&[u8]) -> bool,
) -> std::io::Result<bool> {
    lines_ticking(child, out, m, each, || {})
}

/// [`lines`], calling `tick` at least every [`SLICE_MS`] while the child is unreaped. A line
/// past [`MAX_LINE`] fails the read.
pub(crate) fn lines_ticking(
    child: &mut Child,
    out: impl Read + AsRawFd,
    m: &Meter,
    mut each: impl FnMut(&[u8]) -> bool,
    tick: impl FnMut(),
) -> std::io::Result<bool> {
    let mut pending: Vec<u8> = Vec::new();
    let mut long = false;
    let read = chunks(
        child,
        out,
        m,
        |chunk| {
            pending.extend_from_slice(chunk);
            let mut start = 0;
            while let Some(nl) = pending[start..].iter().position(|&b| b == b'\n') {
                if !each(&pending[start..start + nl]) {
                    return false;
                }
                start += nl + 1;
            }
            pending.drain(..start);
            long = pending.len() > MAX_LINE;
            !long
        },
        tick,
    )?;
    if long {
        return Err(std::io::Error::other(tr!("archive-line-too-long")));
    }
    Ok(read && (pending.is_empty() || each(&pending)))
}

/// Feeds what `out` reads to `each` until it ends (`Ok(true)`), `each` says stop or `m` is
/// cancelled (`Ok(false)`), calling `tick` at least every [`SLICE_MS`] while the child is
/// unreaped. The child is stopped while `m` is paused.
pub(crate) fn chunks(
    child: &mut Child,
    mut out: impl Read + AsRawFd,
    m: &Meter,
    mut each: impl FnMut(&[u8]) -> bool,
    mut tick: impl FnMut(),
) -> std::io::Result<bool> {
    let fd = out.as_raw_fd();
    let pid = child.id() as libc::pid_t;
    let mut stopped = false;
    let mut buf = vec![0u8; 64 * 1024];
    let read = loop {
        if m.cancelled() {
            break Ok(false);
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
        tick();
        if ready == 0 {
            continue;
        }
        if ready < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break Err(e);
        }
        match out.read(&mut buf) {
            Ok(0) => break Ok(true),
            Ok(n) if !each(&buf[..n]) => break Ok(false),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => break Err(e),
        }
    };
    if stopped {
        // SAFETY: as above.
        unsafe { libc::kill(pid, libc::SIGCONT) };
    }
    read
}

/// Reaps `child` after [`lines`] read it to `read`: whether it exited cleanly. A child cut
/// short is killed first and counts as cancelled.
pub(crate) fn finish(child: Child, read: &std::io::Result<bool>) -> Result<bool, Failure> {
    finish_status(child, read).map(|s| s.success())
}

/// [`finish`], with how the child ended.
fn finish_status(
    mut child: Child,
    read: &std::io::Result<bool>,
) -> Result<std::process::ExitStatus, Failure> {
    if !matches!(read, Ok(true)) {
        let _ = child.kill();
        let _ = child.wait();
        return match read {
            Err(e) => Err(Failure::Tool(e.to_string())),
            _ => Err(Failure::Cancelled),
        };
    }
    child.wait().map_err(|e| Failure::Tool(e.to_string()))
}

/// The bytes the files under `root` hold, each file once however many links it has. No link is
/// followed, and an entry gone before it is read holds none.
fn landed_bytes(root: &Path) -> std::io::Result<u64> {
    use crate::ops::at::{c_path, cwd, open_dir, stat_at};
    let gone = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    // A folder still to list holds its parent's descriptor, so about one is open per level.
    let mut pending = Vec::new();
    let mut next = Some(std::rc::Rc::new(open_dir(cwd(), &c_path(root)?, true)?));
    while let Some(fd) = next.take() {
        for (name, _) in crate::ops::entries_at(fd.as_fd())? {
            let st = match stat_at(fd.as_fd(), &name) {
                Err(e) if gone(&e) => continue,
                st => st?,
            };
            match st.st_mode & libc::S_IFMT {
                libc::S_IFDIR => pending.push((fd.clone(), name)),
                libc::S_IFREG if st.st_nlink == 1 || seen.insert((st.st_dev, st.st_ino)) => {
                    bytes = bytes.saturating_add(st.st_size as u64);
                }
                _ => {}
            }
        }
        while let Some((parent, name)) = pending.pop() {
            match open_dir(parent.as_fd(), &name, false) {
                Err(e) if gone(&e) => {}
                fd => {
                    next = Some(fd?.into());
                    break;
                }
            }
        }
    }
    Ok(bytes)
}

/// The bytes of the files an extraction reported landing, each file counted at the largest size
/// it was seen at, however many of its links were reported.
#[derive(Default)]
struct Landed {
    sizes: std::collections::HashMap<(libc::dev_t, libc::ino_t), u64>,
    bytes: u64,
}

impl Landed {
    /// Counts the entry `st` and returns the total.
    fn add(&mut self, st: &libc::stat) -> u64 {
        let size = st.st_size as u64;
        if st.st_mode & libc::S_IFMT == libc::S_IFREG && size > 0 {
            let was = self.sizes.entry((st.st_dev, st.st_ino)).or_insert(0);
            self.bytes = self.bytes.saturating_add(size.saturating_sub(*was));
            *was = size.max(*was);
        }
        self.bytes
    }
}

/// What landed as `entry`, a name bsdtar reported, under the folder open as `into`, each step
/// opened without following a link.
fn landed_at(into: std::os::fd::BorrowedFd, entry: &[u8]) -> std::io::Result<libc::stat> {
    use crate::ops::at::{c_name, open_dir, stat_at};
    let name = crate::archive::unescape(entry);
    let mut parts = Vec::new();
    for c in Path::new(OsStr::from_bytes(&name)).components() {
        match c {
            Component::Normal(part) => parts.push(c_name(part)?),
            Component::CurDir => {}
            _ => return Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
    let (leaf, steps) = parts.split_last().ok_or(std::io::ErrorKind::InvalidInput)?;
    let mut at: Option<std::os::fd::OwnedFd> = None;
    for step in steps {
        let dir = at.as_ref().map_or(into, |fd| fd.as_fd());
        at = Some(open_dir(dir, step, false)?);
    }
    stat_at(at.as_ref().map_or(into, |fd| fd.as_fd()), leaf)
}

#[cfg(test)]
thread_local! {
    /// Set by a test to read no write counts, as on a system without `/proc/<pid>/io`.
    static NO_IO_COUNTS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The count `field` (`rchar`, `wchar`) of process `pid` and its reaped children, from
/// `/proc/<pid>/io`.
pub(crate) fn io_count(pid: u32, field: &str) -> Option<u64> {
    #[cfg(test)]
    if NO_IO_COUNTS.with(|c| c.get()) {
        return None;
    }
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).ok()?;
    io.lines()
        .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))?
        .trim()
        .parse()
        .ok()
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

    fn run_job(items: Vec<Archive>, pick: Decision) -> Vec<ArchiveEvent> {
        let (wtx, woke) = mpsc::channel();
        let mut job = ArchiveJob::extract(items, move || {
            let _ = wtx.send(());
        });
        let mut all = Vec::new();
        while !job.is_finished() {
            woke.recv_timeout(std::time::Duration::from_secs(30))
                .expect("a wake");
            for ev in job.poll() {
                if matches!(ev, ArchiveEvent::Failed { .. }) {
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
        assert_eq!(
            list_argv(a),
            ["bsdtar", "-tv", "--numeric-owner", "-f", "/d/-x.zip"]
        );
        assert_eq!(
            extract_argv(a, d),
            ["bsdtar", "-xv", "-f", "/d/-x.zip", "-C", "/d/-x"],
            "a name like an option is an option's argument"
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
    fn an_extraction_writing_past_its_listing_is_stopped_and_leaves_nothing() {
        let room = |_: &Path| Limits::from_free(u64::MAX, None);
        // One file over the listing's total, and files each under it that pass it together, with
        // the write count read and without it.
        let cases = [(1u64, 8usize << 20, 1000u64), (8, 1 << 20, 3 << 20)];
        for (counted, (files, size, listed)) in [true, false]
            .into_iter()
            .flat_map(|c| cases.map(|case| (c, case)))
        {
            NO_IO_COUNTS.with(|c| c.set(!counted));
            let t = tempfile::tempdir().unwrap();
            let src = t.path().join("src");
            std::fs::create_dir_all(&src).unwrap();
            let names: Vec<String> = (0..files).map(|i| format!("f{i}")).collect();
            for n in &names {
                std::fs::write(src.join(n), vec![b'x'; size]).unwrap();
            }
            let tar = t.path().join("p.tar.gz");
            let mut args = vec!["-czf", tar.to_str().unwrap()];
            args.extend(names.iter().map(String::as_str));
            bsdtar(&src, &args);
            let out = t.path().join("out");
            std::fs::create_dir(&out).unwrap();
            let a = Archive {
                src: tar,
                here: false,
                to: Some(out.clone()),
            };
            let lying = Listed {
                entries: files,
                bytes: listed,
                ..Listed::default()
            };
            assert_eq!(
                extract(&a, &Meter::default(), 0, lying, room),
                Err(Failure::Refused(
                    "it holds more than its listing says.".into()
                )),
                "{files} files, write count read: {counted}"
            );
            assert_eq!(
                std::fs::read_dir(&out).unwrap().count(),
                0,
                "nothing is left"
            );
        }
        NO_IO_COUNTS.with(|c| c.set(false));
    }

    #[test]
    fn without_a_write_count_an_extraction_past_its_listing_is_stopped_part_way() {
        NO_IO_COUNTS.with(|c| c.set(true));
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        // Long names fill bsdtar's stderr pipe in a few hundred lines, so it cannot run ahead of
        // the reader to the last entry.
        let files = 2000;
        let names: Vec<String> = (0..files).map(|i| format!("{i:0>200}")).collect();
        for n in &names {
            std::fs::write(src.join(n), vec![b'x'; 4096]).unwrap();
        }
        let tar = t.path().join("p.tar");
        let list = t.path().join("names");
        std::fs::write(&list, names.join("\n")).unwrap();
        bsdtar(
            &src,
            &["-cf", tar.to_str().unwrap(), "-T", list.to_str().unwrap()],
        );
        let out = t.path().join("out");
        std::fs::create_dir(&out).unwrap();
        let a = Archive {
            src: tar,
            here: false,
            to: Some(out.clone()),
        };
        let lying = Listed {
            entries: files as u64,
            bytes: 100 << 10,
            ..Listed::default()
        };
        let m = Meter::default();
        assert_eq!(
            extract(&a, &m, 0, lying, |_| Limits::from_free(u64::MAX, None)),
            Err(Failure::Refused(
                "it holds more than its listing says.".into()
            ))
        );
        NO_IO_COUNTS.with(|c| c.set(false));
        let reported = m.snapshot().items;
        assert!(
            reported < files as u64 / 2,
            "bsdtar is stopped within a pipe's worth of entries of the 26th, not after all {files}: {reported}"
        );
        assert_eq!(
            std::fs::read_dir(&out).unwrap().count(),
            0,
            "nothing is left"
        );
    }

    #[test]
    fn a_listing_that_floods_stderr_still_ends_with_its_count() {
        let script = "head -c 300000 /dev/zero | tr '\\0' w >&2; echo a; echo b; exit 1";
        let argv: Vec<OsString> = ["sh", "-c", script].map(OsString::from).to_vec();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(listing(&argv, 60, &Meter::default()));
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
    fn an_entry_with_a_parent_step_or_from_the_root_climbs_out() {
        for (entry, climbs) in [
            ("a/b.txt", false),
            ("../b.txt", true),
            ("a/../../b", true),
            ("a/..b/c", false),
            ("/etc/passwd", true),
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
                to: None,
            }],
            Decision::Abort,
        );
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { index: 0, text }, ArchiveEvent::Finished]
                if text.contains("../evil")),
            "{got:?}"
        );
        assert!(!t.path().join("evil").exists());
        assert!(!t.path().join("evil (2)").exists(), "no folder is made");
    }

    #[test]
    fn an_encrypted_zip_is_refused_as_password_protected_without_a_prompt() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::write(d.join("a.txt"), "secret").unwrap();
        let locked = [
            "--options",
            "zip:encryption=zipcrypt",
            "--passphrase",
            "pw",
            "-a",
            "-cf",
            "locked.zip",
            "a.txt",
        ];
        bsdtar(d, &locked);
        // A self-extracting zip, its leading bytes not a zip's, gets past the directory check
        // and is caught when bsdtar wants the passphrase.
        let zip = std::fs::read(d.join("locked.zip")).unwrap();
        std::fs::write(d.join("sfx.zip"), [b"MZ-stub-".as_slice(), &zip].concat()).unwrap();
        std::fs::create_dir(d.join("out")).unwrap();
        for name in ["locked.zip", "sfx.zip"] {
            let got = run_job(
                vec![Archive {
                    src: d.join(name),
                    here: false,
                    to: Some(d.join("out")),
                }],
                Decision::Abort,
            );
            assert!(
                matches!(&got[..], [ArchiveEvent::Failed { index: 0, text }, ArchiveEvent::Finished]
                    if text.ends_with("it is password-protected.")),
                "{name}: {got:?}"
            );
        }
        assert!(!d.join("out/locked").exists() && !d.join("out/sfx").exists());
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
                    to: None,
                },
                Archive {
                    src: d.join("pack.zip"),
                    here: false,
                    to: None,
                },
            ],
            Decision::Skip,
        );
        assert!(
            matches!(
                &got[..],
                [
                    ArchiveEvent::Failed { index: 0, .. },
                    ArchiveEvent::Landed { index: 1, into },
                    ArchiveEvent::Finished,
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
    fn a_paused_extract_let_go_is_cancelled_and_its_thread_ends() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir_all(d.join("src/d")).unwrap();
        std::fs::write(d.join("src/d/a"), "a").unwrap();
        bsdtar(&d.join("src"), &["-cf", "../pack.tar", "d"]);
        std::fs::remove_dir_all(d.join("src")).unwrap();
        let (wtx, woke) = mpsc::channel();
        let meter = Arc::new(Meter::default());
        meter.set_paused(true);
        let items = vec![Archive {
            src: d.join("pack.tar"),
            here: false,
            to: None,
        }];
        let job = ArchiveJob::on_thread(
            "reprisefs-extract",
            meter.clone(),
            move || {
                let _ = wtx.send(());
            },
            move |m, send| run(&items, Limits::of, m, send),
        );
        drop(job);
        assert!(
            crate::testfs::released(&woke, std::time::Duration::from_secs(5)),
            "the extraction's thread ends within 5 s"
        );
        assert!(meter.cancelled(), "the extraction is cancelled");
        let left: Vec<_> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["pack.tar"], "nothing is extracted");
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
                to: None,
            }],
            Decision::Skip,
        );
        assert!(
            matches!(
                &got[..],
                [
                    ArchiveEvent::Failed { index: 0, .. },
                    ArchiveEvent::Finished
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

    fn only(src: &Path, here: bool) -> Vec<ArchiveEvent> {
        run_job(
            vec![Archive {
                src: src.to_path_buf(),
                here,
                to: None,
            }],
            Decision::Abort,
        )
    }

    #[test]
    fn extract_all_makes_the_archives_folder_in_the_destination_numbered_past_a_taken_name() {
        let t = tempfile::tempdir().unwrap();
        let (d, to) = (t.path().join("d"), t.path().join("to"));
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::create_dir_all(to.join("pack")).unwrap();
        std::fs::write(d.join("src/a.txt"), "a").unwrap();
        bsdtar(&d.join("src"), &["-cf", "../pack.tar", "a.txt"]);
        let got = run_job(
            vec![Archive {
                src: d.join("pack.tar"),
                here: false,
                to: Some(to.clone()),
            }],
            Decision::Abort,
        );
        assert!(
            matches!(&got[..], [ArchiveEvent::Landed { into, .. }, ArchiveEvent::Finished]
                if *into == to.join("pack (2)")),
            "{got:?}"
        );
        assert_eq!(names_in(&to.join("pack (2)")), ["a.txt"]);
        assert_eq!(names_in(&d), ["pack.tar", "src"], "nothing lands beside it");
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn extract_here_puts_an_archive_with_hidden_entries_in_a_new_folder() {
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        std::fs::create_dir_all(src.join(".config/autostart")).unwrap();
        std::fs::create_dir(&d).unwrap();
        std::fs::write(src.join(".config/autostart/x.desktop"), "x").unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::os::unix::fs::symlink("a.txt", src.join("ln")).unwrap();
        let tar = d.join("pack.tar");
        bsdtar(
            &src,
            &["-cf", tar.to_str().unwrap(), ".config", "a.txt", "ln"],
        );
        let got = only(&tar, true);
        assert!(
            matches!(&got[..], [ArchiveEvent::Landed { into, .. }, ArchiveEvent::Finished]
                if *into == d.join("pack")),
            "{got:?}"
        );
        assert_eq!(names_in(&d), ["pack", "pack.tar"]);
        assert!(d.join("pack/.config/autostart/x.desktop").exists());
        assert_eq!(
            std::fs::read_link(d.join("pack/ln")).unwrap(),
            Path::new("a.txt"),
            "a link inside the archive's own tree comes out"
        );
    }

    #[test]
    fn extract_here_puts_an_archive_with_a_link_to_nothing_in_it_in_a_new_folder() {
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        std::fs::create_dir_all(d.join(".ssh")).unwrap();
        std::fs::write(d.join(".ssh/id"), "key").unwrap();
        std::fs::create_dir_all(src.join("pkg")).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        for (i, (name, target)) in [("docs", ".ssh"), ("pkg/lnk", "../.ssh")]
            .into_iter()
            .enumerate()
        {
            let _ = std::fs::remove_file(src.join(name));
            std::os::unix::fs::symlink(target, src.join(name)).unwrap();
            let tar = d.join(format!("pack{i}.tar"));
            let top = name.split('/').next().unwrap();
            let mut args = vec!["-cf", tar.to_str().unwrap(), "a.txt", "pkg"];
            args.extend((top != "pkg").then_some(top));
            bsdtar(&src, &args);
            let got = only(&tar, true);
            let into = d.join(format!("pack{i}"));
            assert!(
                matches!(&got[..], [ArchiveEvent::Landed { into: at, .. }, ArchiveEvent::Finished]
                    if *at == into),
                "{name}: {got:?}"
            );
            assert_eq!(
                std::fs::read_link(into.join(name)).unwrap(),
                Path::new(target)
            );
            assert!(into.join("a.txt").exists(), "{name}");
            assert!(!d.join("a.txt").exists(), "{name}: nothing merges into d");
            std::fs::remove_file(src.join(name)).unwrap();
        }
        assert_eq!(names_in(&d.join(".ssh")), ["id"]);
        assert!(!d.join("docs").exists() && !d.join("pkg").exists());
    }

    #[test]
    fn an_archive_with_a_link_out_of_its_folder_is_refused_and_leaves_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        std::fs::create_dir(&d).unwrap();
        for (i, links) in [
            &[("ln", "/etc")][..],
            &[("ln", "../../x")],
            &[("sub/b", ".."), ("ln", "sub/b/..")],
        ]
        .into_iter()
        .enumerate()
        {
            let _ = std::fs::remove_dir_all(&src);
            std::fs::create_dir_all(src.join("sub")).unwrap();
            for (name, target) in links {
                std::os::unix::fs::symlink(target, src.join(name)).unwrap();
            }
            let tar = d.join(format!("pack{i}.tar"));
            bsdtar(&src, &["-cf", tar.to_str().unwrap(), "sub", "ln"]);
            for here in [false, true] {
                let got = only(&tar, here);
                assert!(
                    matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                        if text.contains("ln")),
                    "{links:?} here={here}: {got:?}"
                );
                let mut want: Vec<String> = (0..=i).map(|n| format!("pack{n}.tar")).collect();
                want.sort();
                assert_eq!(names_in(&d), want, "{links:?} here={here}: nothing lands");
            }
        }
    }

    #[test]
    fn entries_that_fail_to_land_name_the_first_and_count_the_others() {
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        for dir in [
            d.clone(),
            src.join("one/d"),
            src.join("one/e"),
            src.join("two"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for f in ["one/d/f", "one/e/f", "one/a: b.txt", "two/d", "two/e"] {
            std::fs::write(src.join(f), "f").unwrap();
        }
        let (one, two) = (src.join("one.tar"), src.join("two.tar"));
        bsdtar(
            &src.join("one"),
            &["-cf", one.to_str().unwrap(), "d", "e", "a: b.txt"],
        );
        bsdtar(&src.join("two"), &["-cf", two.to_str().unwrap(), "d", "e"]);
        let both = d.join("both.tar");
        let cat = |p: &Path| format!("@{}", p.display());
        bsdtar(
            &src,
            &["-cf", both.to_str().unwrap(), &cat(&one), &cat(&two)],
        );
        let got = only(&both, false);
        let want = "Cannot extract both.tar: d: Can't replace existing directory with \
                    non-directory: Directory not empty, and 1 other item could not be extracted";
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text == want),
            "{got:?}"
        );
        assert_eq!(names_in(&d), ["both.tar"]);
    }

    #[test]
    fn an_archive_with_an_absolute_entry_is_refused_and_extracts_nothing() {
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir(&d).unwrap();
        std::fs::write(src.join("b"), "b").unwrap();
        let tar = d.join("abs.tar");
        bsdtar(
            &src,
            &["-cf", tar.to_str().unwrap(), "-P", "-s", ",^b,/abs/b,", "b"],
        );
        for here in [false, true] {
            let got = only(&tar, here);
            assert!(
                matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                    if text.contains("/abs/b")),
                "here={here}: {got:?}"
            );
            assert_eq!(names_in(&d), ["abs.tar"], "here={here}");
        }
    }

    #[test]
    fn extract_here_merges_into_a_folder_there_and_never_through_a_link() {
        let t = tempfile::tempdir().unwrap();
        let (d, src, away) = (
            t.path().join("d"),
            t.path().join("src"),
            t.path().join("away"),
        );
        for dir in [
            d.join("real"),
            src.join("real"),
            src.join("linked"),
            away.clone(),
        ] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(d.join("real/old.txt"), "old").unwrap();
        std::fs::write(src.join("real/new.txt"), "new").unwrap();
        std::fs::write(src.join("linked/x"), "x").unwrap();
        std::os::unix::fs::symlink(&away, d.join("linked")).unwrap();
        let (one, two) = (d.join("one.tar"), d.join("two.tar"));
        bsdtar(&src, &["-cf", one.to_str().unwrap(), "real"]);
        bsdtar(&src, &["-cf", two.to_str().unwrap(), "linked"]);

        let got = only(&one, true);
        assert!(
            matches!(
                &got[..],
                [ArchiveEvent::Landed { .. }, ArchiveEvent::Finished]
            ),
            "{got:?}"
        );
        assert_eq!(names_in(&d.join("real")), ["new.txt", "old.txt"]);

        let got = only(&two, true);
        assert!(
            matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text.contains("linked")),
            "{got:?}"
        );
        assert_eq!(
            std::fs::read_dir(&away).unwrap().count(),
            0,
            "nothing lands through the link"
        );
        assert_eq!(names_in(&d), ["linked", "one.tar", "real", "two.tar"]);
    }

    #[test]
    fn a_file_hard_linked_from_outside_the_extraction_leads_out() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("pack");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a"), "a").unwrap();
        std::fs::hard_link(root.join("sub/a"), root.join("b")).unwrap();
        assert_eq!(
            leads_out(&root).unwrap().out,
            None,
            "links within the folder are fine"
        );
        std::fs::write(t.path().join("secret"), "s").unwrap();
        std::fs::hard_link(t.path().join("secret"), root.join("sub/c")).unwrap();
        assert_eq!(leads_out(&root).unwrap().out.as_deref(), Some("sub/c"));
    }

    #[test]
    fn the_listing_counts_entries_sums_file_sizes_and_sees_hidden_ones() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(src.join("dir name")).unwrap();
        std::fs::write(src.join("dir name/a b"), "abc").unwrap();
        std::fs::write(src.join("e"), "12345").unwrap();
        std::fs::hard_link(src.join("e"), src.join("dir name/e2")).unwrap();
        std::os::unix::fs::symlink("e", src.join("l -> x")).unwrap();
        let tar = t.path().join("p.tar");
        bsdtar(
            &src,
            &["-cf", tar.to_str().unwrap(), "e", "dir name", "l -> x"],
        );
        let got = list(&tar, &Meter::default()).unwrap();
        assert_eq!(
            got,
            Listed {
                entries: 5,
                bytes: 8,
                ..Listed::default()
            },
            "a hard link adds no bytes"
        );
        std::fs::write(src.join(".rc"), "").unwrap();
        bsdtar(&src, &["-cf", tar.to_str().unwrap(), "e", ".rc"]);
        assert!(list(&tar, &Meter::default()).unwrap().hidden);
        std::fs::create_dir_all(src.join("bin")).unwrap();
        std::fs::write(src.join("bin/sudo"), "").unwrap();
        std::fs::create_dir_all(src.join("pkg/bin")).unwrap();
        bsdtar(&src, &["-cf", tar.to_str().unwrap(), "e", "pkg"]);
        assert!(
            !list(&tar, &Meter::default()).unwrap().bin,
            "a bin below the top"
        );
        bsdtar(&src, &["-cf", tar.to_str().unwrap(), "e", "bin"]);
        assert!(list(&tar, &Meter::default()).unwrap().bin);
    }

    #[test]
    fn extract_here_keeps_a_top_level_bin_apart_only_in_the_home_folder() {
        let t = tempfile::tempdir().unwrap();
        let (home, other) = (t.path().join("home"), t.path().join("other"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        let bin = Listed {
            bin: true,
            ..Listed::default()
        };
        assert!(kept_apart(bin, &home, Some(&home)));
        assert!(
            kept_apart(bin, &home.join("."), Some(&t.path().join("other/../home"))),
            "the same folder by another path"
        );
        assert!(!kept_apart(bin, &other, Some(&home)));
        assert!(!kept_apart(bin, &home, None));
        assert!(!kept_apart(Listed::default(), &home, Some(&home)));
        let hidden = Listed {
            hidden: true,
            ..Listed::default()
        };
        assert!(kept_apart(hidden, &other, None));
    }

    #[test]
    fn an_extraction_takes_ninety_percent_of_the_free_space_and_a_million_entries() {
        let at = |bytes, entries| Listed {
            entries,
            bytes,
            ..Listed::default()
        };
        let limits = Limits::from_free(1000, None);
        assert_eq!(
            limits,
            Limits {
                bytes: 900,
                entries: 1_000_000
            }
        );
        assert!(limits.admit(at(900, 1_000_000)).is_ok());
        let e = limits.admit(at(901, 1)).unwrap_err();
        assert!(
            matches!(&e, Failure::Refused(why) if why.contains("expands to")),
            "{e:?}"
        );
        let e = limits.admit(at(1, 1_000_001)).unwrap_err();
        assert!(
            matches!(&e, Failure::Refused(why) if why.contains("1000001 entries")),
            "{e:?}"
        );
        assert_eq!(
            Limits::from_free(1000, Some(7)).entries,
            7,
            "fewer free inodes cap it lower"
        );
    }

    #[test]
    fn an_archive_past_the_folder_limits_fails_and_writes_nothing() {
        let t = tempfile::tempdir().unwrap();
        let d = t.path();
        std::fs::create_dir(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.txt"), "a").unwrap();
        bsdtar(&d.join("src"), &["-cf", "../pack.tar", "a.txt"]);
        std::fs::remove_dir_all(d.join("src")).unwrap();
        for here in [false, true] {
            let (wtx, woke) = mpsc::channel();
            let items = vec![Archive {
                src: d.join("pack.tar"),
                here,
                to: None,
            }];
            let mut job = ArchiveJob::extract_within(
                items,
                |_| Limits::from_free(0, None),
                move || {
                    let _ = wtx.send(());
                },
            );
            let mut got = Vec::new();
            while !job.is_finished() {
                woke.recv_timeout(std::time::Duration::from_secs(30))
                    .expect("a wake");
                for ev in job.poll() {
                    if matches!(ev, ArchiveEvent::Failed { .. }) {
                        job.meter().decide(Decision::Abort);
                    }
                    got.push(ev);
                }
            }
            assert!(
                matches!(&got[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                    if text.contains("free space")),
                "here={here}: {got:?}"
            );
            assert_eq!(names_in(d), ["pack.tar"], "here={here}");
        }
    }

    #[test]
    fn a_device_node_lists_with_no_bytes() {
        let line = b"crw-r--r--  0 0      0         1,3 Jan  1  2020 dev/null";
        assert_eq!(listed_line(line), Some((b'c', 0, &b"dev/null"[..])));
    }

    /// Makes `rel` under `root` writable again so the temp dir can go.
    fn writable(root: &Path, rel: &[&str]) {
        use std::os::unix::fs::PermissionsExt;
        for r in rel {
            let _ = std::fs::set_permissions(root.join(r), std::fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn read_only_folders_extract_and_keep_their_modes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (d, src) = (t.path().join("d"), t.path().join("src"));
        std::fs::create_dir_all(src.join("pkg/sub")).unwrap();
        std::fs::create_dir(&d).unwrap();
        std::fs::write(src.join("pkg/sub/g.txt"), "g").unwrap();
        let mode = |m| std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(src.join("pkg/sub"), mode(0o500)).unwrap();
        std::fs::set_permissions(src.join("pkg"), mode(0o555)).unwrap();
        let tar = d.join("pack.tar");
        bsdtar(&src, &["-cf", tar.to_str().unwrap(), "pkg"]);
        writable(&src, &["pkg", "pkg/sub"]);
        for (here, into) in [(true, d.clone()), (false, d.join("pack"))] {
            let got = only(&tar, here);
            let g = std::fs::read_to_string(into.join("pkg/sub/g.txt")).ok();
            let modes: Vec<u32> = ["pkg", "pkg/sub"]
                .iter()
                .map(|r| {
                    std::fs::metadata(into.join(r)).map_or(0, |m| m.permissions().mode() & 0o777)
                })
                .collect();
            writable(&into, &["pkg", "pkg/sub"]);
            assert!(
                matches!(
                    &got[..],
                    [ArchiveEvent::Landed { .. }, ArchiveEvent::Finished]
                ),
                "here={here}: {got:?}"
            );
            assert_eq!(g.as_deref(), Some("g"), "here={here}");
            assert_eq!(modes, [0o555, 0o500], "here={here}");
            assert_eq!(
                names_in(&d).len(),
                if here { 2 } else { 3 },
                "here={here}: no staging is left"
            );
        }
    }

    #[test]
    fn a_merge_that_stops_part_way_moves_nothing_into_place() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (src, away) = (t.path().join("src"), t.path().join("away"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::create_dir(&away).unwrap();
        std::fs::write(src.join("sub/x.txt"), "x").unwrap();
        let mut names = vec![];
        for i in 0..6 {
            for p in ["a", "z"] {
                std::fs::write(src.join(format!("{p}{i}.txt")), "n").unwrap();
                names.push(format!("{p}{i}.txt"));
            }
        }
        names.sort();
        let mut args: Vec<&str> = vec!["-cf", "../pack.tar"];
        args.extend(names[..6].iter().map(String::as_str));
        args.push("sub");
        args.extend(names[6..].iter().map(String::as_str));
        for (case, there) in ["a link", "a read-only folder"].into_iter().enumerate() {
            let d = t.path().join(format!("d{case}"));
            std::fs::create_dir(&d).unwrap();
            let _ = std::fs::remove_file(t.path().join("pack.tar"));
            bsdtar(&src, &args);
            std::fs::rename(t.path().join("pack.tar"), d.join("pack.tar")).unwrap();
            match case {
                0 => std::os::unix::fs::symlink(&away, d.join("sub")).unwrap(),
                _ => {
                    std::fs::create_dir(d.join("sub")).unwrap();
                    std::fs::set_permissions(d.join("sub"), std::fs::Permissions::from_mode(0o555))
                        .unwrap();
                }
            }
            let got = only(&d.join("pack.tar"), true);
            writable(&d, &["sub"]);
            assert!(
                matches!(
                    &got[..],
                    [ArchiveEvent::Failed { .. }, ArchiveEvent::Finished]
                ),
                "{there}: {got:?}"
            );
            assert_eq!(
                names_in(&d),
                ["pack.tar", "sub"],
                "{there}: nothing moved stays"
            );
            assert_eq!(std::fs::read_dir(&away).unwrap().count(), 0, "{there}");
        }
    }

    #[test]
    fn a_failed_merge_names_the_entry_by_its_path_in_the_archive() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (src, d) = (t.path().join("src"), t.path().join("d"));
        std::fs::create_dir_all(src.join("sub/deep")).unwrap();
        std::fs::create_dir_all(d.join("sub/deep")).unwrap();
        std::fs::write(src.join("sub/x.txt"), "x").unwrap();
        std::fs::write(src.join("sub/deep/y.txt"), "y").unwrap();
        let (one, two) = (d.join("one.tar"), d.join("two.tar"));
        bsdtar(&src, &["-cf", one.to_str().unwrap(), "sub/x.txt"]);
        bsdtar(&src, &["-cf", two.to_str().unwrap(), "sub/deep/y.txt"]);
        let shut = |p: &Path, mode| {
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap()
        };
        let failed = |archive: &Path| match &only(archive, true)[..] {
            [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished] => text.clone(),
            got => panic!("{}: {got:?}", archive.display()),
        };
        let denied = "Permission denied (os error 13)";

        shut(&d.join("sub"), 0o555);
        let got = failed(&one);
        shut(&d.join("sub"), 0o755);
        assert_eq!(got, format!("Cannot extract one.tar: sub/x.txt: {denied}"));

        shut(&d.join("sub/deep"), 0o555);
        let got = failed(&two);
        shut(&d.join("sub/deep"), 0o755);
        assert_eq!(
            got,
            format!("Cannot extract two.tar: sub/deep/y.txt: {denied}")
        );

        shut(&d, 0o555);
        let got = failed(&one);
        shut(&d, 0o755);
        assert_eq!(got, format!("Cannot extract one.tar: {denied}"));
        assert_eq!(names_in(&d), ["one.tar", "sub", "two.tar"]);
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
                to: None,
            }],
            Decision::Abort,
        );
        assert!(
            matches!(&got[..], [ArchiveEvent::Landed { into, .. }, ArchiveEvent::Finished] if into == d),
            "{got:?}"
        );
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b");
    }
}
