use super::{DlgMode, Event, FOCUS_ORDER, FileDlg, Focus, split_names};
use crate::controls::combo::Combo;
use crate::controls::form;
use crate::controls::iconview::IconMove;
use crate::controls::metrics::ControlMetrics;
use crate::controls::pathcomplete;
use crate::controls::shellview;
use crate::pointer::Nav;
use crate::raster::Rect;
use crate::selection::Mods;
use crate::text::access_key;
use crate::textedit::{EditKey, TextEdit, keysym};

fn combo_key(combo: &mut Combo, key: form::Key) -> Event {
    match key {
        form::Key::Drop => {
            combo.toggle();
            Event::Redraw
        }
        form::Key::Up => {
            combo.step(false);
            Event::Redraw
        }
        form::Key::Down => {
            combo.step(true);
            Event::Redraw
        }
        form::Key::Home => {
            combo.jump(false);
            Event::Redraw
        }
        form::Key::End => {
            combo.jump(true);
            Event::Redraw
        }
        form::Key::PageUp => {
            combo.scroll(-(combo.rows as i32));
            Event::Redraw
        }
        form::Key::PageDown => {
            combo.scroll(combo.rows as i32);
            Event::Redraw
        }
        form::Key::Enter if combo.open.is_some() => {
            combo.take();
            Event::Redraw
        }
        _ => Event::None,
    }
}

impl FileDlg {
    /// Alt+letter: a label's key focuses its field, the accept button's
    /// accepts, and any other goes to the extra rows.
    fn access_press(&mut self, ch: char) -> Event {
        let key = Some(ch.to_lowercase().next().unwrap_or(ch));
        let labels = self.labels();
        let focus = match () {
            _ if key == access_key(&labels.lookin) => Focus::LookIn,
            _ if key == access_key(&labels.filename) => Focus::FileName,
            _ if key == access_key(&labels.filetype) && self.has_types() => Focus::FileType,
            _ if key == access_key(&labels.accept) => Focus::Accept,
            _ => {
                let taken = self
                    .extra
                    .as_mut()
                    .is_some_and(|(f, _)| f.key(form::Key::Access(ch)) != form::Event::None);
                if !taken {
                    return Event::None;
                }
                Focus::Extra
            }
        };
        self.lookin.open = None;
        self.filetype.open = None;
        self.focus = focus;
        if focus == Focus::Accept {
            return self.activate(true);
        }
        Event::Redraw
    }

    fn advance_focus(&mut self, back: bool) {
        if self.focus == Focus::Extra
            && let Some((f, _)) = &mut self.extra
            && f.tab_no_wrap(back)
        {
            return;
        }
        self.step_focus(back);
        if self.focus == Focus::Extra {
            let entered = match &mut self.extra {
                Some((f, _)) => f.focus_edge(back),
                None => false,
            };
            if !entered {
                self.step_focus(back);
            }
        }
    }

    fn step_focus(&mut self, back: bool) {
        let i = FOCUS_ORDER
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        let n = FOCUS_ORDER.len() as i32;
        let step = if back { -1 } else { 1 };
        let next = (i as i32 + step).rem_euclid(n);
        self.focus = FOCUS_ORDER[next as usize];
        if self.focus == Focus::FileType && !self.has_types() {
            self.step_focus(back);
        }
    }

    /// Whether the dialog has a Files of type row: not in folder mode,
    /// which lists folders only.
    pub(super) fn has_types(&self) -> bool {
        self.fields.mode != DlgMode::Folder
    }

    /// The default action. A selected folder whose name is all the File
    /// name field holds opens, or, from the accept button in folder mode,
    /// is picked. Otherwise the field's names are accepted, else the
    /// focused file's, else (folder mode) the open folder as `.`.
    pub(super) fn activate(&mut self, from_button: bool) -> Event {
        if self.focus == Focus::Places {
            return match self.places.selected {
                Some(i) if self.places.places.get(i).is_some_and(|p| p.enabled) => {
                    Event::NeedPlace(i)
                }
                _ => Event::None,
            };
        }
        // A selected directory wins when the field holds only its name:
        // `shell_click` writes the selection into the field, so accepting
        // the text would "open" a folder as a file. A typed name still wins.
        let folder_pick = self.fields.mode == DlgMode::Folder && from_button;
        let focused = self.shell.selection.focus;
        if let Some(entry) = focused.and_then(|i| self.shell.entries.get(i))
            && entry.is_dir
            && (self.filename.text.is_empty() || self.filename.text == entry.name)
        {
            if folder_pick {
                return Event::Accept(vec![entry.name.clone()]);
            }
            let (id, name) = (entry.id, entry.name.clone());
            return self.enter(id, &name);
        }
        let names = split_names(&self.filename.text);
        if let [name] = &names[..]
            && let Some(ev) = self.take_wildcard(name)
        {
            return ev;
        }
        if !names.is_empty() {
            return Event::Accept(names);
        }
        if let Some(entry) = focused.and_then(|i| self.shell.entries.get(i)) {
            if !entry.is_dir {
                return Event::Accept(vec![entry.name.clone()]);
            }
            let (id, name) = (entry.id, entry.name.clone());
            return self.enter(id, &name);
        }
        if self.fields.mode == DlgMode::Folder {
            return Event::Accept(vec![".".into()]);
        }
        Event::None
    }

    /// A File name whose last part holds `*` or `?` is a pattern, not a name
    /// to accept, unless a listed entry has that name. Folder mode takes no
    /// pattern.
    fn take_wildcard(&self, name: &str) -> Option<Event> {
        let (dir, pattern) = match name.rfind('/') {
            Some(i) => name.split_at(i + 1),
            None => ("", name),
        };
        if self.fields.mode == DlgMode::Folder
            || !pattern.contains(['*', '?'])
            || self.shell.entries.iter().any(|e| e.name == name)
        {
            return None;
        }
        Some(Event::Wildcard {
            dir: dir.to_string(),
            name: name.to_string(),
        })
    }

    /// A key while a sub-control has the focus. `ch` is the keystroke's
    /// literal character for the shell view's type-ahead; `r` is the rect
    /// `draw` was given.
    pub fn key(&mut self, r: Rect, key: form::Key, ch: Option<char>, mods: Mods) -> Event {
        let before = self.filetype.selected;
        let cm = self.drawn.0.get();
        let ev = self.key_inner(r, key, ch, mods, &cm);
        self.filter_event(before, ev)
    }

    /// `ev`, or `FilterChanged` when the Files of type pick moved, which
    /// also drops a typed pattern's filter.
    pub(super) fn filter_event(&mut self, before: usize, ev: Event) -> Event {
        if self.filetype.selected != before && matches!(ev, Event::Redraw | Event::None) {
            self.wildcard = None;
            Event::FilterChanged(self.filetype.selected)
        } else {
            ev
        }
    }

    fn key_inner(
        &mut self,
        r: Rect,
        key: form::Key,
        ch: Option<char>,
        mods: Mods,
        cm: &ControlMetrics,
    ) -> Event {
        if matches!(key, form::Key::Escape) && self.cancel_marquee() {
            return Event::Redraw;
        }
        if self.context.is_some() {
            return self.context_key(key);
        }
        if self.shell.edit.is_some() {
            return self.rename_key(key);
        }
        if self.views_open {
            return match key {
                form::Key::Escape | form::Key::Drop => {
                    self.views_open = false;
                    Event::Redraw
                }
                _ => Event::None,
            };
        }
        if let form::Key::Access(ch) = key {
            return self.access_press(ch);
        }
        if self.focus == Focus::FileName
            && let Some(k) = pathcomplete::nav_key(&key)
            && self.complete.nav(&mut self.filename, k)
        {
            return Event::Redraw;
        }
        // A dropped list owns Enter and Escape: Enter commits the row (Look in
        // navigates to the crumb), Escape closes the list. Neither may reach
        // `activate` or `combo_key`.
        let combo_open = self.lookin.open.is_some() || self.filetype.open.is_some();
        if combo_open {
            if matches!(key, form::Key::Escape) {
                self.lookin.open = None;
                self.filetype.open = None;
                return Event::Redraw;
            }
            if matches!(key, form::Key::Enter) {
                if let Some(i) = self.lookin.open {
                    self.lookin.take();
                    if let Some(c) = self.crumbs.get(i) {
                        return Event::NeedListing(c.id);
                    }
                    return Event::Redraw;
                }
                self.filetype.take();
                return Event::Redraw;
            }
        }
        if self.focus == Focus::Extra && !matches!(key, form::Key::Tab { .. }) {
            let open = self.extra_list_open();
            match key {
                form::Key::Escape | form::Key::Enter if !open => {}
                _ => {
                    return match &mut self.extra {
                        Some((f, _)) => match f.key(key) {
                            form::Event::None => Event::None,
                            _ => Event::Redraw,
                        },
                        None => Event::None,
                    };
                }
            }
        }
        if matches!(key, form::Key::Escape) {
            return Event::Cancel;
        }
        if let form::Key::Tab { back } = key {
            self.advance_focus(back);
            return Event::Redraw;
        }
        // Space presses a focused push button as Enter does; hosts send
        // it as typed text, which the File name field keeps.
        let space = match &key {
            form::Key::Space => true,
            form::Key::Edit(EditKey::Insert(s)) => s == " ",
            _ => false,
        };
        let pressed = matches!(key, form::Key::Enter)
            || (space && matches!(self.focus, Focus::Accept | Focus::Cancel));
        if pressed && self.focus != Focus::Cancel {
            return self.activate(false);
        }
        if pressed && self.focus == Focus::Cancel {
            return Event::Cancel;
        }
        // Backspace edits the File name field; anywhere else it goes up
        // a folder.
        if self.focus != Focus::FileName
            && matches!(
                key,
                form::Key::Backspace | form::Key::Edit(EditKey::Backspace { .. })
            )
        {
            return self.up();
        }
        match self.focus {
            Focus::FileName => match key {
                form::Key::Edit(ek) => {
                    if self.complete.edit(&mut self.filename, ek) {
                        self.typed = self.filename.text.clone();
                        Event::Redraw
                    } else {
                        Event::None
                    }
                }
                _ => Event::None,
            },
            Focus::FileType => combo_key(&mut self.filetype, key),
            Focus::LookIn => combo_key(&mut self.lookin, key),
            Focus::Accept | Focus::Cancel | Focus::Extra => Event::None,
            Focus::Places => {
                // The nearest live tile past the selection: a greyed one
                // refuses the keyboard as it refuses a click.
                let live = |i: &usize| self.places.places[*i].enabled;
                let next = match (key, self.places.selected) {
                    (form::Key::Up, Some(i)) => (0..i).rev().find(live),
                    (form::Key::Down, sel) => {
                        (sel.map_or(0, |i| i + 1)..self.places.places.len()).find(live)
                    }
                    _ => None,
                };
                match next {
                    Some(i) => {
                        self.places.selected = Some(i);
                        let bar = self.places_rect(r, cm);
                        self.places.reveal(i, bar, cm);
                        Event::Redraw
                    }
                    None => Event::None,
                }
            }
            Focus::Shell => {
                let lay = self.shell_layout(r, cm);
                self.shell_key(key, ch, &lay, mods)
            }
        }
    }

    /// The file-dialog key `sym` with its modifiers, read before [`FileDlg::key`]: F5 and Ctrl+R
    /// refresh, Alt+Left and Alt+Right go back and forward, Alt+Up goes up and Ctrl+Shift+N
    /// makes a folder wherever the focus is. In the list, F2 renames the selected entry, Delete
    /// deletes the selection (Shift+Delete for good), Shift+F10 and the Menu key open the context menu, and Ctrl+A
    /// selects every entry of a dialog that picks several. `None` leaves the key to `key`, so
    /// File name keeps its editing keys (Ctrl+A selecting its text), and so do the rename box,
    /// the menu and an open list.
    pub fn shortcut(
        &mut self,
        r: Rect,
        sym: u32,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<Event> {
        let busy = self.context.is_some()
            || self.shell.edit.is_some()
            || self.views_open
            || self.lookin.open.is_some()
            || self.filetype.open.is_some()
            || self.extra_list_open();
        if busy {
            return None;
        }
        let letter = |c: char| char::from_u32(sym | 0x20) == Some(c);
        let plain = !ctrl && !shift && !alt;
        match sym {
            keysym::F5 if !ctrl && !alt => return Some(Event::Refresh),
            _ if ctrl && !shift && !alt && letter('r') => return Some(Event::Refresh),
            keysym::LEFT | keysym::KP_LEFT if alt && !ctrl => return Some(self.nav(Nav::Back)),
            keysym::RIGHT | keysym::KP_RIGHT if alt && !ctrl => {
                return Some(self.nav(Nav::Forward));
            }
            keysym::UP | keysym::KP_UP if alt && !ctrl => return Some(self.up()),
            _ if ctrl && shift && !alt && letter('n') => {
                return Some(Event::CreateFolder(tr!("filedlg-new-folder")));
            }
            _ => {}
        }
        if self.focus != Focus::Shell {
            return None;
        }
        match sym {
            keysym::F2 if plain => Some(self.begin_rename()),
            keysym::DELETE | keysym::KP_DELETE if !ctrl && !alt => {
                Some(self.delete_selected(shift))
            }
            keysym::MENU => Some(self.context_from_keys(r)),
            keysym::F10 if shift && !ctrl && !alt => Some(self.context_from_keys(r)),
            _ if ctrl && !shift && !alt && letter('a') => {
                if !self.multi() {
                    return Some(Event::None);
                }
                self.shell.selection.select_all(self.shell.entries.len());
                self.sync_name_field();
                Some(Event::Redraw)
            }
            _ => None,
        }
    }

    /// Opens the rename box on the one selected entry, its name but the extension selected.
    pub(super) fn begin_rename(&mut self) -> Event {
        let Some(i) = self.shell.selection.single() else {
            return Event::None;
        };
        let Some(e) = self.shell.entries.get(i) else {
            return Event::None;
        };
        self.focus = Focus::Shell;
        self.shell.edit = Some((i, TextEdit::rename(&e.name, e.is_dir)));
        Event::Redraw
    }

    /// Closes the rename box: a new name goes to the host, the same name or none changes nothing.
    pub(super) fn commit_rename(&mut self) -> Event {
        let Some((i, ed)) = self.shell.edit.take() else {
            return Event::None;
        };
        let to = ed.text.trim();
        match self.shell.entries.get(i) {
            Some(e) if !to.is_empty() && to != e.name => Event::Rename {
                from: e.name.clone(),
                to: to.to_string(),
            },
            _ => Event::Redraw,
        }
    }

    /// The rename box takes every key: Enter and Tab close it on the name typed, Escape on the
    /// old one, and the rest edit it.
    fn rename_key(&mut self, key: form::Key) -> Event {
        let ek = match key {
            form::Key::Enter | form::Key::NewLine | form::Key::Tab { .. } => {
                return self.commit_rename();
            }
            form::Key::Escape => {
                self.shell.edit = None;
                return Event::Redraw;
            }
            form::Key::Edit(ek) => ek,
            form::Key::Backspace => EditKey::Backspace { word: false },
            form::Key::Space => EditKey::Insert(" ".into()),
            form::Key::Char(c) => EditKey::Insert(c.to_string()),
            form::Key::Left => EditKey::Left {
                select: false,
                word: false,
            },
            form::Key::Right => EditKey::Right {
                select: false,
                word: false,
            },
            form::Key::Home => EditKey::Home { select: false },
            form::Key::End => EditKey::End { select: false },
            _ => return Event::None,
        };
        let changed = self.shell.edit.as_mut().is_some_and(|(_, ed)| ed.key(ek));
        if changed { Event::Redraw } else { Event::None }
    }

    /// Delete on the selection (for good with `erase`): its names to the host, nothing with none
    /// selected.
    pub(super) fn delete_selected(&self, erase: bool) -> Event {
        let names: Vec<String> = self.selected().iter().map(|e| e.name.clone()).collect();
        if names.is_empty() {
            Event::None
        } else {
            Event::Delete { names, erase }
        }
    }

    fn shell_key(
        &mut self,
        key: form::Key,
        ch: Option<char>,
        lay: &shellview::Layout,
        mods: Mods,
    ) -> Event {
        if matches!(key, form::Key::Space) && self.multi() && mods.ctrl {
            if let Some(i) = self.shell.selection.focus {
                self.shell.selection.click(
                    i,
                    Mods {
                        ctrl: true,
                        shift: false,
                    },
                );
                self.sync_name_field();
                return Event::Redraw;
            }
            return Event::None;
        }
        if let Some(c) = ch.filter(|c| *c != ' ')
            && shellview::type_ahead(&mut self.shell, c)
        {
            shellview::follow(&mut self.shell, lay);
            self.sync_name_field();
            return Event::Redraw;
        }
        // Hosts send Left, Right, Home and End as edit keys, which only the File name field edits with.
        let key = match key {
            form::Key::Edit(ek) => form::flat_key(&ek).unwrap_or(form::Key::Edit(ek)),
            k => k,
        };
        let dir = match key {
            form::Key::Up => IconMove::Up,
            form::Key::Down => IconMove::Down,
            form::Key::Left => IconMove::Left,
            form::Key::Right => IconMove::Right,
            form::Key::Home => IconMove::Home,
            form::Key::End => IconMove::End,
            _ => return Event::None,
        };
        let mods = self.sel_mods(mods);
        if shellview::step(&mut self.shell, dir, lay, mods) {
            self.sync_name_field();
            Event::Redraw
        } else {
            Event::None
        }
    }
}
