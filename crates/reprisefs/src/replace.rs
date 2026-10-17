//! The replace prompt's decisions. A copy or move landing on a name the
//! destination holds asks first: Confirm File Replace, or Confirm Folder Replace
//! (a merge), whose Yes then asks Confirm File Replace for each file inside the
//! merge would put over a file. The host finds each item's [`Found`] clash, walks an [`Asker`]
//! through the questions, and the answers become one [`Verdict`] per item for the
//! batch ([`crate::ops::put_in_with`]). A Yes replaces only what the prompt
//! showed: each clash keeps a [`Seen`], and a file that changed since is not
//! replaced but numbered past.
//!
//! A copy into its own folder never asks ("Copy of x"); nor do shortcuts and
//! launchers, which number past a taken name.

use crate::ops::DropOp;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub clash: Clash,
    pub seen: Seen,
    /// A folder's files that a merge would put over a file, by path under the folder, each as
    /// the prompt will show it.
    pub inside: Vec<(PathBuf, Seen)>,
}

/// What a Yes puts an item over: what held its name as the prompt saw it and, for a folder
/// merge, the files inside answered Yes, by path under the folder. A clashing file inside that is
/// not listed stays, and the incoming one is left where it was.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replacing {
    pub seen: Seen,
    pub files: BTreeMap<PathBuf, Seen>,
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
    let inside = match clash {
        Clash::Folder => crate::ops::merge_clashes(src, &dest, op == DropOp::Move),
        Clash::File => Vec::new(),
    };
    Some(Found {
        clash,
        seen: Seen::of(&dest)?,
        inside,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Put,
    /// Put it over what holds its name, if that is still what the prompt saw.
    Replace(Replacing),
    Skip,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Ask about item `index`, or with `inside` about that file under the folder item `index`
    /// merges into, after the folder's Yes.
    Ask {
        index: usize,
        clash: Clash,
        inside: Option<PathBuf>,
    },
    Done(Vec<Verdict>),
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asker {
    clashes: Vec<Option<Found>>,
    verdicts: Vec<Verdict>,
    /// The folder answered Yes whose files are being asked about, and how many have been.
    merging: Option<(Replacing, usize)>,
    cancelled: bool,
}

impl Asker {
    pub fn new(clashes: Vec<Option<Found>>) -> Asker {
        Asker {
            verdicts: Vec::with_capacity(clashes.len()),
            clashes,
            merging: None,
            cancelled: false,
        }
        .settle(false)
    }

    /// Record verdicts up to the next item that needs asking. `all` (a
    /// Yes to All) answers every remaining clash with Replace, every file inside included.
    fn settle(mut self, all: bool) -> Asker {
        while let Some(c) = self.clashes.get(self.verdicts.len()) {
            self.verdicts.push(match c {
                None => Verdict::Put,
                Some(f) if all => Verdict::Replace(Replacing {
                    seen: f.seen,
                    files: f.inside.iter().cloned().collect(),
                }),
                Some(_) => break,
            });
        }
        self
    }

    fn asking(&self) -> Option<&Found> {
        self.clashes.get(self.verdicts.len())?.as_ref()
    }

    pub fn step(&self) -> Step {
        if self.cancelled {
            return Step::Cancelled;
        }
        let index = self.verdicts.len();
        match (self.asking(), &self.merging) {
            (Some(found), Some((_, k))) => Step::Ask {
                index,
                clash: Clash::File,
                inside: Some(found.inside[*k].0.clone()),
            },
            (Some(found), None) => Step::Ask {
                index,
                clash: found.clash,
                inside: None,
            },
            (None, _) => Step::Done(self.verdicts.clone()),
        }
    }

    /// Answer the question [`Asker::step`] asked. An answer with nothing
    /// asked changes nothing.
    pub fn answer(&mut self, a: Answer) -> Step {
        let Some(found) = self.asking().cloned() else {
            return self.step();
        };
        if self.cancelled {
            return self.step();
        }
        if a == Answer::Cancel {
            self.cancelled = true;
            return self.step();
        }
        let this = std::mem::replace(self, Asker::new(Vec::new()));
        *self = match (this.merging.clone(), a) {
            (None, Answer::YesToAll) => this.settle(true),
            (Some((mut merge, k)), Answer::YesToAll) => {
                merge.files.extend(found.inside[k..].iter().cloned());
                let mut next = Asker {
                    merging: None,
                    ..this
                };
                next.verdicts.push(Verdict::Replace(merge));
                next.settle(true)
            }
            (None, Answer::No) => this.decide(Verdict::Skip),
            (None, _) if found.inside.is_empty() => this.decide(Verdict::Replace(Replacing {
                seen: found.seen,
                files: BTreeMap::new(),
            })),
            (None, _) => Asker {
                merging: Some((
                    Replacing {
                        seen: found.seen,
                        files: BTreeMap::new(),
                    },
                    0,
                )),
                ..this
            },
            (Some((mut merge, k)), a) => {
                if a == Answer::Yes {
                    let (path, seen) = found.inside[k].clone();
                    merge.files.insert(path, seen);
                }
                if k + 1 < found.inside.len() {
                    Asker {
                        merging: Some((merge, k + 1)),
                        ..this
                    }
                } else {
                    Asker {
                        merging: None,
                        ..this
                    }
                    .decide(Verdict::Replace(merge))
                }
            }
        };
        self.step()
    }

    fn decide(mut self, v: Verdict) -> Asker {
        self.verdicts.push(v);
        self.settle(false)
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
            inside: Vec::new(),
        })
    }

    const R: Verdict = Verdict::Replace(Replacing {
        seen: Seen {
            dev: 0,
            ino: 0,
            mtime: (0, 0),
            ctime: (0, 0),
            len: 0,
            dir: false,
        },
        files: BTreeMap::new(),
    });

    /// A folder clash whose merge would put files `names` over files.
    fn merging(names: &[&str]) -> Option<Found> {
        Some(Found {
            inside: names
                .iter()
                .map(|n| (PathBuf::from(n), Seen::default()))
                .collect(),
            ..c(Clash::Folder)?
        })
    }

    fn ask(index: usize, clash: Clash, inside: Option<&str>) -> Step {
        Step::Ask {
            index,
            clash,
            inside: inside.map(PathBuf::from),
        }
    }

    /// A merge's Replace with `names` answered Yes inside it.
    fn merged(names: &[&str]) -> Verdict {
        Verdict::Replace(Replacing {
            seen: Seen::default(),
            files: names
                .iter()
                .map(|n| (PathBuf::from(n), Seen::default()))
                .collect(),
        })
    }

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
        assert_eq!(a.step(), ask(1, Clash::File, None));
        assert_eq!(a.answer(Answer::No), ask(3, Clash::Folder, None));
        assert_eq!(
            a.answer(Answer::Yes),
            Step::Done(vec![Verdict::Put, Verdict::Skip, Verdict::Put, R])
        );
    }

    /// A folder's Yes asks about each file inside it the merge would put over a file, and only
    /// the files answered Yes are replaced; a folder's No asks nothing inside.
    #[test]
    fn a_merged_folder_asks_for_each_file_inside() {
        let mut a = Asker::new(vec![
            merging(&["a.txt", "q1/b.txt", "c.txt"]),
            c(Clash::File),
        ]);
        assert_eq!(a.step(), ask(0, Clash::Folder, None));
        assert_eq!(a.answer(Answer::Yes), ask(0, Clash::File, Some("a.txt")));
        assert_eq!(a.answer(Answer::Yes), ask(0, Clash::File, Some("q1/b.txt")));
        assert_eq!(a.answer(Answer::No), ask(0, Clash::File, Some("c.txt")));
        assert_eq!(a.answer(Answer::Yes), ask(1, Clash::File, None));
        assert_eq!(
            a.answer(Answer::No),
            Step::Done(vec![merged(&["a.txt", "c.txt"]), Verdict::Skip])
        );

        let mut a = Asker::new(vec![merging(&["a.txt"]), c(Clash::File)]);
        assert_eq!(
            a.answer(Answer::No),
            ask(1, Clash::File, None),
            "no file asked"
        );
    }

    /// Yes to All on a file inside replaces it, the rest inside and every later clash, while the
    /// files already answered No stay; on the folder it replaces every file inside unasked.
    #[test]
    fn yes_to_all_inside_a_merge_keeps_the_earlier_nos() {
        let mut a = Asker::new(vec![
            merging(&["a.txt", "b.txt", "c.txt"]),
            merging(&["d.txt"]),
        ]);
        a.answer(Answer::Yes);
        a.answer(Answer::No);
        assert_eq!(
            a.answer(Answer::YesToAll),
            Step::Done(vec![merged(&["b.txt", "c.txt"]), merged(&["d.txt"])])
        );

        let mut a = Asker::new(vec![merging(&["a.txt", "b.txt"])]);
        assert_eq!(
            a.answer(Answer::YesToAll),
            Step::Done(vec![merged(&["a.txt", "b.txt"])])
        );
    }

    /// Cancel on a file inside a merge stops the whole batch.
    #[test]
    fn cancel_inside_a_merge_stops_the_batch() {
        let mut a = Asker::new(vec![merging(&["a.txt", "b.txt"]), None]);
        a.answer(Answer::Yes);
        assert_eq!(a.answer(Answer::Cancel), Step::Cancelled);
    }

    /// A folder clash lists, from the disk, the files inside its merge would put over a file:
    /// not a file only one side has, nor a file meeting a folder.
    #[test]
    fn a_folder_clash_lists_the_files_its_merge_would_replace() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a/R"), t.path().join("b/R"));
        for d in [&a, &b] {
            std::fs::create_dir_all(d.join("q1/deep")).unwrap();
            std::fs::write(d.join("same.txt"), "").unwrap();
            std::fs::write(d.join("q1/deep/low.txt"), "").unwrap();
        }
        std::fs::write(a.join("mine.txt"), "").unwrap();
        std::fs::write(b.join("theirs.txt"), "").unwrap();
        std::fs::write(a.join("odd"), "").unwrap();
        std::fs::create_dir(b.join("odd")).unwrap();
        let found = clash_at(&a, &t.path().join("b"), DropOp::Copy, false).expect("a clash");
        assert_eq!(found.clash, Clash::Folder);
        let inside: Vec<(PathBuf, Option<Seen>)> = found
            .inside
            .into_iter()
            .map(|(p, seen)| (p, Some(seen)))
            .collect();
        assert_eq!(
            inside,
            [
                (
                    "q1/deep/low.txt".into(),
                    Seen::of(&b.join("q1/deep/low.txt"))
                ),
                ("same.txt".into(), Seen::of(&b.join("same.txt"))),
            ]
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
