use reprisekit::classic::frame::FrameGeometry;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

use smithay::output::Output;

use crate::compositor::State;
use crate::raster::Rect;

use super::{Kind, Window, WindowId};

impl State {
    /// Tell the client whether it is active; an X11 window also takes/gives the X input focus and
    /// raises in X stacking.
    pub fn set_activated(&mut self, w: WindowId, on: bool) {
        if let Some(x) = self.win(w).and_then(Window::x11).cloned() {
            self.activate_x11(&x, on);
            return;
        }
        if let Some(win) = self.win(w)
            && let Some(t) = win.xdg()
        {
            let changed = t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Activated)
                } else {
                    s.states.unset(xdg_toplevel::State::Activated)
                }
            });
            if changed && t.is_initial_configure_sent() {
                t.send_pending_configure();
            }
        }
    }

    /// Whether `w` can roll up: mapped with its own frame, in its normal state, and in no tab
    /// group.
    pub fn can_shade(&self, w: WindowId) -> bool {
        self.win(w).is_some_and(|x| {
            x.mapped && x.has_frame() && !x.maximized && !x.minimized && !x.tab_hidden
        }) && self.tabs.groups.group_of(w).is_none()
    }

    /// Roll `w` up to its caption or back down. The frame keeps its width and place; the client
    /// is sent nothing, keeps its size and is neither drawn nor given input while rolled up.
    pub fn set_shaded(&mut self, w: WindowId, on: bool) {
        if self.win(w).is_none_or(|x| x.shaded == on) || (on && !self.can_shade(w)) {
            return;
        }
        self.redraw_changed(|s| s.shade(w, on));
    }

    fn shade(&mut self, w: WindowId, on: bool) {
        self.cancel_interaction_for(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.shaded = on;
        let content = win.content;
        self.scene.set_enabled(content, !on);
        self.relayout(w);
        if self.focused == Some(w) {
            self.claim_keyboard(w);
        }
        let at = self.pointer_location();
        self.repoint(at, smithay::backend::input::InputTime::now());
    }

    /// Switch between our frame and the client's; the client area keeps its layout position.
    pub fn set_csd(&mut self, w: WindowId, csd: bool) {
        self.redraw_changed(|s| s.switch_csd(w, csd));
    }

    fn switch_csd(&mut self, w: WindowId, csd: bool) {
        if csd {
            self.set_shaded(w, false);
        }
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if csd == win.csd {
            return;
        }
        let (cx, cy) = win.client_origin();
        win.csd = csd;
        if !win.mapped {
            // A client may set its decoration after the initial commit (GTK3's KDE decoration);
            // re-plan the size for the frame it now gets.
            if win.placement.is_some() {
                self.configure_initial(w);
            }
            return;
        }
        self.relayout(w);
        if let Some(c) = self.win(w).map(|x| x.geom.client) {
            self.move_to(w, cx - c.x, cy - c.y);
        }
    }

    /// Hide or show the frame at the user's request; the outer rect stays and the client resizes to
    /// fill it.
    pub fn set_frameless(&mut self, w: WindowId, on: bool) {
        if on {
            self.set_shaded(w, false);
        }
        let m = self.config.theme.metrics.clone();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.frameless || win.csd || win.fullscreen || win.override_redirect {
            return;
        }
        let total = win.frame_rect();
        win.frameless = on;
        if !win.mapped {
            return;
        }
        let maximized = win.maximized;
        let (cw, ch) = FrameGeometry::client_size_for_total(
            &m,
            total.width,
            total.height,
            false,
            win.chrome(),
        );
        self.cancel_interaction_for(w);
        if maximized {
            self.fit_maximized(w);
            return;
        }
        // Relayout before the size request so an X11 configure carries the new position.
        self.relayout(w);
        self.request_size(w, cw, ch);
    }

    pub fn set_maximized(&mut self, w: WindowId, on: bool) {
        let w = self.tabs.groups.shown_of(w);
        let Some(win) = self.win(w) else {
            return;
        };
        if on == win.maximized {
            return;
        }
        if on && (win.is_owned() || !win.is_resizable()) {
            return;
        }
        // Fullscreen owns the geometry; ignored until it ends.
        if win.fullscreen {
            return;
        }
        if on {
            self.set_shaded(w, false);
        }
        self.cancel_interaction_for(w);
        self.toplevels.dirty = true;
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let restore = win.enter(Change::Maximize(on));
        let restore = restore.map(|r| self.restore_to(w, r));
        if on {
            self.fit_maximized(w);
        } else {
            self.relayout(w);
            match restore {
                Some(size) => self.restore_size(w, size),
                None => self.send_configure(w),
            }
        }
    }

    /// Ask `w` for its normal client size `(cw, ch)`, or for one of its own choosing when it
    /// never had one (`Window::own_size`).
    fn restore_size(&mut self, w: WindowId, (cw, ch): (i32, i32)) {
        if !self
            .win_mut(w)
            .is_some_and(|x| std::mem::take(&mut x.own_size))
        {
            return self.request_size(w, cw, ch);
        }
        if let Some(t) = self.win(w).and_then(Window::xdg) {
            t.with_pending_state(|s| s.size = None);
        }
        self.send_configure(w);
    }

    pub fn send_configure(&self, w: WindowId) {
        if let Some(t) = self.win(w).and_then(Window::xdg)
            && t.is_initial_configure_sent()
        {
            t.send_pending_configure();
        }
    }

    pub fn fit_maximized(&mut self, w: WindowId) {
        let out = self.fit_target(w);
        let m = self.config.theme.metrics.clone();
        let Some(win) = self.win_mut(w) else {
            return;
        };
        win.x = out.x;
        win.y = out.y;
        let (cw, ch) =
            FrameGeometry::client_size_for_total(&m, out.width, out.height, true, win.chrome());
        self.relayout(w);
        self.request_size(w, cw, ch);
    }

    pub fn fit_target(&self, w: WindowId) -> Rect {
        if self.win(w).is_some_and(|x| x.fullscreen) {
            self.output_box_for_window(w)
        } else {
            self.usable_area_for_window(w)
        }
    }

    /// The work areas changed: maximized, fullscreen and snapped windows refit to theirs, every
    /// xdg toplevel hears its new bounds, and a normal one that no longer fits them is asked to
    /// and moves inside its work area.
    pub fn refit_maximized(&mut self) {
        let ids: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
        let mut bounded = Vec::new();
        for w in ids {
            let (changed, to) = self.set_bounds(w, true);
            if changed {
                bounded.push(w);
            }
            if let Some((x, y)) = to {
                self.move_to(w, x, y);
            }
        }
        let fits: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|w| w.mapped && (w.maximized || w.fullscreen))
            .map(|w| w.id)
            .collect();
        for w in fits {
            if self.win(w).map(Window::frame_rect) != Some(self.fit_target(w)) {
                self.fit_maximized(w);
            }
        }
        self.refit_snapped();
        // A window refit above already sent its bounds with its size; this sends the rest.
        for w in bounded {
            self.send_configure(w);
        }
    }

    pub fn set_fullscreen(&mut self, w: WindowId, on: bool) {
        self.set_fullscreen_on(w, on, None);
    }

    /// [`State::set_fullscreen`] filling `output` when it is one [`State::fullscreen_target`]
    /// takes, else the window's own output.
    pub fn set_fullscreen_on(&mut self, w: WindowId, on: bool, output: Option<Output>) {
        if self.win(w).is_none_or(|x| x.fullscreen == on) {
            return;
        }
        if on {
            self.set_shaded(w, false);
        }
        self.cancel_interaction_for(w);
        self.toplevels.dirty = true;
        let target = output
            .filter(|o| self.fullscreen_target(o))
            .map(|o| self.layout_box(Some(&o)));
        let from = self.fit_output(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let saved = win.enter(Change::Fullscreen(on));
        if on {
            if let Some(b) = target {
                win.fullscreen_from = from.filter(|_| win.maximized);
                (win.x, win.y) = (b.x, b.y);
            }
            self.fit_maximized(w);
        } else if win.maximized {
            let back = win.fullscreen_from.take();
            let back = back.filter(|o| self.fullscreen_target(o));
            if let Some(o) = back {
                let area = self.usable_area(Some(&o));
                if let Some(win) = self.win_mut(w) {
                    (win.x, win.y) = (area.x, area.y);
                }
            }
            self.fit_maximized(w);
        } else {
            let restore = saved.map(|r| self.restore_to(w, r));
            self.relayout(w);
            match restore {
                Some(size) => self.restore_size(w, size),
                None => self.send_configure(w),
            }
        }
        self.restack();
        crate::idle::inhibition_changed(self);
    }

    /// Sets xdg-shell's `suspended` on `w` when it is hidden (minimized or a tab behind another),
    /// no live picture shows it (Mini View, a task button's panel, Alt+Tab) and no panel is about
    /// to: a suspended client may stop drawing. Returns whether the state changed; the caller
    /// sends the configure.
    pub(crate) fn mark_suspended(&self, w: WindowId) -> bool {
        let Some(win) = self.win(w) else {
            return false;
        };
        let want =
            win.hidden() && !self.live_previews.shows(w) && !self.task_preview.keeps_awake(w);
        let Some(t) = win.xdg() else {
            return false;
        };
        t.with_pending_state(|p| {
            if p.states.contains(xdg_toplevel::State::Suspended) == want {
                return false;
            }
            match want {
                true => p.states.set(xdg_toplevel::State::Suspended),
                false => p.states.unset(xdg_toplevel::State::Suspended),
            };
            true
        })
    }

    pub fn set_minimized(&mut self, w: WindowId, on: bool) {
        self.redraw_changed(|s| s.hold_taskbars(|s| s.minimize(w, on, true)));
    }

    /// `w`'s client restores it (X11 `WM_CHANGE_STATE`): focused as `takes_focus_on_restore`
    /// allows, else behind the focused window with its task button flagged.
    pub fn restore_on_request(&mut self, w: WindowId) {
        if !self.win(w).is_some_and(|x| x.minimized) {
            return;
        }
        let window = self
            .focused
            .filter(|&f| self.win(f).is_some_and(|f| f.mapped && !f.hidden()))
            .map(|f| crate::mapfocus::Holder {
                pid: self.client_pid(f),
                ..Default::default()
            });
        let reprise = self.dialogs.active || self.bar_shared.keys.is_some();
        let takes = crate::mapfocus::takes_focus_on_restore(
            self.desktop_locked(),
            self.on_current_desktop(w),
            self.client_pid(w),
            crate::mapfocus::holder(reprise, window),
        );
        if takes {
            self.set_minimized(w, false);
            return;
        }
        log::info!("window {w:?} restores itself behind the focused one");
        self.redraw_changed(|s| s.hold_taskbars(|s| s.minimize(w, false, false)));
        self.ask_attention(w);
    }

    /// `focus`: a restored `w` takes the focus, else it goes behind the focused window.
    fn minimize(&mut self, w: WindowId, on: bool, focus: bool) {
        let w = if on { self.tabs.groups.shown_of(w) } else { w };
        let shown = self.ripple_frame(w);
        self.toplevels.dirty = true;
        let Some(win) = self.win_mut(w) else {
            return;
        };
        if on == win.minimized {
            return;
        }
        if on && win.is_owned() {
            return;
        }
        win.enter(Change::Minimize(on));
        let (tree, mapped) = (win.tree, win.mapped);
        let here = self.on_current_desktop(w);
        self.scene.set_enabled(tree, !on && mapped && here);
        let ring = if on { shown } else { self.ripple_frame(w) };
        if let Some(r) = ring {
            self.water_ring(r);
        }
        self.show_children(w, !on && here);
        self.mark_suspended(w);
        self.send_configure(w);
        crate::idle::inhibition_changed(self);
        if on {
            self.set_activated(w, false);
            if (self.focused).is_some_and(|f| f == w || self.is_owner_of(w, f)) {
                self.unfocus();
                self.focus_top();
            }
        } else if focus {
            self.focus_window(w);
        } else {
            self.place_behind_focus(w);
        }
        self.redraw_taskbars();
    }

    /// A normal window that took a size of its own after acking every configure keeps it in its
    /// pending state, so a later state-only configure (activation, bounds) does not ask it back
    /// to the size last sent.
    pub(super) fn keep_own_size(&self, w: WindowId) {
        let Some(win) = self.win(w) else {
            return;
        };
        let held = win.maximized || win.fullscreen || win.snapped.is_some();
        if held
            || win.resize_anchor.is_some()
            || self.interaction.is_resize_of(w)
            || self.wm.snap.follows_drag(w)
        {
            return;
        }
        let Some(t) = win.xdg().filter(|t| !t.has_pending_changes()) else {
            return;
        };
        let own = win.client_size();
        let acked = t.with_committed_state(|s| s.cloned());
        t.with_pending_state(|s| {
            if acked.as_ref() == Some(&*s) && s.size.is_some_and(|z| (z.w, z.h) != own) {
                s.size = Some(own.into());
            }
        });
    }

    /// Drop minimized/maximized/fullscreen/snapped state on unmap. An xdg toplevel takes the
    /// cleared state with its next initial configure; X11's `_NET_WM_STATE` clears at once.
    pub(super) fn reset_state(&mut self, w: WindowId) {
        self.toplevels.dirty = true;
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let was_minimized = win.minimized;
        win.enter(Change::Reset);
        if std::mem::take(&mut win.shaded) {
            let content = win.content;
            self.scene.set_enabled(content, true);
        }
        if was_minimized {
            self.show_children(w, self.on_current_desktop(w));
        }
    }

    pub fn set_resizing(&mut self, w: WindowId, on: bool) {
        if let Some(t) = self.win(w).and_then(Window::xdg) {
            t.with_pending_state(|s| {
                if on {
                    s.states.set(xdg_toplevel::State::Resizing);
                } else {
                    s.states.unset(xdg_toplevel::State::Resizing);
                }
            });
            self.send_configure(w);
        }
        if !on {
            self.follow_tabs(w, true);
        }
    }
}

/// A move between window states; `Window::enter` makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Change {
    Maximize(bool),
    Fullscreen(bool),
    Minimize(bool),
    /// Back to no state, as the window unmaps.
    Reset,
}

impl Window {
    /// The state fields and the client's view of them for the `set_*` calls and `reset_state`.
    /// Maximizing or going fullscreen drops the snap; fullscreen keeps `maximized` underneath and
    /// saves the frame only over a normal window. Returns the frame a state that ends gives back.
    pub(super) fn enter(&mut self, change: Change) -> Option<Rect> {
        let back = match change {
            Change::Maximize(on) => {
                self.snapped = None;
                self.resize_anchor = None;
                self.maximized = on;
                let back = self.saved.take();
                if on {
                    self.saved = Some(self.frame_rect());
                }
                back.filter(|_| !on)
            }
            Change::Fullscreen(on) => {
                self.snapped = None;
                self.resize_anchor = None;
                self.fullscreen = on;
                match (on, self.maximized) {
                    (true, false) => {
                        self.saved_fullscreen = Some(self.frame_rect());
                        None
                    }
                    (false, false) => self.saved_fullscreen.take(),
                    (_, true) => None,
                }
            }
            Change::Minimize(on) => {
                self.minimized = on;
                None
            }
            Change::Reset => {
                self.minimized = false;
                self.fullscreen = false;
                self.maximized = false;
                self.saved = None;
                self.saved_fullscreen = None;
                self.fullscreen_from = None;
                self.snapped = None;
                None
            }
        };
        self.tell(change);
        back
    }

    fn tell(&self, change: Change) {
        match &self.kind {
            Kind::Xdg(t) => t.with_pending_state(|s| {
                let mut put = |state, on| match on {
                    true => s.states.set(state),
                    false => s.states.unset(state),
                };
                match change {
                    Change::Maximize(on) => put(xdg_toplevel::State::Maximized, on),
                    Change::Fullscreen(on) => put(xdg_toplevel::State::Fullscreen, on),
                    Change::Minimize(_) => false,
                    Change::Reset => {
                        put(xdg_toplevel::State::Maximized, false);
                        put(xdg_toplevel::State::Fullscreen, false);
                        put(xdg_toplevel::State::Suspended, false);
                        s.size = None;
                        false
                    }
                };
            }),
            Kind::X11(x) => {
                let told = match change {
                    Change::Maximize(on) => x.set_maximized(on),
                    Change::Fullscreen(on) => x.set_fullscreen(on),
                    Change::Minimize(on) => x.set_hidden(on),
                    Change::Reset => x
                        .set_maximized(false)
                        .and_then(|()| x.set_fullscreen(false))
                        .and_then(|()| x.set_hidden(false)),
                };
                if let Err(e) = told {
                    log::debug!("x11 state {change:?}: {e}");
                }
            }
        }
    }
}
