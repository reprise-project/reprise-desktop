//! Process file descriptors: a socket peer's process, and the wait for a process to exit.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

/// `pidfd_open(pid)`.
pub fn open(pid: u32) -> io::Result<OwnedFd> {
    let pid =
        libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: pidfd_open takes a pid and flags, no pointers.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    owned(fd)
}

/// The pidfd of the process that connected `socket`'s peer end (`SO_PEERPIDFD`, Linux 6.5).
pub fn peer(socket: &impl AsRawFd) -> io::Result<OwnedFd> {
    let mut fd: libc::c_int = -1;
    let mut len = size_of::<libc::c_int>() as libc::socklen_t;
    // SAFETY: getsockopt writes at most `len` bytes into `fd`.
    let r = unsafe {
        libc::getsockopt(
            socket.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERPIDFD,
            (&raw mut fd).cast(),
            &mut len,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    if len as usize != size_of::<libc::c_int>() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    owned(fd.into())
}

fn owned(fd: libc::c_long) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = i32::try_from(fd).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    // SAFETY: `fd` is a descriptor the kernel just returned; nothing else owns it.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The pid `fd` refers to, from its fdinfo; `None` once the process is gone or when it lives
/// in another pid namespace.
pub fn pid_of(fd: &OwnedFd) -> io::Result<Option<u32>> {
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd()))?;
    let pid = (info.lines())
        .find_map(|l| l.strip_prefix("Pid:"))
        .ok_or(io::ErrorKind::InvalidData)?;
    let pid: i64 = pid.trim().parse().map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(u32::try_from(pid).ok().filter(|&p| p > 0))
}

/// Whether the process behind `fd` is still running.
pub fn alive(fd: &OwnedFd) -> bool {
    let mut p = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll reads and writes one pollfd and does not wait.
    unsafe { libc::poll(&mut p, 1, 0) == 0 }
}

/// Resolves once the process behind `fd` exits.
pub async fn exited(fd: OwnedFd) -> io::Result<()> {
    // SAFETY: the `AsyncFd` owns `fd`, which stays open and the same until it is dropped.
    let fd = unsafe { AsyncFd::register_with_interest(fd, Interest::READABLE) }
        .map_err(io::Error::from)?;
    let _ready = fd.readable().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_pidfd_turns_readable_when_its_process_exits() {
        let mut child = Command::new("sleep").arg("0.2").spawn().unwrap();
        let fd = open(child.id()).unwrap();
        assert!(alive(&fd));
        let fired = tokio::time::timeout(Duration::from_secs(5), exited(fd.try_clone().unwrap()));
        assert!(matches!(fired.await, Ok(Ok(()))));
        assert!(!alive(&fd));
        child.wait().unwrap();
    }

    #[test]
    fn a_socket_peers_pidfd_names_its_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = peer(&a).unwrap();
        assert_eq!(pid_of(&fd).unwrap(), Some(std::process::id()));
        assert!(alive(&fd));
    }
}
