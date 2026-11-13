//! The Taskbar page's calendar events note: whether evolution-data-server is installed, read off
//! the loop once the page first opens.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(crate) struct Events {
    /// None until read.
    pub(crate) installed: Option<bool>,
    pub(crate) job: Option<Receiver<bool>>,
}

impl SettingsApp {
    pub(crate) fn ensure_events(&mut self) {
        let e = &self.events;
        if registry::open(&self.sheet) == Page::Taskbar && e.installed.is_none() && e.job.is_none()
        {
            self.start_events();
        }
    }

    /// The read finished: show its note. True when the page changed.
    pub(crate) fn events_done(&mut self) -> bool {
        let Some(rx) = &self.events.job else {
            return false;
        };
        let installed = match take(rx) {
            Ok(None) => return false,
            Ok(Some(i)) => i,
            Err(()) => false,
        };
        self.events.job = None;
        self.events.installed = Some(installed);
        pages::show_events(
            &mut self.sheet.pages[Page::Taskbar.index()],
            Some(installed),
        );
        true
    }

    #[cfg(not(test))]
    fn start_events(&mut self) {
        self.events.job = Some(self.spawn(|| {
            let conn = reprisebus::session();
            conn.is_ok_and(|c| reprisebus::wait(reprisebus::eds::installed(&c)))
        }));
    }

    /// Tests feed `events.job` themselves, so nothing reaches the session bus.
    #[cfg(test)]
    fn start_events(&mut self) {}
}
