//! Re-reads the Quick Launch folder when something changes it under the compositor.

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

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
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => return log::warn!("quick launch watch: inotify unavailable: {e}"),
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("quick launch watch: no fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let folder = dir.clone();
    let added = state.handle.insert_source(source, move |_, _, state| {
        state.quick_watch.drain();
        sync(state, folder.clone());
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("quick launch watch: {}", e.error);
    }
    state.quick_watch.watch = Some(watch);
    sync(state, dir);
}

/// Watches `dir` again on a file job (the folder may sit on a mount that stopped answering),
/// then reads it when an event came.
fn sync(state: &mut State, dir: PathBuf) {
    let Some(job) = state.quick_watch.start() else {
        return;
    };
    let hit = job.hit;
    state.spawn_fs_job(move || {
        let want = wants(&dir);
        let added = job.adder.run(&want);
        Box::new(move |state: &mut State| {
            if state.quick_watch.landed(&want, added) {
                sync(state, dir.clone());
            }
            if hit {
                state.quick_reread(&dir);
            }
        })
    });
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
