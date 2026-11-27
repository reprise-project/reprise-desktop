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

pub fn about() -> String {
    tr!("fingerprints-about")
}
pub fn looking() -> String {
    tr!("fingerprints-looking")
}
pub fn no_service() -> String {
    tr!("fingerprints-no-service")
}
pub fn no_reader() -> String {
    tr!("fingerprints-no-reader")
}
pub fn not_authorized() -> String {
    tr!("fingerprints-not-authorized")
}
pub fn silent() -> String {
    tr!("fingerprints-silent")
}
pub fn none_added() -> String {
    tr!("fingerprints-none-added")
}
pub fn all_added() -> String {
    tr!("fingerprints-all-added")
}
pub fn not_yours() -> String {
    tr!("fingerprints-not-yours")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Left,
    Right,
}

impl Hand {
    pub fn label(self) -> String {
        match self {
            Hand::Left => tr!("finger-hand-left"),
            Hand::Right => tr!("finger-hand-right"),
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
    pub fn digit(self) -> String {
        tr!("finger-digit", finger = self.key())
    }

    /// "Right index finger".
    pub fn label(self) -> String {
        tr!("finger-name", finger = self.key())
    }

    /// "right index finger", as it reads inside a sentence.
    pub fn mid_sentence(self) -> String {
        tr!("finger-name-mid", finger = self.key())
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
    /// fprintd refused to read the reader for this session.
    NotAuthorized,
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
    pub note: Option<String>,
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
        s.note = Some(not_yours());
        return s;
    }
    let (name, fingers) = match &f.reader {
        Reader::Looking => {
            return Section {
                note: Some(looking()),
                ..s
            };
        }
        Reader::NoService => {
            return Section {
                note: Some(no_service()),
                ..s
            };
        }
        Reader::NoReader => {
            return Section {
                note: Some(no_reader()),
                ..s
            };
        }
        Reader::NotAuthorized => {
            return Section {
                note: Some(not_authorized()),
                ..s
            };
        }
        Reader::Silent => {
            return Section {
                note: Some(silent()),
                ..s
            };
        }
        Reader::Found { name, fingers } => (name, fingers),
    };
    let note = match fingers.len() {
        0 => Some(none_added()),
        n if n >= Finger::ALL.len() => Some(all_added()),
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
        let note = s.note.clone().unwrap_or_else(not_yours);
        let r = Rect::new(x, y, w, NOTE_LINES * LINE_H);
        return (
            vec![Item::lines(NOTE, r, &[&note]).enabled(false)],
            r.bottom(),
        );
    }
    let mut out = vec![Item::lines(
        INTRO,
        Rect::new(x, y, w, ABOUT_LINES * LINE_H),
        &[&about()],
    )];
    let mut cy = y + ABOUT_LINES * LINE_H + GAP;
    let reader = match &s.reader {
        Some(name) => tr!("fingerprints-reader", name = cut_short(name)),
        None => tr!("fingerprints-reader-none"),
    };
    out.push(Item::label_at(READER, x, cy, w, &reader).enabled(s.reader.is_some()));
    cy += LINE_H + GAP;
    out.push(Item::label_at(
        "fingers_l",
        x,
        cy,
        w - SIDE_W - SIDE_GAP,
        &tr!("fingerprints-list-label"),
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
        Item::button_at(ADD, bx, cy, SIDE_W, &tr!("fingerprints-add"), s.add),
        Item::button_at(
            DELETE,
            bx,
            cy + step,
            SIDE_W,
            &tr!("fingerprints-delete"),
            false,
        ),
        Item::button_at(
            CLEAR,
            bx,
            cy + 2 * step,
            SIDE_W,
            &tr!("fingerprints-delete-all"),
            s.delete,
        ),
    ]);
    cy += list_h;
    if let Some(note) = &s.note {
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
