//! The Search page's content search note: whether localsearch is installed, read off the loop once
//! the page first opens.

use crate::registry::{self, Page};
use crate::{SettingsApp, pages, take};
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub(crate) struct Contents {
    /// None until read.
    pub(crate) installed: Option<bool>,
    pub(crate) job: Option<Receiver<bool>>,
}

impl SettingsApp {
    pub(crate) fn ensure_contents(&mut self) {
        let c = &self.contents;
        if registry::open(&self.sheet) == Page::Search && c.installed.is_none() && c.job.is_none() {
            self.start_contents();
        }
    }

    /// The read finished: show its note. True when the page changed.
    pub(crate) fn contents_done(&mut self) -> bool {
        let Some(rx) = &self.contents.job else {
            return false;
        };
        let installed = match take(rx) {
            Ok(None) => return false,
            Ok(Some(i)) => i,
            Err(()) => false,
        };
        self.contents.job = None;
        self.contents.installed = Some(installed);
        pages::show_contents(&mut self.sheet.pages[Page::Search.index()], Some(installed));
        true
    }

    #[cfg(not(test))]
    fn start_contents(&mut self) {
        self.contents.job = Some(self.spawn(|| {
            let conn = reprisebus::session();
            conn.is_ok_and(|c| reprisebus::wait(reprisebus::localsearch::installed(&c)))
        }));
    }

    /// Tests feed `contents.job` themselves, so nothing reaches the session bus.
    #[cfg(test)]
    fn start_contents(&mut self) {}
}
