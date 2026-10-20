//! File search by name, for the launcher's file rows and the Glass Start
//! menu. An external tool does the work: `plocate` when its database exists
//! and the folder is not one `updatedb` prunes, then `fd` when plocate found
//! nothing (`updatedb` also skips what it takes for a bind mount, which a
//! btrfs `/home` subvolume is). Both match the text as a literal substring
//! of the file name, ignoring case unless the [`Filter`] asks. With neither
//! installed a search finds nothing.
//!
//! A [`Filter`] narrows hits by a date range, size, a phrase in the file,
//! hidden entries and the system's trees. Dates and sizes cost one `stat` per
//! hit, only when named. A phrase ([`holds`]) reads only regular files, only
//! their first [`CONTENT_CAP`] bytes, and skips binaries (a NUL in the first
//! block).
//!
//! [`run`] blocks, so call it on a worker thread.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const PLOCATE_DB: &str = "/var/lib/plocate/plocate.db";
const UPDATEDB_CONF: &str = "/etc/updatedb.conf";
/// Longest wait for output before the budget and cancel flag are rechecked.
const SLICE_MS: i32 = 20;
/// How much of a file a phrase search reads.
pub const CONTENT_CAP: u64 = 16 << 20;
const BLOCK: usize = 64 << 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Engine {
    Plocate(PathBuf),
    /// `fd`, or `fdfind` where the distribution renames it.
    Fd(PathBuf),
}

/// Trees a [`Filter`] with `system` off leaves out.
pub const SYSTEM_DIRS: [&str; 14] = [
    "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/opt", "/proc", "/run", "/sbin", "/srv",
    "/sys", "/usr", "/var",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    Modified,
    Created,
    Accessed,
}

/// What a hit must be beyond its name. The default is a plain name search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// The `Stamp` time lies in `from..=to` (epoch seconds); files lacking it are left out.
    pub time: Option<(Stamp, i64, i64)>,
    /// Size in `min..=max` bytes; folders are left out.
    pub size: Option<(u64, u64)>,
    pub hidden: bool,
    /// Match the name and `content` case-sensitively.
    pub case_sensitive: bool,
    /// A phrase the file holds ([`holds`]); empty for any file. Folders never hold one.
    pub content: String,
    /// Off leaves out [`SYSTEM_DIRS`] unless the search starts inside one.
    pub system: bool,
}

impl Default for Filter {
    fn default() -> Filter {
        Filter {
            time: None,
            size: None,
            hidden: false,
            case_sensitive: false,
            content: String::new(),
            system: true,
        }
    }
}

impl Filter {
    /// Whether an entry passes the dates and sizes; `times` are modified, created, accessed.
    pub fn admits(&self, is_dir: bool, size: u64, times: [Option<i64>; 3]) -> bool {
        if let Some((min, max)) = self.size
            && (is_dir || size < min || size > max)
        {
            return false;
        }
        if let Some((stamp, from, to)) = self.time {
            let t = match stamp {
                Stamp::Modified => times[0],
                Stamp::Created => times[1],
                Stamp::Accessed => times[2],
            };
            if !t.is_some_and(|t| (from..=to).contains(&t)) {
                return false;
            }
        }
        true
    }

    fn reads_meta(&self) -> bool {
        self.time.is_some() || self.size.is_some()
    }

    /// Whether the filter narrows beyond the name, so an empty name still searches.
    pub fn narrows(&self) -> bool {
        self.reads_meta() || !self.content.is_empty()
    }
}

/// What to look for and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    /// Literal substring of the file name; empty matches every name.
    pub text: String,
    pub root: PathBuf,
    /// Levels below `root` a hit may sit; `None` for any.
    pub depth: Option<usize>,
    pub limit: usize,
    pub budget: Duration,
    pub filter: Filter,
}

impl Query {
    /// The launcher's search: home, six levels deep, 300 ms. `limit` leaves the ranking spare candidates.
    pub fn launcher(text: &str, home: &Path) -> Query {
        Query {
            text: text.to_string(),
            root: home.to_path_buf(),
            depth: Some(6),
            limit: 200,
            budget: Duration::from_millis(300),
            filter: Filter::default(),
        }
    }

    pub fn folder(text: &str, root: &Path) -> Query {
        Query {
            text: text.to_string(),
            root: root.to_path_buf(),
            depth: None,
            limit: 2000,
            budget: Duration::from_secs(10),
            filter: Filter::default(),
        }
    }
}

pub(crate) fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
}

/// The folders a `PRUNEPATHS` line in `updatedb.conf` names; plocate knows nothing under them.
pub fn prune_paths(conf: &str) -> Vec<PathBuf> {
    conf.lines()
        .filter_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "PRUNEPATHS").then_some(value)
        })
        .flat_map(|v| v.trim().trim_matches('"').split_whitespace())
        .map(PathBuf::from)
        .collect()
}

fn covered(root: &Path, pruned: &[PathBuf]) -> bool {
    !pruned.iter().any(|p| root.starts_with(p))
}

/// The tools to try for `root`, in order: plocate if its database may cover `root`, then fd.
pub fn detect(root: &Path) -> Vec<Engine> {
    let mut engines = Vec::new();
    if let Some(p) = which("plocate")
        && Path::new(PLOCATE_DB).exists()
    {
        let conf = std::fs::read_to_string(UPDATEDB_CONF).unwrap_or_default();
        if covered(root, &prune_paths(&conf)) {
            engines.push(Engine::Plocate(p));
        }
    }
    engines.extend(which("fd").or_else(|| which("fdfind")).map(Engine::Fd));
    engines
}

/// The tool's argument vector, program first. plocate gets two patterns a
/// whole path must hold (the folder with a trailing slash, and the text unless
/// empty); [`keep`] then holds the text to the name and the folder to the
/// path's start. fd walks the folder itself.
pub fn argv(engine: &Engine, q: &Query) -> Vec<OsString> {
    let case = if q.filter.case_sensitive {
        "--case-sensitive"
    } else {
        "--ignore-case"
    };
    match engine {
        Engine::Plocate(bin) => {
            let mut folder = q.root.as_os_str().to_owned();
            if !folder.as_encoded_bytes().ends_with(b"/") {
                folder.push("/");
            }
            let mut v: Vec<OsString> = vec![bin.into()];
            if !q.filter.case_sensitive {
                v.push(case.into());
            }
            v.push("--".into());
            v.push(folder);
            if !q.text.is_empty() {
                v.push(q.text.as_str().into());
            }
            v
        }
        Engine::Fd(bin) => {
            let mut v: Vec<OsString> = vec![
                bin.into(),
                "--fixed-strings".into(),
                case.into(),
                "--color".into(),
                "never".into(),
                "--absolute-path".into(),
            ];
            if q.filter.hidden {
                v.push("--hidden".into());
            }
            if let Some(d) = q.depth {
                v.push("--max-depth".into());
                v.push(d.to_string().into());
            }
            v.push("--".into());
            v.push(q.text.as_str().into());
            v.push(q.root.as_os_str().to_owned());
            v
        }
    }
}

/// Whether an output line belongs in the answer: the name holds the text, the
/// path is below the root within the depth, and hidden components and system
/// trees pass the filter. plocate matches whole paths, so this is needed.
pub fn keep(q: &Query, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(&q.root) else {
        return false;
    };
    let named = path.file_name().is_some_and(|n| {
        let n = n.to_string_lossy();
        if q.filter.case_sensitive {
            n.contains(&q.text)
        } else {
            n.to_lowercase().contains(&q.text.to_lowercase())
        }
    });
    if !named {
        return false;
    }
    if !q.filter.system && in_system(path) && !in_system(&q.root) {
        return false;
    }
    let mut depth = 0;
    for c in rel.components() {
        depth += 1;
        if !q.filter.hidden && c.as_os_str().as_encoded_bytes().starts_with(b".") {
            return false;
        }
    }
    depth > 0 && q.depth.is_none_or(|d| depth <= d)
}

/// Whether a `.hidden` between `root` and `path` lists a step down, `path`'s name included. Lists are cached in `lists`.
fn listed_below(
    root: &Path,
    path: &Path,
    lists: &mut std::collections::HashMap<PathBuf, crate::hidden::Listed>,
) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    let mut dir = root.to_path_buf();
    for c in rel.components() {
        let listed = lists
            .entry(dir.clone())
            .or_insert_with(|| crate::hidden::Listed::read(&dir));
        if listed.contains(c.as_os_str()) {
            return true;
        }
        dir.push(c);
    }
    false
}

fn in_system(path: &Path) -> bool {
    SYSTEM_DIRS.iter().any(|d| path.starts_with(d))
}

/// Whether `path` passes the dates and sizes (links followed); unreadable entries fail.
fn keep_meta(f: &Filter, path: &Path) -> bool {
    if !f.reads_meta() {
        return true;
    }
    let Ok(m) = std::fs::metadata(path) else {
        return false;
    };
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
    };
    f.admits(
        m.is_dir(),
        m.len(),
        [secs(m.modified()), secs(m.created()), secs(m.accessed())],
    )
}

/// Whether `path` holds the filter's phrase (always, when none). Only regular
/// files are read; the open uses O_NONBLOCK and O_NOCTTY so a FIFO or
/// terminal cannot hang it.
fn keep_content(f: &Filter, path: &Path, cancel: &AtomicBool) -> bool {
    use std::os::unix::fs::OpenOptionsExt;
    if f.content.is_empty() {
        return true;
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
    else {
        return false;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return false;
    }
    holds(file.take(CONTENT_CAP), &f.content, f.case_sensitive, cancel)
}

/// Whether `reader` holds `phrase`, found across block boundaries. Case is
/// folded with Unicode lowercase unless `case_sensitive`; invalid UTF-8
/// matches no letter. A NUL in the first block marks a binary, which holds
/// nothing, as does a read cancelled mid-way.
pub fn holds(
    mut reader: impl Read,
    phrase: &str,
    case_sensitive: bool,
    cancel: &AtomicBool,
) -> bool {
    let fold = |s: &str| {
        if case_sensitive {
            s.to_string()
        } else {
            s.to_lowercase()
        }
    };
    let needle = fold(phrase);
    if needle.is_empty() {
        return true;
    }
    // `raw`: a character cut by the block end; `carry`: folded text a match could start in.
    let mut raw: Vec<u8> = Vec::new();
    let mut carry = String::new();
    let mut buf = vec![0u8; BLOCK];
    let mut first = true;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let n = match reader.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        if std::mem::take(&mut first) && buf[..n].contains(&0) {
            return false;
        }
        raw.extend_from_slice(&buf[..n]);
        let whole = match std::str::from_utf8(&raw) {
            Ok(_) => raw.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(_) => raw.len(),
        };
        let rest = raw.split_off(whole);
        carry.push_str(&fold(&String::from_utf8_lossy(&raw)));
        raw = rest;
        if carry.contains(&needle) {
            return true;
        }
        // A later match starts in the last needle.len() - 1 bytes.
        let mut keep = carry.len().saturating_sub(needle.len() - 1);
        while !carry.is_char_boundary(keep) {
            keep -= 1;
        }
        carry.drain(..keep);
    }
}

/// Runs the search and returns the paths that pass [`keep`] and the filter,
/// in the tool's order, less those a `.hidden` lists unless the filter wants
/// hidden entries. Stops at `q.limit`, `q.budget` or `cancel`, killing the
/// tool. A tool that finds nothing hands the remaining budget to the next
/// [`detect`] lists. Empty text with nothing to narrow it finds nothing.
pub fn run(q: &Query, cancel: &AtomicBool) -> Vec<PathBuf> {
    run_where(q, cancel, |_| true)
}

/// [`run`] with an extra `admit` test (such as file type) applied before paths count toward `q.limit`.
pub fn run_where(
    q: &Query,
    cancel: &AtomicBool,
    admit: impl Fn(&Path) -> bool + Send + Sync + 'static,
) -> Vec<PathBuf> {
    let admit: Admit = Arc::new(admit);
    if q.text.is_empty() && !q.filter.narrows() {
        return Vec::new();
    }
    let started = Instant::now();
    for engine in detect(&q.root) {
        let budget = q.budget.saturating_sub(started.elapsed());
        if budget.is_zero() || cancel.load(Ordering::Relaxed) {
            break;
        }
        let argv = argv(&engine, q);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        let found = collect_where(
            cmd,
            &Query {
                budget,
                ..q.clone()
            },
            cancel,
            admit.clone(),
        );
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// [`run`]'s reading half, for any command printing one path per line.
pub fn collect(cmd: Command, q: &Query, cancel: &AtomicBool) -> Vec<PathBuf> {
    collect_where(cmd, q, cancel, Arc::new(|_| true))
}

type Admit = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

fn collect_where(cmd: Command, q: &Query, cancel: &AtomicBool, admit: Admit) -> Vec<PathBuf> {
    let (filter, root) = (q.filter.clone(), q.root.clone());
    let mut lists = std::collections::HashMap::new();
    collect_with(cmd, q, cancel, move |path, stop| {
        (filter.hidden || !listed_below(&root, path, &mut lists))
            && keep_meta(&filter, path)
            && admit(path)
            && keep_content(&filter, path, stop)
    })
}

/// [`collect`] with `check` deciding each path [`keep`] passes. Checks run in
/// order on their own thread, so one blocked in the file system (a stalled
/// network mount) cannot hold the search: the budget and `cancel` still end it
/// with what passed. `check`'s flag goes up when the search is over; a check
/// still blocked then finishes alone and its answer is dropped.
fn collect_with(
    mut cmd: Command,
    q: &Query,
    cancel: &AtomicBool,
    mut check: impl FnMut(&Path, &AtomicBool) -> bool + Send + 'static,
) -> Vec<PathBuf> {
    let started = Instant::now();
    let child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            log::warn!("search: cannot start {:?}: {e}", cmd.get_program());
            return Vec::new();
        }
    };
    let Some(mut out) = child.stdout.take() else {
        return Vec::new();
    };
    let fd = out.as_raw_fd();
    let over = Arc::new(AtomicBool::new(false));
    let (to_check, checking) = mpsc::channel::<PathBuf>();
    let (verdict, verdicts) = mpsc::channel::<(PathBuf, bool)>();
    let stop = over.clone();
    let spawned = std::thread::Builder::new()
        .name("search-check".into())
        .spawn(move || {
            for path in checking {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let ok = check(&path, &stop);
                if verdict.send((path, ok)).is_err() {
                    return;
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("search: cannot start a thread: {e}");
        let _ = child.kill();
        let _ = child.wait();
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut waiting = 0usize;
    // True once the limit is reached.
    let take = |found: &mut Vec<PathBuf>, waiting: &mut usize, (path, ok): (PathBuf, bool)| {
        *waiting -= 1;
        if ok {
            found.push(path);
        }
        found.len() >= q.limit
    };
    let mut open = true;
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    'read: loop {
        // Read the end conditions before draining, so a verdict sent before the
        // cancel or the budget's end is kept.
        let cancelled = cancel.load(Ordering::Acquire);
        let left = q.budget.saturating_sub(started.elapsed());
        while let Ok(v) = verdicts.try_recv() {
            if take(&mut found, &mut waiting, v) {
                break 'read;
            }
        }
        if cancelled || left.is_zero() {
            break;
        }
        if !open {
            if waiting == 0 {
                break;
            }
            match verdicts.recv_timeout(left.min(Duration::from_millis(SLICE_MS as u64))) {
                Ok(v) => {
                    if take(&mut found, &mut waiting, v) {
                        break;
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = (left.as_millis() as i32).clamp(1, SLICE_MS);
        // SAFETY: `pfd` is one valid pollfd for the duration of the call.
        let ready = unsafe { libc::poll(&mut pfd, 1, wait) };
        if ready == 0 {
            continue;
        }
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        let n = match out.read(&mut buf) {
            Ok(0) => {
                open = false;
                continue;
            }
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                open = false;
                continue;
            }
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(nl) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=nl).collect();
            let path = PathBuf::from(OsStr::from_bytes(&line[..nl]));
            if keep(q, &path) && to_check.send(path).is_ok() {
                waiting += 1;
            }
        }
    }
    over.store(true, Ordering::Relaxed);
    abandon(child);
    found
}

/// Kills `child` and reaps it on a thread of its own. A tool with a thread blocked on a stopped
/// network mount stays a zombie past SIGKILL until the mount answers, so waiting here would keep
/// the search's answer with it.
fn abandon(mut child: std::process::Child) {
    let _ = child.kill();
    if let Ok(Some(_)) = child.try_wait() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("search-reap".into())
        .spawn(move || child.wait());
    if let Err(e) = spawned {
        log::warn!("search: cannot start a thread to reap the tool: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(root: &str, depth: Option<usize>) -> Query {
        Query {
            text: "note".into(),
            root: root.into(),
            depth,
            limit: 100,
            budget: Duration::from_secs(5),
            filter: Filter::default(),
        }
    }

    fn sh(script: &str) -> Command {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        c
    }

    #[test]
    fn fd_walks_the_root_with_a_literal_case_blind_name_match() {
        let v = argv(&Engine::Fd("/usr/bin/fd".into()), &q("/home/u", Some(6)));
        assert_eq!(
            v,
            [
                "/usr/bin/fd",
                "--fixed-strings",
                "--ignore-case",
                "--color",
                "never",
                "--absolute-path",
                "--max-depth",
                "6",
                "--",
                "note",
                "/home/u"
            ]
            .map(OsString::from)
        );
        let v = argv(&Engine::Fd("fd".into()), &q("/", None));
        assert!(!v.iter().any(|a| a == "--max-depth"), "no depth, no cap");
    }

    #[test]
    fn plocate_asks_for_the_folder_and_the_text_in_one_path() {
        let v = argv(
            &Engine::Plocate("/usr/bin/plocate".into()),
            &q("/home/u", None),
        );
        assert_eq!(
            v,
            [
                "/usr/bin/plocate",
                "--ignore-case",
                "--",
                "/home/u/",
                "note"
            ]
            .map(OsString::from)
        );
        let v = argv(&Engine::Plocate("plocate".into()), &q("/", None));
        assert_eq!(v[3], "/", "the root keeps its one slash");
    }

    #[test]
    fn a_path_under_a_listed_name_is_hidden() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        std::fs::create_dir_all(root.join("a/secret")).unwrap();
        std::fs::write(root.join(".hidden"), "b\n").unwrap();
        std::fs::write(root.join("a/.hidden"), "secret\nnote\n").unwrap();
        let mut lists = std::collections::HashMap::new();
        let mut hid = |p: &str| listed_below(root, &root.join(p), &mut lists);
        assert!(hid("b"));
        assert!(hid("b/x.txt"));
        assert!(hid("a/secret/x.txt"));
        assert!(hid("a/note"));
        assert!(!hid("a/x.txt"));
        assert!(!hid("c/b"), "a list speaks for its own folder only");
        assert_eq!(lists.len(), 3, "each folder's list is read once");
    }

    #[test]
    fn a_listed_name_is_found_when_hidden_entries_are_asked_for() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        std::fs::write(root.join(".hidden"), "note-a\n").unwrap();
        let mut query = q(&root.to_string_lossy(), None);
        let lines = format!(
            "printf '%s\\n%s\\n' {} {}",
            root.join("note-a").display(),
            root.join("note-b").display()
        );
        let found = collect(sh(&lines), &query, &AtomicBool::new(false));
        assert_eq!(found, [root.join("note-b")]);
        query.filter.hidden = true;
        let found = collect(sh(&lines), &query, &AtomicBool::new(false));
        assert_eq!(found, [root.join("note-a"), root.join("note-b")]);
    }

    #[test]
    fn keep_takes_paths_under_the_root_within_depth_and_not_hidden() {
        let query = q("/home/u", Some(2));
        assert!(keep(&query, Path::new("/home/u/notes.txt")));
        assert!(keep(&query, Path::new("/home/u/Docs/notes.txt")));
        assert!(!keep(&query, Path::new("/home/u/a/b/notes.txt")), "depth 3");
        assert!(
            !keep(&query, Path::new("/home/u/.cache/notes")),
            "hidden folder"
        );
        assert!(!keep(&query, Path::new("/home/u/.notes")), "hidden file");
        assert!(!keep(&query, Path::new("/home/user2/notes")), "a sibling");
        assert!(!keep(&query, Path::new("/home/u")), "the root itself");
        assert!(
            !keep(&query, Path::new("/home/u/notes/a.txt")),
            "only the folder holds the text"
        );
        assert!(
            keep(&query, Path::new("/home/u/NOTES")),
            "case does not matter"
        );
        assert!(keep(
            &q("/home/u/.local", None),
            Path::new("/home/u/.local/note")
        ));
    }

    #[test]
    fn prune_paths_reads_the_quoted_list() {
        let conf = "PRUNE_BIND_MOUNTS = \"yes\"\nPRUNEPATHS = \"/afs /media /mnt /tmp\"\n";
        let p = prune_paths(conf);
        assert_eq!(p, ["/afs", "/media", "/mnt", "/tmp"].map(PathBuf::from));
        assert!(!covered(Path::new("/media/usb"), &p));
        assert!(covered(Path::new("/home/u"), &p));
        assert!(
            covered(Path::new("/mediakit"), &p),
            "a prefix of a name is not its parent"
        );
    }

    #[test]
    fn collect_reads_lines_and_filters_them() {
        let found = collect(
            sh("printf '/home/u/notes\\n/etc/notes\\n/home/u/.x/notes\\n/home/u/b/note'"),
            &q("/home/u", None),
            &AtomicBool::new(false),
        );
        // The last line has no newline.
        assert_eq!(found, [PathBuf::from("/home/u/notes")]);
    }

    #[test]
    fn collect_stops_at_the_limit() {
        let mut query = q("/r", None);
        query.limit = 2;
        let found = collect(
            sh("printf '/r/note-a\\n/r/note-b\\n/r/note-c\\n'; sleep 5"),
            &query,
            &AtomicBool::new(false),
        );
        assert_eq!(found, ["/r/note-a", "/r/note-b"].map(PathBuf::from));
    }

    #[test]
    fn collect_gives_up_at_the_budget_with_what_arrived() {
        let mut query = q("/r", None);
        query.budget = Duration::from_millis(150);
        let t = Instant::now();
        let found = collect(
            sh("printf '/r/note-a\\n'; sleep 5; printf '/r/note-b\\n'"),
            &query,
            &AtomicBool::new(false),
        );
        assert_eq!(found, [PathBuf::from("/r/note-a")]);
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "the tool is killed, not waited for"
        );
    }

    #[test]
    fn abandoning_a_tool_returns_at_once_and_reaps_it_later() {
        let child = sh("sleep 30").spawn().unwrap();
        let pid = child.id();
        let t = Instant::now();
        abandon(child);
        assert!(t.elapsed() < Duration::from_millis(500), "returned at once");
        let gone = (0..200).any(|_| {
            std::thread::sleep(Duration::from_millis(10));
            !Path::new(&format!("/proc/{pid}")).exists()
        });
        assert!(gone, "the tool is reaped, leaving no zombie");
    }

    #[test]
    fn collect_returns_at_once_when_cancelled() {
        let t = Instant::now();
        let found = collect(sh("sleep 5"), &q("/r", None), &AtomicBool::new(true));
        assert!(found.is_empty());
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_missing_tool_finds_nothing() {
        let found = collect(
            Command::new("/nonexistent/search-tool"),
            &q("/r", None),
            &AtomicBool::new(false),
        );
        assert!(found.is_empty());
    }

    #[test]
    fn the_filter_asks_fd_for_case_and_hidden_entries() {
        let mut query = q("/home/u", None);
        query.filter.case_sensitive = true;
        query.filter.hidden = true;
        let v = argv(&Engine::Fd("fd".into()), &query);
        assert!(v.iter().any(|a| a == "--case-sensitive"));
        assert!(!v.iter().any(|a| a == "--ignore-case"));
        assert!(v.iter().any(|a| a == "--hidden"));
        // Empty text drops the pattern, so the folder alone lists everything.
        query.text.clear();
        let v = argv(&Engine::Plocate("plocate".into()), &query);
        assert_eq!(v, ["plocate", "--", "/home/u/"].map(OsString::from));
    }

    #[test]
    fn keep_follows_the_filter() {
        let mut query = q("/home/u", None);
        query.filter.hidden = true;
        assert!(keep(&query, Path::new("/home/u/.cache/notes")));
        query.filter.case_sensitive = true;
        assert!(!keep(&query, Path::new("/home/u/NOTES")));
        assert!(keep(&query, Path::new("/home/u/notes")));

        let mut root = q("/", None);
        root.filter.system = false;
        assert!(!keep(&root, Path::new("/usr/share/notes")));
        assert!(keep(&root, Path::new("/home/u/notes")));
        let mut usr = q("/usr", None);
        usr.filter.system = false;
        assert!(keep(&usr, Path::new("/usr/share/notes")));
    }

    #[test]
    fn admits_reads_the_chosen_time_and_the_size() {
        let mut f = Filter {
            time: Some((Stamp::Created, 100, 200)),
            ..Filter::default()
        };
        assert!(f.admits(false, 0, [Some(500), Some(150), None]));
        assert!(!f.admits(false, 0, [Some(150), Some(500), None]));
        assert!(
            !f.admits(false, 0, [Some(150), None, None]),
            "no birth time"
        );
        f.time = None;
        f.size = Some((0, 100 * 1024 - 1));
        assert!(f.admits(false, 4096, [None; 3]));
        assert!(!f.admits(false, 100 * 1024, [None; 3]));
        assert!(!f.admits(true, 0, [None; 3]), "a folder has no size");
    }

    #[test]
    fn collect_keeps_the_files_that_hold_the_phrase() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        std::fs::write(root.join("note-a"), "Meeting at NOON on Friday").expect("write");
        std::fs::write(root.join("note-b"), "nothing here").expect("write");
        std::fs::create_dir(root.join("note-noon")).expect("mkdir");
        let mut query = q(&root.to_string_lossy(), None);
        query.filter.content = "noon".into();
        let script = format!(
            "printf '%s\\n%s\\n%s\\n' {} {} {}",
            root.join("note-a").display(),
            root.join("note-b").display(),
            root.join("note-noon").display()
        );
        let found = collect(sh(&script), &query, &AtomicBool::new(false));
        assert_eq!(found, [root.join("note-a")]);
        query.filter.case_sensitive = true;
        let found = collect(sh(&script), &query, &AtomicBool::new(false));
        assert!(found.is_empty(), "NOON is not noon with its case");
    }

    #[test]
    fn holds_finds_a_phrase_across_blocks_and_in_any_case() {
        let no = AtomicBool::new(false);
        let mut text = vec![b'x'; BLOCK - 3];
        text.extend_from_slice("Grüße, world".as_bytes());
        assert!(
            holds(&text[..], "GRÜßE, W", false, &no),
            "across the block's end"
        );
        assert!(holds(&text[..], "Grüße", true, &no));
        assert!(!holds(&text[..], "grüße", true, &no));
        assert!(!holds(&text[..], "worlds", false, &no));
        let mut cut = vec![b'x'; BLOCK - 1];
        cut.extend_from_slice("é".as_bytes());
        assert!(holds(&cut[..], "xé", false, &no));
    }

    #[test]
    fn a_nul_in_the_first_block_marks_a_binary() {
        let no = AtomicBool::new(false);
        let mut bin = b"ELF\0".to_vec();
        bin.extend_from_slice(b" noon");
        assert!(!holds(&bin[..], "noon", false, &no));
        let mut late = vec![b'x'; BLOCK];
        late.extend_from_slice(b"\0 noon");
        assert!(holds(&late[..], "noon", false, &no));
    }

    #[test]
    fn a_phrase_past_the_cap_is_not_found() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("big-note");
        let f = std::fs::File::create(&path).expect("create");
        f.set_len(CONTENT_CAP).expect("grow");
        drop(f);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open");
        std::io::Write::write_all(&mut f, b"noon").expect("write");
        // The sparse start is NULs; make the first block text.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, &vec![b'x'; BLOCK]))
            .expect("write");
        let filter = Filter {
            content: "noon".into(),
            ..Filter::default()
        };
        assert!(!keep_content(&filter, &path, &AtomicBool::new(false)));
        let filter = Filter {
            content: "xxx".into(),
            ..filter
        };
        assert!(keep_content(&filter, &path, &AtomicBool::new(false)));
    }

    #[test]
    fn a_cancelled_phrase_search_stops_reading() {
        /// Blocks of `x` with the phrase in the third; cancels on the second read.
        struct Slow<'a> {
            reads: usize,
            cancel: &'a AtomicBool,
        }
        impl Read for Slow<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                if self.reads == 2 {
                    self.cancel.store(true, Ordering::Relaxed);
                }
                buf.fill(b'x');
                if self.reads == 3 {
                    buf[..4].copy_from_slice(b"noon");
                }
                Ok(buf.len())
            }
        }
        let cancel = AtomicBool::new(false);
        let mut slow = Slow {
            reads: 0,
            cancel: &cancel,
        };
        assert!(!holds(&mut slow, "noon", false, &cancel));
        assert_eq!(slow.reads, 2, "nothing is read after the cancel");

        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path();
        std::fs::write(root.join("note-a"), "noon").expect("write");
        let mut query = q(&root.to_string_lossy(), None);
        query.filter.content = "noon".into();
        let script = format!("printf '%s\\n' {}; sleep 5", root.join("note-a").display());
        let t = Instant::now();
        let found = collect(sh(&script), &query, &AtomicBool::new(true));
        assert!(found.is_empty());
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_phrase_search_passes_over_a_fifo() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let root = dir.path().to_path_buf();
        crate::testfs::mkfifo(&root.join("note-pipe"));
        std::fs::write(root.join("note-a"), "noon").expect("write");
        let mut query = q(&root.to_string_lossy(), None);
        query.filter.content = "noon".into();
        let script = format!(
            "printf '%s\\n%s\\n' {} {}",
            root.join("note-pipe").display(),
            root.join("note-a").display()
        );
        let found =
            crate::testfs::within(move || collect(sh(&script), &query, &AtomicBool::new(false)));
        assert_eq!(found, [root.join("note-a")]);
    }

    #[test]
    fn a_cancel_ends_a_search_whose_check_blocks() {
        let cancel = Arc::new(AtomicBool::new(false));
        let (entered, blocked) = mpsc::channel();
        let (flag_tx, flag) = mpsc::channel();
        let check = move |path: &Path, stop: &AtomicBool| {
            if path.ends_with("note-b") {
                let _ = entered.send(());
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                let _ = flag_tx.send(());
            }
            true
        };
        let stopper = cancel.clone();
        std::thread::spawn(move || {
            let _ = blocked.recv();
            stopper.store(true, Ordering::Release);
        });
        let found = crate::testfs::within(move || {
            collect_with(
                sh("printf '/r/note-a\\n/r/note-b\\n/r/note-c\\n'; sleep 5"),
                &q("/r", None),
                &cancel,
                check,
            )
        });
        assert_eq!(found, [PathBuf::from("/r/note-a")]);
        flag.recv_timeout(Duration::from_secs(5))
            .expect("the blocked check is told the search is over");
    }

    #[test]
    fn a_phrase_narrows_an_empty_name() {
        let mut f = Filter::default();
        assert!(!f.narrows());
        f.content = "noon".into();
        assert!(f.narrows());
    }

    #[test]
    fn collect_drops_what_the_dates_leave_out() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let old = dir.path().join("note-old");
        let new = dir.path().join("note-new");
        std::fs::write(&old, "x").expect("write");
        std::fs::write(&new, "x").expect("write");
        let when = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .and_then(|f| f.set_modified(when))
            .expect("set the time");
        let mut query = q(&dir.path().to_string_lossy(), None);
        query.filter.time = Some((Stamp::Modified, 2_000_000, i64::MAX));
        let script = format!("printf '%s\\n%s\\n' {} {}", old.display(), new.display());
        let found = collect(sh(&script), &query, &AtomicBool::new(false));
        assert_eq!(found, [new]);
    }
}
