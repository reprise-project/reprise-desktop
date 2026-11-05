use crate::classic::widgets::{LINE_H, bar_hit};
use crate::controls::form;
use crate::controls::hosted::Hosted;
use crate::controls::pagelist::{self, PageList, Pick};
use crate::controls::{searchfield, tabstrip};
use crate::text::TextRenderer;
use crate::textedit::EditKey;

use super::nav::{StripKey, nav_click, nav_key, page_clicked};
use super::{APPLY, CANCEL, Event, Hit, Nav, OK, PropSheet};

impl PropSheet {
    /// Scroll the open page's row fully into view in list mode. Opening a page
    /// through the sheet does this itself, [`PropSheet::set_open`] too. True when the rows moved.
    pub fn reveal_open(&mut self) -> bool {
        let open = self.tabs.open;
        self.on_list(|list, lay| list.reveal(Pick::Page(open), lay))
    }

    /// Open page `i` from the host, leaving the focus to the host: the list lets go of a link
    /// it rests on and scrolls the page's row into view.
    pub fn set_open(&mut self, i: usize) {
        self.tabs.open = i;
        self.drop_cursor();
        self.reveal_open();
    }

    /// Take the keyboard from the page, the list and the search field, for a host part beside
    /// the page that takes it.
    pub fn blur(&mut self) {
        self.open_mut().set_focus(None);
        self.tabs.focused = false;
        self.search_focused = false;
        self.drop_cursor();
    }

    /// Give the page list the keyboard from a host part beside the page, as Shift+Tab off the
    /// page's first stop does; with the open row hidden it opens the first shown page.
    pub fn focus_list(&mut self) {
        self.open_mut().set_focus(None);
        self.search_focused = false;
        self.tabs.focused = true;
        self.rest_on_shown();
    }

    /// Give the keyboard to control `id` on the open page, taking it from the page strip, and
    /// scroll the control into view.
    pub fn focus_control(&mut self, id: &str) {
        self.tabs.focused = false;
        self.search_focused = false;
        self.settle_cursor();
        self.open_mut().focus_id(id);
        self.reveal_focus();
    }

    /// The link the page list's keyboard rests on.
    fn cursor(&self) -> Option<usize> {
        match &self.nav {
            Nav::List(list) => list.cursor,
            Nav::Tabs => None,
        }
    }

    /// Let go of the list's cursor. True when there was one.
    fn drop_cursor(&mut self) -> bool {
        match &mut self.nav {
            Nav::List(list) => list.cursor.take().is_some(),
            Nav::Tabs => false,
        }
    }

    /// A list without the keyboard holds no cursor.
    fn settle_cursor(&mut self) {
        if !self.tabs.focused {
            self.drop_cursor();
        }
    }

    /// The search field lets go of the keyboard once the list or a page control holds it.
    fn settle_search(&mut self) {
        if self.tabs.focused || self.open().focus().is_some() {
            self.search_focused = false;
        }
    }

    /// The search field takes the keyboard from the list and the page, closing a dropped list.
    fn take_search(&mut self) {
        self.tabs.focused = false;
        self.drop_cursor();
        self.open_mut().dismiss();
        self.open_mut().set_focus(None);
        self.search_focused = true;
    }

    /// The search field takes the keyboard in the Tab cycle, selecting what it holds.
    fn tab_to_search(&mut self) {
        self.take_search();
        self.search.focus();
    }

    /// The focused edit: the search field's while it has the keyboard, else the open page's.
    pub fn focused_edit(&self) -> Option<form::FocusedEdit<'_>> {
        if self.search_focused {
            return Some(form::FocusedEdit::Line(&self.search.edit));
        }
        self.open().focused_edit()
    }

    /// Open tab `i` from the host, as a click does. True when another page opened.
    pub fn open_tab(&mut self, i: usize) -> bool {
        i != self.tabs.open && self.tab_click(i) == StripKey::Turned
    }

    /// The strip's part of a key, before the open page's (see `nav_key`). For a host
    /// that routes the page's keys itself.
    pub fn strip_key(&mut self, key: &form::Key, ctrl_tab: Option<bool>) -> StripKey {
        let tab = match key {
            form::Key::Tab { back } if ctrl_tab.is_none() && self.is_list() => Some(*back),
            _ => None,
        };
        // In list mode the search field is the stop before the list: Tab runs field, list,
        // page, field.
        match tab {
            Some(back) if self.search_focused => {
                if back {
                    if !self.open_mut().focus_edge(true) {
                        return StripKey::None;
                    }
                    self.search_focused = false;
                    return StripKey::Moved;
                }
                self.search_focused = false;
                self.tabs.focused = true;
                return self.rest_on_shown().unwrap_or(StripKey::Moved);
            }
            Some(true) if self.tabs.focused => {
                self.tab_to_search();
                return StripKey::Moved;
            }
            _ => {}
        }
        let was = self.tabs.focused;
        let list = match &mut self.nav {
            Nav::List(list) => Some(list),
            Nav::Tabs => None,
        };
        let r = nav_key(&mut self.tabs, &mut self.pages, key, ctrl_tab, list);
        if tab == Some(false) && !was && self.tabs.focused {
            self.tab_to_search();
        }
        let r = self.after_nav(r);
        if !was
            && self.tabs.focused
            && let Some(r) = self.rest_on_shown()
        {
            return r;
        }
        r
    }

    /// The list took the keyboard: with the open page's row hidden by the filter, it lands on
    /// the first shown row, so a row shows selected. `None` when it stays where it was.
    fn rest_on_shown(&mut self) -> Option<StripKey> {
        let Nav::List(list) = &self.nav else {
            return None;
        };
        if list.cursor.is_some() || list.shows(Pick::Page(self.tabs.open)) {
            return None;
        }
        let pick = self.first_shown()?;
        Some(self.land(pick))
    }

    /// The first live row the page list shows, where Home on it lands.
    fn first_shown(&self) -> Option<Pick> {
        match &self.nav {
            Nav::List(list) => list.target(self.tabs.open, &form::Key::Home),
            Nav::Tabs => None,
        }
    }

    /// The list takes the keyboard on `pick`: a page opens as a press on its row opens it, a
    /// link takes the list's cursor.
    fn land(&mut self, pick: Pick) -> StripKey {
        match pick {
            Pick::Page(k) => self.tab_click(k),
            Pick::Link(k) => {
                if let Nav::List(list) = &mut self.nav {
                    list.cursor = Some(k);
                }
                self.tabs.focused = true;
                self.open_mut().set_focus(None);
                self.after_nav(StripKey::Moved)
            }
        }
    }

    /// Ctrl+F ([`super::find_key`]): in list mode the search field takes the keyboard with its
    /// text selected.
    pub fn focus_search(&mut self) -> Event {
        if !self.is_list() {
            return Event::None;
        }
        self.tab_to_search();
        Event::Redraw
    }

    pub fn tab_click(&mut self, i: usize) -> StripKey {
        let list = self.is_list();
        let r = nav_click(&mut self.tabs, &mut self.pages, i, list);
        self.after_nav(r)
    }

    /// A page turned by the strip or the list takes the selection from a link, shows its row
    /// and starts unscrolled; a link the keys rest on shows its row.
    fn after_nav(&mut self, r: StripKey) -> StripKey {
        self.settle_cursor();
        self.settle_search();
        match (r, self.cursor()) {
            (StripKey::Turned, _) => {
                self.drop_cursor();
                self.reveal_open();
                self.scroll_to(0);
                self.scroll_across(0);
            }
            (StripKey::Moved, Some(k)) => {
                self.on_list(|list, lay| list.reveal(Pick::Link(k), lay));
            }
            _ => {}
        }
        r
    }

    pub fn page_clicked(&mut self) {
        let i = self.tabs.open;
        page_clicked(&mut self.tabs, &self.pages[i]);
        self.settle_cursor();
        self.settle_search();
    }

    /// A key for the sheet: the strip's keys ([`PropSheet::strip_key`]) first, the open page's
    /// after. `ctrl_tab` is `Some(back)` for Ctrl+Tab and Ctrl+Shift+Tab, which turn
    /// the tabs; `key` is then ignored.
    pub fn key(&mut self, key: form::Key, ctrl_tab: Option<bool>) -> Event {
        if ctrl_tab.is_none()
            && self.is_list()
            && let Some(ev) = self.field_key(&key)
        {
            return ev;
        }
        if let (true, None, form::Key::Edit(k)) = (self.search_focused, ctrl_tab, &key) {
            return self.search_key(k.clone());
        }
        if let (true, None, form::Key::DocEdge { end, select }) =
            (self.search_focused, ctrl_tab, &key)
        {
            let select = *select;
            return self.search_key(if *end {
                EditKey::End { select }
            } else {
                EditKey::Home { select }
            });
        }
        if key == form::Key::Enter
            && ctrl_tab.is_none()
            && self.tabs.focused
            && let Some(k) = self.cursor()
        {
            return Event::Link(k);
        }
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
        self.settle_search();
        match self.event(i, ev) {
            Event::None if self.reveal_focus() => Event::Redraw,
            ev => {
                self.reveal_focus();
                ev
            }
        }
    }

    /// The list-mode keys that reach the search field (see the module doc); `None` passes the
    /// key on. Enter on a link comes back as [`Event::Link`] and the field keeps the keyboard.
    fn field_key(&mut self, key: &form::Key) -> Option<Event> {
        match key {
            form::Key::Edit(k @ (EditKey::Insert(_) | EditKey::Backspace { .. }))
                if self.tabs.focused && !matches!(k, EditKey::Insert(s) if s == " ") =>
            {
                self.take_search();
                let end = self.search.text().len();
                self.search.edit.place(end, false);
                Some(self.search_key(k.clone()))
            }
            // An open list on the page takes Escape first.
            form::Key::Escape if !self.search.text().is_empty() && !self.open().popup_open() => {
                self.search.clear();
                Some(Event::Search)
            }
            form::Key::Enter if self.search_focused => Some(match self.first_shown() {
                _ if self.search.text().is_empty() => Event::None,
                Some(Pick::Link(k)) => Event::Link(k),
                Some(page) => self.land_key(page),
                None => Event::None,
            }),
            form::Key::Down if self.search_focused => Some(
                self.first_shown()
                    .map_or(Event::None, |pick| self.land_key(pick)),
            ),
            _ => None,
        }
    }

    fn land_key(&mut self, pick: Pick) -> Event {
        self.land(pick);
        self.reveal_focus();
        Event::Redraw
    }

    /// An editing key for the search field: [`Event::Search`] when its text changed.
    fn search_key(&mut self, k: EditKey) -> Event {
        let was = self.search.text().to_owned();
        self.search.key(k);
        if self.search.text() != was {
            Event::Search
        } else {
            Event::Redraw
        }
    }

    /// [`Form::shove`] on the open page; `None` while the strip has the focus.
    pub fn shove(&mut self, dir: crate::controls::arrange::Dir) -> Option<Event> {
        if self.tabs.focused || self.search_focused {
            return None;
        }
        let i = self.tabs.open;
        let ev = self.pages.get_mut(i)?.shove(dir)?;
        Some(self.event(i, ev))
    }

    pub fn paste(&mut self, text: String) -> Event {
        if self.search_focused {
            return if self.search.edit.paste(&text, &[]) {
                Event::Search
            } else {
                Event::None
            };
        }
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
        if let Some(field) = l.search.filter(|&f| searchfield::hit(f, x, y)) {
            let font = text.dialog(self.font);
            return Some(Hit::Search(self.search.caret_at(field, x, font, px, 1)));
        }
        if let (Nav::List(list), Some(lay)) = (&self.nav, &l.list)
            && lay.frame.contains(x, y)
        {
            if let Some(b) = pagelist::hit_bar(lay, x, y) {
                return Some(Hit::ListBar(b));
            }
            return Some(match pagelist::hit(list, lay, x, y) {
                Some(Pick::Page(k)) => Hit::Tab(k),
                Some(Pick::Link(k)) => Hit::Link(k),
                None => Hit::List,
            });
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
        if self.open().focus() != Some(h.item)
            || !(moved || self.tabs.focused || self.search_focused)
        {
            return Event::None;
        }
        self.tabs.focused = false;
        self.search_focused = false;
        self.settle_cursor();
        self.reveal_focus();
        Event::Redraw
    }

    /// A reader asked to see row `row` of the list at (`x`, `y`): the page list's item `row`
    /// ([`PageList::item`]), or a row of a list on the open page, scrolls in; nothing is picked
    /// or turned to.
    pub fn reveal(&mut self, x: i32, y: i32, row: usize, text: &TextRenderer, px: u32) -> Event {
        let moved = match self.hit(x, y, text, px) {
            Some(Hit::Tab(_) | Hit::Link(_) | Hit::List) => {
                self.on_list(|list, lay| list.item(row).is_some_and(|pick| list.reveal(pick, lay)))
            }
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
                let dropped = self.drop_cursor();
                return match self.tab_click(i) {
                    StripKey::None | StripKey::Pass if !dropped => Event::None,
                    _ => Event::Redraw,
                };
            }
            Some(Hit::Link(k)) => return self.link_click(k),
            Some(Hit::Search(i)) => {
                self.take_search();
                self.search.edit.place(i, false);
                return Event::Redraw;
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

    /// A press on link `k`: a live one takes the list's cursor and the keyboard and comes back
    /// as [`Event::Link`]; a greyed one does nothing.
    fn link_click(&mut self, k: usize) -> Event {
        let Nav::List(list) = &mut self.nav else {
            return Event::None;
        };
        if !list.links.get(k).is_some_and(|l| l.enabled) {
            return Event::None;
        }
        list.cursor = Some(k);
        self.tabs.focused = true;
        self.search_focused = false;
        self.open_mut().set_focus(None);
        Event::Link(k)
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
        if let Some(Hit::Search(_)) = hit {
            return Event::None;
        }
        if self.is_list()
            && matches!(
                hit,
                Some(Hit::Tab(_) | Hit::Link(_) | Hit::List | Hit::ListBar(_))
            )
        {
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
