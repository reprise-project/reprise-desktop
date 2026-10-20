//! Session restore: the apps open when the session ends start again at the
//! next login, each from its desktop entry, its first window back on the
//! virtual desktop it was on. Where on the screen it opens is the window
//! memory's job (`windowrules::Remembered`).
//!
//! The list is taken when Log Off, Turn Off or Restart starts, before the
//! windows are asked to close (`sessionend`), or when the loop stops for a
//! signal. It lives in the state file and is kept after the login that
//! restores it, so a crashed session gets the last clean one back.
//!
//! Tab groups come back too: each saved group's windows, matched by app id
//! (and title, when a window maps with the one it had) as they map, join in their old tab order on the group's desktop and in
//! the group's own frame, and the tab that showed shows again once its
//! window is back. An app starts once;
//! `RESTART_SETTLE` after a window of it maps, it starts again for each empty
//! slot no start in flight will fill, so an app that brings back its own
//! windows gets no extra one.

use crate::compositor::State;
use crate::windowrules::Remembered;
use reprisefs::apps::AppEntry;
use reprisefs::autostart::Entry;
use serde::{Deserialize, Serialize};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use std::time::{Duration, Instant};

/// How long after login a restored app's first window still goes to its old
/// desktop; a later window is taken to have been started by hand.
const DESKTOP_WINDOW: Duration = Duration::from_secs(120);

/// How long after a restored app's first window maps its other saved tab
/// windows have to come back before it is started again for them.
pub const RESTART_SETTLE: Duration = Duration::from_secs(4);

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

/// A tab group the session ended with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedTabs {
    /// Its windows' app ids, in tab order.
    pub apps: Vec<String>,
    /// Its windows' titles, in tab order.
    #[serde(default)]
    pub titles: Vec<String>,
    /// The index in `apps` of the tab that showed.
    pub shown: usize,
    /// The virtual desktop the group was on, from 0.
    pub desktop: usize,
    /// The group's frame, on the output it was on.
    #[serde(default)]
    pub frame: Option<Remembered>,
}

/// The tab groups to save from `groups`: windows of apps not in `saved`
/// (an empty app id for a window not saved) are left out, and so is a group
/// left with fewer than two.
pub fn pick_tabs(groups: &[SavedTabs], saved: &[SavedApp]) -> Vec<SavedTabs> {
    groups
        .iter()
        .filter_map(|g| {
            let apps = &g.apps;
            let keep = |a: &String| saved.iter().any(|s| s.app_id == *a);
            let kept: Vec<String> = apps.iter().filter(|a| keep(a)).cloned().collect();
            let titles = apps.iter().zip(&g.titles);
            let titles = titles.filter(|(a, _)| keep(a)).map(|(_, t)| t.clone());
            let shown = apps
                .get(g.shown)
                .filter(|a| keep(a))
                .map_or(0, |_| apps[..g.shown].iter().filter(|a| keep(a)).count());
            (kept.len() >= 2).then(|| SavedTabs {
                apps: kept,
                titles: titles.collect(),
                shown,
                ..g.clone()
            })
        })
        .collect()
}

/// Programs that start another program named further down their command line.
const LAUNCHERS: &[&str] = &["env", "sh", "bash", "flatpak", "snap", "gtk-launch"];

/// The program an `Exec` line runs, without its directory; None for a `LAUNCHERS` one.
pub(crate) fn program(exec: &str) -> Option<String> {
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

/// What a mapped window does about the saved tab groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Claim<T> {
    /// It fills no saved group's slot.
    None,
    /// It is the first of its group back: it stands alone for now.
    First,
    /// It joins `target`'s group in front of `before` (at the end when
    /// None); `show` is the tab to show then, when it is back.
    Join {
        target: T,
        before: Option<T>,
        show: Option<T>,
    },
}

#[derive(Debug)]
struct Regroup<T> {
    slots: Vec<(String, Option<T>)>,
    /// The slots' saved titles.
    titles: Vec<String>,
    shown: usize,
    desktop: usize,
    frame: Option<Remembered>,
}

/// The saved tab groups waiting for their windows, while the login is recent.
#[derive(Debug)]
pub struct PendingTabs<T> {
    groups: Vec<Regroup<T>>,
    until: Option<Instant>,
    restarts: Vec<Restart>,
}

/// A restored app that may start again for its empty tab slots.
#[derive(Debug)]
struct Restart {
    app: String,
    /// Starts left within the cap of one per saved slot, the first included.
    left: usize,
    /// Starts whose window has not mapped yet.
    in_flight: usize,
    armed: bool,
}

impl<T> Default for PendingTabs<T> {
    fn default() -> Self {
        PendingTabs {
            groups: Vec::new(),
            until: None,
            restarts: Vec::new(),
        }
    }
}

impl<T: Copy + Eq> PendingTabs<T> {
    pub fn new(tabs: &[SavedTabs], now: Instant) -> PendingTabs<T> {
        PendingTabs {
            groups: tabs
                .iter()
                .map(|t| Regroup {
                    slots: t.apps.iter().map(|a| (a.clone(), None)).collect(),
                    titles: t.titles.clone(),
                    shown: t.shown,
                    desktop: t.desktop,
                    frame: t.frame.clone(),
                })
                .collect(),
            until: Some(now + DESKTOP_WINDOW),
            restarts: Vec::new(),
        }
    }

    /// Restore has started `app_id` once: it may start again for empty
    /// slots, at most once per slot it has in all, that first start included.
    pub fn started(&mut self, app_id: &str) {
        let slots = self.free_slots(app_id);
        if slots > 1 {
            self.restarts.push(Restart {
                app: app_id.to_string(),
                left: slots - 1,
                in_flight: 1,
                armed: false,
            });
        }
    }

    fn free_slots(&self, app_id: &str) -> usize {
        let slots = self.groups.iter().flat_map(|r| &r.slots);
        slots.filter(|(a, w)| w.is_none() && a == app_id).count()
    }

    /// A window of `app_id` has mapped (after its `claim`), landing one of
    /// its starts: whether to arm the `RESTART_SETTLE` timer that ends in
    /// `restart`.
    pub fn settle(&mut self, app_id: &str, now: Instant) -> bool {
        let (open, free) = (self.open(now), self.free_slots(app_id));
        let Some(i) = self.restarts.iter().position(|r| r.app == app_id) else {
            return false;
        };
        if !open || free == 0 {
            self.restarts.remove(i);
            return false;
        }
        let r = &mut self.restarts[i];
        r.in_flight = r.in_flight.saturating_sub(1);
        !std::mem::replace(&mut r.armed, true)
    }

    /// `app_id`'s settle has ended: how many more times to start it, one per
    /// empty slot no start in flight will fill, within its cap. The next
    /// settle waits for its next window.
    pub fn restart(&mut self, app_id: &str, now: Instant) -> usize {
        let (open, free) = (self.open(now), self.free_slots(app_id));
        let Some(i) = self
            .restarts
            .iter()
            .position(|r| r.armed && r.app == app_id)
        else {
            return 0;
        };
        let r = &mut self.restarts[i];
        r.armed = false;
        let n = if open {
            free.saturating_sub(r.in_flight).min(r.left)
        } else {
            0
        };
        r.left -= n;
        r.in_flight += n;
        if !open || r.left == 0 {
            self.restarts.remove(i);
        }
        n
    }

    fn open(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now <= u)
    }

    /// The group and slot the next window of `app_id` titled `title`
    /// fills: the first free one saved with that title, else the first free
    /// one of its app.
    fn free_slot(&self, app_id: &str, title: &str) -> Option<(usize, usize)> {
        let free: Vec<(usize, usize)> = self
            .groups
            .iter()
            .enumerate()
            .flat_map(|(g, r)| {
                let slots = r.slots.iter().enumerate();
                let free = slots.filter(|(_, (a, w))| w.is_none() && a == app_id);
                free.map(move |(s, _)| (g, s))
            })
            .collect();
        let titled = |&&(g, s): &&(usize, usize)| {
            !title.is_empty() && self.groups[g].titles.get(s).is_some_and(|t| t == title)
        };
        free.iter().find(titled).or(free.first()).copied()
    }

    /// The desktop the next window of `app_id` titled `title` goes to,
    /// when it is back for a saved group.
    pub fn desktop(&self, app_id: &str, title: &str, now: Instant) -> Option<usize> {
        if !self.open(now) {
            return None;
        }
        self.free_slot(app_id, title)
            .map(|(g, _)| self.groups[g].desktop)
    }

    /// The frame the next window of `app_id` titled `title` opens in, when
    /// it is back for a saved group that kept one.
    pub fn frame(&self, app_id: &str, title: &str, now: Instant) -> Option<&Remembered> {
        if !self.open(now) {
            return None;
        }
        let (g, _) = self.free_slot(app_id, title)?;
        self.groups[g].frame.as_ref()
    }

    /// Window `w` of `app_id` titled `title` has mapped: it takes its
    /// `free_slot`, and joins the members of that group already back.
    pub fn claim(&mut self, app_id: &str, title: &str, w: T, now: Instant) -> Claim<T> {
        if !self.open(now) {
            return Claim::None;
        }
        let Some((g, s)) = self.free_slot(app_id, title) else {
            return Claim::None;
        };
        let r = &mut self.groups[g];
        r.slots[s].1 = Some(w);
        let back = |range: &[(String, Option<T>)]| range.iter().find_map(|(_, m)| *m);
        let target = back(&r.slots[..s]).or_else(|| back(&r.slots[s + 1..]));
        let claim = match target {
            None => Claim::First,
            Some(target) => Claim::Join {
                target,
                before: back(&r.slots[s + 1..]),
                show: r.slots.get(r.shown).and_then(|(_, m)| *m),
            },
        };
        if r.slots.iter().all(|(a, m)| m.is_some() || a.is_empty()) {
            self.groups.remove(g);
        }
        claim
    }

    /// `w` is gone: its slot stays empty for good, and a window that joins
    /// later takes another member as its target.
    pub fn forget(&mut self, w: T) {
        for (a, m) in self.groups.iter_mut().flat_map(|r| &mut r.slots) {
            if *m == Some(w) {
                a.clear();
                *m = None;
            }
        }
    }
}

/// `State::session`: the last session's apps and tab groups (kept in the
/// state file) and the desktops and groups restored windows go back to.
#[derive(Debug, Default)]
pub struct Session {
    pub apps: Vec<SavedApp>,
    pub tabs: Vec<SavedTabs>,
    pub restoring: Pending,
    pub regrouping: PendingTabs<crate::window::WindowId>,
    /// `end` already kept this session, so the loop's own `end` at shutdown
    /// must not take the emptied screen over it.
    pub kept: bool,
}

impl Session {
    pub fn from_persisted(p: &crate::state::Persisted) -> Session {
        Session {
            apps: p.session.clone(),
            tabs: p.session_tabs.clone(),
            ..Session::default()
        }
    }
}

/// Take the open windows' apps and tab groups as the session to restore at
/// the next login (empty when `[session] restore` is off) and queue the
/// state file's write.
pub fn save(state: &mut State) {
    let (saved, tabs) = if state.config.session.restore {
        let saving = |w: &crate::window::Window| w.mapped && w.placeable();
        let open: Vec<(String, usize)> = state
            .windows
            .iter()
            .filter(|w| saving(w))
            .map(|w| (w.app(), w.desktop))
            .collect();
        let saved = pick(&open, &state.app_index, &state.config.session.exclude);
        let groups: Vec<SavedTabs> = state
            .tabs
            .groups
            .iter()
            .filter_map(|g| {
                let shown = state.win(g.shown())?;
                let members = g.members.iter().map(|&m| state.win(m));
                let members: Vec<_> = members.map(|m| m.filter(|m| saving(m))).collect();
                let apps = members
                    .iter()
                    .map(|m| m.map(|m| m.app()).unwrap_or_default());
                let titles = members
                    .iter()
                    .map(|m| m.map(|m| m.title()).unwrap_or_default());
                Some(SavedTabs {
                    apps: apps.collect(),
                    titles: titles.collect(),
                    shown: g.shown,
                    desktop: shown.desktop,
                    frame: state.frame_of(g.shown()),
                })
            })
            .collect();
        let tabs = pick_tabs(&groups, &saved);
        (saved, tabs)
    } else {
        (Vec::new(), Vec::new())
    };
    log::info!(
        "session: saved {:?}, tabs {:?}",
        saved.iter().map(|s| &s.app_id).collect::<Vec<_>>(),
        tabs.iter().map(|t| &t.apps).collect::<Vec<_>>()
    );
    if state.session.apps != saved || state.session.tabs != tabs {
        crate::state::save_session(&saved, &tabs);
        state.session.apps = saved;
        state.session.tabs = tabs;
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
    let now = Instant::now();
    let mut regrouping = PendingTabs::new(&state.session.tabs, now);
    for (s, e) in &start {
        log::info!("session: restoring {} ({})", s.app_id, e.id);
        let argv = reprisefs::apps::launch_argv(e, &state.config.terminal);
        crate::spawn::spawn_argv_in(&argv, Some(&e.id), None, &state.env, None);
        regrouping.started(&s.app_id);
    }
    let started: Vec<&SavedApp> = start.iter().map(|(s, _)| *s).collect();
    state.session.restoring = Pending::new(&started, now);
    state.session.regrouping = regrouping;
}

/// A window of `app_id` has mapped: when restore started it and some of its
/// saved tab slots are empty, start it again for those after `RESTART_SETTLE`.
pub fn settle_restart(state: &mut State, app_id: &str) {
    if !state.session.regrouping.settle(app_id, Instant::now()) {
        return;
    }
    let app = app_id.to_string();
    let timer = Timer::from_duration(RESTART_SETTLE);
    let armed = state.handle.insert_source(timer, move |_, _, state| {
        let n = state.session.regrouping.restart(&app, Instant::now());
        let entry = state.session.apps.iter().find(|s| s.app_id == app);
        let entry = entry.and_then(|s| state.app_index.iter().find(|e| e.id == s.entry));
        let (Some(e), true) = (entry, n > 0) else {
            return TimeoutAction::Drop;
        };
        log::info!(
            "session: {app} left {n} tab slot(s) empty {RESTART_SETTLE:?} after a window, starting it again"
        );
        let argv = reprisefs::apps::launch_argv(e, &state.config.terminal);
        for _ in 0..n {
            crate::spawn::spawn_argv_in(&argv, Some(&e.id), None, &state.env, None);
        }
        TimeoutAction::Drop
    });
    if let Err(e) = armed {
        log::warn!("session: no restart timer for {app_id}: {e}");
    }
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

    fn tabs(apps: &[&str], shown: usize, desktop: usize) -> SavedTabs {
        SavedTabs {
            apps: apps.iter().map(|a| a.to_string()).collect(),
            titles: Vec::new(),
            shown,
            desktop,
            frame: None,
        }
    }

    fn frame(output: &str, x: i32, maximized: bool) -> Remembered {
        Remembered {
            output: output.into(),
            x,
            y: 40,
            width: 640,
            height: 480,
            maximized,
            text_size: 100,
        }
    }

    #[test]
    fn pick_tabs_keeps_saved_apps_in_order_and_drops_groups_of_one() {
        let saved = [
            saved("foot", "foot.desktop", 0),
            saved("gimp", "gimp.desktop", 1),
        ];
        let got = pick_tabs(
            &[
                SavedTabs {
                    frame: Some(frame("DP-1", 10, true)),
                    ..tabs(&["gimp", "unknown", "foot", "foot"], 3, 1)
                },
                tabs(&["foot", "unknown"], 0, 0),
                tabs(&["unknown", "gimp", "foot"], 0, 2),
            ],
            &saved,
        );
        assert_eq!(
            got,
            [
                SavedTabs {
                    frame: Some(frame("DP-1", 10, true)),
                    ..tabs(&["gimp", "foot", "foot"], 2, 1)
                },
                tabs(&["gimp", "foot"], 0, 2),
            ],
            "a left-out shown tab gives the first tab"
        );
    }

    #[test]
    fn an_app_whose_windows_all_come_back_gets_no_second_start() {
        let t = Instant::now();
        let mut p = PendingTabs::<u32>::new(&[tabs(&["firefox", "firefox"], 0, 0)], t);
        p.started("firefox");
        assert_eq!(p.restart("firefox", t), 0, "no window yet: no settle armed");
        let mut p = PendingTabs::<u32>::new(&[tabs(&["firefox", "firefox"], 0, 0)], t);
        p.started("firefox");
        p.claim("firefox", "", 1, t);
        assert!(p.settle("firefox", t));
        p.claim("firefox", "", 2, t);
        assert!(!p.settle("firefox", t), "the slots are full");
        assert_eq!(p.restart("firefox", t + RESTART_SETTLE), 0);
    }

    #[test]
    fn an_app_that_brings_back_one_window_is_started_for_the_rest() {
        let t = Instant::now();
        let later = t + RESTART_SETTLE;
        let saved = [
            tabs(&["foot", "foot", "gimp"], 0, 0),
            tabs(&["foot", "x"], 0, 0),
        ];
        let mut p = PendingTabs::new(&saved, t);
        p.started("foot");
        p.started("gimp");
        p.claim("foot", "", 1, t);
        assert!(p.settle("foot", t));
        assert!(!p.settle("foot", t), "one settle at a time");
        assert_eq!(p.restart("foot", later), 2, "one start per empty slot");
        p.claim("foot", "", 2, later);
        assert!(!p.settle("foot", later), "three saved slots, three starts");
        assert_eq!(p.restart("foot", later + RESTART_SETTLE), 0);
        assert!(!p.settle("x", t), "an app restore did not start");
        p.claim("gimp", "", 3, t);
        assert!(!p.settle("gimp", t), "gimp's one slot is full");
        let late = t + DESKTOP_WINDOW + Duration::from_secs(1);
        let mut p = PendingTabs::new(&saved, t);
        p.started("foot");
        p.claim("foot", "", 1, t);
        assert!(p.settle("foot", t));
        assert_eq!(p.restart("foot", late), 0, "the restore window has ended");
    }

    #[test]
    fn a_start_whose_window_has_not_mapped_is_not_started_again() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[tabs(&["lo", "lo", "lo", "lo"], 0, 0)], t);
        p.started("lo");
        p.claim("lo", "", 1, t);
        assert!(p.settle("lo", t));
        p.claim("lo", "", 2, t);
        assert_eq!(p.restart("lo", t + RESTART_SETTLE), 2);
        assert_eq!(
            p.restart("lo", t + RESTART_SETTLE * 2),
            0,
            "the next settle waits for a window"
        );
        p.claim("lo", "", 3, t + RESTART_SETTLE * 3);
        assert!(p.settle("lo", t + RESTART_SETTLE * 3));
        assert_eq!(
            p.restart("lo", t + RESTART_SETTLE * 4),
            0,
            "one empty slot, one start still in flight"
        );
    }

    #[test]
    fn windows_regroup_in_tab_order_as_they_come_back() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[tabs(&["a", "b", "c"], 1, 2), tabs(&["b", "d"], 0, 0)], t);
        assert_eq!(p.desktop("c", "", t), Some(2));
        assert_eq!(p.desktop("d", "", t), Some(0));
        assert_eq!(p.claim("x", "", 9, t), Claim::None);
        assert_eq!(p.claim("c", "", 3, t), Claim::First);
        assert_eq!(
            p.claim("a", "", 1, t),
            Claim::Join {
                target: 3,
                before: Some(3),
                show: None
            },
            "before the later tab; the shown one is not back yet"
        );
        assert_eq!(
            p.claim("b", "", 2, t),
            Claim::Join {
                target: 1,
                before: Some(3),
                show: Some(2)
            }
        );
        assert_eq!(
            p.desktop("b", "", t),
            Some(0),
            "the next b is the second group's"
        );
        assert_eq!(p.claim("b", "", 4, t), Claim::First);
        assert_eq!(p.desktop("a", "", t), None, "the first group is complete");
        assert_eq!(p.claim("a", "", 5, t), Claim::None);
    }

    #[test]
    fn a_window_back_with_its_saved_title_takes_that_slot() {
        let t = Instant::now();
        let titled = |titles: &[&str], frame_x| SavedTabs {
            titles: titles.iter().map(|s| s.to_string()).collect(),
            frame: Some(frame("DP-1", frame_x, false)),
            ..tabs(&["ed", "ed"], 0, 0)
        };
        let saved = [
            titled(&["a.txt", "b.txt"], 10),
            titled(&["c.txt", "d.txt"], 300),
        ];
        let mut p = PendingTabs::new(&saved, t);
        assert_eq!(p.frame("ed", "d.txt", t).map(|f| f.x), Some(300));
        assert_eq!(p.claim("ed", "d.txt", 4, t), Claim::First);
        assert_eq!(
            p.claim("ed", "Editor", 1, t),
            Claim::First,
            "an unknown title takes the first free slot"
        );
        assert_eq!(
            p.claim("ed", "c.txt", 3, t),
            Claim::Join {
                target: 4,
                before: Some(4),
                show: Some(3)
            },
            "c.txt joins d.txt, though the first group still has a free slot"
        );
        assert_eq!(
            p.claim("ed", "", 2, t),
            Claim::Join {
                target: 1,
                before: None,
                show: Some(1)
            }
        );
    }

    #[test]
    fn pick_tabs_keeps_the_titles_of_the_windows_it_keeps() {
        let saved = [saved("ed", "ed.desktop", 0)];
        let group = SavedTabs {
            titles: vec!["a".into(), "x".into(), "b".into()],
            ..tabs(&["ed", "unknown", "ed"], 0, 0)
        };
        assert_eq!(
            pick_tabs(&[group], &saved)[0].titles,
            ["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn a_closed_member_leaves_its_slot_empty_and_the_rest_still_join() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[tabs(&["a", "b", "c"], 0, 0)], t);
        assert_eq!(p.claim("a", "", 1, t), Claim::First);
        p.forget(1);
        assert_eq!(p.claim("b", "", 2, t), Claim::First);
        assert_eq!(p.claim("a", "", 7, t), Claim::None, "a's slot is gone");
        assert_eq!(
            p.claim("c", "", 3, t),
            Claim::Join {
                target: 2,
                before: None,
                show: None
            }
        );
        assert_eq!(p.claim("c", "", 4, t), Claim::None, "the group is done");
    }

    #[test]
    fn each_group_opens_in_its_own_frame() {
        let t = Instant::now();
        let saved = [
            SavedTabs {
                frame: Some(frame("DP-1", 10, false)),
                ..tabs(&["foot", "foot"], 0, 0)
            },
            SavedTabs {
                frame: Some(frame("HDMI-A-1", 300, true)),
                ..tabs(&["foot", "foot"], 1, 1)
            },
            tabs(&["gimp", "foot"], 0, 0),
        ];
        let mut p = PendingTabs::new(&saved, t);
        assert_eq!(p.frame("foot", "", t), Some(&frame("DP-1", 10, false)));
        p.claim("foot", "", 1, t);
        p.claim("foot", "", 2, t);
        assert_eq!(
            p.frame("foot", "", t),
            Some(&frame("HDMI-A-1", 300, true)),
            "the first group is full: the next foot opens in the second's frame"
        );
        p.claim("foot", "", 3, t);
        p.claim("foot", "", 4, t);
        assert_eq!(
            p.frame("foot", "", t),
            None,
            "a group saved without a frame"
        );
        assert_eq!(p.frame("gimp", "", t), None);
        let late = t + DESKTOP_WINDOW + Duration::from_secs(1);
        let p = PendingTabs::<u32>::new(&saved, t);
        assert_eq!(p.frame("foot", "", late), None);
    }

    #[test]
    fn saved_tabs_without_a_frame_still_load() {
        #[derive(Deserialize)]
        struct Wrap {
            session_tabs: Vec<SavedTabs>,
        }
        let old: Wrap = toml::from_str(
            "[[session_tabs]]\napps = [\"foot\", \"foot\"]\nshown = 1\ndesktop = 0\n",
        )
        .unwrap();
        assert_eq!(old.session_tabs, [tabs(&["foot", "foot"], 1, 0)]);
    }

    #[test]
    fn regrouping_ends_with_the_restore_window() {
        let t = Instant::now();
        let late = t + DESKTOP_WINDOW + Duration::from_secs(1);
        let mut p = PendingTabs::new(&[tabs(&["a", "b"], 0, 1)], t);
        assert_eq!(p.claim("a", "", 1, t), Claim::First);
        assert_eq!(p.desktop("b", "", late), None);
        assert_eq!(p.claim("b", "", 2, late), Claim::None);
        assert_eq!(
            PendingTabs::<u32>::default().claim("a", "", 1, t),
            Claim::None
        );
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
