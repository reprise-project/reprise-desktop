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

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BUTTON_H, BUTTON_W, BarHit, LINE_H, ScrollParts, bar_hit};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::pagelist::{self, PageList};
use crate::controls::scrollbar::PageScroll;
use crate::controls::tabstrip::{self, TabStrip};
use crate::look::Paint;
use crate::look::widgets::{draw_hscrollbar, draw_page, draw_scrollbar, group_label_ink};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

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
    fn shown(&self) -> (i32, i32) {
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

    /// Put the client's top-left at `origin` on the surface the sheet is drawn on,
    /// moving every control with it. Idempotent.
    pub fn place(&mut self, origin: (i32, i32)) {
        for page in &mut self.pages {
            page.origin = origin;
        }
        self.origin = origin;
    }

    /// The client's top-left on the surface the sheet is drawn on ([`PropSheet::place`]).
    pub fn origin(&self) -> (i32, i32) {
        self.origin
    }

    /// Lay the sheet out for `p`'s look and text size until it is drawn: a host that sizes
    /// its window from [`PropSheet::client`] or calls [`PropSheet::fit_tabs`] before the
    /// first draw calls this first. [`draw`] does it itself.
    pub fn drawn_with(&self, p: &Paint) {
        for page in &self.pages {
            page.drawn_with(p);
        }
    }

    /// The text size the sheet was last drawn at.
    fn units(&self) -> TextUnits {
        self.open().units()
    }

    /// The client size at the text size the sheet was last drawn at: [`PropSheet::size`] grown,
    /// less what [`PropSheet::fit`] cut.
    pub fn client(&self) -> (i32, i32) {
        self.units().size(self.shown())
    }

    /// Show the sheet in a client of `size` at 100%. Smaller than [`PropSheet::size`], the
    /// buttons keep to the client's bottom-right corner and the page scrolls through the rest;
    /// as large or larger, the sheet is whole. Idempotent; true when anything moved.
    pub fn fit(&mut self, size: (i32, i32)) -> bool {
        let short = ((self.size.0 - size.0).max(0), (self.size.1 - size.1).max(0));
        let (dx, dy) = (short.0 - self.hscroll.short, short.1 - self.scroll.short);
        if (dx, dy) != (0, 0) {
            self.hscroll.short = short.0;
            self.scroll.short = short.1;
            self.move_buttons(-dx, -dy);
        }
        let (left, top) = (self.hscroll.top, self.scroll.top);
        let moved = self.scroll_to(top) | self.scroll_across(left);
        moved || (dx, dy) != (0, 0)
    }

    /// The page's controls' 100% rects that show through the page: inside its bevel and off its
    /// scrollbars, from where the page starts to where it ends now.
    fn view(&self) -> form::View {
        let (right, bottom) = self.inside_end();
        let (v, h) = self.bars();
        let bar = self.bar_len();
        let top = self.page_top() + PAGE_EDGE;
        let x = MARGIN + self.page_dx() + PAGE_EDGE;
        let right = right - if v { bar } else { 0 };
        let bottom = bottom - if h { bar } else { 0 };
        form::View {
            rect: Rect::new(x, top, right - x, bottom - top),
            keep: BUTTON_IDS,
        }
    }

    /// Where the inside of the page's bevel ends right and down now, at 100%.
    fn inside_end(&self) -> (i32, i32) {
        let (w, h) = self.shown();
        (w - MARGIN - PAGE_EDGE, h - PAGE_BOTTOM - PAGE_EDGE)
    }

    /// The page's scrollbars' width at 100%: the look's, as the layout reserves it.
    fn bar_len(&self) -> i32 {
        self.pages
            .get(self.tabs.open)
            .map_or(0, |p| p.units().design(p.controls().scroll))
    }

    /// How far right and down the open page's controls reach unscrolled, at 100%, up to where
    /// the inside of the page as built ends, lower by what rewrapped notes pushed down
    /// ([`Form::flowed`]): a rect built past that edge is not scrolled to.
    fn reach(&self) -> (i32, i32) {
        let Some(page) = self.pages.get(self.tabs.open) else {
            return (0, 0);
        };
        let (right, bottom) = page
            .items
            .iter()
            .filter(|it| !is_button(it))
            .map(|it| page.flowed(it))
            .fold((0, 0), |(right, bottom), r| {
                (
                    right.max(r.right() + self.hscroll.top),
                    bottom.max(r.bottom() + self.scroll.top),
                )
            });
        let (w, h) = self.size;
        (
            right.min(w - MARGIN - PAGE_EDGE),
            bottom.min(h - PAGE_BOTTOM - PAGE_EDGE + self.flowed_down()),
        )
    }

    /// Which of the page's scrollbars show: down its right edge, along its bottom. A page cut
    /// short scrolls down; one whose controls reach past its right edge, or under the bar down
    /// it, scrolls sideways, and then down too if its controls reach under that bar.
    fn bars(&self) -> (bool, bool) {
        let (right, bottom) = self.inside_end();
        let (reach, low) = self.reach();
        let bar = self.bar_len();
        let v = low > bottom;
        let h = reach > right - if v { bar } else { 0 };
        (v || (h && low > bottom - bar), h)
    }

    /// The page's top at 100%, under the rows of tabs.
    fn page_top(&self) -> i32 {
        PAGE_TOP + self.page_dy
    }

    /// How much of the page shows and how tall its controls reach, at 100%.
    fn page_span(&self) -> (i32, i32) {
        let h = if self.bars().1 { self.bar_len() } else { 0 };
        let visible = self.shown().1 - PAGE_BOTTOM - self.page_top() - h;
        (visible, visible + self.range())
    }

    /// How much of the page shows across and how wide its controls reach, at 100%.
    fn page_hspan(&self) -> (i32, i32) {
        let v = if self.bars().0 { self.bar_len() } else { 0 };
        let visible = self.shown().0 - 2 * MARGIN - self.page_dx() - v;
        (visible, visible + self.hrange())
    }

    /// How far the page scrolls down at 100%: as far as its controls reach past its bottom
    /// ([`PropSheet::reach`]), less the sideways bar's width while that shows.
    fn range(&self) -> i32 {
        if !self.bars().0 {
            return 0;
        }
        let bar = if self.bars().1 { self.bar_len() } else { 0 };
        (self.reach().1 - (self.inside_end().1 - bar)).max(0)
    }

    /// How far the page scrolls sideways at 100%: as far as its controls reach past its right
    /// edge, less the bar down it while that shows.
    fn hrange(&self) -> i32 {
        let v = if self.bars().0 { self.bar_len() } else { 0 };
        (self.reach().0 - (self.inside_end().0 - v)).max(0)
    }

    /// How much further down the open page's controls reach than they were built to
    /// ([`Form::flowed`]), at 100%.
    fn flowed_down(&self) -> i32 {
        let Some(page) = self.pages.get(self.tabs.open) else {
            return 0;
        };
        let bottom = |r: fn(&Form, &Item) -> Rect| {
            page.items
                .iter()
                .filter(|it| !is_button(it))
                .map(|it| r(page, it).bottom())
                .max()
                .unwrap_or(0)
        };
        (bottom(Form::flowed) - bottom(|_, it| it.rect)).max(0)
    }

    /// Scroll the page's controls so `top` 100% px of the page are above it, held to
    /// `0..=range`. True when they moved.
    fn scroll_to(&mut self, top: i32) -> bool {
        let top = top.clamp(0, self.range());
        let dy = top - self.scroll.top;
        self.scroll.top = top;
        self.shift_controls(0, -dy);
        dy != 0
    }

    /// Scroll the page's controls so `left` 100% px of the page are left of it, held to
    /// `0..=hrange`. True when they moved.
    fn scroll_across(&mut self, left: i32) -> bool {
        let left = left.clamp(0, self.hrange());
        let dx = left - self.hscroll.top;
        self.hscroll.top = left;
        self.shift_controls(-dx, 0);
        dx != 0
    }

    fn shift_controls(&mut self, dx: i32, dy: i32) {
        for page in &mut self.pages {
            for it in page.items.iter_mut().filter(|it| !is_button(it)) {
                it.rect.x += dx;
                it.rect.y += dy;
            }
        }
        self.sync_view();
    }

    fn sync_view(&mut self) {
        let view = Some(self.view());
        for page in &mut self.pages {
            page.view = view;
        }
    }

    /// Scroll the page so its focused control shows whole, as little as it takes. True when
    /// the controls moved.
    fn reveal_focus(&mut self) -> bool {
        if self.tabs.focused || !self.scrolls() {
            return false;
        }
        let page = self.open();
        let Some(r) = page
            .focus()
            .map(|i| &page.items[i])
            .filter(|it| !is_button(it))
            .map(|it| page.flowed(it))
        else {
            return false;
        };
        let (top, (visible, _)) = (self.page_top() + PAGE_EDGE, self.page_span());
        let bottom = top + visible - 2 * PAGE_EDGE;
        let to = self.scroll.top + PageScroll::reveal((top, bottom), (r.y, r.bottom()));
        let (left, (across, _)) = (MARGIN + self.page_dx() + PAGE_EDGE, self.page_hspan());
        let right = left + across - 2 * PAGE_EDGE;
        let across = self.hscroll.top + PageScroll::reveal((left, right), (r.x, r.right()));
        self.scroll_to(to) | self.scroll_across(across)
    }

    /// Whether the page scrolls either way.
    fn scrolls(&self) -> bool {
        self.bars() != (false, false)
    }

    fn move_buttons(&mut self, dx: i32, dy: i32) {
        for page in &mut self.pages {
            for it in page.items.iter_mut().filter(|it| is_button(it)) {
                it.rect.x += dx;
                it.rect.y += dy;
            }
        }
    }

    /// How many rows the tabs take at `px` across the page as it shows.
    fn tab_rows(&self, text: &TextRenderer, px: u32) -> i32 {
        if self.is_list() {
            return 1;
        }
        let text = text.dialog(self.font);
        let u = self.units();
        let width = u.len(self.shown().0 - MARGIN) - u.len(MARGIN);
        tabstrip::row_count(text, px, &self.tabs, width) as i32
    }

    /// How far below their one-row place the pages sit for the rows of tabs, at 100%.
    fn rows_below(&self, text: &TextRenderer, px: u32) -> i32 {
        tabstrip::ROW_H * (self.tab_rows(text, px) - 1)
    }

    /// Make room for tabs that wrap onto more rows across the page as it shows: the sheet
    /// grows by the extra rows' height and every control, buttons included, moves down. A
    /// sheet [`PropSheet::fit`] cut keeps its client and scrolls through the rows' height.
    /// Call with the renderer the sheet is drawn with, before sizing the window from
    /// [`PropSheet::size`], and again after each [`PropSheet::fit`]. Idempotent. Returns how
    /// far the controls moved.
    pub fn fit_tabs(&mut self, text: &TextRenderer, px: u32) -> i32 {
        let dy = self.rows_below(text, px) - self.page_dy;
        if dy != 0 {
            let cut = (self.hscroll.short, self.scroll.short) != (0, 0);
            let client = self.shown();
            self.size.1 += dy;
            self.page_dy += dy;
            for page in &mut self.pages {
                shift(page, 0, dy);
            }
            self.sync_view();
            if cut {
                self.fit(client);
            }
        }
        dy
    }

    /// Make the client `size`, as a resizable window does: the button row keeps to
    /// the bottom-right corner and the page (and list) reach the new edges. The
    /// pages' own controls stay; a host lays them out ([`Form::take_layout`]).
    pub fn resize(&mut self, size: (i32, i32)) {
        let (dx, dy) = (size.0 - self.size.0, size.1 - self.size.1);
        if (dx, dy) == (0, 0) {
            return;
        }
        self.size = size;
        self.move_buttons(dx, dy);
        self.sync_view();
    }

    /// The page under `rows` rows of tabs, on the surface: the open tab's row runs into it.
    fn page(&self, rows: i32) -> Rect {
        let u = self.units();
        let (w, h) = self.size;
        let (ox, oy) = self.origin;
        let top = if self.is_list() {
            oy + u.len(PAGE_TOP)
        } else {
            self.strip().1 + PAGE_TOP - STRIP.1 - tabstrip::TAB_H + u.len(tabstrip::TAB_H) * rows
        };
        let x = ox + u.len(MARGIN + self.page_dx());
        let bottom = oy + u.len(h - self.scroll.short - PAGE_BOTTOM);
        let right = ox + u.len(w - self.hscroll.short - MARGIN);
        Rect::new(x, top, right - x, bottom - top)
    }

    fn strip(&self) -> (i32, i32) {
        let u = self.units();
        (
            self.origin.0 + u.len(STRIP.0),
            self.origin.1 + u.len(STRIP.1),
        )
    }

    /// The strip's top-left and the page under one row of tabs: the part of
    /// [`PropSheet::layout`] that needs no text.
    pub fn frame(&self) -> ((i32, i32), Rect) {
        (self.strip(), self.page(1))
    }

    pub fn layout(&self, text: &TextRenderer, px: u32) -> Layout {
        let page = self.page(self.tab_rows(text, px));
        let u = self.units();
        let (ox, oy) = self.origin;
        let at = |r: Rect| {
            let r = u.rect(r);
            Rect::new(r.x + ox, r.y + oy, r.width, r.height)
        };
        let (ok, cancel, apply) = buttons(self.shown());
        // A shorter set fills the rightmost slots; the empty rects are unreachable by
        // `draw` and `hit`.
        let none = Rect::new(0, 0, 0, 0);
        let (ok, cancel, apply) = match self.buttons {
            Buttons::OkCancelApply => (at(ok), at(cancel), at(apply)),
            Buttons::OkCancel => (at(cancel), at(apply), none),
            Buttons::Close => (at(apply), none, none),
        };
        let strip = self.strip();
        let (tabs, list, title) = match &self.nav {
            Nav::Tabs => (
                tabstrip::layout(
                    text.dialog(self.font),
                    px,
                    &self.tabs,
                    strip.0,
                    strip.1,
                    page.width,
                    u,
                ),
                None,
                none,
            ),
            Nav::List(_) => {
                let (top, bottom) = (u.len(STRIP.1 + 2), u.len(STRIP.1 + 2 + pagelist::ROW_H));
                let title = Rect::new(page.x, oy + top, page.width, bottom - top);
                (Vec::new(), self.list_layout(), title)
            }
        };
        let (bar, hbar) = self.bars_in(page);
        Layout {
            strip,
            page,
            tabs,
            list,
            title,
            ok,
            cancel,
            apply,
            bar,
            hbar,
        }
    }

    /// The page's scrollbars in `page`, while the page scrolls: down its right edge, and along
    /// its bottom up to the first.
    fn bars_in(&self, page: Rect) -> (Option<Rect>, Option<Rect>) {
        let (v, h) = self.bars();
        let w = self.open().controls().scroll;
        let inside = page.inset(PAGE_EDGE);
        let (vw, hw) = (if v { w } else { 0 }, if h { w } else { 0 });
        let bar = Rect::new(inside.right() - w, inside.y, w, inside.height - hw);
        let hbar = Rect::new(inside.x, inside.bottom() - w, inside.width - vw, w);
        (v.then_some(bar), h.then_some(hbar))
    }

    /// The page's scrollbars under the rows of tabs the sheet has made room for.
    fn page_bars(&self) -> (Option<Rect>, Option<Rect>) {
        self.bars_in(self.page(self.page_dy / tabstrip::ROW_H + 1))
    }

    /// A press on part `hit` of the page's scrollbar (its sideways one when `across`): an arrow
    /// scrolls a line, the track a page, the thumb is grabbed.
    fn page_bar_press(&mut self, hit: BarHit, across: bool) {
        if across {
            let (visible, _) = self.page_hspan();
            let to = self.hscroll.press(hit, visible);
            self.scroll_across(to);
        } else {
            let (visible, _) = self.page_span();
            let to = self.scroll.press(hit, visible);
            self.scroll_to(to);
        }
    }

    /// The pointer at (`x`, `y`) while a page thumb is held: the page follows it. `None`
    /// unless a thumb is held, else whether the page moved.
    fn page_bar_drag(&mut self, x: i32, y: i32) -> Option<bool> {
        let (bar, hbar) = self.page_bars();
        if self.hscroll.holding() {
            let parts = self.hbar_parts(hbar?);
            let (visible, total) = self.page_hspan();
            let left = self.hscroll.drag(&parts, (x, y), visible, total)?;
            return Some(self.scroll_across(left));
        }
        let parts = self.bar_parts(bar?);
        let (visible, total) = self.page_span();
        let top = self.scroll.drag(&parts, (x, y), visible, total)?;
        Some(self.scroll_to(top))
    }

    /// The page's scrollbar's parts, laid out in `bar`.
    fn bar_parts(&self, bar: Rect) -> ScrollParts {
        let (visible, total) = self.page_span();
        let thumb_min = self.open().controls().thumb_min;
        self.scroll.parts(bar, visible, total, thumb_min)
    }

    /// The page's sideways scrollbar's parts, laid out in `bar`.
    fn hbar_parts(&self, bar: Rect) -> ScrollParts {
        let (visible, total) = self.page_hspan();
        let thumb_min = self.open().controls().thumb_min;
        self.hscroll.parts(bar, visible, total, thumb_min)
    }

    /// The page's inside off its scrollbars on the surface while it scrolls, for a host that
    /// draws its own controls on the page: they clip to it. None while the page shows whole.
    pub fn page_clip(&self, text: &TextRenderer, px: u32) -> Option<Rect> {
        if !self.scrolls() {
            return None;
        }
        let l = self.layout(text, px);
        let r = l.page.inset(PAGE_EDGE);
        let w = l.bar.map_or(0, |b| b.width);
        let h = l.hbar.map_or(0, |b| b.height);
        Some(Rect::new(r.x, r.y, r.width - w, r.height - h))
    }

    /// `r`, in the coordinates the open page was built in, where the sheet shows it on the
    /// surface: moved with the page ([`PropSheet::page_offset`]) and grown with the text the
    /// page was last drawn at. For a control a host draws on the page.
    pub fn on_page(&self, r: Rect) -> Rect {
        let (dx, dy) = self.page_offset();
        let page = self.open();
        let r = page
            .units()
            .rect(Rect::new(r.x + dx, r.y + dy, r.width, r.height));
        Rect::new(r.x + page.origin.0, r.y + page.origin.1, r.width, r.height)
    }

    /// Whether the pointer at (`x`, `y`) reaches a control a host draws on the page: always while
    /// the page shows whole, else inside [`PropSheet::page_clip`].
    pub fn reaches_page(&self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        self.page_clip(text, px).is_none_or(|r| r.contains(x, y))
    }

    /// The page list's rects in list mode, which need no text; its scrollbar has the
    /// width of the look the open page was last drawn in.
    fn list_layout(&self) -> Option<pagelist::Layout> {
        let Nav::List(rows) = &self.nav else {
            return None;
        };
        let (ox, oy) = self.origin;
        let h = self.shown().1 - STRIP.1 - PAGE_BOTTOM;
        let r = self
            .units()
            .rect(Rect::new(MARGIN, STRIP.1, pagelist::WIDTH, h));
        let frame = Rect::new(r.x + ox, r.y + oy, r.width, r.height);
        Some(pagelist::layout(rows, frame, &self.open().controls()))
    }

    /// Scroll the open page's row fully into view in list mode. Opening a page
    /// through the sheet does this itself; a host that sets `tabs.open` calls it
    /// after. True when the rows moved.
    pub fn reveal_open(&mut self) -> bool {
        let open = self.tabs.open;
        self.on_list(|list, lay| list.reveal(open, lay))
    }

    /// Give the keyboard to control `id` on the open page, taking it from the page strip, and
    /// scroll the control into view.
    pub fn focus_control(&mut self, id: &str) {
        self.tabs.focused = false;
        self.open_mut().focus_id(id);
        self.reveal_focus();
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

    /// Open the next tab, or the previous when `back` (Ctrl+Tab, Ctrl+Shift+Tab).
    /// True when another page opened.
    pub fn cycle_tab(&mut self, back: bool) -> bool {
        self.strip_key(&form::Key::Tab { back }, Some(back)) == StripKey::Turned
    }

    /// Open tab `i` from the host, as a click does. True when another page opened.
    pub fn open_tab(&mut self, i: usize) -> bool {
        i != self.tabs.open && self.tab_click(i) == StripKey::Turned
    }

    /// The strip's part of a key, before the open page's: see [`strip_key`], and in
    /// list mode `nav_key`. For a host that routes the page's keys itself.
    pub fn strip_key(&mut self, key: &form::Key, ctrl_tab: Option<bool>) -> StripKey {
        let list = self.is_list();
        let r = nav_key(&mut self.tabs, &mut self.pages, key, ctrl_tab, list);
        if r == StripKey::Turned {
            self.reveal_open();
            self.scroll_to(0);
            self.scroll_across(0);
        }
        r
    }

    pub fn tab_click(&mut self, i: usize) -> StripKey {
        let list = self.is_list();
        let r = nav_click(&mut self.tabs, &mut self.pages, i, list);
        if r == StripKey::Turned {
            self.reveal_open();
            self.scroll_to(0);
            self.scroll_across(0);
        }
        r
    }

    pub fn page_clicked(&mut self) {
        let i = self.tabs.open;
        page_clicked(&mut self.tabs, &self.pages[i]);
    }

    /// A key for the sheet: the strip's keys ([`strip_key`]) first, the open page's
    /// after. `ctrl_tab` is `Some(back)` for Ctrl+Tab and Ctrl+Shift+Tab, which turn
    /// the tabs; `key` is then ignored.
    pub fn key(&mut self, key: form::Key, ctrl_tab: Option<bool>) -> Event {
        match self.strip_key(&key, ctrl_tab) {
            StripKey::Pass => {}
            StripKey::None => return Event::None,
            StripKey::Moved | StripKey::Turned => {
                self.reveal_focus();
                return Event::Redraw;
            }
        }
        let i = self.tabs.open;
        let ev = self.pages[i].key(key);
        match self.event(i, ev) {
            Event::None if self.reveal_focus() => Event::Redraw,
            ev => {
                self.reveal_focus();
                ev
            }
        }
    }

    /// [`Form::shove`] on the open page; `None` while the strip has the focus.
    pub fn shove(&mut self, dir: crate::controls::arrange::Dir) -> Option<Event> {
        if self.tabs.focused {
            return None;
        }
        let i = self.tabs.open;
        let ev = self.pages.get_mut(i)?.shove(dir)?;
        Some(self.event(i, ev))
    }

    pub fn paste(&mut self, text: String) -> Event {
        let i = self.tabs.open;
        let ev = self.pages[i].paste(text);
        self.event(i, ev)
    }

    /// What a drop on a field of the open page changed ([`Form::drop_text`]), as the sheet's.
    pub fn dropped(&self, ev: form::Event) -> Event {
        self.event(self.tabs.open, ev)
    }

    pub fn hit(&self, x: i32, y: i32, text: &TextRenderer, px: u32) -> Option<Hit> {
        let l = self.layout(text, px);
        if let (Nav::List(list), Some(lay)) = (&self.nav, &l.list)
            && lay.frame.contains(x, y)
        {
            if let Some(b) = pagelist::hit_bar(lay, x, y) {
                return Some(Hit::ListBar(b));
            }
            return Some(pagelist::hit(list, lay, x, y).map_or(Hit::List, Hit::Tab));
        }
        if let Some(i) = tabstrip::hit(&l.tabs, x, y) {
            return Some(Hit::Tab(i));
        }
        if let Some(bar) = l.bar.filter(|b| b.contains(x, y)) {
            return Some(Hit::PageBar(bar_hit(&self.bar_parts(bar), x, y, false)));
        }
        if let Some(bar) = l.hbar.filter(|b| b.contains(x, y)) {
            return Some(Hit::PageHBar(bar_hit(&self.hbar_parts(bar), x, y, true)));
        }
        self.open().hit(x, y, text, px).map(Hit::Control)
    }

    /// A reader moved the keyboard to the open page's control at (`x`, `y`) ([`Form::focus_at`]):
    /// the strip lets go of it and a scrolled page brings the control into view.
    pub fn focus_at(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> Event {
        let Some(Hit::Control(h)) = self.hit(x, y, text, px) else {
            return Event::None;
        };
        let moved = self.open_mut().focus_at(x, y, text, px) != form::Event::None;
        if self.open().focus() != Some(h.item) || !(moved || self.tabs.focused) {
            return Event::None;
        }
        self.tabs.focused = false;
        self.reveal_focus();
        Event::Redraw
    }

    pub fn click(&mut self, hit: Option<Hit>) -> Event {
        match hit {
            Some(Hit::Tab(i)) => {
                return match self.tab_click(i) {
                    StripKey::None | StripKey::Pass => Event::None,
                    StripKey::Moved | StripKey::Turned => Event::Redraw,
                };
            }
            Some(Hit::ListBar(b)) => {
                self.on_list(|list, lay| list.bar_press(b, lay));
                return Event::Redraw;
            }
            Some(Hit::PageBar(b)) => {
                self.page_bar_press(b, false);
                return Event::Redraw;
            }
            Some(Hit::PageHBar(b)) => {
                self.page_bar_press(b, true);
                return Event::Redraw;
            }
            Some(Hit::List) => return Event::None,
            _ => {}
        }
        let i = self.tabs.open;
        let ev = self.pages[i].click(control(hit));
        self.page_clicked();
        self.event(i, ev)
    }

    /// The pointer moved to (`x`, `y`) over `hit`: the closed tab under it lights,
    /// unless a part of the page is held; the page tracks the rest.
    pub fn motion(&mut self, x: i32, y: i32, hit: Option<Hit>) -> Event {
        let bar = match hit {
            Some(Hit::ListBar(b)) => Some(b),
            _ => None,
        };
        let (page_bar, page_hbar) = match hit {
            Some(Hit::PageBar(b)) => (Some(b), None),
            Some(Hit::PageHBar(b)) => (None, Some(b)),
            _ => (None, None),
        };
        let lit = self.on_list(|list, _| list.bar_hover(bar))
            | self.scroll.motion(page_bar)
            | self.hscroll.motion(page_hbar);
        if let Some(moved) = self.page_bar_drag(x, y) {
            return if moved || lit {
                Event::Redraw
            } else {
                Event::None
            };
        }
        if let Some(moved) = self.on_list(|list, lay| list.bar_drag(x, y, lay)) {
            return if moved || lit {
                Event::Redraw
            } else {
                Event::None
            };
        }
        let i = self.tabs.open;
        let tab = match hit {
            Some(Hit::Tab(t)) if !self.pages[i].holding() && !self.is_list() => Some(t),
            _ => None,
        };
        let lit = (self.tabs.hover(tab) && self.pages[i].styled()) || lit;
        let ev = self.pages[i].motion(x, y, control(hit));
        match self.event(i, ev) {
            Event::None if lit => Event::Redraw,
            ev => ev,
        }
    }

    /// The wheel over the page list scrolls it a row per row; elsewhere it is the
    /// open page's ([`Form::wheel`]).
    pub fn wheel(&mut self, hit: Option<Hit>, rows: i32) -> Event {
        if self.is_list() && matches!(hit, Some(Hit::Tab(_) | Hit::List | Hit::ListBar(_))) {
            let moved = self.on_list(|list, lay| list.scroll(rows * pagelist::ROW_H, lay));
            return if moved { Event::Redraw } else { Event::None };
        }
        match hit {
            Some(Hit::PageBar(_)) => return self.wheel_page(rows),
            Some(Hit::PageHBar(_)) => return self.wheel_across(rows),
            _ => {}
        }
        let i = self.tabs.open;
        match self.pages[i].wheel(control(hit), rows) {
            form::Event::None if !matches!(hit, Some(Hit::Tab(_))) => self.wheel_page(rows),
            ev => self.event(i, ev),
        }
    }

    /// The wheel scrolls a page that shows in part a line a row: down, or sideways on a page
    /// that scrolls only that way.
    fn wheel_page(&mut self, rows: i32) -> Event {
        if self.range() == 0 {
            return self.wheel_across(rows);
        }
        let to = self.scroll.top + rows * LINE_H;
        if self.scroll_to(to) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    fn wheel_across(&mut self, rows: i32) -> Event {
        let to = self.hscroll.top + rows * LINE_H;
        if self.scroll_across(to) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    pub fn leave(&mut self) -> Event {
        let bar =
            self.on_list(|list, _| list.bar_leave()) | self.scroll.leave() | self.hscroll.leave();
        let i = self.tabs.open;
        let lit = (self.tabs.hover(None) && self.pages[i].styled()) || bar;
        let ev = self.pages[i].leave();
        match self.event(i, ev) {
            Event::None if lit => Event::Redraw,
            ev => ev,
        }
    }

    pub fn release(&mut self) -> Event {
        let bar = match &mut self.nav {
            Nav::List(list) => list.bar_release(),
            Nav::Tabs => false,
        };
        let bar = self.scroll.release() | self.hscroll.release() || bar;
        let i = self.tabs.open;
        let ev = self.pages[i].release();
        match self.event(i, ev) {
            Event::None if bar => Event::Redraw,
            ev => ev,
        }
    }

    /// Whether the page list or the page has a scrollbar arrow held: the host sends
    /// [`PropSheet::tick`] until it lets go.
    pub fn ticking(&self) -> bool {
        self.scroll.ticking()
            || self.hscroll.ticking()
            || matches!(&self.nav, Nav::List(list) if list.repeating())
    }

    /// A host tick at `now`: a held arrow steps once it repeats, while the pointer is on it.
    pub fn tick(&mut self, now: std::time::Instant) -> Event {
        let page = self.scroll.tick(now).is_some_and(|to| self.scroll_to(to))
            | self
                .hscroll
                .tick(now)
                .is_some_and(|to| self.scroll_across(to));
        if self.on_list(|list, lay| list.bar_tick(now, lay)) || page {
            Event::Redraw
        } else {
            Event::None
        }
    }

    fn on_list<T: Default>(&mut self, f: impl FnOnce(&mut PageList, &pagelist::Layout) -> T) -> T {
        let Some(lay) = self.list_layout() else {
            return T::default();
        };
        match &mut self.nav {
            Nav::List(list) => f(list, &lay),
            Nav::Tabs => T::default(),
        }
    }

    /// The open page's event as the sheet's: its buttons (and the Enter and Escape
    /// that stand for them) are the sheet's, the rest the page's.
    fn event(&self, page: usize, ev: form::Event) -> Event {
        match ev {
            form::Event::Pressed(OK) | form::Event::Default => Event::Ok,
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Event::Cancel,
            form::Event::Pressed(APPLY) => Event::Apply,
            form::Event::None => Event::None,
            form::Event::Redraw => Event::Redraw,
            ev => Event::Page(page, ev),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripKey {
    Pass,
    None,
    Moved,
    Turned,
}

/// The strip's part of a sheet's keys, for any host of a [`TabStrip`] over one
/// [`Form`] per tab. The strip is a tab stop between the page's last control
/// (the buttons come last) and its first:
///
/// - Tab off the page's last tab stop, or Shift+Tab off its first, puts the focus
///   on the strip; the page holds none meanwhile.
/// - Tab on the strip goes to the page's first tab stop, Shift+Tab to its last.
/// - Left and Right open the neighbouring tab, stopping at the ends; the strip
///   keeps the focus. Up and Down do nothing.
/// - `ctrl_tab` (`Some(back)`) opens the next or previous tab wherever the focus is.
///
/// A tab opened with the focus on the strip keeps it there; one opened from the
/// page focuses the new page's first tab stop. Everything else, Enter and Escape
/// included, is [`StripKey::Pass`]: the page's default button or Cancel answers.
pub fn strip_key(
    tabs: &mut TabStrip,
    pages: &mut [Form],
    key: &form::Key,
    ctrl_tab: Option<bool>,
) -> StripKey {
    nav_key(tabs, pages, key, ctrl_tab, false)
}

/// [`strip_key`] for either way of picking the page. With `list` the focused
/// list takes Up, Down, Home and End ([`pagelist::target`]); Left and Right do
/// nothing; Page Up and Down go to the page.
fn nav_key(
    tabs: &mut TabStrip,
    pages: &mut [Form],
    key: &form::Key,
    ctrl_tab: Option<bool>,
    list: bool,
) -> StripKey {
    let was = tabs.open;
    if let Some(back) = ctrl_tab {
        return if tabs.cycle(back) {
            turned(tabs, pages, was);
            StripKey::Turned
        } else {
            StripKey::None
        };
    }
    let Some(page) = pages.get_mut(was) else {
        return StripKey::Pass;
    };
    // Hosts fold the arrows into editing keys for a focused field; on the
    // strip they are its own.
    let flat = match key {
        form::Key::Edit(k) if tabs.focused => form::flat_key(k),
        k => Some(k.clone()),
    };
    let Some(key) = flat else {
        return StripKey::Pass;
    };
    match key {
        form::Key::Tab { back } if tabs.focused => {
            if page.focus_edge(back) {
                tabs.focused = false;
                StripKey::Moved
            } else {
                StripKey::None
            }
        }
        // An open list takes Tab to close itself.
        form::Key::Tab { .. } if page.popup_open() => StripKey::Pass,
        form::Key::Tab { back } => {
            if !page.tab_no_wrap(back) {
                page.set_focus(None);
                tabs.focused = true;
            }
            StripKey::Moved
        }
        form::Key::Up | form::Key::Down | form::Key::Home | form::Key::End
            if tabs.focused && list =>
        {
            let to = pagelist::target(was, tabs.labels.len(), &key).unwrap_or(was);
            if to == was {
                return StripKey::None;
            }
            tabs.open = to;
            turned(tabs, pages, was);
            StripKey::Turned
        }
        form::Key::Left | form::Key::Right if tabs.focused && !list => {
            if tabs.step(key == form::Key::Right) {
                turned(tabs, pages, was);
                StripKey::Turned
            } else {
                StripKey::None
            }
        }
        form::Key::Left | form::Key::Right | form::Key::Up | form::Key::Down if tabs.focused => {
            StripKey::None
        }
        _ => StripKey::Pass,
    }
}

/// A press on tab `i`: a closed tab opens (see [`strip_key`]); the open one takes
/// the focus onto the strip.
pub fn tab_click(tabs: &mut TabStrip, pages: &mut [Form], i: usize) -> StripKey {
    nav_click(tabs, pages, i, false)
}

/// [`tab_click`] for either way of picking the page. With `list`, a press on a
/// closed page's row also focuses the list.
fn nav_click(tabs: &mut TabStrip, pages: &mut [Form], i: usize, list: bool) -> StripKey {
    if i >= tabs.labels.len() {
        return StripKey::None;
    }
    if i != tabs.open {
        let was = tabs.open;
        tabs.open = i;
        tabs.focused |= list;
        turned(tabs, pages, was);
        return StripKey::Turned;
    }
    if tabs.focused {
        return StripKey::None;
    }
    tabs.focused = true;
    if let Some(page) = pages.get_mut(i) {
        page.set_focus(None);
    }
    StripKey::Moved
}

/// Whether a key press steps the sheet's pages, as `ctrl_tab` for
/// [`PropSheet::key`]: `Some(false)` for Ctrl+Tab and Ctrl+Page Down,
/// `Some(true)` for Ctrl+Shift+Tab and Ctrl+Page Up.
pub fn page_step(sym: u32, ctrl: bool, shift: bool) -> Option<bool> {
    use crate::textedit::keysym;
    if !ctrl {
        return None;
    }
    match sym {
        keysym::TAB => Some(shift),
        // Shift+Tab arrives as its own keysym.
        keysym::ISO_LEFT_TAB => Some(true),
        keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => Some(false),
        keysym::PAGE_UP | keysym::KP_PAGE_UP => Some(true),
        _ => None,
    }
}

/// After a press on the open `page`: a control that took the focus takes it from
/// the strip.
pub fn page_clicked(tabs: &mut TabStrip, page: &Form) {
    if page.focus().is_some() {
        tabs.focused = false;
    }
}

/// The open tab changed from `was`: the page left behind releases the pointer,
/// the new tab is no longer lit, and the new page's first tab stop takes the
/// focus unless the strip holds it.
fn turned(tabs: &mut TabStrip, pages: &mut [Form], was: usize) {
    if let Some(old) = pages.get_mut(was) {
        old.cancel_pointer();
    }
    tabs.hot = None;
    if let Some(page) = pages.get_mut(tabs.open) {
        if tabs.focused {
            page.set_focus(None);
        } else {
            page.focus_edge(false);
        }
    }
}

fn control(hit: Option<Hit>) -> Option<form::Hit> {
    match hit {
        Some(Hit::Control(h)) => Some(h),
        _ => None,
    }
}

/// Draw the sheet onto a canvas already filled with face: the page, the tabs (or
/// the list and the open page's name), then the open page's controls and buttons.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sheet: &PropSheet,
    cues: bool,
    px: u32,
) {
    let s = p.s;
    // The list's scrollbar is the look's width, which the layout reads from the
    // open page: keep the look on every page so a page opened next lays the list
    // out as drawn.
    for page in &sheet.pages {
        page.drawn_with(p);
    }
    sheet.open().lay_out_notes(text, s, px);
    let l = sheet.layout(text, px);
    let page = l.page.scaled(s);
    draw_page(c, p, page);
    let nav_text = text.dialog(sheet.font);
    match (&sheet.nav, &l.list) {
        (Nav::List(list), Some(lay)) => {
            pagelist::draw(c, nav_text, p, list, &sheet.tabs, lay, cues, px);
            if let Some(name) = sheet.tabs.labels.get(sheet.tabs.open) {
                let ink = group_label_ink(p);
                nav_text.draw(c, l.title.scaled(s), name, true, ink, Align::Left, px);
            }
        }
        _ => tabstrip::draw(c, nav_text, p, &sheet.tabs, &l.tabs, l.page, cues, px),
    }
    form::draw(c, text, &p.on_page(page), sheet.open(), cues, px);
    if let Some(bar) = l.bar {
        let (visible, total) = sheet.page_span();
        let top = sheet.scroll.top as usize;
        let lit = sheet.scroll.lit();
        draw_scrollbar(
            c,
            p,
            bar.scaled(s),
            top,
            visible as usize,
            total as usize,
            lit,
        );
    }
    if let Some(bar) = l.hbar {
        let (visible, total) = sheet.page_hspan();
        let left = sheet.hscroll.top as usize;
        let lit = sheet.hscroll.lit();
        draw_hscrollbar(
            c,
            p,
            bar.scaled(s),
            left,
            visible as usize,
            total as usize,
            lit,
        );
    }
}

pub fn draw_popups(c: &mut Canvas, text: &TextRenderer, p: &Paint, sheet: &PropSheet, px: u32) {
    let page = sheet.layout(text, px).page.scaled(p.s);
    form::draw_popups(c, text, &p.on_page(page), sheet.open(), px);
}

impl PropSheet {
    /// The sheet's `access` part: the tab list (list mode), then the open page as a
    /// panel labelled by its tab or item. Returns the panel, for a host that draws
    /// controls of its own on the page, and the focused node.
    pub fn access(
        &self,
        tree: &mut Tree,
        parent: NodeId,
        text: &TextRenderer,
        px: u32,
    ) -> (NodeId, Option<NodeId>) {
        let l = self.layout(text, px);
        let (items, role) = match (&self.nav, &l.list) {
            (Nav::List(list), Some(lay)) => (
                pagelist::access(list, &self.tabs, lay, tree, parent, "pages"),
                Role::Group,
            ),
            _ => (
                tabstrip::access(&self.tabs, &l.tabs, tree, parent, "tabs"),
                Role::TabPanel,
            ),
        };
        let mut panel = access::node(role, l.page);
        if let Some(t) = items.get(self.tabs.open) {
            panel.push_labelled_by(*t);
        }
        // Keyed by the page, so a switch replaces the panel whole.
        let panel = tree.push(parent, ("page", self.tabs.open), panel);
        let focus = self.open().access(tree, panel, text, px);
        let nav = items.get(self.tabs.open).filter(|_| self.tabs.focused);
        (panel, nav.copied().or(focus))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::text::Align;
    use crate::theme::{FontConfig, Theme};

    /// A 398x424 sheet whose page's controls reach its bottom.
    fn tall_sheet() -> PropSheet {
        let page = Form::new(vec![
            Item::check("top", 20, 50, "Top"),
            Item::check("low", 20, 424 - PAGE_BOTTOM - PAGE_EDGE - 13, "Low"),
        ]);
        PropSheet::new(&["General"], vec![page], (398, 424))
    }

    fn sheet(labels: &[&str], size: (i32, i32)) -> PropSheet {
        let pages = labels
            .iter()
            .map(|l| {
                Form::new(vec![Item::new(
                    "check",
                    Rect::new(20, 50, 13, 13),
                    Control::Check {
                        label: (*l).into(),
                        checked: false,
                        enabled: true,
                        mixed: None,
                    },
                )])
            })
            .collect();
        PropSheet::new(labels, pages, size)
    }

    fn check_page(label: &str) -> Form {
        Form::new(vec![Item::new(
            "check",
            Rect::new(20, 50, 13, 13),
            Control::Check {
                label: label.into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        )])
    }

    const LIST_GROUPS: &[(&str, &[&str])] =
        &[("Look", &["Night Light", "Effects"]), ("Input", &["Mouse"])];

    fn list_sheet() -> PropSheet {
        let pages = ["Night Light", "Effects", "Mouse"]
            .into_iter()
            .map(check_page)
            .collect();
        PropSheet::with_list(LIST_GROUPS, pages, (398, 424))
    }

    /// A list sheet of 30 pages under one heading: 527 px of rows in the
    /// list's 376 px inside.
    fn long_list_sheet() -> PropSheet {
        let labels: Vec<String> = (0..30).map(|i| format!("Page {i}")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let pages = labels.iter().map(|l| check_page(l)).collect();
        PropSheet::with_list(&[("All", &labels)], pages, (398, 424))
    }

    fn list_top(s: &PropSheet) -> i32 {
        match &s.nav {
            Nav::List(l) => l.top,
            Nav::Tabs => unreachable!(),
        }
    }

    /// A resized list sheet keeps its buttons in the bottom-right corner
    /// and its page and list to the new edges; a page takes a template's
    /// rects by id, moved to where the pages sit, and keeps its state.
    #[test]
    fn a_resized_sheet_moves_its_buttons_and_pages_take_a_layout() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = list_sheet();
        let before = s.layout(&text, px);
        s.pages[0].set_check("check", true);
        s.resize((s.size.0 + 100, s.size.1 + 60));
        let after = s.layout(&text, px);
        assert_eq!(after.page.width, before.page.width + 100);
        assert_eq!(after.page.height, before.page.height + 60);
        assert_eq!(
            after.list.unwrap().frame.height,
            before.list.unwrap().frame.height + 60
        );
        for (b, a) in [
            (before.ok, after.ok),
            (before.cancel, after.cancel),
            (before.apply, after.apply),
        ] {
            assert_eq!((a.x, a.y), (b.x + 100, b.y + 60));
        }
        assert_eq!(s.pages[1].item(OK).unwrap().rect, after.ok);

        let template = Form::new(vec![Item::check("check", 40, 70, "")]);
        let offset = s.page_offset();
        s.pages[0].take_layout(&template, offset);
        let it = s.pages[0].item("check").unwrap();
        assert_eq!((it.rect.x, it.rect.y), (40 + offset.0, 70 + offset.1));
        assert_eq!(s.pages[0].checked("check"), Some(true), "the state stays");
        assert_eq!(
            s.pages[0].item(OK).unwrap().rect,
            after.ok,
            "no template item"
        );
    }

    /// The list's down arrow held: the sheet asks for ticks, repeats on
    /// them after the pause, lights the arrow under the pointer and stops
    /// at the release.
    #[test]
    fn a_held_list_arrow_repeats_on_the_hosts_ticks() {
        use crate::classic::widgets::BarHit;
        use crate::controls::scrollbar::REPEAT_AFTER;
        let mut s = long_list_sheet();
        let down = Some(Hit::ListBar(BarHit::Forward));
        assert_eq!(s.motion(135, 380, down), Event::Redraw, "lit");
        assert!(!s.ticking());
        assert_eq!(s.click(down), Event::Redraw);
        assert_eq!(list_top(&s), 17);
        assert!(s.ticking());
        let t0 = std::time::Instant::now();
        assert_eq!(s.tick(t0), Event::None, "the pause starts");
        assert_eq!(s.tick(t0 + REPEAT_AFTER), Event::Redraw);
        assert_eq!(list_top(&s), 34);
        assert_eq!(s.release(), Event::Redraw, "pressed back to lit");
        assert!(!s.ticking());
        assert_eq!(s.leave(), Event::Redraw, "lit back to rest");
    }

    /// The overflowing list scrolls under the wheel and its bar, and a
    /// page opened from the keys or by the host comes into view.
    #[test]
    fn a_long_page_list_scrolls_to_the_open_page() {
        use crate::classic::widgets::BarHit;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = long_list_sheet();
        let lay = s.layout(&text, px).list.unwrap();
        assert_eq!((lay.total, lay.inner.height), (527, 376));
        assert_eq!(lay.scrollbar, Some(Rect::new(128, 10, 16, 376)));
        assert_eq!(s.hit(30, 30, &text, px), Some(Hit::Tab(0)));
        assert_eq!(s.hit(30, 12, &text, px), Some(Hit::List), "the heading");
        assert_eq!(s.hit(135, 12, &text, px), Some(Hit::ListBar(BarHit::Back)));

        // The keys: End brings the last row's bottom to the inside's.
        assert_eq!(s.click(Some(Hit::Tab(0))), Event::Redraw);
        assert!(s.tabs.focused);
        assert_eq!(s.key(form::Key::End, None), Event::Redraw);
        assert_eq!((s.tabs.open, list_top(&s)), (29, 151));
        assert_eq!(s.hit(30, 380, &text, px), Some(Hit::Tab(29)));
        assert_eq!(s.key(form::Key::Home, None), Event::Redraw);
        assert_eq!(list_top(&s), 0);

        // The wheel over the list scrolls it a row per row; over the
        // page it is the page's.
        assert_eq!(s.wheel(Some(Hit::List), 3), Event::Redraw);
        assert_eq!(list_top(&s), 51);
        assert_eq!(s.wheel(None, 3), Event::None);
        assert_eq!(list_top(&s), 51);
        assert_eq!(s.click(Some(Hit::ListBar(BarHit::Back))), Event::Redraw);
        assert_eq!(list_top(&s), 34);

        // The thumb, grabbed and dragged to the bottom.
        let lay = s.layout(&text, px).list.unwrap();
        let thumb = pagelist::bar_parts(&lay).unwrap().thumb.unwrap();
        let Some(Hit::ListBar(grab)) = s.hit(135, thumb.y + 2, &text, px) else {
            panic!("the thumb is at {thumb:?}");
        };
        assert!(matches!(grab, BarHit::Thumb(_)));
        assert_eq!(
            s.click(Some(Hit::ListBar(grab))),
            Event::Redraw,
            "the thumb draws pressed"
        );
        assert_eq!(s.motion(135, 900, None), Event::Redraw);
        assert_eq!(list_top(&s), 151);
        assert_eq!(s.release(), Event::Redraw, "and at rest once let go");
        assert_eq!(s.motion(135, thumb.y, None), Event::None, "let go");
        assert_eq!(list_top(&s), 151);

        // A host opening a page itself reveals it after.
        s.tabs.open = 0;
        assert!(s.reveal_open());
        assert_eq!(list_top(&s), 0, "the first page brings its heading");
    }

    /// A list sheet is its pages' client plus the list: 146 px wider, the
    /// list from the strip's top-left down to the page's bottom, the page
    /// and its controls 146 px right of where they were built, the
    /// buttons in the whole sheet's bottom-right corner.
    #[test]
    fn a_list_sheets_geometry() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = list_sheet();
        assert_eq!(s.size, (544, 424));
        assert_eq!(s.fit_tabs(&text, px), 0);
        assert_eq!(s.size, (544, 424), "no tabs to wrap");
        let l = s.layout(&text, px);
        assert_eq!(l.page, Rect::new(152, 28, 386, 360));
        assert_eq!(
            l.list.as_ref().map(|l| l.frame),
            Some(Rect::new(6, 8, 140, 380))
        );
        assert_eq!(l.title, Rect::new(152, 10, 386, 17));
        assert!(l.tabs.is_empty());
        assert_eq!(
            (l.ok.x, l.cancel.x, l.apply.x, l.ok.y),
            (301, 382, 463, 394)
        );
        assert_eq!(s.page_offset(), (146, 0));
        assert_eq!(
            s.open().item("check").unwrap().rect,
            Rect::new(166, 50, 13, 13)
        );
        s.place((3, 21));
        assert_eq!(s.page_offset(), (146, 0), "the origin is the pages' own");
        assert_eq!(s.open().origin, (3, 21));
        let l = s.layout(&text, px);
        assert_eq!(l.list.map(|l| l.frame), Some(Rect::new(9, 29, 140, 380)));
        assert_eq!(s.open().rect("check"), Some(Rect::new(169, 71, 13, 13)));
        s.set_page(2, check_page("New"));
        assert_eq!(s.pages[2].rect("check"), Some(Rect::new(169, 71, 13, 13)));
        // A tab sheet's pages have not moved.
        assert_eq!(sheet(&["General"], (398, 424)).page_offset(), (0, 0));
    }

    #[test]
    #[should_panic(expected = "one page per tab")]
    fn a_list_row_without_a_page_is_refused() {
        PropSheet::with_list(LIST_GROUPS, vec![check_page("Night Light")], (398, 424));
    }

    /// The list is the tab stop the strip was: Shift+Tab off the page's
    /// first control reaches it, Up and Down open the page above or below
    /// across a heading and stop at the ends, Home and End open the first
    /// and the last, Left and Right do nothing, Ctrl+Tab still wraps, and
    /// Tab goes into the open page.
    #[test]
    fn the_lists_keys_step_the_pages() {
        use crate::textedit::EditKey;
        let mut s = list_sheet();
        let key = |s: &mut PropSheet, k| s.key(k, None);
        assert_eq!(key(&mut s, form::Key::Tab { back: true }), Event::Redraw);
        assert!(s.tabs.focused);
        assert_eq!(key(&mut s, form::Key::Down), Event::Redraw);
        assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
        assert_eq!(s.open().focus(), None, "the list keeps the focus");
        assert_eq!(key(&mut s, form::Key::Down), Event::Redraw);
        assert_eq!(s.tabs.open, 2, "past the Input heading");
        assert_eq!(key(&mut s, form::Key::Down), Event::None, "the last page");
        assert_eq!(key(&mut s, form::Key::Up), Event::Redraw);
        assert_eq!(s.tabs.open, 1);
        // Home and End reach a host as editing keys.
        assert_eq!(
            key(&mut s, form::Key::Edit(EditKey::Home { select: false })),
            Event::Redraw
        );
        assert_eq!(s.tabs.open, 0);
        assert_eq!(
            key(&mut s, form::Key::Edit(EditKey::End { select: false })),
            Event::Redraw
        );
        assert_eq!(s.tabs.open, 2);
        assert_eq!(key(&mut s, form::Key::Left), Event::None);
        assert_eq!(key(&mut s, form::Key::Right), Event::None);
        assert_eq!(s.tabs.open, 2);
        assert_eq!(
            s.key(form::Key::Tab { back: false }, Some(false)),
            Event::Redraw
        );
        assert_eq!(
            (s.tabs.open, s.tabs.focused),
            (0, true),
            "Ctrl+Tab wraps round"
        );
        assert_eq!(key(&mut s, form::Key::Tab { back: false }), Event::Redraw);
        assert!(!s.tabs.focused);
        assert_eq!(s.open().focus(), Some(0), "Tab goes into the page");
        key(&mut s, form::Key::Down);
        assert_eq!(s.tabs.open, 0, "the page's own control takes the arrows");
    }

    /// A press on a page's row opens that page and gives the list the
    /// focus; a press on a heading opens nothing; a row under the pointer
    /// lights nothing.
    #[test]
    fn a_press_on_a_row_opens_its_page_and_focuses_the_list() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = list_sheet();
        let rows = s.layout(&text, px).list.unwrap().rows;
        let at = |r: Rect| (r.x + 20, r.y + 5);
        let (x, y) = at(rows[2]);
        let hit = s.hit(x, y, &text, px);
        assert_eq!(hit, Some(Hit::Tab(1)), "row 2 is Effects, page 1");
        assert_eq!(s.click(hit), Event::Redraw);
        assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
        assert_eq!(s.open().focus(), None);
        assert_eq!(s.click(hit), Event::None, "open and focused already");
        let (x, y) = at(rows[3]);
        let heading = s.hit(x, y, &text, px);
        assert_eq!(heading, Some(Hit::List), "the Input heading");
        assert_eq!(s.click(heading), Event::None, "opens nothing");
        assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
        let check = s.hit(170, 55, &text, px);
        s.click(check);
        assert!(!s.tabs.focused, "the page's control takes the focus back");
        let (x, y) = at(rows[4]);
        s.motion(x, y, s.hit(x, y, &text, px));
        assert_eq!(s.tabs.hot, None);
    }

    /// In each look: the list exactly as `pagelist::draw` draws it at the
    /// sheet's layout, the open page's name over the page in the look's
    /// group label ink, the page's edges as `draw_page` draws them with no
    /// tab over the top edge, and on Classic OK's black default ring.
    #[test]
    fn a_list_sheet_draws_in_every_look() {
        use crate::interface::Interface;
        use crate::look::widgets::group_label_ink;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(i);
            let text = TextRenderer::load(&theme.fonts);
            let (p, px) = (theme.paint(), theme.fonts.size_px);
            let mut s = list_sheet();
            s.tabs.open = 1;
            let c = drawn(&s, &theme);
            let l = s.layout(&text, px);
            let lay = l.list.clone().unwrap();
            let Nav::List(list) = &s.nav else {
                unreachable!("a list sheet")
            };
            let mut want = Canvas::new(544, 424);
            want.fill_rect(want.bounds(), theme.colors.button_face);
            pagelist::draw(
                &mut want,
                text.dialog(s.font),
                &p,
                list,
                &s.tabs,
                &lay,
                false,
                px,
            );
            for y in lay.frame.y..lay.frame.bottom() {
                for x in lay.frame.x..lay.frame.right() {
                    assert_eq!(c.get(x, y), want.get(x, y), "{i:?}: the list at ({x}, {y})");
                }
            }
            let ink = group_label_ink(&p).argb();
            let t = l.title;
            assert!(
                (t.y..t.bottom()).any(|y| (t.x..t.x + 60).any(|x| c.get(x, y) == ink)),
                "{i:?}: the title in the group label ink"
            );
            assert_eq!(
                c.get(t.right() - 5, t.y + 8),
                theme.colors.button_face.argb()
            );
            let mut bare = Canvas::new(544, 424);
            bare.fill_rect(bare.bounds(), theme.colors.button_face);
            draw_page(&mut bare, &p, l.page);
            let (left, top) = ((l.page.x, l.page.y + 40), (l.page.x + 200, l.page.y));
            assert_eq!(
                c.get(left.0, left.1),
                bare.get(left.0, left.1),
                "{i:?}: the page's left edge"
            );
            assert_eq!(
                c.get(top.0, top.1),
                bare.get(top.0, top.1),
                "{i:?}: its top edge, bare"
            );
            if i == Interface::Classic {
                assert_eq!(
                    c.get(301, 394),
                    theme.colors.button_text.argb(),
                    "OK's default ring"
                );
            }
        }
    }

    #[test]
    fn a_readers_focus_on_a_page_control_takes_the_keyboard_from_the_list() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = list_sheet();
        s.tabs.focused = true;
        let check = s.open().index_of("check").unwrap();
        let r = s.open().items[check].rect;
        let (x, y) = (r.x + 4, r.y + r.height / 2);
        assert_eq!(s.focus_at(x, y, &text, px), Event::Redraw);
        assert!(!s.tabs.focused);
        assert_eq!(s.open().focus(), Some(check));
        assert_eq!(s.focus_at(x, y, &text, px), Event::None, "already there");
        let ok = s.open().index_of(OK).unwrap();
        let r = s.open().items[ok].rect;
        assert_eq!(s.focus_at(r.x + 4, r.y + 4, &text, px), Event::Redraw);
        assert_eq!(s.open().focus(), Some(ok), "OK takes it unpressed");
    }

    /// The open page's item names the page's panel, which holds its
    /// controls; with the list focused, the item has the focus.
    #[test]
    fn a_list_sheets_open_item_names_the_page_it_holds() {
        use crate::access::{ROOT, Tree, accesskit::Role};
        let text = TextRenderer::load(&FontConfig::default());
        let mut s = list_sheet();
        s.tabs.open = 1;
        let mut tree = Tree::window("T", s.size);
        let (panel, focus) = s.access(&mut tree, ROOT, &text, 11);
        let item = tree.find(Role::ListItem, "Effects").unwrap();
        assert_eq!(tree.get(item).unwrap().is_selected(), Some(true));
        assert_eq!(tree.get(panel).unwrap().role(), Role::Group);
        assert_eq!(tree.get(panel).unwrap().labelled_by(), &[item]);
        let check = tree.find(Role::CheckBox, "Effects").unwrap();
        assert!(tree.get(panel).unwrap().children().contains(&check));
        assert_eq!(focus, Some(check));
        assert!(!tree.nodes().any(|(_, n)| n.role() == Role::TabList));
        s.tabs.focused = true;
        s.pages[1].set_focus(None);
        let mut tree = Tree::window("T", s.size);
        let (_, focus) = s.access(&mut tree, ROOT, &text, 11);
        assert_eq!(focus, tree.find(Role::ListItem, "Effects"));
    }

    #[test]
    fn ctrl_tab_and_ctrl_page_keys_step_the_pages() {
        use crate::textedit::keysym;
        assert_eq!(page_step(keysym::TAB, true, false), Some(false));
        assert_eq!(page_step(keysym::TAB, true, true), Some(true));
        assert_eq!(page_step(keysym::ISO_LEFT_TAB, true, true), Some(true));
        assert_eq!(page_step(keysym::PAGE_DOWN, true, false), Some(false));
        assert_eq!(page_step(keysym::KP_PAGE_DOWN, true, false), Some(false));
        assert_eq!(page_step(keysym::PAGE_UP, true, false), Some(true));
        assert_eq!(page_step(keysym::KP_PAGE_UP, true, true), Some(true));
        assert_eq!(
            page_step(keysym::PAGE_DOWN, false, false),
            None,
            "the page's own"
        );
        assert_eq!(page_step(keysym::TAB, false, false), None);
        assert_eq!(page_step(u32::from(b'a'), true, false), None);
    }

    /// A Blue sheet hands its page to the controls on it: the blank that
    /// breaks the Recycle Bin Global page's group line (156x2 at 30,70 on
    /// a 338x376 sheet) shows the page there, and the group's line runs
    /// on past it.
    #[test]
    fn a_blank_on_a_blue_page_shows_the_page() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let blank = Rect::new(30, 70, 156, 2);
        let form = Form::new(vec![
            Item::group("group", Rect::new(21, 70, 296, 161), ""),
            Item::new("gap", blank, Control::Blank),
        ]);
        let sheet = PropSheet::new(&["Global"], vec![form], (338, 376));
        let p = theme.paint();
        let face = theme.colors.button_face;
        let mut c = Canvas::new(338, 376);
        c.fill_rect(c.bounds(), face);
        let mut bare = c.clone();
        draw(&mut c, &text, &p, &sheet, false, px);
        draw_page(&mut bare, &p, sheet.layout(&text, px).page);
        for y in blank.y..blank.bottom() {
            for x in blank.x..blank.right() {
                assert_eq!(c.get(x, y), bare.get(x, y), "({x}, {y}) is the page");
            }
        }
        assert_eq!(c.get(40, 70), 0xFFFA_FCFA, "the page's row 70");
        let line = crate::blue::Variant::Blue
            .scheme()
            .group_box
            .normal
            .rim
            .argb();
        assert_eq!(c.get(200, 70), line, "the group line past the blank");
    }

    /// Tabs on two rows grow the sheet by one row: the page keeps its size
    /// 18 px lower, and the controls and buttons move down with it.
    #[test]
    fn a_second_row_of_tabs_grows_the_sheet_by_a_row() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let labels = [
            "General",
            "Sharing",
            "Customize",
            "Security",
            "Previous Versions",
            "Summary",
            "Compatibility",
        ];
        let mut s = sheet(&labels, (361, 424));
        let before = s.layout(&text, px);
        assert_eq!(s.fit_tabs(&text, px), 18);
        assert_eq!(s.size, (361, 442));
        let l = s.layout(&text, px);
        assert_eq!(l.page, Rect::new(6, 46, 349, 360));
        assert_eq!(l.ok, Rect::new(before.ok.x, before.ok.y + 18, 75, 23));
        assert_eq!(
            s.open().item("check").unwrap().rect,
            Rect::new(20, 68, 13, 13)
        );
        let mut one = sheet(&["Themes", "Desktop"], (398, 424));
        assert_eq!(one.fit_tabs(&text, px), 0);
        assert_eq!(one.size, (398, 424));
    }

    /// Tabs on one row across the sheet as built wrap onto two in a client cut narrower: the
    /// page starts a row lower, the buttons keep to the client's bottom, and a client as wide
    /// as the sheet takes the row back.
    #[test]
    fn a_narrow_client_wraps_the_tabs_to_its_width() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let labels = [
            "General",
            "Sharing",
            "Customize",
            "Security",
            "Previous Versions",
            "Summary",
            "Compatibility",
        ];
        let mut s = sheet(&labels, (700, 424));
        s.drawn_with(&theme.paint());
        assert_eq!(s.fit_tabs(&text, px), 0);
        s.fit((361, 424));
        assert_eq!(s.fit_tabs(&text, px), 18);
        assert_eq!(s.fit_tabs(&text, px), 0, "idempotent");
        assert_eq!(s.client(), (361, 424), "the client keeps its size");
        let l = s.layout(&text, px);
        assert!(
            l.tabs.iter().all(|t| t.right() <= 361 - MARGIN),
            "every tab shows: {:?}",
            l.tabs
        );
        assert_eq!(l.page.y, 46, "the page starts under the second row");
        assert_eq!(l.ok.bottom(), 424 - BUTTON_BOTTOM);
        assert_eq!(s.open().rect("check").unwrap().y, 68);

        s.fit((700, 424));
        assert_eq!(s.fit_tabs(&text, px), -18);
        assert_eq!(s.size, (700, 424));
        assert_eq!(s.page_offset(), (0, 0));
        assert_eq!(s.layout(&text, px).ok.bottom(), 424 - BUTTON_BOTTOM);
    }

    /// A page cut short above where its last control ends shows whole: no scrollbar, and
    /// the wheel moves nothing.
    #[test]
    fn a_page_cut_short_scrolls_no_further_than_its_last_control() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let mut s = sheet(&["General"], (398, 424));
        s.drawn_with(&theme.paint());
        assert!(s.fit((398, 200)));
        assert_eq!(s.layout(&text, px).bar, None, "Check shows whole");
        assert_eq!(s.wheel(None, 3), Event::None);
        assert_eq!(s.page_offset(), (0, 0));
    }

    /// A sheet is set in its first page's font, every page and the tabs
    /// with it, and a page put in later takes it too.
    #[test]
    fn a_sheet_takes_its_first_pages_font() {
        let text = TextRenderer::load(&FontConfig::default())
            .with_shell_dlg(TextRenderer::stub(11, |_| 9));
        let second = text.dialog(DialogFont::ShellDlg);
        let pages = vec![
            Form::new(vec![]).set_in(DialogFont::ShellDlg),
            Form::new(vec![]),
        ];
        let mut s = PropSheet::new(&["General", "Web"], pages, (371, 424));
        assert_eq!(s.font, DialogFont::ShellDlg);
        s.set_page(1, Form::new(vec![]));
        assert!(s.pages.iter().all(|p| p.font == DialogFont::ShellDlg));
        let l = s.layout(&text, 11);
        let want = tabstrip::layout(
            second,
            11,
            &s.tabs,
            l.strip.0,
            l.strip.1,
            l.page.width,
            TextUnits::ONE,
        );
        assert_eq!(l.tabs, want);
        let ui = sheet(&["General", "Web"], (371, 424));
        assert_eq!(ui.font, DialogFont::Ui);
        assert_ne!(ui.layout(&text, 11).tabs, want, "the UI font's tabs");
    }

    #[test]
    fn display_properties_geometry() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let s = sheet(&["Themes", "Desktop", "Screen Saver"], (398, 424));
        let l = s.layout(&text, px);
        // The sheet's margins at a 398x424 client.
        assert_eq!(l.strip, (6, 8));
        assert_eq!(l.page, Rect::new(6, 28, 386, 360));
        assert_eq!(l.ok, Rect::new(155, 394, BUTTON_W, BUTTON_H));
        assert_eq!(l.cancel, Rect::new(236, 394, BUTTON_W, BUTTON_H));
        assert_eq!(l.apply, Rect::new(317, 394, BUTTON_W, BUTTON_H));
        // The open tab's bottom runs into the page's top edge.
        assert_eq!(l.tabs[0].y, 8);
        assert_eq!(l.tabs[0].bottom(), l.page.y);
    }

    #[test]
    fn the_audio_sheets_geometry() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
        let l = s.layout(&text, px);
        assert_eq!(l.page, Rect::new(6, 28, 349, 228));
        assert_eq!(l.ok, Rect::new(118, 262, BUTTON_W, BUTTON_H));
        assert_eq!(l.cancel, Rect::new(199, 262, BUTTON_W, BUTTON_H));
        assert_eq!(l.apply, Rect::new(280, 262, BUTTON_W, BUTTON_H));
    }

    #[test]
    fn every_page_gets_the_buttons_and_apply_greys_together() {
        let mut s = sheet(&["Volume", "Sounds"], (361, 292));
        let (ok, _, apply) = buttons(s.size);
        for page in &s.pages {
            assert_eq!(page.item(OK).map(|i| i.rect), Some(ok));
            assert_eq!(page.item(APPLY).map(|i| i.rect), Some(apply));
            assert_eq!(
                page.item(APPLY).map(|i| i.control.clone()),
                Some(Control::Button {
                    label: "&Apply".into(),
                    enabled: false,
                    default: false,
                }),
                "a sheet opens with nothing to apply"
            );
        }
        s.set_apply_enabled(true);
        assert!(s.pages.iter().all(|p| matches!(
            p.item(APPLY).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )));
    }

    #[test]
    fn close_only_lays_out_one_button() {
        let pages = vec![Form::new(vec![]), Form::new(vec![])];
        let s = close_only(&["General", "Support"], pages, (361, 419));
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        for page in &s.pages {
            let buttons: Vec<&Item> = page
                .items
                .iter()
                .filter(|i| matches!(i.control, Control::Button { .. }))
                .collect();
            assert_eq!(buttons.len(), 1, "one button per page");
            assert_eq!(buttons[0].id, OK);
            assert_eq!(
                buttons[0].control,
                Control::Button {
                    label: "Close".into(),
                    enabled: true,
                    default: true,
                }
            );
        }
        // At a 361x419 client, the Apply slot flush with the page's right
        // edge.
        let l = s.layout(&text, px);
        assert_eq!(l.ok, Rect::new(280, 389, BUTTON_W, BUTTON_H));
        assert_eq!(l.ok.right(), 361 - MARGIN);
        assert_eq!(l.cancel.width, 0);
        assert_eq!(l.apply.width, 0);
    }

    /// A placed sheet reports its rects and holds its controls at the
    /// origin; fitting and placing again move nothing.
    #[test]
    fn a_placed_sheet_moves_with_its_origin() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = sheet(&["General", "Advanced"], (338, 376));
        let before = s.layout(&text, px);
        s.place((3, 21));
        s.fit_tabs(&text, px);
        s.place((3, 21));
        assert_eq!(s.fit_tabs(&text, px), 0);
        let l = s.layout(&text, px);
        assert_eq!(l.strip, (9, 29));
        assert_eq!(l.page, Rect::new(9, 49, 326, 312));
        assert_eq!(l.ok, Rect::new(98, 367, BUTTON_W, BUTTON_H));
        assert_eq!(l.tabs[0].x, before.tabs[0].x + 3);
        assert_eq!(s.open().rect(OK), Some(l.ok));
        assert_eq!(s.open().rect("check"), Some(Rect::new(23, 71, 13, 13)));
        assert!(matches!(
            s.hit(24, 72, &text, px),
            Some(Hit::Control(form::Hit { item: 0, .. }))
        ));
    }

    /// A page set anew lands where the pages sit, under the sheet's
    /// buttons, and keeps Apply as it stood.
    #[test]
    fn a_page_set_anew_joins_the_sheet() {
        let mut s = sheet(&["General", "Advanced"], (338, 376));
        s.place((3, 21));
        s.set_apply_enabled(true);
        let page = Form::new(vec![Item::new(
            "new",
            Rect::new(20, 50, 13, 13),
            Control::Check {
                label: "New".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        )]);
        s.set_page(1, page);
        let p = &s.pages[1];
        assert_eq!(p.rect("new"), Some(Rect::new(23, 71, 13, 13)));
        assert_eq!(p.rect(OK), s.pages[0].rect(OK));
        assert!(matches!(
            p.item(APPLY).unwrap().control,
            Control::Button { enabled: true, .. }
        ));
    }

    /// OK and Cancel alone take the row's two right slots.
    #[test]
    fn ok_cancel_fills_the_right_slots() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let pages = vec![Form::new(vec![]), Form::new(vec![])];
        let s = PropSheet::with_buttons(&["General", "Web"], pages, (371, 424), Buttons::OkCancel);
        let l = s.layout(&text, px);
        assert_eq!(l.ok, Rect::new(209, 394, BUTTON_W, BUTTON_H));
        assert_eq!(l.cancel, Rect::new(290, 394, BUTTON_W, BUTTON_H));
        assert_eq!(l.apply.width, 0);
        for page in &s.pages {
            assert_eq!(page.items.len(), 2);
            assert_eq!(page.item(OK).unwrap().rect, l.ok);
            assert_eq!(page.item(CANCEL).unwrap().rect, l.cancel);
            assert_eq!(page.focus(), page.index_of(OK));
        }
    }

    #[test]
    #[should_panic(expected = "one page per tab")]
    fn a_tab_without_a_page_is_refused() {
        PropSheet::new(&["General", "Sharing"], vec![Form::new(vec![])], (361, 424));
    }

    #[test]
    #[should_panic(expected = "one page per tab")]
    fn a_close_only_tab_without_a_page_is_refused() {
        close_only(&["General", "Support"], vec![Form::new(vec![])], (361, 419));
    }

    #[test]
    fn ctrl_tab_cycles_the_pages() {
        let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
        assert_eq!(s.tabs.open, 0);
        assert_eq!(
            s.key(form::Key::Tab { back: false }, Some(false)),
            Event::Redraw
        );
        assert_eq!(s.tabs.open, 1);
        assert!(
            matches!(&s.open().item("check").unwrap().control,
                Control::Check { label, .. } if label == "Sounds"),
            "the open page followed the tab"
        );
        assert_eq!(
            s.key(form::Key::Tab { back: true }, Some(true)),
            Event::Redraw
        );
        assert_eq!(s.tabs.open, 0);
        assert_eq!(
            s.key(form::Key::Tab { back: true }, Some(true)),
            Event::Redraw
        );
        assert_eq!(s.tabs.open, 2, "wraps");
        // Without the Ctrl flag Tab walks the page's controls instead.
        let before = s.tabs.open;
        assert_eq!(s.key(form::Key::Tab { back: false }, None), Event::Redraw);
        assert_eq!(s.tabs.open, before);
    }

    #[test]
    fn the_sheets_buttons_become_sheet_events() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = sheet(&["Volume", "Sounds"], (361, 292));
        s.set_apply_enabled(true);
        let l = s.layout(&text, px);
        // Enter with the page's own control focused is the default button.
        assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
        assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
        // Press and release Apply.
        let hit = s.hit(l.apply.x + 5, l.apply.y + 5, &text, px);
        assert_eq!(s.click(hit), Event::Redraw, "the button goes down");
        assert_eq!(s.release(), Event::Apply);
        // A control of the page's own stays the page's.
        let check = s.open().index_of("check").unwrap();
        let hit = s.hit(
            s.open().items[check].rect.x + 3,
            s.open().items[check].rect.y + 3,
            &text,
            px,
        );
        assert!(matches!(hit, Some(Hit::Control(_))));
        assert_eq!(s.click(hit), Event::Redraw, "the box goes down");
        assert_eq!(s.release(), Event::Page(0, form::Event::Changed("check")));
    }

    #[test]
    fn a_tab_hit_opens_its_page() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = sheet(&["Volume", "Sounds"], (361, 292));
        let l = s.layout(&text, px);
        let hit = s.hit(l.tabs[1].x + 4, l.tabs[1].y + 10, &text, px);
        assert_eq!(hit, Some(Hit::Tab(1)));
        assert_eq!(s.click(hit), Event::Redraw);
        assert_eq!(s.tabs.open, 1);
        assert!(!s.tabs.focused, "a closed tab opens its page");
        assert_eq!(s.open().focus(), Some(0), "on its first control");
        assert_eq!(s.click(hit), Event::Redraw, "the open tab takes the focus");
        assert!(s.tabs.focused);
        assert_eq!(s.open().focus(), None);
        assert_eq!(s.click(hit), Event::None, "already focused");
        // A control pressed on the page takes the focus back.
        let check = s.hit(25, 55, &text, px);
        s.click(check);
        assert!(!s.tabs.focused);
        assert_eq!(s.open().focus(), Some(0));
    }

    /// Around the strip and back: Shift+Tab off the page's first control
    /// reaches the strip, Right and Left turn the tabs with the focus kept
    /// there, Up and Down do nothing, Ctrl+Tab turns them too, and Tab
    /// goes into the open page.
    #[test]
    fn the_strip_is_a_tab_stop_its_arrows_turn_the_tabs() {
        let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
        let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
        assert_eq!(s.open().focus(), Some(0));
        assert_eq!(tab(&mut s, true), Event::Redraw);
        assert!(s.tabs.focused);
        assert_eq!(s.open().focus(), None);
        assert_eq!(s.key(form::Key::Left, None), Event::None, "first tab");
        // The arrow as a host folds it for a field: an editing key.
        let right = form::Key::Edit(crate::textedit::EditKey::Right {
            select: false,
            word: false,
        });
        assert_eq!(s.key(right, None), Event::Redraw);
        assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
        assert_eq!(s.open().focus(), None, "the new page holds no focus");
        assert_eq!(s.key(form::Key::Down, None), Event::None);
        assert_eq!(s.key(form::Key::Up, None), Event::None);
        assert_eq!(s.tabs.open, 1);
        assert_eq!(
            s.key(form::Key::Tab { back: false }, Some(false)),
            Event::Redraw
        );
        assert_eq!((s.tabs.open, s.tabs.focused), (2, true));
        assert_eq!(s.key(form::Key::Right, None), Event::None, "last tab");
        // Enter on the strip is the default button, Escape Cancel.
        assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
        assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
        // Shift+Tab leaves for the page's last tab stop: Cancel, the
        // greyed Apply skipped.
        assert_eq!(tab(&mut s, true), Event::Redraw);
        assert!(!s.tabs.focused);
        assert_eq!(s.open().focus(), s.open().index_of(CANCEL));
        tab(&mut s, false);
        tab(&mut s, false);
        assert_eq!(s.open().focus(), Some(0), "Tab from the strip");
        // Ctrl+Tab from the page lands on the new page's first control,
        // whatever that page last had focused.
        s.pages[0].focus_id(OK);
        s.key(form::Key::Tab { back: false }, Some(false));
        assert_eq!((s.tabs.open, s.tabs.focused), (0, false));
        assert_eq!(s.open().focus(), Some(0));
    }

    // A folder Properties sheet at a 361x424 client and the pixel each
    // part of it lands on.
    /// 361x424 client (367x448 outer less the 3 px frame and 21 px caption).
    const FOLDER_SIZE: (i32, i32) = (361, 424);
    /// The open tab's white top ring.
    const FOLDER_STRIP_Y: i32 = 8;
    /// Right of the last tab: the page's white top ring, and its
    /// shadow/dk_shadow bottom.
    const FOLDER_PAGE_TOP: i32 = 28;
    const FOLDER_PAGE_BOTTOM: i32 = 387;
    /// The page's white left ring and its shadow/dk_shadow right.
    const FOLDER_PAGE_LEFT: i32 = 6;
    const FOLDER_PAGE_RIGHT: i32 = 354;
    /// OK's black default ring, then Cancel's and Apply's white ones.
    const FOLDER_OK_X: i32 = 118;
    const FOLDER_CANCEL_X: i32 = 199;
    const FOLDER_APPLY_X: i32 = 280;
    /// The button rings' top row; they run 23 rows.
    const FOLDER_BUTTON_Y: i32 = 394;

    /// Two of the compositor's sheets, in client coordinates: Recycle Bin
    /// Properties (344x400 outer) and a 404x448 outer sheet.
    const BIN: (i32, i32) = (338, 376);
    const BIN_PAGE: Rect = Rect::new(6, 28, 326, 312);
    const BIN_BUTTONS: (i32, i32, i32, i32) = (95, 176, 257, 346);
    const DISPLAY: (i32, i32) = (398, 424);
    const DISPLAY_PAGE: Rect = Rect::new(6, 28, 386, 360);
    const DISPLAY_BUTTONS: (i32, i32, i32, i32) = (155, 236, 317, 394);

    /// A folder-Properties-shaped page: a label, a check and a button of
    /// the page's own, so the Tab order has somewhere to start.
    fn folder_page(id: &'static str) -> Form {
        Form::new(vec![
            Item::new(
                "label",
                Rect::new(20, 50, 100, 13),
                Control::Label {
                    text: "Type:".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
            Item::new(
                id,
                Rect::new(20, 70, 13, 13),
                Control::Check {
                    label: "&Read-only".into(),
                    checked: false,
                    enabled: true,
                    mixed: None,
                },
            ),
            Item::new(
                "adv",
                Rect::new(250, 70, BUTTON_W, BUTTON_H),
                Control::Button {
                    label: "Ad&vanced...".into(),
                    enabled: true,
                    default: false,
                },
            ),
        ])
    }

    fn folder_sheet() -> PropSheet {
        PropSheet::new(
            &["General", "Sharing", "Customize"],
            vec![
                folder_page("ro"),
                folder_page("share"),
                folder_page("custom"),
            ],
            FOLDER_SIZE,
        )
    }

    #[test]
    fn folder_properties_geometry() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let l = folder_sheet().layout(&text, px);
        assert_eq!(l.tabs[0].y, FOLDER_STRIP_Y);
        assert_eq!(
            l.tabs[0].x, FOLDER_PAGE_LEFT,
            "the open tab starts at the margin"
        );
        assert_eq!((l.page.x, l.page.y), (FOLDER_PAGE_LEFT, FOLDER_PAGE_TOP));
        assert_eq!(l.page.right() - 1, FOLDER_PAGE_RIGHT);
        assert_eq!(l.page.bottom() - 1, FOLDER_PAGE_BOTTOM);
        for r in [l.ok, l.cancel, l.apply] {
            assert_eq!(
                (r.width, r.height, r.y),
                (BUTTON_W, BUTTON_H, FOLDER_BUTTON_Y)
            );
        }
        assert_eq!(
            (l.ok.x, l.cancel.x, l.apply.x),
            (FOLDER_OK_X, FOLDER_CANCEL_X, FOLDER_APPLY_X)
        );
        assert_eq!(l.apply.right(), FOLDER_SIZE.0 - MARGIN);
        assert_eq!(l.cancel.x - l.ok.right(), BUTTON_PITCH - BUTTON_W);
    }

    #[test]
    fn the_same_rule_gives_the_compositors_sheets() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        for (size, want_page, want_buttons) in [
            (BIN, BIN_PAGE, BIN_BUTTONS),
            (DISPLAY, DISPLAY_PAGE, DISPLAY_BUTTONS),
        ] {
            let mut s = folder_sheet();
            s.size = size;
            let l = s.layout(&text, px);
            assert_eq!(l.page, want_page, "page of {size:?}");
            let (ok, cancel, apply, y) = want_buttons;
            assert_eq!(
                (l.ok.x, l.cancel.x, l.apply.x, l.ok.y),
                (ok, cancel, apply, y),
                "buttons of {size:?}"
            );
            assert_eq!(l.strip, STRIP);
        }
    }

    #[test]
    fn tab_runs_off_the_page_onto_the_buttons_and_back() {
        let mut s = folder_sheet();
        let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
        let focus_id = |s: &PropSheet| s.open().focus().map(|i| s.open().items[i].id);
        // "adv" is the page's own last tab stop; Tab off it lands on OK.
        s.open_mut().focus_id("adv");
        assert_eq!(tab(&mut s, false), Event::Redraw);
        assert_eq!(focus_id(&s), Some(OK));
        assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
        assert_eq!(s.key(form::Key::Space, None), Event::Ok);
        // Shift+Tab off OK goes back to the page's last control.
        assert_eq!(tab(&mut s, true), Event::Redraw);
        assert_eq!(focus_id(&s), Some("adv"));
        // Forward: OK, Cancel, then the tab strip and round to the page's
        // first control: a greyed Apply is not a tab stop.
        tab(&mut s, false);
        tab(&mut s, false);
        assert_eq!(focus_id(&s), Some(CANCEL));
        assert_eq!(s.key(form::Key::Enter, None), Event::Cancel);
        tab(&mut s, false);
        assert!(s.tabs.focused, "skipped the greyed Apply onto the strip");
        assert_eq!(focus_id(&s), None, "the page holds no focus meanwhile");
        tab(&mut s, false);
        assert!(!s.tabs.focused);
        assert_eq!(focus_id(&s), Some("ro"));
        // Live, Apply joins the order after Cancel.
        s.set_apply_enabled(true);
        s.open_mut().focus_id("adv");
        tab(&mut s, false);
        tab(&mut s, false);
        tab(&mut s, false);
        assert_eq!(focus_id(&s), Some(APPLY));
        assert_eq!(s.key(form::Key::Enter, None), Event::Apply);
    }

    #[test]
    fn arrows_on_the_sheet_buttons_move_between_them() {
        let mut s = folder_sheet();
        let focus_id = |s: &PropSheet| s.open().focus().map(|i| s.open().items[i].id);
        s.open_mut().focus_id(OK);
        assert_eq!(s.key(form::Key::Right, None), Event::Redraw);
        assert_eq!(focus_id(&s), Some(CANCEL), "Right moves from OK to Cancel");
        assert_eq!(s.key(form::Key::Right, None), Event::None);
        assert_eq!(focus_id(&s), Some(CANCEL), "a greyed Apply is passed over");
        assert_eq!(s.key(form::Key::Enter, None), Event::Cancel);
        assert_eq!(s.key(form::Key::Left, None), Event::Redraw);
        assert_eq!(focus_id(&s), Some(OK), "Left moves back to OK");
        assert!(!s.tabs.focused, "the strip keeps out of it");
    }

    #[test]
    fn alt_a_presses_apply_only_when_it_is_live() {
        let mut s = folder_sheet();
        // Greyed, nothing answers the access key the button still draws.
        assert_eq!(s.key(form::Key::Char('a'), None), Event::None);
        s.set_apply_enabled(true);
        assert_eq!(s.key(form::Key::Char('A'), None), Event::Apply);
        assert_eq!(s.key(form::Key::Char('a'), None), Event::Apply);
        // A letter the page wants still goes to the page: "&Read-only".
        assert_eq!(
            s.key(form::Key::Char('r'), None),
            Event::Page(0, form::Event::Changed("ro"))
        );
        assert_eq!(s.key(form::Key::Char('z'), None), Event::None);
    }

    #[test]
    fn draws_the_folder_sheets_frames_and_buttons() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let s = folder_sheet();
        let mut c = Canvas::new(FOLDER_SIZE.0 as u32, FOLDER_SIZE.1 as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &s, false, px);
        // The page's rings.
        assert_eq!(
            c.get(FOLDER_PAGE_LEFT, FOLDER_PAGE_TOP + 40),
            cs.button_hilight.argb()
        );
        assert_eq!(
            c.get(FOLDER_PAGE_RIGHT, FOLDER_PAGE_TOP + 40),
            cs.button_dk_shadow.argb()
        );
        assert_eq!(
            c.get(FOLDER_PAGE_LEFT + 40, FOLDER_PAGE_BOTTOM),
            cs.button_dk_shadow.argb()
        );
        // The open tab's left ring, and the page's top ring surviving to
        // the right of the last tab.
        assert_eq!(
            c.get(FOLDER_PAGE_LEFT, FOLDER_STRIP_Y + 12),
            cs.button_hilight.argb()
        );
        assert_eq!(
            c.get(FOLDER_PAGE_RIGHT - 20, FOLDER_PAGE_TOP),
            cs.button_hilight.argb()
        );
        // OK wears the black default ring; Cancel and the greyed Apply
        // are plain raised buttons.
        assert_eq!(c.get(FOLDER_OK_X, FOLDER_BUTTON_Y), cs.button_text.argb());
        assert_eq!(
            c.get(FOLDER_CANCEL_X, FOLDER_BUTTON_Y),
            cs.button_hilight.argb()
        );
        assert_eq!(
            c.get(FOLDER_APPLY_X, FOLDER_BUTTON_Y),
            cs.button_hilight.argb()
        );
        assert_eq!(
            c.get(FOLDER_APPLY_X + BUTTON_W - 1, FOLDER_BUTTON_Y),
            cs.button_dk_shadow.argb()
        );
    }

    /// Writes target/propsheet-folder.png.
    #[test]
    #[ignore]
    fn render_propsheet_folder() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut s = folder_sheet();
        s.set_apply_enabled(true);
        let (w, h) = s.size;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &s,
            true,
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "propsheet-folder.png");
    }

    #[test]
    #[ignore]
    fn render_propsheet() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let s = preview_sheet();
        let (w, h) = s.size;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &s,
            true,
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "propsheet.png");
    }

    /// A Volume page like the audio sheet's, at its 361x292 client.
    fn preview_sheet() -> PropSheet {
        let page = Form::new(vec![
            Item::new(
                "device_group",
                Rect::new(14, 40, 321, 60),
                Control::Group("Device volume".into()),
            ),
            Item::new(
                "volume",
                Rect::new(28, 62, 180, crate::trackbar::HEIGHT),
                Control::TrackBar(crate::trackbar::TrackBar::new(10, 6)),
            ),
            Item::new(
                "mute",
                Rect::new(224, 66, 13, 13),
                Control::Check {
                    label: "&Mute".into(),
                    checked: false,
                    enabled: true,
                    mixed: None,
                },
            ),
            Item::new(
                "hint",
                Rect::new(14, 112, 321, 14),
                Control::Label {
                    text: "Move the slider to set the speaker volume.".into(),
                    enabled: true,
                    align: Align::Left,
                },
            ),
        ]);
        let blank = Form::new(vec![]);
        PropSheet::new(
            &["Volume", "Sounds", "Audio"],
            vec![page, blank.clone(), blank],
            (361, 292),
        )
    }

    /// The open tab is the selected one and names the page's panel, which
    /// holds the page's controls; switching tabs replaces the panel.
    #[test]
    fn the_open_tab_names_the_panel_it_holds() {
        use crate::access::{ROOT, Tree, accesskit::Role};
        let text = TextRenderer::load(&FontConfig::default());
        let mut s = sheet(&["General", "Advanced"], (398, 424));
        let access = |s: &PropSheet| {
            let mut tree = Tree::window("T", s.size);
            let (panel, focus) = s.access(&mut tree, ROOT, &text, 11);
            (tree, panel, focus)
        };
        let (tree, panel, focus) = access(&s);
        let tab = tree.find(Role::Tab, "General").unwrap();
        assert_eq!(tree.get(tab).unwrap().is_selected(), Some(true));
        assert_eq!(tree.get(panel).unwrap().labelled_by(), &[tab]);
        let check = tree.find(Role::CheckBox, "General").unwrap();
        assert!(tree.get(panel).unwrap().children().contains(&check));
        assert_eq!(focus, Some(check));
        s.tabs.open = 1;
        let (_, other, _) = access(&s);
        assert_ne!(panel, other);
    }

    /// A sheet drawn under `theme`, which tells its pages whether the look
    /// lights parts under the pointer.
    fn drawn(s: &PropSheet, theme: &Theme) -> Canvas {
        let text = TextRenderer::load(&theme.fonts);
        let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        draw(&mut c, &text, &theme.paint(), s, false, theme.fonts.size_px);
        c
    }

    /// Under Blue a closed tab lights under the pointer and draws its Hot
    /// frame; the open tab never lights, and a leave clears the lit tab.
    #[test]
    fn a_blue_sheet_lights_the_closed_tab_under_the_pointer() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
        let mut s = sheet(&["General", "Advanced"], (398, 424));
        drawn(&s, &theme);
        let t = s.layout(&text, px).tabs[1];
        let (x, y) = (t.x + 5, t.y + 5);
        assert_eq!(s.motion(x, y, s.hit(x, y, &text, px)), Event::Redraw);
        assert_eq!(s.tabs.hot, Some(1));
        let c = drawn(&s, &theme);
        let hot = &crate::blue::Variant::Blue.scheme().tab_item.hot;
        assert_eq!(
            c.get(t.x + 20, t.y),
            hot.mark.unwrap().argb(),
            "the hot tab's lit outline"
        );
        assert_eq!(
            c.get(t.x + 20, t.y + 1),
            hot.down[0].1.argb(),
            "the hot tab's light band"
        );
        let open = s.layout(&text, px).tabs[0];
        let (ox, oy) = (open.x + 5, open.y + 5);
        assert_eq!(s.motion(ox, oy, s.hit(ox, oy, &text, px)), Event::Redraw);
        assert_eq!(s.tabs.hot, None, "the open tab does not light");
        s.motion(x, y, s.hit(x, y, &text, px));
        assert_eq!(s.leave(), Event::Redraw);
        assert_eq!(s.tabs.hot, None);
        assert_eq!(s.leave(), Event::None);
    }

    /// A motion over a page's control lights it, a leave clears it, and a
    /// switch to another tab leaves the old page and the tab unlit.
    #[test]
    fn a_blue_sheet_lights_its_page_and_a_tab_switch_leaves_it() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
        let mut s = sheet(&["General", "Advanced"], (398, 424));
        drawn(&s, &theme);
        let lit = |s: &PropSheet| s.pages[0].lit(0, form::Light::Body);
        assert_eq!(s.motion(25, 55, s.hit(25, 55, &text, px)), Event::Redraw);
        assert_eq!(lit(&s), crate::pointer::Interaction::Hot);
        assert_eq!(s.leave(), Event::Redraw);
        assert_eq!(lit(&s), crate::pointer::Interaction::Idle);
        s.motion(25, 55, s.hit(25, 55, &text, px));
        let t = s.layout(&text, px).tabs[1];
        let hit = s.hit(t.x + 5, t.y + 5, &text, px);
        assert_eq!(s.click(hit), Event::Redraw);
        assert_eq!(s.tabs.open, 1);
        assert_eq!(s.tabs.hot, None, "the tab just opened is not lit");
        assert_eq!(
            lit(&s),
            crate::pointer::Interaction::Idle,
            "the page left behind"
        );
        // Ctrl+Tab with a button held lets the page left behind go of it.
        let ok = s.pages[1].index_of(OK).unwrap();
        let r = s.pages[1].items[ok].rect;
        s.click(s.hit(r.x + 5, r.y + 5, &text, px));
        assert!(s.pages[1].holding());
        s.key(form::Key::Tab { back: false }, Some(false));
        assert!(!s.pages[1].holding(), "the button is let go");
    }

    /// Classic has no hot tab: the strip tracks the pointer but asks for no
    /// redraw.
    #[test]
    fn a_classic_sheet_asks_no_redraw_for_a_tab_under_the_pointer() {
        let theme = Theme::default();
        let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
        let mut s = sheet(&["General", "Advanced"], (398, 424));
        let rest = drawn(&s, &theme);
        let t = s.layout(&text, px).tabs[1];
        let (x, y) = (t.x + 5, t.y + 5);
        assert_eq!(s.motion(x, y, s.hit(x, y, &text, px)), Event::None);
        assert!(drawn(&s, &theme) == rest, "a Classic tab draws the same");
    }

    /// At 150% and 200% a tabbed sheet grows from its origin: the client, the page and the
    /// buttons grow with the text, the tabs are as tall as the grown row with the open one
    /// running into the page, and a click on a grown button lands on it. 100% is the layout as
    /// built.
    #[test]
    fn a_grown_sheet_lays_its_page_tabs_and_buttons_out_with_the_text() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        for px in [11, 17, 22] {
            let u = TextUnits::new(px, 11);
            let mut s = sheet(&["General", "Sharing"], (398, 424));
            s.place((3, 21));
            let p = Paint {
                text: u,
                ..theme.paint()
            };
            s.drawn_with(&p);
            assert_eq!(s.client(), (u.len(398), u.len(424)));
            let l = s.layout(&text, px);
            let open = l.tabs[0];
            assert_eq!(open.height, u.len(tabstrip::TAB_H) + 2, "{px}");
            assert_eq!(
                open.bottom(),
                l.page.y,
                "{px}: the open tab runs into the page"
            );
            assert_eq!(l.page.x, 3 + u.len(MARGIN));
            assert_eq!(l.page.right(), 3 + u.len(398 - MARGIN));
            assert_eq!(l.apply.right(), 3 + u.len(398 - MARGIN));
            assert_eq!(l.apply.bottom(), 21 + u.len(424 - BUTTON_BOTTOM));
            let apply = s.open().rect(APPLY).unwrap();
            assert_eq!(
                apply, l.apply,
                "{px}: the page's button is where the layout says"
            );
            let mut c = Canvas::new(3 + u.len(398) as u32, 21 + u.len(424) as u32);
            draw(&mut c, &text, &p, &s, false, px);
            let ok = l.ok;
            let hit = s.hit(ok.x + ok.width / 2, ok.y + ok.height / 2, &text, px);
            assert!(matches!(hit, Some(Hit::Control(_))), "{px}");
            assert_eq!(s.click(hit), Event::Redraw);
            assert_eq!(s.release(), Event::Ok, "{px}");
        }
    }

    /// A list sheet at 150%: the list's rows grow with the text and its frame from the origin.
    #[test]
    fn a_grown_list_sheet_grows_its_rows() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let u = TextUnits::new(17, 11);
        let s = list_sheet();
        s.drawn_with(&Paint {
            text: u,
            ..theme.paint()
        });
        let l = s.layout(&text, 17);
        let list = l.list.unwrap();
        assert_eq!(list.row_h, u.len(17));
        assert_eq!(list.rows[1].y - list.rows[0].y, u.len(17));
        assert_eq!(list.frame.x, u.len(MARGIN));
        assert_eq!(l.page.x, u.len(pagelist::ADVANCE + MARGIN));
    }

    /// A 398x424 sheet shown 300 tall at 200%: the buttons keep to the client's bottom, the page
    /// ends above them with a scrollbar, a control below the page's end neither draws nor takes
    /// the pointer there, and the wheel or Tab onto it scrolls it into view, no further.
    #[test]
    fn a_short_client_keeps_its_buttons_and_scrolls_the_page() {
        let theme = Theme::default().with_form_text(200);
        let (u, px) = (theme.text, theme.fonts.size_px);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        let page = Form::new(vec![
            Item::check("top", 20, 50, "Top"),
            Item::check("low", 20, 350, "Low"),
        ]);
        let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
        s.drawn_with(&p);
        assert!(s.fit((398, 300)));
        assert!(!s.fit((398, 300)), "idempotent");
        assert_eq!(
            s.size,
            (398, 424),
            "the pages keep the size they were built for"
        );
        assert_eq!(s.client(), (u.len(398), u.len(300)));
        let l = s.layout(&text, px);
        assert_eq!(l.ok.bottom(), u.len(300 - BUTTON_BOTTOM));
        assert_eq!(l.page.bottom(), u.len(300 - PAGE_BOTTOM));
        let bar = l.bar.expect("a page that shows in part has a scrollbar");
        assert_eq!(bar.right(), l.page.right() - PAGE_EDGE);
        let low = s.open().rect("low").unwrap();
        assert!(low.y > l.page.bottom(), "Low starts below the page's end");
        assert!(
            s.hit(low.x + 4, low.y + low.height / 2, &text, px)
                .is_none(),
            "Low takes no pointer over the buttons' row"
        );
        let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
        draw(&mut c, &text, &p, &s, false, px);

        assert_eq!(s.wheel(None, 100), Event::Redraw);
        assert_eq!(
            s.page_offset(),
            (0, -101),
            "scrolled until Low's bottom meets the page's"
        );
        let low = s.open().rect("low").unwrap();
        assert!(
            low.y >= l.page.y && low.bottom() <= l.page.bottom(),
            "{low:?} shows"
        );
        let hit = s.hit(low.x + 4, low.y + low.height / 2, &text, px);
        assert!(
            matches!(hit, Some(Hit::Control(_))),
            "Low takes the pointer"
        );
        assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

        assert_eq!(s.wheel(None, -100), Event::Redraw);
        s.open_mut().focus_id("top");
        s.key(form::Key::Tab { back: false }, None);
        assert_eq!(s.open().focus().map(|i| s.open().items[i].id), Some("low"));
        let low = s.open().rect("low").unwrap();
        assert!(
            low.bottom() <= l.page.bottom(),
            "Tab onto Low brings it into view"
        );

        assert!(s.fit((398, 424)));
        assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
        assert_eq!(s.layout(&text, px).bar, None);
    }

    /// A note whose lines rewrap onto one more line than its rect holds pushes the controls under
    /// it down a line, and the page scrolls through that line though the sheet is whole.
    #[test]
    fn a_rewrapped_note_pushes_the_page_down_and_scrolls_it() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (px, p) = (theme.fonts.size_px, theme.paint());
        // The second line is the wider, so in any face it no longer fits once the first line's
        // last word wraps onto it.
        let lines = [
            "Sunset and sunrise are worked out",
            "for the time zone's city, not this machine's.",
        ];
        assert!(text.measure(lines[1], false, px) > text.measure(lines[0], false, px));
        let w = text.measure(lines[0], false, px) - 20;
        let page = Form::new(vec![
            Item::lines("note", Rect::new(20, 340, w, 2 * LINE_H), &lines),
            Item::check("low", 20, 373, "Low"),
        ]);
        let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
        s.drawn_with(&p);
        assert!(!s.fit((398, 424)), "the sheet is whole");
        let mut c = Canvas::new(398, 424);
        draw(&mut c, &text, &p, &s, false, px);
        let note = s.open().rect("note").unwrap();
        assert_eq!(note.height, 3 * LINE_H, "the note takes its third line");
        assert_eq!(s.open().rect("low").unwrap().y, 373 + LINE_H);
        assert_eq!(s.open().rect(OK).unwrap(), s.layout(&text, px).ok);
        assert!(s.layout(&text, px).bar.is_some(), "the page scrolls");
        assert_eq!(s.page_span().1 - s.page_span().0, LINE_H);
        assert_eq!(s.wheel(None, 100), Event::Redraw);
        assert_eq!(s.page_offset().1, -LINE_H, "through the pushed line");

        let roomy = Form::new(vec![
            Item::lines("note", Rect::new(20, 340, 360, 2 * LINE_H), &lines),
            Item::check("low", 20, 373, "Low"),
        ]);
        s.set_page(0, roomy);
        draw(&mut c, &text, &p, &s, false, px);
        s.fit((398, 424));
        assert_eq!(s.page_offset().1, 0, "nothing left to scroll");
        assert_eq!(
            s.open().rect("low").unwrap().y,
            373,
            "a note that fits moves nothing"
        );
        assert_eq!(s.layout(&text, px).bar, None);
    }

    /// A press on the page's scrollbar arrow scrolls a line; its thumb drags the page.
    #[test]
    fn the_page_scrollbar_scrolls_the_page() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = tall_sheet();
        s.drawn_with(&Theme::default().paint());
        s.fit((398, 300));
        let bar = s.layout(&text, px).bar.unwrap();
        let down = s.hit(bar.x + 2, bar.bottom() - 3, &text, px);
        assert_eq!(down, Some(Hit::PageBar(BarHit::Forward)));
        assert_eq!(s.click(down), Event::Redraw);
        assert_eq!(s.page_offset().1, -LINE_H);
        s.release();
        let parts = s.bar_parts(bar);
        let grab = s.hit(bar.x + 2, parts.thumb.unwrap().y + 2, &text, px);
        assert!(
            matches!(grab, Some(Hit::PageBar(BarHit::Thumb(_)))),
            "{grab:?}"
        );
        s.click(grab);
        assert_eq!(s.motion(bar.x + 2, bar.bottom(), None), Event::Redraw);
        assert_eq!(s.page_offset().1, -124, "dragged to the end");
        s.release();
    }

    /// The page's arrow held repeats on the host's ticks like the kit's other scrollbars.
    #[test]
    fn a_held_page_arrow_repeats_on_the_hosts_ticks() {
        use crate::controls::scrollbar::{REPEAT_AFTER, REPEAT_EVERY};
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut s = tall_sheet();
        s.drawn_with(&Theme::default().paint());
        s.fit((398, 300));
        let bar = s.layout(&text, px).bar.unwrap();
        let (x, y) = (bar.x + 2, bar.bottom() - 3);
        let down = s.hit(x, y, &text, px);
        s.motion(x, y, down);
        assert!(!s.ticking());
        s.click(down);
        assert!(s.ticking());
        assert_eq!(s.page_offset().1, -LINE_H);
        let t0 = std::time::Instant::now();
        assert_eq!(s.tick(t0), Event::None, "the pause starts");
        assert_eq!(s.tick(t0 + REPEAT_AFTER), Event::Redraw);
        assert_eq!(s.page_offset().1, -2 * LINE_H);
        assert_eq!(s.tick(t0 + REPEAT_AFTER + REPEAT_EVERY), Event::Redraw);
        assert_eq!(s.page_offset().1, -3 * LINE_H);
        s.motion(x - 40, y, None);
        assert_eq!(
            s.tick(t0 + REPEAT_AFTER + 2 * REPEAT_EVERY),
            Event::None,
            "off the arrow"
        );
        s.release();
        assert!(!s.ticking());
    }

    /// A 398x424 sheet shown 300 wide at 200%: the buttons keep to the client's bottom-right
    /// corner, the page ends at its right edge with a bar along its bottom, and the bar's arrow,
    /// or Tab onto a control past that edge, scrolls the page sideways.
    #[test]
    fn a_narrow_client_keeps_its_buttons_and_scrolls_the_page_sideways() {
        let theme = Theme::default().with_form_text(200);
        let (u, px) = (theme.text, theme.fonts.size_px);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        let page = Form::new(vec![
            Item::check("left", 20, 50, "Left"),
            Item::button("right", Rect::new(300, 50, 75, 23), "Right", true, false),
        ]);
        let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
        s.drawn_with(&p);
        assert!(s.fit((300, 424)));
        assert!(!s.fit((300, 424)), "idempotent");
        assert_eq!(
            s.size,
            (398, 424),
            "the pages keep the size they were built for"
        );
        assert_eq!(s.client(), (u.len(300), u.len(424)));
        let l = s.layout(&text, px);
        assert_eq!(l.apply.right(), u.len(300 - MARGIN));
        assert_eq!(l.page.right(), u.len(300 - MARGIN));
        assert_eq!(l.bar, None, "the page shows whole down");
        let hbar = l.hbar.expect("a page cut narrow has a sideways bar");
        assert_eq!(hbar.bottom(), l.page.bottom() - PAGE_EDGE);
        assert_eq!(hbar.right(), l.page.right() - PAGE_EDGE);
        let mut c = Canvas::new(u.len(300) as u32, u.len(424) as u32);
        draw(&mut c, &text, &p, &s, false, px);
        let right = s.open().rect("right").unwrap();
        assert!(
            right.right() > hbar.right(),
            "Right reaches past the page's edge"
        );

        let arrow = s.hit(hbar.right() - 3, hbar.y + 2, &text, px);
        assert_eq!(arrow, Some(Hit::PageHBar(BarHit::Forward)));
        assert_eq!(s.click(arrow), Event::Redraw);
        s.release();
        assert_eq!(s.page_offset(), (-LINE_H, 0));

        s.open_mut().focus_id("left");
        s.key(form::Key::Tab { back: false }, None);
        assert_eq!(
            s.open().focus().map(|i| s.open().items[i].id),
            Some("right")
        );
        let right = s.open().rect("right").unwrap();
        assert!(
            right.right() <= hbar.right(),
            "Tab onto Right brings it into view: {right:?}"
        );
        assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

        assert!(s.fit((398, 424)));
        assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
        assert_eq!(s.layout(&text, px).hbar, None);
    }

    /// A page cut short at 200% scrolls under a bar down its right edge. A control flush with
    /// that edge would lie under the bar, so the page scrolls sideways by the bar's width as
    /// well; one that keeps the page's margin leaves the page whole across.
    #[test]
    fn the_page_scrollbar_covers_no_control() {
        let theme = Theme::default().with_form_text(200);
        let (u, px) = (theme.text, theme.fonts.size_px);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        let edge = 398 - MARGIN - PAGE_EDGE;
        let at = |right: i32| {
            let page = Form::new(vec![
                Item::check("top", 20, 50, "Top"),
                Item::check("low", 20, 373, "Low"),
                Item::button(
                    "flush",
                    Rect::new(right - 75, 100, 75, 23),
                    "Flush",
                    true,
                    false,
                ),
            ]);
            let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
            s.drawn_with(&p);
            s.fit((398, 300));
            s
        };
        let mut s = at(edge);
        let l = s.layout(&text, px);
        let bar = l.bar.expect("a page cut short has a scrollbar");
        let hbar = l
            .hbar
            .expect("a control under the bar brings in the sideways bar");
        assert_eq!(hbar.right(), bar.x, "the bars meet at the corner");
        assert_eq!(bar.bottom(), hbar.y);
        assert_eq!(s.hrange(), u.design(s.open().controls().scroll));
        assert_eq!(
            s.wheel(Some(Hit::PageHBar(BarHit::Track)), 100),
            Event::Redraw
        );
        let flush = s.open().rect("flush").unwrap();
        assert!(
            flush.right() <= bar.x,
            "{flush:?} clears the bar at {bar:?}"
        );
        let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
        draw(&mut c, &text, &p, &s, false, px);

        let s = at(edge - 12);
        assert!(s.layout(&text, px).bar.is_some());
        assert_eq!(s.layout(&text, px).hbar, None, "the margin holds the bar");

        let mut s = at(edge + 30);
        s.fit((398, 424));
        let l = s.layout(&text, px);
        assert_eq!(
            (l.bar, l.hbar),
            (None, None),
            "a whole sheet does not scroll to a rect built past its page"
        );
    }
}
