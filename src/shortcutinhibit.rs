//! keyboard-shortcuts-inhibit-unstable-v1: while a surface holds the seat's
//! keyboard and has an inhibitor, `keyroute` skips the binds and the Super tap.
//! Power, sleep and media keys, VT switching, the binds a dialog lets through
//! (the lock, the magnifier, screenshots) and open menus still work.
//! Super+Shift+Escape suspends the inhibitor until its surface next gains the
//! keyboard.

use smithay::{
    reexports::wayland_server::{
        DisplayHandle, Resource, backend::ClientId, protocol::wl_surface::WlSurface,
    },
    wayland::keyboard_shortcuts_inhibit::{
        KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
    },
};

use crate::compositor::State;

#[derive(Debug)]
pub struct Inhibit {
    pub state: KeyboardShortcutsInhibitState,
    tracked: Vec<KeyboardShortcutsInhibitor>,
    /// The inhibitor Super+Shift+Escape turned off; it stays off until its
    /// surface loses the keyboard.
    suspended: Option<KeyboardShortcutsInhibitor>,
    /// Clients told by a balloon that they took the shortcuts.
    told: Vec<ClientId>,
}

impl Inhibit {
    pub fn new(dh: &DisplayHandle) -> Inhibit {
        Inhibit {
            state: KeyboardShortcutsInhibitState::new::<State>(dh),
            tracked: Vec::new(),
            suspended: None,
            told: Vec::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.tracked.iter().any(|i| i.is_active())
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
        if inhibit.suspended.as_ref() == Some(&inhibitor) {
            inhibit.suspended = None;
        }
        inhibit.tracked.retain(|i| *i != inhibitor);
    }
}

impl State {
    /// Activate the inhibitor on `focus`, the surface with the seat's keyboard,
    /// and deactivate the rest. Takes `focus` as an argument because
    /// `SeatHandler::focus_changed` runs before the seat holds it.
    pub fn refresh_inhibitors(&mut self, focus: Option<&WlSurface>) {
        let inhibit = &mut self.key_input.inhibit;
        if inhibit
            .suspended
            .as_ref()
            .is_some_and(|s| Some(s.wl_surface()) != focus)
        {
            inhibit.suspended = None;
        }
        let mut taken = Vec::new();
        for i in &inhibit.tracked {
            let want = Some(i.wl_surface()) == focus && inhibit.suspended.as_ref() != Some(i);
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

    /// Turn off the active inhibitors until their surface next gains the keyboard.
    pub fn suspend_inhibitors(&mut self) {
        let inhibit = &mut self.key_input.inhibit;
        for i in &inhibit.tracked {
            if i.is_active() {
                log::info!("shortcuts inhibitor on {:?} suspended", i.wl_surface());
                inhibit.suspended = Some(i.clone());
                i.inactivate();
            }
        }
    }
}
