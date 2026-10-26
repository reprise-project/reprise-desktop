//! Deletes, Empty Recycle Bin and Restore off the caller's thread, counted like a put
//! ([`crate::jobs::PutJob`]): a walk of every item for the totals, then the items in order,
//! reported one by one. A pause or cancel takes effect between two files; a job stuck for
//! [`ACTION_DEADLINE`] fails as not responding.

use crate::bincap::Measured;
use crate::deadline::ACTION_DEADLINE;
use crate::progress::{self, Decision, Meter, Phase, Snapshot, Totals};
use crate::trash::{self, Bins, Trashed};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

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
    fn totals(&self, m: &Meter) -> Option<Totals> {
        let path = match self {
            Removal::Trash { path, .. } | Removal::Erase(path) => path.clone(),
            Removal::EraseTrashed { files, .. } | Removal::Restore { files, .. } => files.clone(),
            Removal::Empty(root) => root.join("files"),
        };
        progress::walk(&path, false, m)
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

/// The error a removal fails with once it made no headway for [`ACTION_DEADLINE`].
fn not_responding() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "The location is not responding.")
}

impl RemoveJob {
    /// Start `items` on a thread of their own; `bins` is where [`Removal::Trash`] goes. `wake`
    /// runs after each report. The first failure stops the rest, except that an Empty goes on to
    /// the next bin.
    pub fn start(
        items: Vec<Removal>,
        bins: Option<Bins>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> RemoveJob {
        Self::start_metered(
            items,
            bins,
            Arc::new(Meter::default()),
            ACTION_DEADLINE,
            wake,
        )
    }

    /// [`Self::start`], holding a failed item for the host's [`Decision`] ([`Meter::decide`])
    /// instead of stopping.
    pub fn start_asking(
        items: Vec<Removal>,
        bins: Option<Bins>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> RemoveJob {
        let meter = Arc::new(Meter::default());
        meter.ask_on_failure();
        Self::start_metered(items, bins, meter, ACTION_DEADLINE, wake)
    }

    /// [`Self::start`], counting on `meter` and failing after `deadline` without headway, so a test
    /// can pause it before the thread runs or wait less.
    fn start_metered(
        items: Vec<Removal>,
        bins: Option<Bins>,
        meter: Arc<Meter>,
        deadline: Duration,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> RemoveJob {
        let (tx, events) = mpsc::channel();
        let wake = Arc::new(wake);
        let ended = Arc::new(AtomicBool::new(false));
        let (watch_tx, watch_wake) = (tx.clone(), wake.clone());
        progress::watch(&meter, &ended, deadline, "removal", move |m| {
            let error = not_responding();
            let _ = watch_tx.send(RemoveEvent::Failed {
                index: m.at(),
                error,
            });
            let _ = watch_tx.send(RemoveEvent::Finished);
            (*watch_wake)();
        });
        let m = meter.clone();
        let spawned = std::thread::Builder::new()
            .name("reprisefs-remove".into())
            .spawn(move || {
                let finish = || {
                    if !ended.swap(true, Ordering::AcqRel) {
                        let _ = tx.send(RemoveEvent::Finished);
                        (*wake)();
                    }
                };
                let Some(sizes) = items
                    .iter()
                    .enumerate()
                    .map(|(index, it)| {
                        m.set_at(index);
                        let t = it.totals(&m);
                        m.add_totals(t.unwrap_or_default());
                        t
                    })
                    .collect::<Option<Vec<Totals>>>()
                else {
                    finish();
                    return;
                };
                m.set_phase(Phase::Copying);
                let mut done = Totals::default();
                let mut index = 0;
                while let Some(it) = items.get(index) {
                    if m.hold().is_err() {
                        break;
                    }
                    m.set_at(index);
                    m.set_current(it.name());
                    let r = progress::metered(&m, || it.run(bins.as_ref(), &m));
                    let stop = r.is_err() && !matches!(it, Removal::Empty(_));
                    let ev = match r {
                        Ok(removed) => RemoveEvent::Done { index, removed },
                        Err(_) if m.cancelled() => break,
                        Err(error) => RemoveEvent::Failed { index, error },
                    };
                    if stop {
                        m.ask();
                    }
                    if tx.send(ev).is_err() {
                        ended.store(true, Ordering::Release);
                        return;
                    }
                    (*wake)();
                    if stop {
                        match m.decision() {
                            Decision::Retry => continue,
                            Decision::Skip => {}
                            Decision::Pending | Decision::Abort => break,
                        }
                    }
                    done = done + sizes[index];
                    m.settle(done);
                    index += 1;
                }
                finish();
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
        let job = RemoveJob::start_metered(
            vec![Removal::Erase(a.clone())],
            None,
            meter.clone(),
            ACTION_DEADLINE,
            || {},
        );
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
        let left = progress::walk(&a, false, &Meter::default()).unwrap();
        assert_eq!(left.items, 200, "nothing went past the pause");
    }

    #[test]
    fn a_removal_that_stops_answering_fails_as_not_responding_and_finishes() {
        use std::os::unix::ffi::OsStrExt;
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.txt");
        std::fs::write(&a, "a").unwrap();
        let root = t.path().join("Trash");
        std::fs::create_dir_all(root.join("files/x")).unwrap();
        std::fs::create_dir_all(root.join("info")).unwrap();
        // Reading a FIFO with no writer blocks, as a call on a drive that stopped answering does.
        let info = root.join("info/x.trashinfo");
        let c = std::ffi::CString::new(info.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let (wtx, woke) = channel();
        let job = RemoveJob::start_metered(
            vec![
                Removal::Erase(a.clone()),
                Removal::EraseTrashed {
                    files: root.join("files/x"),
                    record: None,
                },
            ],
            None,
            Arc::new(Meter::default()),
            Duration::from_millis(300),
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
                    RemoveEvent::Done { index: 0, removed: Removed::Erased },
                    RemoveEvent::Failed { index: 1, error },
                    RemoveEvent::Finished,
                ] if error.kind() == io::ErrorKind::TimedOut
            ),
            "a.txt goes, the stuck entry fails as not responding, then Finished: {got:?}"
        );
        assert!(
            meter.cancelled(),
            "the stuck call stops the job once it returns"
        );
        assert!(
            root.join("files/x").exists(),
            "nothing goes past the stuck read"
        );
        let writer = std::fs::OpenOptions::new().write(true).open(&info).unwrap();
        drop(writer);
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
