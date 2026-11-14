//! The Thunderbolt page's devices: boltd's signals followed off the loop from the page's first
//! opening, and Authorize and Forget carried out at once.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use repriseapp::Reply;
use reprisebus::bolt::{self, Change, Found};
use std::sync::mpsc::Receiver;

/// A read or a change off the loop: what boltd knows afterwards (None: it can't be reached), and
/// what to tell when the change failed.
pub(crate) struct Reading {
    pub(crate) found: Option<Found>,
    pub(crate) failed: Option<String>,
}

#[derive(Default)]
pub(crate) struct Thunderbolt {
    pub(crate) found: Option<Found>,
    /// A change and the read after it.
    pub(crate) job: Option<Receiver<Reading>>,
    /// What boltd knows after each change it signals, for the rest of the run.
    pub(crate) follow: Option<Receiver<Option<Found>>>,
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

/// Start boltd with one read, then follow its signals, sending what it knows after each.
#[cfg(not(test))]
fn follow(tx: std::sync::mpsc::Sender<Option<Found>>, wake: impl Fn()) {
    let conn = match reprisebus::system() {
        Ok(conn) => conn,
        Err(e) => {
            log::info!("bolt: no system bus ({e})");
            let _ = tx.send(None);
            return wake();
        }
    };
    if let Err(e) = reprisebus::wait(bolt::found(&conn)) {
        log::info!("bolt: {e}");
    }
    reprisebus::wait(bolt::follow(&conn, |f| {
        let _ = tx.send(f);
        wake();
    }));
}

impl SettingsApp {
    /// Follow boltd off the loop once the page opens.
    pub(crate) fn ensure_thunderbolt(&mut self) {
        if registry::open(&self.sheet) == Page::Thunderbolt && self.thunderbolt.follow.is_none() {
            self.start_follow();
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

    /// boltd signalled a change, or a change finished: show it. True when the page changed.
    pub(crate) fn thunderbolt_done(&mut self) -> bool {
        let mut latest = None;
        while let Some(rx) = &self.thunderbolt.follow {
            match take(rx) {
                Ok(Some(f)) => latest = Some(f),
                Ok(None) => break,
                Err(()) => self.thunderbolt.follow = None,
            }
        }
        let mut changed = latest.is_some_and(|f| self.thunderbolt_found(f, false));
        let Some(rx) = &self.thunderbolt.job else {
            return changed;
        };
        let done = match take(rx) {
            Ok(None) => return changed,
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
        changed |= self.thunderbolt_found(done.found, acted);
        changed
    }

    /// Show `found` when it differs from the list, or when a change just ended (`acted`).
    fn thunderbolt_found(&mut self, found: Option<Found>, acted: bool) -> bool {
        let changed = acted || found != self.thunderbolt.found;
        let shown = std::mem::replace(&mut self.thunderbolt.found, found);
        if changed {
            self.show_thunderbolt(shown.as_ref());
        }
        changed
    }

    #[cfg(not(test))]
    fn start_thunderbolt(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.thunderbolt.job = Some(self.spawn(job));
    }

    #[cfg(not(test))]
    fn start_follow(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        let waker = self.waker.clone();
        let started = std::thread::Builder::new()
            .name("reprise-settings-bolt".into())
            .spawn(move || {
                follow(tx, || {
                    if let Some(w) = &waker {
                        w.wake();
                    }
                })
            });
        match started {
            Ok(_) => self.thunderbolt.follow = Some(rx),
            Err(e) => log::warn!("could not follow Thunderbolt devices: {e}"),
        }
    }

    /// Tests feed `thunderbolt.job` and `thunderbolt.follow` themselves, so nothing reaches the
    /// system bus.
    #[cfg(test)]
    fn start_thunderbolt(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}

    #[cfg(test)]
    fn start_follow(&mut self) {}
}
