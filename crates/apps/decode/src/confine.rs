//! What reprise-decode does to itself before it reads its request ([`repriseconfine::child`]):
//! one thread, address space and CPU time capped, no file or network, and a seccomp filter that
//! fails every open and kills the process on any other system call but reading stdin, writing
//! stdout and stderr, memory, futex waits and wakes, the clock, random bytes, fault handlers,
//! signals to itself and exit. The decoders start no threads, so the filter lets none be made.

use std::io;

use reprise_decode_protocol::Limits;
use repriseconfine::child::{self, Confined};

/// Room for the executable, its libraries and the main heap's slack.
const PROGRAM: u64 = 256 << 20;
/// The main thread's stack when RLIMIT_STACK sets none.
const STACK: u64 = 8 << 20;

/// The address space holds the file, the larger decoder's heap budget, a picture's worth of the
/// small allocations the budget never refuses, and the stack. The CPU time is the deadline.
pub fn limits(limits: Limits, stack: u64) -> child::Limits {
    let heap = crate::heap_budget(limits.pixels).saturating_add(limits.pixels);
    child::Limits {
        address_space: PROGRAM
            .saturating_add(stack)
            .saturating_add(limits.file)
            .saturating_add(heap),
        cpu_seconds: limits.deadline.as_secs().max(1),
        arenas: 1,
    }
}

/// Confines the calling process under `limits`, refused once it has started a thread.
pub fn confine(limits: Limits) -> io::Result<Confined> {
    let set = self::limits(limits, child::stack_size(STACK));
    child::confine(&set, filter_of(std::process::id()).as_deref())
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
    let mut f = s::pipe_helper(pid, &[]);
    // The deadline reads the clock, through the vDSO unless the clock source has none; the
    // decoder's hash sets ask for random keys.
    for call in [libc::SYS_clock_gettime, libc::SYS_getrandom] {
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
    fn the_address_space_holds_the_file_the_heap_and_the_stack() {
        let l = Limits {
            file: 64 << 20,
            pixels: 96 << 20,
            deadline: Duration::from_secs(60),
        };
        let set = limits(l, 8 << 20);
        assert_eq!(
            set.address_space,
            (256 << 20) + (8 << 20) + (64 << 20) + 5 * (96 << 20) + (4 << 20)
        );
        assert_eq!((set.cpu_seconds, set.arenas), (60, 1));
    }

    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    #[test]
    fn the_filter_lets_no_thread_file_or_socket_through() {
        use repriseconfine::seccomp::{decide, nr};
        const PID: u32 = 4242;
        let f = filter(PID);
        let call = |n: libc::c_long, args: [u64; 6]| decide(&f, nr(n), args);
        let allow = libc::SECCOMP_RET_ALLOW;
        let kill = libc::SECCOMP_RET_KILL_PROCESS;
        assert_eq!(call(libc::SYS_read, [0; 6]), allow);
        assert_eq!(call(libc::SYS_read, [3, 0, 0, 0, 0, 0]), kill);
        assert_eq!(call(libc::SYS_write, [2, 0, 0, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_write, [0; 6]), kill);
        let exec = libc::PROT_EXEC as u64;
        assert_eq!(call(libc::SYS_mmap, [0, 4096, 3, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_mmap, [0, 4096, exec, 0, 0, 0]), kill);
        let abrt = libc::SIGABRT as u64;
        assert_eq!(call(libc::SYS_tgkill, [4242, 4242, abrt, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_tgkill, [1, 1, abrt, 0, 0, 0]), kill);
        assert_eq!(call(libc::SYS_clock_gettime, [1, 0, 0, 0, 0, 0]), allow);
        assert_eq!(call(libc::SYS_getrandom, [0; 6]), allow);
        assert_eq!(
            call(libc::SYS_openat, [0; 6]),
            libc::SECCOMP_RET_ERRNO | libc::EACCES as u32
        );
        let thread = libc::CLONE_THREAD as u64;
        assert_eq!(call(libc::SYS_clone, [thread, 0, 0, 0, 0, 0]), kill);
        assert_eq!(call(libc::SYS_clone3, [0; 6]), kill);
        assert_eq!(call(libc::SYS_socket, [1, 2, 0, 0, 0, 0]), kill);
        assert_eq!(call(libc::SYS_execve, [0; 6]), kill);
        let sigaction = |s: i32| call(libc::SYS_rt_sigaction, [s as u64, 0, 0, 0, 0, 0]);
        assert_eq!(sigaction(libc::SIGSEGV), allow);
        assert_eq!(sigaction(33), kill, "no thread, so no SIGSETXID");
        let futex = |op: i32| call(libc::SYS_futex, [0, op as u64, 0, 0, 0, 0]);
        assert_eq!(futex(libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG), allow);
        assert_eq!(futex(libc::FUTEX_CMP_REQUEUE), kill);
    }
}
