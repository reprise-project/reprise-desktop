//! The delete prompt, the error box and the Open dialog.

use super::*;
use repriseapp::filedlg::{Resolved, Step as DlgStep, type_keep};
use repriseapp::oneshot::Outcome;
use reprisefs::bincap::{ConfirmKind, DeleteMode, delete_prompt};
use reprisekit::controls::dlgtemplate::{self, Template};
use reprisekit::controls::filedlg::{DlgMode, Event as FileEvent, Fields};
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
    pub(crate) fn error_box(&self, text: &str) -> MsgBox {
        MsgBox::new(
            NAME,
            msgbox::Fields {
                text: text.into(),
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("ok")],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
    }

    pub(crate) fn error_event(&mut self, ev: Event) -> Reply {
        let Some(b) = &mut self.error else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            Outcome::Closed(_) => {
                self.error = None;
                Reply::redraw_all()
            }
            Outcome::Redraw => Reply::redraw(),
            Outcome::None => Reply::none(),
        }
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
                self.error = Some(self.error_box(&format!("Cannot delete {name}: {e}")));
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

    pub(crate) fn open_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        self.host
            .ensure_listed(dlg, &mut ctx.icons, &type_keep(dlg));
        let step = self.host.event(
            dlg,
            &ev,
            &self.dialog_text,
            &self.forms,
            ctx.size,
            &ctx.theme.metrics,
        );
        let out = match step {
            DlgStep::Closed => FileEvent::Cancel,
            DlgStep::Redraw => return Reply::redraw(),
            DlgStep::Resize => return Reply::resize(),
            DlgStep::None => return Reply::none(),
            DlgStep::Out(FileEvent::NeedListing(id)) => {
                self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                return Reply::redraw();
            }
            DlgStep::Out(FileEvent::FilterChanged(_)) => {
                if let Some(id) = self.host.folder {
                    self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                }
                return Reply::redraw();
            }
            DlgStep::Out(out) => out,
        };
        match out {
            FileEvent::Redraw => Reply::redraw(),
            FileEvent::Accept(names) => match names.first() {
                Some(name) => self.open_accept(&name.clone(), ctx),
                None => Reply::none(),
            },
            FileEvent::Cancel => {
                self.open = None;
                Reply::redraw_all()
            }
            _ => Reply::none(),
        }
    }

    /// A typed folder opens in the dialog; a file shows, with its folder's pictures around it.
    fn open_accept(&mut self, name: &str, ctx: &mut Ctx) -> Reply {
        match self.host.resolve(name) {
            Ok(Resolved::Folder(id, _)) => {
                if let Some(dlg) = self.open.as_mut() {
                    self.host.list(dlg, id, &mut ctx.icons, &type_keep(dlg));
                    dlg.clear_file_name();
                }
                Reply::redraw()
            }
            Ok(Resolved::File(path)) => {
                self.open = None;
                self.slide_due = None;
                self.show(path);
                Reply::redraw_all()
            }
            Err(msg) => {
                self.error = Some(self.error_box(&msg));
                Reply::redraw_all()
            }
        }
    }
}
