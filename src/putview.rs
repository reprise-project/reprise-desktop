//! A desktop drop's or paste's file work: one `reprisefs` put job per folder, polled on the loop,
//! with the copy progress dialog once it runs past [`copyprogress::SHOW_AFTER`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reprisefs::jobs::{PutEvent, PutItem, PutJob};
use reprisefs::ops::DropOp;
use reprisefs::progress::{Phase as JobPhase, Snapshot};
use reprisekit::controls::copyprogress::{self, CopyProgress, Kind, Phase, Status};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::dialog::Dialog;
use crate::dragview::{DragSource, Failure, Put, QueuedPut, shortcut_text};

const TICK: Duration = Duration::from_millis(copyprogress::FRAME_MS);

/// The batch running into one folder.
pub struct Running {
    job: PutJob,
    op: DropOp,
    puts: Vec<Put>,
    started: Instant,
    /// The progress dialog's view once it is up; it stays `Some` after the view is closed.
    dialog: Option<u64>,
    landed: usize,
    failed: Option<Failure>,
}

fn status(s: Snapshot, waiting: usize) -> Status {
    Status {
        phase: match s.phase {
            JobPhase::Preparing => Phase::Preparing,
            JobPhase::Copying => Phase::Copying,
            JobPhase::Finishing => Phase::Finishing,
        },
        paused: s.paused,
        bytes: s.bytes,
        total_bytes: s.total_bytes,
        items: s.items,
        total_items: s.total_items,
        current: s.current,
        waiting,
    }
}

fn folder_name(p: &Path) -> String {
    p.file_name().map_or_else(
        || p.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

impl State {
    /// Start `q` on a put job; when it ends, the next batch for the same folder starts. One batch per
    /// folder at a time, so two drops never race for a name.
    pub(crate) fn spawn_put(&mut self, q: QueuedPut) {
        let QueuedPut {
            dir,
            op,
            puts,
            existing,
        } = q;
        let items = puts
            .iter()
            .map(|((source, _, link), replace)| {
                let (s, l) = (source.clone(), link.clone());
                PutItem {
                    src: source.path().to_path_buf(),
                    display_name: source.name().to_string(),
                    is_app: matches!(source, DragSource::App { .. }),
                    op,
                    shortcut: Box::new(move |name| shortcut_text(&s, l.as_ref(), name)),
                    replace: *replace,
                }
            })
            .collect();
        let job = PutJob::start_past(items, dir.clone(), existing, || {});
        let running = Running {
            job,
            op,
            puts: puts.into_iter().map(|(p, _)| p).collect(),
            started: Instant::now(),
            dialog: None,
            landed: 0,
            failed: None,
        };
        self.drags.running.insert(dir.clone(), running);
        let added = self
            .handle
            .insert_source(Timer::from_duration(TICK), move |_, _, state| {
                if state.poll_put(&dir) {
                    TimeoutAction::ToDuration(TICK)
                } else {
                    TimeoutAction::Drop
                }
            });
        if let Err(e) = added {
            log::warn!("desktop: no timer for a put's progress: {}", e.error);
        }
    }

    /// Take the put into `dir`'s news: place what landed, keep its dialog up to date, and on the end
    /// close the dialog and start the folder's next batch. False once the put is over.
    fn poll_put(&mut self, dir: &Path) -> bool {
        let Some(run) = self.drags.running.get_mut(dir) else {
            return false;
        };
        let mut cells = Vec::new();
        let mut moved = Vec::new();
        let mut finished = false;
        for ev in run.job.poll() {
            match ev {
                PutEvent::Landed { index, landed, .. } => {
                    run.landed += 1;
                    let (source, cell, _) = &run.puts[index];
                    if let Some(cell) = cell {
                        cells.push((landed.to_string_lossy().into_owned(), *cell));
                    }
                    if run.op == DropOp::Move {
                        moved.push(source.path().to_path_buf());
                    }
                }
                PutEvent::Failed { index, error } => {
                    let name = run.puts[index].0.name();
                    log::warn!("drop into {}: {error}", dir.display());
                    run.failed = Some(run.op.failure(name, &error));
                }
                PutEvent::Finished => finished = true,
            }
        }
        for (name, cell) in cells {
            self.land_icon(&name, cell);
        }
        if !moved.is_empty() {
            self.cut_moved(&moved);
        }
        if finished {
            self.put_ended(dir);
            return false;
        }
        self.show_put(dir);
        true
    }

    /// Open the put's dialog once it has run [`copyprogress::SHOW_AFTER`], else bring its status
    /// up to date.
    fn show_put(&mut self, dir: &Path) {
        let waiting = self.drags.put_queues.get(dir).map_or(0, |q| q.len());
        let Some(run) = self.drags.running.get_mut(dir) else {
            return;
        };
        let st = status(run.job.progress(), waiting);
        let ms = run.started.elapsed().as_millis() as u64;
        let Some(id) = run.dialog else {
            if run.started.elapsed() < copyprogress::SHOW_AFTER {
                return;
            }
            let kind = if run.op == DropOp::Move {
                Kind::Move
            } else {
                Kind::Copy
            };
            let from = run
                .puts
                .first()
                .and_then(|(s, _, _)| s.path().parent())
                .map(folder_name)
                .unwrap_or_default();
            let mut cp = CopyProgress::new(kind, &from, &folder_name(dir), self.config.glass.color);
            cp.update(st, ms);
            let before = self.dialogs.views.last().map(|v| v.id);
            self.open_dialog(Dialog::copy_progress(cp, dir.to_path_buf()));
            let id = self
                .dialogs
                .views
                .last()
                .map(|v| v.id)
                .filter(|&id| Some(id) != before);
            if let Some(run) = self.drags.running.get_mut(dir) {
                run.dialog = Some(id.unwrap_or(u64::MAX));
            }
            return;
        };
        let Some(i) = self.dialogs.views.iter().position(|v| v.id == id) else {
            return;
        };
        let changed = self.dialogs.views[i]
            .dialog
            .copy_mut()
            .is_some_and(|c| c.cp.update(st, ms));
        if changed {
            self.redraw_dialog(i);
        }
    }

    /// The put into `dir` is over: its dialog closes, a balloon tells of the end when the dialog was
    /// up but not looked at, an error box says why it stopped, and the next batch starts.
    fn put_ended(&mut self, dir: &Path) {
        let Some(run) = self.drags.running.remove(dir) else {
            return;
        };
        let view = run
            .dialog
            .and_then(|id| self.dialogs.views.iter().position(|v| v.id == id));
        if let Some(i) = view {
            let focused = i + 1 == self.dialogs.views.len() && self.dialog_has_keyboard();
            let note = self.dialogs.views[i]
                .dialog
                .copy()
                .map(|c| c.cp.ended_note(run.landed));
            self.close_dialog_at(i);
            if !focused && !run.job.cancelled() {
                let (summary, body) = match &run.failed {
                    Some((title, text)) => (title.to_string(), text.clone()),
                    None => note.unwrap_or_default(),
                };
                crate::notifyview::notice(self, &summary, &body);
            }
        }
        self.show_drop_failure(run.failed);
        self.put_done(dir);
    }

    fn put_done(&mut self, dir: &Path) {
        let next = self
            .drags
            .put_queues
            .get_mut(dir)
            .and_then(|q| q.pop_front());
        match next {
            Some(q) => self.spawn_put(q),
            None => {
                self.drags.put_queues.remove(dir);
            }
        }
    }

    /// The copy dialog `id`'s Pause or Resume.
    pub(crate) fn pause_put(&mut self, id: u64, on: bool) {
        if let Some(dir) = self.put_of_dialog(id) {
            if let Some(run) = self.drags.running.get(&dir) {
                run.job.set_paused(on);
            }
            self.show_put(&dir);
        }
    }

    /// The copy dialog `id`'s Cancel: the put stops inside its file and the batches waiting for
    /// the same folder never start.
    pub(crate) fn cancel_put(&mut self, id: u64) {
        let Some(dir) = self.put_of_dialog(id) else {
            return;
        };
        if let Some(q) = self.drags.put_queues.get_mut(&dir) {
            q.clear();
        }
        if let Some(run) = self.drags.running.get(&dir) {
            run.job.set_paused(false);
            run.job.cancel();
        }
    }

    fn put_of_dialog(&self, id: u64) -> Option<PathBuf> {
        self.drags
            .running
            .iter()
            .find(|(_, r)| r.dialog == Some(id))
            .map(|(d, _)| d.clone())
    }
}
