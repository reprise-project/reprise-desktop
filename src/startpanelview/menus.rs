//! The popups the panel opens: a row's menu, shutdown options, Recent Items.

use std::ffi::OsStr;

use reprisekit::startpanel::{PowerButton, Shape, Spot};
use reprisekit::taskbar::Edge;

use crate::compositor::State;
use crate::config::{Action, PowerAction};
use crate::dragview::DropOp;
use crate::lockview::Origin;
use crate::menuview::{MenuKind, Skin};
use crate::raster::Rect;
use crate::startmenu::{JumpFileRow, MenuItem, StartRow, place_popup};
use reprisekit::controls::endsession::Goal;

/// The narrowest Recent Items gets to stand beside its row, at 100% text: an icon and about 20
/// characters.
pub(super) const RECENT_LEAST: i32 = 160;

/// Where a flyout `size` from the panel's row `row` stands, its top at `y`: right of the row
/// when it fits there, else left of `panel`, else left of the row over the panel's other
/// column, held in `usable` by `place_popup`.
fn beside_panel(row: Rect, panel: Rect, y: i32, size: (i32, i32), usable: Rect) -> (i32, i32) {
    let x = if row.right() + size.0 <= usable.right() {
        row.right()
    } else if panel.x - size.0 >= usable.x {
        panel.x - size.0
    } else {
        row.x - size.0
    };
    place_popup((x, y), size, usable)
}

/// The room a flyout from `row` has beside it: right of the row or left of `panel`, whichever
/// is wider, or with less than `least` there, the wider of that and the room left of the row.
fn room_beside(row: Rect, panel: Rect, usable: Rect, least: i32) -> i32 {
    let clear = (usable.right() - row.right()).max(panel.x - usable.x);
    if clear >= least {
        clear
    } else {
        clear.max(row.x - usable.x)
    }
}

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
    /// Moves the popup open off the panel to stand by its row `row` again ([`beside_panel`]).
    pub(super) fn move_start_popup(&mut self, row: Rect) {
        let (Some(m), Some(panel)) = (self.menus.first(), self.start_panel_rect()) else {
            return;
        };
        let usable = self.usable_area(Some(&m.output));
        let size = (m.layout.width, m.layout.height);
        let (x, y) = beside_panel(row, panel, row.y, size, usable);
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

    /// The context menu of program row or place `s`.
    pub(super) fn open_start_context(&mut self, s: Spot) {
        let menu = match s {
            Spot::Place(i) => self.start_place_menu(i),
            Spot::Jump(k) => self.start_jump_file_menu(k),
            _ => (self.start_spot_app(s))
                .and_then(|(app, launched)| self.program_menu_of(app, launched)),
        };
        let Some((items, rows)) = menu else {
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

    /// The menu of jump list row `k`, a file ([`crate::startmenu::jump_file_menu`]).
    fn start_jump_file_menu(&self, k: usize) -> Option<(Vec<MenuItem>, Vec<Option<StartRow>>)> {
        let v = self.start_panel.as_ref()?;
        let reprisekit::startpanel::Mode::Jump(i) = v.model.mode() else {
            return None;
        };
        let path = super::lists::jump_file(&v.jumps, v.model.mode(), k)?;
        let id = self.app_index.get(*v.programs.get(i)?)?.id.as_str();
        let pinned = self.jump_pins.is_pinned(id, path);
        Some(crate::startmenu::jump_file_menu(id, path, pinned))
    }

    fn start_place_menu(&self, i: usize) -> Option<(Vec<MenuItem>, Vec<Option<StartRow>>)> {
        let entry = self.start_panel.as_ref()?.places.get(i)?;
        let kind = crate::startmenu::PlaceMenu::of(entry, &self.recent.document_dirs())?;
        Some(crate::startmenu::place_menu(entry, &kind))
    }

    pub(super) fn open_shutdown_options(&mut self) {
        let Some(arrow) = self.start_spot_rect(Spot::ShutdownArrow) else {
            return;
        };
        let Some(output) = self.start_panel_output() else {
            return;
        };
        let edge = self.bar_placement(&output).edge;
        let menu = crate::startmenu::shutdown_menu(
            !self.config.power.hibernate.is_empty(),
            crate::console::leave_offered(self),
            !self.boot_targets.is_empty(),
            self.offline.known.prepared,
        );
        let (items, rows) = match self.config.startmenu_power {
            PowerButton::ShutDown => menu,
            _ => crate::startmenu::with_shut_down(menu),
        };
        crate::restartinto::refresh(self);
        let kind = crate::menuview::MenuKind::StartPanel { rows };
        let at = (arrow.right(), arrow.y);
        let place = |_, size, usable| shutdown_options_at(edge, arrow, size, usable);
        self.push_start_popup(at, items, kind, place, Skin::Options);
        self.light_flyout_from_keys(Spot::ShutdownArrow);
    }

    /// Recent Items beside its row, else left of the panel (a panel by the screen's right edge).
    /// Too wide for the wider of those rooms, it narrows to it and cuts long names with an
    /// ellipsis; with less than [`RECENT_LEAST`] in both it stands left of its row over the
    /// panel's other column.
    pub(super) fn open_recent_items(&mut self, i: usize) {
        if !self.config.recent_files {
            return;
        }
        let (Some(row), Some(panel)) = (
            self.start_spot_rect(Spot::Place(i)),
            self.start_panel_rect(),
        ) else {
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
        let (items, mut kind, icons) = self.documents_rows(px, false);
        let skin = self.start_flyout_skin();
        let least = self.config.theme.text.len(RECENT_LEAST);
        let room = room_beside(row, panel, self.usable_area(Some(&output)), least);
        if let MenuKind::Documents { widest, .. } = &mut kind {
            *widest = Some(room);
        }
        let place = |_, size, usable| beside_panel(row, panel, row.y, size, usable);
        self.push_start_popup((row.right(), row.y), items, kind, place, skin);
        if let Some(m) = self.menus.first_mut()
            && let MenuKind::Documents { grows_left, .. } = &mut m.kind
        {
            *grows_left = m.x + m.layout.width <= row.x;
        }
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

    /// A flyout the keyboard opened (its row is lit but not under the pointer) starts on its first
    /// row.
    fn light_flyout_from_keys(&mut self, row: Spot) {
        let keys = self
            .start_panel
            .as_ref()
            .is_some_and(|v| v.model.hot() != Some(row));
        if keys && !self.menus.is_empty() {
            self.set_menu_hover(0, Some(0));
        }
    }

    /// The panel's row whose menu stands open beside it: All Programs, Recent Items or the Shut
    /// down arrow. Left in the menu's first column closes it.
    pub(crate) fn start_flyout(&self) -> Option<Spot> {
        let v = self.start_panel.as_ref()?;
        let m = self.menus.first()?;
        if m.from != crate::menuview::MenuFrom::StartPanel {
            return None;
        }
        if m.skin == Skin::Options {
            return Some(Spot::ShutdownArrow);
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

    /// The pointer on a program row on its way to a flyout standing left of the panel, which
    /// stays open for it.
    pub(super) fn crossing_to_flyout(&self, spot: Option<Spot>) -> bool {
        let kind = self.menus.first().map(|m| &m.kind);
        let left = matches!(
            kind,
            Some(MenuKind::Documents {
                grows_left: true,
                ..
            })
        );
        left && matches!(spot, Some(Spot::Program(_)))
    }

    /// Closes the menu [`State::start_flyout`] names, as the pointer moved to another row, when
    /// its row opens it on hover: a menu opened by a click stays.
    pub(super) fn close_start_flyout(&mut self) {
        let Some(v) = self.start_panel.as_ref() else {
            return;
        };
        let hover = match self.start_flyout() {
            Some(Spot::AllPrograms) => true,
            Some(Spot::Place(i)) => v.model.opens_on_hover(i),
            _ => false,
        };
        if hover {
            self.close_menu();
        }
    }

    pub(crate) fn quick_file_of(&self, id: &reprisefs::apps::DesktopId) -> Option<String> {
        self.quicklaunch
            .iter()
            .find(|q| q.app.as_ref().is_some_and(|a| a.id == *id))
            .map(|q| q.file.clone())
    }

    /// Row `r` of the menu of file `path` on the jump list of program `app` (`entry`).
    fn jump_file_row(
        &mut self,
        r: JumpFileRow,
        app: &str,
        path: &std::path::Path,
        entry: Option<reprisefs::apps::AppEntry>,
    ) {
        match r {
            JumpFileRow::Open => {
                self.close_start_panel();
                self.open_recent_with(entry.as_ref(), path);
            }
            JumpFileRow::OpenLocation => {
                self.close_start_panel();
                self.open_file_location(path);
            }
            JumpFileRow::Pin | JumpFileRow::Unpin => {
                let changed = match r {
                    JumpFileRow::Pin => self.jump_pins.pin(app, path),
                    _ => self.jump_pins.unpin(app, path),
                };
                if changed {
                    crate::state::save_jump_pins(&self.jump_pins);
                    self.relist_start_jumps();
                }
            }
            JumpFileRow::Remove => {
                if let Some(e) = entry {
                    self.forget_recent(path.to_path_buf(), e);
                }
            }
        }
    }

    pub(crate) fn start_row(&mut self, row: StartRow) {
        let entry = |s: &State, id: &str| s.app_index.iter().find(|a| a.id == id).cloned();
        match &row {
            StartRow::Open(a) => {
                let e = entry(self, a);
                self.close_start_panel();
                if let Some(e) = e {
                    self.launch_app(&e);
                }
            }
            StartRow::PinTaskbar(a) => {
                if let Some(src) = entry(self, a).and_then(|e| reprisefs::apps::find_file(&e.id)) {
                    self.quick_add(vec![src], None, Vec::new());
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
                match &row {
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
            StartRow::CannotUninstall | StartRow::OnDesktop => {}
            StartRow::JumpFile(r, a, path) => {
                let e = entry(self, a);
                self.jump_file_row(*r, a, path, e);
            }
            StartRow::OpenFileLocation(a) => {
                let file = entry(self, a).and_then(|e| reprisefs::apps::find_file(&e.id));
                self.close_start_panel();
                if let Some(file) = file {
                    self.open_file_location(&file);
                }
            }
            StartRow::AddToDesktop(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                let (Some(file), Some(dir)) = (
                    reprisefs::apps::find_file(&e.id),
                    crate::desktop::desktop_dir(),
                ) else {
                    return;
                };
                let taken = (self.desktop.as_ref())
                    .map(|d| d.entries.iter().map(|e| e.name.clone()).collect())
                    .unwrap_or_default();
                let app = crate::dragview::DragSource::App { entry: e, file };
                self.write_launchers(dir, DropOp::Copy, vec![(app, None, None)], taken);
            }
            StartRow::Properties(a) => {
                let Some(e) = entry(self, a) else {
                    return;
                };
                self.close_start_panel();
                if let Some(file) = reprisefs::apps::find_file(&e.id) {
                    self.show_properties(&[file]);
                }
            }
            StartRow::OpenPlace(e) => {
                let e = e.clone();
                self.close_start_panel();
                self.activate_entry(&e);
            }
            StartRow::FolderProperties(dir) => {
                let dir = dir.clone();
                self.close_start_panel();
                self.show_properties(&[dir]);
            }
            StartRow::ComputerProperties => {
                self.close_start_panel();
                self.spawn_argv_activated(&["reprise-about"]);
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
            StartRow::ShutDown => {
                self.close_start_panel();
                crate::power::run_action(self, PowerAction::TurnOff, Origin::Desktop);
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
            StartRow::RestartInto => {}
            StartRow::InstallUpdates(goal) => {
                let goal = *goal;
                self.close_start_panel();
                let cmd = match goal {
                    Goal::Restart => self.config.power.reboot.clone(),
                    Goal::TurnOff | Goal::LogOff => self.config.power.poweroff.clone(),
                };
                let install = reprisekit::powerpanel::Updates::Install;
                crate::sessionend::begin_updates(self, goal, cmd, install);
            }
            StartRow::Sleep => {
                self.close_start_panel();
                let cmd = self.config.power.standby.clone();
                crate::sessionend::sleep(self, crate::sessionend::Act::StandBy, cmd);
            }
            StartRow::Hibernate => {
                self.close_start_panel();
                let cmd = self.config.power.hibernate.clone();
                crate::sessionend::sleep(self, crate::sessionend::Act::Hibernate, cmd);
            }
        }
    }
}
