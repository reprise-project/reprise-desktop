//! Glass's preview pane right of the view: the one selected file through the pane's reader
//! ([`panereader`]) and [`filepreview`]'s control, or a note when it shows none.
//!
//! One read is out at a time: while it is, a selection change only notes the file now wanted,
//! and the read for it starts once the one out answers.

use crate::Explorer;
use crate::panereader::{Key, Parts, Reader, Request, Taken};
use crate::preview::Loaded;
use repriseapp::{Ctx, Reply};
use reprisekit::controls::explorerbar::{NAV_DIVIDER_W, NAV_GRAB};
use reprisekit::controls::filepreview::{self, Body, FilePreview};
use reprisekit::controls::multiedit::{Metrics, MultiEdit};
use reprisekit::look::Paint;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::TextRenderer;
use reprisekit::tr;

/// The text box's face size at 100%, as Space's preview sets it.
const MONO_PX: u32 = 13;
/// How many times the pane's size a picture is read at, so a wider pane still draws it whole.
const READ_ROOM: i32 = 2;
/// The least the view keeps beside the panes, and the preview pane's own least width.
pub(crate) const VIEW_MIN_W: i32 = 200;
pub(crate) const PANE_MIN_W: i32 = reprisekit::controls::explorerbar::MIN_W;

pub(crate) struct Pane {
    pub qv: FilePreview,
    /// The file it shows, or is waiting to show.
    shown: Option<Key>,
    reader: Reader,
    /// What a new reader starts from when the old one's thread is gone.
    parts: Parts,
    wake: repriseapp::Waker,
    /// The file the read out reads.
    read: Option<Key>,
    mono: TextRenderer,
    metrics: Metrics,
    /// Reads started, for tests.
    #[cfg(test)]
    pub started: usize,
}

/// What the pane shows for the selection.
#[derive(Debug, PartialEq)]
enum Want {
    Note(String),
    File(usize),
}

impl Pane {
    fn new(ctx: &Ctx) -> Pane {
        Pane::with_parts(ctx, Parts::installed())
    }

    pub(crate) fn with_parts(ctx: &Ctx, parts: Parts) -> Pane {
        let px = ctx.theme.text.px(MONO_PX);
        let mono = TextRenderer::monospace(px);
        let mut metrics = Metrics {
            bars_always: false,
            ..Metrics::for_font(&mono, px)
        };
        metrics.follow_look(&ctx.theme.controls());
        Pane {
            qv: FilePreview::new(empty_facts(), None),
            shown: None,
            reader: Reader::spawn(parts.clone(), ctx.wake.clone()),
            parts,
            wake: ctx.wake.clone(),
            read: None,
            mono,
            metrics,
            #[cfg(test)]
            started: 0,
        }
    }

    /// Shows `text`; a read out keeps running and its answer is dropped.
    fn note(&mut self, text: String) {
        self.shown = None;
        if !matches!(self.qv.body(), Body::Note(t) if *t == text) {
            self.qv = FilePreview::new(empty_facts(), None);
            self.qv.set_body(Body::Note(text));
        }
    }

    fn land(&mut self, l: Loaded) {
        match l {
            Loaded::Picture(c) => self.qv.set_body(Body::Picture(c)),
            Loaded::Text(t) => self.qv.set_body(Body::Text(MultiEdit::new(&t))),
            Loaded::Nothing => self.qv.set_body(Body::Note(tr!("preview-pane-none"))),
        }
    }

    /// Whether a read is out.
    #[cfg(test)]
    pub fn reading(&self) -> bool {
        self.read.is_some()
    }
}

fn empty_facts() -> filepreview::Facts {
    filepreview::Facts {
        name: String::new(),
        kind: String::new(),
        size: None,
        modified: String::new(),
    }
}

/// The pane's body and the divider at its left edge in `frame`, as the shell layout gives it.
pub(crate) fn split(frame: Rect) -> (Rect, Rect) {
    let divider = Rect::new(frame.x, frame.y, NAV_DIVIDER_W, frame.height);
    let body = Rect::new(
        divider.right(),
        frame.y,
        (frame.width - NAV_DIVIDER_W).max(0),
        frame.height,
    );
    (divider, body)
}

/// Where a press takes the divider: it and [`NAV_GRAB`] px of the pane right of it, clear of the
/// view's scrollbar.
fn grip(frame: Rect) -> Rect {
    let (d, _) = split(frame);
    Rect::new(d.x, d.y, d.width + NAV_GRAB, d.height)
}

impl Explorer {
    /// The pane follows the selection: a note for none, a folder or several, else the file, read
    /// again only when it is another file or the file changed, and only once no read is out.
    /// Off, or out of the shell layout, it goes.
    pub(crate) fn sync_pane(&mut self, ctx: &mut Ctx) {
        if !(self.shell_on && self.settings.preview_pane) {
            self.pane = None;
            return;
        }
        let want = match self.pane_wants() {
            Want::Note(text) => Err(text),
            Want::File(i) => match (self.items.get(i), self.facts_of(i)) {
                (Some(it), Some(facts)) => Ok((it.clone(), facts)),
                _ => Err(tr!("preview-pane-none")),
            },
        };
        let frame = self.shell_layout(ctx).and_then(|sl| sl.preview);
        let busy = self.pane.as_ref().is_some_and(|p| p.read.is_some());
        let pane = self.pane.get_or_insert_with(|| Pane::new(ctx));
        let (it, facts) = match want {
            Err(text) => return pane.note(text),
            Ok(file) => file,
        };
        let key = (it.path.clone(), it.size, it.modified);
        if pane.shown.as_ref() == Some(&key) {
            return;
        }
        pane.qv = FilePreview::new(facts, None);
        pane.qv.set_body(Body::Note(String::new()));
        pane.shown = Some(key.clone());
        if busy {
            return;
        }
        let scale = ctx.icons.scale().max(1);
        let (w, h) = frame.map_or(filepreview::CARD, |r| (r.width, r.height));
        let room = |v: i32| ((v - 2 * filepreview::PAD).max(1) * READ_ROOM * scale) as u32;
        let route = filepreview::route(it.mime.as_deref());
        let request = || Request {
            key: key.clone(),
            route,
            max: (room(w), room(h)),
            scale,
        };
        if !pane.reader.ask(request()) {
            pane.reader = Reader::spawn(pane.parts.clone(), pane.wake.clone());
            pane.reader.ask(request());
        }
        pane.read = Some(key);
        #[cfg(test)]
        {
            pane.started += 1;
        }
    }

    fn pane_wants(&self) -> Want {
        let set = &self.sv.selection.set;
        let none = || Want::Note(tr!("preview-pane-none"));
        match (set.len(), self.sv.selection.single()) {
            (0, _) => Want::Note(tr!("preview-pane-select")),
            (1, Some(i)) => match self.sv.entries.get(i) {
                Some(e) if !e.is_folder() => Want::File(i),
                _ => none(),
            },
            _ => none(),
        }
    }

    /// The pane's read answered: it shows the file when that is still the one wanted, else the
    /// read for the one wanted now starts. Some when the pane changed.
    pub(crate) fn take_pane(&mut self, ctx: &mut Ctx) -> Option<Reply> {
        let pane = self.pane.as_mut()?;
        let (key, landed) = match pane.reader.take() {
            Taken::Answer(key, landed) => (key, landed),
            Taken::Waiting => return None,
            Taken::Gone => {
                pane.reader = Reader::spawn(pane.parts.clone(), pane.wake.clone());
                pane.read = None;
                pane.shown = None;
                self.sync_pane(ctx);
                return Some(Reply::redraw());
            }
        };
        pane.read = None;
        if pane.shown.as_ref() == Some(&key) {
            pane.land(landed);
            return Some(Reply::redraw());
        }
        if pane.shown.is_some() {
            pane.shown = None;
            self.sync_pane(ctx);
        }
        Some(Reply::none())
    }

    pub(crate) fn draw_pane(&self, c: &mut Canvas, frame: Rect, ctx: &Ctx) {
        let (divider, body) = split(frame);
        let paint = Paint {
            s: 1,
            ..ctx.theme.paint()
        };
        let line = (ctx.theme.look.style().and_then(|st| st.panes))
            .map_or(ctx.theme.colors.button_face, |p| p.preview_edge);
        c.fill_rect(divider, line);
        let Some(p) = &self.pane else {
            c.fill_rect(body, ctx.theme.colors.window);
            return;
        };
        let px = ctx.theme.fonts.size_px;
        filepreview::draw_pane(c, &paint, &ctx.text, px, &p.mono, &p.metrics, &p.qv, body);
    }

    pub(crate) fn access_pane(&self, tree: &mut reprisekit::access::Tree, frame: Rect) {
        let Some(p) = &self.pane else {
            return;
        };
        let (_, body) = split(frame);
        let node = reprisekit::access::named(
            reprisekit::access::accesskit::Role::Pane,
            body,
            &tr!("preview-pane-name"),
        );
        let id = tree.push(reprisekit::access::ROOT, "preview pane", node);
        filepreview::access_pane(tree, id, &p.qv, &p.mono, &p.metrics, body);
    }

    /// A press on the pane: its divider takes the drag, its scrollbars scroll; None off it.
    pub(crate) fn pane_press(&mut self, x: i32, y: i32, ctx: &mut Ctx) -> Option<Reply> {
        let frame = self.shell_layout(ctx)?.preview?;
        if grip(frame).contains(x, y) {
            self.pane_drag = Some(x - frame.x);
            return Some(Reply::redraw());
        }
        if !frame.contains(x, y) {
            return None;
        }
        let (_, body) = split(frame);
        let moved = self
            .pane
            .as_mut()
            .is_some_and(|p| p.qv.press((x, y), &p.mono, &p.metrics, body));
        Some(if moved {
            Reply::redraw()
        } else {
            Reply::none()
        })
    }

    /// The narrowest client that keeps the shown panes whole and the view [`VIEW_MIN_W`] wide.
    pub(crate) fn panes_min_w(&self) -> i32 {
        let u = self.forms.text;
        let s = &self.settings;
        let nav = match s.nav_pane {
            true => u.len(s.nav_width) + NAV_DIVIDER_W,
            false => 0,
        };
        let preview = match s.preview_pane {
            true => u.len(PANE_MIN_W) + NAV_DIVIDER_W,
            false => 0,
        };
        nav + u.len(VIEW_MIN_W) + preview
    }

    /// The held divider follows the pointer: the pane keeps at least [`PANE_MIN_W`] and the view
    /// [`VIEW_MIN_W`].
    pub(crate) fn pane_drag_to(&mut self, x: i32, ctx: &Ctx) -> Option<Reply> {
        let grab = self.pane_drag?;
        let u = ctx.theme.controls().text;
        let nav = self
            .shell_layout(ctx)
            .and_then(|sl| sl.nav_pane)
            .map_or(0, |r| r.width);
        let widest = u.within(ctx.size.0 - nav - NAV_DIVIDER_W - VIEW_MIN_W);
        let width = u
            .within(ctx.size.0 - (x - grab) - NAV_DIVIDER_W)
            .clamp(PANE_MIN_W, widest.max(PANE_MIN_W));
        if width == self.settings.preview_width {
            return Some(Reply::none());
        }
        self.settings.preview_width = width;
        Some(Reply::redraw())
    }

    /// The divider let go: its width is kept. True when it was held.
    pub(crate) fn pane_release(&mut self) -> bool {
        if let Some(p) = self.pane.as_mut() {
            p.qv.release();
        }
        let held = self.pane_drag.take().is_some();
        self.settings_dirty |= held;
        held
    }

    /// A held scrollbar thumb in the pane follows the pointer.
    pub(crate) fn pane_motion(&mut self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let Some(frame) = self.shell_layout(ctx).and_then(|sl| sl.preview) else {
            return false;
        };
        let (_, body) = split(frame);
        (self.pane.as_mut()).is_some_and(|p| p.qv.motion((x, y), &p.mono, &p.metrics, body))
    }

    /// The wheel over the pane scrolls its text; None off it.
    pub(crate) fn pane_scroll(&mut self, x: i32, y: i32, rows: i32, ctx: &Ctx) -> Option<Reply> {
        let frame = self.shell_layout(ctx)?.preview?;
        if !frame.contains(x, y) {
            return None;
        }
        let (_, body) = split(frame);
        let moved = (self.pane.as_mut())
            .is_some_and(|p| p.qv.scroll(rows as i64, &p.mono, &p.metrics, body));
        Some(if moved {
            Reply::redraw()
        } else {
            Reply::none()
        })
    }

    /// Whether (`x`, `y`) takes the pane's divider.
    pub(crate) fn on_pane_divider(&self, x: i32, y: i32, ctx: &Ctx) -> bool {
        let shown = self.shell_layout(ctx).and_then(|sl| sl.preview);
        self.pane_drag.is_some()
            || (self.popup.is_none() && shown.is_some_and(|f| grip(f).contains(x, y)))
    }
}

#[cfg(test)]
#[path = "previewpane_tests.rs"]
mod tests;
