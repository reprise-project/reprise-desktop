//! Keeps a compromised front from opening network connections of its own: it serves the socket
//! systemd passed and the connections accepted on it, and reaches reprise-login over AF_UNIX.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const LANDLOCK_CREATE_RULESET_VERSION: libc::c_uint = 1;
/// The first Landlock ABI with TCP rules (Linux 6.7).
const LANDLOCK_NET_ABI: libc::c_long = 4;
const LANDLOCK_ACCESS_NET_BIND_TCP: u64 = 1;
const LANDLOCK_ACCESS_NET_CONNECT_TCP: u64 = 2;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
}

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
    // SAFETY: PR_SET_NO_NEW_PRIVS takes no pointers; the unused arguments are zero.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let seccomp = seccomp::install()?;
    let landlock = refuse_tcp_connects()?;
    Ok(Confined { seccomp, landlock })
}

/// Another arch's filter would need its own `AUDIT_ARCH` and, on i686, `socketcall(2)`.
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
mod seccomp {
    pub fn install() -> std::io::Result<bool> {
        Ok(false)
    }
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
mod seccomp {
    use std::io;

    /// `AUDIT_ARCH_*` of the native system calls; any other arch is refused.
    #[cfg(target_arch = "x86_64")]
    const ARCH: u32 = 0xc000_003e;
    #[cfg(target_arch = "aarch64")]
    const ARCH: u32 = 0xc000_00b7;
    /// x32 system calls carry x86_64's arch and set this bit in their number.
    const X32_BIT: u32 = 0x4000_0000;

    /// Offsets into `seccomp_data`; an argument's low 32 bits come first on little-endian.
    const NR: u32 = 0;
    const ARCH_AT: u32 = 4;
    const fn arg(n: u32) -> u32 {
        16 + 8 * n
    }

    /// Installs [`filter`]; true once it is in.
    pub fn install() -> io::Result<bool> {
        let filter = filter();
        let program = libc::sock_fprog {
            len: filter.len() as libc::c_ushort,
            filter: filter.as_ptr().cast_mut(),
        };
        // SAFETY: program points at `filter`, which outlives the call; the kernel copies it.
        let set = unsafe {
            libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &raw const program,
                0,
                0,
            )
        };
        if set != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(true)
    }

    /// New IPv4 and IPv6 sockets fail with EAFNOSUPPORT, as under `RestrictAddressFamilies`.
    /// io_uring fails with ENOSYS: its requests make sockets this filter never sees. A send
    /// with MSG_FASTOPEN fails with EPERM: it connects without `connect(2)`, and a Landlock
    /// without the 7.2 Fast Open fix lets it through.
    fn filter() -> Vec<libc::sock_filter> {
        let mut f = vec![
            load(ARCH_AT),
            jump(libc::BPF_JEQ, ARCH, 1, 0),
            refuse(libc::ENOSYS),
        ];
        f.push(load(NR));
        if cfg!(target_arch = "x86_64") {
            f.extend([jump(libc::BPF_JGE, X32_BIT, 0, 1), refuse(libc::ENOSYS)]);
        }
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
        f.push(statement(
            libc::BPF_RET | libc::BPF_K,
            libc::SECCOMP_RET_ALLOW,
        ));
        f
    }

    fn nr(call: libc::c_long) -> u32 {
        call as u32
    }

    fn statement(code: u32, k: u32) -> libc::sock_filter {
        libc::sock_filter {
            code: code as u16,
            jt: 0,
            jf: 0,
            k,
        }
    }

    fn load(offset: u32) -> libc::sock_filter {
        statement(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, offset)
    }

    /// Skips `jt` instructions when `op` holds against `k`, else `jf`.
    fn jump(op: u32, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
        libc::sock_filter {
            code: (libc::BPF_JMP | op | libc::BPF_K) as u16,
            jt,
            jf,
            k,
        }
    }

    fn refuse(errno: i32) -> libc::sock_filter {
        statement(
            libc::BPF_RET | libc::BPF_K,
            libc::SECCOMP_RET_ERRNO | errno as u32,
        )
    }
}

/// Refuses every TCP connect and bind; false when the kernel has no Landlock TCP rules.
fn refuse_tcp_connects() -> io::Result<bool> {
    // SAFETY: the version query takes no attribute.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            LANDLOCK_CREATE_RULESET_VERSION,
        )
    };
    if abi < LANDLOCK_NET_ABI {
        return Ok(false);
    }
    let attr = RulesetAttr {
        handled_access_fs: 0,
        handled_access_net: LANDLOCK_ACCESS_NET_BIND_TCP | LANDLOCK_ACCESS_NET_CONNECT_TCP,
    };
    // SAFETY: attr is live and its size is passed.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &raw const attr,
            std::mem::size_of::<RulesetAttr>(),
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: landlock_create_ruleset returned a new fd nothing else owns.
    let ruleset = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    // SAFETY: ruleset is an open Landlock ruleset fd; no flags.
    if unsafe { libc::syscall(libc::SYS_landlock_restrict_self, ruleset.as_raw_fd(), 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
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
