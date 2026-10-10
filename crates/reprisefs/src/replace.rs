//! The replace prompt's decisions. A copy or move landing on a name the
//! destination holds asks first: Confirm File Replace, or Confirm Folder Replace
//! (a merge). The host finds each item's [`Found`] clash, walks an [`Asker`]
//! through the questions, and the answers become one [`Verdict`] per item for the
//! batch ([`crate::ops::put_in_with`]). A Yes replaces only what the prompt
//! showed: each clash keeps a [`Seen`], and a file that changed since is not
//! replaced but numbered past.
//!
//! A copy into its own folder never asks ("Copy of x"); nor do shortcuts and
//! launchers, which number past a taken name.

use crate::ops::DropOp;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clash {
    File,
    Folder,
}

/// What a clash was about, as the prompt saw it: size and modified and changed
/// times. A replacement goes ahead only while the name still holds exactly this;
/// a folder is only ever the same folder.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    dev: u64,
    ino: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
    len: u64,
    dir: bool,
}

impl Seen {
    pub fn of(path: &Path) -> Option<Seen> {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::symlink_metadata(path).ok()?;
        Some(Seen {
            dev: m.dev(),
            ino: m.ino(),
            mtime: (m.mtime(), m.mtime_nsec()),
            ctime: (m.ctime(), m.ctime_nsec()),
            len: m.len(),
            dir: m.is_dir(),
        })
    }

    pub fn still(&self, path: &Path) -> bool {
        match Seen::of(path) {
            Some(now) if self.dir => now.dir && now.id() == self.id(),
            Some(now) => now == *self,
            None => false,
        }
    }

    pub fn id(&self) -> (u64, u64) {
        (self.dev, self.ino)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub clash: Clash,
    pub seen: Seen,
}

/// Whether a put asks before it lands. `same_dir`: a put into the source's own
/// folder; `src_dir`: the source is a folder; `dest_dir`: what holds the name
/// (None if free). A file onto a folder or the reverse asks nothing: the put
/// refuses it.
pub fn clash(
    op: DropOp,
    is_app: bool,
    same_dir: bool,
    src_dir: bool,
    dest_dir: Option<bool>,
) -> Option<Clash> {
    if op == DropOp::Shortcut || is_app || same_dir {
        return None;
    }
    match (src_dir, dest_dir?) {
        (false, false) => Some(Clash::File),
        (true, true) => Some(Clash::Folder),
        _ => None,
    }
}

/// [`clash`] for `src` put into `dir`, looking at the disk. A copied link counts
/// as what it points at; a moved link is a link.
pub fn clash_at(src: &Path, dir: &Path, op: DropOp, is_app: bool) -> Option<Found> {
    let name = src.file_name()?;
    let src_meta = if op == DropOp::Move {
        std::fs::symlink_metadata(src)
    } else {
        std::fs::metadata(src)
    };
    let dest = dir.join(name);
    let clash = clash(
        op,
        is_app,
        crate::ops::same_folder(src, dir),
        src_meta.ok()?.is_dir(),
        std::fs::symlink_metadata(&dest).ok().map(|m| m.is_dir()),
    )?;
    Some(Found {
        clash,
        seen: Seen::of(&dest)?,
    })
}

/// [`clash_at`] for each item of one batch into `dir`, as `(source, op,
/// is_app)`. A later item with an earlier one's file name never asks, so it
/// lands numbered rather than replacing the batch's own put.
pub fn clashes<'a>(
    items: impl IntoIterator<Item = (&'a Path, DropOp, bool)>,
    dir: &Path,
) -> Vec<Option<Found>> {
    let mut seen = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|(src, op, is_app)| {
            let first = src.file_name().is_none_or(|n| seen.insert(n.to_owned()));
            clash_at(src, dir, op, is_app).filter(|_| first)
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    YesToAll,
    No,
    /// Stop: nothing in the batch is put, not even the items that did not clash.
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Put,
    /// Put it over what holds its name, if that is still what the prompt saw.
    Replace(Seen),
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Ask { index: usize, clash: Clash },
    Done(Vec<Verdict>),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asker {
    clashes: Vec<Option<Found>>,
    verdicts: Vec<Verdict>,
    cancelled: bool,
}

impl Asker {
    pub fn new(clashes: Vec<Option<Found>>) -> Asker {
        Asker {
            verdicts: Vec::with_capacity(clashes.len()),
            clashes,
            cancelled: false,
        }
        .settle(false)
    }

    /// Record verdicts up to the next item that needs asking. `all` (a
    /// Yes to All) answers every remaining clash with Replace.
    fn settle(mut self, all: bool) -> Asker {
        while let Some(&c) = self.clashes.get(self.verdicts.len()) {
            self.verdicts.push(match c {
                None => Verdict::Put,
                Some(f) if all => Verdict::Replace(f.seen),
                Some(_) => break,
            });
        }
        self
    }

    pub fn step(&self) -> Step {
        if self.cancelled {
            return Step::Cancelled;
        }
        match self.clashes.get(self.verdicts.len()) {
            Some(Some(found)) => Step::Ask {
                index: self.verdicts.len(),
                clash: found.clash,
            },
            _ => Step::Done(self.verdicts.clone()),
        }
    }

    /// Answer the question [`Asker::step`] asked. An answer with nothing
    /// asked changes nothing.
    pub fn answer(&mut self, a: Answer) -> Step {
        if !matches!(self.step(), Step::Ask { .. }) {
            return self.step();
        }
        let this = std::mem::replace(self, Asker::new(Vec::new()));
        *self = match a {
            Answer::Cancel => Asker {
                cancelled: true,
                ..this
            },
            Answer::Yes | Answer::No => {
                let mut next = this;
                let seen = next.clashes[next.verdicts.len()].map(|f| f.seen);
                next.verdicts.push(match seen {
                    Some(seen) if a == Answer::Yes => Verdict::Replace(seen),
                    _ => Verdict::Skip,
                });
                next.settle(false)
            }
            Answer::YesToAll => this.settle(true),
        };
        self.step()
    }
}

pub fn size_text(bytes: u64) -> String {
    let full = crate::props::format_size(bytes);
    match full.split_once(" (") {
        Some((short, _)) => short.to_string(),
        None => full,
    }
}

/// The date line under a file on the prompt, from its modified time in
/// seconds since the epoch: "modified: Today, September 28, 2026,
/// 9:41:07 AM".
pub fn modified_text(secs: i64, is_12h: bool) -> String {
    format!("modified: {}", crate::props::format_time(secs, is_12h))
}

/// The size and date lines for the file at `path`; a file that cannot be
/// read shows neither.
pub fn facts(path: &Path, is_12h: bool) -> (String, String) {
    use std::os::unix::fs::MetadataExt;
    match std::fs::symlink_metadata(path) {
        Ok(m) => (size_text(m.len()), modified_text(m.mtime(), is_12h)),
        Err(_) => (String::new(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clash of `kind` over a file seen as `Seen::default()`.
    fn c(kind: Clash) -> Option<Found> {
        Some(Found {
            clash: kind,
            seen: Seen::default(),
        })
    }

    const R: Verdict = Verdict::Replace(Seen {
        dev: 0,
        ino: 0,
        mtime: (0, 0),
        ctime: (0, 0),
        len: 0,
        dir: false,
    });

    #[test]
    fn only_a_copy_or_move_elsewhere_onto_the_same_kind_asks() {
        let c = |op, app, same, src, dest| clash(op, app, same, src, dest);
        assert_eq!(
            c(DropOp::Copy, false, false, false, Some(false)),
            Some(Clash::File)
        );
        assert_eq!(
            c(DropOp::Move, false, false, true, Some(true)),
            Some(Clash::Folder)
        );
        assert_eq!(c(DropOp::Copy, false, false, false, None), None, "free");
        assert_eq!(
            c(DropOp::Copy, false, true, false, Some(false)),
            None,
            "a copy into its own folder is named Copy of"
        );
        assert_eq!(c(DropOp::Shortcut, false, false, false, Some(false)), None);
        assert_eq!(c(DropOp::Copy, true, false, false, Some(false)), None);
        assert_eq!(
            c(DropOp::Move, false, false, false, Some(true)),
            None,
            "a file onto a folder is an error, not a question"
        );
    }

    #[test]
    fn clash_at_reads_the_disk() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        std::fs::create_dir_all(a.join("Reports")).unwrap();
        std::fs::create_dir_all(b.join("Reports")).unwrap();
        std::fs::write(a.join("x.txt"), "").unwrap();
        std::fs::write(b.join("x.txt"), "").unwrap();
        std::fs::write(a.join("y.txt"), "").unwrap();
        let at =
            |n: &str, dir: &Path| clash_at(&a.join(n), dir, DropOp::Copy, false).map(|f| f.clash);
        assert_eq!(at("x.txt", &b), Some(Clash::File));
        assert_eq!(at("Reports", &b), Some(Clash::Folder));
        assert_eq!(
            clash_at(&a.join("x.txt"), &b, DropOp::Copy, false).map(|f| f.seen),
            Seen::of(&b.join("x.txt")),
            "the clash keeps what held the name"
        );
        assert_eq!(at("y.txt", &b), None);
        assert_eq!(at("x.txt", &a), None);
    }

    /// Two items of one batch with the same name: only the first asks,
    /// files and folders alike; the second lands numbered.
    #[test]
    fn a_batch_asks_once_per_name() {
        let t = tempfile::tempdir().unwrap();
        let (a, b, dest) = (
            t.path().join("a"),
            t.path().join("b"),
            t.path().join("dest"),
        );
        for d in [&a, &b] {
            std::fs::create_dir_all(d.join("Reports")).unwrap();
            std::fs::write(d.join("x.txt"), "").unwrap();
        }
        std::fs::create_dir_all(dest.join("Reports")).unwrap();
        std::fs::write(dest.join("x.txt"), "").unwrap();
        let paths = [
            a.join("x.txt"),
            b.join("x.txt"),
            a.join("Reports"),
            b.join("Reports"),
        ];
        let got: Vec<_> = clashes(
            paths.iter().map(|p| (p.as_path(), DropOp::Copy, false)),
            &dest,
        )
        .into_iter()
        .map(|f| f.map(|f| f.clash))
        .collect();
        assert_eq!(got, [Some(Clash::File), None, Some(Clash::Folder), None]);
    }

    /// A link to a folder as the source asks as a folder, as a copy
    /// follows it; a copy beside itself through a link never asks.
    #[test]
    fn clash_at_follows_a_linked_source_and_resolves_the_folder() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b"));
        std::fs::create_dir_all(a.join("real")).unwrap();
        std::fs::create_dir_all(b.join("Reports")).unwrap();
        std::os::unix::fs::symlink(a.join("real"), a.join("Reports")).unwrap();
        let at = clash_at(&a.join("Reports"), &b, DropOp::Copy, false).map(|f| f.clash);
        assert_eq!(at, Some(Clash::Folder));
        let moved = clash_at(&a.join("Reports"), &b, DropOp::Move, false);
        assert_eq!(
            moved, None,
            "a moved link is a link: it numbers, so it never asks"
        );
        std::os::unix::fs::symlink(&b, t.path().join("via")).unwrap();
        std::fs::write(b.join("x.txt"), "").unwrap();
        let beside =
            clash_at(&b.join("x.txt"), &t.path().join("via"), DropOp::Copy, false).map(|f| f.clash);
        assert_eq!(beside, None);
    }

    /// Items without a clash go through; each clash waits for its
    /// answer, in item order.
    #[test]
    fn yes_and_no_answer_one_clash_each() {
        let mut a = Asker::new(vec![None, c(Clash::File), None, c(Clash::Folder)]);
        assert_eq!(
            a.step(),
            Step::Ask {
                index: 1,
                clash: Clash::File
            }
        );
        assert_eq!(
            a.answer(Answer::No),
            Step::Ask {
                index: 3,
                clash: Clash::Folder
            }
        );
        assert_eq!(
            a.answer(Answer::Yes),
            Step::Done(vec![Verdict::Put, Verdict::Skip, Verdict::Put, R])
        );
    }

    #[test]
    fn yes_to_all_replaces_every_later_clash_without_asking() {
        let mut a = Asker::new(vec![c(Clash::File), c(Clash::Folder), None]);
        assert_eq!(
            a.answer(Answer::YesToAll),
            Step::Done(vec![R, R, Verdict::Put])
        );
    }

    #[test]
    fn cancel_stops_the_whole_batch() {
        let mut a = Asker::new(vec![None, c(Clash::File), c(Clash::File)]);
        assert_eq!(a.answer(Answer::Yes), a.step());
        assert_eq!(a.answer(Answer::Cancel), Step::Cancelled);
        assert_eq!(a.answer(Answer::Yes), Step::Cancelled, "nothing after");
    }

    /// A file is still what was seen only unchanged; a folder stays the
    /// same folder while its contents change.
    #[test]
    fn a_file_must_be_unchanged_and_a_folder_only_the_same() {
        let t = tempfile::tempdir().unwrap();
        let (f, d) = (t.path().join("f.txt"), t.path().join("d"));
        std::fs::write(&f, "one").unwrap();
        std::fs::create_dir(&d).unwrap();
        let (sf, sd) = (Seen::of(&f).unwrap(), Seen::of(&d).unwrap());
        assert!(sf.still(&f) && sd.still(&d));
        std::fs::write(d.join("new.txt"), "").unwrap();
        assert!(
            sd.still(&d),
            "a folder whose contents changed is the same folder"
        );
        std::fs::write(&f, "two!").unwrap();
        assert!(!sf.still(&f), "a rewritten file is not what was seen");
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::write(&d, "a file now").unwrap();
        assert!(
            !sd.still(&d),
            "a file where the folder was is not the folder"
        );
        std::fs::remove_file(&d).unwrap();
        assert!(!sd.still(&d), "nothing there is not the folder");
    }

    #[test]
    fn a_batch_without_clashes_is_done_at_once() {
        let a = Asker::new(vec![None, None]);
        assert_eq!(a.step(), Step::Done(vec![Verdict::Put, Verdict::Put]));
    }

    #[test]
    fn the_size_line_is_short() {
        assert_eq!(size_text(512), "512 bytes");
        assert_eq!(size_text(1260), "1.23 KB");
        assert!(modified_text(0, true).starts_with("modified: "));
    }
}
