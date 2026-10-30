//! `xdg_system_bell_v1`: a ring plays the `[sounds] theme`'s bell unless `[sounds] bell` is off,
//! and flags its window when unfocused.

use std::time::{Duration, Instant};

use smithay::{
    reexports::wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    wayland::{
        compositor::get_parent,
        xdg_system_bell::{XdgSystemBellHandler, XdgSystemBellState},
    },
};

use crate::compositor::State;
use crate::window::WindowId;

/// Rings closer together than this after a sounded one stay silent, so a client ringing in a loop
/// costs one sound lookup and thread per gap.
const MIN_GAP: Duration = Duration::from_millis(100);

/// XDG sound theme name; `sounds::resolve` falls back to the theme's `bell`.
const SOUND: &str = "bell-window-system";

pub struct Bell {
    _global: XdgSystemBellState,
    last: Option<Instant>,
}

impl Bell {
    pub fn new(dh: &DisplayHandle) -> Self {
        Self {
            _global: XdgSystemBellState::new::<State>(dh),
            last: None,
        }
    }

    /// Whether a ring at `now` sounds, recording it if so.
    fn sounds(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|t| now.saturating_duration_since(t) < MIN_GAP)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

impl State {
    /// The window a ring for `surface` asks attention for: its toplevel when mapped and not focused.
    fn bell_window(&self, surface: &WlSurface) -> Option<WindowId> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        let w = self
            .window_for_surface(&root)
            .or_else(|| self.window_for_popup(&root))?;
        let mapped = self.win(w).is_some_and(|win| win.mapped);
        (mapped && self.focused != Some(w)).then_some(w)
    }

    /// A ring for `surface` at `now`: flags its window and says whether the bell sounds.
    fn ring_at(&mut self, surface: Option<WlSurface>, now: Instant) -> bool {
        if let Some(w) = surface.and_then(|s| self.bell_window(&s)) {
            self.ask_attention(w);
            self.redraw_taskbars();
        }
        self.config.sounds.bell && self.globals.system_bell.sounds(now)
    }
}

impl XdgSystemBellHandler for State {
    fn ring(&mut self, surface: Option<WlSurface>) {
        if self.ring_at(surface, Instant::now()) {
            let theme = self.config.sounds.theme.clone();
            repriseaudio::sounds::play(repriseaudio::sounds::Sound::Name(SOUND.into()), theme);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    #[test]
    fn a_ring_within_the_gap_of_a_sounded_one_is_silent() {
        let mut h = Harness::new();
        let bell = &mut h.state.globals.system_bell;
        let t = Instant::now();
        assert!(bell.sounds(t), "the first ring sounds");
        assert!(!bell.sounds(t + MIN_GAP / 2), "inside the gap");
        assert!(bell.sounds(t + MIN_GAP), "the gap has passed");
        assert!(
            !bell.sounds(t + MIN_GAP + MIN_GAP / 2),
            "the gap runs from the last sounded ring"
        );
    }

    #[test]
    fn a_ring_flags_its_unfocused_window_and_not_the_focused_one() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "test.a", 300, 200);
        let b = h.map_toplevel(&mut c, "test.b", 300, 200);
        h.dispatch();
        assert_eq!(h.state.focused, Some(b.id));
        let surface = |id| h.state.win(id).unwrap().wl_surface().clone();
        let (sa, sb) = (surface(a.id), surface(b.id));
        assert_eq!(h.state.bell_window(&sa), Some(a.id));
        assert_eq!(h.state.bell_window(&sb), None, "the focused window");
    }

    #[test]
    fn with_the_bell_off_a_ring_is_silent_and_still_flags_its_window() {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "test.a", 300, 200);
        h.map_toplevel(&mut c, "test.b", 300, 200);
        h.dispatch();
        h.state.config.sounds.bell = false;
        let sa = h.state.win(a.id).unwrap().wl_surface().clone();
        assert!(!h.state.ring_at(Some(sa), Instant::now()), "no sound");
        assert!(h.state.win(a.id).unwrap().attention.is_some(), "a flashes");
    }
}
