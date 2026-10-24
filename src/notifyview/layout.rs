//! Laying out a balloon or sheet and the glass under it.

use reprisekit::controls::notifysheet::{
    SheetContent, SheetLayout, open_layout, sheet_hit, sheet_layout,
};
use reprisekit::controls::tooltip::{
    NotifyHit, NotifyLayout, NotifyParts, NotifyReply, notify_hit, notify_layout,
};
use reprisekit::glass::notify::WELL_CORNER;
use reprisekit::icon::IconSource;
use reprisekit::style::{NotifyLook, SheetShape};
use reprisekit::taskbar::Edge;

use crate::glass::{Pane, PaneShape};
use crate::raster::Rect;
use crate::text::TextRenderer;
use crate::theme::{Color, Theme};

use super::stack::Item;
use super::{TINT_PX, button_labels};

/// What a balloon's buffer shows, but for a reply row, which always redraws.
#[derive(PartialEq)]
pub(super) struct Painted {
    pub(super) drawn: Drawn,
    pub(super) open: f64,
    pub(super) pushed: Option<NotifyHit>,
    pub(super) hot: Option<NotifyHit>,
}

#[derive(Clone, PartialEq)]
pub(super) enum Drawn {
    Balloon(NotifyLayout),
    Sheet(SheetLayout),
}

impl Drawn {
    /// The canvas, device px.
    pub(super) fn size(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(l) => (l.width, l.height),
            Drawn::Sheet(l) => (l.width, l.height),
        }
    }

    /// `None` off a sheet: on its shadow.
    pub(super) fn hit(&self, x: i32, y: i32) -> Option<NotifyHit> {
        match self {
            Drawn::Balloon(l) => Some(notify_hit(l, x, y)),
            Drawn::Sheet(l) => sheet_hit(l, x, y),
        }
    }

    pub(super) fn parts(&self) -> NotifyParts {
        match self {
            Drawn::Balloon(l) => l.parts(),
            Drawn::Sheet(l) => l.parts.clone(),
        }
    }

    /// Where the tree sits in the canvas, device px: a sheet's tree is at its footprint.
    pub(super) fn origin(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(_) => (0, 0),
            Drawn::Sheet(l) => (l.footprint.x, l.footprint.y),
        }
    }

    /// What the stack places, device px: a sheet's footprint, a balloon's canvas.
    pub(super) fn footprint(&self) -> (i32, i32) {
        match self {
            Drawn::Balloon(l) => (l.width, l.height),
            Drawn::Sheet(l) => (l.footprint.width, l.footprint.height),
        }
    }
}

/// A sheet when the look has them, else a balloon; `when` is the sheet's app line time.
#[expect(clippy::too_many_arguments)]
pub(super) fn lay_out(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    expanded: bool,
    when: &str,
) -> Drawn {
    let labels;
    let burst;
    let (app, title, body, actions, reply) = match item {
        Item::Note(n) => {
            labels = button_labels(n);
            let reply = n.reply.as_ref().filter(|_| replying).map(|r| NotifyReply {
                placeholder: &r.placeholder,
                send: &r.send,
            });
            (
                n.app.as_str(),
                n.summary.as_str(),
                n.body.as_str(),
                &labels[..],
                reply,
            )
        }
        Item::Burst(b) => {
            let lines: Vec<&str> = b.lines.iter().map(String::as_str).collect();
            burst = (b.title(), lines.join("\n"));
            (
                b.app.as_str(),
                burst.0.as_str(),
                burst.1.as_str(),
                &[][..],
                None,
            )
        }
    };
    let content = SheetContent {
        app,
        when,
        title,
        body,
        actions,
        reply,
        has_picture: !item.image().is_empty(),
        has_app_icon: item.has_app_icon(),
    };
    match sheet_layout(theme, text, shape, &content, expanded || replying) {
        Some(lay) => Drawn::Sheet(lay),
        None => {
            Drawn::Balloon(notify_layout(theme, text, title, body, actions, reply).on_edge(edge))
        }
    }
}

/// A Droplet's collapsed and expanded layouts; `None` for any other shape or a look without
/// sheets.
pub(super) fn droplet_ends(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    when: &str,
) -> Option<[SheetLayout; 2]> {
    if shape != SheetShape::Droplet {
        return None;
    }
    let lay =
        |replying, expanded| lay_out(theme, text, item, replying, edge, shape, expanded, when);
    match (lay(false, false), lay(replying, true)) {
        (Drawn::Sheet(shut), Drawn::Sheet(open)) => Some([shut, open]),
        _ => None,
    }
}

/// `item` laid out `t` (0..1) of the way open. A Droplet sheet also returns the collapsed and
/// expanded layouts it opens between.
#[expect(clippy::too_many_arguments)]
pub(super) fn lay_out_open(
    theme: &Theme,
    text: &TextRenderer,
    item: &Item,
    replying: bool,
    edge: Edge,
    shape: SheetShape,
    when: &str,
    t: f64,
) -> (Drawn, Option<[SheetLayout; 2]>) {
    let Some([shut, open]) = droplet_ends(theme, text, item, replying, edge, shape, when) else {
        return (
            lay_out(theme, text, item, replying, edge, shape, false, when),
            None,
        );
    };
    let lay = match t {
        t if t <= 0.0 => shut.clone(),
        t if t >= 1.0 => open.clone(),
        t => open_layout(&shut, &open, t),
    };
    (Drawn::Sheet(lay), Some([shut, open]))
}

/// The glass under a sheet, logical px from the glass tree's origin at the footprint.
pub(super) fn sheet_glass(lay: &SheetLayout, look: &NotifyLook, scale: i32) -> Pane {
    let fp = lay.footprint;
    match lay.shape {
        SheetShape::Lozenge | SheetShape::Droplet => crate::glass::sheet_pane(
            (fp.width / scale, fp.height / scale),
            (look.corner)(lay.shape),
        ),
        SheetShape::Halo => {
            let logical = |r: Rect| {
                Rect::new(
                    (r.x - fp.x) / scale,
                    (r.y - fp.y) / scale,
                    r.width / scale,
                    r.height / scale,
                )
            };
            let (icon, sheet) = (logical(lay.icon), logical(lay.sheet));
            let well = Rect::new(icon.x, icon.y, icon.width, sheet.y - icon.y);
            Pane {
                shapes: vec![
                    PaneShape {
                        rect: well,
                        corner: &WELL_CORNER,
                        square_bottom: true,
                        square_top: false,
                    },
                    PaneShape {
                        rect: sheet,
                        corner: (look.corner)(lay.shape),
                        square_bottom: false,
                        square_top: false,
                    },
                ],
                holes: vec![],
            }
        }
    }
}

/// `None` for a look without sheets, `Some(None)` for a grey icon or none.
pub(super) fn sheet_tint(
    theme: &Theme,
    app_icon: &[IconSource],
    scale: i32,
) -> Option<Option<Color>> {
    let look = theme.look.style().and_then(|st| st.notify)?;
    Some(crate::tray::closest(app_icon, TINT_PX * scale).and_then(|icon| (look.colour)(&icon)))
}
