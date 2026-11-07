//! The greeter's and the sessions' sockets: binding, peer checks and one reader per connection.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::Permissions;
use std::future::Future;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener as StdListener;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::time::Duration;

use repriselogin::frame::{self, FrameError, FrameReader};
use repriselogin::greeter::{GreeterReply, GreeterRequest};
use repriselogin::session::SessionRequest;
use repriselogin::{GREETER_SOCKET, SESSION_SOCKET};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::daemon::{Input, WorkerId};

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

/// What the listeners check peers against; the daemon's loop keeps it current.
pub struct Gate {
    pub greeter_uid: u32,
    /// The running greeter worker, and its logind session once it reported `Started`.
    pub greeter: Option<(WorkerId, Option<String>)>,
    /// The session table: logind session id to uid.
    pub sessions: HashMap<String, u32>,
    link: Option<Link>,
    links: u64,
}

/// The connected greeter's outbound queue.
struct Link {
    serial: u64,
    tx: mpsc::UnboundedSender<Zeroizing<Vec<u8>>>,
}

impl Gate {
    pub fn new(greeter_uid: u32) -> Gate {
        Gate {
            greeter_uid,
            greeter: None,
            sessions: HashMap::new(),
            link: None,
            links: 0,
        }
    }

    /// A new greeter worker runs; the previous greeter's connection is dropped.
    pub fn greeter_spawned(&mut self, worker: WorkerId) {
        self.greeter = Some((worker, None));
        self.link = None;
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
}

/// Looks up the logind session of a process id; logind in the daemon, a fake in tests.
pub type SessionOf = Rc<dyn Fn(u32) -> Pin<Box<dyn Future<Output = Option<String>>>>>;

pub struct Server {
    pub gate: Rc<RefCell<Gate>>,
    pub session_of: SessionOf,
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

async fn accept(l: UnixListener, kind: Listener, s: Rc<Server>) {
    loop {
        match l.accept().await {
            Ok((stream, _)) => {
                tokio::task::spawn_local(admit(stream, kind, s.clone()));
            }
            Err(e) => {
                log::warn!("{kind:?} socket: accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Checks the peer and starts its reader; a refused peer's stream is dropped, closing it.
async fn admit(stream: UnixStream, kind: Listener, s: Rc<Server>) {
    let Ok(cred) = stream.peer_cred() else {
        return;
    };
    let Some(pid) = cred.pid().and_then(|p| u32::try_from(p).ok()) else {
        return;
    };
    let session = (s.session_of)(pid).await;
    let mut gate = s.gate.borrow_mut();
    match kind {
        Listener::Greeter => {
            let greeter_session = gate.greeter.as_ref().and_then(|g| g.1.as_deref());
            let allowed = gate.link.is_none()
                && peer_allowed(
                    kind,
                    cred.uid(),
                    session.as_deref(),
                    gate.greeter_uid,
                    greeter_session,
                    None,
                );
            let Some((worker, _)) = gate.greeter.as_ref().filter(|_| allowed) else {
                log::warn!("greeter socket: refused uid {}", cred.uid());
                return;
            };
            let worker = *worker;
            gate.links += 1;
            let serial = gate.links;
            let (rd, wr) = stream.into_split();
            let (tx, rx) = mpsc::unbounded_channel();
            gate.link = Some(Link { serial, tx });
            drop(gate);
            tokio::task::spawn_local(write_frames(wr, rx));
            tokio::task::spawn_local(read_greeter(rd, serial, worker, s));
        }
        Listener::Session => {
            let uid = session
                .as_ref()
                .and_then(|id| gate.sessions.get(id).copied());
            let allowed = peer_allowed(
                kind,
                cred.uid(),
                session.as_deref(),
                gate.greeter_uid,
                None,
                uid,
            );
            drop(gate);
            match session.filter(|_| allowed) {
                Some(id) => {
                    tokio::task::spawn_local(read_session(stream, id, s));
                }
                None => log::warn!("session socket: refused uid {}", cred.uid()),
            }
        }
    }
}

async fn read_greeter(
    mut rd: tokio::net::unix::OwnedReadHalf,
    serial: u64,
    worker: WorkerId,
    s: Rc<Server>,
) {
    let why = read_frames(&mut rd, |body| {
        let request = frame::decode::<GreeterRequest>(&body)?;
        (s.input)(Input::GreeterRequest(request));
        Ok(())
    })
    .await;
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

async fn read_session(mut stream: UnixStream, id: String, s: Rc<Server>) {
    let why = read_frames(&mut stream, |body| {
        match frame::decode::<SessionRequest>(&body) {
            Ok(request) => (s.input)(Input::Session {
                id: id.clone(),
                request,
            }),
            Err(e) => log::warn!("session {id}: skipped {}", error_kind(&e)),
        }
        Ok(())
    })
    .await;
    if why != FrameError::Closed {
        log::warn!("session {id}: dropping its link: {}", error_kind(&why));
    }
}

/// Feeds what `rd` reads through a `FrameReader`, handing each frame to `on`, until the peer
/// closes, a read fails or `on` refuses a frame; returns why it stopped.
pub async fn read_frames<R: AsyncRead + Unpin>(
    rd: &mut R,
    mut on: impl FnMut(Zeroizing<Vec<u8>>) -> Result<(), FrameError>,
) -> FrameError {
    let mut reader = FrameReader::new();
    let mut buf = Zeroizing::new(vec![0u8; 4096]);
    loop {
        let n = match rd.read(&mut buf).await {
            Ok(0) => return FrameError::Closed,
            Ok(n) => n,
            Err(e) => return e.into(),
        };
        let pushed = reader.push(&buf[..n]);
        while let Some(body) = reader.next_frame() {
            if let Err(e) = on(body) {
                return e;
            }
        }
        if let Err(e) = pushed {
            return e;
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
    /// session "3"; every peer is in `peer_session`.
    fn serve_in(
        dir: &Path,
        greeter_uid: u32,
        peer_session: &'static str,
    ) -> (Rc<RefCell<Gate>>, UnboundedReceiver<Input>) {
        let (greeter, session) = bind(&paths(dir), ids().1).unwrap();
        let mut gate = Gate::new(greeter_uid);
        gate.greeter = Some((GREETER, Some("c1".into())));
        gate.sessions.insert("3".into(), ids().0);
        let gate = Rc::new(RefCell::new(gate));
        let (tx, rx) = mpsc::unbounded_channel();
        let server = Server {
            gate: gate.clone(),
            session_of: Rc::new(move |_| Box::pin(async move { Some(peer_session.into()) })),
            input: Rc::new(move |i| {
                let _ = tx.send(i);
            }),
        };
        serve(greeter, session, server).unwrap();
        (gate, rx)
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
                        request: SessionRequest::SwitchUser
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
}
