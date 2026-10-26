//! Delete, Shift+Delete, the Recycle Bin's Empty, Delete and Restore on a reprisefs
//! [`RemoveJob`], with the progress dialog ([`crate::copydlg`]). One runs at a time; others wait.

use crate::Explorer;
use crate::copydlg::Progress;
use crate::fileops::{BinItem, file_label};
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::bincap::Plan;
use reprisefs::removal::{Removal, RemoveEvent, RemoveJob, Removed};
use reprisefs::trash::Bins;
use reprisefs::undo::Undoable;
use reprisekit::controls::copyprogress::Kind;
use std::path::PathBuf;

/// What a removal is for, which says what its end does with the results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    /// Delete or Shift+Delete outside the bin: one undo for what went to the bin.
    Delete,
    /// Delete in the bin: the items' undos go.
    EraseFromBin,
    Restore,
    Empty,
}

pub(crate) struct PendingRemoval {
    items: Vec<Removal>,
    /// The path each item names, for its error box and the undo stack.
    paths: Vec<PathBuf>,
    bins: Option<Bins>,
    purpose: Purpose,
    kind: Kind,
    from: String,
}

pub(crate) struct Removing {
    job: RemoveJob,
    paths: Vec<PathBuf>,
    purpose: Purpose,
    pub(crate) progress: Progress,
    batch: Vec<Undoable>,
    /// Paths whose undos no longer apply: purged to make room, erased or restored.
    gone: Vec<PathBuf>,
    done: usize,
    failed: Option<(&'static str, String)>,
}

impl PendingRemoval {
    /// Delete or Shift+Delete of `plan` outside the bin, from the folder titled `from`.
    pub(crate) fn delete(plan: Vec<(PathBuf, Plan)>, bins: Option<Bins>, from: String) -> Self {
        let erase = plan.iter().all(|(_, p)| matches!(p, Plan::Erase));
        let (paths, items) = plan
            .into_iter()
            .map(|(path, p)| (path.clone(), Removal::planned(path, p)))
            .unzip();
        PendingRemoval {
            items,
            paths,
            bins,
            purpose: Purpose::Delete,
            kind: if erase { Kind::Erase } else { Kind::Delete },
            from,
        }
    }

    /// Delete (`restore` false) or Restore of the bin's `items`.
    pub(crate) fn from_bin(items: Vec<BinItem>, restore: bool) -> Self {
        let (paths, items) = items
            .into_iter()
            .map(|BinItem { path, record }| {
                let files = path.clone();
                let item = if restore {
                    Removal::Restore { files, record }
                } else {
                    Removal::EraseTrashed { files, record }
                };
                (path, item)
            })
            .unzip();
        PendingRemoval {
            items,
            paths,
            bins: None,
            purpose: if restore {
                Purpose::Restore
            } else {
                Purpose::EraseFromBin
            },
            kind: if restore { Kind::Restore } else { Kind::Erase },
            from: Place::RecycleBin.title(),
        }
    }

    /// Empty every trash `bins` knows.
    pub(crate) fn empty(bins: Bins) -> Self {
        let roots = bins.roots();
        PendingRemoval {
            items: roots.iter().cloned().map(Removal::Empty).collect(),
            paths: roots,
            bins: None,
            purpose: Purpose::Empty,
            kind: Kind::Empty,
            from: Place::RecycleBin.title(),
        }
    }
}

impl Explorer {
    /// Run `p` now, or after the removal running now.
    pub(crate) fn remove(&mut self, p: PendingRemoval, ctx: &Ctx) {
        if p.items.is_empty() {
            return;
        }
        if self.removing.is_some() {
            self.remove_queue.push_back(p);
            return;
        }
        let wake = ctx.wake.clone();
        let job = RemoveJob::start_asking(p.items, p.bins, move || wake.wake());
        crate::fileops::wake_to_show(ctx);
        let progress = Progress::new(job.meter().clone(), p.kind, p.from, String::new());
        self.removing = Some(Removing {
            job,
            paths: p.paths,
            purpose: p.purpose,
            progress,
            batch: Vec::new(),
            gone: Vec::new(),
            done: 0,
            failed: None,
        });
    }

    pub(crate) fn poll_removal(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let r = self.removing.as_mut()?;
        let mut finished = false;
        let mut ask = None;
        for ev in r.job.poll() {
            match ev {
                RemoveEvent::Done { index, removed } => {
                    r.done += 1;
                    let path = r.paths[index].clone();
                    match removed {
                        Removed::Trashed { trashed, purged } => {
                            r.batch.push(Undoable::Trash(trashed));
                            r.gone.extend(purged);
                        }
                        Removed::Erased | Removed::Restored(_) => r.gone.push(path),
                        Removed::Emptied(n) => log::info!("explorer: emptied {n} items"),
                    }
                }
                RemoveEvent::Failed { index, error } => {
                    let path = &r.paths[index];
                    let failure = match r.purpose {
                        Purpose::Restore => (
                            "Error Restoring File or Folder",
                            format!("Cannot restore {}: {error}", file_label(path)),
                        ),
                        Purpose::Empty => (
                            "Error Deleting File or Folder",
                            format!("Cannot empty the Recycle Bin: {error}"),
                        ),
                        _ => (
                            "Error Deleting File or Folder",
                            format!("Cannot delete {}: {error}", file_label(path)),
                        ),
                    };
                    if r.progress.meter.cancelled() || r.purpose == Purpose::Empty {
                        r.failed.get_or_insert(failure);
                    } else {
                        ask = Some(failure);
                    }
                }
                RemoveEvent::Finished => finished = true,
            }
        }
        if let Some((title, text)) = ask {
            let meter = r.progress.meter.clone();
            return Some(self.ask_failed(meter, title, text, ctx));
        }
        if !finished {
            return Some(Reply::none());
        }
        let r = self.removing.take()?;
        r.progress.ended(r.done, r.failed.as_ref());
        self.publish_task_progress();
        if r.purpose == Purpose::Empty && r.failed.is_none() && !r.progress.meter.cancelled() {
            self.undos.retain_non_trash();
        }
        self.undos.push_all(r.batch);
        self.undos.forget_gone(&r.gone);
        self.sync_undo();
        let mut reply = self.refresh(ctx);
        if let Some((title, text)) = r.failed {
            reply = self.message_box(title, text, ctx);
        }
        if let Some(next) = self.remove_queue.pop_front() {
            self.remove(next, ctx);
        }
        Some(reply)
    }
}
