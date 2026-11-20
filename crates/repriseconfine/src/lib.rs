//! Landlock rulesets and seccomp filters for the programs Reprise confines, the steps a helper
//! takes to confine itself ([`child`]), and the parent's side of running one ([`helper`]). Each layer binds the calling thread and the threads it starts
//! later, so a program confines itself before it starts any.

use std::io;

pub mod child;
pub mod helper;
pub mod landlock;
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
pub mod seccomp;

/// Sets `PR_SET_NO_NEW_PRIVS`, which a seccomp filter or Landlock ruleset of an unprivileged
/// process needs.
pub fn no_new_privs() -> io::Result<()> {
    // SAFETY: PR_SET_NO_NEW_PRIVS takes no pointers; the unused arguments are zero.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
