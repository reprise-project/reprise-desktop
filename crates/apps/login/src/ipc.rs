//! The greeter's and the sessions' sockets: binding, peer checks and one reader per connection.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
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
use repriselogin::greeter::{GreeterReply, GreeterRequest};
use repriselogin::session::{SessionReply, SessionRequest};
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
    let greeter = bind_group(&paths.greeter, greeter_gid)?;
    let session = bind_open(&paths.session)?;
    Ok((greeter, session))
}

/// Binds `path` as [`bind_one`] does, 0666 for every local user.
pub fn bind_open(path: &Path) -> io::Result<StdListener> {
    let l = bind_one(path)?;
    std::fs::set_permissions(path, Permissions::from_mode(0o666))?;
    Ok(l)
}

/// Binds `path` as [`bind_one`] does, 0660 in group `gid`.
pub fn bind_group(path: &Path, gid: u32) -> io::Result<StdListener> {
    let l = bind_one(path)?;
    std::os::unix::fs::chown(path, None, Some(gid))?;
    std::fs::set_permissions(path, Permissions::from_mode(0o660))?;
    Ok(l)
}

/// Binds a non-blocking listener at `path` over any stale file.
pub fn bind_one(path: &Path) -> io::Result<StdListener> {
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

/// Most connections a listener checks at once, in all and per uid; past either, new ones
/// are closed unchecked.
const MAX_PENDING: usize = 16;
const MAX_PENDING_PER_UID: usize = 4;
/// How many socket requests wait for the daemon's loop; past it, a reader stops reading.
pub const QUEUE: usize = 16;
/// How many replies wait for the greeter to read them; past it, the greeter is dropped.
const LINK_QUEUE: usize = 16;
/// Most uids a listener records a refusal of; refusals of further uids are not logged.
const MAX_REFUSALS: usize = 256;
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

/// The connected greeter's outbound queue, writer and reader.
struct Link {
    serial: u64,
    worker: WorkerId,
    tx: mpsc::Sender<Zeroizing<Vec<u8>>>,
    _writer: Reader,
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

    /// The greeter `worker` exited; its connection is closed.
    pub fn greeter_exited(&mut self, worker: WorkerId) {
        if self.greeter.as_ref().is_some_and(|g| g.0 == worker) {
            self.greeter = None;
            self.link = None;
        }
    }

    /// Replaces the session table; a session that left it loses its link.
    pub fn set_sessions(&mut self, sessions: HashMap<String, u32>) {
        (self.session_links).retain(|id, _| sessions.contains_key(id));
        self.sessions = sessions;
    }

    /// Queues `reply` for the connected greeter; with none connected it is dropped. A greeter
    /// whose queue is full is disconnected, and its worker returned.
    pub fn reply(&mut self, reply: &GreeterReply) -> Option<WorkerId> {
        let Some(link) = &self.link else {
            log::debug!("no greeter is connected; a reply is dropped");
            return None;
        };
        let f = match frame::encode(reply) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("greeter socket: a reply was not sent: {}", error_kind(&e));
                return None;
            }
        };
        match link.tx.try_send(f) {
            Err(mpsc::error::TrySendError::Full(_)) => {
                log::warn!("greeter socket: dropping the greeter: it reads no replies");
                self.link.take().map(|l| l.worker)
            }
            _ => None,
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
    /// The daemon's loop; capped at [`QUEUE`], so a reader waits while it is full.
    pub input: mpsc::Sender<Input>,
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

/// A listener's checks in flight, in all and per uid.
#[derive(Default)]
pub(crate) struct Checks {
    all: usize,
    by_uid: HashMap<u32, usize>,
}

/// A check counted in its listener's [`Checks`] until it is dropped.
pub(crate) struct InFlight {
    checks: Rc<RefCell<Checks>>,
    uid: u32,
}

impl InFlight {
    /// Counts a check for `uid`; `None` when the listener or the uid is at its cap.
    pub(crate) fn begin(checks: &Rc<RefCell<Checks>>, uid: u32) -> Option<InFlight> {
        let mut c = checks.borrow_mut();
        let mine = c.by_uid.get(&uid).copied().unwrap_or(0);
        if c.all >= MAX_PENDING || mine >= MAX_PENDING_PER_UID {
            return None;
        }
        c.all += 1;
        *c.by_uid.entry(uid).or_default() += 1;
        Some(InFlight {
            checks: checks.clone(),
            uid,
        })
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut c = self.checks.borrow_mut();
        c.all -= 1;
        if let Some(n) = c.by_uid.get_mut(&self.uid) {
            *n -= 1;
            if *n == 0 {
                c.by_uid.remove(&self.uid);
            }
        }
    }
}

/// The uids a listener logged a refusal of: each is logged once, whatever passes after.
struct Refusals {
    kind: Listener,
    uids: RefCell<HashSet<u32>>,
    full: Spell,
}

impl Refusals {
    fn new(kind: Listener) -> Refusals {
        Refusals {
            kind,
            uids: RefCell::default(),
            full: Spell::default(),
        }
    }

    /// Whether to log this refusal: `uid`'s first, while under [`MAX_REFUSALS`] uids.
    fn first(&self, uid: u32) -> bool {
        let mut uids = self.uids.borrow_mut();
        if uids.len() >= MAX_REFUSALS && !uids.contains(&uid) {
            if self.full.fail() {
                log::warn!("{:?} socket: further refusals not logged", self.kind);
            }
            return false;
        }
        uids.insert(uid)
    }
}

async fn accept(l: UnixListener, kind: Listener, s: Rc<Server>) {
    let checks = Rc::new(RefCell::new(Checks::default()));
    let failing = Spell::default();
    let refused = Rc::new(Refusals::new(kind));
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
            if kind == Listener::Session {
                refuse_session(&stream);
            }
            if refused.first(uid) {
                log::warn!("{kind:?} socket: refused uid {uid}");
            }
            continue;
        }
        let Some(in_flight) = InFlight::begin(&checks, uid) else {
            if refused.first(uid) {
                log::warn!("{kind:?} socket: refused uid {uid}: too many connections in checks");
            }
            continue;
        };
        let (s, refused) = (s.clone(), refused.clone());
        tokio::task::spawn_local(async move {
            let _in_flight = in_flight;
            if !admit(stream, uid, kind, &s).await && refused.first(uid) {
                log::warn!("{kind:?} socket: refused uid {uid}");
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
    let (tx, mut rx) = mpsc::channel(LINK_QUEUE);
    let writer = Reader::spawn(write_frames(wr, async move || rx.recv().await));
    let reader = Reader::spawn(read_greeter(rd, serial, worker, s.clone()));
    gate.link = Some(Link {
        serial,
        worker,
        tx,
        _writer: writer,
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
        refuse_session(&stream);
        return false;
    };
    let mut gate = s.gate.borrow_mut();
    let serial = gate.serial();
    let reader = Reader::spawn(read_session(stream, id.clone(), serial, s.clone()));
    gate.session_links.insert(id, (serial, reader));
    true
}

/// Tells a session peer it is not one of the table's, so its link stops calling; best effort,
/// since the stream is dropped next.
fn refuse_session(stream: &UnixStream) {
    if let Ok(frame) = frame::encode(&SessionReply::Refused) {
        let _ = stream.try_write(&frame);
    }
}

/// Forwards the greeter's requests. Each Hello gets a whole Greeting, so one waits at a time
/// and later ones fold into it; any other request stops the reading until it is queued.
async fn read_greeter(
    rd: tokio::net::unix::OwnedReadHalf,
    serial: u64,
    worker: WorkerId,
    s: Rc<Server>,
) {
    let mut frames = Frames::new(rd);
    let mut hello = false;
    let input = |request| Input::GreeterRequest(worker, request);
    let why = loop {
        let next = tokio::select! {
            permit = s.input.reserve(), if hello => {
                let Ok(permit) = permit else { return };
                permit.send(input(GreeterRequest::Hello));
                hello = false;
                continue;
            }
            next = frames.next() => next,
        };
        let request = match next.and_then(|body| frame::decode(&body)) {
            Ok(GreeterRequest::Hello) => {
                hello = true;
                continue;
            }
            Ok(request) => request,
            Err(e) => break e,
        };
        if std::mem::take(&mut hello) && s.input.send(input(GreeterRequest::Hello)).await.is_err() {
            return;
        }
        if s.input.send(input(request)).await.is_err() {
            return;
        }
    };
    if why != FrameError::Closed {
        log::warn!("greeter socket: dropping the greeter: {}", error_kind(&why));
    }
    let _ = s.input.send(Input::GreeterGone(worker)).await;
    // Last: dropping the link aborts this task.
    let mut gate = s.gate.borrow_mut();
    if gate.link.as_ref().is_some_and(|l| l.serial == serial) {
        gate.link = None;
    }
}

/// Forwards a session's requests, each with seat0's active session as it is queued. One Hello
/// waits at a time, a later one replacing it; a SwitchUser pauses reading until it is queued.
async fn read_session(stream: UnixStream, id: String, serial: u64, s: Rc<Server>) {
    let mut frames = Frames::new(stream);
    let bad = Spell::default();
    let mut hello = None;
    let input = |request, seat_active| Input::Session {
        id: id.clone(),
        request,
        seat_active,
    };
    let why = loop {
        let next = tokio::select! {
            permit = s.input.reserve(), if hello.is_some() => {
                let Ok(permit) = permit else { return };
                let seat_active = (s.seat_active)().await;
                permit.send(input(hello.take().expect("a Hello waits"), seat_active));
                continue;
            }
            next = frames.next() => next,
        };
        let body = match next {
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
        match request {
            SessionRequest::Hello { .. } => {
                hello = Some(request);
                continue;
            }
            SessionRequest::SwitchUser => {}
        }
        let seat_active = (s.seat_active)().await;
        if let Some(h) = hello.take()
            && s.input.send(input(h, seat_active.clone())).await.is_err()
        {
            return;
        }
        if s.input.send(input(request, seat_active)).await.is_err() {
            return;
        }
    };
    if let Some(h) = hello.take() {
        let seat_active = (s.seat_active)().await;
        let _ = s.input.send(input(h, seat_active)).await;
    }
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
    /// stream broke framing. Cancel-safe: a dropped call loses no bytes.
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

/// Writes each frame `next` hands out to `wr` until it returns `None` or a write fails.
pub async fn write_frames<W: AsyncWrite + Unpin>(
    mut wr: W,
    mut next: impl AsyncFnMut() -> Option<Zeroizing<Vec<u8>>>,
) {
    while let Some(f) = next().await {
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
        FrameError::Fd => "a file descriptor out of place".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::MetadataExt;

    use repriselogin::frame::encode;
    use repriselogin::greeter::GreeterRequest;
    use tokio::sync::mpsc::Receiver;

    use super::*;

    const GREETER: WorkerId = WorkerId(1);

    /// Hellos in a flood test, one write each: far past what the socket buffers, so a reader
    /// that stops reading stalls the writer.
    const FLOOD: usize = 5000;

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
    ) -> (Rc<RefCell<Gate>>, Receiver<Input>) {
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
    ) -> (Rc<RefCell<Gate>>, Receiver<Input>, Rc<Cell<u32>>) {
        let (greeter, session) = bind(&paths(dir), ids().1).unwrap();
        let mut gate = Gate::new(greeter_uid);
        gate.greeter = Some((GREETER, Some("c1".into())));
        gate.sessions.insert("3".into(), ids().0);
        let gate = Rc::new(RefCell::new(gate));
        let (tx, rx) = mpsc::channel(QUEUE);
        let lookups = Rc::new(Cell::new(0));
        let counted = lookups.clone();
        let server = Server {
            gate: gate.clone(),
            session_of: Rc::new(move |_| {
                counted.set(counted.get() + 1);
                lookup()
            }),
            seat_active: Rc::new(|| Box::pin(async { Some("3".into()) })),
            input: tx,
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

    async fn next(rx: &mut Receiver<Input>) -> Input {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an input arrives")
            .unwrap()
    }

    /// Waits until the peer of `stream` has read every byte written to it, so its reader
    /// queues nothing more.
    async fn read_through(stream: &UnixStream) {
        let unread = || {
            let mut n: libc::c_int = 0;
            // SAFETY: TIOCOUTQ writes one c_int through the pointer; the fd is open.
            let r = unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ, &mut n) };
            assert_eq!(r, 0, "TIOCOUTQ: {}", io::Error::last_os_error());
            n
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while unread() > 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the reader reads the whole flood");
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
                    Input::GreeterRequest(GREETER, GreeterRequest::Hello)
                );
                let mut second = UnixStream::connect(&path).await.unwrap();
                assert_eq!(
                    eof_within(&mut second, Duration::from_secs(5)).await,
                    Some(true)
                );
                gate.borrow_mut().reply(&GreeterReply::Success);
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
                assert_eq!(next(&mut rx).await, Input::GreeterGone(GREETER));
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

    /// Everything `stream` sends before it closes, within `wait`.
    async fn all_within(stream: &mut UnixStream, wait: Duration) -> Vec<u8> {
        let mut all = Vec::new();
        let read = tokio::time::timeout(wait, stream.read_to_end(&mut all)).await;
        assert!(matches!(read, Ok(Ok(_))), "the stream closes: {read:?}");
        all
    }

    fn refused() -> Vec<u8> {
        encode(&SessionReply::Refused).unwrap().to_vec()
    }

    #[tokio::test]
    async fn a_session_peer_outside_the_table_is_told_it_is_refused_and_closed() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, _rx) = serve_in(dir.path(), ids().0, "9");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                assert_eq!(all_within(&mut c, Duration::from_secs(5)).await, refused());
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
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                assert_eq!(eof_within(&mut c, Duration::from_secs(5)).await, Some(true));
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                assert_eq!(all_within(&mut c, Duration::from_secs(5)).await, refused());
                assert_eq!(lookups.get(), 0);
            })
            .await;
    }

    #[tokio::test]
    async fn a_uids_connections_past_its_checks_in_flight_are_closed_at_once() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, _rx, lookups) =
                    serve_with(dir.path(), ids().0, || Box::pin(std::future::pending()));
                let path = dir.path().join("session.sock");
                let mut waiting = Vec::new();
                for _ in 0..MAX_PENDING_PER_UID {
                    waiting.push(UnixStream::connect(&path).await.unwrap());
                }
                let mut over = UnixStream::connect(&path).await.unwrap();
                assert_eq!(
                    eof_within(&mut over, Duration::from_secs(5)).await,
                    Some(true)
                );
                assert_eq!(lookups.get(), MAX_PENDING_PER_UID as u32);
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
                    Input::GreeterRequest(GREETER, GreeterRequest::Hello)
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
                    Input::GreeterRequest(GREETER, GreeterRequest::Hello)
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
    fn checks_in_flight_are_capped_per_uid_and_per_listener() {
        let checks = Rc::new(RefCell::new(Checks::default()));
        let mut held: Vec<_> = (0..MAX_PENDING_PER_UID)
            .map(|_| InFlight::begin(&checks, 1000).expect("under the uid's cap"))
            .collect();
        assert!(InFlight::begin(&checks, 1000).is_none(), "the uid's cap");
        for uid in 1001..1100 {
            match InFlight::begin(&checks, uid) {
                Some(c) => held.push(c),
                None => break,
            }
        }
        assert_eq!(held.len(), MAX_PENDING, "the listener's cap");
        assert!(InFlight::begin(&checks, 2000).is_none());
        held.remove(0);
        assert!(
            InFlight::begin(&checks, 1000).is_some(),
            "a finished check frees its slot"
        );
        drop(held);
        assert_eq!(checks.borrow().all, 0);
        assert!(checks.borrow().by_uid.is_empty());
    }

    #[test]
    fn a_uids_refusal_is_logged_once() {
        let refusals = Refusals::new(Listener::Session);
        assert!(refusals.first(1000));
        assert!(!refusals.first(1000));
        assert!(refusals.first(1001));
        assert!(!refusals.first(1000));
    }

    #[tokio::test]
    async fn a_greeter_whose_worker_exited_loses_its_link() {
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
                    Input::GreeterRequest(GREETER, GreeterRequest::Hello)
                );
                gate.borrow_mut().greeter_exited(GREETER);
                assert_eq!(eof_within(&mut c, Duration::from_secs(2)).await, Some(true));
            })
            .await;
    }

    #[tokio::test]
    async fn a_hello_flood_is_read_through_and_queues_at_most_the_cap() {
        use repriselogin::Interface;
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                let hellos = [Interface::Classic, Interface::Blue]
                    .map(|interface| encode(&SessionRequest::Hello { interface }).unwrap());
                let last = encode(&SessionRequest::Hello {
                    interface: Interface::Glass,
                })
                .unwrap();
                let flood = tokio::task::spawn_local(async move {
                    for i in 0..FLOOD {
                        c.write_all(&hellos[i % 2]).await.unwrap();
                    }
                    c.write_all(&last).await.unwrap();
                    c
                });
                let c = tokio::time::timeout(Duration::from_secs(5), flood)
                    .await
                    .expect("the reader keeps reading while the queue is full")
                    .unwrap();
                read_through(&c).await;
                assert!(rx.len() <= QUEUE, "{} queued", rx.len());
                let glass = Input::Session {
                    id: "3".into(),
                    request: SessionRequest::Hello {
                        interface: Interface::Glass,
                    },
                    seat_active: Some("3".into()),
                };
                let mut heard = vec![next(&mut rx).await];
                while heard.last() != Some(&glass) {
                    heard.push(next(&mut rx).await);
                }
                assert!(heard.len() <= QUEUE + 1, "{} heard", heard.len());
                assert!(rx.try_recv().is_err(), "nothing follows the last Hello");
            })
            .await;
    }

    #[tokio::test]
    async fn a_sessions_hello_carries_seat0s_active_session() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                send(&mut c, &hello()).await;
                assert_eq!(
                    next(&mut rx).await,
                    Input::Session {
                        id: "3".into(),
                        request: hello(),
                        seat_active: Some("3".into()),
                    }
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_hello_before_a_switch_user_is_queued_first() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "3");
                let mut c = UnixStream::connect(dir.path().join("session.sock"))
                    .await
                    .unwrap();
                send(&mut c, &hello()).await;
                send(&mut c, &SessionRequest::SwitchUser).await;
                let first = next(&mut rx).await;
                assert!(
                    matches!(
                        &first,
                        Input::Session {
                            request: SessionRequest::Hello { .. },
                            ..
                        }
                    ),
                    "{first:?}"
                );
                let second = next(&mut rx).await;
                assert!(
                    matches!(
                        &second,
                        Input::Session {
                            request: SessionRequest::SwitchUser,
                            ..
                        }
                    ),
                    "{second:?}"
                );
            })
            .await;
    }

    #[test]
    fn refusals_past_the_cap_are_not_recorded() {
        let refusals = Refusals::new(Listener::Session);
        for uid in 0..MAX_REFUSALS as u32 {
            assert!(refusals.first(uid));
        }
        assert!(!refusals.first(5000), "a uid past the cap");
        assert!(!refusals.first(5001));
        assert!(!refusals.first(0));
        assert_eq!(refusals.uids.borrow().len(), MAX_REFUSALS);
    }

    #[tokio::test]
    async fn a_greeter_hello_flood_is_read_through_and_folded() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (_gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                let hello = encode(&GreeterRequest::Hello).unwrap();
                let flood = tokio::task::spawn_local(async move {
                    for _ in 0..FLOOD {
                        c.write_all(&hello).await.unwrap();
                    }
                    send(&mut c, &GreeterRequest::CancelSession).await;
                    c
                });
                let c = tokio::time::timeout(Duration::from_secs(5), flood)
                    .await
                    .expect("the reader keeps reading while the queue is full")
                    .unwrap();
                read_through(&c).await;
                let cancel = Input::GreeterRequest(GREETER, GreeterRequest::CancelSession);
                let mut heard = vec![next(&mut rx).await];
                while heard.last() != Some(&cancel) {
                    heard.push(next(&mut rx).await);
                }
                assert!(heard.len() <= QUEUE + 2, "{} heard", heard.len());
                assert!(rx.try_recv().is_err(), "nothing follows the CancelSession");
                assert_eq!(
                    heard[heard.len() - 2..],
                    [
                        Input::GreeterRequest(GREETER, GreeterRequest::Hello),
                        Input::GreeterRequest(GREETER, GreeterRequest::CancelSession),
                    ]
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_hello_flooding_greeter_that_reads_nothing_is_dropped_once_its_queue_fills() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (gate, mut rx) = serve_in(dir.path(), ids().0, "c1");
                let mut c = UnixStream::connect(dir.path().join("greeter.sock"))
                    .await
                    .unwrap();
                let flood = tokio::task::spawn_local(async move {
                    let hello = encode(&GreeterRequest::Hello).unwrap();
                    while c.write_all(&hello).await.is_ok() {}
                });
                let big = GreeterReply::Error {
                    kind: repriselogin::greeter::ErrorKind::Failed,
                    text: "x".repeat(60_000),
                };
                let mut dropped = None;
                for _ in 0..1000 {
                    assert_eq!(
                        next(&mut rx).await,
                        Input::GreeterRequest(GREETER, GreeterRequest::Hello)
                    );
                    dropped = gate.borrow_mut().reply(&big);
                    if dropped.is_some() {
                        break;
                    }
                }
                assert_eq!(dropped, Some(GREETER), "the greeter is dropped");
                assert!(gate.borrow().link.is_none());
                tokio::time::timeout(Duration::from_secs(5), flood)
                    .await
                    .expect("the dropped greeter's writes fail")
                    .unwrap();
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
