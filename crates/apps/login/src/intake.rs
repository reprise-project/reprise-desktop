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
/// Hand-overs one link holds at once, those being written or not yet taken included; past it a
/// forward is refused.
pub const MAX_FORWARDS: usize = 4;
/// The wait before a link that did not open is tried again.
pub const RETRY: Duration = Duration::from_millis(50);
/// Reads of a broken link's last reports, enough for a full socket buffer and no more, so a
/// server that keeps writing cannot hold the daemon.
const DRAIN_READS: usize = 16;
/// A link passes a viewer count on at most this often; meanwhile only the latest is kept.
pub const VIEWERS_EVERY: Duration = Duration::from_millis(250);
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
    // On a thread of its own: a FUSE mount over the directory can hang these calls for good,
    // and a caller's timeout then abandons that thread, not the loop or the blocking pool.
    let (tx, rx) = oneshot::channel();
    let dir = dir.to_owned();
    std::thread::Builder::new()
        .name("intake-open".into())
        .spawn(move || {
            let _ = tx.send(socket_at(&dir, uid));
        })?;
    let socket = rx.await.map_err(io::Error::other)??;
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

/// One link per uid, each in a task of the current `LocalSet`. A link counts as open once its
/// server's first report arrives; one that does not within `deadline` is reported lost. A
/// hand-over counts as forwarded once its server reports it took it.
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
    /// False until its server first reports.
    up: bool,
    queue: Rc<Queue>,
    pace: Rc<Pace>,
    task: AbortHandle,
}

/// The wait between an opening link's tries, which a waiting login shortens, and when it gives
/// up, which a later open or reopen extends.
struct Pace {
    every: Cell<Duration>,
    until: Cell<Instant>,
    hurried: Notify,
}

impl Pace {
    fn hurry(&self, every: Duration) {
        if every < self.every.get() {
            self.every.set(every);
            self.hurried.notify_one();
        }
    }

    fn extend(&self, until: Instant) {
        if until > self.until.get() {
            self.until.set(until);
        }
    }

    async fn wait(&self) {
        let _ = tokio::time::timeout(self.every.get(), self.hurried.notified()).await;
    }
}

/// One link's hand-overs: those waiting, the one being written, and those written that its
/// server has not reported taking.
#[derive(Default)]
struct Queue {
    waiting: RefCell<VecDeque<Forward>>,
    /// The conn being written, and the sender that stops its write.
    writing: RefCell<Option<(u64, oneshot::Sender<()>)>>,
    untaken: RefCell<Vec<Forward>>,
    /// Hand-overs reported taken so far.
    takes: Cell<u64>,
    queued: Notify,
}

impl Queue {
    fn len(&self) -> usize {
        self.waiting.borrow().len()
            + usize::from(self.writing.borrow().is_some())
            + self.untaken.borrow().len()
    }

    /// Drops `conn` from those written, closing its socket; false when none is `conn`.
    fn took(&self, conn: u64) -> bool {
        let mut untaken = self.untaken.borrow_mut();
        let at = untaken.iter().position(|f| f.conn == conn);
        let took = at.map(|at| untaken.remove(at)).is_some();
        if took {
            self.takes.set(self.takes.get() + 1);
        }
        took
    }

    /// Puts those written but not taken back in front of those waiting, in the order written.
    fn requeue(&self) {
        let mut waiting = self.waiting.borrow_mut();
        for f in self.untaken.take().into_iter().rev() {
            waiting.push_front(f);
        }
    }
}

/// A link's viewer counts on their way to the daemon: one passes at once, and those within
/// `VIEWERS_EVERY` of it wait, only the latest kept.
struct Counts {
    held: Option<u32>,
    /// When the next count may pass.
    next: tokio::time::Instant,
}

impl Counts {
    /// After the first report, which passed just now.
    fn new() -> Counts {
        Counts {
            held: None,
            next: tokio::time::Instant::now() + VIEWERS_EVERY,
        }
    }

    /// The held count once its turn has come.
    fn due(&mut self, now: tokio::time::Instant) -> Option<u32> {
        if now < self.next {
            return None;
        }
        let count = self.held.take()?;
        self.next = now + VIEWERS_EVERY;
        Some(count)
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

    /// Opens `uid`'s link, trying every [`RETRY`]; answered once by `IntakeOpen` after the server's
    /// first report or by `IntakeLost`, and at once by `IntakeOpen` when the link is up.
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
            Some(Link {
                up: true, serial, ..
            }) => {
                let (input, state, serial) = (self.input.clone(), self.state.clone(), *serial);
                // A link that drops before this runs has sent its IntakeLost, the answer.
                tokio::task::spawn_local(async move {
                    if state.borrow_mut().current(uid, serial).is_some() {
                        let _ = input.send(Input::IntakeOpen { uid }).await;
                    }
                });
                return;
            }
            Some(link) => {
                link.pace.hurry(retry.every);
                return link.pace.extend(Instant::now() + retry.within);
            }
            None => {}
        }
        st.serials += 1;
        let serial = st.serials;
        let pace = Rc::new(Pace {
            every: Cell::new(retry.every),
            until: Cell::new(Instant::now() + retry.within),
            hurried: Notify::new(),
        });
        let queue = Rc::new(Queue::default());
        let run = LinkTask {
            uid,
            serial,
            queue: queue.clone(),
            open: self.open.clone(),
            pace: pace.clone(),
            deadline: self.deadline,
            input: self.input.clone(),
            state: self.state.clone(),
        };
        let task = tokio::task::spawn_local(run.run()).abort_handle();
        let link = Link {
            serial,
            up: false,
            queue,
            pace,
            task,
        };
        st.links.insert(uid, link);
    }

    /// Queues `handover` with `fd` on `uid`'s link, answered by `Forwarded`; a link still opening
    /// tries every [`RETRY`] from then, and a uid with none gets one opened. False, closing `fd`,
    /// when the link holds [`MAX_FORWARDS`] or the hand-over does not fit a frame.
    pub fn forward(&self, uid: u32, conn: u64, fd: OwnedFd, handover: Box<HandOver>) -> bool {
        let frame = match frame::encode(&IntakeMessage::HandOver(handover)) {
            Ok(frame) => frame,
            Err(e) => {
                log::warn!("the hand-over of conn {conn}: {}", ipc::error_kind(&e));
                return false;
            }
        };
        if !self.state.borrow().links.contains_key(&uid) {
            log::info!("uid {uid} has no hand-over link for conn {conn}; opening one");
            self.open(uid);
        }
        let st = self.state.borrow();
        let link = &st.links[&uid];
        if link.queue.len() >= MAX_FORWARDS {
            log::warn!("uid {uid}'s session reads no hand-overs; refusing conn {conn}");
            return false;
        }
        if !link.up {
            link.pace.hurry(RETRY);
        }
        (link.queue.waiting.borrow_mut()).push_back(Forward { conn, frame, fd });
        link.queue.queued.notify_one();
        true
    }

    /// Drops `conn`'s hand-over and closes its socket if a link still holds it, written or not.
    /// One being written takes its link down, since part of its frame may be out.
    pub fn drop_conn(&self, conn: u64) {
        let st = self.state.borrow();
        for queue in st.links.values().map(|l| &l.queue) {
            queue.waiting.borrow_mut().retain(|f| f.conn != conn);
            queue.untaken.borrow_mut().retain(|f| f.conn != conn);
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

/// The task of `uid`'s link `serial`, opening it at its pace.
struct LinkTask {
    uid: u32,
    serial: u64,
    queue: Rc<Queue>,
    open: Opener,
    pace: Rc<Pace>,
    deadline: Duration,
    input: mpsc::Sender<Input>,
    state: Rc<RefCell<State>>,
}

impl LinkTask {
    /// Opens and serves the link. Hand-overs a server leaves untaken go first to the next one
    /// socket activation starts, until `deadline` passes with no server taking one.
    async fn run(self) {
        let LinkTask {
            uid,
            serial,
            queue,
            open,
            pace,
            deadline,
            input,
            state,
        } = self;
        let mut opened = false;
        let mut give_up: Option<Instant> = None;
        loop {
            let takes = queue.takes.get();
            match open_until(uid, &open, &pace).await {
                Ok((stream, reader, count)) => {
                    match state.borrow_mut().current(uid, serial) {
                        Some(link) => link.up = true,
                        None => return,
                    }
                    let told = opened || input.send(Input::IntakeOpen { uid }).await.is_ok();
                    opened = true;
                    if told
                        && input.send(Input::Viewers { uid, count }).await.is_ok()
                        && let Err(e) = relay(uid, &stream, reader, &queue, &input).await
                    {
                        log::warn!("dropped uid {uid}'s hand-over link: {e}");
                    }
                }
                Err(e) => {
                    log::warn!("uid {uid}'s hand-over socket: {e}");
                    break;
                }
            }
            if input.is_closed() || state.borrow_mut().current(uid, serial).is_none() {
                return;
            }
            queue.requeue();
            let left = queue.waiting.borrow().len();
            if left == 0 {
                break;
            }
            log::info!(
                "uid {uid}'s hand-over server left {left} hand-overs untaken; opening the next"
            );
            let now = Instant::now();
            let at = match give_up {
                Some(at) if queue.takes.get() == takes => at,
                _ => now + deadline,
            };
            give_up = Some(at);
            pace.until.set(at);
            pace.every.set(RETRY);
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
}

/// Tries `open` at `pace` until a link's server reports or the pace's `until` has passed. A
/// connect alone proves nothing: systemd holds the socket while the server is down or restarting.
async fn open_until(
    uid: u32,
    open: &Opener,
    pace: &Pace,
) -> io::Result<(UnixStream, FdFrameReader, u32)> {
    loop {
        let mut tried = std::pin::pin!(async {
            let stream = open(uid).await?;
            let (reader, count) = first_report(&stream).await?;
            Ok::<_, io::Error>((stream, reader, count))
        });
        let tried = loop {
            let until = pace.until.get();
            match tokio::time::timeout_at(until.into(), &mut tried).await {
                Ok(tried) => break tried,
                Err(_) if pace.until.get() > until => {}
                Err(_) => return Err(io::ErrorKind::TimedOut.into()),
            }
        };
        match tried {
            Ok(opened) => return Ok(opened),
            Err(e) if Instant::now() + pace.every.get() >= pace.until.get() => return Err(e),
            Err(_) => pace.wait().await,
        }
    }
}

/// Reads `stream` until its first report, the viewer count the server sends as it accepts.
async fn first_report(stream: &UnixStream) -> io::Result<(FdFrameReader, u32)> {
    let mut reader = FdFrameReader::new();
    loop {
        stream.readable().await?;
        match stream.try_io(Interest::READABLE, || reader.read_from(stream.as_fd())) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(e) => return Err(e),
        }
        match next_report(&mut reader)? {
            Some(IntakeReport::Viewers { count }) => return Ok((reader, count)),
            Some(IntakeReport::Took { conn }) => {
                return Err(io::Error::other(format!(
                    "conn {conn} reported taken on a new link"
                )));
            }
            None => {}
        }
    }
}

/// The next whole report `reader` holds.
fn next_report(reader: &mut FdFrameReader) -> io::Result<Option<IntakeReport>> {
    let broken = |e| io::Error::other(ipc::error_kind(&e));
    let Some((body, fd)) = reader.next().map_err(broken)? else {
        return Ok(None);
    };
    if fd.is_some() {
        return Err(io::Error::other("a socket came with a report"));
    }
    frame::decode(&body).map(Some).map_err(broken)
}

/// Writes `queue`'s hand-overs and feeds the reports after `reader`'s first to `input` until
/// the link breaks, then those the server sent before it did.
async fn relay(
    uid: u32,
    stream: &UnixStream,
    mut reader: FdFrameReader,
    queue: &Queue,
    input: &mpsc::Sender<Input>,
) -> io::Result<()> {
    let mut counts = Counts::new();
    let relayed = relay_until_broken(uid, stream, &mut reader, queue, &mut counts, input).await;
    let mut reads = 0;
    while matches!(
        feed(uid, &mut reader, queue, &mut counts, input).await,
        Ok(true)
    ) && reads < DRAIN_READS
        && matches!(reader.read_from(stream.as_fd()), Ok(1..))
    {
        reads += 1;
    }
    if let Some(count) = counts.held {
        let _ = input.send(Input::Viewers { uid, count }).await;
    }
    relayed
}

/// Feeds the reports `reader` holds to `input`, a viewer count once its turn has come; false
/// once `input` is closed.
async fn feed(
    uid: u32,
    reader: &mut FdFrameReader,
    queue: &Queue,
    counts: &mut Counts,
    input: &mpsc::Sender<Input>,
) -> io::Result<bool> {
    while let Some(report) = next_report(reader)? {
        let fed = match report {
            IntakeReport::Viewers { count } => {
                counts.held = Some(count);
                continue;
            }
            IntakeReport::Took { conn } if queue.took(conn) => Input::Forwarded { conn, ok: true },
            IntakeReport::Took { .. } => continue,
        };
        if input.send(fed).await.is_err() {
            return Ok(false);
        }
    }
    match counts.due(tokio::time::Instant::now()) {
        Some(count) => Ok(input.send(Input::Viewers { uid, count }).await.is_ok()),
        None => Ok(true),
    }
}

/// [`relay`] until the link breaks. A hand-over whose write fails goes back in front of those
/// waiting.
async fn relay_until_broken(
    uid: u32,
    stream: &UnixStream,
    reader: &mut FdFrameReader,
    queue: &Queue,
    counts: &mut Counts,
    input: &mpsc::Sender<Input>,
) -> io::Result<()> {
    loop {
        if !feed(uid, reader, queue, counts, input).await? {
            return Ok(());
        }
        let next = queue.waiting.borrow_mut().pop_front();
        if let Some(f) = next {
            match write(stream, queue, &f).await {
                Ok(true) => queue.untaken.borrow_mut().push(f),
                Ok(false) => {
                    let conn = f.conn;
                    drop(f);
                    if input
                        .send(Input::Forwarded { conn, ok: false })
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("conn {conn}'s hand-over was not read by its deadline"),
                    ));
                }
                Err(e) => {
                    queue.waiting.borrow_mut().push_front(f);
                    return Err(e);
                }
            }
            continue;
        }
        let turn = counts.held.map(|_| counts.next);
        tokio::select! {
            () = tokio::time::sleep_until(turn.unwrap_or(counts.next)), if turn.is_some() => {}
            ready = stream.readable() => {
                ready?;
                match stream.try_io(Interest::READABLE, || reader.read_from(stream.as_fd())) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
            }
            () = queue.queued.notified() => {}
        }
    }
}

/// Writes `f` as `queue`'s one being written: true once done, false when [`Links::drop_conn`]
/// stops it.
async fn write(stream: &UnixStream, queue: &Queue, f: &Forward) -> io::Result<bool> {
    let (stop, stopped) = oneshot::channel();
    *queue.writing.borrow_mut() = Some((f.conn, stop));
    let sent = tokio::select! {
        sent = send_with_fd(stream, &f.frame, f.fd.as_fd()) => sent.map(|()| true),
        Ok(()) = stopped => Ok(false),
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
        send_report(stream, &IntakeReport::Viewers { count }).await;
    }

    async fn took(stream: &UnixStream, conn: u64) {
        send_report(stream, &IntakeReport::Took { conn }).await;
    }

    async fn send_report(stream: &UnixStream, report: &IntakeReport) {
        let frame = frame::encode(report).unwrap();
        stream.writable().await.unwrap();
        assert_eq!(stream.try_write(&frame).unwrap(), frame.len());
    }

    /// The next hand-over the server end `rdp` reads, with the fd that came along.
    async fn read_hand_over(rdp: &UnixStream) -> (HandOver, Option<OwnedFd>) {
        let mut reader = FdFrameReader::new();
        let (body, fd) = loop {
            if let Some(got) = reader.next().unwrap() {
                break got;
            }
            tokio::time::timeout(WAIT, rdp.readable())
                .await
                .expect("a hand-over in time")
                .unwrap();
            let read = rdp.try_io(Interest::READABLE, || reader.read_from(rdp.as_fd()));
            match read {
                Ok(n) => assert_ne!(n, 0, "the link stays open"),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                Err(e) => panic!("{e}"),
            }
        };
        let IntakeMessage::HandOver(got) = frame::decode(&body).unwrap();
        (*got, fd)
    }

    /// Links on `dir` and the server end of `uid`'s first link, opened with no viewers.
    async fn linked_at(
        dir: &Path,
        listener: &UnixListener,
        deadline: Duration,
    ) -> (Links, Receiver<Input>, UnixStream) {
        let (links, mut rx) = links_at(dir, deadline);
        let uid = own_uid();
        links.open(uid);
        let (rdp, _) = listener.accept().await.unwrap();
        report(&rdp, 0).await;
        opened(&mut rx, uid).await;
        (links, rx, rdp)
    }

    #[tokio::test]
    async fn a_link_passes_on_its_latest_viewer_count_at_most_every_viewers_every() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (_links, mut rx, rdp) = linked_at(dir.path(), &listener, WAIT).await;
                let uid = own_uid();
                let mut flips = Vec::new();
                for count in (0..400).map(|n| 1 - n % 2).chain([3]) {
                    flips.extend_from_slice(
                        &frame::encode(&IntakeReport::Viewers { count }).unwrap(),
                    );
                }
                tokio::time::pause();
                let rdp = Rc::new(rdp);
                let writer = rdp.clone();
                tokio::task::spawn_local(async move {
                    let mut sent = 0;
                    while sent < flips.len() {
                        writer.writable().await.unwrap();
                        match writer.try_write(&flips[sent..]) {
                            Ok(n) => sent += n,
                            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                            Err(e) => panic!("{e}"),
                        }
                    }
                });
                let mut got = Vec::new();
                let until = tokio::time::Instant::now() + VIEWERS_EVERY * 3;
                while let Ok(Some(input)) = tokio::time::timeout_at(until, rx.recv()).await {
                    got.push(input);
                }
                assert!(got.len() <= 3, "{} inputs: {got:?}", got.len());
                assert_eq!(got.last(), Some(&Input::Viewers { uid, count: 3 }));
                tokio::time::resume();

                report(&rdp, 1).await;
                report(&rdp, 0).await;
                drop(Rc::try_unwrap(rdp).expect("the writer is done"));
                let mut got = Vec::new();
                while let Ok(Some(input)) = tokio::time::timeout(WAIT, rx.recv()).await {
                    got.push(input);
                    if got.last() == Some(&Input::IntakeLost { uid }) {
                        break;
                    }
                }
                assert_eq!(
                    got[got.len() - 2..],
                    [Input::Viewers { uid, count: 0 }, Input::IntakeLost { uid }],
                    "the count held when the link drops goes first"
                );
            })
            .await;
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
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                report(&rdp, 2).await;
                assert_eq!(next(&mut rx).await, Input::IntakeOpen { uid });
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

                let (got, fd) = read_hand_over(&rdp).await;
                assert_eq!(got, hand_over(9));
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                took(&rdp, 9).await;
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: true });
                let mut handed = TcpStream::from(fd.expect("the client's socket came along"));
                handed.write_all(b"hello").unwrap();
                let mut text = [0u8; 5];
                client.read_exact(&mut text).unwrap();
                assert_eq!(&text, b"hello");

                drop(rdp);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
            })
            .await;
    }

    /// Links whose every link has a full send buffer and a peer, kept in the returned list, that
    /// reports no viewers and never reads, so no hand-over is ever written.
    fn stalled_links() -> (Links, Receiver<Input>, Rc<RefCell<Vec<UnixStream>>>) {
        let (tx, rx) = mpsc::channel(ipc::QUEUE);
        let peers: Rc<RefCell<Vec<UnixStream>>> = Rc::default();
        let open: Opener = {
            let peers = peers.clone();
            Rc::new(move |_uid| {
                let (ours, peer) = std::os::unix::net::UnixStream::pair().unwrap();
                ours.set_nonblocking(true).unwrap();
                peer.set_nonblocking(true).unwrap();
                let none = frame::encode(&IntakeReport::Viewers { count: 0 }).unwrap();
                (&peer).write_all(&none).unwrap();
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

    /// Expects `uid`'s link to open with its stand-in's report of no viewers.
    async fn opened(rx: &mut Receiver<Input>, uid: u32) {
        assert_eq!(next(rx).await, Input::IntakeOpen { uid });
        assert_eq!(next(rx).await, Input::Viewers { uid, count: 0 });
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
                opened(&mut rx, uid).await;
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
                opened(&mut rx, uid).await;
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
                assert_closed(&mut ends[0]);
                assert_eq!(
                    next(&mut rx).await,
                    Input::Viewers { uid, count: 0 },
                    "the one behind it goes on a new link"
                );
                assert!(is_open(&mut ends[2]));
                nothing_within(&mut rx, Duration::from_millis(100)).await;

                links.drop_conn(3);
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 3, ok: false });
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert_closed(&mut ends[2]);
            })
            .await;
    }

    #[tokio::test]
    async fn an_open_asked_as_its_link_drops_is_answered_lost_only() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (links, mut rx, _peers) = stalled_links();
                let uid = own_uid();
                links.open(uid);
                opened(&mut rx, uid).await;
                let (pipe, _w) = watched_pipe();
                assert!(links.forward(uid, 1, pipe, Box::new(hand_over(1))));
                nothing_within(&mut rx, Duration::from_millis(100)).await;

                links.drop_conn(1);
                links.open(uid);
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 1, ok: false });
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                nothing_within(&mut rx, Duration::from_millis(200)).await;
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
                let listener = UnixListener::bind(rdp.join(SOCKET)).unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                report(&stream, 0).await;
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
                let mut got = Vec::new();
                for _ in 0..4 {
                    got.push(next(&mut rx).await);
                }
                for uid in [uid, uid + 1] {
                    assert!(got.contains(&Input::IntakeOpen { uid }), "{got:?}");
                    assert!(got.contains(&Input::Viewers { uid, count: 0 }), "{got:?}");
                }
                links.close_all_but(&BTreeSet::from([uid]));
                let (pipe, mut w) = watched_pipe();
                assert!(links.forward(uid, 1, pipe, Box::new(hand_over(1))));
                assert!(is_open(&mut w));
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                assert_eq!(peers.borrow().len(), 2, "the kept link serves its forward");

                let (pipe, _w) = watched_pipe();
                assert!(links.forward(uid + 1, 2, pipe, Box::new(hand_over(2))));
                opened(&mut rx, uid + 1).await;
                assert_eq!(
                    peers.borrow().len(),
                    3,
                    "the closed link's uid gets a new one"
                );
                drop(peers);
            })
            .await;
    }

    #[tokio::test]
    async fn a_link_whose_server_never_reports_is_lost_at_the_deadline() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, _listening) = runtime_dir();
                let (links, mut rx) = links_at(dir.path(), Duration::from_millis(300));
                let uid = own_uid();
                links.open(uid);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
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

    #[tokio::test]
    async fn a_hand_over_written_to_a_stopping_server_reaches_the_next_one() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (links, mut rx, stopping) = linked_at(dir.path(), &listener, WAIT).await;
                let uid = own_uid();
                let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client = TcpStream::connect(tcp.local_addr().unwrap()).unwrap();
                let (server, _) = tcp.accept().unwrap();
                assert!(links.forward(uid, 9, server.into(), Box::new(hand_over(9))));
                let (_, fd) = read_hand_over(&stopping).await;
                drop(fd);
                report(&stopping, 0).await;
                drop(stopping);

                let (rdp, _) = tokio::time::timeout(WAIT, listener.accept())
                    .await
                    .expect("the link opens again")
                    .unwrap();
                report(&rdp, 0).await;
                assert_eq!(next(&mut rx).await, Input::Viewers { uid, count: 0 });
                assert_eq!(next(&mut rx).await, Input::Viewers { uid, count: 0 });
                let (got, fd) = read_hand_over(&rdp).await;
                assert_eq!(got, hand_over(9));
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                took(&rdp, 9).await;
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: true });
                let mut handed = TcpStream::from(fd.expect("the client's socket came along"));
                handed.write_all(b"hello").unwrap();
                let mut text = [0u8; 5];
                client.read_exact(&mut text).unwrap();
                assert_eq!(&text, b"hello");
            })
            .await;
    }

    #[tokio::test]
    async fn a_hand_over_no_server_takes_fails_at_the_deadline() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let deadline = Duration::from_millis(300);
                let (links, mut rx, stopping) = linked_at(dir.path(), &listener, deadline).await;
                let uid = own_uid();
                let (pipe, mut pipe_w) = watched_pipe();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                let (_, fd) = read_hand_over(&stopping).await;
                drop(fd);
                let stopped = Instant::now();
                drop(stopping);
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: false });
                assert!(stopped.elapsed() >= Duration::from_millis(250));
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert_closed(&mut pipe_w);
            })
            .await;
    }

    #[tokio::test]
    async fn a_hand_over_every_server_drops_fails_within_one_deadline() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let deadline = Duration::from_millis(300);
                let (links, mut rx, first) = linked_at(dir.path(), &listener, deadline).await;
                let uid = own_uid();
                let (pipe, mut pipe_w) = watched_pipe();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                let servers = tokio::task::spawn_local(async move {
                    let mut rdp = first;
                    loop {
                        let _ = rdp.readable().await;
                        drop(rdp);
                        let Ok((next, _)) = listener.accept().await else {
                            return;
                        };
                        report(&next, 0).await;
                        rdp = next;
                    }
                });
                let started = Instant::now();
                let ends = async {
                    let mut ends = Vec::new();
                    while ends.len() < 2 {
                        match next(&mut rx).await {
                            Input::Viewers { .. } => {}
                            got => ends.push(got),
                        }
                    }
                    ends
                };
                let ends = tokio::time::timeout(deadline * 4, ends)
                    .await
                    .expect("the reopens end within the deadline");
                assert_eq!(
                    ends,
                    [
                        Input::Forwarded { conn: 9, ok: false },
                        Input::IntakeLost { uid }
                    ]
                );
                assert!(started.elapsed() >= deadline - Duration::from_millis(50));
                assert_closed(&mut pipe_w);
                servers.abort();
            })
            .await;
    }

    #[tokio::test]
    async fn a_took_the_server_sent_before_its_link_broke_is_not_sent_again() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                type Ends = Vec<(UnixStream, std::os::unix::net::UnixStream)>;
                let ends: Rc<RefCell<Ends>> = Rc::default();
                let open: Opener = {
                    let ends = ends.clone();
                    Rc::new(move |_uid| {
                        let (ours, peer) = std::os::unix::net::UnixStream::pair().unwrap();
                        ours.set_nonblocking(true).unwrap();
                        peer.set_nonblocking(true).unwrap();
                        let none = frame::encode(&IntakeReport::Viewers { count: 0 }).unwrap();
                        (&peer).write_all(&none).unwrap();
                        let filler = ours.try_clone().unwrap();
                        let peer = UnixStream::from_std(peer).unwrap();
                        ends.borrow_mut().push((peer, filler));
                        let ours = UnixStream::from_std(ours);
                        Box::pin(async move { ours })
                    })
                };
                let links = Links::new(tx, open, WAIT);
                let uid = own_uid();
                links.open(uid);
                opened(&mut rx, uid).await;
                let (rdp, filler) = ends.borrow_mut().remove(0);
                let (pipe, mut taken_w) = watched_pipe();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                assert_eq!(read_hand_over(&rdp).await.0.conn, 9);
                let chunk = [0u8; 4096];
                while (&filler).write(&chunk).is_ok() {}
                while (&filler).write(&chunk[..1]).is_ok() {}
                drop(filler);
                let mut held = Vec::new();
                for conn in [10, 11] {
                    let (pipe, w) = watched_pipe();
                    assert!(links.forward(uid, conn, pipe, Box::new(hand_over(conn))));
                    held.push(w);
                }
                nothing_within(&mut rx, Duration::from_millis(100)).await;

                took(&rdp, 9).await;
                took(&rdp, 11).await;
                drop(rdp);
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: true });
                assert_closed(&mut taken_w);
                assert_eq!(
                    next(&mut rx).await,
                    Input::Viewers { uid, count: 0 },
                    "the link opens again"
                );
                let (rdp, _filler) = ends.borrow_mut().remove(0);
                assert_eq!(read_hand_over(&rdp).await.0.conn, 10);
                assert_eq!(
                    read_hand_over(&rdp).await.0.conn,
                    11,
                    "a Took for a hand-over not yet written is ignored"
                );
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                for w in &mut held {
                    assert!(is_open(w), "the hand-overs not taken are still held");
                }
            })
            .await;
    }

    #[tokio::test]
    async fn an_untaken_hand_over_counts_toward_max_forwards_until_dropped() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (links, mut rx, rdp) = linked_at(dir.path(), &listener, WAIT).await;
                let uid = own_uid();
                let mut ends = Vec::new();
                for conn in 0..MAX_FORWARDS as u64 {
                    let (pipe, w) = watched_pipe();
                    assert!(links.forward(uid, conn, pipe, Box::new(hand_over(conn))));
                    let (got, fd) = read_hand_over(&rdp).await;
                    assert_eq!(got.conn, conn);
                    drop(fd);
                    ends.push(w);
                }
                let (pipe, mut refused) = watched_pipe();
                assert!(
                    !links.forward(uid, 99, pipe, Box::new(hand_over(99))),
                    "hand-overs written but not taken count toward MAX_FORWARDS"
                );
                assert_closed(&mut refused);
                assert!(
                    is_open(&mut ends[1]),
                    "an untaken hand-over's socket is held"
                );

                links.drop_conn(1);
                assert_closed(&mut ends[1]);
                took(&rdp, 1).await;
                nothing_within(&mut rx, Duration::from_millis(100)).await;
                let (pipe, _w) = watched_pipe();
                assert!(links.forward(uid, 5, pipe, Box::new(hand_over(5))));
                took(&rdp, 0).await;
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 0, ok: true });
                assert_closed(&mut ends[0]);
            })
            .await;
    }

    #[tokio::test]
    async fn a_hand_over_sent_after_its_link_closed_reaches_the_next_server() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (links, mut rx, stopped) = linked_at(dir.path(), &listener, WAIT).await;
                let uid = own_uid();
                drop(stopped);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });

                let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client = TcpStream::connect(tcp.local_addr().unwrap()).unwrap();
                let (server, _) = tcp.accept().unwrap();
                assert!(links.forward(uid, 9, server.into(), Box::new(hand_over(9))));
                let (rdp, _) = tokio::time::timeout(WAIT, listener.accept())
                    .await
                    .expect("the hand-over opens the link again")
                    .unwrap();
                report(&rdp, 0).await;
                opened(&mut rx, uid).await;
                let (got, fd) = read_hand_over(&rdp).await;
                assert_eq!(got, hand_over(9));
                took(&rdp, 9).await;
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: true });
                let mut handed = TcpStream::from(fd.expect("the client's socket came along"));
                handed.write_all(b"hello").unwrap();
                let mut text = [0u8; 5];
                client.read_exact(&mut text).unwrap();
                assert_eq!(&text, b"hello");
            })
            .await;
    }

    #[tokio::test]
    async fn a_hand_over_sent_after_its_link_closed_fails_at_the_deadline_with_no_server() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let deadline = Duration::from_millis(300);
                let (links, mut rx, stopped) = linked_at(dir.path(), &listener, deadline).await;
                let uid = own_uid();
                drop(stopped);
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                drop(listener);

                let (pipe, mut pipe_w) = watched_pipe();
                let sent = Instant::now();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                assert_eq!(next(&mut rx).await, Input::Forwarded { conn: 9, ok: false });
                assert!(sent.elapsed() >= Duration::from_millis(250));
                assert_eq!(next(&mut rx).await, Input::IntakeLost { uid });
                assert_closed(&mut pipe_w);
            })
            .await;
    }

    #[tokio::test]
    async fn a_hand_over_sent_while_its_link_reopens_waits_for_it_trying_every_retry() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let (open, up) = gated_opener(dir.path());
                let retry = Retry {
                    every: Duration::from_secs(1),
                    within: WAIT,
                };
                let links = Links::new(tx, open, WAIT).reopening(retry);
                let uid = own_uid();
                links.reopen(uid);
                tokio::time::sleep(Duration::from_millis(100)).await;
                up.set(true);

                let (pipe, _w) = watched_pipe();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                let (rdp, _) = tokio::time::timeout(Duration::from_millis(500), listener.accept())
                    .await
                    .expect("a waiting hand-over tries every RETRY, not every second")
                    .unwrap();
                report(&rdp, 0).await;
                opened(&mut rx, uid).await;
                let (got, _fd) = read_hand_over(&rdp).await;
                assert_eq!(got, hand_over(9));
            })
            .await;
    }

    /// Opens at `dir` once the flag is set; NotFound before, as with no server's socket yet.
    fn gated_opener(dir: &Path) -> (Opener, Rc<Cell<bool>>) {
        let up = Rc::new(Cell::new(false));
        let open: Opener = {
            let (at, up) = (dir.to_path_buf(), up.clone());
            Rc::new(move |uid| {
                let (at, up) = (at.clone(), up.get());
                Box::pin(async move {
                    if !up {
                        return Err(io::ErrorKind::NotFound.into());
                    }
                    open_at(&at, uid).await
                })
            })
        };
        (open, up)
    }

    #[tokio::test]
    async fn a_reopen_asked_while_a_forward_opens_its_link_tries_as_long_as_a_reopen() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (dir, listener) = runtime_dir();
                let (tx, mut rx) = mpsc::channel(ipc::QUEUE);
                let (open, up) = gated_opener(dir.path());
                let retry = Retry {
                    every: Duration::from_secs(1),
                    within: WAIT,
                };
                let links = Links::new(tx, open, Duration::from_millis(300)).reopening(retry);
                let uid = own_uid();
                let (pipe, _w) = watched_pipe();
                assert!(links.forward(uid, 9, pipe, Box::new(hand_over(9))));
                links.reopen(uid);
                tokio::time::sleep(Duration::from_millis(500)).await;
                up.set(true);

                let (rdp, _) = tokio::time::timeout(WAIT, listener.accept())
                    .await
                    .expect("the link tries past the forward's 300 ms until the reopen's deadline")
                    .unwrap();
                report(&rdp, 0).await;
                opened(&mut rx, uid).await;
                let (got, _fd) = read_hand_over(&rdp).await;
                assert_eq!(got, hand_over(9));
            })
            .await;
    }
}
