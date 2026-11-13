//! The Network page: airplane mode, which acts at once on the radios rather than through Apply.

use super::{Area, X};
use reprisekit::controls::form::{Form, Item};

pub const AIRPLANE: &str = "airplane";
const AIRPLANE_NOTE: &str = "airplane_note";
const AIRPLANE_STATUS: &str = "airplane_status";

pub const NO_RADIO: &str = "This computer has no radio to turn off.";

/// The page before the radios are read: the check box greyed until [`show_airplane`].
pub fn network(a: Area) -> Form {
    let mut page = Form::new(vec![
        Item::check(AIRPLANE, X, 44, "&Airplane mode"),
        Item::label_at(
            AIRPLANE_NOTE,
            X,
            64,
            a.w - 18,
            "Turns off Wi-Fi, mobile broadband and Bluetooth.",
        )
        .under_check(),
        Item::label_at(AIRPLANE_STATUS, X, 80, a.w - 18, "").under_check(),
    ]);
    page.set_enabled(AIRPLANE, false);
    page
}

/// Show airplane mode as `state` has it (None: no radio switch), the check box greyed while
/// `busy` carries out a change.
pub fn show_airplane(page: &mut Form, state: Option<bool>, busy: bool) {
    page.set_check(AIRPLANE, state == Some(true));
    page.set_enabled(AIRPLANE, state.is_some() && !busy);
    page.set_enabled(AIRPLANE_NOTE, state.is_some());
    page.set_text(AIRPLANE_STATUS, if state.is_none() { NO_RADIO } else { "" });
}
