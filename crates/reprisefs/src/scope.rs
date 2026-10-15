//! App scopes: a program started from a desktop entry runs in `app-reprise-<id>-<n>.scope`, where
//! xdg-desktop-portal reads a host app's id. The compositor and the programs both launch this way.

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

/// Prefix that execs a command, same pid, in `app-reprise-<app>-<suffix>.scope`; the id is
/// unit-escaped so its dashes stay apart from the launcher and suffix (systemd's app naming).
fn scope_argv(app: &str, suffix: &str) -> Vec<String> {
    let app = app.strip_suffix(".desktop").unwrap_or(app);
    vec![
        "systemd-run".into(),
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        "--slice=app.slice".into(),
        format!("--unit=app-reprise-{}-{suffix}", unit_escape(app)),
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
