//! A touchscreen whose taps go through `State::process_input` like libinput's.

use smithay::backend::input::{
    AbsolutePositionEvent, Device, DeviceCapability, Event, InputBackend, InputEvent, InputTime,
    TouchDownEvent, TouchEvent, TouchFrameEvent, TouchSlot, TouchUpEvent, UnusedEvent,
};

use super::{Harness, TestClient};

pub struct Touchscreen;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Screen;

impl Device for Screen {
    fn id(&self) -> String {
        "test touchscreen".into()
    }

    fn name(&self) -> String {
        "test touchscreen".into()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Touch)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

/// Finger 0 at (`x`, `y`), each 0..1 across the screen.
pub struct Finger {
    x: f64,
    y: f64,
}

impl Event<Touchscreen> for Finger {
    fn time(&self) -> InputTime {
        InputTime::now()
    }

    fn device(&self) -> Screen {
        Screen
    }
}

impl TouchEvent<Touchscreen> for Finger {
    fn slot(&self) -> TouchSlot {
        TouchSlot::from(Some(0))
    }
}

impl AbsolutePositionEvent<Touchscreen> for Finger {
    fn x(&self) -> f64 {
        self.x
    }

    fn y(&self) -> f64 {
        self.y
    }

    fn x_transformed(&self, width: i32) -> f64 {
        self.x * f64::from(width)
    }

    fn y_transformed(&self, height: i32) -> f64 {
        self.y * f64::from(height)
    }
}

impl TouchDownEvent<Touchscreen> for Finger {}
impl TouchUpEvent<Touchscreen> for Finger {}
impl TouchFrameEvent<Touchscreen> for Finger {}

impl InputBackend for Touchscreen {
    type Device = Screen;

    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = UnusedEvent;
    type PointerMotionEvent = UnusedEvent;
    type PointerMotionAbsoluteEvent = UnusedEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;

    type TouchDownEvent = Finger;
    type TouchUpEvent = Finger;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = Finger;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;

    type SwitchToggleEvent = UnusedEvent;

    type SpecialEvent = UnusedEvent;
}

impl Harness {
    /// Plug in a touchscreen spanning the layout; the seat offers wl_touch from now on.
    pub fn add_touchscreen(&mut self) {
        self.state.touch_added(&Screen);
    }

    /// One finger down and up at layout point (`x`, `y`), each with its frame.
    pub fn tap(&mut self, x: f64, y: f64) {
        let area = self.state.absolute_area().to_f64();
        let finger = || Finger {
            x: (x - area.loc.x) / area.size.w,
            y: (y - area.loc.y) / area.size.h,
        };
        let s = &mut self.state;
        s.process_input::<Touchscreen>(InputEvent::TouchDown { event: finger() });
        s.process_input::<Touchscreen>(InputEvent::TouchFrame { event: finger() });
        s.process_input::<Touchscreen>(InputEvent::TouchUp { event: finger() });
        s.process_input::<Touchscreen>(InputEvent::TouchFrame { event: finger() });
        self.dispatch();
    }
}

impl TestClient {
    /// Bind wl_touch, once the seat offers it ([`Harness::add_touchscreen`]); each down's serial
    /// lands in `touch_down_serial`.
    pub fn bind_touch(&mut self) {
        let qh = self.qh();
        let touch = self.seat().get_touch(&qh, ());
        self.side.touch = Some(touch);
    }
}
