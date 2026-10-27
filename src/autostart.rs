//! XDG autostart at login: `reprisefs::autostart::scan` finds the entries; this
//! spawns the ones that apply to this desktop.

use crate::compositor::State;
use reprisefs::autostart::Entry;
use std::time::Duration;

/// Autostart entries for what Reprise does itself: geoclue's demo agent
/// would take the agent's place from the compositor (`location`).
const REPLACED: [&str; 1] = ["geoclue-demo-agent"];

/// How long after a crash a program whose scope still runs gets to finish exiting before its scope
/// is listed again.
const DYING_GRACE: Duration = Duration::from_secs(3);

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
/// after `WAYLAND_DISPLAY` is exported, so a child inherits the socket. After a crash, an entry
/// whose app scope still has a process a moment later is left running instead of started twice.
/// The scan, the walk of the user manager's session units and the spawns run on a job thread so
/// the first frames don't wait on them; the restore follows on the loop. The walk waits for
/// `session_begun`, so the session target's start jobs are queued and their conditions get
/// checked before it reads them.
pub fn start(state: &mut State, session_begun: std::thread::JoinHandle<()>, after_crash: bool) {
    let desktops = reprisefs::autostart::desktop_names(Some(&state.env.desktop));
    let terminal = state.config.terminal.clone();
    let env = state.env.clone();
    state.spawn_job(move || {
        let _ = session_begun.join();
        let run = find(&desktops);
        launch_left(&run, after_crash, running_scopes, DYING_GRACE, |e| {
            launch(e, &terminal, &env)
        });
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

/// The entries of `run` with no app scope among `running` units.
fn to_launch<'a>(run: &'a [Entry], running: &[String]) -> Vec<&'a Entry> {
    run.iter()
        .filter(|e| {
            let left = running
                .iter()
                .any(|u| reprisefs::scope::is_app_scope(u, &e.id));
            if left {
                log::info!("autostart: {} still runs from before the crash", e.id);
            }
            !left
        })
        .collect()
}

/// `launch` each of `run`. After a crash, an entry with an app scope in `list()` waits `grace` and
/// starts only if its scope is gone from the listing then: a program still exiting does not keep
/// it from starting.
fn launch_left(
    run: &[Entry],
    after_crash: bool,
    list: impl Fn() -> Vec<String>,
    grace: Duration,
    mut launch: impl FnMut(&Entry),
) {
    if !after_crash {
        run.iter().for_each(launch);
        return;
    }
    let first = to_launch(run, &list());
    let waiting: Vec<Entry> = run
        .iter()
        .filter(|e| !first.iter().any(|f| f.id == e.id))
        .cloned()
        .collect();
    first.into_iter().for_each(&mut launch);
    if waiting.is_empty() {
        return;
    }
    std::thread::sleep(grace);
    to_launch(&waiting, &list()).into_iter().for_each(launch);
}

/// The user manager's app scopes that still have a process; none when it does not answer within
/// [`LIST_LIMIT`], so every entry starts.
fn running_scopes() -> Vec<String> {
    let mut cmd = std::process::Command::new("systemctl");
    cmd.args([
        "--user",
        "--no-ask-password",
        "list-units",
        "--state=running",
        "--plain",
        "--no-legend",
    ])
    .arg(reprisefs::scope::PATTERN);
    match output_within(&mut cmd, LIST_LIMIT) {
        Ok(out) => String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .map(String::from)
            .collect(),
        Err(e) => {
            log::warn!("autostart: systemctl list-units: {e}");
            Vec::new()
        }
    }
}

const LIST_LIMIT: Duration = Duration::from_secs(5);

/// `cmd`'s stdout, or Err once it has run `limit` without ending; then it is killed.
fn output_within(cmd: &mut std::process::Command, limit: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut child = (cmd.stdin(std::process::Stdio::null()))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = tx.send(stdout.read_to_end(&mut out).map(|_| out));
    });
    let got = rx.recv_timeout(limit);
    if got.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    match got {
        Ok(read) => read.map_err(|e| e.to_string()),
        Err(_) => Err(format!("no answer in {limit:?}")),
    }
}

fn launch(e: &Entry, terminal: &str, env: &crate::spawn::SessionEnv) {
    log::info!("autostart: {}", e.id);
    let argv = e.argv(terminal);
    crate::spawn::spawn_argv_in(&argv, Some(&e.id), e.path.as_deref(), env, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> Entry {
        reprisefs::autostart::parse("a", text, false).unwrap()
    }

    #[test]
    fn a_listing_that_never_ends_is_killed_at_its_limit() {
        let mut echo = std::process::Command::new("sh");
        echo.args(["-c", "echo app-reprise-a-1.scope running"]);
        assert_eq!(
            output_within(&mut echo, Duration::from_secs(5)).unwrap(),
            b"app-reprise-a-1.scope running\n"
        );
        let started = std::time::Instant::now();
        let mut hang = std::process::Command::new("sleep");
        hang.arg("30");
        assert!(output_within(&mut hang, Duration::from_millis(200)).is_err());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "killed, not waited for"
        );
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
    fn an_entry_whose_scope_still_runs_is_not_started_again() {
        let sync =
            reprisefs::autostart::parse("syncthing.desktop", "[Desktop Entry]\nExec=s\n", false)
                .unwrap();
        let run = [sync, entry("[Desktop Entry]\nExec=a\n")];
        let ids = |l: Vec<&Entry>| l.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(to_launch(&run, &[])), ["syncthing.desktop", "a"]);
        let running = ["app-reprise-syncthing-1ax2.scope".to_string()];
        assert_eq!(ids(to_launch(&run, &running)), ["a"]);
    }

    #[test]
    fn after_a_crash_an_entry_starts_once_its_dying_scope_is_gone() {
        let sync =
            reprisefs::autostart::parse("syncthing.desktop", "[Desktop Entry]\nExec=s\n", false)
                .unwrap();
        let run = [sync, entry("[Desktop Entry]\nExec=a\n")];
        let scope = "app-reprise-syncthing-1ax2.scope".to_string();
        let launched = |after_crash: bool, listings: Vec<Vec<String>>| {
            let listings = std::cell::RefCell::new(listings.into_iter());
            let mut got = Vec::new();
            let list = || {
                listings
                    .borrow_mut()
                    .next()
                    .expect("one listing more than expected")
            };
            launch_left(&run, after_crash, list, Duration::ZERO, |e| {
                got.push(e.id.clone())
            });
            got
        };
        assert_eq!(
            launched(false, vec![]),
            ["syncthing.desktop", "a"],
            "a login lists nothing"
        );
        assert_eq!(
            launched(true, vec![vec![scope.clone()], vec![]]),
            ["a", "syncthing.desktop"],
            "the scope went during the grace"
        );
        assert_eq!(
            launched(true, vec![vec![scope.clone()], vec![scope]]),
            ["a"],
            "the scope still runs"
        );
        assert_eq!(
            launched(true, vec![vec![]]),
            ["syncthing.desktop", "a"],
            "no scope, no second listing"
        );
    }

    #[test]
    fn runnable_drops_an_entry_a_unit_starts() {
        let e = entry("[Desktop Entry]\nExec=a\nX-systemd-skip=true\n");
        let by_unit = |e: &Entry| e.under_systemd;
        assert!(runnable(vec![e], &["reprise".to_string()], |_| true, by_unit).is_empty());
    }
}
