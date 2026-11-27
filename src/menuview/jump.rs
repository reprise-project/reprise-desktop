//! Jump lists on task buttons and their recent files.

use reprisekit::tr;
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
        let mut windows = bar.slots.get(i)?.windows.clone();
        windows.retain(|w| live.contains(w));
        let pin = (bar.slots.get(i))
            .and_then(|s| s.pin)
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
            reprisefs::apps::AppId::new(a)
                .and_then(|a| reprisefs::apps::entry_for_app_id(&self.app_index, &a, |_| true))
                .cloned()
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
                dir: e.path.clone(),
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

    /// The jump target of the task button that holds window `w`, and the button.
    pub(crate) fn jump_target_of(&self, w: WindowId) -> Option<(JumpTarget, usize)> {
        self.bars.iter().find_map(|b| {
            let i = b.slots.iter().position(|s| s.windows.contains(&w))?;
            Some((self.jump_target(b.tree, i)?, i))
        })
    }

    /// Whether the program of window `w`'s task button had recent files at the last read; asks for
    /// another.
    pub(crate) fn has_recent(&mut self, w: WindowId) -> bool {
        let Some((target, _)) = self.jump_target_of(w) else {
            return false;
        };
        self.refresh_recent();
        !self.jump_recent(&target).is_empty()
    }

    /// The Recent Files submenu of `target` on `output`: its recent files with their icons, or a
    /// greyed (Empty) once none are left.
    pub(crate) fn recent_menu(
        &mut self,
        target: &JumpTarget,
        output: &Output,
    ) -> (Vec<MenuItem>, Vec<Option<JumpRow>>, RowIcons) {
        let recent = self.jump_recent(target);
        let [files, _] = crate::taskbar::jump_sections(&recent, &[]);
        let px = self.context_icon_px(output);
        let mut waits = RowIcons::default();
        let entry = target.entry.as_ref();
        let (mut items, mut rows) = (Vec::new(), Vec::new());
        for (n, (label, row)) in files.rows.into_iter().enumerate() {
            let icon = crate::taskbar::jump_icon(entry, &row)
                .and_then(|name| self.menu_icon(n, name, px, &mut waits));
            items.push(MenuItem::entry(label).icon(icon));
            rows.push(Some(row));
        }
        if items.is_empty() {
            items.push(MenuItem::entry(tr!("start-empty")).enabled(false));
            rows.push(None);
        }
        (items, rows, waits)
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

    /// `target`'s jump list on `output`: its rows, what each does, and the icons they wait on.
    pub(crate) fn jump_menu(
        &mut self,
        target: &JumpTarget,
        output: &Output,
    ) -> (Vec<MenuItem>, Vec<Option<JumpRow>>, RowIcons) {
        let recent = self.jump_recent(target);
        let titles: Vec<String> = (target.windows.iter())
            .filter_map(|&w| self.win(w))
            .map(|w| w.title())
            .collect();
        let entry = target.entry.as_ref();
        let tasks = entry.map_or(&[][..], crate::taskbar::jump_tasks);
        let (items, rows) = crate::taskbar::jump_list(
            &crate::taskbar::jump_sections(&recent, tasks),
            &target.name,
            target.pin_file.is_some(),
            target.source.is_some(),
            &titles,
            self.desktops.count,
        );
        let px = self.context_icon_px(output);
        let mut waits = RowIcons::default();
        let items = (items.into_iter().zip(&rows).enumerate())
            .map(|(n, (item, row))| {
                let icon = match row {
                    Some(JumpRow::Window(k)) => (target.windows.get(*k))
                        .and_then(|&w| self.menu_window_icon(n, w, px, &mut waits)),
                    Some(row) => crate::taskbar::jump_icon(entry, row)
                        .and_then(|name| self.menu_icon(n, name, px, &mut waits)),
                    None => None,
                };
                item.icon(icon)
            })
            .collect();
        (items, rows, waits)
    }

    pub fn open_jump_list(&mut self, tree: TreeId, i: usize, at: (i32, i32)) {
        let Some(target) = self.jump_target(tree, i) else {
            return;
        };
        let Some(output) = self.output_at(at.0 as f64, at.1 as f64) else {
            return;
        };
        self.refresh_recent();
        let (items, rows, waits) = self.jump_menu(&target, &output);
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
            JumpRow::Task(k) => {
                if let Some(e) = &target.entry
                    && let Some(a) = crate::taskbar::jump_tasks(e).get(k)
                {
                    self.launch_action(e, a);
                }
            }
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
                    self.quick_add(vec![src.clone()], None, Vec::new());
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

    /// Open the recent file `path` in `entry`, else its default program, as
    /// [`State::open_document`] does.
    pub(crate) fn open_recent_with(&mut self, entry: Option<&AppEntry>, path: &std::path::Path) {
        let how = reprisefs::program::Confirm::on_open(self.config.confirm_scripts);
        self.open_document(path, entry.cloned(), how, false);
    }
}

#[cfg(test)]
mod tests {
    use reprisekit::controls::menu::MenuItem;

    /// With `[taskbar] pinned` off, a task button's window menu opens on Recent Files when the
    /// program has recent files, its submenu listing them; another program's menu has no such row.
    #[test]
    fn a_task_buttons_window_menu_offers_the_programs_recent_files() {
        let dir = std::env::temp_dir().join(format!("reprise-recent-menu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let notes = dir.join("notes.txt");
        std::fs::write(&notes, "x").unwrap();
        let xbel = dir.join("recently-used.xbel");
        std::fs::write(
            &xbel,
            format!(
                "<xbel><bookmark href=\"file://{}\" modified=\"2026-01-01T00:00:00Z\">\
                 <bookmark:application name=\"Mail\" exec=\"'mail %u'\" \
                 modified=\"2026-01-01T00:00:00Z\" count=\"1\"/></bookmark></xbel>",
                notes.display()
            ),
        )
        .unwrap();
        let entry = |name: &str, exec: &str| {
            let text = format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec} %f\n");
            let id = format!("{exec}.desktop").parse().unwrap();
            reprisefs::apps::parse_desktop(&text, &id).unwrap()
        };
        let mut config = crate::config::Config::default();
        config.taskbar.pinned = false;
        let mut h = crate::harness::Harness::with_config(config);
        h.output("o", 1024, 768);
        h.state.app_index = vec![entry("Mail", "mail"), entry("Paint", "paint")];
        h.state.recent = crate::recentdocs::RecentDocs::reading(xbel);
        h.state.refresh_recent();
        h.until("the recent files read", |s| {
            !s.recent.current(10).is_empty()
        });
        let mut c = h.connect();
        let mail = h.map_toplevel(&mut c, "mail", 400, 300);
        let paint = h.map_toplevel(&mut c, "paint", 400, 300);
        let labels = |m: &crate::menuview::Menu| -> Vec<String> {
            (m.items.iter())
                .map(|i| match i {
                    MenuItem::Entry { label, .. } => label.clone(),
                    MenuItem::Separator => "-".into(),
                    MenuItem::Heading(t) => t.clone(),
                })
                .collect()
        };
        h.state.open_window_menu(paint.id, (100, 700), true);
        assert!(
            !labels(&h.state.menus[0]).contains(&"Recent Files".to_string()),
            "a program with no recent files"
        );
        h.state.open_window_menu(mail.id, (100, 700), true);
        assert_eq!(
            labels(&h.state.menus[0])[..3],
            ["Recent Files", "-", "Restore"]
        );
        assert!(matches!(
            h.state.menus[0].items[0],
            MenuItem::Entry { submenu: true, .. }
        ));
        h.state.open_submenu(0, 0);
        assert_eq!(labels(&h.state.menus[1]), ["notes.txt"]);
        assert!(matches!(
            &h.state.menus[1].kind,
            crate::menuview::MenuKind::JumpList { rows, .. }
                if rows[..] == [Some(crate::taskbar::JumpRow::Recent(notes.clone()))]
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A running program's task button lists its desktop actions under its recent files, and
    /// picking one runs that action.
    #[test]
    fn a_task_buttons_jump_list_lists_the_programs_tasks_and_runs_one() {
        let text = "[Desktop Entry]\nType=Application\nName=Mail\nExec=mail\nActions=book;\n\
                    [Desktop Action book]\nName=Address Book\nExec=mail-book\n";
        let mut h = crate::harness::Harness::new();
        h.output("o", 1024, 768);
        h.state.app_index =
            vec![reprisefs::apps::parse_desktop(text, &"mail.desktop".parse().unwrap()).unwrap()];
        let mut c = h.connect();
        let w = h.map_toplevel(&mut c, "mail", 400, 300);
        let bar = &h.state.bars[0];
        let i = (bar.slots.iter())
            .position(|s| s.windows == [w.id])
            .expect("the mail window's task button");
        let tree = bar.tree;
        h.state.open_jump_list(tree, i, (100, 700));
        let labels: Vec<&str> = (h.state.menus[0].items.iter())
            .map(|i| match i {
                MenuItem::Entry { label, .. } => label.as_str(),
                MenuItem::Separator => "-",
                MenuItem::Heading(t) => t.as_str(),
            })
            .collect();
        assert_eq!(labels[..4], ["Tasks", "Address Book", "-", "Mail"]);
        crate::spawn::record();
        h.state.menu_activate(0, 1);
        assert_eq!(crate::spawn::recorded(), ["mail-book"]);
    }
}
