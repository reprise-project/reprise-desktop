//! XDG autostart at login: `reprisefs::autostart::scan` finds the entries; this
//! spawns the ones that apply to this desktop.

use crate::compositor::State;
use reprisefs::autostart::Entry;

/// Autostart entries for what Reprise does itself: geoclue's demo agent
/// would take the agent's place from the compositor (`location`).
const REPLACED: [&str; 1] = ["geoclue-demo-agent"];

/// The entries that should actually be launched on `desktops`: not hidden,
/// not fenced off by `OnlyShowIn` / `NotShowIn`, not one Reprise replaces,
/// with a `TryExec` that `on_path` finds (an entry without one always
/// passes), and not one a unit of the session starts (`by_unit`).
pub fn runnable(
    entries: Vec<Entry>,
    desktops: &[String],
    on_path: impl Fn(&str) -> bool,
    by_unit: impl Fn(&Entry) -> bool,
) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|e| e.runs_on(desktops))
        .filter(|e| !REPLACED.contains(&e.id.as_str()))
        .filter(|e| e.try_exec.as_deref().map(&on_path).unwrap_or(true))
        .filter(|e| !by_unit(e))
        .collect()
}

fn on_path(program: &str) -> bool {
    reprisefs::autostart::find_program(program).is_some()
}

/// Spawn every applicable autostart entry, then restore the last session. Called once at startup,
/// after `WAYLAND_DISPLAY` is exported, so a child inherits the socket. The scan, the walk of the
/// user manager's session units and the spawns run on a job thread so the first frames don't wait
/// on them; the restore follows on the loop. The walk waits for `session_begun`, so the session
/// target's start jobs are queued and their conditions get checked before it reads them.
pub fn start(state: &mut State, session_begun: std::thread::JoinHandle<()>) {
    let desktops = reprisefs::autostart::desktop_names(Some(&state.env.desktop));
    let terminal = state.config.terminal.clone();
    let env = state.env.clone();
    state.spawn_fs_job(move || {
        let _ = session_begun.join();
        let run = find(&desktops);
        launch(&run, &terminal, &env);
        Box::new(move |state: &mut State| crate::session::restore(state, &run))
    });
}

/// The entries to launch on `desktops`. Blocks on the user manager.
fn find(desktops: &[String]) -> Vec<Entry> {
    let found = reprisefs::autostart::scan(
        &reprisefs::autostart::system_dirs(),
        &reprisefs::autostart::config_home(),
    );
    let units = reprisebus::units::session_programs();
    runnable(found, desktops, on_path, |e| {
        let left =
            reprisefs::autostart::left_to_systemd(e, &units, reprisefs::autostart::find_program);
        if left {
            log::info!("autostart: {} is left to its systemd unit", e.id);
        }
        left
    })
}

fn launch(run: &[Entry], terminal: &str, env: &crate::spawn::SessionEnv) {
    for e in run {
        log::info!("autostart: {}", e.id);
        let argv = e.argv(terminal);
        crate::spawn::spawn_argv_in(&argv, Some(&e.id), e.path.as_deref(), env, None);
    }
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
            runnable(vec![e], &["reprise".to_string()], |_| true, |_| false).len(),
            1
        );
    }

    #[test]
    fn runnable_drops_hidden() {
        let e = entry("[Desktop Entry]\nExec=a\nHidden=true\n");
        assert!(runnable(vec![e], &["reprise".to_string()], |_| true, |_| false).is_empty());
    }

    #[test]
    fn runnable_drops_another_desktops_entry() {
        let only = entry("[Desktop Entry]\nExec=a\nOnlyShowIn=GNOME;\n");
        assert!(runnable(vec![only], &["reprise".to_string()], |_| true, |_| false).is_empty());
        let only_us = entry("[Desktop Entry]\nExec=a\nOnlyShowIn=reprise;\n");
        assert_eq!(
            runnable(vec![only_us], &["reprise".to_string()], |_| true, |_| false).len(),
            1
        );
        let not_us = entry("[Desktop Entry]\nExec=a\nNotShowIn=reprise;\n");
        assert!(runnable(vec![not_us], &["reprise".to_string()], |_| true, |_| false).is_empty());
    }

    #[test]
    fn runnable_drops_the_geoclue_demo_agent() {
        let demo = reprisefs::autostart::parse(
            "geoclue-demo-agent",
            "[Desktop Entry]\nExec=/usr/lib/geoclue-2.0/demos/agent\nNotShowIn=GNOME;\n",
            false,
        )
        .unwrap();
        assert!(runnable(vec![demo], &["reprise".to_string()], |_| true, |_| false).is_empty());
    }

    #[test]
    fn runnable_drops_a_missing_try_exec() {
        let e = entry("[Desktop Entry]\nExec=a\nTryExec=missing\n");
        assert!(
            runnable(
                vec![e.clone()],
                &["reprise".to_string()],
                |_| false,
                |_| false
            )
            .is_empty()
        );
        assert_eq!(
            runnable(
                vec![e],
                &["reprise".to_string()],
                |p| p == "missing",
                |_| false
            )
            .len(),
            1
        );
    }

    #[test]
    fn runnable_drops_an_entry_a_unit_starts() {
        let e = entry("[Desktop Entry]\nExec=a\nX-systemd-skip=true\n");
        let by_unit = |e: &Entry| e.under_systemd;
        assert!(runnable(vec![e], &["reprise".to_string()], |_| true, by_unit).is_empty());
    }
}
