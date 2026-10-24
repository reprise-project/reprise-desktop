//! What a picked menu row does.

use reprisescene::TreeId;

use crate::{
    compositor::State,
    startmenu::MenuItem,
    taskbar::{QuickCommand, TaskbarCommand},
    window::WindowId,
};

use super::*;

impl State {
    pub fn menu_activate(&mut self, depth: usize, idx: usize) {
        if self.is_tray_menu(depth) {
            crate::tray::menu_activate(self, depth, idx);
            return;
        }
        if self.row_opens_submenu(depth, idx) {
            if self.submenu_open_for(depth, idx) {
                self.close_menus_above(depth);
            } else {
                self.open_submenu(depth, idx);
            }
            return;
        }
        let Some((pick, enabled)) = self.menus.get(depth).map(|m| {
            let enabled = matches!(
                m.items.get(idx),
                Some(MenuItem::Entry { enabled: true, .. })
            );
            (m.kind.pick(idx), enabled)
        }) else {
            return;
        };
        if !enabled {
            return;
        }
        // The field the menu edits stays open under it.
        if let MenuPick::Edit { editor, command } = pick {
            self.close_menu();
            self.edit_menu_pick(editor, command);
            return;
        }
        if pick_closes_start_panel(&pick) {
            self.close_start_panel();
        }
        // A menu open with the launcher is one of its rows'.
        self.close_launcher();
        self.close_menu();
        match pick {
            MenuPick::Nothing => {}
            MenuPick::Run(a) => self.run_action(&a, 0),
            MenuPick::App(a) => self.launch_app(&a),
            MenuPick::Document(d) => {
                // Documents lists the document folders, then recent files; neither is stat'ed.
                let folder = self.recent.is_document_dir(&d);
                let open = if folder {
                    reprisefs::props::Opening {
                        argv: reprisefs::apps::open_folder_argv(&d),
                        app: None,
                    }
                } else {
                    self.open_file(&d)
                };
                self.spawn_opening_activated(&open);
            }
            MenuPick::Window { window, row } => self.window_menu_action(window, row),
            MenuPick::Taskbar(cmd) => self.taskbar_command(cmd),
            MenuPick::Quick { file, command } => match command {
                QuickCommand::Open => {
                    if let Some(k) = self.quicklaunch.iter().position(|q| q.file == file) {
                        self.run_quick(k);
                    }
                }
                QuickCommand::Delete => self.quick_remove(&file),
            },
            MenuPick::Desktop { command, target } => self.desktop_command(command, target),
            MenuPick::File {
                command,
                path,
                facts,
            } => self.file_command(command, path, &facts),
            MenuPick::WindowList { windows, row } => self.window_list_activate(&windows, row),
            MenuPick::Jump { row, target } => self.jump_activate(row, &target),
            MenuPick::NotifyAction { id, key } => crate::notifyview::invoke(self, id, &key),
            MenuPick::Layout(i) => self.set_layout(i),
            MenuPick::NightLight(row) => crate::nightlight::pick(self, row),
            MenuPick::StartPanel(row) => self.start_row(row),
            MenuPick::QuickOverflow(file) => {
                if let Some(k) = self.quicklaunch.iter().position(|q| q.file == file) {
                    self.run_quick(k);
                }
            }
            MenuPick::Edit { .. } => {}
            MenuPick::TaskOverflow { bar, key } => {
                let button = (self.bar(bar))
                    .and_then(|b| crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key));
                if let Some(button) = button {
                    self.task_button_click(bar, button, false);
                }
            }
        }
    }

    /// How the recent file `path` opens, with no stat.
    pub fn open_file(&self, path: &std::path::Path) -> reprisefs::props::Opening {
        let mime = (path.file_name())
            .and_then(|n| reprisefs::apps::mime_globs().mime_of_name(&n.to_string_lossy()));
        let default = mime
            .as_deref()
            .and_then(|m| reprisefs::props::default_app_entry(m, &self.app_index));
        reprisefs::props::open_file(path, default, &self.config.terminal)
    }

    fn taskbar_command(&mut self, cmd: TaskbarCommand) {
        use TaskbarCommand as Tc;
        match cmd {
            Tc::Cascade | Tc::TileHorizontal | Tc::TileVertical => self.arrange_windows(cmd),
            Tc::ShowDesktop => self.toggle_show_desktop(),
            Tc::Toolbars => {}
            Tc::TaskManager => {
                let cmd = self
                    .config
                    .task_manager
                    .clone()
                    .unwrap_or_else(|| format!("{} -e top", self.config.terminal));
                self.spawn_activated(&cmd);
            }
            Tc::Properties => {
                self.spawn_argv_activated(&["reprise-settings", "--page", "taskbar"]);
            }
            Tc::QuickLaunch => self.toggle_quick_shown(),
            Tc::LockTaskbar => self.toggle_taskbar_lock(),
        }
    }

    fn arrange_windows(&mut self, cmd: TaskbarCommand) {
        use TaskbarCommand as Tc;
        let p = self.pointer_location();
        let out = self.output_at(p.x, p.y);
        let usable = self.usable_area(out.as_ref());
        let mut wins: Vec<WindowId> = self
            .task_windows()
            .into_iter()
            .filter(|&w| {
                self.win(w).is_some_and(|x| {
                    !x.hidden() && self.output_at(x.x as f64 + 1.0, x.y as f64 + 1.0) == out
                })
            })
            .collect();
        wins.sort_by_key(|w| self.focus_order.iter().position(|f| f == w));
        let rects = match cmd {
            Tc::Cascade => {
                crate::arrange::cascade(wins.len(), usable, self.config.theme.metrics.cascade_step)
            }
            Tc::TileHorizontal => crate::arrange::tile_horizontal(wins.len(), usable),
            Tc::TileVertical => crate::arrange::tile_vertical(wins.len(), usable),
            _ => return,
        };
        for (&w, r) in wins.iter().zip(&rects) {
            if self.win(w).is_some_and(|x| x.maximized) {
                self.set_maximized(w, false);
            }
            let Some(chrome) = self.win(w).map(|x| x.chrome()) else {
                continue;
            };
            let (cw, ch) = reprisekit::classic::frame::FrameGeometry::client_size_for_total(
                &self.config.theme.metrics,
                r.width,
                r.height,
                false,
                chrome,
            );
            self.move_to(w, r.x, r.y);
            self.request_size(w, cw, ch);
        }
        for &w in &wins {
            self.focus_order.retain(|p| *p != w);
            self.focus_order.push(w);
            if let Some(tree) = self.win(w).map(|x| x.tree) {
                self.scene.raise_to_top(tree);
            }
            self.raise_children(w);
        }
        if let Some(&top) = wins.last() {
            self.focus_window(top);
        }
    }

    /// Show Desktop: with any task window or windowed dialog shown, minimise them all; with none,
    /// restore them all in the stacking order they had.
    pub fn toggle_show_desktop(&mut self) {
        crate::peek::leave(self);
        let mut items: Vec<(TreeId, Shown)> = (self.task_windows().into_iter())
            .filter_map(|w| Some((self.win(w)?.tree, Shown::Window(w))))
            .collect();
        let dialogs = self.dialogs.views.iter().filter(|v| v.dialog.windowed());
        items.extend(dialogs.map(|v| (v.tree(), Shown::Dialog(v.id))));
        let any_visible = items.iter().any(|&(_, it)| match it {
            Shown::Window(w) => self.win(w).is_some_and(|x| !x.hidden()),
            Shown::Dialog(id) => (self.dialogs.views.iter()).any(|v| v.id == id && !v.minimized),
        });
        if !any_visible {
            items = restore_order(self.scene.children(self.layers.windows), items);
        }
        for (_, it) in items {
            match it {
                Shown::Window(w) => {
                    let minimized = self.win(w).is_some_and(|x| x.minimized);
                    if minimized != any_visible {
                        self.set_minimized(w, any_visible);
                    }
                }
                Shown::Dialog(id) if any_visible => self.minimize_dialog(id),
                Shown::Dialog(id) => self.focus_dialog(id),
            }
        }
    }

    fn quick_reload(&mut self) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        let order: Vec<String> = self.quicklaunch.iter().map(|q| q.file.clone()).collect();
        let mut items = crate::quicklaunch::scan(&dir, &self.config.terminal);
        reprisefs::quicklaunch::apply_order(&mut items, &order, |q| &q.file);
        self.quicklaunch = items;
        self.redraw_taskbars();
    }

    /// Adds Quick Launch buttons for `srcs` from `slot` on (the end without one), once a file job
    /// has read each (a dragged `.desktop` file, or whether it is a folder) and written its
    /// button's file.
    pub(crate) fn quick_add(&mut self, srcs: Vec<std::path::PathBuf>, slot: Option<usize>) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        self.spawn_fs_job(move || {
            let files: Vec<Option<String>> = srcs
                .iter()
                .map(|src| crate::quicklaunch::add(&dir, src))
                .collect();
            Box::new(move |state: &mut State| state.quick_added(&files, slot))
        });
    }

    fn quick_added(&mut self, files: &[Option<String>], slot: Option<usize>) {
        self.quick_reload();
        for (n, file) in files.iter().enumerate() {
            let Some(file) = file else {
                continue;
            };
            let len = self.quicklaunch.len();
            if let Some(from) = self.quicklaunch.iter().position(|q| q.file == *file) {
                let to = slot.map_or(len, |s| s + n);
                crate::taskbar::move_button(&mut self.quicklaunch, from, to);
                self.quick_order_changed();
            }
        }
        self.redraw_taskbars();
    }

    pub(crate) fn quick_remove(&mut self, file: &str) {
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        if !self.quicklaunch.iter().any(|q| q.file == file) {
            return;
        }
        crate::quicklaunch::remove(&dir, file);
        self.quick_reload();
    }
}
