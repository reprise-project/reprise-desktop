//! The folder watch: inotify on the folder the view shows, so a file written by something else
//! appears without an F5.
//!
//! The thread owns the `Inotify`; the window keeps the [`Watches`] handle, so the watch moves with
//! the view while the thread sits in its read. The thread is detached on purpose: Explorer keeps
//! one [`Watch`] for the life of the process.

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use repriseapp::Waker;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long the thread waits after the first event, so a burst of writes costs one refresh.
const COALESCE: Duration = Duration::from_millis(100);

/// The shortest time between two wakes: a folder that never settles re-lists at this pace.
const MIN_GAP: Duration = Duration::from_secs(1);

/// How long to wait before waking for an event, given the time since the last wake (`None` before
/// the first).
fn settle(since_wake: Option<Duration>) -> Duration {
    since_wake.map_or(COALESCE, |t| COALESCE.max(MIN_GAP.saturating_sub(t)))
}

/// What counts as a change to the folder: something arriving, leaving, being renamed either way,
/// changing stamp or attributes, or finishing a write. Not MODIFY: a long download fires it per
/// write.
fn mask() -> WatchMask {
    WatchMask::CREATE
        | WatchMask::DELETE
        | WatchMask::MOVED_FROM
        | WatchMask::MOVED_TO
        | WatchMask::ATTRIB
        | WatchMask::CLOSE_WRITE
}

pub struct Watch {
    watches: Watches,
    wd: Vec<WatchDescriptor>,
    changed: Arc<AtomicBool>,
}

impl Watch {
    /// The inotify instance and the thread that reads it.
    pub fn new(wake: Waker) -> io::Result<Watch> {
        let mut inotify = Inotify::init()?;
        let watches = inotify.watches();
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        std::thread::Builder::new()
            .name("watch".to_string())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                let mut last_wake: Option<Instant> = None;
                loop {
                    match inotify.read_events_blocking(&mut buf) {
                        Ok(mut events) => {
                            // IGNORED is the kernel's answer to our own
                            // `remove` in [`Watch::watch`] (it comes
                            // whatever the mask says). Taking it for a
                            // change would refresh, which re-watches,
                            // which reports IGNORED again: a busy loop.
                            if events.all(|e| e.mask.contains(EventMask::IGNORED)) {
                                continue;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            log::warn!("folder watch stopped: {e}");
                            return;
                        }
                    }
                    std::thread::sleep(settle(last_wake.map(|t| t.elapsed())));
                    // Swallow the rest of the burst; the fd is non-blocking.
                    let _ = inotify.read_events(&mut buf);
                    flag.store(true, Ordering::SeqCst);
                    wake.wake();
                    last_wake = Some(Instant::now());
                }
            })?;
        Ok(Watch {
            watches,
            wd: Vec::new(),
            changed,
        })
    }

    /// Watch `dir` instead of whatever was being watched.
    pub fn watch(&mut self, dir: &Path) {
        self.watch_all(std::slice::from_ref(&dir));
    }

    /// Watch every one of `dirs` (the Recycle Bin's folders, one per drive) instead of whatever was
    /// being watched.
    /// A folder that cannot be watched is logged and left unwatched; F5 still refreshes it.
    pub fn watch_all(&mut self, dirs: &[impl AsRef<Path>]) {
        self.clear();
        for dir in dirs.iter().map(AsRef::as_ref) {
            match self.watches.add(dir, mask()) {
                Ok(wd) => self.wd.push(wd),
                Err(e) => log::info!("cannot watch {}: {e}", dir.display()),
            }
        }
    }

    /// Stop watching anything (Computer and the other places that are not a folder on disk).
    pub fn clear(&mut self) {
        for wd in self.wd.drain(..) {
            let _ = self.watches.remove(wd);
        }
    }

    /// Whether the folder changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }

    #[cfg(test)]
    fn flag(&self) -> Arc<AtomicBool> {
        self.changed.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The thread's half of the coalescing: however many events it sees, it sets one flag, and the
    /// window takes it exactly once.
    #[test]
    fn the_changed_flag_is_set_once_and_taken_once() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        assert!(!watch.take_changed());
        let flag = watch.flag();
        flag.store(true, Ordering::SeqCst);
        flag.store(true, Ordering::SeqCst);
        assert!(watch.take_changed());
        assert!(!watch.take_changed());
    }

    #[test]
    fn a_change_soon_after_a_wake_waits_out_the_gap() {
        assert_eq!(settle(None), COALESCE);
        assert_eq!(settle(Some(MIN_GAP * 2)), COALESCE);
        assert_eq!(settle(Some(COALESCE)), MIN_GAP - COALESCE);
        assert_eq!(settle(Some(MIN_GAP - COALESCE / 2)), COALESCE);
    }

    /// Re-watching (what every listing does) removes the old descriptor, and the kernel reports
    /// that removal as an event on the fd.
    #[test]
    fn rewatching_a_folder_is_not_a_change() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch(dir.path());
        watch.watch(dir.path());
        watch.clear();
        watch.watch(dir.path());
        std::thread::sleep(COALESCE * 4);
        assert!(
            !watch.take_changed(),
            "our own rm_watch was taken for a change"
        );

        std::fs::write(dir.path().join("new.txt"), "x").expect("write");
        std::thread::sleep(COALESCE * 4);
        assert!(watch.take_changed());
    }

    /// A file being written to is not a change until it closes: a download's every write would
    /// otherwise re-list the folder.
    #[test]
    fn writes_to_an_open_file_wait_for_its_close() {
        use std::io::Write;
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch(dir.path());
        let mut f = std::fs::File::create(dir.path().join("big.bin")).expect("create");
        std::thread::sleep(COALESCE * 4);
        assert!(watch.take_changed(), "the file arriving is a change");
        for _ in 0..3 {
            f.write_all(&[0; 4096]).expect("write");
            std::thread::sleep(COALESCE * 2);
        }
        assert!(!watch.take_changed(), "its writes are not");
        drop(f);
        std::thread::sleep(COALESCE * 4);
        assert!(watch.take_changed(), "its close is");
    }

    #[test]
    fn a_folder_that_is_not_there_is_logged_not_fatal() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        watch.watch(Path::new("/definitely/not/a/folder"));
        assert!(watch.wd.is_empty());
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch(dir.path());
        assert_eq!(watch.wd.len(), 1);
        watch.clear();
        assert!(watch.wd.is_empty());
    }
}
