//! wlr-virtual-pointer-unstable-v1: pointers a client drives (wlrctl, remote
//! desktop servers). Events go through `State::process_input` like a real mouse.
//!
//! Follows niri's `src/protocols/virtual_pointer.rs` (GPL-3.0-or-later), with
//! the handler folded into `State`, a scroll source sent before the first axis
//! event kept for the frame (wheel if none, as wlroots does), and absolute
//! motion resolved onto the bound output.

use std::sync::Mutex;

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
        DeviceCapability, Event, InputBackend, InputEvent, InputTime, PointerAxisEvent,
        PointerButtonEvent, PointerMotionAbsoluteEvent, PointerMotionEvent, UnusedEvent,
    },
    input::pointer::AxisFrame,
    output::Output,
    reexports::{
        wayland_protocols_wlr::virtual_pointer::v1::server::{
            zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
            zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
        },
        wayland_server::{
            Client, DataInit, DisplayHandle, New, Resource, WEnum, protocol::wl_pointer,
        },
    },
    wayland::{Dispatch2, GlobalDispatch2},
};

use crate::{compositor::State, raster::Rect};

const VERSION: u32 = 2;

pub struct VirtualPointerManagerState;

impl VirtualPointerManagerState {
    pub fn new(display: &DisplayHandle) -> Self {
        display.create_global::<State, ZwlrVirtualPointerManagerV1, _>(VERSION, ManagerGlobal);
        VirtualPointerManagerState
    }
}

pub struct ManagerGlobal;
pub struct ManagerData;

/// One virtual pointer: its output, if the client named one, and the scroll
/// frame being built.
pub struct PointerData {
    output: Option<Output>,
    pending: Mutex<PendingAxis>,
}

#[derive(Default)]
struct PendingAxis {
    frame: Option<AxisFrame>,
    source: Option<AxisSource>,
}

impl PendingAxis {
    fn update(&mut self, time: u32, f: impl FnOnce(AxisFrame) -> AxisFrame) {
        let frame = self
            .frame
            .take()
            .unwrap_or_else(|| AxisFrame::new(InputTime::from_millis(time)));
        self.frame = Some(f(frame));
    }

    fn finish(&mut self) -> Option<AxisFrame> {
        let source = self.source.take().unwrap_or(AxisSource::Wheel);
        self.frame.take().map(|f| f.source(source))
    }
}

/// Where absolute position (`x` of `x_extent`, `y` of `y_extent`) on `target`
/// lies as a fraction of the layout box `layout`, since `process_input` maps
/// absolute events over the whole layout. None for an empty extent or layout.
pub fn layout_fraction(
    (x, y): (u32, u32),
    (x_extent, y_extent): (u32, u32),
    target: Rect,
    layout: Rect,
) -> Option<(f64, f64)> {
    if x_extent == 0 || y_extent == 0 || layout.width <= 0 || layout.height <= 0 {
        return None;
    }
    let lx = f64::from(target.x - layout.x)
        + f64::from(x) / f64::from(x_extent) * f64::from(target.width);
    let ly = f64::from(target.y - layout.y)
        + f64::from(y) / f64::from(y_extent) * f64::from(target.height);
    Some((lx / f64::from(layout.width), ly / f64::from(layout.height)))
}

impl GlobalDispatch2<ZwlrVirtualPointerManagerV1, State> for ManagerGlobal {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        manager: New<ZwlrVirtualPointerManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(manager, ManagerData);
    }

    fn can_view(&self, client: &Client) -> bool {
        crate::sandbox::client_may_see(client, "zwlr_virtual_pointer_manager_v1")
    }
}

impl Dispatch2<ZwlrVirtualPointerManagerV1, State> for ManagerData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &ZwlrVirtualPointerManagerV1,
        request: zwlr_virtual_pointer_manager_v1::Request,
        _dhandle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        let (id, output) = match request {
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointer { id, .. } => (id, None),
            zwlr_virtual_pointer_manager_v1::Request::CreateVirtualPointerWithOutput {
                output,
                id,
                ..
            } => (id, output.as_ref().and_then(Output::from_resource)),
            zwlr_virtual_pointer_manager_v1::Request::Destroy => return,
            _ => unreachable!(),
        };
        data_init.init(
            id,
            PointerData {
                output,
                pending: Mutex::default(),
            },
        );
    }
}

fn axis_of(resource: &ZwlrVirtualPointerV1, axis: WEnum<wl_pointer::Axis>) -> Option<Axis> {
    match axis {
        WEnum::Value(wl_pointer::Axis::VerticalScroll) => Some(Axis::Vertical),
        WEnum::Value(wl_pointer::Axis::HorizontalScroll) => Some(Axis::Horizontal),
        _ => {
            resource.post_error(zwlr_virtual_pointer_v1::Error::InvalidAxis, "invalid axis");
            None
        }
    }
}

impl Dispatch2<ZwlrVirtualPointerV1, State> for PointerData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &ZwlrVirtualPointerV1,
        request: zwlr_virtual_pointer_v1::Request,
        _dhandle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        let pointer = VirtualPointer(resource.clone());
        match request {
            zwlr_virtual_pointer_v1::Request::Motion { time, dx, dy } => {
                let event = MotionEvent {
                    pointer,
                    time,
                    dx,
                    dy,
                };
                state.process_input(InputEvent::<VirtualPointerBackend>::PointerMotion { event });
            }
            zwlr_virtual_pointer_v1::Request::MotionAbsolute {
                time,
                x,
                y,
                x_extent,
                y_extent,
            } => {
                let layout = state.layout_box(None);
                let target = self
                    .output
                    .as_ref()
                    .map(|o| state.layout_box(Some(o)))
                    .filter(|r| r.width > 0)
                    .unwrap_or(layout);
                let Some((x, y)) = layout_fraction((x, y), (x_extent, y_extent), target, layout)
                else {
                    return;
                };
                let event = MotionAbsoluteEvent {
                    pointer,
                    time,
                    x,
                    y,
                };
                state.process_input(InputEvent::<VirtualPointerBackend>::PointerMotionAbsolute {
                    event,
                });
            }
            zwlr_virtual_pointer_v1::Request::Button {
                time,
                button,
                state: pressed,
            } => {
                // wlroots reads the state as a C boolean: anything but
                // released presses.
                let pressed = match pressed {
                    WEnum::Value(wl_pointer::ButtonState::Released) => ButtonState::Released,
                    _ => ButtonState::Pressed,
                };
                let event = ButtonEvent {
                    pointer,
                    time,
                    button,
                    state: pressed,
                };
                state.process_input(InputEvent::<VirtualPointerBackend>::PointerButton { event });
            }
            zwlr_virtual_pointer_v1::Request::Axis { time, axis, value } => {
                if let Some(axis) = axis_of(resource, axis) {
                    let mut p = self.pending.lock().unwrap();
                    p.update(time, |f| f.value(axis, value));
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisDiscrete {
                time,
                axis,
                value,
                discrete,
            } => {
                if let Some(axis) = axis_of(resource, axis) {
                    let mut p = self.pending.lock().unwrap();
                    p.update(time, |f| {
                        f.value(axis, value)
                            .v120(axis, discrete.saturating_mul(120))
                    });
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisStop { time, axis } => {
                if let Some(axis) = axis_of(resource, axis) {
                    let mut p = self.pending.lock().unwrap();
                    p.update(time, |f| f.stop(axis));
                }
            }
            zwlr_virtual_pointer_v1::Request::AxisSource { axis_source } => {
                let source = match axis_source {
                    WEnum::Value(wl_pointer::AxisSource::Wheel) => AxisSource::Wheel,
                    WEnum::Value(wl_pointer::AxisSource::Finger) => AxisSource::Finger,
                    WEnum::Value(wl_pointer::AxisSource::Continuous) => AxisSource::Continuous,
                    WEnum::Value(wl_pointer::AxisSource::WheelTilt) => AxisSource::WheelTilt,
                    _ => {
                        resource.post_error(
                            zwlr_virtual_pointer_v1::Error::InvalidAxisSource,
                            "invalid axis source",
                        );
                        return;
                    }
                };
                self.pending.lock().unwrap().source = Some(source);
            }
            zwlr_virtual_pointer_v1::Request::Frame => {
                let frame = self.pending.lock().unwrap().finish();
                if let Some(frame) = frame {
                    let event = AxisEvent { pointer, frame };
                    state.process_input(InputEvent::<VirtualPointerBackend>::PointerAxis { event });
                }
            }
            zwlr_virtual_pointer_v1::Request::Destroy => {}
            _ => unreachable!(),
        }
    }
}

pub struct VirtualPointerBackend;

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub struct VirtualPointer(ZwlrVirtualPointerV1);

impl Device for VirtualPointer {
    fn id(&self) -> String {
        format!("wlr virtual pointer {}", self.0.id())
    }

    fn name(&self) -> String {
        String::from("virtual pointer")
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(capability, DeviceCapability::Pointer)
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<std::path::PathBuf> {
        None
    }
}

pub struct MotionEvent {
    pointer: VirtualPointer,
    time: u32,
    dx: f64,
    dy: f64,
}

impl Event<VirtualPointerBackend> for MotionEvent {
    fn time(&self) -> InputTime {
        InputTime::from_millis(self.time)
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl PointerMotionEvent<VirtualPointerBackend> for MotionEvent {
    fn delta_x(&self) -> f64 {
        self.dx
    }

    fn delta_y(&self) -> f64 {
        self.dy
    }

    fn delta_x_unaccel(&self) -> f64 {
        self.dx
    }

    fn delta_y_unaccel(&self) -> f64 {
        self.dy
    }
}

/// An absolute position as a fraction of the layout box.
pub struct MotionAbsoluteEvent {
    pointer: VirtualPointer,
    time: u32,
    x: f64,
    y: f64,
}

impl Event<VirtualPointerBackend> for MotionAbsoluteEvent {
    fn time(&self) -> InputTime {
        InputTime::from_millis(self.time)
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl AbsolutePositionEvent<VirtualPointerBackend> for MotionAbsoluteEvent {
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

impl PointerMotionAbsoluteEvent<VirtualPointerBackend> for MotionAbsoluteEvent {}

pub struct ButtonEvent {
    pointer: VirtualPointer,
    time: u32,
    button: u32,
    state: ButtonState,
}

impl Event<VirtualPointerBackend> for ButtonEvent {
    fn time(&self) -> InputTime {
        InputTime::from_millis(self.time)
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

impl PointerButtonEvent<VirtualPointerBackend> for ButtonEvent {
    fn button_code(&self) -> u32 {
        self.button
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

pub struct AxisEvent {
    pointer: VirtualPointer,
    frame: AxisFrame,
}

impl Event<VirtualPointerBackend> for AxisEvent {
    fn time(&self) -> InputTime {
        self.frame.time
    }

    fn device(&self) -> VirtualPointer {
        self.pointer.clone()
    }
}

fn of_axis<T>((h, v): (T, T), axis: Axis) -> T {
    match axis {
        Axis::Horizontal => h,
        Axis::Vertical => v,
    }
}

impl PointerAxisEvent<VirtualPointerBackend> for AxisEvent {
    fn amount(&self, axis: Axis) -> Option<f64> {
        Some(of_axis(self.frame.axis, axis))
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        self.frame.v120.map(|v| f64::from(of_axis(v, axis)))
    }

    fn source(&self) -> AxisSource {
        self.frame.source.unwrap_or(AxisSource::Wheel)
    }

    fn relative_direction(&self, axis: Axis) -> AxisRelativeDirection {
        of_axis(self.frame.relative_direction, axis)
    }
}

impl InputBackend for VirtualPointerBackend {
    type Device = VirtualPointer;

    type KeyboardKeyEvent = UnusedEvent;
    type PointerAxisEvent = AxisEvent;
    type PointerButtonEvent = ButtonEvent;
    type PointerMotionEvent = MotionEvent;
    type PointerMotionAbsoluteEvent = MotionAbsoluteEvent;

    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
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

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: Rect = Rect::new(0, 0, 1920, 1080);
    const RIGHT: Rect = Rect::new(1920, 0, 1280, 1024);
    const LAYOUT: Rect = Rect::new(0, 0, 3200, 1080);

    #[test]
    fn an_unbound_pointer_spans_the_layout() {
        assert_eq!(
            layout_fraction((50, 25), (100, 100), LAYOUT, LAYOUT),
            Some((0.5, 0.25))
        );
    }

    #[test]
    fn an_output_bound_pointer_lands_on_its_output() {
        let (fx, fy) = layout_fraction((640, 512), (1280, 1024), RIGHT, LAYOUT).unwrap();
        assert_eq!((fx * 3200.0, fy * 1080.0), (2560.0, 512.0));
        let (fx, _) = layout_fraction((0, 0), (1, 1), LEFT, LAYOUT).unwrap();
        assert_eq!(fx, 0.0);
    }

    #[test]
    fn an_empty_extent_moves_nothing() {
        assert_eq!(layout_fraction((1, 1), (0, 10), LAYOUT, LAYOUT), None);
        assert_eq!(
            layout_fraction((1, 1), (10, 10), LAYOUT, Rect::default()),
            None
        );
    }

    #[test]
    fn a_scroll_source_sent_first_reaches_the_frame() {
        let mut p = PendingAxis {
            source: Some(AxisSource::Finger),
            ..Default::default()
        };
        p.update(5, |f| f.value(Axis::Vertical, 3.0));
        let f = p.finish().unwrap();
        assert_eq!(f.source, Some(AxisSource::Finger));
        assert_eq!(f.axis, (0.0, 3.0));
        assert!(p.finish().is_none(), "the frame is spent");
        p.update(6, |f| f.value(Axis::Horizontal, 1.0));
        assert_eq!(p.finish().unwrap().source, Some(AxisSource::Wheel));
    }
}
