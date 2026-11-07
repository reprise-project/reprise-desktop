//! The greeter's socket to reprise-login: one blocking hello, then the replies on the event loop.
//! The socket stays blocking, so the loop reads it once per wake and the write half's sends finish.

use std::error::Error;
use std::io::{self, Read};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use repriselogin::frame::{self, FrameError, FrameReader};
use repriselogin::greeter::{GreeterReply, GreeterRequest, Greeting};
use smithay::reexports::calloop::{Interest, LoopHandle, Mode, PostAction, generic::Generic};

use crate::compositor::State;

/// How long a send waits on a reprise-login that stopped reading.
const SEND_TIMEOUT: Duration = Duration::from_secs(2);

pub struct GreeterLink {
    stream: UnixStream,
}

/// The link's write half, kept on `State` for the login screen's requests.
pub struct GreeterLinkTx(UnixStream);

impl GreeterLink {
    pub fn connect(path: &Path) -> io::Result<GreeterLink> {
        UnixStream::connect(path).map(GreeterLink::from_stream)
    }

    pub fn from_stream(stream: UnixStream) -> GreeterLink {
        GreeterLink { stream }
    }

    /// Sends `Hello` and waits up to `timeout` for the one reply, which must be the greeting.
    pub fn hello(&mut self, timeout: Duration) -> io::Result<Greeting> {
        self.stream.set_read_timeout(Some(timeout))?;
        self.send(&GreeterRequest::Hello)?;
        let reply = frame::read_msg(&mut self.stream).map_err(io_error)?;
        self.stream.set_read_timeout(None)?;
        match reply {
            GreeterReply::Greeting(g) => Ok(g),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the login manager answered hello with something else",
            )),
        }
    }

    pub fn send(&mut self, req: &GreeterRequest) -> io::Result<()> {
        frame::write_msg(&mut self.stream, req).map_err(io_error)
    }

    pub fn tx(&self) -> io::Result<GreeterLinkTx> {
        let stream = self.stream.try_clone()?;
        stream.set_write_timeout(Some(SEND_TIMEOUT))?;
        Ok(GreeterLinkTx(stream))
    }

    /// Puts the socket on the loop: each reply goes to `greeterview::reply`, in order, and the
    /// link's end, or a frame that breaks the protocol, to `greeterview::lost`.
    pub fn register(self, handle: &LoopHandle<'static, State>) -> Result<(), Box<dyn Error>> {
        let mut reader = FrameReader::new();
        handle.insert_source(
            Generic::new(self.stream, Interest::READ, Mode::Level),
            move |_, stream, state| {
                let mut buf = [0; 4096];
                let n = match (&**stream).read(&mut buf) {
                    Ok(0) => return Ok(gone(state, None)),
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {
                        return Ok(PostAction::Continue);
                    }
                    Err(e) => return Ok(gone(state, Some(e.into()))),
                };
                if let Err(e) = reader.push(&buf[..n]) {
                    return Ok(gone(state, Some(e)));
                }
                while let Some(body) = reader.next_frame() {
                    match frame::decode::<GreeterReply>(&body) {
                        Ok(reply) => crate::greeterview::reply(state, reply),
                        Err(e) => return Ok(gone(state, Some(e))),
                    }
                }
                Ok(PostAction::Continue)
            },
        )?;
        Ok(())
    }
}

/// Sends `req` to reprise-login on `State`'s write half. A send that fails or times out may have
/// left part of a frame, so the link is lost: it is shut down and nothing more goes out on it.
pub fn send(state: &mut State, req: &GreeterRequest) {
    let Some(tx) = state.greeter_link.as_mut() else {
        return;
    };
    if let Err(e) = tx.send(req) {
        log::warn!("greeter: sending to the login manager failed: {e}");
        let _ = tx.0.shutdown(Shutdown::Both);
        state.greeter_link = None;
        crate::greeterview::lost(state);
    }
}

impl GreeterLinkTx {
    fn send(&mut self, req: &GreeterRequest) -> io::Result<()> {
        frame::write_msg(&mut self.0, req).map_err(io_error)
    }
}

fn gone(state: &mut State, error: Option<FrameError>) -> PostAction {
    if state.greeter.lost {
        return PostAction::Remove;
    }
    if let Some(e) = error {
        log::warn!("greeter: link to the login manager broke: {e}");
    }
    crate::greeterview::lost(state);
    PostAction::Remove
}

fn io_error(e: FrameError) -> io::Error {
    match e {
        FrameError::Io(kind) => kind.into(),
        FrameError::Closed => io::ErrorKind::UnexpectedEof.into(),
        e => io::Error::new(io::ErrorKind::InvalidData, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::greeterview::Heard;
    use crate::harness::Harness;
    use repriselogin::Interface;
    use repriselogin::greeter::{AuthKind, Keyboard};
    use std::io::Write;

    fn greeting() -> Greeting {
        Greeting {
            interface: Interface::Blue,
            show_users: false,
            users: Vec::new(),
            last_user: Some("anna".into()),
            keyboard: Keyboard::default(),
            notice: None,
        }
    }

    fn prompt(text: &str) -> GreeterReply {
        GreeterReply::AuthMessage {
            kind: AuthKind::Secret,
            text: text.into(),
        }
    }

    #[test]
    fn hello_returns_the_greeting() {
        let (ours, mut daemon) = UnixStream::pair().unwrap();
        let fake = std::thread::spawn(move || {
            let req: GreeterRequest = frame::read_msg(&mut daemon).unwrap();
            frame::write_msg(&mut daemon, &GreeterReply::Greeting(greeting())).unwrap();
            req
        });
        let mut link = GreeterLink::from_stream(ours);
        assert_eq!(link.hello(Duration::from_secs(5)).unwrap(), greeting());
        assert_eq!(fake.join().unwrap(), GreeterRequest::Hello);
    }

    #[test]
    fn replies_arrive_in_order_through_the_loop() {
        let mut h = Harness::new();
        let (ours, mut daemon) = UnixStream::pair().unwrap();
        GreeterLink::from_stream(ours)
            .register(&h.state.handle)
            .unwrap();
        frame::write_msg(&mut daemon, &prompt("Password:")).unwrap();
        frame::write_msg(&mut daemon, &GreeterReply::Success).unwrap();
        h.run_for(50);
        assert_eq!(
            h.state.greeter.heard,
            vec![
                Heard::Reply(prompt("Password:")),
                Heard::Reply(GreeterReply::Success)
            ]
        );
        assert!(!h.state.greeter.lost);
    }

    #[test]
    fn a_closed_socket_reports_lost() {
        let mut h = Harness::new();
        let (ours, daemon) = UnixStream::pair().unwrap();
        GreeterLink::from_stream(ours)
            .register(&h.state.handle)
            .unwrap();
        drop(daemon);
        h.run_for(50);
        assert!(h.state.greeter.lost);
        assert!(h.state.greeter.heard.is_empty());
    }

    fn linked() -> (Harness, UnixStream) {
        let mut h = Harness::new();
        let (ours, daemon) = UnixStream::pair().unwrap();
        let link = GreeterLink::from_stream(ours);
        h.state.greeter_link = Some(link.tx().unwrap());
        link.register(&h.state.handle).unwrap();
        (h, daemon)
    }

    #[test]
    fn a_failed_send_loses_the_link() {
        let (mut h, daemon) = linked();
        drop(daemon);
        send(&mut h.state, &GreeterRequest::StartSession);
        assert!(h.state.greeter.lost);
        assert!(h.state.greeter_link.is_none(), "nothing more goes out");
    }

    #[test]
    fn a_send_that_times_out_loses_the_link() {
        let (mut h, mut daemon) = linked();
        let tx = &h.state.greeter_link.as_ref().unwrap().0;
        tx.set_nonblocking(true).unwrap();
        while (&*tx).write(&[0; 4096]).is_ok() {}
        tx.set_nonblocking(false).unwrap();
        send(&mut h.state, &GreeterRequest::StartSession);
        assert!(h.state.greeter.lost);
        assert!(
            h.state.greeter_link.is_none(),
            "no retry after a part frame"
        );
        daemon.set_nonblocking(true).unwrap();
        while daemon.read(&mut [0; 65536]).is_ok_and(|n| n > 0) {}
        send(&mut h.state, &GreeterRequest::CancelSession);
        assert!(
            !daemon.read(&mut [0; 16]).is_ok_and(|n| n > 0),
            "the lost link sends nothing"
        );
    }

    #[test]
    fn requests_go_out_on_the_write_half() {
        let (mut h, mut daemon) = linked();
        send(&mut h.state, &GreeterRequest::StartSession);
        let req: GreeterRequest = frame::read_msg(&mut daemon).unwrap();
        assert_eq!(req, GreeterRequest::StartSession);
        h.run_for(10);
        assert!(!h.state.greeter.lost, "the read half stays open");
    }
}
