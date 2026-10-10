//! A client popup's pointer grab ([`PopupPointer`]): Smithay's `PopupPointerGrab`, which
//! keeps the pointer on the popup's client and dismisses the popups on a press elsewhere,
//! re-keyed under each press inside that client. Smithay grants a drag out of a menu
//! (`wl_data_device.start_drag`) or a submenu only while the pointer's grab carries that
//! press's serial, and only the popup grab carries the serial the popup was opened with.
//! A drag out of a menu takes the grab along (`dnd`); it comes back if the menu is still open.

use smithay::{
    backend::input::{ButtonState, InputTime},
    desktop::{PopupGrab, PopupPointerGrab, PopupUngrabStrategy},
    input::{
        SeatHandler,
        pointer::{
            AxisFrame, ButtonEvent, Focus, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, GrabStartData,
            MotionEvent, PointerGrab, PointerHandle, PointerInnerHandle, RelativeMotionEvent,
        },
    },
    utils::{Logical, Point, SERIAL_COUNTER, Serial},
};

use crate::compositor::State;

type Focused = Option<(<State as SeatHandler>::PointerFocus, Point<f64, Logical>)>;

pub struct PopupPointer {
    popup: PopupGrab<State>,
    inner: PopupPointerGrab<State>,
    /// The last press inside the popup's client, where and on what it
    /// landed: the start a drag out of the menu takes. The grab's own
    /// start data stays the popup grab's, whose focus is the popups' root.
    press: Option<GrabStartData<State>>,
    /// Set while the grab hands over to its copy under a press's serial:
    /// the popup's keyboard grab stays.
    rekeying: bool,
}

impl PopupPointer {
    pub fn new(popup: &PopupGrab<State>) -> PopupPointer {
        PopupPointer {
            popup: popup.clone(),
            inner: PopupPointerGrab::new(popup),
            press: None,
            rekeying: false,
        }
    }
}

pub fn holding(pointer: &PointerHandle<State>) -> Option<PopupGrab<State>> {
    pointer
        .with_grab(|_, g| g.downcast_ref::<PopupPointer>().map(|p| p.popup.clone()))
        .flatten()
}

/// Close the client popups whose grab holds the seat, as a press outside
/// them would: the keyboard goes back to the surface they opened on.
/// False when no client popup holds it.
pub fn dismiss(state: &mut State) -> bool {
    let Some(pointer) = state.seat.get_pointer() else {
        return false;
    };
    let Some(mut popup) = holding(&pointer) else {
        return false;
    };
    popup.ungrab(PopupUngrabStrategy::All);
    // Unsetting the pointer grab ends the keyboard grab with it.
    pointer.unset_grab(state, SERIAL_COUNTER.next_serial(), InputTime::now());
    true
}

/// The start of the press `pointer` holds: under a client popup's grab,
/// the last press inside its client while its button is down.
pub fn press_start(pointer: &PointerHandle<State>) -> Option<GrabStartData<State>> {
    pointer
        .with_grab(|_, g| match g.downcast_ref::<PopupPointer>() {
            Some(p) => p.press.clone(),
            None => Some(g.start_data().clone()),
        })
        .flatten()
}

/// Whether the pointer may pass from its grab to a popup grab asked for
/// with `serial` on a chain of popups whose topmost grab was asked for
/// with `parent` (the new popup's own serial when it is the first). The
/// pointer's grab is keyed `held` and, when it is a client popup's, that
/// popup's grab was asked for with `popup`. It may when the grab is the
/// press the new popup names, or the chain's own, under its popup serial
/// or a press re-keyed it to.
pub fn joins(held: Serial, popup: Option<Serial>, serial: Serial, parent: Serial) -> bool {
    held == serial || held == parent || popup == Some(parent)
}

/// [`joins`] for `pointer` as it stands; true when it has no grab.
pub fn pointer_joins(pointer: &PointerHandle<State>, serial: Serial, parent: Serial) -> bool {
    pointer
        .with_grab(|held, g| {
            let popup = g.downcast_ref::<PopupPointer>().map(|p| p.popup.serial());
            joins(held, popup, serial, parent)
        })
        .unwrap_or(true)
}

impl PointerGrab<State> for PopupPointer {
    fn motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        focus: Focused,
        event: &MotionEvent,
    ) {
        self.inner.motion(data, handle, focus, event);
    }

    fn relative_motion(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        focus: Focused,
        event: &RelativeMotionEvent,
    ) {
        self.inner.relative_motion(data, handle, focus, event);
    }

    /// A press inside the popup's client goes to it and the grab carries
    /// its serial from then on; one elsewhere dismisses the popups. A drag
    /// starts only from a press still held.
    fn button(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &ButtonEvent,
    ) {
        self.inner.button(data, handle, event);
        if event.state == ButtonState::Released {
            self.press.take_if(|p| p.button == event.button);
        } else if !self.popup.has_ended() {
            self.rekeying = true;
            let mut next = PopupPointer::new(&self.popup);
            next.press = Some(GrabStartData {
                focus: handle.current_focus(),
                button: event.button,
                location: handle.current_location(),
            });
            handle.set_grab(self, data, event.serial, Focus::Keep, next);
        }
    }

    fn axis(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        details: AxisFrame,
    ) {
        self.inner.axis(data, handle, details);
    }

    fn frame(&mut self, data: &mut State, handle: &mut PointerInnerHandle<'_, State>) {
        self.inner.frame(data, handle);
    }

    fn gesture_swipe_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeBeginEvent,
    ) {
        self.inner.gesture_swipe_begin(data, handle, event);
    }

    fn gesture_swipe_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeUpdateEvent,
    ) {
        self.inner.gesture_swipe_update(data, handle, event);
    }

    fn gesture_swipe_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureSwipeEndEvent,
    ) {
        self.inner.gesture_swipe_end(data, handle, event);
    }

    fn gesture_pinch_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchBeginEvent,
    ) {
        self.inner.gesture_pinch_begin(data, handle, event);
    }

    fn gesture_pinch_update(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchUpdateEvent,
    ) {
        self.inner.gesture_pinch_update(data, handle, event);
    }

    fn gesture_pinch_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GesturePinchEndEvent,
    ) {
        self.inner.gesture_pinch_end(data, handle, event);
    }

    fn gesture_hold_begin(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldBeginEvent,
    ) {
        self.inner.gesture_hold_begin(data, handle, event);
    }

    fn gesture_hold_end(
        &mut self,
        data: &mut State,
        handle: &mut PointerInnerHandle<'_, State>,
        event: &GestureHoldEndEvent,
    ) {
        self.inner.gesture_hold_end(data, handle, event);
    }

    fn start_data(&self) -> &GrabStartData<State> {
        self.inner.start_data()
    }

    /// The grab ends, or another takes the pointer: the popup's keyboard
    /// grab goes with it, unless the grab only passed to its own copy.
    fn unset(&mut self, data: &mut State) {
        if !self.rekeying {
            self.inner.unset(data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(n: u32) -> Serial {
        Serial::from(n)
    }

    #[test]
    fn a_first_popup_takes_the_press_that_opened_it() {
        assert!(joins(s(7), None, s(7), s(7)));
        assert!(!joins(s(3), None, s(7), s(7)));
    }

    #[test]
    fn a_submenu_joins_its_menu_after_a_press_on_the_menu() {
        // The menu was opened with 7; a press on it (12) re-keyed the
        // pointer's grab.
        assert!(joins(s(12), Some(s(7)), s(12), s(7)));
        // A submenu opened on hover names another serial (15): the grab
        // is still the menu's.
        assert!(joins(s(12), Some(s(7)), s(15), s(7)));
        assert!(joins(s(7), Some(s(7)), s(15), s(7)));
    }
}
