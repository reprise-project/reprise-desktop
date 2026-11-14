//! Focus, keys and pointer for the list views beside a page's form.

use crate::registry::Page;
use crate::{SettingsApp, defaultspage, pages, registry};
use repriseapp::{Ctx, Reply};
use reprisekit::controls::form;
use reprisekit::controls::listview::{self, ListView, Took};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::raster::Rect;

/// Give a side list the keyboard or take it away; the dotted rectangle goes with it.
pub(crate) fn hand_focus(sheet: &mut PropSheet, list: &mut ListView, held: &mut bool, on: bool) {
    *held = on;
    list.focus_ring = on;
    if on {
        sheet.blur();
    }
}

impl SettingsApp {
    pub(crate) fn focus_list(&mut self, on: bool) {
        hand_focus(&mut self.sheet, &mut self.list, &mut self.list_focus, on);
    }

    pub(crate) fn focus_shortcuts(&mut self, on: bool) {
        hand_focus(
            &mut self.sheet,
            &mut self.shortcuts,
            &mut self.shortcuts_focus,
            on,
        );
    }

    pub(crate) fn focus_permissions(&mut self, on: bool) {
        hand_focus(
            &mut self.sheet,
            &mut self.permissions,
            &mut self.permissions_focus,
            on,
        );
    }

    pub(crate) fn on_page(&self, r: Rect) -> Rect {
        self.sheet.on_page(r)
    }

    pub(crate) fn side_list(&mut self) -> Option<(&mut ListView, Rect, &mut bool)> {
        let a = self.area();
        let (list, shortcuts) = (
            self.on_page(pages::list(a)),
            self.on_page(pages::shortcuts(a)),
        );
        let types = self.on_page(defaultspage::list(a));
        let permissions = self.on_page(pages::permissions(a));
        match registry::open(&self.sheet) {
            Page::Startup => Some((&mut self.list, list, &mut self.list_focus)),
            Page::Keyboard => Some((&mut self.shortcuts, shortcuts, &mut self.shortcuts_focus)),
            Page::DefaultPrograms => Some((&mut self.types, types, &mut self.types_focus)),
            Page::Privacy => Some((
                &mut self.permissions,
                permissions,
                &mut self.permissions_focus,
            )),
            _ => None,
        }
    }

    fn focus_side_list(&mut self, on: bool) {
        match registry::open(&self.sheet) {
            Page::Startup => self.focus_list(on),
            Page::Keyboard => self.focus_shortcuts(on),
            Page::DefaultPrograms => self.focus_types(on),
            Page::Privacy => self.focus_permissions(on),
            _ => {}
        }
    }

    pub(crate) fn blur_side_lists(&mut self) {
        self.focus_list(false);
        self.focus_shortcuts(false);
        self.focus_types(false);
        self.focus_permissions(false);
    }

    /// Let go of the side lists once the search field, the page list or a page control holds the
    /// keyboard, so one thing holds it.
    pub(crate) fn sheet_took_keyboard(&mut self) {
        if self.sheet.search_focused
            || self.sheet.tabs.focused
            || self.sheet.open().focus().is_some()
        {
            self.blur_side_lists();
        }
    }

    /// The side list's keys while focused. Space toggles a Startup check.
    pub(crate) fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Reply {
        let Some((list, rect, _)) = self.side_list() else {
            return Reply::none();
        };
        let lay = listview::layout(list, rect, cm);
        match list.key(key, &lay, true) {
            Some(took) => self.list_took(took),
            None => Reply::none(),
        }
    }

    /// What a press or key on the side list did: a flipped Startup check and a moved
    /// selection reach the page's buttons.
    fn list_took(&mut self, took: Took) -> Reply {
        match took {
            Took::Same => return Reply::none(),
            Took::Toggled(i) => self.startup_toggled(i),
            Took::Moved => {}
        }
        self.sync_apply();
        Reply::redraw()
    }

    /// Tab on a page whose side list comes first: focus runs search field → page list → side list → the form → OK → Cancel → Apply → search field. One part holds the keyboard at a time.
    pub(crate) fn side_tab(&mut self, back: bool) -> Reply {
        if self.sheet.search_focused {
            let ev = self.sheet.key(form::Key::Tab { back }, None);
            return self.sheet_event(ev);
        }
        if let Some(after) = self.side_list_after() {
            return self.mid_list_tab(after, back);
        }
        let held =
            self.list_focus || self.shortcuts_focus || self.types_focus || self.permissions_focus;
        if held {
            self.focus_side_list(false);
            if back {
                self.sheet.focus_list();
            } else {
                self.sheet.open_mut().focus_edge(false);
            }
            return Reply::redraw();
        }
        if self.sheet.tabs.focused && !back {
            self.focus_side_list(true);
            return Reply::redraw();
        }
        if !self.sheet.tabs.focused && back {
            if !self.sheet.open_mut().tab_no_wrap(true) {
                self.focus_side_list(true);
            }
            return Reply::redraw();
        }
        let ev = self.sheet.key(form::Key::Tab { back }, None);
        self.sheet_event(ev)
    }

    /// The form control a side list sitting mid-page follows in the tab order; `None` for a
    /// list that is its page's first tab stop.
    fn side_list_after(&self) -> Option<usize> {
        match registry::open(&self.sheet) {
            Page::Privacy => self.sheet.open().index_of(pages::PRIVACY_LOCATION),
            _ => None,
        }
    }

    /// Tab on a page whose side list sits after form control `after`: the list is the stop
    /// between `after` and the control following it.
    fn mid_list_tab(&mut self, after: usize, back: bool) -> Reply {
        let held =
            self.list_focus || self.shortcuts_focus || self.types_focus || self.permissions_focus;
        if held {
            self.focus_side_list(false);
            let page = self.sheet.open_mut();
            page.set_focus(Some(after));
            if !back {
                page.tab_no_wrap(false);
            }
            return Reply::redraw();
        }
        if !self.sheet.tabs.focused {
            let page = self.sheet.open_mut();
            let at = page.focus();
            if !back && at == Some(after) {
                self.focus_side_list(true);
                return Reply::redraw();
            }
            if back && at.is_some_and(|f| f > after) {
                page.tab_no_wrap(true);
                if page.focus().is_some_and(|f| f <= after) {
                    self.focus_side_list(true);
                }
                return Reply::redraw();
            }
        }
        let ev = self.sheet.key(form::Key::Tab { back }, None);
        self.sheet_event(ev)
    }

    /// A page whose side list is its first tab stop starts on that list unless the page list
    /// kept the focus.
    pub(crate) fn tab_opened(&mut self) {
        if !self.sheet.tabs.focused && self.side_list_after().is_none() {
            self.focus_side_list(true);
        }
    }

    /// Enter or a double click on the side list's focused row: a shortcut row opens its editor, a file
    /// type the program chooser. `None` on a list with no such action.
    pub(crate) fn list_activate(&mut self) -> Option<Reply> {
        match registry::open(&self.sheet) {
            Page::Keyboard => Some(self.edit_shortcut()),
            Page::DefaultPrograms => Some(self.open_chooser()),
            _ => None,
        }
    }

    pub(crate) fn list_press(&mut self, x: i32, y: i32, time: u32, ctx: &Ctx) -> Reply {
        let cm = &ctx.forms().controls();
        // A click gives the list the keyboard and the ring; `select`
        // alone does not turn the ring on.
        self.focus_side_list(true);
        let Some((list, rect, _)) = self.side_list() else {
            return Reply::none();
        };
        let lay = listview::layout(list, rect, cm);
        let hit = list.hit(&lay, x, y);
        let activated = list.activated(hit, ctx.theme.metrics.click(x, y, time));
        // A header column held draws pressed.
        list.header.press(hit.column(), false);
        let took = list.pointer(listview::Input::Press(x, y), &lay, rect, true);
        let r = took.map_or_else(Reply::redraw, |t| self.list_took(t));
        match activated {
            Some(_) => self.list_activate().unwrap_or(r),
            None => r,
        }
    }

    /// A motion, release or wheel turn the side list takes: a held box or thumb follows the
    /// pointer until the release, and the wheel over the list scrolls it.
    fn list_pointer(&mut self, ev: listview::Input, cm: &ControlMetrics) -> Option<Reply> {
        let (list, rect, _) = self.side_list()?;
        let lay = listview::layout(list, rect, cm);
        Some(match list.pointer(ev, &lay, rect, true)? {
            Took::Toggled(i) => {
                self.startup_toggled(i);
                self.sync_apply();
                Reply::redraw()
            }
            Took::Moved => Reply::redraw(),
            Took::Same => Reply::none(),
        })
    }

    /// Whether (`x`, `y`) is on the side list where it shows: in its rect and off the page's
    /// scrollbars and edges while the page scrolls.
    pub(crate) fn over_side_list(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let px = self.sheet.open().units().px(ctx.theme.fonts.size_px);
        self.sheet.reaches_page(x, y, &ctx.text, px)
            && self
                .side_list()
                .is_some_and(|(_, rect, _)| rect.contains(x, y))
    }

    /// Light the side list's header column under the pointer; true when that changed what draws.
    pub(crate) fn list_hover(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let cm = ctx.forms().controls();
        let styled = ctx.styled();
        let over = self.over_side_list(x, y, ctx);
        let Some((list, rect, _)) = self.side_list() else {
            return false;
        };
        let column = if styled && over {
            let lay = listview::layout(list, rect, &cm);
            list.hit(&lay, x, y).column()
        } else {
            None
        };
        list.header.motion(column) && styled
    }

    /// The pointer left or let go: the header column goes back to rest; true when that changed what draws.
    pub(crate) fn list_unhover(&mut self, release: bool, ctx: &Ctx) -> bool {
        let styled = ctx.styled();
        let Some((list, _, _)) = self.side_list() else {
            return false;
        };
        let changed = if release {
            let held = list.header.holding();
            list.header.release();
            held
        } else {
            list.header.leave()
        };
        changed && styled
    }

    pub(crate) fn list_motion(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Option<Reply> {
        self.list_pointer(listview::Input::Motion(x, y), cm)
    }

    pub(crate) fn list_release(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Option<Reply> {
        self.list_pointer(listview::Input::Release(x, y), cm)
    }

    /// The wheel over the side list where it shows.
    pub(crate) fn list_wheel(&mut self, x: i32, y: i32, rows: i32, ctx: &Ctx) -> Reply {
        if !self.over_side_list(x, y, ctx) {
            return Reply::none();
        }
        let ev = listview::Input::Scroll(x, y, rows);
        self.list_pointer(ev, &ctx.forms().controls())
            .unwrap_or_else(Reply::none)
    }
}
