//! The greeter's and the sessions' sockets: binding, peer checks and one reader per connection.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs::Permissions;
use std::future::Future;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener as StdListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use repriselogin::frame::{self, FrameError, FrameReader};
use repriselogin::greeter::GreeterReply;
use repriselogin::session::SessionRequest;
use repriselogin::{GREETER_SOCKET, SESSION_SOCKET};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc};
use tokio::task::AbortHandle;
use zeroize::Zeroizing;

use crate::daemon::{Input, WorkerId};
use crate::pidfd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listener {
    Greeter,
    Session,
}

/// Whether a peer may stay on `l`. `peer_session` is the peer's logind session,
/// `greeter_session` the running greeter's, and `session_uid` the table's uid for `peer_session`.
pub fn peer_allowed(
    l: Listener,
    peer_uid: u32,
    peer_session: Option<&str>,
    greeter_uid: u32,
    greeter_session: Option<&str>,
    session_uid: Option<u32>,
) -> bool {
    match l {
        Listener::Greeter => {
            peer_uid == greeter_uid && peer_session.is_some() && peer_session == greeter_session
        }
        Listener::Session => peer_session.is_some() && session_uid == Some(peer_uid),
    }
}

pub struct Paths {
    pub greeter: PathBuf,
    pub session: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        Paths {
            greeter: GREETER_SOCKET.into(),
            session: SESSION_SOCKET.into(),
        }
    }
}

/// Binds both sockets over any stale files: `greeter` 0660 in group `greeter_gid`, `session`
/// 0666. The listeners are non-blocking, for [`serve`].
pub fn bind(paths: &Paths, greeter_gid: u32) -> io::Result<(StdListener, StdListener)> {
    let greeter = bind_one(&paths.greeter)?;
    std::os::unix::fs::chown(&paths.greeter, None, Some(greeter_gid))?;
    std::fs::set_permissions(&paths.greeter, Permissions::from_mode(0o660))?;
    let session = bind_one(&paths.session)?;
    std::fs::set_permissions(&paths.session, Permissions::from_mode(0o666))?;
    Ok((greeter, session))
}

fn bind_one(path: &Path) -> io::Result<StdListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let l = StdListener::bind(path)?;
    l.set_nonblocking(true)?;
    Ok(l)
}

/// Most connections a listener checks at once; past it, new ones are closed unchecked.
const MAX_PENDING: usize = 16;
/// How long a greeter connection waits for its worker's `Started` to reach the loop.
pub const GREETER_WAIT: Duration = Duration::from_secs(3);

/// A run of failures, so each run is logged once.
#[derive(Default)]
struct Spell(Cell<bool>);

impl Spell {
    /// Marks a failure; true for the first of a run.
    fn fail(&self) -> bool {
        !self.0.replace(true)
    }

    fn pass(&self) {
        self.0.set(false);
    }
}

/// What the listeners check peers against; the daemon's loop keeps it current.
pub struct Gate {
    pub greeter_uid: u32,
    /// The running greeter worker, and its logind session once it reported `Started`.
    greeter: Option<(WorkerId, Option<String>)>,
    /// The session table: logind session id to uid.
    sessions: HashMap<String, u32>,
    link: Option<Link>,
    /// One link per table session, with its serial.
    session_links: HashMap<String, (u64, Reader)>,
    serials: u64,
    /// Woken when the greeter changes, for a greeter connection waiting on it.
    changed: Rc<Notify>,
}

/// The connected greeter's outbound queue and reader.
struct Link {
    serial: u64,
    tx: mpsc::UnboundedSender<Zeroizing<Vec<u8>>>,
    _reader: Reader,
}

/// A connection's reader task, stopped when this is dropped.
struct Reader(AbortHandle);

impl Reader {
    fn spawn(f: impl Future<Output = ()> + 'static) -> Reader {
        Reader(tokio::task::spawn_local(f).abort_handle())
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Gate {
    pub fn new(greeter_uid: u32) -> Gate {
        Gate {
            greeter_uid,
            greeter: None,
            sessions: HashMap::new(),
            link: None,
            session_links: HashMap::new(),
            serials: 0,
            changed: Rc::new(Notify::new()),
        }
    }

    /// A new greeter worker runs; the previous greeter's connection is closed.
    pub fn greeter_spawned(&mut self, worker: WorkerId) {
        self.greeter = Some((worker, None));
        self.link = None;
        self.changed.notify_waiters();
    }

    /// The greeter `worker` opened logind session `session`.
    pub fn greeter_started(&mut self, worker: WorkerId, session: &str) {
        if let Some(g) = self.greeter.as_mut().filter(|g| g.0 == worker) {
            g.1 = Some(session.into());
            self.changed.notify_waiters();
        }
    }

    pub fn greeter_exited(&mut self, worker: WorkerId) {
        if self.greeter.as_ref().is_some_and(|g| g.0 == worker) {
            self.greeter = None;
        }
    }

    /// Replaces the session table; a session that left it loses its link.
    pub fn set_sessions(&mut self, sessions: HashMap<String, u32>) {
        (self.session_links).retain(|id, _| sessions.contains_key(id));
        self.sessions = sessions;
    }

    /// Queues `reply` for the connected greeter; with none connected it is dropped.
    pub fn reply(&self, reply: &GreeterReply) {
        let Some(link) = &self.link else {
            log::debug!("no greeter is connected; a reply is dropped");
            return;
        };
        match frame::encode(reply) {
            Ok(f) => {
                let _ = link.tx.send(f);
            }
            Err(e) => log::warn!("greeter socket: a reply was not sent: {}", error_kind(&e)),
        }
    }

    /// Whether a peer of `uid` could pass on `kind` at all, before its session is looked up.
    fn may_try(&self, kind: Listener, uid: u32) -> bool {
        match kind {
            Listener::Greeter => uid == self.greeter_uid,
            Listener::Session => self.sessions.values().any(|&u| u == uid),
        }
    }

    fn serial(&mut self) -> u64 {
        self.serials += 1;
        self.serials
    }
}

/// Looks up the logind session of a process id; logind in the daemon, a fake in tests.
pub type SessionOf = Rc<dyn Fn(u32) -> Pin<Box<dyn Future<Output = Option<String>>>>>;

/// Reads seat0's active session.
pub type SeatActive = Rc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>>>>>;

pub struct Server {
    pub gate: Rc<RefCell<Gate>>,
    pub session_of: SessionOf,
    pub seat_active: SeatActive,
    pub input: Rc<dyn Fn(Input)>,
}

/// Accepts on both listeners in tasks of the current `LocalSet`.
pub fn serve(greeter: StdListener, session: StdListener, server: Server) -> io::Result<()> {
    let greeter = UnixListener::from_std(greeter)?;
    let session = UnixListener::from_std(session)?;
    let server = Rc::new(server);
    tokio::task::spawn_local(accept(greeter, Listener::Greeter, server.clone()));
    tokio::task::spawn_local(accept(session, Listener::Session, server));
    Ok(())
}

/// Decrements a listener's count of checks in flight when the check ends.
struct InFlight(Rc<Cell<usize>>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

async fn accept(l: UnixListener, kind: Listener, s: Rc<Server>) {
    let pending = Rc::new(Cell::new(0));
    let failing = Spell::default();
    let refused = Rc::new(Spell::default());
    loop {
        let stream = match l.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                if failing.fail() {
                    log::warn!("{kind:?} socket: accept: {e}");
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        failing.pass();
        let Ok(uid) = stream.peer_cred().map(|c| c.uid()) else {
            continue;
        };
        if !s.gate.borrow().may_try(kind, uid) {
            if refused.fail() {
                log::warn!("{kind:?} socket: refused uid {uid}");
            }
            continue;
        }
        if pending.get() >= MAX_PENDING {
            if refused.fail() {
                log::warn!("{kind:?} socket: {MAX_PENDING} connections are being checked");
            }
            continue;
        }
        pending.set(pending.get() + 1);
        let in_flight = InFlight(pending.clone());
        let (s, refused) = (s.clone(), refused.clone());
        tokio::task::spawn_local(async move {
            let _in_flight = in_flight;
            match admit(stream, uid, kind, &s).await {
                true => refused.pass(),
                false if refused.fail() => log::warn!("{kind:?} socket: refused uid {uid}"),
                false => {}
            }
        });
    }
}

/// The pid to look up for `stream`'s peer, with its pidfd when the kernel gives one.
fn peer_process(stream: &UnixStream) -> Option<(u32, Option<OwnedFd>)> {
    match pidfd::peer(stream) {
        Ok(fd) => Some((pidfd::pid_of(&fd).ok()??, Some(fd))),
        Err(e) if e.raw_os_error() == Some(libc::ENOPROTOOPT) => {
            let pid = stream.peer_cred().ok()?.pid()?;
            Some((u32::try_from(pid).ok()?, None))
        }
        Err(_) => None,
    }
}

/// Checks the peer and starts its reader; a refused peer's stream is dropped, closing it.
async fn admit(stream: UnixStream, uid: u32, kind: Listener, s: &Rc<Server>) -> bool {
    let Some((pid, fd)) = peer_process(&stream) else {
        return false;
    };
    let session = (s.session_of)(pid).await;
    // A peer that exited during the lookup may have left its pid to another process.
    let session = session.filter(|_| fd.as_ref().is_none_or(pidfd::alive));
    match kind {
        Listener::Greeter => admit_greeter(stream, uid, session, s).await,
        Listener::Session => admit_session(stream, uid, session, s),
    }
}

/// Links the greeter once its session is the running greeter's, waiting up to
/// `GREETER_WAIT` for that greeter's `Started` to be handled.
async fn admit_greeter(
    stream: UnixStream,
    uid: u32,
    session: Option<String>,
    s: &Rc<Server>,
) -> bool {
    let deadline = tokio::time::Instant::now() + GREETER_WAIT;
    let worker = loop {
        let changed = s.gate.borrow().changed.clone();
        let notified = changed.notified();
        {
            let gate = s.gate.borrow();
            if gate.link.is_some() {
                return false;
            }
            if let Some((worker, greeter_session)) = &gate.greeter
                && peer_allowed(
                    Listener::Greeter,
                    uid,
                    session.as_deref(),
                    gate.greeter_uid,
                    greeter_session.as_deref(),
                    None,
                )
            {
                break *worker;
            }
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            return false;
        }
    };
    let mut gate = s.gate.borrow_mut();
    let serial = gate.serial();
    let (rd, wr) = stream.into_split();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::task::spawn_local(write_frames(wr, rx));
    let reader = Reader::spawn(read_greeter(rd, serial, worker, s.clone()));
    gate.link = Some(Link {
        serial,
        tx,
        _reader: reader,
    });
    true
}

/// Links a table session; its earlier link, if any, is closed.
fn admit_session(stream: UnixStream, uid: u32, session: Option<String>, s: &Rc<Server>) -> bool {
    let allowed = {
        let gate = s.gate.borrow();
        let table_uid = session
            .as_ref()
            .and_then(|id| gate.sessions.get(id).copied());
        peer_allowed(
            Listener::Session,
            uid,
            session.as_deref(),
            gate.greeter_uid,
            None,
            table_uid,
        )
    };
    let Some(id) = session.filter(|_| allowed) else {
        return false;
    };
    let mut gate = s.gate.borrow_mut();
    let serial = gate.serial();
    let reader = Reader::spawn(read_session(stream, id.clone(), serial, s.clone()));
    gate.session_links.insert(id, (serial, reader));
    true
}

async fn read_greeter(
    rd: tokio::net::unix::OwnedReadHalf,
    serial: u64,
    worker: WorkerId,
    s: Rc<Server>,
) {
    let mut frames = Frames::new(rd);
    let why = loop {
        match frames.next().await.and_then(|body| frame::decode(&body)) {
            Ok(request) => (s.input)(Input::GreeterRequest(request)),
            Err(e) => break e,
        }
    };
    if why != FrameError::Closed {
        log::warn!("greeter socket: dropping the greeter: {}", error_kind(&why));
    }
    let mut gate = s.gate.borrow_mut();
    if gate.link.as_ref().is_some_and(|l| l.serial == serial) {
        gate.link = None;
    }
    // A greeter that is no longer the running one ended already.
    let running = gate.greeter.as_ref().is_some_and(|g| g.0 == worker);
    drop(gate);
    if running {
        (s.input)(Input::GreeterGone);
    }
}

async fn read_session(stream: UnixStream, id: String, serial: u64, s: Rc<Server>) {
    let mut frames = Frames::new(stream);
    let bad = Spell::default();
    let why = loop {
        let body = match frames.next().await {
            Ok(body) => body,
            Err(e) => break e,
        };
        let request = match frame::decode::<SessionRequest>(&body) {
            Ok(request) => request,
            Err(e) => {
                if bad.fail() {
                    log::warn!("session {id}: skipped {}", error_kind(&e));
                }
                continue;
            }
        };
        bad.pass();
        // Read before the next frame, so a flood of requests waits on the bus, not the loop.
        let seat_active = match request {
            SessionRequest::SwitchUser => (s.seat_active)().await,
            SessionRequest::Hello { .. } => None,
        };
        (s.input)(Input::Session {
            id: id.clone(),
            request,
            seat_active,
        });
    };
    if why != FrameError::Closed {
        log::warn!("session {id}: dropping its link: {}", error_kind(&why));
    }
    let mut gate = s.gate.borrow_mut();
    if gate.session_links.get(&id).is_some_and(|l| l.0 == serial) {
        gate.session_links.remove(&id);
    }
}

/// The frames a socket's peer sends, read through a `FrameReader`.
pub struct Frames<R> {
    rd: R,
    reader: FrameReader,
    buf: Zeroizing<Vec<u8>>,
    /// A push that failed, reported once the frames before it are handed out.
    failed: Option<FrameError>,
}

impl<R: AsyncRead + Unpin> Frames<R> {
    pub fn new(rd: R) -> Frames<R> {
        Frames {
            rd,
            reader: FrameReader::new(),
            buf: Zeroizing::new(vec![0u8; 4096]),
            failed: None,
        }
    }

    /// The next frame's body, or why there is none: the peer closed, a read failed or the
    /// stream broke framing.
    pub async fn next(&mut self) -> Result<Zeroizing<Vec<u8>>, FrameError> {
        loop {
            if let Some(body) = self.reader.next_frame() {
                return Ok(body);
            }
            if let Some(e) = self.failed.take() {
                return Err(e);
            }
            let n = match self.rd.read(&mut self.buf).await {
                Ok(0) => return Err(FrameError::Closed),
                Ok(n) => n,
                Err(e) => return Err(e.into()),
            };
            if let Err(e) = self.reader.push(&self.buf[..n]) {
                self.failed = Some(e);
            }
        }
    }
}

/// Writes each queued frame to `wr` until the queue closes or a write fails.
pub async fn write_frames<W: AsyncWrite + Unpin>(
    mut wr: W,
    mut rx: mpsc::UnboundedReceiver<Zeroizing<Vec<u8>>>,
) {
    while let Some(f) = rx.recv().await {
        if let Err(e) = wr.write_all(&f).await {
            log::warn!("socket write: {}", e.kind());
            return;
        }
    }
}

/// A frame error for the log, by category only: a message's text never reaches the journal.
pub fn error_kind(e: &FrameError) -> String {
    match e {
        FrameError::TooLong(n) => format!("a frame of {n} bytes, over the limit"),
        FrameError::Json(_) => "a message that does not parse".into(),
        FrameError::Io(kind) => format!("a socket error ({kind})"),
        FrameError::Closed => "the peer closed the socket".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use repriselogin::frame::encode;
    use repriselogin::greeter::GreeterRequest;
    use tokio::sync::mpsc::UnboundedReceiver;

    use super::*;

    const GREETER: WorkerId = WorkerId(1);

    #[test]
    fn peer_allowed_accepts_only_the_running_greeter() {
        let g = Listener::Greeter;
        assert!(!peer_allowed(g, 1001, Some("c1"), 977, Some("c1"), None));
        assert!(!peer_allowed(g, 977, Some("c2"), 977, Some("c1"), None));
        assert!(!peer_allowed(g, 977, None, 977, None, None));
        assert!(!peer_allowed(g, 977, Some("c1"), 977, None, None));
        assert!(peer_allowed(g, 977, Some("c1"), 977, Some("c1"), None));
    }

    #[test]
    fn peer_allowed_accepts_a_session_only_for_its_own_user() {
        let s = Listener::Session;
        assert!(peer_allowed(s, 1000, Some("3"), 977, None, Some(1000)));
        assert!(!peer_allowed(s, 1001, Some("3"), 977, None, Some(1000)));
        assert!(!peer_allowed(s, 1000, Some("4"), 977, None, None));
        assert!(!peer_allowed(s, 1000, None, 977, None, Some(1000)));
    }

    fn paths(dir: &Path) -> Paths {
        Paths {
            greeter: dir.join("greeter.sock"),
            session: dir.join("session.sock"),
        }
    }

    fn ids() -> (u32, u32) {
        // SAFETY: getuid and getgid cannot fail.
        unsafe { (libc::getuid(), libc::getgid()) }
    }

    #[test]
    fn the_sockets_get_their_modes() {
        let dir = tempfile::tempdir().unwrap();
        let p = paths(dir.path());
        std::fs::write(&p.greeter, "stale").unwrap();
        let (_, gid) = ids();
        let _listeners = bind(&p, gid).unwrap();
        let greeter = std::fs::metadata(&p.greeter).unwrap();
        let session = std::fs::metadata(&p.session).unwrap();
        assert_eq!(greeter.mode() & 0o777, 0o660);
        assert_eq!(greeter.gid(), gid);
        assert_eq!(session.mode() & 0o777, 0o666);
    }

    /// Serves sockets in `dir` for a greeter `GREETER` in logind session "c1" and a table
    /// session "3" of this uid; every peer is in `peer_session`.
    fn serve_in(
        dir: &Path,
        greeter_uid: u32,
        peer_session: &'static str,
    ) -> (Rc<RefCell<Gate>>, UnboundedReceiver<Input>) {
        let (gate, rx, _) = serve_with(dir, greeter_uid, move || {
            Box::pin(async move { Some(peer_session.into()) })
        });
        (gate, rx)
    }

    /// Like `serve_in`, with `lookup` answering each session lookup; also returns how many
    /// lookups ran.
    fn serve_with(
        dir: &Path,
        greeter_uid: u32,
        lookup: impl Fn() -> Pin<Box<dyn Future<Output = Option<String>>>> + 'static,
    ) -> (Rc<RefCell<Gate>>, UnboundedReceiver<Input>, Rc<Cell<u32>>) {
        let (greeter, session) = bind(&paths(dir), ids().1).unwrap();
        let mut gate = Gate::new(greeter_uid);
        gate.greeter = Some((GREETER, Some("c1".into())));
        gate.sessions.insert("3".into(), ids().0);
        let gate = Rc::new(RefCell::new(gate));
        let (tx, rx) = mpsc::unbounded_channel();
        let lookups = Rc::new(Cell::new(0));
        let counted = lookups.clone();
        let server = Server {
            gate: gate.clone(),
            session_of: Rc::new(move |_| {
                counted.set(counted.get() + 1);
                lookup()
            }),
            seat_active: Rc::new(|| Box::pin(async { Some("3".into()) })),
            input: Rc::new(move |i| {
                let _ = tx.send(i);
            }),
        };
        serve(greeter, session, server).unwrap();
        (gate, rx, lookups)
    }

    fn hello() -> SessionRequest {
        SessionRequest::Hello {
            interface: repriselogin::Interface::Blue,
        }
    }

    async fn eof_within(stream: &mut UnixStream, wait: Duration) -> Option<bool> {
        let mut b = [0u8; 16];
        match tokio::time::timeout(wait, stream.read(&mut b)).await {
            Ok(Ok(0)) => Some(true),
            Ok(_) => Some(false),
            Err(_) => None,
        }
    }

    async fn send<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) {
        stream.write_all(&encode(msg).unwrap()).await.unwrap();
    }

    async fn next(rx: &mut UnboundedReceiver<Input>) -> Input {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an input arrives")
            .unwrap()
    }

    #[tokio::test]
    async fn a_greeter_client_with_the_wrong_uid_is_closed() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, _rx) = serve_in(dir.path(), ids().0 + 1, "c1");
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
            })
            .await;
    }

    #[tokio::test]
    async fn a_second_greeter_connection_is_closed() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                let path = dir.path().join("greeter.sock");
                let mut first = UnixStream::connect(&path).await.unwrap();
                send(&mut first, &GreeterRequest::Hello).await;
                assert_eq!(
                    next(&mut rx).await,
                    Input::GreeterRequest(GreeterRequest::Hello)
                );
                let mut second = UnixStream::connect(&path).await.unwrap();
                assert_eq!(
                    eof_within(&mut second, Duration::from_secs(5)).await,
                    Some(true)
                );
                gate.borrow().reply(&GreeterReply::Success);
                let mut b = vec![0u8; encode(&GreeterReply::Success).unwrap().len()];
                first.read_exact(&mut b).await.unwrap();
                assert_eq!(
                    frame::decode::<GreeterReply>(&b[4..]).unwrap(),
                    GreeterReply::Success
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_bad_greeter_frame_closes_the_connection() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                send(&mut c, &SessionRequest::SwitchUser).await;
                assert_eq!(next(&mut rx).await, Input::GreeterGone);
                assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
            })
            .await;
    }

    #[tokio::test]
    async fn a_bad_session_frame_is_skipped() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                send(&mut c, &GreeterRequest::StartSession).await;
                send(&mut c, &SessionRequest::SwitchUser).await;
                assert_eq!(
                    next(&mut rx).await,
                    Input::Session {
                        id: "3".into(),
                        request: SessionRequest::SwitchUser,
                        seat_active: Some("3".into()),
                    }
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_session_peer_outside_the_table_is_closed() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, _rx) = serve_in(dir.path(), ids().0, "9");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
            })
            .await;
    }

    #[tokio::test]
    async fn a_peer_with_no_reason_to_connect_is_closed_before_any_lookup() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, _rx, lookups) = serve_with(dir.path(), ids().0 + 1, || {
                    Box::pin(async { Some("3".into()) })
                });
                gate.borrow_mut().sessions = HashMap::from([("3".into(), ids().0 + 1)]);
                for sock in ["greeter.sock", "session.sock"] {
                    let mut c = UnixStream::connect(dir.path().join(sock)).await.unwrap();
                    assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
                }
                assert_eq!(lookups.get(), 0);
            })
            .await;
    }

    #[tokio::test]
    async fn connections_past_the_checks_in_flight_are_closed_at_once() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, _rx, lookups) =
                    serve_with(dir.path(), ids().0, || Box::pin(std::future::pending()));
                let path = dir.path().join("session.sock");
                let mut waiting = Vec::new();
                for _ in 0..MAX_PENDING {
                    waiting.push(UnixStream::connect(&path).await.unwrap());
                }
                let mut over = UnixStream::connect(&path).await.unwrap();
                assert_eq!(
                    eof_within(&mut over, Duration::from_secs(5)).await,
                    Some(true)
                );
                assert_eq!(lookups.get(), MAX_PENDING as u32);
                assert_eq!(
                    eof_within(&mut waiting[0], Duration::from_millis(100)).await,
                    None,
                    "a connection in its check stays open"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_new_link_from_a_session_replaces_its_old_one() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let path = dir.path().join("session.sock");
                let mut old = UnixStream::connect(&path).await.unwrap();
                send(&mut old, &hello()).await;
                assert!(matches!(next(&mut rx).await, Input::Session { .. }));
                let mut new = UnixStream::connect(&path).await.unwrap();
                send(&mut new, &SessionRequest::SwitchUser).await;
                assert!(matches!(
                    next(&mut rx).await,
                    Input::Session {
                        request: SessionRequest::SwitchUser,
                        ..
                    }
                ));
                assert_eq!(
                    eof_within(&mut old, Duration::from_secs(5)).await,
                    Some(true)
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_session_that_leaves_the_table_loses_its_link() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                send(&mut c, &hello()).await;
                assert!(matches!(next(&mut rx).await, Input::Session { .. }));
                gate.borrow_mut().set_sessions(HashMap::new());
                assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
            })
            .await;
    }

    #[tokio::test]
    async fn a_replaced_greeters_link_feeds_nothing_more() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                send(&mut c, &GreeterRequest::Hello).await;
                assert_eq!(
                    next(&mut rx).await,
                    Input::GreeterRequest(GreeterRequest::Hello)
                );
                gate.borrow_mut().greeter_spawned(WorkerId(2));
                let _ = c.write_all(&encode(&GreeterRequest::Hello).unwrap()).await;
                let late = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
                assert!(late.is_err(), "{late:?}");
            })
            .await;
    }

    #[tokio::test]
    async fn a_greeter_connecting_before_its_start_is_handled_waits_for_it() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                gate.borrow_mut().greeter_spawned(GREETER);
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(200)).await;
                gate.borrow_mut().greeter_started(GREETER, "c1");
                send(&mut c, &GreeterRequest::Hello).await;
                assert_eq!(
                    next(&mut rx).await,
                    Input::GreeterRequest(GreeterRequest::Hello)
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_greeter_whose_start_never_comes_is_closed_after_the_wait() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, _rx) = serve_in(dir.path(), ids().0, "c1");
                gate.borrow_mut().greeter_spawned(GREETER);
                let started = std::time::Instant::now();
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                assert_eq!(
                    eof_within(&mut c, Duration::from_secs(10)).await,
                    Some(true)
                );
                assert!(started.elapsed() >= GREETER_WAIT, "{:?}", started.elapsed());
            })
            .await;
    }

    #[test]
    fn a_spell_of_failures_is_reported_once() {
        let spell = Spell::default();
        assert!(spell.fail());
        assert!(!spell.fail());
        spell.pass();
        assert!(spell.fail());
    }
}
