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
        let _ = std::fs::remove_dir_all(&dir);
        dir
    })
    .clone()
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
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[used]
#[unsafe(link_section = ".init_array")]
static ENTER: extern "C" fn() = enter;

extern "C" fn enter() {
    extern "C" fn remove() {
        let _ = std::fs::remove_dir_all(dir());
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
