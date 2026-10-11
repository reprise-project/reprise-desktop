//! Drag and drop between programs. Smithay runs grab callbacks with the pointer locked, so work that needs the pointer or
//! keyboard runs from an idle. A popup grab opened mid-drag would end the drag, so `XdgShellHandler::grab` hands it to the
//! drag ([`Dnd::hold_popup_grab`]).

use std::{
    any::Any,
    os::fd::OwnedFd,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use smithay::{
    backend::input::{InputTime, TouchSlot},
    desktop::PopupGrab,
    input::{
        Seat,
        dnd::{DnDGrab, DndAction, DndGrabHandler, DndTarget, GrabType, Source, SourceMetadata},
        pointer::{CursorImageStatus, Focus, GrabStartData, MotionEvent},
        touch::{GrabStartData as TouchStartData, MotionEvent as TouchMotion},
    },
    reexports::{
        calloop::{
            RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{
            Resource,
            protocol::{
                wl_data_device_manager::DndAction as WlDndAction, wl_data_source::WlDataSource,
                wl_surface::WlSurface,
            },
        },
    },
    utils::{IsAlive, Logical, Point, SERIAL_COUNTER, Serial},
    wayland::{
        compositor::{
            HookId, SurfaceAttributes, add_post_commit_hook, remove_post_commit_hook, with_states,
        },
        selection::data_device::{
            DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, default_action_chooser,
        },
    },
};

use reprisefs::ops::DropOp;
use reprisekit::taskbar::TaskbarHit;
use reprisescene::{Input, SurfaceId, TreeId};

use crate::{
    compositor::State,
    input::{BTN_LEFT, Hit},
    keys,
    xdnd::PointerFocus,
};

const NONE: DndAction = DndAction::None;
const COPY: DndAction = DndAction::Copy;
const MOVE: DndAction = DndAction::Move;
const ASK: DndAction = DndAction::Ask;

/// How often the cursor is brought up to date while the pointer rests, so a
/// target's reply to the last motion shows.
const CURSOR_TICK: Duration = Duration::from_millis(50);

const SPRING_MS: u32 = reprisekit::controls::tooltip::SHOW_MS;

/// The action the compositor asks for, and whether it stands for a
/// shortcut. Wayland has no link action, so a shortcut asks (ASK) and the
/// cursor still says link. The right button always asks. With no modifier
/// the target's own preference stands (NONE): whether a plain drop moves
/// or copies depends on where the data lives, which only the programs
/// know.
pub fn compositor_action(left: bool, ctrl: bool, shift: bool, alt: bool) -> (DndAction, bool) {
    if !left {
        return (ASK, false);
    }
    match reprisefs::ops::modifier_op(ctrl, shift, alt) {
        Some(DropOp::Copy) => (COPY, false),
        Some(DropOp::Move) => (MOVE, false),
        Some(DropOp::Shortcut) => (ASK, true),
        None => (NONE, false),
    }
}

/// The action a target gets out of `available`, the actions both sides
/// allow, when it prefers `preferred`: the one the compositor asks for
/// (`asked`) when it is there, else the target's preference.
pub fn choose_action(
    asked: DndAction,
    available: WlDndAction,
    preferred: WlDndAction,
) -> WlDndAction {
    let asked = WlDndAction::from(asked);
    if !asked.is_empty() && available.contains(asked) {
        asked
    } else {
        default_action_chooser(available, preferred)
    }
}

pub fn cursor_for(accepted: bool, action: DndAction, shortcut: bool) -> &'static [&'static str] {
    const NO_DROP: &[&str] = &["dnd-no-drop", "no-drop", "not-allowed", "default"];
    if !accepted {
        return NO_DROP;
    }
    match action {
        COPY => &["dnd-copy", "copy", "default"],
        MOVE => &["dnd-move", "default"],
        ASK if shortcut => &["dnd-link", "alias", "link", "default"],
        ASK => &["dnd-ask", "dnd-move", "default"],
        _ => NO_DROP,
    }
}

/// Whether a client's drag offering `offered` gains (`Some(true)`) or
/// loses (`Some(false)`) the mark that its URI list names the files
/// themselves ([`reprisefs::dropdata::HOST_FILES`]): a drag offering the
/// portal key from outside a sandbox gains it, and a sandboxed program
/// loses one it offered itself.
pub fn host_mark(offered: &[String], sandboxed: bool) -> Option<bool> {
    use reprisefs::dropdata::{FILE_TRANSFER, HOST_FILES};
    let has = |mime| offered.iter().any(|m| m == mime);
    match (sandboxed, has(HOST_FILES)) {
        (true, true) => Some(false),
        (false, false) if has(FILE_TRANSFER) => Some(true),
        _ => None,
    }
}

fn marked(mut offered: Vec<String>, sandboxed: bool) -> Vec<String> {
    use reprisefs::dropdata::HOST_FILES;
    match host_mark(&offered, sandboxed) {
        Some(true) => offered.push(HOST_FILES.to_string()),
        Some(false) => offered.retain(|m| m != HOST_FILES),
        None => {}
    }
    offered
}

fn rested(since: u32, now: u32) -> bool {
    now.wrapping_sub(since) >= SPRING_MS
}

fn may_start(busy: bool, from_holder: bool, shut: bool) -> bool {
    !busy && from_holder && !shut
}

/// The modifiers spent on Reprise's own keys during a drag once the held
/// ones change to `held`: each stays spent until it is released, and
/// while the switcher is open every held one is the switcher's.
fn spent_after(spent: u32, held: u32, switcher_open: bool) -> u32 {
    if switcher_open { held } else { spent & held }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct DragMods {
    held: u32,
    spent: u32,
}

impl DragMods {
    fn picking(self) -> u32 {
        self.held & !self.spent
    }

    fn held(&mut self, held: u32, switcher_open: bool) -> bool {
        let before = self.picking();
        self.held = held;
        self.spent = spent_after(self.spent, held, switcher_open);
        self.picking() != before
    }

    fn spend(&mut self, mods: u32) -> bool {
        let before = self.picking();
        self.spent |= mods & self.held;
        self.picking() != before
    }
}

#[derive(Debug, Default)]
struct Learnt {
    accepted: Option<String>,
    action: DndAction,
    /// Reprise is the target under the pointer: a drop over no client is
    /// its own, so the source is not cancelled.
    reprise: bool,
    /// The actions an X11 source lists in `XdndActionList`. Smithay's
    /// XDND source tells only Copy and the action the source asks for,
    /// which is a copy unless the user holds Shift.
    listed: Vec<DndAction>,
}

#[derive(Clone)]
pub enum Origin {
    Client(WlDataSource),
    /// A drag without a source, inside the client of this surface.
    Local(WlSurface),
    Own(Arc<dyn Source>),
    /// An X11 program's drag, from the X11 window manager (`xdnd`).
    X11(Arc<dyn Source>),
}

#[derive(Clone)]
pub struct DragSource {
    origin: Origin,
    learnt: Arc<Mutex<Learnt>>,
}

impl std::fmt::Debug for DragSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let origin = match &self.origin {
            Origin::Client(s) => format!("Client({:?})", s.id()),
            Origin::Local(s) => format!("Local({:?})", s.id()),
            Origin::Own(_) => "Own".into(),
            Origin::X11(_) => "X11".into(),
        };
        f.debug_struct("DragSource")
            .field("origin", &origin)
            .field("learnt", &self.learnt)
            .finish()
    }
}

impl DragSource {
    pub fn new(origin: Origin) -> DragSource {
        DragSource {
            origin,
            learnt: Arc::default(),
        }
    }

    fn from_requested<S: Source>(source: S) -> Option<DragSource> {
        let any: Box<dyn Any> = Box::new(source);
        let origin = match any.downcast::<WlDataSource>() {
            Ok(s) => Origin::Client(*s),
            Err(any) => match any.downcast::<WlSurface>() {
                Ok(s) => Origin::Local(*s),
                Err(_) => return None,
            },
        };
        Some(DragSource::new(origin))
    }

    fn learnt(&self) -> MutexGuard<'_, Learnt> {
        self.learnt.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn same_client_as(&self, surface: &WlSurface) -> bool {
        match &self.origin {
            Origin::Client(s) => s.id().same_client_as(&surface.id()),
            Origin::Local(s) => s.id().same_client_as(&surface.id()),
            Origin::Own(_) => true,
            Origin::X11(_) => false,
        }
    }

    /// The target under the pointer changed: nothing is accepted until
    /// the new one says so, and a client's source hears that no type is.
    pub(crate) fn forget_target(&self) {
        let had = self.learnt().accepted.take();
        if had.is_some()
            && let Origin::Client(s) = &self.origin
        {
            s.target(None);
        }
    }

    fn set_reprise_target(&self, on: bool) {
        self.learnt().reprise = on;
    }

    pub(crate) fn reprise_target(&self) -> bool {
        self.learnt().reprise
    }

    pub(crate) fn is_x11(&self) -> bool {
        matches!(self.origin, Origin::X11(_))
    }

    /// Cancel the source, even one whose drop Reprise took: the drop it
    /// reads (`dnddrop`) came to nothing.
    pub(crate) fn refuse(&self) {
        self.set_reprise_target(false);
        self.cancel();
    }
}

impl IsAlive for DragSource {
    fn alive(&self) -> bool {
        match &self.origin {
            Origin::Client(s) => s.alive(),
            Origin::Local(s) => s.alive(),
            Origin::Own(s) | Origin::X11(s) => s.alive(),
        }
    }
}

impl Source for DragSource {
    fn is_client_local(&self, target: &dyn Any) -> bool {
        match &self.origin {
            Origin::Client(s) => s.is_client_local(target),
            Origin::Local(s) => s.is_client_local(target),
            Origin::Own(s) | Origin::X11(s) => s.is_client_local(target),
        }
    }

    fn metadata(&self) -> Option<SourceMetadata> {
        match &self.origin {
            Origin::Client(s) => s.metadata().map(|mut m| {
                m.mime_types = marked(m.mime_types, crate::sandbox::resource_sandboxed(s));
                m
            }),
            Origin::Local(s) => s.metadata(),
            Origin::Own(s) => s.metadata(),
            Origin::X11(s) => s.metadata().map(|mut m| {
                for a in &self.learnt().listed {
                    if !m.dnd_actions.contains(a) {
                        m.dnd_actions.push(*a);
                    }
                }
                m
            }),
        }
    }

    /// Smithay never tells a client's source what the target accepted;
    /// `wl_data_source.target` does.
    fn accepted(&self, mime_type: Option<String>) {
        self.learnt().accepted.clone_from(&mime_type);
        match &self.origin {
            Origin::Client(s) => s.target(mime_type),
            Origin::Local(s) => s.accepted(mime_type),
            Origin::Own(s) | Origin::X11(s) => s.accepted(mime_type),
        }
    }

    fn choose_action(&self, action: DndAction) {
        self.learnt().action = action;
        match &self.origin {
            Origin::Client(s) => s.choose_action(action),
            Origin::Local(s) => s.choose_action(action),
            Origin::Own(s) | Origin::X11(s) => s.choose_action(action),
        }
    }

    fn send(&self, mime_type: &str, fd: OwnedFd) {
        match &self.origin {
            Origin::Client(s) => Source::send(s, mime_type, fd),
            Origin::Local(_) => {}
            Origin::Own(s) | Origin::X11(s) => s.send(mime_type, fd),
        }
    }

    fn drop_performed(&self) {
        match &self.origin {
            Origin::Client(s) => s.drop_performed(),
            Origin::Local(s) => s.drop_performed(),
            Origin::Own(s) | Origin::X11(s) => s.drop_performed(),
        }
    }

    /// Kept from the source while Reprise is the target: Smithay cancels a
    /// drop over no client, and Reprise reads that drop itself.
    fn cancel(&self) {
        if self.learnt().reprise {
            return;
        }
        match &self.origin {
            Origin::Client(s) => s.cancel(),
            Origin::Local(s) => s.cancel(),
            Origin::Own(s) | Origin::X11(s) => s.cancel(),
        }
    }

    fn finished(&self) {
        match &self.origin {
            Origin::Client(s) => s.finished(),
            Origin::Local(s) => s.finished(),
            Origin::Own(s) | Origin::X11(s) => s.finished(),
        }
    }
}

#[derive(Default)]
pub struct Dnd {
    drag: Option<Drag>,
    /// A popup's grab asked for during the drag, set once it ends.
    held_popup: Option<(PopupGrab<State>, Serial)>,
    pub(crate) drops: crate::dnddrop::Drops,
}

impl Dnd {
    pub fn running(&self) -> bool {
        self.drag.is_some()
    }

    pub fn hold_popup_grab(&mut self, grab: PopupGrab<State>, serial: Serial) {
        self.held_popup = Some((grab, serial));
    }
}

struct Drag {
    source: DragSource,
    /// The button that holds the drag; a touch drag's is the left.
    button: u32,
    touch: Option<(TouchSlot, Point<f64, Logical>)>,
    icon: Option<Icon>,
    focus: Option<WlSurface>,
    action: DndAction,
    shortcut: bool,
    mods: DragMods,
    /// The modifiers held at the start, so a change is told to the
    /// focused client at the end.
    mods_at_start: u32,
    rest: Option<Rest>,
    over: Option<&'static [&'static str]>,
    timer: Option<RegistrationToken>,
}

struct Icon {
    surface: WlSurface,
    node: SurfaceId,
    /// Where the icon's origin sits from the pointer, moved by each
    /// commit's buffer offset.
    offset: Point<i32, Logical>,
    hook: HookId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rest {
    bar: TreeId,
    button: usize,
    since: u32,
    /// The window came up already; it does not again until the pointer
    /// leaves the button.
    sprung: bool,
}

impl DataDeviceHandler for State {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }

    fn action_choice(&mut self, available: WlDndAction, preferred: WlDndAction) -> WlDndAction {
        let asked = self.dnd.drag.as_ref().map_or(NONE, |d| d.action);
        choose_action(asked, available, preferred)
    }
}

impl WaylandDndGrabHandler for State {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        let Some(source) = DragSource::from_requested(source) else {
            log::debug!("refusing a drag from an unknown source");
            return;
        };
        let busy = self.dnd.running() || self.interaction != crate::input::Interaction::Idle;
        let shut = self.is_locked() || self.inputproto.gated();
        let ok = |focus: Option<&WlSurface>| {
            may_start(busy, focus.is_some_and(|f| source.same_client_as(f)), shut)
        };
        match type_ {
            GrabType::Pointer => match seat
                .get_pointer()
                .and_then(|p| crate::popupgrab::press_start(&p))
            {
                Some(start) if ok(start.focus.as_ref().and_then(|(f, _)| f.surface())) => {
                    return self.start_drag(source, icon, start, serial);
                }
                _ => {}
            },
            GrabType::Touch => match seat.get_touch().and_then(|t| t.grab_start_data()) {
                Some(start) if ok(start.focus.as_ref().map(|(f, _)| f)) => {
                    return self.start_touch_drag(source, icon, start, serial);
                }
                _ => {}
            },
        }
        log::debug!("refusing a client drag");
        source.cancel();
    }
}

impl DndGrabHandler for State {
    fn dropped(
        &mut self,
        _target: Option<DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        // A drop over Reprise's own target (the source says `reprise`) is
        // read from an idle (`dnddrop`, `dndsource`).
        self.drag_ended(true);
    }

    fn cancelled(&mut self, _seat: Seat<Self>, _location: Point<f64, Logical>) {
        self.drag_ended(false);
    }
}

impl State {
    pub fn drag_running(&self) -> bool {
        self.dnd.running()
    }

    pub fn x11_drag_running(&self) -> bool {
        self.dnd.drag.as_ref().is_some_and(|d| d.source.is_x11())
    }

    /// The X11 window manager's grab carries an X11 program's drag of
    /// `source` (`xdnd`): Reprise takes it up as its own. This runs in
    /// the grab, under the pointer's lock, so the action is asked for
    /// from an idle.
    pub(crate) fn x11_drag_started(&mut self, source: DragSource) -> DragSource {
        log::debug!(
            "an X11 program's drag offers {:?}",
            source.metadata().map(|m| m.mime_types)
        );
        self.begin_drag(source.clone(), None, BTN_LEFT, None);
        self.key_input.chord.spoil();
        crate::tooltip::hide(self);
        self.handle.insert_idle(|state| state.set_drag_action());
        self.xwayland.read_drag_actions();
        source
    }

    pub(crate) fn x11_drag_actions(&mut self, actions: Vec<DndAction>) {
        if let Some(d) = self.dnd.drag.as_ref()
            && d.source.is_x11()
        {
            d.source.learnt().listed = actions;
        }
    }

    pub(crate) fn x11_target_answered(&mut self, accepts: bool) {
        let Some(d) = self.dnd.drag.as_ref() else {
            return;
        };
        if d.source.learnt().accepted.is_some() == accepts {
            return;
        }
        let mime = accepts
            .then(|| d.source.metadata())
            .flatten()
            .and_then(|m| m.mime_types.into_iter().next());
        d.source.accepted(mime);
    }

    fn now_ms(&self) -> u32 {
        self.now().as_millis() as u32
    }

    pub fn start_drag(
        &mut self,
        source: DragSource,
        icon: Option<WlSurface>,
        start: GrabStartData<State>,
        serial: Serial,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            source.cancel();
            return;
        };
        let button = start.button;
        // A drag out of a menu: the menu's grab waits for the drag's end,
        // as one asked for during it does.
        if let Some(popup) = crate::popupgrab::holding(&pointer) {
            let serial = popup.serial();
            self.dnd.hold_popup_grab(popup, serial);
        }
        let grab = DnDGrab::new_pointer(&self.dh, start, source.clone(), self.seat.clone());
        self.begin_drag(source, icon, button, None);
        // The old focus leaves; the motion below enters what is under the
        // pointer.
        pointer.set_grab(self, grab, serial, Focus::Clear);
        // The modifiers held from here pick the drag's action, never a
        // chord.
        self.key_input.chord.spoil();
        crate::tooltip::hide(self);
        self.set_cursor("default");
        self.set_drag_action();
    }

    fn start_touch_drag(
        &mut self,
        source: DragSource,
        icon: Option<WlSurface>,
        start: TouchStartData<State>,
        serial: Serial,
    ) {
        let Some(touch) = self.seat.get_touch() else {
            source.cancel();
            return;
        };
        let finger = (start.slot, start.location);
        let grab = DnDGrab::new_touch(&self.dh, start, source.clone(), self.seat.clone());
        self.begin_drag(source, icon, BTN_LEFT, Some(finger));
        touch.set_grab(self, grab, serial);
        self.key_input.chord.spoil();
        crate::tooltip::hide(self);
        self.set_drag_action();
    }

    fn begin_drag(
        &mut self,
        source: DragSource,
        icon: Option<WlSurface>,
        button: u32,
        touch: Option<(TouchSlot, Point<f64, Logical>)>,
    ) {
        let mods = self
            .seat
            .get_keyboard()
            .map_or(0, |k| keys::mask(&k.modifier_state()));
        let icon = icon.map(|surface| {
            let node = self
                .scene
                .surface_create(self.layers.overlay, surface.clone());
            self.scene.set_input(node, Input::Passthrough);
            let hook = add_post_commit_hook::<State, _>(&surface, |state, _, surface| {
                state.drag_icon_committed(surface)
            });
            Icon {
                surface,
                node,
                offset: Point::default(),
                hook,
            }
        });
        let timer = self
            .handle
            .insert_source(Timer::from_duration(CURSOR_TICK), |_, _, state| {
                state.drag_tick()
            })
            .ok();
        log::debug!("drag started: {source:?}");
        self.dnd.drag = Some(Drag {
            source,
            button,
            touch,
            icon,
            focus: None,
            action: NONE,
            shortcut: false,
            mods: DragMods {
                held: mods,
                spent: 0,
            },
            mods_at_start: mods,
            rest: None,
            over: None,
            timer,
        });
        if let Some(surface) = self
            .dnd
            .drag
            .as_ref()
            .and_then(|d| d.icon.as_ref())
            .map(|i| i.surface.clone())
        {
            self.drag_icon_committed(&surface);
        }
    }

    fn drag_point(&self) -> Point<f64, Logical> {
        match self.dnd.drag.as_ref().and_then(|d| d.touch) {
            Some((_, at)) => at,
            None => self.pointer_location(),
        }
    }

    fn drag_finger(&self) -> Option<TouchSlot> {
        self.dnd.drag.as_ref().and_then(|d| d.touch).map(|(s, _)| s)
    }

    /// Finger `slot` moved to `pos`: during a touch drag held by it, the
    /// drag's focus follows it the way `drag_motion` follows the pointer.
    /// Returns true then, and the finger's motion goes nowhere else.
    pub fn drag_touch_motion(
        &mut self,
        slot: TouchSlot,
        pos: Point<f64, Logical>,
        time: InputTime,
    ) -> bool {
        if self.drag_finger() != Some(slot) {
            return false;
        }
        let focus = match self.scene_hit(pos) {
            Hit::Surface {
                surface, origin, ..
            } => Some((surface, origin)),
            _ => None,
        };
        let surface = focus.as_ref().map(|(s, _)| s.clone());
        if let Some(d) = self.dnd.drag.as_mut() {
            if d.focus != surface {
                d.source.forget_target();
            }
            d.focus = surface;
            d.touch = Some((slot, pos));
        }
        if let Some(touch) = self.seat.get_touch() {
            touch.motion(
                self,
                focus,
                &TouchMotion {
                    slot,
                    location: pos,
                    time,
                },
            );
            touch.frame(self);
        }
        self.place_drag_icon(pos);
        true
    }

    /// The pointer moved to `pos` during the drag: the surface under it
    /// gets the drag's focus. Over the compositor's own drawing the focus
    /// clears, and a desktop or taskbar target answers for itself; a
    /// taskbar button starts the spring-load rest. No other hover of the
    /// compositor's own reacts.
    pub fn drag_motion(&mut self, pos: Point<f64, Logical>, time: InputTime) {
        if !self.drag_running() || self.drag_finger().is_some() {
            return;
        }
        let hit = self.scene_hit(pos);
        let task = match hit {
            Hit::Taskbar {
                bar,
                hit: TaskbarHit::Task(i),
            } => Some((bar, i)),
            _ => None,
        };
        let focus = match hit {
            Hit::Surface {
                surface, origin, ..
            } => Some((surface, origin)),
            _ => None,
        };
        // A new target is heard from afresh: what the old one accepted
        // goes before Reprise answers for its own.
        let surface = focus.as_ref().map(|(s, _)| s.clone());
        if let Some(d) = self.dnd.drag.as_ref()
            && d.focus != surface
        {
            d.source.forget_target();
        }
        // Reprise's target is left before a client is entered, and
        // answers only once the grab has left the client (leaving resets
        // the source's action).
        let over = match focus {
            Some((surface, origin)) => {
                self.reprise_left();
                self.drag_pointer_motion(Some((surface.into(), origin)), pos, time);
                None
            }
            None => {
                // An X11 program's drag drops on a target only: Reprise's
                // drawing is one (`xdnd`).
                let reprise = self
                    .dnd
                    .drag
                    .as_ref()
                    .filter(|d| d.source.is_x11())
                    .map(|_| (PointerFocus::Reprise, pos));
                self.drag_pointer_motion(reprise, pos, time);
                self.reprise_over(pos)
            }
        };
        let now = self.now_ms();
        let Some(d) = self.dnd.drag.as_mut() else {
            return;
        };
        d.focus = surface;
        d.over = over;
        d.source.set_reprise_target(over.is_some());
        d.rest = match (d.rest, task) {
            (Some(r), Some((bar, button))) if r.bar == bar && r.button == button => Some(r),
            (_, Some((bar, button))) => Some(Rest {
                bar,
                button,
                since: now,
                sprung: false,
            }),
            (_, None) => None,
        };
        self.place_drag_icon(pos);
        self.place_drag_image(pos);
        self.update_drag_cursor();
    }

    fn drag_pointer_motion(
        &mut self,
        focus: Option<(PointerFocus, Point<f64, Logical>)>,
        pos: Point<f64, Logical>,
        time: InputTime,
    ) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: pos,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    /// Reprise's own answer for a drag over its drawing at `pos`: the
    /// cursor it shows, or None where nothing of Reprise's takes the drop.
    /// The desktop's own drag answers itself (`dndsource`), another
    /// program's is answered by `dnddrop`, and a drag without a source
    /// stays inside its client.
    fn reprise_over(&mut self, pos: Point<f64, Logical>) -> Option<&'static [&'static str]> {
        let source = self.drag_source()?;
        match source.origin {
            Origin::Own(_) => self.own_over(pos),
            Origin::Client(_) | Origin::X11(_) => self.drop_over(&source, pos),
            Origin::Local(_) => None,
        }
    }

    fn reprise_left(&mut self) {
        self.own_left();
        self.drop_left();
    }

    pub(crate) fn drag_source(&self) -> Option<DragSource> {
        self.dnd.drag.as_ref().map(|d| d.source.clone())
    }

    pub(crate) fn drag_button(&self) -> Option<u32> {
        self.dnd.drag.as_ref().map(|d| d.button)
    }

    pub fn drag_mods(&self) -> u32 {
        self.dnd.drag.as_ref().map_or(0, |d| d.mods.picking())
    }

    pub fn drag_modifiers(&mut self, held: u32) {
        let switcher_open = self.switcher.is_open();
        let Some(d) = self.dnd.drag.as_mut() else {
            return;
        };
        if d.mods.held(held, switcher_open) {
            self.set_drag_action();
        }
    }

    pub fn drag_spend(&mut self, mods: u32) {
        let Some(d) = self.dnd.drag.as_mut() else {
            return;
        };
        if d.mods.spend(mods) {
            self.set_drag_action();
        }
    }

    pub fn cancel_drag(&mut self) {
        let Some(d) = self.dnd.drag.as_ref() else {
            return;
        };
        d.source.set_reprise_target(false);
        let x11 = d.source.is_x11();
        if d.touch.is_some() {
            if let Some(touch) = self.seat.get_touch() {
                touch.unset_grab(self);
            }
        } else if let Some(pointer) = self.seat.get_pointer() {
            let serial = SERIAL_COUNTER.next_serial();
            // An X11 program ends its own side of its drag on the button's
            // release: while the button is held, the press's grab keeps
            // the pointer on its window so the release reaches it.
            match pointer
                .grab_start_data()
                .filter(|_| x11 && self.buttons_down > 0)
            {
                Some(start) => {
                    let grab = smithay::input::SeatHandler::click_grab(self, start);
                    pointer.set_grab(self, grab, serial, Focus::Keep);
                }
                None => pointer.unset_grab(self, serial, InputTime::now()),
            }
        }
        // A grab some other code set in between already ended the drag.
        self.drag_ended(false);
    }

    /// Ask for the action the held modifiers and the drag's button pick.
    /// A target settles its action only on an enter or a change of its
    /// own, so a change over a client leaves and enters that client again.
    fn set_drag_action(&mut self) {
        let Some(d) = self.dnd.drag.as_mut() else {
            return;
        };
        let mods = d.mods.picking();
        let (action, shortcut) = compositor_action(
            d.button == BTN_LEFT,
            mods & keys::MOD_CTRL != 0,
            mods & keys::MOD_SHIFT != 0,
            mods & keys::MOD_ALT != 0,
        );
        d.shortcut = shortcut;
        let changed = d.action != action;
        d.action = action;
        let over_client = d.focus.is_some();
        let touch = d.touch;
        let pos = self.pointer_location();
        let time = InputTime::now();
        if let Some((slot, at)) = touch {
            if changed && over_client {
                if let Some(d) = self.dnd.drag.as_mut() {
                    d.focus = None;
                }
                if let Some(touch) = self.seat.get_touch() {
                    let ev = TouchMotion {
                        slot,
                        location: at,
                        time,
                    };
                    touch.motion(self, None, &ev);
                }
            }
            self.drag_touch_motion(slot, at, time);
            return;
        }
        if !over_client {
            self.drag_motion(pos, time);
        } else if changed {
            if let Some(d) = self.dnd.drag.as_mut() {
                d.focus = None;
            }
            self.drag_pointer_motion(None, pos, time);
            self.drag_motion(pos, time);
        } else {
            self.update_drag_cursor();
        }
    }

    fn drag_icon_committed(&mut self, surface: &WlSurface) {
        let Some(icon) = self
            .dnd
            .drag
            .as_mut()
            .and_then(|d| d.icon.as_mut())
            .filter(|i| &i.surface == surface)
        else {
            return;
        };
        let delta = with_states(surface, |s| {
            s.cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_delta
                .take()
        });
        if let Some(delta) = delta {
            icon.offset += delta;
        }
        let pos = self.drag_point();
        self.place_drag_icon(pos);
    }

    fn place_drag_icon(&mut self, pos: Point<f64, Logical>) {
        let Some(icon) = self.dnd.drag.as_ref().and_then(|d| d.icon.as_ref()) else {
            return;
        };
        let at = pos.to_i32_round() + icon.offset;
        let node = icon.node;
        if self.scene.position(node) != Some(at) {
            self.scene.set_position(node, at.x, at.y);
            self.redraw_content();
        }
    }

    fn update_drag_cursor(&mut self) {
        let Some(d) = self.dnd.drag.as_ref().filter(|d| d.touch.is_none()) else {
            return;
        };
        if d.source.is_x11()
            && d.focus
                .as_ref()
                .is_some_and(|f| self.x11_window_of(f).is_some())
        {
            if self.cursors.drag.take().is_some() {
                self.redraw_content();
            }
            return;
        }
        let names = match d.over {
            Some(names) => names,
            None => {
                let over = d.focus.is_some();
                let learnt = d.source.learnt();
                let (accepted, action) = match d.source.origin {
                    Origin::Local(_) => (over, MOVE),
                    _ => (over && learnt.accepted.is_some(), learnt.action),
                };
                cursor_for(accepted, action, d.shortcut)
            }
        };
        let name = names
            .iter()
            .copied()
            .find(|n| self.cursors.has(n))
            .unwrap_or("default");
        if self.cursors.drag != Some(name) {
            log::debug!("drag cursor {name}");
            self.cursors.drag = Some(name);
            self.cursor = CursorImageStatus::default_named();
            self.redraw_content();
        }
    }

    fn drag_tick(&mut self) -> TimeoutAction {
        let Some(d) = self.dnd.drag.as_ref() else {
            return TimeoutAction::Drop;
        };
        // A client gone mid-drag takes its source along. This timer ends
        // by returning Drop, not through `drag_ended`.
        if !d.source.alive() {
            if let Some(d) = self.dnd.drag.as_mut() {
                d.timer = None;
            }
            self.cancel_drag();
            return TimeoutAction::Drop;
        }
        self.rehover();
        self.update_drag_cursor();
        self.spring();
        self.field_tick();
        TimeoutAction::ToDuration(CURSOR_TICK)
    }

    fn rehover(&mut self) {
        let Some(focus) = self.dnd.drag.as_ref().map(|d| d.focus.clone()) else {
            return;
        };
        let pos = self.drag_point();
        let still = match (self.scene_hit(pos), focus) {
            (Hit::Surface { surface, .. }, Some(f)) => surface == f,
            (Hit::Surface { .. }, None) => false,
            (_, f) => f.is_none() && self.drag_finger().is_some(),
        };
        if still {
            return;
        }
        match self.drag_finger() {
            Some(slot) => {
                self.drag_touch_motion(slot, pos, InputTime::now());
            }
            None => self.drag_motion(pos, InputTime::now()),
        }
    }

    fn spring(&mut self) {
        let now = self.now_ms();
        let Some(rest) = self.dnd.drag.as_mut().and_then(|d| d.rest.as_mut()) else {
            return;
        };
        if rest.sprung || !rested(rest.since, now) {
            return;
        }
        rest.sprung = true;
        let (bar, button) = (rest.bar, rest.button);
        let Some(windows) = self.bar(bar).and_then(|b| b.buttons.get(button).cloned()) else {
            return;
        };
        let Some(w) = self
            .focus_order
            .iter()
            .rev()
            .copied()
            .find(|w| windows.contains(w))
            .or_else(|| windows.first().copied())
        else {
            return;
        };
        if self.win(w).is_some_and(|w| w.minimized) {
            self.set_minimized(w, false);
        }
        self.focus_window(w);
    }

    /// The drag ended: `dropped`, or cancelled or replaced by another grab.
    /// This may run under the pointer's lock (from the grab), so the
    /// follow-up that needs the pointer runs from an idle: a popup grab
    /// held back during it resumes, the pointer enters whatever is under
    /// it again, and the desktop's own drop over a target of Reprise's
    /// lands (`dndsource`), and so does another program's (`dnddrop`).
    fn drag_ended(&mut self, dropped: bool) {
        let Some(d) = self.dnd.drag.take() else {
            return;
        };
        log::debug!("drag ended");
        let own_drop = dropped && d.source.learnt().reprise;
        self.own_drag_ended(own_drop);
        self.drop_drag_ended(&d.source, own_drop);
        if let Some(t) = d.timer {
            self.handle.remove(t);
        }
        if let Some(icon) = d.icon {
            remove_post_commit_hook(&icon.surface, &icon.hook);
            self.scene.destroy(icon.node);
        }
        self.cursors.drag = None;
        // The keys pressed during the drag reached no client (the Escape
        // that cancelled it among them); the key route keeps their
        // releases from it too (`KeyInput::taken`).
        let mods_changed = d.mods.held != d.mods_at_start;
        self.redraw_all();
        self.handle
            .insert_idle(move |state| state.after_drag(mods_changed));
    }

    fn after_drag(&mut self, mods_changed: bool) {
        if let Some((grab, serial)) = self.dnd.held_popup.take()
            && !grab.has_ended()
        {
            self.set_popup_grab(grab, serial);
        }
        if mods_changed
            && let Some(keyboard) = self.seat.get_keyboard()
            && let Some(focus) = keyboard.current_focus()
        {
            keyboard.set_focus(self, None, SERIAL_COUNTER.next_serial());
            keyboard.set_focus(self, Some(focus), SERIAL_COUNTER.next_serial());
        }
        let pos = self.pointer_location();
        self.repoint(pos, InputTime::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifiers_pick_the_compositor_action() {
        assert_eq!(compositor_action(true, false, false, false), (NONE, false));
        assert_eq!(compositor_action(true, true, false, false), (COPY, false));
        assert_eq!(compositor_action(true, false, true, false), (MOVE, false));
        assert_eq!(compositor_action(true, false, false, true), (ASK, true));
        assert_eq!(compositor_action(true, true, true, false), (ASK, true));
    }

    #[test]
    fn the_right_button_asks_whatever_is_held() {
        assert_eq!(compositor_action(false, false, false, false), (ASK, false));
        assert_eq!(compositor_action(false, true, false, false), (ASK, false));
        assert_eq!(compositor_action(false, false, false, true), (ASK, false));
    }

    #[test]
    fn the_target_takes_the_asked_action_when_both_sides_allow_it() {
        let all = WlDndAction::Copy | WlDndAction::Move | WlDndAction::Ask;
        assert_eq!(
            choose_action(COPY, all, WlDndAction::Move),
            WlDndAction::Copy
        );
        assert_eq!(choose_action(ASK, all, WlDndAction::Copy), WlDndAction::Ask);
    }

    #[test]
    fn with_nothing_asked_or_the_asked_action_missing_the_target_prefers() {
        let copy_move = WlDndAction::Copy | WlDndAction::Move;
        assert_eq!(
            choose_action(NONE, copy_move, WlDndAction::Move),
            WlDndAction::Move
        );
        assert_eq!(
            choose_action(ASK, copy_move, WlDndAction::Move),
            WlDndAction::Move
        );
        assert_eq!(
            choose_action(MOVE, WlDndAction::Copy, WlDndAction::Copy),
            WlDndAction::Copy
        );
        assert_eq!(
            choose_action(COPY, WlDndAction::empty(), WlDndAction::Copy),
            WlDndAction::empty()
        );
    }

    #[test]
    fn cursor_names_the_action_a_release_performs() {
        assert_eq!(cursor_for(false, COPY, false)[0], "dnd-no-drop");
        assert_eq!(cursor_for(true, NONE, false)[0], "dnd-no-drop");
        assert_eq!(cursor_for(true, COPY, false)[0], "dnd-copy");
        assert_eq!(cursor_for(true, MOVE, false)[0], "dnd-move");
        assert_eq!(cursor_for(true, ASK, false)[0], "dnd-ask");
        assert_eq!(cursor_for(true, ASK, true)[0], "dnd-link");
        assert_eq!(cursor_for(true, COPY, true)[0], "dnd-copy");
    }

    #[test]
    fn a_drag_from_outside_a_sandbox_is_marked_as_naming_its_files() {
        use reprisefs::dropdata::{FILE_TRANSFER, HOST_FILES, URI_LIST};
        let m = |ms: &[&str]| ms.iter().map(|m| m.to_string()).collect::<Vec<_>>();
        assert_eq!(host_mark(&m(&[FILE_TRANSFER, URI_LIST]), false), Some(true));
        assert_eq!(host_mark(&m(&[FILE_TRANSFER, HOST_FILES]), false), None);
        assert_eq!(host_mark(&m(&[URI_LIST]), false), None);
        assert_eq!(
            host_mark(&m(&[FILE_TRANSFER, HOST_FILES]), true),
            Some(false)
        );
        assert_eq!(host_mark(&m(&[FILE_TRANSFER, URI_LIST]), true), None);
    }

    #[test]
    fn a_target_sees_the_marked_types() {
        use reprisefs::dropdata::{FILE_TRANSFER, HOST_FILES, URI_LIST};
        let m = |ms: &[&str]| ms.iter().map(|m| m.to_string()).collect::<Vec<_>>();
        assert_eq!(
            marked(m(&[FILE_TRANSFER, URI_LIST]), false),
            m(&[FILE_TRANSFER, URI_LIST, HOST_FILES])
        );
        assert_eq!(
            marked(m(&[HOST_FILES, FILE_TRANSFER, HOST_FILES]), true),
            m(&[FILE_TRANSFER])
        );
        assert_eq!(marked(m(&[URI_LIST]), false), m(&[URI_LIST]));
    }

    #[test]
    fn a_modifier_held_for_reprises_keys_picks_no_action_until_released() {
        const SHIFT: u32 = keys::MOD_SHIFT;
        const ALT: u32 = keys::MOD_ALT;
        let spent = spent_after(ALT, ALT | SHIFT, true);
        assert_eq!(spent, ALT | SHIFT);
        let spent = spent_after(spent, SHIFT, false);
        assert_eq!(spent, SHIFT);
        let spent = spent_after(spent, 0, false);
        assert_eq!(spent, 0);
        assert_eq!(spent_after(spent, ALT, false), 0);
    }

    #[test]
    fn the_lock_and_a_capture_refuse_a_drag() {
        assert!(may_start(false, true, false));
        assert!(!may_start(false, true, true), "locked or captured");
        assert!(!may_start(true, true, false), "another drag runs");
        assert!(!may_start(false, false, false), "another client's press");
    }

    #[test]
    fn a_modifier_picks_the_action_until_reprises_own_keys_spend_it() {
        let (ctrl, alt) = (keys::MOD_CTRL, keys::MOD_ALT);
        let mut m = DragMods::default();
        assert!(m.held(ctrl, false), "Ctrl pressed mid-drag picks a copy");
        assert_eq!(m.picking(), ctrl);
        assert!(m.held(ctrl | alt, false));
        assert!(m.spend(alt));
        assert_eq!(m.picking(), ctrl, "the Alt of Alt+Tab picks nothing");
        assert!(
            !m.spend(keys::MOD_SHIFT),
            "a modifier not held is not spent"
        );
        assert!(!m.held(ctrl, false));
        assert!(m.held(ctrl | alt, false));
        assert_eq!(m.picking(), ctrl | alt);
    }

    #[test]
    fn a_taskbar_button_springs_after_the_tooltip_delay() {
        assert_eq!(SPRING_MS, 500);
        assert!(!rested(1000, 1499));
        assert!(rested(1000, 1500));
        assert!(rested(u32::MAX - 100, 400));
        assert!(!rested(u32::MAX - 100, 300));
    }

    #[derive(Default)]
    struct Heard(Mutex<Vec<String>>);

    impl IsAlive for Heard {
        fn alive(&self) -> bool {
            true
        }
    }

    impl Source for Heard {
        fn metadata(&self) -> Option<SourceMetadata> {
            Some(SourceMetadata {
                mime_types: vec!["text/uri-list".into()],
                dnd_actions: [COPY].into_iter().collect(),
            })
        }
        fn choose_action(&self, action: DndAction) {
            self.0.lock().unwrap().push(format!("{action:?}"));
        }
        fn send(&self, mime_type: &str, _fd: OwnedFd) {
            self.0.lock().unwrap().push(format!("send {mime_type}"));
        }
        fn drop_performed(&self) {
            self.0.lock().unwrap().push("drop".into());
        }
        fn cancel(&self) {
            self.0.lock().unwrap().push("cancel".into());
        }
        fn finished(&self) {
            self.0.lock().unwrap().push("finished".into());
        }
    }

    #[test]
    fn an_x11_drag_allows_the_actions_its_source_lists() {
        let source = DragSource::new(Origin::X11(Arc::new(Heard::default())));
        let actions = || source.metadata().unwrap().dnd_actions.to_vec();
        assert_eq!(actions(), [COPY], "only what the source asks for");
        source.learnt().listed = vec![COPY, MOVE];
        assert_eq!(actions(), [COPY, MOVE], "XdndActionList adds the move");
    }

    #[test]
    fn an_x11_drag_on_reprises_target_is_a_drop_its_source_hears_through() {
        use crate::xdnd::Offer;
        use smithay::input::dnd::OfferData;
        let heard = Arc::new(Heard::default());
        let source = DragSource::new(Origin::X11(heard.clone()));
        assert!(source.is_x11());
        let offer = Offer::Reprise(source.clone());
        assert!(!offer.validated(), "nothing of Reprise's takes it yet");
        source.set_reprise_target(true);
        source.choose_action(COPY);
        assert!(offer.validated(), "a folder icon takes it");
        source.cancel();
        source.refuse();
        assert_eq!(*heard.0.lock().unwrap(), ["Copy", "cancel"]);
        assert!(!offer.validated());
    }

    #[test]
    fn every_cursor_list_ends_in_the_default() {
        for accepted in [false, true] {
            for action in [NONE, COPY, MOVE, ASK] {
                for shortcut in [false, true] {
                    let names = cursor_for(accepted, action, shortcut);
                    assert_eq!(names.last(), Some(&"default"));
                }
            }
        }
    }
}
