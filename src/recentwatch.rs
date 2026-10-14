//! Reads the recent files again when `recently-used.xbel` changes, so open lists take a file
//! opened meanwhile.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::Path;

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

pub fn setup(state: &mut State) {
    let Some(xbel) = reprisefs::recent::xbel_path() else {
        return;
    };
    let mut watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("recent files watch: inotify unavailable: {e}");
            return;
        }
    };
    watch.sync(&wants(&xbel));
    let source = Generic::new(Watch(watch), Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, move |_, watch, state| {
        // SAFETY: the watch is not dropped here, only drained and
        // resynced; the source keeps its fd registered for the watch's
        // life.
        let watch = unsafe { &mut watch.get_mut().0 };
        let hit = watch.drain();
        watch.sync(&wants(&xbel));
        if hit {
            state.read_recent();
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("recent files watch: {}", e.error);
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
}
