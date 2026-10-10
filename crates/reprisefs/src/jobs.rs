//! File operations off the caller's thread. A [`PutJob`] runs a batch of [`ops::put_in`] calls on
//! its own thread and reports each item as it lands; cancel stops between items (and between a
//! folder copy's entries). The host polls from its loop.

use crate::ops::{self, DropOp};
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
    cancel: Arc<AtomicBool>,
    finished: bool,
}

impl PutJob {
    pub fn start(items: Vec<PutItem>, dir: PathBuf, wake: impl Fn() + Send + 'static) -> PutJob {
        let (tx, events) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let stop = cancel.clone();
        let spawned = std::thread::Builder::new()
            .name("reprisefs-put".into())
            .spawn(move || {
                let mut existing = existing_names(&dir);
                let mut ours = std::collections::HashSet::new();
                for (index, it) in items.iter().enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let r = ops::put_in_until(
                        &it.src,
                        &it.display_name,
                        it.is_app,
                        &dir,
                        it.op,
                        &existing,
                        &*it.shortcut,
                        it.replace
                            .filter(|_| !ours.contains(&it.src.file_name().map(OsString::from))),
                        &stop,
                    );
                    let ev = match r {
                        Ok(ops::Landing {
                            name: landed,
                            replaced,
                            unlisted,
                        }) => {
                            existing.push(landed.to_string_lossy().into_owned());
                            ours.insert(Some(landed.clone()));
                            PutEvent::Landed {
                                index,
                                landed,
                                replaced,
                                unlisted,
                            }
                        }
                        Err(_) if stop.load(Ordering::Relaxed) => break,
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
                let _ = tx.send(PutEvent::Finished);
                wake();
            });
        if let Err(e) = spawned {
            log::error!("reprisefs: cannot spawn the put thread: {e}");
        }
        PutJob {
            events,
            cancel,
            finished: false,
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
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
