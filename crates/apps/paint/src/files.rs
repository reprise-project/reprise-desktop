//! New, Open, Save, Save As and Set as Desktop Background, the save prompt before a picture with
//! changes is let go, and the Open and Save As dialogs as this host runs them.

use super::*;
use crate::file::Kind;
use repriseapp::filedlg::{Driven, taken};
use reprisekit::controls::combo::{self, Combo};
use reprisekit::controls::filedlg::{self, DlgMode, Fields};
use reprisekit::controls::form::{Control, Item};
use std::path::Path;

const QUALITY: &str = "quality";
const QUALITIES: [u8; 4] = [100, 90, 75, 50];
const QUALITY_ROW_H: i32 = combo::COMBO_H + 8;

/// Save As's types, in its Save as type list.
const SAVE_KINDS: [Kind; 3] = [Kind::Png, Kind::Jpeg, Kind::Bmp];

fn quality_row(quality: u8) -> Form {
    let names = [
        tr!("quality-best"),
        tr!("quality-high"),
        tr!("quality-good"),
        tr!("quality-small"),
    ];
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let at = QUALITIES.iter().position(|&q| q == quality).unwrap_or(1);
    let field = Rect::new(
        filedlg::FIELD_X,
        filedlg::EXTRA_Y,
        filedlg::field_w(filedlg::CLIENT.0),
        combo::COMBO_H,
    );
    Form::new(vec![
        Item::label_at(
            "quality-label",
            filedlg::ROW_LABEL_X,
            filedlg::EXTRA_Y + 3,
            filedlg::FIELD_X - filedlg::ROW_LABEL_X,
            &tr!("quality-label"),
        ),
        Item::new(
            QUALITY,
            field,
            Control::Combo(Combo::new(&names, at, names.len())),
        ),
    ])
}

/// `path` with the type's extension added when its own names no type Paint saves.
pub fn with_ext(path: PathBuf, kind: Kind) -> PathBuf {
    if Kind::of(&path).is_some() {
        return path;
    }
    let mut s = path.into_os_string();
    s.push(".");
    s.push(kind.ext());
    PathBuf::from(s)
}

fn default_dir() -> PathBuf {
    reprisefs::places::user_dir("PICTURES", "Pictures")
        .and_then(|d| d.canonicalize().ok())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"))
}

impl Paint {
    fn dialog_at(&mut self, fields: Fields) -> FileDlg {
        let u = self.forms.text;
        match self
            .path
            .as_deref()
            .and_then(reprisefs::archivecopy::dialog_folder)
        {
            Some(dir) => self.host.open(&dir, fields, u),
            None => self.host.open_last(&default_dir(), fields, u),
        }
    }

    pub(crate) fn open_dialog_files(&mut self) {
        let dlg = self.dialog_at(Fields {
            title: tr!("title-open"),
            accept_label: tr!("open-button"),
            mode: DlgMode::Open,
            multiple: false,
            filters: vec![
                tr!("filter-pictures"),
                tr!("filter-png"),
                tr!("filter-jpeg"),
                tr!("filter-bmp"),
                tr!("filter-all"),
            ],
        });
        self.open = Some(dlg);
    }

    pub(crate) fn saveas_dialog(&mut self) {
        self.commit_text();
        self.doc.finish();
        let mut dlg = self.dialog_at(Fields {
            title: tr!("title-save-as"),
            accept_label: tr!("save-button"),
            mode: DlgMode::Save,
            multiple: false,
            filters: vec![tr!("filter-png"), tr!("filter-jpeg"), tr!("filter-bmp")],
        });
        dlg.set_extra(quality_row(self.quality), QUALITY_ROW_H);
        if let Some(stem) = (self.path.as_deref())
            .filter(|p| Kind::of(p).is_none())
            .and_then(Path::file_stem)
        {
            dlg.set_file_name(&format!("{}.{}", stem.to_string_lossy(), Kind::Png.ext()));
            dlg.set_filter(0);
        }
        self.saveas = Some(dlg);
    }

    /// Runs `then` once the changes are saved or let go; asks first when there are changes. Open
    /// closes when it asks, since the prompt's Yes may open Save As.
    pub(crate) fn confirm(&mut self, then: Then) -> Reply {
        self.commit_text();
        self.doc.finish();
        if !self.doc.dirty {
            return self.run(then);
        }
        self.open = None;
        let name = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        self.ask(
            tr!("save-changes", name = name),
            vec![
                reprisekit::kit_tr!("yes"),
                reprisekit::kit_tr!("no"),
                reprisekit::kit_tr!("cancel"),
            ],
            0,
            Prompt::SaveChanges(then),
        );
        Reply::redraw_all()
    }

    fn run(&mut self, then: Then) -> Reply {
        match then {
            Then::New => {
                self.doc = Doc::new(Bitmap::new(NEW_SIZE.0, NEW_SIZE.1, WHITE));
                self.path = None;
                self.zip_entry = None;
                self.hscroll.top = 0;
                self.vscroll.top = 0;
                Reply::redraw_all()
            }
            Then::Open => {
                self.open_dialog_files();
                Reply::redraw_all()
            }
            Then::Pick(path) => self.open_picked(path),
            Then::Quit => Reply {
                quit: true,
                ..Reply::none()
            },
            Then::Background => self.set_background(),
        }
    }

    pub(crate) fn answered(&mut self, what: Prompt, answer: Option<usize>) -> Reply {
        match (what, answer) {
            (Prompt::SaveChanges(then), Some(0)) => {
                let Some((path, kind)) = self.writable() else {
                    self.after_save = Some(then);
                    self.saveas_dialog();
                    return Reply::redraw_all();
                };
                if self.refuses(&path, Some(then.clone())) {
                    return Reply::redraw_all();
                }
                if self.write(&path, kind) {
                    return self.run(then);
                }
                Reply::redraw_all()
            }
            (Prompt::SaveChanges(then), Some(1)) => {
                self.doc.finish();
                self.doc.dirty = false;
                self.forget_rescued();
                self.run(then)
            }
            (Prompt::Overwrite(path, kind), Some(0)) => self.saveas_write(path, kind),
            (Prompt::ReadOnly(then), Some(0)) | (Prompt::Update(then), Some(1)) => {
                self.after_save = then;
                self.saveas_dialog();
                self.name_saveas();
                Reply::redraw_all()
            }
            (Prompt::Update(then), Some(0)) => match self.update_zip() {
                true => then.map_or_else(Reply::redraw_all, |then| self.run(then)),
                false => Reply::redraw_all(),
            },
            (Prompt::Recovered(file), answer) => self.recovered_answered(file, answer),
            _ => Reply::redraw_all(),
        }
    }

    /// Save: to the picture's file, or through Save As for an untitled one or one whose type
    /// Paint does not save.
    pub(crate) fn save(&mut self) -> Reply {
        self.commit_text();
        self.doc.finish();
        match self.writable() {
            Some((path, kind)) => {
                if !self.refuses(&path, None) {
                    self.write(&path, kind);
                }
            }
            None => self.saveas_dialog(),
        }
        Reply::redraw_all()
    }

    /// Whether `path` takes no writes, in which case a box says so and its OK opens Save As,
    /// with `then` waiting on that. A copy out of a zip asks to update the zip instead.
    fn refuses(&mut self, path: &Path, then: Option<Then>) -> bool {
        if !reprisefs::ops::is_read_only(path) {
            return false;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some((zip, _)) = &self.zip_entry {
            let archive = zip.file_name().unwrap_or_default().to_string_lossy();
            let text = tr!(
                "update-archive",
                name = name.into_owned(),
                archive = archive.into_owned()
            );
            let buttons = vec![
                reprisekit::kit_tr!("yes"),
                reprisekit::kit_tr!("no"),
                reprisekit::kit_tr!("cancel"),
            ];
            self.ask(text, buttons, 0, Prompt::Update(then));
            return true;
        }
        self.ask(
            tr!("read-only", name = name.into_owned()),
            vec![reprisekit::kit_tr!("ok")],
            0,
            Prompt::ReadOnly(then),
        );
        true
    }

    /// Whether `path` takes no writes, in which case a box says so.
    fn refused(&mut self, path: &Path) -> bool {
        if !reprisefs::ops::is_read_only(path) {
            return false;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        self.error(tr!("read-only", name = name.into_owned()));
        true
    }

    /// Save As on the picture's own name and type, for a file that takes no writes.
    fn name_saveas(&mut self) {
        let (Some(dlg), Some(path)) = (self.saveas.as_mut(), self.path.as_deref()) else {
            return;
        };
        let Some(kind) = SAVE_KINDS.iter().position(|&k| Kind::of(path) == Some(k)) else {
            return;
        };
        dlg.set_file_name(&path.file_name().unwrap_or_default().to_string_lossy());
        dlg.set_filter(kind);
    }

    /// The picture's file and its type, when it is one Paint saves; a GIF, WebP or TIFF it
    /// opened goes through Save As.
    fn writable(&self) -> Option<(PathBuf, Kind)> {
        let path = self.path.clone()?;
        Kind::of(&path).map(|kind| (path, kind))
    }

    /// Writes the picture into the zip it came out of, then over its read-only copy, so the copy
    /// keeps matching what the zip holds. A zip that cannot be written raises a box.
    fn update_zip(&mut self) -> bool {
        let (Some((path, kind)), Some((zip, inner))) = (self.writable(), self.zip_entry.clone())
        else {
            return false;
        };
        self.doc.finish();
        let written = file::encode(&self.doc.img, kind, self.quality).and_then(|bytes| {
            reprisefs::zipwrite::update_entry(&zip, &inner, &bytes).map(|()| bytes)
        });
        let bytes = match written {
            Ok(bytes) => bytes,
            Err(why) => {
                log::warn!("update {} in {}: {why}", inner.display(), zip.display());
                self.error(reprisefs::zipwrite::failure(&zip, &why));
                return false;
            }
        };
        if let Err(e) = reprisefs::ops::write_document(&path, &bytes) {
            log::warn!("update the copy {}: {e}", path.display());
        }
        self.doc.dirty = false;
        self.forget_rescued();
        true
    }

    /// Writes the picture as shown: a paste that landed under the box that asked is laid down
    /// first.
    fn write(&mut self, path: &Path, kind: Kind) -> bool {
        self.doc.finish();
        if self.refused(path) {
            return false;
        }
        match file::save(&self.doc.img, path, kind, self.quality) {
            Ok(()) => {
                self.recent.record(path);
                self.doc.dirty = false;
                self.forget_rescued();
                true
            }
            Err(e) => {
                log::warn!("save {}: {e}", path.display());
                self.error(tr!(
                    "save-failed",
                    path = path.display().to_string(),
                    error = e
                ));
                false
            }
        }
    }

    /// The picture's file as the desktop background, saved first when it has changes; a copy out
    /// of a compressed folder is kept elsewhere first.
    pub(crate) fn set_background(&mut self) -> Reply {
        self.commit_text();
        self.doc.finish();
        let path = match (self.path.clone(), self.writable()) {
            (Some(path), _) if !self.doc.dirty => path,
            (_, Some((path, kind))) => {
                if self.refuses(&path, Some(Then::Background)) || !self.write(&path, kind) {
                    return Reply::redraw_all();
                }
                path
            }
            (_, None) => {
                self.after_save = Some(Then::Background);
                self.saveas_dialog();
                return Reply::redraw_all();
            }
        };
        let kept = reprisefs::archivecopy::lasting_background(&path).map_err(|e| e.to_string());
        if let Err(e) = kept.and_then(|kept| set_wallpaper(&repriseconf::config_path(), &kept)) {
            log::warn!("background {}: {e}", path.display());
            self.error(tr!("background-failed", error = e));
        }
        Reply::redraw_all()
    }

    /// Leaves an unsaved picture in the recovery folder as a PNG, in place of the recovery file
    /// it came from.
    pub(crate) fn rescue(&mut self) {
        let Some(dir) = self.rescue_dir.clone() else {
            return;
        };
        self.commit_text();
        self.doc.finish();
        if !self.doc.dirty {
            return;
        }
        let stem = (self.path.as_deref())
            .and_then(Path::file_stem)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tr!("untitled"));
        let kept = file::encode(&self.doc.img, Kind::Png, 100).and_then(|bytes| {
            let pid = std::process::id();
            reprisefs::recovery::write(&dir, pid, &stem, Kind::Png.ext(), &bytes)
                .map_err(|e| e.to_string())
        });
        match kept {
            Ok(kept) => {
                log::error!("unsaved picture kept in {}", kept.display());
                if self.rescued.as_ref().is_some_and(|old| *old != kept) {
                    self.forget_rescued();
                }
            }
            Err(e) => log::error!("cannot keep the unsaved picture in {}: {e}", dir.display()),
        }
    }

    /// Offers the recovery files left in the recovery folder, one question at a time.
    pub(crate) fn offer_recovered(&mut self) {
        let files = (self.rescue_dir.as_deref())
            .map(|dir| reprisefs::recovery::found(dir, Kind::Png.ext()))
            .unwrap_or_default();
        for file in files {
            self.ask(
                tr!("recovered"),
                vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                0,
                Prompt::Recovered(file),
            );
        }
    }

    /// Yes opens the picture in `file` as an unsaved Untitled one and offers no more files; No
    /// deletes the file and the next is offered. Closing the question keeps the files for the
    /// next start.
    pub(crate) fn recovered_answered(&mut self, file: PathBuf, answer: Option<usize>) -> Reply {
        match answer {
            Some(0) => {
                self.prompt.withdraw(|p| matches!(p, Prompt::Recovered(_)));
                match file::load(&file) {
                    Ok(img) => {
                        self.doc = Doc::new(img);
                        self.doc.dirty = true;
                        self.path = None;
                        self.zip_entry = None;
                        self.rescued = Some(file);
                        self.hscroll.top = 0;
                        self.vscroll.top = 0;
                    }
                    Err(e) => self.error(tr!(
                        "open-failed",
                        path = file.display().to_string(),
                        error = e
                    )),
                }
            }
            Some(_) => {
                if let Err(e) = std::fs::remove_file(&file) {
                    log::warn!("remove {}: {e}", file.display());
                }
            }
            None => {
                self.prompt.withdraw(|p| matches!(p, Prompt::Recovered(_)));
            }
        }
        Reply::redraw_all()
    }

    /// The recovered picture was saved or let go: its recovery file goes.
    fn forget_rescued(&mut self) {
        if let Some(file) = self.rescued.take()
            && let Err(e) = std::fs::remove_file(&file)
        {
            log::warn!("remove {}: {e}", file.display());
        }
    }

    pub(crate) fn open_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.open.as_mut() else {
            return Reply::none();
        };
        let driven = (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx);
        self.open_driven(driven)
    }

    /// A pick asks to save first when the picture changed while Open was up (a paste answered
    /// late).
    pub(crate) fn open_driven(&mut self, driven: Driven) -> Reply {
        match driven {
            Driven::Reply(r) => *r,
            Driven::Picked(path) => self.confirm(Then::Pick(path)),
            Driven::Cancelled => {
                self.open = None;
                Reply::redraw_all()
            }
        }
    }

    fn open_picked(&mut self, path: PathBuf) -> Reply {
        match file::load(&path) {
            Ok(img) => {
                self.recent.record(&path);
                self.doc = Doc::new(img);
                self.zip_entry = reprisefs::archivecopy::updatable(&path);
                self.path = Some(path);
                self.open = None;
                self.zoom = 1;
                self.hscroll.top = 0;
                self.vscroll.top = 0;
            }
            Err(e) => {
                self.error(tr!(
                    "open-failed",
                    path = path.display().to_string(),
                    error = e
                ));
            }
        }
        Reply::redraw_all()
    }

    pub(crate) fn saveas_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(dlg) = self.saveas.as_mut() else {
            return Reply::none();
        };
        let driven = (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx);
        self.saveas_driven(driven)
    }

    pub(crate) fn saveas_driven(&mut self, driven: Driven) -> Reply {
        let path = match driven {
            Driven::Reply(r) => return *r,
            Driven::Picked(path) => path,
            Driven::Cancelled => {
                self.saveas = None;
                self.after_save = None;
                return Reply::redraw_all();
            }
        };
        let kind = self
            .saveas
            .as_ref()
            .and_then(|d| SAVE_KINDS.get(d.filter()).copied())
            .unwrap_or(Kind::Png);
        let quoted = self.saveas.as_ref().is_some_and(FileDlg::name_quoted);
        let path = if quoted { path } else { with_ext(path, kind) };
        let kind = Kind::of(&path).unwrap_or(kind);
        if self.refused(&path) {
            return Reply::redraw_all();
        }
        if taken(&path) && self.path.as_deref() != Some(&path) {
            self.ask(
                tr!("overwrite", path = path.display().to_string()),
                vec![reprisekit::kit_tr!("yes"), reprisekit::kit_tr!("no")],
                1,
                Prompt::Overwrite(path, kind),
            );
            return Reply::redraw_all();
        }
        self.saveas_write(path, kind)
    }

    fn saveas_write(&mut self, path: PathBuf, kind: Kind) -> Reply {
        if let Some(q) = (self.saveas.as_ref())
            .and_then(FileDlg::extra)
            .and_then(|f| f.combo_selected(QUALITY))
            .and_then(|i| QUALITIES.get(i))
        {
            self.quality = *q;
        }
        if !self.write(&path, kind) {
            return Reply::redraw_all();
        }
        self.path = Some(path);
        self.zip_entry = None;
        self.saveas = None;
        match self.after_save.take() {
            Some(then) => self.run(then),
            None => Reply::redraw_all(),
        }
    }
}

/// Points `[wallpaper] image` in `config` at `path`, ending a slideshow, which would show in its
/// place.
pub fn set_wallpaper(config: &Path, path: &Path) -> Result<(), String> {
    use toml_edit::{DocumentMut, Value};
    let image = path.to_str().ok_or("the picture's path is not UTF-8")?;
    let image_key = repriseconf::key("wallpaper", "image");
    let slideshow = repriseconf::key("wallpaper", "slideshow");
    repriseconf::edit(config, |text| {
        let mut doc: DocumentMut = text.parse().map_err(|e| format!("{e}"))?;
        repriseconf::set(&mut doc, &image_key, Value::from(image)).map_err(|e| e.to_string())?;
        repriseconf::remove(&mut doc, &slideshow);
        Ok::<_, String>(doc.to_string())
    })
    .map_err(|e| e.to_string())?
    .map(drop)
}
