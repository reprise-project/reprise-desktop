//! The journal line for a Wayland client whose connection ended.

use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;

use smithay::reexports::rustix::event::{PollFd, PollFlags, Timespec, poll};
use smithay::reexports::rustix::net::sockopt;
use smithay::reexports::wayland_server::backend::DisconnectReason;

use crate::sandbox::ClientSandbox;

/// A second handle on a client's socket, kept until the client disconnects
/// so the line can say which end closed it.
#[derive(Debug, Default)]
pub struct ClientPeer(Mutex<Option<OwnedFd>>);

impl ClientPeer {
    pub fn of_stream(stream: &UnixStream) -> ClientPeer {
        ClientPeer(Mutex::new(stream.as_fd().try_clone_to_owned().ok()))
    }

    pub fn take(&self) -> Option<OwnedFd> {
        self.0.lock().ok().and_then(|mut fd| fd.take())
    }
}

/// The socket as the client left it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketEnd {
    /// The client closed its end.
    pub hung_up: bool,
    /// Bytes written to the client that it has not read, in kernel
    /// accounting (`SIOCOUTQ`), against the socket's send buffer.
    pub queued: usize,
    pub send_buffer: usize,
}

pub fn socket_end(fd: BorrowedFd) -> SocketEnd {
    let mut fds = [PollFd::from_borrowed_fd(fd, PollFlags::RDHUP)];
    let hung_up = poll(&mut fds, Some(&Timespec::default())).is_ok()
        && fds[0]
            .revents()
            .intersects(PollFlags::RDHUP | PollFlags::HUP | PollFlags::ERR);
    let mut queued: libc::c_int = 0;
    // SAFETY: SIOCOUTQ (TIOCOUTQ's number) writes one c_int through the
    // pointer, which outlives the call.
    let queued = match unsafe { libc::ioctl(fd.as_raw_fd(), libc::TIOCOUTQ, &mut queued) } {
        0 => queued.max(0) as usize,
        _ => 0,
    };
    SocketEnd {
        hung_up,
        queued,
        send_buffer: sockopt::socket_send_buffer_size(fd).unwrap_or(0),
    }
}

/// The level and text of the line for a client that `reason` ended. A close
/// while the client still held its end (a full send queue) is a warning.
pub fn line(
    sandbox: &ClientSandbox,
    comm: Option<&str>,
    reason: &DisconnectReason,
    end: Option<SocketEnd>,
) -> (log::Level, String) {
    let mut who = match sandbox.pid {
        Some(pid) => format!("pid {pid}"),
        None => "pid ?".to_string(),
    };
    if let Some(comm) = comm {
        who.push_str(&format!(" ({comm})"));
    }
    if let Some(c) = &sandbox.context {
        who.push_str(&format!(
            ", sandbox {} {}",
            c.engine.as_deref().unwrap_or("(unnamed)"),
            c.app_id.as_deref().unwrap_or("(no app id)")
        ));
    } else if sandbox.flatpak {
        who.push_str(", flatpak");
    }
    match reason {
        DisconnectReason::ProtocolError(e) => (
            log::Level::Warn,
            format!("client {who} dropped: {e} (code {})", e.code),
        ),
        DisconnectReason::ConnectionClosed => match end {
            Some(end) if end.hung_up => (log::Level::Info, format!("client {who} disconnected")),
            Some(end) => (
                log::Level::Warn,
                format!(
                    "client {who} dropped while still connected: {} of {} bytes unread in its socket",
                    end.queued, end.send_buffer
                ),
            ),
            None => (log::Level::Warn, format!("client {who} connection closed")),
        },
    }
}

/// Logs the end of a client's connection; `peer` is its [`ClientPeer`].
pub fn disconnected(sandbox: &ClientSandbox, peer: &ClientPeer, reason: &DisconnectReason) {
    let peer = peer.take();
    let end = peer.as_ref().map(|fd| socket_end(fd.as_fd()));
    let comm = sandbox
        .pid
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok());
    let (level, text) = line(sandbox, comm.as_deref().map(str::trim), reason, end);
    log::log!(level, "{text}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::Sandbox;
    use smithay::reexports::wayland_server::backend::protocol::ProtocolError;
    use std::io::Write;

    #[test]
    fn a_closed_client_end_reads_as_hung_up() {
        let (server, client) = UnixStream::pair().unwrap();
        assert!(!socket_end(server.as_fd()).hung_up);
        drop(client);
        assert!(socket_end(server.as_fd()).hung_up);
    }

    #[test]
    fn bytes_the_client_has_not_read_count_as_queued() {
        let (mut server, _client) = UnixStream::pair().unwrap();
        assert_eq!(socket_end(server.as_fd()).queued, 0);
        server.write_all(&[0; 512]).unwrap();
        let end = socket_end(server.as_fd());
        assert!(end.queued >= 512, "queued {}", end.queued);
        assert!(end.send_buffer > 0);
    }

    fn signal() -> ClientSandbox {
        ClientSandbox {
            context: Some(Sandbox {
                engine: Some("org.flatpak".into()),
                app_id: Some("org.signal.Signal".into()),
            }),
            flatpak: true,
            pid: Some(42),
        }
    }

    #[test]
    fn a_protocol_error_is_a_warning_naming_the_object_and_the_sandbox() {
        let reason = DisconnectReason::ProtocolError(ProtocolError {
            code: 2,
            object_id: 30,
            object_interface: "xdg_toplevel_icon_v1".into(),
            message: "no buffer".into(),
        });
        let (level, text) = line(&signal(), Some("signal-desktop"), &reason, None);
        assert_eq!(level, log::Level::Warn);
        assert!(text.contains("pid 42 (signal-desktop)"), "{text}");
        assert!(
            text.contains("sandbox org.flatpak org.signal.Signal"),
            "{text}"
        );
        assert!(text.contains("xdg_toplevel_icon_v1@30"), "{text}");
        assert!(text.contains("no buffer"), "{text}");
    }

    #[test]
    fn a_close_while_the_client_held_its_end_is_a_warning_with_the_queue() {
        let end = SocketEnd {
            hung_up: false,
            queued: 212_000,
            send_buffer: 212_992,
        };
        let (level, text) = line(
            &signal(),
            None,
            &DisconnectReason::ConnectionClosed,
            Some(end),
        );
        assert_eq!(level, log::Level::Warn);
        assert!(text.contains("212000 of 212992 bytes unread"), "{text}");
    }

    #[test]
    fn a_client_that_hung_up_is_an_info_line() {
        let end = SocketEnd {
            hung_up: true,
            queued: 0,
            send_buffer: 212_992,
        };
        let (level, _) = line(
            &ClientSandbox::default(),
            None,
            &DisconnectReason::ConnectionClosed,
            Some(end),
        );
        assert_eq!(level, log::Level::Info);
    }
}
