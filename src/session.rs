//! Session restore: the apps open when the session ends start again at the
//! next login, each from its desktop entry, its first window back on the
//! virtual desktop it was on. Where on the screen it opens is the window
//! memory's job (`windowrules::Remembered`).
//!
//! The list is taken when Log Off, Turn Off or Restart starts, before the
//! windows are asked to close (`sessionend`), or when the loop stops for a
//! signal. It lives in the state file and is kept after the login that
//! restores it, so a crashed session gets the last clean one back.

use crate::compositor::State;
use reprisefs::apps::AppEntry;
use reprisefs::autostart::Entry;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// How long after login a restored app's first window still goes to its old
/// desktop; a later window is taken to have been started by hand.
const DESKTOP_WINDOW: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedApp {
    /// Its windows' app_id (for X11, the class).
    pub app_id: String,
    /// The id of the desktop entry that starts it.
    pub entry: String,
    /// The virtual desktop its newest window was on, from 0.
    #[serde(default)]
    pub desktop: usize,
}

fn excluded(exclude: &[String], app_id: &str) -> bool {
    exclude.iter().any(|g| crate::windowrules::glob(g, app_id))
}

/// The apps to save from the `open` windows, as (app_id, desktop) oldest
/// first: one per app, in the order its first window came, with its newest
/// window's desktop. Apps without an entry in `apps`, and excluded ones, are left out.
pub fn pick(open: &[(String, usize)], apps: &[AppEntry], exclude: &[String]) -> Vec<SavedApp> {
    let mut saved: Vec<SavedApp> = Vec::new();
    for (app_id, desktop) in open {
        if let Some(s) = saved.iter_mut().find(|s| s.app_id == *app_id) {
            s.desktop = *desktop;
            continue;
        }
        if excluded(exclude, app_id) {
            continue;
        }
        let Some(e) = reprisefs::apps::entry_for_app_id(apps, app_id, |_| true) else {
            continue;
        };
        saved.push(SavedApp {
            app_id: app_id.clone(),
            entry: e.id.clone(),
            desktop: *desktop,
        });
    }
    saved
}

/// Programs that start another program named further down their command line.
const LAUNCHERS: &[&str] = &["env", "sh", "bash", "flatpak", "snap", "gtk-launch"];

/// The program an `Exec` line runs, without its directory; None for a `LAUNCHERS` one.
fn program(exec: &str) -> Option<String> {
    let argv = reprisefs::apps::exec_argv(exec);
    let first = argv.first()?;
    let name = first.rsplit('/').next().unwrap_or(first);
    (!LAUNCHERS.contains(&name)).then(|| name.to_string())
}

/// The saved apps to start, each with its entry: those still installed and
/// not excluded now. An app autostart has just started, by the same entry
/// or program, is not started twice.
pub fn to_start<'a>(
    saved: &'a [SavedApp],
    apps: &'a [AppEntry],
    exclude: &[String],
    autostarted: &[Entry],
) -> Vec<(&'a SavedApp, &'a AppEntry)> {
    let started: Vec<Option<String>> = autostarted.iter().map(|a| program(&a.exec)).collect();
    saved
        .iter()
        .filter(|s| !excluded(exclude, &s.app_id))
        .filter_map(|s| Some((s, apps.iter().find(|e| e.id == s.entry)?)))
        .filter(|(_, e)| {
            let prog = program(&e.exec_line);
            !autostarted.iter().any(|a| a.id == e.id)
                && !started.iter().any(|p| p.is_some() && *p == prog)
        })
        .collect()
}

/// The desktops the restored apps' first windows go back to, while the login is recent.
#[derive(Debug, Default)]
pub struct Pending {
    desktops: Vec<(String, usize)>,
    until: Option<Instant>,
}

impl Pending {
    pub fn new(apps: &[&SavedApp], now: Instant) -> Pending {
        Pending {
            desktops: apps.iter().map(|a| (a.app_id.clone(), a.desktop)).collect(),
            until: Some(now + DESKTOP_WINDOW),
        }
    }

    /// The desktop the next window of `app_id` goes to, if it is a
    /// restored app's first and the login is recent.
    pub fn desktop(&self, app_id: &str, now: Instant) -> Option<usize> {
        if self.until.is_none_or(|u| now > u) {
            return None;
        }
        self.desktops
            .iter()
            .find(|(a, _)| a == app_id)
            .map(|&(_, d)| d)
    }

    /// `app_id`'s first window has mapped: its next ones open as usual.
    pub fn mapped(&mut self, app_id: &str) {
        self.desktops.retain(|(a, _)| a != app_id);
    }
}

/// `State::session`: the last session's apps (kept in the state file) and
/// the desktops restored apps' first windows go back to.
#[derive(Debug, Default)]
pub struct Session {
    pub apps: Vec<SavedApp>,
    pub restoring: Pending,
    /// `end` already kept this session, so the loop's own `end` at shutdown
    /// must not take the emptied screen over it.
    pub kept: bool,
}

impl Session {
    pub fn load() -> Session {
        Session {
            apps: crate::state::load().session,
            restoring: Pending::default(),
            kept: false,
        }
    }
}

/// Take the open windows' apps as the session to restore at the next login
/// (empty when `[session] restore` is off) and queue the state file's write.
pub fn save(state: &mut State) {
    let saved = if state.config.session.restore {
        let open: Vec<(String, usize)> = state
            .windows
            .iter()
            .filter(|w| w.mapped && w.placeable())
            .map(|w| (w.app(), w.desktop))
            .collect();
        pick(&open, &state.app_index, &state.config.session.exclude)
    } else {
        Vec::new()
    };
    log::info!(
        "session: saved {:?}",
        saved.iter().map(|s| &s.app_id).collect::<Vec<_>>()
    );
    if state.session.apps != saved {
        crate::state::save_session(&saved);
        state.session.apps = saved;
    }
}

/// Keep the session for the next login: the open windows' places and apps,
/// on disk before anything can stop the compositor. Once kept, the loop's
/// stop keeps nothing more.
pub fn end(state: &mut State) {
    if state.session.kept {
        return;
    }
    state.remember_open_windows();
    save(state);
    crate::state::flush();
}

/// Start the apps the last session ended with, after autostart (which started `autostarted`).
pub fn restore(state: &mut State, autostarted: &[Entry]) {
    if !state.config.session.restore {
        return;
    }
    let start = to_start(
        &state.session.apps,
        &state.app_index,
        &state.config.session.exclude,
        autostarted,
    );
    for (s, e) in &start {
        log::info!("session: restoring {} ({})", s.app_id, e.id);
        let argv = reprisefs::apps::launch_argv(e, &state.config.terminal);
        crate::spawn::spawn_argv(&argv, &state.env, None);
    }
    let started: Vec<&SavedApp> = start.iter().map(|(s, _)| *s).collect();
    state.session.restoring = Pending::new(&started, Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, exec: &str, class: Option<&str>) -> AppEntry {
        let class = class
            .map(|c| format!("StartupWMClass={c}\n"))
            .unwrap_or_default();
        reprisefs::apps::parse_desktop(
            &format!("[Desktop Entry]\nType=Application\nName={id}\nExec={exec}\n{class}"),
            &format!("{id}.desktop"),
        )
        .unwrap()
    }

    fn open(v: &[(&str, usize)]) -> Vec<(String, usize)> {
        v.iter().map(|&(a, d)| (a.to_string(), d)).collect()
    }

    fn saved(app_id: &str, entry: &str, desktop: usize) -> SavedApp {
        SavedApp {
            app_id: app_id.into(),
            entry: entry.into(),
            desktop,
        }
    }

    #[test]
    fn pick_keeps_one_per_app_in_first_window_order_with_the_newest_desktop() {
        let apps = [
            app("org.mozilla.firefox", "firefox %u", None),
            app("foot", "foot", None),
            app("code-oss", "code", Some("Code")),
        ];
        let got = pick(
            &open(&[
                ("foot", 0),
                ("org.mozilla.firefox", 1),
                ("foot", 2),
                ("Code", 3),
                ("unknown", 0),
            ]),
            &apps,
            &[],
        );
        assert_eq!(
            got,
            [
                saved("foot", "foot.desktop", 2),
                saved("org.mozilla.firefox", "org.mozilla.firefox.desktop", 1),
                saved("Code", "code-oss.desktop", 3),
            ]
        );
    }

    #[test]
    fn pick_and_to_start_leave_out_excluded_apps() {
        let apps = [app("foot", "foot", None), app("gimp", "gimp", None)];
        let exclude = ["f*".to_string()];
        assert_eq!(
            pick(&open(&[("foot", 0), ("gimp", 0)]), &apps, &exclude),
            [saved("gimp", "gimp.desktop", 0)]
        );
        let list = [
            saved("foot", "foot.desktop", 0),
            saved("gimp", "gimp.desktop", 0),
        ];
        let ids: Vec<&str> = to_start(&list, &apps, &exclude, &[])
            .iter()
            .map(|(s, _)| s.app_id.as_str())
            .collect();
        assert_eq!(ids, ["gimp"]);
    }

    #[test]
    fn to_start_skips_uninstalled_and_autostarted_apps() {
        let apps = [
            app("foot", "foot", None),
            app("org.mozilla.firefox", "/usr/bin/firefox %u", None),
            app("reprise-power", "reprise-power", None),
            app("org.gimp.GIMP", "flatpak run org.gimp.GIMP %U", None),
        ];
        let list = [
            saved("foot", "foot.desktop", 0),
            saved("gone", "gone.desktop", 0),
            saved("org.mozilla.firefox", "org.mozilla.firefox.desktop", 1),
            saved("reprise-power", "reprise-power.desktop", 0),
            saved("org.gimp.GIMP", "org.gimp.GIMP.desktop", 0),
        ];
        let auto = |id: &str, exec: &str| {
            reprisefs::autostart::parse(id, &format!("[Desktop Entry]\nExec={exec}\n"), false)
                .unwrap()
        };
        // The same entry id, and another entry running the same program;
        // a flatpak autostarted is not every flatpak.
        let autostarted = [
            auto("reprise-power.desktop", "reprise-power --tray"),
            auto("fx.desktop", "firefox --headless"),
            auto("chat.desktop", "/usr/bin/flatpak run org.chat.Chat"),
        ];
        let ids: Vec<&str> = to_start(&list, &apps, &[], &autostarted)
            .iter()
            .map(|(s, _)| s.app_id.as_str())
            .collect();
        assert_eq!(ids, ["foot", "org.gimp.GIMP"]);
    }

    #[test]
    fn pending_desktops_answer_once_per_app_and_only_soon_after_login() {
        let a = saved("foot", "foot.desktop", 2);
        let b = saved("gimp", "gimp.desktop", 0);
        let t = Instant::now();
        let mut p = Pending::new(&[&a, &b], t);
        assert_eq!(p.desktop("foot", t), Some(2));
        assert_eq!(p.desktop("gimp", t), Some(0));
        assert_eq!(p.desktop("other", t), None);
        p.mapped("foot");
        assert_eq!(p.desktop("foot", t), None);
        assert_eq!(
            p.desktop("gimp", t + DESKTOP_WINDOW + Duration::from_secs(1)),
            None
        );
        assert_eq!(Pending::default().desktop("gimp", t), None);
    }

    #[test]
    fn saved_apps_round_trip_through_toml() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Wrap {
            session: Vec<SavedApp>,
        }
        let w = Wrap {
            session: vec![saved("foot", "foot.desktop", 2)],
        };
        let text = toml::to_string(&w).unwrap();
        assert_eq!(toml::from_str::<Wrap>(&text).unwrap(), w);
        let old: Wrap =
            toml::from_str("[[session]]\napp_id = \"foot\"\nentry = \"foot.desktop\"\n").unwrap();
        assert_eq!(old.session[0].desktop, 0);
    }
}
