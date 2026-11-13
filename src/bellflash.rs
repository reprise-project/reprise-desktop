//! `[accessibility] visual_bell`: a ring flashes the window that rang, or every screen, white
//! fading out. A window that can't be seen (minimized, on another desktop, or no window at all)
//! flashes the screens instead.

use std::time::{Duration, Instant};

use reprisescene::{Input, NodeId, RectId};
use smithay::output::Output;

use crate::compositor::State;
use crate::config::VisualBell;
use crate::window::WindowId;

const FLASH: Duration = Duration::from_millis(300);
/// The white's alpha as a flash starts.
const PEAK: f32 = 0.5;
/// Flashes start at least this far apart, so a bell rung in a loop flashes at most three times a
/// second.
const MIN_GAP: Duration = Duration::from_millis(400);

/// The flash on the scene: its rects and when it started.
#[derive(Default)]
pub struct BellFlash {
    rects: Vec<RectId>,
    /// The window flashing, None for the screens.
    window: Option<WindowId>,
    since: Option<Instant>,
}

/// The white at `elapsed` into a flash, premultiplied; None once it is over.
fn color(elapsed: Duration) -> Option<[f32; 4]> {
    (elapsed < FLASH).then(|| {
        let a = PEAK * (1.0 - elapsed.as_secs_f32() / FLASH.as_secs_f32());
        [a, a, a, a]
    })
}

impl State {
    /// A ring from `window` (None: no window, or one not on screen) at `now`: flash as
    /// `visual_bell` says, unless the last flash started within [`MIN_GAP`].
    pub fn flash_bell(&mut self, window: Option<WindowId>, now: Instant) {
        let mode = self.config.accessibility.visual_bell;
        if mode == VisualBell::None
            || (self.bell_flash.since).is_some_and(|t| now.saturating_duration_since(t) < MIN_GAP)
        {
            return;
        }
        end(self);
        let shown = window.filter(|&w| mode == VisualBell::Window && self.flashable(w));
        let Some(start) = color(Duration::ZERO) else {
            return;
        };
        let rects = match shown {
            Some(w) => self.flash_window(w, start),
            None => self.flash_screens(start),
        };
        if rects.is_empty() {
            return;
        }
        self.bell_flash = BellFlash {
            rects,
            window: shown,
            since: Some(now),
        };
        redraw(self);
    }

    fn flashable(&self, w: WindowId) -> bool {
        self.win(w).is_some_and(|win| win.mapped)
            && !self.minimized_along_chain(w)
            && self.on_current_desktop(w)
            && !self.window_locked(w)
    }

    /// A rect over `w`'s whole frame, above its client and popups.
    fn flash_window(&mut self, w: WindowId, color: [f32; 4]) -> Vec<RectId> {
        let Some(win) = self.win(w) else {
            return Vec::new();
        };
        let (tree, total) = (win.tree, win.geom.total);
        let r = self.scene.rect_create(tree, total, color);
        self.scene.set_input(r, Input::Passthrough);
        self.scene.set_in_pictures(r, false);
        vec![r]
    }

    /// A rect over each unlocked output, above the windows, panels and menus.
    fn flash_screens(&mut self, color: [f32; 4]) -> Vec<RectId> {
        let outputs: Vec<Output> = (self.scene.outputs().iter())
            .filter(|o| !self.locked_on(o))
            .cloned()
            .collect();
        let mut rects = Vec::new();
        for o in &outputs {
            let b = self.layout_box(Some(o));
            if b.width <= 0 {
                continue;
            }
            let r = (self.scene).rect_create(self.layers.overlay, (b.width, b.height), color);
            self.scene.set_position(r, b.x, b.y);
            self.scene.set_input(r, Input::Passthrough);
            self.scene.set_in_pictures(r, false);
            rects.push(r);
        }
        rects
    }
}

fn redraw(state: &mut State) {
    let flash = &state.bell_flash;
    match flash.window.and_then(|w| state.win(w)).map(|w| w.tree) {
        Some(tree) => state.redraw_nodes(Vec::new(), &[NodeId::from(tree)]),
        None => state.redraw_all(),
    }
}

/// Before a frame: fade the flash, or take it away once it is over.
pub fn frame(state: &mut State) {
    frame_at(state, Instant::now());
}

fn frame_at(state: &mut State, now: Instant) {
    let Some(since) = state
        .bell_flash
        .since
        .filter(|_| !state.bell_flash.rects.is_empty())
    else {
        return;
    };
    match color(now.saturating_duration_since(since)) {
        Some(c) => {
            for &r in &state.bell_flash.rects {
                state.scene.rect_set_color(r, c);
            }
            redraw(state);
        }
        None => {
            redraw(state);
            end(state);
        }
    }
}

/// Take the flash's rects off the scene; its start time stays for [`MIN_GAP`].
fn end(state: &mut State) {
    for r in std::mem::take(&mut state.bell_flash.rects) {
        state.scene.destroy(r);
    }
    state.bell_flash.window = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    fn ringing(mode: VisualBell) -> (Harness, crate::harness::TestClient, WindowId) {
        let mut h = Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let a = h.map_toplevel(&mut c, "test.a", 300, 200);
        h.dispatch();
        h.state.config.accessibility.visual_bell = mode;
        (h, c, a.id)
    }

    fn alpha(h: &Harness) -> Vec<f32> {
        (h.state.bell_flash.rects.iter())
            .filter_map(|&r| h.state.scene.rect_color(r).map(|c| c[3]))
            .collect()
    }

    #[test]
    fn the_flash_starts_at_half_white_fades_and_goes() {
        assert_eq!(color(Duration::ZERO), Some([0.5; 4]));
        let half = color(FLASH / 2).unwrap();
        assert!((half[3] - 0.25).abs() < 1e-4, "{half:?}");
        assert_eq!(color(FLASH), None);
    }

    #[test]
    fn the_window_mode_covers_the_frame_of_the_window_that_rang() {
        let (mut h, _c, a) = ringing(VisualBell::Window);
        let t0 = Instant::now();
        h.state.flash_bell(Some(a), t0);
        let win = h.state.win(a).unwrap();
        let [r] = h.state.bell_flash.rects[..] else {
            panic!("one rect");
        };
        assert_eq!(
            h.state.scene.parent(r),
            Some(win.tree),
            "in the window's tree"
        );
        let size = h.state.scene.rect_size(r).unwrap();
        assert_eq!((size.w, size.h), win.geom.total, "the whole frame");
        frame_at(&mut h.state, t0 + FLASH / 2);
        assert!((alpha(&h)[0] - 0.25).abs() < 1e-4);
        frame_at(&mut h.state, t0 + FLASH);
        assert!(h.state.bell_flash.rects.is_empty(), "over");
        assert!(h.state.scene.rect_size(r).is_none(), "taken off the scene");
    }

    #[test]
    fn a_minimized_window_or_no_window_flashes_the_screen() {
        let (mut h, _c, a) = ringing(VisualBell::Window);
        h.state.flash_bell(None, Instant::now());
        let [r] = h.state.bell_flash.rects[..] else {
            panic!("one output");
        };
        assert_eq!(h.state.scene.parent(r), Some(h.state.layers.overlay));
        let size = h.state.scene.rect_size(r).unwrap();
        assert_eq!((size.w, size.h), (1024, 768));
        h.state.set_minimized(a, true);
        h.state.flash_bell(Some(a), Instant::now() + MIN_GAP);
        let [r] = h.state.bell_flash.rects[..] else {
            panic!("one output");
        };
        assert_eq!(h.state.scene.parent(r), Some(h.state.layers.overlay));
    }

    #[test]
    fn the_screen_mode_flashes_the_screen_for_a_window_too() {
        let (mut h, _c, a) = ringing(VisualBell::Screen);
        h.state.flash_bell(Some(a), Instant::now());
        let [r] = h.state.bell_flash.rects[..] else {
            panic!("one output");
        };
        assert_eq!(h.state.scene.parent(r), Some(h.state.layers.overlay));
    }

    #[test]
    fn rings_closer_than_the_gap_flash_once_and_none_flashes_nothing() {
        let (mut h, _c, a) = ringing(VisualBell::Window);
        let t0 = Instant::now();
        h.state.flash_bell(Some(a), t0);
        let first = h.state.bell_flash.rects.clone();
        h.state.flash_bell(Some(a), t0 + MIN_GAP / 2);
        assert_eq!(h.state.bell_flash.rects, first, "the same flash goes on");
        h.state.flash_bell(Some(a), t0 + MIN_GAP);
        assert_ne!(h.state.bell_flash.rects, first, "a new flash");
        assert_eq!(h.state.bell_flash.rects.len(), 1, "the old one is gone");

        let (mut h, _c, a) = ringing(VisualBell::None);
        h.state.flash_bell(Some(a), Instant::now());
        assert!(h.state.bell_flash.rects.is_empty());
    }
}
