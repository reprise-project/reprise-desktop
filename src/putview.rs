//! The desktop's file work on `reprisefs` jobs polled on the loop: a drop's or paste's put, one
//! per folder, deletes and Empty Recycle Bin, and Send To's zips, each with the progress dialog
//! once it runs past [`copyprogress::SHOW_AFTER`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use reprisefs::compress::{self, Compress};
use reprisefs::extract::{ArchiveEvent, ArchiveJob};
use reprisefs::jobs::{PutEvent, PutItem, PutJob};
use reprisefs::ops::DropOp;
use reprisefs::progress::{Phase as JobPhase, Snapshot};
use reprisefs::removal::{Removal, RemoveEvent, RemoveJob};
use reprisefs::trash::Bins;
use reprisekit::controls::copyprogress::{self, CopyProgress, Kind, Phase, Status};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::State;
use crate::dialog::Dialog;
use crate::dragview::{DragSource, Failure, Put, QueuedPut, shortcut_text};

const TICK: Duration = Duration::from_millis(copyprogress::FRAME_MS);

/// What a job's dialog shows and where it stands.
struct Shown {
    started: Instant,
    dialog: Option<u64>,
    kind: Kind,
    from: String,
    to: String,
    /// What tells the dialog apart from another job's.
    ident: PathBuf,
    cancelled: bool,
}

/// A delete or Empty Recycle Bin.
pub struct Removing {
    job: RemoveJob,
    /// The path each item names, for its error box.
    paths: Vec<PathBuf>,
    kind: Kind,
    started: Instant,
    dialog: Option<u64>,
    done: usize,
    failed: Option<Failure>,
}

/// A zip being made.
pub struct Zipping {
    job: ArchiveJob,
    from: String,
    to: String,
    ident: PathBuf,
    started: Instant,
    dialog: Option<u64>,
    failed: Option<Failure>,
}

/// The batch running into one folder.
pub struct Running {
    job: PutJob,
    op: DropOp,
    puts: Vec<Put>,
    started: Instant,
    /// The progress dialog's view once it is up; it stays `Some` while the view is closed until
    /// `present` opens it again.
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

/// A number of its own for a removal or a zip.
fn next_key() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
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
                    replace: replace.clone(),
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
        let Some(run) = self.drags.running.get(dir) else {
            return;
        };
        let st = status(run.job.progress(), waiting);
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
        let job = Shown {
            started: run.started,
            dialog: run.dialog,
            kind,
            from,
            to: folder_name(dir),
            ident: dir.to_path_buf(),
            cancelled: run.job.cancelled(),
        };
        let dialog = self.present(job, st);
        if let Some(run) = self.drags.running.get_mut(dir) {
            run.dialog = dialog;
        }
    }

    /// Open `job`'s dialog once it has run [`copyprogress::SHOW_AFTER`], and again when anything but
    /// its Cancel closed it (a Welcome panel, a session end, an unplug), else take `st` into it.
    /// Under a Welcome panel it waits for the panel to close. Returns the dialog's view.
    fn present(&mut self, job: Shown, st: Status) -> Option<u64> {
        let ms = job.started.elapsed().as_millis() as u64;
        let Some(i) = job.dialog.and_then(|id| self.dialog_index(id)) else {
            let early = job.started.elapsed() < copyprogress::SHOW_AFTER;
            if job.cancelled || early || self.dialog_is_modal() {
                return job.dialog;
            }
            let mut cp = CopyProgress::new(job.kind, &job.from, &job.to, self.config.glass.color);
            cp.update(st, ms);
            let opened = self.open_view(Dialog::copy_progress(cp, job.ident), None);
            return opened.or(job.dialog);
        };
        let d = &mut self.dialogs.views[i].dialog;
        let button = |d: &Dialog| {
            d.task_progress()
                .map(|p| ((p.fraction * 100.0) as u32, p.paused))
        };
        let was = button(d);
        if d.copy_mut().is_some_and(|c| c.cp.update(st, ms)) {
            self.redraw_dialog(i);
        }
        if button(&self.dialogs.views[i].dialog) != was {
            self.redraw_taskbars();
        }
        job.dialog
    }

    /// Close dialog `id` of a job that dealt with `done` items: a balloon tells of the end when the
    /// dialog was up but not focused and the job was not cancelled.
    fn close_progress(
        &mut self,
        id: Option<u64>,
        done: usize,
        cancelled: bool,
        failed: Option<&Failure>,
    ) {
        let Some(i) = id.and_then(|id| self.dialogs.views.iter().position(|v| v.id == id)) else {
            return;
        };
        let focused = i + 1 == self.dialogs.views.len() && self.dialog_has_keyboard();
        let note = self.dialogs.views[i]
            .dialog
            .copy()
            .map(|c| c.cp.ended_note(done));
        self.close_dialog_at(i);
        if !focused && !cancelled {
            let (summary, body) = match failed {
                Some((title, text)) => (title.to_string(), text.clone()),
                None => note.unwrap_or_default(),
            };
            crate::notifyview::notice(self, &summary, &body);
        }
    }

    /// The put into `dir` is over: its dialog closes, an error box says why it stopped, and the
    /// next batch starts.
    fn put_ended(&mut self, dir: &Path) {
        let Some(run) = self.drags.running.remove(dir) else {
            return;
        };
        let cancelled = run.job.cancelled();
        self.close_progress(run.dialog, run.landed, cancelled, run.failed.as_ref());
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

    /// The progress dialog `id`'s Pause or Resume.
    pub(crate) fn pause_put(&mut self, id: u64, on: bool) {
        if let Some(m) = self.meter_of_dialog(id) {
            m.set_paused(on);
            return;
        }
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
        if let Some(m) = self.meter_of_dialog(id) {
            m.set_paused(false);
            m.cancel();
            return;
        }
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

    /// What a put, delete or zip under way does, for the session-end prompt; None when none runs.
    pub(crate) fn files_under_way(&self) -> Option<&'static str> {
        let d = &self.drags;
        if let Some(r) = d.running.values().next() {
            return Some(match r.op {
                DropOp::Move => "Moving files",
                _ => "Copying files",
            });
        }
        if let Some(r) = d.removals.values().next() {
            return Some(match r.kind {
                Kind::Empty => "Emptying the Recycle Bin",
                _ => "Deleting files",
            });
        }
        d.zips.values().next().map(|_| "Compressing files")
    }

    /// The meter of the removal or zip whose progress dialog is `id`.
    fn meter_of_dialog(&self, id: u64) -> Option<&std::sync::Arc<reprisefs::progress::Meter>> {
        let d = &self.drags;
        let removal = d.removals.values().find(|r| r.dialog == Some(id));
        removal.map(|r| r.job.meter()).or_else(|| {
            let zip = d.zips.values().find(|z| z.dialog == Some(id));
            zip.map(|z| z.job.meter())
        })
    }

    fn put_of_dialog(&self, id: u64) -> Option<PathBuf> {
        self.drags
            .running
            .iter()
            .find(|(_, r)| r.dialog == Some(id))
            .map(|(d, _)| d.clone())
    }

    /// Run `items` (each naming the path at the same index of `paths`) on a removal job; `bins` is
    /// where a [`Removal::Trash`] goes.
    pub(crate) fn start_removal(
        &mut self,
        items: Vec<Removal>,
        paths: Vec<PathBuf>,
        bins: Option<Bins>,
        kind: Kind,
    ) {
        if items.is_empty() {
            return;
        }
        let key = next_key();
        let job = RemoveJob::start(items, bins, || {});
        let r = Removing {
            job,
            paths,
            kind,
            started: Instant::now(),
            dialog: None,
            done: 0,
            failed: None,
        };
        self.drags.removals.insert(key, r);
        let added = self
            .handle
            .insert_source(Timer::from_duration(TICK), move |_, _, state| {
                if state.poll_removal(key) {
                    TimeoutAction::ToDuration(TICK)
                } else {
                    TimeoutAction::Drop
                }
            });
        if let Err(e) = added {
            log::warn!("desktop: no timer for a delete's progress: {}", e.error);
        }
    }

    /// Take removal `key`'s news into its dialog; at the end close it and say why it stopped.
    /// False once it is over.
    fn poll_removal(&mut self, key: u64) -> bool {
        let Some(r) = self.drags.removals.get_mut(&key) else {
            return false;
        };
        let mut finished = false;
        for ev in r.job.poll() {
            match ev {
                RemoveEvent::Done { .. } => r.done += 1,
                RemoveEvent::Failed { index, error } => {
                    let path = &r.paths[index];
                    log::warn!("desktop: could not delete {}: {error}", path.display());
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    let text = match r.kind {
                        Kind::Empty => format!("Cannot empty the Recycle Bin: {error}"),
                        _ => format!("Cannot delete {name}: {error}"),
                    };
                    r.failed = Some(("Error Deleting File or Folder", text));
                }
                RemoveEvent::Finished => finished = true,
            }
        }
        if finished {
            let Some(r) = self.drags.removals.remove(&key) else {
                return false;
            };
            let cancelled = r.job.meter().cancelled();
            self.close_progress(r.dialog, r.done, cancelled, r.failed.as_ref());
            self.show_drop_failure(r.failed);
            return false;
        }
        let st = status(r.job.progress(), 0);
        let from = match r.kind {
            Kind::Empty => "Recycle Bin".to_string(),
            _ => r
                .paths
                .first()
                .and_then(|p| p.parent())
                .map(folder_name)
                .unwrap_or_default(),
        };
        let job = Shown {
            started: r.started,
            dialog: r.dialog,
            kind: r.kind,
            from,
            to: String::new(),
            ident: r.paths.first().cloned().unwrap_or_default(),
            cancelled: r.job.meter().cancelled(),
        };
        let dialog = self.present(job, st);
        if let Some(r) = self.drags.removals.get_mut(&key) {
            r.dialog = dialog;
        }
        true
    }

    /// Zips `c` on a job with the progress dialog; a failure shows an error box and leaves no zip.
    pub(crate) fn start_zip(&mut self, c: Compress) {
        let key = next_key();
        let name = |p: Option<&Path>| p.map(folder_name).unwrap_or_default();
        let z = Zipping {
            from: name(c.zip.parent()),
            to: name(Some(&c.zip)),
            ident: c.zip.clone(),
            job: ArchiveJob::compress(c, || {}),
            started: Instant::now(),
            dialog: None,
            failed: None,
        };
        self.drags.zips.insert(key, z);
        let added = self
            .handle
            .insert_source(Timer::from_duration(TICK), move |_, _, state| {
                if state.poll_zip(key) {
                    TimeoutAction::ToDuration(TICK)
                } else {
                    TimeoutAction::Drop
                }
            });
        if let Err(e) = added {
            log::warn!("desktop: no timer for a zip's progress: {}", e.error);
        }
    }

    /// Take zip `key`'s news into its dialog; at the end close it and say why it failed. False
    /// once it is over.
    fn poll_zip(&mut self, key: u64) -> bool {
        let Some(z) = self.drags.zips.get_mut(&key) else {
            return false;
        };
        let mut finished = false;
        for ev in z.job.poll() {
            match ev {
                ArchiveEvent::Landed { .. } => {}
                ArchiveEvent::Failed { text, .. } => {
                    z.failed = Some((compress::FAILURE_TITLE, text));
                }
                ArchiveEvent::Finished => finished = true,
            }
        }
        if finished {
            let Some(z) = self.drags.zips.remove(&key) else {
                return false;
            };
            let m = z.job.meter();
            let done = m.snapshot().items as usize;
            self.close_progress(z.dialog, done, m.cancelled(), z.failed.as_ref());
            self.show_drop_failure(z.failed);
            return false;
        }
        let job = Shown {
            started: z.started,
            dialog: z.dialog,
            kind: Kind::Compress,
            from: z.from.clone(),
            to: z.to.clone(),
            ident: z.ident.clone(),
            cancelled: z.job.meter().cancelled(),
        };
        let st = status(z.job.meter().snapshot(), 0);
        let dialog = self.present(job, st);
        if let Some(z) = self.drags.zips.get_mut(&key) {
            z.dialog = dialog;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use reprisefs::compress::Compress;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn sandbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reprise-zip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_zip(h: &mut crate::harness::Harness, c: Compress) {
        h.state.start_zip(c);
        let end = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !h.state.drags.zips.is_empty() {
            assert!(std::time::Instant::now() < end, "the zip ends");
            h.run_for(20);
        }
    }

    /// A copy of one file into a fresh folder, started but not polled: the loop takes its news
    /// only when a test turns it.
    fn copy_into(h: &mut crate::harness::Harness, name: &str) -> (PathBuf, PathBuf) {
        use crate::dragview::{DragSource, QueuedPut};
        let dir = sandbox(name);
        let path = dir.join("a.txt");
        std::fs::write(&path, "a").unwrap();
        let to = dir.join("to");
        std::fs::create_dir(&to).unwrap();
        let source = DragSource::File {
            path,
            name: "a.txt".into(),
        };
        h.state.spawn_put(QueuedPut {
            dir: to.clone(),
            op: reprisefs::ops::DropOp::Copy,
            puts: vec![((source, None, None), None)],
            existing: Vec::new(),
        });
        (dir, to)
    }

    /// The put into `to` as its poll sees it once it has run past `SHOW_AFTER`.
    fn shown_late(h: &mut crate::harness::Harness, to: &std::path::Path) {
        let run = h.state.drags.running.get_mut(to).unwrap();
        run.started -= super::copyprogress::SHOW_AFTER;
        h.state.show_put(to);
    }

    fn copy_view(h: &crate::harness::Harness) -> Option<usize> {
        (h.state.dialogs.views.iter()).position(|v| v.dialog.copy().is_some())
    }

    fn turn_off(h: &mut crate::harness::Harness) {
        let d = crate::dialog::Dialog::turn_off(&h.state.config.power, false);
        h.state.open_dialog(d);
        assert!(h.state.dialog_is_modal(), "the panel is on top");
    }

    fn close_top(h: &mut crate::harness::Harness) {
        h.state.close_dialog_at(h.state.dialogs.views.len() - 1);
    }

    #[test]
    fn a_copy_dialog_a_turn_off_panel_closed_comes_back_once_the_panel_is_cancelled() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let (dir, to) = copy_into(&mut h, "panel");
        shown_late(&mut h, &to);
        assert_eq!(copy_view(&h), Some(0), "the copy dialog is up");
        turn_off(&mut h);
        assert_eq!(copy_view(&h), None, "the panel takes the screen alone");
        h.state.show_put(&to);
        assert_eq!(copy_view(&h), None, "nothing opens under the panel");
        assert!(h.state.dialog_is_modal(), "the panel keeps the keys");
        close_top(&mut h);
        h.state.show_put(&to);
        let i = copy_view(&h).expect("the copy dialog is back");
        let v = &h.state.dialogs.views[i];
        assert!(v.dialog.on_taskbar(), "with its task button");
        assert_eq!(h.state.drags.running[&to].dialog, Some(v.id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_crossing_its_delay_under_a_welcome_panel_opens_once_the_panel_closes() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let (dir, to) = copy_into(&mut h, "under-panel");
        turn_off(&mut h);
        shown_late(&mut h, &to);
        assert_eq!(h.state.dialogs.views.len(), 1, "only the panel");
        assert!(h.state.dialog_is_modal(), "the panel keeps the keys");
        close_top(&mut h);
        h.state.show_put(&to);
        assert!(copy_view(&h).is_some(), "the copy dialog opens");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_copy_whose_dialog_was_closed_opens_no_other() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let (dir, to) = copy_into(&mut h, "cancelled");
        shown_late(&mut h, &to);
        let id = h.state.dialogs.views[copy_view(&h).unwrap()].id;
        h.state.cancel_put(id);
        h.state.close_all_dialogs();
        h.state.show_put(&to);
        assert_eq!(copy_view(&h), None, "the cancelled copy shows nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_copy_dialog_on_an_unplugged_output_moves_to_the_one_left() {
        let mut h = crate::harness::Harness::new();
        let first = h.output("TEST-1", 1024, 768);
        let second = h.output("TEST-2", 1024, 768);
        let b = h.state.layout_box(Some(&second));
        h.pointer_to(f64::from(b.x + b.width / 2), f64::from(b.y + b.height / 2));
        let (dir, to) = copy_into(&mut h, "unplug");
        shown_late(&mut h, &to);
        let i = copy_view(&h).unwrap();
        assert_eq!(
            h.state.dialogs.views[i].output, second,
            "it opened on TEST-2"
        );
        h.state.scene.remove_output(&second);
        h.state.output_removed(&second);
        let i = copy_view(&h).expect("the copy dialog stays up");
        let v = &h.state.dialogs.views[i];
        assert_eq!(v.output, first);
        let area = h.state.usable_area(Some(&first));
        let inside = v.x >= area.x
            && v.y >= area.y
            && v.x + v.layout.size.0 <= area.x + area.width
            && v.y + v.layout.size.1 <= area.y + area.height;
        assert!(inside, "inside TEST-1's usable area");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn send_to_zip_runs_on_a_job_and_lands_the_zip() {
        let dir = sandbox("lands");
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        std::fs::create_dir(dir.join("b")).unwrap();
        let paths = vec![dir.join("a.txt"), dir.join("b")];
        let c = reprisefs::sendto::zip_target(&paths, &[]).unwrap();
        let zip = c.zip.clone();
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        run_zip(&mut h, c);
        assert_eq!(zip, dir.join("a.zip"));
        assert!(zip.is_file(), "the zip landed");
        assert!(h.state.dialogs.views.is_empty(), "no error box");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_zip_that_cannot_be_written_shows_an_error_box_and_leaves_no_file() {
        let dir = sandbox("read-only");
        let src = dir.join("a.txt");
        std::fs::write(&src, "a").unwrap();
        let ro = std::fs::Permissions::from_mode(0o555);
        std::fs::set_permissions(&dir, ro).unwrap();
        let c = reprisefs::sendto::zip_target(&[src], &[]).unwrap();
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        run_zip(&mut h, c);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let titles: Vec<_> = (h.state.dialogs.views.iter())
            .map(|v| v.dialog.title.clone())
            .collect();
        assert_eq!(titles, [reprisefs::compress::FAILURE_TITLE]);
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
        assert_eq!(left.len(), 1, "only the source is left");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
