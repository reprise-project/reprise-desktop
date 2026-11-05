//! Logs a main-loop turn that runs past 2 s with the main thread's backtrace and kernel state.
//! It only writes to the journal: it never stops or restarts anything.

use smithay::reexports::calloop::{
    self, EventIterator, EventSource, LoopHandle, Poll, PostAction, Readiness, Token, TokenFactory,
};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

/// A turn this long gets its first report.
pub const FIRST: Duration = Duration::from_secs(2);
/// A turn still running this long gets a second.
pub const SECOND: Duration = Duration::from_secs(10);
/// Then one report every this long.
pub const REPEAT: Duration = Duration::from_secs(30);
/// How often the watcher reads the beacon.
const POLL: Duration = Duration::from_millis(500);

/// Nanoseconds of CLOCK_MONOTONIC, which stands still in suspend, so a turn that spans a
/// suspend counts only its waking time.
fn now() -> u64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `t` is a valid timespec to write to.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
    t.tv_sec as u64 * 1_000_000_000 + t.tv_nsec as u64
}

/// The loop's side: a source with no fd that stores `(start << 1) | 1` as a turn begins (the
/// poll returned) and `end << 1` as it ends (the next poll is about to sleep), so an idle loop
/// blocked in epoll reads as idle however long it waits.
pub struct Beacon(Arc<AtomicU64>);

impl EventSource for Beacon {
    type Event = ();
    type Metadata = ();
    type Ret = ();
    type Error = std::io::Error;

    const NEEDS_EXTRA_LIFECYCLE_EVENTS: bool = true;

    fn process_events<F>(&mut self, _: Readiness, _: Token, _: F) -> std::io::Result<PostAction>
    where
        F: FnMut((), &mut ()),
    {
        Ok(PostAction::Continue)
    }

    fn register(&mut self, _: &mut Poll, _: &mut TokenFactory) -> calloop::Result<()> {
        Ok(())
    }

    fn reregister(&mut self, _: &mut Poll, _: &mut TokenFactory) -> calloop::Result<()> {
        Ok(())
    }

    fn unregister(&mut self, _: &mut Poll) -> calloop::Result<()> {
        Ok(())
    }

    fn before_sleep(&mut self) -> calloop::Result<Option<(Readiness, Token)>> {
        self.0.store(now() << 1, Ordering::Relaxed);
        Ok(None)
    }

    fn before_handle_events(&mut self, _: EventIterator<'_>) {
        self.0.store((now() << 1) | 1, Ordering::Relaxed);
    }
}

/// Watch the loop behind `handle`, which must run on this thread. The watcher thread ends with
/// the loop.
pub fn start<D>(handle: &LoopHandle<'static, D>) {
    let beacon = Arc::new(AtomicU64::new(0));
    if let Err(e) = handle.insert_source(Beacon(beacon.clone()), |_, _, _| {}) {
        return log::warn!("stall watch: {}", e.error);
    }
    let target = Target::this_thread();
    if let Err(e) = capture::install() {
        log::warn!("stall watch: no backtraces: {e}");
    }
    let weak = Arc::downgrade(&beacon);
    let spawned = std::thread::Builder::new()
        .name("stall-watch".into())
        .spawn(move || watch(weak, target, |level, line| log::log!(level, "{line}")));
    if let Err(e) = spawned {
        log::warn!("stall watch: {e}");
    }
}

/// The watched thread.
#[derive(Clone, Copy)]
struct Target {
    pthread: libc::pthread_t,
    tid: libc::pid_t,
}

impl Target {
    fn this_thread() -> Target {
        // SAFETY: both only read the calling thread's ids.
        unsafe {
            Target {
                pthread: libc::pthread_self(),
                tid: libc::gettid(),
            }
        }
    }
}

/// Reads the beacon every [`POLL`] and hands each line to `log` until the beacon is gone.
fn watch(beacon: Weak<AtomicU64>, target: Target, mut log: impl FnMut(log::Level, String)) {
    let mut w = Watch::default();
    loop {
        std::thread::sleep(POLL);
        let Some(b) = beacon.upgrade() else { return };
        let step = w.step(b.load(Ordering::Relaxed), now());
        drop(b);
        match step {
            Some(Step::Stuck(d)) => log(log::Level::Warn, report(d, target)),
            Some(Step::Ended(d)) => log(
                log::Level::Info,
                format!("main loop stalled for {:.1} s", d.as_secs_f64()),
            ),
            None => {}
        }
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    /// The turn has run this long and a report is due.
    Stuck(Duration),
    /// A reported turn ended after this long.
    Ended(Duration),
}

/// The reports for one stalled turn at a time.
#[derive(Default)]
struct Watch {
    /// The reported turn's start and its reports so far.
    stall: Option<(u64, u32)>,
}

/// How long a turn has run when its report `n` (from 0) is due.
fn due(n: u32) -> Duration {
    match n {
        0 => FIRST,
        n => SECOND + REPEAT * (n - 1),
    }
}

impl Watch {
    /// What `beacon` read at `now` reports.
    fn step(&mut self, beacon: u64, now: u64) -> Option<Step> {
        let (t, busy) = (beacon >> 1, beacon & 1 == 1);
        if let Some((since, _)) = self.stall
            && !(busy && t == since)
        {
            self.stall = None;
            // Idle: `t` is when the loop went back to sleep; busy: a later turn began at `t`.
            return Some(Step::Ended(Duration::from_nanos(t.saturating_sub(since))));
        }
        if !busy {
            return None;
        }
        let ran = Duration::from_nanos(now.saturating_sub(t));
        let n = self.stall.map_or(0, |(_, n)| n);
        (ran >= due(n)).then(|| {
            self.stall = Some((t, n + 1));
            Step::Stuck(ran)
        })
    }
}

/// The warning for a turn stuck for `ran`: the thread's kernel state, then its stack.
fn report(ran: Duration, target: Target) -> String {
    let task = format!("/proc/self/task/{}", target.tid);
    let read = |f: &str| std::fs::read_to_string(format!("{task}/{f}")).unwrap_or_default();
    let state = read("stat")
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "?".into());
    let mut out = format!(
        "main loop stuck in one turn for {:.1} s: thread state {state}, {}, wchan {}",
        ran.as_secs_f64(),
        syscall(read("syscall").trim()),
        match read("wchan").trim() {
            "" | "0" => "-".to_owned(),
            w => w.to_owned(),
        }
    );
    // A thread in uninterruptible sleep takes the signal only once it wakes.
    let frames = if state == "D" {
        Err("the thread is in uninterruptible sleep".into())
    } else {
        capture::take(target)
    };
    match frames {
        Ok(ips) => {
            out.push_str("\nbacktrace:");
            describe(&ips, &mut out);
        }
        Err(e) => {
            let _ = write!(out, "\nno backtrace: {e}");
        }
    }
    out
}

/// `/proc/<tid>/syscall` as `name(arg, arg, arg)`: "running", or the number and its first
/// three arguments.
fn syscall(line: &str) -> String {
    let mut f = line.split_whitespace();
    let Some(nr) = f.next() else {
        return "syscall ?".into();
    };
    let Ok(n) = nr.parse::<i64>() else {
        return format!("syscall {nr}");
    };
    let args: Vec<&str> = f.take(3).collect();
    format!("syscall {}({})", syscall_name(n), args.join(", "))
}

#[cfg(target_arch = "x86_64")]
fn syscall_name(n: i64) -> String {
    let name = match n {
        0 => "read",
        1 => "write",
        7 => "poll",
        16 => "ioctl",
        23 => "select",
        35 => "nanosleep",
        202 => "futex",
        230 => "clock_nanosleep",
        232 => "epoll_wait",
        257 => "openat",
        262 => "newfstatat",
        271 => "ppoll",
        281 => "epoll_pwait",
        _ => return n.to_string(),
    };
    name.into()
}

#[cfg(not(target_arch = "x86_64"))]
fn syscall_name(n: i64) -> String {
    n.to_string()
}

/// One line per frame and inlined call, like a panic's backtrace; a frame without a symbol
/// gives its object and offset for `addr2line`.
fn describe(ips: &[usize], out: &mut String) {
    for (i, &ip) in ips.iter().take_while(|&&ip| ip != 0).enumerate() {
        // `resolve` looks up `ip - 1`, the call; the first frame is the interrupted instruction.
        let at = if i == 0 { ip + 1 } else { ip };
        let mut named = false;
        backtrace::resolve(at as *mut _, |s| {
            named = true;
            let name = s.name().map_or("<unknown>".into(), |n| format!("{n:#}"));
            let _ = write!(out, "\n  {i:>3}: {name}");
            if let (Some(file), Some(line)) = (s.filename(), s.lineno()) {
                let _ = write!(out, "\n        at {}:{line}", file.display());
            }
        });
        if !named {
            let _ = write!(out, "\n  {i:>3}: {}", object_offset(ip));
        }
    }
}

/// `ip` as `object+0xoffset`, or bare when no loaded object holds it.
fn object_offset(ip: usize) -> String {
    // SAFETY: dladdr only reads the loader's tables and fills `info`; on success dli_fname is
    // null or the object's NUL-terminated path, which stays loaded.
    let object = unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        (libc::dladdr(ip as *const _, &mut info) != 0 && !info.dli_fname.is_null()).then(|| {
            let name = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy();
            (name.into_owned(), info.dli_fbase as usize)
        })
    };
    match object {
        Some((name, base)) => format!("{name}+{:#x}", ip - base),
        None => format!("{ip:#x}"),
    }
}

/// The main thread's stack, taken by its own signal handler: the handler stores raw return
/// addresses in static slots and the watcher names them.
mod capture {
    use super::Target;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    const MAX: usize = 64;
    static FRAMES: [AtomicUsize; MAX] = [const { AtomicUsize::new(0) }; MAX];
    static DEPTH: AtomicUsize = AtomicUsize::new(0);
    static PC: AtomicUsize = AtomicUsize::new(0);
    /// IDLE -> ASKED (watcher) -> TAKING -> TAKEN (handler) -> IDLE (watcher). Only ASKED lets
    /// the handler run, so a late signal finds nothing to do.
    static PHASE: AtomicU8 = AtomicU8::new(IDLE);
    const IDLE: u8 = 0;
    const ASKED: u8 = 1;
    const TAKING: u8 = 2;
    const TAKEN: u8 = 3;

    /// How long the watcher waits for the handler.
    const WAIT: Duration = Duration::from_secs(1);

    fn signal() -> libc::c_int {
        libc::SIGRTMIN() + 5
    }

    /// Installs the handler, unless something else owns the signal. It is ignored after the
    /// exec of a restart ([`before_exec`]).
    pub fn install() -> Result<(), String> {
        let sig = signal();
        // SAFETY: sigaction reads and writes only the structs passed; `on_signal` is
        // async-signal-safe (see there).
        unsafe {
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, std::ptr::null(), &mut old) != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if ![libc::SIG_DFL, libc::SIG_IGN].contains(&old.sa_sigaction) {
                return Err(format!("signal {sig} is taken"));
            }
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = on_signal as *const () as libc::sighandler_t;
            sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
            libc::sigemptyset(&mut sa.sa_mask);
            if libc::sigaction(sig, &sa, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }

    /// Ignores the signal, which drops one still pending: the default action of a real-time
    /// signal ends the process, and an exec keeps pending signals but resets handlers.
    pub fn before_exec() {
        // SAFETY: signal only changes this process's disposition of a signal it owns.
        unsafe { libc::signal(signal(), libc::SIG_IGN) };
    }

    /// The target's return addresses from the interrupted instruction outwards.
    pub fn take(target: Target) -> Result<Vec<usize>, String> {
        match PHASE.load(Ordering::Acquire) {
            ASKED | TAKING => return Err("an earlier backtrace never finished".into()),
            _ => PHASE.store(ASKED, Ordering::Release),
        }
        // SAFETY: the pthread_t is the main thread's, which outlives the watcher.
        let e = unsafe { libc::pthread_kill(target.pthread, signal()) };
        if e != 0 {
            PHASE.store(IDLE, Ordering::Release);
            return Err(std::io::Error::from_raw_os_error(e).to_string());
        }
        let asked = Instant::now();
        while PHASE.load(Ordering::Acquire) != TAKEN {
            if asked.elapsed() > WAIT {
                return match PHASE.compare_exchange(
                    ASKED,
                    IDLE,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => Err("the thread did not take the signal".into()),
                    Err(_) => Err("the stack walk did not finish".into()),
                };
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let n = DEPTH.load(Ordering::Relaxed);
        let ips: Vec<usize> = FRAMES[..n]
            .iter()
            .map(|f| f.load(Ordering::Relaxed))
            .collect();
        let pc = PC.load(Ordering::Relaxed);
        PHASE.store(IDLE, Ordering::Release);
        // The frames before the interrupted one are this handler's.
        let skip = ips.iter().position(|&ip| ip == pc).unwrap_or(0);
        Ok(ips[skip..].to_vec())
    }

    #[cfg(target_arch = "x86_64")]
    fn interrupted_pc(ctx: *mut libc::c_void) -> usize {
        // SAFETY: with SA_SIGINFO the kernel passes the interrupted context as a ucontext_t.
        unsafe {
            (*(ctx as *const libc::ucontext_t)).uc_mcontext.gregs[libc::REG_RIP as usize] as usize
        }
    }

    #[cfg(target_arch = "aarch64")]
    fn interrupted_pc(ctx: *mut libc::c_void) -> usize {
        // SAFETY: with SA_SIGINFO the kernel passes the interrupted context as a ucontext_t.
        unsafe { (*(ctx as *const libc::ucontext_t)).uc_mcontext.pc as usize }
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    fn interrupted_pc(_: *mut libc::c_void) -> usize {
        0
    }

    /// Stores only atomics and walks the stack with the unwinder, which takes no lock or
    /// allocation with libgcc 12 and glibc 2.35 or later (`_dl_find_object`).
    extern "C" fn on_signal(_: libc::c_int, _: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
        if PHASE
            .compare_exchange(ASKED, TAKING, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            return;
        }
        // SAFETY: errno is this thread's; it is put back before returning.
        let errno = unsafe { *libc::__errno_location() };
        PC.store(interrupted_pc(ctx), Ordering::Relaxed);
        let mut n = 0;
        // SAFETY: the closure touches only atomics, and this thread is not walking its stack
        // otherwise: the handler runs once per ASKED, and the loop never uses `backtrace`.
        unsafe {
            backtrace::trace_unsynchronized(|f| {
                FRAMES[n].store(f.ip() as usize, Ordering::Relaxed);
                n += 1;
                n < MAX
            })
        };
        DEPTH.store(n, Ordering::Relaxed);
        PHASE.store(TAKEN, Ordering::Release);
        // SAFETY: as above.
        unsafe { *libc::__errno_location() = errno };
    }
}

pub use capture::before_exec;

/// `REPRISE_STALL_ON_RELOAD=<ms>` in Reprise's environment holds the loop `ms` each time a file
/// in the config directory is written, to try the report.
pub fn reload_hook() {
    use std::sync::OnceLock;
    static MS: OnceLock<Option<u64>> = OnceLock::new();
    let ms = *MS.get_or_init(|| {
        std::env::var("REPRISE_STALL_ON_RELOAD")
            .ok()
            .and_then(|v| v.parse().ok())
    });
    if let Some(ms) = ms {
        log::warn!("REPRISE_STALL_ON_RELOAD: holding the main loop {ms} ms");
        std::thread::sleep(Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::reexports::calloop::EventLoop;
    use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
    use std::sync::mpsc;

    const S: u64 = 1_000_000_000;

    fn busy(start: u64) -> u64 {
        (start << 1) | 1
    }

    fn idle(end: u64) -> u64 {
        end << 1
    }

    #[test]
    fn a_turn_is_reported_at_2_and_10_s_then_every_30_s() {
        let mut w = Watch::default();
        let start = 100 * S;
        let mut reported = vec![];
        for tenth in 0..=1000 {
            let now = start + tenth * S / 10;
            if let Some(Step::Stuck(d)) = w.step(busy(start), now) {
                reported.push(d.as_secs());
            }
        }
        assert_eq!(reported, [2, 10, 40, 70, 100]);
    }

    #[test]
    fn a_reported_turn_logs_its_length_once_when_it_ends() {
        let mut w = Watch::default();
        assert_eq!(
            w.step(busy(10 * S), 12 * S),
            Some(Step::Stuck(Duration::from_secs(2)))
        );
        assert_eq!(w.step(busy(10 * S), 12 * S + S / 2), None);
        let end = 13 * S + S / 2;
        assert_eq!(
            w.step(idle(end), 14 * S),
            Some(Step::Ended(Duration::from_millis(3500)))
        );
        assert_eq!(w.step(idle(end), 15 * S), None);
        assert_eq!(
            w.step(busy(20 * S), 22 * S),
            Some(Step::Stuck(Duration::from_secs(2))),
            "the next stalled turn starts over at 2 s"
        );
    }

    #[test]
    fn short_turns_and_an_idle_loop_never_report() {
        let mut w = Watch::default();
        for i in 0..1000u64 {
            let t = i * S;
            assert_eq!(w.step(busy(t), t + S * 19 / 10), None);
            assert_eq!(
                w.step(idle(t + S / 10), t + 3600 * S),
                None,
                "idle for an hour"
            );
        }
        assert_eq!(w.step(0, 3600 * S), None, "the loop never turned");
    }

    #[test]
    fn the_beacon_is_busy_only_between_a_poll_returning_and_the_next_sleep() {
        let mut lp: EventLoop<'static, Option<u64>> = EventLoop::try_new().unwrap();
        let beacon = Arc::new(AtomicU64::new(0));
        lp.handle()
            .insert_source(Beacon(beacon.clone()), |_, _, _| {})
            .unwrap();
        let seen = beacon.clone();
        lp.handle()
            .insert_source(
                Timer::from_duration(Duration::from_millis(300)),
                move |_, _, d| {
                    *d = Some(seen.load(Ordering::Relaxed));
                    TimeoutAction::Drop
                },
            )
            .unwrap();
        let reader = beacon.clone();
        let sleeping = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            reader.load(Ordering::Relaxed)
        });
        let mut in_turn = None;
        lp.dispatch(None, &mut in_turn).unwrap();
        assert_eq!(
            sleeping.join().unwrap() & 1,
            0,
            "idle while blocked in the poll"
        );
        assert_eq!(in_turn.unwrap() & 1, 1, "busy in a callback");
        assert_eq!(
            beacon.load(Ordering::Relaxed) & 1,
            1,
            "busy until the next sleep"
        );
    }

    #[inline(never)]
    fn hold_the_loop_in_a_test(d: Duration) {
        std::thread::sleep(d);
    }

    #[test]
    fn a_turn_blocked_for_2_5_s_is_reported_with_its_stack_and_its_end() {
        capture::before_exec();
        capture::install().expect("installs over the ignored signal a restart's exec keeps");
        let (tx, rx) = mpsc::channel();
        let (done, finished) = mpsc::channel::<()>();
        let main = std::thread::spawn(move || {
            let beacon = Arc::new(AtomicU64::new(0));
            let weak = Arc::downgrade(&beacon);
            let target = Target::this_thread();
            let watcher = std::thread::spawn(move || {
                watch(weak, target, |level, line| {
                    let _ = tx.send((level, line));
                })
            });
            beacon.store((now() << 1) | 1, Ordering::Relaxed);
            hold_the_loop_in_a_test(Duration::from_millis(2500));
            beacon.store(now() << 1, Ordering::Relaxed);
            let _ = finished.recv();
            drop(beacon);
            watcher.join().unwrap();
        });
        let mut lines = vec![];
        while lines.len() < 2
            && let Ok(line) = rx.recv_timeout(Duration::from_secs(10))
        {
            lines.push(line);
        }
        drop(done);
        main.join().unwrap();
        assert!(rx.iter().next().is_none(), "nothing after the end");
        assert_eq!(lines.len(), 2, "{lines:#?}");
        let (level, stuck) = &lines[0];
        assert_eq!(*level, log::Level::Warn);
        assert!(
            stuck.starts_with("main loop stuck in one turn for 2."),
            "{stuck}"
        );
        assert!(stuck.contains("thread state S"), "{stuck}");
        assert!(stuck.contains("nanosleep("), "{stuck}");
        assert!(stuck.contains("hold_the_loop_in_a_test"), "{stuck}");
        assert!(
            !stuck.contains("on_signal"),
            "the handler's frames are dropped: {stuck}"
        );
        let (level, ended) = &lines[1];
        assert_eq!(*level, log::Level::Info);
        assert!(ended.starts_with("main loop stalled for 2."), "{ended}");
    }

    #[test]
    fn syscall_lines_name_the_call_and_its_first_arguments() {
        assert_eq!(syscall("running"), "syscall running");
        assert_eq!(
            syscall("16 0x15 0xc03864bc 0x7ffd1 0x0 0x0 0x0 0x7ffd0 0x7f12"),
            "syscall ioctl(0x15, 0xc03864bc, 0x7ffd1)"
        );
        assert_eq!(syscall(""), "syscall ?");
    }
}
