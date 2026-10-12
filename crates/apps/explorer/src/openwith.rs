use crate::Explorer;
use crate::dialogs::{Dialog, ProgramBrowse};
use repriseapp::filedlg::{Host, Resolved, Step};
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::filedlg::{DlgMode, Event as FileEvent, Fields};
use reprisekit::controls::openwith::{self, OpenWith, Program};
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::Path;

const PROGRAMS_DIR: &str = "/usr/bin";

pub fn chooser_row_icons(ow: &OpenWith, icons: &mut IconCache) -> Vec<Option<Canvas>> {
    ow.list
        .rows
        .iter()
        .map(|r| openwith::icon_name(r).and_then(|n| icons.get(n, 16)))
        .collect()
}

/// A picked program file as a chooser row: its path is its id, its file
/// name its label. `None` for a path that is not UTF-8, which cannot be
/// run as a command line.
pub fn program_pick(path: &Path) -> Option<Program> {
    let id = path.to_str()?.to_string();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| id.clone());
    Some(Program {
        id,
        name,
        icon: Some(openwith::PROGRAM_ICON.into()),
    })
}

/// Folders stay regardless, through `Host::list`'s own `is_dir` check.
fn keep(filter: usize) -> impl Fn(&reprisefs::listing::Item) -> bool {
    move |i| filter != 0 || i.exec
}

impl Explorer {
    pub fn chooser_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::ChooseProgram(cp)) = self.dialog.as_mut() else {
            return Reply::none();
        };
        let lay = cp.ow.layout(
            &self.dialog_text,
            self.forms.fonts.size_px,
            (0, 0),
            &self.forms.controls(),
        );
        let out = cp.input.event(&mut cp.ow, &lay, &ev, &ctx.theme.metrics);
        match out {
            openwith::Event::None => Reply::none(),
            openwith::Event::Redraw => Reply::redraw(),
            openwith::Event::Cancel => {
                self.dialog = None;
                Reply::redraw_all()
            }
            openwith::Event::Browse => {
                let mut host = Host::default();
                host.icons.prime(&mut ctx.icons);
                let dlg = host.open(
                    Path::new(PROGRAMS_DIR),
                    Fields {
                        title: "Browse".into(),
                        accept_label: "Open".into(),
                        filters: vec!["Programs".into(), "All Files".into()],
                        mode: DlgMode::Open,
                        multiple: false,
                    },
                    self.forms.text,
                );
                if let Some(Dialog::ChooseProgram(cp)) = self.dialog.as_mut() {
                    cp.browse = Some(ProgramBrowse { host, dlg });
                }
                Reply::redraw_all()
            }
            openwith::Event::Choose { id, always } => match self.dialog.take() {
                Some(Dialog::ChooseProgram(c)) => self.choose_program_ok(*c, id, always, ctx),
                other => {
                    self.dialog = other;
                    Reply::none()
                }
            },
        }
    }

    pub fn program_browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::ChooseProgram(cp)) = self.dialog.as_mut() else {
            return Reply::none();
        };
        let Some(b) = cp.browse.as_mut() else {
            return Reply::none();
        };
        let k = keep(b.dlg.filter());
        b.host.ensure_listed(&mut b.dlg, &mut ctx.icons, &k);
        let step = b.host.event(
            &mut b.dlg,
            &ev,
            &self.dialog_text,
            self.forms.fonts.size_px,
            ctx.size,
            &ctx.theme.metrics,
        );
        match step {
            Step::Closed | Step::Out(FileEvent::Cancel) => {
                cp.browse = None;
                Reply::redraw_all()
            }
            Step::Redraw => Reply::redraw(),
            Step::Resize => Reply::resize(),
            Step::Out(FileEvent::NeedListing(id)) => {
                b.host.list(&mut b.dlg, id, &mut ctx.icons, &k);
                Reply::redraw()
            }
            Step::Out(FileEvent::FilterChanged(_)) => {
                let k = keep(b.dlg.filter());
                if let Some(id) = b.host.folder {
                    b.host.list(&mut b.dlg, id, &mut ctx.icons, &k);
                }
                Reply::redraw()
            }
            Step::Out(FileEvent::Accept(names)) => {
                let Some(name) = names.first() else {
                    return Reply::none();
                };
                match b.host.resolve(name) {
                    Ok(Resolved::Folder(id, _)) => {
                        b.host.list(&mut b.dlg, id, &mut ctx.icons, &k);
                        b.dlg.clear_file_name();
                        Reply::redraw()
                    }
                    Ok(Resolved::File(path)) if path.is_file() => match program_pick(&path) {
                        Some(p) => {
                            cp.browse = None;
                            cp.ow.add_program(p);
                            cp.row_icons = chooser_row_icons(&cp.ow, &mut ctx.icons);
                            Reply::redraw_all()
                        }
                        None => {
                            log::warn!("explorer: {} is not UTF-8, cannot run it", path.display());
                            Reply::none()
                        }
                    },
                    Ok(Resolved::File(path)) => {
                        log::warn!("explorer: {} is not a file", path.display());
                        Reply::none()
                    }
                    Err(msg) => {
                        log::warn!("explorer: browse {name}: {msg}");
                        Reply::none()
                    }
                }
            }
            Step::None | Step::Out(_) => Reply::none(),
        }
    }

    pub fn draw_program_browse(&self, c: &mut Canvas) {
        if let Some(Dialog::ChooseProgram(cp)) = &self.dialog
            && let Some(b) = &cp.browse
        {
            let (w, h) = b.dlg.client();
            b.dlg.draw(
                c,
                &self.dialog_text,
                &Paint {
                    s: 1,
                    ..self.forms.paint()
                },
                reprisekit::raster::Rect::new(0, 0, w, h),
                self.forms.fonts.size_px,
            );
        }
    }

    pub fn access_program_browse(&self, tree: &mut reprisekit::access::Tree) {
        if let Some(Dialog::ChooseProgram(cp)) = &self.dialog
            && let Some(b) = &cp.browse
        {
            let (w, h) = b.dlg.client();
            b.dlg.access(
                tree,
                reprisekit::raster::Rect::new(0, 0, w, h),
                &self.dialog_text,
                self.forms.fonts.size_px,
            );
        }
    }
}
