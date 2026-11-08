//! [`HandOverIntake`]: connections reprise-login hands over on the user manager's socket, each
//! link told the session's viewer count and which of its hand-overs the server took.

use crate::control::Status;
use crate::intake::{Arrival, Intake};
use repriselogin::fdpass::FdFrameReader;
use repriselogin::frame;
use repriselogin::handover::{IntakeMessage, IntakeReport};
use std::collections::VecDeque;
use std::io;
use std::mem;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::time::Duration;
use tokio::io::Interest;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;

/// The fd systemd passes the first socket on.
const LISTEN_FD: i32 = 3;

/// Conns a server remembers taking: reprise-login sends one again only within its login's 8 s.
const TAKEN_KEPT: usize = 64;

/// Hand-overs from links whose peer runs as `allowed_uid`. Dropping it asks each link to report
/// no viewers and close, best effort: a link whose task never runs again closes unreported.
pub struct HandOverIntake {
    arrivals: mpsc::Receiver<Handed>,
    /// The last [`TAKEN_KEPT`] conns taken, oldest first.
    taken: VecDeque<u64>,
    /// Taken by [`HandOverIntake::take_during`], not yet served.
    early: VecDeque<Arrival>,
}

/// A hand-over's conn and arrival, and the sender that tells its link the server took it.
type Handed = (u64, Arrival, oneshot::Sender<()>);

impl HandOverIntake {
    pub fn new(
        listener: UnixListener,
        allowed_uid: u32,
        status: watch::Receiver<Status>,
    ) -> HandOverIntake {
        let (sender, arrivals) = mpsc::channel(1);
        tokio::spawn(accept(listener, allowed_uid, status, sender));
        HandOverIntake {
            arrivals,
            taken: VecDeque::new(),
            early: VecDeque::new(),
        }
    }

    /// Takes hand-overs while `wait` runs, so their links report them taken at once; `next`
    /// yields them first.
    pub async fn take_during(&mut self, wait: impl Future<Output = ()>) {
        tokio::pin!(wait);
        loop {
            let arrival = tokio::select! {
                biased;
                () = &mut wait => return,
                arrival = self.take() => arrival,
            };
            self.early.push_back(arrival);
        }
    }

    /// The next hand-over, its link told it was taken. One taken before is closed unserved.
    async fn take(&mut self) -> Arrival {
        loop {
            let Some((conn, arrival, took)) = self.arrivals.recv().await else {
                return std::future::pending().await;
            };
            let _ = took.send(());
            if self.taken.contains(&conn) {
                log::info!("closed conn {conn}'s hand-over: this server took it already");
                continue;
            }
            if self.taken.len() == TAKEN_KEPT {
                self.taken.pop_front();
            }
            self.taken.push_back(conn);
            return arrival;
        }
    }
}

impl Intake for HandOverIntake {
    async fn next(&mut self) -> Arrival {
        match self.early.pop_front() {
            Some(arrival) => arrival,
            None => self.take().await,
        }
    }
}

impl Drop for HandOverIntake {
    fn drop(&mut self) {
        if !self.early.is_empty() {
            let n = self.early.len();
            log::info!("closed {n} hand-overs taken during the lock wait; the server is stopping");
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
    arrivals: mpsc::Sender<Handed>,
) {
    let mut links = JoinSet::new();
    loop {
        tokio::select! {
            biased;
            () = arrivals.closed() => break,
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
    drop(listener);
    while links.join_next().await.is_some() {}
}

async fn link(stream: UnixStream, status: watch::Receiver<Status>, arrivals: mpsc::Sender<Handed>) {
    if let Err(e) = serve_link(&stream, status, &arrivals).await {
        log::warn!("dropped a hand-over link: {e}");
    }
}

/// Passes each hand-over on and reports the viewer count until the link ends. Every exit
/// reports `Took` for the hand-overs taken; one where the intake was dropped then no viewers.
async fn serve_link(
    stream: &UnixStream,
    status: watch::Receiver<Status>,
    arrivals: &mpsc::Sender<Handed>,
) -> io::Result<()> {
    let mut taking = VecDeque::new();
    let ended = pass_on(stream, status, arrivals, &mut taking).await;
    let reported = report_taken(stream, taking).await;
    match ended {
        Ok(Ended::Stopping) => {
            reported?;
            report(stream, &IntakeReport::Viewers { count: 0 }).await
        }
        Ok(Ended::Closed) => Ok(()),
        Err(e) => Err(e),
    }
}

/// How [`pass_on`] ended without an error.
enum Ended {
    /// The peer closed the link, or the process is ending.
    Closed,
    /// The intake was dropped.
    Stopping,
}

/// A hand-over passed on and the receiver that hears when the server takes it.
type Taking = VecDeque<(u64, oneshot::Receiver<()>)>;

async fn pass_on(
    stream: &UnixStream,
    mut status: watch::Receiver<Status>,
    arrivals: &mpsc::Sender<Handed>,
    taking: &mut Taking,
) -> io::Result<Ended> {
    let mut count = status.borrow_and_update().clients.len();
    report(stream, &viewers(count)).await?;
    let mut reader = FdFrameReader::new();
    loop {
        tokio::select! {
            biased;
            () = arrivals.closed() => return Ok(Ended::Stopping),
            ready = stream.readable() => {
                ready?;
                match stream.try_io(Interest::READABLE, || reader.read_from(stream.as_fd())) {
                    Ok(0) => return Ok(Ended::Closed),
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
                while let Some((body, fd)) = reader.next().map_err(io::Error::other)? {
                    let (conn, arrival) = arrival(&body, fd)?;
                    let (took, taken) = oneshot::channel();
                    if arrivals.send((conn, arrival, took)).await.is_err() {
                        return Ok(Ended::Stopping);
                    }
                    taking.push_back((conn, taken));
                }
            }
            taken = async { (&mut taking[0].1).await }, if !taking.is_empty() => {
                let (conn, _) = taking.pop_front().expect("one is waiting");
                if taken.is_ok() {
                    report(stream, &IntakeReport::Took { conn }).await?;
                }
            }
            changed = status.changed() => {
                if changed.is_err() {
                    return Ok(Ended::Closed);
                }
                let now = status.borrow_and_update().clients.len();
                if now != count {
                    count = now;
                    report(stream, &viewers(count)).await?;
                }
            }
        }
    }
}

/// Reports `Took` for each hand-over of `taking` the server has taken.
async fn report_taken(stream: &UnixStream, taking: Taking) -> io::Result<()> {
    for (conn, mut taken) in taking {
        if taken.try_recv().is_ok() {
            report(stream, &IntakeReport::Took { conn }).await?;
        }
    }
    Ok(())
}

fn viewers(count: usize) -> IntakeReport {
    let count = u32::try_from(count).unwrap_or(u32::MAX);
    IntakeReport::Viewers { count }
}

/// The hand-over in `body` as an arrival, with its conn.
fn arrival(body: &[u8], fd: Option<OwnedFd>) -> io::Result<(u64, Arrival)> {
    let IntakeMessage::HandOver(handover) = frame::decode(body).map_err(io::Error::other)?;
    let fd = fd.ok_or_else(|| io::Error::other("a hand-over came without its socket"))?;
    let tcp = tcp_stream(fd)?;
    log::debug!(
        "connection {} from {} handed over for {}",
        handover.conn,
        handover.peer,
        handover.user
    );
    Ok((handover.conn, Arrival::HandedOver { tcp, handover }))
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

async fn report(stream: &UnixStream, report: &IntakeReport) -> io::Result<()> {
    let frame = frame::encode(report).map_err(io::Error::other)?;
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
        hand_over_of(9, peer)
    }

    fn hand_over_of(conn: u64, peer: SocketAddr) -> Vec<u8> {
        let message = IntakeMessage::HandOver(Box::new(HandOver {
            conn,
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

    #[tokio::test]
    async fn a_dropped_intake_reports_no_viewers_before_closing_its_links() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        l.status.send_modify(|s| {
            s.clients = vec![Client {
                address: "192.0.2.1:50000".into(),
                user: "anna".into(),
                since: 0,
            }]
        });
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 1 })
        );
        drop(l.intake);
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        assert_eq!(read_report(&mut l.link).await, None, "then the link closes");
    }

    async fn no_report(link: &mut UnixStream) {
        let mut byte = [0; 1];
        let read = tokio::time::timeout(Duration::from_millis(200), link.read(&mut byte)).await;
        assert!(read.is_err(), "no report yet: {read:?}");
    }

    #[tokio::test]
    async fn a_hand_over_is_reported_taken_once_the_server_takes_it() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        send(
            &l.link,
            &hand_over(client.local_addr().unwrap()),
            Some(accepted.as_fd()),
        );
        no_report(&mut l.link).await;
        tokio::time::timeout(WAIT, l.intake.next()).await.unwrap();
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Took { conn: 9 })
        );
    }

    #[tokio::test]
    async fn a_stopping_server_reports_taken_only_the_hand_overs_it_took() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        let peer = client.local_addr().unwrap();
        send(&l.link, &hand_over_of(9, peer), Some(accepted.as_fd()));
        send(&l.link, &hand_over_of(10, peer), Some(accepted.as_fd()));
        no_report(&mut l.link).await;
        tokio::time::timeout(WAIT, l.intake.next()).await.unwrap();
        drop(l.intake);
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Took { conn: 9 })
        );
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        assert_eq!(read_report(&mut l.link).await, None, "then the link closes");
    }

    #[tokio::test]
    async fn a_hand_over_taken_before_its_link_breaks_is_reported_taken() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        let peer = client.local_addr().unwrap();
        send(&l.link, &hand_over_of(9, peer), Some(accepted.as_fd()));
        send(&l.link, &hand_over_of(10, peer), Some(accepted.as_fd()));
        send(&l.link, &[1, 0, 0, 0, b'x'], None);
        tokio::time::timeout(WAIT, l.intake.next()).await.unwrap();
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Took { conn: 9 })
        );
        assert_eq!(read_report(&mut l.link).await, None, "then the link closes");
    }

    #[tokio::test]
    async fn a_hand_over_sent_again_is_reported_taken_and_closed_unserved() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        send(
            &l.link,
            &hand_over(client.local_addr().unwrap()),
            Some(accepted.as_fd()),
        );
        let _served = tokio::time::timeout(WAIT, l.intake.next()).await.unwrap();
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Took { conn: 9 })
        );

        let mut again = UnixStream::connect(l._dir.path().join("handover.sock"))
            .await
            .unwrap();
        assert_eq!(
            read_report(&mut again).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (mut copy_client, copy) = tcp_pair();
        send(
            &again,
            &hand_over(copy_client.local_addr().unwrap()),
            Some(copy.as_fd()),
        );
        drop(copy);
        stays_pending(&mut l.intake).await;
        assert_eq!(
            read_report(&mut again).await,
            Some(IntakeReport::Took { conn: 9 })
        );
        copy_client.set_read_timeout(Some(WAIT)).unwrap();
        assert_eq!(
            std::io::Read::read(&mut copy_client, &mut [0]).unwrap(),
            0,
            "the copy's socket is closed"
        );
    }

    #[tokio::test]
    async fn a_hand_over_sent_during_the_lock_wait_is_taken_at_once_and_served_after() {
        let mut l = linked(own_uid()).await;
        assert_eq!(
            read_report(&mut l.link).await,
            Some(IntakeReport::Viewers { count: 0 })
        );
        let (client, accepted) = tcp_pair();
        send(
            &l.link,
            &hand_over(client.local_addr().unwrap()),
            Some(accepted.as_fd()),
        );
        let (unlock, locked) = oneshot::channel::<()>();
        {
            let lock_wait = l.intake.take_during(async {
                let _ = locked.await;
            });
            tokio::pin!(lock_wait);
            let report = tokio::select! {
                report = read_report(&mut l.link) => report,
                () = &mut lock_wait => panic!("the lock wait is still running"),
            };
            assert_eq!(report, Some(IntakeReport::Took { conn: 9 }));
            unlock.send(()).unwrap();
            tokio::time::timeout(WAIT, lock_wait).await.unwrap();
        }
        let arrival = tokio::time::timeout(Duration::from_millis(10), l.intake.next())
            .await
            .expect("the hand-over taken during the wait comes first");
        assert!(matches!(arrival, Arrival::HandedOver { handover, .. } if handover.conn == 9));
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
