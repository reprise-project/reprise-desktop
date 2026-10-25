//! A set of inotify watches on directories that may come and go; a watch whose directory is deleted or moved away
//! is dropped and the next sync ([`DirWatch::plan`], then [`DirWatch::adopt`]) adds it again.

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask, Watches};
use std::ffi::OsString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
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
    mask: WatchMask,
    only: Vec<OsString>,
}

pub struct DirWatch {
    ino: Inotify,
    watched: Vec<Watched>,
    /// The merged want of the last [`DirWatch::plan`].
    want: Vec<Want>,
}

/// The directories a [`DirWatch::plan`] left to watch, with masks.
pub struct Adds {
    watches: Watches,
    dirs: Vec<(PathBuf, WatchMask)>,
}

/// The watches an [`Adds::run`] made, with the masks they were made with.
pub struct Added(Vec<(PathBuf, WatchDescriptor, WatchMask)>);

/// What [`DirWatch::adder`] saw watched, for [`Adder::run`] on a thread.
pub struct Adder {
    watches: Watches,
    watched: Vec<(PathBuf, WatchMask)>,
}

impl Adder {
    /// Watches each directory of `want` that exists and was not watched for its mask when the
    /// adder was made. The loop's next [`DirWatch::plan`] of a want holding these, then
    /// [`DirWatch::adopt`], takes them.
    pub fn run(self, want: &[Want]) -> Added {
        let dirs = (merged(want).into_iter())
            .filter(|w| !covered(&self.watched, &w.dir, w.mask))
            .map(|w| (w.dir, w.mask))
            .collect();
        Adds {
            watches: self.watches,
            dirs,
        }
        .run()
    }
}

impl Adds {
    /// Each directory with an [`Adds`] of its own, for a thread each.
    pub fn each(self) -> impl Iterator<Item = (PathBuf, Adds)> {
        let watches = self.watches;
        (self.dirs.into_iter()).map(move |(dir, mask)| {
            let adds = Adds {
                watches: watches.clone(),
                dirs: vec![(dir.clone(), mask)],
            };
            (dir, adds)
        })
    }

    /// Watches each directory that exists.
    pub fn run(mut self) -> Added {
        let mut added = Vec::new();
        for (dir, mask) in self.dirs {
            if !dir.is_dir() {
                continue;
            }
            // MASK_ADD: a second path to a watched inode widens its watch instead of replacing it.
            let full = mask | WatchMask::DELETE_SELF | WatchMask::MOVE_SELF | WatchMask::MASK_ADD;
            match (self.watches).add(&dir, full) {
                Ok(wd) => added.push((dir, wd, mask)),
                Err(e) => log::warn!("not watching {}: {e}", dir.display()),
            }
        }
        Added(added)
    }
}

/// A [`DirWatch`] synced on file jobs, one sync out at a time: the job works out the want and
/// runs an [`Adder`], the loop takes what it added.
#[derive(Default)]
pub struct JobWatch {
    pub watch: Option<DirWatch>,
    syncing: bool,
    again: bool,
    /// An event counted since the last sync started.
    hit: bool,
}

/// A sync's job: its adder, and whether an event counted before it started.
pub struct JobSync {
    pub adder: Adder,
    pub hit: bool,
}

impl JobWatch {
    /// [`DirWatch::drain`], remembered for the next sync.
    pub fn drain(&mut self) -> bool {
        let hit = self.watch.as_mut().is_some_and(DirWatch::drain);
        self.hit |= hit;
        hit
    }

    /// The next sync's job; None while one is out, and the next [`JobWatch::landed`] then asks
    /// for another.
    pub fn start(&mut self) -> Option<JobSync> {
        let adder = self.watch.as_ref()?.adder();
        if self.syncing {
            self.again = true;
            return None;
        }
        self.syncing = true;
        let hit = std::mem::take(&mut self.hit);
        Some(JobSync { adder, hit })
    }

    pub fn syncing(&self) -> bool {
        self.syncing
    }

    /// Takes the watches a sync's job added for `want`. True when another sync was asked for
    /// meanwhile.
    pub fn landed(&mut self, want: &[Want], added: Added) -> bool {
        self.syncing = false;
        if let Some(w) = self.watch.as_mut() {
            w.plan(want);
            w.adopt(added);
        }
        std::mem::take(&mut self.again)
    }

    /// Whether the sync a [`JobWatch::landed`] asked for will report an event.
    pub fn hit_pending(&self) -> bool {
        self.hit
    }
}

impl DirWatch {
    pub fn new() -> io::Result<DirWatch> {
        Ok(DirWatch {
            ino: Inotify::init()?,
            watched: Vec::new(),
            want: Vec::new(),
        })
    }

    pub fn fd(&self) -> RawFd {
        self.ino.as_raw_fd()
    }

    /// Make the watched set `want`: drop directories no longer in it, take
    /// each kept one's `only`, and watch every wanted directory that exists
    /// and is not watched yet or for less than its mask. Wants naming the same path are merged;
    /// two paths naming one directory share its watch. `DELETE_SELF | MOVE_SELF` is added to
    /// each mask so the watch notices its directory leaving.
    #[cfg(test)]
    pub fn sync(&mut self, want: &[Want]) {
        let adds = self.plan(want);
        let added = adds.run();
        self.adopt(added);
    }

    /// A sync without looking up a path: drops and updates the watches, and returns
    /// the directories still to watch or to watch for a wider mask, for [`Adds::run`] on a thread (a directory on a mount
    /// that stopped answering blocks it) and [`DirWatch::adopt`] after.
    pub fn plan(&mut self, want: &[Want]) -> Adds {
        self.want = merged(want);
        let want = &self.want;
        let (kept, dropped): (Vec<_>, Vec<_>) =
            (self.watched.drain(..)).partition(|x| want.iter().any(|w| w.dir == x.dir));
        for x in dropped {
            if !kept.iter().any(|k| k.wd == x.wd) {
                let _ = self.ino.watches().remove(x.wd);
            }
        }
        self.watched = kept;
        let mut dirs = Vec::new();
        for w in want {
            let Some(x) = self.watched.iter_mut().find(|x| x.dir == w.dir) else {
                dirs.push((w.dir.clone(), w.mask));
                continue;
            };
            x.only.clone_from(&w.only);
            // Adding the inode again keeps its descriptor and replaces its mask.
            if !x.mask.contains(w.mask) {
                dirs.push((w.dir.clone(), w.mask | x.mask));
            }
        }
        Adds {
            watches: self.ino.watches(),
            dirs,
        }
    }

    /// The watched directories as they are now, for wants worked out on a thread.
    pub fn adder(&self) -> Adder {
        Adder {
            watches: self.ino.watches(),
            watched: (self.watched.iter())
                .map(|x| (x.dir.clone(), x.mask))
                .collect(),
        }
    }

    /// Takes the watches an [`Adds::run`] made, as wanted by the last [`DirWatch::plan`]: one
    /// whose directory left the want meanwhile, or is watched already, is dropped; one that
    /// widened a watch's mask updates it; one landing on a watched inode under another path
    /// records that path too. True when a wanted directory was newly watched or widened.
    pub fn adopt(&mut self, added: Added) -> bool {
        let mut grew = false;
        for (dir, wd, mask) in added.0 {
            let wanted = self.want.iter().find(|w| w.dir == dir);
            let shared = self.watched.iter().any(|x| x.wd == wd);
            let mut joined = mask;
            for x in self.watched.iter_mut().filter(|x| x.wd == wd) {
                grew |= x.dir == dir && !x.mask.contains(mask);
                x.mask |= mask;
                joined |= x.mask;
            }
            match wanted {
                Some(w) if !self.watched.iter().any(|x| x.dir == dir) => {
                    self.watched.push(Watched {
                        dir,
                        wd,
                        mask: joined,
                        only: w.only.clone(),
                    });
                    grew = true;
                }
                _ if !shared => {
                    let _ = self.ino.watches().remove(wd);
                }
                _ => {}
            }
        }
        grew
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
                let mut paths = self.watched.iter().filter(|w| w.wd == ev.wd).peekable();
                if paths.peek().is_none() {
                    continue;
                }
                if ev
                    .mask
                    .intersects(EventMask::DELETE_SELF | EventMask::MOVE_SELF | EventMask::IGNORED)
                {
                    gone.push(ev.wd.clone());
                    changed = true;
                } else if paths.any(|w| {
                    w.only.is_empty() || w.only.iter().any(|n| ev.name == Some(n.as_os_str()))
                }) {
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

/// Whether `dir` is in `watched` for every event of `mask`.
fn covered(watched: &[(PathBuf, WatchMask)], dir: &PathBuf, mask: WatchMask) -> bool {
    (watched.iter()).any(|(d, m)| d == dir && m.contains(mask))
}

/// `want` with each path once: masks joined, `only` joined (empty, meaning
/// every event, wins).
fn merged(want: &[Want]) -> Vec<Want> {
    let mut out: Vec<Want> = Vec::with_capacity(want.len());
    for w in want {
        let Some(m) = out.iter_mut().find(|m| m.dir == w.dir) else {
            out.push(Want {
                dir: w.dir.clone(),
                mask: w.mask,
                only: w.only.clone(),
            });
            continue;
        };
        m.mask |= w.mask;
        if w.only.is_empty() {
            m.only.clear();
        } else if !m.only.is_empty() {
            for n in &w.only {
                if !m.only.contains(n) {
                    m.only.push(n.clone());
                }
            }
        }
    }
    out
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

    /// A want that asks a watched directory for more events widens its watch.
    #[test]
    fn a_wider_want_on_a_watched_directory_widens_its_mask() {
        let home =
            std::env::temp_dir().join(format!("reprise-dirwatch-wide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let want = |mask| Want {
            dir: home.clone(),
            mask,
            only: vec!["Documents".into()],
        };
        let mut w = DirWatch::new().unwrap();
        w.sync(&[want(WatchMask::DELETE)]);
        w.sync(&[want(WatchMask::DELETE), want(WatchMask::CREATE)]);
        std::fs::create_dir(home.join("Documents")).unwrap();
        assert!(w.drain(), "Documents made: the second want's event");
        std::fs::remove_dir(home.join("Documents")).unwrap();
        assert!(w.drain(), "Documents deleted: the first want's event");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// Two wants on one directory (the home folder, for the desktop folder
    /// and for `.local` on the way to the trash) both count.
    #[test]
    fn wants_on_one_directory_are_merged() {
        let home =
            std::env::temp_dir().join(format!("reprise-dirwatch-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let only = |name: &str| Want {
            dir: home.clone(),
            mask: WatchMask::CREATE,
            only: vec![name.into()],
        };
        let mut w = DirWatch::new().unwrap();
        w.sync(&[only("Desktop"), only(".local")]);
        std::fs::write(home.join("notes.txt"), "").unwrap();
        assert!(!w.drain(), "neither name: no rescan");
        std::fs::create_dir(home.join(".local")).unwrap();
        assert!(w.drain(), "the second want's name: rescan");
        std::fs::create_dir(home.join("Desktop")).unwrap();
        assert!(w.drain(), "the first want's name: rescan");

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

    #[test]
    fn a_plan_looks_up_no_folder_and_its_adds_land_later() {
        let home =
            std::env::temp_dir().join(format!("reprise-dirwatch-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Music")).unwrap();
        std::fs::create_dir_all(home.join("Videos")).unwrap();
        let want = |name: &str| Want {
            dir: home.join(name),
            mask: WatchMask::CREATE,
            only: Vec::new(),
        };
        let mut w = DirWatch::new().unwrap();
        let adds = w.plan(&[want("Music"), want("Videos"), want("Gone")]);
        assert_eq!(adds.dirs.len(), 3, "every unwatched folder, found or not");
        std::fs::write(home.join("Music/a.ogg"), "").unwrap();
        assert!(!w.drain(), "nothing watched before the adds land");

        let added = adds.run();
        let again = w.plan(&[want("Music")]);
        assert_eq!(again.dirs.len(), 1, "Music is not watched yet");
        w.adopt(added);
        std::fs::write(home.join("Videos/b.mkv"), "").unwrap();
        assert!(!w.drain(), "Videos left the want before its add landed");
        std::fs::write(home.join("Music/c.ogg"), "").unwrap();
        assert!(w.drain(), "Music is watched once its add landed");

        w.adopt(again.run());
        assert_eq!(
            w.watched.len(),
            1,
            "a second add of Music lands on the same watch"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_job_watch_starts_one_sync_at_a_time_and_one_asked_meanwhile_follows() {
        let dir = std::env::temp_dir().join(format!("reprise-dirwatch-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let want = [Want {
            dir: dir.clone(),
            mask: WatchMask::CREATE,
            only: Vec::new(),
        }];
        let mut j = JobWatch::default();
        assert!(j.start().is_none(), "no watch: no sync");
        j.watch = Some(DirWatch::new().unwrap());
        let first = j.start().expect("a sync starts");
        assert!(!first.hit);
        assert!(j.start().is_none(), "one sync out at a time");
        let added = first.adder.run(&want);
        assert!(j.landed(&want, added), "the one asked meanwhile follows");
        std::fs::write(dir.join("a"), "").unwrap();
        assert!(j.drain(), "the directory is watched");
        let next = j.start().expect("the next sync starts");
        assert!(next.hit, "it reports the event");
        assert!(
            !j.landed(&want, next.adder.run(&want)),
            "nothing asked meanwhile"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_adder_made_on_the_loop_watches_on_a_thread_what_was_not_watched_then() {
        let home =
            std::env::temp_dir().join(format!("reprise-dirwatch-adder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for d in ["Desktop", "STICK", "CARD"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        let want = |name: &str| Want {
            dir: home.join(name),
            mask: WatchMask::CREATE,
            only: Vec::new(),
        };
        let mut w = DirWatch::new().unwrap();
        w.sync(&[want("Desktop")]);
        let (adder, first) = (w.adder(), [want("Desktop"), want("STICK")]);
        let added = std::thread::spawn(move || adder.run(&first))
            .join()
            .unwrap();
        assert_eq!(
            added.0.len(),
            1,
            "Desktop was watched already: only STICK is added"
        );
        w.plan(&[want("Desktop"), want("STICK"), want("CARD")]);
        w.adopt(added);

        let rest: Vec<PathBuf> = (w
            .plan(&[want("Desktop"), want("STICK"), want("CARD")])
            .each())
        .map(|(dir, add)| {
            w.adopt(add.run());
            dir
        })
        .collect();
        assert_eq!(
            rest,
            [home.join("CARD")],
            "a folder an adder added is not planned again"
        );
        for d in ["Desktop", "STICK", "CARD"] {
            std::fs::write(home.join(d).join("a"), "").unwrap();
            assert!(w.drain(), "{d} is watched");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn two_paths_to_one_directory_share_its_watch_and_plan_no_more_adds() {
        let home =
            std::env::temp_dir().join(format!("reprise-dirwatch-alias-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("var/u")).unwrap();
        std::os::unix::fs::symlink(home.join("var"), home.join("home")).unwrap();
        let want = [
            Want {
                dir: home.join("var/u"),
                mask: WatchMask::CREATE,
                only: Vec::new(),
            },
            Want {
                dir: home.join("home/u"),
                mask: WatchMask::MOVED_TO,
                only: vec!["x.txt".into()],
            },
        ];
        let mut w = DirWatch::new().unwrap();
        let first = w.plan(&want[..1]).run();
        assert!(w.adopt(first), "the first path is newly watched");
        let alias = w.plan(&want).run();
        assert!(w.adopt(alias), "the second path is newly recorded");
        assert!(w.plan(&want).dirs.is_empty(), "both paths are watched");
        let again = w.plan(&want).run();
        assert!(!w.adopt(again), "nothing more to add");

        std::fs::write(home.join("var/u/a.txt"), "").unwrap();
        assert!(
            w.drain(),
            "the second path's add kept the first one's CREATE"
        );
        let _ = std::fs::remove_dir_all(&home);
    }
}
