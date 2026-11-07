//! A touchpad whose swipes go through `State::process_input` like libinput's.

use smithay::backend::input::{
    Device, DeviceCapability, Event, GestureBeginEvent, GestureEndEvent, GestureSwipeBeginEvent,
    GestureSwipeEndEvent, GestureSwipeUpdateEvent, InputBackend, InputEvent, InputTime,
    UnusedEvent,
};

use super::Harness;

pub struct Touchpad;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pad;

impl Device for Pad {
    fn id(&self) -> String {
        "test touchpad".into()
    }

    fn name(&self) -> String {
        "test touchpad".into()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Gesture)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// A swipe's begin (`fingers`), update (`dx`, `dy`) or end, at `ms`.
pub struct Swipe {
    ms: u32,
    fingers: u32,
    dx: f64,
    dy: f64,
}

impl Event<Touchpad> for Swipe {
    fn time(&self) -> InputTime {
        InputTime::from_millis(self.ms)
    }

    fn device(&self) -> Pad {
        Pad
    }
}

impl GestureBeginEvent<Touchpad> for Swipe {
    fn fingers(&self) -> u32 {
        self.fingers
    }
}

impl GestureSwipeBeginEvent<Touchpad> for Swipe {}

impl GestureSwipeUpdateEvent<Touchpad> for Swipe {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }
}

impl GestureEndEvent<Touchpad> for Swipe {
    fn cancelled(&self) -> bool {
        false
    }
}

impl GestureSwipeEndEvent<Touchpad> for Swipe {}

impl InputBackend for Touchpad {
    type Device = Pad;

    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;

    type GestureSwipeBeginEvent = Swipe;
    type GestureSwipeUpdateEvent = Swipe;
    type GestureSwipeEndEvent = Swipe;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

impl Harness {
    /// A `fingers` swipe through each `(dx, dy)` step, 10 ms apart, then lifted.
    pub fn swipe(&mut self, fingers: u32, steps: &[(f64, f64)]) {
        let at = |i: usize, dx, dy| Swipe {
            ms: 1000 + 10 * i as u32,
            fingers,
            dx,
            dy,
        };
        let begin = at(0, 0.0, 0.0);
        self.state
            .process_input::<Touchpad>(InputEvent::GestureSwipeBegin { event: begin });
        for (i, &(dx, dy)) in steps.iter().enumerate() {
            let event = at(i + 1, dx, dy);
            self.state
                .process_input::<Touchpad>(InputEvent::GestureSwipeUpdate { event });
        }
        let end = at(steps.len() + 1, 0.0, 0.0);
        self.state
            .process_input::<Touchpad>(InputEvent::GestureSwipeEnd { event: end });
        self.dispatch();
    }
}
