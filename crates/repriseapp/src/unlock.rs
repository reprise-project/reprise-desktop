//! Unlock Drive: the passphrase box for a locked drive, raised when it is
//! plugged in and when it is opened from Computer.

use std::cell::Cell;

use reprisekit::access::Tree;
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;
use zeroize::Zeroizing;

use crate::keys::form_input;
use crate::{Ctx, Event};

pub const CLIENT: (i32, i32) = (378, 150);

const ICON: Rect = Rect::new(10, 12, 32, 32);
const MSG_X: i32 = 52;
const MSG_Y: i32 = 10;
pub const MSG_W: i32 = CLIENT.0 - MSG_X - 16;
const LABEL_X: i32 = 29;
const FIELD_X: i32 = 134;
const FIELD_Y: i32 = 60;
const FIELD_W: i32 = 207;
const FIELD_H: i32 = 20;
const LABEL_DY: i32 = 3;
const STATUS_Y: i32 = 89;
const BTN_Y: i32 = CLIENT.1 - 12 - BUTTON_H;
const CANCEL_X: i32 = CLIENT.0 - 12 - BUTTON_W;
const OK_X: i32 = CANCEL_X - 6 - BUTTON_W;

pub const TITLE: &str = "Unlock Drive";
pub const WRONG: &str = "The passphrase is not correct.";
const ICON_NAME: &str = "drive-removable-media";

pub enum Outcome {
    None,
    Redraw,
    Submit(Zeroizing<String>),
    Cancel,
}

/// The dialog's model: the form and the container it unlocks. Dropping it wipes the field.
pub struct Unlock {
    pub volume: String,
    pub form: Form,
    busy: bool,
}

pub fn message(name: &str) -> String {
    format!("'{name}' is encrypted. Type its passphrase to unlock it.")
}

/// The status line for a failed unlock with UDisks2 `message`.
pub fn failure(message: &str) -> &str {
    // Wrong-passphrase wording varies by cryptsetup version.
    let m = message.to_lowercase();
    let wrong = ["passphrase", "not permitted", "no key"]
        .iter()
        .any(|w| m.contains(w));
    if wrong { WRONG } else { message }
}

impl Unlock {
    /// The box for container `volume`, named `name` in its message.
    pub fn open(volume: String, name: &str, ctx: &mut Ctx) -> Unlock {
        let forms = ctx.forms();
        let width = forms.text.len(MSG_W);
        let lines = reprisekit::text::wrap(&ctx.text, &message(name), width, forms.fonts.size_px);
        Unlock::new(volume, &lines, ctx.icons.get(ICON_NAME, 32))
    }

    pub fn new(volume: String, lines: &[String], icon: Option<Canvas>) -> Unlock {
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let items = vec![
            Item::picture("icon", ICON, icon),
            Item::lines(
                "message",
                Rect::new(MSG_X, MSG_Y, MSG_W, LINE_H * refs.len().min(3) as i32),
                &refs,
            ),
            Item::label(
                "passphrase_l",
                Rect::new(LABEL_X, FIELD_Y + LABEL_DY, 100, LINE_H),
                "&Passphrase:",
            ),
            Item::new(
                "passphrase",
                Rect::new(FIELD_X, FIELD_Y, FIELD_W, FIELD_H),
                Control::Edit {
                    edit: TextEdit::secret(),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
            Item::label(
                "status",
                Rect::new(LABEL_X, STATUS_Y, CLIENT.0 - LABEL_X - 16, LINE_H),
                "",
            ),
            Item::button(
                "ok",
                Rect::new(OK_X, BTN_Y, BUTTON_W, BUTTON_H),
                "OK",
                false,
                true,
            ),
            Item::button(
                "cancel",
                Rect::new(CANCEL_X, BTN_Y, BUTTON_W, BUTTON_H),
                "Cancel",
                true,
                false,
            ),
        ];
        let mut form = Form::new(items);
        form.focus_id("passphrase");
        Unlock {
            volume,
            form,
            busy: false,
        }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                self.sync_ok();
                Outcome::Redraw
            }
            form::Event::Pressed("ok") | form::Event::Default => {
                if !self.can_submit() {
                    return Outcome::None;
                }
                let pass = self.take();
                self.busy = true;
                self.form.set_text("status", "");
                self.sync_ok();
                Outcome::Submit(pass)
            }
            form::Event::Pressed("cancel") | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// An event on the box's window; the close box is Cancel.
    pub fn input(&mut self, ev: Event, ctx: &Ctx) -> Outcome {
        match ev {
            Event::CloseRequest => Outcome::Cancel,
            Event::Pasted(text) => {
                let ev = self.form.paste(text);
                self.event(ev)
            }
            ev => match form_input(&mut self.form, &ev, ctx) {
                Some(ev) => self.event(ev),
                None => Outcome::None,
            },
        }
    }

    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx) {
        let forms = ctx.forms();
        c.fill_rect(c.bounds(), forms.colors.button_face);
        let paint = Paint {
            s: 1,
            ..forms.paint()
        };
        form::draw(c, &ctx.text, &paint, &self.form, true, forms.fonts.size_px);
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let px = self.form.units().px(ctx.theme.fonts.size_px);
        crate::oneshot::access_form(&self.form, tree, &ctx.text, px);
    }

    /// The box's client size at the text size.
    pub fn client(ctx: &Ctx) -> (i32, i32) {
        ctx.forms().text.size(CLIENT)
    }

    pub fn fail(&mut self, why: &str) {
        self.busy = false;
        self.form.set_text("status", why);
        self.form.focus_id("passphrase");
        self.sync_ok();
    }

    #[cfg(test)]
    pub fn status(&self) -> &str {
        match self.form.item("status").map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text,
            _ => "",
        }
    }

    fn passphrase(&self) -> &str {
        match self.form.item("passphrase").map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => &edit.text,
            _ => "",
        }
    }

    fn can_submit(&self) -> bool {
        !self.busy && !self.passphrase().is_empty()
    }

    fn take(&mut self) -> Zeroizing<String> {
        let out = Zeroizing::new(self.passphrase().to_string());
        self.wipe();
        out
    }

    fn wipe(&mut self) {
        if let Some(Control::Edit { edit, .. }) =
            self.form.item_mut("passphrase").map(|i| &mut i.control)
        {
            edit.wipe();
        }
    }

    fn sync_ok(&mut self) {
        let on = self.can_submit();
        let focused = self.form.focus() == self.form.index_of("ok");
        self.form.set_enabled("ok", on);
        if focused && !on {
            self.form.focus_id("passphrase");
        }
    }
}

impl Drop for Unlock {
    fn drop(&mut self) {
        self.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(d: &mut Unlock, s: &str) {
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            d.event(ev);
        }
    }

    #[test]
    fn ok_waits_for_a_passphrase_and_hands_it_over_once() {
        let mut d = Unlock::new("sdc".into(), &[message("VAULT")], None);
        assert!(matches!(d.event(form::Event::Default), Outcome::None));
        typed(&mut d, "hunter2");
        match d.event(form::Event::Default) {
            Outcome::Submit(p) => assert_eq!(p.as_str(), "hunter2"),
            _ => panic!("OK with a passphrase submits it"),
        }
        assert_eq!(d.passphrase(), "", "the field is wiped on submit");
        typed(&mut d, "again");
        assert!(
            matches!(d.event(form::Event::Default), Outcome::None),
            "no second try while the first is out"
        );
    }

    #[test]
    fn a_wrong_passphrase_says_so_and_takes_another() {
        let mut d = Unlock::new("sdc".into(), &[message("VAULT")], None);
        typed(&mut d, "nope");
        d.event(form::Event::Default);
        d.fail(WRONG);
        assert_eq!(d.status(), "The passphrase is not correct.");
        typed(&mut d, "right");
        assert!(matches!(d.event(form::Event::Default), Outcome::Submit(_)));
    }

    #[test]
    fn a_refused_passphrase_reads_as_wrong_and_other_failures_as_given() {
        assert_eq!(
            failure("Error unlocking /dev/sdc: Failed to activate device: Operation not permitted"),
            WRONG
        );
        assert_eq!(
            failure("Device /dev/sdc is busy"),
            "Device /dev/sdc is busy"
        );
    }

    #[test]
    fn the_message_names_the_drive() {
        assert_eq!(
            message("VAULT"),
            "'VAULT' is encrypted. Type its passphrase to unlock it."
        );
    }
}
