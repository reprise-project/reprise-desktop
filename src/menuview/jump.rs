//! Jump lists on task buttons and their recent files.

use reprisekit::tr;
use std::{path::PathBuf, sync::Arc};

use reprisefs::apps::AppEntry;
use reprisescene::TreeId;

use crate::{
    compositor::State, config::Action, raster::Canvas, taskbar::JumpRow, window::WindowId,
};

use super::*;

/// Where a task button's jump list opened: its bar, the button and the press it opened at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpListAt {
    pub tree: TreeId,
    pub button: usize,
    pub at: (i32, i32),
}

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

    /// Whether the program of window `w`'s task button had pinned or recent files at the last
    /// read; asks for another.
    pub(crate) fn has_recent(&mut self, w: WindowId) -> bool {
        let Some((target, _)) = self.jump_target_of(w) else {
            return false;
        };
        self.refresh_recent();
        !self.jump_files(&target).0.is_empty()
    }

    /// The Recent Files submenu of `target` on `output`: its files with their icons, under
    /// Pinned and Recent headings when some are pinned, or a greyed (Empty) once none are left.
    pub(crate) fn recent_menu(
        &mut self,
        target: &JumpTarget,
        output: &Output,
    ) -> (Vec<MenuItem>, Vec<Option<JumpRow>>, RowIcons) {
        let (files, pinned) = self.jump_files(target);
        let [pins, recent, _] = crate::taskbar::jump_sections(&files, pinned, &[]);
        let px = self.context_icon_px(output);
        let mut waits = RowIcons::default();
        let entry = target.entry.as_ref();
        let (mut items, mut rows) = (Vec::new(), Vec::new());
        for section in [pins, recent] {
            if pinned > 0 && !section.rows.is_empty() {
                items.push(MenuItem::heading(section.title));
                rows.push(None);
            }
            for (label, row) in section.rows {
                let icon = crate::taskbar::jump_icon(entry, &row)
                    .and_then(|name| self.menu_icon(items.len(), name, px, &mut waits));
                items.push(MenuItem::entry(label).icon(icon));
                rows.push(Some(row));
            }
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

    /// The files a jump list for `target` lists ([`crate::jumppins::files_for`]) and how many
    /// lead as pinned, from the last recent read and check.
    fn jump_files(&self, target: &JumpTarget) -> (Vec<PathBuf>, usize) {
        match &target.entry {
            Some(e) => {
                let on = self.config.recent_files;
                crate::jumppins::files_for(&self.jump_pins, &self.recent, on, e)
            }
            None => (Vec::new(), 0),
        }
    }

    /// `target`'s jump list on `output`: its rows, what each does, and the icons they wait on.
    pub(crate) fn jump_menu(
        &mut self,
        target: &JumpTarget,
        output: &Output,
    ) -> (Vec<MenuItem>, Vec<Option<JumpRow>>, RowIcons) {
        let (files, pinned) = self.jump_files(target);
        let titles: Vec<String> = (target.windows.iter())
            .filter_map(|&w| self.win(w))
            .map(|w| w.title())
            .collect();
        let entry = target.entry.as_ref();
        let tasks = entry.map_or(&[][..], crate::taskbar::jump_tasks);
        let (items, rows) = crate::taskbar::jump_list(
            &crate::taskbar::jump_sections(&files, pinned, tasks),
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
        self.jump_list_at = Some(JumpListAt {
            tree,
            button: i,
            at,
        });
    }

    /// A right press on file `path` of the jump list at `depth` of program `app`: the jump list
    /// gives way to the file's menu at the pointer. A task button's list (depth 0) opens again
    /// after a pin, unpin or removal.
    pub(crate) fn open_jump_file_menu(&mut self, depth: usize, app: &str, path: &std::path::Path) {
        let pinned = self.jump_pins.is_pinned(app, path);
        let list = self.jump_list_at.filter(|_| depth == 0);
        let (items, rows) = crate::startmenu::jump_file_menu(app, path, pinned, list);
        self.close_menu();
        let p = self.pointer_location();
        let kind = MenuKind::StartPanel { rows };
        self.push_context_menu((p.x as i32, p.y as i32), items, kind, MenuFrom::Bar);
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
    /// program has pinned or recent files, its submenu listing them, under Pinned and Recent
    /// headings once one is pinned; a program with neither has no such row.
    #[test]
    fn a_task_buttons_window_menu_offers_the_programs_pinned_and_recent_files() {
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
        let plan = dir.join("plan.txt");
        h.state.jump_pins.pin("mail.desktop", &plan);
        h.state.jump_pins.pin("paint.desktop", &plan);
        h.state.open_window_menu(mail.id, (100, 700), true);
        h.state.open_submenu(0, 0);
        assert_eq!(
            labels(&h.state.menus[1]),
            ["Pinned", "plan.txt", "Recent", "notes.txt"]
        );
        h.state.open_window_menu(paint.id, (100, 700), true);
        assert_eq!(
            labels(&h.state.menus[0])[0],
            "Recent Files",
            "a pinned file alone"
        );
        h.state.open_submenu(0, 0);
        assert_eq!(labels(&h.state.menus[1]), ["Pinned", "plan.txt"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A right press on a file of a task button's jump list opens the file's menu; Pin to this
    /// list, Unpin from this list and Remove from this list each open the list again, changed.
    #[test]
    fn a_task_buttons_jump_list_file_pins_unpins_and_leaves_from_its_menu() {
        let dir = std::env::temp_dir().join(format!("reprise-jump-pin-{}", std::process::id()));
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
        let text = "[Desktop Entry]\nType=Application\nName=Mail\nExec=mail %f\n";
        let mut h = crate::harness::Harness::new();
        h.output("o", 1024, 768);
        h.state.app_index =
            vec![reprisefs::apps::parse_desktop(text, &"mail.desktop".parse().unwrap()).unwrap()];
        h.state.recent = crate::recentdocs::RecentDocs::reading(xbel);
        h.state.refresh_recent();
        h.until("the recent files read", |s| {
            !s.recent.current(10).is_empty()
        });
        let mut c = h.connect();
        let w = h.map_toplevel(&mut c, "mail", 400, 300);
        let bar = &h.state.bars[0];
        let i = (bar.slots.iter())
            .position(|s| s.windows == [w.id])
            .expect("the mail window's task button");
        let tree = bar.tree;
        let labels = |h: &crate::harness::Harness| -> Vec<String> {
            (h.state.menus[0].items.iter())
                .map(|i| match i {
                    MenuItem::Entry { label, .. } => label.clone(),
                    MenuItem::Separator => "-".into(),
                    MenuItem::Heading(t) => format!("[{t}]"),
                })
                .collect()
        };
        h.state.open_jump_list(tree, i, (100, 700));
        assert_eq!(labels(&h)[..2], ["[Recent]", "notes.txt"]);
        h.state.menu_row_press(0, Some(1), crate::input::BTN_RIGHT);
        assert_eq!(
            labels(&h),
            [
                "Open",
                "Open file location",
                "-",
                "Pin to this list",
                "Remove from this list"
            ]
        );
        let listed = |h: &crate::harness::Harness| {
            assert_eq!(h.state.menus.len(), 1, "only the jump list is open");
            assert!(matches!(
                h.state.menus[0].kind,
                crate::menuview::MenuKind::JumpList { button, .. } if button == i
            ));
        };
        h.state.menu_activate(0, 3);
        listed(&h);
        assert_eq!(labels(&h)[..3], ["[Pinned]", "notes.txt", "-"]);
        h.state.menu_row_press(0, Some(1), crate::input::BTN_RIGHT);
        assert_eq!(labels(&h)[3], "Unpin from this list");
        h.state.menu_activate(0, 3);
        listed(&h);
        assert_eq!(labels(&h)[..3], ["[Recent]", "notes.txt", "-"]);
        h.state.menu_row_press(0, Some(1), crate::input::BTN_RIGHT);
        h.state.menu_activate(0, 4);
        listed(&h);
        h.until("the file leaves the list", |s| {
            !(s.menus[0].items.iter())
                .any(|i| matches!(i, MenuItem::Entry { label, .. } if label == "notes.txt"))
        });
        listed(&h);
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
