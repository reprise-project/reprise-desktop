//! `config.toml` under inotify, so the sheet rereads what the compositor (a key aid's shortcut)
//! or an editor wrote while it was open.

use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use inotify::{Inotify, WatchMask};

/// Watch `path` on a thread: each write or rename onto it sets `changed` and calls `wake`.
pub fn spawn(path: &Path, changed: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) {
    let path = path.to_path_buf();
    let run = move || {
        if let Err(e) = watch(&path, &changed, &wake) {
            log::warn!("{} is not watched: {e}", path.display());
        }
    };
    if let Err(e) = std::thread::Builder::new()
        .name("reprise-settings-config".into())
        .spawn(run)
    {
        log::warn!("config watch thread: {e}");
    }
}

fn watch(path: &Path, changed: &AtomicBool, wake: &dyn Fn()) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(());
    };
    let mut inotify = Inotify::init()?;
    // Writers replace the file by renaming a temporary over it, so the folder is watched.
    inotify
        .watches()
        .add(dir, WatchMask::CLOSE_WRITE | WatchMask::MOVED_TO)?;
    let mut buf = [0u8; 4096];
    loop {
        let events = match inotify.read_events_blocking(&mut buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            r => r?,
        };
        if events.into_iter().any(|e| e.name == Some(name)) {
            changed.store(true, Ordering::Release);
            wake();
        }
    }
}
