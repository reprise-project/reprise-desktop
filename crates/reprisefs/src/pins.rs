//! Pinned folders and Favorites: GTK's bookmarks file, `$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, one
//! `URI[ label]` per line, so a bookmark shows in GTK sidebars and portal dialogs too. Places bars
//! and panes show its local folders, Explorer's Favorites every bookmark. [`Pins`] watches the
//! file and reads it off the caller's loop.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use reprisebus::mutex::lock;

/// The folders a places bar or a pane lists at most; the file may hold more.
pub const MAX_SHOWN: usize = 20;

/// The bookmarks Favorites lists at most.
pub const MAX_FAVORITES: usize = 100;

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
    /// its escapes decoded.
    pub fn default_name(&self) -> String {
        match self {
            Target::Folder(path) => crate::places::dialog_label(path),
            Target::Uri(uri) => String::from_utf8_lossy(&crate::trash::percent_decode(uri)).into(),
        }
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    /// A bookmark: its target and label, and the line as read (None once edited).
    Entry {
        target: Target,
        label: Option<String>,
        raw: Option<String>,
    },
    /// Anything else (a blank line, a comment, text with no URI), written back as read.
    Other(String),
}

impl Line {
    fn folder(&self) -> Option<&Path> {
        match self {
            Line::Entry {
                target: Target::Folder(path),
                ..
            } => Some(path),
            _ => None,
        }
    }

    fn is(&self, want: &Target) -> bool {
        matches!(self, Line::Entry { target, .. } if target == want)
    }
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
                Some((target, label)) => Line::Entry {
                    target,
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
                Line::Entry { raw: Some(raw), .. } | Line::Other(raw) => out.push_str(raw),
                Line::Entry { target, label, .. } => {
                    out.push_str(&target.uri());
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

    /// Every bookmark, in file order, with its label.
    pub fn entries(&self) -> impl Iterator<Item = (&Target, Option<&str>)> {
        self.lines.iter().filter_map(|l| match l {
            Line::Entry { target, label, .. } => Some((target, label.as_deref())),
            Line::Other(_) => None,
        })
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
        self.lines.push(Line::Entry {
            target: Target::Folder(dir.to_path_buf()),
            label: None,
            raw: None,
        });
        true
    }

    /// Drops every line naming `dir`; false when none did.
    pub fn unpin(&mut self, dir: &Path) -> bool {
        self.remove(&Target::Folder(dir.to_path_buf()))
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
        if self.lines.iter().any(|l| l.is(&target)) {
            return self.rename(&target, name);
        }
        let label = label_for(&target, name);
        self.lines.push(Line::Entry {
            target,
            label,
            raw: None,
        });
        true
    }

    /// Names the first bookmark of `target` `name`; false when there is none or the name stays.
    pub fn rename(&mut self, target: &Target, name: &str) -> bool {
        let want = label_for(target, name);
        let Some(Line::Entry { label, raw, .. }) = self.lines.iter_mut().find(|l| l.is(target))
        else {
            return false;
        };
        if *label == want {
            return false;
        }
        *label = want;
        *raw = None;
        true
    }

    /// Drops every bookmark of `target`; false when there was none.
    pub fn remove(&mut self, target: &Target) -> bool {
        let before = self.lines.len();
        self.lines.retain(|l| !l.is(target));
        self.lines.len() != before
    }

    /// Moves the first bookmark of `target` past the bookmark before it (`up`) or after it, the
    /// other lines staying where they are; false at either end.
    pub fn move_entry(&mut self, target: &Target, up: bool) -> bool {
        match self.lines.iter().position(|l| l.is(target)) {
            Some(at) => self.swap_with(at, up, |l| matches!(l, Line::Entry { .. })),
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
/// URI. The URI ends at the first space; the rest of the line is the label.
fn parse_line(line: &str) -> Option<(Target, Option<String>)> {
    if line.starts_with('#') {
        return None;
    }
    let (uri, label) = match line.split_once(' ') {
        Some((uri, label)) => (uri, Some(label.to_string()).filter(|l| !l.is_empty())),
        None => (line, None),
    };
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
    Some((target, label))
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

/// Reads `file`, lets `f` edit it and writes it back in one rename when `f` says it changed;
/// the bookmarks as they now stand. GTK takes no lock either, so a line another program writes
/// between the read and the rename is lost.
pub fn edit_at(file: &Path, f: impl FnOnce(&mut Bookmarks) -> bool) -> std::io::Result<Bookmarks> {
    let mut b = load(file);
    if f(&mut b) {
        crate::ops::write_atomic(file, b.text().as_bytes())?;
    }
    Ok(b)
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
            label: label
                .map(str::to_string)
                .unwrap_or_else(|| crate::places::dialog_label(path)),
            present: present(path, network),
        })
        .collect()
}

/// The bookmarks of `b` as Favorites lists them, each folder looked for as [`present`] does.
pub fn favorites(b: &Bookmarks, network: &std::collections::HashSet<PathBuf>) -> Vec<Favorite> {
    b.entries()
        .take(MAX_FAVORITES)
        .map(|(target, label)| Favorite {
            target: target.clone(),
            label: label.map_or_else(|| target.default_name(), str::to_string),
            present: match target {
                Target::Folder(path) => present(path, network),
                Target::Uri(_) => true,
            },
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
    favorites: Arc<Vec<Favorite>>,
    generation: u64,
}

/// The pins and Favorites of the bookmarks file, kept current by a thread that watches it: the
/// thread reads it and looks for each folder, so a dead mount stalls the thread and not its owner's loop.
pub struct Pins {
    shared: Arc<Mutex<Shared>>,
    file: PathBuf,
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
        let read = file.clone();
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
                    let want = watches(&read);
                    if let Some(w) = &mut watch
                        && watched.as_ref() != Some(&want)
                    {
                        w.watch_all(&want);
                        watched = Some(want);
                    }
                    let mounts = std::fs::read("/proc/self/mounts").unwrap_or_default();
                    let network = crate::places::network_mounts(mounts);
                    let b = load(&read);
                    let (pins, favorites) = (shown(&b, &network), favorites(&b, &network));
                    let mut s = lock(&out);
                    if *s.pins != pins || *s.favorites != favorites || s.generation == 0 {
                        s.pins = Arc::new(pins);
                        s.favorites = Arc::new(favorites);
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
            file,
            seen: 0,
            poke,
            _alive: alive,
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

    const NOTED: &str = "# kept\n\
file:///home/u/Projects\n\
not a bookmark\n\
sftp://h/srv/share Server\n\
smb://nas/My%20Files\n";

    fn folder(p: &str) -> Target {
        Target::Folder(PathBuf::from(p))
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
        let share = Target::Uri("sftp://h/srv/share".into());
        assert!(b.rename(&share, "Line\none"));
        assert!(
            b.text().contains("\nsftp://h/srv/share Line one\n"),
            "{}",
            b.text()
        );
        assert!(b.rename(&share, "  "));
        assert!(b.text().contains("\nsftp://h/srv/share\n"));
        let nas = Target::Uri("smb://nas/My%20Files".into());
        assert!(!b.rename(&nas, "smb://nas/My Files"), "its own name");
        assert!(!b.rename(&folder("/nowhere"), "x"));
    }

    #[test]
    fn remove_and_move_entry_step_over_comments_and_text() {
        let mut b = Bookmarks::parse(NOTED);
        let share = Target::Uri("sftp://h/srv/share".into());
        assert!(b.move_entry(&share, true));
        assert_eq!(
            b.text(),
            "# kept\nsftp://h/srv/share Server\nnot a bookmark\nfile:///home/u/Projects\nsmb://nas/My%20Files\n"
        );
        assert!(!b.move_entry(&share, true), "already first");
        let nas = Target::Uri("smb://nas/My%20Files".into());
        assert!(!b.move_entry(&nas, false), "already last");
        assert!(b.remove(&share));
        assert!(!b.remove(&share));
        assert_eq!(
            b.text(),
            "# kept\nnot a bookmark\nfile:///home/u/Projects\nsmb://nas/My%20Files\n"
        );
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
}
