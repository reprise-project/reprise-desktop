//! Keys and pointer hover on open menus.

use std::time::Duration;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use reprisekit::controls::menu::{KeyAct, MenuKey, SCROLL_REPEAT, ScrollArrow, key_act_in};

use crate::{
    compositor::State,
    startmenu::{KeyStops, step_selection},
};

use super::*;

impl State {
    fn key_open_submenu(&mut self, depth: usize, row: usize) {
        if !self.submenu_open_for(depth, row) {
            if self.is_tray_menu(depth) {
                crate::tray::menu_hover(self, depth, Some(row));
                self.set_menu_hover(depth, Some(row));
                return;
            }
            self.open_submenu(depth, row);
        }
        let Some(items) = self.menus.get(depth + 1).map(|m| &m.items) else {
            return;
        };
        let first = step_selection(items, None, 1, KeyStops::of(&self.config.theme));
        self.set_menu_hover(depth + 1, first);
    }

    pub fn menu_key(&mut self, sym: u32) {
        let Some(depth) = self.menus.len().checked_sub(1) else {
            return;
        };
        self.schedule_menu_hover(None);
        self.enter_menu_keys();
        let ch = smithay::input::keyboard::Keysym::new(sym)
            .key_char()
            .map(String::from);
        let Some(key) = MenuKey::from_keysym(sym, ch.as_deref()) else {
            return;
        };
        let menu = &self.menus[depth];
        let stops = KeyStops::of(&self.config.theme);
        let act = key_act_in(&menu.items, &menu.layout, menu.hovered, key, stops);
        self.apply_menu_key(depth, act);
    }

    fn apply_menu_key(&mut self, depth: usize, act: KeyAct) {
        match act {
            KeyAct::Nothing => {}
            KeyAct::Select(row) => self.set_menu_hover(depth, row),
            KeyAct::SelectAnd(row, then) => {
                self.set_menu_hover(depth, Some(row));
                self.apply_menu_key(depth, *then);
            }
            KeyAct::CloseTopmost => self.close_topmost_menu(),
            KeyAct::CloseSubmenu => {
                if depth > 0 {
                    self.close_menus_above(depth - 1);
                } else if self.start_flyout().is_some() {
                    self.close_menu();
                }
            }
            KeyAct::OpenSubmenu(row) => self.key_open_submenu(depth, row),
            KeyAct::Activate(row) => self.menu_activate(depth, row),
        }
    }

    /// The pointer rests on row `idx` of menu `depth`: the row lights and its infotip comes up.
    pub fn menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        if self.is_tray_menu(depth) {
            crate::tray::menu_hover(self, depth, idx);
            self.set_menu_hover(depth, idx);
            crate::tooltip::hover_menu(self, depth, idx);
            return;
        }
        if let Some(i) = idx {
            let pending = if self.row_opens_submenu(depth, i) {
                (!self.submenu_open_for(depth, i)).then_some(MenuPending::Open { depth, row: i })
            } else {
                (self.menus.len() > depth + 1).then_some(MenuPending::CloseAbove { depth })
            };
            self.schedule_menu_hover(pending);
        }
        self.set_menu_hover(depth, idx);
        crate::tooltip::hover_menu(self, depth, idx);
    }

    pub(super) fn schedule_menu_hover(&mut self, pending: Option<MenuPending>) {
        if self.menu_pending == pending {
            return;
        }
        self.menu_pending = pending;
        if let Some(t) = self.menu_timer.take() {
            self.handle.remove(t);
        }
        if pending.is_none() {
            return;
        }
        let ms = self.config.theme.metrics.menu_hover_ms.max(1) as u64;
        let token = self.handle.insert_source(
            Timer::from_duration(Duration::from_millis(ms)),
            |_, _, state| {
                state.menu_timer = None;
                state.menu_hover_fired();
                TimeoutAction::Drop
            },
        );
        match token {
            Ok(t) => self.menu_timer = Some(t),
            Err(e) => log::warn!("menu hover timer: {}", e.error),
        }
    }

    fn menu_hover_fired(&mut self) {
        match self.menu_pending.take() {
            Some(MenuPending::Open { depth, row }) => {
                let still = self
                    .menus
                    .get(depth)
                    .is_some_and(|m| m.hovered == Some(row));
                if still && !self.submenu_open_for(depth, row) {
                    self.open_submenu(depth, row);
                }
            }
            Some(MenuPending::CloseAbove { depth }) => self.close_menus_above(depth),
            None => {}
        }
    }

    pub fn clear_menu_hover(&mut self) {
        for depth in 0..self.menus.len() {
            self.set_menu_hover(depth, None);
        }
    }

    /// Lights row `idx` of menu `depth`. A tip shows only for the pointer (`menu_hover`), so a
    /// move here takes down the menu's tip.
    pub(crate) fn set_menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        let changed = match self.menus.get_mut(depth) {
            Some(m) if m.hovered != idx => {
                m.hovered = idx;
                if let Some(i) = idx {
                    m.layout.scroll_to_show(i);
                }
                true
            }
            _ => false,
        };
        if changed {
            self.redraw_menu(depth);
            crate::tooltip::hover_menu(self, depth, None);
        }
    }

    fn menu_arrow_at(&self, depth: usize, x: f64, y: f64) -> Option<ScrollArrow> {
        let m = self.menus.get(depth)?;
        m.layout
            .arrow_at(x as i32 - m.x - m.shift.0, y as i32 - m.y - m.shift.1)
    }

    /// Scrolls menu `depth` `rows` rows, closing the menus it had open; true when it moved.
    fn scroll_menu(&mut self, depth: usize, rows: i32) -> bool {
        let Some(m) = self.menus.get_mut(depth) else {
            return false;
        };
        if !m.layout.scroll_by(rows) {
            return false;
        }
        self.close_menus_above(depth);
        self.redraw_menu(depth);
        true
    }

    /// The pointer at (x, y) over menu `depth`: resting on a scroll arrow scrolls the menu a row
    /// at once and every [`SCROLL_REPEAT`] while it stays there.
    pub fn menu_arrow_hover(&mut self, depth: usize, x: f64, y: f64) {
        let on = self.menu_arrow_at(depth, x, y).map(|a| (depth, a));
        if on == self.menu_scrolling {
            return;
        }
        self.menu_scrolling = on;
        if let Some(t) = self.menu_scroll_timer.take() {
            self.handle.remove(t);
        }
        let Some((depth, arrow)) = on else {
            return;
        };
        if !self.scroll_menu(depth, arrow_step(arrow)) {
            return;
        }
        let token =
            self.handle
                .insert_source(Timer::from_duration(SCROLL_REPEAT), move |_, _, state| {
                    let p = state.pointer_location();
                    let still = state.menu_scrolling == Some((depth, arrow))
                        && state.menu_arrow_at(depth, p.x, p.y) == Some(arrow);
                    if still && state.scroll_menu(depth, arrow_step(arrow)) {
                        return TimeoutAction::ToDuration(SCROLL_REPEAT);
                    }
                    state.menu_scrolling = None;
                    state.menu_scroll_timer = None;
                    TimeoutAction::Drop
                });
        match token {
            Ok(t) => self.menu_scroll_timer = Some(t),
            Err(e) => log::warn!("menu scroll timer: {}", e.error),
        }
    }

    /// The wheel turned over menu `depth` (`v120` notches, `travel` px): a menu that scrolls
    /// takes it, and the row under the pointer lights. True when the menu scrolls at all.
    pub fn menu_wheel(&mut self, depth: usize, v120: i32, travel: f64) -> bool {
        if !self
            .menus
            .get(depth)
            .is_some_and(|m| m.layout.scroll.is_some())
        {
            return false;
        }
        let rows = self.panels.wheel.lines(v120, travel);
        if rows != 0 && self.scroll_menu(depth, rows) {
            let p = self.pointer_location();
            let row = self.menu_row_at(depth, p.x, p.y);
            self.menu_hover(depth, row);
        }
        true
    }

    pub fn menu_row_at(&self, depth: usize, x: f64, y: f64) -> Option<usize> {
        let m = self.menus.get(depth)?;
        m.layout
            .hit(x as i32 - m.x - m.shift.0, y as i32 - m.y - m.shift.1)
    }
}

fn arrow_step(arrow: ScrollArrow) -> i32 {
    match arrow {
        ScrollArrow::Up => -1,
        ScrollArrow::Down => 1,
    }
}
