//! File > Page Setup... and Print...: the picture placed on the page as Page Setup's scaling
//! says, at the screen's resolution.

use super::*;
use repriseapp::print::{Doc, Step};

impl Paint {
    /// Open Print for a copy of the picture; the print thread shrinks it while the dialog looks
    /// for printers.
    pub(crate) fn open_print(&mut self, ctx: &Ctx) {
        let img = &self.doc.img;
        let picture = Canvas::from_pixels(img.w.max(0) as u32, img.h.max(0) as u32, img.px.clone());
        let doc = Doc::Picture {
            picture: std::sync::Arc::new(picture),
            turns: 0,
            dpi: repriseapp::print::SCREEN_DPI,
        };
        let title = self
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or_else(|| tr!("untitled"), |n| n.to_string_lossy().into_owned());
        self.printing.print(doc, &title, ctx);
    }

    pub(crate) fn print_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match self.printing.event(win, &ev, ctx) {
            Step::Reply(r) => *r,
            Step::SetupChanged => Reply::redraw_all(),
        }
    }
}
