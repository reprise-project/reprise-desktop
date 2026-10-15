//! The single-level undo behind the Edit > Undo row: reversing the
//! last move, rename, trash or creation.

use std::io;
use std::path::{Path, PathBuf};

use crate::trash::{self, Trashed};

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
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    Trash(Trashed),
    /// A copy or a shortcut that landed at the path. Undoing it trashes
    /// it, a copied folder with everything in it.
    Copied(PathBuf),
    /// A New Folder or New Text Document. Undoing it trashes a file and
    /// removes a folder only while it is empty.
    New(PathBuf),
    Batch(Vec<Undoable>),
}

impl Undoable {
    fn label(&self) -> Option<&'static str> {
        Some(match self {
            Undoable::Move { .. } => "Undo Move",
            Undoable::Rename { .. } => "Undo Rename",
            Undoable::Trash(_) => "Undo Delete",
            Undoable::Copied(_) => "Undo Copy",
            Undoable::New(_) => "Undo New",
            Undoable::Batch(items) => return items.last()?.label(),
        })
    }

    /// Add to `out` the folders reversing this adds an entry to or takes
    /// one from, each once.
    fn dirs(&self, out: &mut Vec<PathBuf>) {
        let paths: Vec<&Path> = match self {
            Undoable::Move { from, to, .. } | Undoable::Rename { from, to } => vec![from, to],
            Undoable::Trash(t) => vec![&t.original],
            Undoable::Copied(p) | Undoable::New(p) => vec![p],
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

    /// Reverse it, in place. A batch drops each member as it goes and stops
    /// at its first failure, so what is left is exactly what was not
    /// reversed, ready to be pushed back and retried. `bins` are where an
    /// undone copy or new file goes; the `files/` path of each item their cap
    /// erases to make room joins `purged`. An item its bin refuses ([`NoBin`]) fails the reversal
    /// unless `erase`, which removes it for good.
    fn reverse(
        &mut self,
        bins: Option<&trash::Bins>,
        erase: bool,
        purged: &mut Vec<PathBuf>,
    ) -> io::Result<()> {
        let mut trash_in = |path: &Path| {
            let bins =
                bins.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no trash directory"))?;
            match bins.trash(path) {
                Ok((_, gone)) => {
                    purged.extend(gone);
                    Ok(())
                }
                Err(e) if NoBin::refuses(&e) && erase => crate::ops::erase(path),
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
            Undoable::Rename { from, to } => crate::hidden::rename(to, from),
            Undoable::Trash(t) => trash::restore(t),
            Undoable::Copied(path) => trash_in(path),
            // A folder goes only while empty and a file goes to the trash,
            // so nothing the user put there since is lost.
            Undoable::New(path) => {
                if std::fs::symlink_metadata(&*path)?.is_dir() {
                    if std::fs::read_dir(&*path)?.next().is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::DirectoryNotEmpty,
                            "the folder is not empty",
                        ));
                    }
                    std::fs::remove_dir(&*path)
                } else {
                    trash_in(path)
                }
            }
            Undoable::Batch(items) => {
                while let Some(last) = items.last_mut() {
                    last.reverse(bins, erase, purged)?;
                    items.pop();
                }
                Ok(())
            }
        }
    }
}

/// Why Undo stopped at a copied or new item: its Recycle Bin refuses it (its drive removes files
/// immediately, or it is too big). The entry stays on the stack for [`UndoStack::undo_erasing`].
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

#[derive(Default)]
pub struct UndoStack(Vec<Undoable>);

impl UndoStack {
    pub fn push(&mut self, u: Undoable) {
        self.0.push(u);
    }

    /// Push a whole operation's worth of undos as one entry: nothing for
    /// an empty batch, the operation itself for a single one.
    pub fn push_all(&mut self, mut batch: Vec<Undoable>) {
        match batch.len() {
            0 => {}
            1 => self.0.push(batch.remove(0)),
            _ => self.0.push(Undoable::Batch(batch)),
        }
    }

    /// Drop every undo that would put a file back from the trash. Emptying
    /// the trash makes a `Trash` entry unreversible but not a move, rename
    /// or creation; a batch keeps whatever survives.
    pub fn retain_non_trash(&mut self) {
        self.0.retain_mut(|u| u.retain_trash(&|_| true));
    }

    /// Drop the undos that would put back an item that has left the
    /// trash from `files` (its path under `files/`), restored or deleted
    /// for good, so it is no longer there to put back.
    pub fn forget_gone(&mut self, files: &[PathBuf]) {
        self.0
            .retain_mut(|u| u.retain_trash(&|t| files.contains(&t.files)));
    }

    pub fn label(&self) -> Option<&'static str> {
        self.0.last()?.label()
    }

    pub fn undo_dirs(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(u) = self.0.last() {
            u.dirs(&mut out);
        }
        out
    }

    /// Reverse the most recent operation. `None` when the stack is empty. A
    /// failed single operation is dropped; a failed batch goes back on the
    /// stack holding the members it did not reverse, so Undo again picks up
    /// where it stopped.
    /// An entry stopped by [`NoBin`] stays whole for [`UndoStack::undo_erasing`].
    pub fn undo(&mut self) -> Option<io::Result<()>> {
        self.undo_in(trash::Bins::system().as_ref(), false)
    }

    /// [`UndoStack::undo`] removing for good each copied or new item its Recycle Bin refuses.
    pub fn undo_erasing(&mut self) -> Option<io::Result<()>> {
        self.undo_in(trash::Bins::system().as_ref(), true)
    }

    /// Whether the entry Undo would reverse next holds more than one item.
    pub fn many(&self) -> bool {
        matches!(self.0.last(), Some(Undoable::Batch(items)) if items.len() > 1)
    }

    fn undo_in(&mut self, bins: Option<&trash::Bins>, erase: bool) -> Option<io::Result<()>> {
        let mut entry = self.0.pop()?;
        let mut purged = Vec::new();
        let done = entry.reverse(bins, erase, &mut purged);
        let keep = match &done {
            Ok(()) => false,
            Err(e) if NoBin::of(e).is_some() => true,
            Err(_) => matches!(&entry, Undoable::Batch(items) if !items.is_empty()),
        };
        if keep {
            self.0.push(entry);
        }
        self.forget_gone(&purged);
        Some(done)
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
        u.push(Undoable::New("/c/New Folder".into()));
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
        assert_eq!(u.0.len(), 1, "a's undo went with it");
        assert!(matches!(&u.0[0], Undoable::Trash(t) if *t == b));
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
        });

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&from).unwrap(), "new file landed here");
        assert_eq!(fs::read_to_string(&to).unwrap(), "hi");
    }

    #[test]
    fn undo_of_new_removes_the_empty_folder() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("New Folder");
        fs::create_dir(&dir).unwrap();

        let mut u = UndoStack::default();
        u.push(Undoable::New(dir.clone()));

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
        u.push(Undoable::New(file.clone()));
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
        u.push(Undoable::New(dir.clone()));

        let err = u.undo().unwrap().unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::DirectoryNotEmpty);
        assert_eq!(fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine");
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
        u.push(Undoable::New(missing));

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
    fn push_all_keeps_a_lone_operation_and_drops_an_empty_batch() {
        let mut u = UndoStack::default();
        u.push_all(Vec::new());
        assert_eq!(u.label(), None);
        u.push_all(vec![Undoable::Copied(PathBuf::from("/x"))]);
        assert_eq!(u.label(), Some("Undo Copy"));
    }
}
