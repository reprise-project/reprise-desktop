//! Keys and pointer hover on open menus.

use std::time::Duration;

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

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
        let act = {
            let menu = &self.menus[depth];
            key_act(
                &menu.items,
                menu.hovered,
                sym,
                KeyStops::of(&self.config.theme),
            )
        };
        if let Some(act) = act {
            self.apply_menu_key(depth, act);
        }
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
                }
            }
            KeyAct::OpenSubmenu(row) => self.key_open_submenu(depth, row),
            KeyAct::Activate(row) => self.menu_activate(depth, row),
        }
    }

    pub fn menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        if self.is_tray_menu(depth) {
            crate::tray::menu_hover(self, depth, idx);
            self.set_menu_hover(depth, idx);
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

    fn set_menu_hover(&mut self, depth: usize, idx: Option<usize>) {
        let changed = match self.menus.get_mut(depth) {
            Some(m) if m.hovered != idx => {
                m.hovered = idx;
                true
            }
            _ => false,
        };
        if changed {
            self.redraw_menu(depth);
            crate::tooltip::hover_menu(self, depth, idx);
        }
    }

    pub fn menu_row_at(&self, depth: usize, x: f64, y: f64) -> Option<usize> {
        let m = self.menus.get(depth)?;
        m.layout
            .hit(x as i32 - m.x - m.shift.0, y as i32 - m.y - m.shift.1)
    }
}
