//! [`HandOverIntake`]: connections reprise-login hands over on the user manager's socket, each
//! link told the session's viewer count.

use crate::control::Status;
use crate::intake::{Arrival, Intake};
use repriselogin::fdpass::FdFrameReader;
use repriselogin::frame;
use repriselogin::handover::{IntakeMessage, IntakeReport};
use std::io;
use std::mem;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::Duration;
use tokio::io::Interest;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, watch};
use tokio::task::{JoinHandle, JoinSet};

/// The fd systemd passes the first socket on.
const LISTEN_FD: i32 = 3;

/// Hand-overs from links whose peer runs as `allowed_uid`.
pub struct HandOverIntake {
    arrivals: mpsc::Receiver<Arrival>,
    links: JoinHandle<()>,
}

impl HandOverIntake {
    pub fn new(
        listener: UnixListener,
        allowed_uid: u32,
        status: watch::Receiver<Status>,
    ) -> HandOverIntake {
        let (sender, arrivals) = mpsc::channel(1);
        let links = tokio::spawn(accept(listener, allowed_uid, status, sender));
        HandOverIntake { arrivals, links }
    }
}

impl Drop for HandOverIntake {
    fn drop(&mut self) {
        self.links.abort();
    }
}

impl Intake for HandOverIntake {
    async fn next(&mut self) -> Arrival {
        match self.arrivals.recv().await {
            Some(arrival) => arrival,
            None => std::future::pending().await,
        }
    }
}

/// Closes the hand-overs waiting on `listener`: reprise-login refuses their clients at once, and
/// the socket starts the server again only for the next hand-over.
pub fn refuse_waiting(listener: &std::os::unix::net::UnixListener) {
    while let Ok((link, _)) = listener.accept() {
        drop(link);
    }
}

/// The socket systemd passed this process, in non-blocking mode.
pub fn activated() -> io::Result<std::os::unix::net::UnixListener> {
    let pid = std::env::var("LISTEN_PID").ok();
    let fds = std::env::var("LISTEN_FDS").ok();
    if !passed_one(pid.as_deref(), fds.as_deref(), std::process::id()) {
        return Err(io::Error::other("systemd passed no hand-over socket"));
    }
    // SAFETY: fcntl takes no pointers; an fd that is not open fails with EBADF.
    if unsafe { libc::fcntl(LISTEN_FD, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: systemd passed fd 3 to this process for its socket, and nothing else owns it.
    let listener = unsafe { std::os::unix::net::UnixListener::from_raw_fd(LISTEN_FD) };
    listener.set_nonblocking(true)?;
    Ok(listener)
}

/// Whether `LISTEN_PID` and `LISTEN_FDS` pass one socket to the process `own`.
fn passed_one(pid: Option<&str>, fds: Option<&str>, own: u32) -> bool {
    pid.and_then(|p| p.parse::<u32>().ok()) == Some(own) && fds == Some("1")
}

async fn accept(
    listener: UnixListener,
    allowed_uid: u32,
    status: watch::Receiver<Status>,
    arrivals: mpsc::Sender<Arrival>,
) {
    let mut links = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => match stream.peer_cred() {
                    Ok(peer) if peer.uid() == allowed_uid => {
                        links.spawn(link(stream, status.clone(), arrivals.clone()));
                    }
                    Ok(peer) => log::warn!("refused a hand-over link from uid {}", peer.uid()),
                    Err(e) => log::warn!("refused a hand-over link: no peer credentials: {e}"),
                },
                Err(e) => {
                    log::warn!("hand-over accept: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = links.join_next() => {}
        }
    }
}

async fn link(
    stream: UnixStream,
    status: watch::Receiver<Status>,
    arrivals: mpsc::Sender<Arrival>,
) {
    if let Err(e) = serve_link(&stream, status, &arrivals).await {
        log::warn!("dropped a hand-over link: {e}");
    }
}

/// Passes each hand-over on and reports the viewer count until the peer closes the link.
async fn serve_link(
    stream: &UnixStream,
    mut status: watch::Receiver<Status>,
    arrivals: &mpsc::Sender<Arrival>,
) -> io::Result<()> {
    let mut count = status.borrow_and_update().clients.len();
    report(stream, count).await?;
    let mut reader = FdFrameReader::new();
    loop {
        tokio::select! {
            ready = stream.readable() => {
                ready?;
                match stream.try_io(Interest::READABLE, || reader.read_from(stream.as_fd())) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
                while let Some((body, fd)) = reader.next().map_err(io::Error::other)? {
                    if arrivals.send(arrival(&body, fd)?).await.is_err() {
                        return Ok(());
                    }
                }
            }
            changed = status.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                let now = status.borrow_and_update().clients.len();
                if now != count {
                    count = now;
                    report(stream, count).await?;
                }
            }
        }
    }
}

fn arrival(body: &[u8], fd: Option<OwnedFd>) -> io::Result<Arrival> {
    let IntakeMessage::HandOver(handover) = frame::decode(body).map_err(io::Error::other)?;
    let fd = fd.ok_or_else(|| io::Error::other("a hand-over came without its socket"))?;
    let tcp = tcp_stream(fd)?;
    log::debug!(
        "connection {} from {} handed over for {}",
        handover.conn,
        handover.peer,
        handover.user
    );
    Ok(Arrival::HandedOver { tcp, handover })
}

/// `fd` as a TCP stream, or an error, closing it, when it is any other file.
fn tcp_stream(fd: OwnedFd) -> io::Result<std::net::TcpStream> {
    let kind = (
        socket_option(fd.as_fd(), libc::SO_TYPE)?,
        socket_option(fd.as_fd(), libc::SO_DOMAIN)?,
        socket_option(fd.as_fd(), libc::SO_PROTOCOL)?,
    );
    match kind {
        (libc::SOCK_STREAM, libc::AF_INET | libc::AF_INET6, libc::IPPROTO_TCP) => Ok(fd.into()),
        _ => Err(io::Error::other("a hand-over's socket is not a TCP socket")),
    }
}

fn socket_option(fd: BorrowedFd<'_>, option: libc::c_int) -> io::Result<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut len = mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: `value` and `len` outlive the call, and `len` is the size of `value`.
    let r = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

async fn report(stream: &UnixStream, count: usize) -> io::Result<()> {
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    let frame = frame::encode(&IntakeReport::Viewers { count }).map_err(io::Error::other)?;
    let mut sent = 0;
    while sent < frame.len() {
        stream.writable().await?;
        match stream.try_write(&frame[sent..]) {
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Client;
    use repriselogin::fdpass;
    use repriselogin::handover::{Cipher, DirectionKeys, HandOver, TlsState, TlsVersion};
    use std::io::Write;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use tokio::io::AsyncReadExt;
    use zeroize::Zeroizing;

    const WAIT: Duration = Duration::from_secs(5);

    struct Linked {
        intake: HandOverIntake,
        link: UnixStream,
        status: watch::Sender<Status>,
        _dir: tempfile::TempDir,
    }

    fn own_uid() -> u32 {
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() }
    }

    async fn linked(allowed_uid: u32) -> Linked {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handover.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (status, viewers) = watch::channel(Status::default());
        let intake = HandOverIntake::new(listener, allowed_uid, viewers);
        let link = UnixStream::connect(&path).await.unwrap();
        Linked {
            intake,
            link,
            status,
            _dir: dir,
        }
    }

    /// The next report on `link`, or `None` once the intake closed it.
    async fn read_report(link: &mut UnixStream) -> Option<IntakeReport> {
        let read = async {
            let mut len = [0; 4];
            link.read_exact(&mut len).await.ok()?;
            let mut body = vec![0; u32::from_le_bytes(len) as usize];
            link.read_exact(&mut body).await.ok()?;
            Some(frame::decode(&body).unwrap())
        };
        tokio::time::timeout(WAIT, read)
            .await
            .expect("a report or the end")
    }

    async fn closed(link: &mut UnixStream) {
        while let Some(report) = read_report(link).await {
            assert!(matches!(report, IntakeReport::Viewers { .. }), "{report:?}");
        }
    }

    async fn stays_pending(intake: &mut HandOverIntake) {
        let next = tokio::time::timeout(Duration::from_millis(200), intake.next()).await;
        assert!(next.is_err(), "no arrival");
    }

    fn keys() -> DirectionKeys {
        DirectionKeys {
            seq: 0,
            key: Zeroizing::new(vec![1; 16]),
            iv: Zeroizing::new(vec![2; 12]),
            secret: None,
        }
    }

    fn hand_over(peer: SocketAddr) -> Vec<u8> {
        let message = IntakeMessage::HandOver(Box::new(HandOver {
            conn: 9,
            user: "anna".into(),
            peer,
            security: 1,
            steps: Vec::new(),
            tls: TlsState {
                version: TlsVersion::Tls13,
                cipher: Cipher::Aes128Gcm,
                tx: keys(),
                rx: keys(),
                record_limit: 1 << 24,
            },
        }));
        frame::encode(&message).unwrap().to_vec()
    }

    fn send(link: &UnixStream, frame: &[u8], fd: Option<BorrowedFd<'_>>) {
        assert_eq!(fdpass::send(link.as_fd(), frame, fd).unwrap(), frame.len());
    }

    /// Both ends of a loopback TCP connection: the client's, then the accepted one.
    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        (client, accepted)
    }

    #[tokio::test]
    async fn a_hand_over_frame_with_its_fd_yields_an_arrival() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        let peer = client.local_addr().unwrap();
        send(&l.link, &hand_over(peer), Some(accepted.as_fd()));
        drop(accepted);
        let arrival = tokio::time::timeout(WAIT, l.intake.next()).await.unwrap();
        let Arrival::HandedOver { tcp, handover } = arrival else {
            panic!("a hand-over arrives");
        };
        assert_eq!(handover.user, "anna");
        assert_eq!(handover.conn, 9);
        assert_eq!(tcp.peer_addr().unwrap(), peer);
    }

    #[tokio::test]
    async fn a_peer_that_is_not_the_allowed_uid_is_dropped() {
        let mut l = linked(own_uid() + 1).await;
        let (client, accepted) = tcp_pair();
        let _ = fdpass::send(
            l.link.as_fd(),
            &hand_over(client.local_addr().unwrap()),
            Some(accepted.as_fd()),
        );
        assert_eq!(read_report(&mut l.link).await, None);
        stays_pending(&mut l.intake).await;
    }

    #[tokio::test]
    async fn a_hand_over_without_an_fd_drops_the_link() {
        let mut l = linked(own_uid()).await;
        let peer = "192.0.2.1:50000".parse().unwrap();
        send(&l.link, &hand_over(peer), None);
        closed(&mut l.link).await;
        stays_pending(&mut l.intake).await;
    }

    #[tokio::test]
    async fn an_fd_that_is_not_a_tcp_socket_drops_the_link() {
        let mut l = linked(own_uid()).await;
        let (pipe, mut pipe_w) = io::pipe().unwrap();
        let peer = "192.0.2.1:50000".parse().unwrap();
        send(&l.link, &hand_over(peer), Some(pipe.as_fd()));
        drop(pipe);
        closed(&mut l.link).await;
        stays_pending(&mut l.intake).await;
        let e = pipe_w.write(b"x").unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EPIPE), "the pipe is closed");
    }

    #[tokio::test]
    async fn the_link_reports_the_viewer_count_on_connect_and_on_change() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let viewer = |address: &str| Client {
            address: address.into(),
            user: "anna".into(),
            since: 0,
        };
        l.status.send_modify(|s| {
            s.clients = vec![viewer("192.0.2.1:50000"), viewer("192.0.2.2:50000")]
        });
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 2 })
        );
    }

    #[test]
    fn the_activated_socket_is_systemds_one_socket_for_this_process() {
        assert!(passed_one(Some("42"), Some("1"), 42));
        assert!(!passed_one(Some("41"), Some("1"), 42));
        assert!(!passed_one(Some("42"), Some("2"), 42));
        assert!(!passed_one(None, Some("1"), 42));
        assert!(!passed_one(Some("42"), None, 42));
    }

    #[test]
    fn refused_hand_overs_are_closed_and_the_listener_left_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handover.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut waiting: Vec<_> = (0..2)
            .map(|_| std::os::unix::net::UnixStream::connect(&path).unwrap())
            .collect();
        refuse_waiting(&listener);
        for link in &mut waiting {
            link.set_read_timeout(Some(WAIT)).unwrap();
            assert_eq!(std::io::Read::read(link, &mut [0]).unwrap(), 0, "closed");
        }
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "none left for systemd to start the server again for"
        );
    }
}
