//! The test binary's own empty HOME and XDG folders, set before `main` so no test reads the
//! user's fonts, icons or caches. Other crates' tests take `reprisebus::test_session!`.

use std::path::PathBuf;

/// The XDG base folder variables and where each lies under HOME.
const HOMES: [(&str, &str); 4] = [
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_STATE_HOME", ".local/state"),
];

/// The process's session folder, removed as it exits.
pub fn dir() -> PathBuf {
    std::env::temp_dir().join(format!("reprisekit-test-{}", std::process::id()))
}

#[used]
#[unsafe(link_section = ".init_array")]
static ENTER: extern "C" fn() = enter;

extern "C" fn enter() {
    extern "C" fn remove() {
        let _ = std::fs::remove_dir_all(dir());
    }
    let home = dir().join("home");
    for (_, rel) in HOMES {
        let _ = std::fs::create_dir_all(home.join(rel));
    }
    // SAFETY: `.init_array` runs `enter` before `main`, so no other thread reads the environment,
    // and `remove` touches nothing torn down before exit handlers.
    unsafe {
        libc::atexit(remove);
        std::env::set_var("HOME", &home);
        for (var, rel) in HOMES {
            std::env::set_var(var, home.join(rel));
        }
        std::env::remove_var("XDG_CONFIG_DIRS");
        std::env::remove_var("XDG_DATA_DIRS");
    }
}
