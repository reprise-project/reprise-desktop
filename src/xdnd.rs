//! The seat's pointer focus, and drag and drop with X11 programs.
//!
//! [`PointerFocus`] is a client surface or, during an X11 program's drag,
//! Reprise's own drawing. As a drag target ([`DndFocus`]) it speaks XDND to
//! X11 windows and `wl_data_device` to other clients, handing both the drag's
//! [`DragSource`].
//!
//! An X11 drag is started by the X11 window manager under its own grab.
//! When that grab first offers the drag to a target, Reprise takes it up
//! ([`State::x11_drag_started`]) and routes motion, cursor, keys and drops
//! as for any other drag (`dnd`). Its source is wrapped like a client's
//! ([`Origin::X11`]). Over Reprise's drawing the focus is
//! [`PointerFocus::Reprise`], whose offer holds when Reprise takes the drop.
//! An X11 drag over its own program's windows stays that program's.

use crate::mutex::lock;
use std::{
    borrow::Cow,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use smithay::{
    backend::input::InputTime,
    input::{
        Seat,
        dnd::{DndFocus, OfferData, Source},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
    },
    reexports::wayland_server::{DisplayHandle, Resource, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Logical, Point, Serial},
    wayland::{seat::WaylandFocus, selection::data_device::WlOfferData},
    xwayland::{X11Surface, XWaylandClientData, xwm::XwmOfferData},
};

use crate::{
    compositor::State,
    dnd::{DragSource, Origin},
};

#[derive(Clone, Debug, PartialEq)]
pub enum PointerFocus {
    Surface(WlSurface),
    /// Reprise's own drawing, as the target of an X11 program's drag. Only a
    /// grab is handed it (`State::pointer_motion`); the pointer is never focused on it.
    Reprise,
}

impl PointerFocus {
    pub fn surface(&self) -> Option<&WlSurface> {
        match self {
            PointerFocus::Surface(s) => Some(s),
            PointerFocus::Reprise => None,
        }
    }
}

impl From<WlSurface> for PointerFocus {
    fn from(s: WlSurface) -> Self {
        PointerFocus::Surface(s)
    }
}

impl PartialEq<WlSurface> for PointerFocus {
    fn eq(&self, other: &WlSurface) -> bool {
        self.surface() == Some(other)
    }
}

impl IsAlive for PointerFocus {
    fn alive(&self) -> bool {
        match self {
            PointerFocus::Surface(s) => s.alive(),
            PointerFocus::Reprise => true,
        }
    }
}

impl WaylandFocus for PointerFocus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.surface().map(Cow::Borrowed)
    }
}

/// Forward a `PointerTarget` call to the client surface after any motion held for it
/// (`coalesce`); Reprise's own drawing hears nothing.
macro_rules! forward {
    ($($name:ident($($arg:ident: $ty:ty),*);)*) => {$(
        fn $name(&self, seat: &Seat<State>, data: &mut State, $($arg: $ty),*) {
            if let PointerFocus::Surface(s) = self {
                data.flush_motion();
                PointerTarget::<State>::$name(s, seat, data, $($arg),*);
            }
        }
    )*};
}

impl PointerTarget<State> for PointerFocus {
    /// Xwayland puts a press where its last motion was, and an enter alone leaves
    /// that at the screen's corner, so a motion to the same point follows the enter.
    fn enter(&self, seat: &Seat<State>, data: &mut State, event: &MotionEvent) {
        if let PointerFocus::Surface(s) = self {
            data.flush_motion();
            PointerTarget::<State>::enter(s, seat, data, event);
            if from_xwayland(s) {
                PointerTarget::<State>::motion(s, seat, data, event);
            }
        }
    }

    fn motion(&self, _: &Seat<State>, data: &mut State, event: &MotionEvent) {
        if let PointerFocus::Surface(s) = self {
            data.hold_motion(s, event);
        }
    }

    fn relative_motion(&self, _: &Seat<State>, data: &mut State, event: &RelativeMotionEvent) {
        if let PointerFocus::Surface(s) = self {
            data.hold_relative(s, event);
        }
    }

    fn frame(&self, seat: &Seat<State>, data: &mut State) {
        if let PointerFocus::Surface(s) = self
            && !data.pointers.motions.frame_held(s)
        {
            data.flush_motion();
            PointerTarget::<State>::frame(s, seat, data);
        }
    }

    forward! {
        button(event: &ButtonEvent);
        axis(frame: AxisFrame);
        gesture_swipe_begin(event: &GestureSwipeBeginEvent);
        gesture_swipe_update(event: &GestureSwipeUpdateEvent);
        gesture_swipe_end(event: &GestureSwipeEndEvent);
        gesture_pinch_begin(event: &GesturePinchBeginEvent);
        gesture_pinch_update(event: &GesturePinchUpdateEvent);
        gesture_pinch_end(event: &GesturePinchEndEvent);
        gesture_hold_begin(event: &GestureHoldBeginEvent);
        gesture_hold_end(event: &GestureHoldEndEvent);
        leave(serial: Serial, time: InputTime);
    }
}

fn from_xwayland(s: &WlSurface) -> bool {
    s.client()
        .is_some_and(|c| c.get_data::<XWaylandClientData>().is_some())
}

pub enum Offer {
    Wl(WlOfferData<DragSource>),
    X11(X11Offer),
    Reprise(DragSource),
}

/// An X11 target's offer, shared with the drag motion `coalesce` holds for it.
#[derive(Clone)]
pub struct X11Offer(Arc<Mutex<XwmOfferData<DragSource>>>);

impl fmt::Debug for X11Offer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("X11Offer")
    }
}

impl X11Offer {
    fn lock(&self) -> MutexGuard<'_, XwmOfferData<DragSource>> {
        lock(&self.0)
    }

    /// Send `x` an `XdndPosition`, or keep `at` for when its `XdndStatus` comes.
    pub fn motion(
        &self,
        x: &X11Surface,
        data: &mut State,
        seat: &Seat<State>,
        at: Point<f64, Logical>,
        time: InputTime,
    ) {
        DndFocus::motion(x, data, Some(&mut *self.lock()), seat, at, time);
    }
}

impl OfferData for Offer {
    fn disable(&self) {
        match self {
            Offer::Wl(o) => o.disable(),
            Offer::X11(o) => o.lock().disable(),
            Offer::Reprise(_) => {}
        }
    }

    fn drop(&self) {
        match self {
            Offer::Wl(o) => o.drop(),
            Offer::X11(o) => OfferData::drop(&*o.lock()),
            Offer::Reprise(_) => {}
        }
    }

    fn validated(&self) -> bool {
        match self {
            Offer::Wl(o) => o.validated(),
            Offer::X11(o) => o.lock().validated(),
            Offer::Reprise(s) => s.reprise_target(),
        }
    }
}

enum Target {
    X11(Box<X11Surface>),
    Wl(WlSurface),
    Reprise,
}

impl PointerFocus {
    fn target(&self, state: &State) -> Target {
        match self {
            PointerFocus::Surface(s) => match state.x11_window_of(s) {
                Some(x) => Target::X11(Box::new(x)),
                None => Target::Wl(s.clone()),
            },
            PointerFocus::Reprise => Target::Reprise,
        }
    }
}

impl DndFocus<State> for PointerFocus {
    type OfferData<S>
        = Offer
    where
        S: Source;

    fn enter<S: Source>(
        &self,
        data: &mut State,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Offer> {
        data.flush_motion();
        data.dnd_actions_changed();
        let source = Arc::new(data.drag_source_for(source)?);
        match self.target(data) {
            Target::X11(x) => DndFocus::enter(&*x, data, dh, source, seat, location, serial)
                .map(|o| Offer::X11(X11Offer(Arc::new(Mutex::new(o))))),
            Target::Wl(s) => {
                DndFocus::enter(&s, data, dh, source, seat, location, serial).map(Offer::Wl)
            }
            Target::Reprise => Some(Offer::Reprise((*source).clone())),
        }
    }

    fn motion<S: Source>(
        &self,
        data: &mut State,
        offer: Option<&mut Offer>,
        seat: &Seat<State>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        match (self.target(data), offer) {
            (Target::X11(_), Some(Offer::X11(o))) => {
                // An X11 target answers in a status the source never hears of;
                // what it last said stands for its accept.
                data.x11_target_answered(o.lock().validated());
                if let PointerFocus::Surface(s) = self {
                    data.hold_drag(s, location, time, Some(o.clone()));
                }
            }
            (Target::X11(x), None) => {
                data.flush_motion();
                DndFocus::<State>::motion::<S>(&*x, data, None, seat, location, time)
            }
            (Target::Wl(s), Some(Offer::Wl(o))) => {
                // The offer only passes on a change of the source's actions.
                if data.dnd_actions_changed() {
                    data.flush_motion();
                    DndFocus::motion(&s, data, Some(o), seat, location, time)
                } else {
                    data.hold_drag(&s, location, time, None)
                }
            }
            (Target::Wl(s), None) => data.hold_drag(&s, location, time, None),
            _ => {}
        }
    }

    fn leave<S: Source>(&self, data: &mut State, offer: Option<&mut Offer>, seat: &Seat<State>) {
        data.flush_motion();
        match (self.target(data), offer) {
            (Target::X11(x), Some(Offer::X11(o))) => {
                DndFocus::leave(&*x, data, Some(&mut *o.lock()), seat)
            }
            (Target::X11(x), None) => DndFocus::<State>::leave::<S>(&*x, data, None, seat),
            (Target::Wl(s), Some(Offer::Wl(o))) => DndFocus::leave(&s, data, Some(o), seat),
            (Target::Wl(s), None) => DndFocus::<State>::leave::<S>(&s, data, None, seat),
            _ => {}
        }
    }

    fn drop<S: Source>(&self, data: &mut State, offer: Option<&mut Offer>, seat: &Seat<State>) {
        data.flush_motion();
        match (self.target(data), offer) {
            (Target::X11(x), Some(Offer::X11(o))) => {
                DndFocus::drop(&*x, data, Some(&mut *o.lock()), seat)
            }
            (Target::X11(x), None) => DndFocus::<State>::drop::<S>(&*x, data, None, seat),
            (Target::Wl(s), Some(Offer::Wl(o))) => DndFocus::drop(&s, data, Some(o), seat),
            (Target::Wl(s), None) => DndFocus::<State>::drop::<S>(&s, data, None, seat),
            _ => {}
        }
    }
}

impl State {
    pub fn x11_window_of(&self, surface: &WlSurface) -> Option<X11Surface> {
        let w = self.window_for_surface(surface)?;
        self.win(w)?.x11().cloned()
    }

    /// The drag source a target is handed for the grab's `source`: the running
    /// drag's, or a new X11 one taken up here. None for any other.
    fn drag_source_for<S: Source>(&mut self, source: Arc<S>) -> Option<DragSource> {
        if let Some(s) = self.drag_source() {
            return Some(s);
        }
        let xwm = self.xwayland.wm_id()?;
        if !source.is_client_local(&xwm) {
            log::debug!("refusing a drag Reprise does not carry");
            return None;
        }
        let source: Arc<dyn Source> = source;
        Some(self.x11_drag_started(DragSource::new(Origin::X11(source))))
    }
}
