//! A test binary's own session: `#[cfg(test)] reprisebus::test_session!();` at its crate root
//! points HOME, the XDG folders and the runtime folder at empty folders of the process's own and
//! drops the buses and displays, before `main`, so no test reads or writes the user's session.

use std::path::PathBuf;

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
const DROPPED: [&str; 7] = [
    "XDG_CONFIG_DIRS",
    "XDG_DATA_DIRS",
    "DBUS_SESSION_BUS_ADDRESS",
    "AT_SPI_BUS_ADDRESS",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "DISPLAY",
];

/// The process's session folder, removed as it exits.
pub fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("reprise-session-{}", std::process::id()))
}

/// Sets up the session [`test_session`] describes. Only before `main`: it writes the environment.
#[doc(hidden)]
pub fn enter() {
    extern "C" fn remove() {
        let _ = std::fs::remove_dir_all(dir());
    }
    let home = dir().join("home");
    let runtime = dir().join("runtime");
    for (_, rel) in HOMES {
        let _ = std::fs::create_dir_all(home.join(rel));
    }
    let _ = std::fs::create_dir_all(&runtime);
    // SAFETY: `.init_array` runs `enter` before `main`, so no other thread reads the environment,
    // and `remove` touches nothing torn down before exit handlers.
    unsafe {
        libc::atexit(remove);
        std::env::set_var("HOME", &home);
        for (var, rel) in HOMES {
            std::env::set_var(var, home.join(rel));
        }
        std::env::set_var("XDG_RUNTIME_DIR", &runtime);
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
}
