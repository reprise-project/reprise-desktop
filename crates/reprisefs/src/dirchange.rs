//! Waits on a thread for any of a set of directories to gain, lose or rewrite an entry, so its
//! host can read again what it read from them.

use std::ffi::CString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

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

/// Blocks until an entry of one of `dirs` changes, then until none has for `settle`, so one
/// package install is one return; or until `stopped`'s [`Stop`] drops. Err when none of `dirs`
/// could be watched.
pub fn wait(dirs: &[PathBuf], settle: Duration, stopped: &Stopped) -> io::Result<Woke> {
    // SAFETY: a plain system call; the fd it returns is owned below.
    let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is a fresh fd nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut watched = 0;
    for dir in dirs {
        let Ok(path) = CString::new(dir.as_os_str().as_bytes()) else {
            continue;
        };
        // SAFETY: `path` is NUL-terminated and outlives the call.
        if unsafe { libc::inotify_add_watch(fd.as_raw_fd(), path.as_ptr(), MASK) } >= 0 {
            watched += 1;
        }
    }
    if watched == 0 {
        return Err(io::Error::new(ErrorKind::NotFound, "no directory to watch"));
    }
    let settle = settle.as_millis().min(i32::MAX as u128) as i32;
    let mut timeout = -1;
    let mut buf = [0u8; 4096];
    loop {
        let mut polls = [fd.as_raw_fd(), stopped.0.as_raw_fd()].map(|fd| libc::pollfd {
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
        let n = unsafe { libc::read(fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(e);
            }
            continue;
        }
        timeout = settle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(wait(&dirs, Duration::ZERO, &stopped).unwrap()));
        drop(stop);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok(Woke::Stopped),
            "a stopped wait returns though no directory changed"
        );
    }

    #[test]
    fn nothing_to_watch_returns_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("missing");
        let (_stop, stopped) = stop().unwrap();
        assert_eq!(
            wait(&[gone], Duration::ZERO, &stopped).unwrap_err().kind(),
            ErrorKind::NotFound
        );
    }
}
