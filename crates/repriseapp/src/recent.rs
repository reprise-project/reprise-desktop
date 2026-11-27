//! The documents a program opens and saves, added to the session's `recently-used.xbel` under
//! its own desktop entry, so Recent Items and its taskbar button's jump list show them.

use reprisefs::apps::{AppEntry, DesktopId};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

/// Records on one worker thread, started at the first record. Dropping it waits up to
/// [`EXIT_WAIT`] for the records queued, so a save made while the program closes lands.
#[derive(Debug)]
pub struct Recent {
    desktop: DesktopId,
    /// None records nothing.
    xbel: Option<PathBuf>,
    worker: Option<Worker>,
}

#[derive(Debug)]
struct Worker {
    paths: mpsc::Sender<PathBuf>,
    /// Disconnects when the worker ends.
    done: mpsc::Receiver<()>,
}

/// Past the xbel lock's own two seconds, so a record waiting on the lock lands or gives up first.
const EXIT_WAIT: Duration = Duration::from_secs(3);

impl Recent {
    /// For the program of desktop file `desktop` (`reprise-notepad.desktop`), into the session's
    /// file.
    pub fn new(desktop: &str) -> Recent {
        Recent::at(desktop, reprisefs::recent::xbel_path())
    }

    /// [`Recent::new`] into the xbel file at `xbel`.
    pub fn at(desktop: &str, xbel: Option<PathBuf>) -> Recent {
        let desktop = DesktopId::named(desktop).expect("a desktop file name");
        Recent {
            desktop,
            xbel,
            worker: None,
        }
    }

    /// Queues `path`, unless `[privacy] recent_files` is off.
    pub fn record(&mut self, path: &Path) {
        let Some(xbel) = &self.xbel else {
            return;
        };
        if !repriseconf::current().recent_files {
            return;
        }
        if self.worker.is_none() {
            self.worker = spawn(xbel.clone(), self.desktop.clone());
        }
        if let Some(w) = &self.worker
            && w.paths.send(path.to_path_buf()).is_err()
        {
            log::warn!("recent files: the recording thread is gone");
            self.worker = None;
        }
    }

    /// Waits for the records queued.
    pub fn settle(&mut self) {
        self.finish(None);
    }

    fn finish(&mut self, limit: Option<Duration>) {
        let Some(Worker { paths, done }) = self.worker.take() else {
            return;
        };
        drop(paths);
        let Some(limit) = limit else {
            let _ = done.recv();
            return;
        };
        if let Err(mpsc::RecvTimeoutError::Timeout) = done.recv_timeout(limit) {
            log::warn!("recent files: gave up waiting on the records queued");
        }
    }
}

impl Drop for Recent {
    fn drop(&mut self) {
        self.finish(Some(EXIT_WAIT));
    }
}

fn spawn(xbel: PathBuf, desktop: DesktopId) -> Option<Worker> {
    let (paths, queued) = mpsc::channel::<PathBuf>();
    let (ended, done) = mpsc::channel::<()>();
    let spawned = std::thread::Builder::new()
        .name("recent files".into())
        .spawn(move || {
            let _ended = ended;
            let entry = own_entry(&desktop);
            let globs = reprisefs::apps::mime_globs();
            while let Ok(first) = queued.recv() {
                let mut batch = vec![first];
                for p in queued.try_iter() {
                    if !batch.contains(&p) {
                        batch.push(p);
                    }
                }
                let mimes: Vec<_> = (batch.iter())
                    .map(|p| reprisefs::apps::mime_of_in(&globs, p))
                    .collect();
                let opened: Vec<_> = (batch.iter().zip(&mimes))
                    .map(|(p, m)| (p.as_path(), Some(&entry), m.as_deref()))
                    .collect();
                if let Err(e) = reprisefs::recent::record_at(&xbel, &opened) {
                    log::warn!("recent files: could not record {batch:?}: {e}");
                }
            }
        });
    match spawned {
        Ok(_) => Some(Worker { paths, done }),
        Err(e) => {
            log::warn!("recent files: no thread to record with ({e})");
            None
        }
    }
}

/// The installed entry of `desktop`, else one named and run as the file's stem, the program the
/// installed one runs.
fn own_entry(desktop: &DesktopId) -> AppEntry {
    let installed = reprisefs::apps::find_file(desktop)
        .and_then(|f| reprisefs::apps::read_desktop_file(&f).ok())
        .and_then(|text| reprisefs::apps::parse_desktop_any(&text, desktop));
    installed.unwrap_or_else(|| {
        let stem = desktop.stem().to_string();
        AppEntry {
            plain_name: None,
            exec_line: format!("{stem} %f"),
            name: stem,
            terminal: false,
            id: desktop.clone(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
            actions: Vec::new(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_queued_as_the_program_ends_lands() {
        let dir = tempfile::tempdir().unwrap();
        let xbel = dir.path().join("recently-used.xbel");
        let docs: Vec<_> = (0..20)
            .map(|i| dir.path().join(format!("{i}.txt")))
            .collect();
        let mut recent = Recent::at("reprise-notepad.desktop", Some(xbel.clone()));
        for d in &docs {
            std::fs::write(d, "").unwrap();
            recent.record(d);
            recent.record(d);
        }
        drop(recent);
        let mut got = reprisefs::recent::RecentFiles::at(Some(xbel)).current(100);
        got.sort();
        let mut want = docs;
        want.sort();
        assert_eq!(got, want);
    }
}
