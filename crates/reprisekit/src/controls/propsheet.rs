//! A property sheet: a [`TabStrip`] over one [`Form`] per tab, with the OK /
//! Cancel / Apply row along the bottom. The sheet appends the three buttons to
//! every page, so a host builds only a page's own controls.
//!
//! For a client area of 398x424: the strip's top-left is (6, 8), the page runs
//! from (6, 28) to 6 px off each side and 36 px off the bottom, and the 75x23
//! buttons sit with their bottoms 7 px above the client's, 6 px apart, the last
//! 6 px from the right edge. Every sheet uses the same margins at its own size.
//!
//! [`PropSheet::with_list`] picks the page from a list ([`pagelist`]) instead.
//! For pages built for a 398x424 client the sheet is 544x424: the list's frame
//! runs from (6, 8), 140 px wide, down to the page's bottom; pages sit 146 px
//! right of where they were built; the open page's name is drawn in bold where
//! the tabs would be. The list is a tab stop like the strip. Up, Down, Home and
//! End on it open pages, and a page opened any way scrolls its row into view
//! ([`PropSheet::reveal_open`]). Ctrl+Tab, Ctrl+Page Down and their reverses step
//! pages in either mode ([`page_step`]).
//!
//! Tabs that wrap onto more rows push the page down 18 px a row; a host calls
//! [`PropSheet::fit_tabs`] to grow the sheet by as much.
//!
//! A client smaller than the sheet was built for ([`PropSheet::fit`]) keeps the buttons in its
//! bottom-right corner and scrolls the page's controls under the look's scrollbars, down the
//! page's right edge and along its bottom. A page whose notes rewrapped onto more lines
//! ([`Form::flowed`]) scrolls through what they pushed past its end the same way. A bar takes
//! its width from the page, so a control under it brings in the other bar.
//!
//! Pages are built in the client's coordinates (outer minus the 3 px frame and
//! 21 px caption) for 100% text. A host that draws the sheet inside a frame of
//! its own places the client with [`PropSheet::place`]; from then on the pages,
//! [`PropSheet::layout`] and [`PropSheet::hit`] are in that surface's coordinates.
//! The sheet grows with the text size it was last drawn at
//! ([`PropSheet::drawn_with`]) as its pages do: [`PropSheet::client`] is its size.
//!
//! The buttons are items on each page's [`Form`], so they get its Tab order and
//! access keys: Tab runs off the page's last control onto OK, Cancel and a live
//! Apply, then onto the tab strip and back into the page ([`strip_key`]). Alt+A
//! presses Apply.

use crate::classic::widgets::{BUTTON_H, BUTTON_W, BarHit};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::pagelist::{self, PageList};
use crate::controls::scrollbar::PageScroll;
use crate::controls::tabstrip::TabStrip;
use crate::raster::Rect;
use crate::text::DialogFont;

mod access_tree;
mod draw;
mod input;
mod layout;
mod nav;
pub use draw::{draw, draw_popups};
pub use nav::{StripKey, page_clicked, page_step, strip_key, tab_click};

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
pub const APPLY: &str = "apply";

const STRIP: (i32, i32) = (6, 8);
const MARGIN: i32 = 6;
/// The page's top with one row of tabs; each further row adds `tabstrip::ROW_H`.
const PAGE_TOP: i32 = 28;
const PAGE_BOTTOM: i32 = 36;
const BUTTON_BOTTOM: i32 = 7;
const BUTTON_PITCH: i32 = BUTTON_W + 6;
/// The page's bevel, which the scrollbar and the scrolled controls keep inside.
const PAGE_EDGE: i32 = 2;
const BUTTON_IDS: &[&str] = &[OK, CANCEL, APPLY];

/// Which buttons the sheet's row holds. Each set fills the slots from the right,
/// so OK / Cancel sit where Cancel and Apply would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buttons {
    OkCancelApply,
    OkCancel,
    Close,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Nav {
    Tabs,
    List(PageList),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PropSheet {
    pub tabs: TabStrip,
    pub pages: Vec<Form>,
    pub size: (i32, i32),
    /// The buttons on every page; [`Layout`] reports an empty rect for a slot the
    /// set leaves out.
    pub buttons: Buttons,
    /// The client's top-left on the surface the sheet is drawn on.
    origin: (i32, i32),
    page_dy: i32,
    /// The dialog font the whole sheet is set in: its first page's.
    pub font: DialogFont,
    pub nav: Nav,
    /// The page's scroll in a client shorter than the sheet ([`PropSheet::fit`]), at 100%:
    /// `short` is how much shorter than `size` the client is.
    scroll: PageScroll,
    /// The page's sideways scroll in a client narrower than the sheet, as `scroll`.
    hscroll: PageScroll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Page(usize, form::Event),
    Ok,
    Cancel,
    Apply,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Tab(usize),
    ListBar(BarHit),
    List,
    /// The page's scrollbar, while the page scrolls ([`PropSheet::fit`]).
    PageBar(BarHit),
    /// The page's sideways scrollbar, along its bottom.
    PageHBar(BarHit),
    Control(form::Hit),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub strip: (i32, i32),
    pub page: Rect,
    pub tabs: Vec<Rect>,
    pub list: Option<pagelist::Layout>,
    pub title: Rect,
    pub ok: Rect,
    pub cancel: Rect,
    pub apply: Rect,
    /// The page's scrollbar, while the page scrolls ([`PropSheet::fit`]).
    pub bar: Option<Rect>,
    /// The page's sideways scrollbar, while the page scrolls sideways.
    pub hbar: Option<Rect>,
}

fn buttons(size: (i32, i32)) -> (Rect, Rect, Rect) {
    let (w, h) = size;
    let y = h - BUTTON_BOTTOM - BUTTON_H;
    let x = w - MARGIN - BUTTON_W;
    let at = |k: i32| Rect::new(x - k * BUTTON_PITCH, y, BUTTON_W, BUTTON_H);
    (at(2), at(1), at(0))
}

fn shift(page: &mut Form, dx: i32, dy: i32) {
    for item in &mut page.items {
        item.rect.x += dx;
        item.rect.y += dy;
    }
}

fn is_button(it: &Item) -> bool {
    BUTTON_IDS.contains(&it.id)
}

impl PropSheet {
    /// A sheet of `pages`, one per label, at client `size`. Each page gains the
    /// sheet's buttons; Apply starts greyed. Panics unless there is one page per
    /// label.
    pub fn new(labels: &[&str], pages: Vec<Form>, size: (i32, i32)) -> PropSheet {
        PropSheet::with_buttons(labels, pages, size, Buttons::OkCancelApply)
    }

    pub fn with_buttons(
        labels: &[&str],
        pages: Vec<Form>,
        size: (i32, i32),
        buttons: Buttons,
    ) -> PropSheet {
        assert_eq!(labels.len(), pages.len(), "one page per tab");
        let mut sheet = PropSheet {
            tabs: TabStrip::new(labels, 0),
            pages: Vec::new(),
            size,
            buttons,
            origin: (0, 0),
            page_dy: 0,
            font: pages.first().map(|p| p.font).unwrap_or_default(),
            nav: Nav::Tabs,
            scroll: PageScroll::default(),
            hscroll: PageScroll::across(),
        };
        sheet.pages = pages.into_iter().map(|p| sheet.dress(p, false)).collect();
        sheet
    }

    /// A sheet whose pages are picked from a list down its left side ([`pagelist`]):
    /// `groups` are the headings, each with its pages' labels, and `pages` one per
    /// label in that order, built for a `client` sized sheet. The sheet is
    /// [`pagelist::ADVANCE`] wider. Labels carry no `&` access key. Panics unless
    /// there is one page per label.
    pub fn with_list(
        groups: &[(&str, &[&str])],
        pages: Vec<Form>,
        client: (i32, i32),
    ) -> PropSheet {
        let labels: Vec<&str> = groups.iter().flat_map(|(_, l)| l.iter().copied()).collect();
        assert_eq!(labels.len(), pages.len(), "one page per tab");
        let mut sheet = PropSheet {
            tabs: TabStrip::new(&labels, 0),
            pages: Vec::new(),
            size: (client.0 + pagelist::ADVANCE, client.1),
            buttons: Buttons::OkCancelApply,
            origin: (0, 0),
            page_dy: 0,
            font: pages.first().map(|p| p.font).unwrap_or_default(),
            nav: Nav::List(PageList::new(groups)),
            scroll: PageScroll::default(),
            hscroll: PageScroll::across(),
        };
        sheet.pages = pages.into_iter().map(|p| sheet.dress(p, false)).collect();
        sheet
    }

    fn is_list(&self) -> bool {
        matches!(self.nav, Nav::List(_))
    }

    fn page_dx(&self) -> i32 {
        if self.is_list() { pagelist::ADVANCE } else { 0 }
    }

    /// How far every page's rects have moved from where they were built, at 100%: the list's
    /// advance, the rows of tabs and the page's scroll. The origin is the pages' [`Form::origin`].
    pub fn page_offset(&self) -> (i32, i32) {
        (
            self.page_dx() - self.hscroll.top,
            self.page_dy - self.scroll.top,
        )
    }

    /// The client size at 100%: [`PropSheet::size`] less what [`PropSheet::fit`] cut.
    pub fn shown(&self) -> (i32, i32) {
        (
            self.size.0 - self.hscroll.short,
            self.size.1 - self.scroll.short,
        )
    }

    fn dress(&self, mut page: Form, apply: bool) -> Form {
        page.font = self.font;
        page.origin = self.origin;
        page.view = Some(self.view());
        let (dx, dy) = self.page_offset();
        shift(&mut page, dx, dy);
        let at = |r: Rect| r;
        let (ok, cancel, apply_r) = buttons(self.shown());
        let row = match self.buttons {
            Buttons::OkCancelApply => vec![
                Item::button(OK, at(ok), "OK", true, true),
                Item::button(CANCEL, at(cancel), "Cancel", true, false),
                Item::button(APPLY, at(apply_r), "&Apply", apply, false),
            ],
            Buttons::OkCancel => vec![
                Item::button(OK, at(cancel), "OK", true, true),
                Item::button(CANCEL, at(apply_r), "Cancel", true, false),
            ],
            Buttons::Close => vec![Item::button(OK, at(apply_r), "Close", true, true)],
        };
        page.items.extend(row);
        // A page with no controls of its own starts on OK.
        if page.focus().is_none() {
            page.focus_id(OK);
        }
        page
    }

    /// Replace page `i` with `page`, built in client coordinates: it gains the
    /// sheet's buttons, Apply as it stood on the page it replaces. The focus is
    /// `page`'s own, none while the strip holds it.
    pub fn set_page(&mut self, i: usize, page: Form) {
        let Some(old) = self.pages.get(i) else {
            return;
        };
        let apply = matches!(
            old.item(APPLY).map(|it| &it.control),
            Some(Control::Button { enabled: true, .. })
        );
        self.pages[i] = self.dress(page, apply);
        if self.tabs.focused && i == self.tabs.open {
            self.pages[i].set_focus(None);
        }
    }

    pub fn set_pages(&mut self, pages: Vec<Form>) {
        assert_eq!(pages.len(), self.pages.len(), "one page per tab");
        for (i, page) in pages.into_iter().enumerate() {
            self.set_page(i, page);
        }
    }

    pub fn open(&self) -> &Form {
        &self.pages[self.tabs.open]
    }

    pub fn open_mut(&mut self) -> &mut Form {
        &mut self.pages[self.tabs.open]
    }

    /// Grey or ungrey Apply on every page (the sheet shares one button row).
    pub fn set_apply_enabled(&mut self, on: bool) {
        for page in &mut self.pages {
            if let Some(Item {
                control: Control::Button { enabled, .. },
                ..
            }) = page.item_mut(APPLY)
            {
                *enabled = on;
            }
        }
    }
}

/// A status window: a sheet whose only button is Close, in the rightmost slot,
/// flush with the page's right edge. It carries the [`OK`] id, so Enter, Escape
/// and the button come back as [`Event::Ok`] or [`Event::Cancel`], treated alike.
/// Panics unless there is one page per label.
pub fn close_only(labels: &[&str], pages: Vec<Form>, size: (i32, i32)) -> PropSheet {
    PropSheet::with_buttons(labels, pages, size, Buttons::Close)
}

#[cfg(test)]
mod tests;
