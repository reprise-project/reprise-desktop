//! Re-reads the session proxy file when Settings writes it, so programs started afterwards get
//! its variables ([`crate::spawn::SessionEnv::set_proxy`]).

use crate::compositor::State;
use crate::dirwatch::{DirWatch, Want};
use inotify::WatchMask;
use smithay::reexports::calloop::{Interest, Mode, PostAction, generic::Generic};
use std::ffi::OsString;
use std::os::fd::BorrowedFd;
use std::path::Path;

/// The file's folder, and its parent for a folder made after the start.
fn wants(file: &Path) -> Vec<Want> {
    let mask = WatchMask::CLOSE_WRITE
        | WatchMask::MOVED_TO
        | WatchMask::MOVED_FROM
        | WatchMask::DELETE
        | WatchMask::CREATE;
    let named = |p: &Path| p.file_name().map(OsString::from).into_iter().collect();
    let Some(dir) = file.parent() else {
        return Vec::new();
    };
    let mut out = vec![Want {
        dir: dir.to_path_buf(),
        mask,
        only: named(file),
    }];
    if let Some(parent) = dir.parent() {
        out.push(Want {
            dir: parent.to_path_buf(),
            mask,
            only: named(dir),
        });
    }
    out
}

pub fn setup(state: &mut State) {
    let watch = match DirWatch::new() {
        Ok(w) => w,
        Err(e) => return log::warn!("proxy watch: inotify unavailable: {e}"),
    };
    // SAFETY: the fd is the watch's inotify fd, open for this call.
    let fd = match unsafe { BorrowedFd::borrow_raw(watch.fd()) }.try_clone_to_owned() {
        Ok(fd) => fd,
        Err(e) => return log::warn!("proxy watch: no fd: {e}"),
    };
    let source = Generic::new(fd, Interest::READ, Mode::Level);
    let added = state.handle.insert_source(source, move |_, _, state| {
        state.proxy_watch.drain();
        sync(state);
        Ok(PostAction::Continue)
    });
    if let Err(e) = added {
        return log::warn!("proxy watch: {}", e.error);
    }
    state.proxy_watch.watch = Some(watch);
    sync(state);
}

/// Watches again on a file job and reads the file there; a file written between the start's
/// read and the first watch is read here too.
fn sync(state: &mut State) {
    let Some(job) = state.proxy_watch.start() else {
        return;
    };
    state.spawn_fs_job(move || {
        let file = reprisefs::proxy::path();
        let want = wants(&file);
        let added = job.adder.run(&want);
        let vars = reprisefs::proxy::read(&file);
        Box::new(move |state: &mut State| {
            if state.proxy_watch.landed(&want, added) {
                sync(state);
            }
            if state.env.set_proxy(vars) {
                log::info!("session proxy changed; programs started from now on get it");
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_and_a_removed_file_are_seen_and_a_neighbour_is_not() {
        let dir = std::env::temp_dir().join(format!("reprise-proxywatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("environment.d").join(reprisefs::proxy::FILE);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&file));
        std::fs::create_dir(file.parent().unwrap()).unwrap();
        assert!(w.drain(), "the folder made");
        w.sync(&wants(&file));
        reprisefs::proxy::write(&file, &[("http_proxy", "http://a:1".into())]).unwrap();
        assert!(w.drain(), "the file written");
        std::fs::write(file.with_file_name("10-other.conf"), "A=1\n").unwrap();
        assert!(!w.drain(), "another file in the folder");
        reprisefs::proxy::write(&file, &[]).unwrap();
        assert!(w.drain(), "the file removed");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
