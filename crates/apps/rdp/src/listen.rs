//! The accept loop: one client at a time, TLS security only, logged in by [`PasswordCheck`]. It
//! follows the configured port and reports where it listens and who is connected.

use crate::auth::{self, PasswordCheck, Throttle};
use crate::cert::Identity;
use crate::control::{Client, Status};
use ironrdp_server::{
    CliprdrServerFactory, GfxServerFactory, RdpServer, RdpServerDisplay, RdpServerInputHandler,
    SoundServerFactory,
};
use repriseconf::remote::RemoteDesktop;
use std::cell::Cell;
use std::convert::Infallible;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};

/// The time a connection has to log in, on top of any wait its address is serving. A password
/// check in flight when it runs out is finished first.
pub const HANDSHAKE: Duration = Duration::from_secs(10);

const KEEPALIVE_IDLE: Duration = Duration::from_secs(15);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const KEEPALIVE_PROBES: u32 = 3;
/// Keepalive probes wait for unacknowledged data; this ends a link that dies mid-send.
const USER_TIMEOUT: Duration = Duration::from_secs(30);

/// All addresses: IPv4 included through the IPv6 socket, IPv4 alone where IPv6 is off.
pub const ALL: IpAddr = IpAddr::V6(Ipv6Addr::UNSPECIFIED);

/// What follows a connection's login, and what follows once its display and input are dropped.
pub trait Leave {
    fn arrived(&self) {}
    fn left(&self) -> impl Future<Output = ()>;
}

/// The accept loop's settings and its links to the bus.
pub struct Server<L> {
    /// The address to listen on; the port is `config`'s, and a change of it rebinds.
    pub ip: IpAddr,
    pub config: watch::Receiver<RemoteDesktop>,
    pub identity: watch::Receiver<Identity>,
    pub user: String,
    /// Its `listening` and `clients` are kept here.
    pub status: watch::Sender<Status>,
    /// Ends the connected client's session.
    pub disconnect: mpsc::UnboundedReceiver<()>,
    pub leave: L,
}

/// Why a connection was ended from this side.
#[derive(Clone, Copy, Debug)]
enum Kick {
    Asked,
    PortChanged,
}

/// What serves one connection: a display, an input handler, maybe a graphics pipeline, maybe a
/// clipboard and maybe sound.
pub type Handlers<D, I> = (
    D,
    I,
    Option<Box<dyn GfxServerFactory>>,
    Option<Box<dyn CliprdrServerFactory>>,
    Option<Box<dyn SoundServerFactory>>,
);

/// Accept clients for `server.user`, each served by `handlers`. A client arriving while another
/// is connected is closed at once.
pub async fn serve<D, I, L>(
    mut server: Server<L>,
    mut handlers: impl FnMut() -> Handlers<D, I>,
) -> Infallible
where
    D: RdpServerDisplay + 'static,
    I: RdpServerInputHandler + 'static,
    L: Leave + 'static,
{
    let throttle = Arc::new(Mutex::new(Throttle::default()));
    let helper = auth::helper();
    let busy = Rc::new(Cell::new(false));
    let shared = Rc::new(Shared {
        user: server.user,
        status: server.status,
        leave: server.leave,
    });
    let mut port = server.config.borrow_and_update().port;
    let mut listener = listen_on(server.ip, port, &shared.status);
    let mut kick: Option<oneshot::Sender<Kick>> = None;
    loop {
        tokio::select! {
            Ok(()) = server.config.changed() => {
                let wanted = server.config.borrow_and_update().port;
                if wanted == port && let Some(l) = &listener {
                    report(l, &shared.status);
                    continue;
                }
                let Some(bound) = listen_on(server.ip, wanted, &shared.status) else {
                    continue;
                };
                if wanted != port && let Some(k) = kick.take() {
                    let _ = k.send(Kick::PortChanged);
                }
                port = wanted;
                listener = Some(bound);
            }
            Some(()) = server.disconnect.recv() => {
                if let Some(k) = kick.take() {
                    let _ = k.send(Kick::Asked);
                }
            }
            accepted = accept(listener.as_ref()) => {
                let (stream, peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(e) => {
                        log::warn!("accept: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let peer = SocketAddr::new(peer.ip().to_canonical(), peer.port());
                if busy.get() {
                    log::info!("{peer}: closed; another client is connected");
                    continue;
                }
                if let Err(e) = tune(&stream) {
                    log::warn!("{peer}: TCP options: {e}");
                }
                let check = Arc::new(PasswordCheck::new(
                    shared.user.clone(),
                    &helper,
                    peer,
                    throttle.clone(),
                ));
                let budget = HANDSHAKE + throttle.lock().unwrap().delay(peer.ip());
                let (display, input, gfx, clipboard, sound) = handlers();
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
                let guard = Busy::claim(&busy);
                let (tx, kicked) = oneshot::channel();
                kick = Some(tx);
                let shared = shared.clone();
                tokio::task::spawn_local(async move {
                    let _guard = guard;
                    run(rdp, stream, peer, check, budget, kicked, shared).await;
                });
            }
        }
    }
}

/// What every connection of one [`serve`] shares.
struct Shared<L> {
    user: String,
    status: watch::Sender<Status>,
    leave: L,
}

/// Report where `listener` listens, as after a failed move it still does.
fn report(listener: &TcpListener, status: &watch::Sender<Status>) {
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    status.send_if_modified(|s| {
        let changed = s.listening != addr.to_string();
        s.listening = addr.to_string();
        changed
    });
}

/// Bind `ip`:`port` and report it, or the error, as `listening`.
fn listen_on(ip: IpAddr, port: u16, status: &watch::Sender<Status>) -> Option<TcpListener> {
    let bound = match bind(SocketAddr::new(ip, port)) {
        Err(_) if ip == ALL => bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port)),
        bound => bound,
    };
    let (listener, listening) = match bound.and_then(|l| Ok((l.local_addr()?, l))) {
        Ok((addr, l)) => {
            log::info!("listening on {addr}");
            (Some(l), addr.to_string())
        }
        Err(e) => {
            log::error!("cannot listen on port {port}: {e}");
            (None, format!("error: {e}"))
        }
    };
    status.send_modify(|s| s.listening = listening);
    listener
}

/// A listener on `addr`; an IPv6 one takes IPv4 too, whatever `net.ipv6.bindv6only` says.
fn bind(addr: SocketAddr) -> io::Result<TcpListener> {
    use socket2::{Domain, Protocol, Socket, Type};
    let socket = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(false)?;
    }
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    TcpListener::from_std(socket.into())
}

async fn accept(listener: Option<&TcpListener>) -> io::Result<(TcpStream, SocketAddr)> {
    match listener {
        Some(l) => l.accept().await,
        None => std::future::pending().await,
    }
}

async fn run<L: Leave + 'static>(
    mut server: RdpServer,
    stream: TcpStream,
    peer: SocketAddr,
    check: Arc<PasswordCheck>,
    budget: Duration,
    kicked: oneshot::Receiver<Kick>,
    shared: Rc<Shared<L>>,
) {
    log::info!("{peer}: connected");
    let _unwound = Unwound {
        shared: shared.clone(),
        check: check.clone(),
    };
    let seen = Arc::new(Mutex::new(Seen::default()));
    let stream = Recorded {
        inner: stream,
        seen: seen.clone(),
    };
    let present = async {
        check.logged_in().await;
        shared.leave.arrived();
        let since = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        shared.status.send_modify(|s| {
            s.clients = vec![Client {
                address: peer.to_string(),
                user: shared.user.clone(),
                since,
            }]
        });
        std::future::pending::<Infallible>().await
    };
    let ended = tokio::select! {
        biased;
        ended = server.run_connection(stream) => Some(ended),
        Ok(kick) = kicked => {
            match kick {
                Kick::Asked => log::info!("{peer}: disconnected by request"),
                Kick::PortChanged => log::info!("{peer}: disconnected; the port changed"),
            }
            None
        }
        () = login_expired(&check, budget) => {
            log::warn!("{peer}: dropped; no login within {} s", budget.as_secs());
            return;
        }
        never = present => match never {},
    };
    shared.status.send_if_modified(|s| {
        let had = !s.clients.is_empty();
        s.clients.clear();
        had
    });
    if let Some(ended) = ended {
        if !check.accepted() && seen.lock().unwrap().refuses_tls() {
            log::warn!(
                "{peer}: refused: the client requires NLA or RDP security; choose TLS security in its settings"
            );
            return;
        }
        match ended {
            Ok(()) => log::info!("{peer}: disconnected"),
            Err(e) => log::info!("{peer}: disconnected: {e:#}"),
        }
    }
    if check.accepted() {
        drop(server);
        shared.leave.left().await;
    }
}

/// Runs `left` for a logged-in connection whose task panicked.
struct Unwound<L: Leave + 'static> {
    shared: Rc<Shared<L>>,
    check: Arc<PasswordCheck>,
}

impl<L: Leave + 'static> Drop for Unwound<L> {
    fn drop(&mut self) {
        if std::thread::panicking() && self.check.accepted() {
            let shared = self.shared.clone();
            tokio::task::spawn_local(async move { shared.leave.left().await });
        }
    }
}

/// Ends once `budget` has passed and no check is in flight, unless the connection has logged in.
async fn login_expired(check: &PasswordCheck, budget: Duration) {
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
        let neg = &h[len - 8..len];
        if len < 19 || neg[0] != 1 || neg[2..4] != [8, 0] {
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

/// Keepalive and a user timeout end a dead link in about 30 s; no Nagle delay for small updates.
fn tune(stream: &TcpStream) -> io::Result<()> {
    let socket = socket2::SockRef::from(stream);
    socket.set_tcp_keepalive(
        &socket2::TcpKeepalive::new()
            .with_time(KEEPALIVE_IDLE)
            .with_interval(KEEPALIVE_INTERVAL)
            .with_retries(KEEPALIVE_PROBES),
    )?;
    socket.set_tcp_user_timeout(Some(USER_TIMEOUT))?;
    socket.set_tcp_nodelay(true)
}

/// Marks the server busy until dropped, including when its task is cancelled.
struct Busy(Rc<Cell<bool>>);

impl Busy {
    fn claim(busy: &Rc<Cell<bool>>) -> Busy {
        busy.set(true);
        Busy(busy.clone())
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ironrdp_server::{CredentialDecision, CredentialValidator};
    use std::sync::Once;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
        async fn left(&self) {}
    }

    /// A running [`serve`] on 127.0.0.1 and its links.
    struct Running {
        addr: SocketAddr,
        status: watch::Receiver<Status>,
        config: watch::Sender<RemoteDesktop>,
        _disconnect: mpsc::UnboundedSender<()>,
    }

    async fn start(local: &LocalSet) -> SocketAddr {
        start_on(local, 0).await.addr
    }

    async fn start_on(local: &LocalSet, port: u16) -> Running {
        let dir = tempfile::tempdir().unwrap();
        let identity = crate::cert::load_or_create(dir.path()).unwrap();
        let (config, config_rx) = watch::channel(RemoteDesktop {
            port,
            ..RemoteDesktop::default()
        });
        let (status, mut status_rx) = watch::channel(Status::default());
        let (disconnect, disconnect_rx) = mpsc::unbounded_channel();
        let server = Server {
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            config: config_rx,
            identity: watch::channel(identity).1,
            user: "alice".into(),
            status,
            disconnect: disconnect_rx,
            leave: (),
        };
        local.spawn_local(serve(server, || {
            (crate::Grey, crate::input::Input::new().0, None, None, None)
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
            _disconnect: disconnect,
        }
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
    async fn a_second_client_is_closed_while_one_is_active() {
        let local = LocalSet::new();
        local
            .run_until(async {
                let addr = start(&local).await;
                let mut first = TcpStream::connect(addr).await.unwrap();
                assert!(confirmed(&mut first).await);
                let mut second = TcpStream::connect(addr).await.unwrap();
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut second)).await;
                assert_eq!(closed.ok(), Some(0), "closed at once with nothing sent");
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
                let mut second = TcpStream::connect(s.addr).await.unwrap();
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut second)).await;
                assert_eq!(
                    closed.ok(),
                    Some(0),
                    "the old port still serves, and the first client is still connected"
                );
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

        async fn left(&self) {
            let lock = async { self.steps.borrow_mut().push("lock") };
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
    enum End {
        ClientLeft,
        Asked,
    }

    /// Run one connection to its end and return the steps after it and the clients reported
    /// while it was connected.
    async fn session(log_in: bool, end: End, portal: Portal) -> (Vec<&'static str>, Vec<Client>) {
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
        });
        let (kick, kicked) = oneshot::channel();
        let ending = async {
            match end {
                End::ClientLeft => {
                    drop(client);
                    Vec::new()
                }
                End::Asked => {
                    let listed = status_rx.wait_for(|s| !s.clients.is_empty()).await;
                    let clients = listed.unwrap().clients.clone();
                    kick.send(Kick::Asked).unwrap();
                    clients
                }
            }
        };
        let ended = tokio::time::timeout(Duration::from_secs(10), async {
            let ((), listed) = tokio::join!(
                run(
                    rdp,
                    stream,
                    peer,
                    check.clone(),
                    HANDSHAKE,
                    kicked,
                    shared.clone()
                ),
                ending
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
        (steps.take(), listed)
    }

    #[tokio::test]
    async fn disconnect_ends_with_release_then_close_then_lock() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(true, End::ClientLeft, Portal::Flag).await;
                assert_eq!(steps, ["arrived", "release", "close", "lock"]);
            })
            .await;
    }

    #[tokio::test]
    async fn the_lock_waits_until_the_dropped_grant_is_closed() {
        LocalSet::new()
            .run_until(async {
                let started = std::time::Instant::now();
                let (steps, _) = session(true, End::ClientLeft, Portal::Grant).await;
                assert_eq!(steps, ["arrived", "release", "close", "lock"]);
                assert!(
                    started.elapsed() < Duration::from_secs(4),
                    "the lock follows the close, not the 5 s cap: {:?}",
                    started.elapsed()
                );
            })
            .await;
    }

    #[tokio::test]
    async fn an_unauthenticated_drop_does_not_lock() {
        LocalSet::new()
            .run_until(async {
                let (steps, _) = session(false, End::ClientLeft, Portal::Flag).await;
                assert_eq!(steps, ["release", "close"]);
            })
            .await;
    }

    #[tokio::test]
    async fn a_disconnect_request_ends_a_listed_client_and_locks() {
        LocalSet::new()
            .run_until(async {
                let (steps, listed) = session(true, End::Asked, Portal::Flag).await;
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

    #[tokio::test]
    async fn an_accepted_link_gives_up_after_30_s_unacknowledged_and_sends_at_once() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let _client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        tune(&stream).unwrap();
        let socket = socket2::SockRef::from(&stream);
        assert_eq!(socket.tcp_user_timeout().unwrap(), Some(USER_TIMEOUT));
        assert!(socket.tcp_nodelay().unwrap());
        assert!(socket.keepalive().unwrap());
        assert_eq!(socket.tcp_keepalive_time().unwrap(), KEEPALIVE_IDLE);
    }

    #[tokio::test]
    async fn listening_on_all_addresses_takes_ipv4() {
        let listener = bind(SocketAddr::new(ALL, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        if addr.is_ipv6() {
            let only_v6 = socket2::SockRef::from(&listener).only_v6().unwrap();
            assert!(!only_v6, "IPV6_V6ONLY is off");
        }
        let v4 = SocketAddr::from(([127, 0, 0, 1], addr.port()));
        let (client, accepted) = tokio::join!(TcpStream::connect(v4), listener.accept());
        client.expect("an IPv4 client connects");
        let (_, peer) = accepted.unwrap();
        assert_eq!(peer.ip().to_canonical(), v4.ip());
    }

    #[tokio::test]
    async fn a_logged_in_connection_whose_task_panics_still_locks() {
        LocalSet::new()
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let _scripts = auth::tests::SCRIPTS.lock().await;
                let helper = auth::tests::script(dir.path(), "read -r pw\necho '= 0'\n");
                let peer = SocketAddr::from(([127, 0, 0, 1], 1));
                for log_in in [false, true] {
                    let check = Arc::new(PasswordCheck::new(
                        "alice".into(),
                        &helper,
                        peer,
                        Arc::default(),
                    ));
                    if log_in {
                        let credentials = auth::tests::credentials("alice", "right");
                        check.validate(&credentials).await.unwrap();
                    }
                    let steps = Steps::default();
                    let shared = Rc::new(Shared {
                        user: "alice".into(),
                        status: watch::channel(Status::default()).0,
                        leave: Locker {
                            steps: steps.clone(),
                            open: Some(watch::channel(false).1),
                        },
                    });
                    let task = tokio::task::spawn_local(async move {
                        let _unwound = Unwound { shared, check };
                        std::panic::resume_unwind(Box::new("the display failed"));
                    });
                    assert!(task.await.unwrap_err().is_panic());
                    for _ in 0..100 {
                        tokio::task::yield_now().await;
                    }
                    let want: &[&str] = if log_in { &["lock"] } else { &[] };
                    assert_eq!(*steps.borrow(), want, "logged in: {log_in}");
                }
            })
            .await;
    }
}
