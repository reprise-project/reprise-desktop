//! Writing into a zip browsed as a compressed folder: a file saved back into it, files dropped
//! or pasted onto it. The zip is rewritten whole by a chain of bsdtar children: the first reads
//! the zip, opened once and never through a link, less the entries being replaced (and is left
//! out when none stays); each next one adds one folder's files by name; the last writes a zip
//! into a temp file beside the old one.
//! The temp file is listed and must hold exactly the old entries kept plus the files added, and
//! is renamed over the zip only while the zip is still the file that was read. A failure, Cancel
//! or the deadline removes the temp file. A crash leaves it hidden beside the zip
//! (`.<name>.<tag>.zipwrite`), the zip untouched; a rewrite holds its
//! temp file locked, and the next rewrite of that zip removes one no rewrite holds that was not
//! written to for that rewrite's deadline. docs/security.md "Writing into zips" has the rules.

use crate::archive::{self, Entry, Kind, Scratch};
use crate::extract::{self, ArchiveEvent, ArchiveJob, Failure, MAX_ENTRIES, STDERR_TAIL};
use crate::progress::{Meter, Phase, Totals};
use reprisekit::tr;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::Permissions;
use std::io::Write;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A zip holding nothing: its end-of-central-directory record alone.
pub const EMPTY_ZIP: [u8; 22] = [
    b'P', b'K', 5, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

/// The most bytes a save into a zip ([`ArchiveJob::update_entry`]) rewrites, counted once on the
/// zip itself and once on the files it holds.
pub const UPDATE_MOST: u64 = 256 << 20;

/// Seconds of running time that replace every rewrite's deadline, to try a timeout by hand.
const DEADLINE_ENV: &str = "REPRISE_ZIP_DEADLINE_SECS";

/// What a rewrite's temp file ends in, apart from a document save's `.tmp`, which only its
/// sweep removes.
const TEMP_SUFFIX: &str = "zipwrite";

/// How often a rewrite's deadline counts the time it ran.
const WATCH_EVERY: Duration = Duration::from_millis(250);

/// Zip bytes per added file or entry kept, past its data: its local and central headers (the
/// name counted apart), data descriptor and zip64 fields, with room to spare.
const ENTRY_OVERHEAD: u64 = 512;

/// Whether `path` starts as a zip does: a local file header, or the end record of an empty one.
pub fn is_zip(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head));
    read.is_ok() && zip_head(&head)
}

/// Whether a rewrite may write the zip whose metadata is `meta`: a file of the user's with its
/// write bit, as the rewrite's own open checks.
pub fn takes_writes(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    // SAFETY: geteuid has no preconditions.
    meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o200 != 0
}

fn zip_head(head: &[u8; 4]) -> bool {
    matches!(head, [b'P', b'K', 3, 4] | [b'P', b'K', 5, 6])
}

/// Files of the folder `dir` added at the zip's top under their paths below it (`names`, plain
/// names only). A file replaces the zip's file of its path, a folder only the folder's own entry,
/// so a folder merges. A file over a folder, a folder over a file, or two of one path, is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adding {
    pub dir: PathBuf,
    pub names: Vec<PathBuf>,
}

/// Why a [`rewrite`] left the zip as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unwritten {
    Cancelled,
    /// Its deadline passed.
    TooLong,
    Failed(String),
}

impl Unwritten {
    /// The reason an error box gives.
    pub fn why(&self) -> String {
        match self {
            Unwritten::Cancelled => Failure::Cancelled.why(),
            Unwritten::TooLong => tr!("archive-update-too-long"),
            Unwritten::Failed(why) => why.clone(),
        }
    }
}

/// What one [`rewrite`] may take: `most` bytes of zip and of files, and `deadline` of running
/// time (a pause stops its clock), else twice the CPU time its bsdtar children may take.
#[derive(Clone, Copy, Debug, Default)]
pub struct Bound {
    pub most: Option<u64>,
    pub deadline: Option<Duration>,
}

impl ArchiveJob {
    /// Adds `adds` to the zip `zip` on a thread ([`rewrite`]): one [`ArchiveEvent::Landed`] with
    /// the zip, or one [`ArchiveEvent::Failed`], a passed deadline among them. A cancelled job
    /// sends neither and leaves the zip as it was.
    pub fn add_to_zip(zip: PathBuf, adds: Vec<Adding>, wake: impl Fn() + Send + 'static) -> Self {
        ArchiveJob::rewriting("reprisefs-add-to-zip", zip, wake, move |zip, m| {
            rewrite(zip, &adds, m, Bound::default())
        })
    }

    /// Writes `bytes` over the file `inner` of the zip `zip` on a thread, keeping the entry's
    /// mode: the save of a file opened out of a compressed folder, refused past [`UPDATE_MOST`].
    /// Once the zip holds it, the read-only archive `copy` the file was opened from is written
    /// again ([`crate::archivecopy::rewrite_copy`]). Its events are [`ArchiveJob::add_to_zip`]'s.
    pub fn update_entry(
        zip: PathBuf,
        inner: PathBuf,
        bytes: Vec<u8>,
        copy: Option<PathBuf>,
        wake: impl Fn() + Send + 'static,
    ) -> Self {
        ArchiveJob::rewriting("reprisefs-update-zip", zip, wake, move |zip, m| {
            update(zip, &inner, &bytes, m)?;
            if let Some(copy) = &copy
                && let Err(e) = crate::archivecopy::rewrite_copy(copy, &bytes)
            {
                log::warn!("reprisefs: cannot rewrite the copy {}: {e}", copy.display());
            }
            Ok(())
        })
    }

    fn rewriting(
        name: &str,
        zip: PathBuf,
        wake: impl Fn() + Send + 'static,
        body: impl FnOnce(&Path, &Arc<Meter>) -> Result<(), Unwritten> + Send + 'static,
    ) -> Self {
        let meter = Arc::new(Meter::default());
        let watched = meter.clone();
        ArchiveJob::on_thread(name, meter, wake, move |_, send| {
            let ev = match body(&zip, &watched) {
                Ok(()) => Some(ArchiveEvent::Landed {
                    index: 0,
                    into: zip.clone(),
                }),
                Err(Unwritten::Cancelled) => None,
                Err(e) => Some(ArchiveEvent::Failed {
                    index: 0,
                    text: failure(&zip, &e.why()),
                }),
            };
            if let Some(ev) = ev
                && !send(ev)
            {
                return;
            }
            send(ArchiveEvent::Finished);
        })
    }
}

/// [`ArchiveJob::update_entry`]'s rewrite: `bytes` staged in a private folder as `inner`, given
/// the mode of the entry it replaces.
fn update(zip: &Path, inner: &Path, bytes: &[u8], m: &Arc<Meter>) -> Result<(), Unwritten> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let failed = |why: String| Unwritten::Failed(why);
    let io = |e: std::io::Error| failed(e.to_string());
    if !plain(inner) {
        return Err(failed(tr!("archive-unread")));
    }
    open_zip(zip).map_err(failed)?;
    if bytes.len() as u64 > UPDATE_MOST {
        return Err(failed(too_large(bytes.len() as u64, UPDATE_MOST)));
    }
    let stage = Scratch::new().map_err(io)?;
    let to = stage.path().join(inner);
    if let Some(parent) = to.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(io)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o644)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&to)
        .map_err(io)?;
    file.write_all(bytes).map_err(io)?;
    drop(file);
    let adding = Adding {
        dir: stage.path().to_path_buf(),
        names: vec![inner.to_path_buf()],
    };
    let bound = Bound {
        most: Some(UPDATE_MOST),
        deadline: None,
    };
    rewrite_keeping(zip, &[adding], m, bound, true)
}

/// The error box's text for a write into `zip` that failed for `why`.
pub fn failure(zip: &Path, why: &str) -> String {
    let name = zip.file_name().unwrap_or_default().to_string_lossy();
    tr!("archive-update-failed", name = &*name, why = why)
}

fn too_large(bytes: u64, most: u64) -> String {
    let (size, most) = (
        crate::props::short_size(bytes),
        crate::props::short_size(most),
    );
    tr!("archive-update-too-large", size = size, most = most)
}

/// Cancels a meter once it has run, outside its pauses, for its deadline, unless dropped first.
struct Watch {
    state: Arc<(Mutex<Watched>, Condvar)>,
}

struct Watched {
    left: Duration,
    done: bool,
    passed: bool,
}

impl Watch {
    fn start(m: Arc<Meter>, after: Duration) -> Watch {
        let state = Arc::new((
            Mutex::new(Watched {
                left: after,
                done: false,
                passed: false,
            }),
            Condvar::new(),
        ));
        let shared = state.clone();
        std::thread::spawn(move || {
            let (lock, cv) = &*shared;
            let Ok(mut held) = lock.lock() else {
                return;
            };
            let mut from = Instant::now();
            while !held.done {
                let now = Instant::now();
                if !m.paused() {
                    held.left = held.left.saturating_sub(now - from);
                }
                from = now;
                if held.left.is_zero() {
                    held.passed = true;
                    m.cancel();
                    return;
                }
                let step = held.left.min(WATCH_EVERY);
                held = match cv.wait_timeout(held, step) {
                    Ok((h, _)) => h,
                    Err(_) => return,
                };
            }
        });
        Watch { state }
    }

    /// Gives the rewrite `after` more running time from now.
    fn reset(&self, after: Duration) {
        if let Ok(mut s) = self.state.0.lock() {
            s.left = after;
        }
        self.state.1.notify_all();
    }

    /// Whether the deadline passed and the meter was cancelled for it.
    fn passed(&self) -> bool {
        self.state.0.lock().is_ok_and(|s| s.passed)
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Ok(mut s) = self.state.0.lock() {
            s.done = true;
        }
        self.state.1.notify_all();
    }
}

/// Whether `path` is relative and plain names only.
fn plain(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

/// One entry as the check of the rewritten zip compares it.
type Listed = (PathBuf, Kind, u64);

fn listed(e: &Entry) -> Listed {
    (e.path.clone(), e.kind, e.size)
}

/// Rewrites the zip `zip` with `adds` added ([`Adding`]), as the module says. Refused when `zip`
/// is a link, not a regular zip file this user owns, or encrypted; past [`MAX_ENTRIES`] entries,
/// `bound`, or what the zip's folder has room for. Cancelling `m` stops it. A rewrite that
/// renamed its zip into place has landed, however late.
pub fn rewrite(zip: &Path, adds: &[Adding], m: &Arc<Meter>, bound: Bound) -> Result<(), Unwritten> {
    rewrite_keeping(zip, adds, m, bound, false)
}

/// [`rewrite`]; with `keep_modes` each added file takes the mode of the zip's file it replaces.
fn rewrite_keeping(
    zip: &Path,
    adds: &[Adding],
    m: &Arc<Meter>,
    bound: Bound,
    keep_modes: bool,
) -> Result<(), Unwritten> {
    let forced = std::env::var(DEADLINE_ENV)
        .ok()
        .and_then(|s| s.parse().ok());
    let bound = Bound {
        deadline: forced.map(Duration::from_secs).or(bound.deadline),
        ..bound
    };
    let cpu = archive::cpu_secs(archive::archive_bytes(zip));
    let first = bound.deadline.unwrap_or(Duration::from_secs(2 * cpu));
    let watch = Watch::start(m.clone(), first);
    let wrote = rewrite_watched(zip, adds, (m, &watch), bound, keep_modes, first);
    match wrote {
        Ok(()) => Ok(()),
        Err(_) if watch.passed() => Err(Unwritten::TooLong),
        Err(_) if m.cancelled() => Err(Unwritten::Cancelled),
        Err(why) => Err(Unwritten::Failed(why)),
    }
}

/// Every entry of `old` by its path, and each folder above one that the zip holds only by the
/// paths below it.
fn held_kinds(old: &[Entry]) -> HashMap<&Path, Kind> {
    let mut held: HashMap<&Path, Kind> = HashMap::new();
    for e in old {
        held.insert(&e.path, e.kind);
    }
    for e in old {
        for a in e.path.ancestors().skip(1) {
            if a.as_os_str().is_empty() {
                break;
            }
            held.entry(a).or_insert(Kind::Folder);
        }
    }
    held
}

fn rewrite_watched(
    zip: &Path,
    adds: &[Adding],
    (m, watch): (&Meter, &Watch),
    Bound { most, deadline }: Bound,
    keep_modes: bool,
    stale: Duration,
) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let io = |e: std::io::Error| e.to_string();
    let (file, before) = open_zip(zip)?;
    if let Some(most) = most
        && before.len() > most
    {
        return Err(too_large(before.len(), most));
    }
    if archive::zip_encrypted(zip) {
        return Err(tr!("archive-locked"));
    }
    let staged: HashSet<&Path> = match keep_modes {
        true => adds
            .iter()
            .flat_map(|a| &a.names)
            .map(PathBuf::as_path)
            .collect(),
        false => HashSet::new(),
    };
    let mut modes: HashMap<PathBuf, u32> = HashMap::new();
    let (old, count) = archive::list_noting(zip, m, &mut |e, mode| {
        if e.kind == Kind::File && staged.contains(e.path.as_path()) {
            modes.insert(e.path.clone(), mode);
        }
    })?;
    let walked = walk(adds, m, MAX_ENTRIES.saturating_sub(old.len() as u64))?;
    let kinds: HashMap<&Path, Kind> = walked.iter().map(|(p, k, _)| (p.as_path(), *k)).collect();
    let held = held_kinds(&old);
    let clash = (walked.iter()).any(|(p, k, _)| held.get(p.as_path()).is_some_and(|h| h != k));
    if clash || kinds.len() != walked.len() {
        return Err(crate::ops::PutError::Occupied.to_string());
    }
    for a in adds {
        for name in &a.names {
            if let Some(&mode) = modes.get(name) {
                // bsdtar reads the staged file, so it keeps owner read and write.
                let mode = std::fs::Permissions::from_mode(mode & 0o777 | 0o600);
                std::fs::set_permissions(a.dir.join(name), mode).map_err(io)?;
            }
        }
    }
    let mut expected: Vec<Listed> = (old.iter())
        .filter(|e| !kinds.contains_key(e.path.as_path()))
        .map(listed)
        .chain(walked.iter().cloned())
        .collect();
    let bytes = expected.iter().map(|e| e.2).fold(0u64, u64::saturating_add);
    if let Some(most) = most
        && bytes > most
    {
        return Err(too_large(bytes, most));
    }
    let names = (expected.iter())
        .map(|e| e.0.as_os_str().len() as u64)
        .fold(0u64, u64::saturating_add);
    let out_most = bytes
        .saturating_add(bytes / 256)
        .saturating_add((expected.len() as u64).saturating_mul(ENTRY_OVERHEAD))
        .saturating_add(names.saturating_mul(2))
        .saturating_add(1 << 20);
    let dir = match zip.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    if out_most > extract::Limits::of(dir).bytes {
        return Err(tr!(
            "archive-no-room",
            size = crate::props::short_size(out_most)
        ));
    }
    m.add_totals(Totals {
        bytes,
        items: expected.len() as u64,
    });
    m.set_phase(Phase::Copying);
    let work = Scratch::new().map_err(io)?;
    sweep_temps(dir, zip, stale);
    let (out, tmp) = temp_beside(zip, &before).map_err(io)?;
    let tmp = Temp(tmp);
    let budget = archive::cpu_secs(before.len().saturating_add(bytes));
    if deadline.is_none() {
        watch.reset(Duration::from_secs(2 * budget));
    }
    let explicit: HashSet<&Path> = old.iter().map(|e| e.path.as_path()).collect();
    let excluded: Vec<&Path> = (walked.iter())
        .map(|(p, _, _)| p.as_path())
        .filter(|p| explicit.contains(p))
        .collect();
    // libarchive 3.8.10 reads an empty tar stream from a pipe as truncated, so a zip none of whose
    // entries stay is not read at all.
    let keeps_old = adds.is_empty()
        || count > old.len() as u64
        || (old.iter()).any(|e| !kinds.contains_key(e.path.as_path()));
    let argvs = stages(&work, keeps_old.then_some(excluded.as_slice()), adds)?;
    let total = Totals {
        bytes,
        items: expected.len() as u64,
    };
    let sink = out.try_clone().map_err(io)?;
    run(&argvs, (file, sink), (budget, out_most), total, m)?;
    out.sync_all().map_err(io)?;
    let mut got: Vec<Listed> = archive::list(&tmp.0, m)?.iter().map(listed).collect();
    got.sort();
    expected.sort();
    if got != expected {
        return Err(tr!("archive-update-mismatch"));
    }
    let now = std::fs::symlink_metadata(zip).map_err(io)?;
    let same = |a: &std::fs::Metadata, b: &std::fs::Metadata| {
        (a.dev(), a.ino(), a.len(), a.mtime(), a.mtime_nsec())
            == (b.dev(), b.ino(), b.len(), b.mtime(), b.mtime_nsec())
    };
    if !now.is_file() || !same(&before, &now) {
        return Err(tr!("archive-update-changed"));
    }
    if m.cancelled() {
        return Err(Failure::Cancelled.why());
    }
    std::fs::rename(&tmp.0, zip).map_err(io)?;
    std::mem::forget(tmp);
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    m.set_phase(Phase::Finishing);
    Ok(())
}

/// The temp file a rewrite of `zip` (as `before`) writes, locked while it lives: the zip's
/// xattrs and ACLs, then its group where this user may give it (else no group bits and no access
/// ACL), then its mode.
fn temp_beside(
    zip: &Path,
    before: &std::fs::Metadata,
) -> std::io::Result<(std::fs::File, PathBuf)> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mut mode = before.mode() & 0o777;
    let (out, tmp) =
        crate::ops::create_temp_ending(zip, Some(Permissions::from_mode(0o600)), TEMP_SUFFIX)?;
    let made = (|| {
        if !archive::flock(&out, libc::LOCK_EX) {
            return Err(std::io::Error::last_os_error());
        }
        // Copied while the temp file is still the owner's to write, before its mode.
        crate::ops::copy_xattrs(zip, &out);
        if std::os::unix::fs::fchown(&out, None, Some(before.gid())).is_err()
            && out.metadata()?.gid() != before.gid()
        {
            mode &= !0o070;
            // SAFETY: the fd is open and the name is NUL-terminated.
            unsafe { libc::fremovexattr(out.as_raw_fd(), c"system.posix_acl_access".as_ptr()) };
        }
        out.set_permissions(Permissions::from_mode(mode))?;
        Ok(())
    })();
    match made {
        Ok(()) => Ok((out, tmp)),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Removes what rewrites whose process died left in the folder `dir` of `zip`: this user's
/// `.<name>.<tag>.zipwrite` files, of `zip` or of any `.zip`, that no rewrite holds locked and nothing
/// wrote to for `stale`.
fn sweep_temps(dir: &Path, zip: &Path, stale: Duration) {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let own = zip.file_name().map(|n| n.to_string_lossy().into_owned());
    let Ok(kids) = std::fs::read_dir(dir) else {
        return;
    };
    // SAFETY: geteuid has no preconditions.
    let me = unsafe { libc::geteuid() };
    for kid in kids.flatten() {
        let file = kid.file_name();
        let Some(of) = file.to_str().and_then(temp_of) else {
            continue;
        };
        if Some(of) != own.as_deref() && !of.to_ascii_lowercase().ends_with(".zip") {
            continue;
        }
        let path = kid.path();
        if !std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
            continue;
        }
        let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(&path)
        else {
            continue;
        };
        let Ok(meta) = f.metadata() else {
            continue;
        };
        let idle = (meta.modified().ok()).and_then(|t| t.elapsed().ok());
        if !meta.is_file() || meta.uid() != me || idle.is_none_or(|t| t < stale) {
            continue;
        }
        if !archive::flock(&f, libc::LOCK_EX) {
            continue;
        }
        let same = std::fs::symlink_metadata(&path)
            .is_ok_and(|now| (now.dev(), now.ino()) == (meta.dev(), meta.ino()));
        if same && std::fs::remove_file(&path).is_ok() {
            log::info!("reprisefs: removed {} left by a rewrite", path.display());
        }
    }
}

/// The name of the file whose rewrite's temp file `name` is (`.<name>.<16 hex digits>.zipwrite`).
fn temp_of(name: &str) -> Option<&str> {
    let inner = name
        .strip_prefix('.')?
        .strip_suffix(TEMP_SUFFIX)?
        .strip_suffix('.')?;
    let (of, tag) = inner.rsplit_once('.')?;
    let tagged = tag.len() == 16 && tag.bytes().all(|b| b.is_ascii_hexdigit());
    (tagged && !of.is_empty()).then_some(of)
}

/// The zip at `zip` opened to read, never through a link, with its metadata: refused unless it
/// is a regular file this user owns and may write that starts as a zip does.
fn open_zip(zip: &Path) -> Result<(std::fs::File, std::fs::Metadata), String> {
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(zip)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP) => tr!("archive-update-not-own"),
            _ => e.to_string(),
        })?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    // SAFETY: geteuid has no preconditions.
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(tr!("archive-update-not-own"));
    }
    if meta.mode() & 0o200 == 0 {
        return Err(tr!("archive-update-read-only"));
    }
    let mut head = [0u8; 4];
    let read = file.read_exact_at(&mut head, 0);
    if read.is_err() || !zip_head(&head) {
        return Err(tr!("archive-update-not-zip"));
    }
    Ok((file, meta))
}

/// The temp file a rewrite writes, removed unless it was renamed over the zip.
struct Temp(PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("reprisefs: cannot remove {}: {e}", self.0.display());
        }
    }
}

/// The files and folders under `adds` as the rewritten zip lists them: regular files with their
/// sizes and folders, by their paths in the zip. Links are not followed, and links and other
/// kinds, which the zip keeps but its listing leaves out, are not counted. Refused past `most`
/// entries; cancelling `m` stops it.
fn walk(adds: &[Adding], m: &Meter, most: u64) -> Result<Vec<Listed>, String> {
    let mut out = Vec::new();
    let mut todo: Vec<(PathBuf, PathBuf)> = Vec::new();
    for a in adds {
        for name in &a.names {
            if !plain(name) {
                return Err(tr!("archive-unread"));
            }
            todo.push((a.dir.join(name), name.clone()));
        }
    }
    while let Some((path, inner)) = todo.pop() {
        if m.cancelled() {
            return Err(Failure::Cancelled.why());
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            return Err(tr!("archive-missing"));
        };
        if meta.is_file() {
            out.push((inner, Kind::File, meta.len()));
        } else if meta.is_dir() {
            let kids = std::fs::read_dir(&path).map_err(|e| e.to_string())?;
            for kid in kids {
                let kid = kid.map_err(|e| e.to_string())?;
                todo.push((kid.path(), inner.join(kid.file_name())));
            }
            out.push((inner, Kind::Folder, 0));
        }
        if out.len() as u64 > most {
            return Err(tr!("archive-too-many", most = MAX_ENTRIES));
        }
    }
    Ok(out)
}

/// The pattern that keeps the zip's entry at `path` out: every byte but `/` escaped, anchored at
/// both ends.
fn pattern(path: &Path) -> Vec<u8> {
    let mut p = b"^".to_vec();
    for &b in path.as_os_str().as_bytes() {
        if b != b'/' {
            p.push(b'\\');
        }
        p.push(b);
    }
    p.push(b'$');
    p
}

/// Writes `bytes` to the new file `name` in the private folder `work`.
fn write_list(work: &Scratch, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = work.path().join(name);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The chain's argument lists: the zip read as a pax stream (its own stdin) less the entries at
/// `excluded`, unless that is `None`, one stage per folder of `adds` adding its names, and a
/// last stage writing the zip.
/// The patterns go in a file bsdtar reads by line, but one holding a line break goes on the
/// command line. The names go by `-T` as NUL-ended lines, where neither `@` nor `-C` mean more.
fn stages(
    work: &Scratch,
    excluded: Option<&[&Path]>,
    adds: &[Adding],
) -> Result<Vec<Vec<OsString>>, String> {
    let os = |s: &str| OsString::from(s);
    let mut argvs = Vec::new();
    if let Some(excluded) = excluded {
        let mut first: Vec<OsString> = ["bsdtar", "-c", "-f", "-", "--format", "pax"]
            .map(os)
            .into();
        let mut lines = Vec::new();
        for path in excluded {
            let p = pattern(path);
            if p.contains(&b'\n') || p.contains(&b'\r') {
                first.extend([os("--exclude"), OsString::from_vec(p)]);
            } else {
                lines.extend_from_slice(&p);
                lines.push(b'\n');
            }
        }
        if !lines.is_empty() {
            first.extend([os("-X"), write_list(work, "exclude", &lines)?.into()]);
        }
        // The zip's stdin opened again by path, so bsdtar reads its central directory.
        first.push(os("@/proc/self/fd/0"));
        argvs.push(first);
    }
    for (i, a) in adds.iter().enumerate() {
        let mut names = Vec::new();
        for n in &a.names {
            names.extend_from_slice(n.as_os_str().as_bytes());
            names.push(0);
        }
        let list = write_list(work, &format!("names-{i}"), &names)?;
        let format = if i + 1 == adds.len() { "zip" } else { "pax" };
        let mut argv: Vec<OsString> = ["bsdtar", "-c", "-f", "-", "--format", format]
            .map(os)
            .into();
        argv.extend([
            os("-C"),
            a.dir.clone().into(),
            os("--null"),
            os("-T"),
            list.into(),
        ]);
        if !argvs.is_empty() {
            argv.push(os("@-"));
        }
        argvs.push(argv);
    }
    if adds.is_empty() {
        argvs.push(
            ["bsdtar", "-c", "-f", "-", "--format", "zip", "@-"]
                .map(os)
                .into(),
        );
    }
    if let Some(last) = argvs.last_mut() {
        // On stdout bsdtar pads the zip with NULs to a whole block; on a path it does not.
        last[3] = os("/proc/self/fd/1");
        last.insert(2, os("-v"));
    }
    Ok(argvs)
}

/// Runs the chain `argvs`, the first reading `zip`, the last writing `out` and stopped past
/// `out_most` bytes, each under `cpu` seconds, counting the entries the last writes against
/// `total`. Cancelling `m` kills them all; pausing it stops the last, and the rest wait on their
/// pipes.
fn run(
    argvs: &[Vec<OsString>],
    (zip, out): (std::fs::File, std::fs::File),
    (cpu, out_most): (u64, u64),
    total: Totals,
    m: &Meter,
) -> Result<(), String> {
    let mut kids: Vec<Child> = Vec::new();
    let mut drains = Vec::new();
    let mut input = Stdio::from(zip);
    let mut out = Some(out);
    let mut last_err = None;
    let kill = |kids: &mut Vec<Child>| {
        for k in kids.iter_mut() {
            let _ = k.kill();
            let _ = k.wait();
        }
    };
    for (i, argv) in argvs.iter().enumerate() {
        let last = i + 1 == argvs.len();
        let output = match last {
            true => Stdio::from(out.take().ok_or_else(|| extract::no_pipe().why())?),
            false => Stdio::piped(),
        };
        let fsize = last.then_some(out_most);
        let spawned = extract::spawn_io(argv, (input, output), |cmd| {
            cmd.env_remove("LANGUAGE").env("LC_ALL", "C.UTF-8");
            archive::limit(cmd, cpu, fsize);
        });
        let mut child = match spawned {
            Ok(c) => c,
            Err(f) => {
                kill(&mut kids);
                return Err(f.why());
            }
        };
        let err = child.stderr.take();
        input = match child.stdout.take() {
            Some(o) => Stdio::from(o),
            None => Stdio::null(),
        };
        match (last, err) {
            (true, err) => last_err = err,
            (false, Some(mut err)) => drains.push(std::thread::spawn(move || {
                extract::tail(&mut err, STDERR_TAIL)
            })),
            (false, None) => {}
        }
        kids.push(child);
    }
    let Some(err) = last_err else {
        kill(&mut kids);
        return Err(extract::no_pipe().why());
    };
    let pids: Vec<u32> = kids.iter().map(Child::id).collect();
    let mut said = Vec::new();
    let mut done = 0u64;
    let mut last = kids.pop().ok_or_else(|| extract::no_pipe().why())?;
    let read = extract::lines_ticking(
        &mut last,
        err,
        m,
        |line| {
            match line.strip_prefix(b"a ") {
                Some(name) => {
                    done = (done + 1).min(total.items);
                    m.set_current(OsStr::from_bytes(name));
                    let share = u128::from(total.bytes) * u128::from(done);
                    m.settle(Totals {
                        items: done,
                        bytes: (share / u128::from(total.items.max(1))) as u64,
                    });
                }
                None => {
                    said.extend_from_slice(line);
                    said.push(b'\n');
                    if said.len() > 2 * STDERR_TAIL {
                        said.drain(..said.len() - STDERR_TAIL);
                    }
                }
            }
            true
        },
        || {},
    );
    if !matches!(read, Ok(true)) {
        let _ = last.kill();
        for &pid in &pids {
            // SAFETY: each pid is our own child, not yet reaped.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
        }
    }
    let mut statuses: Vec<_> = kids.iter_mut().map(Child::wait).collect();
    statuses.push(last.wait());
    let mut stderrs: Vec<Vec<u8>> = drains
        .into_iter()
        .map(|d| d.join().unwrap_or_default())
        .collect();
    stderrs.push(said);
    match read {
        Ok(true) => {}
        Ok(false) => return Err(Failure::Cancelled.why()),
        Err(e) => return Err(e.to_string()),
    }
    for (status, stderr) in statuses.iter().zip(&stderrs) {
        match status {
            Ok(s) if s.success() => {}
            Ok(s) if archive::ran_out(s) => return Err(tr!("archive-update-too-long")),
            Ok(_) if archive::locked(stderr) => return Err(tr!("archive-locked")),
            Ok(_) => return Err(extract::said(stderr, &tr!("archive-unread"))),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zip_none_of_whose_entries_stay_is_read_by_no_stage() {
        let work = || Scratch::new().expect("a scratch folder");
        let adds = [Adding {
            dir: "/d".into(),
            names: vec!["a.txt".into()],
        }];
        let reads = |argvs: &[Vec<OsString>]| -> Vec<bool> {
            (argvs.iter())
                .map(|a| a.iter().any(|w| w.as_bytes().starts_with(b"@")))
                .collect()
        };
        let argvs = stages(&work(), None, &adds).expect("stages");
        assert_eq!(
            reads(&argvs),
            [false],
            "the one stage adds and reads no archive"
        );
        let argvs = stages(&work(), Some(&[]), &adds).expect("stages");
        assert_eq!(
            reads(&argvs),
            [true, true],
            "a kept zip is read and passed on"
        );
    }

    fn events(mut job: ArchiveJob) -> Vec<ArchiveEvent> {
        let until = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        while !job.is_finished() {
            assert!(Instant::now() < until, "the job ends");
            events.extend(job.poll());
            std::thread::sleep(Duration::from_millis(5));
        }
        events
    }

    #[test]
    fn a_passed_deadline_is_a_failure_with_its_own_box_and_a_cancel_says_nothing() {
        let zip = PathBuf::from("/d/a.zip");
        let late = ArchiveJob::rewriting("t", zip.clone(), || {}, |_, _| Err(Unwritten::TooLong));
        assert!(
            matches!(
                events(late).as_slice(),
                [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                    if text == "Cannot update a.zip: it took too long to rewrite."
            ),
            "a timeout gets a box"
        );
        let cancelled = ArchiveJob::rewriting("t", zip, || {}, |_, _| Err(Unwritten::Cancelled));
        assert!(matches!(
            events(cancelled).as_slice(),
            [ArchiveEvent::Finished]
        ));
    }

    #[test]
    fn the_deadline_counts_running_time_and_not_a_pause() {
        let m = Arc::new(Meter::default());
        m.set_paused(true);
        let watch = Watch::start(m.clone(), Duration::from_millis(300));
        std::thread::sleep(Duration::from_millis(900));
        assert!(
            !watch.passed() && !m.cancelled(),
            "a paused rewrite keeps its time"
        );
        m.set_paused(false);
        let until = Instant::now() + Duration::from_secs(10);
        while !watch.passed() {
            assert!(Instant::now() < until, "the deadline passes once it runs");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(m.cancelled(), "a passed deadline cancels the meter");
    }
}
