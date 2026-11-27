//! A dialog of labelled rows: labels in a column at the left margin, each field running to the
//! right margin, OK and Cancel at the bottom right.

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use crate::controls::form::Item;
use crate::raster::Rect;

pub const MARGIN: i32 = 12;
pub const EDIT_H: i32 = 20;
pub const COMBO_H: i32 = 21;
/// A 13 px label sits 4 px into its field.
const LABEL_DY: i32 = 4;
const LABEL_H: i32 = 13;

/// The rows of a `client`-sized dialog whose labels are `label_w` wide.
#[derive(Clone, Copy, Debug)]
pub struct Rows {
    pub client: (i32, i32),
    pub label_w: i32,
}

impl Rows {
    pub const fn field_x(self) -> i32 {
        MARGIN + self.label_w + 4
    }

    pub const fn field_w(self) -> i32 {
        self.client.0 - MARGIN - self.field_x()
    }

    /// The width inside both margins.
    pub const fn inner_w(self) -> i32 {
        self.client.0 - 2 * MARGIN
    }

    pub const fn button_y(self) -> i32 {
        self.client.1 - MARGIN - BUTTON_H
    }

    pub const fn cancel_x(self) -> i32 {
        self.client.0 - MARGIN - BUTTON_W
    }

    pub const fn ok_x(self) -> i32 {
        self.cancel_x() - BUTTON_GAP - BUTTON_W
    }

    /// The label of the row whose field's top is `y`.
    pub fn label(self, id: &'static str, y: i32, text: &str) -> Item {
        Item::label(
            id,
            Rect::new(MARGIN, y + LABEL_DY, self.label_w, LABEL_H),
            text,
        )
    }

    /// The field of the row whose top is `y`, `h` tall.
    pub fn field(self, y: i32, h: i32) -> Rect {
        Rect::new(self.field_x(), y, self.field_w(), h)
    }

    pub fn edit(self, id: &'static str, y: i32, text: &str, enabled: bool) -> Item {
        Item::edit(id, self.field(y, EDIT_H), text, enabled)
    }

    /// A push button with its top left at (`x`, `y`).
    pub fn button(id: &'static str, x: i32, y: i32, text: &str, default: bool) -> Item {
        Item::button(id, Rect::new(x, y, BUTTON_W, BUTTON_H), text, true, default)
    }

    /// OK, the default, and Cancel.
    pub fn ok_cancel(self, ok: &'static str, cancel: &'static str) -> [Item; 2] {
        let y = self.button_y();
        [
            Rows::button(ok, self.ok_x(), y, "OK", true),
            Rows::button(cancel, self.cancel_x(), y, &tr!("cancel"), false),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_reach_the_right_margin_and_the_buttons_the_bottom_right_corner() {
        let rows = Rows {
            client: (320, 104),
            label_w: 64,
        };
        assert_eq!(rows.field(12, EDIT_H), Rect::new(80, 12, 228, 20));
        assert_eq!(
            rows.label("l", 12, "&Name:").rect,
            Rect::new(12, 16, 64, 13)
        );
        let [ok, cancel] = rows.ok_cancel("ok", "cancel");
        assert_eq!(cancel.rect, Rect::new(233, 69, BUTTON_W, BUTTON_H));
        assert_eq!(ok.rect.x + ok.rect.width + BUTTON_GAP, cancel.rect.x);
    }
}
