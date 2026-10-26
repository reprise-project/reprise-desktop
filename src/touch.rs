//! Touchscreens. A finger on a surface whose client bound wl_touch goes to
//! that client as touch. Everywhere else (taskbar, frame, menu, desktop, a
//! client without wl_touch), and while Reprise has the pointer, a finger
//! drives the pointer: the cursor jumps to it and the left button is held
//! until it lifts. Only a finger that lands with no other down does that;
//! the rest are ignored until every finger is up. Fluid desktop: all stir.
//!
//! Each touchscreen covers one output: the one `[touchscreen] output`
//! names, else the one udev tags the device with (`WL_OUTPUT`), else the
//! laptop's own panel when there is exactly one, else the whole layout.
//! No finger reaches a client under the lock or while an input capture
//! holds the input.
//!
//! The first finger down on a client holds the seat's touch grab under its
//! down's serial, so the client may start a drag (`dnd`) or a move or size
//! from a title bar it draws itself (`take_for_grab`).

use reprisekit::fluid::Source;
use smithay::{
    backend::input::{
        AbsolutePositionEvent, ButtonState, Device, Event, InputBackend, InputTime, TouchEvent,
        TouchSlot,
    },
    input::touch::{
        DownEvent, GrabStartData, MotionEvent, OrientationEvent, ShapeEvent, TouchGrab,
        TouchInnerHandle, UpEvent,
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Serial, Transform},
};

use crate::{
    compositor::State,
    input::{BTN_LEFT, Hit, Interaction},
    popups::Popup,
    power::is_internal_output,
    window::WindowId,
};

/// The touchscreens and where each finger that is down went.
#[derive(Default)]
pub struct TouchState {
    /// Each touchscreen's `Device::id` and the output udev tags it with.
    devices: Vec<(String, Option<String>)>,
    points: Vec<(TouchSlot, Route)>,
    /// Where each stirring finger moved since the last frame.
    stir_moves: Vec<(TouchSlot, Point<f64, Logical>)>,
    /// A client had touch events since the last frame.
    frame_due: bool,
}

/// Where one finger's events go, fixed when it goes down.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Route {
    /// To the client surface it went down on; `at` is where it last was, in layout coordinates.
    Client { at: Point<f64, Logical> },
    /// It drives the pointer and holds the left button.
    Pointer,
    /// It stirs the fluid desktop pane it went down on.
    Stir,
    /// Nowhere, until it lifts.
    Ignored,
}

/// Where a finger goes that no client takes.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Landing {
    Stir,
    Pointer,
    Ignored,
}

/// Where a finger landing on `hit` goes when no client takes it: bare desktop stirs while
/// `fluid`, else a `first` finger drives the pointer.
fn desktop_route(fluid: bool, hit: &Hit, first: bool) -> Landing {
    match hit {
        Hit::Desktop { index: None } if fluid => Landing::Stir,
        _ if first => Landing::Pointer,
        _ => Landing::Ignored,
    }
}

/// The fluid's name for the finger in `slot`.
fn finger(slot: TouchSlot) -> Source {
    Source::Finger(u32::try_from(i32::from(slot)).unwrap_or(u32::MAX))
}

/// The fluid style changed: stirring fingers go nowhere until they lift.
fn ignore_stirs(
    points: &mut [(TouchSlot, Route)],
    stir_moves: &mut Vec<(TouchSlot, Point<f64, Logical>)>,
) {
    for (_, r) in points.iter_mut() {
        if *r == Route::Stir {
            *r = Route::Ignored;
        }
    }
    stir_moves.clear();
}

/// Which output a touchscreen maps to: `configured` when in the layout,
/// else `device` (udev's `WL_OUTPUT`), else the one built-in panel among
/// `outputs`. None spans the layout.
pub fn target_output<'a>(
    configured: Option<&str>,
    device: Option<&str>,
    outputs: &[&'a str],
) -> Option<&'a str> {
    let present = |want: Option<&str>| want.and_then(|w| outputs.iter().copied().find(|&o| o == w));
    if let Some(o) = present(configured).or_else(|| present(device)) {
        return Some(o);
    }
    let mut builtin = outputs.iter().copied().filter(|o| is_internal_output(o));
    match (builtin.next(), builtin.next()) {
        (Some(o), None) => Some(o),
        _ => None,
    }
}

/// The seat's touch grab while no other is set: each finger goes to the
/// surface it lands on (Smithay's default holds every later finger on the
/// first one's surface). The first finger swaps in a [`FirstFinger`] grab.
struct EachFinger(GrabStartData<State>);

impl TouchGrab<State> for EachFinger {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, focus.clone(), event);
        let start = GrabStartData {
            focus,
            slot: event.slot,
            location: event.location,
        };
        handle.set_grab(self, data, event.serial, FirstFinger(EachFinger(start)));
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        handle.up(data, event);
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn cancel(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.cancel(data);
    }

    fn shape(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &ShapeEvent,
    ) {
        handle.shape(data, event);
    }

    fn orientation(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &OrientationEvent,
    ) {
        handle.orientation(data, event);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.0
    }

    fn unset(&mut self, _data: &mut State) {}
}

/// The implicit grab of the first finger down: its serial and start data
/// let that finger's client start a drag or a move. Ends when that finger
/// lifts or the clients' touch is cancelled.
struct FirstFinger(EachFinger);

impl TouchGrab<State> for FirstFinger {
    fn down(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &DownEvent,
    ) {
        handle.down(data, focus, event);
    }

    fn up(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>, event: &UpEvent) {
        handle.up(data, event);
        if event.slot == self.0.0.slot {
            handle.unset_grab(self, data);
        }
    }

    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        focus: Option<(WlSurface, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, focus, event);
    }

    fn frame(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.frame(data);
    }

    fn cancel(&mut self, data: &mut State, handle: &mut TouchInnerHandle<'_, State>) {
        handle.cancel(data);
        handle.unset_grab(self, data);
    }

    fn shape(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &ShapeEvent,
    ) {
        handle.shape(data, event);
    }

    fn orientation(
        &mut self,
        data: &mut State,
        handle: &mut TouchInnerHandle<'_, State>,
        event: &OrientationEvent,
    ) {
        handle.orientation(data, event);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        &self.0.0
    }

    fn unset(&mut self, _data: &mut State) {}
}

/// Hand finger `slot` to the pointer and return where it is. None when it
/// is not on a client or another finger already drives the pointer.
fn hand_to_pointer(
    points: &mut [(TouchSlot, Route)],
    slot: TouchSlot,
) -> Option<Point<f64, Logical>> {
    if points.iter().any(|(_, r)| *r == Route::Pointer) {
        return None;
    }
    let (_, r) = points.iter_mut().find(|(s, _)| *s == slot)?;
    let Route::Client { at } = *r else {
        return None;
    };
    *r = Route::Pointer;
    Some(at)
}

/// The udev property `key` of `device`.
pub(crate) fn udev_property(device: &impl Device, key: &str) -> Option<String> {
    let path = device.syspath()?;
    let dev = smithay::reexports::udev::Device::from_syspath(&path).ok()?;
    dev.property_value(key)
        .map(|v| v.to_string_lossy().into_owned())
}

/// The output udev tags `device` with (`WL_OUTPUT`).
fn udev_output(device: &impl Device) -> Option<String> {
    udev_property(device, "WL_OUTPUT")
}

impl State {
    /// A new touchscreen: the seat offers wl_touch while one is plugged in.
    pub fn touch_added(&mut self, device: &impl Device) {
        let output = udev_output(device);
        self.devices.touch.devices.push((device.id(), output));
        if self.seat.get_touch().is_none() {
            self.seat.add_touch_with_default_grab(|| {
                Box::new(EachFinger(GrabStartData {
                    focus: None,
                    slot: TouchSlot::default(),
                    location: Point::default(),
                }))
            });
        }
    }

    /// A touchscreen went away: its fingers will not come up now.
    pub fn touch_removed(&mut self, device: &impl Device) {
        let id = device.id();
        let touch = &mut self.devices.touch;
        let Some(i) = touch.devices.iter().position(|(d, _)| *d == id) else {
            return;
        };
        touch.devices.remove(i);
        self.release_all_fingers(InputTime::now());
        if self.devices.touch.devices.is_empty() {
            self.seat.remove_touch();
        }
    }

    /// The layout point a touch event's 0..1 position stands for, on the
    /// output its device maps to (`target_output`).
    fn touch_point<B: InputBackend>(
        &self,
        ev: &impl AbsolutePositionEvent<B>,
    ) -> Point<f64, Logical> {
        let id = ev.device().id();
        let tagged = self
            .devices
            .touch
            .devices
            .iter()
            .find(|(d, _)| *d == id)
            .and_then(|(_, o)| o.as_deref());
        let outputs: Vec<(String, Transform, Rectangle<i32, Logical>)> = self
            .scene
            .outputs()
            .iter()
            .filter_map(|o| {
                reprisescene::output_rect(o).map(|r| (o.name(), o.current_transform(), r))
            })
            .collect();
        let names: Vec<&str> = outputs.iter().map(|(n, ..)| n.as_str()).collect();
        let configured = self.config.touchscreen.output.as_deref();
        let (transform, area) = target_output(configured, tagged, &names)
            .and_then(|t| outputs.iter().find(|(n, ..)| n == t))
            .map_or_else(
                || (Transform::Normal, self.output_area()),
                |(_, t, r)| (*t, *r),
            );
        let norm = ev.position_transformed((1, 1).into());
        self.zoom_absolute(place(transform, (norm.x, norm.y), area))
    }

    /// Whether a finger may go to a client now: not under the lock, nor
    /// while Reprise has the pointer (a move, a size, one of its popups,
    /// the region overlay, a popup grab, a held button's grab).
    pub fn clients_may_have_touch(&self) -> bool {
        !self.is_locked()
            && self.interaction == Interaction::Idle
            && Popup::open(self).next().is_none()
            && self.panels.region.is_none()
            && !self.seat.get_pointer().is_some_and(|p| p.is_grabbed())
    }

    /// Focus what a press on `surface` lands on as a click does. False when
    /// a modal dialog the window owns came up instead, which takes the press.
    pub fn focus_for_press(&mut self, surface: &WlSurface, window: Option<WindowId>) -> bool {
        let Some(w) = window else {
            crate::layer::press(self, surface);
            return true;
        };
        self.focus_window(w);
        self.modal_target(w) == w
    }

    /// A client asks to move or size its window from the touch it holds on
    /// `surface`: whether `serial` is that touch's down (`FirstFinger`).
    /// The finger then drives the pointer and the clients' touch is
    /// cancelled; the finger's lift ends the move or size.
    pub fn take_for_grab(&mut self, surface: &WlSurface, serial: Serial) -> bool {
        let Some(touch) = self.seat.get_touch() else {
            return false;
        };
        let Some(start) = touch.grab_start_data().filter(|_| touch.has_grab(serial)) else {
            return false;
        };
        let theirs =
            (start.focus.as_ref()).is_some_and(|(f, _)| f.id().same_client_as(&surface.id()));
        if !theirs || self.is_locked() || self.inputproto.gated() {
            return false;
        }
        let Some(at) = hand_to_pointer(&mut self.devices.touch.points, start.slot) else {
            return false;
        };
        let time = InputTime::now();
        // Smithay cancels only points that had an event since the last frame.
        touch.motion(
            self,
            None,
            &MotionEvent {
                slot: start.slot,
                location: at,
                time,
            },
        );
        self.cancel_client_touch(time);
        self.pointer_to(at, time);
        true
    }

    fn route(&self, slot: TouchSlot) -> Option<Route> {
        self.devices
            .touch
            .points
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|&(_, r)| r)
    }

    fn accepts_touch(&self, surface: &WlSurface) -> bool {
        let Some(touch) = self.seat.get_touch() else {
            return false;
        };
        self.dh
            .get_client(surface.id())
            .is_ok_and(|c| touch.client_touch(&c).next().is_some())
    }

    fn touch_button(&mut self, pressed: bool, time: InputTime) {
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        self.handle_button(BTN_LEFT, state, time);
        crate::access::touch(self);
    }

    pub fn touch_down<B: InputBackend>(&mut self, ev: &B::TouchDownEvent) {
        let slot = ev.slot();
        if self.route(slot).is_some() {
            return;
        }
        self.flush_stirs();
        let pos = self.touch_point(ev);
        let time = Event::time(ev);
        let first = (self.devices.touch.points.iter()).all(|(_, r)| *r == Route::Stir);
        // Under a capture the input belongs to the other machine.
        let hit = if self.inputproto.gated() {
            Hit::Nothing
        } else {
            self.scene_hit(pos)
        };
        let r = match hit {
            Hit::Surface {
                surface,
                origin,
                window,
            } if self.clients_may_have_touch() && self.accepts_touch(&surface) => {
                // Every finger focuses as a click does, so a window under a
                // modal dialog brings the dialog up and gets nothing.
                if !self.focus_for_press(&surface, window) {
                    Route::Ignored
                } else if let Some(touch) = self.seat.get_touch() {
                    touch.down(
                        self,
                        Some((surface, origin)),
                        &DownEvent {
                            slot,
                            location: pos,
                            serial: SERIAL_COUNTER.next_serial(),
                            time,
                        },
                    );
                    self.devices.touch.frame_due = true;
                    Route::Client { at: pos }
                } else {
                    Route::Ignored
                }
            }
            _ => {
                let fluid = !self.is_locked() && self.stir_takes(pos);
                match desktop_route(fluid, &hit, first && !self.inputproto.gated()) {
                    Landing::Stir => {
                        self.stir_down(finger(slot), pos);
                        Route::Stir
                    }
                    Landing::Pointer => {
                        self.pointer_to(pos, time);
                        self.touch_button(true, time);
                        Route::Pointer
                    }
                    Landing::Ignored => Route::Ignored,
                }
            }
        };
        self.devices.touch.points.push((slot, r));
    }

    pub fn touch_motion<B: InputBackend>(&mut self, ev: &B::TouchMotionEvent) {
        let slot = ev.slot();
        match self.route(slot) {
            Some(Route::Client { .. }) => {
                if self.is_locked() || self.inputproto.gated() {
                    return;
                }
                let pos = self.touch_point(ev);
                if let Some((_, r)) = self
                    .devices
                    .touch
                    .points
                    .iter_mut()
                    .find(|(s, _)| *s == slot)
                {
                    *r = Route::Client { at: pos };
                }
                if self.drag_touch_motion(slot, pos, Event::time(ev)) {
                    return;
                }
                let Some(touch) = self.seat.get_touch() else {
                    return;
                };
                touch.motion(
                    self,
                    None,
                    &MotionEvent {
                        slot,
                        location: pos,
                        time: Event::time(ev),
                    },
                );
                self.devices.touch.frame_due = true;
            }
            Some(Route::Pointer) => {
                let pos = self.touch_point(ev);
                self.pointer_to(pos, Event::time(ev));
            }
            Some(Route::Stir) => {
                let pos = self.touch_point(ev);
                let moves = &mut self.devices.touch.stir_moves;
                match moves.iter_mut().find(|(s, _)| *s == slot) {
                    Some((_, at)) => *at = pos,
                    None => moves.push((slot, pos)),
                }
            }
            Some(Route::Ignored) | None => {}
        }
    }

    pub fn touch_up<B: InputBackend>(&mut self, ev: &B::TouchUpEvent) {
        self.finger_up(ev.slot(), Event::time(ev));
    }

    fn finger_up(&mut self, slot: TouchSlot, time: InputTime) {
        self.flush_stirs();
        let points = &mut self.devices.touch.points;
        let Some(i) = points.iter().position(|(s, _)| *s == slot) else {
            return;
        };
        match points.remove(i).1 {
            Route::Client { .. } => {
                if let Some(touch) = self.seat.get_touch() {
                    touch.up(
                        self,
                        &UpEvent {
                            slot,
                            serial: SERIAL_COUNTER.next_serial(),
                            time,
                        },
                    );
                    self.devices.touch.frame_due = true;
                }
            }
            Route::Pointer => self.touch_button(false, time),
            Route::Stir => self.stir_up(finger(slot)),
            Route::Ignored => {}
        }
    }

    pub fn touch_cancel<B: InputBackend>(&mut self, ev: &B::TouchCancelEvent) {
        self.finger_cancelled(ev.slot(), Event::time(ev));
    }

    /// libinput gave up on a finger (a palm, say). The clients hear their
    /// sequences are cancelled; the pointer's button comes up, after
    /// cancelling a drag the finger held.
    fn finger_cancelled(&mut self, slot: TouchSlot, time: InputTime) {
        match self.route(slot) {
            Some(Route::Client { .. }) => {
                self.cancel_client_touch(time);
                self.devices.touch.points.retain(|(s, _)| *s != slot);
                return;
            }
            Some(Route::Stir) => {
                self.drop_stir(slot);
                self.devices.touch.points.retain(|(s, _)| *s != slot);
                return;
            }
            Some(Route::Pointer) if self.drag_running() => self.cancel_drag(),
            Some(Route::Pointer) => self.drag_view_cancel(),
            _ => {}
        }
        self.finger_up(slot, time);
    }

    /// Cancel what every client has of touch: its other fingers go nowhere
    /// until they lift. Smithay cancels only points that had an event since
    /// the last frame, so each client point first gets a motion where it is.
    fn cancel_client_touch(&mut self, time: InputTime) {
        let Some(touch) = self.seat.get_touch() else {
            return;
        };
        let clients: Vec<(TouchSlot, Point<f64, Logical>)> = self
            .devices
            .touch
            .points
            .iter()
            .filter_map(|&(s, r)| match r {
                Route::Client { at } => Some((s, at)),
                _ => None,
            })
            .collect();
        for &(slot, at) in &clients {
            touch.motion(
                self,
                None,
                &MotionEvent {
                    slot,
                    location: at,
                    time,
                },
            );
        }
        touch.cancel(self);
        self.devices.touch.frame_due = false;
        for (_, r) in self.devices.touch.points.iter_mut() {
            if matches!(r, Route::Client { .. }) {
                *r = Route::Ignored;
            }
        }
    }

    /// The end of a set of touch events: clients that had some get their frame.
    pub fn touch_frame(&mut self) {
        self.flush_stirs();
        if !std::mem::take(&mut self.devices.touch.frame_due) {
            return;
        }
        if let Some(touch) = self.seat.get_touch() {
            touch.frame(self);
        }
    }

    /// Every finger is gone at once (its device was unplugged).
    fn release_all_fingers(&mut self, time: InputTime) {
        if self
            .devices
            .touch
            .points
            .iter()
            .any(|(_, r)| matches!(r, Route::Client { .. }))
        {
            self.cancel_client_touch(time);
        }
        let points = &mut self.devices.touch.points;
        let stirring: Vec<TouchSlot> = (points.iter())
            .filter(|(_, r)| *r == Route::Stir)
            .map(|(s, _)| *s)
            .collect();
        points.retain(|(_, r)| *r != Route::Stir);
        for slot in stirring {
            self.drop_stir(slot);
        }
        let slots: Vec<TouchSlot> = self.devices.touch.points.iter().map(|(s, _)| *s).collect();
        for slot in slots {
            self.finger_up(slot, time);
        }
        self.touch_frame();
    }

    /// The stirring fingers' moves since the last frame reach the fluid together, so a twist
    /// sees both fingers of one frame. Also runs on a down or up, for devices without frames.
    fn flush_stirs(&mut self) {
        for (slot, at) in std::mem::take(&mut self.devices.touch.stir_moves) {
            self.stir_move(finger(slot), at);
        }
    }

    /// Finger `slot` stops stirring without a tap or flick.
    fn drop_stir(&mut self, slot: TouchSlot) {
        self.devices.touch.stir_moves.retain(|(s, _)| *s != slot);
        self.stir_cancel(finger(slot));
    }

    /// The fluid style changed (`water_configured`): stirring fingers stop until they lift.
    pub fn ignore_stirring_fingers(&mut self) {
        let touch = &mut self.devices.touch;
        ignore_stirs(&mut touch.points, &mut touch.stir_moves);
    }
}

/// The point in `area` that a device's 0..1 position `norm` stands for on an
/// output with `transform`. The device reports in the panel's native axes, which
/// the transform turns into the output's logical ones.
pub(crate) fn place(
    transform: Transform,
    norm: (f64, f64),
    area: Rectangle<i32, Logical>,
) -> Point<f64, Logical> {
    let p = transform.transform_point_in(Point::<f64, Logical>::from(norm), &(1.0, 1.0).into());
    let size = area.size.to_f64();
    Point::from((p.x * size.w, p.y * size.h)) + area.loc.to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(n: u32) -> TouchSlot {
        TouchSlot::from(Some(n))
    }

    fn client(x: f64) -> Route {
        Route::Client {
            at: Point::from((x, 5.0)),
        }
    }

    #[test]
    fn a_device_position_lands_in_the_output_logical_axes() {
        let area = Rectangle::new((1920, 0).into(), (1080, 1920).into());
        let at = |t, norm| {
            let p = place(t, norm, area);
            (p.x, p.y)
        };
        assert_eq!(at(Transform::Normal, (0.25, 0.1)), (2190.0, 192.0));
        // The panel's native top-left is where a 90 output draws its logical top-right.
        assert_eq!(at(Transform::_90, (0.0, 0.0)), (3000.0, 0.0));
        assert_eq!(at(Transform::_90, (0.25, 0.1)), (2892.0, 480.0));
        assert_eq!(at(Transform::Flipped180, (0.25, 0.1)), (2190.0, 1728.0));
    }

    #[test]
    fn a_title_bar_touch_hands_its_finger_to_the_pointer() {
        let mut points = vec![(slot(0), client(10.0)), (slot(1), client(20.0))];
        assert_eq!(
            hand_to_pointer(&mut points, slot(1)),
            Some(Point::from((20.0, 5.0))),
            "the move starts where the finger is"
        );
        assert_eq!(points[1].1, Route::Pointer);
        assert_eq!(points[0].1, client(10.0), "the other finger stays");
    }

    #[test]
    fn only_one_finger_drives_the_pointer() {
        let mut points = vec![(slot(0), Route::Pointer), (slot(1), client(20.0))];
        assert_eq!(hand_to_pointer(&mut points, slot(1)), None);
        assert_eq!(points[1].1, client(20.0));
    }

    #[test]
    fn a_finger_off_the_clients_is_not_handed_over() {
        let mut points = vec![(slot(0), Route::Ignored)];
        assert_eq!(hand_to_pointer(&mut points, slot(0)), None);
        assert_eq!(
            hand_to_pointer(&mut points, slot(3)),
            None,
            "no such finger"
        );
    }

    #[test]
    fn second_finger_on_desktop_stirs_with_fluid() {
        let bare = Hit::Desktop { index: None };
        assert_eq!(desktop_route(true, &bare, false), Landing::Stir);
        assert_eq!(desktop_route(true, &bare, true), Landing::Stir);
    }

    #[test]
    fn finger_on_icon_drives_pointer_with_fluid() {
        let icon = Hit::Desktop { index: Some(0) };
        assert_eq!(desktop_route(true, &icon, true), Landing::Pointer);
        assert_eq!(desktop_route(true, &icon, false), Landing::Ignored);
        assert_eq!(desktop_route(true, &Hit::Nothing, true), Landing::Pointer);
    }

    #[test]
    fn finger_on_desktop_drives_pointer_without_fluid() {
        let bare = Hit::Desktop { index: None };
        assert_eq!(desktop_route(false, &bare, true), Landing::Pointer);
        assert_eq!(desktop_route(false, &bare, false), Landing::Ignored);
    }

    #[test]
    fn style_change_ignores_stirring_fingers() {
        let mut points = vec![
            (slot(0), Route::Stir),
            (slot(1), client(20.0)),
            (slot(2), Route::Stir),
        ];
        let mut moved = vec![
            (slot(0), Point::from((1.0, 2.0))),
            (slot(2), Point::from((3.0, 4.0))),
        ];
        ignore_stirs(&mut points, &mut moved);
        assert_eq!(points[0].1, Route::Ignored);
        assert_eq!(points[1].1, client(20.0), "a client finger stays");
        assert_eq!(points[2].1, Route::Ignored);
        assert!(moved.is_empty(), "no buffered motion reaches the fluid");
    }

    #[test]
    fn a_slot_without_an_id_is_its_own_finger() {
        assert_eq!(finger(slot(3)), Source::Finger(3));
        assert_eq!(finger(TouchSlot::from(None)), Source::Finger(u32::MAX));
    }

    #[test]
    fn the_configured_output_wins_when_present() {
        let outs = ["eDP-1", "DP-1"];
        assert_eq!(
            target_output(Some("DP-1"), Some("eDP-1"), &outs),
            Some("DP-1")
        );
        assert_eq!(
            target_output(Some("HDMI-A-1"), Some("DP-1"), &outs),
            Some("DP-1")
        );
    }

    #[test]
    fn udev_names_the_output_next() {
        assert_eq!(
            target_output(None, Some("DP-1"), &["eDP-1", "DP-1"]),
            Some("DP-1")
        );
    }

    #[test]
    fn a_lone_laptop_panel_is_the_fallback() {
        assert_eq!(target_output(None, None, &["eDP-1", "DP-1"]), Some("eDP-1"));
        assert_eq!(
            target_output(None, Some("DP-9"), &["DP-1", "LVDS-1"]),
            Some("LVDS-1")
        );
    }

    #[test]
    fn otherwise_the_whole_layout() {
        assert_eq!(target_output(None, None, &["DP-1", "HDMI-A-1"]), None);
        assert_eq!(target_output(None, None, &["eDP-1", "eDP-2"]), None);
        assert_eq!(target_output(None, None, &[]), None);
    }
}
