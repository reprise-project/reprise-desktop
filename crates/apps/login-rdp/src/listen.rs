//! The port's listening socket, which `reprise-login-rdp.socket` passes the front.

use std::io;
use std::net::TcpListener;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};

/// `SD_LISTEN_FDS_START`: systemd passes its sockets from fd 3 on.
const FIRST_FD: RawFd = 3;

/// The listening socket systemd passed this process, as `sd_listen_fds` finds it.
pub fn passed() -> io::Result<TcpListener> {
    let fd = passed_fd(
        std::env::var("LISTEN_PID").ok().as_deref(),
        std::env::var("LISTEN_FDS").ok().as_deref(),
        std::process::id(),
    )?;
    // SAFETY: systemd passes fd 3 to this process alone, and nothing else in the front takes it.
    listener(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The passed fd, given `LISTEN_PID`, `LISTEN_FDS` and this process's pid.
fn passed_fd(pid: Option<&str>, fds: Option<&str>, me: u32) -> io::Result<RawFd> {
    if pid.and_then(|p| p.parse::<u32>().ok()) != Some(me) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no socket was passed: the front starts through reprise-login-rdp.socket",
        ));
    }
    match fds.and_then(|n| n.parse::<u32>().ok()) {
        Some(1) => Ok(FIRST_FD),
        n => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("LISTEN_FDS is {n:?}; reprise-login-rdp.socket passes one socket"),
        )),
    }
}

/// `fd` as a listener, once it is a listening TCP socket; it no longer passes to children.
fn listener(fd: OwnedFd) -> io::Result<TcpListener> {
    let raw = fd.as_raw_fd();
    let domain = int_option(raw, libc::SO_DOMAIN)?;
    let tcp = matches!(domain, libc::AF_INET | libc::AF_INET6)
        && int_option(raw, libc::SO_TYPE)? == libc::SOCK_STREAM;
    if !tcp || int_option(raw, libc::SO_ACCEPTCONN)? != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the passed socket is not a listening TCP socket",
        ));
    }
    // SAFETY: F_SETFD sets the descriptor flags of an open fd.
    if unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let listener = TcpListener::from(fd);
    listener.set_nonblocking(true)?;
    Ok(listener)
}

fn int_option(fd: RawFd, name: libc::c_int) -> io::Result<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: value and len are live and sized for an int option.
    let got = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            name,
            (&raw mut value).cast(),
            &raw mut len,
        )
    };
    if got != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn the_socket_is_found_only_when_systemd_passed_one_to_this_process() {
        assert_eq!(passed_fd(Some("7"), Some("1"), 7).unwrap(), FIRST_FD);
        for (pid, fds) in [
            (None, None),
            (Some("7"), None),
            (None, Some("1")),
            (Some("8"), Some("1")),
            (Some("7"), Some("0")),
            (Some("7"), Some("2")),
            (Some("x"), Some("1")),
        ] {
            assert!(passed_fd(pid, fds, 7).is_err(), "{pid:?} {fds:?}");
        }
    }

    #[tokio::test]
    async fn a_passed_listening_socket_takes_connections() {
        let bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = bound.local_addr().unwrap();
        let std_listener = listener(OwnedFd::from(bound)).unwrap();
        // SAFETY: F_GETFD reads the descriptor flags of an open fd.
        let flags = unsafe { libc::fcntl(std_listener.as_raw_fd(), libc::F_GETFD) };
        assert_eq!(flags & libc::FD_CLOEXEC, libc::FD_CLOEXEC);
        let listener = tokio::net::TcpListener::from_std(std_listener).unwrap();
        let (client, accepted) =
            tokio::join!(tokio::net::TcpStream::connect(addr), listener.accept());
        client.unwrap();
        accepted.unwrap();
    }

    #[test]
    fn a_passed_socket_that_is_no_tcp_listener_is_refused() {
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        assert!(listener(OwnedFd::from(udp)).is_err(), "a datagram socket");
        let dir = tempfile::tempdir().unwrap();
        let unix = UnixListener::bind(dir.path().join("s")).unwrap();
        assert!(listener(OwnedFd::from(unix)).is_err(), "a unix socket");
        // SAFETY: socket takes no pointers; the fd is checked before it is owned.
        let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        assert!(fd >= 0);
        // SAFETY: fd is a fresh socket nothing else owns.
        let unbound = unsafe { OwnedFd::from_raw_fd(fd) };
        assert!(
            listener(unbound).is_err(),
            "a TCP socket that does not listen"
        );
    }
}
