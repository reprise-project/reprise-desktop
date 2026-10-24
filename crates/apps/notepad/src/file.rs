//! New, Open and Save as the document sees them, and the unsaved-changes prompt.

use super::*;

impl Notepad {
    /// The edit keeps LF, so a CRLF file gets its breaks back here.
    pub(crate) fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let sep = if self.crlf { "\r\n" } else { "\n" };
        write_document(path, self.edit.lines(), sep)
    }

    /// False when there is nowhere to write yet: the caller opens Save As.
    pub(crate) fn save(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        match self.write_to(&p) {
            Ok(()) => {
                self.dirty = false;
                true
            }
            Err(e) => {
                self.save_failed(&p, &e);
                false
            }
        }
    }

    pub(crate) fn save_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("save {}: {e}", path.display());
        self.error = Some(self.error_box(&format!(
            "Cannot create the {} file.\nMake sure that the path and file name are correct.",
            path.display()
        )));
    }

    pub(crate) fn open_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("open {}: {e}", path.display());
        if let Some(bad) = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<std::string::FromUtf8Error>())
        {
            let text = String::from_utf8_lossy(bad.as_bytes()).into_owned();
            let prompt = MsgBox::new(
                "Notepad",
                msgbox::Fields {
                    text: format!(
                        "{} is not a UTF-8 text file.\nDo you want to open a copy with the unreadable characters replaced?",
                        path.display()
                    ),
                    icon: self.warn_icon.clone(),
                    buttons: vec!["Yes".into(), "No".into()],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                    max_w: repriseapp::dialog_max_w(),
                },
            );
            self.lossy = Some((prompt, text));
            return;
        }
        self.error = Some(self.error_box(&open_failed_text(path, e)));
    }

    /// Yes shows the copy as Untitled, so a save cannot write the replaced characters over the
    /// file.
    pub(crate) fn lossy_event(&mut self, ev: Event) -> Reply {
        let Some((b, _)) = &mut self.lossy else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(answer) => {
                let (_, text) = self.lossy.take().expect("checked above");
                if answer == Some(0) {
                    self.show_document(&text, None);
                }
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    pub(crate) fn savechanges_fields(&self) -> msgbox::Fields {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled".into());
        msgbox::Fields {
            text: format!(
                "The text in the {name} file has changed.\n\
                 Do you want to save the changes?"
            ),
            icon: self.warn_icon.clone(),
            buttons: vec!["&Yes".into(), "&No".into(), "Cancel".into()],
            default: self.savechanges_focus,
            min_text_h: None,
            min_w: 0,
            max_w: repriseapp::dialog_max_w(),
        }
    }

    pub(crate) fn confirm_then(&mut self, action: PendingAction) -> Reply {
        match self.pending.ask(action, self.dirty) {
            Some(action) => self.run_pending(action),
            None => {
                self.savechanges = true;
                self.savechanges_focus = 0;
                self.savechanges_shown = oneshot::Shown::default();
                Reply::redraw_all()
            }
        }
    }

    pub(crate) fn run_pending(&mut self, action: PendingAction) -> Reply {
        match action {
            PendingAction::Quit => Reply {
                quit: true,
                ..Default::default()
            },
            PendingAction::New => {
                self.edit = MultiEdit::new("");
                self.crlf = false;
                self.view = View::default();
                self.path = None;
                self.dirty = false;
                Reply::redraw_all()
            }
            PendingAction::Open => {
                self.open_dialog();
                Reply::redraw_all()
            }
            PendingAction::OpenFile(path) => {
                match load(&path) {
                    Ok(text) => self.show_document(&text, Some(path)),
                    Err(e) => self.open_failed(&path, &e),
                }
                Reply::redraw_all()
            }
        }
    }

    /// With nowhere to write yet, Save As opens and the action keeps waiting on it.
    pub(crate) fn savechanges_yes(&mut self) -> Reply {
        self.savechanges = false;
        if self.path.is_none() {
            self.saveas_dialog();
            return Reply::redraw_all();
        }
        let ok = self.save();
        match self.pending.saved(ok) {
            Some(action) => self.run_pending(action),
            None => Reply::redraw_all(),
        }
    }

    pub(crate) fn savechanges_event(&mut self, ev: Event) -> Reply {
        let mut fields = self.savechanges_fields();
        let step = oneshot::msgbox_event(
            &ev,
            &self.dialog_text,
            &self.forms,
            &mut fields,
            &self.savechanges_shown,
        );
        self.savechanges_focus = fields.default;
        match step {
            FormStep::Closed => {
                self.savechanges = false;
                self.pending.cancelled();
                Reply::redraw_all()
            }
            FormStep::Redraw => Reply::redraw(),
            FormStep::None => Reply::none(),
            FormStep::Out(out) => self.savechanges_outcome(out),
        }
    }

    pub(crate) fn savechanges_outcome(&mut self, out: form::Event) -> Reply {
        match out {
            form::Event::Default => self.savechanges_yes(),
            form::Event::Pressed(id) if id == msgbox::button_id(0) => self.savechanges_yes(),
            form::Event::Pressed(id) if id == msgbox::button_id(1) => {
                self.savechanges = false;
                self.dirty = false;
                match self.pending.discarded() {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            form::Event::Cancel | form::Event::Pressed(_) => {
                self.savechanges = false;
                self.pending.cancelled();
                Reply::redraw_all()
            }
            form::Event::None => Reply::none(),
            _ => Reply::redraw(),
        }
    }
}
