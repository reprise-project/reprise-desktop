//! The accept loop: clients from an [`Intake`], up to `max_viewers` at once, TLS security only,
//! each logged in through a [`Login`]. It reports who is connected.

use crate::auth::{Login, Logins};
use crate::cert::Identity;
use crate::control::{Client, Request, Status};
use crate::intake::{Arrival, Intake};
use crate::viewers::{Kick, LOGGING_IN_PER_IP, Refused, ViewerId, Viewers};
use ironrdp_pdu::rdp::server_error_info::ProtocolIndependentCode;
use ironrdp_server::{
    CliprdrServerFactory, ErrorInfo, GfxServerFactory, RdpServer, RdpServerDisplay,
    RdpServerInputHandler, SoundServerFactory,
};
use repriseconf::remote::RemoteDesktop;
use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

/// What follows a connection being shown the session, and what follows once its display and
/// input are dropped; each called once per connection shown it, so several may be at a time.
/// A connection that logs in but never gets its portal session never had the session.
pub trait Leave {
    fn arrived(&self) {}
    fn left(&self, why: End) -> impl Future<Output = ()>;
}

/// Why a connection shown the session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The client closed it, was disconnected or kicked, or the network dropped.
    Left,
    /// The screen it showed went away; a client that reconnects is shown the screen now there.
    ScreenGone,
}

/// Marked by a connection's display when the screen it shows goes, before the connection ends.
#[derive(Clone, Default)]
pub struct Ending(Arc<AtomicBool>);

impl Ending {
    pub fn screen_gone(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn end(&self) -> End {
        if self.0.load(Ordering::Acquire) {
            End::ScreenGone
        } else {
            End::Left
        }
    }
}

/// The accept loop's settings and its links to the bus.
pub struct Server<L> {
    pub identity: watch::Receiver<Identity>,
    pub user: String,
    /// Its `clients` are kept here.
    pub status: watch::Sender<Status>,
    /// Its `max_viewers` is read as each connection arrives.
    pub config: watch::Receiver<RemoteDesktop>,
    /// Ends connected clients' sessions.
    pub disconnect: mpsc::UnboundedReceiver<Request>,
    pub leave: L,
}

/// Sent by a connection's display once its portal session is granted and its input is served.
pub type Shown = oneshot::Sender<()>;

/// What serves one connection: a display, an input handler, maybe a graphics pipeline, maybe a
/// clipboard and maybe sound.
pub type Handlers<D, I> = (
    D,
    I,
    Option<Box<dyn GfxServerFactory>>,
    Option<Box<dyn CliprdrServerFactory>>,
    Option<Box<dyn SoundServerFactory>>,
);

/// Serve clients from `intake` for `server.user`, each checked by `logins` and served by
/// `handlers`, which reports through its [`Shown`] and [`Ending`]. A client arriving with
/// `max_viewers` connected is closed at once.
pub async fn serve<D, I, L>(
    mut server: Server<L>,
    mut intake: impl Intake,
    logins: impl Logins,
    mut handlers: impl FnMut(Shown, Ending) -> Handlers<D, I>,
) -> Infallible
where
    D: RdpServerDisplay + 'static,
    I: RdpServerInputHandler + 'static,
    L: Leave + 'static,
{
    let shared = Rc::new(Shared {
        user: server.user,
        status: server.status,
        leave: server.leave,
        viewers: RefCell::default(),
    });
    loop {
        tokio::select! {
            Some(request) = server.disconnect.recv() => match request {
                Request::All => shared.viewers.borrow_mut().kick_all(Kick::Asked),
                Request::Client(address, reply) => {
                    let _ = reply.send(shared.viewers.borrow_mut().kick(&address, Kick::Asked));
                }
            },
            arrival = intake.next() => {
                let (stream, peer) = match arrival {
                    Arrival::Connection(stream, peer) => (stream, peer),
                    Arrival::Moved => {
                        shared.viewers.borrow_mut().kick_all(Kick::PortChanged);
                        continue;
                    }
                };
                let cap = server.config.borrow().max_viewers;
                let joined = shared.viewers.borrow_mut().join(peer, cap);
                let (id, kicked) = match joined {
                    Ok(joined) => joined,
                    Err(Refused::Full) => {
                        let n = shared.viewers.borrow().len();
                        log::info!("{peer}: closed; {n} viewers are connected");
                        continue;
                    }
                    Err(Refused::LoggingIn(ip)) => {
                        log::info!("{peer}: closed; {ip} has {LOGGING_IN_PER_IP} connections logging in");
                        continue;
                    }
                };
                let (check, budget) = logins.check(peer);
                let (shown, on_shown) = oneshot::channel();
                let ending = Ending::default();
                let (display, input, gfx, clipboard, sound) = handlers(shown, ending.clone());
                let local = stream
                    .local_addr()
                    .unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)));
                let rdp = RdpServer::builder()
                    .with_addr(local)
                    .with_tls(server.identity.borrow().acceptor.clone())
                    .with_input_handler(input)
                    .with_display_handler(display)
                    .with_credential_validator(Some(check.clone()))
                    .with_gfx_factory(gfx)
                    .with_cliprdr_factory(clipboard)
                    .with_sound_factory(sound)
                    .build();
                let viewer = Viewer { id, peer, kicked, ending };
                let shared = shared.clone();
                tokio::task::spawn_local(run(rdp, stream, viewer, check, budget, on_shown, shared));
            }
        }
    }
}

/// What every connection of one [`serve`] shares.
struct Shared<L> {
    user: String,
    status: watch::Sender<Status>,
    leave: L,
    viewers: RefCell<Viewers>,
}

impl<L> Shared<L> {
    /// Frees `viewer`'s place and takes it off `clients`. Only the first call does anything: a
    /// later viewer may have the same address.
    fn forget(&self, id: ViewerId, peer: SocketAddr) {
        if !self.viewers.borrow_mut().leave(id) {
            return;
        }
        let address = peer.to_string();
        self.status.send_if_modified(|s| {
            let had = s.clients.len();
            s.clients.retain(|c| c.address != address);
            s.clients.len() != had
        });
    }
}

/// One connection's place among the viewers.
struct Viewer {
    id: ViewerId,
    peer: SocketAddr,
    kicked: oneshot::Receiver<Kick>,
    ending: Ending,
}

async fn run<L: Leave + 'static, C: Login>(
    mut server: RdpServer,
    stream: TcpStream,
    viewer: Viewer,
    check: Arc<C>,
    budget: Duration,
    on_shown: oneshot::Receiver<()>,
    shared: Rc<Shared<L>>,
) {
    let Viewer {
        id,
        peer,
        kicked,
        ending,
    } = viewer;
    log::info!("{peer}: connected");
    let arrived = Rc::new(Cell::new(false));
    let _unwound = Unwound {
        shared: shared.clone(),
        arrived: arrived.clone(),
        id,
        peer,
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let stream = Recorded {
        inner: stream,
        seen: seen.clone(),
    };
    let present = async {
        check.logged_in().await;
        shared.viewers.borrow_mut().logged_in(id);
        let since = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        shared.status.send_modify(|s| {
            s.clients.push(Client {
                address: peer.to_string(),
                user: shared.user.clone(),
                since,
            })
        });
        if on_shown.await.is_ok() {
            shared.leave.arrived();
            arrived.set(true);
        }
        std::future::pending::<Infallible>().await
    };
    let told = server.error_info_disconnect_handle();
    let ended = {
        let connection = server.run_connection(stream);
        tokio::pin!(connection);
        tokio::select! {
        biased;
        ended = &mut connection => Some(ended),
        Ok(kick) = kicked => {
            match kick {
                Kick::Asked => log::info!("{peer}: disconnected by request"),
                Kick::PortChanged => log::info!("{peer}: disconnected; the port changed"),
            }
            shared.forget(id, peer);
            tell_and_end(connection.as_mut(), |why| drop(told.disconnect(why))).await;
            None
        }
        () = login_expired(&*check, budget) => {
            log::warn!("{peer}: dropped; no login within {} s", budget.as_secs());
            None
        }
        never = present => match never {},
        }
    };
    shared.forget(id, peer);
    if let Some(ended) = ended {
        if !check.accepted() && seen.lock().unwrap().refuses_tls() {
            log::warn!(
                "{peer}: refused: the client requires NLA or RDP security; choose TLS security in its settings"
            );
            return;
        }
        match ended {
            Ok(()) => log::info!("{peer}: disconnected"),
            Err(e) => log::info!("{peer}: disconnected: {}", e.report()),
        }
    }
    if arrived.replace(false) {
        drop(server);
        shared.leave.left(ending.end()).await;
    } else if check.accepted() {
        log::info!("{peer}: left without being shown the session; it stays as it was");
    }
}

/// Forgets the viewer however its task ends, and runs `left` for an arrived one that panicked.
struct Unwound<L: Leave + 'static> {
    shared: Rc<Shared<L>>,
    /// Set from `arrived` until `left` is called, so each arrival has one leave.
    arrived: Rc<Cell<bool>>,
    id: ViewerId,
    peer: SocketAddr,
}

impl<L: Leave + 'static> Drop for Unwound<L> {
    fn drop(&mut self) {
        self.shared.forget(self.id, self.peer);
        if std::thread::panicking() && self.arrived.replace(false) {
            let shared = self.shared.clone();
            tokio::task::spawn_local(async move { shared.leave.left(End::Left).await });
        }
    }
}

/// What a kicked client is told. A client told nothing takes the closed socket for a network
/// failure and reconnects with its saved password; Remmina does within a second.
const KICKED: ErrorInfo =
    ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::RpcInitiatedDisconnectByuser);
/// How long a kicked connection may take to send [`KICKED`] and close. One still logging in is
/// not told; it is closed when this runs out.
const TOLD: Duration = Duration::from_secs(2);

/// Passes [`KICKED`] to `tell` for the server to send, then runs `connection` until the server has
/// sent it and closed, or for [`TOLD`].
async fn tell_and_end(connection: Pin<&mut impl Future>, tell: impl FnOnce(ErrorInfo)) {
    tell(KICKED);
    let _ = tokio::time::timeout(TOLD, connection).await;
}

/// Ends once `budget` has passed and no check is in flight, unless the connection has logged in.
async fn login_expired(check: &impl Login, budget: Duration) {
    tokio::time::sleep(budget).await;
    if check.settled().await {
        std::future::pending::<()>().await;
    }
}

const SSL: u32 = 0x1;
const HYBRID: u32 = 0x2;
const HYBRID_EX: u32 = 0x8;
/// Enough of the stream to hold an X.224 Connection Request with a cookie.
const HEAD: usize = 512;
/// RDP_NEG_CORRELATION_INFO (MS-RDPBCGR 2.2.1.1.2), which may follow the RDP_NEG_REQ, opens with
/// these bytes: its type, flags and length (36).
const CORRELATION_INFO: [u8; 4] = [6, 0, 36, 0];

/// The start of what a client sent and how much it sent in all.
#[derive(Default)]
struct Seen {
    head: Vec<u8>,
    total: usize,
}

impl Seen {
    /// The `requestedProtocols` of the X.224 Connection Request that opens the stream (MS-RDPBCGR
    /// 2.2.1.1), and the request's length.
    fn request(&self) -> Option<(u32, usize)> {
        let h = &self.head;
        let len = usize::from(u16::from_be_bytes([*h.get(2)?, *h.get(3)?]));
        if h.first() != Some(&3) || h.get(5) != Some(&0xE0) || h.len() < len || len < 11 {
            return None;
        }
        let correlated = len >= 19 + 36 && h[len - 36..][..4] == CORRELATION_INFO;
        let end = if correlated { len - 36 } else { len };
        let neg = &h[end - 8..end];
        if end < 19 || neg[0] != 1 || neg[2..4] != [8, 0] {
            return Some((0, len));
        }
        Some((u32::from_le_bytes([neg[4], neg[5], neg[6], neg[7]]), len))
    }

    /// The client asked for no TLS, or asked for NLA and sent nothing once TLS was chosen.
    fn refuses_tls(&self) -> bool {
        self.request().is_some_and(|(protocols, len)| {
            protocols & SSL == 0 || (protocols & (HYBRID | HYBRID_EX) != 0 && self.total == len)
        })
    }
}

/// A client stream that keeps a [`Seen`] of what it reads.
struct Recorded {
    inner: TcpStream,
    seen: Arc<Mutex<Seen>>,
}

impl AsyncRead for Recorded {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = polled {
            let read = &buf.filled()[before..];
            let mut seen = self.seen.lock().unwrap();
            seen.total += read.len();
            let room = HEAD.saturating_sub(seen.head.len()).min(read.len());
            seen.head.extend_from_slice(&read[..room]);
        }
        polled
    }
}

impl AsyncWrite for Recorded {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::auth::{self, HANDSHAKE, PasswordCheck, PasswordLogins, Throttle};
    use crate::intake::TcpIntake;
    use ironrdp_server::{CredentialDecision, CredentialValidator};
    use std::net::{IpAddr, Ipv4Addr};
    use std::sync::Once;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::LocalSet;

    static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct Capture;

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, r: &log::Record) {
            LINES.lock().unwrap().push(r.args().to_string());
        }
        fn flush(&self) {}
    }

    pub(crate) fn capture_log() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            log::set_logger(&Capture).unwrap();
            log::set_max_level(log::LevelFilter::Info);
        });
    }

    /// An X.224 Connection Request with no cookie, asking for `protocols`.
    fn connection_request(protocols: u32) -> Vec<u8> {
        let mut v = vec![3, 0, 0, 19, 14, 0xE0, 0, 0, 0, 0, 0, 1, 0, 8, 0];
        v.extend(protocols.to_le_bytes());
        v
    }

    impl Leave for () {
        async fn left(&self, _: End) {}
    }

    /// A running [`serve`] on 127.0.0.1 and its links.
    struct Running {
        addr: SocketAddr,
        status: watch::Receiver<Status>,
        config: watch::Sender<RemoteDesktop>,
        disconnect: mpsc::UnboundedSender<Request>,
    }

    /// A login that is accepted as the connection arrives.
    struct Admitted;

    #[async_trait::async_trait]
    impl CredentialValidator for Admitted {
        async fn validate(
            &self,
            _: &ironrdp_server::Credentials,
        ) -> Result<CredentialDecision, ironrdp_server::CredentialValidationError> {
            Ok(CredentialDecision::Accept)
        }
    }

    impl Login for Admitted {
        async fn logged_in(&self) {}
        fn accepted(&self) -> bool {
            true
        }
        async fn settled(&self) -> bool {
            true
        }
    }

    impl Logins for Admitted {
        type Check = Admitted;
        fn check(&self, _: SocketAddr) -> (Arc<Admitted>, Duration) {
            (Arc::new(Admitted), HANDSHAKE)
        }
    }

    async fn start(local: &LocalSet) -> SocketAddr {
        start_on(local, 0).await.addr
    }

    async fn start_on(local: &LocalSet, port: u16) -> Running {
        let config = RemoteDesktop {
            port,
            ..RemoteDesktop::default()
        };
        start_with(local, config, PasswordLogins::new("alice".into())).await
    }

    /// Clients logged in as they connect, on `port` with up to `max_viewers` at once.
    async fn start_admitting(local: &LocalSet, port: u16, max_viewers: usize) -> Running {
        let config = RemoteDesktop {
            port,
            max_viewers,
            ..RemoteDesktop::default()
        };
        start_with(local, config, Admitted).await
    }

    async fn start_with(
        local: &LocalSet,
        config: RemoteDesktop,
        logins: impl Logins + 'static,
    ) -> Running {
        let port = config.port;
        let dir = tempfile::tempdir().unwrap();
        let identity = crate::cert::load_or_create(dir.path()).unwrap();
        let (config, config_rx) = watch::channel(config);
        let (status, mut status_rx) = watch::channel(Status::default());
        let (disconnect, disconnect_rx) = mpsc::unbounded_channel();
        let intake = TcpIntake::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            config_rx.clone(),
            status.clone(),
        );
        let server = Server {
            identity: watch::channel(identity).1,
            user: "alice".into(),
            status,
            config: config_rx,
            disconnect: disconnect_rx,
            leave: (),
        };
        local.spawn_local(serve(server, intake, logins, |_, _| {
            (
                crate::Grey,
                crate::input::Input::new(Default::default()).0,
                None,
                None,
                None,
            )
        }));
        let reported = status_rx.wait_for(|s| !s.listening.is_empty());
        let listening = (tokio::time::timeout(Duration::from_secs(5), reported).await)
            .expect("where it listens is reported")
            .unwrap()
            .listening
            .clone();
        let addr = listening
            .parse()
            .unwrap_or(SocketAddr::from(([127, 0, 0, 1], port)));
        Running {
            addr,
            status: status_rx,
            config,
            disconnect,
        }
    }

    /// Two clients connected to `s` and listed, and the addresses they connect from.
    async fn two_listed(s: &mut Running) -> [(TcpStream, String); 2] {
        let mut connected = Vec::new();
        for _ in 0..2 {
            let mut c = TcpStream::connect(s.addr).await.unwrap();
            assert!(confirmed(&mut c).await, "each client is served");
            let address = c.local_addr().unwrap().to_string();
            connected.push((c, address));
        }
        let listed = s.status.wait_for(|s| s.clients.len() == 2);
        (tokio::time::timeout(Duration::from_secs(5), listed).await)
            .expect("both clients are listed")
            .unwrap();
        connected.try_into().ok().unwrap()
    }

    fn listed(s: &Running) -> Vec<String> {
        let clients = s.status.borrow().clients.clone();
        clients.into_iter().map(|c| c.address).collect()
    }

    async fn listed_become(s: &mut Running, want: &[&str]) {
        let waited = tokio::time::timeout(
            Duration::from_secs(5),
            s.status.wait_for(|s| {
                s.clients
                    .iter()
                    .map(|c| c.address.as_str())
                    .eq(want.iter().copied())
            }),
        )
        .await
        .is_ok();
        assert!(waited, "listed {:?}, not {want:?}", listed(s));
    }

    async fn closed_soon(c: &mut TcpStream) -> bool {
        tokio::time::timeout(Duration::from_secs(5), until_closed(c))
            .await
            .is_ok()
    }

    async fn still_open(c: &mut TcpStream) -> bool {
        let mut byte = [0u8; 1];
        tokio::time::timeout(Duration::from_millis(300), c.read(&mut byte))
            .await
            .is_err()
    }

    fn free_port() -> u16 {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    }

    async fn listening_at(status: &mut watch::Receiver<Status>, want: &str) {
        let waited = tokio::time::timeout(
            Duration::from_secs(5),
            status.wait_for(|s| s.listening == want),
        )
        .await
        .is_ok();
        let now = status.borrow().listening.clone();
        assert!(waited, "listening {now:?}, not {want}");
    }

    /// Ask for TLS on `c` and return whether the server confirmed it.
    async fn confirmed(c: &mut TcpStream) -> bool {
        c.write_all(&connection_request(SSL)).await.unwrap();
        let mut buf = [0u8; 19];
        c.read_exact(&mut buf).await.is_ok() && buf[5] == 0xD0
    }

    /// Bytes read until the server closes `c`.
    async fn until_closed(c: &mut TcpStream) -> usize {
        let mut buf = Vec::new();
        match c.read_to_end(&mut buf).await {
            Ok(_) => buf.len(),
            Err(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::ConnectionReset);
                buf.len()
            }
        }
    }

    fn seen(head: &[u8], total: usize) -> Seen {
        Seen {
            head: head.to_vec(),
            total,
        }
    }

    #[test]
    fn the_requested_protocols_are_read_from_the_connection_request() {
        let plain = connection_request(SSL | HYBRID);
        assert_eq!(seen(&plain, 19).request(), Some((SSL | HYBRID, 19)));
        let mut cookie = vec![3, 0, 0, 45, 40, 0xE0, 0, 0, 0, 0, 0];
        cookie.extend(b"Cookie: mstshash=someone\r\n");
        cookie.extend([1, 0, 8, 0, 8, 0, 0, 0]);
        assert_eq!(seen(&cookie, 45).request(), Some((HYBRID_EX, 45)));
        let mut correlated = vec![3, 0, 0, 55, 50, 0xE0, 0, 0, 0, 0, 0, 1, 8, 8, 0, 1, 0, 0, 0];
        correlated.extend(CORRELATION_INFO);
        correlated.extend([0x5A; 32]);
        assert_eq!(seen(&correlated, 55).request(), Some((SSL, 55)));
        let legacy = [3, 0, 0, 11, 6, 0xE0, 0, 0, 0, 0, 0];
        assert_eq!(seen(&legacy, 11).request(), Some((0, 11)));
        assert_eq!(seen(&plain[..10], 10).request(), None);
        assert_eq!(seen(b"GET / HTTP/1.1\r\n", 16).request(), None);
    }

    #[test]
    fn tls_is_refused_by_a_request_without_it_or_by_silence_after_nla() {
        assert!(seen(&connection_request(HYBRID), 19).refuses_tls());
        assert!(seen(&connection_request(SSL | HYBRID), 19).refuses_tls());
        assert!(!seen(&connection_request(SSL | HYBRID), 300).refuses_tls());
        assert!(!seen(&connection_request(SSL), 19).refuses_tls());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_handshake_is_dropped_after_10_s() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let addr = start(&local).await;
                let mut stalled = TcpStream::connect(addr).await.unwrap();
                let started = tokio::time::Instant::now();
                assert_eq!(until_closed(&mut stalled).await, 0);
                assert!(started.elapsed() >= HANDSHAKE);
                let mut next = TcpStream::connect(addr).await.unwrap();
                assert!(confirmed(&mut next).await, "the next client is served");
            })
            .await;
    }

    #[tokio::test]
    async fn two_clients_are_served_and_both_listed() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_admitting(&local, 0, 4).await;
                let [(_a, first), (_b, second)] = two_listed(&mut s).await;
                assert_eq!(listed(&s), [first, second]);
                let mut third = TcpStream::connect(s.addr).await.unwrap();
                assert!(
                    confirmed(&mut third).await,
                    "logged-in connections leave room for a third from their address"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_third_connection_logging_in_from_one_address_is_closed() {
        capture_log();
        let local = LocalSet::new();
        local
            .run_until(async {
                let addr = start(&local).await;
                let mut first = TcpStream::connect(addr).await.unwrap();
                let _second = TcpStream::connect(addr).await.unwrap();
                let mut third = TcpStream::connect(addr).await.unwrap();
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut third)).await;
                assert_eq!(closed.ok(), Some(0), "closed at once with nothing sent");
                let peer = third.local_addr().unwrap();
                assert!(logged(&format!(
                    "{peer}: closed; 127.0.0.1 has 2 connections logging in"
                )));
                assert!(confirmed(&mut first).await, "the first is still served");
            })
            .await;
    }

    #[tokio::test]
    async fn a_connection_over_the_cap_is_closed_and_not_listed() {
        capture_log();
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_admitting(&local, 0, 1).await;
                let mut first = TcpStream::connect(s.addr).await.unwrap();
                assert!(confirmed(&mut first).await);
                let address = first.local_addr().unwrap().to_string();
                listed_become(&mut s, &[address.as_str()]).await;
                let mut second = TcpStream::connect(s.addr).await.unwrap();
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut second)).await;
                assert_eq!(closed.ok(), Some(0), "closed at once with nothing sent");
                assert_eq!(listed(&s), [address]);
                let peer = second.local_addr().unwrap();
                let line = format!("{peer}: closed; 1 viewers are connected");
                let lines = LINES.lock().unwrap().clone();
                assert!(lines.contains(&line), "{line:?} in {lines:?}");
            })
            .await;
    }

    #[tokio::test]
    async fn disconnect_client_ends_only_that_address() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_admitting(&local, 0, 4).await;
                let [(mut a, first), (mut b, second)] = two_listed(&mut s).await;
                let (reply, replied) = oneshot::channel();
                s.disconnect.send(Request::Client(first, reply)).unwrap();
                assert_eq!(replied.await, Ok(true), "a client was at that address");
                assert!(closed_soon(&mut a).await, "the first client is ended");
                listed_become(&mut s, &[second.as_str()]).await;
                assert!(still_open(&mut b).await, "the second client is kept");
                let (reply, replied) = oneshot::channel();
                s.disconnect
                    .send(Request::Client("127.0.0.1:1".into(), reply))
                    .unwrap();
                assert_eq!(replied.await, Ok(false), "no client at that address");
            })
            .await;
    }

    #[tokio::test]
    async fn disconnect_ends_every_client() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_admitting(&local, 0, 4).await;
                let [(mut a, _), (mut b, _)] = two_listed(&mut s).await;
                s.disconnect.send(Request::All).unwrap();
                assert!(closed_soon(&mut a).await, "the first client is ended");
                assert!(closed_soon(&mut b).await, "the second client is ended");
                listed_become(&mut s, &[]).await;
            })
            .await;
    }

    #[tokio::test]
    async fn a_port_change_ends_every_client() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_admitting(&local, free_port(), 4).await;
                let [(mut a, _), (mut b, _)] = two_listed(&mut s).await;
                let port = free_port();
                s.config.send_modify(|c| c.port = port);
                listening_at(&mut s.status, &format!("127.0.0.1:{port}")).await;
                assert!(closed_soon(&mut a).await, "the first client is ended");
                assert!(closed_soon(&mut b).await, "the second client is ended");
                listed_become(&mut s, &[]).await;
            })
            .await;
    }

    pub(crate) fn logged(line: &str) -> bool {
        times_logged(line) > 0
    }

    pub(crate) fn times_logged(line: &str) -> usize {
        LINES.lock().unwrap().iter().filter(|l| *l == line).count()
    }

    fn hinted(port: u16) -> bool {
        let lines = LINES.lock().unwrap();
        lines.iter().any(|l| {
            l.starts_with(&format!("127.0.0.1:{port}: refused")) && l.contains("choose TLS")
        })
    }

    #[tokio::test]
    async fn an_nla_only_client_is_refused_with_a_tls_hint() {
        capture_log();
        let local = LocalSet::new();
        local
            .run_until(async {
                let addr = start(&local).await;
                let mut c = TcpStream::connect(addr).await.unwrap();
                c.write_all(&connection_request(HYBRID)).await.unwrap();
                tokio::time::timeout(Duration::from_secs(5), until_closed(&mut c))
                    .await
                    .expect("the connection ends at once");
                tokio::task::yield_now().await;
                assert!(
                    hinted(c.local_addr().unwrap().port()),
                    "{:?}",
                    LINES.lock().unwrap()
                );
            })
            .await;
    }

    #[tokio::test]
    async fn an_nla_client_leaving_once_tls_is_chosen_gets_the_hint() {
        capture_log();
        let local = LocalSet::new();
        local
            .run_until(async {
                let addr = start(&local).await;
                let mut c = TcpStream::connect(addr).await.unwrap();
                let port = c.local_addr().unwrap().port();
                c.write_all(&connection_request(SSL | HYBRID))
                    .await
                    .unwrap();
                let mut confirm = [0u8; 19];
                c.read_exact(&mut confirm).await.unwrap();
                drop(c);
                tokio::time::timeout(Duration::from_secs(5), async {
                    while !hinted(port) {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                })
                .await
                .expect("the hint is logged");
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_check_in_flight_when_the_login_time_ends_is_finished_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = auth::tests::SCRIPTS.lock().await;
        let throttle = Arc::<Mutex<Throttle>>::default();
        let peer = SocketAddr::from(([192, 0, 2, 1], 50000));
        let check = PasswordCheck::new(
            "alice".into(),
            &auth::tests::slow_helper(dir.path()),
            peer,
            throttle.clone(),
        );
        let login = async {
            tokio::time::sleep(HANDSHAKE - Duration::from_secs(1)).await;
            let credentials = auth::tests::credentials("alice", "guess");
            let validate = check.validate(&credentials);
            tokio::pin!(validate);
            tokio::select! {
                d = &mut validate => panic!("the helper takes 300 ms: {d:?}"),
                () = auth::tests::started(dir.path()) => {}
            }
            tokio::time::advance(Duration::from_secs(2)).await;
            validate.await.unwrap()
        };
        let decision = tokio::select! {
            biased;
            d = login => Some(d),
            () = login_expired(&check, HANDSHAKE) => None,
        };
        assert_eq!(decision, Some(CredentialDecision::Reject));
        assert_eq!(throttle.lock().unwrap().failures(peer.ip()), 1);
    }

    #[tokio::test]
    async fn listening_reports_a_bind_error() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = taken.local_addr().unwrap().port();
                let mut s = start_on(&local, port).await;
                let listening = s.status.borrow().listening.clone();
                assert!(
                    listening.starts_with("error: Address already in use"),
                    "{listening}"
                );
                drop(taken);
                s.config.send_modify(|_| {});
                listening_at(&mut s.status, &format!("127.0.0.1:{port}")).await;
                let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                assert!(confirmed(&mut c).await, "the next write binds the port");
            })
            .await;
    }

    #[tokio::test]
    async fn a_port_change_rebinds() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_on(&local, free_port()).await;
                let mut client = TcpStream::connect(s.addr).await.unwrap();
                assert!(confirmed(&mut client).await);
                let port = free_port();
                s.config.send_modify(|c| c.port = port);
                listening_at(&mut s.status, &format!("127.0.0.1:{port}")).await;
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut client)).await;
                assert!(closed.is_ok(), "the client on the old port is closed");
                let old = TcpStream::connect(s.addr).await;
                assert_eq!(
                    old.map_err(|e| e.kind()).err(),
                    Some(io::ErrorKind::ConnectionRefused),
                    "the old port is closed"
                );
                let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                assert!(confirmed(&mut c).await, "the new port is served");
            })
            .await;
    }

    #[tokio::test]
    async fn a_port_move_that_cannot_bind_keeps_the_old_port_and_its_client() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let mut s = start_on(&local, free_port()).await;
                let mut client = TcpStream::connect(s.addr).await.unwrap();
                assert!(confirmed(&mut client).await);
                let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = taken.local_addr().unwrap().port();
                s.config.send_modify(|c| c.port = port);
                let error = s.status.wait_for(|s| s.listening.starts_with("error: "));
                (tokio::time::timeout(Duration::from_secs(5), error).await)
                    .expect("the bind error is reported")
                    .unwrap();
                assert!(still_open(&mut client).await, "the first client is kept");
                let mut second = TcpStream::connect(s.addr).await.unwrap();
                assert!(confirmed(&mut second).await, "the old port still serves");
                drop(taken);
                s.config.send_modify(|_| {});
                listening_at(&mut s.status, &format!("127.0.0.1:{port}")).await;
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut client)).await;
                assert!(closed.is_ok(), "the move ends the client once it binds");
            })
            .await;
    }

    #[tokio::test]
    async fn a_quality_change_keeps_the_client() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let s = start_on(&local, free_port()).await;
                let mut client = TcpStream::connect(s.addr).await.unwrap();
                assert!(confirmed(&mut client).await);
                s.config.send_modify(|c| {
                    c.quality = repriseconf::remote::Quality::Sharp;
                    c.frame_rate = 30;
                });
                let mut byte = [0u8; 1];
                let read =
                    tokio::time::timeout(Duration::from_millis(300), client.read(&mut byte)).await;
                assert!(read.is_err(), "the client is kept: {read:?}");
                assert_eq!(s.status.borrow().listening, s.addr.to_string());
            })
            .await;
    }

    type Steps = Rc<std::cell::RefCell<Vec<&'static str>>>;

    /// A client's input handler, which sends `release` as it is dropped.
    struct HeldInput(mpsc::UnboundedSender<&'static str>);

    impl Drop for HeldInput {
        fn drop(&mut self) {
            let _ = self.0.send("release");
        }
    }

    impl RdpServerInputHandler for HeldInput {
        fn keyboard(&mut self, _: ironrdp_server::KeyboardEvent) {}
        fn mouse(&mut self, _: ironrdp_server::MouseEvent) {}
    }

    /// The portal session a connection's input holds until it ends.
    #[derive(Clone, Copy)]
    enum Portal {
        /// A flag the lock waits for.
        Flag,
        /// A grant, closed through the portal thread; the lock waits as [`LockOnLeave`] does.
        ///
        /// [`LockOnLeave`]: crate::lock::LockOnLeave
        Grant,
    }

    /// Locks once the portal session is closed.
    struct Locker {
        steps: Steps,
        /// The flag's receiver, for [`Portal::Flag`].
        open: Option<watch::Receiver<bool>>,
    }

    impl Leave for Locker {
        fn arrived(&self) {
            self.steps.borrow_mut().push("arrived");
        }

        async fn left(&self, why: End) {
            let lock = async {
                if why == End::ScreenGone {
                    self.steps.borrow_mut().push("screen gone");
                }
                self.steps.borrow_mut().push("lock");
            };
            match &self.open {
                Some(open) => {
                    let _ = open.clone().wait_for(|open| !open).await;
                    lock.await;
                }
                None => crate::lock::after_close(lock).await,
            }
        }
    }

    /// How a logged-in or anonymous connection was ended.
    enum Ended {
        ClientLeft,
        Asked,
        /// Its display marked the screen gone, then the client's connection closed.
        ScreenGone,
    }

    /// Run one connection to its end and return the steps after it and the clients reported
    /// while it was connected.
    async fn session(
        log_in: bool,
        shown: bool,
        end: Ended,
        portal: Portal,
    ) -> (Vec<&'static str>, Vec<Client>) {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = auth::tests::SCRIPTS.lock().await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, peer) = listener.accept().await.unwrap();
        let helper = auth::tests::script(dir.path(), "read -r pw\necho '= 0'\n");
        let check = Arc::new(PasswordCheck::new(
            "alice".into(),
            &helper,
            peer,
            Arc::default(),
        ));
        if log_in {
            let credentials = auth::tests::credentials("alice", "right");
            assert_eq!(
                check.validate(&credentials).await.unwrap(),
                CredentialDecision::Accept
            );
        }
        let (shown_tx, on_shown) = oneshot::channel();
        if shown {
            shown_tx.send(()).unwrap();
        }
        let steps = Steps::default();
        let (input, mut events) = mpsc::unbounded_channel();
        let (open, open_rx) = watch::channel(true);
        let grant = matches!(portal, Portal::Grant).then(crate::portal::tests::grant);
        let task_steps = steps.clone();
        let input_task = tokio::task::spawn_local(async move {
            while let Some(e) = events.recv().await {
                task_steps.borrow_mut().push(e);
            }
            task_steps.borrow_mut().push("close");
            drop(grant);
            open.send_replace(false);
        });
        let identity = crate::cert::load_or_create(dir.path()).unwrap();
        let rdp = RdpServer::builder()
            .with_addr(peer)
            .with_tls(identity.acceptor)
            .with_input_handler(HeldInput(input))
            .with_display_handler(crate::Grey)
            .with_credential_validator(Some(check.clone()))
            .build();
        let (status, mut status_rx) = watch::channel(Status::default());
        let shared = Rc::new(Shared {
            user: "alice".into(),
            status,
            leave: Locker {
                steps: steps.clone(),
                open: matches!(portal, Portal::Flag).then_some(open_rx),
            },
            viewers: RefCell::default(),
        });
        let (id, kicked) = shared.viewers.borrow_mut().join(peer, 1).unwrap();
        let ending = Ending::default();
        let ended_by = async {
            match end {
                Ended::ClientLeft => {
                    drop(client);
                    Vec::new()
                }
                Ended::ScreenGone => {
                    ending.screen_gone();
                    drop(client);
                    Vec::new()
                }
                Ended::Asked => {
                    let listed = status_rx.wait_for(|s| !s.clients.is_empty()).await;
                    let clients = listed.unwrap().clients.clone();
                    let address = peer.to_string();
                    assert!(shared.viewers.borrow_mut().kick(&address, Kick::Asked));
                    clients
                }
            }
        };
        let ended = tokio::time::timeout(Duration::from_secs(10), async {
            let ((), listed) = tokio::join!(
                run(
                    rdp,
                    stream,
                    Viewer {
                        id,
                        peer,
                        kicked,
                        ending: ending.clone(),
                    },
                    check.clone(),
                    HANDSHAKE,
                    on_shown,
                    shared.clone()
                ),
                ended_by
            );
            input_task.await.unwrap();
            listed
        })
        .await;
        let steps_so_far = steps.borrow().clone();
        let listed = ended.unwrap_or_else(|_| panic!("the session ended: {steps_so_far:?}"));
        assert!(
            status_rx.borrow().clients.is_empty(),
            "no client is listed once it left"
        );
        assert_eq!(shared.viewers.borrow().len(), 0, "its place is freed");
        (steps.take(), listed)
    }

    #[tokio::test]
    async fn disconnect_ends_with_release_then_close_then_lock() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(true, true, Ended::ClientLeft, Portal::Flag).await;
                assert_eq!(steps, ["arrived", "release", "close", "lock"]);
            })
            .await;
    }

    #[tokio::test]
    async fn a_connection_whose_screen_went_leaves_as_screen_gone() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(true, true, Ended::ScreenGone, Portal::Flag).await;
                assert_eq!(
                    steps,
                    ["arrived", "release", "close", "screen gone", "lock"]
                );
            })
            .await;
    }

    #[tokio::test]
    async fn the_lock_waits_until_the_dropped_grant_is_closed() {
        LocalSet::new()
            .run_until(async {
                let started = std::time::Instant::now();
                let (steps, _) = session(true, true, Ended::ClientLeft, Portal::Grant).await;
                assert_eq!(steps, ["arrived", "release", "close", "lock"]);
                assert!(
                    started.elapsed() < Duration::from_secs(4),
                    "the lock follows the close, not the 5 s cap: {:?}",
                    started.elapsed()
                );
            })
            .await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_kick_tells_the_client_it_was_disconnected_by_the_user_before_closing() {
        let (sent, closes) = oneshot::channel();
        let told = Cell::new(None);
        let closed = Cell::new(false);
        let connection = std::pin::pin!(async {
            closes.await.unwrap();
            closed.set(true);
        });
        let started = tokio::time::Instant::now();
        tell_and_end(connection, |why| {
            told.set(Some(why));
            sent.send(()).unwrap();
        })
        .await;
        let user = ProtocolIndependentCode::RpcInitiatedDisconnectByuser;
        assert_eq!(told.get(), Some(ErrorInfo::ProtocolIndependentCode(user)));
        assert!(
            closed.get(),
            "the connection runs on until it has sent that"
        );
        assert!(started.elapsed() < TOLD, "and is not held once it closes");
    }

    #[tokio::test(start_paused = true)]
    async fn a_kicked_connection_that_does_not_close_is_dropped_after_2_s() {
        let started = tokio::time::Instant::now();
        tell_and_end(std::pin::pin!(std::future::pending::<()>()), drop).await;
        assert_eq!(started.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_logged_in_connection_never_shown_the_session_does_not_lock() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(true, false, Ended::ClientLeft, Portal::Flag).await;
                assert_eq!(steps, ["release", "close"]);
            })
            .await;
    }

    #[tokio::test]
    async fn an_unauthenticated_drop_does_not_lock() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(false, false, Ended::ClientLeft, Portal::Flag).await;
                assert_eq!(steps, ["release", "close"]);
            })
            .await;
    }

    #[tokio::test]
    async fn a_disconnect_request_ends_a_listed_client_and_locks() {
        LocalSet::new()
            .run_until(async {
                let (steps, listed) = session(true, true, Ended::Asked, Portal::Flag).await;
                assert_eq!(listed.len(), 1);
                assert_eq!(listed[0].user, "alice");
                assert!(listed[0].address.starts_with("127.0.0.1:"), "{listed:?}");
                assert!(
                    listed[0].since > 1_700_000_000,
                    "unix seconds: {}",
                    listed[0].since
                );
                assert_eq!(steps, ["arrived", "release", "close", "lock"]);
            })
            .await;
    }

    /// Accepted, but never seen logged in, as a connection that ends in the poll accepting it.
    struct AcceptedUnseen;

    #[async_trait::async_trait]
    impl CredentialValidator for AcceptedUnseen {
        async fn validate(
            &self,
            _: &ironrdp_server::Credentials,
        ) -> Result<CredentialDecision, ironrdp_server::CredentialValidationError> {
            Ok(CredentialDecision::Accept)
        }
    }

    impl Login for AcceptedUnseen {
        fn logged_in(&self) -> impl Future<Output = ()> {
            std::future::pending()
        }
        fn accepted(&self) -> bool {
            true
        }
        async fn settled(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn an_accepted_connection_that_never_arrived_does_not_leave() {
        LocalSet::new()
            .run_until(async {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let client = TcpStream::connect(listener.local_addr().unwrap())
                    .await
                    .unwrap();
                let (stream, peer) = listener.accept().await.unwrap();
                drop(client);
                let dir = tempfile::tempdir().unwrap();
                let identity = crate::cert::load_or_create(dir.path()).unwrap();
                let check = Arc::new(AcceptedUnseen);
                let rdp = RdpServer::builder()
                    .with_addr(peer)
                    .with_tls(identity.acceptor)
                    .with_input_handler(crate::input::Input::new(Default::default()).0)
                    .with_display_handler(crate::Grey)
                    .with_credential_validator(Some(check.clone()))
                    .build();
                let steps = Steps::default();
                let shared = Rc::new(Shared {
                    user: "alice".into(),
                    status: watch::channel(Status::default()).0,
                    leave: Locker {
                        steps: steps.clone(),
                        open: Some(watch::channel(false).1),
                    },
                    viewers: RefCell::default(),
                });
                let (id, kicked) = shared.viewers.borrow_mut().join(peer, 1).unwrap();
                let viewer = Viewer {
                    id,
                    peer,
                    kicked,
                    ending: Ending::default(),
                };
                let ran = run(
                    rdp,
                    stream,
                    viewer,
                    check,
                    HANDSHAKE,
                    oneshot::channel().1,
                    shared,
                );
                tokio::time::timeout(Duration::from_secs(10), ran)
                    .await
                    .expect("the closed connection ends");
                assert!(steps.borrow().is_empty(), "no leave without an arrival");
            })
            .await;
    }

    #[tokio::test]
    async fn a_shown_connection_whose_task_panics_still_locks() {
        LocalSet::new()
            .run_until(async {
                let peer = SocketAddr::from(([127, 0, 0, 1], 1));
                for shown in [false, true] {
                    let steps = Steps::default();
                    let shared = Rc::new(Shared {
                        user: "alice".into(),
                        status: watch::channel(Status::default()).0,
                        leave: Locker {
                            steps: steps.clone(),
                            open: Some(watch::channel(false).1),
                        },
                        viewers: RefCell::default(),
                    });
                    let (id, _kicked) = shared.viewers.borrow_mut().join(peer, 1).unwrap();
                    let task = tokio::task::spawn_local(async move {
                        let _unwound = Unwound {
                            shared,
                            arrived: Rc::new(Cell::new(shown)),
                            id,
                            peer,
                        };
                        std::panic::resume_unwind(Box::new("the display failed"));
                    });
                    assert!(task.await.unwrap_err().is_panic());
                    for _ in 0..100 {
                        tokio::task::yield_now().await;
                    }
                    let want: &[&str] = if shown { &["lock"] } else { &[] };
                    assert_eq!(*steps.borrow(), want, "shown: {shown}");
                }
            })
            .await;
    }
}
