//! The copy progress dialog's host half: when it comes up, its window and events, Pause and
//! Cancel on the job, and the balloon for a put that ends while the dialog is not looked at.

use crate::Explorer;
use repriseapp::{Child, Ctx, Event, PointerInput, Reply, WinId};
use reprisefs::progress::{Phase as JobPhase, Snapshot};
use reprisekit::controls::copyprogress::{CopyProgress, Kind, Outcome, Phase, Status};
use reprisekit::look::Paint;
use reprisekit::theme::Color;
use std::time::Duration;

pub const COPY: WinId = "copy progress";
/// A put done sooner than this never shows the dialog.
pub(crate) const SHOW_AFTER: Duration = Duration::from_millis(500);

/// The dialog of the running put, once it is up.
pub(crate) struct Shown {
    pub cp: CopyProgress,
    pub focused: bool,
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

impl Explorer {
    /// Bring the dialog up for a put still running [`SHOW_AFTER`] after it started.
    pub(crate) fn show_copy_dialog(&mut self) -> bool {
        let Some(put) = self.put.as_mut() else {
            return false;
        };
        if put.shown.is_some() || put.started.elapsed() < SHOW_AFTER {
            return false;
        }
        let kind = if put
            .ops
            .iter()
            .all(|(_, op, _)| *op == reprisefs::ops::DropOp::Move)
        {
            Kind::Move
        } else {
            Kind::Copy
        };
        let from = put
            .ops
            .first()
            .and_then(|(src, _, _)| src.parent())
            .map(crate::place::Place::from_path)
            .map(|p| p.title())
            .unwrap_or_default();
        let to = crate::place::Place::from_path(&put.dir).title();
        put.shown = Some(Shown {
            cp: CopyProgress::new(kind, &from, &to, accent()),
            focused: false,
        });
        true
    }

    pub(crate) fn copy_child(&self) -> Option<Child> {
        let shown = self.put.as_ref()?.shown.as_ref()?;
        Some(Child {
            id: COPY,
            title: shown.cp.title().into(),
            size: self.forms.text.size(shown.cp.client()),
            fixed: true,
            modal: false,
        })
    }

    fn copy_tick(&mut self) -> bool {
        let waiting = self.put_queue.len();
        let Some(put) = self.put.as_mut() else {
            return false;
        };
        let ms = put.started.elapsed().as_millis() as u64;
        let st = status(put.job.progress(), waiting);
        put.shown.as_mut().is_some_and(|s| s.cp.update(st, ms))
    }

    pub(crate) fn copy_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(put) = self.put.as_mut() else {
            return Reply::none();
        };
        let Some(shown) = put.shown.as_mut() else {
            return Reply::none();
        };
        shown.cp.follow(&self.forms.paint());
        let outcome = match &ev {
            Event::Tick | Event::Resized => {
                let redraw = self.copy_tick();
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
                        shown.cp.press(*x, *y, self.forms.text)
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
                put.job.set_paused(on);
                self.copy_tick();
                Reply::redraw()
            }
            Outcome::Cancel => {
                put.job.set_paused(false);
                put.job.cancel();
                Reply::redraw()
            }
            Outcome::Redraw | Outcome::Resized => Reply::redraw(),
            Outcome::None => Reply::none(),
        }
    }

    pub(crate) fn draw_copy(&mut self, c: &mut reprisekit::raster::Canvas) {
        let p = Paint {
            s: 1,
            ..self.forms.paint()
        };
        let px = self.forms.fonts.size_px;
        if let Some(shown) = self.put.as_mut().and_then(|p| p.shown.as_mut()) {
            shown.cp.follow(&p);
            shown.cp.draw(c, &self.dialog_text, &p, px);
        }
    }

    pub(crate) fn access_copy(&self, tree: &mut reprisekit::access::Tree) {
        let p = self.forms.paint();
        if let Some(shown) = self.put.as_ref().and_then(|p| p.shown.as_ref()) {
            shown
                .cp
                .access(tree, &self.dialog_text, &p, self.forms.fonts.size_px);
        }
    }

    /// A balloon for a put whose dialog was up but not focused when it ended: what landed where,
    /// or why it stopped. A cancelled put says nothing.
    pub(crate) fn copy_ended(
        &self,
        shown: &Shown,
        landed: usize,
        failed: Option<&(&'static str, String)>,
    ) {
        if shown.focused {
            return;
        }
        let (summary, body) = match failed {
            Some((title, text)) => (title.to_string(), text.clone()),
            None => {
                let verb = match shown.cp.kind {
                    Kind::Copy => "Copied",
                    Kind::Move => "Moved",
                };
                let noun = if landed == 1 { "item" } else { "items" };
                (
                    format!("{verb} {landed} {noun}"),
                    format!("{verb} {landed} {noun} to {}", shown.cp.to),
                )
            }
        };
        std::thread::spawn(move || {
            if let Err(e) = reprisebus::notify::notify("Explorer", "folder", &summary, &body) {
                log::warn!("explorer: the put's balloon: {e}");
            }
        });
    }
}
