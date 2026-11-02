//! Pinned folders: the `file://` lines of GTK's bookmarks file, `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`,
//! one `URI[ label]` per line, so a pin shows in GTK sidebars and portal dialogs too. [`Pins`]
//! watches the file and reads it off the caller's loop.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use reprisebus::mutex::lock;

/// The folders a places bar or a pane lists at most; the file may hold more.
pub const MAX_SHOWN: usize = 20;

/// The longest bookmarks file read; GTK's own stays far below.
const MAX_BYTES: u64 = 1 << 20;

pub fn bookmarks_file() -> PathBuf {
    crate::autostart::config_home().join("gtk-3.0/bookmarks")
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A local `file://` bookmark: its folder and label, and the line as read (None once edited).
    Folder {
        path: PathBuf,
        label: Option<String>,
        raw: Option<String>,
    },
    /// Anything else (another scheme, a remote host, a blank line), written back as read.
    Other(String),
}

/// The bookmarks file's lines.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bookmarks {
    lines: Vec<Line>,
}

impl Bookmarks {
    pub fn parse(text: &str) -> Bookmarks {
        let lines = text
            .lines()
            .map(|raw| match parse_line(raw) {
                Some((path, label)) => Line::Folder {
                    path,
                    label,
                    raw: Some(raw.to_string()),
                },
                None => Line::Other(raw.to_string()),
            })
            .collect();
        Bookmarks { lines }
    }

    /// The file's text: lines not edited exactly as read, each ending in a newline.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Folder { raw: Some(raw), .. } | Line::Other(raw) => out.push_str(raw),
                Line::Folder { path, label, .. } => {
                    out.push_str(&crate::thumbnails::file_uri(path));
                    if let Some(label) = label {
                        out.push(' ');
                        out.push_str(label);
                    }
                }
            }
            out.push('\n');
        }
        out
    }

    /// The local folders, in file order, with their labels.
    pub fn folders(&self) -> impl Iterator<Item = (&Path, Option<&str>)> {
        self.lines.iter().filter_map(|l| match l {
            Line::Folder { path, label, .. } => Some((path.as_path(), label.as_deref())),
            Line::Other(_) => None,
        })
    }

    pub fn has(&self, dir: &Path) -> bool {
        self.position(dir).is_some()
    }

    fn position(&self, dir: &Path) -> Option<usize> {
        self.lines
            .iter()
            .position(|l| matches!(l, Line::Folder { path, .. } if path == dir))
    }

    /// Pins `dir` last; false when it was pinned already.
    pub fn pin(&mut self, dir: &Path) -> bool {
        if self.has(dir) {
            return false;
        }
        self.lines.push(Line::Folder {
            path: dir.to_path_buf(),
            label: None,
            raw: None,
        });
        true
    }

    /// Drops every line naming `dir`; false when none did.
    pub fn unpin(&mut self, dir: &Path) -> bool {
        let before = self.lines.len();
        self.lines
            .retain(|l| !matches!(l, Line::Folder { path, .. } if path == dir));
        self.lines.len() != before
    }

    /// Moves `dir` past the folder before it (`up`) or after it, the lines of other schemes
    /// staying where they are among the rest; false at either end.
    pub fn shift(&mut self, dir: &Path, up: bool) -> bool {
        let Some(at) = self.position(dir) else {
            return false;
        };
        let folder = |l: &Line| matches!(l, Line::Folder { .. });
        let other = if up {
            self.lines[..at].iter().rposition(folder)
        } else {
            self.lines[at + 1..]
                .iter()
                .position(folder)
                .map(|i| at + 1 + i)
        };
        let Some(other) = other else {
            return false;
        };
        self.lines.swap(at, other);
        true
    }
}

/// A `file://` line's folder and label; None for any other line. The URI ends at the first
/// space; the rest of the line is the label.
fn parse_line(line: &str) -> Option<(PathBuf, Option<String>)> {
    let (uri, label) = match line.split_once(' ') {
        Some((uri, label)) => (uri, Some(label.to_string()).filter(|l| !l.is_empty())),
        None => (line, None),
    };
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(crate::trash::percent_decode(rest)));
    Some((path, label))
}

/// The bookmarks file at `file`; empty when it is missing or unreadable.
pub fn load(file: &Path) -> Bookmarks {
    match crate::ops::read_regular(file, MAX_BYTES) {
        Ok(bytes) => Bookmarks::parse(&String::from_utf8_lossy(&bytes)),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::info!("bookmarks: cannot read {}: {e}", file.display());
            }
            Bookmarks::default()
        }
    }
}

/// Reads `file`, lets `f` edit it and writes it back in one rename when `f` says it changed.
/// GTK takes no lock either, so a line another program writes between the read and the rename
/// is lost.
pub fn edit_at(file: &Path, f: impl FnOnce(&mut Bookmarks) -> bool) -> std::io::Result<()> {
    let mut b = load(file);
    if !f(&mut b) {
        return Ok(());
    }
    crate::ops::write_atomic(file, b.text().as_bytes())
}

/// The shown pins of `b`, each folder looked for unless it sits on one of the `network` mounts,
/// where a look could hang on a dead server: those count as there.
pub fn shown(b: &Bookmarks, network: &std::collections::HashSet<PathBuf>) -> Vec<Pin> {
    b.folders()
        .take(MAX_SHOWN)
        .map(|(path, label)| Pin {
            path: path.to_path_buf(),
            label: label
                .map(str::to_string)
                .unwrap_or_else(|| crate::places::dialog_label(path)),
            present: network.iter().any(|m| path.starts_with(m)) || path.is_dir(),
        })
        .collect()
}

/// What to watch so `file` is seen changing, arriving or leaving: its folder for its name, or,
/// while that folder is missing, the nearest folder above that is there for the next name down.
fn watches(file: &Path) -> Vec<(PathBuf, Option<OsString>)> {
    let mut child = file;
    while let Some(dir) = child.parent() {
        if dir.is_dir() {
            return vec![(
                dir.to_path_buf(),
                child.file_name().map(OsStr::to_os_string),
            )];
        }
        child = dir;
    }
    Vec::new()
}

#[derive(Default)]
struct Shared {
    pins: Arc<Vec<Pin>>,
    generation: u64,
}

/// The pins of the bookmarks file, kept current by a thread that watches it: the thread reads
/// it and looks for each folder, so a dead mount stalls the thread and not its owner's loop.
pub struct Pins {
    shared: Arc<Mutex<Shared>>,
    seen: u64,
    poke: mpsc::Sender<()>,
    /// The thread ends once this drops.
    _alive: Arc<()>,
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
        let spawned = std::thread::Builder::new()
            .name("pins".to_string())
            .spawn(move || {
                let watch = crate::folderwatch::Watch::new(move || {
                    let _ = from_watch.send(());
                });
                let mut watch = watch
                    .map_err(|e| log::warn!("bookmarks: no watch: {e}"))
                    .ok();
                let mut watched = None;
                while pokes.recv().is_ok() {
                    while pokes.try_recv().is_ok() {}
                    if gone.strong_count() == 0 {
                        return;
                    }
                    let want = watches(&file);
                    if let Some(w) = &mut watch
                        && watched.as_ref() != Some(&want)
                    {
                        w.watch_all(&want);
                        watched = Some(want);
                    }
                    let mounts = std::fs::read("/proc/self/mounts").unwrap_or_default();
                    let pins = shown(&load(&file), &crate::places::network_mounts(mounts));
                    let mut s = lock(&out);
                    if *s.pins != pins || s.generation == 0 {
                        s.pins = Arc::new(pins);
                        s.generation += 1;
                        drop(s);
                        wake();
                    }
                }
            });
        if let Err(e) = spawned {
            log::warn!("bookmarks: no reader thread: {e}");
        }
        let _ = poke.send(());
        Pins {
            shared,
            seen: 0,
            poke,
            _alive: alive,
        }
    }

    /// The pins as last read; empty until the first read lands.
    pub fn current(&self) -> Arc<Vec<Pin>> {
        lock(&self.shared).pins.clone()
    }

    /// Whether a read landed since the last call.
    pub fn take_changed(&mut self) -> bool {
        let now = lock(&self.shared).generation;
        std::mem::replace(&mut self.seen, now) != now
    }

    /// Reads the file and looks for the folders again (a drive may have come or gone).
    pub fn refresh(&self) {
        let _ = self.poke.send(());
    }

    /// Waits up to `limit` for the first read, for a caller that shows the pins at once.
    pub fn wait_first(&self, limit: std::time::Duration) {
        let end = std::time::Instant::now() + limit;
        while lock(&self.shared).generation == 0 && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

impl Drop for Pins {
    fn drop(&mut self) {
        let _ = self.poke.send(());
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

/// Moves pinned `dir` one place up or down in the bookmarks file `file`.
pub fn shift(file: &Path, dir: &Path, up: bool) {
    edit_logged(file, |b| b.shift(dir, up));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

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
        assert!(Bookmarks::parse(&b.text()).has(&dir));
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
}
