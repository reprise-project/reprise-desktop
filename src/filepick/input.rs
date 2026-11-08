use super::*;
use crate::dialog::Outcome;
use crate::filedlggeom::OWN_APP;
use crate::raster::Rect;
use reprisekit::controls::form::Key;
use reprisekit::frame::Edges;
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::textedit::EditKey;

impl FilePicker {
    /// A key press read as a file-dialog key before [`FilePicker::key`]
    /// ([`FileDlg::shortcut`]); `None` leaves it to `key`.
    pub fn shortcut(
        &mut self,
        client: Rect,
        sym: u32,
        ctrl: bool,
        shift: bool,
        alt: bool,
    ) -> Option<Outcome> {
        let ev = self.dlg.shortcut(client, sym, ctrl, shift, alt)?;
        Some(self.event(ev))
    }

    /// A right press at `at` on `hit`: the list's context menu, or File name's focus.
    pub fn right_click(
        &mut self,
        client: Rect,
        hit: Option<filedlg::Hit>,
        at: (i32, i32),
    ) -> Outcome {
        let ev = self.dlg.right_click(client, hit, at);
        self.event(ev)
    }

    /// A key from the dialog view. Typed text goes in as an edit so the
    /// File name field takes it, and its first character doubles as the
    /// shell view's type-ahead. `client` is the dialog's page rect, where
    /// `draw` laid it out.
    pub fn key(&mut self, client: Rect, key: Key) -> Outcome {
        let k = match key {
            Key::Backspace => Key::Edit(EditKey::Backspace { word: false }),
            k => k,
        };
        let ch = match &k {
            Key::Edit(EditKey::Insert(t)) => t.chars().next(),
            Key::Char(c) => Some(*c),
            _ => None,
        };
        let ev = self.dlg.key(client, k, ch, Mods::default());
        self.event(ev)
    }

    /// A mouse's thumb button: Back or Forward through the folders shown.
    pub fn nav(&mut self, nav: reprisekit::pointer::Nav) -> Outcome {
        let ev = self.dlg.nav(nav);
        self.event(ev)
    }

    pub fn click(&mut self, client: Rect, hit: Option<filedlg::Hit>) -> Outcome {
        let ev = self.dlg.click(client, hit, Mods::default());
        self.event(ev)
    }

    /// A reader asked to see entry `row` of the list at client point `at`; nothing is picked.
    pub fn reveal(
        &mut self,
        client: Rect,
        at: (i32, i32),
        row: usize,
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        let ev = self.dlg.reveal(text, px, client, at, row);
        self.event(ev)
    }

    /// A reader's Focus on the control at `at`: it takes the keyboard, nothing is pressed.
    pub fn focus_at(
        &mut self,
        client: Rect,
        at: (i32, i32),
        text: &TextRenderer,
        px: u32,
    ) -> Outcome {
        let ev = self.dlg.focus_at(text, px, client, at);
        self.event(ev)
    }

    /// The wheel turned `rows` rows (down is positive) over `hit`.
    pub fn wheel(&mut self, client: Rect, hit: Option<filedlg::Hit>, rows: i32) -> Outcome {
        let ev = self.dlg.wheel(client, hit, rows);
        self.event(ev)
    }

    /// The second press of a double click: a folder is entered, a file
    /// picked; anywhere but an entry it is one more click.
    pub fn double_click(&mut self, client: Rect, hit: Option<filedlg::Hit>) -> Outcome {
        let ev = self.dlg.double_click(client, hit);
        self.event(ev)
    }

    /// A changed Files of type filter relists the folder for it.
    pub(super) fn sync_filter(&mut self) {
        let filter = self.dlg.filter();
        if filter != self.filter {
            self.filter = filter;
            if let Some(id) = self.dlg.folder() {
                self.list(id);
            }
        }
    }

    pub fn release(&mut self) -> Outcome {
        let ev = self.dlg.release();
        self.event(ev)
    }

    /// The pointer rests on `hit`, or drags a held column title there: a
    /// Details column title lights or follows. True when that redraws.
    pub fn hover(&mut self, hit: Option<filedlg::Hit>) -> bool {
        !matches!(self.dlg.hover(hit), Event::None)
    }

    /// The pointer left the dialog: no column title stays lit. True when
    /// that redraws.
    pub fn leave(&mut self) -> bool {
        !matches!(self.dlg.leave(), Event::None)
    }

    /// Takes the view and sort `file` holds (`repriseconf::filedialog`), the one every Open and
    /// Save As dialog shares, and stores the dialog's own there as they change. The file is read
    /// with the first listing, so call this before the host takes the first requests.
    pub fn remember_in(&mut self, file: PathBuf) {
        let first = self
            .requests
            .iter_mut()
            .find(|r| Some(r.seq) == self.listing);
        if let Some(r) = first {
            r.view_file = Some(file.clone());
            self.view_awaited = true;
        }
        self.view_file = Some(file);
    }

    fn remember(&mut self) {
        let now = self.dlg.remembered();
        let Some(file) = self.view_file.clone().filter(|_| now != self.remembered) else {
            return;
        };
        self.remembered = now;
        // A view picked before the stored one arrives wins over it.
        self.view_awaited = false;
        crate::state::run_in_order(move || {
            if let Err(e) = repriseconf::filedialog::save_to(&file, OWN_APP, &now.text()) {
                log::warn!(
                    "browse: could not store the view in {}: {e}",
                    file.display()
                );
            }
        });
    }

    /// What the dialog asked for. Open on a name looks it up off the loop:
    /// a folder is entered, a file is the pick, anything else ignored.
    pub(super) fn event(&mut self, ev: Event) -> Outcome {
        self.sync_filter();
        self.remember();
        let ev = match ev {
            Event::NeedPlace(i) => match self.place_dirs.get(i).cloned().flatten() {
                Some(dir) => Event::NeedListing(self.folders.id(&dir)),
                None => Event::None,
            },
            ev => ev,
        };
        match ev {
            Event::None => Outcome::Nothing,
            Event::Redraw => Outcome::Redraw,
            Event::NeedPlace(_) => Outcome::Nothing,
            Event::PinEntry { .. } | Event::UnpinPlace(_) | Event::ShiftPlace { .. } => {
                self.pin(ev);
                Outcome::Nothing
            }
            Event::NeedListing(id) => {
                // Entering a folder by its typed name empties the field;
                // a name typed for something else stays.
                let typed = self.dlg.file_name().to_string();
                let names_dir = self
                    .dlg
                    .entries()
                    .iter()
                    .any(|e| e.is_dir && e.name == typed);
                self.list(id);
                if names_dir {
                    self.dlg.set_file_name("");
                }
                Outcome::Redraw
            }
            Event::Refresh => {
                self.refresh();
                Outcome::Redraw
            }
            Event::CreateFolder(base) => {
                self.queue(Work::NewFolder(base));
                Outcome::Redraw
            }
            Event::Rename { from, to } => {
                let path = self.typed_path(&from);
                if let Some(path) = path {
                    self.queue(Work::Rename {
                        from: path,
                        old: from,
                        typed: to,
                    });
                }
                Outcome::Redraw
            }
            Event::Delete { names, erase } => {
                let paths: Vec<PathBuf> = names.iter().filter_map(|n| self.typed_path(n)).collect();
                match self
                    .current()
                    .is_some_and(|d| !reprisefs::places::is_recent(d))
                {
                    true => Outcome::AskDelete {
                        paths,
                        shift: erase,
                    },
                    false => Outcome::Nothing,
                }
            }
            Event::Resize => Outcome::Size(Edges::BOTTOM | Edges::RIGHT),
            Event::FilterChanged(_) => Outcome::Redraw,
            // A typed pattern lists the open folder, or looks its folder
            // part up and enters it, through the pattern. The dialog
            // takes the pattern only once there is a folder to list.
            Event::Wildcard { dir, name } => {
                if dir.is_empty() {
                    if let Some(id) = self.dlg.folder() {
                        self.dlg.apply_wildcard(&name);
                        self.list(id);
                    }
                    return Outcome::Redraw;
                }
                let Some(path) = self.typed_path(&dir) else {
                    return Outcome::Nothing;
                };
                let pattern = &name[dir.len()..];
                self.opening = Some(self.request(None, path, Some(pattern)));
                Outcome::Nothing
            }
            Event::Cancel => Outcome::Close,
            Event::Accept(names) => {
                let Some(name) = names.into_iter().next() else {
                    return Outcome::Nothing;
                };
                let Some(path) = self.typed_path(&name) else {
                    return Outcome::Nothing;
                };
                self.opening = Some(self.request(None, path, None));
                Outcome::Nothing
            }
        }
    }

    /// Where a typed `name` points from the open folder. Lexical, so a
    /// typed ".." gets the same folder handle as Up and its parent is the
    /// one above it. None for a relative name in My Recent Documents,
    /// which is no folder to look a new name up in.
    fn typed_path(&self, name: &str) -> Option<PathBuf> {
        Some(match self.current() {
            Some(dir) if reprisefs::places::is_recent(dir) => match self.recent.get(name) {
                Some(p) => p.clone(),
                None if name.starts_with('/') => reprisefs::ops::lexical(Path::new(name)),
                None => return None,
            },
            Some(dir) => reprisefs::ops::lexical(&dir.join(name)),
            None => reprisefs::ops::lexical(Path::new(name)),
        })
    }
}
