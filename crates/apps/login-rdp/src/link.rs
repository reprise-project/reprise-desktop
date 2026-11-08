//! The link to reprise-login over `FRONT_SOCKET`.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use repriselogin::fdpass::{self, FdFrameReader};
use repriselogin::frame;
use repriselogin::front::{FrontEvent, FrontRequest};
use tokio::io::Interest;
use tokio::net::UnixStream;
use tokio::sync::{mpsc, oneshot};
use zeroize::Zeroizing;

/// How long a send waits for reprise-login to read before the link counts as lost.
const SEND_WAIT: Duration = Duration::from_secs(2);

/// A whole frame, the fd that travels with its first byte, and who hears once it is written.
struct Outgoing {
    frame: Zeroizing<Vec<u8>>,
    fd: Option<OwnedFd>,
    written: Option<oneshot::Sender<()>>,
}

/// Resolves once its frame is on the link; fails if the link is lost first.
pub type Written = oneshot::Receiver<()>;

/// Queues frames for the link's one writer, which sends them whole and in order.
#[derive(Clone)]
pub struct LinkSender {
    queue: mpsc::UnboundedSender<Outgoing>,
}

pub struct Events {
    sock: Arc<UnixStream>,
    reader: FdFrameReader,
}

/// Connects to reprise-login at `path` and says `Hello`.
pub async fn connect_to(path: &Path) -> io::Result<(LinkSender, Events)> {
    let sock = Arc::new(UnixStream::connect(path).await?);
    let (queue, outgoing) = mpsc::unbounded_channel();
    tokio::spawn(write_frames(sock.clone(), outgoing));
    let sender = LinkSender { queue };
    sender.send(&FrontRequest::Hello, None)?;
    let events = Events {
        sock,
        reader: FdFrameReader::new(),
    };
    Ok((sender, events))
}

impl LinkSender {
    /// Queues `req`, with a copy of `fd` attached to its first byte. Fails once the link is lost.
    pub fn send(&self, req: &FrontRequest, fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
        self.queue(req, fd, None)
    }

    /// As `send`, and says when the frame is written.
    pub fn send_tracked(
        &self,
        req: &FrontRequest,
        fd: Option<BorrowedFd<'_>>,
    ) -> io::Result<Written> {
        let (tell, written) = oneshot::channel();
        self.queue(req, fd, Some(tell))?;
        Ok(written)
    }

    fn queue(
        &self,
        req: &FrontRequest,
        fd: Option<BorrowedFd<'_>>,
        written: Option<oneshot::Sender<()>>,
    ) -> io::Result<()> {
        let encoded = frame::encode(req).map_err(io::Error::other)?;
        // `encode` reserves MAX_FRAME; a queued frame holds only its own length.
        let frame = Zeroizing::new(encoded.to_vec());
        let fd = fd.map(|fd| fd.try_clone_to_owned()).transpose()?;
        let outgoing = Outgoing { frame, fd, written };
        (self.queue.send(outgoing)).map_err(|_| io::ErrorKind::NotConnected.into())
    }
}

/// Sends the queued frames until the senders are gone. A write that fails or waits `SEND_WAIT`
/// closes the link, since it may leave half a frame on it.
async fn write_frames(sock: Arc<UnixStream>, mut outgoing: mpsc::UnboundedReceiver<Outgoing>) {
    while let Some(Outgoing { frame, fd, written }) = outgoing.recv().await {
        if let Err(e) = write(&sock, &frame, fd.as_ref().map(|fd| fd.as_fd())).await {
            log::warn!("reprise-login: {e}");
            shut_down(&sock);
            return;
        }
        if let Some(written) = written {
            let _ = written.send(());
        }
    }
}

/// Closes the link both ways while the reader or writer still holds the socket.
fn shut_down(sock: &UnixStream) {
    // SAFETY: the fd is the open socket `sock` owns.
    unsafe { libc::shutdown(sock.as_raw_fd(), libc::SHUT_RDWR) };
}

async fn write(sock: &UnixStream, frame: &[u8], mut fd: Option<BorrowedFd<'_>>) -> io::Result<()> {
    let mut sent = 0;
    while sent < frame.len() {
        let wrote = sock.try_io(Interest::WRITABLE, || {
            fdpass::send(sock.as_fd(), &frame[sent..], fd)
        });
        match wrote {
            Ok(n) => {
                sent += n;
                fd = None;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let writable = tokio::time::timeout(SEND_WAIT, sock.writable()).await;
                writable.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

impl Events {
    /// The next event; `None` once the link is lost, which it closes so the writer stops too.
    pub async fn next(&mut self) -> Option<FrontEvent> {
        let event = self.read().await;
        if event.is_none() {
            shut_down(&self.sock);
        }
        event
    }

    async fn read(&mut self) -> Option<FrontEvent> {
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
    use repriselogin::front::{NlaUser, Secret};
    use std::io::{Read, Write};
    use tokio::io::AsyncWriteExt;

    async fn next(
        peer: &UnixStream,
        reader: &mut FdFrameReader,
    ) -> (FrontRequest, Option<OwnedFd>) {
        loop {
            if let Some((body, fd)) = reader.next().unwrap() {
                return (frame::decode(&body).unwrap(), fd);
            }
            peer.readable().await.unwrap();
            match peer.try_io(Interest::READABLE, || reader.read_from(peer.as_fd())) {
                Ok(n) => assert_ne!(n, 0),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            }
        }
    }

    #[tokio::test]
    async fn the_front_says_hello_first_and_reads_the_daemons_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = tokio::net::UnixListener::bind(&path).unwrap();
        let (sender, mut events) = connect_to(&path).await.unwrap();
        let (mut peer, _) = daemon.accept().await.unwrap();
        let mut reader = FdFrameReader::new();
        let (hello, fd) = next(&peer, &mut reader).await;
        assert_eq!(hello, FrontRequest::Hello);
        assert!(fd.is_none());

        let (pipe, _w) = io::pipe().unwrap();
        let pipe = OwnedFd::from(pipe);
        let written = sender
            .send_tracked(&FrontRequest::Gone { conn: 3 }, Some(pipe.as_fd()))
            .unwrap();
        let (gone, fd) = next(&peer, &mut reader).await;
        assert_eq!(gone, FrontRequest::Gone { conn: 3 });
        assert!(fd.is_some(), "the fd travels with its frame");
        assert_eq!(written.await, Ok(()), "a frame on the link says so");

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
        .await
        .unwrap();
        assert_eq!(events.next().await, Some(FrontEvent::Passwords { users }));
        drop(peer);
        assert_eq!(events.next().await, None, "a closed link ends the events");
    }

    #[tokio::test]
    async fn an_undecodable_frame_from_the_daemon_closes_the_link_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = tokio::net::UnixListener::bind(&path).unwrap();
        let (_sender, mut events) = connect_to(&path).await.unwrap();
        let (mut peer, _) = daemon.accept().await.unwrap();
        let mut reader = FdFrameReader::new();
        assert_eq!(next(&peer, &mut reader).await.0, FrontRequest::Hello);

        let garbage = [1, 0, 0, 0, b'{'];
        assert!(frame::decode::<FrontEvent>(&garbage[4..]).is_err());
        peer.write_all(&garbage).await.unwrap();
        assert_eq!(events.next().await, None);
        let read = tokio::time::timeout(SEND_WAIT, async {
            let mut byte = [0; 1];
            loop {
                peer.readable().await.unwrap();
                match peer.try_read(&mut byte) {
                    Ok(n) => return n,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => panic!("{e}"),
                }
            }
        });
        assert_eq!(
            read.await,
            Ok(0),
            "the daemon sees the link closed while the front still holds its sender"
        );
    }

    #[tokio::test]
    async fn a_sent_fd_outlives_the_callers_copy() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = tokio::net::UnixListener::bind(&path).unwrap();
        let (sender, _events) = connect_to(&path).await.unwrap();
        let (peer, _) = daemon.accept().await.unwrap();
        let mut reader = FdFrameReader::new();
        assert_eq!(next(&peer, &mut reader).await.0, FrontRequest::Hello);

        let (pipe, mut w) = io::pipe().unwrap();
        let pipe = OwnedFd::from(pipe);
        sender
            .send(&FrontRequest::Gone { conn: 3 }, Some(pipe.as_fd()))
            .unwrap();
        drop(pipe);
        let (_, fd) = next(&peer, &mut reader).await;
        let mut received = std::fs::File::from(fd.expect("the fd travels with its frame"));
        w.write_all(b"x").unwrap();
        let mut got = [0; 1];
        received.read_exact(&mut got).unwrap();
        assert_eq!(
            &got, b"x",
            "the received fd is the caller's pipe, still open"
        );
    }

    /// Shrinks a socket buffer so that a few frames fill it whatever the host's default.
    fn shrink(fd: BorrowedFd<'_>, which: libc::c_int) {
        let size: libc::c_int = 4096;
        // SAFETY: `fd` is an open socket and `size` outlives the call.
        let set = unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                which,
                (&raw const size).cast(),
                size_of_val(&size) as libc::socklen_t,
            )
        };
        assert_eq!(set, 0, "{}", io::Error::last_os_error());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_daemon_does_not_stall_other_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let daemon = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let (sender, mut events) = connect_to(&path).await.unwrap();
        let (unread, _) = daemon.accept().unwrap();
        shrink(events.sock.as_fd(), libc::SO_SNDBUF);
        shrink(unread.as_fd(), libc::SO_RCVBUF);
        let big = FrontRequest::Authenticate {
            conn: 1,
            peer: "192.0.2.1:50000".parse().unwrap(),
            user: "u".repeat(60_000),
            secret: Secret::Password(Zeroizing::new("secret".into())),
        };
        let started = std::time::Instant::now();
        for _ in 0..8 {
            sender.send(&big, None).unwrap();
        }
        let last = sender.send_tracked(&big, None).unwrap();
        assert!(
            started.elapsed() < SEND_WAIT / 4,
            "sends to a daemon that never reads return at once"
        );
        let lost = tokio::time::timeout(SEND_WAIT * 4, events.next()).await;
        assert_eq!(
            lost,
            Ok(None),
            "the link is closed once a send waits SEND_WAIT"
        );
        assert!(last.await.is_err(), "a frame never written says so");
        assert!(sender.send(&FrontRequest::Gone { conn: 1 }, None).is_err());
    }
}
