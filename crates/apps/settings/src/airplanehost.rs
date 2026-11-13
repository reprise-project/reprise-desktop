//! The Network page's airplane mode: read off the loop while the page shows, again every
//! [`POLL`], and the check box carried out at once.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use reprisebus::airplane;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

/// A read or a change off the loop: airplane mode afterwards (None: no radio switch), and what
/// to tell when the change failed.
pub(crate) struct Reading {
    pub(crate) state: Option<bool>,
    pub(crate) failed: Option<String>,
}

#[derive(Default)]
pub(crate) struct Airplane {
    pub(crate) state: Option<bool>,
    pub(crate) job: Option<std::sync::mpsc::Receiver<Reading>>,
    pub(crate) acting: bool,
}

fn read() -> Option<bool> {
    let conn = reprisebus::system().ok()?;
    reprisebus::wait(airplane::read(&conn))
}

fn set(on: bool) -> Reading {
    let failed = match reprisebus::system() {
        Ok(conn) => reprisebus::wait(airplane::set(&conn, on)).err(),
        Err(e) => Some(e.to_string()),
    };
    let to = if on { "turned on" } else { "turned off" };
    Reading {
        state: read(),
        failed: failed.map(|why| format!("Airplane mode could not be {to}.\n\n{why}")),
    }
}

impl SettingsApp {
    /// Read the radios off the loop when the page opens; [`SettingsApp::airplane_done`] goes on
    /// polling while it stays open.
    pub(crate) fn ensure_airplane(&mut self) {
        if registry::open(&self.sheet) == Page::Network && self.airplane.job.is_none() {
            self.start_airplane(|| Reading {
                state: read(),
                failed: None,
            });
        }
    }

    pub(crate) fn airplane_toggled(&mut self) {
        let page = &mut self.sheet.pages[Page::Network.index()];
        let on = page.checked(pages::AIRPLANE) == Some(true);
        if self.airplane.acting {
            pages::show_airplane(page, self.airplane.state, true);
            return;
        }
        pages::show_airplane(page, Some(on), true);
        self.airplane.acting = true;
        self.start_airplane(move || set(on));
    }

    /// A read or a change finished: show it, and read again in [`POLL`] while the page is open.
    /// True when the page changed.
    pub(crate) fn airplane_done(&mut self) -> bool {
        let Some(rx) = &self.airplane.job else {
            return false;
        };
        let done = match take(rx) {
            Ok(None) => return false,
            Ok(Some(r)) => r,
            Err(()) => Reading {
                state: self.airplane.state,
                failed: None,
            },
        };
        self.airplane.job = None;
        let acted = std::mem::take(&mut self.airplane.acting);
        if let Some(why) = &done.failed {
            self.fail(why);
        }
        let changed = acted || done.state != self.airplane.state;
        self.airplane.state = done.state;
        if changed {
            let page = &mut self.sheet.pages[Page::Network.index()];
            pages::show_airplane(page, done.state, false);
        }
        if registry::open(&self.sheet) == Page::Network {
            self.start_airplane(|| {
                std::thread::sleep(POLL);
                Reading {
                    state: read(),
                    failed: None,
                }
            });
        }
        changed
    }

    #[cfg(not(test))]
    fn start_airplane(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.airplane.job = Some(self.spawn(job));
    }

    /// Tests feed `airplane.job` themselves, so nothing reaches the system bus or rfkill.
    #[cfg(test)]
    fn start_airplane(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}
}
