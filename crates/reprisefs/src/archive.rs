//! An archive browsed as a read-only folder. `bsdtar -tv` lists its entries from their headers,
//! and `bsdtar -xqO` writes one file at a time to its stdout, which Reprise copies into a fresh
//! private folder to open: bsdtar itself writes nothing to disk. Both run in the C.UTF-8 locale
//! (names and dates read back exactly) under CPU and memory limits, and die with the thread that
//! started them. docs/security.md "Extracting archives" has the rules.

use crate::extract::{self, Failure, MAX_ENTRIES, STDERR_TAIL};
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

/// Writes the first file named `entry` to stdout. bsdtar reads the name as a pattern, so every
/// byte but `/` is escaped, the closing `$` stops it matching what lies below the name, and the
/// leading `./` keeps a name like `-C` from reading as an option.
pub fn open_argv(archive: &Path, entry: &Path) -> Vec<OsString> {
    let mut pattern = b"./".to_vec();
    for &b in entry.as_os_str().as_bytes() {
        if b != b'/' {
            pattern.push(b'\\');
        }
        pattern.push(b);
    }
    pattern.push(b'$');
    ["bsdtar", "-xqO", "-f"]
        .map(OsString::from)
        .into_iter()
        .chain([archive.into(), OsString::from_vec(pattern)])
        .collect()
}

/// The CPU seconds a child reading `bytes` of archive (and member) may take.
fn cpu_secs(bytes: u64) -> u64 {
    BASE_CPU_SECS.saturating_add(bytes / CPU_BYTES_PER_SEC)
}

fn archive_bytes(archive: &Path) -> u64 {
    std::fs::metadata(archive).map_or(0, |m| m.len())
}

/// Whether bsdtar was stopped for running over its CPU limit (SIGXCPU, then SIGKILL at the hard
/// limit), which a box words as taking too long.
fn ran_out(status: &std::process::ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    matches!(status.signal(), Some(libc::SIGXCPU | libc::SIGKILL))
}

/// Starts `argv` with its output piped, in the C.UTF-8 locale and under `cpu_secs` of CPU and
/// [`CHILD_MEMORY`], with no core file; it is killed when the thread that started it ends.
fn spawn(argv: &[OsString], cpu_secs: u64) -> Result<std::process::Child, Failure> {
    use std::os::unix::process::CommandExt;
    extract::spawn_with(argv, true, |cmd| {
        cmd.env_remove("LANGUAGE").env("LC_ALL", "C.UTF-8");
        // SAFETY: the hook only makes async-signal-safe calls (setrlimit, prctl) in the child.
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
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
    })
}

/// Every file and folder `archive` holds, read from its headers. An archive of more than
/// [`MAX_ENTRIES`] entries is refused as soon as the listing passes them; cancelling `m` stops it.
pub fn list(archive: &Path, m: &Meter) -> Result<Vec<Entry>, String> {
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
    let read = extract::lines(&mut child, out, m, |line| {
        seen += 1;
        if seen > MAX_ENTRIES {
            return false;
        }
        entries.extend(listed_entry(line, now));
        true
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
        "header is encrypted",
        "content is encrypted",
    ];
    let text = String::from_utf8_lossy(stderr);
    SAID.iter().any(|s| text.contains(s))
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
    let mut f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len();
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
fn unescape(name: &[u8]) -> Vec<u8> {
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
        let stamp = Stamp::of(archive).ok_or_else(|| tr!("archive-missing"))?;
        let meter = Arc::new(Meter::default());
        {
            let mut inner = self.0.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((_, _, listed)) =
                (inner.listed.iter()).find(|(p, s, _)| p == archive && *s == stamp)
            {
                return listed.clone();
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
        if !meter.cancelled() {
            inner.listed.retain(|(p, ..)| p != archive);
            if inner.listed.len() >= KEPT_LISTINGS {
                inner.listed.pop_front();
            }
            (inner.listed).push_back((archive.to_path_buf(), stamp, listed.clone()));
        }
        listed
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
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(base)?;
    let now = std::time::SystemTime::now();
    for e in std::fs::read_dir(base)?.flatten() {
        let old = (e.metadata().and_then(|m| m.modified()).ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age.as_secs() > keep_secs);
        if old && e.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
    let mut template = base.join("XXXXXX").into_os_string().into_vec();
    template.push(0);
    // SAFETY: `template` is a writable NUL-terminated buffer that mkdtemp fills in place.
    let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
    if made.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    template.pop();
    Ok(PathBuf::from(OsString::from_vec(template)))
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
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&to)
        .map_err(|e| e.to_string())?;
    let failed = |why: String| {
        let _ = std::fs::remove_file(&to);
        why
    };
    let tool = |f: Failure| failed(f.why());
    let budget = cpu_secs(archive_bytes(archive).saturating_add(entry.size));
    let mut child = spawn(&open_argv(archive, &entry.path), budget).map_err(tool)?;
    let mut out = child
        .stdout
        .take()
        .ok_or_else(|| tool(extract::no_pipe()))?;
    let mut err = child
        .stderr
        .take()
        .ok_or_else(|| tool(extract::no_pipe()))?;
    let drain = std::thread::spawn(move || extract::tail(&mut err, STDERR_TAIL));
    let mut written = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    let copied = loop {
        let n = match out.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e.to_string()),
        };
        written += n as u64;
        if written > entry.size {
            break Err(tr!("archive-more-than-listed"));
        }
        if let Err(e) = file.write_all(&buf[..n]) {
            break Err(e.to_string());
        }
    };
    if copied.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    let stderr = drain.join().unwrap_or_default();
    if let Ok(s) = &status
        && ran_out(s)
    {
        return Err(failed(tr!("archive-too-long")));
    }
    if status.as_ref().is_ok_and(|s| !s.success()) && locked(&stderr) {
        return Err(failed(tr!("archive-locked")));
    }
    copied.map_err(failed)?;
    if written != entry.size {
        return Err(failed(tr!("archive-other-size")));
    }
    match status {
        Ok(s) if s.success() => {}
        Ok(_) => return Err(failed(extract::said(&stderr, &tr!("archive-unread")))),
        Err(e) => return Err(failed(e.to_string())),
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o400))
        .map_err(|e| failed(e.to_string()))?;
    Ok(to)
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
    }
}
