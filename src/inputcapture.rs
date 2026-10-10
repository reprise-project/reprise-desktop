//! Pointer barriers and captured state behind the InputCapture portal. While a capture is active the physical pointer and keyboard go to its receiver EIS client; `release` hands them back. A crossing under the lock starts none.

use std::time::{Duration, Instant};

use smithay::{
    backend::input::{ButtonState, InputTime, KeyState},
    input::pointer::{AxisFrame, CursorImageStatus},
    reexports::wayland_server::Resource,
    utils::{Logical, Point, SERIAL_COUNTER},
};

use crate::compositor::State;
use crate::eis::{Devices, EisId};
use crate::inputproto::proto::reprise_input_capture_v1::RepriseInputCaptureV1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Barrier {
    pub id: u32,
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl Barrier {
    /// Axis aligned; a single pixel counts as vertical.
    pub fn is_valid(&self) -> bool {
        self.x1 == self.x2 || self.y1 == self.y2
    }
}

/// Where the move `from` to `to` first meets `b`, as the point to clamp the cursor on, or None. A vertical barrier at x is the left edge of column x: a rightward move stops on column x - 1, a leftward one on x. Horizontal is the same with rows.
pub fn crossing(from: (f64, f64), to: (f64, f64), b: &Barrier) -> Option<(f64, f64)> {
    if !b.is_valid() {
        return None;
    }
    let (fx, fy) = from;
    let (tx, ty) = to;
    if b.x1 == b.x2 {
        let x = b.x1 as f64;
        let (lo, hi) = (b.y1.min(b.y2) as f64, b.y1.max(b.y2) as f64 + 1.0);
        let (side_from, side_to) = (fx < x, tx < x);
        if side_from == side_to {
            return None;
        }
        let t = (x - fx) / (tx - fx);
        let y = fy + t * (ty - fy);
        if y < lo || y >= hi {
            return None;
        }
        let cx = if tx > fx { x - 1.0 } else { x };
        Some((cx, y))
    } else {
        let y = b.y1 as f64;
        let (lo, hi) = (b.x1.min(b.x2) as f64, b.x1.max(b.x2) as f64 + 1.0);
        let (side_from, side_to) = (fy < y, ty < y);
        if side_from == side_to {
            return None;
        }
        let t = (y - fy) / (ty - fy);
        let x = fx + t * (tx - fx);
        if x < lo || x >= hi {
            return None;
        }
        let cy = if ty > fy { y - 1.0 } else { y };
        Some((x, cy))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureId(u64);

pub struct Capture {
    id: CaptureId,
    pub barriers: Vec<Barrier>,
    pub enabled: bool,
    pub receiver: Option<EisId>,
    resource: RepriseInputCaptureV1,
    /// Keys (evdev codes) and buttons forwarded as pressed and not yet released, so ending the capture does not leave the receiver holding them down.
    forwarded_keys: Vec<u32>,
    forwarded_buttons: Vec<u32>,
    stall: Stall,
}

/// How long a receiver may leave its socket full while input is forwarded before the capture ends; reis queues unwritten data without bound.
const STALL_LIMIT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Stall(Option<Instant>);

impl Stall {
    fn record(&mut self, drained: bool, now: Instant) -> bool {
        if drained {
            self.0 = None;
            return false;
        }
        now.duration_since(*self.0.get_or_insert(now)) >= STALL_LIMIT
    }
}

pub struct Active {
    pub capture: CaptureId,
    pub activation_id: u32,
}

#[derive(Default)]
pub struct CaptureState {
    captures: Vec<Capture>,
    pub active: Option<Active>,
    next_activation: u32,
    next_id: u64,
    /// Buttons held on this desktop: an activation releases them here before the receiver gets the pointer.
    local_buttons: Vec<u32>,
}

impl CaptureState {
    pub fn get(&self, id: CaptureId) -> Option<&Capture> {
        self.captures.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: CaptureId) -> Option<&mut Capture> {
        self.captures.iter_mut().find(|c| c.id == id)
    }

    pub fn next_id(&mut self) -> CaptureId {
        self.next_id += 1;
        CaptureId(self.next_id)
    }

    pub fn create(&mut self, id: CaptureId, resource: RepriseInputCaptureV1) {
        self.captures.push(Capture {
            id,
            barriers: Vec::new(),
            enabled: false,
            receiver: None,
            resource,
            forwarded_keys: Vec::new(),
            forwarded_buttons: Vec::new(),
            stall: Stall::default(),
        });
    }

    pub fn count_for_client(&self, resource: &impl Resource) -> usize {
        let id = resource.id();
        (self.captures.iter())
            .filter(|c| c.resource.id().same_client_as(&id))
            .count()
    }

    fn is_active(&self, cap: CaptureId) -> bool {
        self.active.as_ref().is_some_and(|a| a.capture == cap)
    }
}

/// Captures one client may hold and barriers one capture may carry; `capture_motion` walks all barriers on every pointer motion.
pub const MAX_CAPTURES_PER_CLIENT: usize = 8;
pub const MAX_BARRIERS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Added {
    Ok,
    Invalid,
    TooMany,
}

pub fn add_barrier(barriers: &mut Vec<Barrier>, b: Barrier) -> Added {
    if !b.is_valid() {
        return Added::Invalid;
    }
    if barriers.len() >= MAX_BARRIERS {
        return Added::TooMany;
    }
    barriers.push(b);
    Added::Ok
}

/// Whether a release request applies to the active capture: same capture, and the current activation id or None ("whatever is active").
fn release_applies(
    active: Option<(CaptureId, u32)>,
    cap: CaptureId,
    activation_id: Option<u32>,
) -> bool {
    let Some((active_cap, active_id)) = active else {
        return false;
    };
    active_cap == cap && !activation_id.is_some_and(|id| id != active_id)
}

fn hold(held: &mut Vec<u32>, code: u32, pressed: bool) {
    if pressed && !held.contains(&code) {
        held.push(code);
    } else if !pressed {
        held.retain(|&x| x != code);
    }
}

fn release_forwarded(d: &Devices, keys: Vec<u32>, buttons: Vec<u32>, time_us: u64) {
    if let Some(dev) = &d.pointer
        && let Some(b) = dev.interface::<reis::eis::Button>()
        && !buttons.is_empty()
    {
        for button in buttons {
            b.button(button, reis::eis::button::ButtonState::Released);
        }
        dev.frame(time_us);
    }
    if let Some(dev) = &d.keyboard
        && let Some(k) = dev.interface::<reis::eis::Keyboard>()
        && !keys.is_empty()
    {
        for key in keys {
            k.key(key, reis::eis::keyboard::KeyState::Released);
        }
        dev.frame(time_us);
    }
}

impl State {
    fn capture_devices(&self, cap: CaptureId) -> Option<&Devices> {
        let receiver = self.inputproto.capture.get(cap)?.receiver?;
        self.inputproto.eis.get(receiver)?.devices()
    }

    fn monotonic_us(&self) -> u64 {
        self.now().as_micros() as u64
    }

    pub fn capture_destroy(&mut self, cap: CaptureId) {
        if self.inputproto.capture.is_active(cap) {
            self.capture_release(cap, None, None);
        }
        let receiver = self
            .inputproto
            .capture
            .get_mut(cap)
            .and_then(|c| c.receiver.take());
        if let Some(r) = receiver {
            if let Some(c) = self.inputproto.eis.get_mut(r) {
                c.capture = None;
            }
            self.eis_disconnect(r);
        }
        self.inputproto.capture.captures.retain(|c| c.id != cap);
    }

    pub fn capture_clear_barriers(&mut self, cap: CaptureId) {
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            c.barriers.clear();
        }
        if self.inputproto.capture.is_active(cap) {
            self.capture_release(cap, None, None);
        }
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            c.enabled = false;
        }
    }

    pub fn capture_disable(&mut self, cap: CaptureId) {
        if self.inputproto.capture.is_active(cap) {
            self.capture_release(cap, None, None);
        }
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            c.enabled = false;
        }
    }

    /// A physical relative motion by `delta`, before it moves the pointer. True when it went to the receiver or just activated a capture.
    pub fn capture_motion(&mut self, delta: Point<f64, Logical>) -> bool {
        if let Some(a) = self.inputproto.capture.active.as_ref() {
            if let Some(d) = self.capture_devices(a.capture)
                && let Some(dev) = &d.pointer
                && let Some(p) = dev.interface::<reis::eis::Pointer>()
            {
                p.motion_relative(delta.x as f32, delta.y as f32);
            }
            self.capture_frame();
            return true;
        }
        // The lock owns the pointer: a crossing under it must not start a capture, or the lock dialog would never see a click again.
        if self.is_locked() {
            return false;
        }
        let from = self.pointer_location();
        let (from, to) = ((from.x, from.y), (from.x + delta.x, from.y + delta.y));
        let st = &self.inputproto.capture;
        let hit = st
            .captures
            .iter()
            // A receiver with no pointer device cannot be handed the motion; activating for it would hide the cursor with no way back.
            .filter(|c| {
                c.enabled
                    && self
                        .capture_devices(c.id)
                        .is_some_and(|d| d.pointer.is_some())
            })
            .find_map(|c| {
                (c.barriers.iter()).find_map(|b| crossing(from, to, b).map(|at| (c.id, b.id, at)))
            });
        let Some((cap, barrier, clamp)) = hit else {
            return false;
        };
        self.pointer_to(clamp.into(), InputTime::now());
        self.capture_activate(cap, barrier, to);
        true
    }

    /// A physical absolute motion to `pos` while captured: the hidden pointer goes there quietly. False when nothing is captured.
    pub fn capture_absolute(&mut self, pos: Point<f64, Logical>) -> bool {
        if !self.inputproto.captured() {
            return false;
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.set_location(pos);
        }
        true
    }

    fn capture_activate(&mut self, cap: CaptureId, barrier_id: u32, target: (f64, f64)) {
        // A drag would take the button releases below as its drop.
        self.cancel_drag();
        // Nothing local keeps a press: the focused client hears every held key come up, and the buttons are released here. This runs before `active` is set; otherwise the releases would go to the receiver and the clients would keep the buttons down.
        let time = InputTime::now();
        if let Some(kb) = self.seat.get_keyboard() {
            for k in kb.pressed_keys() {
                let serial = SERIAL_COUNTER.next_serial();
                kb.input_forward(self, k, KeyState::Released, serial, time, false);
            }
        }
        for b in std::mem::take(&mut self.inputproto.capture.local_buttons) {
            self.handle_button(b, ButtonState::Released, time);
        }
        self.clear_pointer_focus();
        let st = &mut self.inputproto.capture;
        // Step by 16 and skip 0, which a release request reads as "whatever is active".
        st.next_activation = st.next_activation.wrapping_add(16).max(1);
        let id = st.next_activation;
        st.active = Some(Active {
            capture: cap,
            activation_id: id,
        });
        if let Some(c) = st.get_mut(cap) {
            c.resource.activated(id, barrier_id, target.0, target.1);
            c.stall = Stall::default();
        }
        self.cursor = CursorImageStatus::Hidden;
        self.redraw_all();
        if let Some(d) = self.capture_devices(cap) {
            for dev in d.iter() {
                dev.start_emulating(id);
            }
            self.capture_flush(cap);
        }
        log::info!("input capture {id} activated on barrier {barrier_id}");
    }

    /// End the active capture if it is `cap`. An `activation_id` other than the current one is ignored; None forces it. With `pos` the pointer goes there first.
    pub fn capture_release(
        &mut self,
        cap: CaptureId,
        activation_id: Option<u32>,
        pos: Option<(f64, f64)>,
    ) {
        let st = &self.inputproto.capture;
        let active = st.active.as_ref().map(|a| (a.capture, a.activation_id));
        let Some((_, id)) = active.filter(|_| release_applies(active, cap, activation_id)) else {
            return;
        };
        // Everything forwarded as pressed goes back up first: a key left down on the receiver stays down after this capture ends.
        let (keys, buttons) = match self.inputproto.capture.get_mut(cap) {
            Some(c) => (
                std::mem::take(&mut c.forwarded_keys),
                std::mem::take(&mut c.forwarded_buttons),
            ),
            None => Default::default(),
        };
        let time_us = self.monotonic_us();
        if let Some(d) = self.capture_devices(cap) {
            release_forwarded(d, keys, buttons, time_us);
            for dev in d.iter() {
                dev.stop_emulating();
            }
            self.capture_flush(cap);
        }
        self.inputproto.capture.active = None;
        let at = pos.map_or_else(|| self.pointer_location(), Point::from);
        self.set_cursor("default");
        self.pointer_to(at, InputTime::now());
        if let Some(c) = self.inputproto.capture.get(cap) {
            c.resource.deactivated(id);
        }
        log::info!("input capture {id} released");
    }

    fn capture_flush(&mut self, cap: CaptureId) -> bool {
        match self.inputproto.capture.get(cap).and_then(|c| c.receiver) {
            Some(r) => self.eis_flush(r),
            None => false,
        }
    }

    pub fn capture_receiver_gone(&mut self, cap: CaptureId) {
        if self.inputproto.capture.is_active(cap) {
            self.capture_release(cap, None, None);
        }
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            c.receiver = None;
            // The devices went with the client; nothing is left to release on.
            c.forwarded_keys.clear();
            c.forwarded_buttons.clear();
        }
    }

    pub fn capture_devices_replaced(&mut self, cap: CaptureId) {
        let Some(id) = (self.inputproto.capture.active.as_ref())
            .filter(|a| a.capture == cap)
            .map(|a| a.activation_id)
        else {
            return;
        };
        // The new devices never saw the presses; the old tally would make the next release send unmatched releases.
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            c.forwarded_keys.clear();
            c.forwarded_buttons.clear();
        }
        if let Some(d) = self.capture_devices(cap) {
            for dev in d.iter() {
                dev.start_emulating(id);
            }
        }
    }

    pub fn capture_layout_changed(&mut self) {
        let caps: Vec<CaptureId> = (self.inputproto.capture.captures.iter())
            .filter(|c| !c.barriers.is_empty() || c.enabled)
            .map(|c| c.id)
            .collect();
        for cap in caps {
            self.capture_clear_barriers(cap);
            if let Some(c) = self.inputproto.capture.get(cap) {
                c.resource.zones_changed();
            }
        }
    }

    pub fn input_layout_changed(&mut self) {
        self.eis_readvertise();
        self.capture_layout_changed();
    }

    pub fn release_capture(&mut self) {
        if let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture) {
            self.capture_release(cap, None, None);
        }
    }

    /// A pointer button, before it acts: true when it went to the receiver. Buttons this desktop keeps are counted so an activation can release them.
    pub fn capture_button(&mut self, button: u32, state: ButtonState) -> bool {
        let pressed = state == ButtonState::Pressed;
        if !self.inputproto.gated() {
            hold(&mut self.inputproto.capture.local_buttons, button, pressed);
            return false;
        }
        let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture) else {
            return true;
        };
        let has_button = self
            .capture_devices(cap)
            .and_then(|d| d.pointer.as_ref())
            .and_then(|dev| dev.interface::<reis::eis::Button>());
        if let Some(b) = has_button {
            let s = if pressed {
                reis::eis::button::ButtonState::Press
            } else {
                reis::eis::button::ButtonState::Released
            };
            b.button(button, s);
            if let Some(c) = self.inputproto.capture.get_mut(cap) {
                hold(&mut c.forwarded_buttons, button, pressed);
            }
        }
        self.capture_frame();
        true
    }

    pub fn capture_axis(&mut self, frame: &AxisFrame) -> bool {
        if !self.inputproto.gated() {
            return false;
        }
        if let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture)
            && let Some(dev) = self.capture_devices(cap).and_then(|d| d.pointer.as_ref())
            && let Some(s) = dev.interface::<reis::eis::Scroll>()
        {
            let (dx, dy) = frame.axis;
            let (nx, ny) = frame.v120.unwrap_or((0, 0));
            if nx != 0 {
                s.scroll_discrete(nx, 0);
            } else if dx != 0.0 {
                s.scroll(dx as f32, 0.0);
            }
            if ny != 0 {
                s.scroll_discrete(0, ny);
            } else if dy != 0.0 {
                s.scroll(0.0, dy as f32);
            }
        }
        self.capture_frame();
        true
    }

    pub fn capture_key(&mut self, keycode: u32, pressed: bool) {
        let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture) else {
            return;
        };
        let time_us = self.monotonic_us();
        let Some(dev) = self.capture_devices(cap).and_then(|d| d.keyboard.as_ref()) else {
            return;
        };
        let Some(k) = dev.interface::<reis::eis::Keyboard>() else {
            return;
        };
        let s = if pressed {
            reis::eis::keyboard::KeyState::Press
        } else {
            reis::eis::keyboard::KeyState::Released
        };
        k.key(keycode, s);
        dev.frame(time_us);
        if let Some(c) = self.inputproto.capture.get_mut(cap) {
            hold(&mut c.forwarded_keys, keycode, pressed);
        }
        self.capture_flush_active();
    }

    fn capture_frame(&mut self) {
        let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture) else {
            return;
        };
        let time_us = self.monotonic_us();
        if let Some(dev) = self.capture_devices(cap).and_then(|d| d.pointer.as_ref()) {
            dev.frame(time_us);
        }
        self.capture_flush_active();
    }

    fn capture_flush_active(&mut self) {
        let Some(cap) = self.inputproto.capture.active.as_ref().map(|a| a.capture) else {
            return;
        };
        let Some(receiver) = self.inputproto.capture.get(cap).and_then(|c| c.receiver) else {
            return;
        };
        let drained = self.eis_flush(receiver);
        let stalled = (self.inputproto.capture.get_mut(cap))
            .is_some_and(|c| c.stall.record(drained, Instant::now()));
        if stalled {
            log::warn!("input capture: receiver stopped reading, taking input back");
            self.eis_disconnect(receiver);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_carries_at_most_max_barriers() {
        let mut barriers = Vec::new();
        for i in 0..MAX_BARRIERS as u32 {
            let b = vertical(i as i32 * 10, 0, 100);
            assert_eq!(
                add_barrier(&mut barriers, Barrier { id: i, ..b }),
                Added::Ok
            );
        }
        assert_eq!(barriers.len(), MAX_BARRIERS);
        let more = vertical(5000, 0, 100);
        assert_eq!(add_barrier(&mut barriers, more), Added::TooMany);
        let bad = Barrier {
            id: 1,
            x1: 0,
            y1: 0,
            x2: 10,
            y2: 10,
        };
        assert_eq!(add_barrier(&mut barriers, bad), Added::Invalid);
        assert_eq!(barriers.len(), MAX_BARRIERS, "neither was kept");
    }

    #[test]
    fn a_forwarded_press_is_held_once_until_its_release() {
        let mut held = Vec::new();
        hold(&mut held, 30, true);
        hold(&mut held, 30, true);
        hold(&mut held, 31, true);
        assert_eq!(held, [30, 31]);
        hold(&mut held, 30, false);
        assert_eq!(held, [31]);
    }

    fn vertical(x: i32, y1: i32, y2: i32) -> Barrier {
        Barrier {
            id: 1,
            x1: x,
            y1,
            x2: x,
            y2,
        }
    }
    fn horizontal(y: i32, x1: i32, x2: i32) -> Barrier {
        Barrier {
            id: 2,
            x1,
            y1: y,
            x2,
            y2: y,
        }
    }

    #[test]
    fn a_rightward_move_across_the_right_edge_clamps_on_it() {
        let b = vertical(1920, 0, 1079);
        assert_eq!(
            crossing((1910.0, 500.0), (1930.0, 500.0), &b),
            Some((1919.0, 500.0))
        );
    }

    #[test]
    fn a_move_that_stays_inside_does_not_cross() {
        let b = vertical(1920, 0, 1079);
        assert_eq!(crossing((1900.0, 500.0), (1919.0, 500.0), &b), None);
        assert_eq!(crossing((1930.0, 500.0), (1940.0, 500.0), &b), None);
    }

    #[test]
    fn a_diagonal_move_hits_where_it_meets_the_line() {
        let b = vertical(1920, 0, 1079);
        assert_eq!(
            crossing((1910.0, 100.0), (1930.0, 140.0), &b),
            Some((1919.0, 120.0))
        );
    }

    #[test]
    fn a_move_past_the_barriers_extent_misses_it() {
        let b = vertical(1920, 0, 539);
        assert_eq!(crossing((1910.0, 800.0), (1930.0, 800.0), &b), None);
    }

    #[test]
    fn a_top_edge_barrier_catches_an_upward_move() {
        let b = horizontal(0, 0, 1919);
        assert_eq!(
            crossing((100.0, 5.0), (100.0, -10.0), &b),
            Some((100.0, 0.0))
        );
        assert_eq!(crossing((100.0, 5.0), (100.0, 2.0), &b), None);
    }

    #[test]
    fn a_bottom_edge_barrier_catches_a_downward_move() {
        let b = horizontal(1080, 0, 1919);
        assert_eq!(
            crossing((100.0, 1075.0), (100.0, 1090.0), &b),
            Some((100.0, 1079.0))
        );
    }

    #[test]
    fn a_left_edge_barrier_catches_a_leftward_move() {
        let b = vertical(0, 0, 1079);
        assert_eq!(crossing((3.0, 50.0), (-4.0, 50.0), &b), Some((0.0, 50.0)));
    }

    #[test]
    fn a_diagonal_is_rejected_and_a_single_pixel_is_not() {
        assert!(
            !Barrier {
                id: 3,
                x1: 0,
                y1: 0,
                x2: 10,
                y2: 10
            }
            .is_valid()
        );
        assert!(vertical(0, 0, 0).is_valid());
    }

    #[test]
    fn a_receiver_stalls_after_three_seconds_of_failed_flushes() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut stall = Stall::default();
        assert!(!stall.record(false, t0));
        assert!(!stall.record(false, t0 + Duration::from_millis(2999)));
        assert!(stall.record(false, t0 + s(3)));
        assert!(!stall.record(true, t0 + s(4)));
        assert!(!stall.record(false, t0 + s(5)));
        assert!(!stall.record(false, t0 + s(7)));
        assert!(stall.record(false, t0 + s(8)));
    }

    #[test]
    fn a_release_with_a_stale_activation_id_is_ignored() {
        assert!(!release_applies(
            Some((CaptureId(1), 32)),
            CaptureId(1),
            Some(16)
        ));
        assert!(release_applies(
            Some((CaptureId(1), 32)),
            CaptureId(1),
            Some(32)
        ));
    }

    #[test]
    fn a_release_without_an_id_forces_it() {
        assert!(release_applies(
            Some((CaptureId(1), 32)),
            CaptureId(1),
            None
        ));
        assert!(!release_applies(None, CaptureId(1), None));
    }

    #[test]
    fn a_release_for_another_capture_is_ignored() {
        assert!(!release_applies(
            Some((CaptureId(1), 32)),
            CaptureId(2),
            None
        ));
        assert!(!release_applies(
            Some((CaptureId(1), 32)),
            CaptureId(2),
            Some(32)
        ));
    }
}
