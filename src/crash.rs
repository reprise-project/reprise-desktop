//! A panic on the compositor thread starts a new session in place: the hook re-execs this binary
//! with `--after-crash <crashes>`, a new session on the same seat. Wayland programs do not survive
//! it and session restore starts them again; autostart skips a program whose app scope still runs
//! (docs/features.md, "Crash recovery").

use crate::compositor::State;
use std::cell::Cell;
use std::ffi::OsString;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A crash this soon after a restart ends the session instead of restarting again.
pub const LOOP_WINDOW: Duration = Duration::from_secs(60);

const FLAG: &str = "--after-crash";
const LOCKED: &str = "--locked";
const STARTUP: &str = "--startup";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterPanic {
    /// Re-exec, the crashes of this login so far, this one included.
    Restart(u32),
    /// Let the panic end the process: the crash count, this one included.
    Exit(u32),
}

/// What a panic `uptime` after this start does, `prior` being the `--after-crash` count it got
/// and `ending` whether the session was ending (`ending`).
pub fn after_panic(prior: Option<u32>, uptime: Duration, ending: bool) -> AfterPanic {
    let crashes = prior.unwrap_or(0).saturating_add(1);
    if ending || (prior.is_some() && uptime < LOOP_WINDOW) {
        AfterPanic::Exit(crashes)
    } else {
        AfterPanic::Restart(crashes)
    }
}

/// `argv` without an earlier `--after-crash` and its count, `--startup` and its command (whatever
/// it started outlives the crash) or `--locked`, then `--locked` when `locked` and
/// `--after-crash <crashes>`.
pub fn restart_argv(
    argv: impl IntoIterator<Item = OsString>,
    crashes: u32,
    locked: bool,
) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut argv = argv.into_iter();
    let valued = [FLAG, STARTUP];
    while let Some(a) = argv.next() {
        if valued.iter().any(|f| a == *f) {
            argv.next();
        } else if a != LOCKED
            && !a.to_str().is_some_and(|a| {
                valued
                    .iter()
                    .any(|f| a.strip_prefix(f).is_some_and(|v| v.starts_with('=')))
            })
        {
            out.push(a);
        }
    }
    if locked {
        out.push(LOCKED.into());
    }
    out.extend([FLAG.into(), crashes.to_string().into()]);
    out
}

/// The notice a restarted session shows.
pub const NOTICE: &str = "Reprise restarted after a crash; the log has the details";

/// Whether this start engages the lock before its first frame: only a restart whose crashed
/// session was locked.
pub fn starts_locked(after_crash: bool, was_locked: bool) -> bool {
    after_crash && was_locked
}

/// A start's steps once the backend is up: the lock when `starts_locked`, else a stale
/// `screen_locked` cleared; after a crash, the notice after the first frame. `locked`: the
/// restart was passed `--locked`.
pub fn on_start(state: &mut State, after_crash: bool, locked: bool) {
    if starts_locked(after_crash, locked || state.session.was_locked) {
        crate::lockview::lock(state);
    } else if state.session.was_locked {
        crate::state::save_screen_locked(false);
    }
    if after_crash {
        state.handle.insert_idle(|state| {
            crate::notifyview::notice(state, "Reprise", NOTICE);
        });
    }
}

// Per thread: the hook restarts only for a panic on the compositor's thread, the one that locks.
thread_local! {
    static GUARDED: Cell<u32> = const { Cell::new(0) };
    static LOCK_WANTED: Cell<bool> = const { Cell::new(false) };
    static ENDING: Cell<bool> = const { Cell::new(false) };
}

/// The session is ending (the loop stopped, or a signal asked it to): a panic from here on ends
/// the process instead of restarting it.
pub fn ending() {
    ENDING.set(true);
}

/// A refused Turn Off or Restart: the session goes on, and a panic restarts it again.
pub fn not_ending() {
    ENDING.set(false);
}

pub fn is_ending() -> bool {
    ENDING.get()
}

/// Whether a restart from here on starts locked: set as a lock begins, before anything that can
/// panic, and cleared as an unlock ends.
pub fn set_locked(locked: bool) {
    LOCK_WANTED.set(locked);
}

pub fn locked() -> bool {
    LOCK_WANTED.get()
}

/// `catch_unwind` for loop work whose panic the session survives: the hook leaves it alone.
pub fn guarded<R>(f: impl FnOnce() -> R) -> std::thread::Result<R> {
    GUARDED.with(|g| g.set(g.get() + 1));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    GUARDED.with(|g| g.set(g.get() - 1));
    r
}

/// Restart on a panic on this thread, the compositor's, after the hook set before (the log's)
/// ran. `prior` is this start's `--after-crash` count.
pub fn install(prior: Option<u32>) {
    let started = Instant::now();
    let compositor = std::thread::current().id();
    let argv: Vec<OsString> = std::env::args_os().collect();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if std::thread::current().id() != compositor || GUARDED.with(|g| g.get()) > 0 {
            return;
        }
        let uptime = started.elapsed();
        let ending = is_ending();
        match after_panic(prior, uptime, ending) {
            AfterPanic::Exit(n) if ending => {
                log::error!("crash {n} of this login while the session ends: not restarting")
            }
            AfterPanic::Exit(n) => log::error!(
                "crash {n} of this login, {}s after the last restart: not restarting",
                uptime.as_secs()
            ),
            AfterPanic::Restart(n) => match take_end_signal() {
                Some(sig) => {
                    log::error!("crash {n} of this login with signal {sig} pending: not restarting")
                }
                None => {
                    log::error!("crash {n} of this login: restarting");
                    restart(restart_argv(argv.iter().cloned(), n, locked()));
                }
            },
        }
    }));
}

/// A SIGTERM or SIGINT pending for this thread, taken. The loop keeps them blocked for its
/// signalfd, and the exec would empty the mask and let the default action kill the restart.
fn take_end_signal() -> Option<libc::c_int> {
    // SAFETY: the set is initialised by sigemptyset before use; a zero timeout never blocks.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for sig in [libc::SIGTERM, libc::SIGINT] {
            libc::sigaddset(&mut set, sig);
        }
        let zero = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let sig = libc::sigtimedwait(&set, std::ptr::null_mut(), &zero);
        (sig > 0).then_some(sig)
    }
}

const SELF_EXE: &str = "/proc/self/exe";

/// The file a restart execs: the running binary's own path, so the process keeps its name for
/// pgrep and killall, or `/proc/self/exe` once that path no longer holds the running image (a
/// package update replaced it), which still reaches the running image.
fn exec_path(link: std::io::Result<PathBuf>, runs_from: impl Fn(&Path) -> bool) -> PathBuf {
    link.ok()
        .filter(|p| runs_from(p))
        .unwrap_or_else(|| SELF_EXE.into())
}

fn is_running_image(p: &Path) -> bool {
    match (std::fs::metadata(p), std::fs::metadata(SELF_EXE)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Exec the running binary (see [`exec_path`]) with `argv` and this environment. Returns only
/// when the exec failed.
fn restart(argv: Vec<OsString>) {
    crate::state::flush_within(Duration::from_secs(1));
    // Every descriptor but stdio closes at the exec, whoever opened it: the seat's devices, the
    // listening socket and its lock, the bus connections.
    // SAFETY: close_range only sets a flag on this process's own descriptors.
    unsafe {
        libc::close_range(
            3,
            libc::c_uint::MAX,
            libc::CLOSE_RANGE_CLOEXEC as libc::c_int,
        )
    };
    let exe = exec_path(std::fs::read_link(SELF_EXE), is_running_image);
    let mut c = std::process::Command::new(exe);
    if let Some((first, rest)) = argv.split_first() {
        c.arg0(first).args(rest);
    }
    let e = c.exec();
    log::error!("restart: {e}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_end_signal_is_taken_once() {
        std::thread::spawn(|| {
            // SAFETY: the mask and the signal touch only this thread.
            unsafe {
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigaddset(&mut set, libc::SIGINT);
                libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                assert_eq!(take_end_signal(), None);
                libc::pthread_kill(libc::pthread_self(), libc::SIGINT);
            }
            assert_eq!(take_end_signal(), Some(libc::SIGINT));
            assert_eq!(take_end_signal(), None);
        })
        .join()
        .unwrap();
    }

    #[test]
    fn a_restart_execs_the_binary_by_its_path_while_the_path_holds_it() {
        let bin = || Ok(PathBuf::from("/usr/bin/reprise"));
        assert_eq!(exec_path(bin(), |_| true), Path::new("/usr/bin/reprise"));
        assert_eq!(exec_path(bin(), |_| false), Path::new(SELF_EXE), "replaced");
        let deleted = Ok(PathBuf::from("/usr/bin/reprise (deleted)"));
        assert_eq!(exec_path(deleted, is_running_image), Path::new(SELF_EXE));
        let unreadable = Err(std::io::Error::other("no /proc"));
        assert_eq!(exec_path(unreadable, |_| true), Path::new(SELF_EXE));
        let own = std::fs::read_link(SELF_EXE).unwrap();
        assert_eq!(exec_path(Ok(own.clone()), is_running_image), own);
    }

    #[test]
    fn a_first_crash_restarts_and_one_within_a_minute_of_a_restart_exits() {
        let s = Duration::from_secs;
        assert_eq!(after_panic(None, s(1), false), AfterPanic::Restart(1));
        assert_eq!(after_panic(Some(1), s(59), false), AfterPanic::Exit(2));
        assert_eq!(after_panic(Some(1), s(60), false), AfterPanic::Restart(2));
        assert_eq!(after_panic(Some(3), s(600), false), AfterPanic::Restart(4));
    }

    #[test]
    fn a_crash_while_the_session_ends_never_restarts() {
        let s = Duration::from_secs;
        assert_eq!(after_panic(None, s(600), true), AfterPanic::Exit(1));
        assert_eq!(after_panic(Some(1), s(600), true), AfterPanic::Exit(2));
    }

    #[test]
    fn a_restart_shows_its_notice_after_the_first_frame_and_a_login_shows_none() {
        let notice = |after_crash| {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            on_start(&mut h.state, after_crash, false);
            assert!(
                h.state.notify.log.get(u32::MAX).is_none(),
                "nothing before a frame"
            );
            h.dispatch();
            h.state.notify.log.get(u32::MAX).map(|e| e.n.body.clone())
        };
        assert_eq!(notice(true).as_deref(), Some(NOTICE));
        assert_eq!(notice(false), None);
    }

    #[test]
    fn only_a_restart_from_a_locked_session_starts_locked() {
        assert!(starts_locked(true, true));
        assert!(!starts_locked(true, false));
        assert!(
            !starts_locked(false, true),
            "a login is never locked by the last session"
        );
    }

    #[test]
    fn a_restart_from_a_locked_session_is_locked_before_its_first_frame() {
        let start = |after_crash| {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            h.state.session.was_locked = true;
            on_start(&mut h.state, after_crash, false);
            h.state.is_locked()
        };
        assert!(start(true));
        assert!(!start(false));
    }

    #[test]
    fn a_restart_keeps_the_arguments_but_the_startup_command_and_replaces_the_crash_count() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            restart_argv(args(&["reprise", "--config", "c.toml"]), 1, false),
            args(&["reprise", "--config", "c.toml", "--after-crash", "1"])
        );
        assert_eq!(
            restart_argv(
                args(&["reprise", "--after-crash", "1", "--startup", "foot"]),
                2,
                false
            ),
            args(&["reprise", "--after-crash", "2"])
        );
        assert_eq!(
            restart_argv(
                args(&["reprise", "--startup=foot", "--config=c.toml"]),
                1,
                false
            ),
            args(&["reprise", "--config=c.toml", "--after-crash", "1"])
        );
        assert_eq!(
            restart_argv(args(&["reprise", "--after-crash=2"]), 3, false),
            args(&["reprise", "--after-crash", "3"])
        );
    }

    #[test]
    fn a_restart_while_locked_carries_one_locked_flag_and_an_unlocked_one_none() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            restart_argv(
                args(&["reprise", "--locked", "--after-crash", "1"]),
                2,
                true
            ),
            args(&["reprise", "--locked", "--after-crash", "2"])
        );
        assert_eq!(
            restart_argv(
                args(&["reprise", "--locked", "--after-crash", "1"]),
                2,
                false
            ),
            args(&["reprise", "--after-crash", "2"])
        );
    }

    #[test]
    fn a_restart_passed_locked_starts_locked_without_the_state_file() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.session.was_locked = false;
        on_start(&mut h.state, true, true);
        assert!(h.state.is_locked());
    }
}
