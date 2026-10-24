//! Jump lists on task buttons and their recent files.

use std::{path::PathBuf, sync::Arc};

use reprisefs::apps::AppEntry;
use reprisescene::TreeId;

use crate::{
    compositor::State, config::Action, raster::Canvas, taskbar::JumpRow, window::WindowId,
};

use super::*;

impl State {
    fn jump_target(&self, tree: TreeId, i: usize) -> Option<JumpTarget> {
        let bar = self.bar(tree)?;
        let live = self.task_windows();
        let mut windows = bar.buttons.get(i)?.clone();
        windows.retain(|w| live.contains(w));
        let pin = bar
            .pins
            .get(i)
            .copied()
            .flatten()
            .and_then(|k| self.quicklaunch.get(k));
        let app_id = windows
            .first()
            .and_then(|&w| self.win(w))
            .map(|w| w.app_id());
        if pin.is_none() && app_id.is_none() {
            return None;
        }
        let entry = pin.and_then(|q| q.app.clone()).or_else(|| {
            let a = app_id.as_deref()?;
            reprisefs::apps::entry_for_app_id(&self.app_index, a, |_| true).cloned()
        });
        let terminal = &self.config.terminal;
        let name = match (pin, &entry) {
            (Some(q), _) => q.entry.label.clone(),
            (None, Some(e)) => e.name.clone(),
            (None, None) => crate::taskbar::group_name(app_id.as_deref().unwrap_or("")),
        };
        let launch = match (pin, &entry) {
            (Some(q), _) => Some(q.entry.action.clone()),
            (None, Some(e)) => Some(Action::Exec {
                argv: reprisefs::apps::launch_argv(e, terminal),
                app: e.id.clone(),
            }),
            (None, None) => None,
        };
        let source = match pin {
            Some(_) => None,
            None => entry
                .as_ref()
                .and_then(|e| reprisefs::apps::find_file(&e.id)),
        };
        Some(JumpTarget {
            name,
            launch,
            entry,
            pin_file: pin.map(|q| q.file.clone()),
            source,
            windows,
        })
    }

    pub(crate) fn window_icon(&mut self, w: WindowId, px: i32) -> Option<Canvas> {
        let win = self.windows.iter().find(|x| x.id == w)?;
        crate::icon::window_icon(&mut self.icons, &self.app_index, win, px)
            .map(Arc::unwrap_or_clone)
    }

    /// The recent files a jump list for `target` lists, from the last recent read and check.
    pub(crate) fn jump_recent(&mut self, target: &JumpTarget) -> Vec<PathBuf> {
        match &target.entry {
            Some(e) if self.config.recent_files => {
                self.recent.for_app(crate::taskbar::JUMP_RECENT, e)
            }
            _ => Vec::new(),
        }
    }

    pub fn open_jump_list(&mut self, tree: TreeId, i: usize, at: (i32, i32)) {
        let Some(target) = self.jump_target(tree, i) else {
            return;
        };
        self.refresh_recent();
        let recent = self.jump_recent(&target);
        let titles: Vec<String> = target
            .windows
            .iter()
            .filter_map(|&w| self.win(w))
            .map(|w| w.title())
            .collect();
        let (items, rows) = crate::taskbar::jump_list(
            &target.name,
            &recent,
            target.pin_file.is_some(),
            target.source.is_some(),
            &titles,
            self.desktops.count,
        );
        let Some(output) = self.output_at(at.0 as f64, at.1 as f64) else {
            return;
        };
        let px = self.context_icon_px(&output);
        let mut waits = RowIcons::default();
        let items = (items.into_iter().zip(&rows).enumerate())
            .map(|(n, (item, row))| match row {
                Some(JumpRow::Window(k)) => {
                    let icon = (target.windows.get(*k))
                        .and_then(|&w| self.menu_window_icon(n, w, px, &mut waits));
                    item.icon(icon)
                }
                _ => item,
            })
            .collect();
        self.close_menu();
        let kind = MenuKind::JumpList {
            rows,
            target: Box::new(target),
            button: i,
        };
        self.push_context_menu(at, items, kind, MenuFrom::Bar);
        self.set_row_icons(0, waits);
    }

    pub(super) fn jump_activate(&mut self, row: JumpRow, target: &JumpTarget) {
        let live = self.task_windows();
        let windows: Vec<WindowId> = target
            .windows
            .iter()
            .copied()
            .filter(|w| live.contains(w))
            .collect();
        match row {
            JumpRow::Recent(path) => self.open_recent_with(target.entry.as_ref(), &path),
            JumpRow::Window(k) => {
                if let Some(&w) = target.windows.get(k)
                    && live.contains(&w)
                {
                    self.bring_up(w);
                }
            }
            JumpRow::Launch => {
                if let Some(a) = &target.launch {
                    self.run_action(a, 0);
                }
            }
            JumpRow::Pin => {
                if let Some(src) = &target.source {
                    self.quick_add(vec![src.clone()], None);
                }
            }
            JumpRow::Unpin => {
                if let Some(f) = &target.pin_file {
                    self.quick_remove(f);
                }
            }
            JumpRow::Close => {
                for w in windows {
                    self.close(w);
                }
            }
            JumpRow::MoveToMenu => {}
            JumpRow::MoveTo(n) => self.move_to_desktop(&windows, n),
        }
    }

    pub(crate) fn open_recent_with(&mut self, entry: Option<&AppEntry>, path: &std::path::Path) {
        match entry {
            Some(e) => {
                let argv = reprisefs::apps::open_argv(e, &self.config.terminal, path);
                self.spawn_app_activated(&argv, &e.id);
            }
            None => {
                let open = self.open_file(path);
                self.spawn_opening_activated(&open);
            }
        }
    }
}
