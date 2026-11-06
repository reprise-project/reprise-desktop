//! The computer's own pointer in lock mode: a position on the Console outputs that drives only
//! the console's lock dialog, apart from the seat's pointer on the Remote outputs.

use smithay::backend::input::InputTime;
use smithay::input::pointer::{CursorIcon, CursorImageStatus};
use smithay::output::Output;
use smithay::utils::{Logical, Point};

use super::Side;
use crate::compositor::State;
use crate::inputproto::InputSource;

/// The cursor the Console outputs draw in lock mode.
pub static CURSOR: CursorImageStatus = CursorImageStatus::Named(CursorIcon::Default);

/// Lock mode begins: the computer's pointer stays where it was, on its own outputs; the seat lets
/// go of its buttons, fingers and pens and its pointer goes to the Remote outputs.
pub fn begin(state: &mut State) {
    let at = state.pointer_location();
    state.console.pointer = state.clamp_to_side(at, Side::Console);
    state.with_source(InputSource::Other, |s| {
        let now = InputTime::now();
        s.release_all_buttons(now);
        s.cancel_client_touch(now);
        s.tools_leave(now);
        s.pointer_to(at, now);
    });
    redraw(state);
}

/// Lock mode ended: the seat's pointer comes back where the computer's was.
pub fn end(state: &mut State) {
    let at = state.console.pointer;
    state.with_source(InputSource::Other, |s| s.pointer_to(at, InputTime::now()));
}

/// The computer's pointer moved to `pos`, held on the Console outputs.
pub fn to(state: &mut State, pos: Point<f64, Logical>) {
    state.console.pointer = state.clamp_to_side(pos, Side::Console);
    redraw(state);
}

pub fn by(state: &mut State, delta: Point<f64, Logical>) {
    to(state, state.console.pointer + delta);
}

/// A button of the computer's: a press goes to the console's lock dialog where its pointer is.
pub fn button(state: &mut State, button: u32, pressed: bool) {
    if pressed {
        let at = state.console.pointer;
        state.with_source(InputSource::Other, |s| {
            crate::lockview::button(s, button, at)
        });
    }
}

/// Where `output` draws the pointer: in lock mode a Console output draws the computer's,
/// with [`CURSOR`]; any other the seat's at `seat`.
pub fn drawn_on(
    state: &State,
    output: &Output,
    seat: Point<f64, Logical>,
) -> (Point<f64, Logical>, bool) {
    if super::remote_side(state) && super::side_of(output) == Side::Console {
        (state.console.pointer, true)
    } else {
        (seat, false)
    }
}

fn redraw(state: &mut State) {
    let seat = state.pointer_location();
    crate::backend::schedule_pointer(state, seat);
}
