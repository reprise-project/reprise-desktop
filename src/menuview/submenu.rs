//! Submenus: the rows a submenu row opens and placing them beside it.

use std::{path::PathBuf, rc::Rc};

use reprisefs::apps::AppEntry;
use smithay::output::Output;

use crate::{
    compositor::State,
    config::{MenuEntry, MenuTarget},
    desktopview::DesktopCommand,
    raster::Rect,
    startmenu::{MenuItem, MenuStyle, place_submenu_inset},
    taskbar::{JumpRow, TaskbarCommand, WindowListRow},
    window::WindowId,
    winmenu::WindowMenuRow,
};

use super::*;

impl State {
    fn submenu_rows(
        &mut self,
        depth: usize,
        row: usize,
    ) -> Option<(Vec<MenuItem>, MenuKind, bool, RowIcons)> {
        let parent = self.menus.get(depth)?;
        let output = parent.output.clone();
        match &parent.kind {
            MenuKind::Taskbar { commands } => {
                if commands.get(row).copied().flatten() != Some(TaskbarCommand::Toolbars) {
                    return None;
                }
                let (items, commands) = crate::taskbar::toolbars_menu(
                    self.bar_shared.quick_shown,
                    self.config.taskbar.pinned,
                );
                return Some((
                    items,
                    MenuKind::Taskbar { commands },
                    true,
                    RowIcons::default(),
                ));
            }
            MenuKind::Window { window, rows }
                if rows.get(row).copied().flatten() == Some(WindowMenuRow::MoveToMenu)
                    && matches!(
                        parent.items.get(row),
                        Some(MenuItem::Entry { enabled: true, .. })
                    ) =>
            {
                let window = *window;
                let own = self.win(self.home(window)).map_or(0, |h| h.desktop);
                let (items, rows) =
                    crate::winmenu::move_to_menu(self.desktops.count, own, WindowMenuRow::MoveTo);
                return Some((
                    items,
                    MenuKind::Window { window, rows },
                    true,
                    RowIcons::default(),
                ));
            }
            MenuKind::Window { window, rows }
                if rows.get(row).copied().flatten() == Some(WindowMenuRow::TabsOfMenu)
                    && matches!(
                        parent.items.get(row),
                        Some(MenuItem::Entry { enabled: true, .. })
                    ) =>
            {
                let window = *window;
                let px = self.context_icon_px(&output);
                let mut waits = RowIcons::default();
                let titled: Vec<(WindowId, String)> = (self.tab_targets(window).into_iter())
                    .filter_map(|t| Some((t, self.win(t)?.title())))
                    .collect();
                let rows = (titled.into_iter().enumerate())
                    .map(|(n, (t, title))| (t, title, self.menu_window_icon(n, t, px, &mut waits)))
                    .collect();
                let (items, rows) = crate::winmenu::tabs_of_menu(rows);
                return Some((items, MenuKind::Window { window, rows }, true, waits));
            }
            MenuKind::JumpList {
                rows,
                target,
                button,
            } if rows.get(row).cloned().flatten() == Some(JumpRow::MoveToMenu) => {
                let d = self.desktops;
                let (items, rows) =
                    crate::winmenu::move_to_menu(d.count, d.current, JumpRow::MoveTo);
                let kind = MenuKind::JumpList {
                    rows,
                    target: target.clone(),
                    button: *button,
                };
                return Some((items, kind, true, RowIcons::default()));
            }
            MenuKind::WindowList {
                windows,
                rows,
                button,
            } if rows.get(row).copied().flatten() == Some(WindowListRow::MoveToMenu) => {
                let d = self.desktops;
                let (items, rows) =
                    crate::winmenu::move_to_menu(d.count, d.current, WindowListRow::MoveTo);
                let kind = MenuKind::WindowList {
                    windows: windows.clone(),
                    rows,
                    button: *button,
                };
                return Some((items, kind, true, RowIcons::default()));
            }
            MenuKind::Desktop { commands, target } => {
                let target = target.clone();
                let command = commands.get(row).cloned().flatten()?;
                let apps = |s: &Self| {
                    target
                        .as_ref()
                        .map(|t| s.open_with_apps(t))
                        .unwrap_or_default()
                };
                let (items, commands, waits) = self.desktop_submenu(command, &output, apps)?;
                return Some((items, MenuKind::Desktop { commands, target }, true, waits));
            }
            MenuKind::File {
                commands,
                path,
                facts,
            } => {
                let (path, facts) = (path.clone(), facts.clone());
                let command = commands.get(row).cloned().flatten()?;
                let apps = |s: &Self| s.open_with_apps_for(&path);
                let (items, commands, waits) = self.desktop_submenu(command, &output, apps)?;
                let kind = MenuKind::File {
                    commands,
                    path,
                    facts,
                };
                return Some((items, kind, true, waits));
            }
            _ => {}
        }
        let (target, folder) = match &parent.kind {
            MenuKind::Entries(entries) => match entries.get(row) {
                Some(MenuEntry::Item { target, .. })
                    if !matches!(target, MenuTarget::Action(_)) =>
                {
                    (target.clone(), None)
                }
                _ => return None,
            },
            MenuKind::Groups { apps, groups } => {
                let g = groups.get(row)?;
                (
                    MenuTarget::Programs,
                    Some(AppRows {
                        apps: apps.clone(),
                        rows: g.apps.clone(),
                    }),
                )
            }
            _ => return None,
        };
        let style = MenuStyle::submenu(&self.config.theme);
        let px = style.icon_px() * self.draw_scale_at(Some(&output)).max(1);
        let mut waits = RowIcons::default();
        let mut app_items = |s: &mut State, apps: &AppRows| -> Vec<MenuItem> {
            (apps.iter().enumerate())
                .map(|(row, a)| {
                    MenuItem::entry(a.name.clone())
                        .icon((a.icon.as_deref()).and_then(|n| s.menu_icon(row, n, px, &mut waits)))
                })
                .collect()
        };
        let (items, kind) = match target {
            MenuTarget::Programs => match folder {
                Some(apps) => (app_items(self, &apps), MenuKind::Programs(apps)),
                None => {
                    let apps: Rc<[AppEntry]> = Rc::from(self.app_index.as_slice());
                    let groups = reprisefs::apps::group(&apps);
                    let items = (groups.iter().enumerate())
                        .map(|(row, g)| {
                            MenuItem::entry(g.name.clone())
                                .icon(self.menu_icon(row, "folder", px, &mut waits))
                                .submenu(true)
                        })
                        .collect();
                    (items, MenuKind::Groups { apps, groups })
                }
            },
            MenuTarget::Settings => {
                let apps: Rc<[AppEntry]> = Rc::from(self.app_index.as_slice());
                let rows = (0..apps.len())
                    .filter(|&i| apps[i].shown && apps[i].in_category("Settings"))
                    .collect();
                let apps = AppRows { apps, rows };
                (app_items(self, &apps), MenuKind::Programs(apps))
            }
            MenuTarget::Documents => {
                self.refresh_recent();
                let (items, kind, icons) = self.documents_rows(px, true);
                waits = icons;
                (items, kind)
            }
            MenuTarget::Submenu(entries) => {
                let (mut items, entry_waits) = self.entry_items(&entries, px);
                waits = entry_waits;
                if items.is_empty() {
                    items.push(MenuItem::entry("(Empty)").enabled(false));
                }
                (items, MenuKind::Entries(entries))
            }
            MenuTarget::Action(_) => return None,
        };
        Some((items, kind, false, waits))
    }

    /// A Documents menu's rows from the last recent read and check: with `with_dirs` the document
    /// folders, then the recent files, else the recent files alone (Recent Items).
    pub(crate) fn documents_rows(
        &mut self,
        px: i32,
        with_dirs: bool,
    ) -> (Vec<MenuItem>, MenuKind, RowIcons) {
        let dirs = if with_dirs {
            self.recent.document_dirs()
        } else {
            Vec::new()
        };
        let show_recent = self.config.recent_files || !with_dirs;
        let recent = if show_recent {
            self.recent.current(RECENT_DOCUMENTS)
        } else {
            Vec::new()
        };
        let mut items: Vec<MenuItem> = Vec::new();
        let mut paths: Vec<Option<PathBuf>> = Vec::new();
        let mut tips: Vec<Option<String>> = Vec::new();
        let name = |p: &PathBuf| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string())
        };
        let mut icons = RowIcons::default();
        for (dir, icon) in &dirs {
            let art = self.menu_icon(items.len(), icon, px, &mut icons);
            items.push(MenuItem::entry(name(dir)).icon(art));
            paths.push(Some(dir.clone()));
            tips.push(crate::desktop::document_tip(icon).map(String::from));
        }
        if show_recent && !dirs.is_empty() {
            items.push(MenuItem::Separator);
            paths.push(None);
            tips.push(None);
        }
        for file in &recent {
            let icon = reprisefs::icons::icon_for_file(file);
            let art = self.menu_icon(items.len(), icon, px, &mut icons);
            items.push(MenuItem::entry(name(file)).icon(art));
            paths.push(Some(file.clone()));
            tips.push(None);
        }
        if show_recent && recent.is_empty() {
            items.push(MenuItem::entry("(Empty)").enabled(false));
            paths.push(None);
            tips.push(None);
        }
        let kind = MenuKind::Documents {
            paths,
            tips,
            dirs: with_dirs,
            icon_px: px,
        };
        (items, kind, icons)
    }

    /// The submenu desktop row `command` opens: Arrange Icons By, New, Open
    /// With (listing `open_with`'s programs) or Send To.
    fn desktop_submenu(
        &mut self,
        command: DesktopCommand,
        output: &Output,
        open_with: impl FnOnce(&Self) -> Vec<(String, String)>,
    ) -> Option<(Vec<MenuItem>, Vec<Option<DesktopCommand>>, RowIcons)> {
        let px = self.context_icon_px(output);
        let mut waits = RowIcons::default();
        let (items, commands) = match command {
            DesktopCommand::ArrangeIcons => {
                crate::desktopview::arrange_menu(self.desktop_auto_arrange())
            }
            DesktopCommand::New => {
                let (mut items, commands) = crate::desktopview::new_menu();
                let named = items.iter_mut().zip(crate::desktopview::NEW_MENU_ICONS);
                for (row, (item, name)) in named.enumerate() {
                    if let (MenuItem::Entry { icon, .. }, Some(name)) = (item, name) {
                        *icon = self.menu_icon(row, name, px, &mut waits);
                    }
                }
                (items, commands)
            }
            DesktopCommand::OpenWith => {
                let apps = open_with(self);
                let (mut items, commands) = crate::desktopview::open_with_menu(&apps);
                for (row, (item, (id, _))) in items.iter_mut().zip(&apps).enumerate() {
                    let name = self
                        .app_index
                        .iter()
                        .find(|a| &a.id == id)
                        .and_then(|a| a.icon.clone());
                    if let (MenuItem::Entry { icon, .. }, Some(name)) = (item, name) {
                        *icon = self.menu_icon(row, &name, px, &mut waits);
                    }
                }
                (items, commands)
            }
            DesktopCommand::SendTo => {
                let view = self.desktop.as_mut()?;
                let rows = view.drives.send_to_rows(&view.send_dirs);
                let (mut items, commands) = crate::desktopview::send_to_menu(&rows);
                let names: Vec<&'static str> = rows.iter().map(|r| r.icon).collect();
                view.send_to = rows;
                for (row, (item, name)) in items.iter_mut().zip(names).enumerate() {
                    if let MenuItem::Entry { icon, .. } = item {
                        *icon = self.menu_icon(row, name, px, &mut waits);
                    }
                }
                (items, commands)
            }
            _ => return None,
        };
        Some((items, commands, waits))
    }

    pub(super) fn open_submenu(&mut self, depth: usize, row: usize) {
        self.schedule_menu_hover(None);
        self.close_menus_above(depth);
        let Some((items, kind, context, waits)) = self.submenu_rows(depth, row) else {
            return;
        };
        let Some(parent) = self.menus.get(depth) else {
            return;
        };
        let Some(&local_row) = parent.layout.rows.get(row) else {
            return;
        };
        let parent_rect = Rect::new(
            parent.x,
            parent.y,
            parent.layout.width,
            parent.layout.height,
        );
        let row_rect = Rect::new(
            parent.x + local_row.x,
            parent.y + local_row.y,
            local_row.width,
            local_row.height,
        );
        let parent_inset = parent.layout.inset.right;
        let output = parent.output.clone();
        let usable = self.usable_area(Some(&output));
        let style = if context {
            MenuStyle::context_for(&self.config.theme, &items)
        } else {
            MenuStyle::submenu(&self.config.theme)
        };
        let layout = style.layout_in(&self.config.theme, &self.text, &items, usable);
        let (x, y) = place_submenu_inset(
            parent_rect,
            row_rect,
            (layout.width, layout.height),
            usable,
            parent_inset,
            layout.inset,
        );
        let start_open = self
            .bars
            .iter()
            .any(|b| b.output == output && b.start_pressed);
        let floor = start_open
            .then(|| {
                let dock = self.config.theme.taskbar_shape().start_menu;
                let p = self.bar_placement(&output);
                start_submenu_shadow_floor(dock, &p, y)
            })
            .flatten();
        let from = self
            .menus
            .get(depth)
            .map_or(MenuFrom::Elsewhere, |m| m.from);
        self.push_menu(depth + 1, (x, y), layout, items, output, kind, from);
        if let Some(m) = self.menus.last_mut() {
            m.parent_row = Some(row);
            m.shadow_floor = floor;
        }
        self.set_row_icons(depth + 1, waits);
        self.redraw_menu(depth + 1);
    }
}
