//! What the helper does to itself before it reads its request: no new privileges, no descriptor
//! but stdin, stdout and stderr, not dumpable, address space and CPU time capped, every Landlock
//! right handled and none granted, and a seccomp filter that fails every open and kills the
//! process on any other system call but reading stdin, writing stdout and stderr, memory,
//! futex waits and wakes, threads, fault handlers, signals to itself and exit.

use std::io;

use repriseconfine::child::{self, Confined};

use crate::Limits;

/// Room for the executable, the codec's libraries, their data and the main heap's slack.
const PROGRAM: u64 = 512 << 20;
/// A glibc malloc arena: it holds 64 MiB of address space, and making one maps 128 MiB for a
/// moment to align it.
const ARENA: u64 = 128 << 20;
/// A thread's stack when RLIMIT_STACK sets none.
const STACK: u64 = 32 << 20;

/// The limits [`confine`] sets, worked out while the helper may still read `/proc` and `/sys`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The codec's worker threads.
    pub threads: usize,
    /// glibc malloc arenas: one per worker and the main one.
    pub arenas: usize,
    pub address_space: u64,
    /// The soft RLIMIT_CPU.
    pub cpu_seconds: u64,
}

impl Plan {
    /// `limits` with half the cores as workers, each thread's stack as RLIMIT_STACK sets it.
    pub fn new(limits: Limits) -> Plan {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        Plan::of(limits, (cores / 2).max(1), child::stack_size(STACK))
    }

    /// The address space holds the codec's budget, the pixels (refused past the budget) and up
    /// to three times their size for the file on its way out, plus each thread's stack and arena.
    fn of(limits: Limits, threads: usize, stack: u64) -> Plan {
        let per_thread = stack.saturating_add(ARENA);
        let address_space = (limits.budget.saturating_mul(5))
            .saturating_add(PROGRAM)
            .saturating_add(per_thread.saturating_mul(threads as u64 + 1));
        Plan {
            threads,
            arenas: threads + 1,
            address_space,
            cpu_seconds: limits.deadline.as_secs().saturating_mul(threads as u64 + 1),
        }
    }
}

/// Confines the calling process, refused once it has started a thread.
pub fn confine(plan: &Plan) -> io::Result<Confined> {
    let limits = child::Limits {
        address_space: plan.address_space,
        cpu_seconds: plan.cpu_seconds,
        arenas: plan.arenas,
    };
    child::confine(&limits, filter_of(std::process::id()).as_deref())
}

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
fn filter_of(_pid: u32) -> Option<Vec<libc::sock_filter>> {
    None
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter_of(pid: u32) -> Option<Vec<libc::sock_filter>> {
    Some(filter(pid))
}

/// The filter of process `pid`; anything not let through here kills the process.
#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
fn filter(pid: u32) -> Vec<libc::sock_filter> {
    use repriseconfine::seccomp::{self as s, nr};
    // glibc's first pthread_create installs its SIGSETXID handler, the kernel's SIGRTMIN + 1.
    let mut f = s::pipe_helper(pid, &[33]);
    f.extend(s::allow_with(
        nr(libc::SYS_clone),
        0,
        libc::CLONE_THREAD as u32,
    ));
    // clone3 passes its flags in memory a filter cannot read; glibc falls back to clone.
    f.extend(s::refuse_call(nr(libc::SYS_clone3), libc::ENOSYS));
    // Each new glibc thread registers its robust futex list and rseq area.
    for call in [libc::SYS_set_robust_list, libc::SYS_rseq] {
        f.extend(s::allow(nr(call)));
    }
    f.push(s::kill());
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_address_space_holds_the_budget_the_pixels_the_file_and_each_threads_stack_and_arena() {
        let limits = Limits {
            budget: 1 << 30,
            deadline: Duration::from_secs(600),
        };
        let plan = Plan::of(limits, 4, 8 << 20);
        assert_eq!(plan.arenas, 5);
        assert_eq!(
            plan.address_space,
            (5 << 30) + (512 << 20) + 5 * ((8 << 20) + (128 << 20))
        );
        assert_eq!(plan.cpu_seconds, 3000, "the deadline on every thread");
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn the_filter_lets_the_process_signal_itself_and_no_other() {
        use repriseconfine::seccomp::{decide, nr};
        const PID: u32 = 4242;
        let f = filter(PID);
        let call = |n: libc::c_long, args: [u64; 6]| decide(&f, nr(n), args);
        let abrt = libc::SIGABRT as u64;
        let allow = libc::SECCOMP_RET_ALLOW;
        let kill = libc::SECCOMP_RET_KILL_PROCESS;
        assert_eq!(call(libc::SYS_getpid, [0; 6]), allow);
        assert_eq!(call(libc::SYS_gettid, [0; 6]), allow);
        for (n, args) in [
            (libc::SYS_tgkill, [4242, 4243, abrt, 0, 0, 0]),
            (libc::SYS_tkill, [4242, abrt, 0, 0, 0, 0]),
            (libc::SYS_kill, [4242, abrt, 0, 0, 0, 0]),
        ] {
            assert_eq!(call(n, args), allow, "call {n} on itself");
            let mut other = args;
            other[0] = 1;
            assert_eq!(call(n, other), kill, "call {n} on pid 1");
            other[0] = (1 << 32) | 4242;
            assert_eq!(call(n, other), kill, "call {n} with high bits");
        }
        assert_eq!(
            call(libc::SYS_kill, [0, abrt, 0, 0, 0, 0]),
            kill,
            "its group"
        );
        assert_eq!(call(libc::SYS_read, [0; 6]), allow);
        assert_eq!(call(libc::SYS_read, [3, 0, 0, 0, 0, 0]), kill);
        assert_eq!(
            call(libc::SYS_openat, [0; 6]),
            libc::SECCOMP_RET_ERRNO | libc::EACCES as u32
        );
        assert_eq!(
            call(libc::SYS_clone3, [0; 6]),
            libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32
        );
        assert_eq!(call(libc::SYS_socket, [1, 2, 0, 0, 0, 0]), kill);
        let sigaction = |s: i32| call(libc::SYS_rt_sigaction, [s as u64, 0, 0, 0, 0, 0]);
        for s in [33, libc::SIGSEGV, libc::SIGBUS] {
            assert_eq!(sigaction(s), allow, "signal {s}");
        }
        assert_eq!(sigaction(libc::SIGTERM), kill);
        let futex = |op: i32| call(libc::SYS_futex, [0, op as u64, 0, 0, 0, 0]);
        let private = libc::FUTEX_PRIVATE_FLAG;
        assert_eq!(futex(libc::FUTEX_WAIT_BITSET | private), allow);
        assert_eq!(futex(libc::FUTEX_WAKE_OP | private), kill);
    }
}
