//! Waiting on a process file descriptor for its process to exit.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};

use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

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
        let fd = reprisebus::pidfd::open(child.id()).unwrap();
        assert!(alive(&fd));
        let fired = tokio::time::timeout(Duration::from_secs(5), exited(fd.try_clone().unwrap()));
        assert!(matches!(fired.await, Ok(Ok(()))));
        assert!(!alive(&fd));
        child.wait().unwrap();
    }
}
