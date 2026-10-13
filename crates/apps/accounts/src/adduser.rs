//! Add Account: the box behind Add.... Create stays greyed until the fields hold together; a line says what is missing.

use std::cell::Cell;

use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

use crate::acct::Op;

pub const CLIENT: (i32, i32) = (372, 330);

const FULL: &str = "full";
const LOGIN: &str = "login";
const ADMIN: &str = "admin";
const PASSWORD: &str = "password";
const CONFIRM: &str = "confirm";
const AT_LOGIN: &str = "at_login";
const HINT: &str = "hint";
const CREATE: &str = "create";
const CANCEL: &str = "cancel";

const X: i32 = 12;
const FIELD_X: i32 = 120;
const FIELD_W: i32 = CLIENT.0 - X - FIELD_X;
const FIELD_H: i32 = 21;

const LOGIN_MAX: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Create(Op),
    Cancel,
}

pub struct AddUser {
    pub form: Form,
    taken: Vec<String>,
    login_edited: bool,
}

fn field(id: &'static str, y: i32, secret: bool) -> Item {
    Item::new(
        id,
        Rect::new(FIELD_X, y, FIELD_W, FIELD_H),
        Control::Edit {
            edit: if secret {
                TextEdit::secret()
            } else {
                TextEdit::new("", false)
            },
            first: Cell::new(0),
            enabled: true,
        },
    )
}

/// A user name made from a full name: its first word, lower case, kept
/// to the letters and digits a login takes.
pub fn suggest(full: &str) -> String {
    let word = full.split_whitespace().next().unwrap_or("");
    let s: String = word
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-' || *c == '_')
        .collect();
    let s = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-');
    s.chars().take(LOGIN_MAX).collect()
}

pub fn login_problem(login: &str, taken: &[String]) -> Option<&'static str> {
    let mut chars = login.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    let rest_ok = login
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if login.is_empty() {
        Some("Type a user name.")
    } else if login.len() > LOGIN_MAX {
        Some("The user name is longer than 32 characters.")
    } else if !first_ok || !rest_ok {
        Some("Use small letters, digits, - and _, starting with a letter.")
    } else if taken.iter().any(|t| t == login) {
        Some("That user name is taken.")
    } else {
        None
    }
}

impl AddUser {
    pub fn new(taken: Vec<String>) -> AddUser {
        let gy = 104;
        let py = 176;
        let by = CLIENT.1 - X - BUTTON_H;
        let items = vec![
            Item::lines_at(
                "intro",
                X,
                12,
                CLIENT.0 - 2 * X,
                &[
                    "Type the person's name. The user name is what they",
                    "log in with; it cannot be changed later.",
                ],
            ),
            Item::label_at("full_l", X, 52, 100, "&Full name:"),
            field(FULL, 48, false),
            Item::label_at("login_l", X, 80, 100, "&User name:"),
            field(LOGIN, 76, false),
            Item::group(
                "kind_g",
                Rect::new(X, gy, CLIENT.0 - 2 * X, 62),
                "Account type",
            ),
            Item::radio("standard", X + 12, gy + 18, "&Standard user", "kind").checked(true),
            Item::radio(ADMIN, X + 12, gy + 38, "&Administrator", "kind"),
            Item::group(
                "password_g",
                Rect::new(X, py, CLIENT.0 - 2 * X, 92),
                "Password",
            ),
            Item::label_at("password_l", X + 12, py + 22, 96, "&Password:"),
            Item::new(
                PASSWORD,
                Rect::new(FIELD_X, py + 18, FIELD_W - 12, FIELD_H),
                Control::Edit {
                    edit: TextEdit::secret(),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
            Item::label_at("confirm_l", X + 12, py + 48, 96, "C&onfirm:"),
            Item::new(
                CONFIRM,
                Rect::new(FIELD_X, py + 44, FIELD_W - 12, FIELD_H),
                Control::Edit {
                    edit: TextEdit::secret(),
                    first: Cell::new(0),
                    enabled: true,
                },
            ),
            Item::check(
                AT_LOGIN,
                X + 12,
                py + 72,
                "As&k for a password at the first login",
            ),
            Item::label(HINT, Rect::new(X, py + 100, CLIENT.0 - 2 * X, LINE_H), ""),
            Item::button(
                CREATE,
                Rect::new(CLIENT.0 - X - 2 * BUTTON_W - 6, by, BUTTON_W, BUTTON_H),
                "Create",
                false,
                true,
            ),
            Item::button_at(
                CANCEL,
                CLIENT.0 - X - BUTTON_W,
                by,
                BUTTON_W,
                "Cancel",
                true,
            ),
        ];
        let mut d = AddUser {
            form: Form::new(items),
            taken,
            login_edited: false,
        };
        d.form.focus_id(FULL);
        d.sync();
        d
    }

    fn text(&self, id: &str) -> &str {
        match self.form.item(id).map(|i| &i.control) {
            Some(Control::Edit { edit, .. }) => &edit.text,
            _ => "",
        }
    }

    fn at_login(&self) -> bool {
        self.form.checked(AT_LOGIN) == Some(true)
    }

    pub fn problem(&self) -> Option<&'static str> {
        if self.text(FULL).contains([':', ',']) {
            return Some("A full name cannot hold a colon or a comma.");
        }
        if let Some(p) = login_problem(self.text(LOGIN), &self.taken) {
            return Some(p);
        }
        if self.at_login() {
            return None;
        }
        if self.text(PASSWORD).is_empty() {
            Some("Type a password, or let them pick one at the first login.")
        } else if self.text(PASSWORD) != self.text(CONFIRM) {
            Some("The passwords do not match.")
        } else {
            None
        }
    }

    fn sync(&mut self) {
        let at_login = self.at_login();
        self.form.set_enabled(PASSWORD, !at_login);
        self.form.set_enabled(CONFIRM, !at_login);
        let problem = self.problem();
        let typed = !self.text(FULL).is_empty() || !self.text(LOGIN).is_empty();
        self.form
            .set_text(HINT, if typed { problem.unwrap_or("") } else { "" });
        self.form.set_enabled(CREATE, problem.is_none());
    }

    fn fill_login(&mut self) {
        if self.login_edited {
            return;
        }
        let s = suggest(self.text(FULL));
        if let Some(Control::Edit { edit, .. }) = self.form.item_mut(LOGIN).map(|i| &mut i.control)
        {
            *edit = TextEdit::new(s, false);
        }
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(FULL) => {
                self.fill_login();
                self.sync();
                Outcome::Redraw
            }
            form::Event::Changed(LOGIN) => {
                self.login_edited = true;
                self.sync();
                Outcome::Redraw
            }
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed(CREATE) | form::Event::Default => {
                if self.problem().is_some() {
                    return Outcome::None;
                }
                Outcome::Create(Op::Create {
                    login: self.text(LOGIN).to_string(),
                    real_name: self.text(FULL).trim().to_string(),
                    admin: self.form.checked(ADMIN) == Some(true),
                    password: (!self.at_login()).then(|| self.text(PASSWORD).to_string()),
                })
            }
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::None => Outcome::None,
            _ => Outcome::Redraw,
        }
    }
}

impl Drop for AddUser {
    fn drop(&mut self) {
        for id in [PASSWORD, CONFIRM] {
            if let Some(Control::Edit { edit, .. }) = self.form.item_mut(id).map(|i| &mut i.control)
            {
                edit.wipe();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_into(d: &mut AddUser, id: &str, s: &str) {
        d.form.focus_id(id);
        for ch in s.chars() {
            let ev = d.form.key(form::Key::Char(ch));
            d.event(ev);
        }
    }

    fn create_live(d: &AddUser) -> bool {
        matches!(
            d.form.item(CREATE).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    /// The dialog fits its text under every interface and text size.
    #[test]
    fn the_dialog_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("add".into(), AddUser::new(vec![]).form.clone())]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    #[test]
    #[ignore]
    fn render_dialog() {
        use reprisekit::raster::Canvas;
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut d = AddUser::new(vec!["jo".into()]);
        type_into(&mut d, FULL, "Jo Doe");
        let (w, h) = CLIENT;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(&mut c, &text, &theme.paint(), &d.form, false, 11);
        reprisekit::raster::write_preview(&c, "accounts-add-preview.png");
    }

    #[test]
    fn a_user_name_is_suggested_from_the_full_name() {
        assert_eq!(suggest("Jo Doe"), "jo");
        assert_eq!(suggest("  Ádám Kovács"), "dm");
        assert_eq!(suggest("42nd Street"), "nd");
        assert_eq!(suggest(""), "");
    }

    #[test]
    fn user_names_follow_the_login_rules() {
        let taken = vec!["jo".to_string()];
        assert_eq!(login_problem("sam_2-b", &taken), None);
        assert_eq!(
            login_problem("jo", &taken),
            Some("That user name is taken.")
        );
        assert!(login_problem("Sam", &taken).is_some());
        assert!(login_problem("2sam", &taken).is_some());
        assert!(login_problem(&"a".repeat(33), &taken).is_some());
        assert_eq!(login_problem("", &taken), Some("Type a user name."));
    }

    #[test]
    fn create_waits_for_a_name_and_a_matching_password() {
        let mut d = AddUser::new(vec!["jo".into()]);
        assert!(!create_live(&d));
        type_into(&mut d, FULL, "Sam Lee");
        assert_eq!(d.text(LOGIN), "sam", "filled in from the full name");
        assert_eq!(
            d.problem(),
            Some("Type a password, or let them pick one at the first login.")
        );
        type_into(&mut d, PASSWORD, "pw12345!");
        type_into(&mut d, CONFIRM, "pw12345?");
        assert_eq!(d.problem(), Some("The passwords do not match."));
        if let Some(Control::Edit { edit, .. }) = d.form.item_mut(CONFIRM).map(|i| &mut i.control) {
            edit.wipe();
        }
        type_into(&mut d, CONFIRM, "pw12345!");
        assert!(create_live(&d));
        assert_eq!(
            d.event(form::Event::Pressed(CREATE)),
            Outcome::Create(Op::Create {
                login: "sam".into(),
                real_name: "Sam Lee".into(),
                admin: false,
                password: Some("pw12345!".into()),
            })
        );
    }

    #[test]
    fn a_typed_user_name_stays_and_first_login_needs_no_password() {
        let mut d = AddUser::new(Vec::new());
        type_into(&mut d, LOGIN, "lee");
        type_into(&mut d, FULL, "Sam");
        assert_eq!(d.text(LOGIN), "lee");
        d.form.focus_id(AT_LOGIN);
        let ev = d.form.key(form::Key::Space);
        d.event(ev);
        assert!(matches!(
            d.form.item(PASSWORD).map(|i| &i.control),
            Some(Control::Edit { enabled: false, .. })
        ));
        d.form.focus_id(ADMIN);
        let ev = d.form.key(form::Key::Space);
        d.event(ev);
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Create(Op::Create {
                login: "lee".into(),
                real_name: "Sam".into(),
                admin: true,
                password: None,
            })
        );
    }
}
