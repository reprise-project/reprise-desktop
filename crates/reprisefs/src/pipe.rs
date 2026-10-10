//! Writing a clipboard answer into a pipe the receiving client owns, on its own
//! thread with a bounded wait, since the receiver picks the pipe size and
//! whether it ever reads.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Bytes a paste may bring in before the offer is dropped; the owner is any client.
pub const MAX_PASTE_BYTES: usize = 8 << 20;

pub const PASTE_TIMEOUT: Duration = Duration::from_secs(30);

pub fn paste_fits(have: usize, more: usize) -> bool {
    have.saturating_add(more) <= MAX_PASTE_BYTES
}

/// A paste's bytes as they come off the pipe. Any client may be pasting
/// a password, so no buffer that held them is freed unwiped: growth moves
/// to a bigger buffer and zeroizes the old one, and drop zeroizes.
#[derive(Default)]
pub struct PasteBuf(Vec<u8>);

impl PasteBuf {
    pub fn push(&mut self, more: &[u8]) -> bool {
        if !paste_fits(self.0.len(), more.len()) {
            return false;
        }
        let need = self.0.len() + more.len();
        if need > self.0.capacity() {
            let cap = need.max(2 * self.0.capacity()).min(MAX_PASTE_BYTES);
            let mut grown = Vec::with_capacity(cap);
            grown.extend_from_slice(&self.0);
            std::mem::replace(&mut self.0, grown).zeroize();
        }
        self.0.extend_from_slice(more);
        true
    }

    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes as text, invalid UTF-8 replaced. Valid text keeps the
    /// buffer, so the caller owns the only copy and zeroizes it.
    pub fn into_text(mut self) -> String {
        match String::from_utf8(std::mem::take(&mut self.0)) {
            Ok(text) => text,
            Err(e) => {
                let mut bytes = e.into_bytes();
                // Sized up front: `from_utf8_lossy` grows its String as each
                // U+FFFD lands, freeing the outgrown buffer unwiped.
                let size = |c: std::str::Utf8Chunk| {
                    c.valid().len() + if c.invalid().is_empty() { 0 } else { 3 }
                };
                let mut text = String::with_capacity(bytes.utf8_chunks().map(size).sum());
                for c in bytes.utf8_chunks() {
                    text.push_str(c.valid());
                    if !c.invalid().is_empty() {
                        text.push(char::REPLACEMENT_CHARACTER);
                    }
                }
                bytes.zeroize();
                text
            }
        }
    }
}

impl Drop for PasteBuf {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

pub const MAX_WRITERS: usize = 16;

static WRITERS: Gate = Gate::new(MAX_WRITERS);

struct Gate {
    live: AtomicUsize,
    max: usize,
}

impl Gate {
    const fn new(max: usize) -> Self {
        Gate {
            live: AtomicUsize::new(0),
            max,
        }
    }

    fn enter(&'static self) -> Option<Slot> {
        self.live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(self))
    }
}

struct Slot(&'static Gate);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn write_detached(fd: OwnedFd, bytes: Arc<[u8]>) {
    spawn_writer(&WRITERS, fd, bytes, WRITE_TIMEOUT);
}

fn spawn_writer(gate: &'static Gate, fd: OwnedFd, bytes: Arc<[u8]>, timeout: Duration) -> bool {
    let Some(slot) = gate.enter() else {
        log::debug!("pipe write: {} writers busy, dropping", gate.max);
        return false;
    };
    let spawned = std::thread::Builder::new()
        .name("pipe-write".into())
        .spawn(move || {
            // Locals drop in reverse, so the fd closes before the slot frees.
            let _slot = slot;
            let (fd, bytes) = (fd, bytes);
            if let Err(e) = write_bounded(&fd, &bytes, timeout) {
                log::debug!("pipe write: {e}");
            }
        });
    if let Err(e) = &spawned {
        log::warn!("pipe write: cannot spawn: {e}");
    }
    spawned.is_ok()
}

pub fn write_bounded(fd: &OwnedFd, bytes: &[u8], timeout: Duration) -> std::io::Result<()> {
    // SAFETY: F_GETFL and F_SETFL take no pointers, and `fd` is open while
    // borrowed.
    let nonblocking = unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        flags >= 0 && libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0
    };
    if !nonblocking {
        return Err(std::io::Error::last_os_error());
    }
    let deadline = Instant::now() + timeout;
    let mut rest = bytes;
    while !rest.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "reader stalled",
            ));
        }
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let ms = left.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `pfd` is one live `pollfd`, matching the count of 1.
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            continue;
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 && pfd.revents & libc::POLLOUT == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "reader gone",
            ));
        }
        // SAFETY: `rest` is `rest.len()` readable bytes, and `fd` is open
        // while borrowed.
        let w = unsafe {
            libc::write(
                fd.as_raw_fd(),
                rest.as_ptr() as *const libc::c_void,
                rest.len(),
            )
        };
        if w < 0 {
            let e = std::io::Error::last_os_error();
            match e.kind() {
                std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => continue,
                _ => return Err(e),
            }
        }
        rest = &rest[w as usize..];
    }
    Ok(())
}

pub fn read_bounded(fd: &OwnedFd, max: usize, timeout: Duration) -> std::io::Result<Vec<u8>> {
    // SAFETY: F_GETFL and F_SETFL take no pointers, and `fd` is open while
    // borrowed.
    let nonblocking = unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        flags >= 0 && libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0
    };
    if !nonblocking {
        return Err(std::io::Error::last_os_error());
    }
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "writer stalled",
            ));
        }
        let mut pfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = left.as_millis().min(i32::MAX as u128) as i32;
        // SAFETY: `pfd` is one live `pollfd`, matching the count of 1.
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if n == 0 {
            continue;
        }
        // SAFETY: `chunk` is `chunk.len()` writable bytes, and `fd` is open
        // while borrowed.
        let r = unsafe {
            libc::read(
                fd.as_raw_fd(),
                chunk.as_mut_ptr() as *mut libc::c_void,
                chunk.len(),
            )
        };
        match r {
            0 => return Ok(out),
            r if r > 0 => {
                if out.len() + r as usize > max {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::FileTooLarge,
                        "more than a drop may carry",
                    ));
                }
                out.extend_from_slice(&chunk[..r as usize]);
            }
            _ => {
                let e = std::io::Error::last_os_error();
                match e.kind() {
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock => {}
                    _ => return Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::FromRawFd;

    fn pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0i32; 2];
        // SAFETY: `fds` has room for the two fds `pipe2` writes, and after a
        // successful call nothing else owns them.
        unsafe {
            assert_eq!(libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC), 0);
            (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
        }
    }

    #[test]
    fn a_bounded_read_ends_at_eof_the_timeout_or_the_cap() {
        use std::io::Write;
        let (read, write) = pipe();
        std::fs::File::from(write)
            .write_all(b"file:///a\r\n")
            .unwrap();
        assert_eq!(
            read_bounded(&read, 64, Duration::from_secs(5)).unwrap(),
            b"file:///a\r\n"
        );
        let (read, _open) = pipe();
        let err = read_bounded(&read, 64, Duration::from_millis(50)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        let (read, write) = pipe();
        std::fs::File::from(write).write_all(&[b'x'; 65]).unwrap();
        let err = read_bounded(&read, 64, Duration::from_secs(5)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::FileTooLarge);
    }

    #[test]
    fn a_stalled_reader_times_out() {
        let (_read, write) = pipe();
        let big = vec![b'x'; 1 << 20];
        let t = Instant::now();
        let err = write_bounded(&write, &big, Duration::from_millis(100)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_reading_peer_gets_it_all() {
        let (read, write) = pipe();
        let big: Arc<[u8]> = vec![b'y'; 1 << 20].into();
        write_detached(write, big.clone());
        let mut got = Vec::new();
        std::fs::File::from(read).read_to_end(&mut got).unwrap();
        assert_eq!(got, &big[..]);
    }

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let t = Instant::now();
        while !done() {
            assert!(t.elapsed() < Duration::from_secs(5), "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The gate admits two writers, closes the third fd unwritten, and admits
    /// again after a writer ends. Non-timeout writers wait far longer than the test.
    #[test]
    fn writers_past_the_cap_are_refused_until_one_ends() {
        static GATE: Gate = Gate::new(2);
        let live = || GATE.live.load(Ordering::Acquire);
        let big: Arc<[u8]> = vec![b'z'; 1 << 20].into();
        let never = Duration::from_secs(60);

        let (stalled, w1) = pipe();
        let (gone, w2) = pipe();
        assert!(spawn_writer(&GATE, w1, big.clone(), never));
        assert!(spawn_writer(&GATE, w2, big.clone(), never));
        assert_eq!(live(), 2);

        let (refused, w3) = pipe();
        assert!(!spawn_writer(&GATE, w3, big.clone(), never));
        let mut got = Vec::new();
        std::fs::File::from(refused).read_to_end(&mut got).unwrap();
        assert!(got.is_empty(), "a refused receive reads EOF with no bytes");

        drop(gone);
        wait_for("the broken pipe frees its slot", || live() == 1);

        let (_again, w4) = pipe();
        assert!(spawn_writer(
            &GATE,
            w4,
            big.clone(),
            Duration::from_millis(100)
        ));
        wait_for("the timed-out writer frees its slot", || live() == 1);

        drop(stalled);
        wait_for("the last writer frees its slot", || live() == 0);
    }

    #[test]
    fn a_paste_stops_at_the_cap() {
        assert!(paste_fits(MAX_PASTE_BYTES - 1, 1));
        assert!(!paste_fits(MAX_PASTE_BYTES, 1));
        assert!(!paste_fits(usize::MAX, 1));
    }

    #[test]
    fn a_closed_reader_is_an_error() {
        let (read, write) = pipe();
        drop(read);
        let err = write_bounded(&write, b"hello", Duration::from_secs(1)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
    }
}
