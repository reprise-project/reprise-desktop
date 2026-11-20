//! Keeps a compromised front from opening network connections of its own: it serves the socket
//! systemd passed and the connections accepted on it, and reaches reprise-login over AF_UNIX.

use std::io;

use repriseconfine::landlock::{self, ACCESS_NET_BIND_TCP, ACCESS_NET_CONNECT_TCP, Handled};

/// The first Landlock ABI with TCP rules (Linux 6.7).
const LANDLOCK_NET_ABI: i64 = 4;

/// The layers [`confine`] applied; each is missing on some kernels or arches.
#[derive(Clone, Copy, Debug)]
pub struct Confined {
    /// The seccomp filter against new IPv4 and IPv6 sockets, io_uring and Fast Open sends.
    pub seccomp: bool,
    /// The Landlock rules against TCP connects and binds.
    pub landlock: bool,
}

/// Confines the calling thread and every thread it starts later, so it runs before the runtime
/// starts any.
pub fn confine() -> io::Result<Confined> {
    repriseconfine::no_new_privs()?;
    let seccomp = seccomp()?;
    let landlock = refuse_tcp_connects()?;
    Ok(Confined { seccomp, landlock })
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn seccomp() -> io::Result<bool> {
    Ok(false)
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn seccomp() -> io::Result<bool> {
    repriseconfine::seccomp::install(&filter())?;
    Ok(true)
}

/// New IPv4 and IPv6 sockets fail with EAFNOSUPPORT, as under `RestrictAddressFamilies`.
/// io_uring fails with ENOSYS: its requests make sockets this filter never sees. A send
/// with MSG_FASTOPEN fails with EPERM: it connects without `connect(2)`, and a Landlock
/// without the 7.2 Fast Open fix lets it through.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter() -> Vec<libc::sock_filter> {
    use repriseconfine::seccomp::{NR, arg, jump, load, native, nr, refuse, ret_allow};
    let mut f = native(refuse(libc::ENOSYS));
    f.extend([
        jump(libc::BPF_JEQ, nr(libc::SYS_socket), 0, 4),
        load(arg(0)),
        jump(libc::BPF_JEQ, libc::AF_INET as u32, 1, 0),
        jump(libc::BPF_JEQ, libc::AF_INET6 as u32, 0, 1),
        refuse(libc::EAFNOSUPPORT),
        load(NR),
        jump(libc::BPF_JEQ, nr(libc::SYS_io_uring_setup), 0, 1),
        refuse(libc::ENOSYS),
    ]);
    // The flags argument of each send call; sendmmsg ignores each message's msg_flags.
    for (call, flags) in [
        (libc::SYS_sendto, 3),
        (libc::SYS_sendmsg, 2),
        (libc::SYS_sendmmsg, 3),
    ] {
        f.extend([
            load(NR),
            jump(libc::BPF_JEQ, nr(call), 0, 3),
            load(arg(flags)),
            jump(libc::BPF_JSET, libc::MSG_FASTOPEN as u32, 0, 1),
            refuse(libc::EPERM),
        ]);
    }
    f.push(ret_allow());
    f
}

/// Refuses every TCP connect and bind; false when the kernel has no Landlock TCP rules.
fn refuse_tcp_connects() -> io::Result<bool> {
    if landlock::abi() < LANDLOCK_NET_ABI {
        return Ok(false);
    }
    landlock::restrict_self(Handled {
        net: ACCESS_NET_BIND_TCP | ACCESS_NET_CONNECT_TCP,
        ..Handled::default()
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};

    fn sockaddr(addr: SocketAddr) -> libc::sockaddr_in {
        let SocketAddr::V4(v4) = addr else {
            panic!("{addr} is not IPv4")
        };
        libc::sockaddr_in {
            sin_family: libc::AF_INET as libc::sa_family_t,
            sin_port: v4.port().to_be(),
            sin_addr: libc::in_addr {
                s_addr: u32::from(*v4.ip()).to_be(),
            },
            sin_zero: [0; 8],
        }
    }

    fn errno(result: libc::c_long) -> Option<i32> {
        (result < 0).then(|| io::Error::last_os_error().raw_os_error().unwrap())
    }

    fn connect(fd: i32, addr: &libc::sockaddr_in) -> Option<i32> {
        let len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        // SAFETY: addr is a live sockaddr_in of len bytes.
        errno(unsafe { libc::connect(fd, (addr as *const libc::sockaddr_in).cast(), len) }.into())
    }

    /// Sends one byte to `to` with MSG_FASTOPEN through sendto, sendmsg and sendmmsg.
    fn fast_open_sends(fd: i32, to: &libc::sockaddr_in) -> [Option<i32>; 3] {
        let to_len = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
        let flags = libc::MSG_FASTOPEN | libc::MSG_NOSIGNAL;
        let mut byte = *b"x";
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        // SAFETY: an all-zero msghdr is a valid empty one.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_name = (to as *const libc::sockaddr_in).cast_mut().cast();
        msg.msg_namelen = to_len;
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        let mut mmsg = libc::mmsghdr {
            msg_hdr: msg,
            msg_len: 0,
        };
        // SAFETY: byte, to, iov, msg and mmsg are live for each call.
        unsafe {
            [
                errno(libc::sendto(
                    fd,
                    byte.as_ptr().cast(),
                    1,
                    flags,
                    (to as *const libc::sockaddr_in).cast(),
                    to_len,
                ) as libc::c_long),
                errno(libc::sendmsg(fd, &raw const msg, flags) as libc::c_long),
                errno(libc::sendmmsg(fd, &raw mut mmsg, 1, flags as _).into()),
            ]
        }
    }

    #[test]
    fn a_confined_front_serves_its_socket_and_opens_no_connection_of_its_own() {
        let served = TcpListener::bind("127.0.0.1:0").unwrap();
        let served_addr = served.local_addr().unwrap();
        let elsewhere = TcpListener::bind("127.0.0.1:0").unwrap();
        let elsewhere_addr = elsewhere.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("front.sock");
        let daemon = UnixListener::bind(&link).unwrap();

        let front = std::thread::spawn(move || {
            let Confined { seccomp, landlock } = confine().unwrap();
            assert_eq!(
                seccomp,
                cfg!(any(target_arch = "x86_64", target_arch = "aarch64")),
                "the filter is in on the arches it is written for"
            );
            if seccomp {
                let refused = |e: io::Error| e.raw_os_error();
                assert_eq!(
                    TcpStream::connect(elsewhere_addr).map_err(refused).err(),
                    Some(Some(libc::EAFNOSUPPORT)),
                    "no new IPv4 socket"
                );
                assert_eq!(
                    UdpSocket::bind("[::1]:0").map_err(refused).err(),
                    Some(Some(libc::EAFNOSUPPORT)),
                    "no new IPv6 socket"
                );
                let params = [0u8; 120];
                // SAFETY: params is a live, zeroed io_uring_params.
                let uring = unsafe { libc::syscall(libc::SYS_io_uring_setup, 1, params.as_ptr()) };
                assert_eq!(errno(uring), Some(libc::ENOSYS), "no io_uring");
            }
            UnixStream::connect(&link).expect("the link to reprise-login connects");
            let (conn, _) = served
                .accept()
                .expect("the passed socket takes connections");

            let fd = conn.as_raw_fd();
            // SAFETY: the buffer is live for the call.
            let sent = unsafe { libc::send(fd, b"x".as_ptr().cast(), 1, libc::MSG_NOSIGNAL) };
            assert_eq!(
                sent,
                1,
                "the accepted connection sends: {}",
                io::Error::last_os_error()
            );
            let unspec = libc::sockaddr {
                sa_family: libc::AF_UNSPEC as libc::sa_family_t,
                sa_data: [0; 14],
            };
            let len = std::mem::size_of::<libc::sockaddr>() as libc::socklen_t;
            // SAFETY: unspec is a live sockaddr of len bytes.
            let dropped = unsafe { libc::connect(fd, &unspec, len) };
            assert_eq!(dropped, 0, "{}", io::Error::last_os_error());
            let to = sockaddr(elsewhere_addr);
            if seccomp {
                assert_eq!(
                    fast_open_sends(fd, &to),
                    [Some(libc::EPERM); 3],
                    "an accepted socket sends through Fast Open nowhere"
                );
            }
            if landlock {
                assert_eq!(
                    connect(fd, &to),
                    Some(libc::EACCES),
                    "an accepted socket connects nowhere else"
                );
            }
            (seccomp, landlock)
        });
        let _client = TcpStream::connect(served_addr).unwrap();
        let (seccomp, landlock) = front.join().unwrap();
        daemon.set_nonblocking(true).unwrap();
        daemon.accept().expect("the link reached reprise-login");
        elsewhere.set_nonblocking(true).unwrap();
        assert_eq!(
            elsewhere.accept().map_err(|e| e.kind()).err(),
            Some(io::ErrorKind::WouldBlock),
            "nothing reached the other service"
        );
        if !landlock {
            eprintln!("this kernel has no Landlock network rules; connects were not checked");
        }
        if !seccomp {
            eprintln!("no seccomp filter on this arch; Fast Open sends were not checked");
        }
    }
}
