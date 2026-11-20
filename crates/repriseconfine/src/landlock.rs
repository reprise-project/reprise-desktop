//! Landlock rulesets that handle rights and grant none of them.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

const CREATE_RULESET_VERSION: libc::c_uint = 1;

pub const ACCESS_NET_BIND_TCP: u64 = 1;
pub const ACCESS_NET_CONNECT_TCP: u64 = 2;

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

/// What a ruleset refuses: each handled access right, and each scope.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Handled {
    pub fs: u64,
    pub net: u64,
    pub scoped: u64,
}

impl Handled {
    /// Every access right and scope Landlock ABI `abi` knows, up to ABI 9's `RESOLVE_UNIX`.
    pub fn all(abi: i64) -> Handled {
        let mut fs = 0;
        // EXECUTE through MAKE_SYM, then REFER, TRUNCATE, IOCTL_DEV and RESOLVE_UNIX.
        for (since, bits) in [
            (1, (1 << 13) - 1),
            (2, 1 << 13),
            (3, 1 << 14),
            (5, 1 << 15),
            (9, 1 << 16),
        ] {
            if abi >= since {
                fs |= bits;
            }
        }
        Handled {
            fs,
            net: if abi >= 4 {
                ACCESS_NET_BIND_TCP | ACCESS_NET_CONNECT_TCP
            } else {
                0
            },
            // ABSTRACT_UNIX_SOCKET and SIGNAL.
            scoped: if abi >= 6 { 0b11 } else { 0 },
        }
    }
}

/// The kernel's Landlock ABI version, 0 when Landlock is off or missing.
pub fn abi() -> i64 {
    // SAFETY: the version query takes no attribute.
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<RulesetAttr>(),
            0usize,
            CREATE_RULESET_VERSION,
        )
    };
    abi.max(0)
}

/// Restricts the calling thread to `handled` with no rule granting any of it back.
pub fn restrict_self(handled: Handled) -> io::Result<()> {
    let attr = RulesetAttr {
        handled_access_fs: handled.fs,
        handled_access_net: handled.net,
        scoped: handled.scoped,
    };
    // SAFETY: attr is live and its size is passed; a kernel older than `scoped` takes the zero
    // tail of a larger struct.
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_abi_handles_the_rights_it_added() {
        assert_eq!(Handled::all(0), Handled::default());
        assert_eq!(Handled::all(1).fs, 0x1fff);
        assert_eq!(Handled::all(3).fs, 0x7fff);
        assert_eq!(
            Handled::all(4),
            Handled {
                fs: 0x7fff,
                net: 3,
                scoped: 0
            }
        );
        assert_eq!(
            Handled::all(6),
            Handled {
                fs: 0xffff,
                net: 3,
                scoped: 3
            }
        );
        assert_eq!(Handled::all(9).fs, 0x1_ffff);
        assert_eq!(Handled::all(12), Handled::all(9));
    }
}
