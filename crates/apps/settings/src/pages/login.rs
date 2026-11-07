//! The Login Manager page: the login screen and remote logins, held in the machine settings
//! (`repriselogin::machine`) rather than in `config.toml`.

use super::{Area, GX, edit, group, spin};
use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::form::{Control, Form, Item};
use reprisekit::raster::Rect;
use repriselogin::Interface;
use repriselogin::machine::MachineConfig;

pub const LOGIN_INTERFACE: &str = "login_interface";
pub const LOGIN_SHOW_USERS: &str = "login_show_users";
pub const LOGIN_AUTOLOGIN: &str = "login_autologin";
pub const LOGIN_REMOTE: &str = "login_remote";
pub const LOGIN_PORT: &str = "login_port";
pub const LOGIN_GROUP_NOTE: &str = "login_group_note";
pub const LOGIN_SESSIONS: &str = "login_sessions";
pub const LOGIN_END_AFTER: &str = "login_end_after";
pub const LOGIN_FINGERPRINT: &str = "login_fingerprint";

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

const LABEL_W: i32 = 130;
const COMBO_X: i32 = GX + LABEL_W + 4;
const COMBO_W: i32 = 150;

/// The page before the machine settings are read; [`show_login`] fills it in.
pub fn login_manager(a: Area) -> Form {
    let interfaces: Vec<&str> = INTERFACES.iter().map(|i| i.1).collect();
    let ends: Vec<&str> = END_AFTER.iter().map(|e| e.1).collect();
    let mut items = vec![
        group(a, "login_screen_group", 44, 98, "Login Screen"),
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
        group(a, "login_remote_group", 152, 158, "Remote Logins"),
        Item::check(
            LOGIN_REMOTE,
            GX,
            174,
            "Allow &remote logins to this computer",
        ),
    ];
    items.extend(spin(
        "login_port_label",
        LOGIN_PORT,
        a.gright() - 86,
        174,
        32,
        "&Port:",
        (1, 65535),
    ));
    items.push(
        Item::new(
            LOGIN_GROUP_NOTE,
            Rect::new(GX, 194, a.gw() - 18, 2 * LINE_H),
            Control::Lines(Vec::new()),
        )
        .under_check(),
    );
    items.extend(spin(
        "login_sessions_label",
        LOGIN_SESSIONS,
        GX,
        230,
        LABEL_W,
        "Sessions at &most:",
        (1, 50),
    ));
    items.extend([
        Item::label_at(
            "login_end_after_label",
            GX,
            256,
            LABEL_W + 60,
            "End &disconnected sessions after:",
        ),
        Item::combo(
            LOGIN_END_AFTER,
            COMBO_X + 60,
            252,
            a.combo(COMBO_W - 60),
            &ends,
            ends.len(),
        ),
        Item::label_at(
            "login_fingerprint_label",
            GX,
            282,
            LABEL_W,
            "Certificate &fingerprint:",
        ),
        edit(LOGIN_FINGERPRINT, COMBO_X, 278, a.gright() - COMBO_X).read_only(),
    ]);
    let mut page = Form::new(items);
    show_login(&mut page, &MachineConfig::default(), &[], None);
    page
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

/// Put `c` on the page, offering `users` for automatic logon; `fingerprint` is the Remote
/// Desktop front's, `None` while it is not running.
pub fn show_login(page: &mut Form, c: &MachineConfig, users: &[String], fingerprint: Option<&str>) {
    let interface = INTERFACES
        .iter()
        .position(|i| i.0 == c.login.interface)
        .unwrap_or(INTERFACES.len() - 1);
    page.set_combo(LOGIN_INTERFACE, interface);
    page.set_check(LOGIN_SHOW_USERS, c.login.show_users);
    let rows = autologin_rows(c, users);
    let picked = match &c.login.autologin_user {
        Some(u) => rows
            .iter()
            .skip(1)
            .position(|r| r == u)
            .map_or(0, |i| i + 1),
        None => 0,
    };
    page.set_combo_rows(LOGIN_AUTOLOGIN, &rows, picked, DROP_ROWS);
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
            if let Some((row, _)) = combo(id)
                && let Some(e) = END_AFTER.get(row)
            {
                c.remote.end_disconnected_after_min = e.0;
            }
        }
        _ => return false,
    }
    true
}
