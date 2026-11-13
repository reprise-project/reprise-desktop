//! The Fingerprints page of an account's Properties. [`section`] reads the [`Facts`], [`items`]
//! lays it out, [`follow`] lights Delete for the picked row and [`action`] reads a press.

use crate::classic::widgets::{BUTTON_H, LINE_H};
use crate::controls::form::{Control, Event, Form, Item};
use crate::controls::listbox::{self, ListBox};
use crate::raster::Rect;

pub const INTRO: &str = "finger_intro";
pub const READER: &str = "finger_reader";
pub const LIST: &str = "fingers";
pub const ADD: &str = "finger_add";
pub const DELETE: &str = "finger_delete";
pub const CLEAR: &str = "finger_clear";
pub const NOTE: &str = "finger_note";

pub const ABOUT: &str = "Your fingerprints can unlock the screen and answer administrator \
                         password prompts in place of your password.";
pub const LOOKING: &str = "Looking for a fingerprint reader...";
pub const NO_SERVICE: &str = "Fingerprints need fprintd, which isn't installed on this computer.";
pub const NO_READER: &str = "No fingerprint reader is connected to this computer.";
pub const SILENT: &str = "The fingerprint service isn't answering. Close Properties and open it \
                          again to retry.";
pub const NONE_ADDED: &str = "You haven't added a fingerprint yet. Click Add to add one.";
pub const ALL_ADDED: &str = "All ten fingers are added. Delete one to add it again.";
pub const NOT_YOURS: &str = "Each user adds their own fingerprints. You can only change yours.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    pub fn label(self) -> &'static str {
        match self {
            Hand::Left => "Left hand",
            Hand::Right => "Right hand",
        }
    }
}

/// A finger, by the name fprintd and its PAM module give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Finger {
    LeftThumb,
    LeftIndex,
    LeftMiddle,
    LeftRing,
    LeftLittle,
    RightThumb,
    RightIndex,
    RightMiddle,
    RightRing,
    RightLittle,
}

impl Finger {
    /// Left hand thumb to little finger, then the right hand's.
    pub const ALL: [Finger; 10] = [
        Finger::LeftThumb,
        Finger::LeftIndex,
        Finger::LeftMiddle,
        Finger::LeftRing,
        Finger::LeftLittle,
        Finger::RightThumb,
        Finger::RightIndex,
        Finger::RightMiddle,
        Finger::RightRing,
        Finger::RightLittle,
    ];

    /// The finger the dialog offers first.
    pub const FIRST: Finger = Finger::RightIndex;

    fn index(self) -> usize {
        Finger::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// fprintd's name for it.
    pub fn key(self) -> &'static str {
        [
            "left-thumb",
            "left-index-finger",
            "left-middle-finger",
            "left-ring-finger",
            "left-little-finger",
            "right-thumb",
            "right-index-finger",
            "right-middle-finger",
            "right-ring-finger",
            "right-little-finger",
        ][self.index()]
    }

    pub fn from_key(key: &str) -> Option<Finger> {
        Finger::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn hand(self) -> Hand {
        if self.index() < 5 {
            Hand::Left
        } else {
            Hand::Right
        }
    }

    /// The finger on its hand, "Index finger".
    pub fn digit(self) -> &'static str {
        [
            "Thumb",
            "Index finger",
            "Middle finger",
            "Ring finger",
            "Little finger",
        ][self.index() % 5]
    }

    /// "Right index finger".
    pub fn label(self) -> String {
        let side = match self.hand() {
            Hand::Left => "Left",
            Hand::Right => "Right",
        };
        format!("{side} {}", self.digit().to_lowercase())
    }
}

/// What fprintd said of the reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reader {
    /// Not asked yet, or the answer is on its way.
    Looking,
    /// fprintd isn't installed.
    NoService,
    NoReader,
    /// fprintd is there but gave no answer.
    Silent,
    /// The reader's name and the fingers you have added on it, in [`Finger::ALL`]'s order.
    Found {
        name: String,
        fingers: Vec<Finger>,
    },
}

/// What decides the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub reader: Reader,
    /// The account shown is the one logged in.
    pub yours: bool,
    /// An add or a delete is under way: every action greys until it ends.
    pub busy: bool,
}

/// What the page shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    /// False: only the note, greyed.
    pub yours: bool,
    pub reader: Option<String>,
    pub fingers: Vec<Finger>,
    pub add: bool,
    /// Delete All; Delete also needs a row picked ([`follow`]).
    pub delete: bool,
    pub note: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Add,
    Delete(Finger),
    DeleteAll,
}

/// The page for `f`.
pub fn section(f: &Facts) -> Section {
    let mut s = Section {
        yours: f.yours,
        reader: None,
        fingers: Vec::new(),
        add: false,
        delete: false,
        note: None,
    };
    if !f.yours {
        s.note = Some(NOT_YOURS);
        return s;
    }
    let (name, fingers) = match &f.reader {
        Reader::Looking => {
            return Section {
                note: Some(LOOKING),
                ..s
            };
        }
        Reader::NoService => {
            return Section {
                note: Some(NO_SERVICE),
                ..s
            };
        }
        Reader::NoReader => {
            return Section {
                note: Some(NO_READER),
                ..s
            };
        }
        Reader::Silent => {
            return Section {
                note: Some(SILENT),
                ..s
            };
        }
        Reader::Found { name, fingers } => (name, fingers),
    };
    let note = match fingers.len() {
        0 => Some(NONE_ADDED),
        n if n >= Finger::ALL.len() => Some(ALL_ADDED),
        _ => None,
    };
    Section {
        reader: Some(name.clone()),
        fingers: fingers.clone(),
        add: !f.busy && fingers.len() < Finger::ALL.len(),
        delete: !f.busy && !fingers.is_empty(),
        note,
        ..s
    }
}

/// The finger in `form`'s list row picked, if any.
fn picked(form: &Form, s: &Section) -> Option<Finger> {
    let i = match &form.item(LIST)?.control {
        Control::ListBox { list, .. } => list.selected?,
        _ => return None,
    };
    s.fingers.get(i).copied()
}

/// Light Delete while a row is picked and `s` allows deleting.
pub fn follow(form: &mut Form, s: &Section) {
    let lit = s.delete && picked(form, s).is_some();
    form.set_enabled(DELETE, lit);
}

/// The action a press on `form`, laid out for `s`, asks for.
pub fn action(ev: &Event, form: &Form, s: &Section) -> Option<Action> {
    match ev {
        Event::Pressed(ADD) => Some(Action::Add),
        Event::Pressed(DELETE) => picked(form, s).map(Action::Delete),
        Event::Pressed(CLEAR) => Some(Action::DeleteAll),
        _ => None,
    }
}

const ABOUT_LINES: i32 = 2;
const NOTE_LINES: i32 = 3;
const GAP: i32 = 10;
const SIDE_W: i32 = 90;
const SIDE_GAP: i32 = 8;
const LIST_ROWS: i32 = 6;
const BUTTON_GAP: i32 = 6;

/// The most of a reader's name the page shows, so it keeps to one line.
const NAME_CHARS: usize = 40;

fn cut_short(name: &str) -> std::borrow::Cow<'_, str> {
    if name.chars().count() <= NAME_CHARS {
        return name.into();
    }
    let kept: String = name.chars().take(NAME_CHARS - 3).collect();
    format!("{kept}...").into()
}

/// `s` laid out from `(x, y)`, `w` wide, as form items, and the bottom of the last.
pub fn items(s: &Section, x: i32, y: i32, w: i32) -> (Vec<Item>, i32) {
    if !s.yours {
        let note = s.note.unwrap_or(NOT_YOURS);
        let r = Rect::new(x, y, w, NOTE_LINES * LINE_H);
        return (
            vec![Item::lines(NOTE, r, &[note]).enabled(false)],
            r.bottom(),
        );
    }
    let mut out = vec![Item::lines(
        INTRO,
        Rect::new(x, y, w, ABOUT_LINES * LINE_H),
        &[ABOUT],
    )];
    let mut cy = y + ABOUT_LINES * LINE_H + GAP;
    let reader = match &s.reader {
        Some(name) => format!("Reader: {}", cut_short(name)),
        None => "Reader: none".into(),
    };
    out.push(Item::label_at(READER, x, cy, w, &reader).enabled(s.reader.is_some()));
    cy += LINE_H + GAP;
    out.push(Item::label_at(
        "fingers_l",
        x,
        cy,
        w - SIDE_W - SIDE_GAP,
        "&Fingerprints you've added:",
    ));
    cy += LINE_H + 3;
    let list_w = w - SIDE_W - SIDE_GAP;
    let list_h = LIST_ROWS * listbox::ROW_H + 4;
    let rows = s.fingers.iter().map(|f| f.label()).collect();
    out.push(Item::new(
        LIST,
        Rect::new(x, cy, list_w, list_h),
        Control::ListBox {
            list: ListBox::new(rows, None),
            icons: Vec::new(),
        },
    ));
    let bx = x + w - SIDE_W;
    let step = BUTTON_H + BUTTON_GAP;
    out.extend([
        Item::button_at(ADD, bx, cy, SIDE_W, "A&dd...", s.add),
        Item::button_at(DELETE, bx, cy + step, SIDE_W, "D&elete", false),
        Item::button_at(CLEAR, bx, cy + 2 * step, SIDE_W, "Delete A&ll", s.delete),
    ]);
    cy += list_h;
    if let Some(note) = s.note {
        cy += GAP;
        out.push(Item::lines(
            NOTE,
            Rect::new(x, cy, w, NOTE_LINES * LINE_H),
            &[note],
        ));
        cy += NOTE_LINES * LINE_H;
    }
    (out, cy)
}

#[cfg(test)]
#[path = "fingerprints_tests.rs"]
pub(crate) mod tests;
