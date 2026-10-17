//! App scopes: a program started from a desktop entry runs in `app-reprise-<id>-<n>.scope`, where
//! xdg-desktop-portal reads a host app's id. The compositor and the programs both launch this way;
//! a process that has to start unscoped is [`adopt`]ed after its exec.

use std::ffi::{OsStr, OsString};

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

/// Moves the running process `pid` into an app scope of its own named by `app`, for a program
/// started unscoped to keep its exec error (behind `systemd-run` that error would be systemd-run's).
/// Blocks on the session bus; does nothing without a user manager.
pub fn adopt(pid: u32, app: &str) -> zbus::Result<()> {
    if !user_manager_running() {
        return Ok(());
    }
    let (unit, properties) = transient_scope(app, &unit_suffix(), pid);
    let aux: Vec<(&str, Vec<(&str, zbus::zvariant::Value)>)> = Vec::new();
    zbus::blocking::Connection::session()?.call_method(
        Some("org.freedesktop.systemd1"),
        "/org/freedesktop/systemd1",
        Some("org.freedesktop.systemd1.Manager"),
        "StartTransientUnit",
        &(unit, "fail", properties, aux),
    )?;
    Ok(())
}

/// `StartTransientUnit`'s unit name and properties for [`adopt`]: the scope and slice
/// [`scope_argv`] gives, with `pid` moved in.
fn transient_scope(
    app: &str,
    suffix: &str,
    pid: u32,
) -> (String, Vec<(&'static str, zbus::zvariant::Value<'static>)>) {
    let properties = vec![
        ("PIDs", vec![pid].into()),
        ("Slice", "app.slice".into()),
        ("CollectMode", "inactive-or-failed".into()),
    ];
    (format!("{}.scope", unit_name(app, suffix)), properties)
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

/// Two instances of one app need two scopes, also when two processes launch it.
fn unit_suffix() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{:x}{n:x}", std::process::id())
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
                ("PIDs", zbus::zvariant::Value::from(vec![4242u32])),
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
    fn launch_scopes_get_distinct_suffixes() {
        assert_ne!(unit_suffix(), unit_suffix());
    }

    #[test]
    fn a_command_without_an_app_runs_as_it_is() {
        assert_eq!(
            in_scope(&["foot", "-e", "top"], None),
            ["foot", "-e", "top"]
        );
    }
}
