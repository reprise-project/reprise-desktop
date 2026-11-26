//! The test binary's own empty HOME, XDG folders and TMPDIR, set before `main` so no test reads
//! the user's fonts, icons or caches and whatever a test leaves in the temp folder goes with the
//! session folder. Other crates' tests take `reprisebus::test_session!`, which this mirrors.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The XDG base folder variables and where each lies under HOME.
const HOMES: [(&str, &str); 4] = [
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_STATE_HOME", ".local/state"),
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

#[used]
#[unsafe(link_section = ".init_array")]
static ENTER: extern "C" fn() = enter;

extern "C" fn enter() {
    extern "C" fn remove() {
        remove_tree(&dir());
    }
    let home = dir().join("home");
    let tmp = dir().join("tmp");
    for (_, rel) in HOMES {
        let _ = std::fs::create_dir_all(home.join(rel));
    }
    let _ = std::fs::create_dir_all(&tmp);
    // SAFETY: `.init_array` runs `enter` before `main`, so no other thread reads the environment,
    // and `remove` touches nothing torn down before exit handlers.
    unsafe {
        libc::atexit(remove);
        std::env::set_var("HOME", &home);
        for (var, rel) in HOMES {
            std::env::set_var(var, home.join(rel));
        }
        std::env::set_var("TMPDIR", &tmp);
        std::env::remove_var("XDG_CONFIG_DIRS");
        std::env::remove_var("XDG_DATA_DIRS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
