//! The install and uninstall sheet: what a program or package is, a button that installs or
//! uninstalls it, then its progress and how it ended.

use crate::access::{self, ROOT, Tree, accesskit::NodeId, accesskit::Role};
use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{self, Event, Form, Item};
use crate::look::Paint;
use crate::look::widgets::draw_progress;
use crate::raster::{Canvas, Rect};
use crate::text::{TextRenderer, escape_mnemonic};
use crate::theme::TextUnits;

pub const ACT: &str = "act";
pub const CANCEL: &str = "cancel";
pub const CLOSE: &str = "close";
const HEAD: &str = "head";
const NOTE: &str = "note";
const DOING: &str = "doing";
const FACT_IDS: [(&str, &str); 4] = [
    ("fact0", "value0"),
    ("fact1", "value1"),
    ("fact2", "value2"),
    ("fact3", "value3"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verb {
    Install,
    Uninstall,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Its details are being read.
    Reading,
    /// Waiting for the answer. Without `ready` it cannot go ahead, and the note says why.
    Asking {
        ready: bool,
    },
    /// Under way: the share done when known, what it is doing, and whether Cancel stops it.
    Running {
        fraction: Option<f64>,
        doing: String,
        cancellable: bool,
    },
    Done,
    /// Stopped, and why.
    Failed(String),
    /// It may not go ahead at all: only Close is offered, and the note says why.
    Refused,
}

/// What the user picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Install or Uninstall.
    Go,
    /// Cancel, Escape or the close box before the end.
    Cancel,
    /// Close, Enter or Escape once it ended.
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub verb: Verb,
    /// The program's name, or the package file's.
    pub name: String,
    /// 32 px, at the top left; `None` leaves the slot empty.
    pub icon: Option<Canvas>,
    /// Label and value rows (Version, Size, Source), at most four.
    pub facts: Vec<(String, String)>,
    /// What else changes with it, or why it cannot go ahead.
    pub note: String,
    pub stage: Stage,
}

impl Sheet {
    /// The caption the host puts on the frame.
    pub fn title(&self) -> &'static str {
        match self.verb {
            Verb::Install => "Install Software",
            Verb::Uninstall => "Uninstall Software",
        }
    }

    fn headline(&self) -> String {
        let n = &self.name;
        match (&self.stage, self.verb) {
            (Stage::Reading, _) => format!("Looking up {n}..."),
            (Stage::Asking { .. }, Verb::Install) => format!("Do you want to install {n}?"),
            (Stage::Asking { .. }, Verb::Uninstall) => format!("Do you want to uninstall {n}?"),
            (Stage::Running { .. }, Verb::Install) => format!("Installing {n}..."),
            (Stage::Running { .. }, Verb::Uninstall) => format!("Uninstalling {n}..."),
            (Stage::Done, Verb::Install) => format!("{n} was installed."),
            (Stage::Done, Verb::Uninstall) => format!("{n} was uninstalled."),
            (Stage::Failed(_), Verb::Install) => format!("{n} was not installed."),
            (Stage::Failed(_), Verb::Uninstall) => format!("{n} was not uninstalled."),
            (Stage::Refused, Verb::Install) => format!("Reprise cannot install {n}."),
            (Stage::Refused, Verb::Uninstall) => format!("Reprise cannot uninstall {n}."),
        }
    }

    /// Whether Close is the only way left.
    fn ended(&self) -> bool {
        matches!(self.stage, Stage::Done | Stage::Failed(_) | Stage::Refused)
    }

    /// The button a form event stands for. Enter presses the default button: Cancel while asking,
    /// so Enter never goes ahead.
    pub fn choice(&self, ev: &Event) -> Option<Choice> {
        match ev {
            Event::Pressed(ACT) => Some(Choice::Go),
            Event::Pressed(CLOSE) => Some(Choice::Close),
            Event::Pressed(CANCEL) | Event::Default | Event::Cancel if self.ended() => {
                Some(Choice::Close)
            }
            Event::Pressed(CANCEL) | Event::Default | Event::Cancel => Some(Choice::Cancel),
            _ => None,
        }
    }

    /// The share of the bar to fill, while running.
    fn bar(&self) -> Option<f64> {
        match &self.stage {
            Stage::Running { fraction, .. } => Some(fraction.unwrap_or(0.0).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

const W: i32 = 400;
const MARGIN: i32 = 12;
const ICON: Rect = Rect::new(MARGIN, 12, 32, 32);
const TEXT_X: i32 = 56;
const COL_W: i32 = W - TEXT_X - MARGIN;
const HEAD_Y: i32 = 14;
const HEAD_LINES: usize = 2;
const FACTS_Y: i32 = 52;
const FACT_H: i32 = 16;
const LABEL_W: i32 = 64;
/// The note's slot holds this many lines, or the step line and the bar under it.
const NOTE_LINES: usize = 3;
const NOTE_GAP: i32 = 8;
const BAR_GAP: i32 = 4;
const BAR_H: i32 = 16;
const BUTTON_GAP_Y: i32 = 14;

fn note_y(s: &Sheet) -> i32 {
    FACTS_Y + s.facts.len().min(FACT_IDS.len()) as i32 * FACT_H + NOTE_GAP
}

fn button_y(s: &Sheet) -> i32 {
    note_y(s) + NOTE_LINES as i32 * LINE_H + BUTTON_GAP_Y
}

fn bar_rect(s: &Sheet) -> Rect {
    Rect::new(TEXT_X, note_y(s) + LINE_H + BAR_GAP, COL_W, BAR_H)
}

/// The client size at 100%: it stays the same through every stage.
pub fn client(s: &Sheet) -> (i32, i32) {
    (W, button_y(s) + BUTTON_H + MARGIN)
}

/// `body` wrapped to the text column as text set at `px` grown by `u` fills it, at most `max` lines.
fn wrapped(body: &str, max: usize, text: &TextRenderer, px: u32, u: TextUnits) -> Vec<String> {
    let mut lines = crate::text::wrap(text, body, u.len(COL_W), px);
    lines.truncate(max);
    lines.iter().map(|l| escape_mnemonic(l)).collect()
}

/// Lay `s`'s stage out as a consent form ([`Form::set_consent`]) whose default is Cancel (Close once
/// it ended). A new stage builds it again and takes the old one's wait ([`Form::carry_gate`]).
pub fn form(s: &Sheet, text: &TextRenderer, px: u32, u: TextUnits) -> Form {
    let mut items = Vec::new();
    if let Some(icon) = &s.icon {
        items.push(Item::picture("icon", ICON, Some(icon.clone())));
    }
    let head = wrapped(&s.headline(), HEAD_LINES, text, px, u);
    let head: Vec<&str> = head.iter().map(String::as_str).collect();
    items.push(Item::lines_at(HEAD, TEXT_X, HEAD_Y, COL_W, &head));
    let value_w = COL_W - LABEL_W;
    for (i, ((label, value), (lid, vid))) in s.facts.iter().zip(FACT_IDS).enumerate() {
        let y = FACTS_Y + i as i32 * FACT_H;
        let value = text.ellipsize(value, u.len(value_w), false, px);
        items.push(Item::label_at(
            lid,
            TEXT_X,
            y,
            LABEL_W,
            &escape_mnemonic(label),
        ));
        items.push(Item::label_at(
            vid,
            TEXT_X + LABEL_W,
            y,
            value_w,
            &escape_mnemonic(&value),
        ));
    }
    let y = note_y(s);
    match &s.stage {
        Stage::Running { doing, .. } => {
            let doing = text.ellipsize(doing, u.len(COL_W), false, px);
            items.push(Item::label_at(
                DOING,
                TEXT_X,
                y,
                COL_W,
                &escape_mnemonic(&doing),
            ));
        }
        stage => {
            let body = match stage {
                Stage::Failed(why) => why.as_str(),
                _ => s.note.as_str(),
            };
            let note = wrapped(body, NOTE_LINES, text, px, u);
            let note: Vec<&str> = note.iter().map(String::as_str).collect();
            items.push(Item::lines_at(NOTE, TEXT_X, y, COL_W, &note));
        }
    }
    let by = button_y(s);
    let right = Rect::new(W - MARGIN - BUTTON_W, by, BUTTON_W, BUTTON_H);
    let focus = if s.ended() {
        items.push(Item::button(CLOSE, right, "Close", true, true));
        CLOSE
    } else {
        let left = Rect::new(right.x - BUTTON_GAP - BUTTON_W, by, BUTTON_W, BUTTON_H);
        let act = match s.verb {
            Verb::Install => "&Install",
            Verb::Uninstall => "&Uninstall",
        };
        let ready = s.stage == Stage::Asking { ready: true };
        let cancellable = match &s.stage {
            Stage::Running { cancellable, .. } => *cancellable,
            _ => true,
        };
        items.push(Item::button(ACT, left, act, ready, false));
        items.push(Item::button(CANCEL, right, "Cancel", cancellable, true));
        CANCEL
    };
    let mut f = Form::new(items);
    f.focus_id(focus);
    f.set_consent();
    f
}

/// Paint `s` with its `form` on `c`, whose top-left is the client's: the bar shows while it runs.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, s: &Sheet, f: &Form, px: u32) {
    c.fill_rect(c.bounds(), p.cs.button_face);
    form::draw(c, text, p, f, true, px);
    if let Some(fraction) = s.bar() {
        draw_progress(c, p, p.text.rect(bar_rect(s)), fraction, true);
    }
}

/// `s`'s screen reader nodes under the root: the form's, and the bar while it runs. Returns the
/// focused node.
pub fn access(
    s: &Sheet,
    f: &Form,
    tree: &mut Tree,
    text: &TextRenderer,
    px: u32,
    u: TextUnits,
) -> Option<NodeId> {
    let focus = f.access(tree, ROOT, text, px);
    if let Some(fraction) = s.bar() {
        let r = u.rect(bar_rect(s));
        let mut bar = access::range(
            Role::ProgressIndicator,
            r,
            fraction * 100.0,
            0.0,
            100.0,
            true,
        );
        bar.set_label(s.headline());
        tree.push(ROOT, "progress", bar);
    }
    focus
}

#[cfg(test)]
#[path = "installsheet_tests.rs"]
mod tests;
