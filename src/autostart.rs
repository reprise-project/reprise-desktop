//! XDG autostart at login: `reprisefs::autostart::scan` finds the entries; this
//! spawns the ones that apply to this desktop.

use crate::compositor::State;
use reprisefs::autostart::Entry;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Autostart entries for what Reprise does itself: geoclue's demo agent
/// would take the agent's place from the compositor (`location`).
const REPLACED: [&str; 1] = ["geoclue-demo-agent"];

/// The entries that should actually be launched on `desktops`: not hidden,
/// not fenced off by `OnlyShowIn` / `NotShowIn`, not one Reprise replaces,
/// and with a `TryExec` that `on_path` finds (an entry without one always
/// passes).
pub fn runnable(
    entries: Vec<Entry>,
    desktops: &[String],
    on_path: impl Fn(&str) -> bool,
) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| e.runs_on(desktops))
        .filter(|e| !REPLACED.contains(&e.id.as_str()))
        .filter(|e| e.try_exec.as_deref().map(&on_path).unwrap_or(true))
        .collect()
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// A `TryExec` value: an absolute path to an executable file, or a bare name
/// that is an executable file in one of `PATH`'s directories.
fn on_path(program: &str) -> bool {
    let p = Path::new(program);
    if p.is_absolute() {
        return is_executable(p);
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| is_executable(&dir.join(program)))
}

/// Spawn every applicable autostart entry and return them. Called once at
/// startup, after `WAYLAND_DISPLAY` is exported, so a child inherits the
/// socket.
pub fn run(state: &State) -> Vec<Entry> {
    let desktops = reprisefs::autostart::desktop_names(Some(&state.env.desktop));
    let found = reprisefs::autostart::scan(
        &reprisefs::autostart::system_dirs(),
        &reprisefs::autostart::config_home(),
    );
    let run = runnable(found, &desktops, on_path);
    for e in &run {
        log::info!("autostart: {}", e.id);
        let argv = e.argv(&state.config.terminal);
        crate::spawn::spawn_argv_in(&argv, Some(&e.id), e.path.as_deref(), &state.env, None);
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> Entry {
        reprisefs::autostart::parse("a", text, false).unwrap()
    }

    #[test]
    fn runnable_keeps_a_plain_entry() {
        let e = entry("[Desktop Entry]\nExec=a\n");
        assert_eq!(
            runnable(vec![e], &["reprise".to_string()], |_| true).len(),
            1
        );
    }

    #[test]
    fn runnable_drops_hidden() {
        let e = entry("[Desktop Entry]\nExec=a\nHidden=true\n");
        assert!(runnable(vec![e], &["reprise".to_string()], |_| true).is_empty());
    }

    #[test]
    fn runnable_drops_another_desktops_entry() {
        let only = entry("[Desktop Entry]\nExec=a\nOnlyShowIn=GNOME;\n");
        assert!(runnable(vec![only], &["reprise".to_string()], |_| true).is_empty());
        let only_us = entry("[Desktop Entry]\nExec=a\nOnlyShowIn=reprise;\n");
        assert_eq!(
            runnable(vec![only_us], &["reprise".to_string()], |_| true).len(),
            1
        );
        let not_us = entry("[Desktop Entry]\nExec=a\nNotShowIn=reprise;\n");
        assert!(runnable(vec![not_us], &["reprise".to_string()], |_| true).is_empty());
    }

    #[test]
    fn runnable_drops_the_geoclue_demo_agent() {
        let demo = reprisefs::autostart::parse(
            "geoclue-demo-agent",
            "[Desktop Entry]\nExec=/usr/lib/geoclue-2.0/demos/agent\nNotShowIn=GNOME;\n",
            false,
        )
        .unwrap();
        assert!(runnable(vec![demo], &["reprise".to_string()], |_| true).is_empty());
    }

    #[test]
    fn runnable_drops_a_missing_try_exec() {
        let e = entry("[Desktop Entry]\nExec=a\nTryExec=missing\n");
        assert!(runnable(vec![e.clone()], &["reprise".to_string()], |_| false).is_empty());
        assert_eq!(
            runnable(vec![e], &["reprise".to_string()], |p| p == "missing").len(),
            1
        );
    }

    #[test]
    fn an_absolute_try_exec_must_be_executable() {
        let dir = std::env::temp_dir().join(format!("reprise-tryexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prog");
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        let path = file.to_str().unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!on_path(path), "a non-executable file fails TryExec");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(on_path(path), "an executable file passes TryExec");
        assert!(!on_path(dir.to_str().unwrap()), "a directory fails TryExec");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
