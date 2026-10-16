//! File operations off the caller's thread. A [`PutJob`] runs a batch of [`ops::put_in`] calls on
//! its own thread, reports each item as it lands and counts its progress ([`crate::progress`]);
//! cancel stops it inside a file. The host polls from its loop.

use crate::ops::{self, DropOp};
use crate::progress::{self, Meter, Phase, Snapshot, Totals};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

pub type ShortcutText = Box<dyn Fn(&str) -> Option<String> + Send>;

pub struct PutItem {
    pub src: PathBuf,
    pub display_name: String,
    pub is_app: bool,
    pub op: DropOp,
    pub shortcut: ShortcutText,
    pub replace: Option<crate::replace::Seen>,
}

#[derive(Debug)]
pub enum PutEvent {
    Landed {
        index: usize,
        landed: OsString,
        replaced: bool,
        unlisted: bool,
    },
    Failed {
        index: usize,
        error: ops::PutError,
    },
    Finished,
}

pub struct PutJob {
    events: Receiver<PutEvent>,
    meter: Arc<Meter>,
    finished: bool,
}

impl PutJob {
    pub fn start(items: Vec<PutItem>, dir: PathBuf, wake: impl Fn() + Send + 'static) -> PutJob {
        let (tx, events) = mpsc::channel();
        let meter = Arc::new(Meter::default());
        let m = meter.clone();
        let spawned = std::thread::Builder::new()
            .name("reprisefs-put".into())
            .spawn(move || {
                let stop = &m.cancel;
                let Some(sizes) = items
                    .iter()
                    .map(|it| {
                        let t = item_totals(it, &dir, stop);
                        m.add_totals(t.unwrap_or_default());
                        t
                    })
                    .collect::<Option<Vec<Totals>>>()
                else {
                    let _ = tx.send(PutEvent::Finished);
                    wake();
                    return;
                };
                m.set_phase(Phase::Copying);
                let mut existing = existing_names(&dir);
                let mut ours = std::collections::HashSet::new();
                let mut done = Totals::default();
                let mut wrote = false;
                for (index, it) in items.iter().enumerate() {
                    if m.hold().is_err() {
                        break;
                    }
                    m.set_current(it.src.file_name().unwrap_or(it.display_name.as_ref()));
                    wrote |= writes(it, &dir);
                    let r = progress::metered(&m, || {
                        ops::put_in_until(
                            &it.src,
                            &it.display_name,
                            it.is_app,
                            &dir,
                            it.op,
                            &existing,
                            &*it.shortcut,
                            it.replace.filter(|_| {
                                !ours.contains(&it.src.file_name().map(OsString::from))
                            }),
                            stop,
                        )
                    });
                    let ev = match r {
                        Ok(ops::Landing {
                            name: landed,
                            replaced,
                            unlisted,
                        }) => {
                            done = done + sizes[index];
                            m.settle(done);
                            existing.push(landed.to_string_lossy().into_owned());
                            ours.insert(Some(landed.clone()));
                            PutEvent::Landed {
                                index,
                                landed,
                                replaced,
                                unlisted,
                            }
                        }
                        Err(_) if m.cancelled() => break,
                        Err(error) => PutEvent::Failed { index, error },
                    };
                    let failed = matches!(ev, PutEvent::Failed { .. });
                    if tx.send(ev).is_err() {
                        return;
                    }
                    wake();
                    if failed {
                        break;
                    }
                }
                if wrote && !m.cancelled() {
                    m.set_phase(Phase::Finishing);
                    wake();
                    if let Ok(d) = std::fs::File::open(&dir) {
                        progress::sync_fs(&d);
                    }
                }
                let _ = tx.send(PutEvent::Finished);
                wake();
            });
        if let Err(e) = spawned {
            log::error!("reprisefs: cannot spawn the put thread: {e}");
        }
        PutJob {
            events,
            meter,
            finished: false,
        }
    }

    /// Stops the put between two chunks of a file; what that file had written is removed.
    pub fn cancel(&self) {
        self.meter.cancel.store(true, Ordering::Relaxed);
    }

    pub fn set_paused(&self, on: bool) {
        self.meter.set_paused(on);
    }

    pub fn cancelled(&self) -> bool {
        self.meter.cancelled()
    }

    pub fn progress(&self) -> Snapshot {
        self.meter.snapshot()
    }

    pub fn poll(&mut self) -> Vec<PutEvent> {
        let mut out = Vec::new();
        while !self.finished {
            match self.events.try_recv() {
                Ok(PutEvent::Finished) | Err(TryRecvError::Disconnected) => {
                    self.finished = true;
                    out.push(PutEvent::Finished);
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

/// What `it` adds to the totals: a move within one drive is a rename, one item of no bytes.
fn item_totals(it: &PutItem, dir: &Path, stop: &AtomicBool) -> Option<Totals> {
    if !writes(it, dir) {
        return Some(Totals { bytes: 0, items: 1 });
    }
    progress::walk(&it.src, it.op != DropOp::Move || it.is_app, stop)
}

/// Whether putting `it` in `dir` writes file data, rather than renaming or making a shortcut.
fn writes(it: &PutItem, dir: &Path) -> bool {
    match it.op {
        DropOp::Shortcut => false,
        DropOp::Move if !it.is_app => !ops::same_device(&it.src, dir),
        _ => true,
    }
}

fn existing_names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    fn item(src: PathBuf, op: DropOp) -> PutItem {
        PutItem {
            display_name: src.file_name().unwrap().to_string_lossy().into_owned(),
            src,
            is_app: false,
            op,
            shortcut: Box::new(|_| Some(String::new())),
            replace: None,
        }
    }

    fn run(mut job: PutJob, woke: Receiver<()>) -> Vec<PutEvent> {
        let mut all = Vec::new();
        while !job.is_finished() {
            woke.recv_timeout(Duration::from_secs(5)).expect("a wake");
            all.extend(job.poll());
        }
        all
    }

    #[test]
    fn a_batch_lands_in_order_and_finishes() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src"), t.path().join("dest"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::fs::write(src.join("b.txt"), "b").unwrap();
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![
                item(src.join("a.txt"), DropOp::Copy),
                item(src.join("b.txt"), DropOp::Move),
            ],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        let got = run(job, woke);
        assert!(
            matches!(
                got.as_slice(),
                [
                    PutEvent::Landed { index: 0, landed: a, .. },
                    PutEvent::Landed { index: 1, landed: b, .. },
                    PutEvent::Finished,
                ] if a == "a.txt" && b == "b.txt"
            ),
            "a.txt then b.txt landed, then Finished: {got:?}"
        );
        assert!(dest.join("a.txt").exists() && src.join("a.txt").exists());
        assert!(dest.join("b.txt").exists() && !src.join("b.txt").exists());
    }

    #[test]
    fn a_batch_never_replaces_its_own_item() {
        let t = tempfile::tempdir().unwrap();
        let (one, two, dest) = (
            t.path().join("one"),
            t.path().join("two"),
            t.path().join("dest"),
        );
        for d in [&one, &two, &dest] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(one.join("x.txt"), "one").unwrap();
        std::fs::write(two.join("x.txt"), "two").unwrap();
        std::fs::write(dest.join("x.txt"), "old").unwrap();
        let seen = crate::replace::Seen::of(&dest.join("x.txt"));
        let replacing = |src: PathBuf, op| PutItem {
            replace: seen,
            ..item(src, op)
        };
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![
                replacing(one.join("x.txt"), DropOp::Move),
                replacing(two.join("x.txt"), DropOp::Move),
            ],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        let got = run(job, woke);
        assert!(
            matches!(
                got.as_slice(),
                [
                    PutEvent::Landed { index: 0, landed: a, replaced: true, .. },
                    PutEvent::Landed { index: 1, landed: b, replaced: false, .. },
                    PutEvent::Finished,
                ] if a == "x.txt" && b == "x (2).txt"
            ),
            "the second lands numbered: {got:?}"
        );
        assert_eq!(std::fs::read_to_string(dest.join("x.txt")).unwrap(), "one");
        assert_eq!(
            std::fs::read_to_string(dest.join("x (2).txt")).unwrap(),
            "two"
        );
    }

    #[test]
    fn a_failure_stops_the_batch() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(t.path().join("ok.txt"), "").unwrap();
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![
                item(t.path().join("missing.txt"), DropOp::Copy),
                item(t.path().join("ok.txt"), DropOp::Copy),
            ],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        let got = run(job, woke);
        assert!(matches!(got[0], PutEvent::Failed { index: 0, .. }));
        assert!(matches!(got[1], PutEvent::Finished));
        assert!(!dest.join("ok.txt").exists());
    }

    #[test]
    fn a_copy_counts_every_byte_and_flushes_before_it_finishes() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("src"), t.path().join("dest"));
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("a"), vec![1; 3000]).unwrap();
        std::fs::write(src.join("sub/b"), vec![2; 5000]).unwrap();
        std::fs::write(t.path().join("c"), vec![3; 2_500_000]).unwrap();
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![
                item(src.clone(), DropOp::Copy),
                item(t.path().join("c"), DropOp::Copy),
            ],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        let meter = job.meter.clone();
        run(job, woke);
        let got = meter.snapshot();
        assert_eq!((got.bytes, got.total_bytes), (2_508_000, 2_508_000));
        assert_eq!((got.items, got.total_items), (3, 3));
        assert_eq!(got.phase, Phase::Finishing, "a copy ends by flushing");
        assert_eq!(std::fs::read(dest.join("c")).unwrap().len(), 2_500_000);
    }

    #[test]
    fn a_move_within_a_drive_is_one_item_of_no_bytes_and_no_flush() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(t.path().join("big"), vec![0; 100_000]).unwrap();
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![item(t.path().join("big"), DropOp::Move)],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        let meter = job.meter.clone();
        run(job, woke);
        let got = meter.snapshot();
        assert_eq!((got.total_bytes, got.total_items, got.items), (0, 1, 1));
        assert_eq!(got.phase, Phase::Copying);
    }

    #[test]
    fn a_paused_copy_holds_mid_file_and_a_cancel_removes_the_part() {
        let t = tempfile::tempdir().unwrap();
        let (src, dest) = (t.path().join("big"), t.path().join("dest"));
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(&src, vec![7; 3 << 20]).unwrap();
        let meter = Arc::new(Meter::default());
        meter.set_paused(true);
        let m = meter.clone();
        let to = dest.clone();
        let copy = std::thread::spawn(move || {
            progress::metered(&m, || {
                ops::put_in(&src, "big", false, &to, DropOp::Copy, &[], &|_| None)
            })
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while meter.snapshot().bytes == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            meter.snapshot().bytes,
            1 << 20,
            "held after the first chunk"
        );
        assert!(dest.join("big").exists(), "the part is there while paused");
        meter.cancel.store(true, Ordering::Relaxed);
        assert!(copy.join().unwrap().is_err(), "a cancelled copy fails");
        assert!(!dest.join("big").exists(), "and leaves no part behind");
    }

    #[test]
    fn cancel_skips_the_rest() {
        let t = tempfile::tempdir().unwrap();
        let dest = t.path().join("dest");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(t.path().join("a.txt"), "").unwrap();
        let (wtx, woke) = channel();
        let job = PutJob::start(
            vec![item(t.path().join("a.txt"), DropOp::Copy)],
            dest.clone(),
            move || {
                let _ = wtx.send(());
            },
        );
        job.cancel();
        let got = run(job, woke);
        assert!(matches!(got.last(), Some(PutEvent::Finished)));
        assert!(got.len() <= 2);
    }
}
