use crate::Explorer;
use crate::dialogs::{Dialog, ProgramBrowse};
use repriseapp::filedlg::{Driven, Host};
use repriseapp::iconjobs::IconWaits;
use repriseapp::openwith::{PROGRAMS_DIR, browse_fields, program_pick, programs_keep as keep};
use repriseapp::{Ctx, Event, Reply};
use reprisekit::controls::openwith::{self, OpenWith};
use reprisekit::icontheme::IconCache;
use reprisekit::look::Paint;
use reprisekit::raster::Canvas;
use std::path::Path;

/// The chooser's rows' icons at `px`, stand-ins noted in `waits` while one decodes off the loop.
pub fn chooser_row_icons(
    ow: &OpenWith,
    icons: &mut IconCache,
    px: i32,
    waits: &mut IconWaits,
) -> Vec<Option<Canvas>> {
    (ow.list.rows.iter())
        .map(|r| openwith::icon_name(r).and_then(|n| waits.first(icons, &[n], px)))
        .collect()
}

impl Explorer {
    /// A file dialog's host, reading and editing the pinned folders through Explorer's own.
    pub(crate) fn dialog_host(&self) -> Host {
        let mut host = Host::default();
        if let Some(p) = &self.pins {
            host.share_pins(p.share());
        }
        host
    }

    pub fn chooser_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up_mut() else {
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
            openwith::Event::Cancel => self.dismiss_up(ctx),
            openwith::Event::Browse => {
                let mut host = self.dialog_host();
                host.icons.prime(&mut ctx.icons);
                let dlg = host.open(Path::new(PROGRAMS_DIR), browse_fields(), self.forms.text);
                if let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up_mut() {
                    cp.browse = Some(ProgramBrowse { host, dlg });
                }
                Reply::redraw_all()
            }
            openwith::Event::Choose { id, always } => {
                match self.take_up_if(|d| matches!(d, Dialog::ChooseProgram(_))) {
                    Some(Dialog::ChooseProgram(c)) => self.choose_program_ok(*c, id, always, ctx),
                    _ => Reply::none(),
                }
            }
        }
    }

    /// A reader's ScrollIntoView on a program out of view, or Focus on a button; true when the
    /// chooser changed.
    pub fn chooser_reader(&mut self, act: repriseapp::Reader) -> bool {
        let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up_mut() else {
            return false;
        };
        let ev = match act {
            repriseapp::Reader::Reveal { row, .. } => cp.ow.reveal(row),
            repriseapp::Reader::Focus((x, y)) => {
                let lay = cp.ow.layout(
                    &self.dialog_text,
                    self.forms.fonts.size_px,
                    (0, 0),
                    &self.forms.controls(),
                );
                cp.input.reader_focus(&mut cp.ow, &lay, (x, y))
            }
        };
        ev != openwith::Event::None
    }

    pub fn program_browse_event(&mut self, ev: Event, ctx: &mut Ctx) -> Reply {
        if matches!(self.dialogs.up(), Some(Dialog::NewShortcut(_))) {
            return self.shortcut_browse_event(ev, ctx);
        }
        let Some(Dialog::ChooseProgram(cp)) = self.dialogs.up_mut() else {
            return Reply::none();
        };
        let Some(b) = cp.browse.as_mut() else {
            return Reply::none();
        };
        let driven =
            b.host
                .drive_keeping(&mut b.dlg, &ev, &self.dialog_text, &self.forms, ctx, |d| {
                    keep(d.filter())
                });
        match driven {
            Driven::Reply(r) => *r,
            Driven::Cancelled => {
                cp.browse = None;
                Reply::redraw_all()
            }
            Driven::Picked(path) if path.is_file() => match program_pick(&path) {
                Some(p) => {
                    cp.browse = None;
                    cp.ow.add_program(p);
                    cp.wear_icons(&mut ctx.icons, self.forms.controls().text);
                    Reply::redraw_all()
                }
                None => {
                    log::warn!("explorer: {} is not UTF-8, cannot run it", path.display());
                    Reply::none()
                }
            },
            Driven::Picked(path) => {
                log::warn!("explorer: {} is not a file", path.display());
                Reply::none()
            }
        }
    }

    pub fn draw_program_browse(&self, c: &mut Canvas) {
        if let Some(b) = self.dialogs.up().and_then(crate::newshortcut::picker_of) {
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
        if let Some(b) = self.dialogs.up().and_then(crate::newshortcut::picker_of) {
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
