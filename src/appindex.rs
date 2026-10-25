//! Keeps `State::app_index` current: inotify watches on every XDG applications directory, debounced through a timer because a package install touches many files. A missing directory is covered by a watch on its nearest existing ancestor. The scan runs from the timer, never an input callback, and only when something changed; it and the watch syncs run on file jobs.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, JobWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{
    Interest, Mode, PostAction, RegistrationToken, generic::Generic,
};
use std::ffi::OsString;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::time::Duration;

const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct AppIndexWatch {
    settle: Option<RegistrationToken>,
    watch: JobWatch,
    /// A scan is out on a file job.
    scanning: bool,
    /// A scan was asked for while a sync or scan was out.
    scan_again: bool,
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
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("app index: inotify unavailable: {e}");
            return;
        }
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("app index: no watch fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, _, state| {
        if state.apps_watch.watch.drain() {
            // Point the watches at the directories as they are now: one made since is watched before the scan reads it.
            sync(state);
            arm(state);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("app index: {}", e.error);
    }
    state.apps_watch.watch.watch = Some(watch);
    sync(state);
}

fn sync(state: &mut State) {
    sync_with(state, || wants(&reprisefs::apps::default_dirs()));
}

/// Syncs the watch to what `find` returns, worked out on a file job: a directory on a mount
/// that stopped answering holds the job, never the loop.
fn sync_with<F>(state: &mut State, find: F)
where
    F: FnOnce() -> Vec<Want> + Send + 'static,
{
    let Some(job) = state.apps_watch.watch.start() else {
        return;
    };
    state.spawn_fs_job(move || {
        let want = find();
        let added = job.adder.run(&want);
        Box::new(move |state: &mut State| {
            if state.apps_watch.watch.landed(&want, added) {
                sync(state);
            } else if std::mem::take(&mut state.apps_watch.scan_again) {
                rescan(state);
            }
        })
    });
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

/// The directories went quiet: scan them on a file job, then rank the launcher's text against
/// the new index and redraw what shows apps. A scan waits for the sync before it.
fn rescan(state: &mut State) {
    let w = &mut state.apps_watch;
    if w.watch.syncing() || w.scanning {
        w.scan_again = true;
        return;
    }
    w.scanning = true;
    state.spawn_fs_job(|| {
        let apps = reprisefs::apps::scan(&reprisefs::apps::default_dirs());
        Box::new(move |state: &mut State| {
            log::info!("app index rescanned ({} entries)", apps.len());
            state.app_index = apps;
            state.launcher_index_changed();
            state.start_panel_index_changed();
            state.redraw_taskbars();
            let w = &mut state.apps_watch;
            w.scanning = false;
            if std::mem::take(&mut w.scan_again) {
                rescan(state);
            }
        })
    });
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

    /// The directory lookups behind a sync stand in for a stat on a mount that stopped
    /// answering: the loop turns while they hang, and the watch lands once they answer.
    #[test]
    fn a_sync_whose_lookups_hang_leaves_the_loop_turning() {
        let root = crate::harness::scratch().join("appindex-stall/applications");
        std::fs::create_dir_all(&root).unwrap();
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.state.apps_watch.watch.watch = Some(DirWatch::new().unwrap());
        let (answer, stalled) = std::sync::mpsc::channel::<()>();
        let dirs = [root.clone()];
        let started = std::time::Instant::now();
        sync_with(&mut h.state, move || {
            let _ = stalled.recv_timeout(Duration::from_secs(5));
            wants(&dirs)
        });
        h.run_for(50);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "the loop turned while the lookups hung"
        );
        assert!(h.state.apps_watch.watch.syncing(), "the sync is still out");
        rescan(&mut h.state);
        assert!(
            h.state.apps_watch.scan_again && !h.state.apps_watch.scanning,
            "a scan waits for the sync"
        );
        h.state.apps_watch.scan_again = false;

        answer.send(()).unwrap();
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while h.state.apps_watch.watch.syncing() && std::time::Instant::now() < end {
            h.run_for(10);
        }
        assert!(!h.state.apps_watch.watch.syncing(), "the sync landed");
        std::fs::write(root.join("app.desktop"), "").unwrap();
        let watch = h.state.apps_watch.watch.watch.as_mut().unwrap();
        assert!(
            watch.drain(),
            "the directory is watched once the sync lands"
        );
    }
}
