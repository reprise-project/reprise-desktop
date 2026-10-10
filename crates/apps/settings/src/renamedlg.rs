//! Rename dialog for a Start Menu row label.

use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

pub const CLIENT: (i32, i32) = (320, 79);

const MARGIN: i32 = 12;
const LABEL_W: i32 = 64;
const FIELD_X: i32 = MARGIN + LABEL_W + 4;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const OK_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
const NAME: &str = "name";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Accept(String),
    Cancel,
}

pub struct RenameDialog {
    pub form: Form,
}

impl RenameDialog {
    pub fn new(label: &str) -> RenameDialog {
        let mut edit = TextEdit::new(label, false);
        edit.select_all();
        let items = vec![
            Item::label(
                "name_l",
                Rect::new(MARGIN, MARGIN + 4, LABEL_W, 13),
                "&Name:",
            ),
            Item::new(
                NAME,
                Rect::new(FIELD_X, MARGIN, FIELD_W, 20),
                Control::Edit {
                    edit,
                    first: Default::default(),
                    enabled: true,
                },
            ),
            Item::button(
                OK,
                Rect::new(OK_X, BTN_Y, BUTTON_W, BUTTON_H),
                "OK",
                true,
                true,
            ),
            Item::button(
                CANCEL,
                Rect::new(CANCEL_X, BTN_Y, BUTTON_W, BUTTON_H),
                "Cancel",
                true,
                false,
            ),
        ];
        let mut form = Form::new(items);
        form.focus_id(NAME);
        RenameDialog { form }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                let valid = self.value().is_some();
                self.form.set_enabled(OK, valid);
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => match self.value() {
                Some(v) => Outcome::Accept(v),
                None => Outcome::None,
            },
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn value(&self) -> Option<String> {
        match self.form.item(NAME).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) if !edit.text.trim().is_empty() => {
                Some(edit.text.trim().to_string())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::textedit::EditKey;

    #[test]
    fn typing_replaces_the_label_and_a_blank_one_is_refused() {
        let mut d = RenameDialog::new("&Run...");
        assert_eq!(d.value().as_deref(), Some("&Run..."));
        let ev = d.form.key(form::Key::Edit(EditKey::Insert("R&un".into())));
        assert_eq!(d.event(ev), Outcome::Redraw);
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Accept("R&un".into())
        );
        let ev = d.form.key(form::Key::Edit(EditKey::SelectAll));
        d.event(ev);
        let ev = d
            .form
            .key(form::Key::Edit(EditKey::Backspace { word: false }));
        d.event(ev);
        assert_eq!(d.event(form::Event::Pressed(OK)), Outcome::None);
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn everything_sits_inside_the_client() {
        let d = RenameDialog::new("x");
        for i in &d.form.items {
            let r = i.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1,
                "{} at {r:?}",
                i.id
            );
        }
    }
}
