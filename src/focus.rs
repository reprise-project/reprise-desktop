//! Keyboard focus and window stacking: the active window, the surface that
//! holds the seat's keyboard, and each window's scene tree.

use smithay::utils::SERIAL_COUNTER;

use crate::compositor::State;
use crate::mapfocus;
use crate::window::WindowId;

impl State {
    /// Focus the top-most mapped, non-minimized window on the current desktop. A
    /// Reprise dialog or an exclusive layer surface with the keyboard keeps it.
    pub fn focus_top(&mut self) {
        let top = next_focus(
            self.dialogs.active,
            crate::layer::holds_keyboard(self),
            &self.focus_order,
            |p| self.win(p).is_some_and(|p| p.mapped && !p.hidden()) && self.on_current_desktop(p),
        );
        match top {
            Next::Keep => {}
            Next::Layer => {
                if let Some(s) = self.layers_shell.focused.clone() {
                    crate::layer::focus_layer(self, &s);
                }
            }
            Next::Window(w) => self.focus_window(w),
            Next::Nobody => {
                self.enter_keyboard(None);
                self.restack();
            }
        }
    }

    /// A press on the desktop gives it the keyboard: dialogs and the focused window
    /// go inactive and the seat's keyboard clears, so keys reach the icons
    /// (`keyroute`). An exclusive layer surface keeps the keyboard.
    pub fn focus_desktop(&mut self) {
        if crate::layer::holds_keyboard(self) {
            return;
        }
        self.drop_window_focus();
        self.deactivate_dialogs();
    }

    /// The focused window goes inactive and leaves the seat's keyboard. An
    /// exclusive layer surface keeps the seat.
    pub fn drop_window_focus(&mut self) {
        if !crate::layer::holds_keyboard(self) {
            self.enter_keyboard(None);
            self.layers_shell.focused = None;
        }
        if let Some(w) = self.focused.take() {
            self.set_activated(w, false);
            self.redraw_frame(w);
            self.redraw_taskbars();
            self.redraw_all();
        }
    }

    /// Put each window's tree in its layer: `layers.fullscreen` (the fullscreen
    /// `bind_window`), `layers.on_top` (also X11 menus and tooltips) or
    /// `layers.windows`. Walks all windows: one that just left fullscreen or on top
    /// has its flag cleared and still has to come down. Reparenting lands a tree on
    /// top of its new parent, so this runs before the focused window is raised.
    pub fn restack(&mut self) {
        let top = self.bind_window();
        let all: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
        for w in all {
            let Some(win) = self.win(w) else {
                continue;
            };
            let lifted = win.fullscreen && top == Some(w);
            let want = if lifted {
                self.layers.fullscreen
            } else if win.override_redirect || self.stays_on_top(w) {
                self.layers.on_top
            } else {
                self.layers.windows
            };
            let tree = win.tree;
            if self.scene.parent(tree) != Some(want) {
                self.scene.reparent(tree, want);
            }
        }
        crate::autohideview::poke(self);
    }

    /// Focus and raise `w`, showing its desktop first. A window under a modal
    /// dialog comes up with it, and the dialog takes the focus.
    pub fn focus_window(&mut self, owner: WindowId) {
        // A tab its group hides (or a window it owns) shows in the group's place
        // first, coming up with the group if minimized.
        let home = self.home(owner);
        if self.tabs.groups.is_hidden(home) {
            self.select_tab(home);
            if self.win(home).is_some_and(|x| x.minimized) {
                self.set_minimized(home, false);
                return;
            }
        }
        if let Some(d) = self.desktop_of(owner)
            && d != self.desktops.current
        {
            self.show_desktop(d);
        }
        if let Some(win) = self.win_mut(owner) {
            win.attention = false;
        }
        let w = self.modal_target(owner);
        if w != owner {
            if let Some(tree) = self.win(owner).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
            self.raise_children(owner);
        }
        let Some(tree) = self.win(w).map(|x| x.tree) else {
            return;
        };
        self.deactivate_dialogs();
        if self.focused == Some(w) {
            self.scene.raise_to_top(tree);
            self.raise_children(w);
            // Already active, but something may have taken the keyboard since; the seat
            // is the source of truth.
            let surface = self.win(w).map(|x| x.wl_surface().clone());
            let held = self.seat.get_keyboard().and_then(|k| k.current_focus());
            if held != surface {
                self.claim_keyboard(w);
            }
            self.redraw_all();
            return;
        }
        let prev = self.focused;
        self.focused = Some(w);
        self.focus_order.retain(|p| *p != w);
        self.focus_order.push(w);
        self.restack();
        self.scene.raise_to_top(tree);
        self.raise_children(w);
        if let Some(prev) = prev {
            if let Some(p) = self.win_mut(prev) {
                p.launch = None;
            }
            self.set_activated(prev, false);
            self.redraw_frame(prev);
        }
        self.set_activated(w, true);
        self.redraw_frame(w);
        self.claim_keyboard(w);
        self.redraw_taskbars();
        self.redraw_all();
    }

    /// Whether `w`, mapping, takes the keyboard (`mapfocus::takes_focus`);
    /// `activated` is the trusted activation presented before the map. Uses up its
    /// launch either way; the window holds it until the focus first leaves it, and
    /// hands it back if it withdraws before that (`forget_window`).
    pub fn takes_focus_on_map(&mut self, w: WindowId, activated: bool) -> bool {
        // Every X11 window is Xwayland's client, so an X11 window has no
        // client to compare.
        let client =
            |s: &State, w: WindowId| s.win(w).and_then(|x| x.wl_client()).map(|c| client_key(&c));
        let pid = |s: &State, w: WindowId| s.client_pid(w);
        let new_pid = pid(self, w);
        let (lineage, session) = new_pid.map(mapfocus::lineage).unwrap_or_default();
        let launch = mapfocus::take_recorded_launch(&lineage, session);
        if let Some(win) = self.win_mut(w) {
            win.launch = launch;
        }
        let new = mapfocus::Arrival {
            activated,
            client: client(self, w),
            lineage: &lineage,
            launched: launch.is_some(),
            prompt: new_pid.is_some_and(mapfocus::is_prompt),
        };
        let window = self
            .focused
            .filter(|&f| self.win(f).is_some_and(|f| f.mapped && !f.hidden()))
            .map(|f| mapfocus::Holder {
                client: client(self, f),
                pid: pid(self, f),
                owns: self.is_owner_of(f, w),
            });
        let holder = mapfocus::holder(self.dialogs.active, window);
        let takes = mapfocus::takes_focus(&new, holder);
        if !takes {
            log::info!(
                "window {w:?} ({:?}, pid {new_pid:?}) maps behind the focused one",
                self.win(w).map(|x| x.app_id()).unwrap_or_default()
            );
        }
        takes
    }

    /// Stack `w` just under the focused window and everything it owns, and next
    /// after it in `focus_order`, so `w` is focused when that one goes.
    pub fn place_behind_focus(&mut self, w: WindowId) {
        let Some(f) = self.focused else {
            return;
        };
        // Bounded: a parent chain is the client's to loop.
        let mut root = f;
        for _ in 0..self.windows.len() {
            match self.parent_window(root) {
                Some(p) => root = p,
                None => break,
            }
        }
        let (Some(node), Some(below)) =
            (self.win(w).map(|x| x.tree), self.win(root).map(|x| x.tree))
        else {
            return;
        };
        // A fullscreen family sits in a tree above every window already.
        if root != w && self.scene.parent(node) == self.scene.parent(below) {
            self.scene.place_below(node, below);
        }
        let order = &mut self.focus_order;
        order.retain(|p| *p != w);
        let at = order.iter().position(|p| *p == f).unwrap_or(order.len());
        order.insert(at, w);
    }

    /// Move the seat's keyboard to `w`. An exclusive layer surface keeps it
    /// instead: the window still draws as active.
    pub fn claim_keyboard(&mut self, w: WindowId) {
        if self.is_locked() || crate::layer::holds_keyboard(self) {
            return;
        }
        let surface = self.win(w).map(|x| x.wl_surface().clone());
        if surface.is_some() {
            self.layers_shell.focused = None;
            self.enter_keyboard(surface);
        }
    }

    /// Send the seat's keyboard focus to `surface`, or take it from every surface
    /// with None. While locked, every surface but the locker's own is ignored.
    pub fn enter_keyboard(
        &mut self,
        surface: Option<smithay::reexports::wayland_server::protocol::wl_surface::WlSurface>,
    ) {
        if let Some(s) = &surface
            && self.is_locked()
            && !crate::sessionlock::is_lock_surface(self, s)
        {
            return;
        }
        // Whoever takes the keyboard takes it from the taskbar.
        if surface.is_some() {
            self.end_bar_keys();
        }
        log::debug!("keyboard enter {surface:?}");
        if let Some(keyboard) = self.seat.get_keyboard() {
            if keyboard.current_focus() != surface {
                // A moved keyboard closes an open X11 menu, as a press elsewhere would.
                self.handle.insert_idle(crate::xwayland::menus::dismiss);
            }
            self.flush_motion();
            keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Next<T> {
    Keep,
    Layer,
    Window(T),
    Nobody,
}

/// `focus_top`'s choice: a dialog with the keyboard (`dialog`) keeps it,
/// then an exclusive layer surface (`layer`), then the last window in
/// `order` that is `shown`.
fn next_focus<T: Copy>(
    dialog: bool,
    layer: bool,
    order: &[T],
    shown: impl Fn(T) -> bool,
) -> Next<T> {
    if dialog {
        Next::Keep
    } else if layer {
        Next::Layer
    } else {
        order
            .iter()
            .rev()
            .copied()
            .find(|&w| shown(w))
            .map_or(Next::Nobody, Next::Window)
    }
}

/// A client's connection as a number `mapfocus` compares.
fn client_key(c: &smithay::reexports::wayland_server::Client) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c.id().hash(&mut h);
    h.finish() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialog_with_the_keyboard_keeps_it_when_a_window_goes() {
        assert_eq!(next_focus(true, false, &[1, 2], |_| true), Next::Keep);
        assert_eq!(next_focus(false, false, &[1, 2], |_| true), Next::Window(2));
    }

    #[test]
    fn the_top_shown_window_takes_the_keyboard() {
        assert_eq!(next_focus(false, true, &[1, 2], |_| true), Next::Layer);
        assert_eq!(
            next_focus(false, false, &[1, 2], |w| w == 1),
            Next::Window(1)
        );
        assert_eq!(next_focus(false, false, &[1, 2], |_| false), Next::Nobody);
    }
}
