//! User Accounts main window: every account in a list (yours under its own
//! heading) with picture, name and administrator status, over Add...,
//! Remove and Properties. It reads no bus; the host feeds it accounts and
//! acts on the [`Outcome`]s it returns.

use repriseapp::Ctx;
use repriseapp::oneshot;
use reprisekit::access::{self, Tree};
use reprisekit::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Form, Item};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::tilelist::{self, Entry, Press, TILE_H, Tile, TileList};
use reprisekit::look::Paint;
use reprisekit::pointer::Click;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::Align;

use crate::acct::User;

pub const CLIENT: (i32, i32) = (380, 384);

pub const ADD: &str = "add";
pub const REMOVE: &str = "remove";
pub const PROPS: &str = "props";
pub const CLOSE: &str = "close";

const MARGIN: i32 = 12;
pub const LIST: Rect = Rect::new(MARGIN, 32, CLIENT.0 - 2 * MARGIN, 264);
const ROW_Y: i32 = LIST.y + LIST.height + 8;
const CLOSE_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Close,
    Add,
    Remove(u32),
    Properties(u32),
}

pub struct Users {
    pub form: Form,
    pub list: TileList,
    pub rows: Vec<User>,
    pub pictures: Vec<Option<Canvas>>,
    pub list_focus: bool,
    pub failed: Option<String>,
    pub loaded: bool,
}

/// The list's entries: your account under one heading, the rest under another.
pub fn entries(rows: &[User]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut others = false;
    for (i, u) in rows.iter().enumerate() {
        if u.you {
            out.push(Entry::Header("Your account".into()));
        } else if !others {
            out.push(Entry::Header("Other accounts".into()));
            others = true;
        }
        out.push(Entry::Row(i));
    }
    out
}

fn window_form() -> Form {
    let gap = 6;
    Form::new(vec![
        Item::label_at(
            "intro",
            MARGIN,
            12,
            LIST.width,
            "Pick an account to change, or add one for someone new.",
        ),
        Item::button_at(ADD, MARGIN, ROW_Y, BUTTON_W, "&Add...", true),
        Item::button_at(
            REMOVE,
            MARGIN + BUTTON_W + gap,
            ROW_Y,
            BUTTON_W,
            "&Remove",
            false,
        ),
        Item::button_at(
            PROPS,
            LIST.right() - BUTTON_W,
            ROW_Y,
            BUTTON_W,
            "&Properties",
            false,
        ),
        Item::rule("rule", Rect::new(MARGIN, CLOSE_Y - 10, LIST.width, 1)),
        Item::button(
            CLOSE,
            Rect::new(LIST.right() - BUTTON_W, CLOSE_Y, BUTTON_W, BUTTON_H),
            "Close",
            true,
            true,
        ),
    ])
}

impl Users {
    pub fn new() -> Users {
        let mut u = Users {
            form: window_form(),
            list: TileList::new(TILE_H),
            rows: Vec::new(),
            pictures: Vec::new(),
            list_focus: false,
            failed: None,
            loaded: false,
        };
        u.write();
        u
    }

    /// The list's rect at the text size the form was last drawn at.
    pub fn list_rect(&self) -> Rect {
        self.form.units().rect(LIST)
    }

    pub fn selected(&self) -> Option<&User> {
        self.list.selected_row().and_then(|i| self.rows.get(i))
    }

    fn selected_uid(&self) -> Option<u32> {
        self.selected().map(|u| u.uid)
    }

    /// The accounts as read, with pictures. The selection follows its account; with none, yours is picked.
    pub fn refresh(&mut self, rows: Vec<User>, pictures: Vec<Option<Canvas>>) {
        let keep = self.selected_uid();
        self.rows = rows;
        self.pictures = pictures;
        self.list.entries = entries(&self.rows);
        let yours = self.rows.iter().position(|u| u.you);
        let pick = keep
            .and_then(|uid| self.rows.iter().position(|u| u.uid == uid))
            .or(yours);
        self.list.select_row(pick);
        self.list.clamp(self.list_rect(), self.form.units());
        self.failed = None;
        self.loaded = true;
        self.write();
    }

    pub fn fail(&mut self, why: String) {
        self.rows.clear();
        self.pictures.clear();
        self.list.entries.clear();
        self.list.select_row(None);
        self.failed = Some(why);
        self.loaded = true;
        self.write();
    }

    pub fn blank_text(&self) -> Option<&str> {
        match &self.failed {
            Some(why) => Some(why),
            None if !self.loaded => Some("Reading the accounts..."),
            None => None,
        }
    }

    fn write(&mut self) {
        let picked = self.selected().map(|u| u.you);
        let live = self.failed.is_none() && self.loaded;
        self.form.set_enabled(ADD, live);
        // Your own account cannot be deleted from inside its session.
        self.form.set_enabled(REMOVE, picked == Some(false));
        self.form.set_enabled(PROPS, picked.is_some());
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::None => Outcome::None,
            form::Event::Pressed(id) => {
                self.list_focus = false;
                match (id, self.selected()) {
                    (ADD, _) => Outcome::Add,
                    (REMOVE, Some(u)) if !u.you => Outcome::Remove(u.uid),
                    (PROPS, Some(u)) => Outcome::Properties(u.uid),
                    (CLOSE, _) => Outcome::Close,
                    _ => Outcome::Redraw,
                }
            }
            form::Event::Default if self.list_focus => match self.selected_uid() {
                Some(uid) => Outcome::Properties(uid),
                None => Outcome::None,
            },
            form::Event::Default | form::Event::Cancel => Outcome::Close,
            _ => Outcome::Redraw,
        }
    }

    /// A left press inside [`Users::list_rect`]: the list takes the keyboard and picks a row; a double click opens Properties.
    pub fn list_press(&mut self, at: Click, cm: &ControlMetrics) -> Outcome {
        if self.blank_text().is_some() {
            return Outcome::None;
        }
        self.list_focus = true;
        self.form.set_focus(None);
        match self.list.press(self.list_rect(), at, cm) {
            Press::None => Outcome::None,
            Press::Redraw => Outcome::Redraw,
            Press::Row { activate, .. } => {
                self.write();
                match (activate, self.selected_uid()) {
                    (true, Some(uid)) => Outcome::Properties(uid),
                    _ => Outcome::Redraw,
                }
            }
        }
    }

    /// A key while the list holds the keyboard; None leaves it to the form.
    pub fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Option<Outcome> {
        if self.blank_text().is_some() {
            return None;
        }
        if *key == form::Key::Enter {
            return self.selected_uid().map(Outcome::Properties);
        }
        if self.list.key(key, self.list_rect(), cm) {
            self.write();
            return Some(Outcome::Redraw);
        }
        None
    }

    pub fn delete_key(&self) -> Outcome {
        match self.selected() {
            Some(u) if !u.you => Outcome::Remove(u.uid),
            _ => Outcome::None,
        }
    }

    pub fn access(&self, tree: &mut Tree, ctx: &Ctx) {
        let px = ctx.forms().fonts.size_px;
        oneshot::access_form(&self.form, tree, &ctx.text, px);
        let root = access::ROOT;
        let list = self.list_rect();
        if let Some(line) = self.blank_text() {
            tree.push(root, "empty", access::label(list, line, true));
            return;
        }
        let lay = tilelist::layout(&self.list, list, &ctx.theme.controls());
        let row = tilelist::access(
            &self.list,
            &lay,
            |i| {
                (
                    self.rows[i].shown_name().to_string(),
                    caption(&self.rows[i]),
                )
            },
            tree,
            root,
            "accounts",
        );
        if self.list_focus {
            tree.set_focus(row);
        }
    }

    /// The window: the form, then the list over its rect, both at the text size; `face` stands
    /// in for a missing picture.
    pub fn draw(&self, c: &mut Canvas, ctx: &Ctx, face: Option<&Canvas>) {
        let cs = &ctx.theme.colors;
        let forms = ctx.forms();
        let form_paint = Paint {
            s: 1,
            ..forms.paint()
        };
        c.fill_rect(c.bounds(), cs.button_face);
        let fpx = forms.fonts.size_px;
        form::draw(c, &ctx.text, &form_paint, &self.form, !self.list_focus, fpx);
        let px = ctx.theme.fonts.size_px;
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        let cm = ctx.theme.controls();
        let list = self.list_rect();
        if let Some(line) = self.blank_text() {
            let empty = TileList::new(TILE_H);
            let lay = tilelist::layout(&empty, list, &cm);
            tilelist::draw(c, &ctx.text, &paint, &empty, &lay, px, |_, _, _, _| {});
            let u = ctx.theme.text;
            let r = Rect::new(list.x, list.y + u.len(24), list.width, u.len(LINE_H));
            ctx.text
                .draw(c, r, line, false, cs.gray_text, Align::Center, px);
            return;
        }
        let lay = tilelist::layout(&self.list, list, &cm);
        tilelist::draw(
            c,
            &ctx.text,
            &paint,
            &self.list,
            &lay,
            px,
            |c, r, i, sel| {
                let u = &self.rows[i];
                let cap = caption(u);
                let tile = Tile {
                    icon: self.pictures.get(i).and_then(Option::as_ref).or(face),
                    title: u.shown_name(),
                    tag: "",
                    caption: &cap,
                    greyed: false,
                };
                tilelist::draw_tile(c, &ctx.text, cs, r, &tile, sel, px, lay.text);
            },
        );
    }
}

pub fn caption(u: &User) -> String {
    format!("{} - {}", u.kind(), u.login)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn user(uid: u32, login: &str, admin: bool, you: bool) -> User {
        User {
            path: format!("/org/freedesktop/Accounts/User{uid}"),
            uid,
            login: login.into(),
            real_name: String::new(),
            admin,
            icon: String::new(),
            home: format!("/home/{login}"),
            you,
        }
    }

    fn three() -> Vec<User> {
        vec![
            user(1000, "jo", true, true),
            user(1001, "al", false, false),
            user(1002, "sam", true, false),
        ]
    }

    /// The window fits its text under every interface and text size.
    #[test]
    fn the_window_fits_its_text() {
        let over = reprisekit::interfaces::form_overflows(|_, _, _| {
            vec![("users".into(), Users::new().form)]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn enabled(u: &Users, id: &str) -> bool {
        matches!(
            u.form.item(id).map(|i| &i.control),
            Some(form::Control::Button { enabled: true, .. })
        )
    }

    #[test]
    fn yours_and_the_others_get_their_own_headings() {
        assert_eq!(
            entries(&three()),
            vec![
                Entry::Header("Your account".into()),
                Entry::Row(0),
                Entry::Header("Other accounts".into()),
                Entry::Row(1),
                Entry::Row(2),
            ]
        );
    }

    #[test]
    fn your_account_is_picked_first_and_cannot_be_removed() {
        let mut u = Users::new();
        assert_eq!(u.blank_text(), Some("Reading the accounts..."));
        u.refresh(three(), vec![None; 3]);
        assert_eq!(u.selected().map(|s| s.login.as_str()), Some("jo"));
        assert!(enabled(&u, PROPS) && enabled(&u, ADD));
        assert!(!enabled(&u, REMOVE));
        assert_eq!(u.delete_key(), Outcome::None);
        u.list_focus = true;
        u.list_key(&form::Key::Down, &ControlMetrics::CLASSIC);
        assert_eq!(u.selected().map(|s| s.login.as_str()), Some("al"));
        assert!(enabled(&u, REMOVE));
        assert_eq!(u.delete_key(), Outcome::Remove(1001));
        assert_eq!(
            u.event(form::Event::Pressed(PROPS)),
            Outcome::Properties(1001)
        );
    }

    #[test]
    fn the_selection_follows_its_account_across_a_refresh() {
        let mut u = Users::new();
        u.refresh(three(), vec![None; 3]);
        u.list.select_row(Some(2));
        let mut fewer = three();
        fewer.remove(1);
        u.refresh(fewer, vec![None; 2]);
        assert_eq!(u.selected().map(|s| s.uid), Some(1002));
        u.refresh(three()[..1].to_vec(), vec![None]);
        assert_eq!(u.selected().map(|s| s.uid), Some(1000));
    }

    /// At 200% the list's frame and its tiles and headings grow together: a press on the third
    /// grown tile picks it.
    #[test]
    fn at_200_percent_the_tiles_grow_with_the_list() {
        let theme = reprisekit::theme::Theme::default().with_text_size(200);
        let (cm, t) = (theme.controls(), theme.text);
        let mut u = Users::new();
        u.form.drawn_with(&theme.paint());
        u.refresh(three(), vec![None; 3]);
        let list = u.list_rect();
        assert_eq!(list, t.rect(LIST));
        let lay = tilelist::layout(&u.list, list, &cm);
        let third = lay.slots[4].rect;
        assert_eq!(third.height, t.len(TILE_H));
        assert_eq!(third.y, list.y + 2 + 2 * t.len(31) + 2 * t.len(TILE_H));
        let at = theme.metrics.click(third.x + 60, third.bottom() - 4, 0);
        u.list_press(at, &cm);
        assert_eq!(u.selected().map(|s| s.login.as_str()), Some("sam"));
    }

    #[test]
    fn a_failed_read_says_why_and_greys_the_buttons() {
        let mut u = Users::new();
        u.fail("AccountsService is not running.".into());
        assert_eq!(u.blank_text(), Some("AccountsService is not running."));
        assert!(!enabled(&u, ADD) && !enabled(&u, PROPS));
        assert_eq!(caption(&three()[0]), "Administrator - jo");
    }
}
