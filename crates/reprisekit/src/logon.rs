//! The login screen's model, hit and access. Each interface's `logon` module lays it out and
//! paints it; `interfaces::logon` picks the one in force.

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self as ax, ROOT, Tree};
use crate::interface::Interface;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{EditKey, TextEdit};

/// The login screen's strings, one accessor per Fluent id.
pub mod text {
    pub fn title() -> String {
        tr!("logon-title")
    }
    pub fn user_name() -> String {
        tr!("logon-user-name")
    }
    pub fn password() -> String {
        tr!("logon-password")
    }
    pub fn ok() -> String {
        tr!("ok")
    }
    pub fn cancel() -> String {
        tr!("cancel")
    }
    pub fn shut_down() -> String {
        tr!("logon-shut-down")
    }
    pub fn begin() -> String {
        tr!("logon-begin")
    }
    pub fn turn_off() -> String {
        tr!("powerpanel-title-turn-off")
    }
    pub fn switch_user() -> String {
        tr!("powerpanel-switch-user")
    }
    pub fn other_user() -> String {
        tr!("logon-other-user")
    }
    pub fn logged_on() -> String {
        tr!("logon-logged-on")
    }
    pub fn password_cue() -> String {
        tr!("logon-password-cue")
    }
    pub fn incorrect() -> String {
        tr!("logon-incorrect")
    }
    pub fn caps_lock() -> String {
        tr!("logon-caps-lock")
    }
    pub fn power_refused() -> String {
        tr!("logon-power-refused")
    }
    pub fn session_ended_early() -> String {
        tr!("logon-session-ended-early")
    }
    pub fn restart() -> String {
        tr!("powerpanel-restart")
    }
    pub fn power_off() -> String {
        tr!("logon-power-off")
    }
    pub fn verifying() -> String {
        tr!("logon-verifying")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogonUser {
    pub name: String,
    pub real_name: String,
    pub picture: Option<Canvas>,
    pub note: Option<UserNote>,
}

impl LogonUser {
    /// What a tile is called: the real name, else the login name.
    pub fn shown(&self) -> &str {
        if self.real_name.is_empty() {
            &self.name
        } else {
            &self.real_name
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserNote {
    LoggedOn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    /// User name and password fields.
    Typed,
    /// The users to pick from.
    Tiles,
    /// One user's tile with its password field.
    Tile(usize),
    /// Glass's error screen with its OK; Classic and Blue show the text in place.
    Message,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Name,
    Password,
    Submit,
    Cancel,
    Tile(usize),
    OtherUser,
    SwitchUser,
    Power,
    PowerMenu,
    Layout,
    MessageOk,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogonAction {
    /// Log `user` on with the password field's text, or answer the waiting prompt with it.
    Submit {
        user: String,
    },
    /// Drop the conversation in progress, if there is one.
    Cancel,
    Power,
    PowerMenu,
    NextLayout,
}

/// A question the login conversation asks after the password; `echo` shows the answer as typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
    pub echo: bool,
}

/// `Debug` redacts typed text, which may be a password.
#[derive(Clone, PartialEq, Eq)]
pub enum LogonKey<'a> {
    Edit(EditKey),
    Text(&'a str),
    Enter,
    Escape,
    Tab { back: bool },
    Up,
    Down,
}

impl std::fmt::Debug for LogonKey<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogonKey::Edit(EditKey::Insert(_)) => f.write_str("Edit(Insert(<redacted>))"),
            LogonKey::Edit(k) => f.debug_tuple("Edit").field(k).finish(),
            LogonKey::Text(_) => f.write_str("Text(<redacted>)"),
            LogonKey::Enter => f.write_str("Enter"),
            LogonKey::Escape => f.write_str("Escape"),
            LogonKey::Tab { back } => f.debug_struct("Tab").field("back", back).finish(),
            LogonKey::Up => f.write_str("Up"),
            LogonKey::Down => f.write_str("Down"),
        }
    }
}

/// How many tiles a screen shows at once unless the host says otherwise.
pub const COLUMN: usize = 5;

/// `name` holds the user the screen is about, typed or the tile's. The host reads `password.text`
/// on [`LogonAction::Submit`] and sets `caps_lock`, `layout` (the badge's short name) and `lit`.
#[derive(Clone, Debug)]
pub struct Logon {
    pub interface: Interface,
    pub users: Vec<LogonUser>,
    pub screen: Screen,
    pub name: TextEdit,
    pub password: TextEdit,
    pub prompt: Option<Prompt>,
    pub message: Option<String>,
    pub busy: bool,
    pub caps_lock: bool,
    pub layout: Option<String>,
    pub focus: Part,
    pub lit: Option<(Part, Interaction)>,
    /// The first tile in view.
    pub first_tile: usize,
    /// How many tiles are in view at once, for keeping the selection in view.
    pub column: usize,
    /// The screen [`Screen::Message`]'s OK goes back to.
    back: Screen,
}

impl Logon {
    pub fn new(
        interface: Interface,
        users: Vec<LogonUser>,
        show_users: bool,
        last_user: Option<&str>,
    ) -> Logon {
        let listed = show_users && !users.is_empty();
        let last = last_user.and_then(|n| users.iter().position(|u| u.name == n));
        let screen = match interface {
            Interface::Blue if listed => Screen::Tiles,
            Interface::Glass if listed => Screen::Tile(last.unwrap_or(0)),
            _ => Screen::Typed,
        };
        let name = match screen {
            Screen::Tile(i) => users[i].name.clone(),
            _ => last_user.unwrap_or_default().to_owned(),
        };
        let mut m = Logon {
            interface,
            users,
            screen,
            name: TextEdit::new(name, false),
            password: TextEdit::secret(),
            prompt: None,
            message: None,
            busy: false,
            caps_lock: false,
            layout: None,
            focus: Part::Name,
            lit: None,
            first_tile: 0,
            column: COLUMN,
            back: Screen::Typed,
        };
        m.focus = m.home();
        if let Part::Tile(i) = m.focus {
            m.scroll_to(i);
        }
        m
    }

    /// The user a submit logs on: the name shown, trimmed, unless it is empty or the screen
    /// only lists the users.
    pub fn user(&self) -> Option<String> {
        let name = self.name.text.trim();
        (self.screen != Screen::Tiles && !name.is_empty()).then(|| name.to_owned())
    }

    /// The tile `name` belongs to, if any.
    fn selected(&self) -> Option<usize> {
        self.users.iter().position(|u| u.name == self.name.text)
    }

    /// Where the focus starts on the current screen.
    fn home(&self) -> Part {
        match self.screen {
            Screen::Typed if self.name.text.is_empty() => Part::Name,
            Screen::Typed | Screen::Tile(_) => Part::Password,
            Screen::Tiles => Part::Tile(self.selected().unwrap_or(0)),
            Screen::Message => Part::MessageOk,
        }
    }

    /// The parts Tab visits on the current screen, in reading order.
    pub fn stops(&self) -> Vec<Part> {
        use Part::*;
        let tiles = (0..self.users.len()).map(Tile);
        let switch = (!self.users.is_empty()).then_some(SwitchUser);
        match (self.interface, self.screen) {
            (_, Screen::Message) => vec![MessageOk],
            (Interface::Classic, _) => vec![Name, Password, Submit, Cancel, Power],
            (Interface::Blue, Screen::Tiles) => tiles.chain([Power]).collect(),
            (Interface::Blue, Screen::Tile(_)) => vec![Password, Submit, Power],
            (Interface::Blue, Screen::Typed) => vec![Name, Password, Submit, Power],
            (Interface::Glass, Screen::Tiles) => tiles.chain([OtherUser, Power]).collect(),
            (Interface::Glass, Screen::Tile(_)) => [Password, Submit]
                .into_iter()
                .chain(switch)
                .chain([Power, PowerMenu])
                .collect(),
            (Interface::Glass, Screen::Typed) => [Name, Password, Submit]
                .into_iter()
                .chain(switch)
                .chain([Power, PowerMenu])
                .collect(),
        }
    }

    /// While busy only Enter on a focused Power does anything.
    pub fn key(&mut self, key: LogonKey) -> Option<LogonAction> {
        if self.busy {
            return (key == LogonKey::Enter && self.focus == Part::Power)
                .then_some(LogonAction::Power);
        }
        match key {
            LogonKey::Edit(k) => {
                if let Some(edit) = self.focused_edit() {
                    edit.key(k);
                }
                None
            }
            LogonKey::Text(s) => {
                if let Some(edit) = self.focused_edit() {
                    edit.key(EditKey::Insert(s.to_owned()));
                }
                None
            }
            LogonKey::Enter => match self.focus {
                Part::Name => {
                    self.focus = Part::Password;
                    None
                }
                part => self.press(part),
            },
            LogonKey::Escape => self.escape(),
            LogonKey::Tab { back } => {
                let stops = self.stops();
                let n = stops.len();
                self.focus = match stops.iter().position(|&p| p == self.focus) {
                    Some(at) if back => stops[(at + n - 1) % n],
                    Some(at) => stops[(at + 1) % n],
                    None => stops[0],
                };
                None
            }
            LogonKey::Up => self.step(false),
            LogonKey::Down => self.step(true),
        }
    }

    /// A click on `part`: a field takes the focus, anything else is pressed. While busy only
    /// Power answers.
    pub fn click(&mut self, part: Part) -> Option<LogonAction> {
        if self.busy && part != Part::Power {
            return None;
        }
        match part {
            Part::Name | Part::Password => {
                self.focus = part;
                None
            }
            _ => self.press(part),
        }
    }

    fn press(&mut self, part: Part) -> Option<LogonAction> {
        match part {
            Part::Name => None,
            Part::Password | Part::Submit => self.submit(),
            Part::Cancel => self.clear(),
            Part::Tile(i) if self.screen == Screen::Tile(i) => None,
            Part::Tile(i) => self.open(Screen::Tile(i)),
            Part::OtherUser => self.open(Screen::Typed),
            Part::SwitchUser => self.open(Screen::Tiles),
            Part::Power => Some(LogonAction::Power),
            Part::PowerMenu => Some(LogonAction::PowerMenu),
            Part::Layout => Some(LogonAction::NextLayout),
            Part::MessageOk => {
                self.screen = self.back;
                self.focus = self.home();
                None
            }
        }
    }

    fn submit(&mut self) -> Option<LogonAction> {
        match self.user() {
            Some(user) => Some(LogonAction::Submit { user }),
            None => {
                if self.screen == Screen::Typed {
                    self.focus = Part::Name;
                }
                None
            }
        }
    }

    fn escape(&mut self) -> Option<LogonAction> {
        let listed = !self.users.is_empty();
        match (self.interface, self.screen) {
            (_, Screen::Message) => self.press(Part::MessageOk),
            (_, Screen::Tiles) => None,
            (Interface::Blue | Interface::Glass, Screen::Tile(_)) => self.open(Screen::Tiles),
            (Interface::Glass, Screen::Typed) if listed => self.open(Screen::Tiles),
            _ => self.clear(),
        }
    }

    /// Cancel: both fields emptied, the conversation dropped and the focus on the name.
    fn clear(&mut self) -> Option<LogonAction> {
        self.forget();
        self.name = TextEdit::new("", false);
        self.focus = Part::Name;
        Some(LogonAction::Cancel)
    }

    /// Go to `screen` with an empty password and no prompt or message. A tile shows its user;
    /// the tiles select the user shown; typed starts empty. Leaving a waiting prompt cancels it.
    fn open(&mut self, screen: Screen) -> Option<LogonAction> {
        let waiting = self.prompt.is_some();
        self.forget();
        let selected = self.selected();
        self.screen = screen;
        match screen {
            Screen::Tile(i) => self.name = TextEdit::new(self.users[i].name.clone(), false),
            Screen::Typed => self.name = TextEdit::new("", false),
            _ => {}
        }
        self.focus = match screen {
            Screen::Tiles => Part::Tile(selected.unwrap_or(0)),
            _ => self.home(),
        };
        if let Part::Tile(i) = self.focus {
            self.scroll_to(i);
        }
        waiting.then_some(LogonAction::Cancel)
    }

    /// The password wiped, the prompt and message gone, the name editable again.
    fn forget(&mut self) {
        self.wipe_password();
        self.prompt = None;
        self.message = None;
        self.name.set_read_only(false);
    }

    fn wipe_password(&mut self) {
        self.password.wipe();
        if !self.password.is_secret() {
            self.password = TextEdit::secret();
        }
    }

    fn focused_edit(&mut self) -> Option<&mut TextEdit> {
        match self.focus {
            Part::Name => Some(&mut self.name),
            Part::Password => Some(&mut self.password),
            _ => None,
        }
    }

    fn step(&mut self, down: bool) -> Option<LogonAction> {
        let Part::Tile(i) = self.focus else {
            return None;
        };
        let j = if down {
            (i + 1).min(self.users.len().saturating_sub(1))
        } else {
            i.saturating_sub(1)
        };
        self.focus = Part::Tile(j);
        self.scroll_to(j);
        None
    }

    fn scroll_to(&mut self, i: usize) {
        let column = self.column.max(1);
        if i < self.first_tile {
            self.first_tile = i;
        } else if i >= self.first_tile + column {
            self.first_tile = i + 1 - column;
        }
    }

    /// The conversation asks `prompt`: the password field takes its answer, emptied, and the name
    /// stays fixed until the conversation ends.
    pub fn ask(&mut self, prompt: Prompt) {
        self.password = if prompt.echo {
            TextEdit::new("", false)
        } else {
            TextEdit::secret()
        };
        self.name.set_read_only(true);
        self.prompt = Some(prompt);
        self.busy = false;
        if self.screen == Screen::Message {
            self.screen = self.back;
        }
        self.focus = Part::Password;
    }

    /// The login failed with `text`: the password is wiped, the name stays and the focus goes home.
    /// Glass shows the text on its own screen, whose OK comes back here.
    pub fn fail(&mut self, text: String) {
        self.wipe_password();
        self.prompt = None;
        self.name.set_read_only(false);
        self.busy = false;
        self.message = Some(text);
        self.focus = self.home();
        if self.interface == Interface::Glass && self.screen != Screen::Message {
            self.back = self.screen;
            self.screen = Screen::Message;
            self.focus = Part::MessageOk;
        }
    }

    /// Text the conversation shows without asking anything.
    pub fn info(&mut self, text: String) {
        self.message = Some(text);
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    /// What a part is called to a screen reader.
    fn name_of(&self, part: Part) -> String {
        match part {
            Part::Name => text::user_name(),
            Part::Password => self
                .prompt
                .as_ref()
                .map_or_else(text::password, |p| p.text.clone()),
            Part::Submit | Part::MessageOk => text::ok(),
            Part::Cancel => text::cancel(),
            Part::Tile(i) => self
                .users
                .get(i)
                .map(|u| u.shown().to_owned())
                .unwrap_or_default(),
            Part::OtherUser => text::other_user(),
            Part::SwitchUser => text::switch_user(),
            Part::Power => match self.interface {
                Interface::Classic => text::shut_down(),
                Interface::Blue => text::turn_off(),
                Interface::Glass => text::power_off(),
            },
            Part::PowerMenu => String::new(),
            Part::Layout => self.layout.clone().unwrap_or_default(),
        }
    }
}

/// Where an interface put the screen's parts, in design px within `output`. `dialog` is the
/// dialog's frame when the parts sit in one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogonLayout {
    pub output: Rect,
    pub parts: Vec<(Part, Rect)>,
    pub dialog: Option<Rect>,
}

impl LogonLayout {
    pub fn rect(&self, part: Part) -> Option<Rect> {
        self.parts.iter().find(|(p, _)| *p == part).map(|(_, r)| *r)
    }
}

pub fn hit(l: &LogonLayout, x: i32, y: i32) -> Option<Part> {
    l.parts
        .iter()
        .find(|(_, r)| r.contains(x, y))
        .map(|(p, _)| *p)
}

/// The screen's fields and buttons under `tree`'s root, inside a dialog node when there is one,
/// with the focus on the focused part. The message, Caps Lock notice included, is a label.
pub fn access(m: &Logon, l: &LogonLayout, tree: &mut Tree) {
    let parent = match l.dialog {
        Some(r) => tree.push(ROOT, "dialog", ax::titled(Role::Dialog, r, &text::title())),
        None => ROOT,
    };
    let live = |p: Part| !m.busy || p == Part::Power;
    let mut focus: Option<NodeId> = None;
    for &(part, r) in &l.parts {
        let label = m.name_of(part);
        let node = match part {
            Part::Name | Part::Password => {
                let (role, value) = match part {
                    Part::Name => (Role::TextInput, m.name.text.clone()),
                    _ if m.password.is_secret() => {
                        (Role::PasswordInput, m.password.masked().text.clone())
                    }
                    _ => (Role::TextInput, m.password.text.clone()),
                };
                let mut n = ax::named(role, r, &label);
                ax::focusable(&mut n, live(part));
                n.set_value(value);
                if part == Part::Name && m.name.is_read_only() {
                    n.set_read_only();
                }
                n
            }
            Part::Tile(i) => {
                let mut n = ax::named(Role::ListItem, r, &label);
                n.set_selected(m.focus == part || m.screen == Screen::Tile(i));
                if m.users.get(i).and_then(|u| u.note) == Some(UserNote::LoggedOn) {
                    n.set_description(text::logged_on());
                }
                ax::focusable(&mut n, live(part));
                n
            }
            _ => ax::button(r, &label, live(part), part == Part::Submit),
        };
        let id = tree.push(parent, part, node);
        if m.focus == part {
            focus = Some(id);
        }
    }
    let notice = m.caps_lock.then(text::caps_lock);
    let said: Vec<String> = notice.into_iter().chain(m.message.clone()).collect();
    if !said.is_empty() {
        let r = l.dialog.unwrap_or(l.output);
        tree.push(parent, "message", ax::label(r, &said.join(" "), true));
    }
    if let Some(id) = focus {
        tree.set_focus(id);
    }
}

/// `msg` wrapped to `width`, cut to `room` lines, with "..." closing the last line when text was
/// cut and on any line still too wide.
pub fn message_lines(
    text: &TextRenderer,
    msg: &str,
    width: i32,
    px: u32,
    room: usize,
) -> Vec<String> {
    let mut lines = crate::text::wrap(text, msg, width, px);
    if lines.len() > room {
        lines.truncate(room);
        if let Some(last) = lines.last_mut() {
            while text.measure(&format!("{last}..."), false, px) > width && last.pop().is_some() {}
            last.truncate(last.trim_end().len());
            last.push_str("...");
        }
    }
    lines
        .iter()
        .map(|l| text.ellipsize(l, width, false, px).into_owned())
        .collect()
}

#[cfg(test)]
mod tests;
