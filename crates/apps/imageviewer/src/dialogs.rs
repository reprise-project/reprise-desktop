//! The delete prompt, the error box and the Open dialog.

use super::*;
use repriseapp::filedlg::Driven;
use repriseapp::oneshot::Outcome;
use reprisefs::bincap::{ConfirmKind, DeleteMode, delete_prompt};
use reprisekit::controls::dlgtemplate::{self, Template};
use reprisekit::controls::filedlg::{DlgMode, Fields};
use reprisekit::controls::msgbox;

/// The Open dialog's Files of type rows: the pictures the viewer decodes, then everything.
fn filters() -> Vec<String> {
    let globs: Vec<String> = repriseapp::decode::EXTS
        .iter()
        .map(|e| format!("*.{e}"))
        .collect();
    vec![
        format!("All Pictures ({})", globs.join(";")),
        "All Files (*.*)".into(),
    ]
}

impl Viewer {
    /// Shows `text` in the error box.
    pub(crate) fn fail(&mut self, text: &str) {
        let icon = self.warn_icon.clone();
        self.error.show(NAME, text.into(), icon);
    }

    /// Delete, or Shift+Delete (`erase`): the drive and the Recycle Bin settings say whether it
    /// asks first and whether the file can be recycled, read on the worker.
    pub(crate) fn ask_delete(&mut self, erase: bool) -> Reply {
        let Some(path) = self.folder.current().map(Path::to_path_buf) else {
            return Reply::none();
        };
        if self.delete.is_some() {
            return Reply::none();
        }
        self.send(Job::Fate { path, erase });
        Reply::redraw_all()
    }

    /// The worker's answer to [`Viewer::ask_delete`]: the prompt, or the delete at once.
    pub(crate) fn fated(&mut self, path: PathBuf, mode: DeleteMode) -> bool {
        if self.folder.current() != Some(path.as_path()) {
            return false;
        }
        if !mode.ask {
            self.send(Job::Delete {
                path,
                erase: mode.erase,
            });
            return true;
        }
        let name = reprisefs::bincap::label(&path);
        let (title, text) = delete_prompt(ConfirmKind::File, &name, 1, mode.erase, mode.no_bin);
        let buttons = vec![dlgtemplate::YES.to_string(), dlgtemplate::NO.to_string()];
        let default = dlgtemplate::delete_default_index(&buttons, mode.erase);
        let fields = msgbox::Fields {
            text,
            icon: self.trash_icon.clone(),
            buttons,
            default,
            min_text_h: None,
            min_w: 0,
            max_w: 0,
        };
        let b = MsgBox::on_template(title, Template::MessageBox, fields);
        self.delete = Some((b, path, mode.erase));
        true
    }

    pub(crate) fn delete_event(&mut self, ev: Event) -> Reply {
        let Some((b, _, _)) = &mut self.delete else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            Outcome::Closed(picked) => {
                let (_, path, erase) = self.delete.take().expect("the prompt is up");
                if picked == Some(0) {
                    self.send(Job::Delete { path, erase });
                }
                Reply::redraw_all()
            }
            Outcome::Redraw => Reply::redraw(),
            Outcome::None => Reply::none(),
        }
    }

    /// The file is in the Recycle Bin or gone: the next picture shows. A failure says why.
    pub(crate) fn deleted(&mut self, path: &Path, result: Result<(), String>) {
        match result {
            Ok(()) => {
                if self.folder.current() == Some(path) {
                    self.folder.remove_current();
                    self.load_current();
                } else {
                    self.folder.pictures.retain(|p| p != path);
                }
            }
            Err(e) => {
                log::warn!("delete {}: {e}", path.display());
                let name = reprisefs::bincap::label(path);
                self.fail(&format!("Cannot delete {name}: {e}"));
            }
        }
    }

    /// File > Open, on the shown picture's folder or where the last Open was confirmed.
    pub(crate) fn open_dialog(&mut self) {
        let fields = Fields {
            title: "Open".into(),
            accept_label: "&Open".into(),
            mode: DlgMode::Open,
            multiple: false,
            filters: filters(),
        };
        let units = self.forms.text;
        let dir = self.folder.current().and_then(Path::parent);
        self.open = Some(match dir {
            Some(dir) => self.host.open(dir, fields, units),
            None => {
                let home =
                    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from);
                self.host.open_last(&home, fields, units)
            }
        });
    }

    /// A typed folder opens in the dialog; a file shows, with its folder's pictures around it.
    pub(crate) fn open_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        match (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx) {
            Driven::Reply(r) => *r,
            Driven::Picked(path) => {
                self.open = None;
                self.slide_due = None;
                self.show(path);
                Reply::redraw_all()
            }
            Driven::Cancelled => {
                self.open = None;
                Reply::redraw_all()
            }
            Driven::Failed(msg) => {
                self.fail(&msg);
                Reply::redraw_all()
            }
        }
    }
}
