//! Re-reads the Quick Launch folder when something changes it under the compositor.

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

fn wants(dir: &Path) -> Vec<Want> {
    vec![Want {
        dir: dir.to_path_buf(),
        mask: WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE,
        only: Vec::new(),
    }]
}

pub fn setup(state: &mut State) {
    let Some(dir) = reprisefs::quicklaunch::dir() else {
        return;
    };
    let mut watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => {
            log::warn!("quick launch watch: inotify unavailable: {e}");
            return;
        }
    };
    watch.sync(&wants(&dir));
    let source = Generic::new(Watch(watch), Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, move |_, watch, state| {
        // SAFETY: the watch is not dropped here, only drained and
        // resynced; the source keeps its fd registered for the watch's
        // life.
        let watch = unsafe { &mut watch.get_mut().0 };
        let hit = watch.drain();
        watch.sync(&wants(&dir));
        if hit {
            state.quick_reread(&dir);
        }
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        log::warn!("quick launch watch: {}", e.error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_button_and_a_new_order_are_seen() {
        let dir = std::env::temp_dir().join(format!("reprise-quickwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&dir));
        std::fs::write(dir.join("a.desktop"), "x").unwrap();
        assert!(w.drain(), "a written button");
        reprisefs::quicklaunch::write_order(&dir, &["a.desktop".into()]).unwrap();
        assert!(w.drain(), "a renamed order file");
        std::fs::remove_file(dir.join("a.desktop")).unwrap();
        assert!(w.drain(), "a deleted button");
        assert!(!w.drain(), "nothing more");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
