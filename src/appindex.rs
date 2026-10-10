//! Keeps `State::app_index` current: inotify watches on every XDG applications directory, debounced through a timer because a package install touches many files. A missing directory is covered by a watch on its nearest existing ancestor. The scan runs from the timer, never an input callback, and only when something changed.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{
    Interest, Mode, PostAction, RegistrationToken, generic::Generic,
};
use std::ffi::OsString;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct AppIndexWatch {
    settle: Option<RegistrationToken>,
}

struct Watch(DirWatch);

impl AsFd for Watch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: the fd is the watch's inotify fd, open for as long as the
        // watch, which outlives the borrow.
        unsafe { BorrowedFd::borrow_raw(self.0.fd()) }
    }
}

/// What to watch for `dirs`: each existing directory for any change to its entries, and for each missing one its nearest existing ancestor.
fn wants(dirs: &[PathBuf]) -> Vec<Want> {
    let mut want: Vec<Want> = Vec::new();
    for dir in dirs {
        if dir.is_dir() && !want.iter().any(|w| w.dir == *dir) {
            want.push(Want {
                dir: dir.clone(),
                mask: WatchMask::CREATE
                    | WatchMask::DELETE
                    | WatchMask::MOVED_FROM
                    | WatchMask::MOVED_TO
                    | WatchMask::CLOSE_WRITE,
                only: Vec::new(),
            });
        }
    }
    for dir in dirs {
        let Some((parent, child)) = next_missing(dir) else {
            continue;
        };
        match want.iter_mut().find(|w| w.dir == parent) {
            // An empty `only` already counts every entry.
            Some(w) if !w.only.is_empty() && !w.only.contains(&child) => w.only.push(child),
            Some(_) => {}
            None => want.push(Want {
                dir: parent,
                mask: WatchMask::CREATE | WatchMask::MOVED_TO,
                only: vec![child],
            }),
        }
    }
    want
}

fn next_missing(dir: &Path) -> Option<(PathBuf, OsString)> {
    if dir.is_dir() {
        return None;
    }
    let mut child = dir;
    loop {
        let parent = child.parent()?;
        if parent.is_dir() {
            return Some((parent.to_path_buf(), child.file_name()?.to_os_string()));
        }
        child = parent;
    }
}

pub fn setup(state: &mut State) {
    let mut watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("app index: inotify unavailable: {e}");
            return;
        }
    };
    watch.sync(&wants(&reprisefs::apps::default_dirs()));
    let source = Generic::new(Watch(watch), Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, watch, state| {
        // SAFETY: the watch is not dropped here, only drained and
        // resynced; the source keeps its fd registered for the watch's
        // life.
        let watch = unsafe { &mut watch.get_mut().0 };
        if watch.drain() {
            // Point the watches at the directories as they are now: one made since is watched before the scan reads it.
            watch.sync(&wants(&reprisefs::apps::default_dirs()));
            arm(state);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("app index: {}", e.error);
    }
}

fn arm(state: &mut State) {
    if let Some(t) = state.apps_watch.settle.take() {
        state.handle.remove(t);
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(SETTLE), |_, _, state| {
            state.apps_watch.settle = None;
            rescan(state);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => state.apps_watch.settle = Some(t),
        // Without the timer every wakeup rescans at once.
        Err(_) => rescan(state),
    }
}

/// The directories went quiet: scan them, rank the launcher's text against the new index and redraw what shows apps.
fn rescan(state: &mut State) {
    let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
    log::info!("app index rescanned ({} entries)", apps.len());
    state.app_index = apps;
    state.launcher_index_changed();
    state.start_panel_index_changed();
    state.redraw_taskbars();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_applications_directory_made_later_is_watched() {
        let root = std::env::temp_dir().join(format!("reprise-appindex-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let dirs = [root.join("share/applications")];
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&dirs));

        std::fs::write(root.join("notes.txt"), "x").unwrap();
        assert!(!w.drain(), "a file in the ancestor: no rescan");
        std::fs::create_dir(root.join("share")).unwrap();
        assert!(w.drain(), "the next directory down made: rescan");
        w.sync(&wants(&dirs));

        std::fs::create_dir(&dirs[0]).unwrap();
        assert!(w.drain(), "the applications directory made: rescan");
        w.sync(&wants(&dirs));
        std::fs::write(root.join("share/other.txt"), "x").unwrap();
        assert!(
            !w.drain(),
            "a file beside the applications directory: no rescan"
        );
        std::fs::write(dirs[0].join("app.desktop"), "").unwrap();
        assert!(w.drain(), "a new entry: rescan");

        let _ = std::fs::remove_dir_all(&root);
    }
}
