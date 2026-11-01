//! The box a file dialog asks or tells with ([`crate::filedlg::Host::asking`]): a modal child of
//! the dialog's window, made and run here so every program with a file dialog shows it.

use crate::{Caption, Child, Event, Host, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::raster::Canvas;

pub(crate) const FILE_BOX: WinId = WinId::named("reprise file dialog box");

impl Host {
    /// The children the program asks for, and the file dialog's box over its window.
    pub(crate) fn children(&self) -> Vec<Child> {
        let mut out = self.app.children();
        if let Some((owner, c)) = &self.file_box
            && self.slots.iter().any(|s| s.id == *owner)
        {
            out.push(c.clone());
        }
        out
    }

    /// The window `win` sits over: the box's dialog for the box.
    pub(crate) fn parent_of(&self, win: WinId) -> Option<WinId> {
        match &self.file_box {
            Some((owner, _)) if win == FILE_BOX => Some(*owner),
            _ => self.app.parent(win),
        }
    }

    /// Window `win`'s caption: a prompt's for the box.
    pub(crate) fn caption_of(&self, win: WinId) -> Caption {
        match win {
            FILE_BOX => Caption::Prompt,
            _ => self.app.caption(win),
        }
    }

    /// Notes whether the file dialog in `win` (or the one the box is over) has a box up now.
    pub(crate) fn follow_file_box(&mut self, win: WinId) {
        let owner = self.file_box.as_ref().map_or(win, |(o, _)| *o);
        let child = match self.app.drop_file_dialog(owner) {
            Some(f) => {
                f.host.prime_box(&mut self.ctx.icons);
                let forms = self.ctx.forms();
                f.host
                    .asking()
                    .map(|m| m.child(FILE_BOX, &self.ctx.text, &forms))
            }
            None => None,
        };
        self.file_box = child.map(|c| (owner, c));
    }

    /// An event on the box: its answer goes to the dialog's host, which draws again.
    pub(crate) fn file_box_event(&mut self, ev: Event) {
        let Some((owner, _)) = self.file_box else {
            return;
        };
        let forms = self.ctx.forms();
        let Some(f) = self.app.drop_file_dialog(owner) else {
            return;
        };
        let out = f.host.box_event(&ev, &self.ctx.text, &forms);
        let reply = match out {
            crate::oneshot::Outcome::None => Reply::none(),
            crate::oneshot::Outcome::Redraw => Reply::redraw(),
            crate::oneshot::Outcome::Closed(_) => {
                self.mark_dirty(owner);
                Reply::none()
            }
        };
        self.apply(FILE_BOX, reply);
    }

    /// Draws the box over window `owner`'s dialog.
    pub(crate) fn draw_file_box(
        app: &mut dyn crate::App,
        owner: WinId,
        c: &mut Canvas,
        ctx: &crate::Ctx,
    ) {
        if let Some(f) = app.drop_file_dialog(owner)
            && let Some(m) = f.host.asking()
        {
            m.draw(c, &ctx.forms(), &ctx.text);
        }
    }

    /// The box's reader tree.
    pub(crate) fn access_file_box(
        app: &mut dyn crate::App,
        owner: WinId,
        tree: &mut Tree,
        ctx: &crate::Ctx,
    ) {
        if let Some(f) = app.drop_file_dialog(owner)
            && let Some(m) = f.host.asking()
        {
            m.access(tree, &ctx.text, &ctx.forms());
        }
    }
}
