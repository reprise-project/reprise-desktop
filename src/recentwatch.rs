//! Reads the recent files again when `recently-used.xbel` changes, and checks the document
//! folders again when one comes or goes, so open lists follow them.

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

/// The xbel's directory, waking only for the xbel: GLib and KDE replace it by a rename.
fn wants(xbel: &Path) -> Vec<Want> {
    let (Some(dir), Some(name)) = (xbel.parent(), xbel.file_name()) else {
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

pub fn setup(state: &mut State) {
    if let Some(xbel) = reprisefs::recent::xbel_path() {
        watch(state, "recent files", wants(&xbel), State::read_recent);
    }
    // user-dirs.dirs is read off the loop, as the checks read it.
    state.spawn_fs_job(|| {
        let dirs: Vec<PathBuf> = reprisefs::places::document_dir_paths()
            .into_iter()
            .map(|(d, _)| d)
            .collect();
        Box::new(move |state: &mut State| {
            watch(
                state,
                "document folders",
                folder_wants(&dirs),
                State::check_recent,
            )
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
