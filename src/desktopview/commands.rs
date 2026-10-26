use std::path::{Path, PathBuf};

use reprisefs::program::{Confirm, Program, ScriptOpen};
use reprisefs::{apps, names, ops};
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{
    compositor::State,
    desktop::{self, IconId, Target},
    fsjobs::Action,
};

use super::{DesktopCommand, aimed_in};

impl State {
    fn send_to(&mut self, index: usize, row: usize) {
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let existing: Vec<String> = view.entries.iter().map(|e| e.name.clone()).collect();
        let sources = self.aimed_sources(index, true);
        let beside = view.cells.get(index).map(|c| (c.x, c.y));
        self.send_sources_to(row, &sources, &existing, beside);
    }

    /// Send To row `row` (of the rows the submenu last listed) for `sources`:
    /// a zip named apart from `existing`, a desktop shortcut on the cell
    /// nearest `beside` (else the first free one), a mail or a copy.
    pub(crate) fn send_sources_to(
        &mut self,
        row: usize,
        sources: &[crate::dragview::DragSource],
        existing: &[String],
        beside: Option<(i32, i32)>,
    ) {
        use reprisefs::sendto::SendTo;
        let Some(dest) = (self.desktop.as_ref())
            .and_then(|v| v.send_to.get(row))
            .map(|r| r.dest.clone())
        else {
            return;
        };
        let paths: Vec<PathBuf> = sources.iter().map(|s| s.path().to_path_buf()).collect();
        match dest {
            SendTo::Zip => {
                if let Some(argv) = reprisefs::sendto::zip_argv(&paths, existing) {
                    self.spawn_argv_activated(&argv);
                }
            }
            SendTo::DesktopShortcut => {
                let area = self.desktop_area();
                let (x, y) = beside.unwrap_or((area.x, area.y));
                let target = self.desktop_drop_at(x, y);
                self.perform_all(sources, &target, crate::dragview::DropOp::Shortcut);
            }
            SendTo::Mail => self.spawn_argv_activated(&reprisefs::sendto::mail_argv(&paths)),
            SendTo::Folder(dir) => {
                use crate::dragview::{DropOp, DropTarget};
                // A copy onto a drive that stopped answering would hold a file action until it
                // answers: a fresh stat of the drive goes first, and one out already sends nothing.
                match self.desktop.as_mut().and_then(|v| v.drives.send_stat(&dir)) {
                    None => self.perform_all(sources, &DropTarget::Folder(dir), DropOp::Copy),
                    Some(false) => self.send_to_not_responding(&dir),
                    Some(true) => {
                        let sources = sources.to_vec();
                        let settled = self.send_to_deadline(dir.clone());
                        self.stat_drive(dir.clone(), move |state, usable| {
                            if !settled.swap(true, std::sync::atomic::Ordering::AcqRel) && usable {
                                state.perform_all(&sources, &DropTarget::Folder(dir), DropOp::Copy);
                            }
                        });
                    }
                }
            }
        }
    }

    /// Arms the deadline of a send waiting on drive `dir`'s stat. The flag settles once, by the
    /// stat landing or the deadline passing; past the deadline the send is dropped.
    fn send_to_deadline(&mut self, dir: PathBuf) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        let settled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watch = std::sync::Arc::clone(&settled);
        let timer = Timer::from_duration(self.fsjobs.deadline());
        let armed = self.handle.insert_source(timer, move |_, _, state| {
            if !watch.swap(true, std::sync::atomic::Ordering::AcqRel) {
                state.send_to_not_responding(&dir);
            }
            TimeoutAction::Drop
        });
        if let Err(e) = armed {
            log::warn!("send to: no deadline: {}", e.error);
        }
        settled
    }

    fn send_to_not_responding(&mut self, dir: &Path) {
        log::warn!("send to: {} is not answering; nothing sent", dir.display());
        let text = crate::fsjobs::Action::reads("Send To").not_responding(false);
        self.open_dialog(crate::dialog::Dialog::error("Send To", text));
    }

    /// Allow Launching on `paths`, the first failure in an error box.
    /// Marks `paths` executable and trusted in a file job; the first that fails says why once it
    /// answers.
    pub(crate) fn allow_launching(&mut self, paths: &[PathBuf]) {
        let paths = paths.to_vec();
        let action = Action::changes("Allow Launching", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            if !paths.first().is_some_and(|p| gate.change(p)) {
                return Box::new(|_| {});
            }
            let failed = paths
                .iter()
                .find_map(|p| apps::allow_launching(p).err().map(|e| (p.clone(), e)));
            Box::new(move |state: &mut State| {
                if let Some((path, e)) = failed {
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    let text = format!("Cannot allow {name} to launch: {e}");
                    state.open_dialog(crate::dialog::Dialog::error("Allow Launching", text));
                }
                state.rescan_desktop();
            })
        });
    }

    pub fn open_icon(&mut self, index: usize) {
        let how = Confirm::on_open(self.config.confirm_scripts);
        self.open_icon_as(index, self.config.script_open, how);
    }

    /// [`Self::open_icon`], a script running as `open` says and asking first as `how` says.
    fn open_icon_as(&mut self, index: usize, open: ScriptOpen, how: Confirm) {
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let Some(entry) = view.entries.get(index) else {
            return;
        };
        let argv: Vec<std::ffi::OsString> = match &entry.target {
            Target::Launch(app) => {
                let argv = apps::launch_argv(app, &self.config.terminal);
                let app = app.id.clone();
                self.spawn_app_activated(&argv, &app);
                return;
            }
            Target::Uri(_) => match desktop::place_argv(entry) {
                Some(argv) => argv.into_iter().map(Into::into).collect(),
                None => return,
            },
            Target::Run(file, program) => {
                let (file, program) = (file.clone(), *program);
                return self.run_program(&file, program, open, how);
            }
            // No stat: the file or folder may be on a mount that does not answer.
            Target::Open(dir) if entry.is_dir => apps::open_folder_argv(dir),
            Target::Open(file) => {
                let mime = view.mime.mime_of_name(&entry.name);
                let default = mime
                    .as_deref()
                    .and_then(|m| reprisefs::props::default_app_entry(m, &self.app_index));
                let open = reprisefs::props::open_file(file, default, &self.config.terminal);
                return self.spawn_opening_activated(&open);
            }
        };
        self.spawn_argv_activated(&argv);
    }

    /// Run the program file at `path` in its own folder and an app scope named after it, a script
    /// bare or in the terminal as `open` says. A script `how` asks about first raises
    /// [`Dialog::ask_script`] instead. The exec waits on the file's mount, so it runs off the
    /// loop; a program that does not start gets an error box.
    pub(crate) fn run_program(
        &mut self,
        path: &Path,
        program: Program,
        open: ScriptOpen,
        how: Confirm,
    ) {
        let launch = self.new_launch();
        let env = self.env.clone();
        let path = path.to_path_buf();
        let (terminal, end) = (self.config.terminal.clone(), self.config.script_terminal);
        self.spawn_job(move || {
            if reprisefs::program::asks_first(&path, program, how) {
                return Box::new(move |state: &mut State| {
                    state.open_dialog(crate::dialog::Dialog::ask_script(path, open));
                }) as crate::fsjobs::OnLoop;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let started =
                reprisefs::program::run_argv(&path, open, &terminal, end).and_then(|argv| {
                    crate::spawn::try_spawn_held(&argv, &name, path.parent(), &env, Some(&launch))
                });
            Box::new(move |state: &mut State| {
                // A child that exited before its pid was recorded left a SIGCHLD that found nothing.
                crate::spawn::reap_children();
                if let Err(e) = started {
                    log::warn!("run {}: {e}", path.display());
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    let text = reprisefs::program::run_failed_text(&name, &e);
                    state.open_dialog(crate::dialog::Dialog::error(&name, text));
                }
            })
        });
    }

    pub(crate) fn open_selection(&mut self, index: usize) {
        let how = Confirm::on_open(self.config.confirm_scripts);
        self.open_selection_as(index, self.config.script_open, how);
    }

    /// Opens every icon of the selection around `index`, a script running as `open` says and
    /// asking first as `how` says.
    pub(crate) fn open_selection_as(&mut self, index: usize, open: ScriptOpen, how: Confirm) {
        for i in self.selection_around(index) {
            self.open_icon_as(i, open, how);
        }
    }

    /// A new item `name` exists on disk: land it in the cell under `at` (unless Auto Arrange
    /// flows it), then rescan once.
    fn created(&mut self, name: &str, at: Option<(i32, i32)>) {
        let area = self.desktop_area();
        let auto = self.desktop.as_ref().is_none_or(|v| v.auto_arrange);
        if let Some((x, y)) = at.filter(|_| !auto && area.width > 0) {
            let grid = self.desktop_grid();
            self.land_icon(
                name,
                desktop::to_saved(grid, desktop::cell_at(grid, area, x, y)),
            );
        }
        self.rescan_desktop();
    }

    /// Create a new folder with a unique name ("New Folder", "New Folder (2)", ...)
    /// in the cell under `at` or the next free one. Does nothing while the desktop's
    /// files are off.
    fn new_folder(&mut self, at: Option<(i32, i32)>) {
        if !self.config.desktop_icons {
            return;
        }
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let existing: Vec<String> = view.entries.iter().map(|e| e.name.clone()).collect();
        let name = names::new_folder_name(&existing);
        let (dir, path) = (view.dir.clone(), view.dir.join(&name));
        let action = Action::changes("New Folder", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            if !gate.change(&dir) {
                return Box::new(|_| {});
            }
            let made = std::fs::create_dir(&path);
            Box::new(move |state: &mut State| match made {
                Ok(()) => state.created(&name, at),
                Err(e) => log::warn!("desktop: could not create {}: {e}", path.display()),
            })
        });
    }

    /// Create an empty text file at the first free "New Text Document.txt", "New Text
    /// Document (2).txt", ..., claimed with `O_EXCL`. Does nothing while the
    /// desktop's files are off.
    fn new_text_document(&mut self, at: Option<(i32, i32)>) {
        if !self.config.desktop_icons {
            return;
        }
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let dir = view.dir.clone();
        let action = Action::changes("New Text Document", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            if !gate.change(&dir) {
                return Box::new(|_| {});
            }
            let made = ops::create_new_numbered(&dir, "New Text Document.txt");
            Box::new(move |state: &mut State| match made {
                Ok((_, path)) => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    state.created(&name, at)
                }
                Err(e) => log::warn!("desktop: could not create a text document: {e}"),
            })
        });
    }

    /// Run a desktop-menu command; `aim` is the icon it was aimed at, which may have
    /// since gone.
    /// Open each of `paths` in program `app` (a desktop-file id).
    pub(crate) fn open_files_with(&mut self, paths: Vec<PathBuf>, app: &str) {
        let Some(app) = self.app_index.iter().find(|a| a.id == app).cloned() else {
            return;
        };
        for path in paths {
            let argv = apps::open_argv(&app, &self.config.terminal, &path);
            self.spawn_app_activated(&argv, &app.id);
            self.record_recent(path, Some(app.clone()));
        }
    }

    /// Print each of `paths` (files, not folders) with `lp`, or open it in the text/plain
    /// default whatever its own type.
    pub(crate) fn print_or_edit(&mut self, paths: Vec<PathBuf>, print: bool) {
        let editor = reprisefs::props::default_app_entry("text/plain", &self.app_index).cloned();
        for path in paths {
            let open = if print {
                reprisefs::props::Opening {
                    argv: vec!["lp".into(), path.into_os_string()],
                    app: None,
                }
            } else {
                reprisefs::props::open_file(&path, editor.as_ref(), &self.config.terminal)
            };
            self.spawn_opening_activated(&open);
        }
    }

    pub fn desktop_command(&mut self, cmd: DesktopCommand, aim: Option<IconId>) {
        use DesktopCommand as Dc;
        let target = match &aim {
            Some(id) => {
                let found = self
                    .desktop
                    .as_ref()
                    .and_then(|v| desktop::find(&v.entries, id));
                if found.is_none() {
                    return;
                }
                found
            }
            None => None,
        };
        match cmd {
            Dc::Refresh => self.rescan_desktop(),
            Dc::NextBackground => self.next_slide(),
            Dc::NewFolder => self.new_folder(self.desktop_menu_at),
            Dc::NewTextDocument => self.new_text_document(self.desktop_menu_at),
            Dc::Open => {
                if let Some(i) = target {
                    self.open_selection(i);
                }
            }
            Dc::Run(open) => {
                if let Some(i) = target {
                    self.open_selection_as(i, open, Confirm::Skip);
                }
            }
            Dc::Explore => {
                if let Some(i) = target {
                    self.open_icon(i);
                }
            }
            Dc::Trash => {
                if let Some(i) = target {
                    self.trash_selection(i);
                }
            }
            Dc::Rename => {
                if let Some(i) = target {
                    self.begin_rename(i);
                }
            }
            Dc::CreateShortcut => {
                if let Some(i) = target {
                    let sources = self.drag_sources(i);
                    self.shortcuts_beside(i, &sources);
                }
            }
            Dc::Cut | Dc::Copy => {
                if let Some(i) = target {
                    self.copy_icons(i, cmd == Dc::Cut);
                }
            }
            Dc::Paste | Dc::PasteShortcut => {
                // The same as dropping the files on the cell under the
                // right click.
                let area = self.desktop_area();
                let at = self.desktop_menu_at.unwrap_or((area.x, area.y));
                let target = self.desktop_drop_at(at.0, at.1);
                self.paste_files(target, cmd == Dc::PasteShortcut);
            }
            Dc::PasteInto => {
                let dir = target
                    .and_then(|i| self.desktop.as_ref()?.entries.get(i))
                    .map(|e| e.path.clone());
                if let Some(dir) = dir {
                    self.paste_files(crate::dragview::DropTarget::Folder(dir), false);
                }
            }
            Dc::PasteOpen => {
                let app = target
                    .and_then(|i| self.desktop.as_ref()?.entries.get(i))
                    .and_then(|e| match &e.target {
                        Target::Launch(app) => Some(app.clone()),
                        _ => None,
                    });
                if let Some(app) = app {
                    self.paste_open(app);
                }
            }
            Dc::AllowLaunching => {
                let Some(i) = target else {
                    return;
                };
                // The row shows for a launcher that is not trusted yet;
                // the rest of the set gets the same where it is one.
                let set = self.selection_around(i);
                let paths: Vec<PathBuf> = match self.desktop.as_ref() {
                    Some(view) => aimed_in(&view.entries, &set, i, true)
                        .into_iter()
                        .map(|i| &view.entries[i])
                        .filter(|e| e.allow_launching)
                        .map(|e| e.path.clone())
                        .collect(),
                    None => Vec::new(),
                };
                self.allow_launching(&paths);
            }
            Dc::Settings => {
                self.spawn_argv_activated(&["reprise-settings", "--page", "background"])
            }
            Dc::RepriseSettings => self.spawn_argv_activated(&["reprise-settings"]),
            Dc::AboutReprise => self.spawn_argv_activated(&["reprise-about"]),
            Dc::OpenWithApp(app) => {
                let Some(i) = target else {
                    return;
                };
                self.open_files_with(self.aimed_files(i), &app);
            }
            Dc::Print | Dc::Edit => {
                let Some(i) = target else {
                    return;
                };
                self.print_or_edit(self.aimed_files(i), cmd == Dc::Print);
            }
            Dc::ArrangeByName => self.arrange_by_name(),
            Dc::AutoArrange => self.toggle_auto_arrange(),
            Dc::DropCopy | Dc::DropMove | Dc::DropShortcut => {
                use crate::dragview::DropOp;
                let op = match cmd {
                    Dc::DropCopy => DropOp::Copy,
                    Dc::DropMove => DropOp::Move,
                    _ => DropOp::Shortcut,
                };
                // Another program's drop hears the answer first, and one
                // its source does not allow is cancelled.
                if let Some((sources, target)) = self.drags.pending_drop.take()
                    && self.drop_menu_answered(Some(op))
                {
                    self.perform_all(&sources, &target, op);
                }
            }
            Dc::DropCancel => {
                self.drags.pending_drop = None;
                self.drop_menu_answered(None);
            }
            Dc::SendToDest(row) => {
                if let Some(i) = target {
                    self.send_to(i, row);
                }
            }
            // Only ever a submenu opener; picking it does nothing.
            Dc::ArrangeIcons | Dc::New | Dc::OpenWith | Dc::SendTo => {}
            // Only the launcher's file menu (`filemenu`) has this row.
            Dc::OpenFileLocation => {}
            Dc::IconProperties => {
                if let Some(i) = target {
                    self.open_icon_properties(i);
                }
            }
            Dc::ChooseProgram => {
                if let Some(i) = target {
                    self.open_icon_chooser(i);
                }
            }
            Dc::BinProperties => {
                if let Some(i) = target {
                    self.open_bin_properties(i);
                }
            }
            Dc::EmptyBin => self.ask_empty_bin(),
        }
    }
}
