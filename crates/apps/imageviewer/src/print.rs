//! File > Page Setup..., Print Preview... and Print...: the picture shown, turned as it shows, placed on the page
//! as Page Setup's scaling says at the resolution its file states.

use super::*;
use repriseapp::print::{Doc, Step};

impl Viewer {
    /// Open Print for the picture shown; its first frame for an animation.
    pub(crate) fn open_print(&mut self, ctx: &Ctx) {
        if let Some((doc, title)) = self.print_doc() {
            self.printing.print(doc, &title, ctx);
        }
    }

    pub(crate) fn open_preview(&mut self, ctx: &Ctx) {
        if let Some((doc, title)) = self.print_doc() {
            self.printing.preview(doc, &title, ctx);
        }
    }

    fn print_doc(&self) -> Option<(Doc, String)> {
        let p = self.picture.as_ref()?;
        let (frame, _) = p.frames.first()?;
        let doc = Doc::Picture {
            picture: frame.clone(),
            turns: self.view.turns,
            dpi: p.dpi.unwrap_or(repriseapp::print::SCREEN_DPI),
        };
        let title = self
            .folder
            .current()
            .and_then(|p| p.file_name())
            .map_or_else(|| tr!("name"), |n| n.to_string_lossy().into_owned());
        Some((doc, title))
    }

    pub(crate) fn print_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match self.printing.event(win, &ev, ctx) {
            Step::Reply(r) => *r,
            Step::SetupChanged => Reply::redraw_all(),
        }
    }
}
