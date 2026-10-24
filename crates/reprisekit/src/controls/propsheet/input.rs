use crate::classic::widgets::{LINE_H, bar_hit};
use crate::controls::form;
use crate::controls::pagelist::{self, PageList};
use crate::controls::tabstrip;
use crate::text::TextRenderer;

use super::nav::{StripKey, nav_click, nav_key, page_clicked};
use super::{APPLY, CANCEL, Event, Hit, Nav, OK, PropSheet};

impl PropSheet {
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

    /// A reader asked to see row `row` of the list at (`x`, `y`): the page list's page `row`, or
    /// a row of a list on the open page, scrolls in; nothing is picked or turned to.
    pub fn reveal(&mut self, x: i32, y: i32, row: usize, text: &TextRenderer, px: u32) -> Event {
        let moved = match self.hit(x, y, text, px) {
            Some(Hit::Tab(_) | Hit::List) => self.on_list(|list, lay| list.reveal(row, lay)),
            Some(Hit::Control(_)) => {
                self.open_mut().reveal(x, y, row, text, px) != form::Event::None
            }
            _ => false,
        };
        if moved { Event::Redraw } else { Event::None }
    }

    /// [`Form::right_press`] on the open page.
    pub fn right_press(&mut self, hit: Option<Hit>) -> Event {
        let i = self.tabs.open;
        let ev = self.pages[i].right_press(control(hit));
        self.page_clicked();
        self.event(i, ev)
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
    /// that scrolls only that way. A host that routes the wheel to the page itself calls it
    /// when the page took none.
    pub fn wheel_page(&mut self, rows: i32) -> Event {
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
        let bar = self.release_bars() || bar;
        let i = self.tabs.open;
        let ev = self.pages[i].release();
        match self.event(i, ev) {
            Event::None if bar => Event::Redraw,
            ev => ev,
        }
    }

    /// The pointer let go of the page's scrollbars: true when one was held or its look
    /// changed. [`PropSheet::release`] does it; a host that routes the page's release itself
    /// calls it first.
    pub fn release_bars(&mut self) -> bool {
        self.scroll.release() | self.hscroll.release()
    }

    /// Whether the pointer holds a thumb or an arrow of the page's scrollbars.
    pub fn holding_bar(&self) -> bool {
        self.scroll.holding()
            || self.hscroll.holding()
            || self.scroll.ticking()
            || self.hscroll.ticking()
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

pub(super) fn control(hit: Option<Hit>) -> Option<form::Hit> {
    match hit {
        Some(Hit::Control(h)) => Some(h),
        _ => None,
    }
}
