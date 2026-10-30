//! keyboard-shortcuts-inhibit-unstable-v1 and xwayland-keyboard-grab-unstable-v1:
//! while a surface holds the seat's keyboard and has an inhibitor or an X11
//! keyboard grab, `keyroute` skips the binds and the Super tap. Power, sleep
//! and media keys, VT switching, the binds a dialog lets through (the lock,
//! the magnifier, screenshots) and open menus still work. Super+Shift+Escape
//! suspends both until their surface next gains the keyboard.

use smithay::{
    reexports::wayland_server::{
        DisplayHandle, Resource, backend::ClientId, protocol::wl_surface::WlSurface,
    },
    input::Seat,
    reexports::wayland_protocols::xwayland::keyboard_grab::zv1::server::zwp_xwayland_keyboard_grab_v1::ZwpXwaylandKeyboardGrabV1,
    wayland::{
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState,
            KeyboardShortcutsInhibitor,
        },
        xwayland_keyboard_grab::{XWaylandKeyboardGrab, XWaylandKeyboardGrabHandler},
    },
};

use crate::compositor::State;

#[derive(Debug)]
pub struct Inhibit {
    pub state: KeyboardShortcutsInhibitState,
    tracked: Vec<KeyboardShortcutsInhibitor>,
    /// Xwayland's keyboard grabs, each on the surface of the X11 window
    /// that called XGrabKeyboard. Seat focus still moves freely.
    grabs: Vec<(ZwpXwaylandKeyboardGrabV1, WlSurface)>,
    /// The surface with the keyboard, as of the last `refresh_inhibitors`.
    focus: Option<WlSurface>,
    /// The surface whose inhibitor or grab Super+Shift+Escape turned off; it
    /// stays off until the surface loses the keyboard.
    suspended: Option<WlSurface>,
    /// Clients told by a balloon that they took the shortcuts.
    told: Vec<ClientId>,
}

impl Inhibit {
    pub fn new(dh: &DisplayHandle) -> Inhibit {
        Inhibit {
            state: KeyboardShortcutsInhibitState::new::<State>(dh),
            tracked: Vec::new(),
            grabs: Vec::new(),
            focus: None,
            suspended: None,
            told: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.tracked.iter().any(|i| i.is_active()) || self.grab_holds()
    }

    fn grab_holds(&self) -> bool {
        let live = (self.grabs.iter()).filter(|(g, s)| g.is_alive() && s.is_alive());
        grab_holds(
            live.map(|(_, s)| s),
            self.focus.as_ref(),
            self.suspended.as_ref(),
        )
    }
}

/// Whether one of the `grabbed` surfaces holds the keyboard: it has the
/// focus and Super+Shift+Escape has not suspended it.
fn grab_holds<'a, S: PartialEq + 'a>(
    mut grabbed: impl Iterator<Item = &'a S>,
    focus: Option<&S>,
    suspended: Option<&S>,
) -> bool {
    focus.is_some_and(|f| suspended != Some(f) && grabbed.any(|s| s == f))
}

impl XWaylandKeyboardGrabHandler for State {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<WlSurface> {
        Some(surface.clone())
    }

    /// Records the grab instead of installing a seat grab, so the focus
    /// still moves on a click or Alt+Tab once suspended.
    fn grab(&mut self, surface: WlSurface, _seat: Seat<Self>, grab: XWaylandKeyboardGrab<Self>) {
        log::debug!("xwayland keyboard grab on {surface:?}");
        let inhibit = &mut self.key_input.inhibit;
        (inhibit.grabs).retain(|(g, s)| g.is_alive() && s.is_alive());
        inhibit.grabs.push((grab.grab().clone(), surface.clone()));
        if inhibit.grab_holds()
            && let Some(c) = surface.client().map(|c| c.id())
            && !inhibit.told.contains(&c)
        {
            inhibit.told.push(c);
            self.insert_idle_woken(move |state| state.tell_shortcuts_taken(&surface));
        }
    }
}

impl KeyboardShortcutsInhibitHandler for State {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.key_input.inhibit.state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        let dh = &self.dh;
        let inhibit = &mut self.key_input.inhibit;
        (inhibit.told).retain(|c| dh.backend_handle().get_client_data(c.clone()).is_ok());
        inhibit.tracked.push(inhibitor);
        let focus = self.seat.get_keyboard().and_then(|k| k.current_focus());
        self.refresh_inhibitors(focus.as_ref());
    }

    fn inhibitor_destroyed(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        let inhibit = &mut self.key_input.inhibit;
        inhibit.tracked.retain(|i| *i != inhibitor);
    }
}

impl State {
    /// Activate the inhibitor on `focus`, the surface with the seat's keyboard,
    /// and deactivate the rest. Takes `focus` as an argument because
    /// `SeatHandler::focus_changed` runs before the seat holds it.
    pub fn refresh_inhibitors(&mut self, focus: Option<&WlSurface>) {
        let inhibit = &mut self.key_input.inhibit;
        inhibit.focus = focus.cloned();
        if inhibit.suspended.as_ref() != focus {
            inhibit.suspended = None;
        }
        (inhibit.grabs).retain(|(g, s)| g.is_alive() && s.is_alive());
        let mut taken = Vec::new();
        for i in &inhibit.tracked {
            let want = Some(i.wl_surface()) == focus && inhibit.suspended.as_ref() != focus;
            if want && !i.is_active() {
                log::debug!("shortcuts inhibitor on {:?} active", i.wl_surface());
                i.activate();
                if let Some(c) = i.wl_surface().client().map(|c| c.id())
                    && !inhibit.told.contains(&c)
                {
                    inhibit.told.push(c);
                    taken.push(i.wl_surface().clone());
                }
            } else if !want && i.is_active() {
                log::debug!("shortcuts inhibitor on {:?} inactive", i.wl_surface());
                i.inactivate();
            }
        }
        // Shown outside the seat's locks: this runs inside a focus change.
        for surface in taken {
            self.insert_idle_woken(move |state| state.tell_shortcuts_taken(&surface));
        }
    }

    fn tell_shortcuts_taken(&mut self, surface: &WlSurface) {
        let name = (self.window_for_surface(surface).and_then(|w| self.win(w)))
            .map(|w| [w.title(), w.app_id()])
            .and_then(|names| names.into_iter().find(|n| !n.is_empty()))
            .unwrap_or_else(|| "A program".to_string());
        crate::notifyview::notice(
            self,
            &format!("{name} took the keyboard shortcuts"),
            "Super+Shift+Escape gives them back.",
        );
    }

    /// Turn off the active inhibitors and X11 grabs until their surface next
    /// gains the keyboard.
    pub fn suspend_inhibitors(&mut self) {
        let inhibit = &mut self.key_input.inhibit;
        if inhibit.grab_holds() {
            log::info!("xwayland keyboard grab on {:?} suspended", inhibit.focus);
            inhibit.suspended = inhibit.focus.clone();
        }
        for i in &inhibit.tracked {
            if i.is_active() {
                log::info!("shortcuts inhibitor on {:?} suspended", i.wl_surface());
                inhibit.suspended = Some(i.wl_surface().clone());
                i.inactivate();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::grab_holds;

    #[test]
    fn a_grab_holds_only_its_focused_unsuspended_surface() {
        let grabbed = [1, 2];
        assert!(grab_holds(grabbed.iter(), Some(&1), None));
        assert!(
            !grab_holds(grabbed.iter(), Some(&3), None),
            "focus on another surface"
        );
        assert!(!grab_holds(grabbed.iter(), None, None), "no focus");
        assert!(!grab_holds(grabbed.iter(), Some(&1), Some(&1)), "suspended");
        assert!(
            grab_holds(grabbed.iter(), Some(&2), Some(&1)),
            "another surface suspended"
        );
        assert!(!grab_holds([].iter(), Some(&1), None), "no grab");
    }
}
