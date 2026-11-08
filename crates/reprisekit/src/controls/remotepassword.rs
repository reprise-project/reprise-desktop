//! The Remote Desktop password section of an account's Password page: whether you have one,
//! the actions on it and, while one is greyed, why. [`section`] reads the [`Facts`], [`items`]
//! lays a [`Section`] out in a group box, and [`action`] reads a press on it.

use crate::classic::widgets::{BUTTON_H, LINE_H};
use crate::controls::form::{Event, Item};
use crate::raster::Rect;

pub const CREATE: &str = "remote_create";
pub const CHANGE: &str = "remote_change";
pub const REMOVE: &str = "remote_remove";
pub const STATUS: &str = "remote_status";
pub const NOTE: &str = "remote_note";
const GROUP: &str = "remote_group";

pub const TITLE: &str = "Remote Desktop password";

pub const HAS_ONE: &str = "You have a Remote Desktop password. Clients that use Network Level \
                           Authentication log on with it.";
pub const HAS_NONE: &str = "You have no Remote Desktop password. Clients log on with your login \
                            password over TLS.";
/// The status while you can't set one, which the note explains.
pub const HAS_ONE_SHORT: &str = "You have a Remote Desktop password.";
pub const HAS_NONE_SHORT: &str = "You have no Remote Desktop password.";
pub const REMOTE_OFF: &str = "Remote logins are turned off on this computer. An administrator \
                              can turn them on in Settings > Login Manager.";
pub const NO_MANAGER: &str =
    "Reprise's login manager isn't running. Remote Desktop passwords need it.";
pub const SILENT: &str = "Reprise's login manager isn't answering. Trying again...";
pub const ROOT: &str = "The root account can't log on remotely, so it has no Remote Desktop \
                        password.";
pub const NOT_YOURS: &str =
    "Each user sets their own Remote Desktop password. You can only change yours.";

/// Reprise's login manager, which keeps the Remote Desktop passwords.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manager {
    /// Not on this computer: the section does not show.
    Missing,
    Stopped,
    Running(Answer),
}

/// What the running login manager said of your Remote Desktop password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// No answer came; the host asks again.
    Silent,
    /// Whether you have one.
    Has(bool),
}

/// Whether you may set a Remote Desktop password, as the login manager answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    RemoteLoginsOff,
    /// You aren't in the group named, whose members may log on remotely.
    NotInGroup(String),
    Root,
}

/// What decides the section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub manager: Manager,
    /// The account shown is the one logged in.
    pub yours: bool,
    /// Read only once the login manager answered.
    pub access: Access,
    /// A create, change or remove is under way: every action greys until it ends.
    pub busy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Create,
    Change,
    Remove,
}

impl Action {
    /// The id of the action's button.
    pub fn id(self) -> &'static str {
        match self {
            Action::Create => CREATE,
            Action::Change => CHANGE,
            Action::Remove => REMOVE,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Create => "C&reate a Remote Desktop password...",
            Action::Change => "C&hange my Remote Desktop password...",
            Action::Remove => "Re&move my Remote Desktop password...",
        }
    }
}

/// What the section shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub status: Option<&'static str>,
    /// Each action shown, lit or greyed.
    pub actions: Vec<(Action, bool)>,
    pub note: Option<String>,
    pub note_greyed: bool,
}

/// The section for `f`; `None` hides it. Removing a password you have is never greyed but by a
/// change under way: the login manager lets anyone drop their own.
pub fn section(f: &Facts) -> Option<Section> {
    let shown = |status, actions, note: Option<String>| {
        Some(Section {
            status,
            actions,
            note,
            note_greyed: false,
        })
    };
    let answer = match f.manager {
        Manager::Missing => return None,
        _ if !f.yours => {
            return Some(Section {
                status: None,
                actions: Vec::new(),
                note: Some(NOT_YOURS.into()),
                note_greyed: true,
            });
        }
        Manager::Stopped => {
            return shown(None, vec![(Action::Create, false)], Some(NO_MANAGER.into()));
        }
        Manager::Running(answer) => answer,
    };
    let has = match answer {
        Answer::Silent => return shown(None, vec![(Action::Create, false)], Some(SILENT.into())),
        Answer::Has(has) => has,
    };
    let why = match &f.access {
        Access::Allowed => None,
        Access::Root => return shown(None, vec![(Action::Create, false)], Some(ROOT.into())),
        Access::RemoteLoginsOff => Some(REMOTE_OFF.to_string()),
        Access::NotInGroup(group) => Some(format!(
            "You can't set a Remote Desktop password because you aren't in the {} group. An \
             administrator can add you.",
            cut_short(group)
        )),
    };
    let set = why.is_none() && !f.busy;
    let status = match (has, why.is_some()) {
        (true, false) => HAS_ONE,
        (true, true) => HAS_ONE_SHORT,
        (false, false) => HAS_NONE,
        (false, true) => HAS_NONE_SHORT,
    };
    let actions = if has {
        vec![(Action::Change, set), (Action::Remove, !f.busy)]
    } else {
        vec![(Action::Create, set)]
    };
    shown(Some(status), actions, why)
}

/// The longest group name `groupadd` takes; a longer one from a directory is cut to it, so the
/// note keeps to its lines.
const GROUP_CHARS: usize = 32;

fn cut_short(group: &str) -> std::borrow::Cow<'_, str> {
    if group.chars().count() <= GROUP_CHARS {
        return group.into();
    }
    let kept: String = group.chars().take(GROUP_CHARS - 3).collect();
    format!("{kept}...").into()
}

/// The action a press on the section asks for.
pub fn action(ev: &Event) -> Option<Action> {
    match ev {
        Event::Pressed(CREATE) => Some(Action::Create),
        Event::Pressed(CHANGE) => Some(Action::Change),
        Event::Pressed(REMOVE) => Some(Action::Remove),
        _ => None,
    }
}

/// Inset of the group box's contents from its sides.
const INSET: i32 = 12;
/// From the group box's top to its first row, under the title.
const TOP: i32 = 20;
const BOTTOM: i32 = 10;
const ACTION_W: i32 = 260;
const STATUS_LINES: i32 = 2;
const NOTE_LINES: i32 = 3;
const GAP: i32 = 8;
const ACTION_GAP: i32 = 4;

/// `s` laid out in a group box at `(x, y)`, `w` wide, as form items, and the box's bottom.
pub fn items(s: &Section, x: i32, y: i32, w: i32) -> (Vec<Item>, i32) {
    let (ix, iw) = (x + INSET, w - 2 * INSET);
    let mut out = Vec::new();
    let mut cy = y + TOP;
    if let Some(status) = s.status {
        out.push(Item::lines(
            STATUS,
            Rect::new(ix, cy, iw, STATUS_LINES * LINE_H),
            &[status],
        ));
        cy += STATUS_LINES * LINE_H + GAP;
    }
    for (i, (a, lit)) in s.actions.iter().enumerate() {
        if i > 0 {
            cy += ACTION_GAP;
        }
        out.push(Item::button_at(a.id(), ix, cy, ACTION_W, a.label(), *lit));
        cy += BUTTON_H;
    }
    if !s.actions.is_empty() && s.note.is_some() {
        cy += GAP;
    }
    if let Some(note) = &s.note {
        let item = Item::lines(NOTE, Rect::new(ix, cy, iw, NOTE_LINES * LINE_H), &[note]);
        out.push(item.enabled(!s.note_greyed));
        cy += NOTE_LINES * LINE_H;
    }
    let bottom = cy + BOTTOM;
    out.insert(0, Item::group(GROUP, Rect::new(x, y, w, bottom - y), TITLE));
    (out, bottom)
}

#[cfg(test)]
#[path = "remotepassword_tests.rs"]
mod tests;
