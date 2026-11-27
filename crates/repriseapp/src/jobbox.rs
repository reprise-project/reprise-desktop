//! A program's progress box for an [`ArchiveJob`] it waits on, such as a save into a zip: modal
//! over its window once the job has run [`SHOW_AFTER`], with Pause and Cancel, and its close box
//! cancels. The program takes no input in any of its other windows while [`JobBox::running`].

use crate::{Child, Ctx, Event, PointerInput, Reply, Waker, WinId};
use reprisefs::extract::{ArchiveEvent, ArchiveJob};
use reprisefs::progress::{Phase as JobPhase, Snapshot};
use reprisekit::access::Tree;
use reprisekit::controls::copyprogress::{CopyProgress, Kind, Outcome, Phase, SHOW_AFTER, Status};
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use reprisekit::theme::Theme;
use std::time::Instant;

pub const JOB: WinId = WinId::named("job progress");

/// How a job a [`JobBox`] ran ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    Done,
    /// Its error box's text.
    Failed(String),
    Cancelled,
}

/// What [`JobBox::wake`] saw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Woke {
    Nothing,
    /// The box came up.
    Shown,
    Ended(Ended),
}

#[derive(Default)]
pub struct JobBox(Option<Running>);

/// How long a program quitting mid-job waits for the job to stop and remove its temp files.
const QUIT_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// Dropped with its program, after the windows are gone: a running job is cancelled and waited
/// for up to [`QUIT_WAIT`].
impl Drop for JobBox {
    fn drop(&mut self) {
        if let Some(r) = &mut self.0 {
            r.job.stop_and_wait(QUIT_WAIT);
        }
    }
}

struct Running {
    job: ArchiveJob,
    kind: Kind,
    from: String,
    to: String,
    started: Instant,
    shown: Option<CopyProgress>,
    failed: Option<String>,
    landed: bool,
}

/// Whether `ev` is input a program's windows ignore while it waits on its [`JobBox`]: keys, the
/// pointer, pastes, drops and close boxes.
pub fn is_input(ev: &Event) -> bool {
    matches!(
        ev,
        Event::Key(_)
            | Event::KeyUp(_)
            | Event::Pointer(_)
            | Event::Pasted(_)
            | Event::PastedPrimary(_)
            | Event::PastedFiles(_)
            | Event::PastedImage(_)
            | Event::FieldDropped(_)
            | Event::DragEnter { .. }
            | Event::DragMotion { .. }
            | Event::DragFiles { .. }
            | Event::DragDrop { .. }
            | Event::CloseRequest
    )
}

/// A job's counts as its progress dialog shows them, with `waiting` jobs queued behind it.
pub fn status(s: Snapshot, waiting: usize) -> Status {
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

impl JobBox {
    /// Waits on `job`, a `kind` job from `from` to `to`; `wake` brings its box up on time.
    /// Refused, the job let go, while another runs.
    pub fn start(
        &mut self,
        job: ArchiveJob,
        kind: Kind,
        (from, to): (String, String),
        wake: Option<&Waker>,
    ) -> bool {
        if self.running() {
            return false;
        }
        if let Some(wake) = wake.cloned() {
            let later = wake.clone();
            let spawned = std::thread::Builder::new()
                .name("jobbox-show".into())
                .spawn(move || {
                    std::thread::sleep(SHOW_AFTER);
                    later.wake();
                });
            if spawned.is_err() {
                wake.wake();
            }
        }
        self.0 = Some(Running {
            job,
            kind,
            from,
            to,
            started: Instant::now(),
            shown: None,
            failed: None,
            landed: false,
        });
        true
    }

    pub fn running(&self) -> bool {
        self.0.is_some()
    }

    pub fn owns(&self, win: WinId) -> bool {
        win == JOB
    }

    /// The box's window once it is up, sized in `forms`.
    pub fn child(&self, forms: &Theme) -> Option<Child> {
        let cp = self.0.as_ref()?.shown.as_ref()?;
        Some(Child {
            id: JOB,
            title: cp.title(),
            size: forms.text.size(cp.client()),
            fixed: true,
            modal: true,
        })
    }

    /// On `Event::Wake`: takes the job's events, and brings the box up once the job has run
    /// [`SHOW_AFTER`].
    pub fn wake(&mut self) -> Woke {
        let Some(r) = &mut self.0 else {
            return Woke::Nothing;
        };
        let mut finished = false;
        for ev in r.job.poll() {
            match ev {
                ArchiveEvent::Landed { .. } => r.landed = true,
                ArchiveEvent::Failed { text, .. } => r.failed = Some(text),
                ArchiveEvent::Finished => finished = true,
            }
        }
        if !finished {
            if r.shown.is_some() || r.started.elapsed() < SHOW_AFTER {
                return Woke::Nothing;
            }
            let accent = repriseconf::current().glass.color;
            r.shown = Some(CopyProgress::new(r.kind, &r.from, &r.to, accent));
            return Woke::Shown;
        }
        let Some(r) = self.0.take() else {
            return Woke::Nothing;
        };
        Woke::Ended(match (r.failed, r.landed) {
            (Some(text), _) => Ended::Failed(text),
            (None, true) => Ended::Done,
            (None, false) => Ended::Cancelled,
        })
    }

    /// An event on the box's window: Pause, Cancel, the details, and its ticks.
    pub fn event(&mut self, ev: &Event, ctx: &mut Ctx) -> Reply {
        let Some(r) = &mut self.0 else {
            return Reply::none();
        };
        let Some(cp) = &mut r.shown else {
            return Reply::none();
        };
        let forms = ctx.forms();
        cp.follow(&forms.paint());
        let meter = r.job.meter();
        let outcome = match ev {
            Event::Tick | Event::Resized => {
                let ms = r.started.elapsed().as_millis() as u64;
                let redraw = cp.update(status(meter.snapshot(), 0), ms);
                return Reply {
                    redraw: redraw || *ev == Event::Resized,
                    tick: cp.wants_ticks() || !meter.parked(),
                    ..Reply::none()
                };
            }
            Event::CloseRequest => Outcome::Cancel,
            Event::Key(k)
                if k.mods.alt
                    && (k.utf8.as_deref()).is_some_and(|s| s.eq_ignore_ascii_case("d")) =>
            {
                cp.toggle_details()
            }
            _ => {
                let pressed = match ev {
                    Event::Pointer(PointerInput::Press { x, y, .. }) => {
                        cp.press(*x, *y, forms.text)
                    }
                    _ => None,
                };
                match pressed {
                    Some(o) => o,
                    None => match crate::keys::form_input(&mut cp.form, ev, ctx) {
                        Some(e) => cp.event(e),
                        None => Outcome::None,
                    },
                }
            }
        };
        match outcome {
            Outcome::Pause(on) => {
                meter.set_paused(on);
                Reply {
                    tick: true,
                    ..Reply::redraw()
                }
            }
            Outcome::Cancel => {
                meter.cancel();
                Reply::redraw()
            }
            Outcome::Redraw | Outcome::Resized => Reply::redraw(),
            Outcome::None => Reply::none(),
        }
    }

    pub fn draw(&mut self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        let p = Paint {
            s: 1,
            ..forms.paint()
        };
        if let Some(cp) = self.0.as_mut().and_then(|r| r.shown.as_mut()) {
            cp.follow(&p);
            cp.draw(c, &ctx.text, &p, forms.fonts.size_px);
        }
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let forms = ctx.forms();
        if let Some(cp) = self.0.as_ref().and_then(|r| r.shown.as_ref()) {
            cp.access(tree, &ctx.text, &forms.paint(), forms.fonts.size_px);
        }
    }

    /// The running job's meter, for a test to cancel or wait on.
    pub fn meter(&self) -> Option<&std::sync::Arc<reprisefs::progress::Meter>> {
        self.0.as_ref().map(|r| r.job.meter())
    }
}
