//! The Display page's host side in `SettingsApp`: heads, the OK/Apply/Cancel requests and the Monitor Settings
//! countdown. Glue only; the decisions are `displaypage`'s and `displayflow`'s.

use std::time::Instant;

use repriseapp::heads::{Head, HeadRequests};
use repriseapp::keys::form_input;
use repriseapp::{Ctx, Event, Reply, WinId};
use reprisekit::raster::Canvas;

use crate::displayflow::{Effect, Flow};
use crate::displaypage::{self, Display};
use crate::monitordlg::{self, Outcome};
use crate::{SettingsApp, outputwrite};

pub const MONITOR_WIN: WinId = WinId::named("monitor");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    Ok,
    Apply,
    Cancel,
}

#[derive(Default)]
pub struct DisplayHost {
    pub model: Display,
    pub flow: Flow,
    /// The button pressed; the flow starts once the event is back in
    /// `App::event`, which has the `Ctx`.
    pub pending: Option<Press>,
    pub icon: Option<Canvas>,
}

impl SettingsApp {
    pub(crate) fn heads_changed(&mut self, heads: Vec<Head>) -> Reply {
        self.display.model.set_heads(heads);
        displaypage::refresh(&mut self.sheet, &self.display.model);
        let effects = self.display.flow.heads_changed(&self.display.model);
        let mut r = self.display_effects(effects);
        self.sync_apply();
        r.redraw_all = true;
        r
    }

    pub(crate) fn display_changed(&mut self) -> Reply {
        displaypage::refresh(&mut self.sheet, &self.display.model);
        self.sync_apply();
        Reply::redraw()
    }

    /// Identify: the compositor numbers the screens as the well does.
    /// Off the loop, since the call waits on the compositor.
    pub(crate) fn identify(&self) -> Reply {
        let names = self.display.model.names();
        std::thread::spawn(move || {
            if let Err(e) = reprisebus::tray::identify(&names) {
                log::warn!("Identify: {e}");
            }
        });
        Reply::none()
    }

    pub(crate) fn start_display(&mut self, press: Press, req: &mut HeadRequests) -> Reply {
        let flow = &mut self.display.flow;
        let effects = match press {
            Press::Cancel => flow.cancel(req),
            Press::Ok | Press::Apply => {
                flow.apply(&mut self.display.model, req, press == Press::Ok)
            }
        };
        self.display_effects(effects)
    }

    pub(crate) fn heads_applied(&mut self, serial: u32, ok: bool, req: &mut HeadRequests) -> Reply {
        let effects = self
            .display
            .flow
            .applied(serial, ok, &mut self.display.model, req);
        if let (Some(q), Some(icon)) = (self.display.flow.question.as_mut(), &self.display.icon) {
            q.dialog.form.set_picture(monitordlg::ICON, icon.clone());
        }
        displaypage::refresh(&mut self.sheet, &self.display.model);
        self.sync_apply();
        let mut r = self.display_effects(effects);
        r.redraw_all = true;
        r.tick = self.display.flow.question.is_some();
        r
    }

    pub(crate) fn display_tick(&mut self, req: &mut HeadRequests) -> Reply {
        let before = self.display.flow.question.as_ref().map(|q| q.dialog.left);
        let effects = self.display.flow.tick(Instant::now(), req);
        let after = self.display.flow.question.as_ref().map(|q| q.dialog.left);
        let mut r = self.display_effects(effects);
        r.redraw_all = before != after;
        r.tick = after.is_some();
        r
    }

    pub(crate) fn display_close(&mut self, req: &mut HeadRequests) -> Reply {
        let effects = self.display.flow.close(req);
        let mut r = self.display_effects(effects);
        r.redraw_all = true;
        r
    }

    /// An event on the question's window. Its close box, like Escape,
    /// is No: the old settings go back and the sheet stays open.
    pub(crate) fn monitor_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(q) = self.display.flow.question.as_mut() else {
            return Reply::none();
        };
        let out = match ev {
            Event::CloseRequest => Outcome::Revert,
            ev => match form_input(&mut q.dialog.form, &ev, ctx) {
                Some(ev) => q.dialog.event(ev),
                None => return Reply::none(),
            },
        };
        match out {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Keep | Outcome::Revert => {
                let effects = self
                    .display
                    .flow
                    .answer(out == Outcome::Keep, &mut ctx.heads);
                let mut r = self.display_effects(effects);
                r.redraw_all = true;
                r
            }
        }
    }

    /// Carry out what the flow decided. A `Quit` after an error box went
    /// up (a `Fail`, or a `Save` that could not be written) is dropped,
    /// so the box stays on screen over the open sheet.
    pub(crate) fn display_effects(&mut self, effects: Vec<Effect>) -> Reply {
        let mut r = Reply::none();
        let mut failed = false;
        for e in effects {
            match e {
                Effect::Save(name, keys) => {
                    if let Err(why) = outputwrite::save(&self.path, &name, &keys) {
                        self.fail(why);
                        failed = true;
                    }
                }
                Effect::SavePrimary(name) => {
                    if let Err(why) = outputwrite::save_primary(&self.path, &name) {
                        self.fail(why);
                        failed = true;
                    }
                }
                Effect::Fail(why) => {
                    self.fail(why);
                    failed = true;
                }
                Effect::Quit => r.quit = !failed,
            }
        }
        r
    }
}
