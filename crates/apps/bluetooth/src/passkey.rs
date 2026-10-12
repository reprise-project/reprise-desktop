//! A pairing prompt's content per request kind, as form items, and the answer read back. Used by
//! the Add wizard's pairing page and the Pairing Request box.

use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Form, Item};
use reprisekit::raster::Rect;
use reprisekit::text::{TextRenderer, wrap};
use reprisekit::theme::Theme;

use crate::bz::{Answer, Ask};
use crate::props::edit_text;

pub const CODE: &str = "code";
pub const YES: &str = "yes";
pub const NO: &str = "no";
const GROUP: &str = "answer";
const EDIT_H: i32 = 21;
const PIN_MAX: usize = 16;
const PASSKEY_DIGITS: usize = 6;

pub fn sentence(ask: &Ask, name: &str) -> String {
    match ask {
        Ask::PinCode => format!(
            "Type the PIN for {name}. If you do not know it, check the device's manual; many devices use 0000."
        ),
        Ask::Passkey => format!("Type the six-digit passkey that {name} shows."),
        Ask::ShowPin(_) | Ask::ShowPasskey(_) => {
            format!("Type this passkey on {name}, then press its Enter key:")
        }
        Ask::Confirm(_) => format!("Check that {name} shows this passkey:"),
        Ask::Authorize => format!("{name} wants to pair with this computer."),
    }
}

pub fn shown_code(ask: &Ask) -> Option<String> {
    match ask {
        Ask::ShowPin(p) => Some(p.clone()),
        Ask::ShowPasskey(n) | Ask::Confirm(n) => Some(format!("{n:06}")),
        _ => None,
    }
}

fn radio(id: &'static str, x: i32, y: i32, label: &str, checked: bool) -> Item {
    Item::radio(id, x, y, label, GROUP).checked(checked)
}

pub fn items(ask: &Ask, name: &str, body: Rect, text: &TextRenderer, forms: &Theme) -> Vec<Item> {
    let wrap_w = forms.text.len(body.width);
    let lines = wrap(text, &sentence(ask, name), wrap_w, forms.fonts.size_px);
    let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
    let mut out = vec![Item::lines_at(
        "sentence", body.x, body.y, body.width, &refs,
    )];
    let mut y = body.y + LINE_H * lines.len() as i32 + 12;
    if let Some(code) = shown_code(ask) {
        out.push(Item::label_at(
            "shown",
            body.x + 16,
            y,
            body.width - 16,
            &code,
        ));
        y += LINE_H + 12;
    }
    match ask {
        Ask::PinCode | Ask::Passkey => {
            let label = if *ask == Ask::PinCode {
                "&PIN:"
            } else {
                "&Passkey:"
            };
            out.push(Item::label_at("code_label", body.x, y + 4, 60, label));
            out.push(Item::edit(
                CODE,
                Rect::new(body.x + 64, y, 120, EDIT_H),
                "",
                true,
            ));
        }
        Ask::Confirm(_) => {
            out.push(radio(YES, body.x, y, "&Yes, the passkeys match", true));
            out.push(radio(NO, body.x, y + 20, "No, they are &different", false));
        }
        Ask::Authorize => {
            out.push(radio(YES, body.x, y, "&Allow it to pair", true));
            out.push(radio(NO, body.x, y + 20, "&Do not allow it", false));
        }
        Ask::ShowPin(_) | Ask::ShowPasskey(_) => {}
    }
    out
}

fn pin(s: &str) -> Option<String> {
    let n = s.chars().count();
    ((1..=PIN_MAX).contains(&n) && !s.chars().any(char::is_control)).then(|| s.to_string())
}

fn passkey(s: &str) -> Option<u32> {
    ((1..=PASSKEY_DIGITS).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

pub fn ready(ask: &Ask, form: &Form) -> bool {
    answer(ask, form).is_some()
}

pub fn answer(ask: &Ask, form: &Form) -> Option<Answer> {
    match ask {
        Ask::PinCode => pin(&edit_text(form, CODE)).map(Answer::Pin),
        Ask::Passkey => passkey(&edit_text(form, CODE)).map(Answer::Passkey),
        Ask::Confirm(_) | Ask::Authorize => Some(if form.checked(YES) == Some(true) {
            Answer::Accept
        } else {
            Answer::Reject
        }),
        Ask::ShowPin(_) | Ask::ShowPasskey(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::props::set_edit;
    use reprisekit::controls::form::{Control, Form};
    use reprisekit::text::TextRenderer;
    use reprisekit::theme::{FontConfig, Theme};

    /// `Form::set_check` only writes `Control::Check`; tests set a radio's `checked` field
    /// directly.
    fn pick_radio(f: &mut Form, id: &str, on: bool) {
        if let Some(Control::Radio { checked, .. }) = f.item_mut(id).map(|i| &mut i.control) {
            *checked = on;
        }
    }

    fn form(ask: &Ask) -> Form {
        let text = TextRenderer::load(&FontConfig::default());
        Form::new(items(
            ask,
            "Pad",
            Rect::new(10, 10, 300, 150),
            &text,
            &Theme::default(),
        ))
    }

    #[test]
    fn a_passkey_is_one_to_six_digits() {
        let mut f = form(&Ask::Passkey);
        assert!(!ready(&Ask::Passkey, &f), "empty");
        set_edit(&mut f, CODE, "12a");
        assert!(!ready(&Ask::Passkey, &f));
        set_edit(&mut f, CODE, "001234");
        assert_eq!(answer(&Ask::Passkey, &f), Some(Answer::Passkey(1234)));
        set_edit(&mut f, CODE, "1234567");
        assert!(!ready(&Ask::Passkey, &f), "seven digits");
    }

    #[test]
    fn a_pin_is_one_to_sixteen_characters() {
        let mut f = form(&Ask::PinCode);
        set_edit(&mut f, CODE, "0000");
        assert_eq!(answer(&Ask::PinCode, &f), Some(Answer::Pin("0000".into())));
        set_edit(&mut f, CODE, &"9".repeat(17));
        assert!(!ready(&Ask::PinCode, &f));
    }

    #[test]
    fn confirm_starts_on_yes_and_no_rejects() {
        let ask = Ask::Confirm(42);
        let mut f = form(&ask);
        assert_eq!(shown_code(&ask).as_deref(), Some("000042"));
        assert_eq!(answer(&ask, &f), Some(Answer::Accept));
        pick_radio(&mut f, NO, true);
        pick_radio(&mut f, YES, false);
        assert_eq!(answer(&ask, &f), Some(Answer::Reject));
    }

    #[test]
    fn a_shown_passkey_needs_no_answer() {
        let ask = Ask::ShowPasskey(123456);
        let f = form(&ask);
        assert!(!ready(&ask, &f));
        assert_eq!(answer(&ask, &f), None);
        assert_eq!(
            sentence(&ask, "Pad"),
            "Type this passkey on Pad, then press its Enter key:"
        );
    }
}
