//! The Login Manager page: the login screen and remote logins, held in the machine settings
//! (`repriselogin::machine`) rather than in `config.toml`.

use super::{Area, GX, edit, group, spin};
use reprisekit::classic::widgets::{BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::wallpapers::{Builtin, Design};
use repriselogin::Interface;
use repriselogin::machine::{Background, MachineConfig};

pub const LOGIN_INTERFACE: &str = "login_interface";
pub const LOGIN_SHOW_USERS: &str = "login_show_users";
pub const LOGIN_ANIMATION: &str = "login_animation";
pub const LOGIN_AUTOLOGIN: &str = "login_autologin";
pub const LOGIN_REMOTE: &str = "login_remote";
pub const LOGIN_PORT: &str = "login_port";
pub const LOGIN_GROUP_NOTE: &str = "login_group_note";
pub const LOGIN_SESSIONS: &str = "login_sessions";
pub const LOGIN_END_AFTER: &str = "login_end_after";
pub const LOGIN_FINGERPRINT: &str = "login_fingerprint";
pub const LOGIN_BACKGROUND: &str = "login_background";
pub const LOGIN_BROWSE: &str = "login_browse";
pub const LOGIN_SCREENS: &str = "login_screens";
pub const LOGIN_USE_DISPLAYS: &str = "login_use_displays";
pub const LOGIN_RESET_DISPLAYS: &str = "login_reset_displays";

/// What the fingerprint field shows while the Remote Desktop front has none to give.
pub const NOT_RUNNING: &str = "Not running";

/// The interface combo's rows; `None` shows the login screen in the last one used.
const INTERFACES: [(Option<Interface>, &str); 4] = [
    (Some(Interface::Classic), "Classic"),
    (Some(Interface::Blue), "Blue"),
    (Some(Interface::Glass), "Glass"),
    (None, "The last one used"),
];

/// The first row of the automatic logon combo; the listed users follow it.
const NOBODY: &str = "Nobody";
/// The background combo's first row: the interface's own backdrop.
const DEFAULT_BACKGROUND: &str = "Default";
/// The background row of a picture already copied for the login screen.
const PICTURE: &str = "Picture";
const DROP_ROWS: usize = 8;

/// The end disconnected sessions combo's rows, in minutes; `None` never ends them.
const END_AFTER: [(Option<u32>, &str); 5] = [
    (None, "Never"),
    (Some(15), "15 minutes"),
    (Some(60), "1 hour"),
    (Some(240), "4 hours"),
    (Some(1440), "1 day"),
];

/// What greys with remote logins off, [`LOGIN_GROUP_NOTE`] too.
const REMOTE_CONTROLS: [&str; 6] = [
    "login_port_label",
    LOGIN_PORT,
    "login_sessions_label",
    LOGIN_SESSIONS,
    "login_end_after_label",
    LOGIN_END_AFTER,
];

/// How far the Remote Logins group sits below where it would without the background and
/// screen rows.
const REMOTE_DY: i32 = 112;

const LABEL_W: i32 = 130;
const COMBO_X: i32 = GX + LABEL_W + 4;
const COMBO_W: i32 = 150;

/// The page before the machine settings are read; [`show_login`] fills it in.
pub fn login_manager(a: Area) -> Form {
    let interfaces: Vec<&str> = INTERFACES.iter().map(|i| i.1).collect();
    let ends: Vec<&str> = END_AFTER.iter().map(|e| e.1).collect();
    let mut items = vec![
        group(a, "login_screen_group", 44, 98 + REMOTE_DY, "Login Screen"),
        Item::label_at("login_interface_label", GX, 70, LABEL_W, "&Interface:"),
        Item::combo(
            LOGIN_INTERFACE,
            COMBO_X,
            66,
            a.combo(COMBO_W),
            &interfaces,
            interfaces.len(),
        ),
        Item::check(LOGIN_SHOW_USERS, GX, 92, "&Show users"),
        Item::check(LOGIN_ANIMATION, COMBO_X, 92, "A&nimate the screen"),
        Item::label_at(
            "login_autologin_label",
            GX,
            116,
            LABEL_W,
            "Log on a&utomatically as:",
        ),
        Item::combo(
            LOGIN_AUTOLOGIN,
            COMBO_X,
            112,
            a.combo(COMBO_W),
            &[NOBODY],
            1,
        ),
        Item::label_at("login_background_label", GX, 146, LABEL_W, "&Background:"),
        Item::combo(
            LOGIN_BACKGROUND,
            COMBO_X,
            142,
            a.combo(COMBO_W),
            &[DEFAULT_BACKGROUND],
            1,
        ),
        Item::button_at(
            LOGIN_BROWSE,
            a.gright() - BUTTON_W,
            168,
            BUTTON_W,
            "Bro&wse...",
            true,
        ),
        Item::label_at(LOGIN_SCREENS, GX, 202, a.gw() - 18, ""),
        Item::button_at(
            LOGIN_USE_DISPLAYS,
            GX,
            220,
            170,
            "Use M&y Display Settings",
            true,
        ),
        Item::button_at(
            LOGIN_RESET_DISPLAYS,
            GX + 176,
            220,
            BUTTON_W,
            "R&eset",
            false,
        ),
        group(
            a,
            "login_remote_group",
            152 + REMOTE_DY,
            158,
            "Remote Logins",
        ),
        Item::check(
            LOGIN_REMOTE,
            GX,
            174 + REMOTE_DY,
            "Allow &remote logins to this computer",
        ),
    ];
    items.extend(spin(
        "login_port_label",
        LOGIN_PORT,
        a.gright() - 86,
        174 + REMOTE_DY,
        32,
        "&Port:",
        (1, 65535),
    ));
    items.push(
        Item::new(
            LOGIN_GROUP_NOTE,
            Rect::new(GX, 194 + REMOTE_DY, a.gw() - 18, 2 * LINE_H),
            Control::Lines(Vec::new()),
        )
        .under_check(),
    );
    items.extend(spin(
        "login_sessions_label",
        LOGIN_SESSIONS,
        GX,
        230 + REMOTE_DY,
        LABEL_W,
        "Sessions at &most:",
        (1, 50),
    ));
    items.extend([
        Item::label_at(
            "login_end_after_label",
            GX,
            256 + REMOTE_DY,
            LABEL_W + 60,
            "End &disconnected sessions after:",
        ),
        Item::combo(
            LOGIN_END_AFTER,
            COMBO_X + 60,
            252 + REMOTE_DY,
            a.combo(COMBO_W - 60),
            &ends,
            ends.len(),
        ),
        Item::label_at(
            "login_fingerprint_label",
            GX,
            282 + REMOTE_DY,
            LABEL_W,
            "Certificate &fingerprint:",
        ),
        edit(
            LOGIN_FINGERPRINT,
            COMBO_X,
            278 + REMOTE_DY,
            a.gright() - COMBO_X,
        )
        .read_only(),
    ]);
    let mut page = Form::new(items);
    show_login(
        &mut page,
        &MachineConfig::default(),
        &[],
        None,
        Picture::default(),
    );
    page
}

/// The background combo's rows: the interface's own, each built-in by name, then a built-in
/// `c` names that they lack, and the picture `picked` names, else the one `c` or the file shows.
fn background_rows(c: &MachineConfig, picture: Picture<'_>) -> Vec<(Option<Background>, String)> {
    let mut rows = vec![(None, DEFAULT_BACKGROUND.to_string())];
    rows.extend(Design::ALL.iter().flat_map(|&d| {
        (d.variants().iter()).filter_map(move |v| {
            let b = Builtin::named(d, v)?;
            Some((
                Some(Background::Builtin(b.uri())),
                crate::backgroundpage::builtin_label(b),
            ))
        })
    }));
    if let Some(b @ Background::Builtin(uri)) = &c.login.background
        && !rows.iter().any(|r| r.0.as_ref() == Some(b))
    {
        rows.push((Some(b.clone()), uri.clone()));
    }
    let shows = picture.saved || c.login.background == Some(Background::Picture);
    match picture.picked {
        Some(name) => rows.push((Some(Background::Picture), name.to_string())),
        None if shows => rows.push((Some(Background::Picture), PICTURE.to_string())),
        None => {}
    }
    rows
}

/// The pictures the background combo offers beside the built-ins.
#[derive(Clone, Copy, Default)]
pub struct Picture<'a> {
    /// A picture browsed and not saved yet, by name.
    pub picked: Option<&'a str>,
    /// The file shows the picture already copied for the login screen.
    pub saved: bool,
}

/// What the Screens line says of `c`'s outputs.
fn screens_note(c: &MachineConfig) -> String {
    match c.outputs.len() {
        0 => "Screens: automatic".into(),
        1 => "Screens: 1, from your display settings".into(),
        n => format!("Screens: {n}, from your display settings"),
    }
}

/// The automatic logon combo's users: `users`, then the one `c` names when they lack it.
fn autologin_rows(c: &MachineConfig, users: &[String]) -> Vec<String> {
    let mut rows = vec![NOBODY.to_string()];
    rows.extend(users.iter().cloned());
    if let Some(u) = &c.login.autologin_user
        && !users.contains(u)
    {
        rows.push(u.clone());
    }
    rows
}

/// The end disconnected sessions rows, with a row for a time `c` holds that they lack.
fn end_rows(c: &MachineConfig) -> Vec<(Option<u32>, String)> {
    let mut rows: Vec<(Option<u32>, String)> =
        END_AFTER.iter().map(|&(m, l)| (m, l.to_string())).collect();
    let min = c.remote.end_disconnected_after_min;
    if !rows.iter().any(|r| r.0 == min)
        && let Some(n) = min
    {
        rows.push((min, format!("{n} minutes")));
    }
    rows
}

fn set_updown(page: &mut Form, id: &str, v: u32) {
    if let Some(Control::UpDown(u)) = page.item_mut(id).map(|i| &mut i.control) {
        u.set(v as i32);
    }
}

/// Put `c` on the page, offering `users` for automatic logon and `picture` as a background;
/// `fingerprint` is the Remote Desktop front's, `None` while it is not running.
pub fn show_login(
    page: &mut Form,
    c: &MachineConfig,
    users: &[String],
    fingerprint: Option<&str>,
    picture: Picture<'_>,
) {
    let interface = INTERFACES
        .iter()
        .position(|i| i.0 == c.login.interface)
        .unwrap_or(INTERFACES.len() - 1);
    page.set_combo(LOGIN_INTERFACE, interface);
    page.set_check(LOGIN_SHOW_USERS, c.login.show_users);
    page.set_check(LOGIN_ANIMATION, c.login.animation);
    let rows = autologin_rows(c, users);
    let picked_user = match &c.login.autologin_user {
        Some(u) => rows
            .iter()
            .skip(1)
            .position(|r| r == u)
            .map_or(0, |i| i + 1),
        None => 0,
    };
    page.set_combo_rows(LOGIN_AUTOLOGIN, &rows, picked_user, DROP_ROWS);
    let backgrounds = background_rows(c, picture);
    let shown = (backgrounds.iter())
        .position(|r| r.0 == c.login.background)
        .unwrap_or(0);
    let labels: Vec<String> = backgrounds.into_iter().map(|r| r.1).collect();
    page.set_combo_rows(LOGIN_BACKGROUND, &labels, shown, DROP_ROWS);
    page.set_text(LOGIN_SCREENS, &screens_note(c));
    page.set_enabled(LOGIN_RESET_DISPLAYS, !c.outputs.is_empty());
    let r = &c.remote;
    page.set_check(LOGIN_REMOTE, r.enabled);
    set_updown(page, LOGIN_PORT, r.port.into());
    page.set_lines(
        LOGIN_GROUP_NOTE,
        &[format!(
            "Members of the group {} can log on remotely.",
            r.group
        )],
    );
    set_updown(page, LOGIN_SESSIONS, r.max_sessions);
    let ends = end_rows(c);
    let end = ends
        .iter()
        .position(|e| e.0 == r.end_disconnected_after_min)
        .unwrap_or(0);
    let labels: Vec<String> = ends.into_iter().map(|e| e.1).collect();
    page.set_combo_rows(LOGIN_END_AFTER, &labels, end, DROP_ROWS);
    page.show_edit_text(LOGIN_FINGERPRINT, fingerprint.unwrap_or(NOT_RUNNING));
    follow_login(page);
}

/// Grey the remote login controls unless the box is ticked.
pub fn follow_login(page: &mut Form) {
    let on = page.checked(LOGIN_REMOTE) == Some(true);
    for id in REMOTE_CONTROLS {
        page.set_enabled(id, on);
    }
    if let Some(it) = page.item_mut(LOGIN_GROUP_NOTE) {
        let blank = Item::new(LOGIN_GROUP_NOTE, it.rect, Control::Blank);
        *it = std::mem::replace(it, blank).enabled(on);
    }
}

/// Read the background combo back into `c`, which the page shows with `picture`.
pub fn read_login_background(page: &Form, c: &mut MachineConfig, picture: Picture<'_>) {
    let rows = background_rows(c, picture);
    if let Some(row) = page.combo_selected(LOGIN_BACKGROUND)
        && let Some((b, _)) = rows.into_iter().nth(row)
    {
        c.login.background = b;
    }
}

/// Read control `id` back into `c`, which the page shows; false when `id` is none of the page's
/// settings. Only the control that moved is read, so a value no row stands for is kept.
pub fn read_login(page: &Form, id: &str, c: &mut MachineConfig) -> bool {
    let combo = |id| match page.item(id).map(|i| &i.control) {
        Some(Control::Combo(c)) => Some((c.selected, c.items.get(c.selected)?.text.clone())),
        _ => None,
    };
    let updown = |id| match page.item(id).map(|i| &i.control) {
        Some(Control::UpDown(u)) => Some(u.value()),
        _ => None,
    };
    match id {
        LOGIN_INTERFACE => {
            if let Some((row, _)) = combo(id)
                && let Some(i) = INTERFACES.get(row)
            {
                c.login.interface = i.0;
            }
        }
        LOGIN_SHOW_USERS => c.login.show_users = page.checked(id) == Some(true),
        LOGIN_ANIMATION => c.login.animation = page.checked(id) == Some(true),
        LOGIN_AUTOLOGIN => {
            if let Some((row, name)) = combo(id) {
                c.login.autologin_user = (row > 0).then_some(name);
            }
        }
        LOGIN_REMOTE => c.remote.enabled = page.checked(id) == Some(true),
        LOGIN_PORT => {
            if let Some(port) = updown(id).and_then(|v| u16::try_from(v).ok()) {
                c.remote.port = port;
            }
        }
        LOGIN_SESSIONS => {
            if let Some(n) = updown(id).and_then(|v| u32::try_from(v).ok()) {
                c.remote.max_sessions = n;
            }
        }
        LOGIN_END_AFTER => {
            if let Some((row, text)) = combo(id) {
                let minutes = match END_AFTER.get(row) {
                    Some(e) => Ok(e.0),
                    None => text.trim_end_matches(" minutes").parse().map(Some),
                };
                if let Ok(m) = minutes {
                    c.remote.end_disconnected_after_min = m;
                }
            }
        }
        _ => return false,
    }
    true
}
