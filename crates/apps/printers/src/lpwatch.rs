//! The lpoptions files under inotify: the default printer they name changes without a cupsd
//! event.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};

const FILE: &str = "lpoptions";

enum Role {
    /// The folder an lpoptions file is in.
    Folder,
    /// The parent the folder will be made in.
    Parent(OsString),
}

struct Watch {
    inotify: Inotify,
    watched: Vec<(WatchDescriptor, Role, PathBuf)>,
}

impl Watch {
    fn new(folders: &[PathBuf]) -> io::Result<Watch> {
        let mut w = Watch {
            inotify: Inotify::init()?,
            watched: Vec::new(),
        };
        for f in folders {
            if !w.add(f) {
                log::info!("{} is not watched for the default printer", f.display());
            }
        }
        Ok(w)
    }

    /// Watch `folder`, or the parent it will be made in; false when neither is there.
    fn add(&mut self, folder: &Path) -> bool {
        let mut watches = self.inotify.watches();
        let mask = WatchMask::CLOSE_WRITE
            | WatchMask::MOVED_TO
            | WatchMask::MOVED_FROM
            | WatchMask::DELETE
            | WatchMask::ONLYDIR;
        if let Ok(wd) = watches.add(folder, mask) {
            self.watched.push((wd, Role::Folder, folder.to_path_buf()));
            return true;
        }
        let (Some(parent), Some(name)) = (folder.parent(), folder.file_name()) else {
            return false;
        };
        match watches.add(parent, WatchMask::CREATE | WatchMask::MOVED_TO) {
            Ok(wd) => {
                let role = Role::Parent(name.to_os_string());
                self.watched.push((wd, role, folder.to_path_buf()));
                true
            }
            Err(_) => false,
        }
    }

    /// Block for the next events; true when they touched an lpoptions file. A folder that
    /// appears is watched from then on, and one that goes is waited for again.
    fn next(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut changed = false;
        let mut readd = Vec::new();
        let events = loop {
            match self.inotify.read_events_blocking(&mut buf) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                r => break r?,
            }
        };
        for e in events {
            let Some((wd, role, folder)) = self.watched.iter().find(|(wd, ..)| *wd == e.wd) else {
                continue;
            };
            if e.mask.contains(EventMask::IGNORED) {
                readd.push((wd.clone(), folder.clone(), false));
                changed = true;
                continue;
            }
            match role {
                Role::Folder => changed |= e.name == Some(FILE.as_ref()),
                Role::Parent(name) if e.name == Some(name.as_os_str()) => {
                    readd.push((wd.clone(), folder.clone(), true));
                    changed = true;
                }
                Role::Parent(_) => {}
            }
        }
        for (wd, folder, live) in readd {
            self.watched.retain(|(w, _, f)| !(*w == wd && *f == folder));
            if live && !self.watched.iter().any(|(w, ..)| *w == wd) {
                let _ = self.inotify.watches().remove(wd);
            }
            self.add(&folder);
        }
        Ok(changed)
    }
}

/// Call `wake` from a thread of its own whenever an lpoptions file in `folders` is written,
/// replaced or removed. False when inotify is not there.
pub fn watch(folders: Vec<PathBuf>, wake: impl Fn() + Send + 'static) -> bool {
    let mut w = match Watch::new(&folders) {
        Ok(w) => w,
        Err(e) => {
            log::info!("no inotify for the default printer: {e}");
            return false;
        }
    };
    std::thread::Builder::new()
        .name("lpoptions".into())
        .spawn(move || {
            loop {
                match w.next() {
                    Ok(true) => wake(),
                    Ok(false) => {}
                    Err(e) => {
                        log::warn!("watching lpoptions: {e}");
                        return;
                    }
                }
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Duration;

    #[test]
    fn writing_an_lpoptions_file_wakes_and_other_files_do_not() {
        let root = std::env::temp_dir().join(format!("reprise-lpwatch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (home, etc) = (root.join("home"), root.join("etc"));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&etc).unwrap();
        let user = home.join(".cups");
        let (tx, rx) = channel();
        assert!(watch(vec![user.clone(), etc.clone()], move || {
            let _ = tx.send(());
        }));
        let woke = || {
            let r = rx.recv_timeout(Duration::from_secs(2)).is_ok();
            while rx.try_recv().is_ok() {}
            r
        };

        std::fs::write(etc.join("printers.conf"), "x").unwrap();
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "another file in the folder does not wake"
        );
        std::fs::write(etc.join(FILE), "Default Laser\n").unwrap();
        assert!(woke(), "the system lpoptions written");

        std::fs::create_dir(&user).unwrap();
        assert!(woke(), "the user's folder made");
        std::fs::write(user.join(FILE), "Default Inkjet\n").unwrap();
        assert!(
            woke(),
            "the user's lpoptions written in a folder made after the start"
        );
        std::fs::write(user.join("tmp"), "Default Laser\n").unwrap();
        std::fs::rename(user.join("tmp"), user.join(FILE)).unwrap();
        assert!(woke(), "the user's lpoptions replaced");
        std::fs::remove_file(user.join(FILE)).unwrap();
        assert!(woke(), "the user's lpoptions removed");

        let _ = std::fs::remove_dir_all(&root);
    }
}
