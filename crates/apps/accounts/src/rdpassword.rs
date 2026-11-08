//! The dialogs that create, change and remove your Remote Desktop password. Create and Change
//! ask for your login password and the new password twice; Remove asks for the login password
//! alone.

use std::cell::Cell;
use std::fmt;

use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::remotepassword::Action;
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;
use repriselogin::account::{AccountRefusal, AccountRequest, MIN_REMOTE_PASSWORD};
use zeroize::Zeroizing;

const W: i32 = 360;
const MARGIN: i32 = 12;
const LABEL_W: i32 = 170;
const FIELD_X: i32 = MARGIN + LABEL_W + 4;
const FIELD_W: i32 = W - MARGIN - FIELD_X;
const ROW_H: i32 = 28;
/// A 13 px label sits 4 px into its 20 px field.
const LABEL_DY: i32 = 4;
const CANCEL_X: i32 = W - MARGIN - BUTTON_W;
const OK_X: i32 = CANCEL_X - BUTTON_GAP - BUTTON_W;

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
pub const LOGIN: &str = "login_password";
pub const NEW: &str = "remote_password";
pub const CONFIRM: &str = "confirm";
pub const WHY: &str = "why";

pub enum Outcome {
    None,
    Redraw,
    /// OK: the request to send; the dialog waits for its reply ([`RdPasswordDialog::refused`]).
    Send(AccountRequest),
    Cancel,
}

pub struct RdPasswordDialog {
    pub form: Form,
    pub action: Action,
    /// Tells this dialog's replies from those of a dialog closed before it.
    pub generation: u64,
    /// A request is out: OK stays greyed until its reply.
    waiting: bool,
}

impl fmt::Debug for RdPasswordDialog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RdPasswordDialog")
            .field("action", &self.action)
            .field("waiting", &self.waiting)
            .field("why", &self.why())
            .finish_non_exhaustive()
    }
}

/// What the dialog says for a refusal.
pub fn refusal_text(r: AccountRefusal) -> &'static str {
    match r {
        AccountRefusal::WrongPassword => "The password is incorrect.",
        AccountRefusal::TooShort => "Use 8 or more characters.",
        AccountRefusal::SameAsLogin => "Use a password other than your login password.",
        AccountRefusal::NotRemoteUser => "Your account is not allowed to log on remotely.",
        AccountRefusal::Busy | AccountRefusal::Failed => "The password could not be changed.",
    }
}

/// A field's label id, the field's id and the label's text.
const FIELDS: [(&str, &str, &str); 3] = [
    ("login_label", LOGIN, "&Your login password:"),
    ("remote_label", NEW, "&New Remote Desktop password:"),
    ("confirm_label", CONFIRM, "&Confirm:"),
];

fn secret((label_id, id, label): (&'static str, &'static str, &str), row: i32) -> [Item; 2] {
    let y = MARGIN + row * ROW_H;
    [
        Item::label_at(label_id, MARGIN, y + LABEL_DY, LABEL_W, label),
        Item::new(
            id,
            Rect::new(FIELD_X, y, FIELD_W, 20),
            Control::Edit {
                edit: TextEdit::secret(),
                first: Cell::new(0),
                enabled: true,
            },
        ),
    ]
}

impl RdPasswordDialog {
    pub fn new(action: Action, generation: u64) -> RdPasswordDialog {
        let fields = match action {
            Action::Create | Action::Change => &FIELDS[..],
            Action::Remove => &FIELDS[..1],
        };
        let mut items: Vec<Item> = (0..)
            .zip(fields)
            .flat_map(|(row, f)| secret(*f, row))
            .collect();
        let rows = fields.len() as i32;
        let why_y = MARGIN + rows * ROW_H;
        let btn_y = why_y + LINE_H + MARGIN;
        items.extend([
            Item::label_at(WHY, MARGIN, why_y, W - 2 * MARGIN, ""),
            Item::button(
                OK,
                Rect::new(OK_X, btn_y, BUTTON_W, BUTTON_H),
                "OK",
                false,
                true,
            ),
            Item::button(
                CANCEL,
                Rect::new(CANCEL_X, btn_y, BUTTON_W, BUTTON_H),
                "Cancel",
                true,
                false,
            ),
        ]);
        let mut form = Form::new(items);
        form.focus_id(LOGIN);
        RdPasswordDialog {
            form,
            action,
            generation,
            waiting: false,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.action {
            Action::Create => "Create Remote Desktop Password",
            Action::Change => "Change Remote Desktop Password",
            Action::Remove => "Remove Remote Desktop Password",
        }
    }

    pub fn client(&self) -> (i32, i32) {
        let ok = self.form.item(OK).map_or(0, |i| i.rect.bottom());
        (W, ok + MARGIN)
    }

    /// A key or a click the host has already given the form, as what it should do about it.
    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(_) => {
                self.follow();
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => match self.request() {
                Some(req) => {
                    self.waiting = true;
                    self.form.set_text(WHY, "");
                    self.follow();
                    Outcome::Send(req)
                }
                None => Outcome::None,
            },
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// The daemon refused the request: say why, and empty the fields it was about.
    pub fn refused(&mut self, r: AccountRefusal) {
        self.waiting = false;
        self.form.set_text(WHY, refusal_text(r));
        let wrong: &[&str] = match r {
            AccountRefusal::WrongPassword => &[LOGIN],
            AccountRefusal::TooShort | AccountRefusal::SameAsLogin => &[NEW, CONFIRM],
            _ => &[],
        };
        for id in wrong {
            if let Some(Control::Edit { edit, .. }) = self.form.item_mut(id).map(|i| &mut i.control)
            {
                edit.wipe();
            }
        }
        if let Some(id) = wrong.first() {
            self.form.focus_id(id);
        }
        self.follow();
    }

    pub fn why(&self) -> &str {
        match self.form.item(WHY).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text,
            _ => "",
        }
    }

    /// OK is lit while no request is out, the login password is typed and, to set, the new
    /// password is long enough and typed the same twice.
    fn follow(&mut self) {
        let ready = !self.waiting && self.complete();
        self.form.set_enabled(OK, ready);
    }

    fn complete(&self) -> bool {
        let text = |id| self.form.edit_text(id);
        if text(LOGIN).is_empty() {
            return false;
        }
        match self.action {
            Action::Remove => true,
            Action::Create | Action::Change => {
                text(NEW).chars().count() >= MIN_REMOTE_PASSWORD && text(NEW) == text(CONFIRM)
            }
        }
    }

    fn request(&self) -> Option<AccountRequest> {
        if self.waiting || !self.complete() {
            return None;
        }
        let secret = |id| Zeroizing::new(self.form.edit_text(id).to_owned());
        Some(match self.action {
            Action::Create | Action::Change => AccountRequest::SetRemotePassword {
                login_password: secret(LOGIN),
                remote_password: secret(NEW),
            },
            Action::Remove => AccountRequest::ClearRemotePassword {
                login_password: secret(LOGIN),
            },
        })
    }
}

#[cfg(test)]
pub(crate) fn type_into(d: &mut RdPasswordDialog, id: &'static str, text: &str) -> Outcome {
    use reprisekit::textedit::EditKey;
    d.form.focus_id(id);
    let ev = d.form.key(form::Key::Edit(EditKey::Insert(text.into())));
    d.event(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_lit(d: &RdPasswordDialog) -> bool {
        matches!(
            d.form.item(OK).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    #[test]
    fn the_dialog_needs_matching_new_passwords_of_8_or_more() {
        let mut d = RdPasswordDialog::new(Action::Create, 1);
        assert!(!ok_lit(&d), "greyed when empty");
        type_into(&mut d, LOGIN, "hunter2");
        type_into(&mut d, NEW, "sword12");
        type_into(&mut d, CONFIRM, "sword12");
        assert!(!ok_lit(&d), "7 characters");
        assert!(matches!(d.event(form::Event::Default), Outcome::None));
        type_into(&mut d, NEW, "é");
        assert!(!ok_lit(&d), "the two differ");
        type_into(&mut d, CONFIRM, "é");
        assert!(ok_lit(&d), "8 characters, the same twice");
        match d.event(form::Event::Pressed(OK)) {
            Outcome::Send(req) => assert_eq!(
                req,
                AccountRequest::SetRemotePassword {
                    login_password: Zeroizing::new("hunter2".into()),
                    remote_password: Zeroizing::new("sword12é".into()),
                }
            ),
            _ => panic!("OK sends the request"),
        }
        assert!(!ok_lit(&d), "greyed while the request is out");
        assert!(matches!(d.event(form::Event::Default), Outcome::None));
        d.refused(AccountRefusal::SameAsLogin);
        assert_eq!(d.why(), "Use a password other than your login password.");
        assert_eq!(d.form.edit_text(NEW), "");
        assert_eq!(d.form.edit_text(CONFIRM), "");
        assert_eq!(
            d.form.edit_text(LOGIN),
            "hunter2",
            "the login password stays"
        );
        assert!(!ok_lit(&d));
    }

    #[test]
    fn clear_asks_only_for_the_login_password() {
        let mut d = RdPasswordDialog::new(Action::Remove, 1);
        assert!(d.form.item(NEW).is_none() && d.form.item(CONFIRM).is_none());
        assert!(!ok_lit(&d));
        type_into(&mut d, LOGIN, "hunter2");
        assert!(ok_lit(&d));
        match d.event(form::Event::Default) {
            Outcome::Send(req) => assert_eq!(
                req,
                AccountRequest::ClearRemotePassword {
                    login_password: Zeroizing::new("hunter2".into()),
                }
            ),
            _ => panic!("Enter sends the request"),
        }
        assert!(matches!(d.event(form::Event::Cancel), Outcome::Cancel));
    }

    #[test]
    fn debug_of_the_dialog_never_shows_the_passwords() {
        let mut d = RdPasswordDialog::new(Action::Create, 1);
        type_into(&mut d, LOGIN, "hunter2");
        type_into(&mut d, NEW, "swordfish");
        type_into(&mut d, CONFIRM, "swordfish");
        let Outcome::Send(req) = d.event(form::Event::Default) else {
            panic!("OK sends the request");
        };
        for shown in [
            format!("{d:?}"),
            format!("{req:?}"),
            format!("{:?}", d.form),
        ] {
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("swordfish"), "{shown}");
        }
        assert_eq!(
            format!("{d:?}"),
            "RdPasswordDialog { action: Create, waiting: true, why: \"\", .. }"
        );
    }

    #[test]
    fn everything_sits_inside_the_client() {
        for p in [Action::Create, Action::Change, Action::Remove] {
            let d = RdPasswordDialog::new(p, 1);
            let (w, h) = d.client();
            for i in &d.form.items {
                let r = i.rect;
                assert!(
                    r.x >= 0 && r.y >= 0 && r.right() <= w && r.bottom() <= h,
                    "{p:?}: {} at {r:?}",
                    i.id
                );
            }
        }
        assert_eq!(
            RdPasswordDialog::new(Action::Create, 1).client(),
            (360, 156)
        );
        assert_eq!(
            RdPasswordDialog::new(Action::Change, 1).client(),
            (360, 156)
        );
        assert_eq!(
            RdPasswordDialog::new(Action::Remove, 1).client(),
            (360, 100)
        );
    }

    #[test]
    fn the_dialogs_fit_their_text_under_every_interface() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            [Action::Create, Action::Change, Action::Remove]
                .into_iter()
                .map(|a| {
                    let mut d = RdPasswordDialog::new(a, 1);
                    d.refused(AccountRefusal::SameAsLogin);
                    (d.title().into(), d.form)
                })
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// A hash of `d` drawn in each of Classic, Blue and Glass, with the test fonts.
    fn pins(d: &RdPasswordDialog) -> [u64; 3] {
        use reprisekit::interface::Interface;
        use reprisekit::raster::Canvas;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::Theme;
        reprisekit::pins::fonts();
        let mut got = [0; 3];
        for (k, i) in [Interface::Classic, Interface::Blue, Interface::Glass]
            .into_iter()
            .enumerate()
        {
            let theme = Theme {
                access_keys: true,
                ..reprisekit::interfaces::theme_for(i)
            };
            let text = TextRenderer::load(&theme.fonts);
            let (w, h) = d.client();
            let mut c = Canvas::new(w as u32, h as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            form::draw(
                &mut c,
                &text,
                &theme.paint(),
                &d.form,
                false,
                theme.fonts.size_px,
            );
            got[k] = c.pixels.iter().fold(0xcbf29ce484222325, |h, p| {
                (h ^ *p as u64).wrapping_mul(0x100000001b3)
            });
        }
        got
    }

    /// Create then Remove: empty, typed into, then refused.
    #[test]
    fn the_dialogs_pin_their_pixels_in_every_look() {
        let mut got = Vec::new();
        for a in [Action::Create, Action::Remove] {
            let mut d = RdPasswordDialog::new(a, 1);
            got.push(pins(&d));
            for id in [LOGIN, NEW, CONFIRM] {
                if d.form.item(id).is_some() {
                    type_into(&mut d, id, "swordfish");
                }
            }
            got.push(pins(&d));
            d.refused(AccountRefusal::NotRemoteUser);
            got.push(pins(&d));
        }
        assert_eq!(
            got,
            [
                [0x2257e143d4ae3064, 0x62ee661248c965f0, 0x053345cacc82ef33],
                [0xfbfd5ca4ae78819f, 0x9189bdc2eb60b61d, 0x9c99f9185f026ec6],
                [0x13e6874b649e692f, 0xb6d0e8a9e682b0d9, 0x1669cff72d11d12a],
                [0xbabbbdfc0331d2cc, 0x8a87746c7e76c83b, 0xd311d41a3e1af5f0],
                [0x23a60d53daf65d43, 0xc34d4f182551418e, 0x969db73445924479],
                [0x90839a3197dc33b3, 0x7fad7217f0b61d3a, 0xf9b3cd0d18d1666d],
            ],
            "Create then Remove: empty, typed, refused; Classic, Blue, Glass"
        );
    }
}
