//! The accept loop: clients from an [`Intake`], up to `max_viewers` at once, TLS security only,
//! each logged in through a [`Login`]; or connections reprise-login handed over, resumed over
//! kernel TLS. It reports who is connected.

#[cfg(test)]
mod reason_tests;
#[cfg(test)]
pub(crate) mod tests;

use crate::auth::{Login, Logins};
use crate::client::{self, ClientLayout};
use crate::clipboard::Cut;
use crate::console::{Console, Joined};
use crate::control::{Client, Request, Status};
use crate::intake::{Admission, Arrival, Intake};
use crate::portal::Screen;
use crate::viewers::{Kick, LOGGING_IN_PER_IP, Refused, ViewerId, Viewers};
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::rdp::server_error_info::ProtocolIndependentCode;
use ironrdp_server::{
    CliprdrServerFactory, ErrorInfo, GfxServerFactory, RdpServer, RdpServerDisplay,
    RdpServerInputHandler, ReplayStep, Resumed, SoundServerFactory,
};
use repriseconf::remote::{RemoteDesktop, SecondClient};
use repriselogin::handover::{HandOver, Step};
use reprisetls::cert::Identity;
use reprisetls::ktls::{self, KtlsStream};
use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::os::fd::AsFd;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};

/// What follows a connection being shown the session, and what follows once its display and
/// input are dropped; each called once per connection shown it, so several may be at a time.
/// A connection that logs in but never gets its portal session never had the session, unless it
/// logged in to replace one that had.
pub trait Leave {
    fn arrived(&self) {}
    fn left(&self, why: End) -> impl Future<Output = ()>;
    /// Runs once as the server stops, before any connection is told to end.
    fn stopping(&self) -> impl Future<Output = ()> {
        std::future::ready(())
    }
    /// Whether this computer's own screen was unlocked, asked as lock mode's screens go.
    fn console_unlocked(&self) -> impl Future<Output = bool> {
        std::future::ready(false)
    }
}

/// Why a connection shown the session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The client closed it, was disconnected or kicked, or the network dropped.
    Left,
    /// The screen it showed went away; a client that reconnects is shown the screen now there.
    ScreenGone,
    /// It was shown lock mode's virtual screens, however it ended: this computer's screen is
    /// locked for lock mode, so leaving never locks it.
    LockMode,
}

/// Marked by a connection's display when it is granted lock mode's screens or the screen it shows
/// goes, before the connection ends; it hands the display the run a replacing login took over.
#[derive(Clone, Default)]
pub struct Ending(Arc<Marks>);

#[derive(Default)]
struct Marks {
    state: AtomicU8,
    /// Set once a lock-mode grant's screens all went; [`run`] then ends the connection.
    gone: Flag,
    /// Set once [`run`] leaves the end to the display, whose updates then end.
    released: Flag,
    clipboard: Cut,
    /// The place in the console's run this connection took over as it logged in.
    taken_over: Mutex<Option<Joined>>,
    /// Whether logging in ends every other connection, its run of viewers going on with this one;
    /// fixed as the connection arrives.
    replaces: bool,
}

/// A flag set once and waited for.
struct Flag(watch::Sender<bool>);

impl Default for Flag {
    fn default() -> Flag {
        Flag(watch::Sender::new(false))
    }
}

impl Flag {
    fn set(&self) {
        self.0.send_replace(true);
    }

    async fn wait(&self) {
        let _ = self.0.subscribe().wait_for(|set| *set).await;
    }

    fn is_set(&self) -> bool {
        *self.0.borrow()
    }
}

const SCREEN_GONE: u8 = 1;
const LOCK_MODE: u8 = 2;

impl Ending {
    pub fn new(replaces: bool) -> Ending {
        Ending(Arc::new(Marks {
            replaces,
            ..Marks::default()
        }))
    }

    pub fn replaces(&self) -> bool {
        self.0.replaces
    }

    pub fn screen_gone(&self) {
        let was = self.0.state.fetch_max(SCREEN_GONE, Ordering::AcqRel);
        if was == LOCK_MODE {
            self.0.gone.set();
        }
    }

    /// The clipboard bridge of this connection, stopped as lock mode's screens go.
    pub fn clipboard(&self) -> Cut {
        self.0.clipboard.clone()
    }

    /// Whether a lock-mode grant's screens all went; their streams' receivers close after this.
    pub(crate) fn is_gone(&self) -> bool {
        self.0.gone.is_set()
    }

    /// Until [`run`] leaves the end of a lock-mode grant whose screens went to the display.
    pub async fn released(&self) {
        self.0.released.wait().await;
    }

    pub(crate) fn release(&self) {
        self.0.released.set();
    }

    pub(crate) fn took_over(&self, place: Option<Joined>) {
        *self.0.taken_over.lock().unwrap() = place;
    }

    /// The place in the console's run this connection took over as it logged in, once.
    pub fn taken_over(&self) -> Option<Joined> {
        self.0.taken_over.lock().unwrap().take()
    }

    /// Marks a grant of lock mode's screens, true if it was one: a grant to a run that `locks`
    /// the console, every stream cast from a virtual output.
    pub fn granted(&self, locks: bool, screens: &[Screen]) -> bool {
        let lock_mode = locks && !screens.is_empty() && screens.iter().all(|s| s.virtual_output);
        if lock_mode {
            self.0.state.fetch_max(LOCK_MODE, Ordering::AcqRel);
        }
        lock_mode
    }

    pub(crate) fn end(&self) -> End {
        match self.0.state.load(Ordering::Acquire) {
            LOCK_MODE => End::LockMode,
            SCREEN_GONE => End::ScreenGone,
            _ => End::Left,
        }
    }
}

/// The accept loop's settings and its links to the bus.
pub struct Server<L> {
    /// None for a server that takes only hand-overs, which arrive past their TLS.
    pub identity: Option<watch::Receiver<Identity>>,
    pub user: String,
    /// Its `clients` are kept here.
    pub status: watch::Sender<Status>,
    /// Its `max_viewers` is read as each connection arrives.
    pub config: watch::Receiver<RemoteDesktop>,
    /// Ends connected clients' sessions.
    pub disconnect: mpsc::UnboundedReceiver<Request>,
    pub leave: L,
    /// The run of viewers a replacing login takes over.
    pub console: Console,
    /// Every connection with a place, kept here and read by the stop.
    pub viewers: Rc<RefCell<Viewers>>,
}

/// Sent by a connection's display once its portal session is granted and its input is served.
#[cfg_attr(test, derive(Default))]
pub struct Shown(watch::Sender<bool>);

impl Shown {
    pub fn send(self) {
        self.0.send_replace(true);
    }

    /// True once sent; closed with it false if the display goes first.
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

/// Until `shown` is sent; whether it was.
pub async fn shown(mut shown: watch::Receiver<bool>) -> bool {
    shown.wait_for(|s| *s).await.is_ok()
}

/// What serves one connection: a display, an input handler, maybe a graphics pipeline, maybe a
/// clipboard, maybe sound, and the layout its client sends, which the display reads.
pub type Handlers<D, I> = (
    D,
    I,
    Option<Box<dyn GfxServerFactory>>,
    Option<Box<dyn CliprdrServerFactory>>,
    Option<Box<dyn SoundServerFactory>>,
    ClientLayout,
);

/// Serve clients from `intake` through `logins` and `handlers`, closing one past `max_viewers`
/// (one more under `"replace"`) or in back-off. `stop` runs [`Leave::stopping`] and ends them all.
pub async fn serve<D, I, L>(
    mut server: Server<L>,
    mut intake: impl Intake,
    logins: impl Logins,
    mut handlers: impl FnMut(Shown, Ending) -> Handlers<D, I>,
    stop: impl Future<Output = ()>,
) where
    D: RdpServerDisplay + 'static,
    I: RdpServerInputHandler + 'static,
    L: Leave + 'static,
{
    let shared = Rc::new(Shared {
        user: server.user,
        status: server.status,
        leave: server.leave,
        viewers: server.viewers,
        console: server.console,
    });
    let (alive, mut all_closed) = mpsc::channel::<Infallible>(1);
    tokio::pin!(stop);
    loop {
        tokio::select! {
            () = &mut stop => break,
            Some(request) = server.disconnect.recv() => answer(&shared, request),
            arrival = intake.next() => {
                let (incoming, peer, admission) = match arrival {
                    Arrival::Connection { stream, peer, admission } => {
                        let Some(identity) = &server.identity else {
                            log::info!("{peer}: closed; this server takes only hand-overs");
                            continue;
                        };
                        let tls = identity.borrow().acceptor.clone();
                        (Incoming::Fresh(stream, tls), peer, admission)
                    }
                    Arrival::Moved => {
                        shared.viewers.borrow_mut().kick_all(Kick::PortChanged);
                        continue;
                    }
                    Arrival::HandedOver { tcp, handover } => {
                        let (peer, user) = (handover.peer, handover.user.clone());
                        match resume(tcp, *handover) {
                            Ok(incoming) => (incoming, peer, Admission::HandedOver { user }),
                            Err(e) => {
                                log::warn!("{peer}: closed; the handed-over socket: {e}");
                                continue;
                            }
                        }
                    }
                };
                let backoff = logins.backoff(peer.ip());
                if !backoff.is_zero() {
                    let ip = peer.ip();
                    log::info!("{peer}: closed; {ip} has {} s of back-off left after failed logins", backoff.as_secs_f32().ceil());
                    continue;
                }
                let (max, second) = {
                    let config = server.config.borrow();
                    (config.max_viewers, config.second_client)
                };
                let replaces = second == SecondClient::Replace;
                let joined = shared.viewers.borrow_mut().join(peer, max + usize::from(replaces));
                let (id, kicked) = match joined {
                    Ok(joined) => joined,
                    Err(Refused::Full) => {
                        let n = shared.viewers.borrow().len();
                        log::info!("{peer}: closed; {n} viewers are connected");
                        continue;
                    }
                    Err(Refused::LoggingIn(place)) => {
                        log::info!("{peer}: closed; {place} has {LOGGING_IN_PER_IP} connections logging in");
                        continue;
                    }
                };
                let (check, budget) = logins.check(peer, admission);
                let (shown, on_shown) = watch::channel(false);
                let ending = Ending::new(replaces);
                let (display, input, gfx, clipboard, sound, layout) =
                    handlers(Shown(shown), ending.clone());
                let builder = RdpServer::builder().with_addr(incoming.local_addr());
                let builder = match &incoming {
                    Incoming::Fresh(_, tls) => builder.with_tls(tls.clone()),
                    Incoming::Resumed { .. } => builder.with_no_security(),
                };
                let rdp = builder
                    .with_input_handler(input)
                    .with_display_handler(display)
                    .with_credential_validator(Some(check.clone()))
                    .with_gfx_factory(gfx)
                    .with_cliprdr_factory(clipboard)
                    .with_sound_factory(sound)
                    .with_connection_handler(Some(Box::new(client::Handler(layout))))
                    .build();
                let viewer = Viewer { id, peer, kicked, ending, alive: alive.clone() };
                let shared = shared.clone();
                match incoming {
                    Incoming::Fresh(stream, _) => tokio::task::spawn_local(run(
                        rdp, stream, None, viewer, check, budget, on_shown, shared,
                    )),
                    Incoming::Resumed { stream, resumed, .. } => tokio::task::spawn_local(run(
                        rdp, stream, Some(resumed), viewer, check, budget, on_shown, shared,
                    )),
                };
            }
        }
    }
    log::info!("stopping");
    // Closed before the drain, so a hand-over sent from now on goes to the next server.
    drop(intake);
    shared.leave.stopping().await;
    // A session end queued as the stop came still tells its clients they logged off.
    while let Ok(request) = server.disconnect.try_recv() {
        answer(&shared, request);
    }
    shared.viewers.borrow_mut().kick_all(Kick::Stopped);
    drop(alive);
    let _ = all_closed.recv().await;
}

fn answer<L>(shared: &Shared<L>, request: Request) {
    let mut viewers = shared.viewers.borrow_mut();
    match request {
        Request::All => viewers.kick_all(Kick::Asked),
        Request::SessionEnding => viewers.kick_all(Kick::LoggedOff),
        Request::Client(address, reply) => {
            let _ = reply.send(viewers.kick(&address, Kick::Asked));
        }
    }
}

/// A connection to serve: one accepted here, under the server's TLS, or one handed over past its
/// TLS with the accept another process ran.
enum Incoming {
    Fresh(TcpStream, tokio_rustls::TlsAcceptor),
    Resumed {
        stream: Box<KtlsStream>,
        local: SocketAddr,
        resumed: Resumed,
    },
}

impl Incoming {
    fn local_addr(&self) -> SocketAddr {
        match self {
            Incoming::Fresh(stream, _) => (stream.local_addr()).unwrap_or(([0, 0, 0, 0], 0).into()),
            Incoming::Resumed { local, .. } => *local,
        }
    }
}

/// A handed-over connection as [`run`] takes it: its socket, whose TLS the kernel already
/// carries, and the accept to replay. A socket another server took first and used is refused.
fn resume(tcp: std::net::TcpStream, handover: HandOver) -> io::Result<Incoming> {
    ktls::check_untouched(tcp.as_fd(), &handover.tls)?;
    tcp.set_nonblocking(true)?;
    let tcp = TcpStream::from_std(tcp)?;
    let local = tcp.local_addr()?;
    let HandOver {
        security,
        steps,
        tls,
        ..
    } = handover;
    let resumed = Resumed {
        security: SecurityProtocol::from_bits_truncate(security),
        steps: steps.into_iter().map(replay_step).collect(),
    };
    let stream = Box::new(KtlsStream::new(tcp, tls));
    Ok(Incoming::Resumed {
        stream,
        local,
        resumed,
    })
}

fn replay_step(step: Step) -> ReplayStep {
    match step {
        Step::Pdu {
            mut input,
            mut output,
        } => ReplayStep::Pdu {
            input: std::mem::take(&mut *input),
            output: std::mem::take(&mut *output),
        },
        Step::SecurityUpgradeDone => ReplayStep::SecurityUpgradeDone,
        Step::CredsspDone { user, domain } => ReplayStep::CredsspDone {
            username: user,
            domain,
        },
    }
}

/// What every connection of one [`serve`] shares.
struct Shared<L> {
    user: String,
    status: watch::Sender<Status>,
    leave: L,
    viewers: Rc<RefCell<Viewers>>,
    console: Console,
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
    /// Held until the connection has closed; [`serve`] waits for every one as it stops.
    alive: mpsc::Sender<Infallible>,
}

/// Runs one connection from its first byte, or from the end of `resumed`'s replay.
#[expect(clippy::too_many_arguments, reason = "each is one connection's own")]
async fn run<L: Leave + 'static, C: Login, S>(
    mut server: RdpServer,
    stream: S,
    resumed: Option<Resumed>,
    viewer: Viewer,
    check: Arc<C>,
    budget: Duration,
    on_shown: watch::Receiver<bool>,
    shared: Rc<Shared<L>>,
) where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let Viewer {
        id,
        peer,
        kicked,
        ending,
        alive,
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
    let arrive = || {
        shared.viewers.borrow_mut().shown(id);
        shared.leave.arrived();
        arrived.set(true);
    };
    let present = async {
        check.logged_in().await;
        shared.viewers.borrow_mut().logged_in(id);
        if ending.replaces() {
            ending.took_over(shared.console.take_over());
            let ended = shared.viewers.borrow_mut().kick_others(id, Kick::Replaced);
            // Arrived before the shown ones it ended leave, so the last of them does not lock
            // the session.
            if ended > 0 {
                arrive();
            }
        }
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
        if shown(on_shown).await && !arrived.get() {
            arrive();
        }
        std::future::pending::<Infallible>().await
    };
    let told = server.error_info_disconnect_handle();
    let mut stopped = false;
    let ended = {
        let connection = async {
            match resumed {
                Some(resumed) => server.run_connection_resumed(stream, resumed).await,
                None => server.run_connection(stream).await,
            }
        };
        tokio::pin!(connection);
        tokio::select! {
        biased;
        ended = &mut connection => Some(ended),
        Ok(kick) = kicked => {
            match kick {
                Kick::Asked => log::info!("{peer}: disconnected by request"),
                Kick::PortChanged => log::info!("{peer}: disconnected; the port changed"),
                Kick::Stopped => log::info!("{peer}: disconnected; the server is stopping"),
                Kick::Replaced => log::info!("{peer}: disconnected; another client logged in"),
                Kick::LoggedOff => log::info!("{peer}: disconnected; the session logged off"),
            }
            shared.forget(id, peer);
            tell_and_end(connection.as_mut(), told_for(kick), |why| drop(told.disconnect(why))).await;
            stopped = kick == Kick::Stopped;
            None
        }
        () = login_expired(&*check, budget) => {
            log::warn!("{peer}: dropped; no login within {} s", budget.as_secs());
            None
        }
        () = ending.0.gone.wait() => {
            let tell = |why| drop(told.disconnect(why));
            let ended = lock_mode_gone(connection.as_mut(), &ending, &shared.leave, tell).await;
            match ended {
                Some(_) => {}
                None if ending.0.released.is_set() => {
                    log::info!("{peer}: dropped; lock mode's display did not end it in time");
                }
                None => log::info!("{peer}: disconnected; the console was unlocked"),
            }
            ended
        }
        never = present => match never {},
        }
    };
    shared.forget(id, peer);
    drop(alive);
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
    // A stop has locked already, through `Leave::stopping`.
    if stopped {
        return;
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
/// failure and reconnects with its saved password; Remmina does within a second. FreeRDP names
/// a stop "initiated by an administrative tool on the server".
fn told_for(kick: Kick) -> ErrorInfo {
    let code = match kick {
        Kick::Asked | Kick::PortChanged => ProtocolIndependentCode::RpcInitiatedDisconnectByuser,
        Kick::Stopped => ProtocolIndependentCode::RpcInitiatedDisconnect,
        Kick::Replaced => ProtocolIndependentCode::DisconnectedByOtherconnection,
        Kick::LoggedOff => ProtocolIndependentCode::LogoffByUser,
    };
    ErrorInfo::ProtocolIndependentCode(code)
}

/// Ends a connection whose lock-mode grant's screens all went, its clipboard cut first: told
/// [`TOOK_OVER`] if `leave` finds the console unlocked, else ended by its display within [`TOLD`].
async fn lock_mode_gone<F: Future>(
    connection: Pin<&mut F>,
    ending: &Ending,
    leave: &impl Leave,
    tell: impl FnOnce(ErrorInfo),
) -> Option<F::Output> {
    ending.0.clipboard.cut();
    if leave.console_unlocked().await {
        tell_and_end(connection, TOOK_OVER, tell).await;
        return None;
    }
    ending.release();
    tokio::time::timeout(TOLD, connection).await.ok()
}

/// What a client is told when this computer's own screen was unlocked, so it does not
/// reconnect and lock the screen again.
const TOOK_OVER: ErrorInfo =
    ErrorInfo::ProtocolIndependentCode(ProtocolIndependentCode::DisconnectedByOtherconnection);

/// How long a kicked connection may take to send its [`told_for`] and close. One still logging
/// in is not told; it is closed when this runs out.
const TOLD: Duration = Duration::from_secs(2);

/// Passes `why` to `tell` for the server to send, then runs `connection` until the server has
/// sent it and closed, or for [`TOLD`].
async fn tell_and_end(
    connection: Pin<&mut impl Future>,
    why: ErrorInfo,
    tell: impl FnOnce(ErrorInfo),
) {
    tell(why);
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
struct Recorded<S> {
    inner: S,
    seen: Arc<Mutex<Seen>>,
}

impl<S: AsyncRead + Unpin> AsyncRead for Recorded<S> {
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

impl<S: AsyncWrite + Unpin> AsyncWrite for Recorded<S> {
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
