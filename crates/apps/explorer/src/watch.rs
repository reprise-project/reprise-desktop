//! The folder watch: inotify on the folder the view shows, so a file written by something else
//! appears without an F5.
//!
//! The thread owns the `Inotify`; the window keeps the [`Watches`] handle, so the watch moves with
//! the view while the thread sits in its read. The thread is detached on purpose: Explorer keeps
//! one [`Watch`] for the life of the process.

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use repriseapp::Waker;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

/// What the thread and the window share: the watches that count only events naming one entry (with
/// that entry), and the folder [`Watch::watch`] shows.
#[derive(Default)]
struct Shared {
    only: Vec<(WatchDescriptor, OsString)>,
    shown: Option<Shown>,
}

/// The shown folder's own watch and its parent's, which names it when it is renamed in place.
struct Shown {
    dir: PathBuf,
    wd: WatchDescriptor,
    parent: Option<WatchDescriptor>,
    /// The cookie of the parent's MOVED_FROM naming the folder, to pair with its MOVED_TO.
    leaving: Option<u32>,
    /// Set once the folder left: its new path after a rename in place, else its old path.
    gone: Option<PathBuf>,
}

impl Shared {
    /// Take in one event; true when it counts as a change.
    fn note(&mut self, e: &inotify::Event<&std::ffi::OsStr>) -> bool {
        // IGNORED is the kernel's answer to our own `remove` in [`Watch::clear`] (it comes
        // whatever the mask says). Taking it for a change would refresh, which re-watches,
        // which reports IGNORED again: a busy loop.
        if e.mask.contains(EventMask::IGNORED) {
            return false;
        }
        if let Some(s) = self.shown.as_mut() {
            if s.parent.as_ref() == Some(&e.wd) {
                let ours = e.name == s.dir.file_name();
                if e.mask.contains(EventMask::MOVED_FROM) && ours {
                    s.leaving = Some(e.cookie);
                } else if e.mask.contains(EventMask::MOVED_TO)
                    && s.leaving == Some(e.cookie)
                    && let Some(name) = e.name
                {
                    s.gone.get_or_insert_with(|| s.dir.with_file_name(name));
                }
                return false;
            }
            if e.wd == s.wd
                && e.mask
                    .intersects(EventMask::MOVE_SELF | EventMask::DELETE_SELF)
            {
                s.gone.get_or_insert_with(|| s.dir.clone());
                return true;
            }
        }
        self.only
            .iter()
            .find(|(wd, _)| *wd == e.wd)
            .is_none_or(|(_, name)| e.name == Some(name.as_os_str()))
    }
}

pub struct Watch {
    watches: Watches,
    wd: Vec<WatchDescriptor>,
    shared: Arc<Mutex<Shared>>,
    changed: Arc<AtomicBool>,
}

impl Watch {
    /// The inotify instance and the thread that reads it.
    pub fn new(wake: Waker) -> io::Result<Watch> {
        let mut inotify = Inotify::init()?;
        let watches = inotify.watches();
        let changed = Arc::new(AtomicBool::new(false));
        let flag = changed.clone();
        let shared: Arc<Mutex<Shared>> = Arc::default();
        let seen = shared.clone();
        std::thread::Builder::new()
            .name("watch".to_string())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                let mut last_wake: Option<Instant> = None;
                let note = |events: inotify::Events| {
                    let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
                    events.fold(false, |any, e| seen.note(&e) | any)
                };
                loop {
                    match inotify.read_events_blocking(&mut buf) {
                        Ok(events) => {
                            if !note(events) {
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
                    // The rest of the burst is one change; the fd is non-blocking.
                    if let Ok(events) = inotify.read_events(&mut buf) {
                        note(events);
                    }
                    flag.store(true, Ordering::SeqCst);
                    wake.wake();
                    last_wake = Some(Instant::now());
                }
            })?;
        Ok(Watch {
            watches,
            wd: Vec::new(),
            shared,
            changed,
        })
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Watch `dir` instead of whatever was being watched, and notice it leaving ([`Watch::take_gone`]).
    pub fn watch(&mut self, dir: &Path) {
        self.clear();
        let Some(wd) = self.add(dir, mask() | WatchMask::MOVE_SELF | WatchMask::DELETE_SELF) else {
            return;
        };
        let parent = dir
            .parent()
            .and_then(|p| self.add(p, WatchMask::MOVED_FROM | WatchMask::MOVED_TO));
        self.shared().shown = Some(Shown {
            dir: dir.to_path_buf(),
            wd,
            parent,
            leaving: None,
            gone: None,
        });
    }

    /// Watch every one of `dirs` (the Recycle Bin's folders, one per drive, and the folders above
    /// the home trash) instead of whatever was being watched. A folder paired with a name counts
    /// only events naming that entry.
    pub fn watch_all(&mut self, dirs: &[(PathBuf, Option<OsString>)]) {
        self.clear();
        for (dir, name) in dirs {
            if let Some(wd) = self.add(dir, mask())
                && let Some(name) = name
            {
                self.shared().only.push((wd, name.clone()));
            }
        }
    }

    /// A folder that cannot be watched is logged and left unwatched; F5 still refreshes it.
    fn add(&mut self, dir: &Path, mask: WatchMask) -> Option<WatchDescriptor> {
        match self.watches.add(dir, mask) {
            Ok(wd) => {
                self.wd.push(wd.clone());
                Some(wd)
            }
            Err(e) => {
                log::info!("cannot watch {}: {e}", dir.display());
                None
            }
        }
    }

    /// Stop watching anything (Computer and the other places that are not a folder on disk).
    pub fn clear(&mut self) {
        *self.shared() = Shared::default();
        for wd in self.wd.drain(..) {
            let _ = self.watches.remove(wd);
        }
    }

    /// Whether the folder changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        self.changed.swap(false, Ordering::SeqCst)
    }

    /// Once the folder [`Watch::watch`] shows was deleted or moved away: its new path when it was
    /// renamed in place, else its old one.
    pub fn take_gone(&mut self) -> Option<PathBuf> {
        self.shared().shown.as_mut()?.gone.take()
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

    /// A folder watched for one entry wakes for that entry only.
    #[test]
    fn a_named_watch_counts_only_its_entry() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch_all(&[(dir.path().to_path_buf(), Some("Trash".into()))]);
        std::fs::write(dir.path().join("recently-used.xbel"), "x").expect("write");
        std::thread::sleep(COALESCE * 4);
        assert!(!watch.take_changed(), "another entry is not a change");

        std::fs::create_dir(dir.path().join("Trash")).expect("mkdir");
        std::thread::sleep(COALESCE * 4);
        assert!(watch.take_changed(), "the named entry is");
    }

    #[test]
    fn a_folder_that_is_not_there_is_logged_not_fatal() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        watch.watch(Path::new("/definitely/not/a/folder"));
        assert!(watch.wd.is_empty());
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch(dir.path());
        assert_eq!(watch.wd.len(), 2, "the folder and its parent");
        watch.clear();
        assert!(watch.wd.is_empty());
    }

    fn wait() {
        std::thread::sleep(COALESCE * 4);
    }

    #[test]
    fn a_folder_renamed_in_place_is_gone_to_its_new_name() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let top = tempfile::tempdir().expect("tempdir");
        let (old, new) = (top.path().join("old"), top.path().join("new"));
        std::fs::create_dir(&old).expect("mkdir");
        watch.watch(&old);
        std::fs::write(top.path().join("other.txt"), "x").expect("write");
        std::fs::rename(top.path().join("other.txt"), top.path().join("moved.txt")).expect("mv");
        wait();
        assert_eq!(
            watch.take_gone(),
            None,
            "another entry moving in the parent"
        );
        assert!(
            !watch.take_changed(),
            "the parent's events are not a change"
        );

        std::fs::rename(&old, &new).expect("rename");
        wait();
        assert_eq!(watch.take_gone(), Some(new));
        assert_eq!(watch.take_gone(), None, "taken once");
    }

    #[test]
    fn a_folder_moved_elsewhere_or_deleted_is_gone_from_its_old_path() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let top = tempfile::tempdir().expect("tempdir");
        let shown = top.path().join("a").join("shown");
        std::fs::create_dir_all(&shown).expect("mkdir");
        std::fs::create_dir(top.path().join("b")).expect("mkdir");
        watch.watch(&shown);
        std::fs::rename(&shown, top.path().join("b").join("shown")).expect("mv");
        wait();
        assert_eq!(
            watch.take_gone(),
            Some(shown.clone()),
            "moved to another folder"
        );

        std::fs::create_dir(&shown).expect("mkdir");
        watch.watch(&shown);
        std::fs::remove_dir(&shown).expect("rmdir");
        wait();
        assert_eq!(watch.take_gone(), Some(shown), "deleted");
    }
}
