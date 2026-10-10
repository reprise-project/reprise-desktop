//! The taskbar with the keyboard: Super+T (`focus-taskbar`) puts it on Start, Super+B (`focus-tray`)
//! on the notification area. Escape hands the keyboard back; anything else taking it ends this.

use reprisekit::taskbar::TaskbarHit;
use reprisekit::taskbarfocus::{self, BarKey};
use reprisescene::TreeId;

use crate::compositor::State;
use crate::config::StartMenuStyle;
use crate::window::WindowId;

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Back {
    Window(WindowId),
    Dialog(u64),
    Nobody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarKeys {
    pub bar: TreeId,
    pub at: TaskbarHit,
    back: Back,
}

impl State {
    pub fn focus_taskbar(&mut self, tray: bool) {
        let p = self.pointer_location();
        let Some(output) = self.output_at(p.x, p.y) else {
            return;
        };
        let Some((tree, items)) = (self.bars.iter())
            .find(|b| b.output == output)
            .map(|b| (b.tree, taskbarfocus::items(&b.layout)))
        else {
            return;
        };
        let at = if tray {
            taskbarfocus::tray_entry(&items)
        } else {
            TaskbarHit::Start
        };
        self.focus_bar_item(tree, at);
    }

    pub fn focus_bar_item(&mut self, tree: TreeId, at: TaskbarHit) {
        if self.is_locked() || crate::layer::holds_keyboard(self) || self.dialog_is_modal() {
            return;
        }
        let Some(output) = self.bar(tree).map(|b| b.output.clone()) else {
            return;
        };
        let back = match self.bar_shared.keys {
            Some(k) => k.back,
            None if self.dialog_has_keyboard() => {
                self.active_dialog_id().map_or(Back::Nobody, Back::Dialog)
            }
            None => self.focused.map_or(Back::Nobody, Back::Window),
        };
        let was = self.bar_shared.keys.map(|k| k.bar);
        crate::autohideview::keys(self, &output, true);
        self.deactivate_dialogs();
        self.enter_keyboard(None);
        self.bar_shared.keys = Some(BarKeys {
            bar: tree,
            at,
            back,
        });
        if let Some(t) = was.filter(|&t| t != tree) {
            self.redraw_taskbar(t);
        }
        self.redraw_taskbar(tree);
    }

    pub fn bar_focus(&self, bar: &crate::taskbarview::Bar) -> Option<TaskbarHit> {
        let k = self.bar_shared.keys.filter(|k| k.bar == bar.tree)?;
        taskbarfocus::fit(&taskbarfocus::items(&bar.layout), k.at)
    }

    /// A press for the bar with the keyboard: its first keysym, Shift
    /// held or not. A key the bar has no use for is swallowed.
    pub fn bar_key(&mut self, sym: u32, shift: bool) {
        let Some(keys) = self.bar_shared.keys else {
            return;
        };
        let Some(items) = self.bar(keys.bar).map(|b| taskbarfocus::items(&b.layout)) else {
            self.bar_shared.keys = None;
            return;
        };
        let Some(at) = taskbarfocus::fit(&items, keys.at) else {
            return;
        };
        match taskbarfocus::key(sym, shift) {
            None => {}
            Some(BarKey::Leave) => self.leave_bar_keys(),
            Some(BarKey::Activate) => self.bar_item_act(keys.bar, at, BTN_LEFT),
            Some(BarKey::Menu) => self.bar_item_act(keys.bar, at, BTN_RIGHT),
            Some(k) => self.set_bar_focus(keys.bar, taskbarfocus::step(&items, at, k)),
        }
    }

    fn set_bar_focus(&mut self, tree: TreeId, at: TaskbarHit) {
        if let Some(k) = self.bar_shared.keys.as_mut().filter(|k| k.bar == tree) {
            k.at = at;
        }
        self.redraw_taskbar(tree);
    }

    pub fn leave_bar_keys(&mut self) {
        let Some(k) = self.bar_shared.keys.take() else {
            return;
        };
        self.redraw_taskbar(k.bar);
        crate::trayreveal::keys_gone(self, k.bar);
        match k.back {
            Back::Window(w) if self.win(w).is_some_and(|x| x.mapped && !x.minimized) => {
                self.focus_window(w)
            }
            Back::Dialog(id) if self.dialogs.views.iter().any(|v| v.id == id) => {
                self.focus_dialog(id)
            }
            Back::Nobody => {}
            _ => self.focus_top(),
        }
    }

    pub fn end_bar_keys(&mut self) {
        if let Some(k) = self.bar_shared.keys.take() {
            self.redraw_taskbar(k.bar);
            crate::trayreveal::keys_gone(self, k.bar);
        }
    }

    /// Enter (`BTN_LEFT`) or the Menu key (`BTN_RIGHT`) on item `at` of bar
    /// `tree`: the click on the item's middle. Enter on Start opens the
    /// Start menu as the Start key does; on the active window's own button
    /// it hands the keyboard back to that window rather than minimizing
    /// it. A menu it opens takes the arrows with its access keys shown,
    /// and Escape there comes back to the bar.
    fn bar_item_act(&mut self, tree: TreeId, at: TaskbarHit, button: u32) {
        let Some(b) = self.bar(tree) else { return };
        if button == BTN_LEFT
            && let TaskbarHit::Task(i) = at
            && b.dialogs.get(i).copied().flatten().is_none()
            && let Some(&[w]) = b.buttons.get(i).map(Vec::as_slice)
            && self.focused == Some(w)
            && self.win(w).is_some_and(|x| !x.minimized)
        {
            self.bar_shared.keys = None;
            self.redraw_taskbar(tree);
            crate::trayreveal::keys_gone(self, tree);
            self.focus_window(w);
            return;
        }
        let shown = self.bar_drawn(&b.output).shown;
        let Some(r) = taskbarfocus::focus_ring(&b.layout, at, 1) else {
            return;
        };
        let menus_were = self.menus.len();
        if button == BTN_LEFT && at == TaskbarHit::Start {
            self.toggle_start(tree);
        } else if enter_toggles_run(crate::trayreveal::arrow_look(self), at, button) {
            crate::trayreveal::toggle(self, tree);
        } else {
            let mid = (
                f64::from(shown.x + r.x + r.width / 2),
                f64::from(shown.y + r.y + r.height / 2),
            );
            crate::access::click(self, mid, button);
        }
        if menus_were == 0
            && !self.menus.is_empty()
            && (at != TaskbarHit::Start || self.config.startmenu_style == StartMenuStyle::Classic)
        {
            self.enter_menu_keys();
        }
    }
}

/// Whether Enter (`BTN_LEFT`) on item `at` opens or shuts the tray run in
/// place (`trayreveal::toggle`) rather than clicking the item's middle:
/// on the arrow of a look that has one (`arrow_look`). A click would land
/// on nothing while an auto-hidden bar still slides in.
fn enter_toggles_run(arrow_look: bool, at: TaskbarHit, button: u32) -> bool {
    arrow_look && at == TaskbarHit::Chevron && button == BTN_LEFT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_on_the_tray_arrow_toggles_its_run_without_a_click() {
        assert!(enter_toggles_run(true, TaskbarHit::Chevron, BTN_LEFT));
        assert!(
            !enter_toggles_run(false, TaskbarHit::Chevron, BTN_LEFT),
            "the ^ flyout's cell clicks"
        );
        assert!(
            !enter_toggles_run(true, TaskbarHit::Chevron, BTN_RIGHT),
            "the Menu key clicks"
        );
        assert!(!enter_toggles_run(true, TaskbarHit::Clock, BTN_LEFT));
    }
}
