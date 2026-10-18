//! The Run As dialog for polkit: the key icon and polkit's message, then
//! "The following user:" with the identities polkitd offered in the User
//! name combo, the Password field, OK and Cancel. polkit picks who may
//! authenticate, so there is no "Current user" group or browse button.

use std::cell::Cell;

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::combo::{self, Combo, ListItem};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;
use zeroize::Zeroizing;

/// The client area for a message of one or two lines; see `client_size`.
pub const CLIENT: (i32, i32) = (378, 207);

const ICON: Rect = Rect::new(10, 12, 32, 32);
const MSG_X: i32 = 52;
/// The message's cap top shares the icon's top margin (a 13 px label draws
/// its caps 2 px into its rect).
const MSG_Y: i32 = 10;
pub const MSG_W: i32 = CLIENT.0 - MSG_X - 16;
/// Characters that fit `MSG_W`, for the char-wrap the tests use; the host
/// wraps by measured pixels.
#[cfg(test)]
const MSG_COLS: usize = 58;
const FOLLOWING_X: i32 = 20;
/// The "The following user:" caption's row; everything below sits relative to it.
const FOLLOWING_Y: i32 = 62;
const LABEL_X: i32 = 29;
const FIELD_X: i32 = 134;
const COMBO_Y: i32 = 90;
const COMBO_W: i32 = 207;
const EDIT_Y: i32 = 118;
const EDIT_W: i32 = COMBO_W;
const EDIT_H: i32 = 20;
/// A 13 px label centred in its field.
const LABEL_DY: i32 = 3;
const STATUS_Y: i32 = 146;
const BTN_Y: i32 = 172;
const OK_X: i32 = 208;
/// 4 px between these two, not the usual 6.
const CANCEL_X: i32 = 287;

pub const FAILED: &str = "The user name or password is incorrect.";

/// Rows the block under the message shifts down by: one per line past two.
fn extra_rows(lines: usize) -> i32 {
    lines.saturating_sub(2) as i32
}

pub fn client_size(lines: usize) -> (i32, i32) {
    (CLIENT.0, CLIENT.1 + extra_rows(lines) * LINE_H)
}

pub enum Outcome {
    None,
    Redraw,
    /// Try this user and password.
    Submit {
        user: String,
        password: Zeroizing<String>,
    },
    /// Another user was picked: the conversation starts over for them.
    UserChanged(String),
    Cancel,
}

// `Zeroizing` forwards `Debug` to its contents, so a derived `Debug`
// here would print the password; this prints everything but it.
impl std::fmt::Debug for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::None => write!(f, "None"),
            Outcome::Redraw => write!(f, "Redraw"),
            Outcome::Submit { user, .. } => f
                .debug_struct("Submit")
                .field("user", user)
                .field("password", &"<redacted>")
                .finish(),
            Outcome::UserChanged(user) => f.debug_tuple("UserChanged").field(user).finish(),
            Outcome::Cancel => write!(f, "Cancel"),
        }
    }
}

/// The dialog's model: the form, and the users polkit offered in combo order.
pub struct RunAs {
    pub form: Form,
    pub size: (i32, i32),
    users: Vec<String>,
    busy: bool,
}

impl RunAs {
    /// `message` is polkit's message already wrapped to `MSG_W`. The combo
    /// starts on `users[selected]`.
    pub fn new(
        message: &[String],
        users: &[String],
        selected: usize,
        key: Option<Canvas>,
        user_icon: Option<Canvas>,
    ) -> RunAs {
        let refs: Vec<&str> = message.iter().map(|s| s.as_str()).collect();
        let size = client_size(refs.len().max(1));
        let extra = size.1 - CLIENT.1;
        let mut combo = Combo::with_items(
            users
                .iter()
                .map(|u| ListItem::shell(u.clone(), user_icon.clone(), 0))
                .collect(),
            selected.min(users.len().saturating_sub(1)),
            8,
        );
        combo.enabled = users.len() > 1;
        // The closed field draws this icon; the open list's rows use the one above.
        combo.icon = user_icon.clone();
        let items = vec![
            Item::picture("icon", ICON, key),
            Item::lines(
                "message",
                Rect::new(MSG_X, MSG_Y, MSG_W, LINE_H * refs.len() as i32),
                &refs,
            ),
            Item::label(
                "following",
                Rect::new(FOLLOWING_X, FOLLOWING_Y + extra, 200, LINE_H),
                "The following user:",
            ),
            Item::label(
                "user_l",
                Rect::new(LABEL_X, COMBO_Y + extra + LABEL_DY, 100, LINE_H),
                "&User name:",
            ),
            Item::new(
                "user",
                Rect::new(FIELD_X, COMBO_Y + extra, COMBO_W, combo::COMBO_H),
                Control::Combo(combo),
            ),
            Item::label(
                "password_l",
                Rect::new(LABEL_X, EDIT_Y + extra + LABEL_DY, 100, LINE_H),
                "&Password:",
            ),
            Item::new(
                "password",
                Rect::new(FIELD_X, EDIT_Y + extra, EDIT_W, EDIT_H),
                Control::Edit {
                    edit: TextEdit::secret(),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
            // A long PAM or helper message runs onto a second line, which ends at the buttons' top.
            Item::lines(
                "status",
                Rect::new(
                    LABEL_X,
                    STATUS_Y + extra,
                    CLIENT.0 - LABEL_X - 16,
                    2 * LINE_H,
                ),
                &[],
            ),
            Item::button(
                "ok",
                Rect::new(OK_X, BTN_Y + extra, BUTTON_W, BUTTON_H),
                "OK",
                false,
                true,
            ),
            Item::button(
                "cancel",
                Rect::new(CANCEL_X, BTN_Y + extra, BUTTON_W, BUTTON_H),
                "Cancel",
                true,
                false,
            ),
        ];
        let mut form = Form::new(items);
        form.focus_id("password");
        RunAs {
            form,
            size,
            users: users.to_vec(),
            busy: false,
        }
    }

    /// A key or a click the host has already given the form, as an `Outcome`.
    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed("user") => {
                self.sync_ok();
                Outcome::UserChanged(self.user())
            }
            form::Event::Changed(_) => {
                self.sync_ok();
                Outcome::Redraw
            }
            form::Event::Pressed("ok") | form::Event::Default => {
                if !self.can_submit() {
                    return Outcome::None;
                }
                let user = self.user();
                let password = self.take_password();
                Outcome::Submit { user, password }
            }
            form::Event::Pressed("cancel") | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn user(&self) -> String {
        self.form
            .combo_selected("user")
            .and_then(|i| self.users.get(i))
            .cloned()
            .unwrap_or_default()
    }

    /// The helper is running: OK stays greyed until it answers.
    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
        self.sync_ok();
    }

    /// The attempt failed: empty field, focus back in it, and `pam_text`
    /// (the helper's PAM text, if any) or the generic failure in the status line.
    pub fn fail(&mut self, pam_text: Option<&str>) {
        self.set_busy(false);
        self.take_password();
        self.set_status(pam_text.unwrap_or(FAILED));
        self.form.focus_id("password");
    }

    /// Wipe the password field in place, for a request that ends without a submit.
    pub fn wipe(&mut self) {
        if let Some(Control::Edit { edit, .. }) =
            self.form.item_mut("password").map(|i| &mut i.control)
        {
            edit.wipe();
        }
    }

    pub fn set_status(&mut self, s: &str) {
        self.form.set_text("status", s);
    }

    #[cfg(test)]
    pub fn status(&self) -> &str {
        match self.form.item("status").map(|i| &i.control) {
            Some(Control::Lines(lines)) => lines.first().map_or("", |l| l),
            _ => "",
        }
    }

    pub fn password(&self) -> &str {
        match self.form.item("password").map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => &edit.text,
            _ => "",
        }
    }

    fn can_submit(&self) -> bool {
        !self.busy && !self.password().is_empty()
    }

    fn take_password(&mut self) -> Zeroizing<String> {
        let out = Zeroizing::new(self.password().to_string());
        if let Some(Control::Edit { edit, .. }) =
            self.form.item_mut("password").map(|i| &mut i.control)
        {
            edit.wipe();
        }
        self.sync_ok();
        out
    }

    fn sync_ok(&mut self) {
        let on = self.can_submit();
        let focused = self.form.focus() == self.form.index_of("ok");
        self.form.set_enabled("ok", on);
        // A greyed button is no tab stop, so the focus cannot stay on it.
        if focused && !on {
            self.form.focus_id("password");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::text::wrap_chars;

    fn dialog(users: &[&str]) -> RunAs {
        let users: Vec<String> = users.iter().map(|s| s.to_string()).collect();
        RunAs::new(
            &wrap_chars("Authentication is required to mount the disk.", MSG_COLS),
            &users,
            0,
            None,
            None,
        )
    }

    fn dialog_with(message: &str) -> RunAs {
        RunAs::new(
            &wrap_chars(message, MSG_COLS),
            &["root".to_string()],
            0,
            None,
            None,
        )
    }

    fn type_into(d: &mut RunAs, id: &str, s: &str) -> Outcome {
        d.form.focus_id(id);
        let mut last = Outcome::None;
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            last = d.event(ev);
        }
        last
    }

    fn rect(d: &RunAs, id: &str) -> Rect {
        d.form.item(id).unwrap_or_else(|| panic!("no {id}")).rect
    }

    fn button_enabled(d: &RunAs, id: &str) -> bool {
        match d.form.item(id).unwrap().control {
            Control::Button { enabled, .. } => enabled,
            _ => panic!("{id} is no button"),
        }
    }

    #[test]
    fn the_layout_places_the_rows() {
        let d = dialog(&["root"]);
        assert_eq!(rect(&d, "icon"), ICON);
        assert_eq!(rect(&d, "message").x, MSG_X);
        assert_eq!(rect(&d, "following").x, FOLLOWING_X);
        assert_eq!(
            rect(&d, "user"),
            Rect::new(FIELD_X, COMBO_Y, COMBO_W, combo::COMBO_H)
        );
        assert_eq!(
            rect(&d, "password"),
            Rect::new(FIELD_X, EDIT_Y, EDIT_W, EDIT_H)
        );
        assert_eq!(rect(&d, "ok"), Rect::new(OK_X, BTN_Y, BUTTON_W, BUTTON_H));
        assert_eq!(
            rect(&d, "cancel"),
            Rect::new(CANCEL_X, BTN_Y, BUTTON_W, BUTTON_H)
        );
        assert_eq!(rect(&d, "cancel").right(), CLIENT.0 - 16);
        assert_eq!(
            rect(&d, "status").bottom(),
            BTN_Y,
            "two status lines end at the buttons"
        );
        assert_eq!(rect(&d, "ok").bottom(), CLIENT.1 - 12);
        assert!(matches!(
            &d.form.item("password").unwrap().control,
            Control::Edit { edit, .. } if edit.is_secret()
        ));
        // The password field takes the first keystroke.
        assert_eq!(d.form.focus(), d.form.index_of("password"));
    }

    #[test]
    fn one_identity_greys_the_combo_and_several_do_not() {
        let one = dialog(&["root"]);
        let Control::Combo(c) = &one.form.item("user").unwrap().control else {
            panic!("no combo");
        };
        assert!(!c.enabled);
        assert_eq!(c.items.len(), 1);
        let two = dialog(&["root", "alice"]);
        let Control::Combo(c) = &two.form.item("user").unwrap().control else {
            panic!("no combo");
        };
        assert!(c.enabled);
        assert_eq!(c.items[1].text, "alice");

        let users = vec!["root".to_string()];
        let iconed = RunAs::new(
            &wrap_chars("Authentication is required to mount the disk.", MSG_COLS),
            &users,
            0,
            None,
            Some(Canvas::new(16, 16)),
        );
        let Control::Combo(c) = &iconed.form.item("user").unwrap().control else {
            panic!("no combo");
        };
        assert!(c.icon.is_some());
        assert!(c.items[0].icon.is_some());
    }

    #[test]
    fn ok_waits_for_a_password_and_submits_the_picked_user() {
        let mut d = dialog(&["root", "alice"]);
        assert!(!button_enabled(&d, "ok"));
        assert!(matches!(d.event(form::Event::Default), Outcome::None));
        type_into(&mut d, "password", "hunter2");
        assert!(button_enabled(&d, "ok"));
        d.form.set_combo("user", 1);
        match d.event(form::Event::Default) {
            Outcome::Submit { user, password } => {
                assert_eq!(user, "alice");
                assert_eq!(&*password, "hunter2");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ok_submits_the_preselected_user_untouched() {
        let users = vec!["alice".to_string(), "bob".to_string()];
        let mut d = RunAs::new(&wrap_chars("Mount.", MSG_COLS), &users, 1, None, None);
        type_into(&mut d, "password", "hunter2");
        match d.event(form::Event::Default) {
            Outcome::Submit { user, .. } => assert_eq!(user, "bob"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn picking_another_user_says_who() {
        let mut d = dialog(&["root", "alice"]);
        assert_eq!(d.user(), "root");
        d.form.set_combo("user", 1);
        match d.event(form::Event::Changed("user")) {
            Outcome::UserChanged(user) => assert_eq!(user, "alice"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            d.event(form::Event::Changed("password")),
            Outcome::Redraw
        ));
    }

    #[test]
    fn a_failure_clears_the_field_and_says_so() {
        let mut d = dialog(&["root"]);
        type_into(&mut d, "password", "wrong");
        d.set_busy(true);
        assert!(!button_enabled(&d, "ok"), "busy greys OK");
        d.fail(None);
        assert_eq!(d.password(), "");
        assert!(!button_enabled(&d, "ok"));
        assert_eq!(d.status(), FAILED);
        assert_eq!(d.form.focus(), d.form.index_of("password"));
        d.set_status("Try again");
        assert_eq!(d.status(), "Try again");
    }

    #[test]
    fn a_pam_error_message_replaces_the_generic_failure_text() {
        let mut d = dialog(&["root"]);
        type_into(&mut d, "password", "wrong");
        d.fail(Some("Account locked."));
        assert_eq!(d.status(), "Account locked.");
    }

    #[test]
    fn wipe_empties_the_field_in_place() {
        let mut d = dialog(&["root"]);
        type_into(&mut d, "password", "hunter2");
        d.wipe();
        assert_eq!(d.password(), "");
        assert!(!button_enabled(&d, "ok") || d.password().is_empty());
    }

    #[test]
    fn cancel_and_redraw_pass_through() {
        let mut d = dialog(&["root"]);
        assert!(matches!(
            d.event(form::Event::Pressed("cancel")),
            Outcome::Cancel
        ));
        assert!(matches!(d.event(form::Event::Cancel), Outcome::Cancel));
        assert!(matches!(d.event(form::Event::Redraw), Outcome::Redraw));
        assert!(matches!(d.event(form::Event::None), Outcome::None));
    }

    #[test]
    fn the_message_wraps_inside_its_column() {
        let long = "Authentication is required to change the system time zone and this program needs it now";
        let lines = wrap_chars(long, MSG_COLS);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| l.chars().count() <= MSG_COLS));
        let d = dialog_with(long);
        assert_eq!(rect(&d, "message").height, lines.len() as i32 * LINE_H);
    }

    #[test]
    fn a_taller_message_grows_the_client_and_pushes_the_rows_below_it() {
        let one_line = "Authentication is required.";
        assert_eq!(client_size(1), CLIENT, "one line is the baseline too");
        let d = dialog_with(one_line);
        assert_eq!(d.size, CLIENT);
        assert_eq!(rect(&d, "following").y, FOLLOWING_Y);

        let three_lines = "Authentication is required to change the system time zone \
            for this program and it needs your password right now to make sure you \
            are authorized";
        assert_eq!(wrap_chars(three_lines, MSG_COLS).len(), 3);
        assert_eq!(client_size(3), (CLIENT.0, CLIENT.1 + LINE_H));
        let d = dialog_with(three_lines);
        assert_eq!(d.size, (CLIENT.0, CLIENT.1 + LINE_H));
        assert_eq!(rect(&d, "following").y, FOLLOWING_Y + LINE_H);
    }

    /// The box fits a long action message wrapped as the host wraps it, several identities
    /// with a long user name, and each status line the agent shows, under every interface
    /// and text size.
    #[test]
    fn the_box_fits_its_text() {
        let users: Vec<String> = ["root", "alice", "maintenance-administrator", "backup"]
            .map(String::from)
            .into();
        let message = "Authentication is required to mount Kingston DataTraveler 3.0 \
            (/dev/sdc1) referenced in the /etc/fstab file as a removable drive";
        let statuses = [
            FAILED,
            "The account is locked due to 3 failed logins.",
            "(9 minutes left to unlock)",
            "Place your finger on the fingerprint reader",
            "Failed to match fingerprint",
            "The polkit helper could not be started.",
            "Cannot reach the polkit helper: No such file or directory (os error 2)",
        ];
        let over = reprisekit::interfaces::form_overflows(|_, forms, text| {
            let width = forms.text.len(MSG_W);
            let lines = reprisekit::text::wrap(text, message, width, forms.fonts.size_px);
            statuses
                .iter()
                .map(|status| {
                    let mut d = RunAs::new(&lines, &users, 2, None, Some(Canvas::new(16, 16)));
                    d.set_status(status);
                    (format!("runas \"{status}\""), d.form)
                })
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    #[ignore]
    fn render_runas() {
        let theme = reprisekit::theme::Theme {
            access_keys: true,
            ..reprisekit::theme::Theme::default()
        };
        let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig::default());
        let mut d = dialog(&["root"]);
        type_into(&mut d, "password", "hunter2");
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &d.form,
            false,
            theme.fonts.size_px,
        );
        reprisekit::raster::write_preview(&c, "runas.png");
    }
}
