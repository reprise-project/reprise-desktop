//! View > Select Columns...: a check box per Processes column, OK and Cancel.

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{Form, Item};
use reprisekit::raster::Rect;

use crate::columns::{Column, SPECS};

pub const CLIENT: (i32, i32) = (260, 232);
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";

const EDGE: i32 = 10;
const FIRST: i32 = 34;
const STEP: i32 = 20;

pub fn form(shown: &[Column]) -> Form {
    let (w, h) = CLIENT;
    let mut items = vec![Item::label_at(
        "intro",
        EDGE,
        EDGE,
        w - 2 * EDGE,
        "Pick the columns the Processes page shows.",
    )];
    for (i, s) in SPECS.iter().enumerate() {
        let y = FIRST + i as i32 * STEP;
        items.push(
            Item::check(s.key, EDGE + 4, y, s.label)
                .checked(shown.contains(&s.col))
                .enabled(s.col != Column::Name),
        );
    }
    let by = h - BUTTON_H - 8;
    items.push(Item::button(
        OK,
        Rect::new(w - 2 * BUTTON_W - 14, by, BUTTON_W, BUTTON_H),
        &reprisekit::kit_tr!("ok"),
        true,
        true,
    ));
    items.push(Item::button(
        CANCEL,
        Rect::new(w - BUTTON_W - 8, by, BUTTON_W, BUTTON_H),
        &reprisekit::kit_tr!("cancel"),
        true,
        false,
    ));
    Form::new(items)
}

/// The columns ticked in `f`.
pub fn picked(f: &Form) -> Vec<Column> {
    (SPECS.iter())
        .filter(|s| f.checked(s.key) == Some(true))
        .map(|s| s.col)
        .collect()
}
