//! An archive browsed as a read-only folder. `bsdtar -tv` lists its entries from their headers,
//! and `bsdtar -xqO` writes the files picked to its stdout in archive order, which Reprise splits
//! by their listed sizes into a fresh private folder: bsdtar itself writes nothing to disk. Both
//! run in the C.UTF-8 locale (names and dates read back exactly) under CPU and memory limits, and
//! die with the thread that started them. docs/security.md "Extracting archives" has the rules.

use crate::extract::{self, ArchiveEvent, ArchiveJob, Failure, MAX_ENTRIES, STDERR_TAIL};
use crate::progress::Meter;
use reprisekit::tr;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Folders under the scratch folder older than this are removed when another file is opened.
const SCRATCH_KEEP_SECS: u64 = 24 * 60 * 60;

/// CPU seconds any bsdtar child may take, before [`CPU_BYTES_PER_SEC`] adds to it.
const BASE_CPU_SECS: u64 = 60;

/// Archive bytes each further CPU second buys. A compressed tar decompresses whole to reach
/// its headers; xz, the slowest, writes 50 MB/s or more, so at a 10:1 ratio an honest archive
/// needs about 1 s per 5 MB, and this gives 2.5 times that. A small bomb still gets about a minute.
const CPU_BYTES_PER_SEC: u64 = 2 << 20;

/// The most listings one window keeps.
const KEPT_LISTINGS: usize = 4;

/// The most bytes one listing's names may add up to, which a window keeps [`KEPT_LISTINGS`] of.
const MAX_NAME_BYTES: usize = 128 << 20;

/// Address space a bsdtar child may map, in bytes.
const CHILD_MEMORY: u64 = 2 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Folder,
}

/// One file or folder an archive holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its path inside the archive: plain names only, no `.`, `..` or root.
    pub path: PathBuf,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<i64>,
}

/// An entry directly inside one of an archive's folders. A folder the archive holds only by the
/// paths below it is listed too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Child {
    pub name: OsString,
    pub kind: Kind,
    pub size: u64,
    pub modified: Option<i64>,
}

/// Whether a file of `mime` opens as a compressed folder: an archive type bsdtar reads, with
/// bsdtar installed and no default program the user chose for it.
pub fn opens_as_folder(mime: &str, apps: &[crate::apps::AppEntry]) -> bool {
    extract::is_archive(mime)
        && crate::props::chosen_default(mime, apps).is_none()
        && crate::search::which("bsdtar").is_some()
}

/// Writes the first file named `entry` to stdout.
pub fn open_argv(archive: &Path, entry: &Path) -> Vec<OsString> {
    members_argv(archive, [entry])
}

/// Writes the first file named each of `entries` to stdout, in archive order. bsdtar reads a
/// name as a pattern, so every byte but `/` is escaped, the closing `$` stops it matching what
/// lies below the name, and the leading `./` keeps a name like `-C` from reading as an option.
fn members_argv<'a>(archive: &Path, entries: impl IntoIterator<Item = &'a Path>) -> Vec<OsString> {
    let pattern = |entry: &Path| {
        let mut pattern = b"./".to_vec();
        for &b in entry.as_os_str().as_bytes() {
            if b != b'/' {
                pattern.push(b'\\');
            }
            pattern.push(b);
        }
        pattern.push(b'$');
        OsString::from_vec(pattern)
    };
    ["bsdtar", "-xqO", "-f"]
        .map(OsString::from)
        .into_iter()
        .chain([archive.into()])
        .chain(entries.into_iter().map(pattern))
        .collect()
}

/// The most files one bsdtar run copies out, each run decompressing a compressed tar from its
/// start: more names cost each entry more pattern matches.
const BATCH_FILES: usize = 128;

/// The most bytes of escaped names one bsdtar run's arguments hold.
const BATCH_NAME_BYTES: usize = 256 << 10;

/// The CPU seconds a child reading `bytes` of archive (and member) may take.
pub(crate) fn cpu_secs(bytes: u64) -> u64 {
    BASE_CPU_SECS.saturating_add(bytes / CPU_BYTES_PER_SEC)
}

pub(crate) fn archive_bytes(archive: &Path) -> u64 {
    std::fs::metadata(archive).map_or(0, |m| m.len())
}

/// Whether bsdtar was stopped for running over its CPU limit (SIGXCPU, then SIGKILL at the hard
/// limit), which a box words as taking too long.
pub(crate) fn ran_out(status: &std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    matches!(status.signal(), Some(libc::SIGXCPU | libc::SIGKILL))
}

/// Puts the child `cmd` starts under `cpu_secs` of CPU and [`CHILD_MEMORY`], with no core file,
/// and when set no file it writes past `file_bytes` (SIGXFSZ).
pub(crate) fn limit(cmd: &mut std::process::Command, cpu_secs: u64, file_bytes: Option<u64>) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the hook only makes async-signal-safe calls (setrlimit) in the child.
    unsafe {
        cmd.pre_exec(move || {
            let limit = |res, n: u64| {
                let r = libc::rlimit {
                    rlim_cur: n as libc::rlim_t,
                    rlim_max: n as libc::rlim_t,
                };
                match libc::setrlimit(res, &r) {
                    0 => Ok(()),
                    _ => Err(std::io::Error::last_os_error()),
                }
            };
            limit(libc::RLIMIT_CPU, cpu_secs)?;
            limit(libc::RLIMIT_AS, CHILD_MEMORY)?;
            limit(libc::RLIMIT_CORE, 0)?;
            if let Some(n) = file_bytes {
                limit(libc::RLIMIT_FSIZE, n)?;
            }
            Ok(())
        });
    }
}

/// Starts `argv` with its output piped, in the C.UTF-8 locale and under [`limit`]'s `cpu_secs`.
fn spawn(argv: &[OsString], cpu_secs: u64) -> Result<std::process::Child, Failure> {
    extract::spawn_with(argv, true, |cmd| {
        cmd.env_remove("LANGUAGE").env("LC_ALL", "C.UTF-8");
        limit(cmd, cpu_secs, None);
    })
}

/// Every file and folder `archive` holds, read from its headers. An archive of more than
/// [`MAX_ENTRIES`] entries, or whose names add up to more than [`MAX_NAME_BYTES`], is refused as
/// soon as the listing passes them; cancelling `m` stops it.
pub fn list(archive: &Path, m: &Meter) -> Result<Vec<Entry>, String> {
    list_within(archive, m, MAX_NAME_BYTES)
}

fn list_within(archive: &Path, m: &Meter, most_names: usize) -> Result<Vec<Entry>, String> {
    if zip_encrypted(archive) {
        return Err(tr!("archive-locked"));
    }
    let why = |f: Failure| f.why();
    let budget = cpu_secs(archive_bytes(archive));
    let mut child = spawn(&extract::list_argv(archive), budget).map_err(why)?;
    let out = child.stdout.take().ok_or_else(|| why(extract::no_pipe()))?;
    let mut err = child.stderr.take().ok_or_else(|| why(extract::no_pipe()))?;
    let drain = std::thread::spawn(move || extract::tail(&mut err, STDERR_TAIL));
    let now = now();
    let mut entries = Vec::new();
    let mut seen = 0u64;
    let mut names = 0usize;
    let read = extract::lines(&mut child, out, m, |line| {
        seen += 1;
        if seen > MAX_ENTRIES {
            return false;
        }
        let Some(e) = listed_entry(line, now) else {
            return true;
        };
        names += e.path.as_os_str().len();
        entries.push(e);
        names <= most_names
    });
    let status = match read {
        Ok(true) => child.wait().map_err(|e| e.to_string()),
        _ => Err(extract::finish(child, &read)
            .err()
            .map_or_else(String::new, why)),
    };
    let stderr = drain.join().unwrap_or_default();
    if seen > MAX_ENTRIES {
        return Err(tr!("archive-too-many", most = MAX_ENTRIES));
    }
    if names > most_names {
        let most = crate::props::short_size(most_names as u64);
        return Err(tr!("archive-names-too-long", most = most));
    }
    match status? {
        s if s.success() => Ok(entries),
        s if ran_out(&s) => Err(tr!("archive-too-long")),
        _ if locked(&stderr) => Err(tr!("archive-locked")),
        _ => Err(extract::said(&stderr, &tr!("archive-unread"))),
    }
}

/// Whether bsdtar's `stderr` says an entry or the archive's header is encrypted: it wanted a
/// passphrase it had no way to read, or the format's decryption is not supported.
pub(crate) fn locked(stderr: &[u8]) -> bool {
    const SAID: [&str; 5] = [
        "Couldn't read passphrase",
        "Incorrect passphrase",
        "Passphrase required",
        "The archive header is encrypted",
        "The file content is encrypted",
    ];
    // Only as bsdtar's `<name>: <message>`, so an entry named for one of these is no error.
    let text = String::from_utf8_lossy(stderr);
    SAID.iter().any(|s| text.contains(&format!(": {s}")))
}

/// The most central directory bytes [`zip_encrypted`] reads: about a million entries.
const MAX_ZIP_DIRECTORY: u64 = 64 << 20;

/// Whether `file` is a zip with an encrypted entry, by bit 0 of an entry's flags in its central
/// directory. A file that does not start as a zip, or whose end records do not read, is not.
pub(crate) fn zip_encrypted(file: &Path) -> bool {
    zip_flags_encrypted(file).unwrap_or(false)
}

fn zip_flags_encrypted(file: &Path) -> Option<bool> {
    use std::io::{Seek, SeekFrom};
    const EOCD: usize = 22;
    let le = |b: &[u8], at: usize, n: usize| {
        let bytes = b.get(at..at.checked_add(n)?)?;
        Some(bytes.iter().rev().fold(0u64, |v, &x| v << 8 | u64::from(x)))
    };
    use std::os::unix::fs::OpenOptionsExt;
    // Nonblocking, so a FIFO under an archive's name cannot hold the thread at the open.
    let mut f = (std::fs::OpenOptions::new().read(true))
        .custom_flags(libc::O_NONBLOCK)
        .open(file)
        .ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let len = meta.len();
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic).ok()?;
    if magic != *b"PK\x03\x04" {
        return None;
    }
    let tail_len = len.min((EOCD + 0xffff) as u64);
    let mut tail = vec![0; tail_len as usize];
    f.seek(SeekFrom::Start(len - tail_len)).ok()?;
    f.read_exact(&mut tail).ok()?;
    let end = (0..=tail.len().checked_sub(EOCD)?)
        .rev()
        .find(|&i| tail[i..i + 4] == *b"PK\x05\x06")?;
    let (mut size, mut offset) = (le(&tail, end + 12, 4)?, le(&tail, end + 16, 4)?);
    if size == 0xffff_ffff || offset == 0xffff_ffff {
        let locator = end.checked_sub(20)?;
        if tail.get(locator..locator + 4)? != b"PK\x06\x07" {
            return None;
        }
        let mut record = [0u8; 56];
        f.seek(SeekFrom::Start(le(&tail, locator + 8, 8)?)).ok()?;
        f.read_exact(&mut record).ok()?;
        if record[..4] != *b"PK\x06\x06" {
            return None;
        }
        (size, offset) = (le(&record, 40, 8)?, le(&record, 48, 8)?);
    }
    if size > MAX_ZIP_DIRECTORY || offset.checked_add(size)? > len {
        return None;
    }
    let mut dir = vec![0; size as usize];
    f.seek(SeekFrom::Start(offset)).ok()?;
    f.read_exact(&mut dir).ok()?;
    let mut at = 0usize;
    while dir.get(at..at + 4) == Some(b"PK\x01\x02") {
        if le(&dir, at + 8, 2)? & 1 != 0 {
            return Some(true);
        }
        let names = le(&dir, at + 28, 2)? + le(&dir, at + 30, 2)? + le(&dir, at + 32, 2)?;
        at = at.checked_add(46 + names as usize)?;
    }
    Some(false)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The file or folder a `bsdtar -tv --numeric-owner` line lists: mode, links, uid, gid, size, a
/// three-word date, then the name. Links, devices and names that leave the folder give `None`.
fn listed_entry(line: &[u8], now: i64) -> Option<Entry> {
    let mut rest = line;
    let mut words = [&line[..0]; 8];
    for w in &mut words {
        rest = &rest[rest.iter().position(|&b| b != b' ')?..];
        (*w, rest) = rest.split_at(rest.iter().position(|&b| b == b' ')?);
    }
    let kind = match words[0].first()? {
        b'-' => Kind::File,
        b'd' => Kind::Folder,
        _ => return None,
    };
    let size = std::str::from_utf8(words[4]).ok()?.parse().ok()?;
    let [month, day, third] = [words[5], words[6], words[7]].map(|w| std::str::from_utf8(w).ok());
    Some(Entry {
        path: clean(&unescape(rest.strip_prefix(b" ")?))?,
        kind,
        size: if kind == Kind::File { size } else { 0 },
        modified: listed_time(month?, day?, third?, now),
    })
}

/// A listing's local date, `Mon D` then `HH:MM` within half a year of `now` or the year further
/// off, as seconds since the epoch.
fn listed_time(month: &str, day: &str, third: &str, now: i64) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MONTHS.iter().position(|m| *m == month)? as i32;
    let day: i32 = day.parse().ok()?;
    let at = |year: i32, hour: i32, min: i32| {
        // SAFETY: `tm` is a zeroed struct mktime reads and normalises in place.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        (tm.tm_year, tm.tm_mon, tm.tm_mday) = (year - 1900, mon, day);
        (tm.tm_hour, tm.tm_min, tm.tm_isdst) = (hour, min, -1);
        // SAFETY: as above.
        let t = unsafe { libc::mktime(&mut tm) };
        (t != -1).then_some(t as i64)
    };
    match third.split_once(':') {
        None => at(third.parse().ok()?, 0, 0),
        Some((h, m)) => {
            let (h, m) = (h.parse().ok()?, m.parse().ok()?);
            let year = crate::locale::localtime(now).tm_year + 1900;
            (year - 1..=year + 1)
                .filter_map(|y| at(y, h, m))
                .min_by_key(|t| (t - now).abs())
        }
    }
}

/// A listed name with bsdtar's escapes read back to bytes: `\\`, the C control letters and
/// `\ooo` octal for every other byte it does not print.
pub(crate) fn unescape(name: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(name.len());
    let mut i = 0;
    while i < name.len() {
        if name[i] != b'\\' {
            out.push(name[i]);
            i += 1;
            continue;
        }
        let octal = name
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
        if let Some(d) = octal {
            out.push(d.iter().fold(0u8, |n, b| n.wrapping_mul(8) + (b - b'0')));
            i += 4;
            continue;
        }
        let byte = match name.get(i + 1) {
            Some(b'\\') => b'\\',
            Some(b'a') => 0x07,
            Some(b'b') => 0x08,
            Some(b'f') => 0x0c,
            Some(b'n') => b'\n',
            Some(b'r') => b'\r',
            Some(b't') => b'\t',
            Some(b'v') => 0x0b,
            _ => {
                out.push(b'\\');
                i += 1;
                continue;
            }
        };
        out.push(byte);
        i += 2;
    }
    out
}

/// `name` without its leading `./` and trailing `/`, when every step of it is a plain name;
/// `None` for the top folder itself and for a path from the root or with a `..` step.
fn clean(name: &[u8]) -> Option<PathBuf> {
    let mut path = PathBuf::new();
    for c in Path::new(OsStr::from_bytes(name)).components() {
        match c {
            Component::Normal(n) => path.push(n),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    (!path.as_os_str().is_empty()).then_some(path)
}

/// An archive as it was listed: its device, inode, size and modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp(u64, u64, u64, i64, i64);

impl Stamp {
    fn of(file: &Path) -> Option<Stamp> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(file).ok()?;
        Some(Stamp(m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec()))
    }
}

/// One window's archive listings: each archive's entries, kept while it is unchanged, so moving
/// around inside it lists it once. Clones share them.
#[derive(Clone, Default)]
pub struct Listings(Arc<Mutex<ListingsInner>>);

/// One archive's listing, or why it failed.
type Listed = Result<Arc<Vec<Entry>>, String>;

#[derive(Default)]
struct ListingsInner {
    /// The last [`KEPT_LISTINGS`] archives listed, oldest first; a failure is kept as well, so
    /// a visit back does not pay for it again.
    listed: VecDeque<(PathBuf, Stamp, Listed)>,
    running: Option<Arc<Meter>>,
}

impl Listings {
    /// `archive`'s entries, listed again only when it changed. A listing started here cancels
    /// the one running, which [`Listings::cancel`] also stops.
    pub fn entries(&self, archive: &Path) -> Listed {
        self.listing(archive)
            .unwrap_or_else(|| Err(Failure::Cancelled.why()))
    }

    /// [`Listings::entries`], or None when another listing or [`Listings::cancel`] stopped it.
    pub fn listing(&self, archive: &Path) -> Option<Listed> {
        let stamp = match Stamp::of(archive) {
            Some(s) => s,
            None => return Some(Err(tr!("archive-missing"))),
        };
        let meter = Arc::new(Meter::default());
        {
            let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, _, listed)) =
                (inner.listed.iter()).find(|(p, s, _)| p == archive && *s == stamp)
            {
                return Some(listed.clone());
            }
            if let Some(old) = inner.running.replace(meter.clone()) {
                old.cancel();
            }
        }
        let listed = list(archive, &meter).map(Arc::new);
        let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner
            .running
            .as_ref()
            .is_some_and(|r| Arc::ptr_eq(r, &meter))
        {
            inner.running = None;
        }
        if meter.cancelled() {
            return None;
        }
        inner.listed.retain(|(p, ..)| p != archive);
        if inner.listed.len() >= KEPT_LISTINGS {
            inner.listed.pop_front();
        }
        (inner.listed).push_back((archive.to_path_buf(), stamp, listed.clone()));
        Some(listed)
    }

    /// The entries `archive` was last listed with, without asking whether it changed since: what
    /// a compressed folder on view shows.
    pub fn kept(&self, archive: &Path) -> Option<Arc<Vec<Entry>>> {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let listed = inner.listed.iter().rev().find(|(p, ..)| p == archive);
        listed.and_then(|(_, _, l)| l.as_ref().ok().cloned())
    }

    /// Stops the listing running, if any: the window has left the archive.
    pub fn cancel(&self) {
        let inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = &inner.running {
            m.cancel();
        }
    }
}

/// Types that run when opened besides [`crate::program::opens_as_a_program`]'s: launchers,
/// scripts and binaries, which a desktop opens by running them or handing them to an interpreter.
const RUN_TYPES: [&str; 11] = [
    "application/x-desktop",
    "application/x-shellscript",
    "application/x-executable",
    "application/x-pie-executable",
    "application/x-sharedlib",
    "application/x-perl",
    "application/x-ruby",
    "text/x-python",
    "text/x-python3",
    "application/x-java-jnlp-file",
    "application/x-msdos-program",
];

/// Name suffixes that run whatever type the MIME database gives them, as an interpreter or a
/// Windows layer registered for them would.
const RUN_SUFFIXES: [&str; 17] = [
    "desktop", "sh", "py", "rb", "pl", "jnlp", "exe", "com", "scr", "pif", "bat", "cmd", "vbs",
    "msi", "lnk", "jar", "appimage",
];

/// Whether `copy`, a file copied out of an archive with the type `mime` by its name, would run
/// as a program where it lies: a runnable type or suffix, or an ELF binary or `#!` script by its
/// head.
pub fn runs_as_a_program(copy: &Path, mime: Option<&str>) -> bool {
    let mut head = [0u8; 4];
    let read = std::fs::File::open(copy).and_then(|mut f| f.read(&mut head));
    let head = &head[..read.unwrap_or(0)];
    let suffix = copy
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    crate::program::opens_as_a_program(copy, mime)
        || mime.is_some_and(|m| RUN_TYPES.contains(&m.to_ascii_lowercase().as_str()))
        || suffix.is_some_and(|s| RUN_SUFFIXES.contains(&s.as_str()))
        || head.starts_with(b"\x7fELF")
        || head.starts_with(b"#!")
}

/// The file and the path inside it an address names when `path` reaches below a file: `path`'s
/// nearest ancestor (or itself) that is a file, and the plain names under it. `None` when the
/// nearest that exists is a folder, or the rest holds a `..`.
pub fn address(path: &Path) -> Option<(PathBuf, PathBuf)> {
    // Rebuilt from its steps, so a trailing `/` cannot make the file read as missing.
    let path: PathBuf = path.components().collect();
    let mut file = path.as_path();
    loop {
        match std::fs::metadata(file) {
            Ok(m) if m.is_file() => break,
            Ok(_) => return None,
            Err(_) => file = file.parent()?,
        }
    }
    let mut inner = PathBuf::new();
    for c in path.strip_prefix(file).ok()?.components() {
        match c {
            Component::Normal(n) => inner.push(n),
            Component::CurDir => {}
            Component::RootDir | Component::ParentDir | Component::Prefix(_) => return None,
        }
    }
    Some((file.to_path_buf(), inner))
}

/// Whether `inner` is a folder of the archive `entries` list: its top (empty), a folder entry, or
/// a folder the paths below it imply.
pub fn holds_folder(entries: &[Entry], inner: &Path) -> bool {
    inner.as_os_str().is_empty()
        || entries.iter().any(|e| match e.path.strip_prefix(inner) {
            Ok(rest) => e.kind == Kind::Folder || !rest.as_os_str().is_empty(),
            Err(_) => false,
        })
}

/// What the folder `inner` of the archive holds directly, by name. An entry listed twice keeps
/// its first listing, and a folder another entry implies is a [`Kind::Folder`] of size 0.
pub fn children(entries: &[Entry], inner: &Path) -> Vec<Child> {
    let mut kids: BTreeMap<OsString, Child> = BTreeMap::new();
    for e in entries {
        let Ok(rest) = e.path.strip_prefix(inner) else {
            continue;
        };
        let mut steps = rest.components();
        let Some(Component::Normal(name)) = steps.next() else {
            continue;
        };
        let below = steps.next().is_some();
        let child = match below {
            true => Child {
                name: name.to_owned(),
                kind: Kind::Folder,
                size: 0,
                modified: None,
            },
            false => Child {
                name: name.to_owned(),
                kind: e.kind,
                size: e.size,
                modified: e.modified,
            },
        };
        match kids.get_mut(name) {
            None => {
                kids.insert(name.to_owned(), child);
            }
            // A folder's own entry gives it its date when another entry named it first.
            Some(k) if !below && k.kind == Kind::Folder && e.kind == Kind::Folder => {
                k.modified = e.modified;
            }
            Some(_) => {}
        }
    }
    kids.into_values().collect()
}

/// A fresh folder only this user can enter, under `$XDG_CACHE_HOME/reprise/archives`, to copy a
/// file to. Folders an earlier open left there over a day ago are removed first.
pub fn scratch() -> std::io::Result<PathBuf> {
    let base = scratch_base().ok_or_else(|| std::io::Error::other(tr!("archive-no-cache")))?;
    scratch_in(&base, SCRATCH_KEEP_SECS)
}

fn scratch_base() -> Option<PathBuf> {
    Some(reprisexdg::cache_home()?.join("reprise/archives"))
}

/// Whether `path` is a copy [`copy_out`] made in a [`scratch`] folder, which Recent files skips.
/// The path is made absolute with `..` dropped by name first.
pub fn is_copy(path: &Path) -> bool {
    let Ok(joined) = std::path::absolute(path) else {
        return false;
    };
    let mut clean = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                clean.pop();
            }
            c => clean.push(c),
        }
    }
    scratch_base().is_some_and(|base| clean.starts_with(base))
}

fn scratch_in(base: &Path, keep_secs: u64) -> std::io::Result<PathBuf> {
    sweep_scratch(base, keep_secs)?;
    make_temp(base, "XXXXXX")
}

/// A fresh folder named after `template` (ending in `XXXXXX`) in the private folder `base`.
fn make_temp(base: &Path, template: &str) -> std::io::Result<PathBuf> {
    let mut template = base.join(template).into_os_string().into_vec();
    template.push(0);
    // SAFETY: `template` is a writable NUL-terminated buffer that mkdtemp fills in place.
    let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
    if made.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    template.pop();
    Ok(PathBuf::from(OsString::from_vec(template)))
}

/// Makes the private scratch folder `base`.
fn make_base(base: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(base)
}

/// Makes the private scratch folder `base`, removing the folders in it older than `keep_secs`
/// and the ones a [`Scratch`] let go of, or whose holders all died, unless something holds them.
fn sweep_scratch(base: &Path, keep_secs: u64) -> std::io::Result<()> {
    make_base(base)?;
    let now = std::time::SystemTime::now();
    for e in std::fs::read_dir(base)?.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = e.file_name();
        let gone = name.as_bytes().ends_with(GONE_SUFFIX.as_bytes());
        let meta = e.metadata().ok();
        let age = (meta.as_ref().and_then(|m| m.modified().ok()))
            .and_then(|t| now.duration_since(t).ok())
            .map_or(0, |age| age.as_secs());
        // A held folder is locked from a moment after it is made until its last holder ends,
        // unless it was handed over ([`Scratch::hand_over`]).
        let held = name.as_bytes().starts_with(HELD_PREFIX.as_bytes())
            && !meta.as_ref().is_some_and(handed_over);
        let sweep = gone || (held && age > ORPHAN_SECS) || age > keep_secs;
        if !sweep {
            continue;
        }
        // The lock stays until the folder is gone, so no hold is taken on it meanwhile.
        if let Some(d) = open_dir(&e.path()).filter(|d| flock(d, libc::LOCK_EX)) {
            let _ = std::fs::remove_dir_all(e.path());
            drop(d);
        }
    }
    Ok(())
}

/// Names of the folders a [`Scratch`] makes; other scratch folders hold copies opened in programs.
const HELD_PREFIX: &str = "copy-";

/// How old a [`Scratch`] folder nothing holds must be before a sweep takes it: one made this
/// moment is not locked yet.
const ORPHAN_SECS: u64 = 60;

/// Whether a scratch folder's metadata `m` marks it handed over ([`Scratch::hand_over`]): its
/// sticky bit.
fn handed_over(m: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o1000 != 0
}

/// What a [`Scratch`]'s folder is renamed to end with while it is removed.
const GONE_SUFFIX: &str = ".gone";

/// A private folder under the scratch folder that copies out of an archive go into, held by each
/// consumer of the copies: the job writing them, the clipboard or drag offering them, each put
/// reading them, in this process or another ([`Scratch::hold`]). The folder is removed when the
/// last holder anywhere lets go; the sweep only catches folders whose holders all died.
#[derive(Clone)]
pub struct Scratch(Arc<Held>);

/// One process's hold: a shared `flock` on its open folder. The last holder to let go takes the
/// exclusive lock, renames the folder to end in [`GONE_SUFFIX`] and removes it on a thread.
struct Held {
    path: PathBuf,
    dir: Option<std::fs::File>,
}

impl Scratch {
    /// A fresh, empty, held folder under `$XDG_CACHE_HOME/reprise/archives`. Folders left there by
    /// crashes are swept on a thread.
    pub fn new() -> std::io::Result<Scratch> {
        let base = scratch_base().ok_or_else(|| std::io::Error::other(tr!("archive-no-cache")))?;
        Self::new_in(&base, SCRATCH_KEEP_SECS)
    }

    fn new_in(base: &Path, keep_secs: u64) -> std::io::Result<Scratch> {
        make_base(base)?;
        let swept = base.to_path_buf();
        std::thread::spawn(move || sweep_scratch(&swept, keep_secs));
        let path = make_temp(base, &format!("{HELD_PREFIX}XXXXXX"))?;
        let dir = open_dir(&path).ok_or_else(std::io::Error::last_os_error)?;
        if !flock(&dir, libc::LOCK_SH) {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Scratch(Arc::new(Held {
            path,
            dir: Some(dir),
        })))
    }

    /// Holds the [`Scratch`] folder `path` lies in, for as long as the value lives. None when
    /// `path` is in none, or it is gone or going.
    pub fn hold(path: &Path) -> Option<Scratch> {
        Self::hold_in(&scratch_base()?, path)
    }

    fn hold_in(base: &Path, path: &Path) -> Option<Scratch> {
        let Some(Component::Normal(name)) = path.strip_prefix(base).ok()?.components().next()
        else {
            return None;
        };
        let held = name.as_bytes();
        if !held.starts_with(HELD_PREFIX.as_bytes()) || held.ends_with(GONE_SUFFIX.as_bytes()) {
            return None;
        }
        let path = base.join(name);
        let dir = open_dir(&path)?;
        flock(&dir, libc::LOCK_SH).then(|| {
            Scratch(Arc::new(Held {
                path,
                dir: Some(dir),
            }))
        })
    }

    /// Lets go of the folder without removing it, for a program that may read it at any later
    /// time (the target of a drop): no holder removes it from then on, and it goes with the
    /// sweep [`SCRATCH_KEEP_SECS`] after the hand-over.
    pub fn hand_over(self) {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::Permissions::from_mode(0o1700);
        // The sweep ages a folder by its mtime, which the chmod leaves alone.
        let touched =
            (self.0.dir.as_ref()).map_or(Ok(()), |d| d.set_modified(std::time::SystemTime::now()));
        if let Err(e) = touched.and_then(|()| std::fs::set_permissions(self.path(), mode)) {
            log::debug!("archive: cannot hand over {}: {e}", self.path().display());
        }
    }

    pub fn path(&self) -> &Path {
        &self.0.path
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let Some(dir) = self.dir.take() else {
            return;
        };
        if !flock(&dir, libc::LOCK_EX) || dir.metadata().is_ok_and(|m| handed_over(&m)) {
            return;
        }
        let mut gone = self.path.clone().into_os_string();
        gone.push(GONE_SUFFIX);
        let gone = PathBuf::from(gone);
        if let Err(e) = std::fs::rename(&self.path, &gone) {
            log::debug!("archive: cannot remove {}: {e}", self.path.display());
            return;
        }
        // The lock stays until the folder is gone, so no sweep removes it twice.
        std::thread::spawn(move || {
            let _ = std::fs::remove_dir_all(&gone);
            drop(dir);
        });
    }
}

impl PartialEq for Scratch {
    fn eq(&self, other: &Scratch) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for Scratch {}

impl std::fmt::Debug for Scratch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Scratch").field(&self.0.path).finish()
    }
}

/// The folder `path` opened to lock, never through a link.
fn open_dir(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .ok()
}

/// Takes `how` (`LOCK_SH` or `LOCK_EX`) on `file` without waiting: whether it was granted.
fn flock(file: &std::fs::File, how: libc::c_int) -> bool {
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a file descriptor `file` keeps open.
    unsafe { libc::flock(file.as_raw_fd(), how | libc::LOCK_NB) == 0 }
}

/// [`copy_tree_out`] into the empty folder of `scratch` on a job, which holds it while it runs,
/// the copies going on to `dest`: each copy as an [`ArchiveEvent::Landed`], or one
/// [`ArchiveEvent::Failed`] saying why. A drag's copy (`drag_most`) is refused past that many
/// bytes. A cancelled job lands nothing.
pub fn copy_job(
    archive: PathBuf,
    entries: Arc<Vec<Entry>>,
    picked: Vec<PathBuf>,
    (scratch, dest): (Scratch, Option<PathBuf>),
    drag_most: Option<u64>,
    wake: impl Fn() + Send + 'static,
) -> ArchiveJob {
    let meter = Arc::new(Meter::default());
    ArchiveJob::on_thread("reprisefs-copy-out", meter, wake, move |m, send| {
        let into = scratch.path();
        let to = (into, dest.as_deref());
        let copied = copy_tree_within(&archive, &entries, &picked, to, drag_most, m);
        // Let go before the end is told, so the host's holds decide when the folder goes.
        drop(scratch);
        match copied {
            Ok(_) if m.cancelled() => {}
            Ok(copies) => {
                for (index, into) in copies.into_iter().enumerate() {
                    if !send(ArchiveEvent::Landed { index, into }) {
                        return;
                    }
                }
            }
            Err(_) if m.cancelled() => {}
            Err(text) => {
                send(ArchiveEvent::Failed { index: 0, text });
            }
        }
        send(ArchiveEvent::Finished);
    })
}

/// Copies the file `entry` of `archive` into the empty folder `into` under its own name, read-only:
/// what the program opening it sees. Refused when it would fill more than
/// [`extract::FREE_SHARE_PERCENT`] of the free space, or when bsdtar writes other than the
/// size the listing gave.
pub fn copy_out(archive: &Path, entry: &Entry, into: &Path) -> Result<PathBuf, String> {
    copy_out_within(archive, entry, into, extract::Limits::of(into).bytes)
}

fn copy_out_within(
    archive: &Path,
    entry: &Entry,
    into: &Path,
    room: u64,
) -> Result<PathBuf, String> {
    let name = entry
        .path
        .file_name()
        .ok_or_else(|| tr!("archive-unread"))?;
    if entry.size > room {
        return Err(tr!(
            "archive-no-room",
            size = crate::props::short_size(entry.size)
        ));
    }
    let to = into.join(name);
    write_entries(
        archive,
        &[(entry, to.clone())],
        0o400,
        &Meter::default(),
        |_| {},
    )?;
    Ok(to)
}

/// Writes the files `files` of `archive`, in archive order, to their new paths, never through a
/// link, each given `mode` once its listed size is in and then passed to `landed` by index. None
/// of them is left when bsdtar fails or writes other than their listed sizes in all, or `m` is
/// cancelled; bsdtar is stopped while `m` is paused.
fn write_entries(
    archive: &Path,
    files: &[(&Entry, PathBuf)],
    mode: u32,
    m: &Meter,
    mut landed: impl FnMut(usize),
) -> Result<(), String> {
    let mut to = Landing {
        files,
        mode,
        made: 0,
        file: None,
        left: 0,
    };
    let failed = |made: usize, why: String| {
        for (_, path) in &files[..made] {
            let _ = std::fs::remove_file(path);
        }
        why
    };
    // The first file is made before bsdtar runs, so a paused copy shows it begun.
    to.step(&mut landed).map_err(|why| failed(to.made, why))?;
    let made = to.made;
    let tool = |f: Failure| failed(made, f.why());
    let bytes = (files.iter()).fold(0u64, |n, (e, _)| n.saturating_add(e.size));
    let budget = cpu_secs(archive_bytes(archive).saturating_add(bytes));
    let argv = members_argv(archive, files.iter().map(|(e, _)| e.path.as_path()));
    let mut child = spawn(&argv, budget).map_err(tool)?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| tool(extract::no_pipe()))?;
    let mut err = child
        .stderr
        .take()
        .ok_or_else(|| tool(extract::no_pipe()))?;
    let drain = std::thread::spawn(move || extract::tail(&mut err, STDERR_TAIL));
    let mut wrong = None;
    let read = extract::chunks(
        &mut child,
        out,
        m,
        |chunk| {
            let wrote = to.write(chunk, &mut landed);
            wrote.map_err(|e| wrong = Some(e)).is_ok()
        },
        || {},
    );
    let copied = match (read, wrong) {
        (_, Some(why)) => Err(why),
        (Err(e), None) => Err(e.to_string()),
        (Ok(false), None) => Err(Failure::Cancelled.why()),
        (Ok(true), None) => Ok(()),
    };
    if copied.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    let stderr = drain.join().unwrap_or_default();
    let made = to.made;
    // A child killed here for writing too much died of SIGKILL too, not of its CPU limit.
    if let Ok(s) = &status
        && copied.is_ok()
        && ran_out(s)
    {
        return Err(failed(made, tr!("archive-too-long")));
    }
    if status.as_ref().is_ok_and(|s| !s.success()) && locked(&stderr) {
        return Err(failed(made, tr!("archive-locked")));
    }
    copied.map_err(|why| failed(made, why))?;
    let finished = to.finish(&mut landed);
    let made = to.made;
    finished.map_err(|why| failed(made, why))?;
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(_) => Err(failed(made, extract::said(&stderr, &tr!("archive-unread")))),
        Err(e) => Err(failed(made, e.to_string())),
    }
}

/// The files [`write_entries`] fills from one stream, each taking its listed size in turn.
struct Landing<'a> {
    files: &'a [(&'a Entry, PathBuf)],
    mode: u32,
    /// How many of `files` were made.
    made: usize,
    /// The last one made, while bytes are still `left` for it.
    file: Option<std::fs::File>,
    left: u64,
}

impl Landing<'_> {
    /// Gives the file made last its mode once it is full, and makes the next ones until one still
    /// takes bytes or none is left.
    fn step(&mut self, landed: &mut impl FnMut(usize)) -> Result<(), String> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        loop {
            if self.file.is_some() && self.left > 0 {
                return Ok(());
            }
            if let Some(file) = self.file.take() {
                (file.set_permissions(std::fs::Permissions::from_mode(self.mode)))
                    .map_err(|e| e.to_string())?;
                landed(self.made - 1);
            }
            let Some((entry, to)) = self.files.get(self.made) else {
                return Ok(());
            };
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(to)
                .map_err(|e| e.to_string())?;
            self.made += 1;
            self.file = Some(file);
            self.left = entry.size;
        }
    }

    fn write(&mut self, mut chunk: &[u8], landed: &mut impl FnMut(usize)) -> Result<(), String> {
        while !chunk.is_empty() {
            self.step(landed)?;
            let Some(file) = self.file.as_mut() else {
                return Err(tr!("archive-more-than-listed"));
            };
            let n = chunk
                .len()
                .min(usize::try_from(self.left).unwrap_or(usize::MAX));
            file.write_all(&chunk[..n]).map_err(|e| e.to_string())?;
            self.left -= n as u64;
            chunk = &chunk[n..];
        }
        Ok(())
    }

    /// Makes and lands the empty files at the end; a file still short of its size fails.
    fn finish(&mut self, landed: &mut impl FnMut(usize)) -> Result<(), String> {
        self.step(landed)?;
        match self.file {
            Some(_) => Err(tr!("archive-other-size")),
            None => Ok(()),
        }
    }
}

/// A name [`copy_tree_out`] makes: a folder, or a file with its entry and that entry's index in
/// the listing, and its own names in listing order (indices into [`TreePlan::nodes`]).
struct Node<'a> {
    name: &'a OsStr,
    file: Option<(usize, &'a Entry)>,
    kids: Vec<usize>,
}

/// What [`copy_tree_out`] makes under its folder (node 0), as the compressed folder shows it:
/// each name keeps its first listing, a folder another entry implies included, and an entry
/// under a name first listed as a file is left out.
struct TreePlan<'a> {
    nodes: Vec<Node<'a>>,
    bytes: u64,
}

/// The [`TreePlan`] for the entries `picked` of the listing `entries`, each path under the
/// folder at most `room` bytes long; cancelling `m` stops it.
fn tree_plan<'a>(
    entries: &'a [Entry],
    picked: &[PathBuf],
    room: usize,
    m: &Meter,
) -> Result<TreePlan<'a>, String> {
    use std::collections::{HashMap, HashSet};
    let mut names: BTreeMap<&Path, HashSet<&OsStr>> = BTreeMap::new();
    for p in picked {
        let name = p.file_name().ok_or_else(|| tr!("archive-unread"))?;
        let parent = p.parent().unwrap_or(Path::new(""));
        names.entry(parent).or_default().insert(name);
    }
    let mut nodes = vec![Node {
        name: OsStr::new(""),
        file: None,
        kids: Vec::new(),
    }];
    let mut named: HashMap<(usize, &OsStr), usize> = HashMap::new();
    let mut bytes = 0u64;
    for (i, e) in entries.iter().enumerate() {
        if i % 4096 == 0 && m.cancelled() {
            return Err(Failure::Cancelled.why());
        }
        let rest = names.iter().find_map(|(parent, names)| {
            let rest = e.path.strip_prefix(parent).ok()?;
            names.contains(rest.iter().next()?).then_some(rest)
        });
        let Some(rest) = rest else {
            continue;
        };
        if rest.as_os_str().len() > room {
            return Err(tr!("archive-path-too-long"));
        }
        let mut at = 0;
        let mut steps = rest.iter().peekable();
        while let Some(name) = steps.next() {
            let last = steps.peek().is_none();
            at = match named.get(&(at, name)) {
                Some(_) if last => break,
                Some(&n) if nodes[n].file.is_some() => break,
                Some(&n) => n,
                None => {
                    let file = (last && e.kind == Kind::File).then_some((i, e));
                    bytes = bytes.saturating_add(file.map_or(0, |(_, e)| e.size));
                    nodes.push(Node {
                        name,
                        file,
                        kids: Vec::new(),
                    });
                    let n = nodes.len() - 1;
                    nodes[at].kids.push(n);
                    named.insert((at, name), n);
                    n
                }
            };
        }
    }
    for p in picked {
        let name = p.file_name().unwrap_or_default();
        if !named.contains_key(&(0, name)) {
            return Err(tr!(
                "archive-not-held",
                name = name.to_string_lossy().into_owned()
            ));
        }
    }
    Ok(TreePlan { nodes, bytes })
}

/// Whether `a` and `b` are on one file system.
fn same_device(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

/// Copies the entries `picked` of `archive` (paths inside it, each a file or a folder with all
/// that `entries` lists below it) into the empty private folder `into` under their own names,
/// files 0644 and folders 0755, laid out as [`TreePlan`] says: the folders first, then the files
/// in archive order, up to [`BATCH_FILES`] per [`write_entries`]. Refused past
/// [`extract::FREE_SHARE_PERCENT`] of the free space, [`MAX_ENTRIES`] or a path longer than the
/// system takes. When the copies then go on to the folder `dest`, they must fit there too: twice
/// over when it shares `into`'s file system. Cancelling `m` stops it, part way through a file
/// too. Returns each picked entry's copy.
pub fn copy_tree_out(
    archive: &Path,
    entries: &[Entry],
    picked: &[PathBuf],
    into: &Path,
    dest: Option<&Path>,
    m: &Meter,
) -> Result<Vec<PathBuf>, String> {
    copy_tree_within(archive, entries, picked, (into, dest), None, m)
}

/// [`copy_tree_out`], refused past `drag_most` bytes when it is a drag's.
fn copy_tree_within(
    archive: &Path,
    entries: &[Entry],
    picked: &[PathBuf],
    (into, dest): (&Path, Option<&Path>),
    drag_most: Option<u64>,
    m: &Meter,
) -> Result<Vec<PathBuf>, String> {
    use crate::progress::{Phase, Totals};
    use std::os::unix::fs::DirBuilderExt;
    let room = (libc::PATH_MAX as usize).saturating_sub(into.as_os_str().len() + 2);
    let TreePlan { nodes, bytes } = tree_plan(entries, picked, room, m)?;
    if let Some(most) = drag_most.filter(|&most| bytes > most) {
        return Err(tr!(
            "archive-drag-too-large",
            size = crate::props::short_size(bytes),
            most = crate::props::short_size(most)
        ));
    }
    let items = (nodes.len() - 1) as u64;
    let limits = extract::Limits::of(into);
    if items > limits.entries {
        return Err(tr!("archive-too-many", most = limits.entries));
    }
    let twice = dest.is_some_and(|d| same_device(into, d));
    let fits_dest = || dest.is_none_or(|d| twice || bytes <= extract::Limits::of(d).bytes);
    let need = if twice {
        bytes.saturating_mul(2)
    } else {
        bytes
    };
    if need > limits.bytes || !fits_dest() {
        return Err(tr!(
            "archive-no-room",
            size = crate::props::short_size(bytes)
        ));
    }
    m.add_totals(Totals { bytes, items });
    m.set_phase(Phase::Copying);
    let mut done = Totals::default();
    let mut files = Vec::new();
    let mut path = into.to_path_buf();
    // Each folder being filled, with the next of its names to make.
    let mut open = vec![(0, 0)];
    while let Some(top) = open.last_mut() {
        let (n, next) = *top;
        top.1 += 1;
        let Some(&kid) = nodes[n].kids.get(next) else {
            open.pop();
            path.pop();
            continue;
        };
        path.push(nodes[kid].name);
        if let Some((i, e)) = nodes[kid].file {
            files.push((i, (e, path.clone())));
            path.pop();
            continue;
        }
        if m.cancelled() {
            return Err(Failure::Cancelled.why());
        }
        (std::fs::DirBuilder::new().mode(0o755).create(&path)).map_err(|e| e.to_string())?;
        m.set_current(path.strip_prefix(into).unwrap_or(&path).as_os_str());
        done.items += 1;
        m.settle(done);
        open.push((kid, 0));
    }
    // bsdtar writes the files it finds in archive order, the listing's.
    files.sort_unstable_by_key(|&(i, _)| i);
    let files: Vec<(&Entry, PathBuf)> = files.into_iter().map(|(_, f)| f).collect();
    let mut rest = files.as_slice();
    while !rest.is_empty() {
        if m.cancelled() {
            return Err(Failure::Cancelled.why());
        }
        let mut names = 0;
        let take = (rest.iter().take(BATCH_FILES))
            .take_while(|(e, _)| {
                names += 2 * e.path.as_os_str().len() + 3;
                names <= BATCH_NAME_BYTES
            })
            .count()
            .max(1);
        let (batch, after) = rest.split_at(take);
        write_entries(archive, batch, 0o644, m, |k| {
            let (e, path) = &batch[k];
            m.set_current(path.strip_prefix(into).unwrap_or(path).as_os_str());
            done.items += 1;
            done.bytes = done.bytes.saturating_add(e.size);
            m.settle(done);
        })?;
        rest = after;
    }
    if m.cancelled() {
        return Err(Failure::Cancelled.why());
    }
    Ok((picked.iter())
        .filter_map(|p| Some(into.join(p.file_name()?)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn bsdtar(dir: &Path, args: &[&str]) {
        let st = Command::new("bsdtar")
            .current_dir(dir)
            .args(args)
            .status()
            .expect("bsdtar runs");
        assert!(st.success(), "bsdtar {args:?}");
    }

    fn entry(path: &str, kind: Kind, size: u64) -> Entry {
        Entry {
            path: path.into(),
            kind,
            size,
            modified: None,
        }
    }

    fn listed(archive: &Path) -> Vec<Entry> {
        list(archive, &Meter::default()).expect("listed")
    }

    #[test]
    fn the_argv_opens_one_name_with_every_byte_escaped_and_its_end_anchored() {
        assert_eq!(
            open_argv(Path::new("/d/-x.zip"), Path::new("d/a$")),
            ["bsdtar", "-xqO", "-f", "/d/-x.zip", "./\\d/\\a\\$$"]
        );
    }

    #[test]
    fn listed_lines_read_names_back_and_skip_links_and_paths_that_leave() {
        let now = 1_800_000_000;
        let line = |l: &str| listed_entry(l.as_bytes(), now);
        let e = line("-rw-r--r--  0 1000   1000        2 Jan  2  2020 ./n\\nl \\303\\251\\\\x")
            .expect("a file");
        assert_eq!(e.path, Path::new("n\nl é\\x"));
        assert_eq!((e.kind, e.size), (Kind::File, 2));
        assert_eq!(
            line("drwxr-xr-x  0 1000   1000        0 Jan  2  2020 ./x -> y/").map(|e| e.path),
            Some("x -> y".into())
        );
        for skipped in [
            "lrwxrwxrwx  0 1000   1000        0 Jan  2  2020 l -> /etc/passwd",
            "hrw-r--r--  0 1000   1000        0 Jan  2  2020 h link to f",
            "crw-r--r--  0 0      0         1,3 Jan  2  2020 null",
            "drwxr-xr-x  0 1000   1000        0 Jan  2  2020 ./",
            "-rw-r--r--  0 1000   1000        1 Jan  2  2020 a/../../up",
            "-rw-r--r--  0 1000   1000        1 Jan  2  2020 ../evil",
            "-rw-r--r--  0 1000   1000        1 Jan  2  2020 /etc/abs",
        ] {
            assert_eq!(line(skipped), None, "{skipped}");
        }
    }

    #[test]
    fn a_listed_clock_time_takes_the_year_nearest_now() {
        let now = 1_800_000_000;
        let year = |t: i64| crate::locale::localtime(t).tm_year + 1900;
        let this = year(now);
        let t = listed_time("Mar", "5", "2020", now).expect("a time");
        let tm = crate::locale::localtime(t);
        assert_eq!((tm.tm_year + 1900, tm.tm_mon, tm.tm_mday), (2020, 2, 5));
        let tm = crate::locale::localtime(now);
        let month = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ][tm.tm_mon as usize];
        let t = listed_time(month, &tm.tm_mday.to_string(), "00:00", now).expect("a time");
        assert_eq!(year(t), this);
        assert!((now - t).abs() < 2 * 86_400);
        assert_eq!(listed_time("Foo", "5", "2020", now), None);
    }

    #[test]
    fn children_list_one_level_and_imply_folders() {
        let entries = [
            entry("a.txt", Kind::File, 3),
            entry("deep/sub/f", Kind::File, 1),
            entry("dir", Kind::Folder, 0),
            entry("dir/x", Kind::File, 5),
            entry("a.txt", Kind::File, 9),
        ];
        let names = |inner: &str| -> Vec<(String, Kind, u64)> {
            children(&entries, Path::new(inner))
                .into_iter()
                .map(|c| (c.name.to_string_lossy().into_owned(), c.kind, c.size))
                .collect()
        };
        assert_eq!(
            names(""),
            [
                ("a.txt".into(), Kind::File, 3),
                ("deep".into(), Kind::Folder, 0),
                ("dir".into(), Kind::Folder, 0),
            ]
        );
        assert_eq!(names("deep"), [("sub".into(), Kind::Folder, 0)]);
        assert_eq!(names("dir"), [("x".into(), Kind::File, 5)]);
        assert!(names("dir/x").is_empty());
    }

    #[test]
    fn every_name_lists_as_stored_and_opens_its_own_bytes() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let work = t.path().join("work");
        std::fs::create_dir_all(work.join("dir")).unwrap();
        let names = [
            "a",
            "a$",
            "dir/a*b.txt",
            "dir/a?b.txt",
            "-C",
            "é.txt",
            "n\nl",
            "t\tb",
            "b\\x",
            "q[1]",
            "x -> y",
        ];
        for n in names {
            std::fs::write(work.join(n), format!("<{n}>")).unwrap();
        }
        for (name, format) in [("t.zip", "zip"), ("t.tar.gz", "ustar")] {
            let st = Command::new("bsdtar")
                .current_dir(&work)
                .args(["--format", format, "-a", "-cf"])
                .arg(t.path().join(name))
                .args(["."])
                .status()
                .unwrap();
            assert!(st.success());
            let archive = t.path().join(name);
            let entries = listed(&archive);
            for n in names {
                let e = (entries.iter())
                    .find(|e| e.path == Path::new(n))
                    .unwrap_or_else(|| panic!("{name} lists {n:?}: {entries:?}"));
                let into = tempfile::tempdir().unwrap();
                let got = copy_out(&archive, e, into.path()).expect("copied");
                assert_eq!(
                    std::fs::read_to_string(&got).unwrap(),
                    format!("<{n}>"),
                    "{name}"
                );
                let mode = std::fs::metadata(&got).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o400, "the copy opens read-only");
            }
        }
    }

    #[test]
    fn a_zip_lists_from_its_headers_without_reading_member_data() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("big"), vec![0u8; 4 << 20]).unwrap();
        bsdtar(t.path(), &["-a", "-cf", "t.zip", "big"]);
        let zip = t.path().join("t.zip");
        let mut bytes = std::fs::read(&zip).unwrap();
        let field = |at: usize| usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
        let data = 30 + field(26) + field(28);
        for b in &mut bytes[data + 8..data + 64] {
            *b = 0xff;
        }
        std::fs::write(&zip, &bytes).unwrap();
        let entries = listed(&zip);
        assert_eq!(
            entries,
            [entry("big", Kind::File, 4 << 20)].map(|e| Entry {
                modified: entries[0].modified,
                ..e
            })
        );
        let into = tempfile::tempdir().unwrap();
        assert!(
            copy_out(&zip, &entries[0], into.path()).is_err(),
            "the data is damaged: only reading it fails"
        );
    }

    #[test]
    fn a_copy_out_larger_than_listed_or_than_the_room_is_refused_and_removed() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("big"), "0123456789").unwrap();
        bsdtar(t.path(), &["-cf", "t.tar", "big"]);
        let tar = t.path().join("t.tar");
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let lying = entry("big", Kind::File, 4);
        assert!(copy_out_within(&tar, &lying, &into, u64::MAX).is_err());
        assert!(!into.join("big").exists(), "a refused copy leaves nothing");
        let honest = entry("big", Kind::File, 10);
        assert!(copy_out_within(&tar, &honest, &into, 9).is_err());
        assert!(!into.join("big").exists());
        assert!(copy_out_within(&tar, &honest, &into, 10).is_ok());
    }

    #[test]
    fn a_name_listed_twice_opens_its_first_copy_and_a_file_named_like_a_folder_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let (one, two) = (t.path().join("one"), t.path().join("two"));
        std::fs::create_dir_all(one.join("b")).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("a"), "first").unwrap();
        std::fs::write(two.join("a"), "second!").unwrap();
        std::fs::write(one.join("b/y"), "below").unwrap();
        std::fs::write(two.join("b"), "file b").unwrap();
        let tar = t.path().join("t.tar");
        bsdtar(&one, &["-cf", tar.to_str().unwrap(), "a", "b"]);
        bsdtar(&two, &["-rf", tar.to_str().unwrap(), "a", "b"]);
        let entries = listed(&tar);
        let first = |p: &str| entries.iter().find(|e| e.path == Path::new(p)).unwrap();
        let into = tempfile::tempdir().unwrap();
        let a = copy_out(&tar, first("a"), into.path()).unwrap();
        assert_eq!(std::fs::read_to_string(a).unwrap(), "first");
        let file_b = (entries.iter())
            .find(|e| e.path == Path::new("b") && e.kind == Kind::File)
            .unwrap();
        assert!(
            copy_out(&tar, file_b, into.path()).is_err(),
            "the folder `b` is the first match, so the copy comes out short and is refused"
        );
        assert!(!into.path().join("b").exists());
    }

    #[test]
    fn a_copy_out_never_writes_through_a_link_in_its_folder() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("f"), "new").unwrap();
        bsdtar(t.path(), &["-cf", "t.tar", "f"]);
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let victim = t.path().join("victim");
        std::fs::write(&victim, "old").unwrap();
        std::os::unix::fs::symlink(&victim, into.join("f")).unwrap();
        let e = entry("f", Kind::File, 3);
        assert!(copy_out(&t.path().join("t.tar"), &e, &into).is_err());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "old");
    }

    #[test]
    fn a_hostile_archive_lists_without_its_climbing_and_rooted_entries() {
        let t = tempfile::tempdir().unwrap();
        let work = t.path().join("work");
        std::fs::create_dir(&work).unwrap();
        for n in ["evil", "abs", "fine"] {
            std::fs::write(work.join(n), "x").unwrap();
        }
        bsdtar(
            &work,
            &[
                "-cf",
                "../t.tar",
                "-P",
                "-s",
                ",^evil,../evil,",
                "-s",
                ",^abs,/etc/abs,",
                "evil",
                "abs",
                "fine",
            ],
        );
        let raw = Command::new("bsdtar")
            .args(["-tf", "t.tar"])
            .current_dir(t.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&raw.stdout),
            "../evil\n/etc/abs\nfine\n"
        );
        let names: Vec<PathBuf> = (listed(&t.path().join("t.tar")).into_iter())
            .map(|e| e.path)
            .collect();
        assert_eq!(names, [PathBuf::from("fine")]);
    }

    #[test]
    fn a_listing_past_the_cap_or_cancelled_stops_and_a_file_that_is_no_archive_says_so() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("x.zip");
        std::fs::write(&f, "not a zip").unwrap();
        assert!(list(&f, &Meter::default()).is_err());
        std::fs::write(t.path().join("a"), "a").unwrap();
        bsdtar(t.path(), &["-cf", "t.tar", "a"]);
        let m = Meter::default();
        m.cancel();
        assert!(list(&t.path().join("t.tar"), &m).is_err());
    }

    #[test]
    fn a_listing_whose_names_pass_their_most_is_refused_as_it_passes_them() {
        let t = tempfile::tempdir().unwrap();
        let work = t.path().join("work");
        std::fs::create_dir(&work).unwrap();
        let names: Vec<String> = (0..50)
            .map(|i| format!("{i:02}{}", "n".repeat(98)))
            .collect();
        for n in &names {
            std::fs::write(work.join(n), "").unwrap();
        }
        let args: Vec<&str> = ["-czf", "../many.tar.gz"]
            .into_iter()
            .chain(names.iter().map(String::as_str))
            .collect();
        bsdtar(&work, &args);
        let archive = t.path().join("many.tar.gz");
        let m = Meter::default();
        assert_eq!(list_within(&archive, &m, 5000).map(|e| e.len()), Ok(50));
        assert_eq!(
            list_within(&archive, &m, 4000),
            Err("its names add up to more than 3.90 KB.".into())
        );
    }

    #[test]
    fn listings_are_kept_until_the_archive_changes() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a"), "a").unwrap();
        bsdtar(t.path(), &["-cf", "t.tar", "a"]);
        let tar = t.path().join("t.tar");
        let l = Listings::default();
        let one = l.entries(&tar).unwrap();
        assert!(
            Arc::ptr_eq(&one, &l.clone().entries(&tar).unwrap()),
            "listed once"
        );
        std::fs::write(t.path().join("b"), "b").unwrap();
        bsdtar(t.path(), &["-rf", "t.tar", "b"]);
        let two = l.entries(&tar).unwrap();
        assert_eq!(two.len(), 2, "a changed archive lists again");
    }

    #[test]
    fn listings_keep_the_last_few_archives_and_their_failures() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a"), "a").unwrap();
        let l = Listings::default();
        for n in 0..=KEPT_LISTINGS {
            bsdtar(t.path(), &["-cf", &format!("{n}.tar"), "a"]);
            l.entries(&t.path().join(format!("{n}.tar"))).unwrap();
        }
        let kept = |l: &Listings| l.0.lock().unwrap().listed.len();
        assert_eq!(kept(&l), KEPT_LISTINGS);
        let bad = t.path().join("bad.zip");
        std::fs::write(&bad, "not a zip").unwrap();
        assert!(l.entries(&bad).is_err());
        let inner = l.0.lock().unwrap();
        assert!(
            inner.listed.iter().any(|(p, _, r)| *p == bad && r.is_err()),
            "the failure is kept for a visit back"
        );
        drop(inner);
        assert_eq!(
            l.entries(&t.path().join("gone.zip")).unwrap_err(),
            tr!("archive-missing")
        );
    }

    #[test]
    fn a_child_over_its_cpu_limit_reads_as_taking_too_long() {
        let argv = ["sh", "-c", "while :; do :; done"].map(OsString::from);
        let mut child = spawn(&argv, 1).unwrap();
        let status = child.wait().unwrap();
        assert!(ran_out(&status), "{status:?}");
        assert_eq!(cpu_secs(0), BASE_CPU_SECS);
        assert_eq!(cpu_secs(1 << 30), BASE_CPU_SECS + 512);
    }

    #[test]
    fn programs_scripts_and_launchers_run_and_documents_do_not() {
        let t = tempfile::tempdir().unwrap();
        let put = |name: &str, body: &[u8]| {
            let p = t.path().join(name);
            std::fs::write(&p, body).unwrap();
            p
        };
        let elf = put("tool", b"\x7fELF\x02\x01");
        let script = put("run", b"#!/bin/sh\n");
        let note = put("note.txt", b"hello");
        assert!(runs_as_a_program(&elf, None));
        assert!(runs_as_a_program(&script, Some("text/plain")));
        assert!(runs_as_a_program(&note, Some("application/x-desktop")));
        assert!(runs_as_a_program(&note, Some("application/x-msdownload")));
        assert!(!runs_as_a_program(&note, Some("text/plain")));
        for name in [
            "tool.py",
            "Setup.CMD",
            "a.vbs",
            "x.scr",
            "y.pif",
            "z.com",
            "app.jnlp",
        ] {
            let p = put(name, b"plain");
            assert!(runs_as_a_program(&p, Some("text/plain")), "{name}");
        }
    }

    #[test]
    fn scratch_folders_are_private_and_old_ones_go() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("archives");
        let one = scratch_in(&base, 3600).unwrap();
        assert!(one.starts_with(&base));
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&one), 0o700);
        assert_eq!(mode(&base), 0o700);
        std::fs::write(one.join("f"), "x").unwrap();
        let two = scratch_in(&base, 3600).unwrap();
        assert_ne!(one, two);
        assert!(one.exists(), "a folder opened within the hour stays");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::open(&one)
            .unwrap()
            .set_modified(old)
            .unwrap();
        scratch_in(&base, 3600).unwrap();
        assert!(!one.exists(), "an older folder goes");
        assert!(two.exists());
    }

    /// A zip of `a.txt` encrypted with `how` (`zipcrypt` or `aes256`), in `dir`.
    fn locked_zip(dir: &Path, how: &str, extra: &str) -> PathBuf {
        std::fs::write(dir.join("a.txt"), "secret").unwrap();
        let options = format!("zip:encryption={how}{extra}");
        let args = [
            "--options",
            &options,
            "--passphrase",
            "pw",
            "-a",
            "-cf",
            "locked.zip",
        ];
        bsdtar(dir, &[&args[..], &["a.txt"]].concat());
        dir.join("locked.zip")
    }

    #[test]
    fn an_encrypted_zip_lists_and_opens_as_password_protected_at_once() {
        for how in ["zipcrypt", "aes256"] {
            let t = tempfile::tempdir().unwrap();
            let zip = locked_zip(t.path(), how, "");
            assert!(zip_encrypted(&zip), "{how}");
            let started = std::time::Instant::now();
            assert_eq!(
                list(&zip, &Meter::default()),
                Err("it is password-protected.".into()),
                "{how}"
            );
            let into = tempfile::tempdir().unwrap();
            let e = entry("a.txt", Kind::File, 6);
            assert_eq!(
                copy_out(&zip, &e, into.path()),
                Err("it is password-protected.".into()),
                "bsdtar asks for no passphrase ({how})"
            );
            assert!(
                started.elapsed().as_secs() < 10,
                "{how} took {:?}",
                started.elapsed()
            );
            assert!(!into.path().join("a.txt").exists());
        }
    }

    #[test]
    fn a_zip64_directory_is_read_for_encrypted_entries() {
        let t = tempfile::tempdir().unwrap();
        let zip = locked_zip(t.path(), "zipcrypt", ",zip:zip64");
        let mut bytes = std::fs::read(&zip).unwrap();
        let end = bytes.len() - 22;
        assert_eq!(&bytes[end..end + 4], b"PK\x05\x06");
        bytes[end + 12..end + 20].fill(0xff);
        std::fs::write(&zip, &bytes).unwrap();
        assert!(zip_encrypted(&zip), "found through the zip64 record");
    }

    #[test]
    fn plain_zips_tars_and_truncated_files_are_not_encrypted() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a.txt"), "plain").unwrap();
        bsdtar(t.path(), &["-a", "-cf", "plain.zip", "a.txt"]);
        bsdtar(t.path(), &["-czf", "plain.tar.gz", "a.txt"]);
        for name in ["plain.zip", "plain.tar.gz", "a.txt"] {
            assert!(!zip_encrypted(&t.path().join(name)), "{name}");
        }
        let zip = std::fs::read(t.path().join("plain.zip")).unwrap();
        for cut in [4, 30, zip.len() - 10] {
            std::fs::write(t.path().join("cut.zip"), &zip[..cut]).unwrap();
            assert!(!zip_encrypted(&t.path().join("cut.zip")), "cut at {cut}");
        }
        let fifo = std::ffi::CString::new(t.path().join("fifo.zip").into_os_string().into_vec());
        // SAFETY: a NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(fifo.unwrap().as_ptr(), 0o600) }, 0);
        assert!(
            !zip_encrypted(&t.path().join("fifo.zip")),
            "a FIFO is not read"
        );
    }

    #[test]
    fn bsdtar_saying_a_passphrase_or_encryption_reads_as_locked() {
        for said in [
            "bsdtar: Couldn't read passphrase: Bad file descriptor",
            "a.txt: Incorrect passphrase: Unknown error -1",
            "bsdtar: The archive header is encrypted, but currently not supported",
            "a.txt: The file content is encrypted, but currently not supported",
        ] {
            assert!(locked(said.as_bytes()), "{said}");
        }
        assert!(!locked(
            b"bsdtar: Error opening archive: Unrecognized archive format"
        ));
        assert!(!locked(b"x Incorrect passphrase\n"), "an entry's name");
    }

    #[test]
    fn an_address_below_a_file_splits_into_the_file_and_its_inner_path() {
        let t = tempfile::tempdir().unwrap();
        let zip = t.path().join("a.zip");
        std::fs::write(&zip, "x").unwrap();
        std::fs::create_dir(t.path().join("dir")).unwrap();
        let split = |rest: &str| address(&t.path().join(rest));
        assert_eq!(split("a.zip"), Some((zip.clone(), PathBuf::new())));
        assert_eq!(split("a.zip/"), Some((zip.clone(), PathBuf::new())));
        assert_eq!(split("a.zip/docs"), Some((zip.clone(), "docs".into())));
        assert_eq!(
            split("a.zip/docs/./more/"),
            Some((zip.clone(), "docs/more".into()))
        );
        assert_eq!(split("a.zip/docs/../.."), None, "no step climbs out");
        assert_eq!(split("dir"), None, "a folder is no archive");
        assert_eq!(
            split("dir/none/deeper"),
            None,
            "the nearest there is a folder"
        );
        assert_eq!(split("gone.zip/docs"), None);
    }

    #[test]
    fn an_archive_holds_its_listed_and_implied_folders_only() {
        let entries = [
            entry("docs", Kind::Folder, 0),
            entry("docs/a.txt", Kind::File, 1),
            entry("deep/er/b.txt", Kind::File, 1),
            entry("note.txt", Kind::File, 1),
        ];
        for (inner, held) in [
            ("", true),
            ("docs", true),
            ("deep", true),
            ("deep/er", true),
            ("note.txt", false),
            ("docs/a.txt", false),
            ("doc", false),
            ("missing", false),
        ] {
            assert_eq!(holds_folder(&entries, Path::new(inner)), held, "{inner:?}");
        }
    }

    /// `pack.tar.gz` in `dir` holding `docs/` (with an awkward name and a nested folder), a top
    /// file and an empty folder.
    fn pack(dir: &Path) -> PathBuf {
        let work = dir.join("work");
        std::fs::create_dir_all(work.join("docs/deep")).unwrap();
        std::fs::create_dir_all(work.join("empty")).unwrap();
        std::fs::write(work.join("docs/a.txt"), "a").unwrap();
        std::fs::write(work.join("docs/deep/-C n\nl"), "odd").unwrap();
        std::fs::write(work.join("top.txt"), "top").unwrap();
        bsdtar(
            &work,
            &["-czf", "../pack.tar.gz", "docs", "top.txt", "empty"],
        );
        dir.join("pack.tar.gz")
    }

    #[test]
    fn picked_files_and_folders_copy_out_whole_as_plain_files_and_folders() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let archive = pack(t.path());
        let entries = listed(&archive);
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let picked = ["docs", "top.txt", "empty"].map(PathBuf::from);
        let m = Meter::default();
        let got = copy_tree_out(&archive, &entries, &picked, &into, None, &m).expect("copied");
        assert_eq!(got, ["docs", "top.txt", "empty"].map(|n| into.join(n)));
        let read = |p: &str| std::fs::read_to_string(into.join(p)).unwrap();
        assert_eq!(read("docs/a.txt"), "a");
        assert_eq!(read("docs/deep/-C n\nl"), "odd");
        assert_eq!(read("top.txt"), "top");
        assert!(into.join("empty").is_dir());
        let mode = |p: &str| {
            std::fs::metadata(into.join(p))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("top.txt"), 0o644);
        assert_eq!(mode("docs/deep") & 0o700, 0o700);
        let s = m.snapshot();
        assert_eq!(
            (s.items, s.bytes),
            (6, 7),
            "three folders and three files, seven bytes"
        );
    }

    #[test]
    fn a_tree_copies_out_in_archive_order_across_batches_each_file_with_its_own_bytes() {
        let t = tempfile::tempdir().unwrap();
        let work = t.path().join("work");
        std::fs::create_dir_all(work.join("a")).unwrap();
        std::fs::create_dir_all(work.join("b")).unwrap();
        // The archive interleaves the folders, and some files are empty.
        let mut names = Vec::new();
        for i in 0..BATCH_FILES + 5 {
            let name = format!("{}/{i}", ["a", "b"][i % 2]);
            std::fs::write(work.join(&name), format!("{i}-").repeat(i % 4)).unwrap();
            names.push(name);
        }
        let mut args = vec!["-czf", "../p.tar.gz"];
        args.extend(names.iter().map(String::as_str));
        bsdtar(&work, &args);
        let archive = t.path().join("p.tar.gz");
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let picked = ["a", "b"].map(PathBuf::from);
        let m = Meter::default();
        copy_tree_out(&archive, &listed(&archive), &picked, &into, None, &m).expect("copied");
        for (i, name) in names.iter().enumerate() {
            let got = std::fs::read_to_string(into.join(name)).unwrap();
            assert_eq!(got, format!("{i}-").repeat(i % 4), "{name}");
        }
        assert_eq!(m.snapshot().items, 2 + names.len() as u64);
    }

    #[test]
    fn a_batch_whose_bytes_fall_short_of_the_listing_leaves_none_of_its_files() {
        let t = tempfile::tempdir().unwrap();
        let archive = pack(t.path());
        let mut entries = listed(&archive);
        let a = (entries.iter_mut()).find(|e| e.path == Path::new("docs/a.txt"));
        a.expect("listed").size += 1;
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let picked = [PathBuf::from("docs")];
        assert_eq!(
            copy_tree_out(&archive, &entries, &picked, &into, None, &Meter::default()),
            Err("it holds less than its listing says.".into())
        );
        assert!(!into.join("docs/a.txt").exists() && !into.join("docs/deep/-C n\nl").exists());
    }

    #[test]
    fn a_copy_out_takes_the_first_of_a_name_listed_twice_and_no_file_under_a_folder() {
        let t = tempfile::tempdir().unwrap();
        let (one, two) = (t.path().join("one"), t.path().join("two"));
        std::fs::create_dir_all(one.join("d/b")).unwrap();
        std::fs::create_dir_all(two.join("d")).unwrap();
        std::fs::write(one.join("d/a"), "first").unwrap();
        std::fs::write(two.join("d/a"), "second!").unwrap();
        std::fs::write(one.join("d/b/y"), "below").unwrap();
        std::fs::write(two.join("d/b"), "file b").unwrap();
        let tar = t.path().join("t.tar");
        bsdtar(&one, &["-cf", tar.to_str().unwrap(), "d"]);
        bsdtar(&two, &["-rf", tar.to_str().unwrap(), "d/a", "d/b"]);
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let picked = [PathBuf::from("d")];
        copy_tree_out(&tar, &listed(&tar), &picked, &into, None, &Meter::default())
            .expect("copied");
        assert_eq!(std::fs::read_to_string(into.join("d/a")).unwrap(), "first");
        assert_eq!(
            std::fs::read_to_string(into.join("d/b/y")).unwrap(),
            "below"
        );
    }

    #[test]
    fn a_copy_out_of_an_unlisted_entry_too_large_or_lying_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let archive = pack(t.path());
        let entries = listed(&archive);
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let m = Meter::default();
        let copy = |entries: &[Entry], picked: &str| {
            copy_tree_out(&archive, entries, &[PathBuf::from(picked)], &into, None, &m)
        };
        assert_eq!(
            copy(&entries, "gone"),
            Err("it no longer holds gone.".into())
        );
        let huge = [entry("top.txt", Kind::File, u64::MAX / 2)];
        assert!(copy(&huge, "top.txt").unwrap_err().starts_with("it takes "));
        let short = [entry("top.txt", Kind::File, 2)];
        assert_eq!(
            copy(&short, "top.txt"),
            Err("it holds more than its listing says.".into())
        );
        assert!(
            !into.join("top.txt").exists(),
            "a refused copy leaves nothing"
        );
        m.cancel();
        assert!(copy(&entries, "docs").is_err(), "a cancelled copy stops");
        assert!(!into.join("docs/a.txt").exists());
    }

    /// Runs `job` to its end, returning its events.
    fn job_events(mut job: ArchiveJob, woke: std::sync::mpsc::Receiver<()>) -> Vec<ArchiveEvent> {
        let mut events = Vec::new();
        while !job.is_finished() {
            woke.recv_timeout(std::time::Duration::from_secs(30))
                .expect("a wake");
            events.extend(job.poll());
        }
        events
    }

    /// The wake a job sends, and where it lands.
    fn waker() -> (impl Fn() + Send + 'static, std::sync::mpsc::Receiver<()>) {
        let (wtx, woke) = std::sync::mpsc::channel();
        (
            move || {
                let _ = wtx.send(());
            },
            woke,
        )
    }

    /// Waits until `path` is gone.
    fn until_gone(path: &Path) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while path.exists() {
            assert!(std::time::Instant::now() < end, "{} goes", path.display());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn a_copy_job_lands_each_picked_copy_in_its_private_scratch_folder() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("archives");
        let archive = pack(t.path());
        let entries = Arc::new(listed(&archive));
        let picked = ["top.txt", "docs"].map(PathBuf::from).to_vec();
        let scratch = Scratch::new_in(&base, 3600).unwrap();
        let dir = scratch.path().to_path_buf();
        let (wake, woke) = waker();
        let job = copy_job(
            archive.clone(),
            entries.clone(),
            picked,
            (scratch.clone(), None),
            None,
            wake,
        );
        let landed: Vec<PathBuf> = (job_events(job, woke).into_iter())
            .filter_map(|ev| match ev {
                ArchiveEvent::Landed { into, .. } => Some(into),
                ArchiveEvent::Failed { text, .. } => panic!("{text}"),
                ArchiveEvent::Finished => None,
            })
            .collect();
        let [top, docs] = &landed[..] else {
            panic!("{landed:?}")
        };
        assert_eq!((top.parent(), docs.parent()), (Some(&*dir), Some(&*dir)));
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&dir), mode(&base)), (0o700, 0o700));
        assert_eq!(std::fs::read_to_string(docs.join("a.txt")).unwrap(), "a");

        let failed = Scratch::new_in(&base, 3600).unwrap();
        let (wake, woke) = waker();
        let gone = vec![PathBuf::from("gone")];
        let job = copy_job(archive, entries, gone, (failed, None), None, wake);
        let events = job_events(job, woke);
        assert!(
            matches!(&events[..], [ArchiveEvent::Failed { text, .. }, ArchiveEvent::Finished]
                if text == "it no longer holds gone."),
            "{events:?}"
        );
        assert!(top.exists(), "the first copies stay while held");
    }

    /// `big.tar.gz` in `dir`, holding `big`: `size` zero bytes.
    fn big_pack(dir: &Path, size: usize) -> PathBuf {
        let work = dir.join("big-work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("big"), vec![0u8; size]).unwrap();
        bsdtar(&work, &["-czf", "../big.tar.gz", "big"]);
        std::fs::remove_file(work.join("big")).unwrap();
        dir.join("big.tar.gz")
    }

    #[test]
    fn a_copy_out_paused_mid_file_stops_writing_and_a_cancel_then_leaves_nothing() {
        use std::time::Duration;
        let t = tempfile::tempdir().unwrap();
        let archive = big_pack(t.path(), 64 << 20);
        let entries = listed(&archive);
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let m = Arc::new(Meter::default());
        m.set_paused(true);
        let copy = {
            let (m, into) = (m.clone(), into.clone());
            std::thread::spawn(move || {
                let picked = [PathBuf::from("big")];
                copy_tree_out(&archive, &entries, &picked, &into, None, &m)
            })
        };
        let size = || std::fs::metadata(into.join("big")).map_or(0, |m| m.len());
        std::thread::sleep(Duration::from_millis(300));
        let held = size();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(size(), held, "nothing is written while paused");
        assert!(held < 64 << 20, "the file is not out yet");
        m.cancel();
        assert_eq!(
            copy.join().unwrap(),
            Err("the operation was cancelled.".into())
        );
        assert!(!into.join("big").exists(), "the part written is removed");
    }

    #[test]
    fn a_copy_job_cancelled_during_its_last_file_lands_nothing() {
        let t = tempfile::tempdir().unwrap();
        let archive = big_pack(t.path(), 64 << 20);
        let entries = Arc::new(listed(&archive));
        let scratch = Scratch::new_in(&t.path().join("archives"), 3600).unwrap();
        let dir = scratch.path().to_path_buf();
        let picked = vec![PathBuf::from("big")];
        let (wake, woke) = waker();
        let job = copy_job(archive, entries, picked, (scratch, None), None, wake);
        job.meter().set_paused(true);
        for _ in 0..2000 {
            if dir.join("big").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(dir.join("big").exists(), "the file is being written");
        job.meter().cancel();
        let events = job_events(job, woke);
        assert!(
            matches!(&events[..], [ArchiveEvent::Finished]),
            "{events:?}"
        );
        until_gone(&dir);
    }

    #[test]
    fn a_scratch_folder_stays_while_any_holder_holds_it_and_goes_with_the_last() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("archives");
        let owner = Scratch::new_in(&base, 3600).unwrap();
        let dir = owner.path().to_path_buf();
        assert!(dir.file_name().unwrap().as_bytes().starts_with(b"copy-"));
        std::fs::create_dir(dir.join("docs")).unwrap();
        std::fs::write(dir.join("docs/a.txt"), "a").unwrap();
        // A hold opens the folder again, as a put in another process does.
        let reader = Scratch::hold_in(&base, &dir.join("docs/a.txt")).expect("held");
        assert_eq!(reader.path(), dir);
        let clone = owner.clone();
        drop(owner);
        drop(clone);
        assert!(dir.join("docs/a.txt").exists(), "the reader still holds it");
        drop(reader);
        assert!(
            !dir.exists(),
            "the last holder's drop takes it away at once"
        );
        until_gone(&base.join(format!("{}.gone", dir.file_name().unwrap().display())));
        assert!(
            Scratch::hold_in(&base, &dir.join("docs")).is_none(),
            "a folder that went is not held again"
        );
    }

    #[test]
    fn a_handed_over_scratch_folder_outlives_its_holders_and_goes_with_the_day_old_sweep() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("archives");
        let owner = Scratch::new_in(&base, 3600).unwrap();
        let dir = owner.path().to_path_buf();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let reader = Scratch::hold_in(&base, &dir.join("a.txt")).expect("held");
        let day_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(7200);
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(day_ago)
            .unwrap();
        owner.hand_over();
        drop(reader);
        assert!(dir.join("a.txt").exists(), "no holder removes it");
        sweep_scratch(&base, 3600).unwrap();
        assert!(
            dir.exists(),
            "its age counts from the hand-over, not its last copy"
        );
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(hour_ago)
            .unwrap();
        sweep_scratch(&base, 3600).unwrap();
        assert!(dir.exists(), "nor does the sweep of folders nobody holds");
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(day_ago)
            .unwrap();
        sweep_scratch(&base, 3600).unwrap();
        assert!(!dir.exists(), "the age sweep takes it");
    }

    #[test]
    fn only_a_held_scratch_folder_is_held_and_a_sweep_leaves_held_folders() {
        let t = tempfile::tempdir().unwrap();
        let base = t.path().join("archives");
        let opened = scratch_in(&base, 3600).unwrap();
        assert!(
            Scratch::hold_in(&base, &opened.join("f")).is_none(),
            "a file opened in a program is no held copy"
        );
        assert!(Scratch::hold_in(&base, t.path()).is_none());
        let held = Scratch::new_in(&base, 3600).unwrap();
        let dir = held.path().to_path_buf();
        let crashed = make_temp(&base, "copy-XXXXXX").unwrap();
        let fresh = make_temp(&base, "copy-XXXXXX").unwrap();
        let removing = make_temp(&base, "copy-XXXXXX").unwrap();
        let left = PathBuf::from(format!("{}.gone", removing.display()));
        std::fs::rename(&removing, &left).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(120);
        for d in [&dir, &crashed] {
            std::fs::File::open(d).unwrap().set_modified(old).unwrap();
        }
        sweep_scratch(&base, 3600).unwrap();
        assert!(dir.exists(), "a held folder stays");
        assert!(!crashed.exists(), "one whose holders died goes");
        assert!(fresh.exists(), "one being made is left to be locked");
        assert!(!left.exists(), "one a removal left goes at once");
    }

    #[test]
    fn a_copy_out_plans_one_step_per_name_and_refuses_a_path_too_long_up_front() {
        let chain = |first: char, depth: usize| {
            let mut p = String::from(first);
            p.push_str(&"/a".repeat(depth));
            p
        };
        let firsts: Vec<char> = ('b'..='z').collect();
        let entries: Vec<Entry> = (firsts.iter())
            .map(|&c| entry(&chain(c, 1000), Kind::File, 1))
            .collect();
        let picked: Vec<PathBuf> = firsts
            .iter()
            .map(|c| PathBuf::from(c.to_string()))
            .collect();
        let plan = tree_plan(&entries, &picked, 4000, &Meter::default()).expect("planned");
        assert_eq!(
            plan.nodes.len(),
            firsts.len() * 1001 + 1,
            "each step of each chain once, and the folder itself"
        );
        // 64 KB names, where a plan holding each step's whole path would take 1 GB per entry.
        let long: Vec<Entry> = (firsts.iter())
            .map(|&c| entry(&chain(c, 32_000), Kind::File, 1))
            .collect();
        let into = tempfile::tempdir().unwrap();
        let started = std::time::Instant::now();
        let refused = copy_tree_out(
            Path::new("/nowhere.tar"),
            &long,
            &picked,
            into.path(),
            None,
            &Meter::default(),
        );
        assert_eq!(refused, Err("it holds a path too long to copy out.".into()));
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        assert_eq!(std::fs::read_dir(into.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_name_first_listed_as_a_file_copies_out_as_that_file_as_the_folder_shows_it() {
        let t = tempfile::tempdir().unwrap();
        let (one, two) = (t.path().join("one"), t.path().join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(two.join("docs")).unwrap();
        std::fs::write(one.join("docs"), "a file").unwrap();
        std::fs::write(two.join("docs/x"), "below").unwrap();
        let tar = t.path().join("t.tar");
        bsdtar(&one, &["-cf", tar.to_str().unwrap(), "docs"]);
        bsdtar(&two, &["-rf", tar.to_str().unwrap(), "docs/x"]);
        let entries = listed(&tar);
        let shown = children(&entries, Path::new(""));
        assert_eq!(shown[0].kind, Kind::File, "{shown:?}");
        let into = t.path().join("into");
        std::fs::create_dir(&into).unwrap();
        let picked = [PathBuf::from("docs")];
        copy_tree_out(&tar, &entries, &picked, &into, None, &Meter::default()).expect("copied");
        assert_eq!(
            std::fs::read_to_string(into.join("docs")).unwrap(),
            "a file"
        );
    }

    #[test]
    fn a_copy_out_going_on_to_a_folder_on_its_own_file_system_must_fit_there_twice() {
        let t = tempfile::tempdir().unwrap();
        let archive = pack(t.path());
        let (into, dest) = (t.path().join("into"), t.path().join("dest"));
        std::fs::create_dir(&into).unwrap();
        std::fs::create_dir(&dest).unwrap();
        let share = extract::Limits::of(&into).bytes;
        let sized = [entry("top.txt", Kind::File, share / 10 * 6)];
        let picked = [PathBuf::from("top.txt")];
        let m = Meter::default();
        assert_eq!(
            copy_tree_out(&archive, &sized, &picked, &into, None, &m),
            Err("it holds less than its listing says.".into()),
            "alone it fits, and bsdtar runs"
        );
        let twice = copy_tree_out(&archive, &sized, &picked, &into, Some(&dest), &m);
        assert!(twice.unwrap_err().starts_with("it takes "), "refused unrun");
    }
}
