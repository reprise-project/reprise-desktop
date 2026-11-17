//! Delete, Shift+Delete, the Recycle Bin's Empty, Delete and Restore, and Undo, on a reprisefs
//! [`RemoveJob`], with the progress dialog ([`crate::copydlg`]). One runs at a time; others wait.

use crate::Explorer;
use crate::copydlg::Progress;
use crate::fileops::{BinItem, file_label};
use crate::place::Place;
use repriseapp::{Ctx, Reply};
use reprisefs::bincap::Plan;
use reprisefs::netshare::Share;
use reprisefs::removal::{Removal, RemoveEvent, RemoveJob, Removed};
use reprisefs::trash::Bins;
use reprisefs::undo::{Undo, Undoable};
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
    /// Undo's steps: what they leave undone goes back on the stack.
    Undo,
}

/// The entry an undo removal reverses, the folders it touches and the failure that stopped it.
pub(crate) struct Undoing {
    undo: Undo,
    dirs: Vec<PathBuf>,
    error: Option<std::io::Error>,
}

pub(crate) struct PendingRemoval {
    items: Vec<Removal>,
    /// The path each item names, for its error box and the undo stack.
    paths: Vec<PathBuf>,
    bins: Option<Bins>,
    purpose: Purpose,
    kind: Kind,
    from: String,
    /// Where an undo puts its items back; the other removals name no folder.
    to: String,
    undo: Option<Undoing>,
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
    undo: Option<Undoing>,
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
            to: String::new(),
            undo: None,
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
            to: String::new(),
            undo: None,
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
            to: String::new(),
            undo: None,
        }
    }

    /// Undo's steps of `undo`, which touches `dirs`; `erase` removes for good what `bins` refuse.
    pub(crate) fn undo(
        undo: Undo,
        erase: bool,
        dirs: Vec<PathBuf>,
        bins: Option<Bins>,
        shares: &[Share],
    ) -> Self {
        let (paths, items) = undo
            .steps()
            .iter()
            .map(|step| {
                let path = match step {
                    Undoable::Move { to, .. } | Undoable::Rename { to, .. } => to.clone(),
                    Undoable::Trash(t) => t.files.clone(),
                    Undoable::Copied(p) | Undoable::New(p) => p.clone(),
                    Undoable::Batch(_) => PathBuf::new(),
                };
                let step = step.clone();
                (path, Removal::Undo { step, erase })
            })
            .unzip();
        let title = |p: &std::path::Path| p.parent().map(|d| Place::from_path(d).title_in(shares));
        let (kind, from, to) = match undo.steps().first() {
            Some(Undoable::Move { from, to, .. } | Undoable::Rename { from, to }) => {
                (Kind::Move, title(to), title(from))
            }
            Some(Undoable::Trash(t)) => (
                Kind::Restore,
                Some(Place::RecycleBin.title()),
                title(&t.original),
            ),
            Some(Undoable::Copied(p) | Undoable::New(p)) => (
                if erase { Kind::Erase } else { Kind::Delete },
                title(p),
                None,
            ),
            _ => (Kind::Delete, None, None),
        };
        PendingRemoval {
            items,
            paths,
            bins,
            purpose: Purpose::Undo,
            kind,
            from: from.unwrap_or_default(),
            to: to.unwrap_or_default(),
            undo: Some(Undoing {
                undo,
                dirs,
                error: None,
            }),
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
        let wake = move || wake.wake();
        // Undo stops at its first failure, as the stack keeps what it did not reverse.
        let job = match p.purpose {
            Purpose::Undo => RemoveJob::start(p.items, p.bins, wake),
            _ => RemoveJob::start_asking(p.items, p.bins, wake),
        };
        crate::fileops::wake_to_show(ctx);
        let progress = Progress::new(job.meter().clone(), p.kind, p.from, p.to);
        self.removing = Some(Removing {
            job,
            paths: p.paths,
            purpose: p.purpose,
            progress,
            batch: Vec::new(),
            gone: Vec::new(),
            done: 0,
            failed: None,
            undo: p.undo,
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
                        Removed::Undone { purged } => r.gone.extend(purged),
                    }
                }
                RemoveEvent::Failed { error, .. } if r.purpose == Purpose::Undo => {
                    if let Some(u) = r.undo.as_mut() {
                        u.error.get_or_insert(error);
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
        if !finished && let Some((title, text)) = ask {
            let meter = r.progress.meter.clone();
            return Some(self.ask_failed(reprisefs::progress::Asked::new(meter), title, text, ctx));
        }
        // A job that ended does not wait for an answer: its end tells the failure.
        if let Some(failure) = ask {
            r.failed.get_or_insert(failure);
        }
        if !finished {
            return Some(Reply::none());
        }
        let mut r = self.removing.take()?;
        self.job_ended(&r.progress.meter, ctx);
        let undo_failed = r.undo.as_ref().and_then(|u| u.error.as_ref());
        let failed = (r.failed.as_ref().cloned())
            .or_else(|| undo_failed.map(|e| ("Error", format!("Cannot undo: {e}"))));
        r.progress.ended(r.done, failed.as_ref());
        self.publish_task_progress();
        if r.purpose == Purpose::Empty && r.failed.is_none() && !r.progress.meter.cancelled() {
            self.undos.retain_non_trash();
        }
        self.undos.push_all(r.batch);
        let undo = r.undo.take();
        if let Some(u) = &undo {
            for d in &u.dirs {
                self.refill_tree_node(&Place::from_path(d), ctx);
            }
        }
        let error = undo.and_then(|u| {
            if let Some(left) = u.undo.left(r.done, u.error.as_ref()) {
                self.undos.push(left);
            }
            u.error
        });
        self.undos.forget_gone(&r.gone);
        self.sync_undo();
        let mut reply = self.refresh(ctx);
        if let Some((title, text)) = r.failed {
            reply = self.message_box(title, text, ctx);
        }
        if let Some(e) = error {
            reply = reply.and(self.undo_failed(&e, ctx));
        }
        if let Some(next) = self.remove_queue.pop_front() {
            self.remove(next, ctx);
        }
        Some(reply)
    }
}
