//! New, Open and Save as the document sees them, and the unsaved-changes prompt.

use super::*;

impl Notepad {
    /// The edit keeps LF, so a CRLF file gets its breaks back here.
    pub(crate) fn write_to(&self, path: &Path) -> std::io::Result<()> {
        let sep = if self.crlf { "\r\n" } else { "\n" };
        write_document(path, self.edit.lines(), sep)
    }

    /// False when there is nowhere to write yet (the caller opens Save As), when the write failed,
    /// or when the file changed on disk and `changed` now asks whether to write anyway.
    pub(crate) fn save(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        if Stamp::of(&p) != self.stamp {
            self.changed = Some(self.changed_box(&p));
            return false;
        }
        self.save_anyway()
    }

    fn changed_box(&self, path: &Path) -> MsgBox {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("changed-on-disk", name = name),
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                default: 1,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
    }

    /// Yes writes over the other program's change and runs the action waiting on the save; No
    /// keeps the file and drops the action.
    pub(crate) fn changed_event(&mut self, ev: Event) -> Reply {
        let Some(b) = &mut self.changed else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(answer) => {
                self.changed = None;
                if answer != Some(0) {
                    self.pending.cancelled();
                    return Reply::redraw_all();
                }
                let ok = self.save_anyway();
                match self.pending.saved(ok) {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    fn save_anyway(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        match self.write_to(&p) {
            Ok(()) => {
                self.stamp = Stamp::of(&p);
                self.dirty = false;
                self.forget_rescued();
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
        self.error = Some(self.error_box(&tr!("save-failed", path = path.display().to_string())));
    }

    pub(crate) fn open_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("open {}: {e}", path.display());
        if let Some(bad) = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<std::string::FromUtf8Error>())
        {
            let text = String::from_utf8_lossy(bad.as_bytes()).into_owned();
            let prompt = MsgBox::new(
                tr!("name"),
                msgbox::Fields {
                    text: tr!("not-utf8", path = path.display().to_string()),
                    icon: self.warn_icon.clone(),
                    buttons: vec![
                        reprisekit::kit_tr!("yes-bare"),
                        reprisekit::kit_tr!("no-bare"),
                    ],
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

    /// Leaves unsaved text in the recovery dir, in place of the recovery file it was opened from.
    pub(crate) fn rescue(&mut self) {
        let Some(dir) = self.rescue_dir.as_deref().filter(|_| self.dirty) else {
            return;
        };
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        let sep = if self.crlf { "\r\n" } else { "\n" };
        match write_recovery(dir, std::process::id(), &stem, self.edit.lines(), sep) {
            Ok(kept) => {
                log::error!("unsaved text kept in {}", kept.display());
                if self.rescued.as_ref().is_some_and(|old| *old != kept) {
                    self.forget_rescued();
                }
            }
            Err(e) => log::error!("cannot keep the unsaved text in {}: {e}", dir.display()),
        }
    }

    /// Offers the recovery files left in the recovery dir, one prompt at a time.
    pub(crate) fn offer_recovered(&mut self) {
        self.recover_queue = self
            .rescue_dir
            .as_deref()
            .map(recovered_in)
            .unwrap_or_default();
        self.offer_next_recovered();
    }

    fn offer_next_recovered(&mut self) {
        if self.recover_queue.is_empty() {
            return;
        }
        let file = self.recover_queue.remove(0);
        let prompt = MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("recovered"),
                icon: self.warn_icon.clone(),
                buttons: vec![
                    reprisekit::kit_tr!("yes-bare"),
                    reprisekit::kit_tr!("no-bare"),
                ],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        );
        self.recovered = Some((prompt, file));
    }

    /// Yes opens the text as an unsaved Untitled document and offers no more files; No deletes
    /// the file and offers the next. Closing the prompt keeps the files for the next start.
    pub(crate) fn recovered_event(&mut self, ev: Event) -> Reply {
        let Some((b, _)) = &mut self.recovered else {
            return Reply::none();
        };
        match b.event(&ev, &self.dialog_text, &self.forms) {
            oneshot::Outcome::Closed(answer) => {
                let (_, file) = self.recovered.take().expect("checked above");
                match answer {
                    Some(0) => {
                        self.recover_queue.clear();
                        match read_text(&file) {
                            Ok(text) => {
                                self.show_document(&text, None);
                                self.dirty = true;
                                self.rescued = Some(file);
                            }
                            Err(e) => self.open_failed(&file, &e),
                        }
                    }
                    Some(_) => {
                        if let Err(e) = std::fs::remove_file(&file) {
                            log::warn!("remove {}: {e}", file.display());
                        }
                        self.offer_next_recovered();
                    }
                    None => self.recover_queue.clear(),
                }
                Reply::redraw_all()
            }
            oneshot::Outcome::Redraw => Reply::redraw(),
            oneshot::Outcome::None => Reply::none(),
        }
    }

    /// The recovered document was saved or discarded: its recovery file goes.
    pub(crate) fn forget_rescued(&mut self) {
        if let Some(file) = self.rescued.take()
            && let Err(e) = std::fs::remove_file(&file)
        {
            log::warn!("remove {}: {e}", file.display());
        }
    }

    pub(crate) fn savechanges_fields(&self) -> msgbox::Fields {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        msgbox::Fields {
            text: tr!("save-changes", name = name),
            icon: self.warn_icon.clone(),
            buttons: vec![
                reprisekit::kit_tr!("yes"),
                reprisekit::kit_tr!("no"),
                reprisekit::kit_tr!("cancel"),
            ],
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
        if self.changed.is_some() {
            return Reply::redraw_all();
        }
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
                self.forget_rescued();
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
