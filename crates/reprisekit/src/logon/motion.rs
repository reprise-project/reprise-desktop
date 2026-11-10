//! The screen's changes as the host animates them: what changed and what the screen showed
//! before. No clock: the host times each change.

use super::{Logon, Part, Screen};
use crate::interface::Interface;
use crate::textedit::{EditKey, TextEdit};

/// How much the screen may move. The host sets it from the interface, `[effects]
/// logon_animation` and whether the screen is seen from another computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Moves {
    Full,
    #[default]
    Off,
}

/// What changed the screen. `Pick` and `Back` name the picture frame that moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// From the tiles to one user's screen.
    Pick(Part),
    /// From one user's screen back to the tiles.
    Back(Part),
    /// The screen came forward: a later greeting, the lock engaging.
    Enter,
    Busy,
    /// The busy text changed: the session is starting, or the computer going down.
    Status,
    Fail,
    Ask,
    MessageOk,
    Admin,
    /// A check in progress ended without an answer.
    Idle,
}

/// The screen as it was when a change began, its typed text left out: a secret field holds as
/// many stand-ins as it had characters, an echoed answer none.
#[derive(Clone, Debug)]
pub struct Was(Box<Logon>);

/// The stand-in a secret field of a [`Was`] holds per character; drawn masked like any other.
const STAND_IN: char = '*';

impl Was {
    pub fn of(m: &Logon) -> Was {
        let mut was = m.clone();
        was.motion = None;
        let len = m.password.text.chars().count();
        was.password = TextEdit::secret();
        if m.password.is_secret() && len > 0 {
            let filler: String = std::iter::repeat_n(STAND_IN, len).collect();
            was.password.key(EditKey::Insert(filler));
        }
        Was(Box::new(was))
    }

    pub fn model(&self) -> &Logon {
        &self.0
    }
}

/// A change the host has not taken yet, with what the screen showed before it.
#[derive(Clone, Debug)]
pub struct Motion {
    pub change: Change,
    pub was: Was,
}

/// A stretch of a change, ms from its start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub from: u32,
    pub len: u32,
}

impl Window {
    pub const fn new(from: u32, len: u32) -> Window {
        Window { from, len }
    }

    pub fn end(self) -> u32 {
        self.from + self.len
    }

    /// How far through the window `at` ms into the change is, 0 to 1.
    pub fn at(self, at: u32) -> f32 {
        match self.len {
            0 if at >= self.from => 1.0,
            0 => 0.0,
            len => (at.saturating_sub(self.from) as f32 / len as f32).min(1.0),
        }
    }
}

/// A slice of the screen after a change: the device columns it holds (all of them when None),
/// when it fades in, and how far below its place it starts, device px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub columns: Option<(i32, i32)>,
    pub window: Window,
    pub rise: i32,
}

/// How a change plays: when the screen before fades out (None: it starts from the chrome alone),
/// the screen after in slices, and the picture frame that flies with how long it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub out: Option<Window>,
    pub into: Vec<Slice>,
    pub carried: Option<(Part, u32)>,
}

impl Plan {
    /// How long the change takes, ms.
    pub fn ms(&self) -> u32 {
        let into = self.into.iter().map(|s| s.window.end());
        let rest = [self.out.map(Window::end), self.carried.map(|c| c.1)];
        into.chain(rest.into_iter().flatten()).max().unwrap_or(0)
    }
}

impl Logon {
    /// Note `change` before the screen changes, unless nothing may move. A change the host has
    /// not taken yet keeps the screen it began from.
    pub(super) fn note(&mut self, change: Change) {
        if self.moves == Moves::Off || self.interface == Interface::Classic {
            return;
        }
        match &mut self.motion {
            Some(m) => m.change = change,
            None => {
                let was = Was::of(self);
                self.motion = Some(Motion { change, was });
            }
        }
    }

    /// The change [`Logon::open`] makes going from the current screen to `to`.
    pub(super) fn opening(&self, to: Screen) -> Option<Change> {
        let frame = |s: Screen| match s {
            Screen::Tile(i) => Some(Part::Tile(i)),
            Screen::Typed => Some(Part::OtherUser),
            _ => None,
        };
        match (self.screen, to) {
            (from, to) if from == to => None,
            (Screen::Message, _) => Some(Change::MessageOk),
            (Screen::Tiles, to) => frame(to).map(Change::Pick),
            (from, Screen::Tiles) => frame(from).map(Change::Back),
            _ => Some(Change::Admin),
        }
    }

    /// The screen comes forward: on a later greeting, or as the lock engages.
    pub fn enter(&mut self) {
        self.note(Change::Enter);
    }

    /// The password was sent: it is wiped and the screen is busy.
    pub fn sent(&mut self) {
        self.note(Change::Busy);
        self.wipe_password();
        self.busy = true;
        self.starting = false;
    }

    /// The password was right and the session is starting or resuming: the busy text welcomes.
    pub fn starting(&mut self) {
        self.note(Change::Status);
        self.starting = true;
        self.busy = true;
    }

    /// The change the host should animate next, taken.
    pub fn take_motion(&mut self) -> Option<Motion> {
        self.motion.take()
    }
}

#[cfg(test)]
mod tests;
