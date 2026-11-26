//! New, Open and Save as the document sees them, and the unsaved-changes prompt.

use super::*;

impl Notepad {
    /// The edit keeps LF, so a CRLF or CR file gets its breaks back here.
    pub(crate) fn write_to(&self, path: &Path, format: Format) -> std::io::Result<()> {
        reprisefs::ops::write_document(path, &encode(self.edit.lines(), format))
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
        match self.pending.saved(ok) {
            Some(action) => self.run_pending(action),
            None => Reply::redraw_all(),
        }
    }

    fn save_anyway(&mut self) -> bool {
        let Some(p) = self.path.clone() else {
            return false;
        };
        match self.write_to(&p, self.format) {
            Ok(()) => {
                self.recent.record(&p);
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
        let Some(dir) = self.rescue_dir.as_deref().filter(|_| self.dirty) else {
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

    /// Yes opens the text of `file` as an unsaved Untitled document and offers no more files; No
    /// sends the file to the Recycle Bin and the next is offered. Closing the prompt keeps the
    /// files for the next start.
    pub(crate) fn recovered_answered(&mut self, file: PathBuf, answer: Option<usize>) -> Reply {
        match answer {
            Some(0) => {
                self.asks.withdraw(|ask| matches!(ask, Ask::Recovered(_)));
                match read_text(&file) {
                    Ok((text, format)) => {
                        self.show_document(&text, format, None);
                        self.dirty = true;
                        self.rescued = Some(file);
                    }
                    Err(e) => self.open_failed(&file, &e),
                }
            }
            Some(_) => recycle(&file),
            None => {
                self.asks.withdraw(|ask| matches!(ask, Ask::Recovered(_)));
            }
        }
        Reply::redraw_all()
    }

    /// The recovered document was saved or discarded: its recovery file goes.
    pub(crate) fn forget_rescued(&mut self) {
        if let Some(file) = self.rescued.take()
            && let Err(e) = std::fs::remove_file(&file)
        {
            log::warn!("remove {}: {e}", file.display());
        }
    }

    fn savechanges_box(&self) -> MsgBox {
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        MsgBox::new(
            tr!("name"),
            msgbox::Fields {
                text: tr!("save-changes", name = name),
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
        let Some(action) = self.pending.ask(action, self.dirty) else {
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
                self.dirty = false;
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
            PendingAction::Pick(path) => self.open_picked(&path),
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
        let ok = self.save();
        if (self.asks.iter()).any(|(_, ask)| matches!(ask, Ask::Changed)) {
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
                self.dirty = false;
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
