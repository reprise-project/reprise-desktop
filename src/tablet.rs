//! Drawing tablets through tablet-v2. A tool over a surface whose client bound
//! tablet-v2 sends it tablet events. Everywhere else, and while Reprise has the
//! pointer (a move, a menu), the tool drives the pointer: the tip is the left
//! button, the stylus buttons the right and middle. No client gets the tool
//! under the lock, Reprise's own panels or an input capture. Every tool event
//! counts as activity (`process_input`). Pads are `tabletpad`'s.

use smithay::{
    backend::input::{
        AbsolutePositionEvent, ButtonState, Device, Event, InputBackend, InputTime, ProximityState,
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
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER, Size, Transform},
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
    /// Each tablet's active area in mm (udev's `ID_INPUT_WIDTH_MM`, `ID_INPUT_HEIGHT_MM`), by
    /// device id.
    sizes: Vec<(String, Option<(f64, f64)>)>,
}

impl TabletState {
    pub fn new(dh: &DisplayHandle) -> TabletState {
        TabletState {
            manager: TabletManagerState::new::<State>(dh),
            tools: Vec::new(),
            sizes: Vec::new(),
        }
    }
}

/// Which output a tablet maps to: its `[[tablets]]` entry's `configured` output
/// (`touch::target_output`), else the only output. None spans the layout.
fn tablet_output<'a>(configured: Option<&str>, outputs: &[&'a str]) -> Option<&'a str> {
    match (configured, outputs) {
        (Some(_), _) => crate::touch::target_output(configured, None, outputs),
        (None, [only]) => Some(*only),
        (None, _) => None,
    }
}

/// The largest rect of a `size` tablet's shape centred in `area`, on an output with
/// `transform`, which turns the tablet's native axes as `touch::place` does.
fn letterbox(
    area: Rectangle<i32, Logical>,
    transform: Transform,
    size: (f64, f64),
) -> Rectangle<i32, Logical> {
    let shape = transform.transform_size(Size::<f64, Logical>::from(size));
    if !(shape.w > 0.0 && shape.h > 0.0) {
        return area;
    }
    let a = area.size.to_f64();
    let k = (a.w / shape.w).min(a.h / shape.h);
    let (w, h) = (shape.w * k, shape.h * k);
    let x = f64::from(area.loc.x) + (a.w - w) / 2.0;
    let y = f64::from(area.loc.y) + (a.h - h) / 2.0;
    Rectangle::new(
        (x.round() as i32, y.round() as i32).into(),
        (w.round() as i32, h.round() as i32).into(),
    )
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
        self.redraw_cursor();
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

    /// A new tablet: its size, for `keep_aspect`.
    pub fn tablet_added(&mut self, device: &impl Device) {
        let mm = |key| {
            crate::touch::udev_property(device, key)?
                .parse::<f64>()
                .ok()
        };
        let size = mm("ID_INPUT_WIDTH_MM").zip(mm("ID_INPUT_HEIGHT_MM"));
        let id = device.id();
        let sizes = &mut self.devices.tablet.sizes;
        sizes.retain(|(d, _)| *d != id);
        sizes.push((id, size));
    }

    /// A tablet went away. With none left, every tool goes too.
    pub fn tablet_removed(&mut self, device: &impl Device, desc: &TabletDescriptor) {
        let id = device.id();
        self.devices.tablet.sizes.retain(|(d, _)| *d != id);
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

    /// The layout point a tablet event's 0..1 position stands for: on the output its
    /// `[[tablets]]` entry maps it to (`tablet_output`), turned with it and, with
    /// `keep_aspect`, letterboxed to the tablet's shape; on a zoomed output it points at
    /// what the zoom shows.
    fn tool_point<B: InputBackend>(
        &self,
        ev: &impl AbsolutePositionEvent<B>,
    ) -> Point<f64, Logical> {
        let device = ev.device();
        let entry = crate::config::find_tablet(&self.config.tablets, &device.name());
        let outputs: Vec<(String, Transform, Rectangle<i32, Logical>)> = self
            .scene
            .outputs()
            .iter()
            .filter_map(|o| {
                reprisescene::output_rect(o).map(|r| (o.name(), o.current_transform(), r))
            })
            .collect();
        let names: Vec<&str> = outputs.iter().map(|(n, ..)| n.as_str()).collect();
        let (transform, mut area) = tablet_output(entry.and_then(|e| e.output.as_deref()), &names)
            .and_then(|t| outputs.iter().find(|(n, ..)| n == t))
            .map_or_else(
                || (Transform::Normal, self.output_area()),
                |(_, t, r)| (*t, *r),
            );
        if entry.is_some_and(|e| e.keep_aspect) {
            let id = device.id();
            let sizes = &self.devices.tablet.sizes;
            if let Some(size) = sizes.iter().find(|(d, _)| *d == id).and_then(|(_, s)| *s) {
                area = letterbox(area, transform, size);
            }
        }
        let norm = ev.position_transformed((1, 1).into());
        self.zoom_absolute(crate::touch::place(transform, (norm.x, norm.y), area))
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
    fn a_tablet_maps_to_its_configured_output_else_the_only_one() {
        let outs = ["eDP-1", "DP-2"];
        assert_eq!(tablet_output(Some("DP-2"), &outs), Some("DP-2"));
        assert_eq!(tablet_output(None, &outs), None, "spans the layout");
        assert_eq!(tablet_output(None, &["DP-2"]), Some("DP-2"));
        assert_eq!(tablet_output(Some("HDMI-A-1"), &["DP-1", "DP-2"]), None);
    }

    #[test]
    fn keep_aspect_letterboxes_the_tablet_inside_its_output() {
        let area = Rectangle::new((1920, 0).into(), (1920, 1200).into());
        let wide = (200.0, 100.0);
        assert_eq!(
            letterbox(area, Transform::Normal, wide),
            Rectangle::new((1920, 120).into(), (1920, 960).into()),
            "bands above and below"
        );
        assert_eq!(
            letterbox(area, Transform::_90, wide),
            Rectangle::new((2580, 0).into(), (600, 1200).into()),
            "a turned output stands the tablet up: bands at the sides"
        );
        assert_eq!(
            letterbox(area, Transform::Normal, (192.0, 120.0)),
            area,
            "the output's own shape fills it"
        );
        assert_eq!(letterbox(area, Transform::Normal, (0.0, 100.0)), area);
        let boxed = letterbox(area, Transform::Normal, wide);
        let at = |norm| {
            let p = crate::touch::place(Transform::Normal, norm, boxed);
            (p.x, p.y)
        };
        assert_eq!(at((0.0, 0.0)), (1920.0, 120.0));
        assert_eq!(at((1.0, 1.0)), (3840.0, 1080.0));
    }

    #[test]
    fn stylus_buttons_stand_for_right_and_middle() {
        assert_eq!(emulated_button(BTN_STYLUS), Some(BTN_RIGHT));
        assert_eq!(emulated_button(BTN_STYLUS2), Some(BTN_MIDDLE));
        assert_eq!(emulated_button(BTN_STYLUS3), None);
        assert_eq!(emulated_button(BTN_LEFT), None);
    }
}
