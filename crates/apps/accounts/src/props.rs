//! An account's Properties: a sheet with three tabs (General, Picture, Password). On your
//! own account Password asks for the current password first. Apply and OK hand the host a
//! list of [`Op`]s for what changed; the sheet is a pure model and the host loads pictures,
//! opens the file dialog and runs the calls.

use std::cell::Cell;
use std::path::PathBuf;

use reprisekit::art::userpic::{self, ALL};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::iconlist::{self, IconList};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::textedit::TextEdit;

use crate::acct::{Op, User};
use crate::picture::Source;

pub const CLIENT: (i32, i32) = (398, 380);
pub const TABS: [&str; 3] = ["General", "Picture", "Password"];
pub const TAB_GENERAL: usize = 0;
pub const TAB_PICTURE: usize = 1;
pub const TAB_PASSWORD: usize = 2;

const NAME: &str = "name";
const STANDARD: &str = "standard";
const ADMIN: &str = "admin";
const KIND: &str = "kind";
const FACE: &str = "face";
const STOCK: &str = "stock";
const PREVIEW: &str = "preview";
pub const BROWSE: &str = "browse";
const CURRENT: &str = "current";
const NEW: &str = "new";
const CONFIRM: &str = "confirm";
const AT_LOGIN: &str = "at_login";

const FACE_PX: u32 = 48;
pub const PREVIEW_PX: u32 = 96;

const X: i32 = 16;
const FIELD_X: i32 = 150;
const FIELD_W: i32 = 232;
const FIELD_H: i32 = 21;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Apply(Vec<Op>),
    ApplyAndClose(Vec<Op>),
    /// Something typed cannot be applied; the host shows why.
    Refuse(String),
    Browse,
    Close,
}

pub struct Props {
    pub sheet: PropSheet,
    pub user: User,
    /// Whether this is the only administrator, who cannot be made
    /// standard.
    last_admin: bool,
    /// A picture picked and not yet applied.
    picture: Option<Source>,
}

fn secret(id: &'static str, y: i32) -> Item {
    Item::new(
        id,
        Rect::new(FIELD_X, y, FIELD_W, FIELD_H),
        Control::Edit {
            edit: TextEdit::secret(),
            first: Cell::new(0),
            enabled: true,
        },
    )
}

fn general_page(user: &User, face: Option<Canvas>, last_admin: bool) -> Form {
    let gy = 172;
    Form::new(vec![
        Item::new(
            FACE,
            Rect::new(X, 40, FACE_PX as i32 + 4, FACE_PX as i32 + 4),
            Control::Picture {
                canvas: face,
                frame: 2,
            },
        ),
        Item::label_at("login", 84, 50, 290, &user.login),
        Item::label_at(KIND, 84, 50 + LINE_H + 4, 290, user.kind()),
        Item::new(
            "sep",
            Rect::new(X, 104, CLIENT.0 - 2 * X, 2),
            Control::Separator,
        ),
        Item::label_at("name_l", X, 122, 120, "&Full name:"),
        Item::edit(
            NAME,
            Rect::new(FIELD_X - 50, 118, FIELD_W + 50, FIELD_H),
            &user.real_name,
            true,
        ),
        Item::label_at("login_l", X, 148, 80, "User name:"),
        Item::label_at("login_v", FIELD_X - 50, 148, FIELD_W + 50, &user.login),
        Item::group(
            "kind_g",
            Rect::new(X, gy, CLIENT.0 - 2 * X, 112),
            "Account type",
        ),
        Item::radio(STANDARD, X + 12, gy + 20, "&Standard user", "kind")
            .checked(!user.admin)
            .enabled(!last_admin),
        Item::lines_at(
            "standard_d",
            X + 30,
            gy + 38,
            320,
            &["Can use programs and change their own settings."],
        ),
        Item::radio(ADMIN, X + 12, gy + 60, "&Administrator", "kind").checked(user.admin),
        Item::lines_at(
            "admin_d",
            X + 30,
            gy + 78,
            320,
            &["Can change any setting and manage other accounts."],
        ),
    ])
}

fn picture_page(face: Option<Canvas>) -> Form {
    let icons = ALL.iter().map(|p| Some(userpic::draw(*p, 32))).collect();
    let names = ALL.iter().map(|p| p.name().to_string()).collect();
    let list_w = 2 * iconlist::CELL_W + 4;
    let list_h = iconlist::ROWS as i32 * iconlist::CELL_H + 2 + 17 + 4;
    let px = PREVIEW_PX as i32 + 4;
    let right = X + list_w + 24;
    Form::new(vec![
        Item::lines_at(
            "intro",
            X,
            40,
            CLIENT.0 - 2 * X,
            &[
                "Pick a picture for the account, or browse for one of your",
                "own. A picture that is not square is cut to its middle.",
            ],
        ),
        Item::new(
            STOCK,
            Rect::new(X, 80, list_w, list_h),
            Control::IconList(Box::new(IconList::new(names, icons, None))),
        ),
        Item::new(
            PREVIEW,
            Rect::new(right, 80, px, px),
            Control::Picture {
                canvas: face,
                frame: 2,
            },
        ),
        Item::button_at(BROWSE, right, 80 + px + 12, px, "&Browse...", true),
    ])
}

fn password_page(user: &User) -> Form {
    if user.you {
        return Form::new(vec![
            Item::lines_at(
                "intro",
                X,
                40,
                CLIENT.0 - 2 * X,
                &[
                    "To change your password, type the one you use now,",
                    "then the new one twice.",
                ],
            ),
            Item::label_at("current_l", X, 84, 130, "&Current password:"),
            secret(CURRENT, 80),
            Item::label_at("new_l", X, 114, 130, "&New password:"),
            secret(NEW, 110),
            Item::label_at("confirm_l", X, 144, 130, "C&onfirm new password:"),
            secret(CONFIRM, 140),
        ]);
    }
    Form::new(vec![
        Item::lines_at(
            "intro",
            X,
            40,
            CLIENT.0 - 2 * X,
            &[
                "Set a new password for this account. Anything its old",
                "password unlocked, such as saved keyring passwords,",
                "stays locked until it is unlocked with the old one.",
            ],
        ),
        Item::label_at("new_l", X, 98, 130, "&New password:"),
        secret(NEW, 94),
        Item::label_at("confirm_l", X, 128, 130, "C&onfirm new password:"),
        secret(CONFIRM, 124),
        Item::check(
            AT_LOGIN,
            X,
            160,
            "As&k for a new password at the next login",
        ),
    ])
}

fn edit_text<'a>(form: &'a Form, id: &str) -> &'a str {
    match form.item(id).map(|i| &i.control) {
        Some(Control::Edit { edit, .. }) => &edit.text,
        _ => "",
    }
}

fn wipe(form: &mut Form, id: &str) {
    if let Some(Control::Edit { edit, .. }) = form.item_mut(id).map(|i| &mut i.control) {
        edit.wipe();
    }
}

impl Props {
    /// The sheet for `user`. `face` is its picture at [`PREVIEW_PX`];
    /// `last_admin` greys Standard on the only administrator.
    pub fn new(user: User, face: Option<Canvas>, last_admin: bool) -> Props {
        let small = face.as_ref().map(|c| c.resample_box(FACE_PX, FACE_PX));
        let pages = vec![
            general_page(&user, small, last_admin),
            picture_page(face),
            password_page(&user),
        ];
        let mut p = Props {
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            user,
            last_admin,
            picture: None,
        };
        p.sync();
        p
    }

    pub fn window_title(&self) -> String {
        format!("{} Properties", self.user.shown_name())
    }

    fn pending_name(&self) -> String {
        edit_text(&self.sheet.pages[TAB_GENERAL], NAME)
            .trim()
            .to_string()
    }

    fn pending_admin(&self) -> bool {
        self.sheet.pages[TAB_GENERAL]
            .checked(ADMIN)
            .unwrap_or(self.user.admin)
    }

    fn password_typed(&self) -> bool {
        let page = &self.sheet.pages[TAB_PASSWORD];
        [CURRENT, NEW, CONFIRM]
            .iter()
            .any(|id| !edit_text(page, id).is_empty())
            || page.checked(AT_LOGIN) == Some(true)
    }

    pub fn dirty(&self) -> bool {
        self.pending_name() != self.user.real_name.trim()
            || self.pending_admin() != self.user.admin
            || self.picture.is_some()
            || self.password_typed()
    }

    fn sync(&mut self) {
        let at_login = self.sheet.pages[TAB_PASSWORD].checked(AT_LOGIN) == Some(true);
        let page = &mut self.sheet.pages[TAB_PASSWORD];
        page.set_enabled(NEW, !at_login);
        page.set_enabled(CONFIRM, !at_login);
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
    }

    /// The calls for what changed, or why it cannot be applied.
    /// `picture_file` saves a picked picture for AccountsService.
    pub fn ops(
        &self,
        picture_file: impl FnOnce(&Source) -> Result<PathBuf, String>,
    ) -> Result<Vec<Op>, String> {
        let mut ops = Vec::new();
        let path = self.user.path.clone();
        let page = &self.sheet.pages[TAB_PASSWORD];
        if self.password_typed() {
            let new = edit_text(page, NEW);
            let at_login = page.checked(AT_LOGIN) == Some(true);
            if self.user.you {
                let current = edit_text(page, CURRENT);
                if current.is_empty() {
                    return Err("Type your current password first.".into());
                }
                if new.is_empty() {
                    return Err("Type a new password.".into());
                }
            } else if !at_login && new.is_empty() {
                return Err("Type a new password.".into());
            }
            if !at_login && new != edit_text(page, CONFIRM) {
                return Err("The new passwords do not match. Type them again.".into());
            }
        }
        let name = self.pending_name();
        if name != self.user.real_name.trim() {
            if name.contains([':', ',', '\n']) {
                return Err("A full name cannot hold a colon or a comma.".into());
            }
            ops.push(Op::RealName {
                path: path.clone(),
                name,
            });
        }
        if self.pending_admin() != self.user.admin {
            ops.push(Op::Admin {
                path: path.clone(),
                admin: self.pending_admin(),
            });
        }
        if let Some(src) = &self.picture {
            ops.push(Op::Picture {
                path: path.clone(),
                file: picture_file(src)?,
                you: self.user.you,
            });
        }
        if self.password_typed() {
            let new = edit_text(page, NEW).to_string();
            ops.push(if self.user.you {
                Op::OwnPassword {
                    current: edit_text(page, CURRENT).to_string(),
                    new,
                }
            } else if page.checked(AT_LOGIN) == Some(true) {
                Op::PasswordAtLogin { path }
            } else {
                Op::Password {
                    path,
                    password: new,
                }
            });
        }
        Ok(ops)
    }

    fn submit(
        &mut self,
        close: bool,
        picture_file: impl FnOnce(&Source) -> Result<PathBuf, String>,
    ) -> Outcome {
        if !self.dirty() {
            return if close {
                Outcome::Close
            } else {
                Outcome::Redraw
            };
        }
        match self.ops(picture_file) {
            Ok(ops) if close => Outcome::ApplyAndClose(ops),
            Ok(ops) => {
                self.sheet.set_apply_enabled(false);
                Outcome::Apply(ops)
            }
            Err(why) => Outcome::Refuse(why),
        }
    }

    pub fn event(
        &mut self,
        ev: propsheet::Event,
        picture_file: impl FnOnce(&Source) -> Result<PathBuf, String>,
    ) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Pressed(BROWSE)) => Outcome::Browse,
            propsheet::Event::Page(_, form::Event::Changed(STOCK)) => {
                let picked =
                    self.sheet.pages[TAB_PICTURE]
                        .item(STOCK)
                        .and_then(|i| match &i.control {
                            Control::IconList(l) => l.selected,
                            _ => None,
                        });
                if let Some(i) = picked {
                    let p = ALL[i];
                    self.show_picture(Source::Stock(p), userpic::draw(p, PREVIEW_PX));
                }
                Outcome::Redraw
            }
            propsheet::Event::Page(..) => {
                self.sync();
                Outcome::Redraw
            }
            propsheet::Event::Apply => self.submit(false, picture_file),
            propsheet::Event::Ok => self.submit(true, picture_file),
            propsheet::Event::Cancel => Outcome::Close,
        }
    }

    fn show_picture(&mut self, src: Source, preview: Canvas) {
        let small = preview.resample_box(FACE_PX, FACE_PX);
        self.sheet.pages[TAB_PICTURE].set_picture(PREVIEW, preview);
        self.sheet.pages[TAB_GENERAL].set_picture(FACE, small);
        self.picture = Some(src);
        self.sync();
    }

    /// Browse... came back with a picture file, `preview` its square at
    /// [`PREVIEW_PX`]. The stock pick, if any, is let go.
    pub fn browsed(&mut self, path: PathBuf, preview: Canvas) {
        if let Some(Control::IconList(l)) = self.sheet.pages[TAB_PICTURE]
            .item_mut(STOCK)
            .map(|i| &mut i.control)
        {
            l.selected = None;
        }
        self.show_picture(Source::File(path), preview);
    }

    /// The host carried the change out and read `user` back: the new
    /// baseline. The password fields empty.
    pub fn applied(&mut self, user: User) {
        let last_admin = self.last_admin;
        self.user = user;
        self.picture = None;
        let page = &mut self.sheet.pages[TAB_GENERAL];
        page.set_text("login", &self.user.login);
        page.set_text(KIND, self.user.kind());
        if let Some(Control::Edit { edit, .. }) = page.item_mut(NAME).map(|i| &mut i.control) {
            edit.set_selected(self.user.real_name.clone());
        }
        page.set_enabled(STANDARD, !last_admin);
        self.wipe_passwords();
        self.sync();
    }

    /// A failed Apply leaves the edits pending, but never a typed
    /// password.
    pub fn unapplied(&mut self) {
        self.wipe_passwords();
        self.sync();
    }

    fn wipe_passwords(&mut self) {
        let page = &mut self.sheet.pages[TAB_PASSWORD];
        for id in [CURRENT, NEW, CONFIRM] {
            wipe(page, id);
        }
        page.set_check(AT_LOGIN, false);
    }

    pub fn set_last_admin(&mut self, last: bool) {
        self.last_admin = last;
        self.sheet.pages[TAB_GENERAL].set_enabled(STANDARD, !last);
    }

    pub fn paste(&mut self, t: String) -> Outcome {
        let ev = self.sheet.open_mut().paste(t);
        let page = self.sheet.tabs.open;
        self.event(propsheet::Event::Page(page, ev), |_| Err(String::new()))
    }
}

impl Drop for Props {
    fn drop(&mut self) {
        self.wipe_passwords();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acct::User;

    fn user(you: bool, admin: bool) -> User {
        User {
            path: "/u/1000".into(),
            uid: 1000,
            login: "jo".into(),
            real_name: "Jo Doe".into(),
            admin,
            icon: String::new(),
            home: "/home/jo".into(),
            you,
        }
    }

    fn type_into(p: &mut Props, tab: usize, id: &str, s: &str) {
        p.sheet.tabs.open = tab;
        let page = p.sheet.open_mut();
        page.focus_id(id);
        for ch in s.chars() {
            let ev = page.key(form::Key::Char(ch));
            assert_eq!(
                ev,
                form::Event::Changed(match id {
                    NAME => NAME,
                    CURRENT => CURRENT,
                    NEW => NEW,
                    _ => CONFIRM,
                })
            );
        }
        p.event(
            propsheet::Event::Page(tab, form::Event::Changed("x")),
            no_file,
        );
    }

    fn no_file(_: &Source) -> Result<PathBuf, String> {
        Ok(PathBuf::from("/run/user/1000/reprise-accounts/p.png"))
    }

    /// Every page fits its text under every interface and text size, for your account and
    /// for another one.
    #[test]
    fn every_page_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            [("yours", user(true, true)), ("another", user(false, false))]
                .into_iter()
                .flat_map(|(whose, u)| {
                    let s = Props::new(u, None, false).sheet.clone();
                    let tabs = s
                        .tabs
                        .labels
                        .into_iter()
                        .map(move |t| format!("{whose} {t}"));
                    tabs.zip(s.pages)
                })
                .collect()
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    /// Writes target/accounts-props-preview.png (the three tabs, yours and
    /// another account's Password) for a visual check.
    #[test]
    #[ignore]
    fn render_sheet() {
        use reprisekit::text::TextRenderer;
        use reprisekit::theme::{FontConfig, Theme};
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let (w, h) = CLIENT;
        let mut out = Canvas::new(4 * w as u32, h as u32);
        let face = Some(userpic::draw(userpic::Picture::Tree, PREVIEW_PX));
        for (i, (you, tab)) in [(true, 0), (true, 1), (true, 2), (false, 2)]
            .iter()
            .enumerate()
        {
            let mut p = Props::new(user(*you, true), face.clone(), false);
            p.sheet.tabs.open = *tab;
            let mut c = Canvas::new(w as u32, h as u32);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            propsheet::draw(&mut c, &text, &theme.paint(), &p.sheet, false, 11);
            out.blit_canvas(i as i32 * w, 0, &c);
        }
        reprisekit::raster::write_preview(&out, "accounts-props-preview.png");
    }

    #[test]
    fn nothing_changed_applies_nothing() {
        let mut p = Props::new(user(true, true), None, false);
        assert!(!p.dirty());
        assert_eq!(p.event(propsheet::Event::Ok, no_file), Outcome::Close);
        assert_eq!(p.window_title(), "Jo Doe Properties");
    }

    #[test]
    fn a_new_name_and_type_become_their_calls() {
        let mut p = Props::new(user(false, false), None, false);
        type_into(&mut p, TAB_GENERAL, NAME, " Jr");
        let page = &mut p.sheet.pages[TAB_GENERAL];
        page.focus_id(ADMIN);
        let ev = page.key(form::Key::Space);
        p.event(propsheet::Event::Page(TAB_GENERAL, ev), no_file);
        assert_eq!(
            p.event(propsheet::Event::Apply, no_file),
            Outcome::Apply(vec![
                Op::RealName {
                    path: "/u/1000".into(),
                    name: "Jo Doe Jr".into()
                },
                Op::Admin {
                    path: "/u/1000".into(),
                    admin: true
                },
            ])
        );
    }

    #[test]
    fn the_only_administrator_cannot_be_made_standard() {
        let p = Props::new(user(true, true), None, true);
        assert!(matches!(
            p.sheet.pages[TAB_GENERAL]
                .item(STANDARD)
                .map(|i| &i.control),
            Some(Control::Radio { enabled: false, .. })
        ));
    }

    #[test]
    fn your_password_needs_the_current_one_and_a_match() {
        let mut p = Props::new(user(true, false), None, false);
        type_into(&mut p, TAB_PASSWORD, NEW, "newpass1");
        type_into(&mut p, TAB_PASSWORD, CONFIRM, "newpass1");
        assert_eq!(
            p.event(propsheet::Event::Apply, no_file),
            Outcome::Refuse("Type your current password first.".into())
        );
        type_into(&mut p, TAB_PASSWORD, CURRENT, "old");
        type_into(&mut p, TAB_PASSWORD, CONFIRM, "x");
        assert_eq!(
            p.event(propsheet::Event::Apply, no_file),
            Outcome::Refuse("The new passwords do not match. Type them again.".into())
        );
        let page = &mut p.sheet.pages[TAB_PASSWORD];
        wipe(page, CONFIRM);
        type_into(&mut p, TAB_PASSWORD, CONFIRM, "newpass1");
        assert_eq!(
            p.event(propsheet::Event::Ok, no_file),
            Outcome::ApplyAndClose(vec![Op::OwnPassword {
                current: "old".into(),
                new: "newpass1".into()
            }])
        );
        p.unapplied();
        assert_eq!(edit_text(&p.sheet.pages[TAB_PASSWORD], CURRENT), "");
        assert!(!p.dirty());
    }

    #[test]
    fn another_accounts_password_is_set_or_left_to_their_next_login() {
        let mut p = Props::new(user(false, false), None, false);
        assert!(p.sheet.pages[TAB_PASSWORD].item(CURRENT).is_none());
        let page = &mut p.sheet.pages[TAB_PASSWORD];
        page.focus_id(AT_LOGIN);
        let ev = page.key(form::Key::Space);
        p.event(propsheet::Event::Page(TAB_PASSWORD, ev), no_file);
        assert!(matches!(
            p.sheet.pages[TAB_PASSWORD].item(NEW).map(|i| &i.control),
            Some(Control::Edit { enabled: false, .. })
        ));
        assert_eq!(
            p.event(propsheet::Event::Apply, no_file),
            Outcome::Apply(vec![Op::PasswordAtLogin {
                path: "/u/1000".into()
            }])
        );
    }

    #[test]
    fn a_stock_pick_shows_and_applies_as_a_picture() {
        let mut p = Props::new(user(true, false), None, false);
        if let Some(Control::IconList(l)) = p.sheet.pages[TAB_PICTURE]
            .item_mut(STOCK)
            .map(|i| &mut i.control)
        {
            l.selected = Some(4);
        }
        p.event(
            propsheet::Event::Page(TAB_PICTURE, form::Event::Changed(STOCK)),
            no_file,
        );
        assert!(p.dirty());
        assert_eq!(
            p.picture,
            Some(Source::Stock(reprisekit::art::userpic::Picture::Planet))
        );
        let shown = match p.sheet.pages[TAB_GENERAL].item(FACE).map(|i| &i.control) {
            Some(Control::Picture {
                canvas: Some(c), ..
            }) => (c.width, c.height),
            _ => (0, 0),
        };
        assert_eq!(shown, (FACE_PX, FACE_PX), "General shows it too");
        assert_eq!(
            p.event(propsheet::Event::Apply, no_file),
            Outcome::Apply(vec![Op::Picture {
                path: "/u/1000".into(),
                file: "/run/user/1000/reprise-accounts/p.png".into(),
                you: true
            }])
        );
        let browse = p.sheet.pages[TAB_PICTURE].item(BROWSE).unwrap().rect;
        assert!(
            browse.bottom() < CLIENT.1 - 40,
            "Browse clears the sheet's buttons"
        );
    }
}
