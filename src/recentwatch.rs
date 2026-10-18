//! Reads the recent files again when `recently-used.xbel` changes, and checks the document
//! folders again when one comes or goes or `user-dirs.dirs` changes, so open lists follow them.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

struct Watch(DirWatch);

impl AsFd for Watch {
    fn as_fd(&self) -> BorrowedFd<'_> {
        // SAFETY: the fd is the watch's inotify fd, open for as long as the
        // watch, which outlives the borrow.
        unsafe { BorrowedFd::borrow_raw(self.0.fd()) }
    }
}

/// The file's directory, waking only for the file: GLib and KDE replace the xbel by a rename.
fn wants(file: &Path) -> Vec<Want> {
    let (Some(dir), Some(name)) = (file.parent(), file.file_name()) else {
        return Vec::new();
    };
    vec![Want {
        dir: dir.to_path_buf(),
        mask: WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE,
        only: vec![name.to_os_string()],
    }]
}

/// Each document folder's parent, waking only for the folder's name.
fn folder_wants(dirs: &[PathBuf]) -> Vec<Want> {
    dirs.iter()
        .filter_map(|d| {
            Some(Want {
                dir: d.parent()?.to_path_buf(),
                mask: WatchMask::CREATE
                    | WatchMask::DELETE
                    | WatchMask::MOVED_TO
                    | WatchMask::MOVED_FROM,
                only: vec![d.file_name()?.to_os_string()],
            })
        })
        .collect()
}

/// `user-dirs.dirs`, and the document folders `dirs` read from it.
fn document_wants(user_dirs: &Path, dirs: &[PathBuf]) -> Vec<Want> {
    let mut want = wants(user_dirs);
    want.extend(folder_wants(dirs));
    want
}

pub fn setup(state: &mut State) {
    if let Some(xbel) = reprisefs::recent::xbel_path() {
        watch(state, "recent files", wants(&xbel), State::read_recent);
    }
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => return log::warn!("document folders watch: inotify unavailable: {e}"),
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("document folders watch: no fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, |_, _, state| {
        let changed = state
            .recent
            .folder_watch
            .as_mut()
            .is_some_and(|w| w.drain());
        follow_document_dirs(state);
        if changed {
            state.check_recent();
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("document folders watch: {}", e.error);
    }
    state.recent.folder_watch = Some(watch);
    follow_document_dirs(state);
}

/// Watches `user-dirs.dirs` and the document folders the last recent check read from it. The
/// folders are looked up on a file job, one at a time, since one may sit on a dead mount.
pub fn follow_document_dirs(state: &mut State) {
    if state.recent.folder_syncing {
        state.recent.folder_resync = true;
        return;
    }
    let want = document_wants(
        &reprisefs::places::user_dirs_file(),
        &state.recent.document_dir_paths(),
    );
    let Some(adds) = state.recent.folder_watch.as_mut().map(|w| w.plan(&want)) else {
        return;
    };
    if adds.is_empty() {
        return;
    }
    state.recent.folder_syncing = true;
    state.spawn_fs_job(move || {
        let added = adds.run();
        Box::new(move |s: &mut State| {
            s.recent.folder_syncing = false;
            if let Some(w) = s.recent.folder_watch.as_mut() {
                w.adopt(added);
            }
            if std::mem::take(&mut s.recent.folder_resync) {
                follow_document_dirs(s);
            }
        })
    });
}

/// Runs `hit` on the loop whenever an event `want` asks for arrives.
fn watch(state: &mut State, what: &str, want: Vec<Want>, hit: fn(&mut State)) {
    let mut watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("{what} watch: inotify unavailable: {e}");
            return;
        }
    };
    watch.sync(&want);
    let source = Generic::new(Watch(watch), Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, move |_, watch, state| {
        // SAFETY: the watch is not dropped here, only drained and
        // resynced; the source keeps its fd registered for the watch's
        // life.
        let watch = unsafe { &mut watch.get_mut().0 };
        let changed = watch.drain();
        watch.sync(&want);
        if changed {
            hit(state);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("{what} watch: {}", e.error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replaced_xbel_wakes_the_watch_and_a_neighbour_does_not() {
        let dir = std::env::temp_dir().join(format!("reprise-recentwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let xbel = dir.join("recently-used.xbel");
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&xbel));
        std::fs::write(dir.join("other.txt"), "").unwrap();
        assert!(!w.drain(), "another file in the folder");
        std::fs::write(dir.join("recently-used.xbel.TMP"), "<xbel/>").unwrap();
        std::fs::rename(dir.join("recently-used.xbel.TMP"), &xbel).unwrap();
        assert!(w.drain(), "the xbel replaced by a rename");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edited_user_dirs_wakes_the_watch_and_the_folders_follow_the_last_read() {
        let home =
            std::env::temp_dir().join(format!("reprise-userdirswatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join(".config")).unwrap();
        let user_dirs = home.join(".config/user-dirs.dirs");
        let mut w = DirWatch::new().unwrap();
        w.sync(&document_wants(&user_dirs, &[home.join("Documents")]));
        std::fs::write(home.join(".config/other.conf"), "").unwrap();
        assert!(!w.drain(), "another file in the config folder");
        std::fs::write(home.join(".config/user-dirs.dirs.tmp"), "").unwrap();
        std::fs::rename(home.join(".config/user-dirs.dirs.tmp"), &user_dirs).unwrap();
        assert!(w.drain(), "user-dirs.dirs replaced by a rename");

        w.sync(&document_wants(&user_dirs, &[home.join("Papers")]));
        std::fs::create_dir(home.join("Documents")).unwrap();
        assert!(!w.drain(), "Documents is no document folder any more");
        std::fs::create_dir(home.join("Papers")).unwrap();
        assert!(w.drain(), "Papers made");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_document_folder_coming_or_going_wakes_the_watch_and_a_file_beside_it_does_not() {
        let home = std::env::temp_dir().join(format!("reprise-folderwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Music")).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&folder_wants(&[home.join("Music"), home.join("Videos")]));
        std::fs::write(home.join("notes.txt"), "").unwrap();
        assert!(!w.drain(), "a file beside the folders");
        std::fs::remove_dir(home.join("Music")).unwrap();
        assert!(w.drain(), "Music deleted");
        std::fs::create_dir(home.join("Videos")).unwrap();
        assert!(w.drain(), "Videos made");
        let _ = std::fs::remove_dir_all(&home);
    }
}
