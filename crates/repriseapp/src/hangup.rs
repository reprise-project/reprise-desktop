//! Hangs up the Wayland connection before the process ends: a thread blocked on a dead mount keeps
//! the fd table past exit, and with it the socket and the program's windows.

use std::os::fd::{AsRawFd, BorrowedFd};
use std::sync::atomic::{AtomicI32, Ordering};

static SOCKET: AtomicI32 = AtomicI32::new(-1);

/// The signals another process ends a program with; their handler hangs up, then ends it as the
/// default action would. SIGKILL can't be caught: End Program drops the connection itself.
const ENDING: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// Hangs up `socket` if the program is ended by a signal.
pub(crate) fn on_signals(socket: BorrowedFd) {
    SOCKET.store(socket.as_raw_fd(), Ordering::Release);
    for sig in ENDING {
        // SAFETY: the handler calls only async-signal-safe functions; the struct is zeroed and
        // filled before the call reads it.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_ending as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_RESETHAND;
            libc::sigemptyset(&mut sa.sa_mask);
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                log::warn!(
                    "no handler for signal {sig}: {}",
                    std::io::Error::last_os_error()
                );
            }
        }
    }
}

/// Hangs up `socket` whoever else holds it: the compositor reads the end of the connection.
pub(crate) fn hang_up(socket: BorrowedFd) {
    SOCKET.store(-1, Ordering::Release);
    // SAFETY: plain syscall on an open fd.
    unsafe { libc::shutdown(socket.as_raw_fd(), libc::SHUT_RDWR) };
}

extern "C" fn on_ending(sig: libc::c_int) {
    let fd = SOCKET.load(Ordering::Acquire);
    // SAFETY: shutdown and raise are async-signal-safe. SA_RESETHAND restored the default action,
    // which the raise takes once this handler returns.
    unsafe {
        if fd >= 0 {
            libc::shutdown(fd, libc::SHUT_RDWR);
        }
        libc::raise(sig);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn ends(mut peer: UnixStream) -> bool {
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        matches!(peer.read(&mut [0; 8]), Ok(0))
    }

    #[test]
    fn a_hang_up_ends_the_connection_while_another_fd_still_holds_it() {
        let (ours, peer) = UnixStream::pair().unwrap();
        let held = ours.try_clone().unwrap();
        hang_up(ours.as_fd());
        assert!(ends(peer), "the peer reads the end of the connection");
        drop(held);
    }

    #[test]
    fn a_program_ended_by_sigterm_hangs_up_though_its_socket_outlives_it() {
        let (ours, peer) = UnixStream::pair().unwrap();
        // SAFETY: the child calls only sigaction, raise and _exit before it ends.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork");
        if pid == 0 {
            on_signals(ours.as_fd());
            // SAFETY: raise and _exit in the forked child.
            unsafe {
                libc::raise(libc::SIGTERM);
                libc::_exit(0);
            }
        }
        let mut status = 0;
        // SAFETY: waits on the child forked above.
        unsafe { libc::waitpid(pid, &mut status, 0) };
        assert!(
            libc::WIFSIGNALED(status) && libc::WTERMSIG(status) == libc::SIGTERM,
            "the child ends by SIGTERM as before: status {status:#x}"
        );
        assert!(
            ends(peer),
            "the peer reads the end while this process still holds the child's socket"
        );
        drop(ours);
    }
}
