//! Fitting the sheet to its size, the page's scroll and where the parts go.

use crate::classic::widgets::{BarHit, ScrollParts};
use crate::controls::form::{self, Form, Item};
use crate::controls::pagelist;
use crate::controls::scrollbar::PageScroll;
use crate::controls::tabstrip;
use crate::look::Paint;
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::theme::TextUnits;

use super::{
    BUTTON_IDS, Buttons, Layout, MARGIN, Nav, PAGE_BOTTOM, PAGE_EDGE, PAGE_TOP, PropSheet, STRIP,
    buttons, is_button, shift,
};

impl PropSheet {
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
    pub(super) fn view(&self) -> form::View {
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
    pub(super) fn page_span(&self) -> (i32, i32) {
        let h = if self.bars().1 { self.bar_len() } else { 0 };
        let visible = self.shown().1 - PAGE_BOTTOM - self.page_top() - h;
        (visible, visible + self.range())
    }

    /// How much of the page shows across and how wide its controls reach, at 100%.
    pub(super) fn page_hspan(&self) -> (i32, i32) {
        let v = if self.bars().0 { self.bar_len() } else { 0 };
        let visible = self.shown().0 - 2 * MARGIN - self.page_dx() - v;
        (visible, visible + self.hrange())
    }

    /// How far the page scrolls down at 100%: as far as its controls reach past its bottom
    /// ([`PropSheet::reach`]), less the sideways bar's width while that shows.
    pub(super) fn range(&self) -> i32 {
        if !self.bars().0 {
            return 0;
        }
        let bar = if self.bars().1 { self.bar_len() } else { 0 };
        (self.reach().1 - (self.inside_end().1 - bar)).max(0)
    }

    /// How far the page scrolls sideways at 100%: as far as its controls reach past its right
    /// edge, less the bar down it while that shows.
    pub(super) fn hrange(&self) -> i32 {
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
    pub(super) fn scroll_to(&mut self, top: i32) -> bool {
        let top = top.clamp(0, self.range());
        let dy = top - self.scroll.top;
        self.scroll.top = top;
        self.shift_controls(0, -dy);
        dy != 0
    }

    /// Scroll the page's controls so `left` 100% px of the page are left of it, held to
    /// `0..=hrange`. True when they moved.
    pub(super) fn scroll_across(&mut self, left: i32) -> bool {
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
    /// the controls moved. A host that routes the page's keys itself calls it after each.
    pub fn reveal_focus(&mut self) -> bool {
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
    pub(super) fn page_bar_press(&mut self, hit: BarHit, across: bool) {
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
    pub(super) fn page_bar_drag(&mut self, x: i32, y: i32) -> Option<bool> {
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
    pub(super) fn bar_parts(&self, bar: Rect) -> ScrollParts {
        let (visible, total) = self.page_span();
        let thumb_min = self.open().controls().thumb_min;
        self.scroll.parts(bar, visible, total, thumb_min)
    }

    /// The page's sideways scrollbar's parts, laid out in `bar`.
    pub(super) fn hbar_parts(&self, bar: Rect) -> ScrollParts {
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
    pub(super) fn list_layout(&self) -> Option<pagelist::Layout> {
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
}
