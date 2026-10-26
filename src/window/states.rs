use reprisekit::classic::frame::FrameGeometry;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;

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

    /// Switch between our frame and the client's; the client area keeps its layout position.
    pub fn set_csd(&mut self, w: WindowId, csd: bool) {
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
        self.cancel_interaction_for(w);
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
                Some((cw, ch)) => self.request_size(w, cw, ch),
                None => self.send_configure(w),
            }
        }
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

    /// The work areas changed: maximized, fullscreen and snapped windows refit to theirs, and
    /// every xdg toplevel hears its new bounds.
    pub fn refit_maximized(&mut self) {
        let bounded: Vec<WindowId> = (self.windows.iter())
            .map(|w| w.id)
            .filter(|&w| self.set_bounds(w))
            .collect();
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
        if self.win(w).is_none_or(|x| x.fullscreen == on) {
            return;
        }
        self.cancel_interaction_for(w);
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let saved = win.enter(Change::Fullscreen(on));
        if on || win.maximized {
            self.fit_maximized(w);
        } else {
            let restore = saved.map(|r| self.restore_to(w, r));
            self.relayout(w);
            match restore {
                Some((cw, ch)) => self.request_size(w, cw, ch),
                None => self.send_configure(w),
            }
        }
        self.restack();
        crate::idle::inhibition_changed(self);
    }

    pub fn set_minimized(&mut self, w: WindowId, on: bool) {
        let w = if on { self.tabs.groups.shown_of(w) } else { w };
        let shown = self.ripple_frame(w);
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
        crate::idle::inhibition_changed(self);
        if on {
            self.set_activated(w, false);
            if self.focused == Some(w) {
                self.focused = None;
                self.focus_top();
            }
        } else {
            self.focus_window(w);
        }
        self.redraw_taskbars();
        self.redraw_all();
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
        let Some(win) = self.win_mut(w) else {
            return;
        };
        let was_minimized = win.minimized;
        win.enter(Change::Reset);
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
