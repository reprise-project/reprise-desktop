//! Frames over a Unix stream socket with at most one fd each: one `sendmsg` per frame, and a
//! reader that gives each fd to the frame it came with.

use std::collections::VecDeque;
use std::io;
use std::mem;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use libc::c_int;
use zeroize::Zeroizing;

use crate::frame::{FrameError, FrameReader};

// SAFETY: CMSG_SPACE is arithmetic on its argument.
const SPACE: usize = unsafe { libc::CMSG_SPACE(mem::size_of::<c_int>() as u32) } as usize;
/// `u64`s keep the control buffer aligned for `cmsghdr`.
const SPACE_WORDS: usize = SPACE.div_ceil(mem::size_of::<u64>());
const READ_BUF: usize = 16384;

/// Sends `frame` with `pass` attached in one `sendmsg`, returning the bytes sent; the caller
/// sends any rest without an fd. A non-blocking socket gives `WouldBlock` when it is full.
pub fn send(sock: BorrowedFd<'_>, frame: &[u8], pass: Option<BorrowedFd<'_>>) -> io::Result<usize> {
    let mut iov = libc::iovec {
        iov_base: frame.as_ptr() as *mut libc::c_void,
        iov_len: frame.len(),
    };
    let mut control = [0u64; SPACE_WORDS];
    // SAFETY: an all-zero msghdr is an empty message.
    let mut msg: libc::msghdr = unsafe { mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    if let Some(fd) = pass {
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = SPACE as _;
        // SAFETY: the control buffer holds SPACE bytes, room for one header and one c_int, so
        // the first header is not null and its data is in bounds.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(mem::size_of::<c_int>() as u32) as _;
            libc::CMSG_DATA(c)
                .cast::<c_int>()
                .write_unaligned(fd.as_raw_fd());
        }
    }
    loop {
        // SAFETY: `msg` points at `iov`, `frame` and `control`, all alive for the call.
        let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
        if n >= 0 {
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// A frame body and the fd sent with it.
pub type Received = (Zeroizing<Vec<u8>>, Option<OwnedFd>);

/// Splits a stream into frames and the fds sent with them. After an error every `next` fails:
/// drop the peer.
pub struct FdFrameReader {
    frames: FrameReader,
    buf: Zeroizing<Vec<u8>>,
    /// Fds by the index of their frame.
    fds: VecDeque<(u64, OwnedFd)>,
    given: u64,
    failed: Option<FrameError>,
}

impl Default for FdFrameReader {
    fn default() -> Self {
        FdFrameReader {
            frames: FrameReader::new(),
            buf: Zeroizing::new(vec![0; READ_BUF]),
            fds: VecDeque::new(),
            given: 0,
            failed: None,
        }
    }
}

impl FdFrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// One `recvmsg`; 0 is the end of the stream. A frame or fd error waits for `next`.
    pub fn read_from(&mut self, sock: BorrowedFd<'_>) -> io::Result<usize> {
        let mut iov = libc::iovec {
            iov_base: self.buf.as_mut_ptr().cast(),
            iov_len: self.buf.len(),
        };
        let mut control = [0u64; SPACE_WORDS];
        // SAFETY: an all-zero msghdr is an empty message.
        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = SPACE as _;
        let n = loop {
            // SAFETY: `msg` points at `iov`, `self.buf` and `control`, all alive for the call.
            let n = unsafe { libc::recvmsg(sock.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC) };
            if n >= 0 {
                break n as usize;
            }
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        };
        let mut fds = Vec::new();
        // SAFETY: the kernel wrote `msg_controllen` bytes of headers into `control`, and each
        // SCM_RIGHTS payload holds fds it just installed here, owned by nothing else.
        unsafe {
            let mut c = libc::CMSG_FIRSTHDR(&msg);
            while !c.is_null() {
                if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                    let data = libc::CMSG_DATA(c).cast::<c_int>();
                    let len = ((*c).cmsg_len as usize).saturating_sub(libc::CMSG_LEN(0) as usize);
                    for i in 0..len / mem::size_of::<c_int>() {
                        fds.push(OwnedFd::from_raw_fd(data.add(i).read_unaligned()));
                    }
                }
                c = libc::CMSG_NXTHDR(&msg, c);
            }
        }
        if self.failed.is_some() {
            return Ok(n);
        }
        let before = self.frames.begun();
        if let Err(e) = self.frames.push(&self.buf[..n]) {
            self.failed = Some(e);
            return Ok(n);
        }
        let begun = self.frames.begun();
        // A read ends after the segment that carried an fd, so its frame is the last begun.
        if msg.msg_flags & libc::MSG_CTRUNC != 0
            || fds.len() > 1
            || (begun == before && !fds.is_empty())
        {
            self.failed = Some(FrameError::Fd);
        } else if let Some(fd) = fds.pop() {
            self.fds.push_back((begun - 1, fd));
        }
        Ok(n)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<Option<Received>, FrameError> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let Some(body) = self.frames.next_frame() else {
            return Ok(None);
        };
        let index = self.given;
        self.given += 1;
        let fd = match self.fds.front() {
            Some((i, _)) if *i == index => self.fds.pop_front().map(|(_, fd)| fd),
            _ => None,
        };
        Ok(Some((body, fd)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::fd::{AsFd, RawFd};
    use std::os::unix::net::UnixStream;

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut f = (body.len() as u32).to_le_bytes().to_vec();
        f.extend_from_slice(body);
        f
    }

    fn pipe_with(text: &[u8]) -> OwnedFd {
        let (r, mut w) = io::pipe().unwrap();
        w.write_all(text).unwrap();
        r.into()
    }

    fn read_frames(r: &mut FdFrameReader, sock: &UnixStream, want: usize) -> Vec<Received> {
        let mut frames = Vec::new();
        while frames.len() < want {
            assert_ne!(
                r.read_from(sock.as_fd()).unwrap(),
                0,
                "the stream ended early"
            );
            while let Some(f) = r.next().unwrap() {
                frames.push(f);
            }
        }
        frames
    }

    /// A `sendmsg` carrying every fd of `fds` in one SCM_RIGHTS message.
    fn send_raw(sock: &UnixStream, data: &[u8], fds: &[RawFd]) {
        let payload = mem::size_of_val(fds) as u32;
        let mut control = [0u64; 8];
        let iov = libc::iovec {
            iov_base: data.as_ptr() as *mut _,
            iov_len: data.len(),
        };
        // SAFETY: an all-zero msghdr is an empty message.
        let mut msg: libc::msghdr = unsafe { mem::zeroed() };
        msg.msg_iov = &iov as *const _ as *mut _;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        // SAFETY: CMSG_SPACE is arithmetic on its argument.
        msg.msg_controllen = unsafe { libc::CMSG_SPACE(payload) } as _;
        assert!(msg.msg_controllen <= mem::size_of_val(&control));
        // SAFETY: the control buffer holds one header and `payload` bytes, checked above.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(payload) as _;
            std::ptr::copy_nonoverlapping(
                fds.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(c),
                payload as usize,
            );
        }
        // SAFETY: `msg` points at `iov` and `control`, both alive for the call.
        let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &msg, 0) };
        assert_eq!(n, data.len() as isize, "{}", io::Error::last_os_error());
    }

    fn set_buffers(sock: &UnixStream, bytes: c_int) {
        for opt in [libc::SO_RCVBUF, libc::SO_SNDBUF] {
            // SAFETY: `bytes` is a c_int that outlives the call.
            let r = unsafe {
                libc::setsockopt(
                    sock.as_raw_fd(),
                    libc::SOL_SOCKET,
                    opt,
                    (&bytes as *const c_int).cast(),
                    mem::size_of::<c_int>() as u32,
                )
            };
            assert_eq!(r, 0, "{}", io::Error::last_os_error());
        }
    }

    #[test]
    fn a_frame_and_its_fd_arrive_together() {
        let (a, b) = UnixStream::pair().unwrap();
        let pipe = pipe_with(b"through the pipe");
        let first = frame(b"A");
        let second = frame(b"B");
        assert_eq!(send(a.as_fd(), &first, None).unwrap(), first.len());
        assert_eq!(
            send(a.as_fd(), &second, Some(pipe.as_fd())).unwrap(),
            second.len()
        );
        drop(pipe);
        let mut r = FdFrameReader::new();
        let mut frames = read_frames(&mut r, &b, 2).into_iter();
        let (body, fd) = frames.next().unwrap();
        assert_eq!((&body[..], fd.is_none()), (&b"A"[..], true));
        let (body, fd) = frames.next().unwrap();
        assert_eq!(&body[..], b"B");
        let mut text = String::new();
        std::fs::File::from(fd.expect("B carries the pipe"))
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "through the pipe");
    }

    /// A pipe's read end to send, and its write end, which sees EPIPE once every read end closes.
    fn watched_pipe() -> (OwnedFd, io::PipeWriter) {
        let (r, w) = io::pipe().unwrap();
        (r.into(), w)
    }

    fn assert_closed(w: &mut io::PipeWriter) {
        let e = w.write(b"x").unwrap_err();
        assert_eq!(
            e.raw_os_error(),
            Some(libc::EPIPE),
            "the read end is still open"
        );
    }

    #[test]
    fn two_fds_on_one_frame_are_refused_and_closed() {
        let (a, b) = UnixStream::pair().unwrap();
        let ((one, mut one_w), (two, mut two_w)) = (watched_pipe(), watched_pipe());
        send_raw(&a, &frame(b"A"), &[one.as_raw_fd(), two.as_raw_fd()]);
        drop((one, two));
        let mut r = FdFrameReader::new();
        assert!(r.read_from(b.as_fd()).unwrap() > 0);
        assert_eq!(r.next().map(|f| f.is_some()), Err(FrameError::Fd));
        assert_eq!(r.next().map(|f| f.is_some()), Err(FrameError::Fd));
        assert_closed(&mut one_w);
        assert_closed(&mut two_w);
    }

    #[test]
    fn three_fds_overflowing_the_control_buffer_are_all_closed() {
        let (a, b) = UnixStream::pair().unwrap();
        let (reads, mut writers): (Vec<OwnedFd>, Vec<_>) = (0..3).map(|_| watched_pipe()).unzip();
        let raw: Vec<RawFd> = reads.iter().map(|r| r.as_raw_fd()).collect();
        send_raw(&a, &frame(b"A"), &raw);
        drop(reads);
        let mut r = FdFrameReader::new();
        assert!(r.read_from(b.as_fd()).unwrap() > 0);
        assert_eq!(r.next().map(|f| f.is_some()), Err(FrameError::Fd));
        for w in &mut writers {
            assert_closed(w);
        }
    }

    #[test]
    fn an_fd_inside_a_frame_is_refused_and_closed() {
        let (a, b) = UnixStream::pair().unwrap();
        let whole = frame(b"a frame sent in two parts");
        let mut r = FdFrameReader::new();
        send(a.as_fd(), &whole[..6], None).unwrap();
        r.read_from(b.as_fd()).unwrap();
        let (pipe, mut pipe_w) = watched_pipe();
        send(a.as_fd(), &whole[6..], Some(pipe.as_fd())).unwrap();
        drop(pipe);
        pipe_w.write_all(b"x").unwrap();
        r.read_from(b.as_fd()).unwrap();
        assert_eq!(r.next().map(|f| f.is_some()), Err(FrameError::Fd));
        assert_closed(&mut pipe_w);
    }

    #[test]
    fn a_long_frame_read_in_pieces_keeps_its_fd() {
        let (a, b) = UnixStream::pair().unwrap();
        set_buffers(&a, 4096);
        set_buffers(&b, 4096);
        let body: Vec<u8> = (0..40_000u32).map(|i| i as u8).collect();
        let whole = frame(&body);
        let pipe = pipe_with(b"long");
        let sender = std::thread::spawn(move || send(a.as_fd(), &whole, Some(pipe.as_fd())));
        let mut r = FdFrameReader::new();
        let mut reads = 0;
        let mut frames = Vec::new();
        while frames.is_empty() {
            assert_ne!(r.read_from(b.as_fd()).unwrap(), 0);
            reads += 1;
            while let Some(f) = r.next().unwrap() {
                frames.push(f);
            }
        }
        assert_eq!(sender.join().unwrap().unwrap(), 40_004);
        assert!(reads > 1, "the frame came in {reads} read");
        assert_eq!(frames.len(), 1);
        let (got, fd) = frames.pop().unwrap();
        assert_eq!(*got, body);
        let mut text = String::new();
        std::fs::File::from(fd.expect("the frame keeps its fd"))
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "long");
    }
}
