//! The front's link on `FRONT_SOCKET`: one connection at a time, from the front's uid. The
//! client sockets its `Ready` frames carry wait here by conn until forwarded or dropped.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::net::UnixListener as StdListener;
use std::rc::Rc;
use std::time::Duration;

use repriselogin::fdpass::FdFrameReader;
use repriselogin::frame::{self, FrameError};
use repriselogin::front::{FrontEvent, FrontRequest};
use repriselogin::handover::HandOver;
use tokio::io::Interest;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, mpsc};
use zeroize::Zeroizing;

use crate::daemon::Input;
use crate::ipc;

/// Most client sockets held at once; a `Ready` past it drops the link.
pub const MAX_READY: usize = 32;
/// Events waiting for the front to read them; past it, the front is dropped.
const LINK_QUEUE: usize = 64;

/// A client's socket and the hand-over its `Ready` carried.
pub type Held = (OwnedFd, Box<HandOver>);

/// The linked front's outbound queue, and the sockets its `Ready` frames passed. Every held
/// socket closes when the link ends.
#[derive(Default)]
pub struct Front {
    link: Option<Link>,
    ready: HashMap<u64, Held>,
    serials: u64,
}

struct Link {
    serial: u64,
    tx: mpsc::Sender<Zeroizing<Vec<u8>>>,
    /// Ends the link's reader, which then reports `FrontLost`.
    close: Rc<Notify>,
}

impl Front {
    /// Queues `event` for the linked front; a front whose queue is full is dropped.
    pub fn send(&mut self, event: &FrontEvent) {
        let Some(link) = &self.link else {
            return log::debug!("no front is linked for {event:?}");
        };
        let frame = match frame::encode(event) {
            Ok(frame) => frame,
            Err(e) => return log::warn!("front: {}", ipc::error_kind(&e)),
        };
        if link.tx.try_send(frame).is_err() {
            log::warn!("the front reads nothing; dropping its link");
            link.close.notify_one();
        }
    }

    /// The socket and hand-over of `conn`'s `Ready`, if one is held.
    pub fn take(&mut self, conn: u64) -> Option<Held> {
        self.ready.remove(&conn)
    }

    /// Ends link `serial`, closing its held sockets; false when another link is current.
    fn end(&mut self, serial: u64) -> bool {
        if !self.link.as_ref().is_some_and(|l| l.serial == serial) {
            return false;
        }
        self.link = None;
        self.ready.clear();
        true
    }
}

/// Accepts the front's link on `listener` in a task of the current `LocalSet`. Its requests
/// go to `input` as `Front`, its `Ready` frames as `FrontReady` with the socket kept in
/// `front`.
pub fn serve(
    listener: StdListener,
    front_uid: u32,
    input: mpsc::Sender<Input>,
    front: Rc<RefCell<Front>>,
) -> io::Result<()> {
    let listener = UnixListener::from_std(listener)?;
    tokio::task::spawn_local(accept(listener, front_uid, input, front));
    Ok(())
}

async fn accept(
    listener: UnixListener,
    front_uid: u32,
    input: mpsc::Sender<Input>,
    front: Rc<RefCell<Front>>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                log::warn!("front socket: accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        match stream.peer_cred().map(|c| c.uid()) {
            Ok(uid) if uid == front_uid => {}
            Ok(uid) => {
                log::warn!("front socket: refused uid {uid}");
                continue;
            }
            Err(e) => {
                log::warn!("front socket: no peer credentials: {e}");
                continue;
            }
        }
        let mut f = front.borrow_mut();
        if f.link.is_some() {
            log::warn!("front socket: refused a second link while one lives");
            continue;
        }
        f.serials += 1;
        let serial = f.serials;
        let (tx, rx) = mpsc::channel(LINK_QUEUE);
        let close = Rc::new(Notify::new());
        f.link = Some(Link {
            serial,
            tx,
            close: close.clone(),
        });
        drop(f);
        let (input, front) = (input.clone(), front.clone());
        tokio::task::spawn_local(link(stream, serial, rx, close, input, front));
    }
}

async fn link(
    stream: UnixStream,
    serial: u64,
    mut rx: mpsc::Receiver<Zeroizing<Vec<u8>>>,
    close: Rc<Notify>,
    input: mpsc::Sender<Input>,
    front: Rc<RefCell<Front>>,
) {
    let (rd, wr) = stream.into_split();
    let writer = tokio::task::spawn_local(ipc::write_frames(wr, async move || rx.recv().await));
    if input.send(Input::FrontLinked).await.is_ok() {
        match read(rd.as_ref(), &input, &front, &close).await {
            Ok(()) => log::info!("the front closed its link"),
            Err(e) => log::warn!("dropped the front's link: {e}"),
        }
    }
    writer.abort();
    if front.borrow_mut().end(serial) {
        let _ = input.send(Input::FrontLost).await;
    }
}

/// Feeds the front's frames to `input` until it closes the link or breaks a rule.
async fn read(
    stream: &UnixStream,
    input: &mpsc::Sender<Input>,
    front: &RefCell<Front>,
    close: &Notify,
) -> io::Result<()> {
    let mut reader = FdFrameReader::new();
    loop {
        tokio::select! {
            () = close.notified() => return Err(io::Error::other("its queue is full")),
            ready = stream.readable() => {
                ready?;
                match stream.try_io(Interest::READABLE, || reader.read_from(stream.as_fd())) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => return Err(e),
                }
                while let Some((body, fd)) = reader.next().map_err(broken)? {
                    let request = admit(&body, fd, front)?;
                    if input.send(request).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
}

fn broken(e: FrameError) -> io::Error {
    io::Error::other(ipc::error_kind(&e))
}

/// The input for one frame. Only `Ready` carries a socket, which it must.
fn admit(body: &[u8], fd: Option<OwnedFd>, front: &RefCell<Front>) -> io::Result<Input> {
    let request: FrontRequest = frame::decode(body).map_err(broken)?;
    match (request, fd) {
        (FrontRequest::Ready { conn, handover }, Some(fd)) => {
            if handover.conn != conn {
                return Err(io::Error::other(
                    "a Ready whose hand-over names another conn",
                ));
            }
            let mut f = front.borrow_mut();
            if f.ready.contains_key(&conn) {
                return Err(io::Error::other("a second Ready for one conn"));
            }
            if f.ready.len() >= MAX_READY {
                return Err(io::Error::other("more Ready sockets than are held"));
            }
            let user = handover.user.clone();
            f.ready.insert(conn, (fd, handover));
            Ok(Input::FrontReady { conn, user })
        }
        (FrontRequest::Ready { .. }, None) => Err(io::Error::other("a Ready without its socket")),
        (_, Some(_)) => Err(io::Error::other("a socket on a frame other than Ready")),
        (request, None) => Ok(Input::Front(request)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use repriselogin::fdpass;
    use tokio::io::AsyncReadExt;
    use tokio::sync::mpsc::Receiver;

    use super::*;
    use crate::intake::tests::{assert_closed, hand_over, watched_pipe};

    const WAIT: Duration = Duration::from_secs(5);

    fn own_uid() -> u32 {
        // SAFETY: getuid has no preconditions.
        unsafe { libc::getuid() }
    }

    fn own_gid() -> u32 {
        // SAFETY: getgid has no preconditions.
        unsafe { libc::getgid() }
    }

    struct Served {
        path: std::path::PathBuf,
        rx: Receiver<Input>,
        front: Rc<RefCell<Front>>,
        _dir: tempfile::TempDir,
    }

    fn serve_for(front_uid: u32) -> Served {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let listener = ipc::bind_group(&path, own_gid()).unwrap();
        let (tx, rx) = mpsc::channel(ipc::QUEUE);
        let front = Rc::new(RefCell::new(Front::default()));
        serve(listener, front_uid, tx, front.clone()).unwrap();
        Served {
            path,
            rx,
            front,
            _dir: dir,
        }
    }

    async fn next(rx: &mut Receiver<Input>) -> Input {
        tokio::time::timeout(WAIT, rx.recv())
            .await
            .expect("an input in time")
            .expect("the channel is open")
    }

    async fn nothing_within(rx: &mut Receiver<Input>, wait: Duration) {
        if let Ok(got) = tokio::time::timeout(wait, rx.recv()).await {
            panic!("no input was due, got {got:?}");
        }
    }

    /// Whether the peer closes `stream` within `WAIT`.
    async fn closed(stream: &mut UnixStream) -> bool {
        let mut buf = [0u8; 256];
        loop {
            match tokio::time::timeout(WAIT, stream.read(&mut buf)).await {
                Ok(Ok(0)) | Ok(Err(_)) => return true,
                Ok(Ok(_)) => continue,
                Err(_) => return false,
            }
        }
    }

    async fn send(stream: &UnixStream, request: &FrontRequest, fd: Option<&OwnedFd>) {
        let frame = frame::encode(request).unwrap();
        stream.writable().await.unwrap();
        let sent = fdpass::send(stream.as_fd(), &frame, fd.map(|f| f.as_fd())).unwrap();
        assert_eq!(sent, frame.len());
    }

    fn ready(conn: u64) -> FrontRequest {
        FrontRequest::Ready {
            conn,
            handover: Box::new(hand_over(conn)),
        }
    }

    #[tokio::test]
    async fn the_front_socket_takes_only_the_front_user() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut s = serve_for(own_uid() + 1);
                let mut stream = UnixStream::connect(&s.path).await.unwrap();
                send(&stream, &FrontRequest::Hello, None).await;
                assert!(
                    closed(&mut stream).await,
                    "a link from another uid is closed"
                );
                nothing_within(&mut s.rx, Duration::from_millis(200)).await;
            })
            .await;
    }

    #[tokio::test]
    async fn a_ready_without_its_fd_drops_the_link() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut s = serve_for(own_uid());
                let mut stream = UnixStream::connect(&s.path).await.unwrap();
                assert_eq!(next(&mut s.rx).await, Input::FrontLinked);
                send(&stream, &ready(7), None).await;
                assert_eq!(next(&mut s.rx).await, Input::FrontLost);
                assert!(closed(&mut stream).await);
                let mut stream = UnixStream::connect(&s.path).await.unwrap();
                assert_eq!(next(&mut s.rx).await, Input::FrontLinked);
                let (pipe, _w) = watched_pipe();
                send(&stream, &FrontRequest::Hello, Some(&pipe)).await;
                assert_eq!(
                    next(&mut s.rx).await,
                    Input::FrontLost,
                    "a socket on another frame drops the link too"
                );
                assert!(closed(&mut stream).await);
            })
            .await;
    }

    #[tokio::test]
    async fn a_second_front_link_is_refused_while_one_lives() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut s = serve_for(own_uid());
                let first = UnixStream::connect(&s.path).await.unwrap();
                assert_eq!(next(&mut s.rx).await, Input::FrontLinked);
                let mut second = UnixStream::connect(&s.path).await.unwrap();
                assert!(closed(&mut second).await, "the second link is closed");
                send(&first, &FrontRequest::Hello, None).await;
                assert_eq!(
                    next(&mut s.rx).await,
                    Input::Front(FrontRequest::Hello),
                    "the first link lives on"
                );
                drop(first);
                assert_eq!(next(&mut s.rx).await, Input::FrontLost);
                let _third = UnixStream::connect(&s.path).await.unwrap();
                assert_eq!(next(&mut s.rx).await, Input::FrontLinked);
            })
            .await;
    }

    #[tokio::test]
    async fn a_ready_socket_is_held_by_conn_until_taken_or_the_link_ends() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut s = serve_for(own_uid());
                let mut stream = UnixStream::connect(&s.path).await.unwrap();
                assert_eq!(next(&mut s.rx).await, Input::FrontLinked);
                let (taken, mut taken_w) = watched_pipe();
                let (kept, mut kept_w) = watched_pipe();
                send(&stream, &ready(7), Some(&taken)).await;
                send(&stream, &ready(8), Some(&kept)).await;
                drop((taken, kept));
                for conn in [7, 8] {
                    let want = Input::FrontReady {
                        conn,
                        user: "anna".into(),
                    };
                    assert_eq!(next(&mut s.rx).await, want);
                }
                let (fd, handover) = s.front.borrow_mut().take(7).expect("conn 7 is held");
                assert_eq!(handover.conn, 7);
                drop(fd);
                assert_closed(&mut taken_w);
                kept_w.write_all(b"x").unwrap();

                s.front.borrow_mut().send(&FrontEvent::Accepted { conn: 8 });
                let mut got = vec![0u8; 64];
                let n = tokio::time::timeout(WAIT, stream.read(&mut got))
                    .await
                    .unwrap()
                    .unwrap();
                let body = &got[4..n];
                assert_eq!(
                    frame::decode::<FrontEvent>(body).unwrap(),
                    FrontEvent::Accepted { conn: 8 }
                );

                drop(stream);
                assert_eq!(next(&mut s.rx).await, Input::FrontLost);
                assert_closed(&mut kept_w);
                assert!(s.front.borrow_mut().take(8).is_none());
            })
            .await;
    }
}
