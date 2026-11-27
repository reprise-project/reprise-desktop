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

/// The Encoding row of Open and Save As: its label under Files of type's, its combo of `names`
/// under that field, on `selected`.
fn encoding_row(names: &[&str], selected: usize) -> Form {
    let field = Rect::new(
        filedlg::FIELD_X,
        filedlg::EXTRA_Y,
        filedlg::field_w(CLIENT.0),
        combo::COMBO_H,
    );
    let combo = Item::new(
        ENCODING,
        field,
        Control::Combo(Combo::new(names, selected, names.len())),
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

/// Where a fresh dialog starts when there is no current path: the Documents folder.
pub(crate) fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    start_in(reprisefs::places::user_dir("DOCUMENTS", "Documents"), home)
}

/// `documents` when it is a folder, else `home`, else `/`.
pub(crate) fn start_in(documents: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    (documents.filter(|d| d.is_dir()))
        .or(home)
        .unwrap_or_else(|| PathBuf::from("/"))
}

impl Notepad {
    /// Where a fresh dialog starts: the current document's folder, or for a copy out of a
    /// compressed folder the folder holding the archive. None for an Untitled one (or a copy
    /// that does not say), which opens where Notepad's last dialog was confirmed, else in
    /// `default_dir()`.
    pub(crate) fn start_dir(&self) -> Option<PathBuf> {
        (self.path.as_deref()).and_then(reprisefs::archivecopy::dialog_folder)
    }

    fn open_at_start(&mut self, fields: Fields) -> FileDlg {
        match self.start_dir() {
            Some(dir) => self.host.open(&dir, fields, self.forms.text),
            None => self.host.open_last(&default_dir(), fields, self.forms.text),
        }
    }

    /// Declares the Open dialog open, on `start_dir()`, its Encoding on Auto-Detect. The listing
    /// waits for the first event, which has the icon cache.
    pub(crate) fn open_dialog(&mut self) {
        let mut dlg = self.open_at_start(Fields {
            title: tr!("title-open"),
            accept_label: tr!("open-button"),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec![tr!("filter-text"), tr!("filter-all")],
        });
        let auto = tr!("encoding-auto");
        let names: Vec<&str> = [auto.as_str()]
            .into_iter()
            .chain(READ_AS.iter().map(|&(name, _)| name))
            .collect();
        dlg.set_extra(encoding_row(&names, 0), ENCODING_ROW_H);
        self.open = Some(dlg);
    }

    /// The encoding picked in Open, `None` for Auto-Detect.
    fn read_as(&self) -> Option<Encoding> {
        let row = (self.open.as_ref())
            .and_then(FileDlg::extra)
            .and_then(|f| f.combo_selected(ENCODING))?;
        READ_AS.get(row.checked_sub(1)?).map(|&(_, e)| e)
    }

    pub(crate) fn saveas_dialog(&mut self) {
        let mut dlg = self.open_at_start(Fields {
            title: tr!("title-save-as"),
            accept_label: tr!("save-button"),
            mode: DlgMode::Save,
            multiple: false,
            filters: vec![tr!("filter-text"), tr!("filter-all")],
        });
        let names = ENCODINGS.map(|(name, _, _)| name);
        dlg.set_extra(
            encoding_row(&names, self.format.encoding_row()),
            ENCODING_ROW_H,
        );
        self.saveas = Some(dlg);
    }

    pub(crate) fn open_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        let driven = (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx);
        self.open_driven(driven)
    }

    /// A picked file is read into the buffer, after the save prompt when the text changed while
    /// Open was up (a paste answered late). Cancel leaves `pending` alone: any action has run by
    /// the time Open is up.
    pub(crate) fn open_driven(&mut self, driven: Driven) -> Reply {
        let path = match driven {
            Driven::Reply(r) => return *r,
            Driven::Picked(path) => path,
            Driven::Cancelled => {
                self.open = None;
                return Reply::redraw_all();
            }
        };
        let read_as = self.read_as();
        self.confirm_then(PendingAction::Pick(path, read_as))
    }

    pub(crate) fn open_picked(&mut self, path: &Path, read_as: Option<Encoding>) -> Reply {
        match read_text_as(path, read_as) {
            Ok((text, format)) => self.show_document(&text, format, Some(path.to_path_buf())),
            Err(e) => self.open_failed(path, &e),
        }
        Reply::redraw_all()
    }

    /// Replace the buffer with `text`, saved as `path` (`None` is Untitled), and close the Open dialog if it is up.
    pub(crate) fn show_document(&mut self, text: &str, format: Format, path: Option<PathBuf>) {
        if let Some(p) = &path {
            self.recent.record(p);
        }
        self.edit = MultiEdit::new(&format.line_end.to_edit(text));
        self.format = format;
        self.view = View::default();
        self.stamp = path.as_deref().and_then(Stamp::of);
        self.path = path;
        self.open = None;
        self.sync_menus();
    }

    pub(crate) fn saveas_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.saveas.as_mut() else {
            return Reply::none();
        };
        let driven = (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx);
        self.saveas_driven(driven)
    }

    /// A picked name is written, after the replace prompt when another file has it. Cancel drops
    /// any pending action: Save As is where an Untitled document's Yes on the save prompt waits.
    pub(crate) fn saveas_driven(&mut self, driven: Driven) -> Reply {
        let path = match driven {
            Driven::Reply(r) => return *r,
            Driven::Picked(path) => match &self.saveas {
                Some(dlg) => dlg.with_type_ext(path),
                None => path,
            },
            Driven::Cancelled => {
                self.close_saveas();
                self.pending.cancelled();
                return Reply::redraw_all();
            }
        };
        // Ask before writing over another file; the document's own path is what Save As opened on, so it is not "another".
        if taken(&path) && self.path.as_deref() != Some(&path) {
            let b = self.overwrite_box(&path);
            self.asks.ask_over(SAVEAS, b, Ask::Overwrite(path));
            return Reply::redraw_all();
        }
        self.saveas_write(path)
    }

    /// Save As goes, and the overwrite question it raised with it.
    fn close_saveas(&mut self) {
        self.saveas = None;
        self.asks.withdraw(|ask| matches!(ask, Ask::Overwrite(_)));
    }

    /// Write the document to `path` in the encoding picked in Save As and adopt both: Save As's
    /// accept once nothing stands in its way.
    pub(crate) fn saveas_write(&mut self, path: PathBuf) -> Reply {
        let picked = (self.saveas.as_ref())
            .and_then(FileDlg::extra)
            .and_then(|f| f.combo_selected(ENCODING));
        let format = picked.map_or(self.format, |row| self.format.with_encoding_row(row));
        self.saveas_write_in(path, format)
    }

    /// Save As's write in `format`; one that cannot hold the text asks over Save As first.
    pub(crate) fn saveas_write_in(&mut self, path: PathBuf, format: Format) -> Reply {
        if !format.holds(self.edit.lines()) {
            let b = self.unheld_box(&path, format);
            self.asks.ask_over(SAVEAS, b, Ask::Unheld(Some(path)));
            return Reply::redraw_all();
        }
        if reprisefs::ops::is_read_only(&path) {
            let b = self.read_only_box(&path);
            self.asks.ask_over(SAVEAS, b, Ask::Error);
            return Reply::redraw_all();
        }
        match self.write_to(&path, format) {
            Ok(()) => {
                self.recent.record(&path);
                self.format = format;
                self.stamp = Stamp::of(&path);
                self.path = Some(path);
                self.edit.mark_saved();
                self.forget_rescued();
                self.close_saveas();
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

    /// The "already exists" prompt: warning triangle, full path, Yes / No with No the default.
    /// Yes (Y) writes over the file; No (N), Escape and the close box go back to Save As with the
    /// file untouched.
    pub(crate) fn overwrite_box(&self, path: &Path) -> MsgBox {
        MsgBox::new(
            tr!("title-save-as"),
            msgbox::Fields {
                text: tr!("overwrite", path = path.display().to_string()),
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning)
    }
}
