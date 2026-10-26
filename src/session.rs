//! Session restore: the apps open when the session ends start again at the
//! next login or the restart after a crash (`crash`), each from its desktop entry, its first window back on the
//! virtual desktop it was on. Where on the screen it opens is the window
//! memory's job (`windowrules::Remembered`).
//!
//! The list is taken when Log Off, Turn Off or Restart starts, before the
//! windows are asked to close (`sessionend`), when the loop stops for a
//! signal, and every `SAVE_EVERY` while the session runs, so a crash leaves
//! a fresh one. It lives in the state file.
//!
//! Tab groups come back too: each saved group's windows, matched by app id
//! (and title, when a window maps with the one it had) as they map, join in their old tab order on the group's desktop and in
//! the group's own frame, and the tab that showed shows again once its
//! window is back. A window that took a slot by map order alone moves to the
//! slot saved with the title it takes in `GUESS_SETTLE`, or out of the
//! group when that title was one of its app's windows outside any group. An app starts once;
//! `RESTART_SETTLE` after a window of it maps, it starts again for each empty
//! slot or missing outside window no start in flight will fill, so an app that
//! brings back its own windows gets no extra one.

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

/// How long after a window takes a tab slot by map order alone a title it
/// takes can still move it.
const GUESS_SETTLE: Duration = Duration::from_secs(10);

/// How often the running session's list is taken again, written only when it changed.
pub const SAVE_EVERY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedApp {
    /// Its windows' app_id (for X11, the class).
    pub app_id: String,
    /// The id of the desktop entry that starts it.
    pub entry: String,
    /// The virtual desktop its newest window was on, from 0.
    #[serde(default)]
    pub desktop: usize,
    /// The titles of its windows in no saved tab group, when it has one in a group.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loose: Vec<String>,
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
            loose: Vec::new(),
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

/// Give each app in `saved` with a window in `tabs` the titles of its `open` windows
/// (app id, title) outside them.
pub fn note_loose(saved: &mut [SavedApp], tabs: &[SavedTabs], open: &[(String, String)]) {
    for s in saved {
        if !tabs.iter().any(|t| t.apps.contains(&s.app_id)) {
            continue;
        }
        let titles = open.iter().filter(|(a, _)| *a == s.app_id);
        s.loose = titles.map(|(_, t)| t.clone()).collect();
    }
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

    /// Whether a restored app's first window is still awaited at `now`.
    pub fn waiting(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now <= u) && !self.desktops.is_empty()
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
struct Slot<T> {
    /// The app id it waits for; empty once its window closed.
    app: String,
    /// The saved title.
    title: String,
    window: Option<T>,
    /// When `window` took it by map order alone, while its title may still move it.
    guessed: Option<Instant>,
}

#[derive(Debug)]
struct Regroup<T> {
    slots: Vec<Slot<T>>,
    shown: usize,
    desktop: usize,
    frame: Option<Remembered>,
}

/// The saved tab groups waiting for their windows, while the login is recent.
#[derive(Debug)]
pub struct PendingTabs<T> {
    groups: Vec<Regroup<T>>,
    /// Restored apps' windows outside any group, as (app id, title).
    loose: Vec<(String, String)>,
    /// Windows a claim or retitle moved besides its own, for `moved`.
    bumped: Vec<T>,
    until: Option<Instant>,
    restarts: Vec<Restart>,
}

/// A restored app that may start again for its empty tab slots and missing loose windows.
#[derive(Debug)]
struct Restart {
    app: String,
    /// Starts left within the cap of one per saved window, the first included.
    left: usize,
    /// Its saved loose windows not back yet.
    loose: usize,
    /// Starts whose window has not mapped yet.
    in_flight: usize,
    armed: bool,
}

impl<T> Default for PendingTabs<T> {
    fn default() -> Self {
        PendingTabs {
            groups: Vec::new(),
            loose: Vec::new(),
            bumped: Vec::new(),
            until: None,
            restarts: Vec::new(),
        }
    }
}

impl<T: Copy + Eq> PendingTabs<T> {
    pub fn new(tabs: &[SavedTabs], apps: &[SavedApp], now: Instant) -> PendingTabs<T> {
        let slot = |(i, a): (usize, &String), t: &SavedTabs| Slot {
            app: a.clone(),
            title: t.titles.get(i).cloned().unwrap_or_default(),
            window: None,
            guessed: None,
        };
        let loose = apps
            .iter()
            .flat_map(|a| a.loose.iter().map(|t| (a.app_id.clone(), t.clone())));
        PendingTabs {
            groups: tabs
                .iter()
                .map(|t| Regroup {
                    slots: t.apps.iter().enumerate().map(|a| slot(a, t)).collect(),
                    shown: t.shown,
                    desktop: t.desktop,
                    frame: t.frame.clone(),
                })
                .collect(),
            loose: loose.collect(),
            bumped: Vec::new(),
            until: Some(now + DESKTOP_WINDOW),
            restarts: Vec::new(),
        }
    }

    /// Restore has started `app_id` once: it may start again for empty
    /// slots and missing loose windows, at most once per window it had, that
    /// first start included.
    pub fn started(&mut self, app_id: &str) {
        let loose = self.loose.iter().filter(|(a, _)| a == app_id).count();
        let windows = self.free_slots(app_id) + loose;
        if windows > 1 {
            self.restarts.push(Restart {
                app: app_id.to_string(),
                left: windows - 1,
                loose,
                in_flight: 1,
                armed: false,
            });
        }
    }

    fn free_slots(&self, app_id: &str) -> usize {
        let slots = self.groups.iter().flat_map(|r| &r.slots);
        slots
            .filter(|s| s.window.is_none() && s.app == app_id)
            .count()
    }

    /// `app_id`'s windows still missing: its empty slots and loose windows not back.
    fn missing(&self, app_id: &str) -> usize {
        let loose = self.restarts.iter().find(|r| r.app == app_id);
        self.free_slots(app_id) + loose.map_or(0, |r| r.loose)
    }

    /// A window of `app_id` came back outside every group.
    fn loose_back(&mut self, app_id: &str) {
        if let Some(r) = self.restarts.iter_mut().find(|r| r.app == app_id) {
            r.loose = r.loose.saturating_sub(1);
        }
    }

    /// A window of `app_id` has mapped (after its `claim`), landing one of
    /// its starts: whether to arm the `RESTART_SETTLE` timer that ends in
    /// `restart`.
    pub fn settle(&mut self, app_id: &str, now: Instant) -> bool {
        let (open, missing) = (self.open(now), self.missing(app_id));
        let Some(i) = self.restarts.iter().position(|r| r.app == app_id) else {
            return false;
        };
        if !open || missing == 0 {
            self.restarts.remove(i);
            return false;
        }
        let r = &mut self.restarts[i];
        r.in_flight = r.in_flight.saturating_sub(1);
        !std::mem::replace(&mut r.armed, true)
    }

    /// `app_id`'s settle has ended: how many more times to start it, one per
    /// missing window no start in flight will fill, within its cap. The next
    /// settle waits for its next window.
    pub fn restart(&mut self, app_id: &str, now: Instant) -> usize {
        let (open, missing) = (self.open(now), self.missing(app_id));
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
            missing.saturating_sub(r.in_flight).min(r.left)
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

    /// The group and slot the next window of `app_id` titled `title` takes
    /// at `now`: one saved with that title, free or held by a guess; else,
    /// unless the title is one of the app's loose windows', its first free one.
    fn slot_for(&self, app_id: &str, title: &str, now: Instant) -> Option<(usize, usize)> {
        self.titled(app_id, title, now, None).or_else(|| {
            let loose = self.is_loose(app_id, title);
            self.slots(app_id)
                .find(|&(g, s)| !loose && self.groups[g].slots[s].window.is_none())
        })
    }

    fn slots(&self, app_id: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
        let app = app_id.to_string();
        self.groups.iter().enumerate().flat_map(move |(g, r)| {
            let app = app.clone();
            let mine = r
                .slots
                .iter()
                .enumerate()
                .filter(move |(_, s)| s.app == app);
            mine.map(move |(s, _)| (g, s))
        })
    }

    /// The slot of `app_id` saved with `title`, other than `but`: a free one
    /// first, else one a window holds by a guess still settling at `now`.
    fn titled(
        &self,
        app_id: &str,
        title: &str,
        now: Instant,
        but: Option<(usize, usize)>,
    ) -> Option<(usize, usize)> {
        if title.is_empty() {
            return None;
        }
        let at = |&(g, s): &(usize, usize)| &self.groups[g].slots[s];
        let named: Vec<(usize, usize)> = self
            .slots(app_id)
            .filter(|gs| Some(*gs) != but && at(gs).title == title)
            .collect();
        let free = named.iter().find(|gs| at(gs).window.is_none());
        free.or_else(|| named.iter().find(|gs| settling(at(gs).guessed, now)))
            .copied()
    }

    fn is_loose(&self, app_id: &str, title: &str) -> bool {
        !title.is_empty() && self.loose.iter().any(|(a, t)| a == app_id && t == title)
    }

    fn take_loose(&mut self, app_id: &str, title: &str) -> bool {
        let i = (self.loose.iter()).position(|(a, t)| a == app_id && t == title);
        i.map(|i| self.loose.remove(i)).is_some()
    }

    fn holding(&self, w: T) -> Option<(usize, usize)> {
        self.groups.iter().enumerate().find_map(|(g, r)| {
            let s = r.slots.iter().position(|s| s.window == Some(w))?;
            Some((g, s))
        })
    }

    /// Put `w` into slot `(g, s)`, out of any it held. A window guessed into
    /// that slot moves to its app's first free slot, or out of every one, and
    /// waits in `bumped`.
    fn put(&mut self, (g, s): (usize, usize), w: T, guessed: Option<Instant>) {
        if let Some(was) = self.holding(w) {
            self.groups[was.0].slots[was.1].window = None;
        }
        let slot = &mut self.groups[g].slots[s];
        let out = slot.window.replace(w).filter(|&o| o != w);
        let out_guessed = std::mem::replace(&mut slot.guessed, guessed);
        let Some(o) = out else {
            return;
        };
        let app = slot.app.clone();
        let free = self
            .slots(&app)
            .find(|&(g, s)| self.groups[g].slots[s].window.is_none());
        if let Some((g, s)) = free {
            let slot = &mut self.groups[g].slots[s];
            slot.window = Some(o);
            slot.guessed = out_guessed;
        }
        self.bumped.push(o);
    }

    /// Where the window in slot `(g, s)` goes among the members of its group back.
    fn joining(&self, (g, s): (usize, usize)) -> Claim<T> {
        let r = &self.groups[g];
        let back = |range: &[Slot<T>]| range.iter().find_map(|s| s.window);
        let target = back(&r.slots[..s]).or_else(|| back(&r.slots[s + 1..]));
        match target {
            None => Claim::First,
            Some(target) => Claim::Join {
                target,
                before: back(&r.slots[s + 1..]),
                show: r.slots.get(r.shown).and_then(|s| s.window),
            },
        }
    }

    /// The desktop the next window of `app_id` titled `title` goes to,
    /// when it is back for a saved group.
    pub fn desktop(&self, app_id: &str, title: &str, now: Instant) -> Option<usize> {
        if !self.open(now) {
            return None;
        }
        self.slot_for(app_id, title, now)
            .map(|(g, _)| self.groups[g].desktop)
    }

    /// The frame the next window of `app_id` titled `title` opens in, when
    /// it is back for a saved group that kept one.
    pub fn frame(&self, app_id: &str, title: &str, now: Instant) -> Option<&Remembered> {
        if !self.open(now) {
            return None;
        }
        let (g, _) = self.slot_for(app_id, title, now)?;
        self.groups[g].frame.as_ref()
    }

    /// Window `w` of `app_id` titled `title` has mapped: it takes its
    /// `slot_for`, and joins the members of that group already back. A
    /// window it takes the slot from is in `moved`.
    pub fn claim(&mut self, app_id: &str, title: &str, w: T, now: Instant) -> Claim<T> {
        if !self.open(now) {
            return Claim::None;
        }
        let Some(gs) = self.slot_for(app_id, title, now) else {
            self.take_loose(app_id, title);
            self.loose_back(app_id);
            return Claim::None;
        };
        let by_title = self.groups[gs.0].slots[gs.1].title == title && !title.is_empty();
        self.put(gs, w, (!by_title).then_some(now));
        self.joining(gs)
    }

    /// Window `w` of `app_id`, in a slot by a guess still settling, is titled
    /// `title` now: it moves to the slot saved with that title, or out of its
    /// group when that is a loose window's title. Whether it moved; the
    /// windows to place again are in `moved`.
    pub fn retitle(&mut self, app_id: &str, title: &str, w: T, now: Instant) -> bool {
        let Some(gs) = self.holding(w).filter(|_| self.open(now)) else {
            return false;
        };
        let slot = &mut self.groups[gs.0].slots[gs.1];
        if !settling(slot.guessed, now) {
            return false;
        }
        if slot.title == title {
            slot.guessed = None;
            return false;
        }
        if let Some(to) = self.titled(app_id, title, now, Some(gs)) {
            self.put(to, w, None);
        } else if self.take_loose(app_id, title) {
            let slot = &mut self.groups[gs.0].slots[gs.1];
            slot.window = None;
            slot.guessed = None;
            self.loose_back(app_id);
        } else {
            return false;
        }
        self.bumped.retain(|&o| o != w);
        self.bumped.insert(0, w);
        true
    }

    /// The windows a claim or retitle moved, to place again with `placing`.
    pub fn moved(&mut self) -> Vec<T> {
        std::mem::take(&mut self.bumped)
    }

    /// Where moved window `w` goes now: `Claim::None` when it holds no slot.
    pub fn placing(&self, w: T) -> Claim<T> {
        self.holding(w).map_or(Claim::None, |gs| self.joining(gs))
    }

    /// `w` is gone: its slot stays empty for good, and a window that joins
    /// later takes another member as its target.
    pub fn forget(&mut self, w: T) {
        for slot in self.groups.iter_mut().flat_map(|r| &mut r.slots) {
            if slot.window == Some(w) {
                slot.app.clear();
                slot.window = None;
                slot.guessed = None;
            }
        }
    }
}

/// Whether a slot guessed at `at` may still move at `now`.
fn settling(at: Option<Instant>, now: Instant) -> bool {
    at.is_some_and(|t| now <= t + GUESS_SETTLE)
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
    /// The state file's `screen_locked` at start.
    pub was_locked: bool,
}

impl Session {
    pub fn from_persisted(p: &crate::state::Persisted) -> Session {
        Session {
            apps: p.session.clone(),
            tabs: p.session_tabs.clone(),
            was_locked: p.screen_locked,
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
        let loose: Vec<(String, String)> = (state.windows.iter())
            .filter(|w| saving(w) && state.tabs.groups.group_of(w.id).is_none())
            .map(|w| (w.app(), w.title()))
            .collect();
        let mut saved = saved;
        note_loose(&mut saved, &tabs, &loose);
        (saved, tabs)
    } else {
        (Vec::new(), Vec::new())
    };
    if state.session.apps != saved || state.session.tabs != tabs {
        log::info!(
            "session: saved {:?}, tabs {:?}",
            saved.iter().map(|s| &s.app_id).collect::<Vec<_>>(),
            tabs.iter().map(|t| &t.apps).collect::<Vec<_>>()
        );
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

/// Take the list again every `every` while the session runs. A tick passes while restored apps'
/// first windows are still awaited, which would drop them, and once `end` kept the session.
pub fn save_every(state: &mut State, every: Duration) {
    let armed = state
        .handle
        .insert_source(Timer::from_duration(every), move |_, _, state| {
            if !state.session.kept && !state.session.restoring.waiting(Instant::now()) {
                save(state);
            }
            TimeoutAction::ToDuration(every)
        });
    if let Err(e) = armed {
        log::warn!("session: no save timer: {e}; the list is taken at the session's end only");
    }
}

/// Start the apps the last session ended with, after autostart (which started `autostarted`),
/// then keep the list fresh (`save_every`).
pub fn restore(state: &mut State, autostarted: &[Entry]) {
    restore_now(state, autostarted);
    save_every(state, SAVE_EVERY);
}

fn restore_now(state: &mut State, autostarted: &[Entry]) {
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
    let mut regrouping = PendingTabs::new(&state.session.tabs, &state.session.apps, now);
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
/// saved windows are still missing, start it again for those after `RESTART_SETTLE`.
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
            "session: {app} is {n} window(s) short {RESTART_SETTLE:?} after a window, starting it again"
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
            loose: Vec::new(),
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
        let mut p = PendingTabs::<u32>::new(&[tabs(&["firefox", "firefox"], 0, 0)], &[], t);
        p.started("firefox");
        assert_eq!(p.restart("firefox", t), 0, "no window yet: no settle armed");
        let mut p = PendingTabs::<u32>::new(&[tabs(&["firefox", "firefox"], 0, 0)], &[], t);
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
        let mut p = PendingTabs::new(&saved, &[], t);
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
        let mut p = PendingTabs::new(&saved, &[], t);
        p.started("foot");
        p.claim("foot", "", 1, t);
        assert!(p.settle("foot", t));
        assert_eq!(p.restart("foot", late), 0, "the restore window has ended");
    }

    #[test]
    fn a_window_back_outside_the_group_leaves_a_start_for_each_slot() {
        let t = Instant::now();
        let later = t + RESTART_SETTLE;
        let ed = SavedApp {
            loose: vec!["notes.txt".into()],
            ..saved("ed", "ed.desktop", 0)
        };
        let group = [titled(&["a.txt", "b.txt"])];
        let mut p = PendingTabs::new(&group, std::slice::from_ref(&ed), t);
        p.started("ed");
        assert_eq!(p.claim("ed", "notes.txt", 1, t), Claim::None);
        assert!(p.settle("ed", t));
        assert_eq!(p.restart("ed", later), 2, "both slots are empty");
        let mut p = PendingTabs::new(&[titled(&["a.txt"])], &[ed], t);
        p.started("ed");
        p.claim("ed", "notes.txt", 1, t);
        assert!(p.settle("ed", t));
        assert_eq!(p.restart("ed", later), 1, "a one-tab group's slot is empty");
    }

    fn tt() -> (SavedApp, [SavedTabs; 1]) {
        let tt = SavedApp {
            loose: vec!["Charlie".into()],
            ..saved("ed", "ed.desktop", 0)
        };
        (tt, [titled(&["Alpha", "Bravo"])])
    }

    #[test]
    fn a_loose_window_the_first_start_left_out_gets_a_start() {
        let t = Instant::now();
        let later = t + RESTART_SETTLE;
        let (tt, group) = tt();
        let mut p = PendingTabs::new(&group, std::slice::from_ref(&tt), t);
        p.started("ed");
        p.claim("ed", "Alpha", 1, t);
        assert!(p.settle("ed", t));
        p.claim("ed", "Bravo", 2, t);
        assert!(!p.settle("ed", t), "one settle at a time");
        assert_eq!(p.restart("ed", later), 1, "Charlie is missing");
        assert_eq!(p.claim("ed", "Charlie", 3, later), Claim::None);
        assert!(!p.settle("ed", later), "all three are back");
        assert_eq!(p.restart("ed", later + RESTART_SETTLE), 0);
    }

    #[test]
    fn a_loose_window_back_under_another_title_counts_as_back() {
        let t = Instant::now();
        let (tt, group) = tt();
        let mut p = PendingTabs::new(&group, std::slice::from_ref(&tt), t);
        p.started("ed");
        p.claim("ed", "Alpha", 1, t);
        assert!(p.settle("ed", t));
        p.claim("ed", "Bravo", 2, t);
        p.settle("ed", t);
        assert_eq!(p.claim("ed", "Editor", 3, t), Claim::None);
        p.settle("ed", t);
        assert_eq!(p.restart("ed", t + RESTART_SETTLE), 0);
        let mut p = PendingTabs::new(&group, &[tt], t);
        p.started("ed");
        p.claim("ed", "Editor", 1, t);
        assert!(p.settle("ed", t));
        p.claim("ed", "Bravo", 2, t);
        p.settle("ed", t);
        assert!(p.retitle("ed", "Charlie", 1, t));
        assert_eq!(
            p.restart("ed", t + RESTART_SETTLE),
            1,
            "Alpha's slot is empty"
        );
    }

    #[test]
    fn a_start_that_maps_no_window_is_not_repeated_for_a_loose_window() {
        let t = Instant::now();
        let (tt, group) = tt();
        let mut p = PendingTabs::new(&group, &[tt], t);
        p.started("ed");
        p.claim("ed", "Alpha", 1, t);
        assert!(p.settle("ed", t));
        p.claim("ed", "Bravo", 2, t);
        p.settle("ed", t);
        assert_eq!(p.restart("ed", t + RESTART_SETTLE), 1);
        assert_eq!(
            p.restart("ed", t + RESTART_SETTLE * 2),
            0,
            "no window mapped: no settle"
        );
        assert_eq!(p.claim("ed", "Alpha", 3, t), Claim::None);
        assert!(!p.settle("ed", t), "a second Alpha leaves nothing missing");
        assert_eq!(p.restart("ed", t + RESTART_SETTLE * 3), 0);
    }

    #[test]
    fn a_start_whose_window_has_not_mapped_is_not_started_again() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[tabs(&["lo", "lo", "lo", "lo"], 0, 0)], &[], t);
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
        let mut p = PendingTabs::new(
            &[tabs(&["a", "b", "c"], 1, 2), tabs(&["b", "d"], 0, 0)],
            &[],
            t,
        );
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
        let mut p = PendingTabs::new(&saved, &[], t);
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

    fn titled(titles: &[&str]) -> SavedTabs {
        SavedTabs {
            titles: titles.iter().map(|s| s.to_string()).collect(),
            ..tabs(&vec!["ed"; titles.len()], 0, 0)
        }
    }

    #[test]
    fn a_window_guessed_into_a_slot_moves_to_the_one_its_title_names() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[titled(&["a.txt", "b.txt", "c.txt"])], &[], t);
        assert_eq!(p.claim("ed", "Editor", 1, t), Claim::First);
        assert!(matches!(p.claim("ed", "Editor", 2, t), Claim::Join { .. }));
        assert!(p.retitle("ed", "c.txt", 1, t));
        assert_eq!(p.moved(), [1]);
        assert_eq!(
            p.placing(1),
            Claim::Join {
                target: 2,
                before: None,
                show: None
            }
        );
        assert!(p.retitle("ed", "a.txt", 2, t));
        assert_eq!(p.moved(), [2]);
        assert_eq!(
            p.placing(2),
            Claim::Join {
                target: 1,
                before: Some(1),
                show: Some(2)
            }
        );
        assert_eq!(
            p.claim("ed", "b.txt", 3, t),
            Claim::Join {
                target: 2,
                before: Some(1),
                show: Some(2)
            },
            "the slot the guess left"
        );
        assert!(
            !p.retitle("ed", "b.txt", 1, t),
            "a window moved by its title stays"
        );
    }

    #[test]
    fn a_guess_settles_by_its_title_or_in_time() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[titled(&["a.txt", "b.txt"])], &[], t);
        p.claim("ed", "Editor", 1, t);
        p.claim("ed", "Editor", 2, t);
        assert!(!p.retitle("ed", "a.txt", 1, t), "its own slot's title");
        assert!(!p.retitle("ed", "b.txt", 1, t), "and then it stays");
        let late = t + GUESS_SETTLE + Duration::from_secs(1);
        assert!(!p.retitle("ed", "a.txt", 2, late));
        assert!(!p.retitle("ed", "x.txt", 2, t), "a title no slot has");
        assert!(p.moved().is_empty());
    }

    #[test]
    fn a_window_back_with_a_slots_title_takes_it_from_a_guess() {
        let t = Instant::now();
        let mut p = PendingTabs::new(&[titled(&["a.txt", "b.txt"])], &[], t);
        assert_eq!(p.claim("ed", "Editor", 1, t), Claim::First);
        assert_eq!(
            p.claim("ed", "a.txt", 2, t),
            Claim::Join {
                target: 1,
                before: Some(1),
                show: Some(2)
            }
        );
        assert_eq!(p.moved(), [1]);
        assert_eq!(
            p.placing(1),
            Claim::Join {
                target: 2,
                before: None,
                show: Some(2)
            },
            "the guess moves to the free slot"
        );
    }

    #[test]
    fn a_loose_windows_title_keeps_its_window_out_of_the_group() {
        let t = Instant::now();
        let ed = SavedApp {
            loose: vec!["notes.txt".into()],
            ..saved("ed", "ed.desktop", 0)
        };
        let group = [titled(&["a.txt", "b.txt"])];
        let mut p = PendingTabs::new(&group, std::slice::from_ref(&ed), t);
        assert_eq!(p.desktop("ed", "notes.txt", t), None);
        assert_eq!(p.claim("ed", "notes.txt", 1, t), Claim::None);
        assert_eq!(
            p.claim("ed", "notes.txt", 2, t),
            Claim::First,
            "one loose window was saved"
        );
        let mut p = PendingTabs::new(&group, &[ed], t);
        p.claim("ed", "Editor", 1, t);
        p.claim("ed", "Editor", 2, t);
        assert!(p.retitle("ed", "notes.txt", 1, t));
        assert_eq!(p.moved(), [1]);
        assert_eq!(p.placing(1), Claim::None);
        assert_eq!(
            p.desktop("ed", "Editor", t),
            Some(0),
            "its slot is free again"
        );
    }

    #[test]
    fn note_loose_keeps_the_titles_outside_groups_of_grouped_apps() {
        let mut apps = [
            saved("ed", "ed.desktop", 0),
            saved("gimp", "gimp.desktop", 0),
        ];
        let open = [
            ("ed".to_string(), "notes.txt".to_string()),
            ("gimp".to_string(), "pic".to_string()),
        ];
        note_loose(&mut apps, &[tabs(&["ed", "ed"], 0, 0)], &open);
        assert_eq!(apps[0].loose, ["notes.txt"]);
        assert!(apps[1].loose.is_empty(), "gimp is in no group");
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
        let mut p = PendingTabs::new(&[tabs(&["a", "b", "c"], 0, 0)], &[], t);
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
        let mut p = PendingTabs::new(&saved, &[], t);
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
        let p = PendingTabs::<u32>::new(&saved, &[], t);
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
        let mut p = PendingTabs::new(&[tabs(&["a", "b"], 0, 1)], &[], t);
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

    #[test]
    fn the_running_sessions_list_is_saved_on_the_timer_once_restored_windows_are_back() {
        let mut h = crate::harness::Harness::new();
        assert!(h.state.config.session.restore, "restore is on by default");
        h.state.app_index = vec![app("foot", "foot", None)];
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        h.map_toplevel(&mut c, "foot", 300, 200);
        let foot = saved("foot", "foot.desktop", 0);
        h.state.session.restoring = Pending::new(&[&foot], Instant::now());
        save_every(&mut h.state, Duration::from_millis(20));
        h.run_for(100);
        assert!(
            h.state.session.apps.is_empty(),
            "nothing is saved while foot is awaited"
        );

        h.state.session.restoring.mapped("foot");
        let queued = crate::state::QUEUED.with(|n| n.get());
        h.run_for(100);
        assert_eq!(h.state.session.apps, vec![foot]);
        assert_eq!(
            crate::state::QUEUED.with(|n| n.get()),
            queued + 1,
            "one write, the unchanged ticks after it write nothing"
        );
    }
}
