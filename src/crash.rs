//! A panic on the compositor thread starts a new session in place: the hook re-execs this binary
//! with `--after-crash <crashes>`, a new session on the same seat. Programs do not survive it;
//! session restore starts them again (docs/features.md, "Crash recovery").

use crate::compositor::State;
use std::cell::Cell;
use std::ffi::OsString;
use std::os::unix::process::CommandExt;
use std::time::{Duration, Instant};

/// A crash this soon after a restart ends the session instead of restarting again.
pub const LOOP_WINDOW: Duration = Duration::from_secs(60);

const FLAG: &str = "--after-crash";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AfterPanic {
    /// Re-exec, the crashes of this login so far, this one included.
    Restart(u32),
    /// Let the panic end the process: the crash count, this one included.
    Exit(u32),
}

/// What a panic `uptime` after this start does, `prior` being the `--after-crash` count it got.
pub fn after_panic(prior: Option<u32>, uptime: Duration) -> AfterPanic {
    let crashes = prior.unwrap_or(0).saturating_add(1);
    if prior.is_some() && uptime < LOOP_WINDOW {
        AfterPanic::Exit(crashes)
    } else {
        AfterPanic::Restart(crashes)
    }
}

/// `argv` without an earlier `--after-crash` and its count, then `--after-crash <crashes>`.
pub fn restart_argv(argv: impl IntoIterator<Item = OsString>, crashes: u32) -> Vec<OsString> {
    let mut out = Vec::new();
    let mut argv = argv.into_iter();
    while let Some(a) = argv.next() {
        if a == FLAG {
            argv.next();
        } else if !a
            .to_str()
            .is_some_and(|a| a.starts_with(&format!("{FLAG}=")))
        {
            out.push(a);
        }
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
/// `screen_locked` cleared; after a crash, the notice after the first frame.
pub fn on_start(state: &mut State, after_crash: bool) {
    if starts_locked(after_crash, state.session.was_locked) {
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

thread_local! {
    static GUARDED: Cell<u32> = const { Cell::new(0) };
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
        match after_panic(prior, uptime) {
            AfterPanic::Exit(n) => log::error!(
                "crash {n} of this login, {}s after the last restart: not restarting",
                uptime.as_secs()
            ),
            AfterPanic::Restart(n) => {
                log::error!("crash {n} of this login: restarting");
                restart(restart_argv(argv.iter().cloned(), n));
            }
        }
    }));
}

/// Exec `/proc/self/exe` (the running binary, even once a package update replaced its file) with
/// `argv` and this environment. Returns only when the exec failed.
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
    let mut c = std::process::Command::new("/proc/self/exe");
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
    fn a_first_crash_restarts_and_one_within_a_minute_of_a_restart_exits() {
        let s = Duration::from_secs;
        assert_eq!(after_panic(None, s(1)), AfterPanic::Restart(1));
        assert_eq!(after_panic(Some(1), s(59)), AfterPanic::Exit(2));
        assert_eq!(after_panic(Some(1), s(60)), AfterPanic::Restart(2));
        assert_eq!(after_panic(Some(3), s(600)), AfterPanic::Restart(4));
    }

    #[test]
    fn a_restart_shows_its_notice_after_the_first_frame_and_a_login_shows_none() {
        let notice = |after_crash| {
            let mut h = crate::harness::Harness::new();
            h.output("TEST-1", 1024, 768);
            on_start(&mut h.state, after_crash);
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
            on_start(&mut h.state, after_crash);
            h.state.is_locked()
        };
        assert!(start(true));
        assert!(!start(false));
    }

    #[test]
    fn a_restart_keeps_the_arguments_and_replaces_the_crash_count() {
        let args = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            restart_argv(args(&["reprise", "--config", "c.toml"]), 1),
            args(&["reprise", "--config", "c.toml", "--after-crash", "1"])
        );
        assert_eq!(
            restart_argv(
                args(&["reprise", "--after-crash", "1", "--startup", "foot"]),
                2
            ),
            args(&["reprise", "--startup", "foot", "--after-crash", "2"])
        );
        assert_eq!(
            restart_argv(args(&["reprise", "--after-crash=2"]), 3),
            args(&["reprise", "--after-crash", "3"])
        );
    }
}
