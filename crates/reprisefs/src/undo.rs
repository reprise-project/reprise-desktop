//! The undo behind the Edit > Undo row: reversing the last move, rename, trash or creation. The
//! host takes the top entry ([`SharedUndo::take`]) and reverses its steps on a removal job
//! ([`crate::removal::Removal::Undo`]).

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::trash::{self, Trashed};

mod store;
pub use store::{Running, SharedUndo};

#[derive(Clone, Debug)]
pub enum Undoable {
    /// A move into another folder. `listed` when `from`'s folder's
    /// `.hidden` named the item and let go of it with the move
    /// ([`crate::jobs::PutEvent::Landed`]'s `unlisted`): undoing it lists
    /// the item there again. `id` is the moved item's [`file_id`] at `to`.
    Move {
        from: PathBuf,
        to: PathBuf,
        listed: bool,
        id: Option<FileId>,
    },
    /// A rename, with the [`file_id`] of the item at `to` after it: undoing it refuses a different
    /// item that took the name since.
    Rename {
        from: PathBuf,
        to: PathBuf,
        id: Option<FileId>,
    },
    Trash(Trashed),
    /// A copy or a shortcut that landed at the path, as it landed ([`landed`]). Undoing it trashes
    /// it, a copied folder with everything in it, and refuses one changed since or one whose
    /// landing could not be read (`None`).
    Copied(PathBuf, Option<Landed>),
    /// A New Folder or New Text Document, with its [`file_id`] when it was made. Undoing it
    /// trashes a file and removes a folder only while it is empty, and refuses a different item
    /// that took the path since.
    New(PathBuf, Option<FileId>),
    Batch(Vec<Undoable>),
}

impl Undoable {
    fn label(&self) -> Option<&'static str> {
        Some(match self {
            Undoable::Move { .. } => "Undo Move",
            Undoable::Rename { .. } => "Undo Rename",
            Undoable::Trash(_) => "Undo Delete",
            Undoable::Copied(..) => "Undo Copy",
            Undoable::New(..) => "Undo New",
            Undoable::Batch(items) => return items.last()?.label(),
        })
    }

    /// Whether it renamed an item of `dir` within `dir` or made one there, so reversing it renames,
    /// removes or trashes that item alone.
    pub fn only_in(&self, dir: &Path) -> bool {
        match self {
            Undoable::Rename { from, to, .. } => {
                from.parent() == Some(dir) && to.parent() == Some(dir)
            }
            Undoable::New(p, _) => p.parent() == Some(dir),
            _ => false,
        }
    }

    /// Add to `out` the folders reversing this adds an entry to or takes
    /// one from, each once.
    fn dirs(&self, out: &mut Vec<PathBuf>) {
        let paths: Vec<&Path> = match self {
            Undoable::Move { from, to, .. } | Undoable::Rename { from, to, .. } => vec![from, to],
            Undoable::Trash(t) => vec![&t.original],
            Undoable::Copied(p, _) | Undoable::New(p, _) => vec![p],
            Undoable::Batch(items) => {
                items.iter().for_each(|i| i.dirs(out));
                return;
            }
        };
        for dir in paths.into_iter().filter_map(Path::parent) {
            if !out.iter().any(|d| d == dir) {
                out.push(dir.to_path_buf());
            }
        }
    }

    /// Whether this entry survives dropping the `Trash` entries `gone`
    /// picks: a batch keeps the members that survive and survives while
    /// any is left; every other kind survives.
    fn retain_trash(&mut self, gone: &impl Fn(&Trashed) -> bool) -> bool {
        match self {
            Undoable::Trash(t) => !gone(t),
            Undoable::Batch(items) => {
                items.retain_mut(|i| i.retain_trash(gone));
                !items.is_empty()
            }
            _ => true,
        }
    }

    /// Reverse it; a batch last member first, stopping at the first failure. `bins` are where an
    /// undone copy or new file goes; the `files/` path of each item their cap erases to make room
    /// joins `purged`. An item its bin refuses ([`NoBin`]) fails the reversal unless `erase`, which
    /// removes it for good: a copy with everything in it, a new file alone.
    pub(crate) fn reverse(
        &self,
        bins: Option<&trash::Bins>,
        erase: bool,
        purged: &mut Vec<PathBuf>,
    ) -> io::Result<()> {
        let mut trash_in = |path: &Path, remove: fn(&Path) -> io::Result<()>| {
            let bins =
                bins.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no trash directory"))?;
            match bins.trash(path) {
                Ok((_, gone)) => {
                    purged.extend(gone);
                    Ok(())
                }
                Err(e) if NoBin::refuses(&e) && erase => remove(path),
                Err(e) if NoBin::refuses(&e) => Err(io::Error::new(
                    e.kind(),
                    NoBin {
                        path: path.to_path_buf(),
                        reason: e,
                    },
                )),
                Err(e) => Err(e),
            }
        };
        match self {
            Undoable::Move {
                from,
                to,
                listed,
                id,
            } => {
                same_item(to, *id)?;
                crate::hidden::rename(to, from)?;
                if *listed && let Err(e) = crate::hidden::list(from) {
                    log::warn!(
                        "reprisefs: moved {} back, but could not hide it: {e}",
                        from.display()
                    );
                }
                Ok(())
            }
            Undoable::Rename { from, to, id } => {
                same_item(to, *id)?;
                crate::hidden::rename(to, from)
            }
            Undoable::Trash(t) => {
                let now = trash::trashed(&t.files)?;
                if (&now.original, now.deleted) != (&t.original, t.deleted) {
                    return Err(replaced(&t.files));
                }
                trash::restore(&now)
            }
            Undoable::Copied(path, landed) => {
                let name = crate::bincap::label(path);
                let Some(l) = landed else {
                    return Err(io::Error::other(format!(
                        "'{name}' could not be read when it was copied, so it may have changed since"
                    )));
                };
                same_item(path, Some(l.id))?;
                if newest_change(path)? > l.newest {
                    return Err(io::Error::other(format!(
                        "'{name}' has changed since it was copied"
                    )));
                }
                trash_in(path, crate::ops::erase)
            }
            // A folder goes only while empty and a file goes to the trash,
            // so nothing the user put there since is lost.
            Undoable::New(path, id) => {
                let meta = same_item(path, *id)?;
                if meta.is_dir() {
                    if std::fs::read_dir(path)?.next().is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::DirectoryNotEmpty,
                            "the folder is not empty",
                        ));
                    }
                    std::fs::remove_dir(path)
                } else {
                    // Not `ops::erase`: a folder swapped in since the check must not be emptied.
                    trash_in(path, |p| std::fs::remove_file(p))
                }
            }
            Undoable::Batch(items) => items
                .iter()
                .rev()
                .try_for_each(|i| i.reverse(bins, erase, purged)),
        }
    }

    /// The single steps that reverse it, in the order they run: a batch's members last first.
    fn into_steps(self, out: &mut Vec<Undoable>) {
        match self {
            Undoable::Batch(items) => items.into_iter().rev().for_each(|i| i.into_steps(out)),
            step => out.push(step),
        }
    }
}

/// An entry taken off the [`UndoStack`] to reverse step by step.
pub struct Undo {
    id: EntryId,
    steps: Vec<Undoable>,
    batch: bool,
    /// The entries below it when it was taken, and the stack's [`UndoStack::clear`] count then.
    below: Vec<EntryId>,
    epoch: u64,
}

impl Undo {
    /// The steps, in the order they are reversed.
    pub fn steps(&self) -> &[Undoable] {
        &self.steps
    }

    /// The folders reversing it adds an entry to or takes one from, each once.
    pub fn dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        self.steps.iter().for_each(|s| s.dirs(&mut out));
        out
    }

    /// Reverses the steps in order up to the first failure ([`Undoable::reverse`]): how many went,
    /// that failure, and the `files/` paths a bin's cap erased to make room.
    pub fn reverse(
        &self,
        bins: Option<&trash::Bins>,
        erase: bool,
    ) -> (usize, Option<io::Error>, Vec<PathBuf>) {
        let mut purged = Vec::new();
        let mut done = 0;
        for step in &self.steps {
            match step.reverse(bins, erase, &mut purged) {
                Ok(()) => done += 1,
                Err(e) => return (done, Some(e), purged),
            }
        }
        (done, None, purged)
    }

    /// What goes back on the stack once the first `done` steps were reversed and the next one
    /// failed with `error`, or a cancel stopped there (`None`), with where it goes ([`Undo::left`]).
    fn leftover(mut self, done: usize, error: Option<&io::Error>) -> Option<Leftover> {
        let (id, epoch, below) = (self.id, self.epoch, std::mem::take(&mut self.below));
        let asked = error.and_then(NoBin::of).is_some();
        let item = self.left(done, error)?;
        Some(Leftover {
            id,
            item,
            below,
            epoch,
            asked,
        })
    }

    /// What goes back on the stack once the first `done` steps were reversed and the next one
    /// failed with `error`, or a cancel stopped there (`None`). A step its Recycle Bin refused
    /// ([`NoBin`]) keeps the rest for an Undo that erases, a cancel keeps it to try again, and
    /// another failure keeps a batch's rest and drops a single step.
    pub fn left(self, done: usize, error: Option<&io::Error>) -> Option<Undoable> {
        let keep = match error {
            Some(e) => NoBin::of(e).is_some() || self.batch,
            None => true,
        };
        let mut rest: Vec<Undoable> = self.steps.into_iter().skip(done).collect();
        if rest.is_empty() || !keep {
            return None;
        }
        rest.reverse();
        match self.batch {
            true => Some(Undoable::Batch(rest)),
            false => rest.pop(),
        }
    }
}

/// What an undo left, on its way back to the stack ([`UndoStack::restore`]).
#[derive(Clone)]
struct Leftover {
    id: EntryId,
    item: Undoable,
    below: Vec<EntryId>,
    epoch: u64,
    /// Its Recycle Bin refused it ([`NoBin`]): it goes on top, where the question about it is.
    asked: bool,
}

/// Why Undo stopped at a copied or new item: its Recycle Bin refuses it (its drive removes files
/// immediately, or it is too big). The entry stays on the stack for an Undo that erases.
#[derive(Debug)]
pub struct NoBin {
    pub path: PathBuf,
    reason: io::Error,
}

impl NoBin {
    /// Whether [`trash::Bins::trash`] refused with nothing moved for one of those reasons.
    fn refuses(e: &io::Error) -> bool {
        matches!(
            e.kind(),
            io::ErrorKind::Unsupported | io::ErrorKind::FileTooLarge
        )
    }

    pub fn of(e: &io::Error) -> Option<&NoBin> {
        e.get_ref()?.downcast_ref()
    }

    /// The question before Undo erases: about this item, or about every item the entry still
    /// removes when `many`.
    pub fn question(&self, many: bool) -> String {
        if many {
            return "The items Undo removes cannot go to the Recycle Bin. Do you want to delete them permanently?".to_string();
        }
        let name = crate::bincap::label(&self.path);
        match self.reason.kind() {
            io::ErrorKind::FileTooLarge => crate::bincap::too_big_text(&name),
            _ => format!(
                "'{name}' is on a drive that removes files immediately. Do you want to delete it permanently?"
            ),
        }
    }
}

impl std::fmt::Display for NoBin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.reason.fmt(f)
    }
}

impl std::error::Error for NoBin {}

/// An item's (device, inode), told apart from a different item that later takes its path.
pub type FileId = (u64, u64);

/// `path`'s own metadata, unless the item is gone or `id` names another item than the one there
/// now: it was moved, deleted or replaced since, so reversing it would act on the wrong file.
fn same_item(path: &Path, id: Option<FileId>) -> io::Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "'{}' was moved or deleted since",
                crate::bincap::label(path)
            ),
        ),
        _ => e,
    })?;
    if id.is_some_and(|id| id != (meta.dev(), meta.ino())) {
        return Err(replaced(path));
    }
    Ok(meta)
}

fn replaced(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "a different item has the name '{}' now",
            crate::bincap::label(path)
        ),
    )
}

/// A copy as it landed: its [`FileId`], and the newest change or modification time in it then
/// ([`newest_change`]), which anything added, edited or removed in it since passes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Landed {
    pub id: FileId,
    pub newest: (i64, i64),
}

/// `path`'s [`Landed`], for a folder read through everything in it; None when any of it cannot
/// be read.
pub fn landed(path: &Path) -> Option<Landed> {
    Some(Landed {
        id: file_id(path)?,
        newest: newest_change(path).ok()?,
    })
}

/// The newest change or modification time, as seconds and nanoseconds, of `path` and of
/// everything in it. A copy sets the times it copies before it lands, and on some drives the
/// change time is the creation time, so both count.
fn newest_change(path: &Path) -> io::Result<(i64, i64)> {
    let mut newest = (i64::MIN, 0);
    let mut todo = vec![path.to_path_buf()];
    while let Some(p) = todo.pop() {
        let m = std::fs::symlink_metadata(&p)?;
        newest = newest
            .max((m.ctime(), m.ctime_nsec()))
            .max((m.mtime(), m.mtime_nsec()));
        if m.is_dir() {
            for e in std::fs::read_dir(&p)? {
                todo.push(e?.path());
            }
        }
    }
    Ok(newest)
}

/// `path`'s own [`FileId`], a link's rather than its target's.
pub fn file_id(path: &Path) -> Option<FileId> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// One entry of an [`UndoStack`], the same while what an undo left of it is put back: a question
/// about it names it, so its answer never reaches a newer entry. `history` is the stack's
/// [`UndoStack::nonce`], so no id outlives the history it was made in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId {
    history: u64,
    n: u64,
}

/// How many entries the history keeps; a push past it drops the oldest.
pub const CAP: usize = 100;

#[derive(Clone)]
pub struct UndoStack {
    entries: Vec<(EntryId, Undoable)>,
    /// Random, picked when the history starts: one that starts over never takes an answer, an
    /// id or a clear count meant for the one before.
    nonce: u64,
    epoch: u64,
    pushed: u64,
}

impl Default for UndoStack {
    fn default() -> Self {
        use std::hash::{BuildHasher, Hasher};
        UndoStack {
            entries: Vec::new(),
            nonce: std::hash::RandomState::new().build_hasher().finish(),
            epoch: 0,
            pushed: 0,
        }
    }
}

impl UndoStack {
    pub fn push(&mut self, u: Undoable) {
        self.pushed += 1;
        let id = EntryId {
            history: self.nonce,
            n: self.pushed,
        };
        self.entries.push((id, u));
        self.trim();
    }

    fn trim(&mut self) {
        let over = self.entries.len().saturating_sub(CAP);
        self.entries.drain(..over);
    }

    /// Drops every entry: an action Undo cannot reverse ran, so Undo must not reach past it.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.epoch += 1;
    }

    /// Push a whole operation's worth of undos as one entry: nothing for
    /// an empty batch, the operation itself for a single one.
    pub fn push_all(&mut self, mut batch: Vec<Undoable>) {
        match batch.len() {
            0 => {}
            1 => self.push(batch.remove(0)),
            _ => self.push(Undoable::Batch(batch)),
        }
    }

    /// Drop every undo that would put a file back from the trash. Emptying
    /// the trash makes a `Trash` entry unreversible but not a move, rename
    /// or creation; a batch keeps whatever survives.
    pub fn retain_non_trash(&mut self) {
        self.entries.retain_mut(|(_, u)| u.retain_trash(&|_| true));
    }

    /// Drop the undos that would put back an item that has left the
    /// trash from `files` (its path under `files/`), restored or deleted
    /// for good, so it is no longer there to put back.
    pub fn forget_gone(&mut self, files: &[PathBuf]) {
        self.entries
            .retain_mut(|(_, u)| u.retain_trash(&|t| files.contains(&t.files)));
    }

    pub fn label(&self) -> Option<&'static str> {
        self.entries.last()?.1.label()
    }

    /// The most recent entry, off the stack. What its reversal leaves undone goes back with
    /// [`UndoStack::put_back`], and the bin items it purged with [`UndoStack::forget_gone`].
    pub fn take(&mut self) -> Option<Undo> {
        let (id, entry) = self.entries.pop()?;
        let batch = matches!(entry, Undoable::Batch(_));
        let mut steps = Vec::new();
        entry.into_steps(&mut steps);
        Some(Undo {
            id,
            steps,
            batch,
            below: self.entries.iter().map(|(e, _)| *e).collect(),
            epoch: self.epoch,
        })
    }

    /// [`UndoStack::take`] when the top entry is still `id`, the one a question asked about.
    pub fn take_entry(&mut self, id: EntryId) -> Option<Undo> {
        match self.entries.last() {
            Some((top, _)) if *top == id => self.take(),
            _ => None,
        }
    }

    /// What `undo` left once its first `done` steps were reversed ([`Undo::leftover`]), back
    /// where it was taken from ([`UndoStack::restore`]). Returns the entry put back.
    pub fn put_back(
        &mut self,
        undo: Undo,
        done: usize,
        error: Option<&io::Error>,
    ) -> Option<EntryId> {
        self.restore(undo.leftover(done, error)?)
    }

    /// `left` back where it was taken from: above the highest entry that was below it then, under
    /// the entries pushed since. Nothing after a [`UndoStack::clear`] or once the history started
    /// over, or when the cap drops it. What its Recycle Bin refused ([`NoBin`]) goes on top
    /// instead, since the question about it is up now. An entry already back, from a write that
    /// landed though it reported a failure, stays as it is.
    fn restore(&mut self, left: Leftover) -> Option<EntryId> {
        let id = left.id;
        if (id.history, left.epoch) != (self.nonce, self.epoch) {
            return None;
        }
        if self.entries.iter().any(|(e, _)| *e == id) {
            return Some(id);
        }
        let at = match left.asked {
            true => self.entries.len(),
            false => (self.entries.iter())
                .rposition(|(e, _)| left.below.contains(e))
                .map_or(0, |i| i + 1),
        };
        self.entries.insert(at, (id, left.item));
        self.trim();
        self.entries.iter().any(|(e, _)| *e == id).then_some(id)
    }

    /// Whether the entry Undo would reverse next holds more than one item.
    pub fn many(&self) -> bool {
        matches!(self.entries.last(), Some((_, Undoable::Batch(items))) if items.len() > 1)
    }

    /// The most recent entry reversed here, as a removal job reverses it: `None` when the stack is
    /// empty, else the first failure.
    #[cfg(test)]
    fn undo_in(&mut self, bins: Option<&trash::Bins>, erase: bool) -> Option<io::Result<()>> {
        let undo = self.take()?;
        let (done, error, purged) = undo.reverse(bins, erase);
        self.put_back(undo, done, error.as_ref());
        self.forget_gone(&purged);
        Some(error.map_or(Ok(()), Err))
    }

    #[cfg(test)]
    fn undo(&mut self) -> Option<io::Result<()>> {
        self.undo_in(trash::Bins::system().as_ref(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn undo_of_empty_stack_is_none() {
        let mut u = UndoStack::default();
        assert!(u.undo().is_none());
        assert_eq!(u.label(), None);
    }

    #[test]
    fn undo_dirs_name_each_folder_the_undo_touches_once() {
        let mut u = UndoStack::default();
        u.push(Undoable::New("/c/New Folder".into(), None));
        u.push_all(vec![
            Undoable::Move {
                from: "/a/x".into(),
                to: "/b/x".into(),
                listed: false,
                id: None,
            },
            Undoable::Move {
                from: "/a/y".into(),
                to: "/b/y".into(),
                listed: false,
                id: None,
            },
        ]);
        let dirs = |u: &mut UndoStack| u.take().unwrap().dirs();
        assert_eq!(dirs(&mut u), [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(dirs(&mut u), [PathBuf::from("/c")]);
    }

    #[test]
    fn undo_reverses_a_rename() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("old.txt");
        let to = t.path().join("new.txt");
        fs::write(&from, "hi").unwrap();
        fs::rename(&from, &to).unwrap();
        fs::write(t.path().join(".hidden"), "new.txt\n").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Rename {
            from: from.clone(),
            to: to.clone(),
            id: file_id(&to),
        });
        assert_eq!(u.label(), Some("Undo Rename"));

        u.undo().unwrap().unwrap();

        assert!(from.exists() && !to.exists());
        assert!(
            crate::hidden::is_listed(&from),
            "the old name is hidden again"
        );
        assert_eq!(u.label(), None);
    }

    #[test]
    fn undo_reverses_a_trash() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let trashed = trash::trash_in(&root, &src).unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Trash(trashed));
        assert_eq!(u.label(), Some("Undo Delete"));

        u.undo().unwrap().unwrap();

        assert_eq!(fs::read_to_string(&src).unwrap(), "hi");
    }

    #[test]
    fn retain_non_trash_keeps_everything_a_trashed_file_does_not_hold() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let path = |n: &str| t.path().join(n);
        let trashed = |n: &str| {
            fs::write(path(n), "hi").unwrap();
            Undoable::Trash(trash::trash_in(&root, &path(n)).unwrap())
        };
        let mut u = UndoStack::default();
        u.push(Undoable::Copied(path("made.txt"), None));
        u.push(trashed("a.txt"));
        fs::write(path("new.txt"), "hi").unwrap();
        u.push(Undoable::Batch(vec![
            trashed("b.txt"),
            Undoable::Rename {
                from: path("old.txt"),
                to: path("new.txt"),
                id: None,
            },
        ]));
        u.push(Undoable::Batch(vec![trashed("c.txt"), trashed("d.txt")]));

        u.retain_non_trash();

        assert_eq!(u.label(), Some("Undo Rename"));
        u.undo_in(None, false);
        assert_eq!(u.label(), Some("Undo Copy"), "the copy is still here");
        u.undo_in(None, false);
        assert_eq!(u.label(), None);
    }

    #[test]
    fn an_undo_that_purges_the_bin_forgets_the_purged_item() {
        let t = tempfile::tempdir().unwrap();
        let mut bins = trash::Bins::new(t.path().join("Trash"), Vec::new());
        bins.policy.global.max_percent = 1;
        let cap = bins
            .policy
            .global
            .cap(crate::bincap::drive_size(&bins.home));
        let item = |n: &str| {
            let p = t.path().join(n);
            fs::File::create(&p).unwrap().set_len(cap * 2 / 5).unwrap();
            p
        };
        let mut u = UndoStack::default();
        let (a, _) = bins.trash(&item("a")).unwrap();
        u.push(Undoable::Trash(a));
        let (b, _) = bins.trash(&item("b")).unwrap();
        u.push(Undoable::Trash(b.clone()));
        let c = item("c");
        u.push(Undoable::Copied(c.clone(), landed(&c)));

        u.undo_in(Some(&bins), false).unwrap().unwrap();

        assert!(!t.path().join("Trash/files/a").exists(), "a made room");
        assert_eq!(u.entries.len(), 1, "a's undo went with it");
        assert!(matches!(&u.entries[0].1, Undoable::Trash(t) if *t == b));
    }

    #[test]
    fn forget_gone_drops_only_the_items_that_left_the_trash() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let path = |n: &str| t.path().join(n);
        for n in ["a.txt", "b.txt"] {
            fs::write(path(n), n).unwrap();
        }
        let a = trash::trash_in(&root, &path("a.txt")).unwrap();
        let b = trash::trash_in(&root, &path("b.txt")).unwrap();
        let restored = a.files.clone();
        let mut u = UndoStack::default();
        u.push(Undoable::Batch(vec![
            Undoable::Trash(a),
            Undoable::Trash(b),
        ]));

        trash::restore_free(&trash::trashed(&restored).unwrap()).unwrap();
        u.forget_gone(&[restored]);

        u.undo_in(None, false).unwrap().unwrap();
        assert_eq!(fs::read_to_string(path("b.txt")).unwrap(), "b.txt");
        assert_eq!(u.label(), None);
    }

    #[test]
    fn undo_of_a_move_uses_the_from_to_pair() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("a").join("x.txt");
        let to = t.path().join("b").join("x.txt");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::write(&from, "hi").unwrap();
        fs::rename(&from, &to).unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Move {
            from: from.clone(),
            to: to.clone(),
            listed: false,
            id: None,
        });

        u.undo().unwrap().unwrap();

        assert!(from.exists() && !to.exists());
    }

    #[test]
    fn undo_of_a_move_refuses_an_item_gone_or_replaced_at_its_destination() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("a.txt"), t.path().join("b.txt"));
        fs::write(&to, "moved").unwrap();
        let mut u = UndoStack::default();
        let moved = Undoable::Move {
            from: from.clone(),
            to: to.clone(),
            listed: false,
            id: file_id(&to),
        };
        fs::rename(&to, t.path().join("c.txt")).unwrap();
        u.push(moved.clone());
        let err = u.undo().unwrap().unwrap_err();
        assert_eq!(err.to_string(), "'b.txt' was moved or deleted since");

        fs::write(&to, "saved by another program").unwrap();
        u.push(moved);
        let err = u.undo().unwrap().unwrap_err();
        assert_eq!(err.to_string(), "a different item has the name 'b.txt' now");
        assert!(!from.exists());
        assert_eq!(fs::read_to_string(&to).unwrap(), "saved by another program");
    }

    #[test]
    fn undo_of_a_copy_never_erases_a_different_item_that_took_its_path() {
        let t = tempfile::tempdir().unwrap();
        let copy = t.path().join("copy.txt");
        fs::write(&copy, "copied").unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::Copied(copy.clone(), landed(&copy)));
        // Kept under another name so the new file can't reuse the copy's inode.
        fs::rename(&copy, t.path().join("old.txt")).unwrap();
        fs::write(&copy, "saved by another program").unwrap();

        let err = u.undo_in(None, true).unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            fs::read_to_string(&copy).unwrap(),
            "saved by another program"
        );
    }

    #[test]
    fn undo_of_a_copy_whose_landing_could_not_be_read_touches_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Photos");
        let locked = dir.join("locked");
        fs::create_dir_all(&locked).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let landing = landed(&dir);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        // SAFETY: geteuid takes no arguments and cannot fail.
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(landing, None, "a folder in it could not be read");
        }
        fs::rename(&dir, t.path().join("Moved")).unwrap();
        fs::write(&dir, "saved by another program").unwrap();
        let mut bins = trash::Bins::new(t.path().join("Trash"), Vec::new());
        bins.policy.global.remove_immediately = true;
        let mut u = UndoStack::default();
        u.push(Undoable::Copied(dir.clone(), None));

        let err = u.undo_in(Some(&bins), true).unwrap().unwrap_err();

        assert_eq!(
            err.to_string(),
            "'Photos' could not be read when it was copied, so it may have changed since"
        );
        assert_eq!(
            fs::read_to_string(&dir).unwrap(),
            "saved by another program"
        );
        assert_eq!(u.label(), None, "the entry is dropped");
    }

    #[test]
    fn undo_of_a_trash_removes_only_the_record_its_bin_holds() {
        let t = tempfile::tempdir().unwrap();
        let src = t.path().join("x.txt");
        fs::write(&src, "hi").unwrap();
        let trashed = trash::trash_in(&t.path().join("Trash"), &src).unwrap();
        let other = t.path().join("other.txt");
        fs::write(&other, "keep").unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::Trash(Trashed {
            info: other.clone(),
            ..trashed.clone()
        }));

        u.undo().unwrap().unwrap();

        assert_eq!(fs::read_to_string(&src).unwrap(), "hi");
        assert_eq!(fs::read_to_string(&other).unwrap(), "keep");
        assert!(!trashed.info.exists());
    }

    #[test]
    fn undo_of_a_trash_refuses_a_different_item_in_its_bin_slot() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let (x, y) = (t.path().join("x"), t.path().join("y"));
        for d in [&x, &y] {
            fs::create_dir(d).unwrap();
            fs::write(d.join("a.txt"), d.to_string_lossy().as_bytes()).unwrap();
        }
        let first = trash::trash_in(&root, &x.join("a.txt")).unwrap();
        trash::restore(&first).unwrap();
        fs::remove_file(x.join("a.txt")).unwrap();
        let second = trash::trash_in(&root, &y.join("a.txt")).unwrap();
        assert_eq!(second.files, first.files, "the slot was free again");

        let mut u = UndoStack::default();
        u.push(Undoable::Trash(first));
        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!x.join("a.txt").exists() && second.files.exists());
    }

    #[test]
    fn undo_of_rename_fails_without_clobbering_a_name_taken_since() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("old.txt");
        let to = t.path().join("new.txt");
        fs::write(&from, "hi").unwrap();
        fs::rename(&from, &to).unwrap();
        fs::write(&from, "new file landed here").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Rename {
            from: from.clone(),
            to: to.clone(),
            id: file_id(&to),
        });

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "new file landed here");
        assert_eq!(fs::read_to_string(&to).unwrap(), "hi");
    }

    #[test]
    fn undo_of_rename_refuses_a_different_file_that_took_the_new_name() {
        let t = tempfile::tempdir().unwrap();
        let from = t.path().join("a.txt");
        let to = t.path().join("b.txt");
        fs::write(&from, "renamed").unwrap();
        fs::rename(&from, &to).unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::Rename {
            from: from.clone(),
            to: to.clone(),
            id: file_id(&to),
        });
        fs::rename(&to, t.path().join("c.txt")).unwrap();
        fs::write(&to, "saved by another program").unwrap();

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(!from.exists(), "nothing took the old name");
        assert_eq!(fs::read_to_string(&to).unwrap(), "saved by another program");
        assert_eq!(u.label(), None, "the entry is dropped");
    }

    #[test]
    fn undo_of_new_removes_the_empty_folder() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("New Folder");
        fs::create_dir(&dir).unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::New(dir.clone(), None));

        u.undo_in(None, false).unwrap().unwrap();

        assert!(!dir.exists());
    }

    #[test]
    fn undo_of_a_copied_folder_refuses_once_anything_in_it_changed_since() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let dir = t.path().join("Photos");
        fs::create_dir_all(dir.join("2024")).unwrap();
        fs::write(dir.join("2024/a.jpg"), "a").unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::Copied(dir.clone(), landed(&dir)));
        // Past the file system's time granularity.
        std::thread::sleep(std::time::Duration::from_millis(50));
        fs::write(dir.join("2024/mine.txt"), "added since").unwrap();

        let mut bins = trash::Bins::new(root.clone(), Vec::new());
        bins.policy.global.remove_immediately = true;
        let err = u.undo_in(Some(&bins), true).unwrap().unwrap_err();

        assert_eq!(err.to_string(), "'Photos' has changed since it was copied");
        assert_eq!(
            fs::read_to_string(dir.join("2024/mine.txt")).unwrap(),
            "added since"
        );
        assert!(!root.join("files").exists());
    }

    #[test]
    fn undo_of_a_copied_folder_trashes_it_with_its_contents() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let dir = t.path().join("Photos");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("a.jpg"), "a").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Copied(dir.clone(), landed(&dir)));
        assert_eq!(u.label(), Some("Undo Copy"));

        u.undo_in(Some(&trash::Bins::new(root.clone(), Vec::new())), false)
            .unwrap()
            .unwrap();

        assert!(!dir.exists());
        assert_eq!(
            fs::read_to_string(root.join("files/Photos/a.jpg")).unwrap(),
            "a"
        );
        assert_eq!(u.label(), None);
    }

    /// A copy onto a drive that removes files immediately is not erased by Undo unasked: Undo fails
    /// naming it, keeps the entry, and only Undo with erasing removes the copies, none into the bin.
    #[test]
    fn undo_of_a_copy_its_bin_refuses_waits_for_erasing() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let (a, b) = (t.path().join("a.txt"), t.path().join("b.txt"));
        fs::write(&a, "a").unwrap();
        fs::write(&b, "b").unwrap();
        let mut bins = trash::Bins::new(root.clone(), Vec::new());
        bins.policy.global.remove_immediately = true;

        let mut u = UndoStack::default();
        u.push_all(vec![
            Undoable::Copied(a.clone(), landed(&a)),
            Undoable::Copied(b.clone(), landed(&b)),
        ]);
        let err = u.undo_in(Some(&bins), false).unwrap().unwrap_err();

        let no_bin = NoBin::of(&err).expect("the bin refused it");
        assert_eq!(no_bin.path, b);
        assert_eq!(
            no_bin.question(false),
            "'b.txt' is on a drive that removes files immediately. Do you want to delete it permanently?"
        );
        assert!(a.exists() && b.exists(), "nothing erased unasked");
        assert_eq!(u.label(), Some("Undo Copy"), "the entry stays");
        assert!(u.many());

        u.undo_in(Some(&bins), true).unwrap().unwrap();
        assert!(!a.exists() && !b.exists());
        assert!(!root.join("files").exists(), "nothing went to the bin");
        assert_eq!(u.label(), None);
    }

    #[test]
    fn undo_of_new_trashes_the_file() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let file = t.path().join("New Text Document.txt");
        fs::write(&file, "typed since").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::New(file.clone(), None));
        assert_eq!(u.label(), Some("Undo New"));

        u.undo_in(Some(&trash::Bins::new(root.clone(), Vec::new())), false)
            .unwrap()
            .unwrap();

        assert!(!file.exists());
        assert_eq!(
            fs::read_to_string(root.join("files/New Text Document.txt")).unwrap(),
            "typed since"
        );
    }

    #[test]
    fn undo_of_new_refuses_a_folder_that_is_no_longer_empty() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("New Folder");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("notes.txt"), "mine").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::New(dir.clone(), None));

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::DirectoryNotEmpty);
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine");
    }

    #[test]
    fn undo_of_new_refuses_a_different_file_that_took_its_path() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let file = t.path().join("New Text Document.txt");
        fs::write(&file, "").unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::New(file.clone(), file_id(&file)));
        fs::rename(&file, t.path().join("Notes.txt")).unwrap();
        fs::write(&file, "saved by another program").unwrap();

        let err = u
            .undo_in(Some(&trash::Bins::new(root.clone(), Vec::new())), false)
            .unwrap()
            .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "saved by another program"
        );
        assert!(!root.join("files").exists(), "nothing went to the bin");
        assert_eq!(u.label(), None, "the entry is dropped");
    }

    #[test]
    fn what_an_undo_leaves_goes_back_below_newer_entries_and_not_past_a_clear() {
        let mut u = UndoStack::default();
        u.push(Undoable::Copied("/old".into(), None));
        u.push(Undoable::Copied("/x".into(), None));
        let undo = u.take().unwrap();
        u.push(Undoable::New("/newer".into(), None));
        u.put_back(undo, 0, None);
        let order: Vec<PathBuf> = std::iter::from_fn(|| u.take())
            .map(|t| match t.steps() {
                [Undoable::Copied(p, _) | Undoable::New(p, _)] => p.clone(),
                _ => panic!("one step"),
            })
            .collect();
        assert_eq!(order, ["/newer", "/x", "/old"].map(PathBuf::from));

        u.push(Undoable::Copied("/x".into(), None));
        let undo = u.take().unwrap();
        u.clear();
        u.push(Undoable::New("/after".into(), None));
        u.put_back(undo, 0, None);
        assert!(u.take().is_some());
        assert!(
            u.take().is_none(),
            "an entry from before the clear stays gone"
        );
    }

    #[test]
    fn the_history_keeps_the_newest_entries_and_an_undo_under_way_keeps_its_place() {
        let copied = |n: usize| Undoable::Copied(format!("/c{n}").into(), None);
        let path = |u: &Undo| match u.steps() {
            [Undoable::Copied(p, _)] => p.to_string_lossy().into_owned(),
            _ => panic!("one copy"),
        };
        let ids = |u: &UndoStack| u.entries.iter().map(|(e, _)| e.n).collect::<Vec<_>>();
        let mut u = UndoStack::default();
        for n in 1..=CAP + 5 {
            u.push(copied(n));
        }
        assert_eq!(
            ids(&u),
            (6..=105).collect::<Vec<u64>>(),
            "the five oldest went"
        );

        let top = u.take().unwrap();
        let next = u.take().unwrap();
        assert_eq!((path(&top), path(&next)), ("/c105".into(), "/c104".into()));
        for n in 1..=3 {
            u.push(copied(1000 + n));
        }
        let next_id = next.id;
        assert_eq!(u.put_back(next, 0, None), Some(next_id));
        assert_eq!(next_id.n, 104);
        let kept = ids(&u);
        assert_eq!(kept.len(), CAP);
        assert_eq!(kept[..2], [8, 9], "a push past the cap dropped 7 too");
        assert_eq!(kept[CAP - 6..], [102, 103, 104, 106, 107, 108]);
        let order: Vec<String> = std::iter::from_fn(|| u.take()).map(|t| path(&t)).collect();
        assert_eq!(order[..4], ["/c1003", "/c1002", "/c1001", "/c104"]);
        assert_eq!(order.last().map(String::as_str), Some("/c8"));

        u.push(copied(1));
        let oldest = u.take().unwrap();
        for n in 0..=CAP {
            u.push(copied(2000 + n));
        }
        assert_eq!(
            u.put_back(oldest, 0, None),
            None,
            "older than every entry the cap kept"
        );
        assert_eq!(u.entries.len(), CAP);
    }

    #[test]
    fn a_put_back_restored_twice_is_back_once() {
        let mut u = UndoStack::default();
        u.push(Undoable::Copied("/x".into(), None));
        let left = u.take().unwrap().leftover(0, None).unwrap();
        let id = left.id;
        assert_eq!(u.restore(left.clone()), Some(id));
        u.push(Undoable::New("/newer".into(), None));
        assert_eq!(u.restore(left), Some(id));
        let ids: Vec<EntryId> = u.entries.iter().map(|(e, _)| *e).collect();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], id, "it stays where the first put-back left it");
    }

    #[test]
    fn what_an_undo_leaves_goes_back_under_the_entries_pushed_since_whatever_left_below_it() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("a.txt"), "a").unwrap();
        let trashed = trash::trash_in(&t.path().join("Trash"), &t.path().join("a.txt")).unwrap();
        let mut u = UndoStack::default();
        u.push(Undoable::Trash(trashed.clone()));
        for p in ["/b", "/c", "/d"] {
            u.push(Undoable::Copied(p.into(), None));
        }
        let d = u.take().unwrap();
        u.push(Undoable::New("/e".into(), None));
        u.forget_gone(&[trashed.files]);
        u.put_back(d, 0, None);
        let order: Vec<PathBuf> = std::iter::from_fn(|| u.take())
            .map(|t| match t.steps() {
                [Undoable::Copied(p, _) | Undoable::New(p, _)] => p.clone(),
                s => panic!("one step: {s:?}"),
            })
            .collect();
        assert_eq!(order, ["/e", "/d", "/c", "/b"].map(PathBuf::from));
    }

    #[test]
    fn a_failed_batch_keeps_what_it_did_not_reverse() {
        let t = tempfile::tempdir().unwrap();
        let (from, to) = (t.path().join("old.txt"), t.path().join("new.txt"));
        fs::write(&from, "hi").unwrap();
        fs::rename(&from, &to).unwrap();
        let (moved_from, moved_to) = (t.path().join("a.txt"), t.path().join("b.txt"));
        fs::write(&moved_to, "moved").unwrap();
        fs::write(&moved_from, "in the way").unwrap();

        let mut u = UndoStack::default();
        u.push_all(vec![
            Undoable::Move {
                from: moved_from.clone(),
                to: moved_to.clone(),
                listed: false,
                id: None,
            },
            Undoable::Rename {
                from: from.clone(),
                to: to.clone(),
                id: file_id(&to),
            },
        ]);

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(from.exists() && !to.exists(), "the rename was reversed");
        assert_eq!(u.label(), Some("Undo Move"), "the rest is still here");

        fs::remove_file(&moved_from).unwrap();
        u.undo().unwrap().unwrap();

        assert_eq!(fs::read_to_string(&moved_from).unwrap(), "moved");
        assert_eq!(u.label(), None);
    }

    #[test]
    fn failed_undo_is_not_restored_to_the_stack() {
        let t = tempfile::tempdir().unwrap();
        let missing = t.path().join("gone.txt");

        let mut u = UndoStack::default();
        u.push(Undoable::New(missing, None));

        assert!(u.undo().unwrap().is_err());
        assert_eq!(u.label(), None, "a failed undo must not stay on the stack");
    }

    #[test]
    fn a_batch_is_pushed_and_undone_as_one_entry() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let mut batch = Vec::new();
        for name in ["a.txt", "b.txt"] {
            let src = t.path().join(name);
            fs::write(&src, name).unwrap();
            batch.push(Undoable::Trash(trash::trash_in(&root, &src).unwrap()));
        }

        let mut u = UndoStack::default();
        u.push_all(batch);
        assert_eq!(u.label(), Some("Undo Delete"));

        u.undo().unwrap().unwrap();

        assert_eq!(fs::read_to_string(t.path().join("a.txt")).unwrap(), "a.txt");
        assert_eq!(fs::read_to_string(t.path().join("b.txt")).unwrap(), "b.txt");
        assert_eq!(u.label(), None, "the batch was one entry");
    }

    #[test]
    fn a_cancelled_undo_keeps_what_it_did_not_reverse_and_a_failed_lone_step_goes() {
        let mut u = UndoStack::default();
        u.push(Undoable::Copied("/x".into(), None));
        let lone = u.take().expect("the copy");
        let kept = lone.left(0, None);
        assert!(
            matches!(kept, Some(Undoable::Copied(..))),
            "a cancel keeps it"
        );
        u.push(kept.unwrap());
        let failed = io::Error::new(io::ErrorKind::NotFound, "gone");
        let lone = u.take().expect("the copy");
        assert!(lone.left(0, Some(&failed)).is_none(), "a failure drops it");

        u.push_all(vec![
            Undoable::Copied("/a".into(), None),
            Undoable::Copied("/b".into(), None),
            Undoable::Copied("/c".into(), None),
        ]);
        let batch = u.take().expect("the batch");
        let paths: Vec<_> = (batch.steps().iter())
            .map(|s| match s {
                Undoable::Copied(p, _) => p.clone(),
                _ => panic!("a copy"),
            })
            .collect();
        assert_eq!(paths, ["/c", "/b", "/a"].map(PathBuf::from), "last first");
        match batch.left(1, None) {
            Some(Undoable::Batch(rest)) => assert!(matches!(
                rest.as_slice(),
                [Undoable::Copied(a, _), Undoable::Copied(b, _)] if *a == Path::new("/a") && *b == Path::new("/b")
            )),
            _ => panic!("the batch keeps /a and /b"),
        }
    }

    #[test]
    fn push_all_keeps_a_lone_operation_and_drops_an_empty_batch() {
        let mut u = UndoStack::default();
        u.push_all(Vec::new());
        assert_eq!(u.label(), None);
        u.push_all(vec![Undoable::Copied(PathBuf::from("/x"), None)]);
        assert_eq!(u.label(), Some("Undo Copy"));
    }
}
