//! The link to reprise-login over `FRONT_SOCKET`.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use repriselogin::fdpass::{self, FdFrameReader};
use repriselogin::frame;
use repriselogin::front::{FrontEvent, FrontRequest};
use tokio::io::Interest;
use tokio::net::UnixStream;

/// How long a send waits for reprise-login to read before the link counts as lost.
const SEND_WAIT: Duration = Duration::from_secs(2);

/// Sends whole frames: a frame never waits on the runtime halfway, so two never interleave.
#[derive(Clone)]
pub struct LinkSender {
    sock: Rc<UnixStream>,
}

pub struct Events {
    sock: Rc<UnixStream>,
    reader: FdFrameReader,
}

/// Connects to reprise-login at `path` and says `Hello`.
pub async fn connect_to(path: &Path) -> io::Result<(LinkSender, Events)> {
    let sock = Rc::new(UnixStream::connect(path).await?);
    let sender = LinkSender { sock: sock.clone() };
    sender.send(&FrontRequest::Hello, None)?;
    let events = Events {
        sock,
        reader: FdFrameReader::new(),
    };
    Ok((sender, events))
}

impl LinkSender {
    /// Sends `req`, with `fd` attached to its first byte. A failed write closes the link, since
    /// it may leave half a frame on it.
    pub fn send(&self, req: &FrontRequest, fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
        let frame = frame::encode(req).map_err(io::Error::other)?;
        let written = self.write(&frame, fd);
        if written.is_err() {
            // SAFETY: the fd is the open socket `self.sock` owns.
            unsafe { libc::shutdown(self.sock.as_raw_fd(), libc::SHUT_RDWR) };
        }
        written
    }

    fn write(&self, frame: &[u8], mut fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
        let mut sent = 0;
        while sent < frame.len() {
            match fdpass::send(self.sock.as_fd(), &frame[sent..], fd) {
                Ok(n) => {
                    sent += n;
                    fd = None;
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    wait_writable(self.sock.as_fd(), SEND_WAIT)?
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

fn wait_writable(fd: BorrowedFd<'_>, wait: Duration) -> io::Result<()> {
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };
    loop {
        // SAFETY: `poll` is one valid pollfd for the call.
        let n = unsafe { libc::poll(&mut poll, 1, wait.as_millis() as libc::c_int) };
        match n {
            0 => return Err(io::ErrorKind::TimedOut.into()),
            n if n > 0 => return Ok(()),
            _ => {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
        }
    }
}

impl Events {
    /// The next event; `None` once the link is lost.
    pub async fn next(&mut self) -> Option<FrontEvent> {
        loop {
            match self.reader.next() {
                Ok(Some((body, _fd))) => {
                    return frame::decode(&body)
                        .inspect_err(|e| log::warn!("reprise-login sent {e}"))
                        .ok();
                }
                Ok(None) => {}
                Err(e) => {
                    log::warn!("reprise-login: {e}");
                    return None;
                }
            }
            if self.sock.readable().await.is_err() {
                return None;
            }
            let read = (self.sock).try_io(Interest::READABLE, || {
                self.reader.read_from(self.sock.as_fd())
            });
            match read {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    log::warn!("reprise-login: {e}");
                    return None;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use repriselogin::front::NlaUser;
    use std::io::Write;
    use std::os::fd::OwnedFd;
    use zeroize::Zeroizing;

    #[tokio::test]
    async fn the_front_says_hello_first_and_reads_the_daemons_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let (sender, mut events) = connect_to(&path).await.unwrap();
        let (mut peer, _) = daemon.accept().unwrap();
        let mut reader = FdFrameReader::new();
        let next = |reader: &mut FdFrameReader| loop {
            if let Some((body, fd)) = reader.next().unwrap() {
                return (frame::decode::<FrontRequest>(&body).unwrap(), fd);
            }
            assert_ne!(reader.read_from(peer.as_fd()).unwrap(), 0);
        };
        let (hello, fd) = next(&mut reader);
        assert_eq!(hello, FrontRequest::Hello);
        assert!(fd.is_none());

        let (pipe, _w) = io::pipe().unwrap();
        let pipe = OwnedFd::from(pipe);
        sender
            .send(&FrontRequest::Gone { conn: 3 }, Some(pipe.as_fd()))
            .unwrap();
        let (gone, fd) = next(&mut reader);
        assert_eq!(gone, FrontRequest::Gone { conn: 3 });
        assert!(fd.is_some(), "the fd travels with its frame");

        let users = vec![NlaUser {
            user: "anna".into(),
            nt_hash: Zeroizing::new("8846F7EAEE8FB117AD06BDD830B7586C".into()),
        }];
        peer.write_all(
            &frame::encode(&FrontEvent::Passwords {
                users: users.clone(),
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(events.next().await, Some(FrontEvent::Passwords { users }));
        drop(peer);
        assert_eq!(events.next().await, None, "a closed link ends the events");
    }

    #[tokio::test]
    async fn a_send_that_fails_drops_the_link() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let (sender, mut events) = connect_to(&path).await.unwrap();
        let (_unread, _) = daemon.accept().unwrap();
        let failed = (0..1_000_000)
            .find_map(|_| sender.send(&FrontRequest::Gone { conn: 1 }, None).err())
            .expect("a daemon that never reads fills the socket");
        assert_eq!(failed.kind(), io::ErrorKind::TimedOut);
        assert_eq!(
            events.next().await,
            None,
            "the link is closed after a failed send"
        );
    }
}
