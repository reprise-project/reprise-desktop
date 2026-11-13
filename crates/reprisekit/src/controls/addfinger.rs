//! The Add a Fingerprint dialog: pick a finger, then scan it until the bar is full. The host runs
//! the reader and hands its answers in ([`AddFinger::started`], [`AddFinger::scanned`]).

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::fingerprints::{Finger, Hand};
use crate::controls::form::{self, Form, Item};
use crate::raster::Rect;

pub const START: &str = "start";
pub const RETRY: &str = "retry";
pub const CANCEL: &str = "cancel";
pub const CLOSE: &str = "close";
pub const TEXT: &str = "text";
pub const BAR: &str = "bar";
pub const COUNT: &str = "count";
pub const HINT: &str = "hint";
const RADIOS: &str = "finger";

pub const TITLE: &str = "Add a Fingerprint";
pub const PICK: &str = "Pick the finger to add. The fingers you've already added are greyed.";
pub const WAITING: &str = "Waiting for the fingerprint reader...";

/// How the reader takes a finger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Press,
    Swipe,
}

/// One answer of the reader while a finger is added, by fprintd's name for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scanned {
    Passed,
    Retry,
    TooShort,
    NotCentred,
    LiftAndRetry,
    Completed,
    Failed,
    Full,
    Duplicate,
    Disconnected,
    Error,
}

impl Scanned {
    /// An unknown name reads as an error.
    pub fn from_key(key: &str) -> Scanned {
        match key {
            "enroll-stage-passed" => Scanned::Passed,
            "enroll-retry-scan" => Scanned::Retry,
            "enroll-swipe-too-short" => Scanned::TooShort,
            "enroll-finger-not-centered" => Scanned::NotCentred,
            "enroll-remove-and-retry" => Scanned::LiftAndRetry,
            "enroll-completed" => Scanned::Completed,
            "enroll-failed" => Scanned::Failed,
            "enroll-data-full" => Scanned::Full,
            "enroll-duplicate" => Scanned::Duplicate,
            "enroll-disconnected" => Scanned::Disconnected,
            _ => Scanned::Error,
        }
    }
}

/// Why the reader could not be used at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The password prompt was cancelled or refused.
    NotAuthorized,
    /// Another program holds the reader.
    InUse,
    Failed,
}

impl Refusal {
    /// What a dialog says for it; `doing` is what was refused, "add a fingerprint".
    pub fn text(self, doing: &str) -> String {
        match self {
            Refusal::NotAuthorized => format!("You weren't allowed to {doing}."),
            Refusal::InUse => "Another program is using the fingerprint reader. Try again once \
                               it is done."
                .into(),
            Refusal::Failed => format!("The fingerprint reader couldn't {doing}."),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Pick,
    /// The reader is being claimed, which may ask for your password.
    Waiting(Finger),
    Scanning {
        finger: Finger,
        done: u32,
        hint: Option<&'static str>,
    },
    Added(Finger),
    Failed(Finger, String),
}

pub enum Outcome {
    None,
    Redraw,
    /// Start adding the finger; the host answers with [`AddFinger::started`] or
    /// [`AddFinger::refused`].
    Start(Finger),
    /// Stop the reader and close.
    Stop,
    Close,
}

pub struct AddFinger {
    pub form: Form,
    pub step: Step,
    /// Greyed in the picker.
    added: Vec<Finger>,
    scan: Scan,
    /// Scans to a finger; 0 when the reader doesn't say.
    stages: u32,
    /// Tells this dialog's answers from those of one closed before it.
    pub generation: u64,
}

const W: i32 = 340;
const M: i32 = 12;
const GROUP_W: i32 = (W - 3 * M) / 2;
const GROUP_Y: i32 = M + 2 * LINE_H + 10;
const RADIO_DY: i32 = 20;
const GROUP_H: i32 = 26 + 5 * RADIO_DY;
const BUTTON_Y: i32 = GROUP_Y + GROUP_H + M;
const BAR_Y: i32 = M + 3 * LINE_H + 10;
const BAR_H: i32 = 16;

impl AddFinger {
    /// `added` are greyed and the first other finger is picked, the right index finger if free.
    pub fn new(added: Vec<Finger>, scan: Scan, stages: u32, generation: u64) -> AddFinger {
        let mut d = AddFinger {
            form: Form::new(Vec::new()),
            step: Step::Pick,
            added,
            scan,
            stages,
            generation,
        };
        d.build();
        d
    }

    pub fn client(&self) -> (i32, i32) {
        (W, BUTTON_Y + BUTTON_H + M)
    }

    fn first_free(&self) -> Option<Finger> {
        let free = |f: &Finger| !self.added.contains(f);
        std::iter::once(Finger::FIRST).chain(Finger::ALL).find(free)
    }

    /// The finger checked in the picker.
    pub fn picked(&self) -> Option<Finger> {
        Finger::ALL
            .into_iter()
            .find(|f| self.form.checked(f.key()) == Some(true))
    }

    fn buttons(&self, list: &[(&'static str, &str)]) -> Vec<Item> {
        let n = list.len() as i32;
        let mut x = W - M - n * BUTTON_W - (n - 1) * BUTTON_GAP;
        let mut out = Vec::new();
        for (k, (id, label)) in list.iter().enumerate() {
            let r = Rect::new(x, BUTTON_Y, BUTTON_W, BUTTON_H);
            out.push(Item::button(id, r, label, true, k == 0));
            x += BUTTON_W + BUTTON_GAP;
        }
        out
    }

    fn text(&self, lines: i32, s: &str) -> Item {
        Item::lines(TEXT, Rect::new(M, M, W - 2 * M, lines * LINE_H), &[s])
    }

    fn picker(&self) -> Vec<Item> {
        let mut items = vec![self.text(2, PICK)];
        let pick = self.first_free();
        for (k, hand) in [Hand::Left, Hand::Right].into_iter().enumerate() {
            let gx = M + k as i32 * (GROUP_W + M);
            items.push(Item::group(
                ["left_g", "right_g"][k],
                Rect::new(gx, GROUP_Y, GROUP_W, GROUP_H),
                hand.label(),
            ));
            let fingers = Finger::ALL.into_iter().filter(|f| f.hand() == hand);
            for (row, f) in fingers.enumerate() {
                let y = GROUP_Y + 20 + row as i32 * RADIO_DY;
                items.push(
                    Item::radio(f.key(), gx + 12, y, f.digit(), RADIOS)
                        .checked(pick == Some(f))
                        .enabled(!self.added.contains(&f)),
                );
            }
        }
        items.extend(self.buttons(&[(START, "&Start"), (CANCEL, "Cancel")]));
        items
    }

    /// `text` over the bar `done` scans full; `None` leaves it empty with no count.
    fn progress(&self, text: &str, done: Option<u32>, hint: Option<&str>) -> Vec<Item> {
        let of = match done {
            Some(done) if self.stages > 0 => self.stages.max(done),
            _ => 0,
        };
        let done = done.unwrap_or(0);
        let bar = Rect::new(M, BAR_Y, W - 2 * M, BAR_H);
        let count = if of == 0 {
            String::new()
        } else {
            format!("{done} of {of} scans")
        };
        vec![
            self.text(3, text),
            Item::progress(BAR, bar, done, of).named("Scans"),
            Item::label_at(COUNT, M, BAR_Y + BAR_H + 6, W - 2 * M, &count),
            Item::lines(
                HINT,
                Rect::new(M, BAR_Y + BAR_H + 6 + LINE_H + 10, W - 2 * M, 3 * LINE_H),
                &[hint.unwrap_or("")],
            ),
        ]
    }

    fn how(&self, f: Finger) -> String {
        let finger = f.label().to_lowercase();
        match self.scan {
            Scan::Press => format!(
                "Put your {finger} on the reader, then lift it. Do it again each time the bar \
                 moves on, until it is full."
            ),
            Scan::Swipe => format!(
                "Swipe your {finger} across the reader. Swipe it again each time the bar moves \
                 on, until it is full."
            ),
        }
    }

    fn build(&mut self) {
        let mut items = match &self.step {
            Step::Pick => self.picker(),
            Step::Waiting(_) => self.progress(WAITING, None, None),
            Step::Scanning { finger, done, hint } => {
                self.progress(&self.how(*finger), Some(*done), *hint)
            }
            Step::Added(f) => {
                let text = format!(
                    "Your {} is added. It can unlock the screen from now on.",
                    f.label().to_lowercase()
                );
                self.progress(&text, Some(self.stages), None)
            }
            Step::Failed(_, why) => self.progress(why, None, None),
        };
        let buttons: &[(&'static str, &str)] = match &self.step {
            Step::Pick => &[],
            Step::Waiting(_) | Step::Scanning { .. } => &[(CANCEL, "Cancel")],
            Step::Added(_) => &[(CLOSE, "Close")],
            Step::Failed(..) => &[(RETRY, "&Try Again"), (CLOSE, "Close")],
        };
        items.extend(self.buttons(buttons));
        let old = std::mem::replace(&mut self.form, Form::new(items));
        if self.step == Step::Pick {
            self.form.carry_pointer(&old);
        }
        let focus = match &self.step {
            Step::Pick => START,
            Step::Waiting(_) | Step::Scanning { .. } => CANCEL,
            Step::Added(_) => CLOSE,
            Step::Failed(..) => RETRY,
        };
        self.form.focus_id(focus);
    }

    fn go(&mut self, step: Step) {
        self.step = step;
        self.build();
    }

    /// A key or a click the host has already given the form, as what it should do about it.
    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match (&self.step, ev) {
            (_, form::Event::Redraw | form::Event::Changed(_)) => Outcome::Redraw,
            (Step::Pick, form::Event::Pressed(START) | form::Event::Default) => {
                let Some(f) = self.picked() else {
                    return Outcome::None;
                };
                self.go(Step::Waiting(f));
                Outcome::Start(f)
            }
            (Step::Failed(f, _), form::Event::Pressed(RETRY) | form::Event::Default) => {
                let f = *f;
                self.go(Step::Waiting(f));
                Outcome::Start(f)
            }
            (Step::Waiting(_) | Step::Scanning { .. }, form::Event::Pressed(CANCEL))
            | (Step::Waiting(_) | Step::Scanning { .. }, form::Event::Cancel) => Outcome::Stop,
            (_, form::Event::Pressed(CANCEL | CLOSE) | form::Event::Cancel) => Outcome::Close,
            (Step::Added(_), form::Event::Default) => Outcome::Close,
            _ => Outcome::None,
        }
    }

    /// The reader is claimed and waits for the finger.
    pub fn started(&mut self) {
        if let Step::Waiting(finger) = self.step {
            self.go(Step::Scanning {
                finger,
                done: 0,
                hint: None,
            });
        }
    }

    /// The reader's answer to a scan.
    pub fn scanned(&mut self, s: Scanned) {
        let (finger, done) = match self.step {
            Step::Scanning { finger, done, .. } => (finger, done),
            Step::Waiting(finger) => (finger, 0),
            _ => return,
        };
        let again = match self.scan {
            Scan::Press => "Lift your finger, then put it on the reader again.",
            Scan::Swipe => "Swipe it again.",
        };
        let (done, hint) = match s {
            Scanned::Passed => (done + 1, again),
            Scanned::Retry => (done, "That scan couldn't be read. Try again."),
            Scanned::TooShort => (done, "The swipe was too short. Swipe again."),
            Scanned::NotCentred => (done, "Put your finger in the middle of the reader."),
            Scanned::LiftAndRetry => (done, "Lift your finger and try again."),
            Scanned::Completed => return self.go(Step::Added(finger)),
            Scanned::Failed => return self.fail(finger, "The fingerprint couldn't be added."),
            Scanned::Full => {
                return self.fail(
                    finger,
                    "The reader has no room for another fingerprint. Delete one first.",
                );
            }
            Scanned::Duplicate => {
                return self.fail(
                    finger,
                    "That fingerprint is already added, as another finger or for another user.",
                );
            }
            Scanned::Disconnected => {
                return self.fail(finger, "The fingerprint reader was disconnected.");
            }
            Scanned::Error => {
                return self.fail(finger, "The fingerprint reader reported an error.");
            }
        };
        self.go(Step::Scanning {
            finger,
            done,
            hint: Some(hint),
        });
    }

    fn fail(&mut self, finger: Finger, why: &str) {
        self.go(Step::Failed(finger, why.into()));
    }

    /// The reader could not be used.
    pub fn refused(&mut self, r: Refusal) {
        let finger = match self.step {
            Step::Waiting(f) | Step::Scanning { finger: f, .. } => f,
            _ => return,
        };
        self.fail(finger, &r.text("add a fingerprint"));
    }

    /// The text shown above the bar, or the picker's.
    pub fn shown_text(&self) -> String {
        match self.form.item(TEXT).map(|i| &i.control) {
            Some(form::Control::Lines(l)) => l.join(" "),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
#[path = "addfinger_tests.rs"]
mod tests;
