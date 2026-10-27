//! App scopes: a program started from a desktop entry runs in `app-reprise-<id>-<n>.scope`, where
//! xdg-desktop-portal reads a host app's id. The compositor and the programs both launch this way;
//! a launch that must report its own exec error is [`hold`]en before the exec instead.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::num::NonZeroUsize;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::time::Duration;

/// Every app scope [`in_scope`] names, as a `systemctl` pattern.
pub const PATTERN: &str = "app-reprise-*.scope";

/// `argv` behind the [`scope_argv`] prefix of `app`, or alone without an app or a user manager.
pub fn in_scope<S: AsRef<OsStr>>(argv: &[S], app: Option<&str>) -> Vec<OsString> {
    let scope = app.filter(|_| user_manager_running());
    let prefix = scope.map(|app| scope_argv(app, &unit_suffix()));
    (prefix.into_iter().flatten().map(Into::into))
        .chain(argv.iter().map(|a| a.as_ref().to_os_string()))
        .collect()
}

/// How long a held child waits for its scope before it runs unscoped.
const HOLD_LIMIT: Duration = Duration::from_secs(5);

/// Makes `c`'s child wait between fork and exec until it is in an app scope of its own named by
/// `app`, as `systemd-run --scope` does, so the program and whatever it forks run there from their
/// first instruction and the spawn still returns the exec's own error. The spawn then blocks on
/// the session bus. Without a user manager the child runs unscoped.
pub fn hold(c: &mut std::process::Command, app: &str) -> std::io::Result<()> {
    if !user_manager_running() {
        return Ok(());
    }
    let app = app.to_owned();
    hold_with(c, HOLD_LIMIT, move |pid| {
        if let Err(e) = adopt(pid, &app) {
            log::warn!("app scope for {app}: {e}");
        }
    })
}

/// [`hold`] running `settle` with the held child's pid, the child going on to its exec once
/// `settle` returns or after `limit`.
fn hold_with(
    c: &mut std::process::Command,
    limit: Duration,
    settle: impl FnOnce(u32) + Send + 'static,
) -> std::io::Result<()> {
    let (pid_r, pid_w) = pipe()?;
    let (go_r, go_w) = pipe()?;
    let go_w_fd = go_w.as_raw_fd();
    std::thread::spawn(move || {
        let mut pid = [0; 4];
        // EOF: the Command was dropped without a fork reaching the hook.
        if std::fs::File::from(pid_r).read_exact(&mut pid).is_err() {
            return;
        }
        let (done, settled) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            settle(u32::from_ne_bytes(pid));
            let _ = done.send(());
        });
        if settled.recv_timeout(limit).is_err() {
            log::warn!("app scope: no answer in {limit:?}, running unscoped");
        }
        // A byte, not EOF: a sibling forked meanwhile may hold a copy of `go_w` until its exec.
        let _ = std::fs::File::from(go_w).write_all(&[1]);
    });
    // SAFETY: runs between fork and exec; close, getpid, write and read are async-signal-safe and
    // nothing allocates. The fds stay open in the parent while the Command lives.
    unsafe {
        c.pre_exec(move || {
            libc::close(go_w_fd);
            let pid = libc::getpid().to_ne_bytes();
            while libc::write(pid_w.as_raw_fd(), pid.as_ptr().cast(), pid.len()) < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() != std::io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
            let mut go = 0u8;
            while libc::read(go_r.as_raw_fd(), (&raw mut go).cast(), 1) < 0 {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    break;
                }
            }
            Ok(())
        });
    }
    Ok(())
}

/// A close-on-exec pipe, read end first.
fn pipe() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` is the two-fd out-param pipe2 fills.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: pipe2 succeeded, so both fds are open and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Moves process `pid` into a new app scope named by `app` and waits for the scope's start job, as
/// `systemd-run --scope` does before its exec. Blocks on the session bus.
fn adopt(pid: u32, app: &str) -> zbus::Result<()> {
    const MANAGER: &str = "org.freedesktop.systemd1.Manager";
    let conn = reprisebus::session()?;
    let (unit, properties) = transient_scope(app, &unit_suffix(), pid);
    // Matched before the call: the job can end before its reply arrives.
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.systemd1")
        .interface(MANAGER)
        .member("JobRemoved")
        .arg(2, unit.as_str())
        .build()?;
    let removed = reprisebus::wait(zbus::MessageStream::for_match_rule(
        rule,
        &conn,
        NonZeroUsize::new(4),
    ))?;
    let aux: Vec<(&str, Vec<(&str, zbus::Variant)>)> = Vec::new();
    let job: zbus::OwnedObjectPath = reprisebus::wait(conn.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some(MANAGER),
        "StartTransientUnit",
        &(&unit, "fail", properties, aux),
    ))?
    .body()
    .deserialize()?;
    for m in reprisebus::blocking_iter(removed) {
        let (_, path, _, result): (u32, zbus::OwnedObjectPath, String, String) =
            m?.body().deserialize()?;
        if path == job {
            return match result.as_str() {
                "done" => Ok(()),
                _ => Err(zbus::Error::Failure(format!("{unit}: {result}"))),
            };
        }
    }
    Ok(())
}

/// `StartTransientUnit`'s unit name and properties for [`adopt`]: the scope and slice
/// [`scope_argv`] gives, with `pid` moved in.
fn transient_scope(
    app: &str,
    suffix: &str,
    pid: u32,
) -> (String, Vec<(&'static str, zbus::Variant<'static>)>) {
    let properties = vec![
        ("PIDs", vec![pid].into()),
        ("Slice", "app.slice".into()),
        ("CollectMode", "inactive-or-failed".into()),
    ];
    (format!("{}.scope", unit_name(app, suffix)), properties)
}

/// Whether `unit` is one of `app`'s scopes, whatever its suffix.
pub fn is_app_scope(unit: &str, app: &str) -> bool {
    unit.strip_prefix(&unit_name(app, ""))
        .and_then(|rest| rest.strip_suffix(".scope"))
        .is_some_and(|suffix| !suffix.is_empty())
}

/// `app-reprise-<app>-<suffix>`; the id is unit-escaped so its dashes stay apart from the
/// launcher and suffix (systemd's app naming).
fn unit_name(app: &str, suffix: &str) -> String {
    let app = app.strip_suffix(".desktop").unwrap_or(app);
    format!("app-reprise-{}-{suffix}", unit_escape(app))
}

/// Prefix that execs a command, same pid, in the [`unit_name`] scope.
fn scope_argv(app: &str, suffix: &str) -> Vec<String> {
    vec![
        "systemd-run".into(),
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        "--slice=app.slice".into(),
        format!("--unit={}", unit_name(app, suffix)),
        "--".into(),
    ]
}

/// `systemd-escape` for one name part.
fn unit_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, b) in s.bytes().enumerate() {
        if b.is_ascii_alphanumeric() || b == b':' || b == b'_' || (b == b'.' && i > 0) {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// Two instances of one app need two scopes, also when two processes launch it and when a
/// process re-execs itself, keeping its pid and starting its count over.
fn unit_suffix() -> String {
    static START: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    suffix(
        std::process::id(),
        *START.get_or_init(monotonic_us),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    )
}

fn monotonic_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes only the timespec it is given.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64) * 1_000_000 + (ts.tv_nsec as u64) / 1_000
}

/// Launch `n` of process `pid` in the run that started at `start`; each `x` keeps the hex fields
/// apart, so pid 0x1a's launch 0x23 is not pid 0x1a2's 0x3.
fn suffix(pid: u32, start: u64, n: u32) -> String {
    format!("{pid:x}x{start:x}x{n:x}")
}

/// Without a user manager `systemd-run --user` fails, so the command runs unscoped.
fn user_manager_running() -> bool {
    static UP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *UP.get_or_init(|| {
        std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|d| std::path::Path::new(&d).join("systemd/private").exists())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_held_child_is_settled_before_its_exec() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut c = std::process::Command::new("true");
        hold_with(&mut c, Duration::from_secs(5), move |pid| {
            let _ = tx.send(std::fs::read_link(format!("/proc/{pid}/exe")).ok());
        })
        .unwrap();
        assert!(c.spawn().unwrap().wait().unwrap().success());
        let exe = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            exe,
            std::env::current_exe().ok(),
            "still this program when settled"
        );
    }

    #[test]
    fn a_held_program_that_cannot_exec_reports_the_exec_error() {
        let mut c = std::process::Command::new("/srv/no-such-program");
        hold_with(&mut c, Duration::from_secs(5), |_| {}).unwrap();
        let e = c.spawn().unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn a_held_child_runs_once_settling_outlasts_the_limit() {
        let mut c = std::process::Command::new("true");
        hold_with(&mut c, Duration::from_millis(50), |_| {
            std::thread::sleep(Duration::from_secs(3));
        })
        .unwrap();
        let t = std::time::Instant::now();
        assert!(c.spawn().unwrap().wait().unwrap().success());
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "ran after {:?}",
            t.elapsed()
        );
    }

    #[test]
    fn a_desktop_entry_launch_runs_in_an_app_scope_named_by_its_id() {
        assert_eq!(
            scope_argv("org.deskflow.deskflow.desktop", "1a"),
            [
                "systemd-run",
                "--user",
                "--scope",
                "--quiet",
                "--collect",
                "--slice=app.slice",
                "--unit=app-reprise-org.deskflow.deskflow-1a",
                "--",
            ]
        );
    }

    #[test]
    fn an_adopted_process_gets_the_app_scope_a_launch_would_run_in() {
        let (unit, properties) = transient_scope("run-me.sh", "1a", 4242);
        assert_eq!(unit, "app-reprise-run\\x2dme.sh-1a.scope");
        assert_eq!(
            properties,
            [
                ("PIDs", zbus::Variant::from(vec![4242u32])),
                ("Slice", "app.slice".into()),
                ("CollectMode", "inactive-or-failed".into()),
            ]
        );
        assert_eq!(properties[0].1.value_signature(), "au");
    }

    #[test]
    fn scope_ids_escape_dashes_and_other_bytes() {
        assert_eq!(unit_escape("reprise-settings"), "reprise\\x2dsettings");
        assert_eq!(unit_escape("a b/c"), "a\\x20b\\x2fc");
        assert_eq!(unit_escape(".hidden"), "\\x2ehidden");
        assert_eq!(unit_escape("org.kde.ark"), "org.kde.ark");
    }

    #[test]
    fn an_app_scope_is_told_apart_from_another_apps_with_a_longer_id() {
        assert!(is_app_scope(
            "app-reprise-syncthing-1ax2.scope",
            "syncthing.desktop"
        ));
        assert!(is_app_scope(
            "app-reprise-run\\x2dme.sh-1a.scope",
            "run-me.sh"
        ));
        assert!(!is_app_scope(
            "app-reprise-syncthing\\x2dgtk-1ax2.scope",
            "syncthing"
        ));
        assert!(!is_app_scope("app-reprise-syncthing-.scope", "syncthing"));
        assert!(!is_app_scope(
            "app-reprise-syncthing-1ax2.service",
            "syncthing"
        ));
    }

    #[test]
    fn launch_scopes_get_distinct_suffixes() {
        assert_ne!(unit_suffix(), unit_suffix());
    }

    #[test]
    fn two_processes_never_share_a_scope_suffix() {
        assert_ne!(suffix(0x1a, 0x5, 0x23), suffix(0x1a2, 0x5, 0x3));
        assert_eq!(suffix(0x1a, 0x5, 0x23), "1ax5x23");
    }

    #[test]
    fn a_process_restarted_under_its_pid_never_reuses_a_scope_suffix() {
        let run = |start| (0..64).map(move |n| suffix(0x1a, start, n));
        let first: std::collections::HashSet<_> = run(0x10).collect();
        assert!(run(0x11).all(|s| !first.contains(&s)));
        assert!(run(0x100).all(|s| !first.contains(&s)));
        assert!(monotonic_us() > 0);
        let unit = format!("{}.scope", unit_name("syncthing", &unit_suffix()));
        assert!(is_app_scope(&unit, "syncthing"), "{unit}");
    }

    #[test]
    fn a_command_without_an_app_runs_as_it_is() {
        assert_eq!(
            in_scope(&["foot", "-e", "top"], None),
            ["foot", "-e", "top"]
        );
    }
}
