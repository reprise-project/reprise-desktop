//! The progress dialogs' host half, for a put (copy, move), a removal (delete, Empty, Restore) and an
//! extraction or compression:
//! when each comes up, its window and events, Pause and Cancel on its job, the task button's
//! progress for all of them together, the balloon for a job that ends while its dialog is not looked at, and
//! the logout inhibitor the process holds while one runs.

use crate::Explorer;
use repriseapp::{Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisefs::progress::{Meter, Phase as JobPhase, Snapshot};
use reprisekit::controls::copyprogress::{CopyProgress, Kind, Outcome, Phase, SHOW_AFTER, Status};
use reprisekit::look::Paint;
use reprisekit::theme::Color;
use std::sync::Arc;
use std::time::Instant;

pub const COPY: WinId = WinId::named("copy progress");
pub const REMOVE: WinId = WinId::named("remove progress");
pub const EXTRACT: WinId = WinId::named("extract progress");
/// The desktop file a task button's progress is sent for.
pub(crate) const DESKTOP_ID: &str = "reprise-explorer";

/// The dialog of a running job, once it is up.
pub(crate) struct Shown {
    pub cp: CopyProgress,
    pub focused: bool,
}

/// A job a progress dialog shows: its counts, what it does from where to where, and what its
/// dialog and task button show.
pub(crate) struct Progress {
    pub meter: Arc<Meter>,
    pub kind: Kind,
    pub from: String,
    pub to: String,
    pub started: Instant,
    pub shown: Option<Shown>,
}

pub(crate) fn status(s: Snapshot, waiting: usize) -> Status {
    Status {
        phase: match s.phase {
            JobPhase::Preparing => Phase::Preparing,
            JobPhase::Copying => Phase::Copying,
            JobPhase::Finishing => Phase::Finishing,
        },
        paused: s.paused,
        asking: s.asking,
        bytes: s.bytes,
        total_bytes: s.total_bytes,
        items: s.items,
        total_items: s.total_items,
        current: s.current,
        waiting,
    }
}

/// The glass colour `[glass] color` names, for the graph.
fn accent() -> Color {
    repriseconf::current().glass.color
}

impl Progress {
    pub fn new(meter: Arc<Meter>, kind: Kind, from: String, to: String) -> Progress {
        Progress {
            meter,
            kind,
            from,
            to,
            started: Instant::now(),
            shown: None,
        }
    }

    /// Bring the dialog up for a job still running [`SHOW_AFTER`] after it started. A job held on
    /// a failure box waits for the answer, so the dialog never maps over the box.
    fn show(&mut self) -> bool {
        if self.shown.is_some() || self.started.elapsed() < SHOW_AFTER || self.meter.asking() {
            return false;
        }
        let cp = CopyProgress::new(self.kind, &self.from, &self.to, accent());
        self.shown = Some(Shown { cp, focused: false });
        true
    }

    /// Take the job's counts in.
    fn tick(&mut self, waiting: usize) -> bool {
        let ms = self.started.elapsed().as_millis() as u64;
        let st = status(self.meter.snapshot(), waiting);
        self.shown.as_mut().is_some_and(|s| s.cp.update(st, ms))
    }

    /// Whether its window ticks: not while its job waits on a box, which holds the dialog still.
    fn ticks(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.cp.wants_ticks())
    }

    /// The job's share of the task button, once its dialog is up: its fraction done (0 while it
    /// counts what it has to do) and whether it is paused.
    fn task_state(&self) -> Option<(f64, bool)> {
        let cp = &self.shown.as_ref()?.cp;
        let fraction = match cp.status.phase {
            Phase::Preparing => 0.0,
            _ => cp.fraction(),
        };
        Some((fraction, cp.status.paused))
    }

    /// The job is over, having dealt with `done` items: a job whose dialog was up but not focused
    /// leaves a balloon with what was done or why it stopped. A cancelled job says nothing.
    pub fn ended(&self, done: usize, failed: Option<&(&'static str, String)>) {
        let Some(shown) = self.shown.as_ref().filter(|s| !s.focused) else {
            return;
        };
        if self.meter.cancelled() {
            return;
        }
        let (summary, body) = match failed {
            Some((title, text)) => (title.to_string(), text.clone()),
            None => shown.cp.ended_note(done),
        };
        std::thread::spawn(move || {
            if let Err(e) = reprisebus::notify::notify("Explorer", "folder", &summary, &body) {
                log::warn!("explorer: the job's balloon: {e}");
            }
        });
    }
}

impl Explorer {
    fn progress(&self, win: WinId) -> Option<&Progress> {
        match win {
            COPY => self.put.as_ref().map(|p| &p.progress),
            REMOVE => self.removing.as_ref().map(|r| &r.progress),
            EXTRACT => self.extracting.as_ref().map(|e| &e.progress),
            _ => None,
        }
    }

    fn progress_mut(&mut self, win: WinId) -> Option<&mut Progress> {
        match win {
            COPY => self.put.as_mut().map(|p| &mut p.progress),
            REMOVE => self.removing.as_mut().map(|r| &mut r.progress),
            EXTRACT => self.extracting.as_mut().map(|e| &mut e.progress),
            _ => None,
        }
    }

    fn waiting(&self, win: WinId) -> usize {
        match win {
            COPY => self.put_queue.len(),
            EXTRACT => self.extract_queue.len(),
            _ => self.remove_queue.len(),
        }
    }

    /// What Log Off waits for while a job runs: the first running job's work.
    pub(crate) fn logout_reason(&self) -> Option<&'static str> {
        let running = [COPY, REMOVE, EXTRACT]
            .into_iter()
            .find_map(|w| self.progress(w));
        running.map(|p| holds_off(p.kind))
    }

    /// Hold off log off while [`Explorer::logout_reason`] says why, in a folder window and a
    /// process of one job alike, and let go once the last job ends.
    pub(crate) fn hold_logout(&mut self) {
        let reason = self.logout_reason();
        if reason != self.logout_held {
            self.logout_held = reason;
            reprisebus::inhibit::hold_logout(reason);
        }
    }

    /// Bring up the dialog of each job running past [`SHOW_AFTER`].
    pub(crate) fn show_progress_dialogs(&mut self) -> bool {
        let mut shown = false;
        for win in [COPY, REMOVE, EXTRACT] {
            shown |= self.progress_mut(win).is_some_and(Progress::show);
        }
        shown
    }

    pub(crate) fn progress_children(&self) -> Vec<Child> {
        [COPY, REMOVE, EXTRACT]
            .into_iter()
            .filter_map(|win| {
                let shown = self.progress(win)?.shown.as_ref()?;
                Some(Child {
                    id: win,
                    title: shown.cp.title().into(),
                    size: self.forms.text.size(shown.cp.client()),
                    fixed: true,
                    modal: false,
                })
            })
            .collect()
    }

    /// The progress windows whose ticks stopped while their job waited on a box and whose job has
    /// moved since (an answer, a cancel, another job queued), each due one tick to read it.
    pub(crate) fn held_progress_moved(&self) -> Vec<WinId> {
        [COPY, REMOVE, EXTRACT]
            .into_iter()
            .filter(|&win| {
                let Some(p) = self.progress(win) else {
                    return false;
                };
                p.shown.as_ref().is_some_and(|s| {
                    !s.cp.wants_ticks()
                        && status(p.meter.snapshot(), self.waiting(win)) != s.cp.status
                })
            })
            .collect()
    }

    fn progress_tick(&mut self, win: WinId) -> bool {
        let waiting = self.waiting(win);
        let redraw = self.progress_mut(win).is_some_and(|p| p.tick(waiting));
        self.publish_task_progress();
        redraw
    }

    /// Put the jobs' [`combined`] progress on the task button when its whole percent or paused
    /// state changes. Both jobs share one app id, so one value covers them.
    pub(crate) fn publish_task_progress(&mut self) {
        let jobs: Vec<_> = [COPY, REMOVE, EXTRACT]
            .into_iter()
            .filter_map(|win| self.progress(win)?.task_state())
            .collect();
        let now = combined(&jobs);
        let key = now.map(|(f, paused)| ((f * 100.0) as u32, paused));
        if key != self.task_progress {
            self.task_progress = key;
            reprisebus::launcher::update(DESKTOP_ID, now);
        }
    }

    pub(crate) fn progress_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let text = self.forms.text;
        let shown = match win {
            COPY => self.put.as_mut().and_then(|p| p.progress.shown.as_mut()),
            EXTRACT => (self.extracting.as_mut()).and_then(|e| e.progress.shown.as_mut()),
            _ => self
                .removing
                .as_mut()
                .and_then(|r| r.progress.shown.as_mut()),
        };
        let Some(shown) = shown else {
            return Reply::none();
        };
        shown.cp.follow(&self.forms.paint());
        let outcome = match &ev {
            Event::Tick | Event::Resized => {
                let redraw = self.progress_tick(win);
                return Reply {
                    redraw: redraw || ev == Event::Resized,
                    tick: self.progress(win).is_some_and(Progress::ticks),
                    ..Reply::none()
                };
            }
            Event::Focus(on) => {
                shown.focused = *on;
                return Reply::redraw();
            }
            Event::CloseRequest => Outcome::Cancel,
            Event::Key(k)
                if k.mods.alt
                    && k.utf8
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case("d")) =>
            {
                shown.cp.toggle_details()
            }
            _ => {
                let toggled = match &ev {
                    Event::Pointer(PointerInput::Press { x, y, .. }) => {
                        shown.cp.press(*x, *y, text)
                    }
                    _ => None,
                };
                match toggled {
                    Some(o) => o,
                    None => match repriseapp::keys::form_input(&mut shown.cp.form, &ev, ctx) {
                        Some(e) => shown.cp.event(e),
                        None => Outcome::None,
                    },
                }
            }
        };
        match outcome {
            Outcome::Pause(on) => {
                if let Some(p) = self.progress(win) {
                    p.meter.set_paused(on);
                }
                self.progress_tick(win);
                Reply::redraw()
            }
            Outcome::Cancel => {
                match win {
                    COPY => self.cancel_puts(),
                    EXTRACT => self.cancel_extracts(),
                    _ => self.cancel_removals(),
                }
                Reply::redraw()
            }
            Outcome::Redraw | Outcome::Resized => Reply::redraw(),
            Outcome::None => Reply::none(),
        }
    }

    /// Cancel: the running put stops inside its file and the pastes queued behind it never start.
    pub(crate) fn cancel_puts(&mut self) {
        self.put_queue.clear();
        if let Some(put) = &self.put {
            stop(&put.progress.meter);
        }
    }

    /// Cancel: the running removal stops at its next file and the ones queued behind it never
    /// start.
    pub(crate) fn cancel_removals(&mut self) {
        self.remove_queue.clear();
        if let Some(r) = &self.removing {
            stop(&r.progress.meter);
        }
    }

    pub(crate) fn draw_progress(&mut self, win: WinId, c: &mut reprisekit::raster::Canvas) {
        let p = Paint {
            s: 1,
            ..self.forms.paint()
        };
        let px = self.forms.fonts.size_px;
        let text = &self.dialog_text;
        let shown = match win {
            COPY => self.put.as_mut().and_then(|p| p.progress.shown.as_mut()),
            EXTRACT => (self.extracting.as_mut()).and_then(|e| e.progress.shown.as_mut()),
            _ => self
                .removing
                .as_mut()
                .and_then(|r| r.progress.shown.as_mut()),
        };
        if let Some(shown) = shown {
            shown.cp.follow(&p);
            shown.cp.draw(c, text, &p, px);
        }
    }

    pub(crate) fn access_progress(&self, win: WinId, tree: &mut reprisekit::access::Tree) {
        let p = self.forms.paint();
        if let Some(shown) = self.progress(win).and_then(|p| p.shown.as_ref()) {
            shown
                .cp
                .access(tree, &self.dialog_text, &p, self.forms.fonts.size_px);
        }
    }
}

/// The reason a job of `kind` gives for holding off log off.
fn holds_off(kind: Kind) -> &'static str {
    match kind {
        Kind::Copy => "Copying files",
        Kind::Move => "Moving files",
        Kind::Delete | Kind::Erase => "Deleting files",
        Kind::Empty => "Emptying the Recycle Bin",
        Kind::Restore => "Restoring files",
        Kind::Extract => "Extracting files",
        Kind::Compress => "Compressing files",
    }
}

/// One task button state for running jobs given as `(fraction, paused)`: their fractions averaged
/// with equal weight (a put counts bytes, a removal items), paused only when every job is, and
/// `None` once none runs.
pub(crate) fn combined(jobs: &[(f64, bool)]) -> Option<(f64, bool)> {
    if jobs.is_empty() {
        return None;
    }
    let fraction = jobs.iter().map(|j| j.0).sum::<f64>() / jobs.len() as f64;
    Some((fraction, jobs.iter().all(|j| j.1)))
}

pub(crate) fn stop(meter: &Meter) {
    meter.set_paused(false);
    meter.cancel();
}

#[cfg(test)]
mod tests {
    use super::{COPY, Color, Shown, combined};
    use crate::commands::Command;
    use crate::place::Place;
    use repriseapp::{Event, FileClip, KeyInput, Mods};
    use reprisekit::controls::copyprogress::CopyProgress;
    use reprisekit::textedit::keysym;

    #[test]
    fn a_progress_window_stops_ticking_while_its_job_waits_on_its_failure_box() {
        let home = tempfile::tempdir().expect("a temp dir");
        let (from, dir) = (home.path().join("from"), home.path().join("to"));
        std::fs::create_dir_all(&from).expect("mkdir");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(from.join("b.txt"), "b").expect("write");
        let mut ctx = crate::tests::test_ctx();
        let mut ex = crate::tests::test_explorer(Place::Folder(dir.clone()), &mut ctx);
        ex.run(Command::Paste, &mut ctx);
        ex.pasted(
            FileClip {
                cut: false,
                paths: vec![from.join("gone.txt"), from.join("b.txt")],
            },
            &mut ctx,
        );
        ex.finish_puts(&mut ctx);
        assert!(ex.holds_a_job(), "the paste holds on the failure box");
        let p = &mut ex.put.as_mut().expect("the paste").progress;
        let cp = CopyProgress::new(p.kind, &p.from, &p.to, Color::rgb(0, 0, 128));
        p.shown = Some(Shown { cp, focused: false });

        let tick = ex.event(COPY, Event::Tick, &mut ctx);
        assert!(!tick.tick, "no tick while the box waits");

        let enter = Event::Key(KeyInput {
            sym: keysym::RETURN,
            utf8: None,
            mods: Mods::default(),
            edit: None,
        });
        let skipped = ex.event(ex.dialog_win(), enter, &mut ctx);
        assert_eq!(skipped.tick_wins, [COPY], "Skip sets the window ticking");
        let tick = ex.event(COPY, Event::Tick, &mut ctx);
        assert!(tick.tick, "ticking again once the job goes on");
    }

    #[test]
    fn two_running_jobs_average_their_fractions() {
        assert_eq!(combined(&[(0.2, false), (0.8, false)]), Some((0.5, false)));
    }

    #[test]
    fn one_paused_job_leaves_the_bar_running() {
        assert_eq!(combined(&[(0.2, true), (0.8, false)]), Some((0.5, false)));
    }

    #[test]
    fn the_bar_is_paused_only_when_every_job_is() {
        assert_eq!(combined(&[(0.2, true), (0.8, true)]), Some((0.5, true)));
    }

    #[test]
    fn a_preparing_job_counts_as_zero_and_keeps_the_bar() {
        assert_eq!(combined(&[(0.0, false)]), Some((0.0, false)));
        assert_eq!(combined(&[(0.0, false), (0.6, false)]), Some((0.3, false)));
    }

    #[test]
    fn one_job_finishing_leaves_the_other_alone_on_the_bar() {
        assert_eq!(combined(&[(0.4, true)]), Some((0.4, true)));
        assert_eq!(combined(&[]), None);
    }
}
