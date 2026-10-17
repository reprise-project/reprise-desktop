//! `config.toml` and the slideshow folder under inotify, so the sheet rereads what the compositor
//! (a key aid's shortcut) or an editor wrote while it was open, and relists the folder.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};

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

/// One folder at a time under inotify on a thread: a file in it written, moved or removed sets
/// `changed` and calls `wake`.
pub struct FolderWatch {
    watches: Watches,
    watched: Option<(PathBuf, WatchDescriptor)>,
}

impl FolderWatch {
    pub fn spawn(changed: Arc<AtomicBool>, wake: impl Fn() + Send + 'static) -> io::Result<Self> {
        let mut inotify = Inotify::init()?;
        let watches = inotify.watches();
        std::thread::Builder::new()
            .name("reprise-settings-folder".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    let events = match inotify.read_events_blocking(&mut buf) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => return log::warn!("folder watch: {e}"),
                        Ok(events) => events,
                    };
                    // A watch dropped by `follow` reports IGNORED; nothing in a folder changed.
                    if events
                        .into_iter()
                        .any(|e| !e.mask.contains(EventMask::IGNORED))
                    {
                        changed.store(true, Ordering::Release);
                        wake();
                    }
                }
            })?;
        Ok(Self {
            watches,
            watched: None,
        })
    }

    /// Watch `folder` in place of the one watched before.
    pub fn follow(&mut self, folder: &Path) {
        if self.watched.as_ref().is_some_and(|(f, _)| f == folder) {
            return;
        }
        if let Some((_, wd)) = self.watched.take() {
            let _ = self.watches.remove(wd);
        }
        let mask = WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE
            | WatchMask::ONLYDIR;
        match self.watches.add(folder, mask) {
            Ok(wd) => self.watched = Some((folder.to_path_buf(), wd)),
            Err(e) => log::warn!("{} is not watched: {e}", folder.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn settles(changed: &AtomicBool) -> bool {
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if changed.swap(false, Ordering::AcqRel) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn a_picture_written_into_the_followed_folder_is_reported_and_the_one_left_is_not() {
        let dir = std::env::temp_dir().join(format!("reprise-folderwatch-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let changed = Arc::new(AtomicBool::new(false));
        let mut watch = FolderWatch::spawn(changed.clone(), || {}).unwrap();

        watch.follow(&a);
        std::fs::write(a.join("one.png"), b"x").unwrap();
        assert!(settles(&changed), "a write in the followed folder");

        watch.follow(&b);
        // The IGNORED for the dropped watch on a reports nothing.
        std::thread::sleep(Duration::from_millis(50));
        changed.store(false, Ordering::Release);
        std::fs::write(a.join("two.png"), b"x").unwrap();
        assert!(!settles(&changed), "a write in the folder left");
        std::fs::rename(a.join("two.png"), b.join("two.png")).unwrap();
        assert!(
            settles(&changed),
            "a picture moved into the followed folder"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
