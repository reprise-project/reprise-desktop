//! Customize's folder picture: Choose Picture... opens a file dialog (`PICTURE_BROWSE`) over the
//! sheet, the picture picked shows in the preview, and OK or Apply writes it beside the icon
//! ([`reprisefs::meta::set_folder_look`]). Restore Default goes back to none.

use crate::Explorer;
use crate::dialogs::ProgramBrowse;
use crate::props_dialog::{PROPS, Props};
use repriseapp::filedlg::{Driven, type_keep};
use repriseapp::listjob::Job;
use repriseapp::{Child, Ctx, Event, Reply, WinId};
use reprisekit::controls::filedlg::{DlgMode, Fields};
use reprisekit::controls::filepreview::Route;
use reprisekit::controls::filesheet;
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::tr;
use std::path::{Path, PathBuf};

pub const PICTURE_BROWSE: WinId = WinId::named("picture browse");

/// The preview of the picture the sheet shows, and Choose Picture...'s file dialog while it is up.
#[derive(Default)]
pub(crate) struct PicturePick {
    pub canvas: Option<Canvas>,
    job: Option<Job<Option<Canvas>>>,
    pub browse: Option<ProgramBrowse>,
}

/// Customize's preview of picture `path` drawn at `scale`: fitted inside the preview square and
/// centred in it, read as Space's preview reads a picture ([`crate::preview::read`]).
pub(crate) fn preview_picture(path: &Path, scale: i32) -> Option<Canvas> {
    let px = filesheet::PREVIEW_PX;
    let edge = (px * scale) as u32;
    let jailed = crate::preview::Jailed::installed();
    let read = crate::preview::read(path, Route::Picture, None, &jailed, (edge, edge), scale);
    let crate::preview::Loaded::Picture(picture) = read else {
        return None;
    };
    let mut square = Canvas::with_scale(px as u32, px as u32, scale);
    let (w, h) = (picture.width as i32, picture.height as i32);
    square.blit_over((px - w) / 2, (px - h) / 2, &picture);
    Some(square)
}

/// The file dialog's Files of type: the pictures Reprise decodes, then everything.
fn filters() -> Vec<String> {
    let globs: Vec<String> = (repriseapp::decode::EXTS.iter())
        .map(|e| format!("*.{e}"))
        .collect();
    vec![
        tr!("props-picture-all-pictures", globs = globs.join(";")),
        tr!("props-picture-all-files"),
    ]
}

impl Explorer {
    /// Choose Picture...: the file dialog over the sheet, in the folder itself.
    pub(crate) fn choose_picture(&mut self, ctx: &mut Ctx) -> Reply {
        let Some(dir) = self.props.as_ref().map(|p| p.info.path.clone()) else {
            return Reply::none();
        };
        let mut host = self.dialog_host();
        host.icons.prime(&mut ctx.icons);
        let fields = Fields {
            title: tr!("props-picture-browse"),
            accept_label: tr!("props-picture-open"),
            filters: filters(),
            mode: DlgMode::Open,
            multiple: false,
        };
        let dlg = host.open(&dir, fields, self.forms.text);
        if let Some(p) = self.props.as_mut() {
            p.picks.picture.browse = Some(ProgramBrowse { host, dlg });
        }
        Reply::redraw_all()
    }

    pub(crate) fn picture_browse(&self) -> Option<&ProgramBrowse> {
        self.props.as_ref()?.picks.picture.browse.as_ref()
    }

    pub(crate) fn picture_browse_mut(&mut self) -> Option<&mut ProgramBrowse> {
        self.props.as_mut()?.picks.picture.browse.as_mut()
    }

    pub(crate) fn picture_browse_child(&self) -> Option<Child> {
        let b = self.picture_browse()?;
        Some(Child {
            id: PICTURE_BROWSE,
            title: tr!("props-picture-browse"),
            size: b.dlg.min_client(),
            fixed: false,
            modal: true,
        })
    }

    pub(crate) fn picture_browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let (text, forms) = (&self.dialog_text, &self.forms);
        let Some(b) = (self.props.as_mut()).and_then(|p| p.picks.picture.browse.as_mut()) else {
            return Reply::none();
        };
        match (b.host).drive_keeping(&mut b.dlg, &ev, text, forms, ctx, type_keep) {
            Driven::Reply(r) => *r,
            Driven::Cancelled => self.close_picture_browse(),
            Driven::Picked(path) => {
                self.close_picture_browse();
                self.picture_chosen(Some(path), ctx)
            }
        }
    }

    fn close_picture_browse(&mut self) -> Reply {
        if let Some(p) = self.props.as_mut() {
            p.picks.picture.browse = None;
        }
        Reply::redraw_all()
    }

    pub(crate) fn draw_picture_browse(&self, c: &mut Canvas) {
        if let Some(b) = self.picture_browse() {
            let (w, h) = b.dlg.client();
            let paint = Paint {
                s: 1,
                ..self.forms.paint()
            };
            let px = self.forms.fonts.size_px;
            b.dlg
                .draw(c, &self.dialog_text, &paint, Rect::new(0, 0, w, h), px);
        }
    }

    pub(crate) fn access_picture_browse(&self, tree: &mut reprisekit::access::Tree) {
        if let Some(b) = self.picture_browse() {
            let (w, h) = b.dlg.client();
            let at = Rect::new(0, 0, w, h);
            b.dlg
                .access(tree, at, &self.dialog_text, self.forms.fonts.size_px);
        }
    }

    /// The sheet shows `picture`, or the folder's icon for None, until OK or Apply writes it. The
    /// picture is read off the loop; the icon shows until it lands.
    pub(crate) fn picture_chosen(&mut self, picture: Option<PathBuf>, ctx: &mut Ctx) -> Reply {
        let Some(p) = self.props.as_mut() else {
            return Reply::none();
        };
        p.picture_chosen = Some(picture.clone());
        let scale = ctx.icons.scale();
        let pick = &mut p.picks.picture;
        pick.canvas = None;
        pick.job =
            picture.map(|path| Job::spawn(Some(&ctx.wake), move || preview_picture(&path, scale)));
        crate::props_host::wear_picks(p, ctx);
        show_picture_buttons(p);
        Reply::redraw_all()
    }

    /// The preview of a picture chosen landed.
    pub(crate) fn take_sheet_picture(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let p = self.props.as_mut()?;
        let landed = p.picks.picture.job.as_ref()?.take()?;
        p.picks.picture.job = None;
        p.picks.picture.canvas = landed;
        crate::props_host::wear_picks(p, ctx);
        Some(Reply {
            redraw_wins: vec![PROPS],
            ..Reply::none()
        })
    }
}

/// One folder's sheet: Choose Picture... works, Restore Default while a picture is shown, and
/// Apply while anything changed.
pub(crate) fn show_picture_buttons(p: &mut Props) {
    let one_folder = p.set.is_none() && p.info.is_dir && p.info.launcher.is_none();
    let shown = p.shown_picture().is_some();
    filesheet::set_picture_buttons(&mut p.sheet, one_folder, shown);
    let on = p.dirty();
    p.sheet.set_apply_enabled(on);
}

/// Customize's preview wears the picture shown once it is read.
pub(crate) fn wear_picture(p: &mut Props) {
    let Some(canvas) = p.picks.picture.canvas.clone() else {
        return;
    };
    if p.shown_picture().is_none() {
        return;
    }
    for page in &mut p.sheet.pages {
        page.set_picture(filesheet::CUSTOM_PREVIEW, canvas.clone());
    }
}
