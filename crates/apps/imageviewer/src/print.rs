//! File > Page Setup... and Print...: the picture shown, turned as it shows, fitted to the page.

use super::*;
use repriseapp::print::{Doc, Step};

impl Viewer {
    /// Open Print for the picture shown; its first frame for an animation.
    pub(crate) fn open_print(&mut self, ctx: &Ctx) {
        let Some((frame, _)) = self.picture.as_ref().and_then(|p| p.frames.first()) else {
            return;
        };
        let doc = Doc::Picture {
            picture: frame.clone(),
            turns: self.view.turns,
        };
        let title = self
            .folder
            .current()
            .and_then(|p| p.file_name())
            .map_or_else(|| tr!("name"), |n| n.to_string_lossy().into_owned());
        self.printing.print(doc, &title, ctx);
    }

    pub(crate) fn print_event(&mut self, win: WinId, ev: Event, ctx: &mut Ctx) -> Reply {
        match self.printing.event(win, &ev, ctx) {
            Step::Reply(r) => *r,
            Step::SetupChanged => Reply::redraw_all(),
        }
    }
}
