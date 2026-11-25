//! Pinned folders and Favorites: GTK's bookmarks file, `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, one
//! `URI[ label]` per line, so a bookmark shows in GTK sidebars and portal dialogs too. Places bars
//! and panes show its local folders, Explorer's Favorites every bookmark. [`Pins`] watches the
//! file and reads it off the caller's loop.

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use reprisebus::mutex::lock;

/// The folders a places bar or a pane lists at most; the file may hold more.
pub const MAX_SHOWN: usize = 20;

/// The bookmarks Favorites lists at most.
pub const MAX_FAVORITES: usize = 100;

/// The longest name, in characters, a pin or favorite shows; the line keeps the whole label.
pub const MAX_LABEL: usize = 256;

/// The longest bookmarks file read; GTK's own stays far below.
const MAX_BYTES: u64 = 1 << 20;

pub fn bookmarks_file() -> PathBuf {
    reprisexdg::config_home()
        .unwrap_or_default()
        .join("gtk-3.0/bookmarks")
}

/// A pinned folder as a places bar or a pane shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub path: PathBuf,
    /// The bookmark's label, else the folder's name.
    pub label: String,
    /// False for a folder that is not there (an unplugged drive's): shown greyed.
    pub present: bool,
}

/// What a bookmark points at.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// A local folder, from a `file://` URI with no host or `localhost`.
    Folder(PathBuf),
    /// Any other URI (`sftp://`, `smb://`, a `file://` on another host), as written.
    Uri(String),
}

impl Target {
    /// The URI a line for it carries, percent-encoded as GLib writes a file URI.
    pub fn uri(&self) -> String {
        match self {
            Target::Folder(path) => crate::thumbnails::file_uri(path),
            Target::Uri(uri) => uri.clone(),
        }
    }

    /// The name shown for a bookmark without a label: the folder's name, or the address with
    /// any password left out and its escapes decoded.
    pub fn default_name(&self) -> String {
        match self {
            Target::Folder(path) => crate::places::dialog_label(path),
            Target::Uri(uri) => {
                let shown = crate::netshare::Address::parse(uri)
                    .map_or_else(|| without_password(uri), |a| a.uri());
                String::from_utf8_lossy(&crate::trash::percent_decode(&shown)).into()
            }
        }
    }
}

/// `uri` with the `:password` of its `user:password@` cut out.
fn without_password(uri: &str) -> String {
    let Some((scheme, rest)) = uri.split_once("://") else {
        return uri.to_string();
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    match authority.rsplit_once('@') {
        Some((userinfo, host)) => {
            let user = userinfo.split_once(':').map_or(userinfo, |(user, _)| user);
            format!("{scheme}://{user}@{host}{path}")
        }
        None => uri.to_string(),
    }
}

/// The name a pin or favorite shows: its label, else the target's own name, cut at
/// [`MAX_LABEL`] characters.
fn shown_label(target: &Target, label: Option<&str>) -> String {
    let name = label.map_or_else(|| target.default_name(), str::to_string);
    match name.char_indices().nth(MAX_LABEL) {
        Some((cut, _)) => name[..cut].to_string(),
        None => name,
    }
}

/// A bookmark as Favorites shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favorite {
    pub target: Target,
    /// The bookmark's label, else [`Target::default_name`].
    pub label: String,
    /// False for a folder that is not there: shown greyed. True for any other URI.
    pub present: bool,
    /// The line it was read from, which an edit of it changes.
    pub line: LineKey,
}

/// A bookmark's line: its URI as written and how many lines before it carry the same, so two
/// spellings of one folder (`file:///a`, `file://localhost/a`) or two copies of a line stay apart.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineKey {
    uri: String,
    nth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    target: Target,
    /// The URI as the line spells it, written back as is.
    uri: String,
    label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Line {
    /// The bookmark on it; None for a blank line, a comment or text with no URI.
    entry: Option<Entry>,
    /// The line as read, without its ending, written back byte for byte; None once edited.
    raw: Option<Vec<u8>>,
    /// The line ends in CR LF rather than LF.
    crlf: bool,
}

impl Line {
    fn folder(&self) -> Option<&Path> {
        match &self.entry {
            Some(Entry {
                target: Target::Folder(path),
                ..
            }) => Some(path),
            _ => None,
        }
    }

    fn is(&self, want: &Target) -> bool {
        self.entry.as_ref().is_some_and(|e| e.target == *want)
    }
}

/// The bookmarks file's lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    lines: Vec<Line>,
}

impl Bookmarks {
    pub fn parse(bytes: impl AsRef<[u8]>) -> Bookmarks {
        let mut lines = Vec::new();
        let mut rest = bytes.as_ref();
        while !rest.is_empty() {
            let (line, next) = match rest.iter().position(|&b| b == b'\n') {
                Some(end) => (&rest[..end], &rest[end + 1..]),
                None => (rest, &[][..]),
            };
            let (line, crlf) = match line.strip_suffix(b"\r") {
                Some(line) => (line, true),
                None => (line, false),
            };
            lines.push(Line {
                entry: parse_line(line),
                raw: Some(line.to_vec()),
                crlf,
            });
            rest = next;
        }
        Bookmarks { lines }
    }

    /// The file's bytes: lines not edited exactly as read, each ending as it did (a last line
    /// with no ending gets a LF).
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for line in &self.lines {
            match (&line.raw, &line.entry) {
                (Some(raw), _) => out.extend_from_slice(raw),
                (None, Some(Entry { uri, label, .. })) => {
                    out.extend_from_slice(uri.as_bytes());
                    if let Some(label) = label {
                        out.push(b' ');
                        out.extend_from_slice(label.as_bytes());
                    }
                }
                (None, None) => {}
            }
            out.extend_from_slice(if line.crlf { b"\r\n" } else { b"\n" });
        }
        out
    }

    /// Every bookmark, in file order, with its label.
    pub fn entries(&self) -> impl Iterator<Item = (&Target, Option<&str>)> {
        self.lines
            .iter()
            .filter_map(|l| l.entry.as_ref())
            .map(|e| (&e.target, e.label.as_deref()))
    }

    /// Every bookmark, in file order, with the key of its line.
    fn keyed(&self) -> impl Iterator<Item = (LineKey, &Entry)> {
        let mut seen = std::collections::HashMap::<&str, usize>::new();
        self.lines
            .iter()
            .filter_map(|l| l.entry.as_ref())
            .map(move |e| {
                let nth = seen.entry(e.uri.as_str()).or_default();
                let key = LineKey {
                    uri: e.uri.clone(),
                    nth: *nth,
                };
                *nth += 1;
                (key, e)
            })
    }

    /// The index of `key`'s line.
    fn find(&self, key: &LineKey) -> Option<usize> {
        let mut seen = 0;
        self.lines.iter().position(|l| {
            let hit = l.entry.as_ref().is_some_and(|e| e.uri == key.uri);
            seen += usize::from(hit);
            hit && seen == key.nth + 1
        })
    }

    /// A new bookmark line, ending as the last line does.
    fn push(&mut self, target: Target, label: Option<String>) {
        let crlf = self.lines.last().is_some_and(|l| l.crlf);
        self.lines.push(Line {
            entry: Some(Entry {
                uri: target.uri(),
                target,
                label,
            }),
            raw: None,
            crlf,
        });
    }

    /// The local folders, in file order, with their labels.
    pub fn folders(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.entries().filter_map(|(t, label)| match t {
            Target::Folder(path) => Some((path.as_path(), label)),
            Target::Uri(_) => None,
        })
    }

    pub fn has(&self, dir: &Path) -> bool {
        self.position(dir).is_some()
    }

    fn position(&self, dir: &Path) -> Option<usize> {
        self.lines.iter().position(|l| l.folder() == Some(dir))
    }

    /// Pins `dir` last; false when it was pinned already.
    pub fn pin(&mut self, dir: &Path) -> bool {
        if self.has(dir) {
            return false;
        }
        self.push(Target::Folder(dir.to_path_buf()), None);
        true
    }

    /// Drops every line naming `dir`; false when none did.
    pub fn unpin(&mut self, dir: &Path) -> bool {
        let before = self.lines.len();
        self.lines.retain(|l| l.folder() != Some(dir));
        self.lines.len() != before
    }

    /// Moves `dir` past the folder before it (`up`) or after it, the lines of other schemes
    /// staying where they are among the rest; false at either end.
    pub fn shift(&mut self, dir: &Path, up: bool) -> bool {
        match self.position(dir) {
            Some(at) => self.swap_with(at, up, |l| l.folder().is_some()),
            None => false,
        }
    }

    /// Adds `dir` last named `name`, or names it `name` where it is already a bookmark; false
    /// when nothing changed. A name equal to the folder's own is left out of the line.
    pub fn add(&mut self, dir: &Path, name: &str) -> bool {
        let target = Target::Folder(dir.to_path_buf());
        if let Some(at) = self.lines.iter().position(|l| l.is(&target)) {
            return self.rename_at(at, name);
        }
        let label = label_for(&target, name);
        self.push(target, label);
        true
    }

    /// Names the bookmark on `key`'s line `name`, its URI kept as written; false when the line is
    /// gone or the name stays.
    pub fn rename(&mut self, key: &LineKey, name: &str) -> bool {
        self.find(key).is_some_and(|at| self.rename_at(at, name))
    }

    fn rename_at(&mut self, at: usize, name: &str) -> bool {
        let line = &mut self.lines[at];
        let Some(entry) = &mut line.entry else {
            return false;
        };
        let want = label_for(&entry.target, name);
        if entry.label == want {
            return false;
        }
        entry.label = want;
        line.raw = None;
        true
    }

    /// Drops `key`'s line; false when it is gone.
    pub fn remove(&mut self, key: &LineKey) -> bool {
        self.find(key).map(|at| self.lines.remove(at)).is_some()
    }

    /// Moves `key`'s line past the bookmark before it (`up`) or after it, the other lines staying
    /// where they are; false at either end.
    pub fn move_entry(&mut self, key: &LineKey, up: bool) -> bool {
        match self.find(key) {
            Some(at) => self.swap_with(at, up, |l| l.entry.is_some()),
            None => false,
        }
    }

    /// Swaps line `at` with the nearest line before (`up`) or after it that `counts`.
    fn swap_with(&mut self, at: usize, up: bool, counts: impl Fn(&Line) -> bool) -> bool {
        let other = if up {
            self.lines[..at].iter().rposition(&counts)
        } else {
            (self.lines[at + 1..].iter().position(&counts)).map(|i| at + 1 + i)
        };
        let Some(other) = other else {
            return false;
        };
        self.lines.swap(at, other);
        true
    }
}

/// The label a line keeps for `name`: `None` for a blank name or the target's own name, so GTK
/// goes on showing a renamed folder's new name. A line break or other control character in the
/// name, which would end or garble the line, becomes a space.
pub fn label_for(target: &Target, name: &str) -> Option<String> {
    let clean: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let clean = clean.trim();
    (!clean.is_empty() && clean != target.default_name()).then(|| clean.to_string())
}

/// A bookmark line's target and label; None for a blank line, a `#` comment or a line with no
/// URI (or one that is not UTF-8). The URI ends at the first space; the rest of the line is the
/// label, its bytes that are not UTF-8 shown as U+FFFD.
fn parse_line(line: &[u8]) -> Option<Entry> {
    if line.starts_with(b"#") {
        return None;
    }
    let (uri, label) = match line.iter().position(|&b| b == b' ') {
        Some(at) => (&line[..at], Some(&line[at + 1..]).filter(|l| !l.is_empty())),
        None => (line, None),
    };
    let uri = std::str::from_utf8(uri).ok()?;
    let label = label.map(|l| String::from_utf8_lossy(l).into_owned());
    let (scheme, _) = uri.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    if !valid {
        return None;
    }
    let local = (uri.strip_prefix("file://"))
        .map(|rest| rest.strip_prefix("localhost").unwrap_or(rest))
        .filter(|rest| rest.starts_with('/'));
    let target = match local {
        Some(rest) => Target::Folder(PathBuf::from(OsString::from_vec(
            crate::trash::percent_decode(rest),
        ))),
        None => Target::Uri(uri.to_string()),
    };
    Some(Entry {
        target,
        uri: uri.to_string(),
        label,
    })
}

/// The bookmarks file at `file`, empty when it is missing. Any other failure (too large, not a
/// regular file, unreadable) is an error, and a FIFO there is never opened.
fn read(file: &Path) -> io::Result<Bookmarks> {
    match crate::ops::read_regular(file, MAX_BYTES) {
        Ok(bytes) => Ok(Bookmarks::parse(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Bookmarks::default()),
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!("cannot read {}: {e}", file.display()),
        )),
    }
}

/// The bookmarks file at `file`; empty when it is missing or unreadable.
pub fn load(file: &Path) -> Bookmarks {
    read(file).unwrap_or_else(|e| {
        log::info!("bookmarks: {e}");
        Bookmarks::default()
    })
}

/// Reads `file`, lets `f` edit it and writes it back in one rename when `f` says it changed;
/// the bookmarks as they now stand. A file that cannot be read is an error and never written
/// over. GTK takes no lock either, so a line another program writes between the read and the
/// rename is lost.
pub fn edit_at(file: &Path, f: impl FnOnce(&mut Bookmarks) -> bool) -> io::Result<Bookmarks> {
    let mut b = read(file)?;
    if f(&mut b) {
        clear_stale_temps(file);
        crate::ops::write_atomic(file, &b.bytes())?;
    }
    Ok(b)
}

/// How old a temp file beside the bookmarks is before a write takes it for one a killed writer
/// left, rather than another writer's mid-write.
const STALE_TEMP: std::time::Duration = std::time::Duration::from_secs(60);

/// Removes the temp files of this user's writes to `file` (`.NAME.<16 hex>.tmp` beside where it
/// leads) older than [`STALE_TEMP`].
fn clear_stale_temps(file: &Path) {
    use std::os::unix::fs::MetadataExt;
    let Ok(real) = crate::ops::real_path(file) else {
        return;
    };
    let (Some(dir), Some(name)) = (real.parent(), real.file_name()) else {
        return;
    };
    let prefix = format!(".{}.", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    for entry in entries.flatten() {
        let entry_name = entry.file_name();
        let ours = (entry_name.to_str())
            .and_then(|n| n.strip_prefix(&prefix)?.strip_suffix(".tmp"))
            .is_some_and(|tag| tag.len() == 16 && tag.bytes().all(|b| b.is_ascii_hexdigit()));
        let stale = (entry.metadata().ok()).is_some_and(|m| {
            m.file_type().is_file()
                && m.uid() == uid
                && m.modified()
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age > STALE_TEMP)
        });
        if ours && stale {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether `path` is a folder, counting one on any of the `network` mounts as there: a look
/// there could hang on a dead server.
fn present(path: &Path, network: &std::collections::HashSet<PathBuf>) -> bool {
    network.iter().any(|m| path.starts_with(m)) || path.is_dir()
}

/// The shown pins of `b`, each folder looked for as [`present`] does.
pub fn shown(b: &Bookmarks, network: &std::collections::HashSet<PathBuf>) -> Vec<Pin> {
    b.folders()
        .take(MAX_SHOWN)
        .map(|(path, label)| Pin {
            path: path.to_path_buf(),
            label: shown_label(&Target::Folder(path.to_path_buf()), label),
            present: present(path, network),
        })
        .collect()
}

/// The bookmarks of `b` as Favorites lists them, each folder looked for as [`present`] does.
pub fn favorites(b: &Bookmarks, network: &std::collections::HashSet<PathBuf>) -> Vec<Favorite> {
    b.keyed()
        .take(MAX_FAVORITES)
        .map(|(line, e)| Favorite {
            target: e.target.clone(),
            label: shown_label(&e.target, e.label.as_deref()),
            present: match &e.target {
                Target::Folder(path) => present(path, network),
                Target::Uri(_) => true,
            },
            line,
        })
        .collect()
}

/// What to watch so `file` is seen changing, arriving or leaving: its folder for its name, or,
/// while that folder is missing, the nearest folder above that is there for the next name down.
/// A symlinked file (a dotfiles checkout) is watched where its link ends too.
fn watches(file: &Path) -> Vec<(PathBuf, Option<OsString>)> {
    let mut out: Vec<_> = nearest_watch(file).into_iter().collect();
    if std::fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_symlink())
        && let Ok(real) = crate::ops::real_path(file)
        && let Some(also) = nearest_watch(&real)
        && !out.contains(&also)
    {
        out.push(also);
    }
    out
}

fn nearest_watch(file: &Path) -> Option<(PathBuf, Option<OsString>)> {
    let mut child = file;
    while let Some(dir) = child.parent() {
        if dir.is_dir() {
            return Some((
                dir.to_path_buf(),
                child.file_name().map(OsStr::to_os_string),
            ));
        }
        child = dir;
    }
    None
}

#[derive(Default)]
struct Shared {
    pins: Arc<Vec<Pin>>,
    favorites: Arc<Vec<Favorite>>,
    generation: u64,
    /// Edits sent and not yet written.
    pending: usize,
    /// The last edit that could not be written, for [`Pins::take_failed`].
    failed: Option<io::Error>,
}

type Edit = Box<dyn FnOnce(&mut Bookmarks) -> bool + Send>;

/// What the thread is asked: read the file again, or edit it first.
enum Job {
    Read,
    Edit(Edit),
}

/// The pins and Favorites of the bookmarks file, kept current by a thread that watches it: the
/// thread reads and writes it and looks for each folder, so a dead mount or a slow sync stalls the
/// thread and not its owner's loop.
pub struct Pins {
    shared: Arc<Mutex<Shared>>,
    file: PathBuf,
    seen: u64,
    poke: mpsc::Sender<Job>,
    /// The thread ends once this drops, which [`Drop`] does before it wakes the thread.
    alive: Option<Arc<()>>,
}

impl Pins {
    /// Starts the thread on [`bookmarks_file`]; `wake` runs on it each time the pins change.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Pins {
        Pins::start_at(bookmarks_file(), wake)
    }

    pub fn start_at(file: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Pins {
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let (poke, pokes) = mpsc::channel();
        let alive = Arc::new(());
        let (out, gone) = (shared.clone(), Arc::downgrade(&alive));
        let from_watch = poke.clone();
        let read = file.clone();
        let spawned = std::thread::Builder::new()
            .name("pins".to_string())
            .spawn(move || {
                let watch = crate::folderwatch::Watch::new(move || {
                    let _ = from_watch.send(Job::Read);
                });
                let mut watch = watch
                    .map_err(|e| log::warn!("bookmarks: no watch: {e}"))
                    .ok();
                while let Ok(first) = pokes.recv() {
                    let edits: Vec<Edit> = std::iter::once(first)
                        .chain(pokes.try_iter())
                        .filter_map(|job| match job {
                            Job::Edit(f) => Some(f),
                            Job::Read => None,
                        })
                        .collect();
                    let edited = !edits.is_empty();
                    for f in edits {
                        let written = edit_at(&read, f);
                        let mut s = lock(&out);
                        s.pending -= 1;
                        if let Err(e) = written {
                            log::warn!("bookmarks: cannot write {}: {e}", read.display());
                            s.failed = Some(e);
                        }
                    }
                    if gone.strong_count() == 0 {
                        return;
                    }
                    // Re-armed on every read: a folder on the way renamed and made again keeps
                    // its path but not its inode.
                    if let Some(w) = &mut watch {
                        w.watch_all(&watches(&read));
                    }
                    let mounts = std::fs::read("/proc/self/mounts").unwrap_or_default();
                    let network = crate::places::network_mounts(mounts);
                    let b = load(&read);
                    let (pins, favorites) = (shown(&b, &network), favorites(&b, &network));
                    let mut s = lock(&out);
                    let changed = *s.pins != pins || *s.favorites != favorites || s.generation == 0;
                    if changed {
                        s.pins = Arc::new(pins);
                        s.favorites = Arc::new(favorites);
                        s.generation += 1;
                    }
                    drop(s);
                    if changed || edited {
                        wake();
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("bookmarks: no reader thread: {e}");
        }
        let _ = poke.send(Job::Read);
        Pins {
            shared,
            file,
            seen: 0,
            poke,
            alive: Some(alive),
        }
    }

    /// The bookmarks file this reads, where a pin from its owner goes.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The pins as last read; empty until the first read lands.
    pub fn current(&self) -> Arc<Vec<Pin>> {
        lock(&self.shared).pins.clone()
    }

    /// The bookmarks as last read, for Favorites; empty until the first read lands.
    pub fn favorites(&self) -> Arc<Vec<Favorite>> {
        lock(&self.shared).favorites.clone()
    }

    /// Whether a read landed since the last call.
    pub fn take_changed(&mut self) -> bool {
        let now = lock(&self.shared).generation;
        std::mem::replace(&mut self.seen, now) != now
    }

    /// Reads the file and looks for the folders again (a drive may have come or gone).
    pub fn refresh(&self) {
        let _ = self.poke.send(Job::Read);
    }

    /// Edits the file on the thread as [`edit_at`] does, then reads it again; a failure comes
    /// back from [`Pins::take_failed`] after a wake.
    pub fn edit(&self, f: impl FnOnce(&mut Bookmarks) -> bool + Send + 'static) {
        lock(&self.shared).pending += 1;
        if self.poke.send(Job::Edit(Box::new(f))).is_err() {
            let mut s = lock(&self.shared);
            s.pending -= 1;
            s.failed = Some(io::Error::other("the bookmarks thread is not running"));
        }
    }

    /// Whether an edit is still to be written.
    pub fn busy(&self) -> bool {
        lock(&self.shared).pending > 0
    }

    /// The last edit's failure since the last call.
    pub fn take_failed(&mut self) -> Option<io::Error> {
        lock(&self.shared).failed.take()
    }

    /// Whether a failure waits for [`Pins::take_failed`]; asked after [`Pins::busy`], an edit
    /// ending between the two is seen by one of them.
    pub fn has_failed(&self) -> bool {
        lock(&self.shared).failed.is_some()
    }

    /// Waits up to `limit` for the first read, for a caller that shows the pins at once.
    pub fn wait_first(&self, limit: std::time::Duration) {
        let end = std::time::Instant::now() + limit;
        while lock(&self.shared).generation == 0 && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// How long a dropped [`Pins`] waits for its edits to be written, so a program quitting right
/// after an edit (a one-shot file dialog) keeps it.
const DROP_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

impl Drop for Pins {
    fn drop(&mut self) {
        let end = std::time::Instant::now() + DROP_WAIT;
        while self.busy() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        self.alive = None;
        let _ = self.poke.send(Job::Read);
    }
}

/// `true` when `dir` is pinned in `pins`.
pub fn is_pinned(pins: &[Pin], dir: &Path) -> bool {
    pins.iter().any(|p| p.path == dir)
}

/// [`edit_at`], logging a failure.
fn edit_logged(file: &Path, f: impl FnOnce(&mut Bookmarks) -> bool) {
    if let Err(e) = edit_at(file, f) {
        log::warn!("bookmarks: cannot write {}: {e}", file.display());
    }
}

/// Pins `dir` in the bookmarks file `file`, or unpins it with `pin` false.
pub fn set_pinned(file: &Path, dir: &Path, pin: bool) {
    edit_logged(file, |b| if pin { b.pin(dir) } else { b.unpin(dir) });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    impl Bookmarks {
        fn text(&self) -> String {
            String::from_utf8(self.bytes()).expect("UTF-8")
        }
    }

    const FILE: &str = "file:///home/u/Projects\n\
file:///home/u/My%20Music Tunes\n\
sftp://h/srv/share Server\n\
file://otherhost/srv/x\n\
\n\
file:///home/u/caf%C3%A9 \n";

    #[test]
    fn parse_reads_file_lines_and_their_labels() {
        let b = Bookmarks::parse(FILE);
        let got: Vec<(PathBuf, Option<String>)> = b
            .folders()
            .map(|(p, l)| (p.to_path_buf(), l.map(str::to_string)))
            .collect();
        assert_eq!(
            got,
            vec![
                (PathBuf::from("/home/u/Projects"), None),
                (PathBuf::from("/home/u/My Music"), Some("Tunes".into())),
                (PathBuf::from("/home/u/café"), None),
            ]
        );
    }

    #[test]
    fn an_unedited_file_writes_back_byte_for_byte() {
        assert_eq!(Bookmarks::parse(FILE).text(), FILE);
    }

    #[test]
    fn localhost_counts_as_local_and_a_relative_path_does_not() {
        let b = Bookmarks::parse("file://localhost/srv/a\nfile://srv/b\n");
        let got: Vec<&Path> = b.folders().map(|(p, _)| p).collect();
        assert_eq!(got, [Path::new("/srv/a")]);
    }

    #[test]
    fn pin_appends_an_encoded_line_and_keeps_other_schemes() {
        let mut b = Bookmarks::parse(FILE);
        assert!(b.pin(Path::new("/home/u/A b#c")));
        assert!(!b.pin(Path::new("/home/u/Projects")), "already pinned");
        let text = b.text();
        assert!(text.starts_with(FILE), "every earlier line kept: {text}");
        assert!(text.ends_with("file:///home/u/A%20b%23c\n"), "{text}");
        assert!(Bookmarks::parse(&text).has(Path::new("/home/u/A b#c")));
    }

    #[test]
    fn a_name_that_is_not_utf8_round_trips() {
        let dir = PathBuf::from(OsString::from_vec(b"/home/u/\xff".to_vec()));
        let mut b = Bookmarks::default();
        b.pin(&dir);
        assert_eq!(b.text(), "file:///home/u/%FF\n");
        assert!(Bookmarks::parse(b.text()).has(&dir));
    }

    #[test]
    fn unpin_drops_the_folder_only() {
        let mut b = Bookmarks::parse(FILE);
        assert!(b.unpin(Path::new("/home/u/My Music")));
        assert!(!b.unpin(Path::new("/home/u/My Music")));
        assert_eq!(
            b.text(),
            "file:///home/u/Projects\nsftp://h/srv/share Server\nfile://otherhost/srv/x\n\nfile:///home/u/caf%C3%A9 \n"
        );
    }

    #[test]
    fn shift_swaps_with_the_next_folder_past_other_lines() {
        let mut b = Bookmarks::parse(FILE);
        assert!(b.shift(Path::new("/home/u/My Music"), false));
        let order: Vec<&Path> = b.folders().map(|(p, _)| p).collect();
        assert_eq!(
            order,
            [
                Path::new("/home/u/Projects"),
                Path::new("/home/u/café"),
                Path::new("/home/u/My Music"),
            ]
        );
        assert!(
            b.text()
                .contains("sftp://h/srv/share Server\nfile://otherhost/srv/x\n\n")
        );
        assert!(
            !b.shift(Path::new("/home/u/My Music"), false),
            "already last"
        );
        assert!(
            !b.shift(Path::new("/home/u/Projects"), true),
            "already first"
        );
        assert!(b.shift(Path::new("/home/u/Projects"), false));
        assert_eq!(b.folders().next().unwrap().0, Path::new("/home/u/café"));
    }

    #[test]
    fn edit_at_creates_the_file_and_its_folder() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gtk-3.0/bookmarks");
        edit_at(&file, |b| b.pin(Path::new("/home/u/Work"))).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "file:///home/u/Work\n"
        );
        edit_at(&file, |_| false).unwrap();
        edit_at(&file, |b| b.unpin(Path::new("/home/u/Work"))).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    }

    const NOTED: &str = "# kept\n\
file:///home/u/Projects\n\
not a bookmark\n\
sftp://h/srv/share Server\n\
smb://nas/My%20Files\n";

    fn folder(p: &str) -> Target {
        Target::Folder(PathBuf::from(p))
    }

    /// The line of `b`'s bookmark `i`.
    fn key(b: &Bookmarks, i: usize) -> LineKey {
        favorites(b, &HashSet::new()).swap_remove(i).line
    }

    #[test]
    fn entries_list_every_scheme_and_skip_comments_and_text() {
        let b = Bookmarks::parse(NOTED);
        let got: Vec<(Target, Option<&str>)> = b.entries().map(|(t, l)| (t.clone(), l)).collect();
        assert_eq!(
            got,
            [
                (folder("/home/u/Projects"), None),
                (Target::Uri("sftp://h/srv/share".into()), Some("Server")),
                (Target::Uri("smb://nas/My%20Files".into()), None),
            ]
        );
        assert_eq!(b.text(), NOTED);
    }

    #[test]
    fn a_file_uri_on_another_host_is_a_uri_not_a_folder() {
        let b = Bookmarks::parse("file://otherhost/srv/x\n");
        assert_eq!(
            b.entries().next().unwrap().0,
            &Target::Uri("file://otherhost/srv/x".into())
        );
        assert_eq!(b.folders().count(), 0);
    }

    #[test]
    fn add_names_a_new_folder_and_leaves_its_own_name_out() {
        let mut b = Bookmarks::parse(NOTED);
        assert!(b.add(Path::new("/home/u/Work"), "Work"));
        assert!(b.add(Path::new("/home/u/A b#c%"), "Odd  one "));
        assert_eq!(
            b.text(),
            format!("{NOTED}file:///home/u/Work\nfile:///home/u/A%20b%23c%25 Odd  one\n")
        );
    }

    #[test]
    fn add_on_a_bookmarked_folder_renames_it() {
        let mut b = Bookmarks::parse(NOTED);
        assert!(b.add(Path::new("/home/u/Projects"), "Code"));
        assert!(!b.add(Path::new("/home/u/Projects"), "Code"), "same name");
        assert_eq!(b.entries().count(), 3, "no second line");
        assert!(b.text().contains("\nfile:///home/u/Projects Code\n"));
    }

    #[test]
    fn rename_writes_a_label_and_a_blank_or_own_name_drops_it() {
        let mut b = Bookmarks::parse(NOTED);
        let share = key(&b, 1);
        assert!(b.rename(&share, "Line\none"));
        assert!(
            b.text().contains("\nsftp://h/srv/share Line one\n"),
            "{}",
            b.text()
        );
        assert!(b.rename(&share, "  "));
        assert!(b.text().contains("\nsftp://h/srv/share\n"));
        let nas = key(&b, 2);
        assert!(!b.rename(&nas, "smb://nas/My Files"), "its own name");
        assert!(!b.rename(&key(&Bookmarks::parse("file:///nowhere\n"), 0), "x"));
    }

    #[test]
    fn remove_and_move_entry_step_over_comments_and_text() {
        let mut b = Bookmarks::parse(NOTED);
        let share = key(&b, 1);
        assert!(b.move_entry(&share, true));
        assert_eq!(
            b.text(),
            "# kept\nsftp://h/srv/share Server\nnot a bookmark\nfile:///home/u/Projects\nsmb://nas/My%20Files\n"
        );
        assert!(!b.move_entry(&share, true), "already first");
        let nas = key(&b, 2);
        assert!(!b.move_entry(&nas, false), "already last");
        assert!(b.remove(&share));
        assert!(!b.remove(&share));
        assert_eq!(
            b.text(),
            "# kept\nnot a bookmark\nfile:///home/u/Projects\nsmb://nas/My%20Files\n"
        );
    }

    #[test]
    fn an_edit_takes_the_line_picked_and_keeps_its_uri_as_written() {
        let text = "file:///ab A\nfile://localhost/ab B\nfile:///a%2Fb\nfile:///ab A\n";
        let mut b = Bookmarks::parse(text);
        assert!(b.rename(&key(&b, 1), "Local"));
        assert!(b.rename(&key(&b, 2), "Slash"));
        assert_eq!(
            b.text(),
            "file:///ab A\nfile://localhost/ab Local\nfile:///a%2Fb Slash\nfile:///ab A\n"
        );
        assert!(b.move_entry(&key(&b, 1), true));
        assert_eq!(
            b.text(),
            "file://localhost/ab Local\nfile:///ab A\nfile:///a%2Fb Slash\nfile:///ab A\n"
        );
        assert!(b.remove(&key(&b, 3)), "the second copy of a line");
        assert!(b.remove(&key(&b, 0)));
        assert_eq!(b.text(), "file:///ab A\nfile:///a%2Fb Slash\n");
    }

    #[test]
    fn favorites_name_and_grey_as_the_file_says() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().join("Here");
        std::fs::create_dir(&here).unwrap();
        let mut b = Bookmarks::parse("smb://nas/My%20Files\nsftp://h/srv Server\n");
        b.add(&here, "Mine");
        b.add(Path::new("/nonexistent-pin/Gone"), "");
        let got: Vec<(String, bool)> = favorites(&b, &HashSet::new())
            .into_iter()
            .map(|f| (f.label, f.present))
            .collect();
        assert_eq!(
            got,
            [
                ("smb://nas/My Files".to_string(), true),
                ("Server".to_string(), true),
                ("Mine".to_string(), true),
                ("Gone".to_string(), false),
            ]
        );
    }

    #[test]
    fn a_missing_file_reads_empty_and_an_edit_writes_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("none/gtk-3.0/bookmarks");
        assert_eq!(load(&file), Bookmarks::default());
        edit_at(&file, |b| b.add(Path::new("/srv/a"), "A")).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "file:///srv/a A\n");
    }

    #[test]
    fn shown_labels_by_name_and_greys_a_missing_folder() {
        let dir = tempfile::tempdir().unwrap();
        let here = dir.path().join("Here");
        std::fs::create_dir(&here).unwrap();
        let mut b = Bookmarks::default();
        b.pin(&here);
        b.pin(Path::new("/nonexistent-pin/Gone"));
        b.pin(Path::new("/mnt/remote/Share"));
        let network: HashSet<PathBuf> = [PathBuf::from("/mnt/remote")].into();
        let pins = shown(&b, &network);
        let got: Vec<(&str, bool)> = pins.iter().map(|p| (p.label.as_str(), p.present)).collect();
        assert_eq!(
            got,
            [("Here", true), ("Gone", false), ("Share", true)],
            "a folder on a network mount is never looked for"
        );
    }

    #[test]
    fn shown_stops_at_the_cap() {
        let mut b = Bookmarks::default();
        for i in 0..MAX_SHOWN + 5 {
            b.pin(&PathBuf::from(format!("/home/u/{i}")));
        }
        assert_eq!(shown(&b, &HashSet::new()).len(), MAX_SHOWN);
    }

    #[test]
    fn watches_name_the_file_or_the_nearest_folder_on_its_way() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gtk-3.0/bookmarks");
        assert_eq!(
            watches(&file),
            [(dir.path().to_path_buf(), Some(OsString::from("gtk-3.0")))]
        );
        std::fs::create_dir(dir.path().join("gtk-3.0")).unwrap();
        assert_eq!(
            watches(&file),
            [(
                dir.path().join("gtk-3.0"),
                Some(OsString::from("bookmarks"))
            )]
        );
    }

    #[test]
    fn the_thread_sees_a_pin_written_after_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gtk-3.0/bookmarks");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let mut pins = Pins::start_at(file.clone(), move || {
            let _ = lock(&tx).send(());
        });
        let wait = std::time::Duration::from_secs(10);
        rx.recv_timeout(wait).expect("the first read");
        assert!(pins.take_changed());
        assert!(pins.current().is_empty());
        edit_at(&file, |b| b.pin(dir.path())).unwrap();
        // The folder's own creation may land first; wait until the pin shows.
        let end = std::time::Instant::now() + wait;
        while pins.current().is_empty() && std::time::Instant::now() < end {
            let _ = rx.recv_timeout(std::time::Duration::from_millis(200));
        }
        assert!(pins.take_changed());
        assert_eq!(pins.current()[0].path, dir.path());
        assert!(pins.current()[0].present);
    }

    #[test]
    fn an_edit_never_writes_over_a_file_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big");
        let text = "file:///srv/a\n".repeat(MAX_BYTES as usize / 14 + 1);
        std::fs::write(&big, &text).unwrap();
        let e = edit_at(&big, |b| b.pin(Path::new("/srv/b"))).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::FileTooLarge);
        assert_eq!(
            std::fs::read_to_string(&big).unwrap(),
            text,
            "left as it was"
        );

        let fifo = dir.path().join("fifo");
        crate::testfs::mkfifo(&fifo);
        let e = edit_at(&fifo, |b| b.pin(Path::new("/srv/b"))).unwrap_err();
        assert_eq!(
            e.kind(),
            io::ErrorKind::InvalidInput,
            "never opened to read"
        );
        use std::os::unix::fs::FileTypeExt;
        assert!(
            std::fs::symlink_metadata(&fifo)
                .unwrap()
                .file_type()
                .is_fifo()
        );

        let folder = dir.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        assert!(edit_at(&folder, |b| b.pin(Path::new("/srv/b"))).is_err());
        assert!(folder.is_dir());
    }

    #[test]
    fn bytes_that_are_not_utf8_write_back_as_read() {
        let file: &[u8] = b"# caf\xe9\nfile:///srv/a A\xff\nsmb://nas/x\n";
        let mut b = Bookmarks::parse(file);
        assert_eq!(b.entries().next().unwrap().1, Some("A\u{FFFD}"));
        assert!(b.remove(&key(&b, 1)));
        assert!(b.pin(Path::new("/srv/b")));
        assert_eq!(
            b.bytes(),
            b"# caf\xe9\nfile:///srv/a A\xff\nfile:///srv/b\n"
        );
    }

    #[test]
    fn each_line_keeps_its_own_ending() {
        let mut b = Bookmarks::parse("file:///srv/a A\r\n# note\nfile:///srv/b\r\n");
        assert_eq!(b.text(), "file:///srv/a A\r\n# note\nfile:///srv/b\r\n");
        assert!(b.rename(&key(&b, 0), "Z"));
        assert!(b.pin(Path::new("/srv/c")));
        assert_eq!(
            b.text(),
            "file:///srv/a Z\r\n# note\nfile:///srv/b\r\nfile:///srv/c\r\n",
            "a new line ends as the last one does"
        );
        assert_eq!(Bookmarks::parse("file:///srv/a").text(), "file:///srv/a\n");
    }

    #[test]
    fn an_address_shows_without_its_password() {
        let name = |uri: &str| Target::Uri(uri.into()).default_name();
        assert_eq!(name("sftp://user:secret@host/srv"), "sftp://user@host/srv");
        assert_eq!(
            name("smb://u:p%40ss@nas/My%20Files"),
            "smb://u@nas/My Files"
        );
        assert_eq!(name("webdav://u:secret@host/x"), "webdav://u@host/x");
        assert_eq!(name("recent:///"), "recent:///");
        let b = Bookmarks::parse("sftp://user:secret@host/srv\n");
        let shown = favorites(&b, &HashSet::new());
        assert_eq!(shown[0].label, "sftp://user@host/srv");
    }

    #[test]
    fn a_long_label_shows_cut_and_stays_whole_in_the_file() {
        let long = "é".repeat(MAX_LABEL + 50);
        let mut b = Bookmarks::default();
        b.add(Path::new("/srv/a"), &long);
        assert_eq!(
            favorites(&b, &HashSet::new())[0].label,
            "é".repeat(MAX_LABEL)
        );
        assert_eq!(shown(&b, &HashSet::new())[0].label, "é".repeat(MAX_LABEL));
        assert!(b.text().ends_with(&format!(" {long}\n")));
    }

    #[test]
    fn a_symlinked_file_is_watched_where_its_link_ends_too() {
        let dir = tempfile::tempdir().unwrap();
        let (gtk, real) = (dir.path().join("gtk-3.0"), dir.path().join("dotfiles/gtk"));
        std::fs::create_dir_all(&gtk).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        let file = gtk.join("bookmarks");
        std::os::unix::fs::symlink("../dotfiles/gtk/bookmarks", &file).unwrap();
        let name = Some(OsString::from("bookmarks"));
        assert_eq!(
            watches(&file),
            [(gtk.clone(), name.clone()), (real.clone(), name.clone())],
            "dangling"
        );
        std::fs::write(real.join("bookmarks"), "").unwrap();
        assert_eq!(watches(&file), [(gtk, name.clone()), (real, name)]);
    }

    /// Pins on `file`, and the wakes it sends.
    fn reading(file: &Path) -> (Pins, mpsc::Receiver<()>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let pins = Pins::start_at(file.to_path_buf(), move || {
            let _ = lock(&tx).send(());
        });
        (pins, rx)
    }

    fn wait_until(pins: &Pins, woke: &mpsc::Receiver<()>, done: impl Fn(&Pins) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !done(pins) {
            assert!(std::time::Instant::now() < end, "timed out");
            let _ = woke.recv_timeout(std::time::Duration::from_millis(200));
        }
    }

    #[test]
    fn the_thread_sees_an_edit_where_a_symlinked_file_ends() {
        let dir = tempfile::tempdir().unwrap();
        let (gtk, real) = (dir.path().join("gtk-3.0"), dir.path().join("dotfiles"));
        std::fs::create_dir_all(&gtk).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("bookmarks"), "").unwrap();
        std::os::unix::fs::symlink(real.join("bookmarks"), gtk.join("bookmarks")).unwrap();
        let (pins, woke) = reading(&gtk.join("bookmarks"));
        wait_until(&pins, &woke, |p| lock(&p.shared).generation > 0);
        std::fs::write(real.join(".new"), "file:///srv/a A\n").unwrap();
        std::fs::rename(real.join(".new"), real.join("bookmarks")).unwrap();
        wait_until(&pins, &woke, |p| p.favorites().len() == 1);
    }

    #[test]
    fn the_thread_follows_its_folder_renamed_and_made_again() {
        let dir = tempfile::tempdir().unwrap();
        let gtk = dir.path().join("gtk-3.0");
        std::fs::create_dir(&gtk).unwrap();
        std::fs::write(gtk.join("bookmarks"), "file:///srv/a A\n").unwrap();
        let (pins, woke) = reading(&gtk.join("bookmarks"));
        wait_until(&pins, &woke, |p| p.favorites().len() == 1);
        std::fs::rename(&gtk, dir.path().join("old")).unwrap();
        std::fs::create_dir(&gtk).unwrap();
        wait_until(&pins, &woke, |p| p.favorites().is_empty());
        std::fs::write(gtk.join("bookmarks"), "file:///srv/b B\nfile:///srv/c C\n").unwrap();
        wait_until(&pins, &woke, |p| p.favorites().len() == 2);
    }

    #[test]
    fn an_edit_runs_on_the_thread_and_a_failure_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gtk-3.0/bookmarks");
        let (mut pins, woke) = reading(&file);
        pins.edit(|b| b.add(Path::new("/srv/a"), "A"));
        wait_until(&pins, &woke, |p| !p.busy() && p.favorites().len() == 1);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "file:///srv/a A\n");
        assert!(pins.take_failed().is_none());

        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        pins.edit(|b| b.add(Path::new("/srv/b"), "B"));
        wait_until(&pins, &woke, |p| !p.busy());
        assert_eq!(
            pins.take_failed().map(|e| e.kind()),
            Some(io::ErrorKind::InvalidInput)
        );
        assert!(pins.take_failed().is_none(), "taken once");
    }

    #[test]
    fn a_dropped_pins_waits_for_its_edits_to_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("gtk-3.0/bookmarks");
        let (pins, _woke) = reading(&file);
        pins.edit(|b| {
            std::thread::sleep(std::time::Duration::from_millis(300));
            b.pin(Path::new("/srv/a"))
        });
        drop(pins);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "file:///srv/a\n");
    }

    #[test]
    fn a_write_clears_an_old_temp_file_left_beside_the_bookmarks() {
        let dir = tempfile::tempdir().unwrap();
        let gtk = dir.path().join("gtk-3.0");
        std::fs::create_dir(&gtk).unwrap();
        let old = gtk.join(".bookmarks.0123456789abcdef.tmp");
        let fresh = gtk.join(".bookmarks.fedcba9876543210.tmp");
        let other = gtk.join(".bookmarks.mine.tmp");
        let hour_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for f in [&old, &fresh, &other] {
            std::fs::write(f, "file:///half").unwrap();
        }
        for f in [&old, &other] {
            let f = std::fs::File::options().write(true).open(f).unwrap();
            f.set_modified(hour_ago).unwrap();
        }
        let file = gtk.join("bookmarks");
        edit_at(&file, |_| false).unwrap();
        assert!(old.exists(), "a read alone clears nothing");
        edit_at(&file, |b| b.pin(Path::new("/srv/a"))).unwrap();
        assert!(!old.exists(), "an hour-old temp file is gone");
        assert!(
            fresh.exists(),
            "a new one may be another writer's, mid-write"
        );
        assert!(other.exists(), "a name no write gives its temp file stays");
    }
}
