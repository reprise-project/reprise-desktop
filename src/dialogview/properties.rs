//! Properties sheets, Open With and Change Icon: opening them and writing what they set.

use crate::compositor::State;
use crate::dialog::{self, Dialog};
use crate::fsjobs::Action;

use super::*;

impl State {
    /// Open the Properties sheet for `info`, or raise the one open on its file. An unwalked folder reads
    /// "Calculating..." until a walk off the loop returns totals.
    pub fn open_properties(
        &mut self,
        info: &reprisefs::props::FileInfo,
        icon: &str,
        at: (i32, i32),
    ) {
        let open_already = self.sheet_on(&info.path).is_some();
        self.open_dialog(Dialog::properties(info, icon, at));
        if open_already || !info.is_dir || info.contains.is_some() {
            return;
        }
        let Some(id) = self.sheet_on(&info.path) else {
            return;
        };
        let path = info.path.clone();
        self.spawn_fs_job(move || {
            let totals = reprisefs::props::folder_totals(&path);
            Box::new(move |state| state.folder_totals_in(id, totals))
        });
    }

    /// A folder walk for sheet `id` is done; a sheet closed meanwhile drops the totals.
    fn folder_totals_in(&mut self, id: u64, totals: reprisefs::props::FolderTotals) {
        let Some(i) = self.dialog_index(id) else {
            return;
        };
        if self.dialogs.views[i].dialog.set_folder_totals(totals) {
            self.redraw_dialog(i);
        }
    }

    /// The id of the open Properties sheet on `path`; `open_dialog` keeps it unique.
    fn sheet_on(&self, path: &std::path::Path) -> Option<u64> {
        self.dialogs
            .views
            .iter()
            .find(|v| v.dialog.sheet_path() == Some(path))
            .map(|v| v.id)
    }

    /// The rest of a Properties OK or Apply after `write_properties`: the desktop follows a rename to
    /// `renamed`, then sheet `sheet` closes or stays open on `written`; a closed sheet is left alone.
    fn finish_properties(
        &mut self,
        sheet: Option<u64>,
        path: std::path::PathBuf,
        renamed: Option<std::path::PathBuf>,
        written: &dialog::Written,
        header: Option<crate::desktop::DesktopEntry>,
        close: bool,
    ) {
        if let Some(to) = &renamed {
            self.desktop_renamed(&path, to);
        }
        let Some(i) = sheet.and_then(|id| self.dialog_index(id)) else {
            return;
        };
        if close {
            self.close_dialog_at(i);
            return;
        }
        match &written.reread {
            // A rewritten shortcut's sheet is rebuilt, its header wearing the icon the desktop gives it now.
            Some(info) => {
                // The desktop may still list the file under its name before a rename.
                let on_desktop = self.desktop.as_ref().filter(|v| {
                    v.entries
                        .iter()
                        .any(|e| e.path == written.path || e.path == path)
                });
                let worn = on_desktop.zip(header).map(|(v, mut e)| {
                    let has = |n: &str| self.icons.has(n);
                    crate::desktop::wear_file_icons(std::slice::from_mut(&mut e), &v.mime, &has);
                    e.icon
                });
                let d = &mut self.dialogs.views[i].dialog;
                let icon = worn.or_else(|| d.icon.clone()).unwrap_or_default();
                d.sheet_reread(info, &icon);
                self.refit_dialog(i);
            }
            None => self.dialogs.views[i].dialog.sheet_applied(written),
        }
        self.redraw_dialog(i);
    }

    /// OK on the Open With chooser: run `path` with `app` (a desktop-file id, or a program path from
    /// Browse...) and with `always` make it the default for `mime`; a browsed program gets a desktop
    /// entry first ([`reprisefs::apps::program_entry`]).
    pub(super) fn apply_open_with(
        &mut self,
        path: &std::path::Path,
        app: &str,
        always: bool,
        mime: Option<&str>,
    ) {
        self.close_dialog();
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let written = match (always, mime) {
            (true, Some(mime)) => browsed_entry(apps, app, mime),
            _ => None,
        };
        match apps.iter().find(|a| a.id == app).or(written.as_ref()) {
            Some(entry) => {
                if let (true, Some(mime)) = (always, mime)
                    && let Err(e) = reprisefs::props::set_default_app(mime, &entry.id)
                {
                    log::warn!("open with: could not set the default for {mime}: {e}");
                }
                let argv = reprisefs::apps::open_argv(entry, &self.config.terminal, path);
                let app = entry.id.clone();
                self.spawn_app_activated(&argv, &app);
            }
            // A browsed program not made default runs by path with the file as its one argument.
            None if app.starts_with('/') => {
                let argv = [std::ffi::OsStr::new(app), path.as_os_str()];
                self.spawn_argv_activated(&argv);
            }
            None => {}
        }
    }

    /// OK on the chooser a sheet's Change... opened: make `app` the default for `mime` and show it on
    /// the sheet for `path`; a browsed program gets a desktop entry first.
    pub(super) fn apply_set_default(
        &mut self,
        path: &std::path::Path,
        app: &str,
        mime: Option<&str>,
    ) {
        let sheet_open = self.close_to_parent();
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let written = mime.and_then(|m| browsed_entry(apps, app, m));
        let Some(entry) = apps.iter().find(|a| a.id == app).or(written.as_ref()) else {
            return;
        };
        let (name, icon) = (entry.name.clone(), entry.icon.clone());
        if let Some(mime) = mime
            && let Err(e) = reprisefs::props::set_default_app(mime, &entry.id)
        {
            log::warn!("change: could not set the default for {mime}: {e}");
            return;
        }
        // The sheet is back on top; its Opens with row shows the new program.
        let Some(i) = self.dialogs.views.len().checked_sub(1) else {
            return;
        };
        if !sheet_open {
            return;
        }
        let sheet = &mut self.dialogs.views[i];
        if sheet.dialog.sheet_path() == Some(path) && sheet.dialog.set_opens_with(name, icon) {
            self.redraw_dialog(i);
        }
    }

    /// A Properties OK or Apply: Read-only and Hidden (asking first for a non-empty folder), then the rename.
    pub(super) fn apply_properties(&mut self, write: Props, close: bool, recurse: Option<bool>) {
        let sheet = self.sheet_on(&write.path);
        match (!write.attrs.is_empty() || write.hidden.is_some(), recurse) {
            // A non-empty folder gets Confirm Attribute Changes first, since the change can reach the
            // whole tree; the look is a folder read, so it runs off the loop.
            (true, None) => {
                self.spawn_fs_action(Action::reads("Properties"), move |_| {
                    let ask = has_entries(&write.path);
                    Box::new(move |state| {
                        if ask {
                            let c = dialog::AttrConfirm {
                                write: dialog::AttrWrite::One {
                                    path: write.path,
                                    name: write.name,
                                    hidden: write.hidden,
                                    folder_icon: write.folder_icon,
                                },
                                attrs: write.attrs,
                                close,
                            };
                            state.confirm_attributes_over(sheet, c);
                        } else {
                            state.write_properties(sheet, write, false, close);
                        }
                    })
                });
            }
            _ => {
                if recurse.is_some() {
                    self.close_dialog();
                }
                self.write_properties(sheet, write, recurse == Some(true), close);
            }
        }
    }

    /// A set's Properties OK or Apply: Confirm Attribute Changes first when Read-only or Hidden moved
    /// and a non-empty folder is among the paths; then `write_set_properties`.
    pub(super) fn apply_set_properties(
        &mut self,
        write: SetWrite,
        close: bool,
        recurse: Option<bool>,
    ) {
        let sheet = self
            .dialogs
            .views
            .iter()
            .find(|v| v.dialog.set_paths() == Some(write.paths.as_slice()))
            .map(|v| v.id);
        match (!write.attrs.is_empty() || write.hidden.is_some(), recurse) {
            (true, None) => {
                self.spawn_fs_action(Action::reads("Properties"), move |_| {
                    let ask = write.paths.iter().any(|p| has_entries(p));
                    Box::new(move |state| {
                        if ask {
                            let c = dialog::AttrConfirm {
                                write: dialog::AttrWrite::Set {
                                    paths: write.paths,
                                    hidden: write.hidden,
                                    folder_icon: write.folder_icon,
                                },
                                attrs: write.attrs,
                                close,
                            };
                            state.confirm_attributes_over(sheet, c);
                        } else {
                            state.write_set_properties(sheet, write, false, close);
                        }
                    })
                });
            }
            _ => {
                if recurse.is_some() {
                    self.close_dialog();
                }
                let tree = recurse == Some(true);
                self.write_set_properties(sheet, write, tree, close);
            }
        }
    }

    /// Write a set's Properties off the loop (`reprisefs::props::write_set`); then the desktop follows
    /// renames and sheet `sheet` closes or takes the new paths.
    fn write_set_properties(
        &mut self,
        sheet: Option<u64>,
        write: SetWrite,
        tree: bool,
        close: bool,
    ) {
        let action = Action::changes("Properties", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            let SetWrite {
                paths,
                attrs,
                hidden,
                folder_icon,
            } = write;
            if !paths.first().is_some_and(|p| gate.change(p)) {
                return Box::new(|_| {});
            }
            let mut failed = Vec::new();
            if let Some(icon) = &folder_icon {
                for dir in paths.iter().filter(|p| p.is_dir()) {
                    if let Err(e) = reprisefs::meta::set_folder_icon(dir, icon.as_deref()) {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            dir.display()
                        );
                        failed.push((dir.clone(), e.to_string()));
                    }
                }
            }
            let w = reprisefs::props::write_set(paths, attrs, hidden, tree);
            let failed = failed.into_iter().chain(w.failed).next().map(|(p, e)| {
                let name = p.file_name().unwrap_or(p.as_os_str());
                format!("Cannot change {}: {e}", name.to_string_lossy())
            });
            let icons = folder_icon.is_some();
            Box::new(move |state| {
                if icons {
                    state.rescan_desktop();
                }
                state.finish_set_properties(sheet, w.now, &w.renames, close);
                if let Some(text) = failed {
                    state.open_dialog_over_id(Dialog::error("Properties", text), sheet);
                }
            })
        });
    }

    /// The Change Icon box for the folder `name`, over the top sheet: icons from
    /// `filesheet::FOLDER_ICON_NAMES`, a folder's own icon first, `current` picked (standard folder for
    /// None). A launcher (`launcher`) lists installed programs' icons (`reprisefs::launcher::icon_names`).
    pub(super) fn open_change_icon(&mut self, name: &str, current: Option<String>, launcher: bool) {
        use reprisekit::controls::filesheet::{FOLDER_ICON_NAMES, STANDARD_FOLDER_ICON};
        let scale = self
            .dialogs
            .views
            .last()
            .map_or(1, |v| self.draw_scale_at(Some(&v.output)).max(1));
        let apps: &[reprisefs::apps::AppEntry] = &self.app_index;
        let icons = &mut self.icons;
        let mut names: Vec<String> = if launcher {
            reprisefs::launcher::icon_names(apps, |n| icons.has(n))
        } else {
            FOLDER_ICON_NAMES
                .iter()
                .filter(|n| icons.has(n))
                .map(|n| n.to_string())
                .collect()
        };
        // A folder always offers the standard folder; either offers its own icon from outside the list first.
        let standard = (!launcher).then(|| STANDARD_FOLDER_ICON.to_string());
        for must in [standard, current.clone()].into_iter().flatten() {
            if !names.contains(&must) {
                names.insert(0, must);
            }
        }
        let pictures = names
            .iter()
            .map(|n| icons.get(n, dialog::ICON_PX * scale))
            .collect();
        let mut list = reprisekit::controls::iconlist::IconList::new(names, pictures, None);
        match current.as_deref() {
            Some(c) => {
                list.pick(c);
            }
            None if !launcher => {
                list.pick(STANDARD_FOLDER_ICON);
            }
            None => {}
        }
        let source = self.config.icons.theme.clone();
        self.open_dialog_over(Dialog::change_icon(name, &source, list, launcher));
    }

    /// The rest of a set's OK or Apply after `write_set_properties`: the desktop follows renames, then
    /// sheet `sheet` closes or takes the items' paths `now`; a closed sheet is left alone.
    fn finish_set_properties(
        &mut self,
        sheet: Option<u64>,
        now: Vec<std::path::PathBuf>,
        renames: &[(std::path::PathBuf, std::path::PathBuf)],
        close: bool,
    ) {
        for (from, to) in renames {
            self.desktop_renamed(from, to);
        }
        let Some(i) = sheet.and_then(|id| self.dialog_index(id)) else {
            return;
        };
        if close {
            self.close_dialog_at(i);
        } else {
            self.dialogs.views[i].dialog.set_applied(now);
            self.redraw_dialog(i);
        }
    }

    /// Open the combined Properties sheet for `set` titled `name`, or raise the one open on it. Unwalked
    /// folders are walked off the loop and their totals join the sheet's.
    pub fn open_set_properties(
        &mut self,
        set: &reprisefs::props::SetInfo,
        name: &str,
        at: (i32, i32),
    ) {
        let find = |state: &State| {
            state
                .dialogs
                .views
                .iter()
                .find(|v| v.dialog.set_paths() == Some(set.paths.as_slice()))
                .map(|v| v.id)
        };
        let open_already = find(self).is_some();
        self.open_dialog(Dialog::set_properties(set, name, at));
        if open_already || !set.walking {
            return;
        }
        let Some(id) = find(self) else {
            return;
        };
        let paths = set.paths.clone();
        self.spawn_fs_job(move || {
            let mut sum = reprisefs::props::FolderTotals::default();
            for d in paths.iter().filter(|p| p.is_dir()) {
                let t = reprisefs::props::folder_totals(d);
                sum.files += t.files;
                sum.folders += t.folders;
                sum.size += t.size;
                sum.on_disk += t.on_disk;
            }
            Box::new(move |state| state.folder_totals_in(id, sum))
        });
    }

    /// Ask Confirm Attribute Changes over sheet `sheet`, raised first; a closed sheet drops the change.
    fn confirm_attributes_over(&mut self, sheet: Option<u64>, c: dialog::AttrConfirm) {
        let Some(i) = sheet.and_then(|id| self.dialog_index(id)) else {
            return;
        };
        self.raise_dialog(i);
        let t = &self.config.theme;
        let d = Dialog::confirm_attributes(c, &self.text, t.fonts.size_px, t.text);
        self.open_view(d, sheet);
    }

    /// Write a Properties OK or Apply off the loop, so a hung network mount cannot stall the desktop:
    /// Summary fields, launcher keys, folder icon, attributes and Hidden lists (`reprisefs::props::write_item`,
    /// whole tree with `tree`), then the rename to `name`. `finish_properties` follows on the loop.
    fn write_properties(&mut self, sheet: Option<u64>, w: Props, tree: bool, close: bool) {
        let reread = w
            .rereads()
            .then(|| self.dialogs.views.iter().find(|v| Some(v.id) == sheet))
            .flatten()
            .and_then(|v| v.dialog.sheet_name())
            .map(|name| (name.to_string(), self.app_index.clone()));
        let mime = self.desktop.as_ref().map(|v| v.mime.clone());
        let shown = self.config.desktop_extensions;
        let action = Action::changes("Properties", State::rescan_desktop);
        self.spawn_fs_action(action, move |gate| {
            let Props {
                path,
                name,
                attrs,
                hidden,
                summary,
                folder_icon,
                launcher,
            } = w;
            if !gate.change(&path) {
                return Box::new(|_| {});
            }
            // Every write is tried; the first failure is shown and each landed field goes back as written.
            let mut failed: Option<std::io::Error> = None;
            let mut note = |r: std::io::Result<()>| match r {
                Ok(()) => true,
                Err(e) => {
                    failed.get_or_insert(e);
                    false
                }
            };
            let summary_ok = note(reprisefs::meta::write_summary(&path, &summary));
            let launcher_ok = launcher.is_empty()
                || note(
                    reprisefs::launcher::write_keys(&path, &launcher).inspect_err(|e| {
                        log::warn!("properties: could not write {}: {e}", path.display())
                    }),
                );
            let icon_ok = match &folder_icon {
                Some(icon) => note(
                    reprisefs::meta::set_folder_icon(&path, icon.as_deref()).inspect_err(|e| {
                        log::warn!(
                            "properties: could not set the icon of {}: {e}",
                            path.display()
                        )
                    }),
                ),
                None => true,
            };
            let attrs_ok = note(reprisefs::props::write_item(&path, &attrs, hidden, tree));
            let mut rename_ok = true;
            let renamed = name.map(|n| path.with_file_name(n)).filter(|to| {
                let r = reprisefs::hidden::rename(&path, to).inspect_err(|e| {
                    log::warn!("properties: could not rename {}: {e}", path.display())
                });
                rename_ok = note(r);
                rename_ok
            });
            let failed = failed.map(|e| {
                let name = path.file_name().unwrap_or(path.as_os_str());
                format!("Cannot change {}: {e}", name.to_string_lossy())
            });
            let icon = folder_icon.is_some() || !launcher.is_empty();
            let now = renamed.clone().unwrap_or_else(|| path.clone());
            // A new target or execute bit can change the header icon, Opens with, Description,
            // Target rows and tabs.
            let reread = reread
                .filter(|_| launcher_ok && attrs_ok)
                .and_then(|(name, apps)| reprisefs::props::glance(&now, &name, &apps).ok());
            // The header wears the icon the desktop gives the rewritten file, read here rather
            // than waiting on the desktop's rescan.
            let header = reread
                .as_ref()
                .zip(mime)
                .map(|(_, mime)| crate::desktop::entry_at(&now, &mime, shown));
            let written = dialog::Written {
                path: now,
                attrs: if attrs_ok { attrs } else { Default::default() },
                hidden: hidden.filter(|_| attrs_ok && rename_ok),
                summary: if summary_ok { summary } else { Vec::new() },
                folder_icon: folder_icon.filter(|_| icon_ok),
                launcher: if launcher_ok { launcher } else { Vec::new() },
                reread,
            };
            Box::new(move |state| {
                // The desktop draws the new icon.
                if icon {
                    state.rescan_desktop();
                }
                let ok = failed.is_none();
                state.finish_properties(sheet, path, renamed, &written, header, close && ok);
                if let Some(text) = failed {
                    state.open_dialog_over_id(Dialog::error("Properties", text), sheet);
                }
            })
        });
    }
}

/// The desktop entry written for `app` when it is a program path picked through Browse..., so it can
/// be `mime`'s default; None for an id or when the entry cannot be written.
fn browsed_entry(
    apps: &[reprisefs::apps::AppEntry],
    app: &str,
    mime: &str,
) -> Option<reprisefs::apps::AppEntry> {
    if !app.starts_with('/') || apps.iter().any(|a| a.id == app) {
        return None;
    }
    reprisefs::apps::program_entry(std::path::Path::new(app), Some(mime))
        .map_err(|e| log::warn!("open with: no desktop entry for {app}: {e}"))
        .ok()
}

fn has_entries(path: &std::path::Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}
