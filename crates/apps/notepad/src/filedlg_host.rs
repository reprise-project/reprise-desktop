//! The Open and Save As dialogs as this host runs them: the places bar and its folders, the listing,
//! and what Accept and Cancel do to the document. The dialog itself is `reprisekit::controls::filedlg`; nothing here draws.

use super::*;
use reprisekit::controls::combo::{self, Combo};
use reprisekit::controls::filedlg;

const ENCODING: &str = "encoding";
/// The Encoding row with 8 px under it, as above it.
const ENCODING_ROW_H: i32 = combo::COMBO_H + 8;
/// The Encoding label sits as far below its combo's top as the Files of type label does.
const ENCODING_LABEL_DY: i32 = 3;

/// Save As's Encoding row: its label under Files of type's, its combo under that field, on `format`'s row.
fn encoding_row(format: Format) -> Form {
    let names = ENCODINGS.map(|(name, _, _)| name);
    let field = Rect::new(
        filedlg::FIELD_X,
        filedlg::EXTRA_Y,
        filedlg::field_w(CLIENT.0),
        combo::COMBO_H,
    );
    let combo = Item::new(
        ENCODING,
        field,
        Control::Combo(Combo::new(&names, format.encoding_row(), names.len())),
    );
    Form::new(vec![
        Item::label_at(
            "encoding-label",
            filedlg::ROW_LABEL_X,
            filedlg::EXTRA_Y + ENCODING_LABEL_DY,
            filedlg::FIELD_X - filedlg::ROW_LABEL_X,
            &tr!("encoding-label"),
        ),
        combo,
    ])
}

/// Where a fresh dialog starts when there is no current path: the process's working directory, or `/` if that fails.
pub(crate) fn default_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

impl Notepad {
    /// Where a fresh dialog starts: the current document's folder, or `default_dir()` for an Untitled one.
    pub(crate) fn start_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(default_dir)
    }

    /// Declares the Open dialog open, on `start_dir()`. The listing waits for the first event, which has the icon cache.
    pub(crate) fn open_dialog(&mut self) {
        let dir = self.start_dir();
        let dlg = self.host.open(
            &dir,
            Fields {
                title: tr!("title-open"),
                accept_label: tr!("open-button"),
                mode: DlgMode::Open,
                multiple: false,
                filters: vec![tr!("filter-text"), tr!("filter-all")],
            },
            self.forms.text,
        );
        self.open = Some(dlg);
    }

    pub(crate) fn saveas_dialog(&mut self) {
        let dir = self.start_dir();
        let mut dlg = self.host.open(
            &dir,
            Fields {
                title: tr!("title-save-as"),
                accept_label: tr!("save-button"),
                mode: DlgMode::Save,
                multiple: false,
                filters: vec![tr!("filter-text"), tr!("filter-all")],
            },
            self.forms.text,
        );
        dlg.set_extra(
            encoding_row(self.format),
            ENCODING_ROW_H,
            &self.forms.look.controls(),
        );
        self.saveas = Some(dlg);
    }

    pub(crate) fn open_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        self.host
            .ensure_listed(dlg, &mut ctx.icons, &type_keep(dlg));
        match self.host.event(
            dlg,
            &ev,
            &self.dialog_text,
            &self.forms,
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed => self.open_outcome(FileEvent::Cancel, &mut ctx.icons),
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::None => Reply::none(),
            Step::Out(FileEvent::NeedListing(id)) => {
                self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                if let Some(id) = self.host.folder {
                    self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                }
                Reply::redraw()
            }
            Step::Out(out) => self.open_outcome(out, &mut ctx.icons),
        }
    }

    /// `Accept` reads the file and replaces the buffer. `Cancel` leaves `pending` alone: any action has run by the time Open is up.
    pub(crate) fn open_outcome(&mut self, out: FileEvent, icons: &mut IconCache) -> Reply {
        match out {
            FileEvent::None => Reply::none(),
            FileEvent::Redraw => Reply::redraw(),
            // New Folder isn't wired up: naming one needs a dialog that isn't built.
            FileEvent::CreateFolder(_) => Reply::none(),
            FileEvent::Accept(names) => match names.first() {
                Some(name) => self.open_accept(name, icons),
                None => Reply::none(),
            },
            FileEvent::Cancel => {
                self.open = None;
                Reply::redraw_all()
            }
            FileEvent::NeedListing(_)
            | FileEvent::FilterChanged(_)
            | FileEvent::Wildcard { .. }
            | FileEvent::NeedPlace(_)
            | FileEvent::Resize => Reply::none(),
        }
    }

    /// A typed folder opens in the dialog; a file is read into the buffer. A name whose folder does not
    /// exist says so in a box and leaves the dialog up.
    pub(crate) fn open_accept(&mut self, name: &str, icons: &mut IconCache) -> Reply {
        let path = match self.host.resolve(name) {
            Ok(Resolved::Folder(id, _)) => {
                if let Some(dlg) = self.open.as_mut() {
                    self.host.list(dlg, id, icons, &type_keep(dlg));
                    dlg.clear_file_name();
                }
                return Reply::redraw();
            }
            Ok(Resolved::File(path)) => path,
            Err(msg) => {
                log::warn!("open {name}: {msg}");
                self.error = Some(self.error_box(&msg));
                return Reply::redraw_all();
            }
        };
        match read_text(&path) {
            Ok((text, format)) => self.show_document(&text, format, Some(path)),
            Err(e) => self.open_failed(&path, &e),
        }
        Reply::redraw_all()
    }

    /// Replace the buffer with `text`, saved as `path` (`None` is Untitled), and close the Open dialog if it is up.
    pub(crate) fn show_document(&mut self, text: &str, format: Format, path: Option<PathBuf>) {
        self.edit = MultiEdit::new(&format.line_end.to_edit(text));
        self.format = format;
        self.view = View::default();
        self.stamp = path.as_deref().and_then(Stamp::of);
        self.path = path;
        self.dirty = false;
        self.open = None;
        self.sync_menus();
    }

    pub(crate) fn saveas_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.saveas.as_mut() else {
            return Reply::none();
        };
        self.host
            .ensure_listed(dlg, &mut ctx.icons, &type_keep(dlg));
        match self.host.event(
            dlg,
            &ev,
            &self.dialog_text,
            &self.forms,
            ctx.size,
            &ctx.theme.metrics,
        ) {
            Step::Closed => self.saveas_outcome(FileEvent::Cancel, &mut ctx.icons),
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::None => Reply::none(),
            Step::Out(FileEvent::NeedListing(id)) => {
                self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                if let Some(id) = self.host.folder {
                    self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                }
                Reply::redraw()
            }
            Step::Out(out) => self.saveas_outcome(out, &mut ctx.icons),
        }
    }

    /// `Accept` writes the file. `Cancel` drops any pending action: Save As is where an Untitled document's Yes on the save prompt waits.
    pub(crate) fn saveas_outcome(&mut self, out: FileEvent, icons: &mut IconCache) -> Reply {
        match out {
            FileEvent::None => Reply::none(),
            FileEvent::Redraw => Reply::redraw(),
            FileEvent::CreateFolder(_) => Reply::none(),
            FileEvent::Accept(names) => match names.first() {
                Some(name) => self.saveas_accept(name, icons),
                None => Reply::none(),
            },
            FileEvent::Cancel => {
                self.saveas = None;
                self.pending.cancelled();
                Reply::redraw_all()
            }
            FileEvent::NeedListing(_)
            | FileEvent::FilterChanged(_)
            | FileEvent::Wildcard { .. }
            | FileEvent::NeedPlace(_)
            | FileEvent::Resize => Reply::none(),
        }
    }

    /// A typed folder opens in the dialog; a file name is written, after the replace prompt when another file has it.
    pub(crate) fn saveas_accept(&mut self, name: &str, icons: &mut IconCache) -> Reply {
        let path = match self.host.resolve(name) {
            Ok(Resolved::Folder(id, _)) => {
                if let Some(dlg) = self.saveas.as_mut() {
                    self.host.list(dlg, id, icons, &type_keep(dlg));
                    dlg.clear_file_name();
                }
                return Reply::redraw();
            }
            Ok(Resolved::File(path)) => path,
            Err(msg) => {
                log::warn!("save as {name}: {msg}");
                self.error = Some(self.error_box(&msg));
                return Reply::redraw_all();
            }
        };
        // Ask before writing over another file; the document's own path is what Save As opened on, so it is not "another".
        if taken(&path) && self.path.as_deref() != Some(&path) {
            self.overwrite = Some(path);
            self.overwrite_focus = 1;
            self.overwrite_shown = oneshot::Shown::default();
            return Reply::redraw_all();
        }
        self.saveas_write(path)
    }

    /// Write the document to `path` in the encoding picked in Save As and adopt both: Save As's
    /// accept once nothing stands in its way.
    pub(crate) fn saveas_write(&mut self, path: PathBuf) -> Reply {
        let picked = (self.saveas.as_ref())
            .and_then(FileDlg::extra)
            .and_then(|f| f.combo_selected(ENCODING));
        let format = picked.map_or(self.format, |row| self.format.with_encoding_row(row));
        match self.write_to(&path, format) {
            Ok(()) => {
                self.format = format;
                self.stamp = Stamp::of(&path);
                self.path = Some(path);
                self.dirty = false;
                self.forget_rescued();
                self.saveas = None;
                match self.pending.saved(true) {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            Err(e) => {
                self.save_failed(&path, &e);
                Reply::redraw_all()
            }
        }
    }

    /// The "already exists" prompt: warning triangle, full path, Yes / No with No the default and
    /// neither carrying an access key; the default follows the focus Tab and the arrows move.
    pub(crate) fn overwrite_fields(&self, path: &Path) -> msgbox::Fields {
        msgbox::Fields {
            text: tr!("overwrite", path = path.display().to_string()),
            icon: self.warn_icon.clone(),
            buttons: vec![
                reprisekit::kit_tr!("yes-bare"),
                reprisekit::kit_tr!("no-bare"),
            ],
            default: self.overwrite_focus,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        }
    }

    /// The replace prompt's loop: Yes writes over the file, No (also the close box and Escape) goes back
    /// to Save As. Enter and Space press the focused button, which `msgbox_event` keeps across the per-event forms.
    pub(crate) fn overwrite_event(&mut self, ev: Event) -> Reply {
        let Some(path) = self.overwrite.clone() else {
            return Reply::none();
        };
        let mut fields = self.overwrite_fields(&path);
        let step = oneshot::msgbox_event(
            &ev,
            &self.dialog_text,
            &self.forms,
            &mut fields,
            &self.overwrite_shown,
        );
        self.overwrite_focus = fields.default;
        match step {
            FormStep::Closed => self.overwrite_outcome(form::Event::Cancel),
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.overwrite_outcome(out),
        }
    }

    /// Yes writes over the file; No, Escape and the close box go back to Save As with the file untouched.
    pub(crate) fn overwrite_outcome(&mut self, out: form::Event) -> Reply {
        let Some(path) = self.overwrite.clone() else {
            return Reply::none();
        };
        match out {
            form::Event::Pressed(id) if id == msgbox::button_id(0) => {
                self.overwrite = None;
                self.saveas_write(path)
            }
            form::Event::Default | form::Event::Cancel | form::Event::Pressed(_) => {
                self.overwrite = None;
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }
}
