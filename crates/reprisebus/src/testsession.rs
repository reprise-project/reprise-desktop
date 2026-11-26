//! A test binary's own session: `#[cfg(test)] reprisebus::test_session!();` at its crate root
//! points HOME, the XDG folders, the runtime folder and TMPDIR at empty folders of the process's
//! own and drops the buses and displays, before `main`, so no test reads or writes the user's
//! session, and whatever a test leaves in the temp folder goes with the session folder.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Makes the crate's test binary [`enter`] its own session before `main`.
#[macro_export]
macro_rules! test_session {
    () => {
        #[used]
        #[unsafe(link_section = ".init_array")]
        static TEST_SESSION: extern "C" fn() = {
            extern "C" fn enter() {
                $crate::testsession::enter();
            }
            enter
        };
    };
}

/// The XDG base folder variables and where each lies under HOME.
const HOMES: [(&str, &str); 4] = [
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_STATE_HOME", ".local/state"),
];

/// Variables that lead to the user's own files, buses or displays.
const DROPPED: [&str; 9] = [
    "REPRISE_CONFIG",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_DIRS",
    "DBUS_SESSION_BUS_ADDRESS",
    "AT_SPI_BUS_ADDRESS",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "DISPLAY",
    "XAUTHORITY",
];

/// The process's session folder, `reprise-tests/<pid>` in the temp folder the process started
/// with: removed as the process exits, or by a later test process once that one is gone.
pub fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let parent = std::env::temp_dir().join("reprise-tests");
        sweep(&parent);
        let dir = parent.join(std::process::id().to_string());
        remove_tree(&dir);
        dir
    })
    .clone()
}

/// Removes `path` after making every folder under it listable, so a test that left one at
/// mode 0o300 does not leave the session behind.
fn remove_tree(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fn open_up(dir: &Path) {
        let Ok(meta) = std::fs::symlink_metadata(dir) else {
            return;
        };
        if !meta.is_dir() {
            return;
        }
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            open_up(&entry.path());
        }
    }
    open_up(path);
    let _ = std::fs::remove_dir_all(path);
}

/// Removes the folders under `parent` named for a process that no longer runs.
fn sweep(parent: &Path) {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<libc::pid_t>().ok()) else {
            continue;
        };
        // SAFETY: signal 0 sends nothing; it only asks whether the process exists.
        let gone = pid > 0
            && unsafe { libc::kill(pid, 0) } == -1
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        if gone {
            remove_tree(&entry.path());
        }
    }
}

/// Sets up the session [`test_session`] describes. Only before `main`: it writes the environment.
#[doc(hidden)]
pub fn enter() {
    extern "C" fn remove() {
        remove_tree(&dir());
    }
    let home = dir().join("home");
    let runtime = dir().join("runtime");
    let tmp = dir().join("tmp");
    for (_, rel) in HOMES {
        let _ = std::fs::create_dir_all(home.join(rel));
    }
    let _ = std::fs::create_dir_all(&runtime);
    let _ = std::fs::create_dir_all(&tmp);
    // SAFETY: `.init_array` runs `enter` before `main`, so no other thread reads the environment,
    // and `remove` touches nothing torn down before exit handlers.
    unsafe {
        libc::atexit(remove);
        std::env::set_var("HOME", &home);
        for (var, rel) in HOMES {
            std::env::set_var(var, home.join(rel));
        }
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
        std::env::set_var("TMPDIR", &tmp);
        let no_bus = format!("unix:path={}", runtime.join("no-system-bus").display());
        std::env::set_var("DBUS_SYSTEM_BUS_ADDRESS", no_bus);
        for var in DROPPED {
            std::env::remove_var(var);
        }
    }
}

#[cfg(test)]
crate::test_session!();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_binary_runs_in_empty_folders_of_its_own_with_no_bus_or_display() {
        let home = dir().join("home");
        assert_eq!(std::env::var_os("HOME"), Some(home.clone().into()));
        assert_eq!(
            std::env::var_os("XDG_CONFIG_HOME"),
            Some(home.join(".config").into())
        );
        assert_eq!(
            std::env::var_os("XDG_RUNTIME_DIR"),
            Some(dir().join("runtime").into())
        );
        assert_eq!(std::env::temp_dir(), dir().join("tmp"));
        for var in DROPPED {
            assert_eq!(std::env::var_os(var), None, "{var} is unset");
        }
        assert!(
            zbus::block_on(zbus::Connection::system()).is_err(),
            "no system bus"
        );
        assert!(
            zbus::block_on(zbus::Connection::session()).is_err(),
            "no session bus"
        );
    }

    #[test]
    fn a_sweep_removes_only_the_folders_of_processes_that_exited() {
        let parent = tempfile::tempdir().unwrap();
        let mut exited = std::process::Command::new("true").spawn().unwrap();
        exited.wait().unwrap();
        let names = [
            exited.id().to_string(),
            std::process::id().to_string(),
            "x".into(),
        ];
        for name in &names {
            std::fs::create_dir(parent.path().join(name)).unwrap();
        }
        sweep(parent.path());
        let kept: Vec<bool> = names
            .iter()
            .map(|n| parent.path().join(n).exists())
            .collect();
        assert_eq!(
            kept,
            [false, true, true],
            "only the exited process's folder goes"
        );
    }

    #[test]
    fn the_sweep_removes_a_dead_process_folder_holding_an_unlistable_one() {
        use std::os::unix::fs::PermissionsExt;
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let parent = dir().join("tmp").join("sweep-test");
        let shut = parent.join(pid.to_string()).join("shut");
        std::fs::create_dir_all(&shut).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o300)).unwrap();
        sweep(&parent);
        assert!(!parent.join(pid.to_string()).exists());
        let _ = std::fs::remove_dir_all(&parent);
    }
}
