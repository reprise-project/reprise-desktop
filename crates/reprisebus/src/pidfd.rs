//! Process file descriptors: by pid, a socket peer's, and the pid one names.

use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};

/// `pidfd_open(pid)`.
pub fn open(pid: u32) -> io::Result<OwnedFd> {
    let pid =
        libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: pidfd_open takes a pid and flags, no pointers.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    owned(fd)
}

/// The pidfd of the process that connected `socket`'s peer end (`SO_PEERPIDFD`, Linux 6.5);
/// `ENOPROTOOPT` on an older kernel.
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
pub fn pid_of(fd: &impl AsFd) -> io::Result<Option<u32>> {
    let fd = fd.as_fd().as_raw_fd();
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{fd}"))?;
    let pid = (info.lines())
        .find_map(|l| l.strip_prefix("Pid:"))
        .ok_or(io::ErrorKind::InvalidData)?;
    let pid: i64 = pid.trim().parse().map_err(|_| io::ErrorKind::InvalidData)?;
    Ok(u32::try_from(pid).ok().filter(|&p| p > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_peers_pidfd_names_its_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let fd = peer(&a).unwrap();
        assert_eq!(pid_of(&fd).unwrap(), Some(std::process::id()));
    }

    #[test]
    fn a_reaped_process_has_no_pid() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let fd = open(child.id()).unwrap();
        assert_eq!(pid_of(&fd).unwrap(), Some(child.id()));
        child.wait().unwrap();
        assert_eq!(pid_of(&fd).unwrap(), None);
    }
}
