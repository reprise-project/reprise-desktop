//! seccomp BPF filters for the native arch. Another arch's filter would need its own `AUDIT_ARCH`
//! and, on i686, `socketcall(2)`.

use std::io;

/// `AUDIT_ARCH_*` of the native system calls.
#[cfg(target_arch = "x86_64")]
const ARCH: u32 = 0xc000_003e;
#[cfg(target_arch = "aarch64")]
const ARCH: u32 = 0xc000_00b7;
/// x32 system calls carry x86_64's arch and set this bit in their number.
const X32_BIT: u32 = 0x4000_0000;

/// Offsets into `seccomp_data`; an argument's low 32 bits come first on little-endian.
pub const NR: u32 = 0;
const ARCH_AT: u32 = 4;
pub const fn arg(n: u32) -> u32 {
    16 + 8 * n
}
const fn arg_high(n: u32) -> u32 {
    arg(n) + 4
}

/// Installs `filter` on the calling thread and the threads it starts later.
pub fn install(filter: &[libc::sock_filter]) -> io::Result<()> {
    let program = program(filter);
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
    Ok(())
}

/// Installs `filter` on every thread of the process and the threads they start later, or on none
/// of them.
pub fn install_on_every_thread(filter: &[libc::sock_filter]) -> io::Result<()> {
    let program = program(filter);
    // SAFETY: program points at `filter`, which outlives the call; the kernel copies it.
    let set = unsafe {
        libc::syscall(
            libc::SYS_seccomp,
            libc::SECCOMP_SET_MODE_FILTER,
            libc::SECCOMP_FILTER_FLAG_TSYNC,
            &raw const program,
        )
    };
    match set {
        0 => Ok(()),
        -1 => Err(io::Error::last_os_error()),
        tid => Err(io::Error::other(format!(
            "thread {tid} cannot take the seccomp filter"
        ))),
    }
}

fn program(filter: &[libc::sock_filter]) -> libc::sock_fprog {
    libc::sock_fprog {
        len: libc::c_ushort::try_from(filter.len())
            .expect("a filter of at most 65535 instructions"),
        filter: filter.as_ptr().cast_mut(),
    }
}

/// A filter's start: a call of another arch, or an x32 call, takes `foreign`; the call's number
/// is loaded for what follows.
pub fn native(foreign: libc::sock_filter) -> Vec<libc::sock_filter> {
    let mut f = vec![load(ARCH_AT), jump(libc::BPF_JEQ, ARCH, 1, 0), foreign];
    f.push(load(NR));
    if cfg!(target_arch = "x86_64") {
        f.extend([jump(libc::BPF_JGE, X32_BIT, 0, 1), foreign]);
    }
    f
}

pub fn nr(call: libc::c_long) -> u32 {
    call as u32
}

pub fn statement(code: u32, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code: code as u16,
        jt: 0,
        jf: 0,
        k,
    }
}

pub fn load(offset: u32) -> libc::sock_filter {
    statement(libc::BPF_LD | libc::BPF_W | libc::BPF_ABS, offset)
}

/// Skips `jt` instructions when `op` holds against `k`, else `jf`.
pub fn jump(op: u32, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter {
        code: (libc::BPF_JMP | op | libc::BPF_K) as u16,
        jt,
        jf,
        k,
    }
}

fn ret(action: u32) -> libc::sock_filter {
    statement(libc::BPF_RET | libc::BPF_K, action)
}

pub fn ret_allow() -> libc::sock_filter {
    ret(libc::SECCOMP_RET_ALLOW)
}

pub fn kill() -> libc::sock_filter {
    ret(libc::SECCOMP_RET_KILL_PROCESS)
}

pub fn refuse(errno: i32) -> libc::sock_filter {
    ret(libc::SECCOMP_RET_ERRNO | errno as u32)
}

// The blocks below expect the call's number loaded and leave it loaded for the next block when
// the call is another; a call they match is decided inside them.

/// Lets `call` through.
pub fn allow(call: u32) -> [libc::sock_filter; 2] {
    [jump(libc::BPF_JEQ, call, 0, 1), ret_allow()]
}

/// Fails `call` with `errno`.
pub fn refuse_call(call: u32, errno: i32) -> [libc::sock_filter; 2] {
    [jump(libc::BPF_JEQ, call, 0, 1), refuse(errno)]
}

/// Lets `call` through with one of `values` as its first argument, kills the process on any
/// other.
pub fn allow_first_in(call: u32, values: &[u32]) -> Vec<libc::sock_filter> {
    allow_masked_in(call, 0, u32::MAX, values)
}

/// Lets `call` through when the low 32 bits of argument `n`, masked with `mask`, are one of
/// `values`; kills the process on any other.
pub fn allow_masked_in(call: u32, n: u32, mask: u32, values: &[u32]) -> Vec<libc::sock_filter> {
    let k = values.len();
    let and =
        (mask != u32::MAX).then(|| statement(libc::BPF_ALU | libc::BPF_AND | libc::BPF_K, mask));
    let mut f = vec![
        jump(
            libc::BPF_JEQ,
            call,
            0,
            skip(k + 3 + usize::from(and.is_some())),
        ),
        load(arg(n)),
    ];
    f.extend(and);
    for (i, &v) in values.iter().enumerate() {
        let jf = if i + 1 == k { 1 } else { 0 };
        f.push(jump(libc::BPF_JEQ, v, skip(k - 1 - i), jf));
    }
    f.extend([ret_allow(), kill()]);
    f
}

/// Lets `call` through when its whole 64-bit first argument is `value`, kills the process
/// otherwise.
pub fn allow_first_is(call: u32, value: u32) -> [libc::sock_filter; 7] {
    [
        jump(libc::BPF_JEQ, call, 0, 6),
        load(arg(0)),
        jump(libc::BPF_JEQ, value, 0, 3),
        load(arg_high(0)),
        jump(libc::BPF_JEQ, 0, 0, 1),
        ret_allow(),
        kill(),
    ]
}

/// A jump over `n` instructions, which a BPF jump holds up to 255 of.
fn skip(n: usize) -> u8 {
    u8::try_from(n).expect("a seccomp block of at most 255 instructions")
}

/// Lets `call` through when argument `n` has `bit` set, kills the process otherwise.
pub fn allow_with(call: u32, n: u32, bit: u32) -> [libc::sock_filter; 5] {
    [
        jump(libc::BPF_JEQ, call, 0, 4),
        load(arg(n)),
        jump(libc::BPF_JSET, bit, 0, 1),
        ret_allow(),
        kill(),
    ]
}

/// Lets `call` through when argument `n` has none of `bits`, kills the process otherwise.
pub fn allow_without(call: u32, n: u32, bits: u32) -> [libc::sock_filter; 5] {
    [
        jump(libc::BPF_JEQ, call, 0, 4),
        load(arg(n)),
        jump(libc::BPF_JSET, bits, 1, 0),
        ret_allow(),
        kill(),
    ]
}

/// The start of a pipe helper's filter in process `pid`, which kills on any other arch: read on
/// stdin, write on stdout and stderr, memory without new code, futex waits and wakes, handlers
/// for SIGSEGV, SIGBUS and `handled` and their return, signals to itself (so `abort()` dies of
/// SIGABRT), exit, and every open failing. The helper adds its own calls and ends the filter
/// with [`kill`].
pub fn pipe_helper(pid: u32, handled: &[u32]) -> Vec<libc::sock_filter> {
    let mut f = native(kill());
    f.extend(allow_first_in(nr(libc::SYS_read), &[0]));
    f.extend(allow_first_in(nr(libc::SYS_write), &[1, 2]));
    for call in [libc::SYS_mmap, libc::SYS_mprotect] {
        f.extend(allow_without(nr(call), 2, libc::PROT_EXEC as u32));
    }
    // Rust's handler meets a fault outside a stack guard by restoring SIG_DFL and returning
    // through rt_sigreturn, so the process dies of the signal itself.
    let signals = [libc::SIGSEGV as u32, libc::SIGBUS as u32];
    f.extend(allow_first_in(
        nr(libc::SYS_rt_sigaction),
        &[&signals[..], handled].concat(),
    ));
    f.extend(allow_masked_in(
        nr(libc::SYS_futex),
        1,
        libc::FUTEX_CMD_MASK as u32,
        &[
            libc::FUTEX_WAIT,
            libc::FUTEX_WAKE,
            libc::FUTEX_WAIT_BITSET,
            libc::FUTEX_WAKE_BITSET,
        ]
        .map(|op| op as u32),
    ));
    // glibc reads /proc/sys/vm/overcommit_memory the first time a thread's arena shrinks and does
    // without it when the open fails.
    f.extend(refuse_call(nr(libc::SYS_openat), libc::EACCES));
    // abort() raises SIGABRT with tgkill(getpid(), gettid(), ..); the process may signal itself
    // and no other.
    for call in [libc::SYS_kill, libc::SYS_tkill, libc::SYS_tgkill] {
        f.extend(allow_first_is(nr(call), pid));
    }
    for call in [
        libc::SYS_munmap,
        libc::SYS_mremap,
        libc::SYS_madvise,
        libc::SYS_brk,
        libc::SYS_rt_sigprocmask,
        libc::SYS_rt_sigreturn,
        libc::SYS_getpid,
        libc::SYS_gettid,
        libc::SYS_exit,
        libc::SYS_exit_group,
    ] {
        f.extend(allow(nr(call)));
    }
    f
}

/// What the kernel returns for native system call `nr` with `args` under `filter`, for tests.
/// Knows the instructions this module writes.
pub fn decide(filter: &[libc::sock_filter], nr: u32, args: [u64; 6]) -> u32 {
    let mut data = [0u8; 64];
    data[NR as usize..][..4].copy_from_slice(&nr.to_ne_bytes());
    data[ARCH_AT as usize..][..4].copy_from_slice(&ARCH.to_ne_bytes());
    for (n, a) in (0..).zip(args) {
        data[arg(n) as usize..][..8].copy_from_slice(&a.to_ne_bytes());
    }
    let (mut pc, mut acc) = (0, 0u32);
    loop {
        let i = filter[pc];
        let code = u32::from(i.code);
        pc += 1;
        if code == libc::BPF_RET | libc::BPF_K {
            return i.k;
        } else if code == libc::BPF_LD | libc::BPF_W | libc::BPF_ABS {
            let at = i.k as usize;
            acc = u32::from_ne_bytes(data[at..at + 4].try_into().unwrap());
        } else if code == libc::BPF_ALU | libc::BPF_AND | libc::BPF_K {
            acc &= i.k;
        } else {
            let holds = match code {
                c if c == libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K => acc == i.k,
                c if c == libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K => acc >= i.k,
                c if c == libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K => acc & i.k != 0,
                _ => panic!("instruction {code:#x} at {}", pc - 1),
            };
            pc += usize::from(if holds { i.jt } else { i.jf });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(block: &[libc::sock_filter]) -> Vec<libc::sock_filter> {
        let mut f = native(kill());
        f.extend_from_slice(block);
        f.push(refuse(libc::EPERM));
        f
    }

    #[test]
    fn a_whole_first_argument_check_compares_both_halves() {
        let f = filter(&allow_first_is(62, 4242));
        let first = |a0: u64| decide(&f, 62, [a0, 9, 0, 0, 0, 0]);
        assert_eq!(first(4242), libc::SECCOMP_RET_ALLOW);
        assert_eq!(first(4243), libc::SECCOMP_RET_KILL_PROCESS);
        assert_eq!(first((1 << 32) | 4242), libc::SECCOMP_RET_KILL_PROCESS);
        assert_eq!(
            decide(&f, 63, [4242, 0, 0, 0, 0, 0]),
            libc::SECCOMP_RET_ERRNO | libc::EPERM as u32,
            "another call goes on to the next block"
        );
    }

    #[test]
    fn a_first_argument_list_lets_through_each_value_and_kills_on_others() {
        let f = filter(&allow_first_in(1, &[1, 2]));
        for (a0, want) in [
            (1, libc::SECCOMP_RET_ALLOW),
            (2, libc::SECCOMP_RET_ALLOW),
            (3, libc::SECCOMP_RET_KILL_PROCESS),
        ] {
            assert_eq!(decide(&f, 1, [a0, 0, 0, 0, 0, 0]), want, "fd {a0}");
        }
    }

    #[test]
    fn a_futex_op_is_let_through_by_its_command_whatever_its_flags() {
        let f = [&pipe_helper(4242, &[])[..], &[kill()]].concat();
        let futex = |op: i32| decide(&f, nr(libc::SYS_futex), [0, op as u64, 0, 0, 0, 0]);
        let private = libc::FUTEX_PRIVATE_FLAG;
        for op in [
            libc::FUTEX_WAIT,
            libc::FUTEX_WAKE | private,
            libc::FUTEX_WAIT_BITSET | private | libc::FUTEX_CLOCK_REALTIME,
            libc::FUTEX_WAKE_BITSET,
        ] {
            assert_eq!(futex(op), libc::SECCOMP_RET_ALLOW, "op {op:#x}");
        }
        for op in [
            libc::FUTEX_REQUEUE,
            libc::FUTEX_CMP_REQUEUE,
            libc::FUTEX_LOCK_PI | private,
        ] {
            assert_eq!(futex(op), libc::SECCOMP_RET_KILL_PROCESS, "op {op:#x}");
        }
    }

    #[test]
    fn a_signal_handler_is_let_through_for_segv_bus_and_the_ones_asked() {
        let f = [&pipe_helper(4242, &[33])[..], &[kill()]].concat();
        let sigaction = |s: i32| decide(&f, nr(libc::SYS_rt_sigaction), [s as u64, 0, 0, 0, 0, 0]);
        for s in [libc::SIGSEGV, libc::SIGBUS, 33] {
            assert_eq!(sigaction(s), libc::SECCOMP_RET_ALLOW, "signal {s}");
        }
        for s in [libc::SIGTERM, libc::SIGSYS, libc::SIGALRM] {
            assert_eq!(sigaction(s), libc::SECCOMP_RET_KILL_PROCESS, "signal {s}");
        }
        let sigreturn = decide(&f, nr(libc::SYS_rt_sigreturn), [0; 6]);
        assert_eq!(sigreturn, libc::SECCOMP_RET_ALLOW, "a handler returns");
    }

    #[test]
    #[should_panic(expected = "at most 255")]
    fn a_block_past_a_jumps_reach_is_refused() {
        allow_first_in(1, &[0; 253]);
    }
}
