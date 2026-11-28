//! The details pane along a shell window's foot: a 48 px icon of the selection or folder, its
//! name over a second line (its type, or how many items), then the host's named facts in
//! columns, each a name right-aligned to its column's widest and the value after it. A column
//! holds as many facts as the pane has lines; a column that would run past the pane's right
//! edge is left out with the ones after it.

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::TextUnits;

/// The pane's height, its icon's edge and inset, the gap after the icon, a line's pitch, the
/// air above the first line and below the last, the gap between columns and between a fact's
/// name and its value, and the widest the first column grows, all at 100% text.
pub const HEIGHT: i32 = 56;
pub const ICON: i32 = 48;
const ICON_X: i32 = 14;
const TEXT_GAP: i32 = 14;
const LINE_H: i32 = 15;
const PAD_Y: i32 = 5;
const COLUMN_GAP: i32 = 28;
const NAME_GAP: i32 = 6;
const TITLE_MAX_W: i32 = 260;

/// What the pane shows: the host words every line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub icon: Option<Canvas>,
    pub title: String,
    /// The line under the title; empty for none.
    pub subtitle: String,
    /// (name, value) pairs, the name with its own colon.
    pub rows: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DetailsPane {
    pub facts: Facts,
}

impl DetailsPane {
    /// Show `facts`; true when that changed what draws.
    pub fn set(&mut self, facts: Facts) -> bool {
        let changed = self.facts != facts;
        self.facts = facts;
        changed
    }
}

/// The pane's height at a text size: its icon grows with the text and keeps its air.
pub fn height(u: TextUnits) -> i32 {
    u.len(HEIGHT).max(u.icon(ICON) + u.len(HEIGHT - ICON))
}

/// A fact's name and value boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: Rect,
    pub value: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub icon: Rect,
    pub title: Rect,
    pub subtitle: Option<Rect>,
    /// The facts shown, in order: the first [`Layout::rows`]`.len()` of the host's.
    pub rows: Vec<Row>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Icon,
    Title,
    Row(usize),
    Pane,
    Nothing,
}

pub fn layout(
    dp: &DetailsPane,
    text: &TextRenderer,
    px: u32,
    r: Rect,
    cm: &ControlMetrics,
) -> Layout {
    let u = cm.text;
    let edge = u.icon(ICON);
    let icon = Rect::new(r.x + u.len(ICON_X), r.y + (r.height - edge) / 2, edge, edge);
    let line = u.len(LINE_H);
    let per_col = ((r.height - 2 * u.len(PAD_Y)) / line).max(1) as usize;
    let top = r.y + (r.height - per_col as i32 * line) / 2;
    let x = icon.right() + u.len(TEXT_GAP);
    let right = r.right() - u.len(ICON_X);
    let f = &dp.facts;
    let measure = |s: &str| text.measure(s, false, px);
    let first_w = measure(&f.title)
        .max(measure(&f.subtitle))
        .min(u.len(TITLE_MAX_W))
        .min((right - x).max(0));
    let title = Rect::new(x, top, first_w, line);
    let subtitle = (!f.subtitle.is_empty()).then(|| Rect::new(x, top + line, first_w, line));
    let mut rows = Vec::new();
    let mut col_x = x + first_w + u.len(COLUMN_GAP);
    for chunk in f.rows.chunks(per_col) {
        let name_w = chunk.iter().map(|(n, _)| measure(n)).max().unwrap_or(0);
        let value_w = chunk.iter().map(|(_, v)| measure(v)).max().unwrap_or(0);
        let value_x = col_x + name_w + u.len(NAME_GAP);
        if value_x + value_w > right {
            break;
        }
        for (k, (n, _)) in chunk.iter().enumerate() {
            let y = top + k as i32 * line;
            let w = measure(n);
            rows.push(Row {
                name: Rect::new(col_x + name_w - w, y, w, line),
                value: Rect::new(value_x, y, value_w, line),
            });
        }
        col_x = value_x + value_w + u.len(COLUMN_GAP);
    }
    Layout {
        frame: r,
        icon,
        title,
        subtitle,
        rows,
    }
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Hit {
    let on = |r: &Rect| r.contains(x, y);
    if !on(&lay.frame) {
        Hit::Nothing
    } else if on(&lay.icon) {
        Hit::Icon
    } else if on(&lay.title) || lay.subtitle.as_ref().is_some_and(on) {
        Hit::Title
    } else if let Some(i) = (lay.rows.iter()).position(|r| on(&r.name) || on(&r.value)) {
        Hit::Row(i)
    } else {
        Hit::Pane
    }
}

/// The backdrop (the look's, else the face colour under a shadow line), the icon centred in its
/// box, the title in window text, the subtitle and names in the look's label ink (grey text
/// without one) and the values in its value ink (window text without one).
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    dp: &DetailsPane,
    lay: &Layout,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    let panes = p.look.style().and_then(|st| st.panes);
    match panes {
        Some(pl) => (pl.details)(c, frame, s * c.scale()),
        None => {
            c.fill_rect(frame, cs.button_face);
            c.fill_rect(
                Rect::new(frame.x, frame.y, frame.width, s),
                cs.button_shadow,
            );
        }
    }
    let label = panes.map_or(cs.gray_text, |pl| pl.details_label);
    let value = panes.map_or(cs.window_text, |pl| pl.details_value);
    let f = &dp.facts;
    c.with_clip(frame, |c| {
        if let Some(ic) = &f.icon {
            let r = lay.icon.scaled(s);
            let k = ic.scale().max(1);
            let (w, h) = (ic.width as i32 / k, ic.height as i32 / k);
            c.blit_over(r.x + (r.width - w) / 2, r.y + (r.height - h) / 2, ic);
        }
        // Text set at a scaled size can run a px or two past its 100% measure: each box takes
        // the gap after it (before it, for a name ending at its column's edge) as slack.
        let slack = NAME_GAP * s;
        let mut put = |r: Rect, line: &str, ink, align| {
            let r = r.scaled(s);
            let r = match align {
                Align::Right => Rect::new(r.x - slack, r.y, r.width + slack, r.height),
                _ => Rect::new(r.x, r.y, r.width + slack, r.height),
            };
            let shown = text.ellipsize(line, r.width, false, px);
            text.draw(c, r, &shown, false, ink, align, px);
        };
        put(lay.title, &f.title, cs.window_text, Align::Left);
        if let Some(r) = lay.subtitle {
            put(r, &f.subtitle, label, Align::Left);
        }
        for (row, (name, v)) in lay.rows.iter().zip(&f.rows) {
            put(row.name, name, label, Align::Right);
            put(row.value, v, value, Align::Left);
        }
    });
}

/// The pane, named, holding the title, the subtitle and each fact shown as text.
pub fn access(dp: &DetailsPane, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let pane = tree.push(
        parent,
        "details pane",
        access::named(Role::Pane, lay.frame, &tr!("detailspane-name")),
    );
    let f = &dp.facts;
    tree.push(
        pane,
        "title",
        access::static_text(lay.title, &f.title, true),
    );
    if let Some(r) = lay.subtitle {
        tree.push(pane, "subtitle", access::static_text(r, &f.subtitle, true));
    }
    for (i, (row, (name, value))) in lay.rows.iter().zip(&f.rows).enumerate() {
        let both = row.name.union(&row.value);
        let line = tr!(
            "detailspane-fact",
            name = name.as_str(),
            value = value.as_str()
        );
        tree.push(pane, ("fact", i), access::static_text(both, &line, true));
    }
    pane
}

#[cfg(test)]
mod tests;
