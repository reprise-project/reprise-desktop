//! The popups the panel opens: a row's menu, shutdown options, Recent Items.

use std::ffi::OsStr;

use reprisekit::startpanel::{Shape, Spot};
use reprisekit::taskbar::Edge;

use crate::compositor::State;
use crate::config::{Action, PowerAction};
use crate::lockview::Origin;
use crate::menuview::{MenuKind, Skin};
use crate::raster::Rect;
use crate::startmenu::{MenuItem, StartRow, place_popup};

pub(super) fn shutdown_options_at(
    edge: Edge,
    arrow: Rect,
    size: (i32, i32),
    usable: Rect,
) -> (i32, i32) {
    let h = size.1;
    let drops = edge != Edge::Bottom;
    let y = if drops && arrow.y + h <= usable.bottom() {
        arrow.y
    } else {
        arrow.bottom() - h
    };
    place_popup((arrow.right(), y), size, usable)
}

impl State {
    /// Moves the popup open off the panel to hang at `at`, as `place_popup` puts it.
    pub(super) fn move_start_popup(&mut self, at: (i32, i32)) {
        let Some(m) = self.menus.first() else {
            return;
        };
        let usable = self.usable_area(Some(&m.output));
        let (x, y) = place_popup(at, (m.layout.width, m.layout.height), usable);
        if (x, y) == (m.x, m.y) {
            return;
        }
        let tree = m.tree;
        if let Some(m) = self.menus.first_mut() {
            (m.x, m.y) = (x, y);
        }
        let outputs = self.scene.move_on_outputs(tree, x, y);
        self.redraw_nodes(outputs, &[]);
    }

    fn push_start_popup(
        &mut self,
        at: (i32, i32),
        items: Vec<MenuItem>,
        kind: crate::menuview::MenuKind,
        place: impl FnOnce((i32, i32), (i32, i32), Rect) -> (i32, i32),
        skin: Skin,
    ) {
        if let Some(t) = self.start_panel.as_mut().and_then(|v| v.hover.take()) {
            self.handle.remove(t);
        }
        crate::tooltip::hover_start_panel(self, None);
        self.close_menu();
        let from = crate::menuview::MenuFrom::StartPanel;
        self.push_context_menu_looked(at, items, kind, from, place, skin);
    }

    pub(super) fn open_start_context(&mut self, s: Spot) {
        let Some((app, launched)) = self.start_spot_app(s) else {
            return;
        };
        let Some((items, rows)) = self.program_menu_of(app, launched) else {
            return;
        };
        let p = self.pointer_location();
        let at = if self.start_panel_hit(p.x, p.y) == Some(s) {
            (p.x as i32, p.y as i32)
        } else {
            match self.start_spot_rect(s) {
                Some(r) => (r.x, r.bottom()),
                None => return,
            }
        };
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        self.push_start_popup(at, items, kind, place_popup, Skin::Theme);
    }

    pub(super) fn open_shutdown_options(&mut self) {
        let Some(arrow) = self.start_spot_rect(Spot::ShutdownArrow) else {
            return;
        };
        let Some(output) = self.start_panel_output() else {
            return;
        };
        let edge = self.bar_placement(&output).edge;
        let (items, rows) = crate::startmenu::shutdown_menu(
            !self.config.power.hibernate.is_empty(),
            crate::console::leave_offered(self),
        );
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        let at = (arrow.right(), arrow.y);
        let place = |_, size, usable| shutdown_options_at(edge, arrow, size, usable);
        self.push_start_popup(at, items, kind, place, Skin::Options);
    }

    pub(super) fn open_recent_items(&mut self, i: usize) {
        if !self.config.recent_files {
            return;
        }
        let Some(row) = self.start_spot_rect(Spot::Place(i)) else {
            return;
        };
        let Some(output) = self.start_panel.as_ref().map(|v| v.output.clone()) else {
            return;
        };
        let px = crate::startmenu::MenuStyle::context(
            &self.config.theme,
            crate::startmenu::ContextBitmaps::Icons,
        )
        .icon_px()
            * self.draw_scale_at(Some(&output)).max(1);
        self.refresh_recent();
        let (items, kind, icons) = self.documents_rows(px, false);
        let skin = self.start_flyout_skin();
        self.push_start_popup((row.right(), row.y), items, kind, place_popup, skin);
        self.set_row_icons(0, icons);
        self.light_flyout_from_keys(Spot::Place(i));
    }

    /// The Blue menu's All Programs: the program folders in a menu beside the row, its bottom
    /// level with the row's.
    pub(super) fn open_all_programs(&mut self) {
        let Some(row) = self.start_spot_rect(Spot::AllPrograms) else {
            return;
        };
        let Some(output) = self.start_panel_output() else {
            return;
        };
        let px = crate::startmenu::MenuStyle::context(
            &self.config.theme,
            crate::startmenu::ContextBitmaps::Icons,
        )
        .icon_px()
            * self.draw_scale_at(Some(&output)).max(1);
        let (items, kind, icons) = self.program_groups_rows(px);
        let place = |_, size: (i32, i32), usable| {
            place_popup((row.right(), row.bottom() - size.1), size, usable)
        };
        self.push_start_popup((row.right(), row.y), items, kind, place, Skin::BlueStart);
        self.set_row_icons(0, icons);
        self.light_flyout_from_keys(Spot::AllPrograms);
    }

    /// The Blue menu's flyouts wear its own look; the Glass panel's Recent Items the theme's.
    fn start_flyout_skin(&self) -> Skin {
        match self.start_panel.as_ref().map(|v| v.model.shape) {
            Some(Shape::Blue) => Skin::BlueStart,
            _ => Skin::Theme,
        }
    }

    /// A Blue flyout the keyboard opened (its row is lit but not under the pointer) starts on its
    /// first row.
    fn light_flyout_from_keys(&mut self, row: Spot) {
        let keys = self
            .start_panel
            .as_ref()
            .is_some_and(|v| v.model.shape == Shape::Blue && v.model.hot() != Some(row));
        if keys && !self.menus.is_empty() {
            self.set_menu_hover(0, Some(0));
        }
    }

    /// The Blue menu's row whose menu stands open beside the panel: All Programs or Recent Items.
    pub(crate) fn start_flyout(&self) -> Option<Spot> {
        let v = self.start_panel.as_ref()?;
        if v.model.shape != Shape::Blue {
            return None;
        }
        let m = self.menus.first()?;
        if m.from != crate::menuview::MenuFrom::StartPanel {
            return None;
        }
        match m.kind {
            MenuKind::Groups { .. } => Some(Spot::AllPrograms),
            MenuKind::Documents { dirs: false, .. } => v
                .model
                .places
                .iter()
                .position(|p| p.submenu)
                .map(Spot::Place),
            _ => None,
        }
    }

    /// Closes the menu [`State::start_flyout`] names, as the pointer moved to another row.
    pub(super) fn close_start_flyout(&mut self) {
        if self.start_flyout().is_some() {
            self.close_menu();
        }
    }

    pub(crate) fn quick_file_of(&self, id: &str) -> Option<String> {
        self.quicklaunch
            .iter()
            .find(|q| q.app.as_ref().is_some_and(|a| a.id == id))
            .map(|q| q.file.clone())
    }

    pub(crate) fn start_row(&mut self, row: StartRow) {
        let entry = |s: &State, a: usize| s.app_index.get(a).cloned();
        match row {
            StartRow::Open(a) => {
                let e = entry(self, a);
                self.close_start_panel();
                if let Some(e) = e {
                    self.launch_app(&e);
                }
            }
            StartRow::PinTaskbar(a) => {
                if let Some(src) = entry(self, a).and_then(|e| reprisefs::apps::find_file(&e.id)) {
                    self.quick_add(vec![src], None);
                }
            }
            StartRow::UnpinTaskbar(a) => {
                if let Some(f) = entry(self, a).and_then(|e| self.quick_file_of(&e.id)) {
                    self.quick_remove(&f);
                }
            }
            StartRow::PinStart(a) | StartRow::UnpinStart(a) | StartRow::Forget(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                match row {
                    StartRow::PinStart(_) => {
                        self.launches.pin(&e.id);
                        if let Some(v) = self.start_panel.as_mut() {
                            v.new_pins.push(e.id.clone());
                        }
                    }
                    StartRow::UnpinStart(_) => {
                        self.launches.unpin(&e.id);
                        if let Some(v) = self.start_panel.as_mut() {
                            v.new_pins.retain(|p| *p != e.id);
                        }
                    }
                    _ => self.launches.forget(&e.id),
                }
                crate::state::save_start(&self.launches);
                self.list_start_programs();
                self.redraw_start_panel();
            }
            StartRow::Uninstall(a) => {
                let file = entry(self, a).and_then(|e| reprisefs::apps::find_file(&e.id));
                self.close_start_panel();
                if let Some(file) = file {
                    let program = [OsStr::new("reprise-updates"), OsStr::new("--uninstall")];
                    self.spawn_argv_activated(&[program[0], program[1], file.as_os_str()]);
                }
            }
            StartRow::CannotUninstall(_) => {}
            StartRow::Properties(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                let p = self.pointer_location();
                self.close_start_panel();
                let Some(file) = reprisefs::apps::find_file(&e.id) else {
                    return;
                };
                match reprisefs::props::glance(&file, &e.name, &self.app_index) {
                    Ok(info) => {
                        let icon = e.icon.as_deref().unwrap_or("application-x-executable");
                        self.open_properties(&info, icon, (p.x as i32, p.y as i32));
                    }
                    Err(err) => log::warn!("properties: {}: {err}", file.display()),
                }
            }
            StartRow::SwitchUser => {
                self.close_start_panel();
                if crate::loginlink::desk_offers(self) {
                    crate::loginlink::switch_user(self);
                }
            }
            StartRow::Disconnect => {
                self.close_start_panel();
                if crate::console::disconnect_offered(self) {
                    crate::console::disconnect(self);
                }
            }
            StartRow::LogOff => {
                self.close_start_panel();
                self.run_action(&Action::Exit, 0);
            }
            StartRow::Lock => {
                self.close_start_panel();
                self.run_action(&Action::Lock, 0);
            }
            StartRow::Restart => {
                self.close_start_panel();
                let cmd = self.config.power.reboot.clone();
                crate::sessionend::begin(
                    self,
                    reprisekit::controls::endsession::Goal::Restart,
                    Some(cmd),
                    Origin::Desktop,
                );
            }
            StartRow::Sleep => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::Standby, Origin::Desktop);
            }
            StartRow::Hibernate => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::Hibernate, Origin::Desktop);
            }
        }
    }
}
