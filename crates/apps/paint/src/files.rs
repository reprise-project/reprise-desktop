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
        match self.path.as_deref().and_then(Path::parent) {
            Some(dir) => self.host.open(dir, fields, u),
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
        dlg.set_extra(
            quality_row(self.quality),
            QUALITY_ROW_H,
            &self.forms.look.controls_at_100(),
        );
        if let Some(stem) = (self.path.as_deref())
            .filter(|p| Kind::of(p).is_none())
            .and_then(Path::file_stem)
        {
            dlg.set_file_name(&format!("{}.{}", stem.to_string_lossy(), Kind::Png.ext()));
            dlg.set_filter(0);
        }
        self.saveas = Some(dlg);
    }

    /// Runs `then` once the changes are saved or let go; asks first when there are changes.
    pub(crate) fn confirm(&mut self, then: Then) -> Reply {
        self.commit_text();
        self.doc.finish();
        if !self.doc.dirty {
            return self.run(then);
        }
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
                self.hscroll.top = 0;
                self.vscroll.top = 0;
                Reply::redraw_all()
            }
            Then::Open => {
                self.open_dialog_files();
                Reply::redraw_all()
            }
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
                if self.write(&path, kind) {
                    return self.run(then);
                }
                Reply::redraw_all()
            }
            (Prompt::SaveChanges(then), Some(1)) => {
                self.doc.dirty = false;
                self.forget_rescued();
                self.run(then)
            }
            (Prompt::Overwrite(path, kind), Some(0)) => self.saveas_write(path, kind),
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
                self.write(&path, kind);
            }
            None => self.saveas_dialog(),
        }
        Reply::redraw_all()
    }

    /// The picture's file and its type, when it is one Paint saves; a GIF, WebP or TIFF it
    /// opened goes through Save As.
    fn writable(&self) -> Option<(PathBuf, Kind)> {
        let path = self.path.clone()?;
        Kind::of(&path).map(|kind| (path, kind))
    }

    fn write(&mut self, path: &Path, kind: Kind) -> bool {
        match file::save(&self.doc.img, path, kind, self.quality) {
            Ok(()) => {
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

    /// The picture's file as the desktop background, saved first when it has changes.
    pub(crate) fn set_background(&mut self) -> Reply {
        self.commit_text();
        self.doc.finish();
        let path = match (self.path.clone(), self.writable()) {
            (Some(path), _) if !self.doc.dirty => path,
            (_, Some((path, kind))) => {
                if !self.write(&path, kind) {
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
        if let Err(e) = set_wallpaper(&repriseconf::config_path(), &path) {
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
                vec![
                    reprisekit::kit_tr!("yes-bare"),
                    reprisekit::kit_tr!("no-bare"),
                ],
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
        let path = match (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx) {
            Driven::Reply(r) => return *r,
            Driven::Picked(path) => path,
            Driven::Cancelled => {
                self.open = None;
                return Reply::redraw_all();
            }
        };
        match file::load(&path) {
            Ok(img) => {
                self.doc = Doc::new(img);
                self.path = Some(path);
                self.open = None;
                self.zoom = 1;
                self.hscroll.top = 0;
                self.vscroll.top = 0;
                self.hold_scroll(ctx);
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
        let path = match (self.host).drive(dlg, &ev, &self.dialog_text, &self.forms, ctx) {
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
        let path = with_ext(path, kind);
        let kind = Kind::of(&path).unwrap_or(kind);
        if taken(&path) && self.path.as_deref() != Some(&path) {
            self.ask(
                tr!("overwrite", path = path.display().to_string()),
                vec![
                    reprisekit::kit_tr!("yes-bare"),
                    reprisekit::kit_tr!("no-bare"),
                ],
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
