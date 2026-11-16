//! The boxes a file dialog asks or tells with ([`crate::filedlg::Host::asking`]): modal children
//! of the dialog's window, made and run here so every program with a file dialog shows them. A
//! box raised while one is up waits behind it, and the next takes the other of two windows
//! ([`crate::asks::Asks`]).

use crate::asks::Asks;
use crate::oneshot::Outcome;
use crate::{App, Caption, Child, Ctx, Event, Host, Reply, WinId};
use reprisekit::access::Tree;
use reprisekit::raster::Canvas;

pub(crate) const FILE_BOX: WinId = WinId::named("reprise file dialog box");

/// Whether `win` is one of the two windows the file dialog's boxes take.
pub(crate) fn owns(win: WinId) -> bool {
    Asks::<()>::new(FILE_BOX).owns(win)
}

/// The window of the dialog the box `shown` is over, when `win` is a box's window: the box
/// sits over it, and that dialog's host takes the box's events, draws and reader tree.
pub(crate) fn owner(shown: &Option<(WinId, Child)>, win: WinId) -> Option<WinId> {
    shown.as_ref().filter(|_| owns(win)).map(|(o, _)| *o)
}

/// The window of the box up over the dialog in `owner`, its icon given, and whether the icon
/// changed.
pub(crate) fn follow(app: &mut dyn App, owner: WinId, ctx: &mut Ctx) -> (Option<Child>, bool) {
    let Some(f) = app.drop_file_dialog(owner) else {
        return (None, false);
    };
    let changed = f.host.prime_box(&mut ctx.icons);
    (f.host.box_child(&ctx.text, &ctx.forms()), changed)
}

/// `ev` on box window `win` over the dialog in `owner`, for the dialog's host to answer.
pub(crate) fn event(app: &mut dyn App, owner: WinId, win: WinId, ev: &Event, ctx: &Ctx) -> Outcome {
    match app.drop_file_dialog(owner) {
        Some(f) => f.host.box_event(win, ev, &ctx.text, &ctx.forms()),
        None => Outcome::None,
    }
}

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

    /// The window `win` sits over: the box's dialog for a box.
    pub(crate) fn parent_of(&self, win: WinId) -> Option<WinId> {
        owner(&self.file_box, win).or_else(|| self.app.parent(win))
    }

    /// Window `win`'s caption: a prompt's for a box.
    pub(crate) fn caption_of(&self, win: WinId) -> Caption {
        match owns(win) {
            true => Caption::Prompt,
            false => self.app.caption(win),
        }
    }

    /// Notes which box the file dialog in `win` (or the one the box is over) has up now, and
    /// redraws the box when its icon changed.
    pub(crate) fn follow_file_box(&mut self, win: WinId) {
        let owner = self.file_box.as_ref().map_or(win, |(o, _)| *o);
        let (child, changed) = follow(&mut *self.app, owner, &mut self.ctx);
        self.file_box = child.map(|c| (owner, c));
        if changed && let Some((_, c)) = &self.file_box {
            self.mark_dirty(c.id);
        }
    }

    /// An event on box window `win`: its answer goes to the dialog's host, which draws again.
    pub(crate) fn file_box_event(&mut self, win: WinId, ev: Event) {
        let Some((owner, _)) = self.file_box else {
            return;
        };
        let reply = match event(&mut *self.app, owner, win, &ev, &self.ctx) {
            Outcome::None => Reply::none(),
            Outcome::Redraw => Reply::redraw(),
            Outcome::Closed(_) => {
                self.mark_dirty(owner);
                Reply::none()
            }
        };
        self.apply(win, reply);
    }

    /// Draws the box up over window `owner`'s dialog.
    pub(crate) fn draw_file_box(app: &mut dyn App, owner: WinId, c: &mut Canvas, ctx: &Ctx) {
        if let Some(f) = app.drop_file_dialog(owner)
            && let Some(m) = f.host.asking()
        {
            m.draw(c, &ctx.forms(), &ctx.text);
        }
    }

    /// The reader tree of the box up.
    pub(crate) fn access_file_box(app: &mut dyn App, owner: WinId, tree: &mut Tree, ctx: &Ctx) {
        if let Some(f) = app.drop_file_dialog(owner)
            && let Some(m) = f.host.asking()
        {
            m.access(tree, &ctx.text, &ctx.forms());
        }
    }
}
