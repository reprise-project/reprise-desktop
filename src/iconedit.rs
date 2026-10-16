//! Editing the desktop icons: inline rename, the file clipboard (Cut, Copy, Paste, Paste Shortcut) and the
//! keys on the icons. Delete and Properties go through `desktopfiles`.

use std::path::{Path, PathBuf};

use reprisekit::textedit::EditKey;
use smithay::input::keyboard::Keycode;

use crate::{
    clipboard::{FileClip, PasteTarget, file_offers},
    compositor::State,
    desktop::{self, DesktopEntry, IconMove},
    desktopview::{DesktopCommand, aimed_in},
    dragview::{DragSource, DropOp, DropTarget},
    keys,
};

/// What a Paste of `paths` drops: a path with an icon on the desktop as
/// that icon (Documents is a place, not a file), any other as a file
/// named after itself.
pub fn clip_sources(entries: &[DesktopEntry], paths: &[PathBuf]) -> Vec<DragSource> {
    paths
        .iter()
        .map(|p| match entries.iter().find(|e| e.path == *p) {
            Some(e) => DragSource::Icon {
                id: e.id(),
                path: e.path.clone(),
                name: e.shortcut_base().to_string(),
            },
            None => DragSource::File {
                path: p.clone(),
                name: p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            },
        })
        .collect()
}

/// What is left of clipboard `clip` once `moved` went elsewhere: a cut
/// file that moved leaves it, and a cut with nothing left is gone. A
/// copy stays whole.
fn after_move(mut clip: FileClip, moved: &[PathBuf]) -> Option<FileClip> {
    if clip.cut {
        clip.paths.retain(|p| !moved.contains(p));
    }
    (!clip.paths.is_empty()).then_some(clip)
}

pub fn properties_row(commands: &[Option<DesktopCommand>]) -> Option<DesktopCommand> {
    use DesktopCommand::*;
    commands
        .iter()
        .flatten()
        .find(|c| matches!(c, IconProperties | BinProperties))
        .cloned()
}

impl State {
    pub fn is_renaming(&self) -> bool {
        self.desktop.as_ref().is_some_and(|d| d.rename.is_some())
    }

    pub(crate) fn select_all_icons(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.selection.select_all(view.entries.len());
        }
        self.relayout_desktop();
    }

    /// Move the icon selection with an arrow (or Home/End) by the icons'
    /// on-screen positions; the focus ring shows from now on.
    pub(crate) fn navigate(&mut self, dir: IconMove) {
        let next = match self.desktop.as_mut() {
            Some(v) => {
                v.focus_ring = true;
                desktop::move_selection(v.selection.focus, &v.cells, dir)
            }
            None => return,
        };
        self.select_icon(next);
        self.relayout_desktop();
    }

    pub(crate) fn open_key_menu(&mut self, focus: Option<usize>) {
        let cell = focus.and_then(|i| Some((i, *self.desktop.as_ref()?.cells.get(i)?)));
        match cell {
            Some((i, c)) => self.open_icon_menu(i, (c.x + c.width / 2, c.y + c.height / 2)),
            None => {
                let area = self.desktop_area();
                self.open_desktop_menu((area.x, area.y));
            }
        }
        if !self.menus.is_empty() {
            self.enter_menu_keys();
        }
    }

    /// Alt+Enter on icon `index`: what its menu's Properties row does. An
    /// icon whose menu has no Properties row does nothing.
    pub(crate) fn properties_key(&mut self, index: usize) {
        let Some(id) = self
            .desktop
            .as_ref()
            .and_then(|d| d.entries.get(index))
            .map(|e| e.id())
        else {
            return;
        };
        let Some((_, commands)) = self.icon_menu_rows(index) else {
            return;
        };
        if let Some(cmd) = properties_row(&commands) {
            self.desktop_command(cmd, Some(id));
        }
    }

    /// Put a launcher for each of `sources` on the desktop, as a right-drag
    /// "Create Shortcuts Here" onto icon `index`'s own cell would: each takes
    /// the free cell nearest its icon.
    pub fn shortcuts_beside(&mut self, index: usize, sources: &[DragSource]) {
        let Some(cell) = self
            .desktop
            .as_ref()
            .and_then(|v| v.cells.get(index))
            .copied()
        else {
            return;
        };
        let area = self.desktop_area();
        let cell = desktop::cell_at(self.desktop_grid(), area, cell.x, cell.y);
        self.perform_all(sources, &DropTarget::Desktop(cell), DropOp::Shortcut);
    }

    /// Cut or Copy aimed at icon `index`: its file and the other files of
    /// its selection go on the clipboard, `index` first.
    pub fn copy_icons(&mut self, index: usize, cut: bool) {
        if self.is_locked() {
            return;
        }
        let set = self.selection_around(index);
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let paths = aimed_in(&view.entries, &set, index, true)
            .into_iter()
            .map(|i| view.entries[i].path.clone())
            .collect();
        if let Some(offers) = file_offers(&FileClip { paths, cut }) {
            self.clip_offer_each(offers);
        }
    }

    /// Paste (or Paste Shortcut) on the desktop: the cell under `at` (the
    /// right click that opened the menu), else the first free one from the
    /// top left.
    pub fn paste_on_desktop(&mut self, at: Option<(i32, i32)>, shortcut: bool) {
        let area = self.desktop_area();
        let at = at.unwrap_or((area.x, area.y));
        let cell = desktop::cell_at(self.desktop_grid(), area, at.0, at.1);
        self.paste_files(DropTarget::Desktop(cell), shortcut);
    }

    /// Paste the clipboard's files onto `target` as a drop there would:
    /// cut files move, copied ones are copied, and with `shortcut` each
    /// gets a launcher instead. The files may come from a client, so they
    /// land once its pipe has closed.
    pub fn paste_files(&mut self, target: DropTarget, shortcut: bool) {
        if self.is_locked() {
            return;
        }
        self.clip_files(move |state, clip| state.paste_clip(clip, &target, shortcut));
    }

    /// Paste on a launcher: the clipboard's files open in `app`, as Open With would open them.
    pub fn paste_open(&mut self, app: reprisefs::apps::AppEntry) {
        if self.is_locked() {
            return;
        }
        self.clip_files(move |state, clip| {
            for path in &clip.paths {
                let argv = reprisefs::apps::open_argv(&app, &state.config.terminal, path);
                state.spawn_app_activated(&argv, &app.id);
            }
        });
    }

    /// [`Self::paste_files`] once the files are known. A cut file leaves
    /// Reprise's clipboard once it has moved ([`Self::cut_moved`]), so a
    /// move that fails keeps it. A file gone since it was cut or copied is
    /// skipped.
    fn paste_clip(&mut self, clip: FileClip, target: &DropTarget, shortcut: bool) {
        let dir = self.desktop.as_ref().map(|v| v.dir.clone());
        let target = target.clone();
        // A clipboard file may sit on a mount that never answers.
        self.spawn_fs_job(move || {
            let paths: Vec<PathBuf> = clip.paths.iter().filter(|p| p.exists()).cloned().collect();
            let here = match &dir {
                Some(dir) => crate::dragview::in_folder(paths.clone(), dir),
                None => Default::default(),
            };
            Box::new(move |state: &mut State| {
                state.paste_found(paths, clip.cut, &here, &target, shortcut)
            })
        });
    }

    /// [`Self::paste_clip`] once `paths` holds the clipboard's files that still exist and `here`
    /// those of them in the desktop folder.
    fn paste_found(
        &mut self,
        paths: Vec<PathBuf>,
        cut: bool,
        here: &std::collections::HashSet<PathBuf>,
        target: &DropTarget,
        shortcut: bool,
    ) {
        if paths.is_empty() {
            return;
        }
        let sources = match self.desktop.as_ref() {
            Some(view) => clip_sources(&view.entries, &paths),
            None => clip_sources(&[], &paths),
        };
        let op = match (shortcut, cut) {
            (true, _) => DropOp::Shortcut,
            (_, true) => DropOp::Move,
            (_, false) => DropOp::Copy,
        };
        self.put_all(&sources, target, op, here);
        if op == DropOp::Move && matches!(target, DropTarget::Desktop(_)) {
            let in_place: Vec<PathBuf> = paths.into_iter().filter(|p| here.contains(p)).collect();
            self.cut_moved(&in_place);
        }
    }

    /// Cut files a move just took elsewhere (`moved`) leave Reprise's
    /// clipboard; it empties once none is left.
    pub fn cut_moved(&mut self, moved: &[PathBuf]) {
        let Some(clip) = self.own_file_clip() else {
            return;
        };
        let left = after_move(clip.clone(), moved);
        if left.as_ref() == Some(&clip) {
            return;
        }
        match left.as_ref().and_then(file_offers) {
            Some(offers) => self.clip_offer_each(offers),
            None => self.clip_clear(),
        }
    }

    /// Begin renaming icon `index`: its label becomes an editable box
    /// holding the current name (`desktop::rename_edit`). The Recycle Bin keeps its name.
    pub fn begin_rename(&mut self, index: usize) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let Some(entry) = view.entries.get(index) else {
            return;
        };
        if entry.kind == desktop::Kind::RecycleBin {
            return;
        }
        view.rename = Some((
            index,
            desktop::rename_edit(entry, self.config.desktop_extensions),
        ));
        view.rename_clicks.reset();
        view.rename_drop = None;
        view.selection.set_only(Some(index));
        self.restart_caret();
        self.relayout_desktop();
    }

    /// A key during an inline rename, whose keysyms are `syms`: Enter
    /// renames the file, Escape abandons the edit, anything else edits the
    /// name (`textedit`).
    pub(crate) fn rename_key(&mut self, code: Keycode, syms: &[u32]) {
        for &sym in syms {
            match sym {
                keys::KEY_Return | keys::KEY_KP_Enter => return self.commit_rename(),
                keys::KEY_Escape => return self.cancel_rename(),
                _ => {}
            }
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        let Some(key) = crate::editkeys::from_keyboard(&keyboard, self, code) else {
            return;
        };
        self.rename_edit(key);
    }

    /// An editing key for the rename in progress: typed, or the input
    /// method's delete or commit.
    pub(crate) fn rename_edit(&mut self, key: EditKey) {
        if matches!(&key, EditKey::Insert(s) if s.contains('/')) {
            return;
        }
        let Some((index, edit)) = self.desktop.as_ref().and_then(|d| d.rename.as_ref()) else {
            return;
        };
        match key {
            EditKey::Paste => self.clip_paste(PasteTarget::Rename(*index)),
            EditKey::Copy | EditKey::Cut => {
                let text = edit.selected_text().to_string();
                self.clip_copy(&text);
            }
            _ => {}
        }
        if let Some((_, edit)) = self.desktop.as_mut().and_then(|d| d.rename.as_mut()) {
            edit.key(key);
        }
        self.restart_caret();
        self.relayout_desktop();
    }

    /// A left press at layout (`lx`, `ly`) while a rename is in progress:
    /// inside the edit box it puts the caret under the pointer (Shift
    /// extends) or selects the word on a double click, and returns true so
    /// the caller can drag-select; elsewhere it commits the rename and
    /// returns false. False when nothing is being renamed.
    pub fn rename_press(&mut self, lx: f64, ly: f64, time_msec: u32) -> bool {
        let Some(view) = self.desktop.as_ref() else {
            return false;
        };
        let Some((index, edit)) = view.rename.as_ref() else {
            return false;
        };
        let Some(cell) = view.cells.get(*index) else {
            return false;
        };
        let theme = &self.config.theme;
        let lay = desktop::edit_layout(theme, &self.text, edit, self.desktop_grid().w);
        let (x, y) = (lx as i32 - cell.x, ly as i32 - cell.y);
        if !lay.rect.contains(x, y) {
            self.commit_rename();
            return false;
        }
        let i = lay.index_at(&self.text, theme.fonts.size_px, x, y);
        let shift = self
            .seat
            .get_keyboard()
            .is_some_and(|k| k.modifier_state().shift);
        let click = theme.metrics.click(lx as i32, ly as i32, time_msec);
        let mut double = false;
        if let Some(d) = self.desktop.as_mut() {
            double = d.rename_clicks.press((), click);
            if let Some((_, edit)) = d.rename.as_mut() {
                if double {
                    edit.select_word_at(i);
                } else {
                    edit.place(i, shift);
                }
            }
        }
        self.restart_caret();
        self.relayout_desktop();
        !double
    }

    /// The pointer, held since a press in the edit box, is at layout
    /// (`lx`, `ly`): extend the selection to the char under it.
    pub fn drag_rename(&mut self, lx: f64, ly: f64) {
        let Some(view) = self.desktop.as_ref() else {
            return;
        };
        let Some((index, edit)) = view.rename.as_ref() else {
            return;
        };
        let Some(cell) = view.cells.get(*index) else {
            return;
        };
        let theme = &self.config.theme;
        let lay = desktop::edit_layout(theme, &self.text, edit, self.desktop_grid().w);
        let i = lay.index_at(
            &self.text,
            theme.fonts.size_px,
            lx as i32 - cell.x,
            ly as i32 - cell.y,
        );
        if let Some((_, edit)) = self.desktop.as_mut().and_then(|d| d.rename.as_mut())
            && edit.place(i, true)
        {
            self.relayout_desktop();
        }
    }

    /// Clipboard text for the rename box (Ctrl+V's answer): its first line,
    /// without any `/`.
    pub fn rename_paste(&mut self, text: &str) {
        let changed = self
            .desktop
            .as_mut()
            .and_then(|d| d.rename.as_mut())
            .is_some_and(|(_, edit)| edit.paste(text, &['/']));
        if changed {
            self.relayout_desktop();
        }
    }

    /// Finish a rename: rename the file to the edited name
    /// (`desktop::rename_file`), rewrite a launcher's `Name=`, or relabel a
    /// place, unless the name is empty or unchanged. The file work runs in a file job; a name the
    /// file system refuses leaves the file as it was and an error box says why.
    pub fn commit_rename(&mut self) {
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let Some((index, edit)) = view.rename.take() else {
            return;
        };
        self.stop_caret();
        let Some(view) = self.desktop.as_mut() else {
            return;
        };
        let typed = edit.text.trim().to_string();
        let Some(entry) = view.entries.get(index).cloned() else {
            self.relayout_desktop();
            return;
        };
        if let desktop::Kind::Place(place) = entry.kind {
            if desktop::rename_place(&mut view.system_names, place, &typed) {
                crate::state::save_system_names(&view.system_names);
                self.rescan_desktop();
            } else {
                self.relayout_desktop();
            }
            return;
        }
        if entry.label_is_name_key() && (typed.is_empty() || typed == entry.label) {
            self.relayout_desktop();
            return;
        }
        // The file may sit on a mount that never answers: the label shows the old name until the
        // rename lands.
        self.relayout_desktop();
        self.spawn_fs_job(move || {
            let done = if entry.label_is_name_key() {
                // A launcher's or a link's label is its `Name=`; the file keeps its name.
                if let Err(e) = reprisefs::apps::set_launcher_name(&entry.path, &typed) {
                    log::warn!("desktop: rename {} failed: {e}", entry.path.display());
                }
                Ok(None)
            } else {
                desktop::rename_file(&entry, &typed)
            };
            Box::new(move |state: &mut State| match done {
                Ok(Some(to)) => state.renamed(&entry.path, &to),
                Ok(None) => state.rescan_desktop(),
                Err(text) => {
                    log::warn!("desktop: rename {}: {text}", entry.path.display());
                    state.open_dialog(crate::dialog::Dialog::error("Rename", text));
                }
            })
        });
    }

    pub fn cancel_rename(&mut self) {
        if let Some(view) = self.desktop.as_mut() {
            view.rename = None;
        }
        self.stop_caret();
        self.relayout_desktop();
    }

    /// A desktop file was renamed on disk (an inline rename, later a
    /// Properties sheet's OK): its dragged cell follows the new name, then
    /// the desktop rescans.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        if let Some(view) = self.desktop.as_mut() {
            let old = view
                .entries
                .iter()
                .find(|e| e.path == from)
                .map(|e| e.name.clone());
            let new = to
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if let Some(cell) = old.and_then(|o| view.positions.remove(&o)) {
                view.positions.insert(new, cell);
                crate::state::save_desktop(view.auto_arrange, &view.positions);
            }
        }
        self.rescan_desktop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktopview::{IconMenu, bin_menu, icon_menu, place_menu};

    fn file(name: &str) -> DesktopEntry {
        let path = PathBuf::from("/d").join(name);
        DesktopEntry {
            name: name.into(),
            label: name.into(),
            icon: "text-x-generic".into(),
            target: desktop::Target::Open(path.clone()),
            path,
            shortcut: false,
            kind: desktop::Kind::File,
            is_dir: false,
            allow_launching: false,
        }
    }

    #[test]
    fn a_cut_leaves_the_clipboard_only_once_it_moved() {
        let (a, b) = (PathBuf::from("/d/a"), PathBuf::from("/d/b"));
        let cut = FileClip {
            paths: vec![a.clone(), b.clone()],
            cut: true,
        };
        assert_eq!(after_move(cut.clone(), &[]), Some(cut.clone()));
        assert_eq!(
            after_move(cut.clone(), std::slice::from_ref(&a)),
            Some(FileClip {
                paths: vec![b.clone()],
                cut: true
            })
        );
        assert_eq!(after_move(cut, &[a.clone(), b.clone()]), None);
        let copy = FileClip {
            paths: vec![a.clone()],
            cut: false,
        };
        assert_eq!(after_move(copy.clone(), &[a]), Some(copy));
    }

    #[test]
    fn a_paste_drops_a_desktop_path_as_its_icon_and_any_other_as_a_file() {
        let docs = desktop::place("documents").unwrap();
        let docs_path = docs.path.clone();
        let entries = vec![docs, file("a.txt")];
        let paths = [
            PathBuf::from("/d/a.txt"),
            PathBuf::from("/elsewhere/b.txt"),
            docs_path.clone(),
        ];
        assert_eq!(
            clip_sources(&entries, &paths),
            vec![
                DragSource::Icon {
                    id: desktop::IconId::File("/d/a.txt".into()),
                    path: "/d/a.txt".into(),
                    name: "a.txt".into(),
                },
                DragSource::File {
                    path: "/elsewhere/b.txt".into(),
                    name: "b.txt".into(),
                },
                DragSource::Icon {
                    id: desktop::IconId::System(desktop::Kind::Place(desktop::Place::Documents)),
                    path: docs_path,
                    name: "Documents".into(),
                },
            ]
        );
    }

    #[test]
    fn alt_enter_takes_the_properties_row_of_each_icon_menu() {
        use DesktopCommand::*;
        let row = |(_, c): (Vec<crate::startmenu::MenuItem>, Vec<Option<DesktopCommand>>)| {
            properties_row(&c)
        };
        assert_eq!(row(icon_menu(IconMenu::default())), Some(IconProperties));
        assert_eq!(row(bin_menu(false)), Some(BinProperties));
        assert_eq!(row(place_menu(desktop::Place::Computer, false)), None);
        assert_eq!(
            row(place_menu(desktop::Place::Documents, true)),
            Some(IconProperties)
        );
        assert_eq!(
            row(place_menu(desktop::Place::Home, true)),
            Some(IconProperties)
        );
    }
}
