//! The undo behind the Edit > Undo row: reversing the last move, rename, trash or creation. The
//! host takes the top entry ([`UndoStack::take`]) and reverses its steps on a removal job
//! ([`crate::removal::Removal::Undo`]).

use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::trash::{self, Trashed};

#[derive(Clone, Debug)]
pub enum Undoable {
    /// A move into another folder. `listed` when `from`'s folder's
    /// `.hidden` named the item and let go of it with the move
    /// ([`crate::jobs::PutEvent::Landed`]'s `unlisted`): undoing it lists
    /// the item there again.
    Move {
        from: PathBuf,
        to: PathBuf,
        listed: bool,
    },
    /// A rename, with the [`file_id`] of the item at `to` after it: undoing it refuses a different
    /// item that took the name since.
    Rename {
        from: PathBuf,
        to: PathBuf,
        id: Option<FileId>,
    },
    Trash(Trashed),
    /// A copy or a shortcut that landed at the path. Undoing it trashes
    /// it, a copied folder with everything in it.
    Copied(PathBuf),
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
            Undoable::Copied(_) => "Undo Copy",
            Undoable::New(..) => "Undo New",
            Undoable::Batch(items) => return items.last()?.label(),
        })
    }

    /// Add to `out` the folders reversing this adds an entry to or takes
    /// one from, each once.
    fn dirs(&self, out: &mut Vec<PathBuf>) {
        let paths: Vec<&Path> = match self {
            Undoable::Move { from, to, .. } | Undoable::Rename { from, to, .. } => vec![from, to],
            Undoable::Trash(t) => vec![&t.original],
            Undoable::Copied(p) | Undoable::New(p, _) => vec![p],
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
            Undoable::Move { from, to, listed } => {
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
            Undoable::Trash(t) => trash::restore(t),
            Undoable::Copied(path) => trash_in(path, crate::ops::erase),
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
    /// Where the entry sat on the stack, and the stack's [`UndoStack::clear`] count then.
    at: usize,
    epoch: u64,
}

impl Undo {
    /// The steps, in the order they are reversed.
    pub fn steps(&self) -> &[Undoable] {
        &self.steps
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

/// `path`'s own metadata, unless `id` names another item than the one there now.
fn same_item(path: &Path, id: Option<FileId>) -> io::Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path)?;
    if id.is_some_and(|id| id != (meta.dev(), meta.ino())) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "a different item has its name now",
        ));
    }
    Ok(meta)
}

/// `path`'s own [`FileId`], a link's rather than its target's.
pub fn file_id(path: &Path) -> Option<FileId> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    Some((meta.dev(), meta.ino()))
}

/// One entry of an [`UndoStack`], the same while what an undo left of it is put back: a question
/// about it names it, so its answer never reaches a newer entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryId(u64);

#[derive(Default)]
pub struct UndoStack {
    entries: Vec<(EntryId, Undoable)>,
    epoch: u64,
    pushed: u64,
}

impl UndoStack {
    pub fn push(&mut self, u: Undoable) {
        self.pushed += 1;
        self.entries.push((EntryId(self.pushed), u));
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

    pub fn undo_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some((_, u)) = self.entries.last() {
            u.dirs(&mut out);
        }
        out
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
            at: self.entries.len(),
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

    /// What `undo` left once its first `done` steps were reversed ([`Undo::left`]), back where
    /// it was taken from, below the entries pushed since; nothing after a [`UndoStack::clear`].
    /// What its Recycle Bin refused ([`NoBin`]) goes on top instead, since the question about it
    /// is up now. Returns the entry put back.
    pub fn put_back(
        &mut self,
        undo: Undo,
        done: usize,
        error: Option<&io::Error>,
    ) -> Option<EntryId> {
        let (id, at, epoch) = (undo.id, undo.at, undo.epoch);
        let asked = error.and_then(NoBin::of).is_some();
        let left = undo.left(done, error).filter(|_| epoch == self.epoch)?;
        let at = if asked { self.entries.len() } else { at };
        self.entries.insert(at.min(self.entries.len()), (id, left));
        Some(id)
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
        assert!(u.undo_dirs().is_empty());
        u.push_all(vec![
            Undoable::Move {
                from: "/a/x".into(),
                to: "/b/x".into(),
                listed: false,
            },
            Undoable::Move {
                from: "/a/y".into(),
                to: "/b/y".into(),
                listed: false,
            },
        ]);
        assert_eq!(u.undo_dirs(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        u.push(Undoable::New("/c/New Folder".into(), None));
        assert_eq!(u.undo_dirs(), [PathBuf::from("/c")], "the top entry only");
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
        u.push(Undoable::Copied(path("made.txt")));
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
        u.push(Undoable::Copied(item("c")));

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
        });

        u.undo().unwrap().unwrap();

        assert!(from.exists() && !to.exists());
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
    fn undo_of_a_copied_folder_trashes_it_with_its_contents() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let dir = t.path().join("Photos");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("a.jpg"), "a").unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::Copied(dir.clone()));
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
            Undoable::Copied(a.clone()),
            Undoable::Copied(b.clone()),
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
        u.push(Undoable::Copied("/old".into()));
        u.push(Undoable::Copied("/x".into()));
        let undo = u.take().unwrap();
        u.push(Undoable::New("/newer".into(), None));
        u.put_back(undo, 0, None);
        let order: Vec<PathBuf> = std::iter::from_fn(|| u.take())
            .map(|t| match t.steps() {
                [Undoable::Copied(p) | Undoable::New(p, _)] => p.clone(),
                _ => panic!("one step"),
            })
            .collect();
        assert_eq!(order, ["/newer", "/x", "/old"].map(PathBuf::from));

        u.push(Undoable::Copied("/x".into()));
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
        u.push(Undoable::Copied("/x".into()));
        let lone = u.take().expect("the copy");
        let kept = lone.left(0, None);
        assert!(
            matches!(kept, Some(Undoable::Copied(_))),
            "a cancel keeps it"
        );
        u.push(kept.unwrap());
        let failed = io::Error::new(io::ErrorKind::NotFound, "gone");
        let lone = u.take().expect("the copy");
        assert!(lone.left(0, Some(&failed)).is_none(), "a failure drops it");

        u.push_all(vec![
            Undoable::Copied("/a".into()),
            Undoable::Copied("/b".into()),
            Undoable::Copied("/c".into()),
        ]);
        let batch = u.take().expect("the batch");
        let paths: Vec<_> = (batch.steps().iter())
            .map(|s| match s {
                Undoable::Copied(p) => p.clone(),
                _ => panic!("a copy"),
            })
            .collect();
        assert_eq!(paths, ["/c", "/b", "/a"].map(PathBuf::from), "last first");
        match batch.left(1, None) {
            Some(Undoable::Batch(rest)) => assert!(matches!(
                rest.as_slice(),
                [Undoable::Copied(a), Undoable::Copied(b)] if *a == Path::new("/a") && *b == Path::new("/b")
            )),
            _ => panic!("the batch keeps /a and /b"),
        }
    }

    #[test]
    fn push_all_keeps_a_lone_operation_and_drops_an_empty_batch() {
        let mut u = UndoStack::default();
        u.push_all(Vec::new());
        assert_eq!(u.label(), None);
        u.push_all(vec![Undoable::Copied(PathBuf::from("/x"))]);
        assert_eq!(u.label(), Some("Undo Copy"));
    }
}
