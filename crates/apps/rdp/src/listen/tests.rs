//! Tests of the accept loop.

use super::*;
use crate::auth::{self, HANDSHAKE, PasswordCheck, PasswordLogins, Throttle};
use crate::intake::{Admission, TcpIntake};
use ironrdp_server::{CredentialDecision, CredentialValidator};
use repriseconf::remote::SecondClient;
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

/// A console whose runs of viewers are in lock mode.
fn lock_mode_console() -> crate::console::Console {
    crate::console::tests::configured(repriseconf::remote::LaptopScreen::Lock, false).0
}

/// A running [`serve`] on 127.0.0.1 and its links.
struct Running {
    addr: SocketAddr,
    status: watch::Receiver<Status>,
    config: watch::Sender<RemoteDesktop>,
    disconnect: mpsc::UnboundedSender<Request>,
    stop: Option<oneshot::Sender<()>>,
    /// Ends once [`serve`] returned.
    served: tokio::task::JoinHandle<()>,
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
    fn check(&self, _: SocketAddr, _: Admission) -> (Arc<Admitted>, Duration) {
        (Arc::new(Admitted), HANDSHAKE)
    }
    fn backoff(&self, _: IpAddr) -> Duration {
        Duration::ZERO
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
    start_leaving(local, config, logins, |_| ()).await
}

/// [`start_with`], leaving through what `leave` makes of the status.
async fn start_leaving<L: Leave + 'static>(
    local: &LocalSet,
    config: RemoteDesktop,
    logins: impl Logins + 'static,
    leave: impl FnOnce(watch::Receiver<Status>) -> L,
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
        leave: leave(status_rx.clone()),
        console: lock_mode_console(),
    };
    let (stop, stopped) = oneshot::channel::<()>();
    let handlers = |_, _| {
        (
            crate::Grey,
            crate::input::Input::new(Default::default()).0,
            None,
            None,
            None,
            ClientLayout::default(),
        )
    };
    // A dropped `Running` leaves it serving.
    let stopped = async {
        if stopped.await.is_err() {
            std::future::pending().await
        }
    };
    let served = local.spawn_local(serve(server, intake, logins, handlers, stopped));
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
        stop: Some(stop),
        served,
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
async fn a_second_client_joins_by_default() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let mut s = start_admitting(&local, 0, 4).await;
            let [(mut a, first), (mut b, second)] = two_listed(&mut s).await;
            assert!(still_open(&mut a).await, "the first client is kept");
            assert!(still_open(&mut b).await, "the second client is kept");
            assert_eq!(listed(&s), [first, second]);
        })
        .await;
}

/// With `second_client = "replace"` and `max_viewers`, a client connects, then a second one
/// logs in and ends it.
async fn the_second_client_replaces_the_first(max_viewers: usize) {
    let local = LocalSet::new();
    local
        .run_until(async {
            let config = RemoteDesktop {
                port: 0,
                max_viewers,
                second_client: SecondClient::Replace,
                ..RemoteDesktop::default()
            };
            let mut s = start_with(&local, config, Admitted).await;
            let mut first = TcpStream::connect(s.addr).await.unwrap();
            assert!(confirmed(&mut first).await);
            let replaced = first.local_addr().unwrap().to_string();
            listed_become(&mut s, &[replaced.as_str()]).await;
            let mut second = TcpStream::connect(s.addr).await.unwrap();
            assert!(confirmed(&mut second).await, "the second client is let in");
            let kept = second.local_addr().unwrap().to_string();
            assert!(closed_soon(&mut first).await, "the first client is ended");
            let line = format!("{replaced}: disconnected; another client logged in");
            assert!(logged(&line), "{line:?} in {:?}", LINES.lock().unwrap());
            listed_become(&mut s, &[kept.as_str()]).await;
            assert!(still_open(&mut second).await, "the second client is kept");
        })
        .await;
}

#[tokio::test]
async fn with_replace_a_client_logging_in_ends_the_earlier_ones() {
    capture_log();
    the_second_client_replaces_the_first(4).await;
}

#[tokio::test]
async fn with_replace_a_client_past_max_viewers_is_let_in_to_replace() {
    capture_log();
    the_second_client_replaces_the_first(1).await;
}

/// A connection to `addr` from `ip`, one of the 127/8 addresses.
async fn connect_from(ip: [u8; 4], addr: SocketAddr) -> TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::from((ip, 0))).unwrap();
    socket.connect(addr).await.unwrap()
}

/// A server for `max_viewers` whose logins share `throttle`, where 127.0.0.1 has failed
/// `failures` times.
async fn start_throttled(
    local: &LocalSet,
    max_viewers: usize,
    failures: u32,
) -> (Running, Arc<Mutex<Throttle>>) {
    let throttle = Arc::<Mutex<Throttle>>::default();
    for _ in 0..failures {
        throttle
            .lock()
            .unwrap()
            .failed(IpAddr::V4(Ipv4Addr::LOCALHOST));
    }
    let config = RemoteDesktop {
        port: 0,
        max_viewers,
        ..RemoteDesktop::default()
    };
    let logins = PasswordLogins::sharing("alice".into(), throttle.clone());
    (start_with(local, config, logins).await, throttle)
}

#[tokio::test]
async fn an_address_in_back_off_is_closed_before_it_takes_a_place() {
    capture_log();
    let local = LocalSet::new();
    local
        .run_until(async {
            let (s, _throttle) = start_throttled(&local, 1, 6).await;
            for _ in 0..3 {
                let mut refused = TcpStream::connect(s.addr).await.unwrap();
                let closed =
                    tokio::time::timeout(Duration::from_secs(5), until_closed(&mut refused)).await;
                assert_eq!(closed.ok(), Some(0), "closed at once with nothing sent");
                let peer = refused.local_addr().unwrap();
                assert!(logged(&format!(
                    "{peer}: closed; 127.0.0.1 has 8 s of back-off left after failed logins"
                )));
            }
            let mut other = connect_from([127, 0, 0, 2], s.addr).await;
            assert!(
                confirmed(&mut other).await,
                "another address takes the only place"
            );
        })
        .await;
}

#[tokio::test(start_paused = true)]
async fn an_address_is_let_in_once_its_back_off_ends() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (s, throttle) = start_throttled(&local, 4, 4).await;
            let mut refused = TcpStream::connect(s.addr).await.unwrap();
            assert_eq!(
                until_closed(&mut refused).await,
                0,
                "refused in its back-off"
            );
            let wait = throttle
                .lock()
                .unwrap()
                .delay(IpAddr::V4(Ipv4Addr::LOCALHOST));
            tokio::time::advance(wait).await;
            let mut next = TcpStream::connect(s.addr).await.unwrap();
            assert!(confirmed(&mut next).await, "let in after the back-off");
        })
        .await;
}

#[tokio::test]
async fn an_address_with_a_free_failure_left_is_let_in() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let (s, _throttle) = start_throttled(&local, 4, 3).await;
            let mut c = TcpStream::connect(s.addr).await.unwrap();
            assert!(confirmed(&mut c).await, "the fourth try is checked");
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
    lines
        .iter()
        .any(|l| l.starts_with(&format!("127.0.0.1:{port}: refused")) && l.contains("choose TLS"))
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
    let on_shown = watch::channel(shown).1;
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
        console: lock_mode_console(),
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
                    replaces: false,
                    alive: mpsc::channel(1).0,
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

/// How many clients were listed as the server stopped.
struct ListedAtStop(watch::Receiver<Status>, Rc<Cell<Option<usize>>>);

impl Leave for ListedAtStop {
    async fn left(&self, _: End) {}
    async fn stopping(&self) {
        self.1.set(Some(self.0.borrow().clients.len()));
    }
}

#[tokio::test]
async fn a_stop_locks_with_every_client_connected_then_ends_them() {
    let local = LocalSet::new();
    local
        .run_until(async {
            let at_stop = Rc::new(Cell::new(None));
            let config = RemoteDesktop {
                port: 0,
                max_viewers: 4,
                ..RemoteDesktop::default()
            };
            let leave = |status| ListedAtStop(status, at_stop.clone());
            let mut s = start_leaving(&local, config, Admitted, leave).await;
            let [(mut a, _), (mut b, _)] = two_listed(&mut s).await;
            s.stop.take().unwrap().send(()).unwrap();
            assert!(closed_soon(&mut a).await, "the first client is ended");
            assert!(closed_soon(&mut b).await, "the second client is ended");
            let served = tokio::time::timeout(Duration::from_secs(5), s.served).await;
            assert!(served.is_ok(), "the server returns once both closed");
            assert_eq!(at_stop.get(), Some(2), "the lock comes before either ends");
        })
        .await;
}

#[test]
fn a_stop_tells_the_client_the_server_ended_it_and_a_kick_that_the_user_did() {
    let code = |kick| match told_for(kick) {
        ErrorInfo::ProtocolIndependentCode(code) => code,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        code(Kick::Stopped),
        ProtocolIndependentCode::RpcInitiatedDisconnect
    );
    for kick in [Kick::Asked, Kick::PortChanged] {
        assert_eq!(
            code(kick),
            ProtocolIndependentCode::RpcInitiatedDisconnectByuser
        );
    }
}

#[test]
fn a_replaced_client_is_told_another_connection_took_over() {
    let ErrorInfo::ProtocolIndependentCode(code) = told_for(Kick::Replaced) else {
        panic!("{:?}", told_for(Kick::Replaced));
    };
    assert_eq!(code, ProtocolIndependentCode::DisconnectedByOtherconnection);
    assert_eq!(code as u32, 5, "ERRINFO_DISCONNECTED_BY_OTHERCONNECTION");
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
    tell_and_end(connection, told_for(Kick::Asked), |why| {
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

/// A console whose screen was unlocked as `unlocked` says, noting whether `ending`'s
/// clipboard was cut by the time it was asked.
struct Console {
    unlocked: bool,
    ending: Ending,
    cut_when_asked: Cell<Option<bool>>,
}

impl Leave for Console {
    async fn left(&self, _: End) {}
    async fn console_unlocked(&self) -> bool {
        self.cut_when_asked
            .set(Some(self.ending.clipboard().is_cut()));
        self.unlocked
    }
}

/// A lock-mode grant's ending whose screens all went, and a console unlocked or not.
async fn lock_mode_screens_gone(unlocked: bool) -> (Ending, Console) {
    let ending = Ending::default();
    let screen = crate::portal::Screen {
        node: 0,
        position: (0, 0),
        size: (1920, 1080),
        mapping_id: None,
        virtual_output: true,
    };
    assert!(ending.granted(true, &[screen]));
    ending.screen_gone();
    tokio::time::timeout(Duration::from_secs(1), ending.0.gone.wait())
        .await
        .expect("the run hears the screens went");
    let console = Console {
        unlocked,
        ending: ending.clone(),
        cut_when_asked: Cell::new(None),
    };
    (ending, console)
}

#[tokio::test(start_paused = true)]
async fn a_console_unlock_ending_lock_mode_tells_the_client_another_connection_took_over() {
    let (ending, console) = lock_mode_screens_gone(true).await;
    let (sent, closes) = oneshot::channel();
    let told = Cell::new(None);
    let connection = std::pin::pin!(async { closes.await.unwrap() });
    let ended = lock_mode_gone(connection, &ending, &console, |why| {
        told.set(Some(why));
        sent.send(()).unwrap();
    });
    let ended = tokio::time::timeout(TOLD * 2, ended).await;
    let ended = ended.expect("the connection ends once told");
    assert_eq!(told.get(), Some(TOOK_OVER));
    assert_eq!(
        TOOK_OVER,
        ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::DisconnectedByOtherconnection)
    );
    assert!(ended.is_none());
    assert_eq!(
        console.cut_when_asked.get(),
        Some(true),
        "the clipboard is cut before the console is asked"
    );
}

#[tokio::test(start_paused = true)]
async fn a_monitor_count_change_ending_lock_mode_tells_the_client_nothing() {
    let (ending, console) = lock_mode_screens_gone(false).await;
    let told = Cell::new(None);
    let display = ending.clone();
    let connection = std::pin::pin!(async move { display.released().await });
    let ended = lock_mode_gone(connection, &ending, &console, |why| told.set(Some(why))).await;
    assert_eq!(told.get(), None, "the client reconnects to the new screens");
    assert!(ended.is_some(), "the display ends the connection");
    assert_eq!(console.cut_when_asked.get(), Some(true));
}

#[tokio::test(start_paused = true)]
async fn a_lock_mode_connection_its_display_does_not_end_is_dropped_after_2_s() {
    let (ending, console) = lock_mode_screens_gone(false).await;
    let started = tokio::time::Instant::now();
    let connection = std::pin::pin!(std::future::pending::<()>());
    let ended = lock_mode_gone(connection, &ending, &console, drop);
    let ended = tokio::time::timeout(TOLD * 2, ended).await;
    assert!(ended.expect("dropped, not held").is_none());
    assert_eq!(started.elapsed(), TOLD);
}

#[tokio::test(start_paused = true)]
async fn a_kicked_connection_that_does_not_close_is_dropped_after_2_s() {
    let started = tokio::time::Instant::now();
    let why = told_for(Kick::Asked);
    tell_and_end(std::pin::pin!(std::future::pending::<()>()), why, drop).await;
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
                console: lock_mode_console(),
            });
            let (id, kicked) = shared.viewers.borrow_mut().join(peer, 1).unwrap();
            let viewer = Viewer {
                id,
                peer,
                kicked,
                ending: Ending::default(),
                replaces: false,
                alive: mpsc::channel(1).0,
            };
            let ran = run(
                rdp,
                stream,
                viewer,
                check,
                HANDSHAKE,
                watch::channel(false).1,
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
                    console: lock_mode_console(),
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
