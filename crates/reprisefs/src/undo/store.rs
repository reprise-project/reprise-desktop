//! The session's one undo history: every Explorer window, the desktop and Explorer's file jobs
//! push to it and undo from it, so Undo anywhere reverses the most recent file operation done
//! anywhere.
//!
//! It is the file `$XDG_RUNTIME_DIR/reprise-undo`, rewritten whole through a rename under an
//! exclusive lock on `reprise-undo.lock`; readers take no lock and reparse only when the file
//! changed. An undo holds `reprise-undo.run` until it ends ([`Running`]), so another window's
//! Undo does nothing meanwhile instead of reversing the entry below a half-reversed one. Without
//! a runtime directory the history is the process's own.

use std::cell::RefCell;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{EntryId, FileId, Undo, UndoStack, Undoable};
use crate::trash::Trashed;

/// How long a write waits for another process's, which only rewrites a small file.
const PATIENCE: Duration = Duration::from_millis(200);

const MAGIC: &[u8] = b"reprise-undo 1";

/// The history and the file it lives in, read again whenever another process rewrote it.
#[derive(Default)]
pub struct SharedUndo {
    file: Option<PathBuf>,
    seen: RefCell<Seen>,
}

#[derive(Default)]
struct Seen {
    stamp: Option<Stamp>,
    stack: UndoStack,
}

/// The file's device, inode, change time and size: every write renames a new inode in.
type Stamp = (u64, u64, i64, i64, u64);

/// Held while an undo taken off a [`SharedUndo`] runs; dropping it lets the next Undo in.
pub struct Running {
    _lock: Option<File>,
}

impl SharedUndo {
    /// The session's history in `$XDG_RUNTIME_DIR`, or the process's own without one.
    pub fn session() -> Self {
        match std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            Some(dir) => Self::at(dir.join("reprise-undo")),
            None => Self::default(),
        }
    }

    pub fn at(file: PathBuf) -> Self {
        SharedUndo {
            file: Some(file),
            seen: RefCell::default(),
        }
    }

    fn read<R>(&self, f: impl FnOnce(&UndoStack) -> R) -> R {
        let mut seen = self.seen.borrow_mut();
        if let Some(file) = &self.file {
            seen.refresh(file);
        }
        f(&seen.stack)
    }

    /// `f` on the history as the file holds it now, written back before another process reads
    /// it. A lock or write that fails leaves the change to this process alone.
    fn edit<R>(&mut self, f: impl FnOnce(&mut UndoStack) -> R) -> R {
        let seen = self.seen.get_mut();
        let Some(file) = &self.file else {
            return f(&mut seen.stack);
        };
        let lock = hold(&beside(file, ".lock"), Some(PATIENCE))
            .map_err(|e| log::warn!("reprisefs: undo history {}: {e}", file.display()))
            .ok();
        if lock.is_some() {
            seen.refresh(file);
        }
        let r = f(&mut seen.stack);
        if lock.is_some() {
            match save(file, &seen.stack) {
                Ok(stamp) => seen.stamp = Some(stamp),
                Err(e) => log::warn!("reprisefs: undo history {}: {e}", file.display()),
            }
        }
        r
    }

    pub fn push(&mut self, u: Undoable) {
        self.edit(|s| s.push(u))
    }

    /// [`UndoStack::push_all`].
    pub fn push_all(&mut self, batch: Vec<Undoable>) {
        if !batch.is_empty() {
            self.edit(|s| s.push_all(batch))
        }
    }

    /// [`UndoStack::clear`].
    pub fn clear(&mut self) {
        self.edit(UndoStack::clear)
    }

    /// [`UndoStack::retain_non_trash`].
    pub fn retain_non_trash(&mut self) {
        self.edit(UndoStack::retain_non_trash)
    }

    /// [`UndoStack::forget_gone`].
    pub fn forget_gone(&mut self, files: &[PathBuf]) {
        if !files.is_empty() {
            self.edit(|s| s.forget_gone(files))
        }
    }

    pub fn label(&self) -> Option<&'static str> {
        self.read(UndoStack::label)
    }

    /// [`UndoStack::many`].
    pub fn many(&self) -> bool {
        self.read(UndoStack::many)
    }

    /// The most recent entry, off the history ([`UndoStack::take`]), with the hold that keeps
    /// every other Undo off until it drops. None while another undo runs.
    pub fn take(&mut self) -> Option<(Undo, Running)> {
        self.take_with(UndoStack::take)
    }

    /// [`SharedUndo::take`] when the top entry is still `id` ([`UndoStack::take_entry`]).
    pub fn take_entry(&mut self, id: EntryId) -> Option<(Undo, Running)> {
        self.take_with(|s| s.take_entry(id))
    }

    fn take_with(
        &mut self,
        take: impl FnOnce(&mut UndoStack) -> Option<Undo>,
    ) -> Option<(Undo, Running)> {
        let lock = match &self.file {
            None => None,
            Some(file) => match hold(&beside(file, ".run"), None) {
                Ok(f) => Some(f),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return None,
                Err(e) => {
                    log::warn!("reprisefs: undo history {}: {e}", file.display());
                    None
                }
            },
        };
        let undo = self.edit(take)?;
        Some((undo, Running { _lock: lock }))
    }

    /// [`UndoStack::put_back`].
    pub fn put_back(
        &mut self,
        undo: Undo,
        done: usize,
        error: Option<&io::Error>,
    ) -> Option<EntryId> {
        self.edit(|s| s.put_back(undo, done, error))
    }
}

impl Seen {
    /// Reparse `file` when it changed since it was last read; a missing file is an empty history
    /// and an unreadable one starts over.
    fn refresh(&mut self, file: &Path) {
        let mut f = match File::open(file) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if self.stamp.take().is_some() {
                    self.stack = UndoStack::default();
                }
                return;
            }
            Err(e) => {
                log::warn!("reprisefs: undo history {}: {e}", file.display());
                return;
            }
        };
        let Ok(meta) = f.metadata() else {
            return;
        };
        let now = stamp(&meta);
        if self.stamp == Some(now) {
            return;
        }
        let mut data = Vec::new();
        if let Err(e) = f.read_to_end(&mut data) {
            log::warn!("reprisefs: undo history {}: {e}", file.display());
            return;
        }
        self.stack = decode(&data).unwrap_or_else(|| {
            log::warn!("reprisefs: undo history {} is unreadable", file.display());
            UndoStack::default()
        });
        self.stamp = Some(now);
    }
}

fn stamp(m: &std::fs::Metadata) -> Stamp {
    (m.dev(), m.ino(), m.ctime(), m.ctime_nsec(), m.size())
}

fn beside(file: &Path, suffix: &str) -> PathBuf {
    let mut name = file.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// `path` (made 0600 when missing) under an exclusive `flock`, waiting up to `patience` for
/// another holder; `WouldBlock` once it runs out.
fn hold(path: &Path, patience: Option<Duration>) -> io::Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    let deadline = Instant::now() + patience.unwrap_or_default();
    loop {
        match f.try_lock() {
            Ok(()) => return Ok(f),
            Err(std::fs::TryLockError::Error(e)) => return Err(e),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1))
            }
            Err(std::fs::TryLockError::WouldBlock) => return Err(io::ErrorKind::WouldBlock.into()),
        }
    }
}

fn save(file: &Path, stack: &UndoStack) -> io::Result<Stamp> {
    let new = beside(file, ".new");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&new)?;
    f.write_all(&encode(stack))?;
    std::fs::rename(&new, file)?;
    Ok(stamp(&f.metadata()?))
}

/// The history as NUL-terminated fields: [`MAGIC`], the clear count, the push count, the
/// entry count, then each entry's id and item.
fn encode(stack: &UndoStack) -> Vec<u8> {
    let mut out = Out(Vec::new());
    out.field(MAGIC);
    out.num(stack.epoch);
    out.num(stack.pushed);
    out.num(stack.entries.len());
    for (id, u) in &stack.entries {
        out.num(id.0);
        out.item(u);
    }
    out.0
}

fn decode(data: &[u8]) -> Option<UndoStack> {
    let mut fields = data.strip_suffix(b"\0")?.split(|&b| b == 0);
    let mut i = In(&mut fields);
    if i.field()? != MAGIC {
        return None;
    }
    let (epoch, pushed) = (i.num()?, i.num()?);
    let entries = (0..i.num::<usize>()?)
        .map(|_| Some((EntryId(i.num()?), i.item()?)))
        .collect::<Option<_>>()?;
    fields.next().is_none().then_some(UndoStack {
        entries,
        epoch,
        pushed,
    })
}

struct Out(Vec<u8>);

impl Out {
    fn field(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
        self.0.push(0);
    }

    fn num(&mut self, n: impl std::fmt::Display) {
        self.field(n.to_string().as_bytes())
    }

    fn path(&mut self, p: &Path) {
        self.field(p.as_os_str().as_bytes())
    }

    fn opt<T: std::fmt::Display>(&mut self, n: Option<T>) {
        match n {
            Some(n) => self.num(n),
            None => self.field(b"-"),
        }
    }

    fn id(&mut self, id: Option<FileId>) {
        self.opt(id.map(|(dev, ino)| format!("{dev}:{ino}")))
    }

    fn item(&mut self, u: &Undoable) {
        match u {
            Undoable::Move {
                from,
                to,
                listed,
                id,
            } => {
                self.field(b"move");
                self.path(from);
                self.path(to);
                self.num(u8::from(*listed));
                self.id(*id);
            }
            Undoable::Rename { from, to, id } => {
                self.field(b"rename");
                self.path(from);
                self.path(to);
                self.id(*id);
            }
            Undoable::Trash(t) => {
                self.field(b"trash");
                self.path(&t.original);
                self.field(t.name.as_bytes());
                self.path(&t.files);
                self.path(&t.info);
                self.opt(t.deleted);
            }
            Undoable::Copied(p, id) => {
                self.field(b"copied");
                self.path(p);
                self.id(*id);
            }
            Undoable::New(p, id) => {
                self.field(b"new");
                self.path(p);
                self.id(*id);
            }
            Undoable::Batch(items) => {
                self.field(b"batch");
                self.num(items.len());
                items.iter().for_each(|i| self.item(i));
            }
        }
    }
}

struct In<'a, 'b>(&'a mut dyn Iterator<Item = &'b [u8]>);

impl<'b> In<'_, 'b> {
    fn field(&mut self) -> Option<&'b [u8]> {
        self.0.next()
    }

    fn text(&mut self) -> Option<&'b str> {
        std::str::from_utf8(self.field()?).ok()
    }

    fn num<T: std::str::FromStr>(&mut self) -> Option<T> {
        self.text()?.parse().ok()
    }

    fn path(&mut self) -> Option<PathBuf> {
        let p = PathBuf::from(OsString::from_vec(self.field()?.to_vec()));
        p.is_absolute().then_some(p)
    }

    /// `Some(None)` for `-`.
    fn opt<T: std::str::FromStr>(&mut self) -> Option<Option<T>> {
        match self.text()? {
            "-" => Some(None),
            n => n.parse().ok().map(Some),
        }
    }

    fn id(&mut self) -> Option<Option<FileId>> {
        match self.opt::<String>()? {
            None => Some(None),
            Some(s) => {
                let (dev, ino) = s.split_once(':')?;
                Some(Some((dev.parse().ok()?, ino.parse().ok()?)))
            }
        }
    }

    fn item(&mut self) -> Option<Undoable> {
        Some(match self.field()? {
            b"move" => Undoable::Move {
                from: self.path()?,
                to: self.path()?,
                listed: self.num::<u8>()? == 1,
                id: self.id()?,
            },
            b"rename" => Undoable::Rename {
                from: self.path()?,
                to: self.path()?,
                id: self.id()?,
            },
            b"trash" => Undoable::Trash(Trashed {
                original: self.path()?,
                name: self.text()?.to_string(),
                files: self.path()?,
                info: self.path()?,
                deleted: self.opt()?,
            }),
            b"copied" => Undoable::Copied(self.path()?, self.id()?),
            b"new" => Undoable::New(self.path()?, self.id()?),
            b"batch" => Undoable::Batch(
                (0..self.num::<usize>()?)
                    .map(|_| self.item())
                    .collect::<Option<_>>()?,
            ),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn two_windows() -> (tempfile::TempDir, SharedUndo, SharedUndo) {
        let t = tempfile::tempdir().unwrap();
        let file = t.path().join("reprise-undo");
        (t, SharedUndo::at(file.clone()), SharedUndo::at(file))
    }

    fn path_of(undo: &Undo) -> PathBuf {
        match undo.steps() {
            [Undoable::Copied(p, _) | Undoable::New(p, _) | Undoable::Rename { to: p, .. }] => {
                p.clone()
            }
            s => panic!("one step: {s:?}"),
        }
    }

    #[test]
    fn two_windows_undo_one_history_newest_first() {
        let (_t, mut a, mut b) = two_windows();
        a.push(Undoable::Copied("/a/copy".into(), None));
        b.push(Undoable::Rename {
            from: "/b/old".into(),
            to: "/b/new".into(),
            id: None,
        });
        a.push(Undoable::New("/a/New Folder".into(), None));
        assert_eq!(b.label(), Some("Undo New"), "b sees a's latest");

        let order: Vec<PathBuf> =
            std::iter::from_fn(|| a.take().map(|(u, _)| path_of(&u))).collect();
        assert_eq!(
            order,
            ["/a/New Folder", "/b/new", "/a/copy"].map(PathBuf::from)
        );
        assert_eq!(b.label(), None);
    }

    #[test]
    fn a_second_undo_does_nothing_while_one_runs() {
        let (_t, mut a, mut b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        b.push(Undoable::Copied("/y".into(), None));
        let (undo, running) = a.take().expect("a takes the newest");
        assert!(b.take().is_none(), "b waits for a's undo");
        assert!(a.take().is_none(), "so does a second Undo in a");
        assert_eq!(b.label(), Some("Undo Copy"), "the entry below stays");
        a.put_back(undo, 1, None);
        drop(running);
        let (undo, _) = b.take().expect("b's turn");
        assert_eq!(path_of(&undo), Path::new("/x"));
    }

    #[test]
    fn what_an_undo_leaves_goes_below_what_another_window_pushed_since() {
        let (_t, mut a, mut b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let (undo, running) = a.take().unwrap();
        b.push(Undoable::New("/newer".into(), None));
        let left = a.put_back(undo, 0, None).expect("a cancel keeps it");
        drop(running);
        assert_eq!(path_of(&b.take().unwrap().0), Path::new("/newer"));
        let (undo, _) = b.take_entry(left).expect("ids hold across windows");
        assert_eq!(path_of(&undo), Path::new("/x"));
    }

    #[test]
    fn a_clear_in_one_window_drops_an_undo_another_has_under_way() {
        let (_t, mut a, mut b) = two_windows();
        a.push(Undoable::Copied("/x".into(), None));
        let (undo, running) = a.take().unwrap();
        b.clear();
        assert_eq!(a.put_back(undo, 0, None), None);
        drop(running);
        assert_eq!(b.label(), None);
    }

    #[test]
    fn every_kind_of_entry_reads_back_as_written() {
        let (_t, mut a, b) = two_windows();
        let odd = PathBuf::from(OsStr::from_bytes(b"/d/caf\xe9\nnew"));
        let entries = vec![
            Undoable::Move {
                from: "/a/x".into(),
                to: odd.clone(),
                listed: true,
                id: Some((8, u64::MAX)),
            },
            Undoable::Rename {
                from: "/a/y".into(),
                to: "/a/z".into(),
                id: None,
            },
            Undoable::Trash(Trashed {
                original: "/a/t.txt".into(),
                name: "t.txt".into(),
                files: "/T/files/t.txt".into(),
                info: "/T/info/t.txt.trashinfo".into(),
                deleted: Some(-5),
            }),
            Undoable::Batch(vec![
                Undoable::Copied("/a/c".into(), Some((1, 2))),
                Undoable::New("/a/n".into(), None),
            ]),
        ];
        for u in entries.clone() {
            a.push(u);
        }
        a.clear();
        for u in entries {
            a.push(u);
        }
        let s = &b.seen;
        b.label();
        let read = &s.borrow().stack;
        let written = &a.seen.borrow().stack;
        assert_eq!(
            format!("{:?}", read.entries),
            format!("{:?}", written.entries)
        );
        assert_eq!((read.epoch, read.pushed), (1, 8));
    }

    #[test]
    fn an_unreadable_history_starts_over() {
        let (t, mut a, _) = two_windows();
        std::fs::write(t.path().join("reprise-undo"), b"reprise-undo 1\0x\0").unwrap();
        assert_eq!(a.label(), None);
        a.push(Undoable::Copied("/x".into(), None));
        assert_eq!(a.label(), Some("Undo Copy"));
    }
}
