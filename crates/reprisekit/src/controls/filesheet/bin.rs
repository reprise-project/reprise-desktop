use super::{
    HIDDEN, ICON_PX, LABEL_X, NAME, READ_ONLY, SEP_IDS, SEP_RIGHT, button, check_enabled, sep, tick,
};
use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::escape_mnemonic;

/// What the General page of an item in the Recycle Bin says, already
/// worded. Nothing on it can be changed: the item is renamed or edited
/// only once Restore has put it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinFacts {
    pub type_name: String,
    pub origin: String,
    pub size: String,
    pub deleted: Option<String>,
    pub created: Option<String>,
    pub read_only: bool,
    pub hidden: bool,
}

pub const RESTORE: &str = "restore";
pub const BIN_SHEET_H: i32 = 418;
const BIN_ICON_Y: i32 = 40;
const BIN_NAME: (i32, i32) = (63, 47);
const BIN_VALUE_X: i32 = 85;
const BIN_DATE_X: i32 = 115;
const BIN_SEPS: [i32; 3] = [81, 176, 244];
const BIN_BLOCKS: [i32; 3] = [94, 189, 257];
/// The type, origin and date rows hold a second line for a long value.
const BIN_WRAP_PITCH: i32 = 2 * LINE_H;
const BIN_CHECK_X: [i32; 2] = [112, 204];
const BIN_CHECK_Y: [i32; 4] = [258, 277, 297, 316];
const BIN_RESTORE: Rect = Rect::new(265, 344, BUTTON_W, BUTTON_H);

/// Read-only and Hidden show the item's own state. Archive, Compressed
/// and Encrypt contents stand for attributes nothing maps to here, so
/// they are drawn unchecked.
pub fn bin_page(facts: &BinFacts, name: &str, icon: Option<Canvas>) -> Form {
    let mut items = vec![
        Item::picture(
            "icon",
            Rect::new(LABEL_X, BIN_ICON_Y, ICON_PX, ICON_PX),
            icon,
        ),
        Item::label_at(
            NAME,
            BIN_NAME.0,
            BIN_NAME.1,
            SEP_RIGHT - BIN_NAME.0,
            &escape_mnemonic(name),
        ),
    ];
    for (id, y) in SEP_IDS.iter().zip(BIN_SEPS) {
        items.push(sep(id, y, SEP_RIGHT));
    }
    // A row that wraps holds two lines; Size keeps one.
    let mut row = |(l, v): (&'static str, &'static str), label, value: &str, x, y, wraps| {
        items.push(Item::label_at(l, LABEL_X, y, x - LABEL_X, label));
        let w = SEP_RIGHT - x;
        if wraps {
            items.push(Item::lines(v, Rect::new(x, y, w, BIN_WRAP_PITCH), &[value]));
            y + BIN_WRAP_PITCH
        } else {
            items.push(Item::label_at(v, x, y, w, &escape_mnemonic(value)));
            y + LINE_H
        }
    };
    let [first, second, attrs] = BIN_BLOCKS;
    let types = ("l_type", "v_type");
    let y = row(types, "Type:", &facts.type_name, BIN_VALUE_X, first, true);
    let origins = ("l_origin", "v_origin");
    let y = row(origins, "Origin:", &facts.origin, BIN_VALUE_X, y, true);
    row(
        ("l_size", "v_size"),
        "Size:",
        &facts.size,
        BIN_VALUE_X,
        y,
        false,
    );
    let dates = [
        (("l_deleted", "v_deleted"), "Deleted:", &facts.deleted),
        (("l_made", "v_made"), "Created:", &facts.created),
    ];
    let mut y = second;
    for (ids, label, value) in dates {
        if let Some(value) = value {
            y = row(ids, label, value, BIN_DATE_X, y, true);
        }
    }
    items.push(Item::label_at(
        "l_attr",
        LABEL_X,
        attrs,
        BIN_CHECK_X[0] - LABEL_X,
        "Attributes",
    ));
    let [left, right] = BIN_CHECK_X;
    for (id, x, y, label, checked) in [
        (
            READ_ONLY,
            left,
            BIN_CHECK_Y[0],
            "&Read-only",
            facts.read_only,
        ),
        (HIDDEN, right, BIN_CHECK_Y[0], "&Hidden", facts.hidden),
        ("archive", left, BIN_CHECK_Y[1], "Archi&ve", false),
        ("compress", left, BIN_CHECK_Y[2], "Co&mpressed", false),
        ("encrypt", left, BIN_CHECK_Y[3], "&Encrypt contents", false),
    ] {
        items.push(check_enabled(id, x, y, label, tick(checked), false));
    }
    items.push(button(RESTORE, BIN_RESTORE, "Restore", true));
    Form::new(items)
}
