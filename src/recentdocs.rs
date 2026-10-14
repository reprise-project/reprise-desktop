//! The recent files and document folders the Start menus and jump lists show. A recent file can
//! sit on a mount that never answers, so what each path is comes from a file job: the loop reads
//! the bookmarks and what the last check found, and shows a path not checked yet.

use reprisefs::apps::AppEntry;
use reprisefs::recent::{Bookmark, RecentFiles};
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::compositor::State;

/// What a path was found to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    File,
    Dir,
    Neither,
}

type Stat = fn(&Path) -> io::Result<std::fs::Metadata>;

#[derive(Default)]
pub struct RecentDocs {
    files: RecentFiles,
    /// Filled by the check job one path at a time, so a path that hangs keeps only itself and
    /// the paths after it unchecked.
    found: Arc<Mutex<HashMap<PathBuf, Found>>>,
    /// A check is running; another is not started until it answers.
    checking: bool,
}

impl RecentDocs {
    fn found(&self, path: &Path) -> Option<Found> {
        self.found.lock().ok()?.get(path).copied()
    }

    /// The newest `limit` bookmarks `keep` takes that were not found missing or a folder.
    fn pick(&mut self, limit: usize, keep: impl Fn(&Bookmark) -> bool) -> Vec<PathBuf> {
        let marks = self.files.bookmarks();
        let found = self.found.lock().unwrap_or_else(|e| e.into_inner());
        marks
            .iter()
            .filter(|b| keep(b) && matches!(found.get(&b.path), None | Some(Found::File)))
            .take(limit)
            .map(|b| b.path.clone())
            .collect()
    }

    pub fn current(&mut self, limit: usize) -> Vec<PathBuf> {
        self.pick(limit, |_| true)
    }

    /// `current`, keeping only the files the application `entry` used: its jump list.
    pub fn for_app(&mut self, limit: usize, entry: &AppEntry) -> Vec<PathBuf> {
        self.pick(limit, |b| reprisefs::recent::used_by_entry(b, entry))
    }

    /// The document folders that were not found missing.
    pub fn document_dirs(&self) -> Vec<(PathBuf, &'static str)> {
        reprisefs::places::document_dir_paths()
            .into_iter()
            .filter(|(dir, _)| matches!(self.found(dir), None | Some(Found::Dir)))
            .collect()
    }

    /// The paths a check asks about (the document folders, and the bookmarks with `files`), or
    /// None while a check is running.
    fn paths_to_check(&mut self, files: bool) -> Option<Vec<PathBuf>> {
        if self.checking {
            return None;
        }
        self.checking = true;
        let mut paths: Vec<PathBuf> = reprisefs::places::document_dir_paths()
            .into_iter()
            .map(|(dir, _)| dir)
            .collect();
        if files {
            paths.extend(self.files.bookmarks().iter().map(|b| b.path.clone()));
        }
        Some(paths)
    }
}

/// Stats `paths` in order into `found`, then drops what `found` holds of other paths.
fn check(paths: Vec<PathBuf>, found: &Mutex<HashMap<PathBuf, Found>>, stat: Stat) {
    for p in &paths {
        let what = match stat(p) {
            Ok(m) if m.is_file() => Found::File,
            Ok(m) if m.is_dir() => Found::Dir,
            _ => Found::Neither,
        };
        if let Ok(mut f) = found.lock() {
            f.insert(p.clone(), what);
        }
    }
    let asked: HashSet<&PathBuf> = paths.iter().collect();
    if let Ok(mut f) = found.lock() {
        f.retain(|p, _| asked.contains(p));
    }
}

impl State {
    /// Checks the recent files and document folders off the loop, unless a check is running. The
    /// menus opened meanwhile show what the last check found.
    pub fn refresh_recent(&mut self) {
        let files = self.config.recent_files;
        let Some(paths) = self.recent.paths_to_check(files) else {
            return;
        };
        let found = self.recent.found.clone();
        self.spawn_fs_job(move || {
            check(paths, &found, |p| std::fs::metadata(p));
            Box::new(|state: &mut State| state.recent.checking = false)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn xbel(paths: &[&Path]) -> String {
        let marks: String = paths
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "<bookmark href=\"file://{}\" modified=\"2026-01-0{}T00:00:00Z\"></bookmark>\n",
                    p.display(),
                    9 - i
                )
            })
            .collect();
        format!("<xbel>{marks}</xbel>")
    }

    fn open_then_stat(p: &Path) -> io::Result<std::fs::Metadata> {
        std::fs::File::open(p)?.metadata()
    }

    /// The menus read only the bookmarks and what the check found: with the check blocked on a
    /// path that does not answer, they list at once, a path not checked yet included, and drop
    /// a missing file and a folder as soon as the check has passed them.
    #[test]
    fn recent_files_list_while_the_check_hangs() {
        use std::os::unix::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("reprise-recentdocs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        let (kept, gone, folder, hang, later) = (
            dir.join("kept.txt"),
            dir.join("gone.txt"),
            dir.join("folder"),
            dir.join("hang"),
            dir.join("later.txt"),
        );
        std::fs::write(&kept, "").unwrap();
        let c = std::ffi::CString::new(hang.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path for the call's duration.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        let xbel_path = dir.join("recently-used.xbel");
        let order = [&*kept, &*gone, &*folder, &*hang, &*later];
        std::fs::write(&xbel_path, xbel(&order)).unwrap();
        let mut docs = RecentDocs {
            files: RecentFiles::at(Some(xbel_path)),
            ..Default::default()
        };
        assert_eq!(docs.current(10).len(), 5, "nothing checked: all listed");

        let paths = docs.paths_to_check(true).unwrap();
        assert!(docs.paths_to_check(true).is_none(), "one check at a time");
        let found = docs.found.clone();
        let job = std::thread::spawn(move || check(paths, &found, open_then_stat));
        let end = Instant::now() + Duration::from_secs(5);
        while docs.found(&folder).is_none() {
            assert!(Instant::now() < end, "timed out waiting for the check");
            std::thread::sleep(Duration::from_millis(5));
        }
        let listed = Instant::now();
        assert_eq!(
            docs.current(10),
            [kept.clone(), hang.clone(), later.clone()]
        );
        assert!(listed.elapsed() < Duration::from_secs(1));
        assert!(!job.is_finished(), "the check still hangs");

        // A writer opening the pipe answers the blocked open.
        let _w = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&hang)
            .unwrap();
        job.join().unwrap();
        assert_eq!(docs.current(10), [kept], "the pipe is not a file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
