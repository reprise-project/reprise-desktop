//! Extracting archives with `bsdtar` (zip, 7z, tar), never with `-P`, each into a fresh folder;
//! docs/security.md "Extracting archives" has the rules.

use crate::names;
use crate::progress::{Decision, Meter, Phase, Totals};
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

pub const FAILURE_TITLE: &str = "Error Extracting File";

/// Suffixes a folder named after its archive drops, longest first.
const SUFFIXES: [&str; 10] = [
    ".tar.gz", ".tar.xz", ".tar.zst", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tar", ".zip", ".7z",
];

const SLICE_MS: i32 = 50;

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
    fn of(dir: &Path) -> Limits {
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
            return Err(Failure::Refused(format!(
                "it holds {} entries, more than the {} one extraction here takes.",
                l.entries, self.entries
            )));
        }
        if l.bytes > self.bytes {
            return Err(Failure::Refused(format!(
                "it expands to {}, more than the {} the free space here allows.",
                crate::props::short_size(l.bytes),
                crate::props::short_size(self.bytes)
            )));
        }
        Ok(())
    }
}

/// One archive of a batch: into its own folder, or `here` beside it.
#[derive(Clone, Debug)]
pub struct Archive {
    pub src: PathBuf,
    pub here: bool,
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
        format!("Cannot extract {name}: {}", self.why())
    }

    fn why(&self) -> String {
        match self {
            Failure::Climbs(entry) => {
                format!("it holds {entry}, which would land outside the folder.")
            }
            Failure::Tool(why) | Failure::Refused(why) | Failure::Kept(why) => why.clone(),
            Failure::Cancelled => "the operation was cancelled.".into(),
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
    listing(&list_argv(archive), m)
}

/// Reads the `bsdtar -tv` listing `argv` prints, refusing an entry that lands outside, and
/// draining its stderr on a thread so a chatty listing cannot fill that pipe and stall.
fn listing(argv: &[OsString], m: &Meter) -> Result<Listed, Failure> {
    let mut child = spawn_with(argv, true, c_dates)?;
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
        got.hidden |= hidden_at_top(entry);
        true
    });
    let status = finish(child, &read);
    let stderr = drain.join().unwrap_or_default();
    if let Some(entry) = climbs {
        return Err(Failure::Climbs(entry));
    }
    match status? {
        true if unread => Err(Failure::Tool(UNREAD.into())),
        true => Ok(got),
        false => Err(Failure::Tool(said(&stderr, UNREAD))),
    }
}

/// The listing's dates in the C locale's three words, the rest of the locale kept.
fn c_dates(cmd: &mut Command) {
    if let Some(all) = std::env::var_os("LC_ALL") {
        cmd.env_remove("LC_ALL").env("LC_CTYPE", all);
    }
    cmd.env("LC_TIME", "C");
}

/// The type letter, size and the rest (the name, then a link's target) of a `bsdtar -tv
/// --numeric-owner` line: mode, links, uid, gid, size, a three-word date, one space. A device
/// node lists `major,minor` as its size and holds no bytes.
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

fn hidden_at_top(entry: &[u8]) -> bool {
    let first = Path::new(OsStr::from_bytes(entry))
        .components()
        .find_map(|c| match c {
            Component::Normal(n) => Some(n.as_bytes().starts_with(b".")),
            _ => None,
        });
    first == Some(true)
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
    let dir = a
        .src
        .parent()
        .ok_or_else(|| Failure::Tool("it has no folder".into()))?;
    limits(dir).admit(listed)?;
    let here = a.here && !listed.hidden;
    let name = folder_name(a.src.file_name().unwrap_or_default());
    let into = make_folder(dir, &if here { staging(&name) } else { name.clone() })?;
    let remove = || discard(&into);
    let undo = |e: Failure| {
        remove();
        e
    };
    let argv = extract_argv(&a.src, &into);
    let mut child = spawn(&argv, false, None).map_err(undo)?;
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
        return Err(undo(Failure::Tool(said(last.as_bytes(), UNREAD))));
    }
    let io = |e: std::io::Error| Failure::Tool(e.to_string());
    let modes = open_up(&into).map_err(io).map_err(undo)?;
    let reach = leads_out(&into).map_err(io).map_err(undo)?;
    if let Some(entry) = reach.out {
        return Err(undo(Failure::Refused(format!(
            "it holds {entry}, a link to outside the folder."
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

/// Move what folder `from` holds into `to`, keeping what `to` has: a folder in both is merged
/// into (never through a link), one meeting a non-folder there refuses. Returns what moved, by
/// path under both; on a failure each goes back, or `Kept` says some could not and `from` stays.
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
        true => Failure::Kept(format!(
            "{} Some of its entries stay in place, and the rest in {}.",
            why.why(),
            from.display()
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

fn merge_at(
    from: std::os::fd::BorrowedFd,
    to: std::os::fd::BorrowedFd,
    rel: &Path,
    moved: &mut Vec<PathBuf>,
) -> Result<(), Failure> {
    use crate::ops::at::{open_dir, rename_free};
    let io = |e: std::io::Error| Failure::Tool(e.to_string());
    for (name, is_dir) in crate::ops::entries_at(from).map_err(io)? {
        let rel = rel.join(OsStr::from_bytes(name.to_bytes()));
        match rename_free(from, &name, to, &name) {
            Ok(()) => {
                moved.push(rel);
                continue;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io(e)),
        }
        if !is_dir {
            continue;
        }
        let there = open_dir(to, &name, false).map_err(|_| {
            Failure::Refused(format!(
                "it holds the folder {}, which is not a folder here.",
                rel.display()
            ))
        })?;
        let here = open_dir(from, &name, false).map_err(io)?;
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
    Failure::Tool("bsdtar gave no output".into())
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

const UNREAD: &str = "bsdtar could not read it.";

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

/// [`spawn`], `env` setting the command's environment.
fn spawn_with(
    argv: &[OsString],
    stdout: bool,
    env: impl FnOnce(&mut Command),
) -> Result<Child, Failure> {
    let mut cmd = Command::new(&argv[0]);
    env(&mut cmd);
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
pub(crate) fn lines(
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
pub(crate) fn finish(mut child: Child, read: &std::io::Result<bool>) -> Result<bool, Failure> {
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
    fn a_listing_that_floods_stderr_still_ends_with_its_count() {
        let script = "head -c 300000 /dev/zero | tr '\\0' w >&2; echo a; echo b; exit 1";
        let argv: Vec<OsString> = ["sh", "-c", script].map(OsString::from).to_vec();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(listing(&argv, &Meter::default()));
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
            }],
            Decision::Abort,
        )
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
                hidden: false
            },
            "a hard link adds no bytes"
        );
        std::fs::write(src.join(".rc"), "").unwrap();
        bsdtar(&src, &["-cf", tar.to_str().unwrap(), "e", ".rc"]);
        assert!(list(&tar, &Meter::default()).unwrap().hidden);
    }

    #[test]
    fn an_extraction_takes_ninety_percent_of_the_free_space_and_a_million_entries() {
        let at = |bytes, entries| Listed {
            entries,
            bytes,
            hidden: false,
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
            matches!(&got[..], [ArchiveEvent::Landed { into, .. }, ArchiveEvent::Finished] if into == d),
            "{got:?}"
        );
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(d.join("b.txt")).unwrap(), "b");
    }
}
