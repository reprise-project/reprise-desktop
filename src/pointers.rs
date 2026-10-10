//! The pointer devices and protocols: `[mouse]` and `[touchpad]` are pushed to
//! libinput (a pointer that can tap is a touchpad), every pointer is kept for
//! config reloads. Gestures go to the surface with pointer focus; a swipe with
//! `[gestures] fingers` is the compositor's own. Cursor shapes land in
//! `SeatHandler::cursor_image`.

use smithay::{
    backend::input::{
        Event, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent,
        GestureSwipeUpdateEvent, InputBackend, InputEvent,
    },
    input::pointer::{
        GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
        GesturePinchUpdateEvent as PinchUpdate, GestureSwipeBeginEvent, GestureSwipeEndEvent,
        GestureSwipeUpdateEvent as SwipeUpdate,
    },
    reexports::{
        calloop::RegistrationToken,
        input::{AccelProfile, Device, DeviceCapability, ScrollMethod, TapButtonMap},
        wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    },
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::{
        cursor_shape::CursorShapeManagerState, pointer_constraints::PointerConstraintsState,
        pointer_gestures::PointerGesturesState, relative_pointer::RelativePointerManagerState,
    },
};

use crate::{
    compositor::State,
    config::{Config, PointerConfig},
};

pub struct Pointers {
    devices: Vec<Device>,
    pub focus: Option<(WlSurface, Point<f64, Logical>)>,
    pub constrained: Option<WlSurface>,
    pub hovered_at: Option<std::time::Instant>,
    pub owed: Option<smithay::backend::input::InputTime>,
    pub settle_timer: Option<RegistrationToken>,
    pub swipe: Option<KeptSwipe>,
    _globals: Globals,
}

struct Globals {
    _constraints: PointerConstraintsState,
    _relative: RelativePointerManagerState,
    _gestures: PointerGesturesState,
    _cursor_shape: CursorShapeManagerState,
}

impl Pointers {
    pub fn new(dh: &DisplayHandle) -> Pointers {
        Pointers {
            devices: Vec::new(),
            focus: None,
            constrained: None,
            hovered_at: None,
            owed: None,
            settle_timer: None,
            swipe: None,
            _globals: Globals {
                _constraints: PointerConstraintsState::new::<State>(dh),
                _relative: RelativePointerManagerState::new::<State>(dh),
                _gestures: PointerGesturesState::new::<State>(dh),
                _cursor_shape: CursorShapeManagerState::new::<State>(dh),
            },
        }
    }

    pub fn added(&mut self, device: &mut Device, cfg: &Config) {
        if !device.has_capability(DeviceCapability::Pointer) {
            return;
        }
        apply(cfg, device);
        self.devices.push(device.clone());
    }

    pub fn removed(&mut self, device: &Device) {
        self.devices.retain(|d| d != device);
    }

    pub fn apply_all(&mut self, cfg: &Config) {
        for d in &mut self.devices {
            apply(cfg, d);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    pub tap_fingers: u32,
    pub accel: bool,
    pub accel_profiles: Vec<AccelProfile>,
    pub scroll_methods: Vec<ScrollMethod>,
    pub natural: bool,
    pub dwt: bool,
}

impl Caps {
    pub fn is_touchpad(&self) -> bool {
        self.tap_fingers > 0
    }

    fn of(d: &Device) -> Caps {
        Caps {
            tap_fingers: d.config_tap_finger_count(),
            accel: d.config_accel_is_available(),
            accel_profiles: d.config_accel_profiles(),
            scroll_methods: d.config_scroll_methods(),
            natural: d.config_scroll_has_natural_scroll(),
            dwt: d.config_dwt_is_available(),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    pub speed: Option<f64>,
    pub profile: Option<AccelProfile>,
    pub natural: Option<bool>,
    pub tap: Option<bool>,
    pub tap_map: Option<TapButtonMap>,
    pub scroll: Option<ScrollMethod>,
    pub dwt: Option<bool>,
}

pub fn plan(cfg: &Config, caps: &Caps) -> Plan {
    let pointer: &PointerConfig = if caps.is_touchpad() {
        &cfg.touchpad.pointer
    } else {
        &cfg.mouse
    };
    let mut plan = Plan::default();
    if caps.accel {
        plan.speed = Some(pointer.speed);
        let want = if pointer.acceleration {
            AccelProfile::Adaptive
        } else {
            AccelProfile::Flat
        };
        if caps.accel_profiles.contains(&want) {
            plan.profile = Some(want);
        }
    }
    if caps.natural {
        plan.natural = Some(pointer.natural_scroll);
    }
    if caps.is_touchpad() {
        let tp = &cfg.touchpad;
        plan.tap = Some(tp.tap_to_click);
        if caps.tap_fingers >= 2 {
            plan.tap_map = Some(if tp.two_finger_tap_right_click {
                TapButtonMap::LeftRightMiddle
            } else {
                TapButtonMap::LeftMiddleRight
            });
        }
        let want = if tp.two_finger_scroll {
            ScrollMethod::TwoFinger
        } else {
            ScrollMethod::Edge
        };
        if caps.scroll_methods.contains(&want) {
            plan.scroll = Some(want);
        }
        if caps.dwt {
            plan.dwt = Some(tp.disable_while_typing);
        }
    }
    plan
}

fn apply(cfg: &Config, d: &mut Device) {
    let caps = Caps::of(d);
    let plan = plan(cfg, &caps);
    if let Some(v) = plan.speed {
        let _ = d.config_accel_set_speed(v);
    }
    if let Some(v) = plan.profile {
        let _ = d.config_accel_set_profile(v);
    }
    if let Some(v) = plan.natural {
        let _ = d.config_scroll_set_natural_scroll_enabled(v);
    }
    if let Some(v) = plan.tap {
        let _ = d.config_tap_set_enabled(v);
    }
    if let Some(v) = plan.tap_map {
        let _ = d.config_tap_set_button_map(v);
    }
    if let Some(v) = plan.scroll {
        let _ = d.config_scroll_set_method(v);
    }
    if let Some(v) = plan.dwt {
        let _ = d.config_dwt_set_enabled(v);
    }
    log::info!(
        "{} {}: {plan:?}",
        if caps.is_touchpad() {
            "touchpad"
        } else {
            "mouse"
        },
        d.name()
    );
}

/// Whole wheel notches an axis event carries for a StatusNotifierItem, plus the
/// 120ths left over. Two conventions meet here: Wayland counts a wheel step positive towards the
/// user, `discrete` in 120ths of a notch (0 for touchpads, where only the sign
/// of `continuous` counts), while a StatusNotifierItem `Scroll` is positive
/// away from the user. The sign flips and the sub-notch remainder is carried
/// in `residual`. `discrete` can come raw off the EIS wire, so the sum saturates.
pub fn wheel_steps(discrete: i32, continuous: f64, residual: i32) -> (i32, i32) {
    if discrete != 0 {
        let total = residual.saturating_add(discrete);
        return (-(total / 120), total % 120);
    }
    if continuous > 0.0 {
        (-1, 0)
    } else if continuous < 0.0 {
        (1, 0)
    } else {
        (0, residual)
    }
}

impl State {
    pub fn gesture<B: InputBackend>(&mut self, event: InputEvent<B>) {
        if self.keep_swipe(&event) {
            return;
        }
        if self.inputproto.gated()
            || self.is_locked()
            || self.switcher.is_open()
            || self.panels.region.is_some()
        {
            return;
        }
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        match event {
            InputEvent::GestureSwipeBegin { event } => pointer.gesture_swipe_begin(
                self,
                &GestureSwipeBeginEvent {
                    serial,
                    time: Event::time(&event),
                    fingers: event.fingers(),
                },
            ),
            InputEvent::GestureSwipeUpdate { event } => pointer.gesture_swipe_update(
                self,
                &SwipeUpdate {
                    time: Event::time(&event),
                    delta: GestureSwipeUpdateEvent::delta(&event),
                },
            ),
            InputEvent::GestureSwipeEnd { event } => pointer.gesture_swipe_end(
                self,
                &GestureSwipeEndEvent {
                    serial,
                    time: Event::time(&event),
                    cancelled: event.cancelled(),
                },
            ),
            InputEvent::GesturePinchBegin { event } => pointer.gesture_pinch_begin(
                self,
                &GesturePinchBeginEvent {
                    serial,
                    time: Event::time(&event),
                    fingers: event.fingers(),
                },
            ),
            InputEvent::GesturePinchUpdate { event } => pointer.gesture_pinch_update(
                self,
                &PinchUpdate {
                    time: Event::time(&event),
                    delta: GesturePinchUpdateEvent::delta(&event),
                    scale: event.scale(),
                    rotation: event.rotation(),
                },
            ),
            InputEvent::GesturePinchEnd { event } => pointer.gesture_pinch_end(
                self,
                &GesturePinchEndEvent {
                    serial,
                    time: Event::time(&event),
                    cancelled: event.cancelled(),
                },
            ),
            InputEvent::GestureHoldBegin { event } => pointer.gesture_hold_begin(
                self,
                &GestureHoldBeginEvent {
                    serial,
                    time: Event::time(&event),
                    fingers: event.fingers(),
                },
            ),
            InputEvent::GestureHoldEnd { event } => pointer.gesture_hold_end(
                self,
                &GestureHoldEndEvent {
                    serial,
                    time: Event::time(&event),
                    cancelled: event.cancelled(),
                },
            ),
            _ => {}
        }
    }

    /// Take `event` when it belongs to a swipe the compositor keeps (begun with
    /// `[gestures] fingers`, nothing holding the input); false for events that go to the client.
    fn keep_swipe<B: InputBackend>(&mut self, event: &InputEvent<B>) -> bool {
        match event {
            InputEvent::GestureSwipeBegin { event } => {
                let g = self.config.gestures;
                let free = !self.inputproto.gated()
                    && !self.is_locked()
                    && self.switcher.panel.is_none()
                    && self.panels.region.is_none();
                if g.enabled && free && event.fingers() == g.fingers {
                    self.pointers.swipe = Some(KeptSwipe {
                        tracker: crate::gestures::Tracker::default(),
                        overview_was_open: self.overview.view.is_some(),
                    });
                    return true;
                }
                self.pointers.swipe = None;
                false
            }
            InputEvent::GestureSwipeUpdate { event } => match self.pointers.swipe.as_mut() {
                Some(k) => {
                    let d = GestureSwipeUpdateEvent::delta(event);
                    let moved = k
                        .tracker
                        .update(d.x, d.y, u64::from(Event::time(event).millis()));
                    let was_open = k.overview_was_open;
                    if let Some((axis, p)) = moved {
                        self.swipe_moved(axis, p, was_open);
                    }
                    true
                }
                None => false,
            },
            InputEvent::GestureSwipeEnd { event } => match self.pointers.swipe.take() {
                Some(k) => {
                    let ms = u64::from(Event::time(event).millis());
                    if let Some(r) = k.tracker.release(ms, event.cancelled()) {
                        self.swipe_released(r, k.overview_was_open);
                    }
                    true
                }
                None => false,
            },
            _ => false,
        }
    }

    fn swipe_moved(&mut self, axis: crate::gestures::Axis, p: f64, was_open: bool) {
        use crate::gestures::Axis;
        if self.is_locked() {
            return;
        }
        match axis {
            Axis::Horizontal => {
                self.desktop_drag(p);
            }
            Axis::Vertical if was_open => self.set_overview_fade((1.0 + p).clamp(0.0, 1.0) as f32),
            Axis::Vertical => {
                if p > 0.0 && self.overview.view.is_none() {
                    self.open_overview();
                }
                self.set_overview_fade(p.clamp(0.0, 1.0) as f32);
            }
        }
    }

    /// A kept swipe ended at progress `p` along `axis`: commit the way `p`
    /// points or not (`gestures::Tracker::release`). Without a rendered slide, a commit switches here.
    fn swipe_released(
        &mut self,
        (axis, p, commit): (crate::gestures::Axis, f64, bool),
        was_open: bool,
    ) {
        use crate::desktops::Step;
        use crate::gestures::Axis;
        if self.is_locked() {
            return;
        }
        match axis {
            Axis::Horizontal => {
                if !self.desktop_drag_end(commit) && commit {
                    let step = if p > 0.0 { Step::Next } else { Step::Prev };
                    self.step_desktop(step, false);
                }
            }
            Axis::Vertical => {
                let close = if was_open {
                    commit && p < 0.0
                } else {
                    !(commit && p > 0.0)
                };
                if close {
                    self.close_overview();
                } else {
                    self.set_overview_fade(1.0);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeptSwipe {
    tracker: crate::gestures::Tracker,
    overview_was_open: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touchpad() -> Caps {
        Caps {
            tap_fingers: 3,
            accel: true,
            accel_profiles: vec![AccelProfile::Flat, AccelProfile::Adaptive],
            scroll_methods: vec![ScrollMethod::TwoFinger, ScrollMethod::Edge],
            natural: true,
            dwt: true,
        }
    }

    fn mouse() -> Caps {
        Caps {
            tap_fingers: 0,
            accel: true,
            accel_profiles: vec![AccelProfile::Flat, AccelProfile::Adaptive],
            scroll_methods: Vec::new(),
            natural: true,
            dwt: false,
        }
    }

    #[test]
    fn defaults_are_tap_two_finger_scroll_and_adaptive() {
        let cfg = Config::default();
        assert_eq!(
            plan(&cfg, &touchpad()),
            Plan {
                speed: Some(0.0),
                profile: Some(AccelProfile::Adaptive),
                natural: Some(false),
                tap: Some(true),
                tap_map: Some(TapButtonMap::LeftRightMiddle),
                scroll: Some(ScrollMethod::TwoFinger),
                dwt: Some(true),
            }
        );
        assert_eq!(
            plan(&cfg, &mouse()),
            Plan {
                speed: Some(0.0),
                profile: Some(AccelProfile::Adaptive),
                natural: Some(false),
                ..Plan::default()
            }
        );
    }

    #[test]
    fn a_mouse_takes_its_own_section() {
        let mut cfg = Config::default();
        cfg.mouse.speed = -0.5;
        cfg.mouse.acceleration = false;
        cfg.mouse.natural_scroll = true;
        cfg.touchpad.pointer.speed = 0.5;
        let m = plan(&cfg, &mouse());
        assert_eq!(m.speed, Some(-0.5));
        assert_eq!(m.profile, Some(AccelProfile::Flat));
        assert_eq!(m.natural, Some(true));
        let t = plan(&cfg, &touchpad());
        assert_eq!(t.speed, Some(0.5));
        assert_eq!(t.profile, Some(AccelProfile::Adaptive));
        assert_eq!(t.natural, Some(false));
        cfg.touchpad.pointer.natural_scroll = true;
        assert_eq!(plan(&cfg, &touchpad()).natural, Some(true));
    }

    #[test]
    fn gestures_off_pick_the_other_libinput_option() {
        let mut cfg = Config::default();
        cfg.touchpad.tap_to_click = false;
        cfg.touchpad.two_finger_scroll = false;
        cfg.touchpad.two_finger_tap_right_click = false;
        cfg.touchpad.disable_while_typing = false;
        let t = plan(&cfg, &touchpad());
        assert_eq!(t.tap, Some(false));
        assert_eq!(t.tap_map, Some(TapButtonMap::LeftMiddleRight));
        assert_eq!(t.scroll, Some(ScrollMethod::Edge));
        assert_eq!(t.dwt, Some(false));
    }

    #[test]
    fn unsupported_options_are_left_alone() {
        let mut cfg = Config::default();
        cfg.touchpad.two_finger_scroll = false;
        cfg.mouse.acceleration = false;
        let caps = Caps {
            tap_fingers: 1,
            accel: true,
            accel_profiles: vec![AccelProfile::Flat, AccelProfile::Adaptive],
            scroll_methods: vec![ScrollMethod::TwoFinger],
            natural: false,
            dwt: false,
        };
        let t = plan(&cfg, &caps);
        assert_eq!(t.tap_map, None, "one finger has no map");
        assert_eq!(t.scroll, None, "no edge scrolling to fall back on");
        assert_eq!(t.dwt, None, "no keyboard paired with the pad");
        let caps = Caps {
            accel_profiles: vec![AccelProfile::Adaptive],
            ..mouse()
        };
        assert_eq!(plan(&cfg, &caps).profile, None);
        let caps = Caps {
            accel: false,
            natural: false,
            ..mouse()
        };
        assert_eq!(plan(&cfg, &caps), Plan::default());
    }

    #[test]
    fn wheel_steps_flips_wayland_sign_for_the_item() {
        assert_eq!(wheel_steps(120, 15.0, 0), (-1, 0));
        assert_eq!(wheel_steps(-120, -15.0, 0), (1, 0));
        assert_eq!(wheel_steps(360, 45.0, 0), (-3, 0));
    }

    #[test]
    fn wheel_steps_reads_a_continuous_source_by_sign() {
        assert_eq!(wheel_steps(0, 8.5, 0), (-1, 0));
        assert_eq!(wheel_steps(0, -0.5, 0), (1, 0));
    }

    #[test]
    fn wheel_steps_carries_a_sub_notch_remainder() {
        let (steps, residual) = wheel_steps(40, 5.0, 0);
        assert_eq!((steps, residual), (0, 40));
        let (steps, residual) = wheel_steps(40, 5.0, residual);
        assert_eq!((steps, residual), (0, 80));
        assert_eq!(wheel_steps(40, 5.0, residual), (-1, 0));
    }

    #[test]
    fn wheel_steps_keeps_the_remainder_on_an_empty_event() {
        assert_eq!(wheel_steps(0, 0.0, 40), (0, 40));
    }

    #[test]
    fn wheel_steps_saturates_an_extreme_discrete_value() {
        assert_eq!(
            wheel_steps(i32::MAX, 1.0, 40),
            (-(i32::MAX / 120), i32::MAX % 120)
        );
    }
}
