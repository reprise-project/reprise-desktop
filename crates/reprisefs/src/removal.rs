//! Deletes, Empty Recycle Bin and Restore off the caller's thread, counted like a put
//! ([`crate::jobs::PutJob`]): a walk of every item for the totals, then the items in order,
//! reported one by one. A pause or cancel takes effect between two files.

use crate::bincap::Measured;
use crate::progress::{self, Meter, Phase, Snapshot, Totals};
use crate::trash::{self, Bins, Trashed};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, TryRecvError};

/// One item of a removal job.
#[derive(Clone, Debug)]
pub enum Removal {
    /// To the Recycle Bin, its size from the room check when it still holds.
    Trash {
        path: PathBuf,
        known: Option<Measured>,
    },
    /// For good, file by file.
    Erase(PathBuf),
    /// A Recycle Bin entry for good, once it is still the item `record` lists.
    EraseTrashed {
        files: PathBuf,
        record: Option<Trashed>,
    },
    /// A Recycle Bin entry back where it came from, once it is still the item `record` lists.
    Restore {
        files: PathBuf,
        record: Option<Trashed>,
    },
    /// Everything in the trash at this root ([`trash::empty_in`]).
    Empty(PathBuf),
}

/// What became of an item.
#[derive(Debug)]
pub enum Removed {
    /// In the bin, with the `files/` paths of what was erased to make room for it.
    Trashed {
        trashed: Trashed,
        purged: Vec<PathBuf>,
    },
    Erased,
    /// Back at this path.
    Restored(PathBuf),
    /// That many entries went.
    Emptied(usize),
}

#[derive(Debug)]
pub enum RemoveEvent {
    Done { index: usize, removed: Removed },
    Failed { index: usize, error: io::Error },
    Finished,
}

pub struct RemoveJob {
    events: Receiver<RemoveEvent>,
    meter: Arc<Meter>,
    finished: bool,
}

impl Removal {
    /// Delete's item for `path` as `plan` says: to the bin or for good.
    pub fn planned(path: PathBuf, plan: crate::bincap::Plan) -> Removal {
        match plan {
            crate::bincap::Plan::Erase => Removal::Erase(path),
            crate::bincap::Plan::Trash(known) => Removal::Trash { path, known },
        }
    }

    /// The files and links under the item, which the bar counts.
    fn totals(&self, stop: &AtomicBool) -> Option<Totals> {
        let path = match self {
            Removal::Trash { path, .. } | Removal::Erase(path) => path.clone(),
            Removal::EraseTrashed { files, .. } | Removal::Restore { files, .. } => files.clone(),
            Removal::Empty(root) => root.join("files"),
        };
        progress::walk(&path, false, stop)
    }

    fn name(&self) -> &std::ffi::OsStr {
        let path = match self {
            Removal::Trash { path, .. } | Removal::Erase(path) => path,
            Removal::EraseTrashed { files, .. } | Removal::Restore { files, .. } => files,
            Removal::Empty(root) => root,
        };
        path.file_name().unwrap_or(path.as_os_str())
    }

    fn run(&self, bins: Option<&Bins>, m: &Meter) -> io::Result<Removed> {
        let erase = |p: &Path| crate::ops::erase_counted(p, m);
        match self {
            Removal::Trash { path, known } => {
                let bins = bins.ok_or_else(|| io::Error::other("cannot find the Recycle Bin"))?;
                let (trashed, purged) = bins.trash_measured(path, *known)?;
                Ok(Removed::Trashed { trashed, purged })
            }
            Removal::Erase(path) => erase(path).map(|()| Removed::Erased),
            Removal::EraseTrashed { files, record } => {
                trash::erase_trashed_with(files, record.as_ref(), erase).map(|()| Removed::Erased)
            }
            Removal::Restore { files, record } => {
                trash::check_listed(files, record.as_ref())?;
                let t = trash::trashed(files)?;
                trash::restore_free(&t).map(Removed::Restored)
            }
            Removal::Empty(root) => trash::empty_with(root, |p: &Path| {
                m.set_current(p.file_name().unwrap_or(p.as_os_str()));
                erase(p)
            })
            .map(Removed::Emptied),
        }
    }
}

impl RemoveJob {
    /// Start `items` on a thread of their own; `bins` is where [`Removal::Trash`] goes. `wake`
    /// runs after each report. The first failure stops the rest, except that an Empty goes on to
    /// the next bin.
    pub fn start(
        items: Vec<Removal>,
        bins: Option<Bins>,
        wake: impl Fn() + Send + 'static,
    ) -> RemoveJob {
        Self::start_metered(items, bins, Arc::new(Meter::default()), wake)
    }

    /// [`Self::start`], counting on `meter`, so a test can pause it before the thread runs.
    fn start_metered(
        items: Vec<Removal>,
        bins: Option<Bins>,
        meter: Arc<Meter>,
        wake: impl Fn() + Send + 'static,
    ) -> RemoveJob {
        let (tx, events) = mpsc::channel();
        let m = meter.clone();
        let spawned = std::thread::Builder::new()
            .name("reprisefs-remove".into())
            .spawn(move || {
                let Some(sizes) = items
                    .iter()
                    .map(|it| {
                        let t = it.totals(&m.cancel);
                        m.add_totals(t.unwrap_or_default());
                        t
                    })
                    .collect::<Option<Vec<Totals>>>()
                else {
                    let _ = tx.send(RemoveEvent::Finished);
                    wake();
                    return;
                };
                m.set_phase(Phase::Copying);
                let mut done = Totals::default();
                for (index, it) in items.iter().enumerate() {
                    if m.hold().is_err() {
                        break;
                    }
                    m.set_current(it.name());
                    let r = progress::metered(&m, || it.run(bins.as_ref(), &m));
                    done = done + sizes[index];
                    m.settle(done);
                    let stop = match &r {
                        Err(_) if m.cancelled() => true,
                        Err(_) => !matches!(it, Removal::Empty(_)),
                        Ok(_) => false,
                    };
                    let ev = match r {
                        Ok(removed) => RemoveEvent::Done { index, removed },
                        Err(_) if m.cancelled() => break,
                        Err(error) => RemoveEvent::Failed { index, error },
                    };
                    if tx.send(ev).is_err() {
                        return;
                    }
                    wake();
                    if stop {
                        break;
                    }
                }
                let _ = tx.send(RemoveEvent::Finished);
                wake();
            });
        if let Err(e) = spawned {
            log::error!("reprisefs: cannot spawn the removal thread: {e}");
        }
        RemoveJob {
            events,
            meter,
            finished: false,
        }
    }

    pub fn meter(&self) -> &Arc<Meter> {
        &self.meter
    }

    pub fn progress(&self) -> Snapshot {
        self.meter.snapshot()
    }

    pub fn poll(&mut self) -> Vec<RemoveEvent> {
        let mut out = Vec::new();
        while !self.finished {
            match self.events.try_recv() {
                Ok(RemoveEvent::Finished) | Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    out.push(RemoveEvent::Finished);
                }
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => break,
            }
        }
        out
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn run(mut job: RemoveJob, woke: Receiver<()>) -> Vec<RemoveEvent> {
        let mut all = Vec::new();
        while !job.is_finished() {
            woke.recv_timeout(Duration::from_secs(5)).expect("a wake");
            all.extend(job.poll());
        }
        all
    }

    fn tree(root: &Path, files: usize) {
        std::fs::create_dir_all(root.join("sub/deeper")).unwrap();
        for i in 0..files {
            let dir = match i % 3 {
                0 => root.to_path_buf(),
                1 => root.join("sub"),
                _ => root.join("sub/deeper"),
            };
            std::fs::write(dir.join(format!("f{i}")), "x").unwrap();
        }
    }

    #[test]
    fn an_erase_counts_every_file_and_reports_each_item() {
        let t = tempfile::tempdir().unwrap();
        let (a, b) = (t.path().join("a"), t.path().join("b.txt"));
        tree(&a, 30);
        std::fs::write(&b, "b").unwrap();
        let (wtx, woke) = channel();
        let job = RemoveJob::start(
            vec![Removal::Erase(a.clone()), Removal::Erase(b.clone())],
            None,
            move || {
                let _ = wtx.send(());
            },
        );
        let meter = job.meter().clone();
        let got = run(job, woke);
        assert!(
            matches!(
                got.as_slice(),
                [
                    RemoveEvent::Done {
                        index: 0,
                        removed: Removed::Erased
                    },
                    RemoveEvent::Done {
                        index: 1,
                        removed: Removed::Erased
                    },
                    RemoveEvent::Finished,
                ]
            ),
            "{got:?}"
        );
        let s = meter.snapshot();
        assert_eq!((s.items, s.total_items), (31, 31));
        assert!(!a.exists() && !b.exists());
    }

    #[test]
    fn a_cancel_stops_between_files_and_keeps_the_rest() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a");
        tree(&a, 200);
        let meter = Arc::new(Meter::default());
        meter.set_paused(true);
        let job =
            RemoveJob::start_metered(vec![Removal::Erase(a.clone())], None, meter.clone(), || {});
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while meter.snapshot().phase != Phase::Copying {
            assert!(std::time::Instant::now() < deadline, "the walk ends");
            std::thread::sleep(Duration::from_millis(2));
        }
        meter.cancel();
        meter.set_paused(false);
        let (wtx, woke) = channel::<()>();
        drop(wtx);
        let mut job = job;
        while !job.is_finished() {
            let _ = woke.recv_timeout(Duration::from_millis(5));
            let evs = job.poll();
            assert!(
                evs.iter().all(|e| matches!(e, RemoveEvent::Finished)),
                "a cancelled item reports nothing: {evs:?}"
            );
        }
        let left = progress::walk(&a, false, &AtomicBool::new(false)).unwrap();
        assert_eq!(left.items, 200, "nothing went past the pause");
    }

    #[test]
    fn empty_and_restore_report_per_item() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Trash");
        let home = t.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let keep = home.join("keep.txt");
        std::fs::write(&keep, "k").unwrap();
        let gone = home.join("gone");
        tree(&gone, 9);
        let kept = trash::trash_in(&root, &keep).unwrap();
        trash::trash_in(&root, &gone).unwrap();
        let (wtx, woke) = channel();
        let job = RemoveJob::start(
            vec![
                Removal::Restore {
                    files: kept.files.clone(),
                    record: Some(kept.clone()),
                },
                Removal::Empty(root.clone()),
            ],
            None,
            move || {
                let _ = wtx.send(());
            },
        );
        let got = run(job, woke);
        assert!(
            matches!(
                got.as_slice(),
                [
                    RemoveEvent::Done { index: 0, removed: Removed::Restored(to) },
                    RemoveEvent::Done { index: 1, removed: Removed::Emptied(1) },
                    RemoveEvent::Finished,
                ] if *to == keep
            ),
            "{got:?}"
        );
        assert!(keep.exists());
        assert_eq!(std::fs::read_dir(root.join("files")).unwrap().count(), 0);
    }
}
