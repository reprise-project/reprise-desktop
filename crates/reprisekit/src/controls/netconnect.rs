//! Connect to Server and the network password box, whose Remember greys without a keyring. The
//! host wraps the messages to [`MSG_W`] and fills the status line.

use std::cell::Cell;

use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::textedit::TextEdit;

pub const ADDRESS: &str = "address";
pub const USER: &str = "user";
pub const DOMAIN: &str = "domain";
pub const PASSWORD: &str = "password";
pub const REMEMBER: &str = "remember";
pub const STATUS: &str = "status";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";

pub const CONNECT_TITLE: &str = "Connect to Server";
pub const LOGIN_TITLE: &str = "Connect As";
pub const CONNECT_MESSAGE: &str = "Type the address of a shared folder or a server, for example smb://server/share or sftp://user@server/folder.";
pub const WRONG: &str = "The user name or password is not correct.";
pub const CONNECT_ICON: &str = "network-server";
pub const LOGIN_ICON: &str = "dialog-password";

const WIDTH: i32 = 378;
const ICON: Rect = Rect::new(10, 12, 32, 32);
const MSG_X: i32 = 52;
const MSG_Y: i32 = 10;
pub const MSG_W: i32 = WIDTH - MSG_X - 16;
/// The most message lines either box shows.
pub const MSG_LINES: usize = 3;
const LABEL_X: i32 = 29;
const FIELD_X: i32 = 134;
const FIELD_W: i32 = WIDTH - FIELD_X - 37;
const FIELD_H: i32 = 20;
const LABEL_DY: i32 = 3;
const FIRST_Y: i32 = 60;
const ROW: i32 = 26;
const GAP: i32 = 6;
const BOTTOM: i32 = 12;

pub const CONNECT_CLIENT: (i32, i32) = (WIDTH, 150);

/// The password box's Remember check box and status line, from its field rows down.
fn login_rows(domain: bool) -> (i32, i32) {
    let fields = if domain { 3 } else { 2 };
    let check = FIRST_Y + ROW * fields + 4;
    (check, check + 22)
}

/// The password box's size: a row taller when it asks for a domain.
pub fn login_client(domain: bool) -> (i32, i32) {
    let (_, status) = login_rows(domain);
    (WIDTH, status + LINE_H + 10 + BUTTON_H + BOTTOM)
}

fn message(lines: &[String]) -> Item {
    let refs: Vec<&str> = lines.iter().take(MSG_LINES).map(String::as_str).collect();
    Item::lines(
        "message",
        Rect::new(MSG_X, MSG_Y, MSG_W, LINE_H * MSG_LINES as i32),
        &refs,
    )
}

fn field(label_id: &'static str, label: &str, edit: Item, y: i32) -> [Item; 2] {
    [
        Item::label(
            label_id,
            Rect::new(LABEL_X, y + LABEL_DY, FIELD_X - LABEL_X - 4, LINE_H),
            label,
        ),
        Item {
            rect: Rect::new(FIELD_X, y, FIELD_W, FIELD_H),
            ..edit
        },
    ]
}

fn buttons(ok: &str, (w, h): (i32, i32)) -> [Item; 2] {
    let y = h - BOTTOM - BUTTON_H;
    let cancel_x = w - BOTTOM - BUTTON_W;
    let ok_x = cancel_x - GAP - BUTTON_W;
    [
        Item::button(OK, Rect::new(ok_x, y, BUTTON_W, BUTTON_H), ok, false, true),
        Item::button(
            CANCEL,
            Rect::new(cancel_x, y, BUTTON_W, BUTTON_H),
            "Cancel",
            true,
            false,
        ),
    ]
}

/// Connect to Server with `address` typed, its message wrapped into `lines`. Connect greys
/// while the address is empty ([`follow`]).
pub fn connect_form(lines: &[String], address: &str, icon: Option<Canvas>) -> Form {
    let mut items = vec![Item::picture("icon", ICON, icon), message(lines)];
    items.extend(field(
        "address_l",
        "&Address:",
        Item::edit(ADDRESS, Rect::default(), address, true),
        FIRST_Y + 8,
    ));
    items.push(Item::label(
        STATUS,
        Rect::new(LABEL_X, FIRST_Y + 8 + ROW + 4, WIDTH - LABEL_X - 16, LINE_H),
        "",
    ));
    items.extend(buttons("C&onnect", CONNECT_CLIENT));
    let mut form = Form::new(items);
    form.focus_id(ADDRESS);
    follow(&mut form);
    form
}

pub struct Login<'a> {
    /// The server's message, wrapped to [`MSG_W`].
    pub lines: &'a [String],
    pub user: &'a str,
    /// The user name can be changed.
    pub asks_user: bool,
    /// Asks for a domain, offering this one.
    pub domain: Option<&'a str>,
    /// A keyring is there to remember the password.
    pub can_remember: bool,
}

/// The password box for `l`, focus on the first field to fill.
pub fn login_form(l: &Login, icon: Option<Canvas>) -> Form {
    let size = login_client(l.domain.is_some());
    let mut items = vec![Item::picture("icon", ICON, icon), message(l.lines)];
    let mut y = FIRST_Y;
    items.extend(field(
        "user_l",
        "&User name:",
        Item::edit(USER, Rect::default(), l.user, l.asks_user),
        y,
    ));
    if let Some(domain) = l.domain {
        y += ROW;
        items.extend(field(
            "domain_l",
            "&Domain:",
            Item::edit(DOMAIN, Rect::default(), domain, true),
            y,
        ));
    }
    y += ROW;
    let secret = Item::new(
        PASSWORD,
        Rect::default(),
        Control::Edit {
            edit: TextEdit::secret(),
            first: Cell::new(0),
            enabled: true,
        },
    );
    items.extend(field("password_l", "&Password:", secret, y));
    let (check_y, status_y) = login_rows(l.domain.is_some());
    let remember = match l.can_remember {
        true => "&Remember this password",
        false => "&Remember this password (needs a keyring)",
    };
    items.push(Item::check(REMEMBER, LABEL_X, check_y, remember).enabled(l.can_remember));
    items.push(Item::label(
        STATUS,
        Rect::new(LABEL_X, status_y, WIDTH - LABEL_X - 16, LINE_H),
        "",
    ));
    items.extend(buttons("OK", size));
    let mut form = Form::new(items);
    form.focus_id(if l.asks_user && l.user.is_empty() {
        USER
    } else {
        PASSWORD
    });
    follow(&mut form);
    form
}

/// Greys OK while the address, or the user name the box asks for, is empty.
pub fn follow(form: &mut Form) {
    let ready = match form.item(ADDRESS) {
        Some(_) => !form.edit_text(ADDRESS).trim().is_empty(),
        None => !form.edit_text(USER).is_empty() || !enabled(form, USER),
    };
    let focused = form.focus() == form.index_of(OK);
    form.set_enabled(OK, ready);
    if focused && !ready {
        form.focus_id(if form.item(ADDRESS).is_some() {
            ADDRESS
        } else {
            USER
        });
    }
}

pub fn enabled(form: &Form, id: &str) -> bool {
    match form.item(id).map(|i| &i.control) {
        Some(
            Control::Edit { enabled, .. }
            | Control::Button { enabled, .. }
            | Control::Check { enabled, .. }
            | Control::Label { enabled, .. },
        ) => *enabled,
        _ => false,
    }
}

/// While connecting, everything but Cancel greys and the status line says so.
pub fn set_busy(form: &mut Form, busy: bool, status: &str) {
    for id in [
        ADDRESS,
        USER,
        DOMAIN,
        PASSWORD,
        REMEMBER,
        "address_l",
        "user_l",
        "domain_l",
    ] {
        if form.item(id).is_some() {
            form.set_enabled(id, !busy);
        }
    }
    form.set_text(STATUS, status);
    if busy {
        form.set_enabled(OK, false);
        form.focus_id(CANCEL);
    } else {
        follow(form);
        let first = [ADDRESS, PASSWORD]
            .into_iter()
            .find(|id| form.item(id).is_some());
        if let Some(id) = first {
            form.focus_id(id);
        }
    }
}

pub fn wipe(form: &mut Form) {
    if let Some(Control::Edit { edit, .. }) = form.item_mut(PASSWORD).map(|i| &mut i.control) {
        edit.wipe();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::text::TextRenderer;
    use crate::theme::Theme;

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    fn smb() -> Form {
        let msg = lines(
            "Authentication Required\nEnter user and password for share \u{201c}private\u{201d} on \u{201c}nas\u{201d}:",
        );
        login_form(
            &Login {
                lines: &msg,
                user: "alice",
                asks_user: true,
                domain: Some(""),
                can_remember: true,
            },
            None,
        )
    }

    fn sftp() -> Form {
        let msg = lines("Enter password for \u{201c}bob\u{201d} on \u{201c}box\u{201d}:");
        login_form(
            &Login {
                lines: &msg,
                user: "bob",
                asks_user: false,
                domain: None,
                can_remember: false,
            },
            None,
        )
    }

    fn connect() -> Form {
        connect_form(
            &lines(
                "Type the address of a shared folder or a server, for example\nsmb://server/share or sftp://user@server/folder.",
            ),
            "",
            None,
        )
    }

    #[test]
    fn connect_greys_until_an_address_is_typed() {
        let mut f = connect();
        assert!(!enabled(&f, OK));
        f.show_edit_text(ADDRESS, "smb://nas/public");
        follow(&mut f);
        assert!(enabled(&f, OK));
        set_busy(&mut f, true, "Connecting to nas...");
        assert!(!enabled(&f, OK) && !enabled(&f, ADDRESS) && enabled(&f, CANCEL));
        set_busy(&mut f, false, "Could not connect.");
        assert!(enabled(&f, OK) && enabled(&f, ADDRESS));
    }

    #[test]
    fn the_password_box_asks_for_a_domain_only_when_told_and_starts_on_the_empty_field() {
        let f = smb();
        assert!(f.item(DOMAIN).is_some());
        assert_eq!(f.focus(), f.index_of(PASSWORD), "the user name is offered");
        assert!(enabled(&f, REMEMBER));
        let f = sftp();
        assert!(f.item(DOMAIN).is_none());
        assert!(!enabled(&f, USER), "the address names the user");
        assert!(enabled(&f, OK), "no user name to wait for");
        assert!(!enabled(&f, REMEMBER), "no keyring");
        assert_eq!(login_client(true).1 - login_client(false).1, ROW);
    }

    #[test]
    fn the_boxes_fit_their_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, _, _| {
            let mut retry = smb();
            retry.set_text(STATUS, WRONG);
            vec![
                ("connect".into(), connect()),
                ("smb".into(), retry),
                ("sftp".into(), sftp()),
            ]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    fn looks() -> Vec<(String, Theme)> {
        let mut out = vec![
            ("Classic".to_string(), Theme::default()),
            (
                "Blue".to_string(),
                interfaces::blue_theme(crate::blue::Variant::ALL[0]),
            ),
        ];
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let g = crate::glass::Settings {
                mode,
                ..Default::default()
            };
            let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
            out.push((format!("Glass {mode:?}"), t));
        }
        out
    }

    fn drawn(theme: &Theme, f: &Form, (w, h): (i32, i32), s: i32) -> Canvas {
        let theme = theme.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
        c.fill_rect(c.bounds(), p.cs.button_face);
        crate::controls::form::draw(&mut c, &text, &p, f, true, theme.fonts.size_px);
        c
    }

    #[test]
    fn network_boxes_pin_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut busy = connect();
        busy.show_edit_text(ADDRESS, "smb://nas/public");
        set_busy(&mut busy, true, "Connecting to nas...");
        let mut retry = smb();
        retry.set_text(STATUS, WRONG);
        let states = [
            ("connect", connect(), CONNECT_CLIENT),
            ("connecting", busy, CONNECT_CLIENT),
            ("smb retry", retry, login_client(true)),
            ("sftp no keyring", sftp(), login_client(false)),
        ];
        let mut got = Vec::new();
        for (name, theme) in looks() {
            for (state, f, size) in &states {
                for s in [1, 2] {
                    got.push((
                        format!("{name} {state} {s}"),
                        fnv(&drawn(&theme, f, *size, s)),
                    ));
                }
            }
        }
        crate::pins::check("network_boxes", &got);
    }
}
