//! The Magnifier & Filters page's screen reader check boxes. Whether Orca is installed and running
//! is read off the loop while the page shows, again every [`POLL`]; both boxes act at once.

use crate::registry::{self, Page};
use crate::{SettingsApp, autostart, magnifierpage, take};
use reprisefs::screenreader;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reader {
    pub(crate) installed: bool,
    pub(crate) running: bool,
}

/// A read or a switch off the loop: the state afterwards, and what to tell when the switch failed.
pub(crate) struct Reading {
    pub(crate) reader: Reader,
    pub(crate) failed: Option<String>,
}

fn read() -> Reader {
    Reader {
        installed: screenreader::installed(),
        running: !screenreader::running().is_empty(),
    }
}

/// Start the reader in its app scope, or ask every copy to quit, then read back after a [`POLL`].
fn switch(on: bool) -> Reading {
    let failed = if on {
        let argv = [screenreader::PROGRAM.into()];
        repriseapp::spawn_in(&argv, None, Some(screenreader::PROGRAM))
            .err()
            .map(|e| format!("The screen reader could not be started.\n\n{e}"))
    } else {
        screenreader::stop(&screenreader::running());
        None
    };
    std::thread::sleep(POLL);
    Reading {
        reader: read(),
        failed,
    }
}

impl SettingsApp {
    /// Read the reader off the loop when the page opens; [`SettingsApp::reader_done`] goes on
    /// polling while it stays open.
    pub(crate) fn ensure_reader(&mut self) {
        if registry::open(&self.sheet) == Page::MagnifierFilters && self.reader_job.is_none() {
            self.start_reader(|| Reading {
                reader: read(),
                failed: None,
            });
        }
    }

    /// A read or a switch finished: show it, and poll again while the page is open. True when the
    /// page changed.
    pub(crate) fn reader_done(&mut self) -> bool {
        let Some(rx) = &self.reader_job else {
            return false;
        };
        let done = match take(rx) {
            Ok(None) => return false,
            Ok(Some(r)) => r,
            Err(()) => Reading {
                reader: self.reader,
                failed: None,
            },
        };
        self.reader_job = None;
        if let Some(why) = &done.failed {
            self.fail(why);
        }
        let changed = done.reader != self.reader;
        self.reader = done.reader;
        self.show_reader();
        if registry::open(&self.sheet) == Page::MagnifierFilters {
            self.start_reader(|| {
                std::thread::sleep(POLL);
                Reading {
                    reader: read(),
                    failed: None,
                }
            });
        }
        changed
    }

    /// Use the screen reader moved. A poll under way is dropped, since it may have read the
    /// state from before.
    pub(crate) fn reader_toggled(&mut self) {
        let page = &self.sheet.pages[Page::MagnifierFilters.index()];
        let on = page.checked(magnifierpage::READER) == Some(true);
        self.reader.running = on;
        self.start_reader(move || switch(on));
    }

    /// Start the screen reader when I log on moved: hide or show its autostart entry now.
    pub(crate) fn reader_logon_toggled(&mut self) {
        let page = &self.sheet.pages[Page::MagnifierFilters.index()];
        let on = page.checked(magnifierpage::READER_LOGON) == Some(true);
        let dir = self.user_autostart();
        let config_dirs = reprisefs::autostart::system_dirs();
        let set =
            autostart::set_reader_at_logon(&dir, &self.entries, on, &self.desktops, &config_dirs);
        if let Err(e) = set {
            log::warn!("screen reader autostart: {e}");
            self.fail("The autostart entry could not be written.");
        }
        self.rescan();
        self.show_reader();
    }

    /// Put the reader as last read, and its autostart entries, on the page.
    pub(crate) fn show_reader(&mut self) {
        let at_logon = autostart::reader_at_logon(&self.entries, &self.desktops);
        magnifierpage::show_reader(
            &mut self.sheet.pages[Page::MagnifierFilters.index()],
            self.reader.installed,
            self.reader.running,
            at_logon,
        );
    }

    #[cfg(not(test))]
    fn start_reader(&mut self, job: impl FnOnce() -> Reading + Send + 'static) {
        self.reader_job = Some(self.spawn(job));
    }

    /// Tests feed `reader_job` themselves, so nothing walks `/proc` or starts a reader.
    #[cfg(test)]
    fn start_reader(&mut self, _: impl FnOnce() -> Reading + Send + 'static) {}
}
