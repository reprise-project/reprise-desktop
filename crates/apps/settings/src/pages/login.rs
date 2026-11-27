//! The Login Manager page: the login screen and remote logins, held in the machine settings
//! (`repriselogin::machine`) rather than in `config.toml`.

use super::{Area, GX, combo_of, edit, group, spin};
use crate::model::Label;
use reprisekit::classic::widgets::{BUTTON_W, LINE_H};
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;
use reprisekit::tr;
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
pub const LOGIN_THUMBPRINT: &str = "login_thumbprint";
pub const LOGIN_BACKGROUND: &str = "login_background";
pub const LOGIN_BROWSE: &str = "login_browse";
pub const LOGIN_SCREENS: &str = "login_screens";
pub const LOGIN_USE_DISPLAYS: &str = "login_use_displays";
pub const LOGIN_RESET_DISPLAYS: &str = "login_reset_displays";

/// What the fingerprint field shows while the Remote Desktop front has none to give.
pub fn not_running() -> String {
    tr!("pages-login-not-running")
}

/// A certificate's SHA-256 fingerprint and SHA-1 thumbprint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Prints {
    pub fingerprint: String,
    pub thumbprint: String,
}

impl Prints {
    /// The front's file: the fingerprint's line, then the thumbprint's, which a front that
    /// writes only the fingerprint leaves empty. None for a file without a fingerprint.
    pub fn parse(text: &str) -> Option<Prints> {
        let mut lines = text.lines().map(str::trim);
        let fingerprint = lines.next().filter(|l| !l.is_empty())?.to_string();
        let thumbprint = lines.next().unwrap_or_default().to_string();
        Some(Prints {
            fingerprint,
            thumbprint,
        })
    }

    /// What the fields show while the front has no certificate to give.
    pub fn not_running() -> Prints {
        Prints {
            fingerprint: not_running().into(),
            thumbprint: String::new(),
        }
    }
}

/// The interface combo's rows; `None` shows the login screen in the last one used.
const INTERFACES: [(Option<Interface>, Label); 4] = [
    (Some(Interface::Classic), || tr!("pages-login-classic")),
    (Some(Interface::Blue), || tr!("pages-login-blue")),
    (Some(Interface::Glass), || tr!("pages-login-glass")),
    (None, || tr!("pages-login-the-last-one-used")),
];

/// The first row of the automatic logon combo; the listed users follow it.
fn nobody() -> String {
    tr!("pages-login-nobody")
}
/// The background combo's first row: the interface's own backdrop.
fn default_background() -> String {
    tr!("pages-login-default")
}
/// The background row of a picture already copied for the login screen.
fn picture_row() -> String {
    tr!("pages-login-picture")
}
const DROP_ROWS: usize = 8;

/// The end disconnected sessions combo's rows, in minutes; `None` never ends them.
const END_AFTER: [(Option<u32>, Label); 5] = [
    (None, || tr!("pages-login-never")),
    (Some(15), || tr!("pages-login-15-minutes")),
    (Some(60), || tr!("pages-login-1-hour")),
    (Some(240), || tr!("pages-login-4-hours")),
    (Some(1440), || tr!("pages-login-1-day")),
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
    let mut items = vec![
        group(
            a,
            "login_screen_group",
            44,
            98 + REMOTE_DY,
            &tr!("pages-login-login-screen"),
        ),
        Item::label_at(
            "login_interface_label",
            GX,
            70,
            LABEL_W,
            &tr!("pages-login-interface"),
        ),
        combo_of(LOGIN_INTERFACE, COMBO_X, 66, a.combo(COMBO_W), &INTERFACES),
        Item::check(LOGIN_SHOW_USERS, GX, 92, &tr!("pages-login-show-users")),
        Item::check(
            LOGIN_ANIMATION,
            COMBO_X,
            92,
            &tr!("pages-login-animate-the-screen"),
        ),
        Item::label_at(
            "login_autologin_label",
            GX,
            116,
            LABEL_W,
            &tr!("pages-login-log-on-automatically-as"),
        ),
        Item::combo(
            LOGIN_AUTOLOGIN,
            COMBO_X,
            112,
            a.combo(COMBO_W),
            &[&nobody()],
            1,
        ),
        Item::label_at(
            "login_background_label",
            GX,
            146,
            LABEL_W,
            &tr!("pages-login-background"),
        ),
        Item::combo(
            LOGIN_BACKGROUND,
            COMBO_X,
            142,
            a.combo(COMBO_W),
            &[&default_background()],
            1,
        ),
        Item::button_at(
            LOGIN_BROWSE,
            a.gright() - BUTTON_W,
            168,
            BUTTON_W,
            &tr!("pages-login-browse"),
            true,
        ),
        Item::label_at(LOGIN_SCREENS, GX, 202, a.gw() - 18, ""),
        Item::button_at(
            LOGIN_USE_DISPLAYS,
            GX,
            220,
            170,
            &tr!("pages-login-use-my-display-settings"),
            true,
        ),
        Item::button_at(
            LOGIN_RESET_DISPLAYS,
            GX + 176,
            220,
            BUTTON_W,
            &tr!("pages-login-reset"),
            false,
        ),
        group(
            a,
            "login_remote_group",
            152 + REMOTE_DY,
            182,
            &tr!("pages-login-remote-logins"),
        ),
        Item::check(
            LOGIN_REMOTE,
            GX,
            174 + REMOTE_DY,
            &tr!("pages-login-allow-remote-logins-to-this-computer"),
        ),
    ];
    items.extend(spin(
        "login_port_label",
        LOGIN_PORT,
        a.gright() - 86,
        174 + REMOTE_DY,
        32,
        &tr!("pages-login-port"),
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
        &tr!("pages-login-sessions-at-most"),
        (1, 50),
    ));
    items.extend([
        Item::label_at(
            "login_end_after_label",
            GX,
            256 + REMOTE_DY,
            LABEL_W + 60,
            &tr!("pages-login-end-disconnected-sessions-after"),
        ),
        combo_of(
            LOGIN_END_AFTER,
            COMBO_X + 60,
            252 + REMOTE_DY,
            a.combo(COMBO_W - 60),
            &END_AFTER,
        ),
        Item::label_at(
            "login_fingerprint_label",
            GX,
            282 + REMOTE_DY,
            LABEL_W,
            &tr!("pages-login-certificate-fingerprint"),
        ),
        edit(
            LOGIN_FINGERPRINT,
            COMBO_X,
            278 + REMOTE_DY,
            a.gright() - COMBO_X,
        )
        .read_only(),
        Item::label_at(
            "login_thumbprint_label",
            GX,
            306 + REMOTE_DY,
            LABEL_W,
            &tr!("pages-login-thumbprint-sha-1"),
        ),
        edit(
            LOGIN_THUMBPRINT,
            COMBO_X,
            302 + REMOTE_DY,
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
    let mut rows = vec![(None, default_background().to_string())];
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
        None if shows => rows.push((Some(Background::Picture), picture_row())),
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
        0 => tr!("pages-login-screens-automatic"),
        1 => tr!("pages-login-screens-1-from-your-display-settings"),
        n => format!("Screens: {n}, from your display settings"),
    }
}

/// The automatic logon combo's users: `users`, then the one `c` names when they lack it.
fn autologin_rows(c: &MachineConfig, users: &[String]) -> Vec<String> {
    let mut rows = vec![nobody()];
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
    let mut rows: Vec<(Option<u32>, String)> = END_AFTER.iter().map(|&(m, l)| (m, l())).collect();
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
/// `prints` are the Remote Desktop front's certificate's, `None` while it is not running.
pub fn show_login(
    page: &mut Form,
    c: &MachineConfig,
    users: &[String],
    prints: Option<&Prints>,
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
    let prints = prints.cloned().unwrap_or_else(Prints::not_running);
    page.show_edit_text(LOGIN_FINGERPRINT, &prints.fingerprint);
    page.show_edit_text(LOGIN_THUMBPRINT, &prints.thumbprint);
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
                    None => text
                        .trim_end_matches(&tr!("pages-login-minutes"))
                        .parse()
                        .map(Some),
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
