//! The `[keyboard] layout` list at run time: which layout is active and the switch between them
//! (`Action::NextLayout`, the tray indicator's menu).
//!
//! The active layout is the xkb group of the seat's one keyboard. A switch sets the group on its xkb
//! state, which the focused client hears with the modifiers. A switch xkb makes on its own (a `grp:`
//! option) shows up in that state after the key, and `follow_layout` picks it up.

use reprisefs::xkb::Described;
use smithay::output::Output;

use crate::compositor::State;
use crate::menuview::{MenuFrom, MenuKind};
use crate::raster::Rect;
use crate::startmenu::{MenuItem, MenuStyle};

#[derive(Debug, Default)]
pub struct Layouts {
    /// What each configured layout shows, in group order. Empty for a single layout or the system default: then nothing switches.
    pub labels: Vec<Described>,
    pub current: u32,
}

impl Layouts {
    pub fn switchable(&self) -> bool {
        self.labels.len() > 1
    }

    pub fn active(&self) -> Option<&Described> {
        self.labels.get(self.current as usize)
    }
}

/// The labels `[keyboard]` gives: one per configured layout, from the xkb registry, or none when fewer than two are configured.
pub fn labels(kb: &crate::config::KeyboardConfig) -> Vec<Described> {
    let pairs = reprisefs::xkb::split(&kb.layout, &kb.variant);
    if pairs.len() < 2 {
        return Vec::new();
    }
    let reg = reprisefs::xkb::load(&kb.rules);
    pairs
        .iter()
        .map(|(l, v)| reprisefs::xkb::describe(&reg, l, v))
        .collect()
}

impl State {
    /// Compile the keymap from `[keyboard]` and put it, with the repeat settings, on the seat's keyboard. Runs again when the config file changes.
    pub fn apply_keyboard(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let kb = self.config.keyboard.clone();
        let layout = if kb.layout.is_empty() {
            "default"
        } else {
            kb.layout.as_str()
        };
        self.ime.vk.forget_keymap();
        match keyboard.set_xkb_config(self, crate::config::xkb_config(&kb)) {
            Ok(()) => log::info!("keyboard layout {layout}"),
            Err(e) => log::error!(
                "could not compile the xkb keymap for layout {layout} ({e}); the keyboard keeps its keymap"
            ),
        }
        keyboard.change_repeat_info(kb.repeat_rate, kb.repeat_delay);
        self.eis_readvertise();
        self.layouts_configured();
    }

    /// The keymap was just compiled from `[keyboard]`: relabel, and read the group back (a new keymap starts on the first layout).
    pub fn layouts_configured(&mut self) {
        self.key_input.layouts.labels = labels(&self.config.keyboard);
        self.key_input.layouts.current = self.active_layout();
        self.redraw_taskbars();
        crate::lockview::redraw(self);
    }

    fn active_layout(&mut self) -> u32 {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return 0;
        };
        keyboard.with_xkb_state(self, |ctx| ctx.xkb().lock().unwrap().active_layout().0)
    }

    /// The keyboard's state changed: follow a layout switch, whoever made it.
    pub fn follow_layout(&mut self) {
        let group = self.active_layout();
        let layouts = &mut self.key_input.layouts;
        if layouts.current == group {
            return;
        }
        layouts.current = group;
        if layouts.switchable() {
            self.redraw_taskbars();
            crate::lockview::redraw(self);
        }
    }

    pub fn set_layout(&mut self, idx: u32) {
        let Some((mut mask, _)) = self.xkb_mask() else {
            return;
        };
        mask.layout_depressed = 0;
        mask.layout_latched = 0;
        mask.layout_locked = idx;
        self.set_xkb_mask(mask);
        self.follow_layout();
    }

    pub fn next_layout(&mut self) {
        let layouts = &self.key_input.layouts;
        if !layouts.switchable() {
            return;
        }
        let n = layouts.labels.len() as u32;
        self.set_layout((layouts.current + 1) % n);
    }

    /// The indicator's menu: every layout by its description, the active one checked, opening from the
    /// cell at `cell` (layout coordinates) on `output`, on the screen side of the bar.
    pub fn open_layout_menu(&mut self, output: &Output, cell: Rect) {
        let layouts = &self.key_input.layouts;
        if !layouts.switchable() || self.layout_box(Some(output)).width <= 0 {
            return;
        }
        let items: Vec<MenuItem> = layouts
            .labels
            .iter()
            .enumerate()
            .map(|(i, d)| MenuItem::entry(&d.description).checked(i as u32 == layouts.current))
            .collect();
        let usable = self.usable_area(Some(output));
        let layout = MenuStyle::context_for(&self.config.theme, &items).layout_in(
            &self.config.theme,
            &self.text,
            &items,
            usable,
        );
        let (x, y) = crate::barplace::bar_popup(
            &self.bar_placement(output),
            cell,
            (layout.width, layout.height),
            usable,
        );
        self.push_menu(
            0,
            (x, y),
            layout,
            items,
            output.clone(),
            MenuKind::Layouts,
            MenuFrom::Bar,
        );
        self.redraw_menu(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_layout_or_the_default_offers_no_switch() {
        let mut kb = crate::config::KeyboardConfig::default();
        assert!(labels(&kb).is_empty());
        kb.layout = "us".into();
        assert!(labels(&kb).is_empty());
        let layouts = Layouts {
            labels: labels(&kb),
            current: 0,
        };
        assert!(!layouts.switchable());
    }
}
