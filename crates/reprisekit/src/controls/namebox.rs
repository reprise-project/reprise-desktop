//! A box asking for one name: a Name field, OK and Cancel. OK is live while the name is not
//! blank; [`NameBox::value`] reads it trimmed.

use crate::controls::form::{self, Control, Form, Item};
use crate::controls::rowsdlg::{EDIT_H, MARGIN, Rows};
use crate::textedit::TextEdit;

pub const CLIENT: (i32, i32) = (320, 79);

const ROWS: Rows = Rows {
    client: CLIENT,
    label_w: 64,
};

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

pub struct NameBox {
    pub form: Form,
}

impl NameBox {
    pub fn new(label: &str) -> NameBox {
        let mut edit = TextEdit::new(label, false);
        edit.select_all();
        let mut items = vec![
            ROWS.label("name_l", MARGIN, "&Name:"),
            Item::new(
                NAME,
                ROWS.field(MARGIN, EDIT_H),
                Control::Edit {
                    edit,
                    first: Default::default(),
                    enabled: true,
                },
            ),
        ];
        items.extend(ROWS.ok_cancel(OK, CANCEL));
        let mut form = Form::new(items);
        form.focus_id(NAME);
        NameBox { form }
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
    use crate::textedit::EditKey;

    #[test]
    fn typing_replaces_the_label_and_a_blank_one_is_refused() {
        let mut d = NameBox::new("&Run...");
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
        let d = NameBox::new("x");
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
