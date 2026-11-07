//! Links to each user's hand-over socket, `/run/user/<uid>/reprise-rdp/handover.sock`, whose
//! peer must run as that user: hand-overs go out with their sockets, viewer counts come back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::future::Future;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::time::{Duration, Instant};

use repriselogin::fdpass::{self, FdFrameReader};
use repriselogin::frame;
use repriselogin::handover::{HandOver, IntakeMessage, IntakeReport};
use tokio::io::Interest;
use tokio::net::UnixStream;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use zeroize::Zeroizing;

use crate::daemon::Input;
use crate::ipc;

const RUNTIME: &str = "/run/user";
const DIR: &str = "reprise-rdp";
const SOCKET: &str = "handover.sock";
/// The wait before a link that did not open is tried again.
pub const RETRY: Duration = Duration::from_millis(50);

/// Connects to `uid`'s hand-over socket.
pub async fn open(uid: u32) -> io::Result<UnixStream> {
    open_at(&Path::new(RUNTIME).join(uid.to_string()), uid).await
}

/// [`open`] with `dir` for `/run/user/<uid>`. Both directories must be `uid`'s, writable by
/// nobody else and no symlinks; the socket must be `uid`'s, and so must its peer.
pub async fn open_at(dir: &Path, uid: u32) -> io::Result<UnixStream> {
    let runtime = open_dir(None, dir, uid)?;
    let rdp = open_dir(Some(runtime.as_fd()), Path::new(DIR), uid)?;
    let st = stat_at(rdp.as_fd(), SOCKET)?;
    if st.st_mode & libc::S_IFMT != libc::S_IFSOCK || st.st_uid != uid {
        return Err(refused(format!("{SOCKET} is no socket of uid {uid}")));
    }
    let path = PathBuf::from(format!("/proc/self/fd/{}/{SOCKET}", rdp.as_raw_fd()));
    connect(&path, uid).await
}

fn refused(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why)
}

/// An `O_PATH` fd of directory `path`, relative to `at` when given, after checking it.
fn open_dir(at: Option<BorrowedFd<'_>>, path: &Path, uid: u32) -> io::Result<OwnedFd> {
    let cpath = CString::new(path.as_os_str().as_bytes())?;
    let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC;
    let at = at.map_or(libc::AT_FDCWD, |fd| fd.as_raw_fd());
    // SAFETY: `cpath` is a NUL-terminated string that outlives the call.
    let fd = unsafe { libc::openat(at, cpath.as_ptr(), flags) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat just returned this fd, and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let st = stat_at(fd.as_fd(), "")?;
    if st.st_mode & libc::S_IFMT != libc::S_IFDIR || st.st_uid != uid || st.st_mode & 0o022 != 0 {
        return Err(refused(format!(
            "{} is not uid {uid}'s alone",
            path.display()
        )));
    }
    Ok(fd)
}

/// `fstatat` of `name` in `dir` without following a symlink; `""` is `dir` itself.
fn stat_at(dir: BorrowedFd<'_>, name: &str) -> io::Result<libc::stat> {
    let cname = CString::new(name)?;
    // SAFETY: `libc::stat` is a plain C struct; all zeroes is a valid value.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let flags = libc::AT_SYMLINK_NOFOLLOW | libc::AT_EMPTY_PATH;
    // SAFETY: `cname` and `st` outlive the call.
    if unsafe { libc::fstatat(dir.as_raw_fd(), cname.as_ptr(), &mut st, flags) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(st)
}

/// Connects to `path` and checks that its peer runs as `uid`.
async fn connect(path: &Path, uid: u32) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path).await?;
    let peer = stream.peer_cred()?.uid();
    if peer != uid {
        return Err(refused(format!(
            "the hand-over socket's peer is uid {peer}"
        )));
    }
    Ok(stream)
}

/// Opens a link; production passes [`open`].
pub type Opener = Rc<dyn Fn(u32) -> Pin<Box<dyn Future<Output = io::Result<UnixStream>>>>>;

/// One link per uid, each in a task of the current `LocalSet`. A link that does not open
/// within `deadline` is reported lost.
pub struct Links {
    state: Rc<RefCell<State>>,
    input: mpsc::Sender<Input>,
    open: Opener,
    deadline: Duration,
}

#[derive(Default)]
struct State {
    links: HashMap<u32, Link>,
    serials: u64,
}

impl State {
    /// `uid`'s link if it is still link `serial`.
    fn current(&mut self, uid: u32, serial: u64) -> Option<&mut Link> {
        (self.links.get_mut(&uid)).filter(|l| l.serial == serial)
    }
}

struct Link {
    serial: u64,
    /// `None` while it opens.
    forwards: Option<mpsc::UnboundedSender<Forward>>,
    task: AbortHandle,
}

struct Forward {
    conn: u64,
    frame: Zeroizing<Vec<u8>>,
    fd: OwnedFd,
}

impl Links {
    pub fn new(input: mpsc::Sender<Input>, open: Opener, deadline: Duration) -> Links {
        Links {
            state: Rc::default(),
            input,
            open,
            deadline,
        }
    }

    /// Opens `uid`'s link, answered by `IntakeOpen` or `IntakeLost`. A link that is opening
    /// already answers once; one that is up answers `IntakeOpen` again.
    pub fn open(&self, uid: u32) {
        let mut st = self.state.borrow_mut();
        match st.links.get(&uid) {
            Some(Link {
                forwards: Some(_), ..
            }) => {
                let input = self.input.clone();
                tokio::task::spawn_local(async move {
                    let _ = input.send(Input::IntakeOpen { uid }).await;
                });
                return;
            }
            Some(_) => return,
            None => {}
        }
        st.serials += 1;
        let serial = st.serials;
        let run = run(
            uid,
            serial,
            self.open.clone(),
            self.deadline,
            self.input.clone(),
            self.state.clone(),
        );
        let task = tokio::task::spawn_local(run).abort_handle();
        let link = Link {
            serial,
            forwards: None,
            task,
        };
        st.links.insert(uid, link);
    }

    /// Sends `handover` with `fd` over `uid`'s link, answered by `Forwarded`. False, closing
    /// `fd`, when the link is not up or the hand-over does not fit a frame.
    pub fn forward(&self, uid: u32, conn: u64, fd: OwnedFd, handover: Box<HandOver>) -> bool {
        let st = self.state.borrow();
        let Some(tx) = st.links.get(&uid).and_then(|l| l.forwards.as_ref()) else {
            return false;
        };
        match frame::encode(&IntakeMessage::HandOver(handover)) {
            Ok(frame) => tx.send(Forward { conn, frame, fd }).is_ok(),
            Err(e) => {
                log::warn!("the hand-over of conn {conn}: {}", ipc::error_kind(&e));
                false
            }
        }
    }

    /// Closes every link; none reports itself lost.
    pub fn close_all(&self) {
        for (_, link) in self.state.borrow_mut().links.drain() {
            link.task.abort();
        }
    }
}

async fn run(
    uid: u32,
    serial: u64,
    open: Opener,
    deadline: Duration,
    input: mpsc::Sender<Input>,
    state: Rc<RefCell<State>>,
) {
    match open_until(uid, &open, deadline).await {
        Ok(stream) => {
            let (tx, rx) = mpsc::unbounded_channel();
            match state.borrow_mut().current(uid, serial) {
                Some(link) => link.forwards = Some(tx),
                None => return,
            }
            if input.send(Input::IntakeOpen { uid }).await.is_ok() {
                serve(uid, &stream, rx, &input).await;
            }
        }
        Err(e) => log::warn!("uid {uid}'s hand-over socket: {e}"),
    }
    let ended = state.borrow_mut().current(uid, serial).is_some();
    if ended {
        state.borrow_mut().links.remove(&uid);
        let _ = input.send(Input::IntakeLost { uid }).await;
    }
}

/// Tries `open` every `RETRY` until it connects or `deadline` passes.
async fn open_until(uid: u32, open: &Opener, deadline: Duration) -> io::Result<UnixStream> {
    let give_up = Instant::now() + deadline;
    loop {
        let left = give_up.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, open(uid)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) if Instant::now() + RETRY >= give_up => return Err(e),
            Ok(Err(_)) => tokio::time::sleep(RETRY).await,
            Err(_) => return Err(io::ErrorKind::TimedOut.into()),
        }
    }
}

/// Passes forwards on and feeds reports to `input` until the link breaks; a forward still
/// queued then counts as failed.
async fn serve(
    uid: u32,
    stream: &UnixStream,
    mut forwards: mpsc::UnboundedReceiver<Forward>,
    input: &mpsc::Sender<Input>,
) {
    if let Err(e) = relay(uid, stream, &mut forwards, input).await {
        log::warn!("dropped uid {uid}'s hand-over link: {e}");
    }
    forwards.close();
    while let Ok(f) = forwards.try_recv() {
        let failed = Input::Forwarded {
            conn: f.conn,
            ok: false,
        };
        if input.send(failed).await.is_err() {
            return;
        }
    }
}

async fn relay(
    uid: u32,
    stream: &UnixStream,
    forwards: &mut mpsc::UnboundedReceiver<Forward>,
    input: &mpsc::Sender<Input>,
) -> io::Result<()> {
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
                let broken = |e| io::Error::other(ipc::error_kind(&e));
                while let Some((body, fd)) = reader.next().map_err(broken)? {
                    if fd.is_some() {
                        return Err(io::Error::other("a socket came with a report"));
                    }
                    let IntakeReport::Viewers { count } = frame::decode(&body).map_err(broken)?;
                    if input.send(Input::Viewers { uid, count }).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Some(f) = forwards.recv() => {
                let sent = send_with_fd(stream, &f.frame, f.fd.as_fd()).await;
                let ok = sent.is_ok();
                if input.send(Input::Forwarded { conn: f.conn, ok }).await.is_err() {
                    return Ok(());
                }
                sent?;
            }
        }
    }
}

/// Writes `frame` with `fd` on its first `sendmsg`.
async fn send_with_fd(stream: &UnixStream, frame: &[u8], fd: BorrowedFd<'_>) -> io::Result<()> {
    let mut sent = 0;
    while sent < frame.len() {
        stream.writable().await?;
        let pass = (sent == 0).then_some(fd);
        let rest = &frame[sent..];
        match stream.try_io(Interest::WRITABLE, || {
            fdpass::send(stream.as_fd(), rest, pass)
        }) {
            Ok(n) => sent += n,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::{PermissionsExt, symlink};

    use repriselogin::handover::{Cipher, DirectionKeys, TlsState, TlsVersion};
    use tokio::net::UnixListener;
    use tokio::sync::mpsc::Receiver;

    use super::*;

    const WAIT: Duration = Duration::from_secs(5);

    pub(crate) fn hand_over(conn: u64) -> HandOver {
        let keys = || DirectionKeys {
            seq: 1,
            key: Zeroizing::new(vec![7; 16]),
            iv: Zeroizing::new(vec![9; 12]),
            secret: None,
        };
        HandOver {
            conn,
            user: "anna".into(),
            peer: "192.0.2.1:50000".parse().unwrap(),
            security: 1,
            steps: Vec::new(),
            tls: TlsState {
                version: TlsVersion::Tls12,
                cipher: Cipher::Aes128Gcm,
                tx: keys(),
                rx: keys(),
                record_limit: 1 << 24,
            },
        }
    }

    /// A pipe's read end, and its write end, which sees EPIPE once every read end closes.
    pub(crate) fn watched_pipe() -> (OwnedFd, io::PipeWriter) {
        let (r, w) = io::pipe().unwrap();
        (r.into(), w)
    }

    pub(crate) fn assert_closed(w: &mut io::PipeWriter) {
        let e = w.write(b"x").unwrap_err();
        assert_eq!(e.raw_os_error(), Some(libc::EPIPE), "the read end is open");
    }

    fn own_uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    /// A stand-in for `/run/user/<uid>` with `reprise-rdp/handover.sock` listening.
    fn runtime_dir() -> (tempfile::TempDir, UnixListener) {
        let dir = tempfile::tempdir().unwrap();
        let rdp = dir.path().join(DIR);
        std::fs::create_dir(&rdp).unwrap();
        std::fs::set_permissions(&rdp, std::fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(rdp.join(SOCKET)).unwrap();
        (dir, listener)
    }

    fn mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[tokio::test]
    async fn intake_refuses_a_socket_not_owned_by_the_user() {
        let (dir, _l) = runtime_dir();
        assert!(
            open_at(dir.path(), own_uid()).await.is_ok(),
            "the owner's own socket opens"
        );
        let e = open_at(dir.path(), own_uid() + 1).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");

        mode(dir.path(), 0o777);
        let e = open_at(dir.path(), own_uid()).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        mode(dir.path(), 0o700);

        let rdp = dir.path().join(DIR);
        mode(&rdp, 0o770);
        let e = open_at(dir.path(), own_uid()).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
        mode(&rdp, 0o700);

        let (elsewhere, _l2) = runtime_dir();
        let linked = tempfile::tempdir().unwrap();
        mode(linked.path(), 0o700);
        symlink(elsewhere.path().join(DIR), linked.path().join(DIR)).unwrap();
        assert!(
            open_at(linked.path(), own_uid()).await.is_err(),
            "reprise-rdp as a symlink is refused"
        );

        let plain = tempfile::tempdir().unwrap();
        mode(plain.path(), 0o700);
        std::fs::create_dir(plain.path().join(DIR)).unwrap();
        mode(&plain.path().join(DIR), 0o700);
        std::fs::write(plain.path().join(DIR).join(SOCKET), b"").unwrap();
        let e = open_at(plain.path(), own_uid()).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
    }

    #[tokio::test]
    async fn an_intake_peer_of_another_uid_is_refused() {
        let (dir, _l) = runtime_dir();
        let path = dir.path().join(DIR).join(SOCKET);
        assert!(connect(&path, own_uid()).await.is_ok());
        let e = connect(&path, own_uid() + 1).await.unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::PermissionDenied, "{e}");
    }

    async fn next(rx: &mut Receiver<Input>) -> Input {
        tokio::time::timeout(WAIT, rx.recv())
            .await
            .expect("an input in time")
            .expect("the channel is open")
    }

    fn links_at(dir: &Path, deadline: Duration) -> (Links, Receiver<Input>) {
        let (tx, rx) = mpsc::channel(ipc::QUEUE);
        let dir = dir.to_path_buf();
        let open: Opener = Rc::new(move |uid| {
            let dir = dir.clone();
            Box::pin(async move { open_at(&dir, uid).await })
        });
        (Links::new(tx, open, deadline), rx)
    }

    async fn report(stream: &UnixStream, count: u32) {
        let frame = frame::encode(&IntakeReport::Viewers { count }).unwrap();
        stream.writable().await.unwrap();
        assert_eq!(stream.try_write(&frame).unwrap(), frame.len());
    }

    #[tokio::test]
    async fn a_handover_reaches_the_intake_with_its_fd() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (links, mut rx) = links_at(dir.path(), WAIT);
                let uid = own_uid();
                links.open(uid);
                let (rdp, _) = listener.accept().await.unwrap();
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
                report(&rdp, 2).await;
                assert_eq!(next(&mut rx).await, Input::Viewers { uid, count: 2 });
                links.open(uid);
                assert_eq!(
                    next(&mut rx).await,
                    Input::IntakeOpen { uid },
                    "a link that is up answers again"
                );

                let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client = TcpStream::connect(tcp.local_addr().unwrap()).unwrap();
                let (server, _) = tcp.accept().unwrap();
                assert!(links.forward(uid, 9, server.into(), Box::new(hand_over(9))));
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: true });

                let mut reader = FdFrameReader::new();
                let (body, fd) = loop {
                    rdp.readable().await.unwrap();
                    let read = rdp.try_io(Interest::READABLE, || reader.read_from(rdp.as_fd()));
                    match read {
                        Ok(n) => assert_ne!(n, 0, "the link stays open"),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                        Err(e) => panic!("{e}"),
                    }
                    if let Some(got) = reader.next().unwrap() {
                        break got;
                    }
                };
                let IntakeMessage::HandOver(got) = frame::decode(&body).unwrap();
                assert_eq!(*got, hand_over(9));
                let mut handed = TcpStream::from(fd.expect("the client's socket came along"));
                handed.write_all(b"hello").unwrap();
                let mut text = [0u8; 5];
                client.read_exact(&mut text).unwrap();
                assert_eq!(&text, b"hello");

                drop(rdp);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                let (pipe, mut pipe_w) = watched_pipe();
                assert!(!links.forward(uid, 10, pipe, Box::new(hand_over(10))));
                assert_closed(&mut pipe_w);
            })
            .await;
    }

    #[tokio::test]
    async fn a_link_that_never_opens_is_lost_once_at_the_deadline() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (links, mut rx) = links_at(dir.path(), Duration::from_millis(300));
                let uid = own_uid();
                let asked = Instant::now();
                links.open(uid);
                links.open(uid);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert!(asked.elapsed() >= Duration::from_millis(250));
                let again = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
                assert!(again.is_err(), "one answer for both asks: {again:?}");
            })
            .await;
    }
}
