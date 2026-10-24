//! Focus, keys and pointer for the list views beside a page's form.

use crate::registry::Page;
use crate::{SettingsApp, defaultspage, pages, registry};
use repriseapp::{Ctx, Reply};
use reprisekit::classic::widgets::BarHit;
use reprisekit::controls::form;
use reprisekit::controls::listview::{self, ListView};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::controls::propsheet::PropSheet;
use reprisekit::raster::Rect;

/// Give a side list the keyboard or take it away; the dotted rectangle goes with it.
pub(crate) fn hand_focus(sheet: &mut PropSheet, list: &mut ListView, held: &mut bool, on: bool) {
    *held = on;
    list.focus_ring = on;
    if on {
        sheet.open_mut().set_focus(None);
        sheet.tabs.focused = false;
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

    pub(crate) fn focus_locations(&mut self, on: bool) {
        hand_focus(
            &mut self.sheet,
            &mut self.locations,
            &mut self.locations_focus,
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
        let locations = self.on_page(pages::locations(a));
        match registry::open(&self.sheet) {
            Page::Startup => Some((&mut self.list, list, &mut self.list_focus)),
            Page::Keyboard => Some((&mut self.shortcuts, shortcuts, &mut self.shortcuts_focus)),
            Page::DefaultPrograms => Some((&mut self.types, types, &mut self.types_focus)),
            Page::Advanced => Some((&mut self.locations, locations, &mut self.locations_focus)),
            _ => None,
        }
    }

    fn focus_side_list(&mut self, on: bool) {
        match registry::open(&self.sheet) {
            Page::Startup => self.focus_list(on),
            Page::Keyboard => self.focus_shortcuts(on),
            Page::DefaultPrograms => self.focus_types(on),
            Page::Advanced => self.focus_locations(on),
            _ => {}
        }
    }

    pub(crate) fn blur_side_lists(&mut self) {
        self.focus_list(false);
        self.focus_shortcuts(false);
        self.focus_types(false);
        self.focus_locations(false);
    }

    /// The side list's keys while focused. Space toggles a Startup check.
    pub(crate) fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Reply {
        let Some((list, rect, _)) = self.side_list() else {
            return Reply::none();
        };
        let lay = listview::layout(list, rect, cm);
        // Both lists are single-select: no Ctrl or Shift reaches them.
        let mods = reprisekit::selection::Mods::default();
        let moved = match key {
            form::Key::Up => list.step(-1, &lay, mods),
            form::Key::Down => list.step(1, &lay, mods),
            form::Key::Home => list.step(-(list.rows.len() as i64), &lay, mods),
            form::Key::End => list.step(list.rows.len() as i64, &lay, mods),
            form::Key::PageUp => list.step(-(lay.rows as i64), &lay, mods),
            form::Key::PageDown => list.step(lay.rows as i64, &lay, mods),
            form::Key::Space => {
                let Some(i) = list.selection.focus else {
                    return Reply::none();
                };
                list.toggle(i)
            }
            _ => false,
        };
        if !moved {
            return Reply::none();
        }
        self.sync_apply();
        Reply::redraw()
    }

    /// Tab on a page whose side list comes first: focus runs page list → side list → the form → OK → Cancel → Apply → page list. Only one dotted rectangle is ever on the sheet.
    pub(crate) fn side_tab(&mut self, back: bool) -> Reply {
        if let Some(after) = self.side_list_after() {
            return self.mid_list_tab(after, back);
        }
        let held =
            self.list_focus || self.shortcuts_focus || self.types_focus || self.locations_focus;
        if held {
            self.focus_side_list(false);
            let page = self.sheet.open_mut();
            if back {
                page.set_focus(None);
                self.sheet.tabs.focused = true;
            } else {
                page.focus_edge(false);
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
            Page::Advanced => self.sheet.open().index_of(pages::PRIVACY_LOCATION),
            _ => None,
        }
    }

    /// Tab on a page whose side list sits after form control `after`: the list is the stop
    /// between `after` and the control following it.
    fn mid_list_tab(&mut self, after: usize, back: bool) -> Reply {
        let held =
            self.list_focus || self.shortcuts_focus || self.types_focus || self.locations_focus;
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
        // A box pressed is held; it toggles on the release over it
        // (`list_release`). A header column held draws pressed.
        list.press_check(hit);
        list.header.press(hit.column(), false);
        match hit {
            listview::Hit::Check(i) => list.select(Some(i), &lay),
            listview::Hit::Row(i) => {
                list.select(Some(i), &lay);
                if activated.is_some() {
                    self.sync_apply();
                    return self.list_activate().unwrap_or_else(Reply::redraw);
                }
            }
            listview::Hit::VBar(BarHit::Back) => {
                list.scroll(-1, &lay);
            }
            listview::Hit::VBar(BarHit::Forward) => {
                list.scroll(1, &lay);
            }
            _ => return Reply::redraw(),
        }
        self.sync_apply();
        Reply::redraw()
    }

    fn held_check_hit(
        &mut self,
        x: i32,
        y: i32,
        cm: &ControlMetrics,
    ) -> Option<(&mut ListView, listview::Hit)> {
        let (list, rect, _) = self.side_list()?;
        if !list.held_check.holding() {
            return None;
        }
        let lay = listview::layout(list, rect, cm);
        let hit = list.hit(&lay, x, y);
        Some((list, hit))
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
        let (list, hit) = self.held_check_hit(x, y, cm)?;
        Some(if list.motion_check(hit) {
            Reply::redraw()
        } else {
            Reply::none()
        })
    }

    pub(crate) fn list_release(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> Option<Reply> {
        let (list, hit) = self.held_check_hit(x, y, cm)?;
        if list.release_check(hit).is_some() {
            self.sync_apply();
        }
        Some(Reply::redraw())
    }
}
