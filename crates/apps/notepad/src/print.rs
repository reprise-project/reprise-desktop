//! File > Page Setup... and Print...: the text printed wrapped between the margins in the
//! document's font, with Page Setup's header and footer in the margins.

use super::*;
use repriseapp::print::{Doc, Step, TextFont};

impl Notepad {
    /// Open Print for the text in the font Format > Font picked.
    pub(crate) fn open_print(&mut self, ctx: &Ctx) {
        let s = &self.settings;
        let doc = Doc::Text {
            font: TextFont {
                family: s.font.clone(),
                bold: s.bold,
                italic: s.italic,
                pt: s.size,
            },
            lines: self.edit.lines().to_vec(),
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
            Step::SetupChanged => {
                self.settings.page = Some(self.printing.setup.clone());
                self.save_settings();
                Reply::redraw_all()
            }
        }
    }
}
