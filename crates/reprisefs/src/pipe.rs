//! Writing a clipboard answer into a pipe the receiving client owns, off the
//! caller's thread with a bounded wait, since the receiver picks the pipe size
//! and whether it ever reads.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use zeroize::Zeroize;

pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// Readers the writer holds at once; past it the oldest is closed, so a
/// client that never reads cannot run the compositor out of fds.
const MAX_JOBS: usize = 256;

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

    /// The bytes as Latin-1 text (X11's `STRING`), one character each.
    pub fn into_latin1_text(self) -> String {
        let size = self.0.iter().map(|&b| if b < 0x80 { 1 } else { 2 }).sum();
        let mut text = String::with_capacity(size);
        text.extend(self.0.iter().map(|&b| char::from(b)));
        text
    }
}

impl Drop for PasteBuf {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

struct Job {
    fd: OwnedFd,
    bytes: Arc<[u8]>,
    sent: usize,
    deadline: Instant,
}

struct Writer {
    jobs: Mutex<mpsc::Sender<Job>>,
    wake: OwnedFd,
}

static WRITER: OnceLock<Option<Writer>> = OnceLock::new();

/// Hands `bytes` to the one writer thread, which drops `fd` once all went out,
/// the reader left, or [`WRITE_TIMEOUT`] passed. A stalled reader holds back
/// no other.
pub fn write_detached(fd: OwnedFd, bytes: Arc<[u8]>) {
    submit(fd, bytes, WRITE_TIMEOUT);
}

fn submit(fd: OwnedFd, bytes: Arc<[u8]>, timeout: Duration) {
    if let Err(e) = set_nonblocking(&fd) {
        log::debug!("pipe write: {e}");
        return;
    }
    let Some(writer) = WRITER.get_or_init(start_writer) else {
        return;
    };
    let job = Job {
        fd,
        bytes,
        sent: 0,
        deadline: Instant::now() + timeout,
    };
    if writer.jobs.lock().unwrap().send(job).is_err() {
        return;
    }
    // SAFETY: one readable byte, and `wake` is open for the program's life.
    unsafe { libc::write(writer.wake.as_raw_fd(), [1u8].as_ptr().cast(), 1) };
}

fn start_writer() -> Option<Writer> {
    let (woken, wake) = match std::io::pipe() {
        Ok((r, w)) => (OwnedFd::from(r), OwnedFd::from(w)),
        Err(e) => {
            log::warn!("pipe write: {e}");
            return None;
        }
    };
    if let Err(e) = set_nonblocking(&woken).and_then(|_| set_nonblocking(&wake)) {
        log::warn!("pipe write: {e}");
        return None;
    }
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("pipe-write".into())
        .spawn(move || run_writer(&woken, &rx));
    if let Err(e) = spawned {
        log::warn!("pipe write: cannot spawn: {e}");
        return None;
    }
    Some(Writer {
        jobs: Mutex::new(tx),
        wake,
    })
}

fn run_writer(woken: &OwnedFd, rx: &mpsc::Receiver<Job>) {
    let mut jobs: Vec<Job> = Vec::new();
    let mut pfds: Vec<libc::pollfd> = Vec::new();
    let mut drain = [0u8; 64];
    let mut failing = false;
    loop {
        // SAFETY: `drain` is `drain.len()` writable bytes, and `woken` is open.
        while unsafe { libc::read(woken.as_raw_fd(), drain.as_mut_ptr().cast(), drain.len()) } > 0 {
        }
        jobs.extend(rx.try_iter());
        cap_jobs(&mut jobs);
        let now = Instant::now();
        jobs.retain(|j| {
            let live = j.deadline > now;
            if !live {
                log::debug!("pipe write: reader stalled");
            }
            live
        });
        let next = jobs.iter().map(|j| j.deadline).min();
        let ms = next.map_or(-1, |d| {
            // Rounded up, so a deadline under a millisecond away does not spin.
            (d.saturating_duration_since(now).as_millis() + 1).min(i32::MAX as u128) as i32
        });
        pfds.clear();
        pfds.push(libc::pollfd {
            fd: woken.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        pfds.extend(jobs.iter().map(|j| libc::pollfd {
            fd: j.fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        }));
        // SAFETY: `pfds` is `pfds.len()` live `pollfd`s.
        let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, ms) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() != std::io::ErrorKind::Interrupted {
                if !failing {
                    log::warn!("pipe write: poll: {e}");
                }
                failing = true;
                let pause = if ms < 0 { 100 } else { ms.min(100) };
                std::thread::sleep(Duration::from_millis(pause as u64));
            }
            continue;
        }
        failing = false;
        if n == 0 {
            continue;
        }
        let mut ready = pfds[1..].iter().map(|p| p.revents);
        jobs.retain_mut(|j| {
            let revents = ready.next().unwrap_or(0);
            revents == 0 || step(j, revents)
        });
    }
}

/// Closes the jobs with the earliest deadlines until at most [`MAX_JOBS`] are left.
fn cap_jobs(jobs: &mut Vec<Job>) {
    let over = jobs.len().saturating_sub(MAX_JOBS);
    if over == 0 {
        return;
    }
    log::warn!("pipe write: too many readers, closing {over}");
    jobs.sort_by_key(|j| j.deadline);
    jobs.drain(..over);
}

/// Writes what the pipe takes; false once the job is over.
fn step(job: &mut Job, revents: libc::c_short) -> bool {
    if revents & libc::POLLOUT == 0 {
        log::debug!("pipe write: reader gone");
        return false;
    }
    while job.sent < job.bytes.len() {
        let rest = &job.bytes[job.sent..];
        // SAFETY: `rest` is `rest.len()` readable bytes, and `job.fd` is open.
        let w = unsafe { libc::write(job.fd.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
        if w < 0 {
            let e = std::io::Error::last_os_error();
            match e.kind() {
                std::io::ErrorKind::Interrupted => continue,
                std::io::ErrorKind::WouldBlock => return true,
                _ => {
                    log::debug!("pipe write: {e}");
                    return false;
                }
            }
        }
        job.sent += w as usize;
    }
    false
}

fn set_nonblocking(fd: &OwnedFd) -> std::io::Result<()> {
    // SAFETY: F_GETFL and F_SETFL take no pointers, and `fd` is open while
    // borrowed.
    let ok = unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        flags >= 0 && libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) >= 0
    };
    if ok {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn read_bounded(fd: &OwnedFd, max: usize, timeout: Duration) -> std::io::Result<Vec<u8>> {
    set_nonblocking(fd)?;
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

    fn read_all(read: OwnedFd) -> Vec<u8> {
        let mut got = Vec::new();
        std::fs::File::from(read).read_to_end(&mut got).unwrap();
        got
    }

    #[test]
    fn a_reading_peer_gets_it_all() {
        let (read, write) = pipe();
        let big: Arc<[u8]> = vec![b'y'; 1 << 20].into();
        write_detached(write, big.clone());
        assert_eq!(read_all(read), &big[..]);
    }

    #[test]
    fn seventeen_stalled_readers_hold_back_neither_the_last_nor_another() {
        let big: Arc<[u8]> = vec![b'z'; 1 << 20].into();
        let stalled: Vec<OwnedFd> = (0..17)
            .map(|_| {
                let (read, write) = pipe();
                write_detached(write, big.clone());
                read
            })
            .collect();
        let (other, write) = pipe();
        write_detached(write, big.clone());
        assert_eq!(
            read_all(other).len(),
            big.len(),
            "another reader gets it all"
        );
        let last = stalled.into_iter().last().unwrap();
        assert_eq!(
            read_all(last).len(),
            big.len(),
            "the 17th reader gets it all"
        );
    }

    #[test]
    fn a_stalled_reader_is_cut_off_at_its_deadline() {
        let (read, write) = pipe();
        let big: Arc<[u8]> = vec![b'x'; 1 << 20].into();
        let t = Instant::now();
        submit(write, big.clone(), Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(300));
        let got = read_all(read);
        assert!(got.len() < big.len(), "the fd closes with the rest unsent");
        assert!(t.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_closed_reader_ends_its_write_and_not_the_next() {
        let (read, write) = pipe();
        drop(read);
        write_detached(write, b"hello"[..].into());
        let (read, write) = pipe();
        write_detached(write, b"again"[..].into());
        assert_eq!(read_all(read), b"again");
    }

    #[test]
    fn past_the_job_cap_the_oldest_readers_are_closed() {
        let start = Instant::now();
        let mut reads = Vec::new();
        let mut jobs: Vec<Job> = (0..MAX_JOBS + 10)
            .map(|i| {
                let (read, write) = pipe();
                reads.push(read);
                Job {
                    fd: write,
                    bytes: b"x"[..].into(),
                    sent: 0,
                    deadline: start + Duration::from_millis(i as u64),
                }
            })
            .collect();
        jobs.swap(0, MAX_JOBS + 9);
        cap_jobs(&mut jobs);
        assert_eq!(jobs.len(), MAX_JOBS, "the writer keeps MAX_JOBS readers");
        let oldest = start + Duration::from_millis(10);
        assert!(
            jobs.iter().all(|j| j.deadline >= oldest),
            "the 10 earliest deadlines are the ones closed"
        );
        for read in reads.drain(..10) {
            assert_eq!(read_all(read), b"", "a closed reader sees EOF");
        }
        cap_jobs(&mut jobs);
        assert_eq!(jobs.len(), MAX_JOBS, "under the cap nothing more is closed");
    }

    /// Whether `read`'s writer closed within `wait`, data still unread or not.
    fn hung_up(read: &OwnedFd, wait: Duration) -> bool {
        let mut pfd = libc::pollfd {
            fd: read.as_raw_fd(),
            events: 0,
            revents: 0,
        };
        let ms = wait.as_millis() as i32;
        // SAFETY: `pfd` is one live `pollfd`, matching the count of 1.
        unsafe { libc::poll(&mut pfd, 1, ms) == 1 && pfd.revents & libc::POLLHUP != 0 }
    }

    #[test]
    fn the_writer_closes_the_oldest_readers_past_the_job_cap() {
        let big: Arc<[u8]> = vec![b'c'; 1 << 20].into();
        // A deadline earlier than the other tests' keeps their readers out of the cut.
        let reads: Vec<OwnedFd> = (0..MAX_JOBS + 10)
            .map(|_| {
                let (read, write) = pipe();
                submit(write, big.clone(), Duration::from_secs(20));
                read
            })
            .collect();
        for (i, read) in reads[..10].iter().enumerate() {
            assert!(
                hung_up(read, Duration::from_secs(1)),
                "reader {i} of the 10 oldest is closed"
            );
        }
        assert!(
            !hung_up(reads.last().unwrap(), Duration::ZERO),
            "the newest reader stays open"
        );
    }

    #[test]
    fn a_paste_stops_at_the_cap() {
        assert!(paste_fits(MAX_PASTE_BYTES - 1, 1));
        assert!(!paste_fits(MAX_PASTE_BYTES, 1));
        assert!(!paste_fits(usize::MAX, 1));
    }
}
