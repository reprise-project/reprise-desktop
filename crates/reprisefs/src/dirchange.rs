//! Waits on a thread for any of a set of directories to gain, lose or rewrite an entry, so its
//! host can read again what it read from them.

use std::collections::HashMap;
use std::ffi::{CString, OsStr, OsString};
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

/// On a directory above a path not made yet: only an entry appearing there matters.
const APPEAR: u32 = libc::IN_CREATE | libc::IN_MOVED_TO | libc::IN_ONLYDIR;

const HEADER: usize = std::mem::size_of::<libc::inotify_event>();

/// Held by whoever started a [`wait`]: dropping it ends the wait at once.
pub struct Stop {
    _writer: OwnedFd,
}

/// What a [`wait`] polls besides its directories: the read end of its [`Stop`]'s pipe, which
/// hangs up when the `Stop` drops.
pub struct Stopped(OwnedFd);

/// A [`Stop`] and the [`Stopped`] to hand the [`wait`] it ends.
pub fn stop() -> io::Result<(Stop, Stopped)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two fds pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds are fresh and owned only here.
    let (reader, writer) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    Ok((Stop { _writer: writer }, Stopped(reader)))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Woke {
    Changed,
    Stopped,
}

/// What one watch descriptor stands for.
#[derive(Default)]
struct Watched {
    /// One of the directories asked for: any change of an entry counts.
    whole: bool,
    /// Paths not made yet, each with its next component below this directory.
    awaited: Vec<(OsString, PathBuf)>,
}

struct Watches {
    fd: OwnedFd,
    by_wd: HashMap<i32, Watched>,
}

impl Watches {
    fn add(&mut self, dir: &Path, mask: u32) -> Option<i32> {
        let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
        // SAFETY: `path` is NUL-terminated and outlives the call.
        let wd = unsafe {
            libc::inotify_add_watch(self.fd.as_raw_fd(), path.as_ptr(), mask | libc::IN_MASK_ADD)
        };
        (wd >= 0).then_some(wd)
    }

    fn whole(&mut self, dir: &Path) -> bool {
        let Some(wd) = self.add(dir, MASK) else {
            return false;
        };
        self.by_wd.entry(wd).or_default().whole = true;
        true
    }

    /// Watches the closest directory above `path` for the next component on the way down.
    /// Some(true) once `path` is there, Some(false) while it is awaited, None when nothing
    /// above it could be watched.
    fn await_path(&mut self, path: &Path) -> Option<bool> {
        loop {
            if path.is_dir() {
                return Some(true);
            }
            let above = path.ancestors().skip(1).find(|p| p.is_dir())?;
            let next = path.strip_prefix(above).ok()?.components().next()?;
            let next = next.as_os_str().to_os_string();
            let wd = self.add(above, APPEAR)?;
            // Made between the check and the watch: go on down.
            if above.join(&next).is_dir() {
                continue;
            }
            let awaited = &mut self.by_wd.entry(wd).or_default().awaited;
            if !awaited.iter().any(|(_, p)| p == path) {
                awaited.push((next, path.to_path_buf()));
            }
            return Some(false);
        }
    }

    /// Whether the event on `wd` changes what the wait is for.
    fn changes(&mut self, wd: i32, mask: u32, name: &OsStr) -> bool {
        if mask & libc::IN_Q_OVERFLOW != 0 {
            return true;
        }
        if mask & libc::IN_IGNORED != 0 {
            let Some(gone) = self.by_wd.remove(&wd) else {
                return false;
            };
            return gone.whole || self.await_all(gone.awaited);
        }
        let Some(watched) = self.by_wd.get_mut(&wd) else {
            return false;
        };
        if watched.whole {
            return true;
        }
        if mask & (libc::IN_CREATE | libc::IN_MOVED_TO) == 0 {
            return false;
        }
        let (made, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut watched.awaited)
            .into_iter()
            .partition(|(next, _)| next == name);
        watched.awaited = waiting;
        self.await_all(made)
    }

    /// [`Watches::await_path`] for each awaited path; true when one is there now.
    fn await_all(&mut self, awaited: Vec<(OsString, PathBuf)>) -> bool {
        let mut there = false;
        for (_, path) in awaited {
            there |= self.await_path(&path) == Some(true);
        }
        there
    }
}

/// Blocks until an entry of one of `dirs` changes, or one of `dirs` not made yet appears, then
/// until nothing has for `settle`, so one package install is one return; or until `stopped`'s
/// [`Stop`] drops. A missing directory is awaited through the closest one above it, which wakes
/// the wait only for the name on the way down. Err when nothing could be watched.
pub fn wait(dirs: &[PathBuf], settle: Duration, stopped: &Stopped) -> io::Result<Woke> {
    // SAFETY: a plain system call; the fd it returns is owned below.
    let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut watches = Watches {
        // SAFETY: `raw` is a fresh fd nothing else owns.
        fd: unsafe { OwnedFd::from_raw_fd(raw) },
        by_wd: HashMap::new(),
    };
    let mut watched = 0;
    for dir in dirs {
        let armed = match watches.await_path(dir) {
            Some(true) => watches.whole(dir),
            Some(false) => true,
            None => false,
        };
        watched += usize::from(armed);
    }
    if watched == 0 {
        return Err(io::Error::new(ErrorKind::NotFound, "no directory to watch"));
    }
    let settle = settle.as_millis().min(i32::MAX as u128) as i32;
    let mut timeout = -1;
    let mut buf = [0u8; 4096];
    loop {
        let mut polls = [watches.fd.as_raw_fd(), stopped.0.as_raw_fd()].map(|fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        });
        // SAFETY: two valid pollfds.
        match unsafe { libc::poll(polls.as_mut_ptr(), 2, timeout) } {
            0 => return Ok(Woke::Changed),
            n if n < 0 => {
                let e = io::Error::last_os_error();
                if e.kind() != ErrorKind::Interrupted {
                    return Err(e);
                }
                continue;
            }
            _ if polls[1].revents != 0 => return Ok(Woke::Stopped),
            _ => {}
        }
        // SAFETY: `buf` is writable for its length.
        let n = unsafe { libc::read(watches.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(e);
            }
            continue;
        }
        let mut at = 0;
        let mut changed = false;
        while at + HEADER <= n as usize {
            // SAFETY: the kernel wrote a whole event header at `at`.
            let ev: libc::inotify_event =
                unsafe { std::ptr::read_unaligned(buf[at..].as_ptr().cast()) };
            let tail = &buf[at + HEADER..at + HEADER + ev.len as usize];
            let name = OsStr::from_bytes(tail.split(|&b| b == 0).next().unwrap_or_default());
            changed |= watches.changes(ev.wd, ev.mask, name);
            at += HEADER + ev.len as usize;
        }
        if changed {
            timeout = settle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn a_file_made_in_a_watched_directory_ends_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("48x48/apps");
        std::fs::create_dir_all(&sub).unwrap();
        let dirs = vec![dir.path().to_path_buf(), sub.clone()];
        let (stop, stopped) = stop().unwrap();
        let waiting = std::thread::spawn(move || wait(&dirs, Duration::from_millis(50), &stopped));
        // The watches go in after the thread starts: write, slower than the settle, until the
        // wait sees one.
        for i in 0..50 {
            std::fs::write(sub.join(format!("{i}.png")), b"").unwrap();
            std::thread::sleep(Duration::from_millis(200));
            if waiting.is_finished() {
                break;
            }
        }
        assert_eq!(waiting.join().unwrap().unwrap(), Woke::Changed);
        drop(stop);
    }

    #[test]
    fn dropping_the_stop_ends_the_wait_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let dirs = vec![dir.path().to_path_buf()];
        let (stop, stopped) = stop().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || tx.send(wait(&dirs, Duration::ZERO, &stopped).unwrap()));
        drop(stop);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(Woke::Stopped),
            "a stopped wait returns though no directory changed"
        );
    }

    #[test]
    fn a_missing_directory_ends_the_wait_once_made_and_nothing_else_beside_it_does() {
        let home = tempfile::tempdir().unwrap();
        let icons = home.path().join(".local/share/icons");
        let (stop, stopped) = stop().unwrap();
        let (tx, rx) = mpsc::channel();
        let dirs = vec![icons.clone()];
        std::thread::spawn(move || {
            tx.send(wait(&dirs, Duration::from_millis(50), &stopped).unwrap())
        });
        // Give the thread time to watch; whichever step it watches from, only the last counts.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(home.path().join("notes.txt"), b"").unwrap();
        std::fs::create_dir(home.path().join(".cache")).unwrap();
        std::fs::create_dir_all(home.path().join(".local/share/applications")).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(500)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "entries beside the path do not end the wait"
        );
        std::fs::create_dir(&icons).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(Woke::Changed));
        drop(stop);
    }

    #[test]
    fn a_path_without_a_watchable_directory_above_it_returns_at_once() {
        let (_stop, stopped) = stop().unwrap();
        assert_eq!(
            wait(&[PathBuf::from("relative/icons")], Duration::ZERO, &stopped)
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
    }
}
