//! The folder watch: inotify on the folder the view shows, so a file written by something else
//! appears without an F5.
//!
//! The thread owns the `Inotify`; the window keeps the [`Watches`] handle, so the watch moves with
//! the view while the thread sits in its read. The thread ends, freeing its inotify instance, once
//! the [`Watch`] drops.

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
    #[cfg(test)]
    fence: Fence,
}

/// A watch tests use to learn the thread has dealt with every event before it ([`Watch::sync`]).
#[cfg(test)]
#[derive(Default)]
struct Fence {
    wd: Option<WatchDescriptor>,
    read: bool,
    passed: bool,
}

/// The shown folder's watch and one on every folder above it, which name the folder on the path
/// when it is renamed or moved and report their own move.
struct Shown {
    dir: PathBuf,
    /// `levels[k]` watches `dir.ancestors().nth(k)`; `None` where the folder could not be watched.
    levels: Vec<Option<WatchDescriptor>>,
    /// The cookie and level of a MOVED_FROM naming the folder on the path, to pair with its
    /// MOVED_TO.
    leaving: Option<(u32, usize)>,
    /// Set once the folder or one above it left: the folder's new path when the move landed in a
    /// watched level, else its old path.
    gone: Option<PathBuf>,
}

impl Shown {
    /// Take in an event on `levels[k]`; true when it counts as a change.
    fn note(&mut self, k: usize, e: &inotify::Event<&std::ffi::OsStr>) -> bool {
        if e.mask
            .intersects(EventMask::MOVE_SELF | EventMask::DELETE_SELF)
        {
            self.gone.get_or_insert_with(|| self.dir.clone());
            return true;
        }
        if k == 0 {
            return true;
        }
        let path: Vec<&Path> = self.dir.ancestors().collect();
        if e.mask.contains(EventMask::MOVED_FROM) && e.name == path[k - 1].file_name() {
            self.leaving = Some((e.cookie, k));
        } else if e.mask.contains(EventMask::MOVED_TO)
            && let Some((cookie, from)) = self.leaving
            && cookie == e.cookie
            && k >= from
            && let Some(name) = e.name
        {
            let mut to = path[k].join(name);
            if let Ok(rest) = self.dir.strip_prefix(path[from - 1])
                && !rest.as_os_str().is_empty()
            {
                to.push(rest);
            }
            self.gone.get_or_insert(to);
        }
        false
    }
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
        #[cfg(test)]
        if self.fence.wd.as_ref() == Some(&e.wd) {
            self.fence.read = true;
            return false;
        }
        if let Some(s) = self.shown.as_mut()
            && let Some(k) = s.levels.iter().position(|w| w.as_ref() == Some(&e.wd))
        {
            return s.note(k, e);
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
        let flag = Arc::downgrade(&changed);
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
                    #[cfg(test)]
                    {
                        let mut seen = seen.lock().unwrap_or_else(|e| e.into_inner());
                        let fence = &mut seen.fence;
                        fence.passed |= std::mem::take(&mut fence.read);
                    }
                    match inotify.read_events_blocking(&mut buf) {
                        Ok(events) => {
                            if flag.strong_count() == 0 {
                                return;
                            }
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
                    let Some(flag) = flag.upgrade() else {
                        return;
                    };
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

    /// Watch `dir` instead of whatever was being watched, and notice it or a folder above it
    /// leaving ([`Watch::take_gone`]). Each level up costs one inotify watch.
    pub fn watch(&mut self, dir: &Path) {
        self.clear();
        let Some(wd) = self.add(dir, mask() | WatchMask::MOVE_SELF | WatchMask::DELETE_SELF) else {
            return;
        };
        let mut levels = vec![Some(wd)];
        for up in dir.ancestors().skip(1) {
            let mask = WatchMask::MOVED_FROM
                | WatchMask::MOVED_TO
                | WatchMask::MOVE_SELF
                | WatchMask::DELETE_SELF;
            levels.push(match self.watches.add(up, mask) {
                Ok(wd) => {
                    self.wd.push(wd.clone());
                    Some(wd)
                }
                Err(e) => {
                    log::debug!("cannot watch {}: {e}", up.display());
                    None
                }
            });
        }
        self.shared().shown = Some(Shown {
            dir: dir.to_path_buf(),
            levels,
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

    /// Once the folder [`Watch::watch`] shows, or one above it, was deleted or moved away: the
    /// folder's new path when the move stayed within the watched levels, else its old one.
    pub fn take_gone(&mut self) -> Option<PathBuf> {
        self.shared().shown.as_mut()?.gone.take()
    }

    #[cfg(test)]
    fn flag(&self) -> Arc<AtomicBool> {
        self.changed.clone()
    }

    /// Returns once the thread has finished with every event queued before the call, its settle
    /// and flag included: events on one fd arrive in order, so it waits for a fence event's round.
    #[cfg(test)]
    fn sync(&mut self) {
        let dir = tempfile::tempdir().expect("tempdir");
        let wd = self
            .watches
            .add(dir.path(), WatchMask::CREATE)
            .expect("fence watch");
        self.shared().fence = Fence {
            wd: Some(wd.clone()),
            ..Fence::default()
        };
        std::fs::write(dir.path().join("fence"), "").expect("fence");
        let until = Instant::now() + Duration::from_secs(30);
        while !self.shared().fence.passed {
            assert!(
                Instant::now() < until,
                "the watch thread never passed the fence"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        self.shared().fence = Fence::default();
        let _ = self.watches.remove(wd);
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // The thread holds a Weak of the flag. Taking a watch off queues IN_IGNORED, which wakes it
        // to find the flag gone.
        self.changed = Arc::default();
        if let Ok(wd) = self.watches.add("/", WatchMask::DELETE_SELF) {
            let _ = self.watches.remove(wd);
        }
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
        watch.sync();
        assert!(
            !watch.take_changed(),
            "our own rm_watch was taken for a change"
        );

        std::fs::write(dir.path().join("new.txt"), "x").expect("write");
        watch.sync();
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
        watch.sync();
        assert!(watch.take_changed(), "the file arriving is a change");
        for _ in 0..3 {
            f.write_all(&[0; 4096]).expect("write");
        }
        watch.sync();
        assert!(!watch.take_changed(), "its writes are not");
        drop(f);
        watch.sync();
        assert!(watch.take_changed(), "its close is");
    }

    /// A folder watched for one entry wakes for that entry only.
    #[test]
    fn a_named_watch_counts_only_its_entry() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch_all(&[(dir.path().to_path_buf(), Some("Trash".into()))]);
        std::fs::write(dir.path().join("recently-used.xbel"), "x").expect("write");
        watch.sync();
        assert!(!watch.take_changed(), "another entry is not a change");

        std::fs::create_dir(dir.path().join("Trash")).expect("mkdir");
        watch.sync();
        assert!(watch.take_changed(), "the named entry is");
    }

    /// Each window's watch is an inotify instance, and a user gets max_user_instances of them.
    #[test]
    fn a_dropped_watch_frees_its_inotify_instance() {
        let limit: usize = std::fs::read_to_string("/proc/sys/fs/inotify/max_user_instances")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(128);
        let dir = tempfile::tempdir().expect("tempdir");
        for _ in 0..limit + 1 {
            let mut watch = Watch::new(Waker::noop()).expect("inotify");
            watch.watch(dir.path());
        }
    }

    #[test]
    fn a_folder_that_is_not_there_is_logged_not_fatal() {
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        watch.watch(Path::new("/definitely/not/a/folder"));
        assert!(watch.wd.is_empty());
        let dir = tempfile::tempdir().expect("tempdir");
        watch.watch(dir.path());
        assert_eq!(
            watch.wd.len(),
            dir.path().ancestors().count(),
            "the folder and every folder above it"
        );
        watch.clear();
        assert!(watch.wd.is_empty());
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
        watch.sync();
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
        watch.sync();
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
        watch.sync();
        assert_eq!(
            watch.take_gone(),
            Some(shown.clone()),
            "moved to another folder"
        );

        std::fs::create_dir(&shown).expect("mkdir");
        watch.watch(&shown);
        std::fs::remove_dir(&shown).expect("rmdir");
        watch.sync();
        assert_eq!(watch.take_gone(), Some(shown), "deleted");
    }
    /// `top/a/b/shown`, watched at `shown`.
    fn deep() -> (tempfile::TempDir, Watch, PathBuf) {
        let top = tempfile::tempdir().expect("tempdir");
        let shown = top.path().join("a").join("b").join("shown");
        std::fs::create_dir_all(&shown).expect("mkdir");
        let mut watch = Watch::new(Waker::noop()).expect("inotify");
        watch.watch(&shown);
        (top, watch, shown)
    }

    #[test]
    fn a_folder_above_renamed_in_place_is_followed_to_the_new_path() {
        let (top, mut watch, _) = deep();
        let a = top.path().join("a");
        std::fs::create_dir(a.join("c")).expect("mkdir");
        std::fs::rename(a.join("c"), a.join("d")).expect("mv");
        watch.sync();
        assert_eq!(watch.take_gone(), None, "a sibling renamed");
        assert!(!watch.take_changed(), "events above are not a change");

        std::fs::rename(&a, top.path().join("z")).expect("rename");
        watch.sync();
        assert_eq!(
            watch.take_gone(),
            Some(top.path().join("z").join("b").join("shown"))
        );
    }

    #[test]
    fn a_folder_above_moved_up_the_path_is_followed() {
        let (top, mut watch, _) = deep();
        std::fs::rename(top.path().join("a").join("b"), top.path().join("b2")).expect("mv");
        watch.sync();
        assert_eq!(watch.take_gone(), Some(top.path().join("b2").join("shown")));
    }

    #[test]
    fn a_folder_above_moved_off_the_path_or_deleted_is_gone_from_the_old_path() {
        let (top, mut watch, shown) = deep();
        let away = tempfile::tempdir().expect("tempdir");
        std::fs::rename(top.path().join("a"), away.path().join("a")).expect("mv");
        watch.sync();
        assert_eq!(watch.take_gone(), Some(shown.clone()), "moved elsewhere");

        std::fs::create_dir_all(&shown).expect("mkdir");
        watch.watch(&shown);
        std::fs::remove_dir_all(top.path().join("a")).expect("rm");
        watch.sync();
        assert_eq!(watch.take_gone(), Some(shown), "deleted");
    }
}
