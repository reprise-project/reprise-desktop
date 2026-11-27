//! The Print dialog: the printer, which pages and how many copies. The host lists the printers
//! ([`Dest`]); [`follow`] keeps the Status, Where and Comment rows on the one picked.

use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::controls::updown::{self, UpDown};
use crate::raster::Rect;
use crate::text::DialogFont;

/// What a printer is doing, as the Status row says it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DestState {
    #[default]
    Ready,
    Printing,
    /// Paused or stopped on an error; the printer's own message, which may be empty.
    Stopped(String),
    /// It takes no new jobs.
    Refusing,
}

/// A printer the dialog offers: its queue name, description and location.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dest {
    pub name: String,
    pub info: String,
    pub location: String,
    pub state: DestState,
}

impl Dest {
    pub fn status(&self) -> String {
        match &self.state {
            DestState::Ready => tr!("printdlg-ready"),
            DestState::Printing => tr!("printdlg-printing"),
            DestState::Stopped(m) if m.is_empty() => tr!("printdlg-stopped"),
            DestState::Stopped(m) => tr!("printdlg-stopped-why", why = m.as_str()),
            DestState::Refusing => tr!("printdlg-refusing"),
        }
    }
}

/// Why no printer can be picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoDest {
    /// The host is still asking for the printers.
    Looking,
    /// The print service did not answer.
    NotRunning,
    /// It answered with no printers.
    NoneInstalled,
    /// It answered with an error or a reply that does not read.
    Unlisted,
}

/// The pages to print, counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    All,
    Pages(u32, u32),
}

impl Range {
    /// The pages of a document of `pages` it covers, from 0.
    pub fn of(self, pages: u32) -> std::ops::Range<u32> {
        match self {
            Range::All => 0..pages,
            Range::Pages(from, to) => from.saturating_sub(1)..to.min(pages),
        }
    }
}

/// What Print sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The picked printer, in the list the form was built with.
    pub dest: usize,
    pub copies: u32,
    pub range: Range,
}

pub const PRINTER: &str = "printer";
pub const STATUS: &str = "status";
pub const WHERE: &str = "where";
pub const COMMENT: &str = "comment";
pub const ALL: &str = "all";
pub const PAGES: &str = "pages";
pub const FROM: &str = "from";
pub const TO: &str = "to";
pub const COPIES: &str = "copies";
pub const PRINT: &str = "print";
pub const CANCEL: &str = "cancel";

pub const CLIENT: (i32, i32) = (408, 262);
const PAD: i32 = 8;
const PRINTER_GROUP: Rect = Rect::new(PAD, PAD, CLIENT.0 - 2 * PAD, 112);
const RANGE_GROUP: Rect = Rect::new(PAD, 128, 236, 92);
const COPIES_GROUP: Rect = Rect::new(252, 128, CLIENT.0 - 252 - PAD, 92);
const LABEL_X: i32 = 20;
const VALUE_X: i32 = 84;
const VALUE_W: i32 = CLIENT.0 - VALUE_X - 20;
const PAGE_W: i32 = 40;
const EDIT_H: i32 = 20;
const PRINTER_ROWS: usize = 6;
const BUTTONS_Y: i32 = 230;
pub const MAX_COPIES: i32 = 999;

/// The dialog's form: `dests` in the Name box with `at` picked (or why there are none), and a
/// document of `pages` pages. Pages and its boxes are greyed for a one-page document.
pub fn form(dests: Result<&[Dest], NoDest>, at: usize, pages: u32) -> Form {
    let pages = pages.max(1);
    let names: Vec<String> = match dests {
        Ok(d) if !d.is_empty() => d.iter().map(|d| d.name.clone()).collect(),
        Ok(_) => vec![tr!("printdlg-none-installed")],
        Err(NoDest::Looking) => vec![tr!("printdlg-looking")],
        Err(NoDest::NoneInstalled) => vec![tr!("printdlg-none-installed")],
        Err(NoDest::NotRunning) => vec![tr!("printdlg-not-running")],
        Err(NoDest::Unlisted) => vec![tr!("printdlg-unlisted")],
    };
    let live = matches!(dests, Ok(d) if !d.is_empty());
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut printer = Item::combo(PRINTER, VALUE_X, 26, VALUE_W, &names, PRINTER_ROWS);
    if let Control::Combo(c) = &mut printer.control {
        c.selected = if live { at.min(names.len() - 1) } else { 0 };
        c.enabled = live;
    }
    let row = |id, y, label: String| Item::label_at(id, LABEL_X, y, VALUE_X - LABEL_X - 4, &label);
    let value = |id, y| Item::label_at(id, VALUE_X, y, VALUE_W, "");
    let several = pages > 1;
    let page_box =
        |id, x, text: String| Item::edit(id, Rect::new(x, 168, PAGE_W, EDIT_H), &text, several);
    let mut copies = UpDown::new(1, 1, MAX_COPIES);
    copies.enabled = live;
    let (cx, cy) = (COPIES_GROUP.x + 12, COPIES_GROUP.y);
    let mut f = Form::new(vec![
        Item::group("printergroup", PRINTER_GROUP, &tr!("printdlg-printer")),
        row("namelabel", 30, tr!("printdlg-name")),
        printer,
        row("statuslabel", 58, tr!("printdlg-status")),
        value(STATUS, 58),
        row("wherelabel", 78, tr!("printdlg-where")),
        value(WHERE, 78),
        row("commentlabel", 98, tr!("printdlg-comment")),
        value(COMMENT, 98),
        Item::group("rangegroup", RANGE_GROUP, &tr!("printdlg-range")),
        Item::radio(ALL, LABEL_X, 148, &tr!("printdlg-all"), "range").checked(true),
        Item::radio(PAGES, LABEL_X, 172, &tr!("printdlg-pages"), "range").enabled(several),
        Item::label_at("fromlabel", 92, 172, 36, &tr!("printdlg-from")).enabled(several),
        page_box(FROM, 128, "1".into()),
        Item::label_at("tolabel", 174, 172, 24, &tr!("printdlg-to")).enabled(several),
        page_box(TO, 198, pages.to_string()),
        Item::group("copiesgroup", COPIES_GROUP, &tr!("printdlg-copies")),
        Item::label_at(
            "copieslabel",
            cx,
            cy + 22,
            COPIES_GROUP.width - 24,
            &tr!("printdlg-number-of-copies"),
        ),
        Item::new(
            COPIES,
            Rect::new(cx, cy + 42, updown::WIDTH, updown::HEIGHT),
            Control::UpDown(copies),
        ),
        Item::button(
            PRINT,
            Rect::new(
                CLIENT.0 - PAD - 2 * BUTTON_W - 6,
                BUTTONS_Y,
                BUTTON_W,
                BUTTON_H,
            ),
            &tr!("printdlg-print"),
            live,
            true,
        ),
        Item::button(
            CANCEL,
            Rect::new(CLIENT.0 - PAD - BUTTON_W, BUTTONS_Y, BUTTON_W, BUTTON_H),
            &tr!("cancel"),
            true,
            false,
        ),
    ])
    .set_in(DialogFont::ShellDlg)
    .labels_at_ink();
    if let Ok(d) = dests {
        follow(&mut f, d);
    }
    f.focus_id(if live { PRINTER } else { CANCEL });
    f
}

/// Show the picked printer's status, location and description.
pub fn follow(f: &mut Form, dests: &[Dest]) {
    if let Some(d) = f.combo_selected(PRINTER).and_then(|i| dests.get(i)) {
        let (status, location, info) = (d.status(), d.location.clone(), d.info.clone());
        f.set_text(STATUS, &status);
        f.set_text(WHERE, &location);
        f.set_text(COMMENT, &info);
    }
}

/// [`follow`] for the change `id` the form reported: an edit of a page box picks Pages.
pub fn changed(f: &mut Form, dests: &[Dest], id: &str) {
    if id == FROM || id == TO {
        f.set_check(PAGES, true);
        f.set_check(ALL, false);
    }
    follow(f, dests);
}

/// The most pages one job prints. A text page deflates to a few hundred KB at 300 dpi, so the
/// job stays under about 150 MB and a few minutes of drawing; a longer document prints in ranges.
pub const MAX_PAGES: u32 = 500;

/// Why Print left the dialog open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The page boxes do not hold pages of a document of this many pages.
    BadRange { pages: u32 },
    /// The pages picked are more than [`MAX_PAGES`].
    TooMany,
}

impl Refusal {
    pub fn text(&self) -> String {
        match self {
            Refusal::BadRange { pages } => tr!("printdlg-bad-range", pages = *pages),
            Refusal::TooMany => tr!("printdlg-too-many", most = MAX_PAGES),
        }
    }
}

/// What the form says for a document of `pages` pages.
pub fn read(f: &Form, pages: u32) -> Result<Choice, Refusal> {
    let copies = match f.item(COPIES).map(|i| &i.control) {
        Some(Control::UpDown(u)) => u.value().max(1) as u32,
        _ => 1,
    };
    let range = if f.checked(PAGES) == Some(true) {
        let page = |id| f.edit_text(id).trim().parse::<u32>().ok();
        match (page(FROM), page(TO)) {
            (Some(a), Some(b)) if 1 <= a && a <= b && b <= pages => Range::Pages(a, b),
            _ => return Err(Refusal::BadRange { pages }),
        }
    } else {
        Range::All
    };
    if range.of(pages).len() > MAX_PAGES as usize {
        return Err(Refusal::TooMany);
    }
    Ok(Choice {
        dest: f.combo_selected(PRINTER).unwrap_or(0),
        copies,
        range,
    })
}

/// Send the user back to the From box with its text selected.
pub fn refused(f: &mut Form) {
    f.focus_id(FROM);
    f.select_edit_text(FROM);
}

#[cfg(test)]
mod tests;
