//! The progress dialogs' host half, for a put (copy, move) and a removal (delete, Empty, Restore):
//! when each comes up, its window and events, Pause and Cancel on its job, the task button's
//! progress, and the balloon for a job that ends while its dialog is not looked at.

use crate::Explorer;
use repriseapp::{Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisefs::progress::{Meter, Phase as JobPhase, Snapshot};
use reprisekit::controls::copyprogress::{CopyProgress, Kind, Outcome, Phase, SHOW_AFTER, Status};
use reprisekit::look::Paint;
use reprisekit::theme::Color;
use std::sync::Arc;
use std::time::Instant;

pub const COPY: WinId = "copy progress";
pub const REMOVE: WinId = "remove progress";
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
    /// The percent and paused state last put on the task button.
    pub published: Option<(u32, bool)>,
}

pub(crate) fn status(s: Snapshot, waiting: usize) -> Status {
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

/// The glass colour `[glass] color` names, for the graph; the first swatch when unset or unknown.
fn accent() -> Color {
    repriseconf::glass_color()
        .and_then(|name| reprisekit::glass::color_named(&name))
        .unwrap_or(reprisekit::glass::SWATCHES[0].color)
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
            published: None,
        }
    }

    /// Bring the dialog up for a job still running [`SHOW_AFTER`] after it started.
    fn show(&mut self) -> bool {
        if self.shown.is_some() || self.started.elapsed() < SHOW_AFTER {
            return false;
        }
        let cp = CopyProgress::new(self.kind, &self.from, &self.to, accent());
        self.shown = Some(Shown { cp, focused: false });
        true
    }

    /// Take the job's counts in; the task button follows once the dialog is up.
    fn tick(&mut self, waiting: usize) -> bool {
        let ms = self.started.elapsed().as_millis() as u64;
        let st = status(self.meter.snapshot(), waiting);
        let Some(shown) = self.shown.as_mut() else {
            return false;
        };
        let redraw = shown.cp.update(st, ms);
        let fraction = shown.cp.fraction();
        let now = ((fraction * 100.0) as u32, shown.cp.status.paused);
        if self.published != Some(now) {
            self.published = Some(now);
            reprisebus::launcher::update(DESKTOP_ID, Some((fraction, now.1)));
        }
        redraw
    }

    /// The job is over, having dealt with `done` items: the task button lets go, and a job whose
    /// dialog was up but not focused leaves a balloon with what was done or why it stopped. A
    /// cancelled job says nothing.
    pub fn ended(&self, done: usize, failed: Option<&(&'static str, String)>) {
        if self.published.is_some() {
            reprisebus::launcher::update(DESKTOP_ID, None);
        }
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
            _ => None,
        }
    }

    fn progress_mut(&mut self, win: WinId) -> Option<&mut Progress> {
        match win {
            COPY => self.put.as_mut().map(|p| &mut p.progress),
            REMOVE => self.removing.as_mut().map(|r| &mut r.progress),
            _ => None,
        }
    }

    fn waiting(&self, win: WinId) -> usize {
        match win {
            COPY => self.put_queue.len(),
            _ => self.remove_queue.len(),
        }
    }

    /// Bring up the dialog of each job running past [`SHOW_AFTER`].
    pub(crate) fn show_progress_dialogs(&mut self) -> bool {
        let mut shown = false;
        for win in [COPY, REMOVE] {
            shown |= self.progress_mut(win).is_some_and(Progress::show);
        }
        shown
    }

    pub(crate) fn progress_children(&self) -> Vec<Child> {
        [COPY, REMOVE]
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

    fn progress_tick(&mut self, win: WinId) -> bool {
        let waiting = self.waiting(win);
        self.progress_mut(win).is_some_and(|p| p.tick(waiting))
    }

    pub(crate) fn progress_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        let text = self.forms.text;
        let shown = match win {
            COPY => self.put.as_mut().and_then(|p| p.progress.shown.as_mut()),
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
                    tick: true,
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

fn stop(meter: &Meter) {
    meter.set_paused(false);
    meter.cancel();
}
