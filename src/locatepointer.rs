//! `[accessibility] locate_pointer`: a tap of Ctrl alone rings the pointer (`reprisekit::locate`).
//! The rings sit in `layers.overlay`, let the pointer through, and follow it while they grow;
//! under `[effects] reduce_motion` they hold still for the same time.

use std::time::{Duration, Instant};

use reprisekit::locate::{self, SIZE};
use reprisescene::{BufferId, Input, NodeId};

use crate::compositor::State;
use crate::keys::{KEY_Control_L, KEY_Control_R};
use crate::output::upload_scaled;

const LENGTH: Duration = Duration::from_millis(900);
/// Where the still rings stop: both showing, the first further out.
const STILL_AT: f64 = 0.5;

/// Watches the keys for a lone Ctrl tap: Ctrl going down with no other key down, and up again
/// with no key pressed nor button, wheel, touch, pen or gesture used in between.
#[derive(Debug, Default)]
pub struct CtrlTap {
    down: Vec<u32>,
    armed: Option<u32>,
}

impl CtrlTap {
    /// Key `code` (Ctrl when `ctrl`) went down or up; true when it ends a lone tap.
    pub fn key(&mut self, code: u32, ctrl: bool, pressed: bool) -> bool {
        if pressed {
            let alone = self.down.iter().all(|&c| c == code);
            if !self.down.contains(&code) {
                self.down.push(code);
            }
            self.armed = (ctrl && alone).then_some(code);
            return false;
        }
        self.down.retain(|&c| c != code);
        self.armed.take_if(|&mut c| c == code).is_some()
    }

    /// A button, the wheel, a finger, a pen or a gesture was used: the Ctrl held goes with it.
    pub fn spoil(&mut self) {
        self.armed = None;
    }
}

/// The rings while they show.
#[derive(Default)]
pub struct Rings {
    node: Option<BufferId>,
    since: Option<Instant>,
    /// What the buffer holds: the animation's step and the scale it was painted at.
    drawn: Option<(u32, i32)>,
}

impl State {
    /// A key the typing aids let through (`syms` its keysyms): ring the pointer on a lone Ctrl tap.
    pub fn locate_key(&mut self, code: u32, syms: &[u32], pressed: bool) {
        let ctrl = syms
            .iter()
            .any(|&s| s == KEY_Control_L || s == KEY_Control_R);
        // A Ctrl tap under a held button or finger is a drag's copy or move switch.
        let held =
            self.buttons_down > 0 || !self.seat_buttons.is_empty() || self.devices.touch.any_down();
        if self.key_input.ctrl_tap.key(code, ctrl, pressed)
            && !held
            && self.config.accessibility.locate_pointer
            && !self.console_holds_input()
        {
            start(self, Instant::now());
        }
    }
}

fn start(state: &mut State, now: Instant) {
    stop(state);
    let node = state.scene.buffer_create(state.layers.overlay);
    state.scene.set_input(node, Input::Passthrough);
    state.scene.set_in_pictures(node, false);
    state.locate_rings = Rings {
        node: Some(node),
        since: Some(now),
        drawn: None,
    };
    frame_at(state, now);
}

fn stop(state: &mut State) {
    if let Some(node) = state.locate_rings.node.take() {
        let was = state.outputs_of_nodes(&[NodeId::from(node)]);
        state.scene.destroy(node);
        state.redraw_nodes(was, &[]);
    }
    state.locate_rings = Rings::default();
}

/// Before a frame: step the rings round where the pointer is now, or take them away at the end.
pub fn frame(state: &mut State) {
    frame_at(state, Instant::now());
}

fn frame_at(state: &mut State, now: Instant) {
    let (Some(node), Some(since)) = (state.locate_rings.node, state.locate_rings.since) else {
        return;
    };
    let elapsed = now.saturating_duration_since(since);
    if elapsed >= LENGTH {
        stop(state);
        return;
    }
    let t = match state.config.effects.reduce_motion {
        true => STILL_AT,
        false => elapsed.as_secs_f64() / LENGTH.as_secs_f64(),
    };
    let at = state.pointer_location();
    let output = state.output_at(at.x, at.y);
    let scale = state.draw_scale_at(output.as_ref()).max(1);
    let step = (t * 1000.0) as u32;
    let was = state.outputs_of_nodes(&[NodeId::from(node)]);
    let half = SIZE / 2;
    state
        .scene
        .set_position(node, at.x.floor() as i32 - half, at.y.floor() as i32 - half);
    if state.locate_rings.drawn != Some((step, scale)) {
        let canvas = locate::rings(t, state.config.theme.colors.hilight, scale);
        upload_scaled(&mut state.scene, node, &canvas, scale);
        state.locate_rings.drawn = Some((step, scale));
    }
    state.redraw_nodes(was, &[NodeId::from(node)]);
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: u32 = 37;
    const A: u32 = 38;

    fn tap(t: &mut CtrlTap, code: u32, ctrl: bool) -> bool {
        t.key(code, ctrl, true);
        t.key(code, ctrl, false)
    }

    #[test]
    fn a_lone_ctrl_tap_fires_and_a_chord_or_click_does_not() {
        let mut t = CtrlTap::default();
        assert!(tap(&mut t, CTRL, true));
        assert!(!tap(&mut t, A, false), "another key");
        t.key(CTRL, true, true);
        tap(&mut t, A, false);
        assert!(!t.key(CTRL, true, false), "Ctrl+A");
        t.key(A, false, true);
        assert!(!tap(&mut t, CTRL, true), "Ctrl under a held key");
        t.key(A, false, false);
        t.key(CTRL, true, true);
        t.spoil();
        assert!(!t.key(CTRL, true, false), "Ctrl+click");
        assert!(tap(&mut t, CTRL, true), "a tap again");
    }

    fn rang(h: &mut crate::harness::Harness) -> Option<BufferId> {
        for (code, pressed) in [(29, true), (29, false)] {
            h.key(code, pressed);
        }
        h.state.locate_rings.node
    }

    #[test]
    fn a_ctrl_tap_rings_the_pointer_only_when_set() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        assert_eq!(rang(&mut h), None, "off by default");
        h.state.config.accessibility.locate_pointer = true;
        let node = rang(&mut h).expect("the rings");
        assert_eq!(h.state.scene.parent(node), Some(h.state.layers.overlay));
        let at = h.state.pointer_location();
        let pos = h.state.scene.position(node).unwrap();
        assert_eq!(
            (pos.x, pos.y),
            (at.x as i32 - SIZE / 2, at.y as i32 - SIZE / 2),
            "round the pointer"
        );
        let since = h.state.locate_rings.since.unwrap();
        frame_at(&mut h.state, since + LENGTH);
        assert_eq!(h.state.locate_rings.node, None, "gone at the end");
        assert!(h.state.scene.position(node).is_none(), "off the scene");
    }

    #[test]
    fn a_ctrl_tap_during_a_drag_or_with_a_taken_click_does_not_ring() {
        use crate::input::{BTN_LEFT, BTN_RIGHT};
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.config.accessibility.locate_pointer = true;
        h.pointer_to(500.0, 400.0);
        h.button(BTN_LEFT, true);
        assert_eq!(rang(&mut h), None, "Ctrl under a held button");
        h.button(BTN_LEFT, false);
        h.state.open_region(false);
        h.key(29, true);
        h.button(BTN_RIGHT, true);
        assert!(
            h.state.panels.region.is_none(),
            "the overlay took the click"
        );
        h.key(29, false);
        h.button(BTN_RIGHT, false);
        assert_eq!(
            h.state.locate_rings.node, None,
            "Ctrl+click the overlay took"
        );
        assert!(rang(&mut h).is_some(), "a lone tap after it rings");
    }

    #[test]
    fn reduced_motion_holds_the_rings_still() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.config.accessibility.locate_pointer = true;
        h.state.config.effects.reduce_motion = true;
        rang(&mut h).expect("the rings");
        let since = h.state.locate_rings.since.unwrap();
        let first = h.state.locate_rings.drawn;
        frame_at(&mut h.state, since + LENGTH / 2);
        assert_eq!(h.state.locate_rings.drawn, first, "the same picture");
        assert_eq!(first.map(|d| d.0), Some((STILL_AT * 1000.0) as u32));
    }
}
