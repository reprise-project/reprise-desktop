//! A set of inotify watches on directories that may come and go; a watch whose directory is deleted or moved away
//! is dropped and the next [`DirWatch::sync`] adds it again.

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;

pub struct Want {
    pub dir: PathBuf,
    pub mask: WatchMask,
    /// Only events naming one of these entries count, so watching `$HOME`
    /// for the desktop directory does not wake on every file written there.
    /// Empty: every event counts.
    pub only: Vec<OsString>,
}

struct Watched {
    dir: PathBuf,
    wd: WatchDescriptor,
    only: Vec<OsString>,
}

pub struct DirWatch {
    ino: Inotify,
    watched: Vec<Watched>,
}

impl DirWatch {
    pub fn new() -> io::Result<DirWatch> {
        Ok(DirWatch {
            ino: Inotify::init()?,
            watched: Vec::new(),
        })
    }

    pub fn fd(&self) -> RawFd {
        self.ino.as_raw_fd()
    }

    /// Make the watched set `want`: drop directories no longer in it, take
    /// each kept one's `only`, and watch every wanted directory that exists
    /// and is not watched yet. Each path must name a different directory,
    /// since inotify gives one inode one watch. `DELETE_SELF | MOVE_SELF` is
    /// added to each mask so the watch notices its directory leaving.
    pub fn sync(&mut self, want: &[Want]) {
        let ino = &mut self.ino;
        self.watched.retain(|x| {
            let keep = want.iter().any(|w| w.dir == x.dir);
            if !keep {
                let _ = ino.watches().remove(x.wd.clone());
            }
            keep
        });
        for w in want {
            if let Some(x) = self.watched.iter_mut().find(|x| x.dir == w.dir) {
                x.only.clone_from(&w.only);
                continue;
            }
            if !w.dir.is_dir() {
                continue;
            }
            let mask = w.mask | WatchMask::DELETE_SELF | WatchMask::MOVE_SELF;
            match self.ino.watches().add(&w.dir, mask) {
                Ok(wd) => self.watched.push(Watched {
                    dir: w.dir.clone(),
                    wd,
                    only: w.only.clone(),
                }),
                Err(e) => log::warn!("not watching {}: {e}", w.dir.display()),
            }
        }
    }

    /// Read every pending event. True when any calls for a rescan: a change
    /// inside a watched directory, a watched directory going away (its
    /// watch is dropped, for `sync` to add again), or a queue overflow
    /// (every watch is dropped).
    pub fn drain(&mut self) -> bool {
        let mut buf = [0u8; 4096];
        let mut changed = false;
        loop {
            let events = match self.ino.read_events(&mut buf) {
                Ok(events) => events,
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => {
                    log::warn!("inotify read failed: {e}");
                    break;
                }
            };
            let mut any = false;
            let mut gone = Vec::new();
            for ev in events {
                any = true;
                if ev.mask.contains(EventMask::Q_OVERFLOW) {
                    gone.extend(self.watched.iter().map(|w| w.wd.clone()));
                    changed = true;
                    continue;
                }
                let Some(w) = self.watched.iter().find(|w| w.wd == ev.wd) else {
                    continue;
                };
                if ev
                    .mask
                    .intersects(EventMask::DELETE_SELF | EventMask::MOVE_SELF | EventMask::IGNORED)
                {
                    gone.push(ev.wd.clone());
                    changed = true;
                } else if w.only.is_empty() || w.only.iter().any(|n| ev.name == Some(n.as_os_str()))
                {
                    changed = true;
                }
            }
            for wd in gone {
                self.watched.retain(|w| w.wd != wd);
                let _ = self.ino.watches().remove(wd);
            }
            if !any {
                break;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn wants(home: &std::path::Path) -> Vec<Want> {
        vec![
            Want {
                dir: home.join("Desktop"),
                mask: WatchMask::CREATE | WatchMask::DELETE | WatchMask::ATTRIB,
                only: Vec::new(),
            },
            Want {
                dir: home.to_path_buf(),
                mask: WatchMask::CREATE | WatchMask::MOVED_TO,
                only: vec!["Desktop".into()],
            },
        ]
    }

    /// A desktop directory made after the watch started is picked up
    /// through its parent, a chmod inside it counts, and after it is
    /// deleted and made again the watch comes back. Files written in the
    /// parent do not count.
    #[test]
    fn the_desktop_watch_follows_its_directory() {
        let home = std::env::temp_dir().join(format!("reprise-dirwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let desk = home.join("Desktop");
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&home));
        assert!(!w.drain(), "no events yet: no rescan");

        std::fs::write(home.join("notes.txt"), "x").unwrap();
        assert!(!w.drain(), "a file in the parent: no rescan");

        std::fs::create_dir(&desk).unwrap();
        assert!(w.drain(), "the desktop directory appearing: rescan");
        w.sync(&wants(&home));

        let file = desk.join("app.desktop");
        std::fs::write(&file, "").unwrap();
        assert!(w.drain(), "a new file on the desktop: rescan");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(w.drain(), "chmod +x on a desktop file: rescan");

        std::fs::remove_dir_all(&desk).unwrap();
        assert!(w.drain(), "the desktop directory deleted: rescan");
        std::fs::create_dir(&desk).unwrap();
        assert!(w.drain(), "the desktop directory made again: rescan");
        w.sync(&wants(&home));
        std::fs::write(desk.join("b.txt"), "").unwrap();
        assert!(w.drain(), "a file in the new desktop directory: rescan");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A desktop directory moved away drops its watch, so a new one at the
    /// same path is watched instead of the moved one.
    #[test]
    fn a_moved_desktop_directory_is_no_longer_watched() {
        let home = std::env::temp_dir().join(format!("reprise-dirwatch-mv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let desk = home.join("Desktop");
        std::fs::create_dir_all(&desk).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&wants(&home));

        let old = home.join("Old");
        std::fs::rename(&desk, &old).unwrap();
        assert!(w.drain(), "the desktop directory moved away: rescan");
        std::fs::write(old.join("a.txt"), "").unwrap();
        assert!(!w.drain(), "a file in the moved directory: no rescan");

        std::fs::create_dir(&desk).unwrap();
        assert!(w.drain(), "a new desktop directory: rescan");
        w.sync(&wants(&home));
        std::fs::write(desk.join("b.txt"), "").unwrap();
        assert!(w.drain(), "a file in the new desktop directory: rescan");

        let _ = std::fs::remove_dir_all(&home);
    }

    /// A full event queue loses events, so an overflow counts as a change
    /// even when none of the events that did arrive named a wanted entry,
    /// and the watch still works after `sync`.
    #[test]
    fn a_queue_overflow_calls_for_a_rescan() {
        let dir = std::env::temp_dir().join(format!("reprise-dirwatch-ovf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = DirWatch::new().unwrap();
        w.sync(&[Want {
            dir: dir.clone(),
            mask: WatchMask::CLOSE_WRITE,
            only: vec!["config.toml".into()],
        }]);
        let max: usize = std::fs::read_to_string("/proc/sys/fs/inotify/max_queued_events")
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // Two names in turn, since the kernel merges an event identical to
        // the one queued before it. Twice the queue, because a child forked
        // by a concurrent test can hold a close back past the next close of
        // the same name, and the two merge.
        for i in 0..2 * max {
            std::fs::File::create(dir.join(if i % 2 == 0 { "a" } else { "b" })).unwrap();
        }
        assert!(w.drain(), "an overflowed queue: rescan");
        w.sync(&[Want {
            dir: dir.clone(),
            mask: WatchMask::CLOSE_WRITE,
            only: vec!["config.toml".into()],
        }]);
        assert!(!w.drain(), "the watch added again: no rescan");
        std::fs::write(dir.join("config.toml"), "").unwrap();
        assert!(w.drain(), "a write after the overflow: rescan");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
