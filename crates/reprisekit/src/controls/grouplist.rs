//! The grouped list of the Open With chooser: headers with indented entries, 18 px rows. It
//! behaves as a listview: a header takes the selection like any row, and the highlight covers the
//! label only.

use crate::classic::widgets::{
    BarHit, ScrollHit, ScrollParts, draw_focus_ring_screen, scrollbar_hit, scrollbar_parts,
    scrollbar_pos_at,
};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, draw_scrollbar};
use crate::raster::{Canvas, Rect};
use crate::selection::{step_index, type_ahead};
use crate::text::{Align, TextRenderer};

pub const ROW_H: i32 = 18;
const GROUP_ICON_X: i32 = 5;
const GROUP_TEXT_X: i32 = 26;
const ITEM_ICON_X: i32 = 25;
const ITEM_TEXT_X: i32 = 45;
/// The highlight starts this far before the pen and runs this far past the label's advance.
const SEL_PAD: (i32, i32) = (2, 7);

/// A row: a group header, or an entry under the header above it carrying the host's `data`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row<T> {
    Group(String),
    Item { name: String, data: T },
}

impl<T> Row<T> {
    pub fn label(&self) -> &str {
        match self {
            Row::Group(label) => label,
            Row::Item { name, .. } => name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupList<T> {
    pub rows: Vec<Row<T>>,
    /// The highlighted row: a header can hold it as well as an entry.
    pub selected: Option<usize>,
    /// First row in view.
    pub top: usize,
    /// Per row, how many px at its right end the label stays clear of, for what the host draws
    /// there (the history's times and its groups' Clear buttons); empty for none.
    pub trail: Vec<i32>,
}

impl<T> GroupList<T> {
    /// The rows with the first entry selected, if there is one.
    pub fn new(rows: Vec<Row<T>>) -> GroupList<T> {
        let selected = rows.iter().position(|r| matches!(r, Row::Item { .. }));
        GroupList {
            rows,
            selected,
            top: 0,
            trail: Vec::new(),
        }
    }

    /// The selected entry's data; None on a header or with nothing selected.
    pub fn chosen(&self) -> Option<&T> {
        match self.selected.and_then(|i| self.rows.get(i)) {
            Some(Row::Item { data, .. }) => Some(data),
            _ => None,
        }
    }

    /// Select row `i` and scroll it into a list of `visible` rows, bringing a header just above it
    /// into view too.
    pub fn select(&mut self, i: usize, visible: usize) {
        if i >= self.rows.len() {
            return;
        }
        self.selected = Some(i);
        let above = if i > 0 && matches!(self.rows[i - 1], Row::Group(_)) {
            i - 1
        } else {
            i
        };
        if above < self.top {
            self.top = above;
        } else if i >= self.top + visible {
            self.top = i + 1 - visible;
        }
    }

    /// Up / Down / Home / End / PageUp / PageDown: move the highlight `delta` rows, clamped at the
    /// list's ends.
    pub fn step(&mut self, delta: i64, visible: usize) -> bool {
        let Some(next) = step_index(self.selected, delta, self.rows.len()) else {
            return false;
        };
        if self.selected == Some(next) {
            return false;
        }
        self.select(next, visible);
        true
    }

    /// A letter typed on the list: type-ahead over every row, the headers included ("o" lands on
    /// "Other Programs:").
    pub fn jump(&mut self, ch: char, visible: usize) -> bool {
        match type_ahead(self.rows.iter().map(Row::label), self.selected, ch) {
            Some(k) if self.selected != Some(k) => {
                self.select(k, visible);
                true
            }
            _ => false,
        }
    }

    /// Scroll by whole rows. Returns whether the view moved.
    pub fn scroll(&mut self, rows: i32, visible: usize) -> bool {
        let max_top = self.rows.len().saturating_sub(visible);
        let top = (self.top as i32 + rows).clamp(0, max_top as i32) as usize;
        if top == self.top {
            return false;
        }
        self.top = top;
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    /// Inside the sunken edge, less the scrollbar.
    pub inner: Rect,
    /// Rows fully in view; the strip under the last one stays empty.
    pub visible: usize,
    pub scrollbar: Option<Rect>,
    /// The look's shortest scrollbar thumb, which hit and drag lay the bar out with as `draw` does.
    pub thumb_min: i32,
}

pub fn layout<T>(list: &GroupList<T>, frame: Rect, cm: &ControlMetrics) -> Layout {
    let mut inner = frame.inset(2);
    let visible = (inner.height / ROW_H).max(1) as usize;
    let scrollbar = (list.rows.len() > visible).then(|| {
        let sb = Rect::new(inner.right() - cm.scroll, inner.y, cm.scroll, inner.height);
        inner.width -= cm.scroll;
        sb
    });
    Layout {
        frame,
        inner,
        visible,
        scrollbar,
        thumb_min: cm.thumb_min,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Thumb(i32),
}

fn scroll_parts<T>(list: &GroupList<T>, lay: &Layout, sb: Rect) -> ScrollParts {
    scrollbar_parts(
        sb,
        1,
        list.top,
        lay.visible,
        list.rows.len(),
        false,
        lay.thumb_min,
    )
}

/// Row `i`'s box while it is in view, in the layout's coordinates: the list's inside across, one
/// row tall.
pub fn row_rect<T>(list: &GroupList<T>, lay: &Layout, i: usize) -> Option<Rect> {
    let k = i
        .checked_sub(list.top)
        .filter(|&k| k < lay.visible && i < list.rows.len())?;
    let y = lay.inner.y + k as i32 * ROW_H;
    Some(Rect::new(lay.inner.x, y, lay.inner.width, ROW_H))
}

/// What a point lands on: a row in view, or a part of the scrollbar.
pub fn hit<T>(list: &GroupList<T>, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
        let p = scroll_parts(list, lay, sb);
        return match scrollbar_hit(&p, x, y, false)? {
            ScrollHit::Back => Some(Hit::ScrollUp),
            ScrollHit::Forward => Some(Hit::ScrollDown),
            ScrollHit::PageBack => Some(Hit::PageUp),
            ScrollHit::PageForward => Some(Hit::PageDown),
            ScrollHit::Thumb => Some(Hit::Thumb(y - p.thumb.map_or(0, |t| t.y))),
        };
    }
    if !lay.inner.contains(x, y) {
        return None;
    }
    let k = ((y - lay.inner.y) / ROW_H) as usize;
    let row = list.top + k;
    (k < lay.visible && row < list.rows.len()).then_some(Hit::Row(row))
}

impl Hit {
    /// The part of the scrollbar this is on; `None` for a row.
    pub fn bar(self) -> Option<BarHit> {
        match self {
            Hit::ScrollUp => Some(BarHit::Back),
            Hit::ScrollDown => Some(BarHit::Forward),
            Hit::PageUp => Some(BarHit::PageBack),
            Hit::PageDown => Some(BarHit::PageForward),
            Hit::Thumb(grab) => Some(BarHit::Thumb(grab)),
            Hit::Row(_) => None,
        }
    }
}

/// The top row a thumb grabbed `grab` px below its own top scrolls to when the pointer sits at `y`.
pub fn top_at<T>(list: &GroupList<T>, lay: &Layout, y: i32, grab: i32) -> usize {
    match lay.scrollbar {
        Some(sb) => {
            let p = scroll_parts(list, lay, sb);
            scrollbar_pos_at(&p, y - grab, lay.visible, list.rows.len(), false)
        }
        None => list.top,
    }
}

/// How the pick shows: whether the list has the focus, and the keyboard cues.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// Focus elsewhere or window inactive: the pick filled with the face colour, label in button
    /// text.
    Away,
    Focused,
    Cued,
}

/// Draw the list; `icons` is parallel to the rows (16 px, or None for none).
#[allow(clippy::too_many_arguments)]
pub fn draw<T>(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    list: &GroupList<T>,
    lay: &Layout,
    icons: &[Option<&Canvas>],
    shown: Shown,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.window);
    draw_field_frame(c, p, frame, cs.window);
    if let Some(sb) = lay.scrollbar {
        draw_scrollbar(
            c,
            p,
            sb.scaled(s),
            list.top,
            lay.visible,
            list.rows.len(),
            None,
        );
    }
    let inner = lay.inner.scaled(s);
    let rh = ROW_H * s;
    let last = list.rows.len().min(list.top + lay.visible);
    for (k, i) in (list.top..last).enumerate() {
        let y = inner.y + k as i32 * rh;
        let (icon_x, text_x) = match &list.rows[i] {
            Row::Group(_) => (GROUP_ICON_X, GROUP_TEXT_X),
            Row::Item { .. } => (ITEM_ICON_X, ITEM_TEXT_X),
        };
        if let Some(Some(ic)) = icons.get(i) {
            c.blit_over(frame.x + icon_x * s, y + (rh - ic.height as i32) / 2, ic);
        }
        let tx = frame.x + text_x * s;
        let trail = list.trail.get(i).copied().unwrap_or(0) * s;
        let tr = Rect::new(tx, y, (inner.right() - trail - tx).max(0), rh);
        let label = text.ellipsize(list.rows[i].label(), tr.width, false, px);
        let fg = if list.selected == Some(i) {
            let tw = text.measure(&label, false, px);
            let hi = Rect::new(tx - SEL_PAD.0 * s, y, tw + SEL_PAD.1 * s, rh);
            if shown == Shown::Away {
                c.fill_rect(hi, cs.button_face);
                cs.button_text
            } else {
                c.fill_rect(hi, cs.hilight);
                if shown == Shown::Cued {
                    // Black where x + y counts even from the list's inside, the inverted highlight
                    // between.
                    let o = (-lay.inner.x, -lay.inner.y);
                    draw_focus_ring_screen(c, hi, s, cs.hilight.inverted(), o);
                    draw_focus_ring_screen(c, hi, s, cs.button_text, (o.0 + 1, o.1));
                }
                cs.hilight_text
            }
        } else {
            cs.button_text
        };
        text.draw(c, tr, &label, false, fg, Align::Left, px);
    }
}

/// The list's `access` part: a list box with an option per row, headers included (they take the
/// selection too), the selected one marked. Rows scrolled out of view have no bounds. Returns the
/// node focused when the list is: the selected row, else the list.
pub fn access<T>(
    list: &GroupList<T>,
    lay: &Layout,
    tree: &mut crate::access::Tree,
    parent: crate::access::accesskit::NodeId,
    key: impl std::hash::Hash,
) -> crate::access::accesskit::NodeId {
    use crate::access::{self, accesskit::Node, accesskit::Role};
    let mut n = access::node(Role::ListBox, lay.frame);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for (i, row) in list.rows.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(row.label());
        if (list.top..list.top + lay.visible).contains(&i) {
            let y = lay.inner.y + (i - list.top) as i32 * ROW_H;
            let r = Rect::new(lay.inner.x, y, lay.inner.width, ROW_H);
            o.set_bounds(access::bounds(r));
        }
        o.set_selected(list.selected == Some(i));
        let oid = tree.push(id, i, o);
        if list.selected == Some(i) {
            focus = oid;
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn the_selected_row_has_the_focus() {
        let l = list(3);
        let lay = layout(&l, Rect::new(0, 0, 200, 100), &ControlMetrics::CLASSIC);
        let mut t = crate::access::Tree::window("", (200, 100));
        let focus = access(&l, &lay, &mut t, crate::access::ROOT, "list");
        let n = t.get(focus).unwrap();
        assert_eq!(n.is_selected(), Some(true));
        assert_eq!(n.label(), Some(l.rows[l.selected.unwrap()].label()));
    }

    fn list(n: usize) -> GroupList<usize> {
        let mut rows = vec![
            Row::Group("Recommended Programs:".into()),
            Row::Item {
                name: "Micro".into(),
                data: 1,
            },
            Row::Item {
                name: "Vim".into(),
                data: 2,
            },
            Row::Group("Other Programs:".into()),
        ];
        rows.extend((0..n).map(|i| Row::Item {
            name: format!("Program {i:02}"),
            data: 10 + i,
        }));
        GroupList::new(rows)
    }

    #[test]
    fn eleven_rows_fill_the_list_and_the_strip_under_them_is_no_row() {
        let frame = Rect::new(27, 102, 338, 210);
        let l = layout(&list(12), frame, &ControlMetrics::CLASSIC);
        assert_eq!(l.visible, 11);
        assert_eq!(l.scrollbar, Some(Rect::new(347, 104, 16, 206)));
        assert_eq!(l.inner, Rect::new(29, 104, 318, 206));
        assert_eq!(hit(&list(12), &l, 100, 104 + 3 * 18 + 5), Some(Hit::Row(3)));
        assert_eq!(hit(&list(12), &l, 100, 104 + 10 * 18), Some(Hit::Row(10)));
        assert_eq!(hit(&list(12), &l, 100, 104 + 11 * 18 + 2), None);
        assert_eq!(hit(&list(12), &l, 355, 106), Some(Hit::ScrollUp));
        assert!(
            layout(&list(3), frame, &ControlMetrics::CLASSIC)
                .scrollbar
                .is_none()
        );
        assert_eq!(hit(&list(3), &l, 100, 104 + 8 * 18), None, "past the rows");
    }

    #[test]
    fn the_highlight_walks_every_row_and_stays_in_view() {
        let mut g = list(12);
        assert_eq!(g.selected, Some(1));
        assert_eq!(g.chosen(), Some(&1));
        assert!(g.step(-1, 11));
        assert_eq!((g.selected, g.chosen()), (Some(0), None));
        assert!(!g.step(-1, 11));
        for _ in 0..20 {
            g.step(1, 11);
        }
        assert_eq!((g.selected, g.top), (Some(15), 5));
        assert!(!g.step(1, 11));
        g.select(4, 11);
        assert_eq!(g.top, 3, "the header above comes into view");
        assert!(g.jump('o', 11));
        assert_eq!(g.selected, Some(3));
        assert!(g.jump('p', 11));
        assert_eq!(g.selected, Some(4));
        assert!(!g.jump('z', 11));
        assert!(g.scroll(100, 11));
        assert_eq!(g.top, 5);
        assert!(!g.scroll(1, 11));
        let empty: GroupList<usize> = GroupList::new(Vec::new());
        assert_eq!(empty.selected, None);
        assert!(!empty.clone().step(1, 11));
    }

    #[test]
    fn a_page_step_moves_a_list_height_and_the_ends_clamp() {
        let mut g = list(12);
        assert!(g.step(11, 11));
        assert_eq!((g.selected, g.top), (Some(12), 2));
        assert!(g.step(11, 11));
        assert_eq!((g.selected, g.top), (Some(15), 5), "the last row");
        assert!(g.step(-11, 11));
        assert_eq!((g.selected, g.top), (Some(4), 3), "the header above shows");
        assert!(g.step(i64::MIN / 2, 11));
        assert_eq!((g.selected, g.top), (Some(0), 0), "the first row");
        assert!(g.step(i64::MAX / 2, 11));
        assert_eq!(g.selected, Some(15));
    }

    #[test]
    fn the_highlight_covers_the_label_the_full_row_tall() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let g = list(2);
        let frame = Rect::new(0, 0, 200, 120);
        let l = layout(&g, frame, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(200, 120);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &g,
            &l,
            &[],
            Shown::Focused,
            px,
        );
        let hi = cs.hilight.argb();
        let cols: Vec<i32> = (0..200)
            .filter(|&x| (0..120).any(|y| c.get(x, y) == hi))
            .collect();
        let rows: Vec<i32> = (0..120)
            .filter(|&y| (0..200).any(|x| c.get(x, y) == hi))
            .collect();
        let w = text.measure("Micro", false, px);
        assert_eq!((cols[0], *cols.last().unwrap()), (43, 43 + w + 7 - 1));
        assert_eq!((rows[0], *rows.last().unwrap()), (20, 37));
        assert_eq!(c.get(42, 28), cs.window.argb());
        assert_eq!(c.get(0, 50), cs.button_shadow.argb(), "the sunken edge");
    }

    fn shot(shown: Shown) -> (Canvas, Theme, i32) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let g = list(2);
        let l = layout(&g, Rect::new(0, 0, 200, 120), &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(200, 120);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &g,
            &l,
            &[],
            shown,
            px,
        );
        let w = text.measure("Micro", false, px);
        (c, theme, 43 + w + 7 - 1)
    }

    #[test]
    fn a_cued_focused_pick_is_ringed_on_its_highlight_edge() {
        let (c, theme, right) = shot(Shown::Cued);
        let cs = &theme.colors;
        let (black, inv) = (cs.button_text.argb(), cs.hilight.inverted().argb());
        assert_eq!(c.get(43, 20), inv, "top-left corner");
        assert_eq!(c.get(44, 20), black);
        assert_eq!(c.get(43, 21), black, "left edge");
        assert_eq!(c.get(43, 37), black, "bottom-left corner");
        let edge: Vec<u32> = (21..37).map(|y| c.get(right, y)).collect();
        assert!(
            edge.windows(2)
                .all(|w| w[0] != w[1] && [black, inv].contains(&w[0])),
            "the right edge alternates the two inks"
        );
        assert_eq!(c.get(44, 21), cs.hilight.argb(), "the highlight inside");
        let (plain, _, _) = shot(Shown::Focused);
        assert_eq!(plain.get(44, 20), cs.hilight.argb(), "no ring without cues");
    }

    #[test]
    fn a_pick_without_the_focus_is_face_filled() {
        let (c, theme, right) = shot(Shown::Away);
        let cs = &theme.colors;
        let row = |x0: i32, x1: i32, y0: i32, y1: i32, color: u32| {
            (x0..x1).any(|x| (y0..y1).any(|y| c.get(x, y) == color))
        };
        assert!(!row(0, 200, 0, 120, cs.hilight.argb()), "no highlight");
        assert_eq!(c.get(43, 20), cs.button_face.argb());
        assert_eq!(c.get(right, 37), cs.button_face.argb());
        assert!(row(45, right, 20, 38, cs.button_text.argb()), "black label");
        assert!(!row(45, right, 20, 38, cs.hilight_text.argb()));
    }
}
