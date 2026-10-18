//! Tab groups. A window held over another window's title bar for
//! [`DROP_DELAY`] shows there as a ghost tab and joins that window's group on
//! release; the window menu's "Add to tabs of" and "Remove from tab group"
//! rows do the same without a drag.
//!
//! The group's shown window carries the frame and draws the tabs; the others
//! are hidden as a minimized window is (`Window::tab_hidden`) and keep their
//! task buttons. A window that comes to show takes the frame rect and the
//! maximized, snapped and minimized state of the one it replaces
//! (`adopt_frame`); the hidden ones follow the shown one's moves and sizes
//! (`follow_tabs`). Fullscreen passes the same way, and a hidden tab's own
//! fullscreen request waits until it shows (`ask_fullscreen`). A tab
//! dragged along the caption reorders; dragged off it, it leaves the group
//! and carries on as a move of its own window.
//!
//! A window also joins a group as it maps, for session restore, a
//! `tab_group` window rule or its client's reprise-tabs-v1 join (`tab_on_map`).

use std::time::{Duration, Instant};

use reprisekit::controls::captiontabs::{self, TabHit};
use reprisekit::frame::FrameHit;
use reprisekit::raster::Rect;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::{SceneOwner, State};
use crate::input::Interaction;
use crate::session::Claim;
use crate::tabs::Groups;
use crate::window::{Kind, WindowId};

/// How long a dragged window rests over a caption before a release groups it.
pub const DROP_DELAY: Duration = Duration::from_millis(450);

const HELD_OPACITY: f32 = 0.55;

/// A window held over another window's caption during a move.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drop {
    pub dragged: WindowId,
    /// The shown window whose caption the pointer is over.
    pub target: WindowId,
    /// Where in `target`'s tabs a release puts `dragged`.
    pub slot: usize,
    pub since: Instant,
    /// The delay is up: the caption shows the ghost tab, and the release
    /// joins the group.
    pub armed: bool,
}

#[derive(Default)]
pub struct TabView {
    pub groups: Groups<WindowId>,
    pub drop: Option<Drop>,
    timer: Option<RegistrationToken>,
    pub hot: Option<(WindowId, TabHit)>,
    pub held: Option<(WindowId, TabHit)>,
    /// The tab the move in progress tore off its group, for Escape to put
    /// back.
    torn: Option<Torn>,
    /// Fullscreen held for a hidden tab until it shows, with the frame its
    /// unfullscreen goes back to when it came from the tab it replaced.
    fullscreen_waits: Vec<(WindowId, Option<Rect>)>,
}

/// A tab torn off its group by a drag: its window, a member left in the
/// group and the tab's slot there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Torn {
    pub window: WindowId,
    pub beside: WindowId,
    pub slot: usize,
}

impl TabView {
    /// `w` is about to leave its group by a drag: remember where it was.
    fn tear_off(&mut self, w: WindowId) {
        let members = self.groups.members_of(w);
        let slot = members.iter().position(|&m| m == w);
        let beside = members.iter().find(|&&m| m != w);
        self.torn = slot.zip(beside).map(|(slot, &beside)| Torn {
            window: w,
            beside,
            slot,
        });
    }

    pub fn forget_torn(&mut self) {
        self.torn = None;
    }

    /// The tear-off of `w`, if the move in progress is one.
    pub fn take_torn(&mut self, w: WindowId) -> Option<Torn> {
        self.torn.take_if(|t| t.window == w)
    }

    /// A fullscreen request (`on`) or unfullscreen from `w`. A hidden tab
    /// never takes the screen: its request waits until it shows, and its
    /// unfullscreen drops the wait. Returns whether the request applies now.
    pub fn ask_fullscreen(&mut self, w: WindowId, on: bool) -> bool {
        if !self.groups.is_hidden(w) {
            return true;
        }
        self.fullscreen_waits.retain(|&(x, _)| x != w);
        if on {
            self.fullscreen_waits.push((w, None));
        }
        false
    }

    /// Fullscreen tab `m` went into hiding, `lead` being the tab shown
    /// before the change, `restore` the frame from before fullscreen.
    /// When `m` was that tab its fullscreen passes to the one shown in its
    /// place, so the group stays fullscreen; otherwise `m` keeps it for
    /// when it shows.
    fn hid_fullscreen(&mut self, m: WindowId, lead: Option<WindowId>, restore: Option<Rect>) {
        let heir = if lead == Some(m) {
            self.groups.shown_of(m)
        } else {
            m
        };
        self.pass_fullscreen(heir, restore);
    }

    /// `w` leaves its group as it closes; returns the tab shown in its
    /// place. When `w` showed fullscreen, `fullscreen` holding the frame
    /// from before, that tab takes the fullscreen.
    fn close_tab(&mut self, w: WindowId, fullscreen: Option<Option<Rect>>) -> Option<WindowId> {
        self.fullscreen_waits.retain(|&(x, _)| x != w);
        let shown = self.groups.shown_of(w) == w;
        let now = self.groups.leave(w)?;
        if shown
            && now != w
            && let Some(restore) = fullscreen
        {
            self.pass_fullscreen(now, restore);
        }
        Some(now)
    }

    fn pass_fullscreen(&mut self, heir: WindowId, restore: Option<Rect>) {
        self.fullscreen_waits.retain(|&(x, _)| x != heir);
        self.fullscreen_waits.push((heir, restore));
    }

    /// The fullscreen waiting for `w`, now shown: `Some(restore)` when it
    /// goes fullscreen.
    fn take_fullscreen(&mut self, w: WindowId) -> Option<Option<Rect>> {
        let i = self.fullscreen_waits.iter().position(|&(x, _)| x == w)?;
        Some(self.fullscreen_waits.remove(i).1)
    }

    /// Put torn tab `t` back at its slot, showing. Returns the window
    /// that showed in the group before.
    fn put_back(&mut self, t: Torn) -> WindowId {
        let lead = self.groups.shown_of(t.beside);
        self.groups.join(t.window, t.beside);
        self.groups.reorder(t.window, t.slot);
        lead
    }
}

pub struct StripTab {
    pub title: String,
    pub icon: Option<std::sync::Arc<reprisekit::raster::Canvas>>,
}

/// What `w`'s caption draws in place of its title, when anything.
pub struct Strip {
    pub tabs: Vec<StripTab>,
    pub selected: usize,
    pub ghost: Option<usize>,
    pub hot: Option<TabHit>,
    pub held: Option<TabHit>,
}

impl State {
    /// Whether `w` can be in a tab group: a mapped, resizable window with
    /// our frame that no other window owns and that has a task button.
    /// None can while `[windows] tabs` is off.
    pub fn tabbable(&self, w: WindowId) -> bool {
        self.config.window_tabs
            && self.win(w).is_some_and(|x| {
                x.mapped && x.has_frame() && !x.is_owned() && x.is_resizable() && !x.skips_taskbar()
            })
    }

    /// The tab windows `w`'s caption shows: its group's, and the ghost of
    /// a window held over it at its slot. None for a window in no group
    /// with nothing held over it.
    fn strip_windows(&self, w: WindowId) -> Option<(Vec<WindowId>, usize, Option<usize>)> {
        let group = self.tabs.groups.group_of(w).filter(|g| g.shown() == w);
        let held = self.tabs.drop.filter(|d| d.armed && d.target == w);
        if group.is_none() && held.is_none() {
            return None;
        }
        let (mut members, mut selected) = match group {
            Some(g) => (g.members.clone(), g.shown),
            None => (vec![w], 0),
        };
        let ghost = held.map(|d| {
            let slot = d.slot.min(members.len());
            members.insert(slot, d.dragged);
            if slot <= selected {
                selected += 1;
            }
            slot
        });
        Some((members, selected, ghost))
    }

    /// The tabs `w`'s caption draws, with icons `px` px square.
    pub fn strip_for(&mut self, w: WindowId, px: i32) -> Option<Strip> {
        let (members, selected, ghost) = self.strip_windows(w)?;
        let State {
            windows,
            icons,
            app_index,
            config,
            ..
        } = self;
        // A window without an icon gets the glyph its own caption draws.
        let mut fallback = None;
        let tabs = members
            .iter()
            .filter_map(|&m| windows.iter().find(|x| x.id == m))
            .map(|win| StripTab {
                title: win.title(),
                icon: crate::icon::window_icon(icons, app_index, win, px).or_else(|| {
                    let glyph = fallback.get_or_insert_with(|| {
                        std::sync::Arc::new(reprisekit::icon::fallback(&config.theme, px))
                    });
                    Some(glyph.clone())
                }),
            })
            .collect();
        let on_w = |h: Option<(WindowId, TabHit)>| h.filter(|h| h.0 == w).map(|h| h.1);
        Some(Strip {
            tabs,
            selected,
            ghost,
            hot: on_w(self.tabs.hot),
            held: on_w(self.tabs.held),
        })
    }

    fn strip_layout(&self, w: WindowId) -> Option<captiontabs::Layout> {
        let (members, _, _) = self.strip_windows(w)?;
        let win = self.win(w)?;
        Some(captiontabs::layout(
            &self.config.theme,
            &win.geom,
            members.len(),
        ))
    }

    /// What on `w`'s tab strip layout point (`x`, `y`) is over, and the
    /// window of that tab.
    pub fn tab_at(&self, w: WindowId, x: f64, y: f64) -> Option<(TabHit, WindowId)> {
        self.tabs.groups.group_of(w).filter(|g| g.shown() == w)?;
        let (members, _, ghost) = self.strip_windows(w)?;
        let win = self.win(w)?;
        let l = self.strip_layout(w)?;
        let hit = captiontabs::hit(&l, x as i32 - win.x, y as i32 - win.y)?;
        if ghost == Some(hit.tab) {
            return None;
        }
        Some((hit, *members.get(hit.tab)?))
    }

    /// The slot in `target`'s tabs a window dropped at layout x `x` takes:
    /// the one its ghost tab would stand in under the pointer.
    fn drop_slot(&self, target: WindowId, x: f64) -> usize {
        let Some(win) = self.win(target) else {
            return usize::MAX;
        };
        let n = self.tabs.groups.members_of(target).len() + 1;
        let l = captiontabs::layout(&self.config.theme, &win.geom, n);
        captiontabs::slot_at(&l, x as i32 - win.x)
    }

    /// Light the tab the pointer is on: `over` is the frame the pointer
    /// is over, or None. Redraws the captions whose lit tab changed.
    pub fn tab_hover(&mut self, over: Option<WindowId>, x: f64, y: f64) {
        let hot = over.and_then(|w| self.tab_at(w, x, y).map(|(h, _)| (w, h)));
        if hot == self.tabs.hot {
            return;
        }
        let was = std::mem::replace(&mut self.tabs.hot, hot);
        for w in [was.map(|h| h.0), hot.map(|h| h.0)].into_iter().flatten() {
            self.redraw_caption(w);
        }
    }

    /// Bring the windows in `members` in line with the groups: each one a
    /// group hides is hidden, each one it shows is shown, taking `lead`'s
    /// frame and state when it comes to show in `lead`'s place (it was
    /// hidden, or `lead` now is). The focus follows a focused window that
    /// went into hiding to the window shown in its place.
    pub(crate) fn retab(&mut self, members: &[WindowId], lead: Option<WindowId>) {
        for &m in members {
            let Some(win) = self.win(m) else {
                continue;
            };
            let was_hidden = win.tab_hidden;
            if self.tabs.groups.is_hidden(m) {
                if !was_hidden {
                    self.hide_tab(m);
                }
                if self.win(m).is_some_and(|x| x.fullscreen) {
                    let restore = self.win_mut(m).and_then(|x| x.saved_fullscreen.take());
                    self.set_fullscreen(m, false);
                    self.tabs.hid_fullscreen(m, lead, restore);
                }
            } else {
                let lead = lead.filter(|&l| {
                    l != m
                        && self.win(l).is_some()
                        && (was_hidden || self.tabs.groups.shown_of(l) == m)
                });
                if let Some(l) = lead {
                    self.adopt_frame(m, l);
                }
                if was_hidden {
                    self.show_tab(m);
                }
                if let Some(l) = lead {
                    self.stack_in_place_of(m, l);
                }
            }
            self.redraw_frame(m);
        }
        for &m in members {
            if self.tabs.groups.is_hidden(m) {
                continue;
            }
            if let Some(restore) = self.tabs.take_fullscreen(m) {
                self.set_fullscreen(m, true);
                if let Some(r) = restore
                    && let Some(win) = self.win_mut(m)
                {
                    win.saved_fullscreen = Some(r);
                }
            }
        }
        if let Some(f) = self.focused
            && self.tabs.groups.is_hidden(self.home(f))
        {
            let shown = self.tabs.groups.shown_of(self.home(f));
            self.focus_window(shown);
        }
        self.redraw_taskbars();
        self.redraw_all();
    }

    /// Hide `m`, a window its group no longer shows, as a minimize does:
    /// its tree and the windows it owns go, and it is no longer active.
    fn hide_tab(&mut self, m: WindowId) {
        if self.interaction.acts_on(m) {
            self.cancel_interaction();
        }
        let Some(win) = self.win_mut(m) else {
            return;
        };
        win.tab_hidden = true;
        let tree = win.tree;
        if let Some(x) = win.x11()
            && let Err(e) = x.set_hidden(true)
        {
            log::debug!("x11 hidden: {e}");
        }
        self.set_activated(m, false);
        self.scene.set_enabled(tree, false);
        self.show_children(m, false);
        crate::idle::inhibition_changed(self);
    }

    /// or on another desktop.
    fn show_tab(&mut self, m: WindowId) {
        let here = self.on_current_desktop(m);
        let Some(win) = self.win_mut(m) else {
            return;
        };
        win.tab_hidden = false;
        let show = win.mapped && !win.minimized && here;
        let tree = win.tree;
        if let Some(x) = win.x11()
            && let Err(e) = x.set_hidden(!show)
        {
            log::debug!("x11 hidden: {e}");
        }
        self.scene.set_enabled(tree, show);
        self.show_children(m, show);
        crate::idle::inhibition_changed(self);
    }

    /// Stack `m` where `lead` stands, when they share a layer.
    fn stack_in_place_of(&mut self, m: WindowId, lead: WindowId) {
        let (Some(a), Some(b)) = (self.win(m).map(|x| x.tree), self.win(lead).map(|x| x.tree))
        else {
            return;
        };
        if self.scene.parent(a) == self.scene.parent(b) {
            self.scene.place_above(a, b);
            self.raise_children(m);
        }
    }

    /// Give `m` `lead`'s frame outside fullscreen: its place and size,
    /// maximized and snapped state, the rect a restore goes back to, its
    /// desktop, and, when `lead` is hidden now, its minimized state.
    /// Fullscreen passes on its own (`hid_fullscreen`, `close_tab`). The
    /// client is asked for the size at once; the frame follows its commit.
    fn adopt_frame(&mut self, m: WindowId, lead: WindowId) {
        let lead_hidden = self.tabs.groups.is_hidden(lead);
        let usable = self.usable_area_for_window(lead);
        let Some(l) = self.win(lead) else {
            return;
        };
        let rect = frame_outside_fullscreen(
            l.frame_rect(),
            l.fullscreen,
            l.maximized,
            l.saved_fullscreen,
            usable,
        );
        let (maximized, saved, snapped) = (l.maximized, l.saved, l.snapped);
        let (desktop, sticky, minimized) = (l.desktop, l.sticky, l.minimized);
        if lead_hidden && let Some(l) = self.win_mut(lead) {
            l.minimized = false;
        }
        let metrics = self.config.theme.metrics.clone();
        let Some(win) = self.win_mut(m) else {
            return;
        };
        win.x = rect.x;
        win.y = rect.y;
        win.saved = saved;
        win.snapped = snapped;
        win.desktop = desktop;
        win.sticky = sticky;
        win.resize_anchor = None;
        if lead_hidden {
            win.minimized = minimized;
        }
        if win.maximized != maximized {
            win.maximized = maximized;
            match &win.kind {
                Kind::Xdg(t) => t.with_pending_state(|s| {
                    if maximized {
                        s.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
                    } else {
                        s.states.unset(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized);
                    }
                }),
                Kind::X11(x) => {
                    if let Err(e) = x.set_maximized(maximized) {
                        log::debug!("x11 maximized: {e}");
                    }
                }
            }
        }
        let chrome = win.chrome();
        let (cw, ch) = reprisekit::frame::FrameGeometry::client_size_for_total(
            &metrics,
            rect.width,
            rect.height,
            maximized,
            chrome,
        );
        self.relayout(m);
        self.request_size(m, cw, ch);
    }

    /// Keep the tabs hidden behind `w`, when it is its group's shown window,
    /// on its frame: they move with it, and with `size` they also take its
    /// size and state. Sizes wait for the end of a drag resize, so hidden
    /// clients get one configure instead of one per motion.
    pub(crate) fn follow_tabs(&mut self, w: WindowId, size: bool) {
        let hidden = self.tabs.groups.hidden_behind(w);
        if hidden.is_empty() {
            return;
        }
        let Some(rect) = self.win(w).map(|x| x.frame_rect()) else {
            return;
        };
        for m in hidden {
            if size {
                self.adopt_frame(m, w);
                continue;
            }
            let Some(win) = self.win_mut(m) else {
                continue;
            };
            if (win.x, win.y) == (rect.x, rect.y) {
                continue;
            }
            win.x = rect.x;
            win.y = rect.y;
            let tree = win.tree;
            self.scene.set_position(tree, rect.x, rect.y);
            self.place_x11(m);
        }
    }

    pub fn select_tab(&mut self, m: WindowId) {
        if let Some(prev) = self.tabs.groups.select(m) {
            let members = self.tabs.groups.members_of(m);
            self.retab(&members, Some(prev));
        }
    }

    /// The `tab-next` and `tab-prev` binds: the focused window's group
    /// shows its next tab, or the one before.
    pub fn step_tab(&mut self, backwards: bool) {
        let Some(f) = self.focused else {
            return;
        };
        let home = self.home(f);
        if let Some(next) = self.tabs.groups.step(home, backwards) {
            self.select_tab(next);
            self.focus_window(next);
        }
    }

    /// Put `w` into `target`'s group at `slot` (past the end appends) and
    /// show it there: with `whole`, `w`'s own group comes along; otherwise
    /// `w` leaves it alone. Either must be [`State::tabbable`].
    pub fn join_tabs(&mut self, w: WindowId, target: WindowId, slot: usize, whole: bool) {
        if self.join_tabs_quiet(w, target, slot, whole) {
            self.focus_window(w);
        }
    }

    /// `join_tabs` without the focus. Returns whether `w` joined.
    fn join_tabs_quiet(&mut self, w: WindowId, target: WindowId, slot: usize, whole: bool) -> bool {
        if !self.tabbable(w) || !self.tabbable(target) {
            return false;
        }
        let groups = &self.tabs.groups;
        if groups.members_of(target).contains(&w) {
            return false;
        }
        if !whole && groups.group_of(w).is_some() {
            self.leave_tabs_quiet(w);
        }
        let lead = self.tabs.groups.shown_of(target);
        self.tabs.groups.join_at(w, target, slot);
        let members = self.tabs.groups.members_of(w);
        self.regrouped(&members);
        self.retab(&members, Some(lead));
        true
    }

    fn regrouped(&mut self, members: &[WindowId]) {
        for &m in members {
            if let Some(win) = self.win_mut(m) {
                win.tab_pulled_out = false;
            }
        }
    }

    fn pulled_out(&mut self, w: WindowId) {
        if let Some(win) = self.win_mut(w) {
            win.tab_pulled_out = true;
        }
    }

    /// A window that just mapped joins the tab group it belongs in: the
    /// restored session's group it is back for, or else the group of the
    /// oldest open window its `tab_group` rule names that was not pulled
    /// out of its group by hand (`rule_target`). It goes to the
    /// group's desktop. A restored app still short of its saved windows
    /// may start again (`session::settle_restart`).
    pub(crate) fn tab_on_map(&mut self, w: WindowId) {
        let Some((app, title)) = self.win(w).map(|x| (x.app(), x.title())) else {
            return;
        };
        let desktop = self.desktop_of(w);
        let now = Instant::now();
        let claim = self.session.regrouping.claim(&app, &title, w, now);
        crate::session::settle_restart(self, &app);
        match claim {
            Claim::First => {}
            Claim::Join {
                target,
                before,
                show,
            } => {
                let lead = self.tabs.groups.shown_of(target);
                let members = self.tabs.groups.members_of(target);
                let slot = before
                    .and_then(|b| members.iter().position(|&m| m == b))
                    .unwrap_or(usize::MAX);
                if self.join_tabs_quiet(w, target, slot, false) {
                    self.select_tab(show.unwrap_or(lead));
                }
            }
            Claim::None => {
                if let Some(l) = self.win_mut(w).and_then(|x| x.tab_leader.take()) {
                    self.join_next_to(w, l);
                } else if let Some(name) = self.win(w).and_then(|x| x.tab_group.clone()) {
                    let open = self
                        .windows
                        .iter()
                        .filter(|x| x.id != w && self.tabbable(x.id));
                    let open = open.map(|x| (x.id, x.tab_group.as_deref(), x.tab_pulled_out));
                    if let Some(t) = rule_target(open, &name) {
                        self.join_tabs_quiet(w, t, usize::MAX, false);
                    }
                }
            }
        }
        if self.desktop_of(w) != desktop {
            self.desktops_changed();
        }
    }

    /// reprise-tabs-v1's join: `w` joins `leader`'s group as it maps, or now when it is mapped.
    pub(crate) fn join_leader(&mut self, w: WindowId, leader: WindowId) {
        match self.win(w).map(|x| x.mapped) {
            Some(true) => self.join_next_to(w, leader),
            Some(false) => {
                if let Some(win) = self.win_mut(w) {
                    win.tab_leader = Some(leader);
                }
            }
            None => {}
        }
    }

    /// `w` joins `leader`'s group right after it and shows, with the focus when the group had it.
    fn join_next_to(&mut self, w: WindowId, leader: WindowId) {
        let members = self.tabs.groups.members_of(leader);
        let had_focus = self
            .focused
            .is_some_and(|f| members.contains(&self.home(f)));
        let slot = after(&members, leader);
        if self.join_tabs_quiet(w, leader, slot, false) && had_focus {
            self.focus_window(w);
        }
    }

    /// Take `w` out of its group, the group showing another tab in its
    /// place when it showed; `w` keeps the group's frame. Returns whether
    /// it was in one.
    fn leave_tabs_quiet(&mut self, w: WindowId) -> bool {
        let Some(g) = self.tabs.groups.group_of(w).cloned() else {
            return false;
        };
        let lead = g.shown();
        self.tabs.groups.leave(w);
        let mut all = g.members;
        // `w` itself shows again, in the group's frame when it was hidden.
        all.retain(|&m| m != w);
        all.insert(0, w);
        self.retab(&all, Some(lead));
        true
    }

    /// The window menu's "Remove from tab group": `w` leaves its group
    /// and stands on its own, a cascade step down and right of the
    /// group's frame unless maximized, and takes the focus.
    pub fn leave_tabs(&mut self, w: WindowId) {
        if !self.leave_tabs_quiet(w) {
            return;
        }
        self.pulled_out(w);
        self.step_off_group(w);
        self.focus_window(w);
    }

    /// `[windows] tabs` turned off: every group comes apart, each window
    /// that was hidden showing a cascade step off its group's frame.
    pub fn ungroup_all(&mut self) {
        self.end_tab_drop();
        while let Some(w) = self.tabs.groups.first_hidden() {
            self.leave_tabs_quiet(w);
            self.step_off_group(w);
        }
    }

    /// the group's frame unless maximized. A fullscreen window stays put
    /// and its unfullscreen lands there instead.
    fn step_off_group(&mut self, w: WindowId) {
        let Some(win) = self.win(w).filter(|x| !x.maximized) else {
            return;
        };
        let fullscreen = win.fullscreen;
        let m = &self.config.theme.metrics;
        let frame = if fullscreen {
            m.frame_width
        } else {
            win.geom.frame
        };
        let step = m.caption_height + 2 * frame;
        let r = match win.saved_fullscreen {
            Some(r) if fullscreen => r,
            _ => win.frame_rect(),
        };
        let area = self.usable_area_for_window(w);
        let (x, y) =
            crate::window::keep_inside((r.x + step, r.y + step), (r.width, r.height), area);
        if fullscreen {
            if let Some(win) = self.win_mut(w) {
                win.saved_fullscreen = Some(Rect::new(x, y, r.width, r.height));
            }
        } else {
            self.move_to(w, x, y);
        }
    }

    /// The window menu's "Add to tabs of" choices for `w`: every other
    /// window that can take tabs on the current desktop, outside `w`'s
    /// own group, a group's shown window only, in stacking order from the
    /// top.
    pub fn tab_targets(&self, w: WindowId) -> Vec<WindowId> {
        if !self.tabbable(w) {
            return Vec::new();
        }
        let own = self.tabs.groups.members_of(self.home(w));
        self.focus_order
            .iter()
            .rev()
            .copied()
            .filter(|&t| {
                !own.contains(&t)
                    && !self.tabs.groups.is_hidden(t)
                    && self.tabbable(t)
                    && self.on_current_desktop(t)
            })
            .collect()
    }

    /// Everything that remembers `w` lets it go; its group closes up
    /// around it, showing the next tab when `w` showed, which takes the
    /// focus `w` had (`was_focused`).
    pub(crate) fn forget_tabs(&mut self, w: WindowId, was_focused: bool) {
        if self
            .tabs
            .drop
            .is_some_and(|d| d.dragged == w || d.target == w)
        {
            self.end_tab_drop();
        }
        if self.tabs.hot.is_some_and(|h| h.0 == w) {
            self.tabs.hot = None;
        }
        if self.tabs.held.is_some_and(|h| h.0 == w) {
            self.tabs.held = None;
        }
        let Some(g) = self.tabs.groups.group_of(w).cloned() else {
            self.tabs.fullscreen_waits.retain(|&(x, _)| x != w);
            return;
        };
        let fullscreen = self
            .win(w)
            .filter(|x| x.fullscreen)
            .map(|x| x.saved_fullscreen);
        let Some(now) = self.tabs.close_tab(w, fullscreen) else {
            return;
        };
        if let Some(win) = self.win_mut(w) {
            win.tab_hidden = false;
        }
        let rest: Vec<WindowId> = g.members.into_iter().filter(|&m| m != w).collect();
        self.retab(&rest, Some(w));
        if was_focused && now != w {
            self.focus_order.retain(|&p| p != now);
            self.focus_order.push(now);
        }
    }

    /// release now.
    pub fn tab_drop_armed(&self, w: WindowId) -> bool {
        self.tabs.drop.is_some_and(|d| d.armed && d.dragged == w)
    }

    /// The shown window whose caption lies under layout point (`x`, `y`),
    /// leaving out `skip`: the top window there, when the point is on its
    /// title bar and it can take tabs. A window over the caption hides it.
    fn caption_under(&self, skip: &[WindowId], x: f64, y: f64) -> Option<WindowId> {
        let m = &self.config.theme.metrics;
        for layer in [self.layers.on_top, self.layers.windows] {
            for &node in self.scene.children(layer).iter().rev() {
                if !self.scene.enabled(node) {
                    continue;
                }
                let Some((at, SceneOwner::Window(id))) = self.scene.find_data(node) else {
                    continue;
                };
                if at != node || skip.contains(id) || skip.contains(&self.home(*id)) {
                    continue;
                }
                let Some(win) = self.win(*id) else {
                    continue;
                };
                if !win.frame_rect().contains(x as i32, y as i32) {
                    continue;
                }
                let on_title = matches!(win.hit_at(x, y, m), FrameHit::Title | FrameHit::Icon);
                return (on_title && self.tabbable(*id)).then_some(*id);
            }
        }
        None
    }

    /// After a settled motion of a move of `w`: note the caption under the
    /// pointer, arming it once the pointer has rested there for
    /// [`DROP_DELAY`]. On the snap bar's area no caption counts: the bar
    /// wins (`snapbarview::holds`). Returns whether a release now would
    /// join a group, in which case the move snaps nowhere.
    pub fn track_tab_drop(&mut self, w: WindowId) -> bool {
        let pos = self.pointer_location();
        let on_bar = (self.output_exactly_at(pos.x, pos.y))
            .is_some_and(|o| crate::snapbarview::holds(self, &o, pos.x, pos.y));
        let target = if self.tabbable(w) && !on_bar && !self.win(w).is_some_and(|x| x.maximized) {
            let skip = self.tabs.groups.members_of(w);
            self.caption_under(&skip, pos.x, pos.y)
        } else {
            None
        };
        match (self.tabs.drop, target) {
            (Some(d), Some(t)) if d.target == t && d.dragged == w => {
                let slot = self.drop_slot(t, pos.x);
                if let Some(d) = self.tabs.drop.as_mut() {
                    d.slot = slot;
                }
                if !d.armed && d.since.elapsed() >= DROP_DELAY {
                    self.arm_tab_drop();
                } else if d.armed && d.slot != slot {
                    self.redraw_caption(t);
                }
            }
            (_, Some(t)) => {
                self.end_tab_drop();
                self.tabs.drop = Some(Drop {
                    dragged: w,
                    target: t,
                    slot: self.drop_slot(t, pos.x),
                    since: Instant::now(),
                    armed: false,
                });
                let token = self
                    .handle
                    .insert_source(Timer::from_duration(DROP_DELAY), |_, _, state| {
                        state.tabs.timer = None;
                        let still = state.tabs.drop.filter(|d| !d.armed).is_some_and(|d| {
                            matches!(state.interaction, Interaction::Move { window, .. } if window == d.dragged)
                        });
                        if still {
                            state.track_tab_drop_rest();
                        }
                        TimeoutAction::Drop
                    });
                match token {
                    Ok(t) => self.tabs.timer = Some(t),
                    Err(e) => log::warn!("tab drop timer: {}", e.error),
                }
            }
            (Some(_), None) => self.end_tab_drop(),
            (None, None) => {}
        }
        self.tab_drop_armed(w)
    }

    /// The delay ran out with the pointer at rest: arm the drop if the
    /// pointer is still over the same caption.
    fn track_tab_drop_rest(&mut self) {
        let Some(d) = self.tabs.drop else {
            return;
        };
        if self.track_tab_drop(d.dragged) {
            self.hide_snap_outline();
            if let Interaction::Move { snap, .. } = &mut self.interaction {
                *snap = None;
            }
        }
    }

    /// let the dragged window go see-through.
    fn arm_tab_drop(&mut self) {
        let Some(d) = self.tabs.drop.as_mut() else {
            return;
        };
        d.armed = true;
        let d = *d;
        if let Some(tree) = self.win(d.dragged).map(|x| x.tree) {
            self.scene.set_opacity(tree, HELD_OPACITY);
        }
        self.redraw_caption(d.target);
        self.redraw_all();
    }

    /// window is solid again.
    pub fn end_tab_drop(&mut self) {
        if let Some(t) = self.tabs.timer.take() {
            self.handle.remove(t);
        }
        let Some(d) = self.tabs.drop.take() else {
            return;
        };
        if d.armed {
            if let Some(tree) = self.win(d.dragged).map(|x| x.tree) {
                self.scene.set_opacity(tree, 1.0);
            }
            self.redraw_caption(d.target);
            self.redraw_all();
        }
    }

    /// under it when the drop is armed. Returns whether it joined.
    pub fn finish_tab_drop(&mut self, w: WindowId) -> bool {
        let d = self.tabs.drop.filter(|d| d.armed && d.dragged == w);
        self.end_tab_drop();
        let Some(d) = d else {
            return false;
        };
        self.join_tabs(d.dragged, d.target, d.slot, true);
        true
    }

    /// A press on tab `hit` of `window`'s caption, with `button`, whose
    /// tab is `member`'s. Left picks the tab and may drag it (or holds its
    /// close box), middle closes it, right opens its window menu at `at`.
    /// `double` is a second left click on the caption in time.
    pub fn tab_press(
        &mut self,
        window: WindowId,
        hit: TabHit,
        member: WindowId,
        button: u32,
        at: (i32, i32),
        double: bool,
    ) {
        use crate::input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT};
        match button {
            BTN_MIDDLE => self.close(member),
            BTN_RIGHT => self.open_window_menu(member, at, false),
            BTN_LEFT if hit.close => {
                self.tabs.held = Some((window, hit));
                self.redraw_caption(window);
                self.interaction = Interaction::TabPress {
                    window,
                    member,
                    close: true,
                    press: self.press_at,
                };
            }
            BTN_LEFT if double => {
                let maximized = self.win(window).is_some_and(|w| w.maximized);
                self.set_maximized(window, !maximized);
            }
            BTN_LEFT => {
                self.select_tab(member);
                self.focus_window(member);
                self.tabs.held = Some((member, hit));
                self.interaction = Interaction::TabPress {
                    window: member,
                    member,
                    close: false,
                    press: self.press_at,
                };
            }
            _ => {}
        }
    }

    /// The pointer moved to (`x`, `y`) with a tab held: a close box lights
    /// while the pointer is on it; a tab dragged past `DRAG_PX` along the
    /// caption moves among the tabs, and dragged off the caption leaves the
    /// group and carries on as a move of its own window.
    pub fn tab_press_moved(&mut self, x: f64, y: f64) {
        let Interaction::TabPress {
            window,
            member,
            close,
            press,
        } = self.interaction
        else {
            return;
        };
        self.tab_hover(Some(window), x, y);
        if close {
            return;
        }
        let slop = f64::from(crate::desktop::DRAG_PX);
        if (x - press.0).abs() < slop && (y - press.1).abs() < slop {
            return;
        }
        let Some(win) = self.win(window) else {
            return;
        };
        let cap = win.geom.caption;
        let band = Rect::new(
            win.x,
            win.y + cap.y - cap.height / 2,
            win.geom.total.0,
            2 * cap.height,
        );
        let in_group = self.tabs.groups.members_of(member).len() > 1;
        if in_group && band.contains(x as i32, y as i32) {
            if let Some(l) = self.strip_layout(window) {
                let slot = captiontabs::slot_at(&l, x as i32 - win.x);
                let before = self.tabs.groups.members_of(member);
                self.tabs.groups.reorder(member, slot);
                if self.tabs.groups.members_of(member) != before {
                    if let Some(h) = self.tabs.held.as_mut() {
                        h.1.tab = slot;
                    }
                    self.redraw_caption(window);
                }
            }
            return;
        }
        self.tabs.held = None;
        self.interaction = Interaction::Idle;
        if in_group {
            self.tabs.tear_off(member);
            self.leave_tabs_quiet(member);
            self.pulled_out(member);
            if let Some(tree) = self.win(member).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
            self.raise_children(member);
        }
        self.redraw_caption(window);
        let torn = self.tabs.torn.take();
        self.begin_move(member, true);
        self.tabs.torn = torn;
    }

    /// Escape undid the move of `w`, a tab torn off its group: it goes back
    /// to its tab, showing, while the member it left is still there.
    pub fn untear(&mut self, w: WindowId) {
        let Some(t) = self.tabs.take_torn(w) else {
            return;
        };
        if !self.tabbable(t.window) || !self.tabbable(t.beside) {
            return;
        }
        let lead = self.tabs.put_back(t);
        let members = self.tabs.groups.members_of(w);
        self.regrouped(&members);
        self.retab(&members, Some(lead));
        self.focus_window(w);
    }

    /// the pointer is still on it.
    pub fn tab_press_released(&mut self) {
        let Interaction::TabPress {
            window,
            member,
            close,
            ..
        } = self.interaction
        else {
            return;
        };
        let held = self.tabs.held.take();
        let on = held.is_some() && self.tabs.hot == held;
        self.redraw_caption(window);
        if close && on {
            self.close(member);
        }
    }

    pub fn tab_press_cancelled(&mut self, window: WindowId) {
        self.tabs.held = None;
        self.redraw_caption(window);
    }

    /// `windows` and every window in their tab groups, each once: what a
    /// desktop move or Show on All Desktops takes along.
    pub fn with_tab_groups(&self, windows: &[WindowId]) -> Vec<WindowId> {
        let mut all = Vec::new();
        for &w in windows {
            for m in self.tabs.groups.members_of(self.home(w)) {
                if !all.contains(&m) {
                    all.push(m);
                }
            }
            if !all.contains(&w) {
                all.push(w);
            }
        }
        all
    }
}

/// A window's frame rect when it is not fullscreen: the one it had before,
/// or the `usable` area when maximized underneath.
fn frame_outside_fullscreen(
    frame: Rect,
    fullscreen: bool,
    maximized: bool,
    saved_fullscreen: Option<Rect>,
    usable: Rect,
) -> Rect {
    match saved_fullscreen {
        _ if !fullscreen => frame,
        Some(r) => r,
        None if maximized => usable,
        None => frame,
    }
}

/// The window a new window of `tab_group` rule `name` joins, from the open
/// `windows` (id, rule name, pulled out by hand) oldest first.
fn rule_target<'a, T>(
    windows: impl IntoIterator<Item = (T, Option<&'a str>, bool)>,
    name: &str,
) -> Option<T> {
    windows
        .into_iter()
        .find(|&(_, n, pulled_out)| n == Some(name) && !pulled_out)
        .map(|(id, ..)| id)
}

/// The slot right after `leader` in its group's `members`; past the end appends.
fn after<T: PartialEq>(members: &[T], leader: T) -> usize {
    members
        .iter()
        .position(|m| *m == leader)
        .map_or(usize::MAX, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_joining_window_goes_right_after_its_leader() {
        assert_eq!(after(&[1, 2, 3], 2), 2);
        assert_eq!(after(&[1, 2, 3], 3), 3);
        assert_eq!(after(&[7], 7), 1, "a leader in no group starts one");
    }

    #[test]
    fn a_rule_passes_over_a_window_pulled_out_of_its_group() {
        let open = [
            (1, Some("shells"), true),
            (2, None, false),
            (3, Some("build"), false),
            (4, Some("shells"), false),
        ];
        assert_eq!(rule_target(open, "shells"), Some(4));
        assert_eq!(rule_target(open, "build"), Some(3));
        assert_eq!(rule_target(open, "mail"), None);
        assert_eq!(rule_target([(1, Some("shells"), true)], "shells"), None);
    }

    #[test]
    fn a_hidden_tabs_fullscreen_request_waits_until_it_shows() {
        let (a, b, lone) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.select(a);
        assert!(t.ask_fullscreen(a, true), "the shown tab goes fullscreen");
        assert!(t.ask_fullscreen(lone, true), "a window in no group too");
        assert!(!t.ask_fullscreen(b, true), "the hidden tab waits");
        assert_eq!(t.take_fullscreen(a), None);
        assert_eq!(
            t.take_fullscreen(b),
            Some(None),
            "b goes fullscreen as it shows"
        );
        assert_eq!(t.take_fullscreen(b), None, "taken once");
    }

    #[test]
    fn a_hidden_tabs_unfullscreen_drops_its_wait() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.select(a);
        assert!(!t.ask_fullscreen(b, true));
        assert!(!t.ask_fullscreen(b, false), "b stays hidden");
        assert_eq!(t.take_fullscreen(b), None);
    }

    #[test]
    fn a_fullscreen_tab_switched_away_hands_the_fullscreen_to_the_tab_shown() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.join(c, a);
        t.groups.select(a);
        let restore = Rect::new(40, 30, 640, 480);
        t.groups.select(b);
        t.hid_fullscreen(a, Some(a), Some(restore));
        assert_eq!(
            t.take_fullscreen(a),
            None,
            "a hidden tab holds no fullscreen"
        );
        assert_eq!(t.take_fullscreen(c), None);
        assert_eq!(
            t.take_fullscreen(b),
            Some(Some(restore)),
            "b fullscreen, restoring to a's frame from before"
        );
    }

    #[test]
    fn a_fullscreen_window_hidden_behind_another_keeps_its_fullscreen() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.select(a);
        t.hid_fullscreen(b, Some(a), None);
        assert_eq!(t.take_fullscreen(a), None);
        assert_eq!(t.take_fullscreen(b), Some(None));
    }

    #[test]
    fn closing_the_fullscreen_tab_hands_the_fullscreen_to_the_tab_shown_next() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.join(c, a);
        t.groups.select(a);
        let restore = Rect::new(40, 30, 640, 480);
        let next = t.close_tab(a, Some(Some(restore)));
        assert_eq!(next, Some(b));
        assert_eq!(
            t.take_fullscreen(b),
            Some(Some(restore)),
            "b fullscreen, restoring to a's frame from before"
        );
        assert_eq!(t.take_fullscreen(c), None);
    }

    #[test]
    fn a_tab_shown_in_a_fullscreen_tabs_place_takes_the_frame_from_before_fullscreen() {
        let screen = Rect::new(0, 0, 1920, 1080);
        let before = Rect::new(200, 150, 800, 500);
        let usable = Rect::new(0, 0, 1920, 1050);
        assert_eq!(
            frame_outside_fullscreen(screen, true, false, Some(before), usable),
            before
        );
        assert_eq!(
            frame_outside_fullscreen(screen, true, true, None, usable),
            usable,
            "maximized underneath: the usable area"
        );
        assert_eq!(
            frame_outside_fullscreen(before, false, false, None, usable),
            before
        );
    }

    #[test]
    fn closing_a_tab_that_was_not_fullscreen_passes_no_fullscreen() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.join(c, a);
        t.groups.select(a);
        assert_eq!(t.close_tab(a, None), Some(b));
        assert_eq!(t.take_fullscreen(b), None);
        assert_eq!(t.close_tab(c, Some(None)), Some(b), "c was hidden");
        assert_eq!(t.take_fullscreen(b), None);
    }

    #[test]
    fn a_torn_tab_goes_back_to_its_slot_showing() {
        let (a, b, c) = (WindowId::next(), WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.join(c, a);
        t.groups.select(b);
        t.tear_off(b);
        t.groups.leave(b);
        assert_eq!(t.groups.members_of(a), vec![a, c]);
        assert_eq!(t.take_torn(c), None, "only the torn window's move");
        let torn = t.take_torn(b).expect("b was torn off");
        assert_eq!(t.take_torn(b), None, "taken once");
        assert_eq!(t.put_back(torn), c, "c showed while b was out");
        assert_eq!(t.groups.members_of(a), vec![a, b, c]);
        assert_eq!(t.groups.shown_of(a), b);
    }

    #[test]
    fn a_torn_tab_of_two_makes_the_group_again() {
        let (a, b) = (WindowId::next(), WindowId::next());
        let mut t = TabView::default();
        t.groups.join(b, a);
        t.groups.select(a);
        t.tear_off(a);
        t.groups.leave(a);
        assert!(t.groups.group_of(b).is_none());
        let torn = t.take_torn(a).expect("a was torn off");
        t.put_back(torn);
        assert_eq!(
            t.groups.members_of(b),
            vec![a, b],
            "a back in the first slot"
        );
        assert_eq!(t.groups.shown_of(b), a);
        t.tear_off(a);
        t.forget_torn();
        assert_eq!(t.take_torn(a), None, "a new move forgets the tear-off");
    }
}
