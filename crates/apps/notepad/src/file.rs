//! New, Open and Save as the document sees them, and the unsaved-changes prompt.

use super::*;

impl Notepad {
    /// The edit keeps LF, so a CRLF or CR file gets its breaks back here.
    pub(crate) fn write_to(&self, path: &Path, format: Format) -> std::io::Result<()> {
        let bytes = encode(self.edit.lines(), format).ok_or_else(|| unheld(format))?;
        reprisefs::ops::write_document(path, &bytes)
    }

    /// False when there is nowhere to write yet (the caller opens Save As), when the write failed,
    /// or when the file changed on disk and a box now asks whether to write anyway.
    pub(crate) fn save(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        if Stamp::of(&p) != self.stamp {
            let b = self.changed_box(&p);
            self.ask(b, Ask::Changed);
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
        .sounding(repriseapp::sound::Event::Warning)
    }

    /// Yes writes over the other program's change and runs the action waiting on the save; No
    /// keeps the file and drops the action.
    pub(crate) fn changed_answered(&mut self, answer: Option<usize>) -> Reply {
        if answer != Some(0) {
            self.pending.cancelled();
            return Reply::redraw_all();
        }
        let ok = self.save_anyway();
        if self.save_waits() {
            return Reply::redraw_all();
        }
        match self.pending.saved(ok) {
            Some(action) => self.run_pending(action),
            None => Reply::redraw_all(),
        }
    }

    fn save_anyway(&mut self) -> bool {
        self.save_in(self.format)
    }

    /// Writes the document over its file in `format` and adopts it; a format that cannot hold
    /// the text writes nothing and asks whether to save in UTF-8.
    fn save_in(&mut self, format: Format) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        if self.zip_entry.is_none() && reprisefs::ops::is_read_only(&p) {
            let b = self.read_only_box(&p);
            self.ask(b, Ask::ReadOnly);
            return false;
        }
        if !format.holds(self.edit.lines()) {
            let b = self.unheld_box(&p, format);
            self.ask(b, Ask::Unheld(None));
            return false;
        }
        if self.zip_entry.is_some() {
            self.format = format;
            if self.zip_entry.as_ref().is_some_and(|z| z.2) {
                self.update_zip();
            } else {
                let b = self.update_box(&p);
                self.ask(b, Ask::Update);
            }
            return false;
        }
        match self.write_to(&p, format) {
            Ok(()) => {
                self.format = format;
                self.recent.record(&p);
                self.stamp = Stamp::of(&p);
                self.edit.mark_saved();
                self.forget_rescued();
                true
            }
            Err(e) => {
                self.save_failed(&p, &e);
                false
            }
        }
    }

    /// Whether a save is waiting on a box (the file changed on disk, takes no writes, or its
    /// encoding cannot hold the text) or on its write into a zip.
    fn save_waits(&self) -> bool {
        self.zip_save.running()
            || (self.asks.iter()).any(|(_, ask)| {
                matches!(
                    ask,
                    Ask::Changed | Ask::ReadOnly | Ask::Update | Ask::Unheld(None)
                )
            })
    }

    fn update_box(&self, path: &Path) -> MsgBox {
        let name = |p: &Path| {
            (p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let zip = self
            .zip_entry
            .as_ref()
            .map_or_else(String::new, |(z, _, _)| name(z));
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("update-archive", name = name(path), archive = zip),
                icon: self.warn_icon.clone(),
                buttons: vec![
                    reprisekit::kit_tr!("yes"),
                    reprisekit::kit_tr!("no"),
                    reprisekit::kit_tr!("cancel"),
                ],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning)
    }

    /// Yes starts writing the document into its zip, and the action waiting on the save waits on
    /// that; No opens Save As as a read-only file's OK does; Cancel, Escape and the close box
    /// drop the action.
    pub(crate) fn update_answered(&mut self, answer: Option<usize>) -> Reply {
        match answer {
            Some(0) if self.zip_save.running() => return Reply::redraw_all(),
            Some(0) => self.agree_to_zip(),
            Some(1) => return self.read_only_answered(Some(0)),
            _ => {
                self.pending.cancelled();
                return Reply::redraw_all();
            }
        }
        if !self.update_zip() {
            self.pending.cancelled();
        }
        Reply::redraw_all()
    }

    /// Notes a Yes to updating the zip the document came out of, so its saves stop asking.
    fn agree_to_zip(&mut self) {
        if let Some(z) = &mut self.zip_entry {
            z.2 = true;
        }
    }

    /// Starts the job writing the document into the zip it came out of, and then over its copy.
    /// Its windows take no input until it ends ([`Notepad::zip_saved`]); refused while one runs.
    pub(crate) fn update_zip(&mut self) -> bool {
        let (Some(p), Some((zip, inner, _))) = (self.path.clone(), self.zip_entry.clone()) else {
            return false;
        };
        if self.zip_save.running() {
            return false;
        }
        let Some(bytes) = encode(self.edit.lines(), self.format) else {
            return false;
        };
        let name = |p: &Path| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let names = (name(&p), name(&zip));
        let wake = self.waker.clone();
        let job =
            reprisefs::extract::ArchiveJob::update_entry(zip, inner, bytes, Some(p), move || {
                if let Some(w) = &wake {
                    w.wake();
                }
            });
        let kind = reprisekit::controls::copyprogress::Kind::Compress;
        self.zip_save.start(job, kind, names, self.waker.as_ref())
    }

    /// The save into a zip ended: once it landed the document is saved and the action waiting
    /// on the save runs; a failure raises its box, and it and a Cancel drop the action.
    pub(crate) fn zip_saved(&mut self, ended: repriseapp::jobbox::Ended) -> Reply {
        use repriseapp::jobbox::Ended;
        match ended {
            Ended::Done => {
                self.stamp = self.path.as_deref().and_then(Stamp::of);
                self.edit.mark_saved();
                self.forget_rescued();
                match self.pending.saved(true) {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            Ended::Failed(text) => {
                log::warn!("update a zip: {text}");
                self.fail(&text);
                self.pending.cancelled();
                Reply::redraw_all()
            }
            Ended::Cancelled => {
                self.pending.cancelled();
                Reply::redraw_all()
            }
        }
    }

    pub(crate) fn read_only_box(&self, path: &Path) -> MsgBox {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ok_box(tr!("read-only", name = name), self.warn_icon.clone())
            .sounding(repriseapp::sound::Event::Warning)
    }

    /// OK opens Save As on the file's name; Escape and the close box drop the action waiting.
    pub(crate) fn read_only_answered(&mut self, answer: Option<usize>) -> Reply {
        if answer != Some(0) {
            self.pending.cancelled();
            return Reply::redraw_all();
        }
        self.saveas_dialog();
        let name = (self.path.as_deref())
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        if let (Some(dlg), Some(name)) = (self.saveas.as_mut(), name) {
            if Path::new(&name).extension().is_none() {
                dlg.set_filter(1);
            }
            dlg.set_file_name(&name);
        }
        Reply::redraw_all()
    }

    pub(crate) fn unheld_box(&self, path: &Path, format: Format) -> MsgBox {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let encoding = encoding_name(format.encoding);
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("cannot-hold", name = name, encoding = encoding),
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning)
    }

    /// Yes saves in UTF-8 instead, to `save_as` (Save As's pick) or else over the document's
    /// file. No writes nothing: Save As stays up, and a Save drops the action waiting on it.
    pub(crate) fn unheld_answered(
        &mut self,
        save_as: Option<PathBuf>,
        answer: Option<usize>,
    ) -> Reply {
        let utf8 = self.format.utf8();
        match (save_as, answer) {
            (Some(path), Some(0)) => self.saveas_write_in(path, utf8),
            (None, Some(0)) => {
                let ok = self.save_in(utf8);
                if self.save_waits() {
                    return Reply::redraw_all();
                }
                match self.pending.saved(ok) {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            (Some(_), _) => Reply::redraw_all(),
            (None, _) => {
                self.pending.cancelled();
                Reply::redraw_all()
            }
        }
    }

    pub(crate) fn save_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("save {}: {e}", path.display());
        self.fail(&tr!("save-failed", path = path.display().to_string()));
    }

    pub(crate) fn open_failed(&mut self, path: &Path, e: &std::io::Error) {
        log::warn!("open {}: {e}", path.display());
        if let Some(bad) = e
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Undecodable>())
        {
            let text = bad.lossy.clone();
            let prompt = MsgBox::new(
                tr!("name"),
                msgbox::Fields {
                    text: tr!(
                        "undecodable",
                        path = path.display().to_string(),
                        encoding = encoding_name(bad.encoding)
                    ),
                    icon: self.warn_icon.clone(),
                    buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                    default: 0,
                    min_text_h: None,
                    min_w: 0,
                    max_w: repriseapp::dialog_max_w(),
                },
            )
            .sounding(repriseapp::sound::Event::Warning);
            self.ask(prompt, Ask::Lossy(text));
            return;
        }
        self.fail(&open_failed_text(path, e));
    }

    /// The lossy copy's Yes: shown as Untitled, so a save cannot write the replaced characters
    /// over the file.
    pub(crate) fn show_lossy(&mut self, text: &str) {
        let line_end = document::LineEnd::of(text);
        let format = Format {
            line_end,
            ..Format::default()
        };
        self.show_document(text, format, None);
    }

    /// Leaves unsaved text in the recovery dir, in place of the recovery file it was opened from.
    pub(crate) fn rescue(&mut self) {
        let Some(dir) = self.rescue_dir.as_deref().filter(|_| self.dirty()) else {
            return;
        };
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        match write_recovery(
            dir,
            std::process::id(),
            &stem,
            self.edit.lines(),
            self.format,
        ) {
            Ok(kept) => {
                log::error!("unsaved text kept in {}", kept.display());
                if self.rescued.as_ref().is_some_and(|old| *old != kept) {
                    self.forget_rescued();
                }
            }
            Err(e) => log::error!("cannot keep the unsaved text in {}: {e}", dir.display()),
        }
    }

    /// Offers the recovery files left in the recovery dir, one prompt at a time, each naming its
    /// document and its place in the run.
    pub(crate) fn offer_recovered(&mut self) {
        let files = (self.rescue_dir.as_deref())
            .map(recovered_in)
            .unwrap_or_default();
        let count = files.len();
        for (i, file) in files.into_iter().enumerate() {
            let prompt = self.recovered_box(&recovered_name(&file), i + 1, count);
            self.ask(prompt, Ask::Recovered(file));
        }
    }

    fn recovered_box(&self, name: &str, index: usize, count: usize) -> MsgBox {
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("recovered", name = name, index = index, count = count),
                icon: self.warn_icon.clone(),
                buttons: vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning)
    }

    /// Yes opens the text of `file` as an unsaved Untitled document and offers no more files: here
    /// over an untouched Untitled, else in a Notepad of its own, so the document open stays. No
    /// sends the file to the Recycle Bin and the next is offered. Closing the prompt keeps the
    /// files for the next start.
    pub(crate) fn recovered_answered(&mut self, file: PathBuf, answer: Option<usize>) -> Reply {
        match answer {
            Some(0) => {
                self.asks.withdraw(|ask| matches!(ask, Ask::Recovered(_)));
                if self.path.is_some() || self.dirty() {
                    return Reply {
                        spawn: vec![recovered_argv(&file)],
                        ..Reply::redraw_all()
                    };
                }
                self.open_recovered(file);
            }
            Some(_) => recycle(&file),
            None => {
                self.asks.withdraw(|ask| matches!(ask, Ask::Recovered(_)));
            }
        }
        Reply::redraw_all()
    }

    /// Opens the text of recovery file `file` as an unsaved Untitled document.
    pub(crate) fn open_recovered(&mut self, file: PathBuf) {
        match read_text(&file) {
            Ok((text, format)) => {
                self.show_document(&text, format, None);
                self.edit.mark_unsaved();
                self.rescued = Some(file);
            }
            Err(e) => self.open_failed(&file, &e),
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

    /// Save changes; for a copy out of a zip its Yes also updates the zip, so the save asks
    /// nothing more.
    fn savechanges_box(&self) -> MsgBox {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        let doc = (self.path.as_deref())
            .and_then(name)
            .unwrap_or_else(|| tr!("untitled"));
        let text = match self.zip_entry.as_ref().and_then(|(zip, _, _)| name(zip)) {
            Some(zip) => tr!("save-changes-archive", name = doc, archive = zip),
            None => tr!("save-changes", name = doc),
        };
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text,
                icon: self.warn_icon.clone(),
                buttons: vec![
                    reprisekit::kit_tr!("yes"),
                    reprisekit::kit_tr!("no"),
                    reprisekit::kit_tr!("cancel"),
                ],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: repriseapp::dialog_max_w(),
            },
        )
        .sounding(repriseapp::sound::Event::Warning)
    }

    /// Runs `action` on a clean document; on a dirty one it waits on Save changes, which is
    /// asked once however often the action is. Open closes first, since the prompt's Yes may
    /// open Save As.
    pub(crate) fn confirm_then(&mut self, action: PendingAction) -> Reply {
        let Some(action) = self.pending.ask(action, self.dirty()) else {
            self.open = None;
            if !(self.asks.iter()).any(|(_, ask)| matches!(ask, Ask::SaveChanges)) {
                let b = self.savechanges_box();
                self.ask(b, Ask::SaveChanges);
            }
            return Reply::redraw_all();
        };
        self.run_pending(action)
    }

    pub(crate) fn run_pending(&mut self, action: PendingAction) -> Reply {
        match action {
            PendingAction::Quit => Reply {
                quit: true,
                ..Default::default()
            },
            PendingAction::New => {
                self.edit = MultiEdit::new("");
                self.format = Format::default();
                self.view = View::default();
                self.path = None;
                self.zip_entry = None;
                Reply::redraw_all()
            }
            PendingAction::Open => {
                self.open_dialog();
                Reply::redraw_all()
            }
            PendingAction::OpenFile(path) => {
                match load(&path) {
                    Ok((text, format)) => self.show_document(&text, format, Some(path)),
                    Err(e) => self.open_failed(&path, &e),
                }
                Reply::redraw_all()
            }
            PendingAction::Pick(path, read_as) => self.open_picked(&path, read_as),
            PendingAction::Lossy(text) => {
                self.show_lossy(&text);
                Reply::redraw_all()
            }
        }
    }

    /// With nowhere to write yet, Save As opens and the action keeps waiting on it.
    pub(crate) fn savechanges_yes(&mut self) -> Reply {
        if self.path.is_none() {
            self.saveas_dialog();
            return Reply::redraw_all();
        }
        self.agree_to_zip();
        let ok = self.save();
        if self.save_waits() {
            return Reply::redraw_all();
        }
        match self.pending.saved(ok) {
            Some(action) => self.run_pending(action),
            None => Reply::redraw_all(),
        }
    }

    /// Yes saves, No discards the changes, and either runs the action waiting; Cancel, Escape and
    /// the close box drop it.
    pub(crate) fn savechanges_answered(&mut self, button: Option<usize>) -> Reply {
        match button {
            Some(0) => self.savechanges_yes(),
            Some(1) => {
                self.edit.mark_saved();
                self.forget_rescued();
                match self.pending.discarded() {
                    Some(action) => self.run_pending(action),
                    None => Reply::redraw_all(),
                }
            }
            _ => {
                self.pending.cancelled();
                Reply::redraw_all()
            }
        }
    }
}

/// The command line of a Notepad that opens the text of recovery file `file` ([`RECOVERED`]).
pub(crate) fn recovered_argv(file: &Path) -> Vec<std::ffi::OsString> {
    vec!["reprise-notepad".into(), RECOVERED.into(), file.into()]
}

/// How a recovery prompt names the text kept in `file`: its document's name, and for an Untitled
/// document its first line as well.
fn recovered_name(file: &Path) -> String {
    const LINE_CHARS: usize = 40;
    let untitled = tr!("untitled");
    let stem = reprisefs::recovery::stem(file).unwrap_or(&untitled);
    if stem != untitled {
        return stem.to_string();
    }
    let Ok((text, _)) = read_text(file) else {
        return untitled;
    };
    let Some(line) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return untitled;
    };
    let line = match line.char_indices().nth(LINE_CHARS) {
        Some((cut, _)) => format!("{}...", line[..cut].trim_end()),
        None => line.to_string(),
    };
    tr!("recovered-untitled", line = line)
}

/// Sends a declined recovery file to the Recycle Bin under [`recycled_name`], or deletes it on
/// a drive set to remove files immediately; one that can go neither way stays and is offered at
/// the next start.
fn recycle(file: &Path) {
    let Some(bins) = reprisefs::trash::Bins::system() else {
        log::warn!("no Recycle Bin; {} is kept", file.display());
        return;
    };
    let gone = match bins.fate(file) {
        reprisefs::bincap::Fate::RemoveImmediately => std::fs::remove_file(file),
        _ => trash_named(&bins, file),
    };
    if let Err(e) = gone {
        log::warn!("{} is kept: {e}", file.display());
    }
}

/// Renamed beside itself first; a refused trash renames it back, so it keeps the recovery name
/// the next start looks for.
fn trash_named(bins: &reprisefs::trash::Bins, file: &Path) -> std::io::Result<()> {
    use reprisefs::trash::move_to_free;
    let named = file.with_file_name(format!("{}.txt", recycled_name(file)));
    if move_to_free(file, &named).is_err() {
        return bins.trash(file).map(drop);
    }
    bins.trash(&named).map(drop).inspect_err(|_| {
        if let Err(e) = move_to_free(&named, file) {
            log::warn!("cannot name {} back: {e}", named.display());
        }
    })
}

/// The name a declined recovery file shows in the Recycle Bin: its document's, and for an
/// Untitled one the time its text was kept.
fn recycled_name(file: &Path) -> String {
    use reprisefs::locale;
    let untitled = tr!("untitled");
    let stem = reprisefs::recovery::stem(file).unwrap_or(&untitled);
    let name = if stem == untitled {
        let kept = (file.metadata().and_then(|m| m.modified()).ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let when = locale::strftime(&locale::localtime(kept), c"%Y-%m-%d %H.%M");
        format!("{untitled} {when}")
    } else {
        stem.to_string()
    };
    tr!("recycled-name", name = name)
}
