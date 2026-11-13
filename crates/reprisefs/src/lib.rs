//! The file-system side of the shell, shared by the compositor (desktop icons, Send To, Properties, the
//! launcher's file search) and Explorer. `std` plus `libc` for system calls `std` lacks, zbus for
//! UDisks2, png and md5 for the thumbnail cache, and toml_edit for the Recycle Bin's settings in
//! `config.toml`; nothing here knows about a display, a seat or the compositor.

#[cfg(test)]
reprisebus::test_session!();

pub mod apps;
pub mod autostart;
pub mod batchrename;
pub mod binage;
pub mod bincap;
mod bindir;
pub mod checksum;
pub mod complete;
pub mod compress;
pub mod confined;
pub mod deadline;
pub mod dialogops;
pub mod dirchange;
pub mod drawncache;
pub mod dropdata;
pub mod extract;
pub mod fileclip;
pub mod flatpak;
pub mod folders;
pub mod folderwatch;
pub mod gvfsmeta;
pub mod hidden;
pub mod history;
pub mod icons;
pub mod imagesize;
pub mod jobs;
pub mod launcher;
pub mod listing;
pub mod locale;
pub mod meta;
pub mod mimetree;
pub mod multicall;
pub mod names;
pub mod netshare;
pub mod netspeed;
pub mod ops;
pub mod pictures;
pub mod pins;
pub mod pipe;
pub mod places;
pub mod program;
pub mod progress;
pub mod props;
pub mod proxy;
pub mod quicklaunch;
pub mod recent;
pub mod region;
pub mod removal;
pub mod replace;
pub mod scope;
pub mod screenreader;
pub mod search;
pub mod searchproviders;
pub mod sendto;
pub mod software;
pub mod sysinfo;
pub mod thumbnailers;
pub mod thumbnails;
pub mod thumbpipe;
pub mod trash;
pub mod udisks;
pub mod undo;
pub mod usage;
pub mod xkb;

#[cfg(test)]
pub(crate) mod testfs {
    use std::path::Path;

    pub fn mkfifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a NUL-terminated path that outlives the call.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    /// `f`'s answer, run on a thread of its own; panics when it takes over five seconds, as a read blocked on a FIFO does.
    pub fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("returns without blocking")
    }

    /// A stand-in for a stopped network mount, which `ops::resolved` asks before it resolves a path.
    pub mod dead {
        use std::path::{Path, PathBuf};
        use std::sync::{Condvar, Mutex};

        static DEAD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
        static ANSWERED: Condvar = Condvar::new();

        pub fn stop(dir: &Path) {
            DEAD.lock().unwrap().push(dir.to_path_buf());
        }

        pub fn resume(dir: &Path) {
            DEAD.lock().unwrap().retain(|d| d != dir);
            ANSWERED.notify_all();
        }

        /// Waits while resolving `path` reaches a stopped folder: `path` lies in one, or a link on
        /// its way points into one.
        pub fn stall(path: &Path) {
            let mut dead = DEAD.lock().unwrap();
            while reaches(path, &dead) {
                dead = ANSWERED.wait(dead).unwrap();
            }
        }

        fn reaches(path: &Path, dead: &[PathBuf]) -> bool {
            !dead.is_empty()
                && path.ancestors().any(|at| {
                    let to = match std::fs::read_link(at) {
                        Ok(t) => at.parent().map_or(t.clone(), |p| p.join(&t)),
                        Err(_) => at.to_path_buf(),
                    };
                    dead.iter().any(|d| to.starts_with(d))
                })
        }
    }
}
