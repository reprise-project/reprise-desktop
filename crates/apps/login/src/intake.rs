//! Links to each user's hand-over socket, `/run/user/<uid>/reprise-rdp/handover.sock`, whose
//! peer must run as that user: hand-overs go out with their sockets, viewer counts come back.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap, VecDeque};
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
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::AbortHandle;
use zeroize::Zeroizing;

use crate::daemon::Input;
use crate::ipc;

const RUNTIME: &str = "/run/user";
const DIR: &str = "reprise-rdp";
const SOCKET: &str = "handover.sock";
/// Hand-overs one link holds at once, the one being written included; past it a forward is
/// refused.
pub const MAX_FORWARDS: usize = 4;
/// The wait before a link that did not open is tried again.
pub const RETRY: Duration = Duration::from_millis(50);
/// How a lost link is opened again: every second for half a minute.
pub const REOPEN: Retry = Retry {
    every: Duration::from_secs(1),
    within: Duration::from_secs(30),
};

/// Tries every `every` until one opens or `within` has passed.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    pub every: Duration,
    pub within: Duration,
}

/// Connects to `uid`'s hand-over socket.
pub async fn open(uid: u32) -> io::Result<UnixStream> {
    open_at(&Path::new(RUNTIME).join(uid.to_string()), uid).await
}

/// [`open`] with `dir` for `/run/user/<uid>`. Both directories must be `uid`'s, writable by
/// nobody else and no symlinks; the socket must be `uid`'s, and so must its peer.
pub async fn open_at(dir: &Path, uid: u32) -> io::Result<UnixStream> {
    let socket = socket_at(dir, uid)?;
    connect_to(&socket, uid).await
}

/// An `O_PATH` fd of `dir`'s hand-over socket after checking it and both directories.
fn socket_at(dir: &Path, uid: u32) -> io::Result<OwnedFd> {
    let runtime = open_owned(None, dir, libc::S_IFDIR, uid)?;
    let rdp = open_owned(Some(runtime.as_fd()), Path::new(DIR), libc::S_IFDIR, uid)?;
    open_owned(Some(rdp.as_fd()), Path::new(SOCKET), libc::S_IFSOCK, uid)
}

fn refused(why: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, why)
}

/// An `O_PATH` fd of `path`, which must be a `kind` owned by `uid` and, for a directory,
/// writable by nobody else. Below `at`, a symlink or a mount crossing anywhere is refused.
fn open_owned(
    at: Option<BorrowedFd<'_>>,
    path: &Path,
    kind: libc::mode_t,
    uid: u32,
) -> io::Result<OwnedFd> {
    let cpath = CString::new(path.as_os_str().as_bytes())?;
    let flags = libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = match at {
        // SAFETY: `cpath` is a NUL-terminated string that outlives the call.
        None => unsafe { libc::openat(libc::AT_FDCWD, cpath.as_ptr(), flags) },
        Some(at) => {
            // SAFETY: `open_how` is a plain C struct; all zeroes is a valid value.
            let mut how: libc::open_how = unsafe { std::mem::zeroed() };
            how.flags = flags as u64;
            how.resolve = libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_XDEV;
            // SAFETY: `cpath` and `how` outlive the call, and `how`'s size is passed with it.
            let fd = unsafe {
                libc::syscall(
                    libc::SYS_openat2,
                    at.as_raw_fd(),
                    cpath.as_ptr(),
                    &how,
                    std::mem::size_of::<libc::open_how>(),
                )
            };
            fd as libc::c_int
        }
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the open just returned this fd, and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: `libc::stat` is a plain C struct; all zeroes is a valid value.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: `fd` is open and `st` outlives the call.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let shared = kind == libc::S_IFDIR && st.st_mode & 0o022 != 0;
    if st.st_mode & libc::S_IFMT != kind || st.st_uid != uid || shared {
        return Err(refused(format!(
            "{} is not uid {uid}'s alone",
            path.display()
        )));
    }
    Ok(fd)
}

/// Connects to the socket `socket` names through its `/proc/self/fd` link, which resolves to
/// that inode with no further lookup.
async fn connect_to(socket: &OwnedFd, uid: u32) -> io::Result<UnixStream> {
    let path = PathBuf::from(format!("/proc/self/fd/{}", socket.as_raw_fd()));
    connect(&path, uid).await
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
    reopen: Retry,
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
    queue: Option<Rc<Queue>>,
    pace: Rc<Pace>,
    task: AbortHandle,
}

/// The wait between an opening link's tries, which a login waiting on a reopen shortens.
struct Pace {
    every: Cell<Duration>,
    hurried: Notify,
}

impl Pace {
    fn hurry(&self, every: Duration) {
        if every < self.every.get() {
            self.every.set(every);
            self.hurried.notify_one();
        }
    }

    async fn wait(&self) {
        let _ = tokio::time::timeout(self.every.get(), self.hurried.notified()).await;
    }
}

/// One link's hand-overs: those waiting, and the one being written.
#[derive(Default)]
struct Queue {
    waiting: RefCell<VecDeque<Forward>>,
    /// The conn being written, and the sender that stops its write.
    writing: RefCell<Option<(u64, oneshot::Sender<()>)>>,
    queued: Notify,
}

impl Queue {
    fn len(&self) -> usize {
        self.waiting.borrow().len() + usize::from(self.writing.borrow().is_some())
    }
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
            reopen: REOPEN,
        }
    }

    /// Links that reopen a lost link on `reopen` instead of [`REOPEN`].
    pub fn reopening(self, reopen: Retry) -> Links {
        Links { reopen, ..self }
    }

    /// Opens `uid`'s link, answered by `IntakeOpen` or `IntakeLost`. A link that is opening
    /// already answers once and tries every [`RETRY`]; one that is up answers `IntakeOpen` again.
    pub fn open(&self, uid: u32) {
        let retry = Retry {
            every: RETRY,
            within: self.deadline,
        };
        self.open_with(uid, retry);
    }

    /// Opens `uid`'s lost link again, answered like [`Links::open`].
    pub fn reopen(&self, uid: u32) {
        self.open_with(uid, self.reopen);
    }

    fn open_with(&self, uid: u32, retry: Retry) {
        let mut st = self.state.borrow_mut();
        match st.links.get(&uid) {
            Some(Link { queue: Some(_), .. }) => {
                let input = self.input.clone();
                tokio::task::spawn_local(async move {
                    let _ = input.send(Input::IntakeOpen { uid }).await;
                });
                return;
            }
            Some(link) => return link.pace.hurry(retry.every),
            None => {}
        }
        st.serials += 1;
        let serial = st.serials;
        let pace = Rc::new(Pace {
            every: Cell::new(retry.every),
            hurried: Notify::new(),
        });
        let run = run(
            uid,
            serial,
            self.open.clone(),
            pace.clone(),
            retry.within,
            self.input.clone(),
            self.state.clone(),
        );
        let task = tokio::task::spawn_local(run).abort_handle();
        let link = Link {
            serial,
            queue: None,
            pace,
            task,
        };
        st.links.insert(uid, link);
    }

    /// Queues `handover` with `fd` on `uid`'s link, answered by `Forwarded`. False, closing
    /// `fd`, when the link is not up, holds [`MAX_FORWARDS`], or the hand-over does not fit
    /// a frame.
    pub fn forward(&self, uid: u32, conn: u64, fd: OwnedFd, handover: Box<HandOver>) -> bool {
        let st = self.state.borrow();
        let Some(queue) = st.links.get(&uid).and_then(|l| l.queue.as_ref()) else {
            return false;
        };
        if queue.len() >= MAX_FORWARDS {
            log::warn!("uid {uid}'s session reads no hand-overs; refusing conn {conn}");
            return false;
        }
        match frame::encode(&IntakeMessage::HandOver(handover)) {
            Ok(frame) => {
                queue
                    .waiting
                    .borrow_mut()
                    .push_back(Forward { conn, frame, fd });
                queue.queued.notify_one();
                true
            }
            Err(e) => {
                log::warn!("the hand-over of conn {conn}: {}", ipc::error_kind(&e));
                false
            }
        }
    }

    /// Drops `conn`'s hand-over and closes its socket if a link still holds it. One being
    /// written takes its link down, since part of its frame may be out.
    pub fn drop_conn(&self, conn: u64) {
        let st = self.state.borrow();
        for queue in st.links.values().filter_map(|l| l.queue.as_ref()) {
            queue.waiting.borrow_mut().retain(|f| f.conn != conn);
            let writing = queue.writing.borrow_mut().take_if(|(c, _)| *c == conn);
            if let Some((_, stop)) = writing {
                let _ = stop.send(());
            }
        }
    }

    /// Closes every link but those of `keep`; none reports itself lost.
    pub fn close_all_but(&self, keep: &BTreeSet<u32>) {
        let mut st = self.state.borrow_mut();
        st.links.retain(|uid, link| {
            let kept = keep.contains(uid);
            if !kept {
                link.task.abort();
            }
            kept
        });
    }
}

/// Opens and serves link `serial`. When it ends, every hand-over still waiting counts as
/// failed.
async fn run(
    uid: u32,
    serial: u64,
    open: Opener,
    pace: Rc<Pace>,
    within: Duration,
    input: mpsc::Sender<Input>,
    state: Rc<RefCell<State>>,
) {
    let queue = Rc::new(Queue::default());
    match open_until(uid, &open, &pace, within).await {
        Ok(stream) => {
            match state.borrow_mut().current(uid, serial) {
                Some(link) => link.queue = Some(queue.clone()),
                None => return,
            }
            if input.send(Input::IntakeOpen { uid }).await.is_ok()
                && let Err(e) = relay(uid, &stream, &queue, &input).await
            {
                log::warn!("dropped uid {uid}'s hand-over link: {e}");
            }
        }
        Err(e) => log::warn!("uid {uid}'s hand-over socket: {e}"),
    }
    if state.borrow_mut().current(uid, serial).is_none() {
        return;
    }
    state.borrow_mut().links.remove(&uid);
    let failed: Vec<_> = queue.waiting.take().into_iter().map(|f| f.conn).collect();
    for conn in failed {
        if input
            .send(Input::Forwarded { conn, ok: false })
            .await
            .is_err()
        {
            return;
        }
    }
    let _ = input.send(Input::IntakeLost { uid }).await;
}

/// Tries `open` at `pace` until it connects or `within` has passed.
async fn open_until(
    uid: u32,
    open: &Opener,
    pace: &Pace,
    within: Duration,
) -> io::Result<UnixStream> {
    let give_up = Instant::now() + within;
    loop {
        let left = give_up.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, open(uid)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) if Instant::now() + pace.every.get() >= give_up => return Err(e),
            Ok(Err(_)) => pace.wait().await,
            Err(_) => return Err(io::ErrorKind::TimedOut.into()),
        }
    }
}

/// Writes `queue`'s hand-overs and feeds reports to `input` until the link breaks.
async fn relay(
    uid: u32,
    stream: &UnixStream,
    queue: &Queue,
    input: &mpsc::Sender<Input>,
) -> io::Result<()> {
    let mut reader = FdFrameReader::new();
    loop {
        let next = queue.waiting.borrow_mut().pop_front();
        if let Some(f) = next {
            let sent = write(stream, queue, &f).await;
            let conn = f.conn;
            drop(f);
            let ok = sent.is_ok();
            if input.send(Input::Forwarded { conn, ok }).await.is_err() {
                return Ok(());
            }
            sent?;
            continue;
        }
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
            () = queue.queued.notified() => {}
        }
    }
}

/// Writes `f` as `queue`'s one being written, until done or [`Links::drop_conn`] stops it.
async fn write(stream: &UnixStream, queue: &Queue, f: &Forward) -> io::Result<()> {
    let (stop, stopped) = oneshot::channel();
    *queue.writing.borrow_mut() = Some((f.conn, stop));
    let sent = tokio::select! {
        sent = send_with_fd(stream, &f.frame, f.fd.as_fd()) => sent,
        Ok(()) = stopped => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("conn {}'s hand-over was not read by its deadline", f.conn),
        )),
    };
    queue.writing.borrow_mut().take();
    sent
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

        let (other, other_l) = runtime_dir();
        let socket = plain.path().join(DIR).join(SOCKET);
        std::fs::remove_file(&socket).unwrap();
        symlink(other.path().join(DIR).join(SOCKET), &socket).unwrap();
        assert!(
            open_at(plain.path(), own_uid()).await.is_err(),
            "handover.sock as a symlink is refused"
        );
        no_connection(&other_l).await;
    }

    async fn no_connection(listener: &UnixListener) {
        let got = tokio::time::timeout(Duration::from_millis(200), listener.accept()).await;
        assert!(got.is_err(), "nothing connects to the other socket");
    }

    #[tokio::test]
    async fn a_socket_swapped_for_a_symlink_after_its_check_is_not_followed() {
        let (dir, checked) = runtime_dir();
        let (other, other_l) = runtime_dir();
        let socket = socket_at(dir.path(), own_uid()).unwrap();
        let path = dir.path().join(DIR).join(SOCKET);
        std::fs::rename(&path, dir.path().join(DIR).join("moved.sock")).unwrap();
        symlink(other.path().join(DIR).join(SOCKET), &path).unwrap();
        let _stream = connect_to(&socket, own_uid()).await.unwrap();
        let reached = tokio::time::timeout(WAIT, checked.accept()).await;
        assert!(reached.is_ok(), "the checked socket is the one reached");
        no_connection(&other_l).await;
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

    /// Links whose every link is one end of a pair whose send buffer is full and whose peer
    /// never reads, so no hand-over is ever written. The peers live in the returned list.
    fn stalled_links() -> (Links, Receiver<Input>, Rc<RefCell<Vec<UnixStream>>>) {
        let (tx, rx) = mpsc::channel(ipc::QUEUE);
        let peers: Rc<RefCell<Vec<UnixStream>>> = Rc::default();
        let open: Opener = {
            let peers = peers.clone();
            Rc::new(move |_uid| {
                let (ours, peer) = std::os::unix::net::UnixStream::pair().unwrap();
                ours.set_nonblocking(true).unwrap();
                peer.set_nonblocking(true).unwrap();
                let chunk = [0u8; 4096];
                while (&ours).write(&chunk).is_ok() {}
                while (&ours).write(&chunk[..1]).is_ok() {}
                peers.borrow_mut().push(UnixStream::from_std(peer).unwrap());
                let ours = UnixStream::from_std(ours);
                Box::pin(async move { ours })
            })
        };
        (Links::new(tx, open, WAIT), rx, peers)
    }

    async fn nothing_within(rx: &mut Receiver<Input>, wait: Duration) {
        if let Ok(got) = tokio::time::timeout(wait, rx.recv()).await {
            panic!("no input was due, got {got:?}");
        }
    }

    fn is_open(w: &mut io::PipeWriter) -> bool {
        w.write(b"x").is_ok()
    }

    #[tokio::test]
    async fn a_link_that_reads_nothing_holds_at_most_max_forwards() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (links, mut rx, _peers) = stalled_links();
                let uid = own_uid();
                links.open(uid);
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
                let mut held = Vec::new();
                for conn in 0..MAX_FORWARDS as u64 {
                    let (pipe, w) = watched_pipe();
                    assert!(links.forward(uid, conn, pipe, Box::new(hand_over(conn))));
                    held.push(w);
                }
                let (pipe, mut w) = watched_pipe();
                assert!(
                    !links.forward(uid, 99, pipe, Box::new(hand_over(99))),
                    "a forward past MAX_FORWARDS is refused"
                );
                assert_closed(&mut w);
                nothing_within(&mut rx, Duration::from_millis(200)).await;
                for w in &mut held {
                    assert!(is_open(w), "every queued socket is still held");
                }
            })
            .await;
    }

    #[tokio::test]
    async fn a_forward_dropped_at_its_deadline_closes_its_socket() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (links, mut rx, _peers) = stalled_links();
                let uid = own_uid();
                links.open(uid);
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
                let mut ends = Vec::new();
                for conn in 1..=3 {
                    let (pipe, w) = watched_pipe();
                    assert!(links.forward(uid, conn, pipe, Box::new(hand_over(conn))));
                    ends.push(w);
                }
                nothing_within(&mut rx, Duration::from_millis(100)).await;

                links.drop_conn(2);
                assert_closed(&mut ends[1]);
                assert!(is_open(&mut ends[0]), "the one being written is held");
                assert!(is_open(&mut ends[2]), "the one behind it is held");
                nothing_within(&mut rx, Duration::from_millis(100)).await;

                links.drop_conn(1);
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 1, ok: false });
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 3, ok: false });
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert_closed(&mut ends[0]);
                assert_closed(&mut ends[2]);
            })
            .await;
    }

    #[tokio::test]
    async fn a_reopen_tries_every_reopen_retry_until_reopen_for_then_is_lost() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let tries = Rc::new(std::cell::Cell::new(0));
                let open: Opener = {
                    let tries = tries.clone();
                    Rc::new(move |_uid| {
                        tries.set(tries.get() + 1);
                        Box::pin(async { Err(io::ErrorKind::NotFound.into()) })
                    })
                };
                let retry = Retry {
                    every: Duration::from_millis(100),
                    within: Duration::from_millis(450),
                };
                let links = Links::new(tx, open, WAIT).reopening(retry);
                let uid = own_uid();
                let asked = Instant::now();
                links.reopen(uid);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert!(asked.elapsed() >= Duration::from_millis(350));
                let tries = tries.get();
                assert!((3..=5).contains(&tries), "one try every 100 ms: {tries}");
            })
            .await;
    }

    #[tokio::test]
    async fn a_login_waiting_on_a_reopen_tries_every_retry() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, _rx) = mpsc::channel(ipc::QUEUE);
                let tries = Rc::new(std::cell::Cell::new(0));
                let open: Opener = {
                    let tries = tries.clone();
                    Rc::new(move |_uid| {
                        tries.set(tries.get() + 1);
                        Box::pin(async { Err(io::ErrorKind::NotFound.into()) })
                    })
                };
                let retry = Retry {
                    every: Duration::from_secs(1),
                    within: WAIT,
                };
                let links = Links::new(tx, open, WAIT).reopening(retry);
                let uid = own_uid();
                links.reopen(uid);
                tokio::time::sleep(Duration::from_millis(100)).await;
                assert_eq!(tries.get(), 1, "the reopen waits a second");
                links.open(uid);
                tokio::time::sleep(Duration::from_millis(400)).await;
                let tries = tries.get();
                assert!(
                    tries >= 5,
                    "one try every 50 ms once a login waits: {tries}"
                );
            })
            .await;
    }

    #[tokio::test]
    async fn a_reopen_links_once_the_socket_comes_back() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let dir = tempfile::tempdir().unwrap();
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let at = dir.path().to_path_buf();
                let open: Opener = Rc::new(move |uid| {
                    let at = at.clone();
                    Box::pin(async move { open_at(&at, uid).await })
                });
                let retry = Retry {
                    every: Duration::from_millis(100),
                    within: WAIT,
                };
                let links = Links::new(tx, open, Duration::from_millis(100)).reopening(retry);
                let uid = own_uid();
                links.reopen(uid);
                nothing_within(&mut rx, Duration::from_millis(300)).await;
                let rdp = dir.path().join(DIR);
                std::fs::create_dir(&rdp).unwrap();
                mode(&rdp, 0o700);
                let _listener = UnixListener::bind(rdp.join(SOCKET)).unwrap();
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
            })
            .await;
    }

    #[tokio::test]
    async fn closing_links_keeps_those_named() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (links, mut rx, peers) = stalled_links();
                let uid = own_uid();
                links.open(uid);
                links.open(uid + 1);
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid: uid + 1 });
                links.close_all_but(&BTreeSet::from([uid]));
                let (pipe, mut w) = watched_pipe();
                assert!(links.forward(uid, 1, pipe, Box::new(hand_over(1))));
                let (pipe, mut gone) = watched_pipe();
                assert!(!links.forward(uid + 1, 2, pipe, Box::new(hand_over(2))));
                assert_closed(&mut gone);
                assert!(is_open(&mut w));
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                drop(peers);
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
