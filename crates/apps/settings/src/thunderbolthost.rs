//! The Thunderbolt page's devices: read from boltd off the loop while the page shows, again every
//! [`POLL`], and Authorize and Forget carried out at once.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriseapp::Reply;
use reprisebus::bolt::{self, Change, Found};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

/// A read or a change off the loop: what boltd knows afterwards (None: it can't be reached), and
/// what to tell when the change failed.
pub(crate) struct Reading {
    pub(crate) found: Option<Found>,
    pub(crate) failed: Option<String>,
}

#[derive(Default)]
pub(crate) struct Thunderbolt {
    pub(crate) found: Option<Found>,
    pub(crate) job: Option<std::sync::mpsc::Receiver<Reading>>,
    pub(crate) acting: bool,
}

fn carry_out(d: bolt::Device, change: Change) -> Reading {
    let failed = match bolt::carry_out(&d, change) {
        Ok(()) | Err(None) => None,
        Err(Some(why)) => {
            let what = match change {
                Change::Forget => "forgotten",
                Change::Authorize | Change::Always => "authorized",
            };
            Some(format!("{} could not be {what}.\n\n{why}", d.label()))
        }
    };
    Reading {
        found: bolt::read(),
        failed,
    }
}

impl SettingsApp {
    /// Read boltd off the loop when the page opens; [`SettingsApp::thunderbolt_done`] goes on
    /// polling while it stays open.
    pub(crate) fn ensure_thunderbolt(&mut self) {
        if registry::open(&self.sheet) == Page::Thunderbolt && self.thunderbolt.job.is_none() {
            self.start_thunderbolt(|| Reading {
                found: bolt::read(),
                failed: None,
            });
        }
    }

    /// `shown`: what the list showed, when `thunderbolt.found` just replaced it.
    fn show_thunderbolt(&mut self, shown: Option<&Found>) {
        let page = &mut self.sheet.pages[Page::Thunderbolt.index()];
        let busy = self.thunderbolt.acting;
        pages::show_thunderbolt(page, self.thunderbolt.found.as_ref(), shown, busy);
    }

    /// The list picked another device: the buttons follow it.
    pub(crate) fn thunderbolt_picked(&mut self) -> Reply {
        self.show_thunderbolt(None);
        Reply::redraw()
    }

    /// Authorize or Forget on the picked device.
    pub(crate) fn thunderbolt_pressed(&mut self, id: &str) -> Reply {
        let change = if id == pages::TB_FORGET {
            Change::Forget
        } else {
            Change::Always
        };
        let devices = self.thunderbolt.found.as_ref().map(|f| &f.devices[..]);
        let page = &self.sheet.pages[Page::Thunderbolt.index()];
        let Some(d) = devices
            .and_then(|d| pages::selected_device(page, d))
            .cloned()
        else {
            return Reply::none();
        };
        if self.thunderbolt.acting {
            return Reply::none();
        }
        self.thunderbolt.acting = true;
        self.show_thunderbolt(None);
        self.start_thunderbolt(move || carry_out(d, change));
        Reply::redraw()
    }

    /// A read or a change finished: show it, and read again in [`POLL`] while the page is open.
    /// True when the page changed.
    pub(crate) fn thunderbolt_done(&mut self) -> bool {
        let Some(rx) = &self.thunderbolt.job else {
            return false;
        };
        let done = match take(rx) {
            Ok(None) => return false,
            Ok(Some(r)) => r,
            Err(()) => Reading {
                found: self.thunderbolt.found.clone(),
                failed: None,
            },
        };
        self.thunderbolt.job = None;
        let acted = std::mem::take(&mut self.thunderbolt.acting);
        if let Some(why) = &done.failed {
            self.fail(why);
        }
        let changed = acted || done.found != self.thunderbolt.found;
        let shown = std::mem::replace(&mut self.thunderbolt.found, done.found);
        if changed {
            self.show_thunderbolt(shown.as_ref());
        }
        if registry::open(&self.sheet) == Page::Thunderbolt {
            self.start_thunderbolt(|| {
                std::thread::sleep(POLL);
                Reading {
                    found: bolt::read(),
                    failed: None,
                }
            });
        }
        changed
    }

    #[cfg(not(test))]
    fn start_thunderbolt(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.thunderbolt.job = Some(self.spawn(job));
    }

    /// Tests feed `thunderbolt.job` themselves, so nothing reaches the system bus.
    #[cfg(test)]
    fn start_thunderbolt(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}
}
