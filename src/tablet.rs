//! Drawing tablets through tablet-v2. A tool over a surface whose client bound
//! tablet-v2 sends it tablet events. Everywhere else, and while Reprise has the
//! pointer (a move, a menu), the tool drives the pointer: the tip is the left
//! button, the stylus buttons the right and middle. No client gets the tool
//! under the lock, Reprise's own panels or an input capture. Every tool event
//! counts as activity (`process_input`). Pads are `tabletpad`'s.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, ButtonState, Event, InputBackend, InputTime, ProximityState,
        TabletToolButtonEvent, TabletToolDescriptor, TabletToolEvent, TabletToolProximityEvent,
        TabletToolTipEvent, TabletToolTipState,
    },
    input::{
        pointer::{CursorImageStatus, MotionEvent as PointerMotion},
        tablet::{
            TabletDescriptor, TabletSeatHandler, TabletSeatTrait,
            tool::{
                AxisFrame, ButtonEvent, DownEvent, MotionEvent, ProximityInEvent,
                ProximityOutEvent, TabletToolHandle, UpEvent,
            },
        },
    },
    reexports::wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::tablet_manager::TabletManagerState,
};

use crate::{
    compositor::State,
    input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit},
};

const BTN_STYLUS: u32 = 0x14b;
const BTN_STYLUS2: u32 = 0x14c;
#[cfg(test)]
const BTN_STYLUS3: u32 = 0x149;

pub struct TabletState {
    #[allow(dead_code)]
    manager: TabletManagerState,
    tools: Vec<Tool>,
}

impl TabletState {
    pub fn new(dh: &DisplayHandle) -> TabletState {
        TabletState {
            manager: TabletManagerState::new::<State>(dh),
            tools: Vec::new(),
        }
    }
}

struct Tool {
    handle: TabletToolHandle<State>,
    focused: Option<WlSurface>,
    /// Pointer buttons this tool pressed; each is released as a pointer button
    /// whatever the tool is over by then.
    emulated: Vec<u32>,
}

/// The pointer button a stylus button stands for when the tool drives the
/// pointer: the lower is right, the upper middle.
pub fn emulated_button(button: u32) -> Option<u32> {
    match button {
        BTN_STYLUS => Some(BTN_RIGHT),
        BTN_STYLUS2 => Some(BTN_MIDDLE),
        _ => None,
    }
}

impl TabletSeatHandler for State {
    type ToolFocus = WlSurface;

    /// A cursor surface from the client the tool is on, as `cursor_image` does for
    /// the pointer.
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        self.cursor = image;
        self.redraw_content();
    }
}

impl State {
    fn tool(&self, desc: &TabletToolDescriptor) -> Option<&Tool> {
        self.devices
            .tablet
            .tools
            .iter()
            .find(|t| t.handle.descriptor() == desc)
    }

    fn tool_mut(&mut self, desc: &TabletToolDescriptor) -> Option<&mut Tool> {
        self.devices
            .tablet
            .tools
            .iter_mut()
            .find(|t| t.handle.descriptor() == desc)
    }

    fn tool_handle(&mut self, desc: &TabletToolDescriptor) -> TabletToolHandle<State> {
        if let Some(t) = self.tool(desc) {
            return t.handle.clone();
        }
        let dh = self.dh.clone();
        let handle = self.seat.tablet_seat().add_wp_tool(self, &dh, desc);
        self.devices.tablet.tools.push(Tool {
            handle: handle.clone(),
            focused: None,
            emulated: Vec::new(),
        });
        handle
    }

    /// A tablet went away. With none left, every tool goes too.
    pub fn tablet_removed(&mut self, desc: &TabletDescriptor) {
        let tablets = self.seat.tablet_seat();
        tablets.remove_tablet(desc);
        if tablets.count_tablets() > 0 {
            return;
        }
        let descs: Vec<TabletToolDescriptor> = self
            .devices
            .tablet
            .tools
            .iter()
            .map(|t| t.handle.descriptor().clone())
            .collect();
        for d in &descs {
            self.release_emulated(d, InputTime::now());
        }
        tablets.clear_tools();
        self.devices.tablet.tools.clear();
    }

    /// Whether tablet events may go to a client now: as for touch
    /// (`clients_may_have_touch`), and not under an input capture.
    fn clients_may_have_tool(&self) -> bool {
        self.clients_may_have_touch() && !self.inputproto.gated()
    }

    /// The layout point a tablet event's 0..1 position stands for: a tablet spans
    /// the whole layout; on a zoomed output it points at what the zoom shows.
    fn tool_point<B: InputBackend>(
        &self,
        ev: &impl AbsolutePositionEvent<B>,
    ) -> Point<f64, Logical> {
        let area = self.output_area();
        self.zoom_absolute(ev.position_transformed(area.size) + area.loc.to_f64())
    }

    fn accepts_tool(&self, tool: &TabletToolHandle<State>, surface: &WlSurface) -> bool {
        self.dh
            .get_client(surface.id())
            .is_ok_and(|c| tool.client_tools(&c).next().is_some())
    }

    /// Move the cursor to `pos` without a pointer focus; the tool's client gets
    /// tablet events instead.
    fn cursor_to(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.motion(
                self,
                None,
                &PointerMotion {
                    location: pos,
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            pointer.frame(self);
        }
        crate::autohideview::poke(self);
        self.redraw_pointer();
    }

    /// Move the tool to `pos` and send the motion to the surface it is over when
    /// its client takes tablet events or holds the tip's grab, else to the pointer.
    fn tool_moved(
        &mut self,
        desc: &TabletToolDescriptor,
        pos: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(tool) = self.tool(desc) else {
            return;
        };
        let handle = tool.handle.clone();
        let emulating = !tool.emulated.is_empty();
        let focused = tool.focused.clone();
        let motion = MotionEvent {
            location: pos,
            serial: SERIAL_COUNTER.next_serial(),
            time,
        };
        if !emulating && self.clients_may_have_tool() {
            if handle.is_grabbed() && focused.is_some() {
                self.cursor_to(pos, time);
                handle.motion(self, None, &motion);
                return;
            }
            if let Hit::Surface {
                surface, origin, ..
            } = self.scene_hit(pos)
                && self.accepts_tool(&handle, &surface)
            {
                // A new surface starts from the arrow until its client sets a cursor.
                if focused.as_ref() != Some(&surface) {
                    self.set_cursor("default");
                }
                if let Some(t) = self.tool_mut(desc) {
                    t.focused = Some(surface.clone());
                }
                self.cursor_to(pos, time);
                handle.motion(self, Some((surface, origin)), &motion);
                return;
            }
        }
        self.tool_leave(desc, pos, time);
        self.pointer_to(pos, time);
    }

    /// The tool leaves its tablet-v2 surface; its cursor goes back to the arrow.
    fn tool_leave(
        &mut self,
        desc: &TabletToolDescriptor,
        pos: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(tool) = self.tool_mut(desc) else {
            return;
        };
        if tool.focused.take().is_none() {
            return;
        }
        let handle = tool.handle.clone();
        let serial = SERIAL_COUNTER.next_serial();
        if handle.is_grabbed() {
            handle.unset_grab(self, serial, time);
        }
        handle.motion(
            self,
            None,
            &MotionEvent {
                location: pos,
                serial,
                time,
            },
        );
        self.set_cursor("default");
    }

    pub fn tool_proximity<B: InputBackend>(&mut self, ev: &B::TabletToolProximityEvent) {
        let desc = ev.tool();
        let time = Event::time(ev);
        let pos = self.tool_point(ev);
        let handle = self.tool_handle(&desc);
        match ev.state() {
            ProximityState::In => {
                let tablets = self.seat.tablet_seat();
                let id = TabletDescriptor::from(&ev.device());
                let tablet = tablets.get_tablet(&id).unwrap_or_else(|| {
                    let dh = self.dh.clone();
                    tablets.add_wp_tablet(&dh, &id)
                });
                // In proximity first, on no surface; the motion then finds where it goes.
                handle.proximity_in(
                    self,
                    None,
                    tablet,
                    &ProximityInEvent {
                        location: pos,
                        axis: None,
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
                self.tool_moved(&desc, pos, time);
            }
            ProximityState::Out => {
                self.release_emulated(&desc, time);
                handle.proximity_out(
                    self,
                    &ProximityOutEvent {
                        serial: SERIAL_COUNTER.next_serial(),
                        time,
                    },
                );
                if let Some(t) = self.tool_mut(&desc)
                    && t.focused.take().is_some()
                {
                    self.set_cursor("default");
                }
            }
        }
        handle.frame(self, time);
    }

    pub fn tool_axis<B: InputBackend>(&mut self, ev: &B::TabletToolAxisEvent) {
        let desc = ev.tool();
        let time = Event::time(ev);
        let handle = self.tool_handle(&desc);
        // Only a move of the tool moves the cursor.
        if ev.delta() != Point::default() {
            let pos = self.tool_point(ev);
            self.tool_moved(&desc, pos, time);
        }
        if self.tool(&desc).is_some_and(|t| t.focused.is_some()) {
            handle.axis(
                self,
                AxisFrame {
                    pressure: ev.pressure_has_changed().then(|| ev.pressure()),
                    distance: ev.distance_has_changed().then(|| ev.distance()),
                    tilt: ev.tilt_has_changed().then(|| ev.tilt()),
                    rotation: ev.rotation_has_changed().then(|| ev.rotation()),
                    slider: ev.slider_has_changed().then(|| ev.slider_position()),
                    wheel: ev
                        .wheel_has_changed()
                        .then(|| (ev.wheel_delta(), ev.wheel_delta_discrete())),
                },
            );
        }
        handle.frame(self, time);
    }

    /// The tip touched or left the tablet. On a tablet-v2 surface it focuses like a
    /// click and holds the surface until it comes up; elsewhere it is the left
    /// button.
    pub fn tool_tip<B: InputBackend>(&mut self, ev: &B::TabletToolTipEvent) {
        let desc = ev.tool();
        let time = Event::time(ev);
        let handle = self.tool_handle(&desc);
        self.tip(
            &desc,
            &handle,
            ev.tip_state() == TabletToolTipState::Down,
            time,
        );
        handle.frame(self, time);
    }

    fn tip(
        &mut self,
        desc: &TabletToolDescriptor,
        handle: &TabletToolHandle<State>,
        down: bool,
        time: InputTime,
    ) {
        if !down
            && self
                .tool(desc)
                .is_some_and(|t| t.emulated.contains(&BTN_LEFT))
        {
            self.emulate(desc, BTN_LEFT, false, time);
            return;
        }
        let pos = self.pointer_location();
        // A tool still on a surface from before a menu opened leaves it first.
        if down && !self.clients_may_have_tool() {
            self.tool_leave(desc, pos, time);
        }
        if self.tool(desc).is_none_or(|t| t.focused.is_none()) {
            if down {
                self.emulate(desc, BTN_LEFT, true, time);
            }
            return;
        }
        let serial = SERIAL_COUNTER.next_serial();
        if !down {
            handle.up(self, &UpEvent { serial, time });
            return;
        }
        if let Hit::Surface {
            window, surface, ..
        } = self.scene_hit(pos)
            && !self.focus_for_press(&surface, window)
        {
            return;
        }
        // Smithay holds the surface (its down grab) until the tip is up.
        handle.down(self, &DownEvent { serial, time });
    }

    /// A stylus button: to the tablet-v2 surface the tool is on, else a pointer
    /// button.
    pub fn tool_button<B: InputBackend>(&mut self, ev: &B::TabletToolButtonEvent) {
        let desc = ev.tool();
        let time = Event::time(ev);
        let handle = self.tool_handle(&desc);
        let pressed = ev.button_state() == ButtonState::Pressed;
        let as_pointer = emulated_button(ev.button());
        let Some(tool) = self.tool(&desc) else {
            return;
        };
        if let Some(b) = as_pointer
            && !pressed
            && tool.emulated.contains(&b)
        {
            self.emulate(&desc, b, false, time);
        } else if tool.focused.is_some() {
            handle.button(
                self,
                &ButtonEvent {
                    serial: SERIAL_COUNTER.next_serial(),
                    button: ev.button(),
                    state: ev.button_state(),
                    time,
                },
            );
        } else if let Some(b) = as_pointer
            && pressed
        {
            self.emulate(&desc, b, true, time);
        }
        handle.frame(self, time);
    }

    /// Press or release pointer `button` for the tool, remembered until its release.
    fn emulate(
        &mut self,
        desc: &TabletToolDescriptor,
        button: u32,
        pressed: bool,
        time: InputTime,
    ) {
        let Some(tool) = self.tool_mut(desc) else {
            return;
        };
        let state = if pressed {
            tool.emulated.push(button);
            ButtonState::Pressed
        } else {
            tool.emulated.retain(|&b| b != button);
            ButtonState::Released
        };
        self.handle_button(button, state, time);
    }

    /// Release every pointer button the tool holds; its ups will not come.
    fn release_emulated(&mut self, desc: &TabletToolDescriptor, time: InputTime) {
        while let Some(&b) = self.tool(desc).and_then(|t| t.emulated.last()) {
            self.emulate(desc, b, false, time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stylus_buttons_stand_for_right_and_middle() {
        assert_eq!(emulated_button(BTN_STYLUS), Some(BTN_RIGHT));
        assert_eq!(emulated_button(BTN_STYLUS2), Some(BTN_MIDDLE));
        assert_eq!(emulated_button(BTN_STYLUS3), None);
        assert_eq!(emulated_button(BTN_LEFT), None);
    }
}
