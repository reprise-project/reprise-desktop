//! libinput's event queue, handed to the backend's input handler. The backend's fd source calls [`take_queued`]; tablet pad events go to `tabletpad`, the rest convert as Smithay's libinput source would (which drops pads). libinput queues devices on assign/resume and releases on suspend without making its fd readable, so [`take_queued`] also runs right after each.

use smithay::{
    backend::{
        input::InputEvent,
        libinput::{LibinputInputBackend, PointerScrollAxis},
    },
    reexports::input::{
        self as libinput, AsRaw, FromRaw,
        event::{
            DeviceEvent, EventTrait, GestureEvent, KeyboardEvent, PointerEvent, SwitchEvent,
            TabletToolEvent, TouchEvent,
            gesture::{GestureHoldEvent, GesturePinchEvent, GestureSwipeEvent},
            tablet_pad::{TabletPadDialEvent, TabletPadEvent},
        },
        ffi,
    },
};

use crate::compositor::State;

/// Feed every queued libinput event to the backend's input handler, without reading the devices.
pub fn take_queued(state: &mut State, mut libinput: libinput::Libinput) {
    loop {
        if let Some(dial) = take_dial(&libinput) {
            crate::tabletpad::event(state, TabletPadEvent::Dial(dial));
            continue;
        }
        let Some(event) = libinput.next() else {
            break;
        };
        match event {
            libinput::Event::TabletPad(e) => crate::tabletpad::event(state, e),
            event => {
                if let Some(event) = convert(event) {
                    crate::backend::on_input(state, event);
                }
            }
        }
    }
}

/// The next queued event, if it is a pad dial's. The input crate's event
/// iterator skips dial events (it converts no `TABLET_PAD_DIAL`), so they
/// are taken off the queue here first.
fn take_dial(libinput: &libinput::Libinput) -> Option<TabletPadDialEvent> {
    // SAFETY: the context is live for as long as `libinput` is.
    let next = unsafe { ffi::libinput_next_event_type(libinput.as_raw_mut()) };
    if next != ffi::libinput_event_type_LIBINPUT_EVENT_TABLET_PAD_DIAL {
        return None;
    }
    // SAFETY: the queue's head is a dial event, so `libinput_get_event`
    // returns it and it is a tablet pad event; the wrapper owns it and
    // destroys it on drop.
    unsafe {
        let event = ffi::libinput_get_event(libinput.as_raw_mut());
        let pad = ffi::libinput_event_get_tablet_pad_event(event);
        Some(TabletPadDialEvent::from_raw(pad, libinput))
    }
}

fn convert(event: libinput::Event) -> Option<InputEvent<LibinputInputBackend>> {
    use libinput::Event as E;
    Some(match event {
        E::Device(DeviceEvent::Added(e)) => InputEvent::DeviceAdded { device: e.device() },
        E::Device(DeviceEvent::Removed(e)) => InputEvent::DeviceRemoved { device: e.device() },
        E::Keyboard(KeyboardEvent::Key(event)) => InputEvent::Keyboard { event },
        E::Pointer(e) => match e {
            PointerEvent::Motion(event) => InputEvent::PointerMotion { event },
            PointerEvent::MotionAbsolute(event) => InputEvent::PointerMotionAbsolute { event },
            PointerEvent::Button(event) => InputEvent::PointerButton { event },
            PointerEvent::ScrollWheel(e) => InputEvent::PointerAxis {
                event: PointerScrollAxis::Wheel(e),
            },
            PointerEvent::ScrollFinger(e) => InputEvent::PointerAxis {
                event: PointerScrollAxis::Finger(e),
            },
            PointerEvent::ScrollContinuous(e) => InputEvent::PointerAxis {
                event: PointerScrollAxis::Continuous(e),
            },
            _ => return None,
        },
        E::Touch(e) => match e {
            TouchEvent::Down(event) => InputEvent::TouchDown { event },
            TouchEvent::Motion(event) => InputEvent::TouchMotion { event },
            TouchEvent::Up(event) => InputEvent::TouchUp { event },
            TouchEvent::Cancel(event) => InputEvent::TouchCancel { event },
            TouchEvent::Frame(event) => InputEvent::TouchFrame { event },
            _ => return None,
        },
        E::Gesture(e) => match e {
            GestureEvent::Swipe(GestureSwipeEvent::Begin(event)) => {
                InputEvent::GestureSwipeBegin { event }
            }
            GestureEvent::Swipe(GestureSwipeEvent::Update(event)) => {
                InputEvent::GestureSwipeUpdate { event }
            }
            GestureEvent::Swipe(GestureSwipeEvent::End(event)) => {
                InputEvent::GestureSwipeEnd { event }
            }
            GestureEvent::Pinch(GesturePinchEvent::Begin(event)) => {
                InputEvent::GesturePinchBegin { event }
            }
            GestureEvent::Pinch(GesturePinchEvent::Update(event)) => {
                InputEvent::GesturePinchUpdate { event }
            }
            GestureEvent::Pinch(GesturePinchEvent::End(event)) => {
                InputEvent::GesturePinchEnd { event }
            }
            GestureEvent::Hold(GestureHoldEvent::Begin(event)) => {
                InputEvent::GestureHoldBegin { event }
            }
            GestureEvent::Hold(GestureHoldEvent::End(event)) => {
                InputEvent::GestureHoldEnd { event }
            }
            _ => return None,
        },
        E::Tablet(e) => match e {
            TabletToolEvent::Axis(event) => InputEvent::TabletToolAxis { event },
            TabletToolEvent::Proximity(event) => InputEvent::TabletToolProximity { event },
            TabletToolEvent::Tip(event) => InputEvent::TabletToolTip { event },
            TabletToolEvent::Button(event) => InputEvent::TabletToolButton { event },
            _ => return None,
        },
        E::Switch(SwitchEvent::Toggle(event)) => InputEvent::SwitchToggle { event },
        _ => return None,
    })
}
