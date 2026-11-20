//! What a helper does to itself before it reads its request: no new privileges, no descriptor but
//! stdin, stdout and stderr, not dumpable, address space and CPU time capped, every Landlock right
//! handled and none granted, and its seccomp filter on every thread.

use std::io;

use crate::landlock::{self, Handled};

/// What the soft CPU limit's SIGXCPU leaves before the hard limit's SIGKILL.
const CPU_GRACE: u64 = 5;

/// The limits [`confine`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub address_space: u64,
    /// The soft RLIMIT_CPU; the hard one is [`CPU_GRACE`] later.
    pub cpu_seconds: u64,
    /// glibc malloc arenas.
    pub arenas: usize,
}

/// The layers [`confine`] applied beyond the ones every Linux has.
#[derive(Clone, Copy, Debug)]
pub struct Confined {
    /// False without a filter, on an arch none is written for.
    pub seccomp: bool,
    /// The kernel's Landlock ABI, 0 without Landlock.
    pub landlock: i64,
}

/// Confines the calling process under `limits` and `filter`, refused once it has started a
/// thread: Landlock binds only the calling thread.
pub fn confine(limits: &Limits, filter: Option<&[libc::sock_filter]>) -> io::Result<Confined> {
    let threads = threads()?;
    if threads != 1 {
        return Err(io::Error::other(format!(
            "{threads} threads are running, and only the calling one would be confined"
        )));
    }
    // A glibc that picks its own arena count reads the CPU affinity, a system call a filter may
    // refuse.
    #[cfg(target_env = "gnu")]
    // SAFETY: plain mallopt, with no other thread to allocate meanwhile.
    unsafe {
        libc::mallopt(libc::M_ARENA_MAX, limits.arenas as libc::c_int);
    }
    crate::no_new_privs()?;
    // SAFETY: closes descriptors this process owns; nothing holds one above 2 past this point.
    if unsafe { libc::close_range(3, libc::c_uint::MAX, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: PR_SET_DUMPABLE takes no pointers.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    limit(
        "RLIMIT_AS",
        libc::RLIMIT_AS,
        limits.address_space,
        limits.address_space,
    )?;
    limit(
        "RLIMIT_CPU",
        libc::RLIMIT_CPU,
        limits.cpu_seconds,
        limits.cpu_seconds.saturating_add(CPU_GRACE),
    )?;
    let abi = landlock::abi();
    if abi > 0 {
        landlock::restrict_self(Handled::all(abi))?;
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    if let Some(f) = filter {
        crate::seccomp::install_on_every_thread(f)?;
    }
    Ok(Confined {
        seccomp: filter.is_some() && cfg!(any(target_arch = "x86_64", target_arch = "aarch64")),
        landlock: abi,
    })
}

fn threads() -> io::Result<usize> {
    Ok(std::fs::read_dir("/proc/self/task")?.count())
}

/// Sets `resource` (named `name`) to `soft` and `hard`, each held at the current hard limit; a
/// held limit is said on stderr, so a failure past it is not read as the helper's own budget.
fn limit(name: &str, resource: libc::__rlimit_resource_t, soft: u64, hard: u64) -> io::Result<()> {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: r is a live rlimit.
    if unsafe { libc::getrlimit(resource, &mut r) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let (rlim_cur, rlim_max) = clamped(soft, hard, r.rlim_max);
    if rlim_cur < soft {
        eprintln!("{name} is held at its hard limit of {rlim_cur} rather than {soft}");
    }
    let r = libc::rlimit { rlim_cur, rlim_max };
    // SAFETY: r is a live rlimit.
    if unsafe { libc::setrlimit(resource, &r) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `soft` and `hard` no higher than the current hard limit, which an unprivileged process cannot
/// raise.
fn clamped(soft: u64, hard: u64, current_hard: u64) -> (u64, u64) {
    (soft.min(current_hard), hard.min(current_hard))
}

/// RLIMIT_STACK's soft limit, `otherwise` when it sets none.
pub fn stack_size(otherwise: u64) -> u64 {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: r is a live rlimit.
    let read = unsafe { libc::getrlimit(libc::RLIMIT_STACK, &mut r) } == 0;
    match read && r.rlim_cur != libc::RLIM_INFINITY {
        true => r.rlim_cur,
        false => otherwise,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limits_stay_under_the_current_hard_limit() {
        assert_eq!(clamped(100, 105, libc::RLIM_INFINITY), (100, 105));
        assert_eq!(clamped(100, 105, 102), (100, 102));
        assert_eq!(clamped(100, 105, 50), (50, 50));
    }

    #[test]
    fn a_second_thread_is_counted() {
        let (stop, wait) = std::sync::mpsc::channel::<()>();
        let second = std::thread::spawn(move || wait.recv());
        let counted = threads().unwrap();
        drop(stop);
        let _ = second.join();
        assert!(counted >= 2, "{counted} threads");
    }
}
