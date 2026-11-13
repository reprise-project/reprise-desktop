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
use crate::registry::{self, Page};
use crate::{SettingsApp, outputwrite, take};

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
    /// Every head the compositor lists, disabled ones too: the set a saved layout is kept for.
    pub connected: Vec<Head>,
    /// `repriseconf::displays::path()`; a scratch file in tests.
    pub layouts: std::path::PathBuf,
    pub flow: Flow,
    /// The button pressed; the flow starts once the event is back in
    /// `App::event`, which has the `Ctx`.
    pub pending: Option<Press>,
    pub icon: Option<Canvas>,
    /// The accelerometer read off the loop, once the page first opens.
    pub sensor_job: Option<std::sync::mpsc::Receiver<bool>>,
    /// The display profile listing, off the loop since colord may be asked.
    pub profile_scan: Option<std::sync::mpsc::Receiver<Vec<reprisefs::icc::Listed>>>,
}

impl SettingsApp {
    pub(crate) fn heads_changed(&mut self, heads: Vec<Head>) -> Reply {
        self.display.connected = heads.clone();
        self.display.model.set_heads(heads);
        self.display.model.layouts_saved = self.display.layouts.exists();
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
        let effects = self.display.flow.heads_changed(&self.display.model);
        let mut r = self.display_effects(effects);
        self.sync_apply();
        r.redraw_all = true;
        r
    }

    /// List the display profiles on a thread; `profiles_done` takes them in.
    pub(crate) fn scan_profiles(&mut self) {
        self.display.profile_scan = Some(self.spawn(|| {
            reprisefs::icc::list(
                &reprisefs::icc::dirs(),
                &reprisebus::colord::profile_files(),
            )
        }));
    }

    pub(crate) fn profiles_done(&mut self) -> bool {
        let Some(rx) = &self.display.profile_scan else {
            return false;
        };
        let found = match crate::take(rx) {
            Ok(None) => return false,
            Ok(Some(found)) => found,
            Err(()) => Vec::new(),
        };
        self.display.profile_scan = None;
        self.display.model.set_profiles(found);
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
        true
    }

    pub(crate) fn display_changed(&mut self) -> Reply {
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
        self.sync_apply();
        Reply::redraw()
    }

    pub(crate) fn ensure_sensor(&mut self) {
        let d = &self.display;
        if registry::open(&self.sheet) == Page::Display
            && d.model.sensor.is_none()
            && d.sensor_job.is_none()
        {
            self.start_sensor();
        }
    }

    /// The accelerometer read finished: grey or offer Rotate automatically. True when the page
    /// changed.
    pub(crate) fn sensor_done(&mut self) -> bool {
        let Some(rx) = &self.display.sensor_job else {
            return false;
        };
        let found = match take(rx) {
            Ok(None) => return false,
            Ok(Some(f)) => f,
            Err(()) => false,
        };
        self.display.sensor_job = None;
        self.display.model.sensor = Some(found);
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
        true
    }

    #[cfg(not(test))]
    fn start_sensor(&mut self) {
        self.display.sensor_job = Some(self.spawn(|| {
            reprisebus::system()
                .is_ok_and(|c| reprisebus::wait(reprisebus::sensorproxy::has_accelerometer(&c)))
        }));
    }

    /// Tests feed `sensor_job` themselves, so nothing reaches the system bus.
    #[cfg(test)]
    fn start_sensor(&mut self) {}

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

    /// Forget saved layouts: the compositor places the next set of monitors from `[[outputs]]`.
    pub(crate) fn forget_layouts(&mut self) -> Reply {
        match std::fs::remove_file(&self.display.layouts) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                log::warn!("{}: {e}", self.display.layouts.display());
                self.fail("The saved layouts could not be forgotten.");
                return Reply::redraw();
            }
        }
        self.display.model.layouts_saved = false;
        self.display_changed()
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
        displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
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
        // The compositor saves the layout once it has reloaded what was written.
        let mut saved = false;
        for e in effects {
            match e {
                Effect::Save(name, keys) => {
                    if let Err(why) = outputwrite::save(&self.path, &name, &keys) {
                        self.fail(why);
                        failed = true;
                    } else {
                        saved = true;
                    }
                }
                Effect::SavePrimary(name) => {
                    if let Err(why) = outputwrite::save_primary(&self.path, &name) {
                        self.fail(why);
                        failed = true;
                    } else {
                        saved = true;
                    }
                }
                Effect::Fail(why) => {
                    self.fail(why);
                    failed = true;
                }
                Effect::Quit => r.quit = !failed,
            }
        }
        if saved && !self.display.model.layouts_saved {
            self.display.model.layouts_saved = true;
            displaypage::refresh(&mut self.sheet, &self.display.model, &self.want);
            r.redraw = true;
        }
        r
    }
}
