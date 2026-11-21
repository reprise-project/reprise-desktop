//! The single-selection list box with an icon per row, an optional dimmed detail, per-row check boxes and the
//! vertical scrollbar once the rows outgrow the box. A list given icons grows its 16 px icon column
//! with the text size ([`icon_px`]); its labels keep their 100% gap past the icon.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BarHit, ScrollHit, ScrollParts};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::RowScroll;
use crate::icon::ICON_SIZE;
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_field_frame, draw_list_check, draw_scrollbar};
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::selection::{scroll_into_view, step_index, type_ahead};
use crate::text::{Align, TextRenderer};
use accesskit::{Node, NodeId, Role};

pub const ROW_H: i32 = 17;
const ICON_X: i32 = 3;
pub const TEXT_X: i32 = 22;
pub(crate) const DETAIL_GAP: i32 = 8;
const CHECK_Y: i32 = 2;
const CHECK_X: i32 = 6;
const CHECK_TEXT_X: i32 = 25;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListBox {
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub top: usize,
    /// Parallel to `items`: rows drawn in `gray_text`, still selectable
    /// (a device that is switched off). A missing entry is not greyed.
    pub greyed: Vec<bool>,
    /// Parallel to `items`: a second, dimmed text after the label (the
    /// launcher's folder of a file row), cut to the row with an ellipsis.
    /// A missing or empty entry draws nothing.
    pub detail: Vec<String>,
    /// Parallel to `items`: each row's check box, before its icon.
    /// Empty for a list with no check boxes.
    pub checks: Vec<bool>,
}

impl ListBox {
    pub fn new(items: Vec<String>, selected: Option<usize>) -> ListBox {
        ListBox {
            items,
            selected,
            top: 0,
            greyed: Vec::new(),
            detail: Vec::new(),
            checks: Vec::new(),
        }
    }

    pub fn toggle(&mut self, i: usize) -> bool {
        match self.checks.get_mut(i) {
            Some(c) => {
                *c = !*c;
                true
            }
            None => false,
        }
    }

    /// Select `i`, scrolling it into a box of `visible` rows. Returns
    /// whether the selection changed.
    pub fn select(&mut self, i: usize, visible: usize) -> bool {
        if i >= self.items.len() {
            return false;
        }
        let changed = self.selected != Some(i);
        self.selected = Some(i);
        scroll_into_view(&mut self.top, i, visible);
        changed
    }

    pub fn step(&mut self, delta: i64, visible: usize) -> bool {
        let cur = Some(self.selected.unwrap_or(0));
        let Some(to) = step_index(cur, delta, self.items.len()) else {
            return false;
        };
        self.select(to, visible)
    }

    pub fn jump(&mut self, ch: char, visible: usize) -> bool {
        match type_ahead(self.items.iter().map(String::as_str), self.selected, ch) {
            Some(k) => self.select(k, visible),
            None => false,
        }
    }

    /// Scroll row `i` into a box of `visible` rows without picking it. True when it moved.
    pub fn reveal(&mut self, i: usize, visible: usize) -> bool {
        let top = self.top;
        if i < self.items.len() {
            scroll_into_view(&mut self.top, i, visible);
        }
        self.top != top
    }

    pub fn scroll(&mut self, rows: i32, visible: usize) -> bool {
        let top = self.at(visible).scrolled(rows);
        let moved = top != self.top;
        self.top = top;
        moved
    }

    fn at(&self, visible: usize) -> RowScroll {
        RowScroll {
            top: self.top,
            visible,
            total: self.items.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub inner: Rect,
    pub visible: usize,
    pub scrollbar: Option<Rect>,
    /// The look's shortest scrollbar thumb, which hit and drag lay the
    /// bar out with as `draw` does.
    pub thumb_min: i32,
    pub check: i32,
    pub row_h: i32,
    /// The rows' icon edge: [`icon_px`].
    pub icon: i32,
}

impl Layout {
    /// The rows shown whole: the strip under the last one is empty.
    fn shown(&self) -> Rect {
        let i = self.inner;
        Rect::new(i.x, i.y, i.width, self.visible as i32 * self.row_h)
    }
}

pub fn layout(lb: &ListBox, frame: Rect, cm: &ControlMetrics) -> Layout {
    let mut inner = frame.inset(2);
    let visible = (inner.height / cm.list_row).max(1) as usize;
    let scrollbar = (lb.items.len() > visible).then(|| {
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
        check: cm.check,
        row_h: cm.list_row,
        icon: icon_px(cm),
    }
}

/// A row's icon edge (logical px): 16 px at 100% text, grown with the text and never smaller.
/// Hosts load the rows' icons at it.
pub fn icon_px(cm: &ControlMetrics) -> i32 {
    cm.text.icon(ICON_SIZE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    /// A part of the scrollbar, a thumb with its grab offset.
    Bar(BarHit),
    Check(usize),
}

/// Row `i`'s check box, when it is in view: the look's check box side
/// square, at the row's start.
pub fn check_box(lb: &ListBox, lay: &Layout, i: usize) -> Option<Rect> {
    if i >= lb.checks.len() || i < lb.top || i >= lb.top + lay.visible {
        return None;
    }
    let y = lay.inner.y + (i - lb.top) as i32 * lay.row_h + CHECK_Y + (lay.row_h - ROW_H) / 2;
    Some(Rect::new(lay.inner.x + CHECK_X, y, lay.check, lay.check))
}

pub fn scroll_parts(lb: &ListBox, lay: &Layout) -> Option<ScrollParts> {
    lay.scrollbar
        .map(|sb| lb.at(lay.visible).parts(sb, lay.thumb_min))
}

pub fn hit(lb: &ListBox, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
        return lb
            .at(lay.visible)
            .hit(sb, lay.thumb_min, x, y)
            .map(Hit::Bar);
    }
    if !lay.inner.contains(x, y) {
        return None;
    }
    let row = ((y - lay.inner.y) / lay.row_h) as usize + lb.top;
    if check_box(lb, lay, row).is_some_and(|b| b.contains(x, y)) {
        return Some(Hit::Check(row));
    }
    (row < lb.items.len()).then_some(Hit::Row(row))
}

impl Hit {
    pub fn bar(self) -> Option<BarHit> {
        match self {
            Hit::Bar(b) => Some(b),
            Hit::Row(_) | Hit::Check(_) => None,
        }
    }
}

/// The top row a thumb grabbed `grab` px below its own top scrolls to
/// when the pointer sits at `y`.
pub fn top_at(lb: &ListBox, lay: &Layout, y: i32, grab: i32) -> usize {
    lay.scrollbar.map_or(lb.top, |sb| {
        lb.at(lay.visible).top_at(sb, lay.thumb_min, y, grab)
    })
}

/// A row's icon x: after its check box in a checked list.
fn icon_x(lb: &ListBox) -> i32 {
    if lb.checks.is_empty() {
        ICON_X
    } else {
        CHECK_TEXT_X
    }
}

/// A row's label x: a checked list leaves room for icons only when it is given some, and only
/// a list given icons widens its icon column with the text.
pub fn text_x(lb: &ListBox, lay: &Layout, icons: &[Option<Canvas>]) -> i32 {
    let grown = lay.icon - ICON_SIZE;
    match (lb.checks.is_empty(), icons.is_empty()) {
        (true, true) => TEXT_X,
        (true, false) => TEXT_X + grown,
        (false, true) => CHECK_TEXT_X,
        (false, false) => CHECK_TEXT_X + TEXT_X - ICON_X + grown,
    }
}

/// Draw the box; `icons` is parallel to the items ([`icon_px`] at the scale, or None for
/// no icon), drawn after the check boxes in a checked list. A partly visible last row is clipped.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lb: &ListBox,
    lay: &Layout,
    icons: &[Option<Canvas>],
    bar: Lit<ScrollHit>,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.window);
    draw_field_frame(c, p, frame, cs.window);
    if let Some(sb) = lay.scrollbar {
        draw_scrollbar(c, p, sb.scaled(s), lb.top, lay.visible, lb.items.len(), bar);
    }
    let inner = lay.inner.scaled(s);
    let rh = lay.row_h * s;
    // A taller row (grown text) centres its check box and icon down the row.
    let glyph_dy = (lay.row_h - ROW_H) / 2 * s;
    let icon_dy = (lay.row_h - lay.icon - 1) / 2 * s;
    let slot = lay.icon * s;
    let mut y = inner.y;
    for i in lb.top..lb.items.len() {
        if y >= inner.bottom() {
            break;
        }
        let row = Rect::new(inner.x, y, inner.width, rh).intersect(&inner);
        // A look that boxes its items boxes the selected row, 1 px in from
        // either side, under its icon and its text, which stay as they are.
        let mark = Mark {
            selected: lb.selected == Some(i),
            ..Mark::default()
        };
        let boxed = c.with_clip(inner, |c| {
            let r = Rect::new(inner.x + s, y, inner.width - 2 * s, rh);
            item::draw_item(c, p, r, mark)
        });
        if let Some(&checked) = lb.checks.get(i) {
            let b = Rect::new(
                inner.x + CHECK_X * s,
                y + CHECK_Y * s + glyph_dy,
                lay.check * s,
                lay.check * s,
            );
            if b.bottom() <= inner.bottom() {
                draw_list_check(c, p, b, checked);
            }
        }
        let icon_x = inner.x + icon_x(lb) * s;
        if let Some(Some(ic)) = icons.get(i) {
            // An icon smaller than the column (not yet loaded at the text size) sits centred in it.
            let icon_x = icon_x + ((slot - ic.width as i32) / 2).max(0);
            let iy = y + icon_dy + ((slot - ic.height as i32) / 2).max(0);
            let clip = Rect::new(icon_x, iy, ic.width as i32, ic.height as i32).intersect(&inner);
            c.blit_over_clipped(icon_x, iy, ic, clip);
        }
        let text_x = text_x(lb, lay, icons);
        let tx = inner.x + text_x * s;
        let tw = text.measure(&lb.items[i], false, px);
        let (fg, bg) = if lb.selected == Some(i) && !boxed {
            (cs.hilight_text, Some(cs.hilight))
        } else if lb.greyed.get(i).copied().unwrap_or(false) && !mark.selected {
            (cs.gray_text, None)
        } else {
            (cs.button_text, None)
        };
        if let Some(bg) = bg {
            c.fill_rect(Rect::new(tx - 2 * s, y, tw + 4 * s, rh).intersect(&row), bg);
        }
        // A partly shown row keeps the baseline of a whole one; only the clip shrinks.
        let tr = Rect::new(tx, y, (inner.right() - tx).max(0), rh);
        text.draw_clipped(c, tr, row, &lb.items[i], false, fg, Align::Left, px);
        if let Some(d) = lb.detail.get(i).filter(|d| !d.is_empty()) {
            let dx = tx + tw + DETAIL_GAP * s;
            let dw = (inner.right() - dx - 2 * s).max(0);
            let shown = text.ellipsize(d, dw, false, px);
            let dr = Rect::new(dx, y, dw, rh);
            text.draw_clipped(c, dr, row, &shown, false, cs.gray_text, Align::Left, px);
        }
        y += rh;
    }
}

/// The list box's `access` part: a list with one option per item, the
/// selected one marked. The list's bounds are the rows shown whole, so a row
/// scrolled out lies outside them ([`access::row_bounds`]), where a reader can
/// ask to scroll it in. Returns
/// the node that has the focus when the list does: the selected row, else
/// the list.
pub fn access(
    lb: &ListBox,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, lay.shown());
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for (i, item) in lb.items.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(item.as_str());
        if let Some(d) = lb.detail.get(i).filter(|d| !d.is_empty()) {
            o.set_description(d.as_str());
        }
        let y = lay.inner.y + (i as i32 - lb.top as i32) * lay.row_h;
        let row = Rect::new(lay.inner.x, y, lay.inner.width, lay.row_h);
        o.set_bounds(access::row_bounds(lay.shown(), row));
        o.set_selected(lb.selected == Some(i));
        if let Some(&checked) = lb.checks.get(i) {
            o.set_toggled(checked.into());
        }
        let oid = tree.push(id, i, o);
        if lb.selected == Some(i) {
            focus = oid;
        }
    }
    focus
}

/// A filter field over a list box, as the launcher and the clipboard
/// history draw them: `edit` in the sunken `field` as a text input, then
/// the list at `lay`, the field read as `name`. Returns the highlighted
/// row's node, else the field's: the arrows move the highlight while the
/// text stays in the field, so a reader follows the row.
#[allow(clippy::too_many_arguments)]
pub fn access_filtered(
    edit: &crate::textedit::TextEdit,
    name: &str,
    field: Rect,
    lb: &ListBox,
    lay: Option<&Layout>,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let inner_w = field.width - 2 * crate::classic::widgets::FIELD_PAD;
    let first = crate::classic::widgets::edit_first_visible(text, edit, inner_w, px, 0);
    let f = crate::classic::widgets::access_edit(
        edit, first, false, field, true, text, px, tree, parent, "field",
    );
    if let Some(n) = tree.get_mut(f) {
        n.set_label(name);
    }
    match lay {
        Some(lay) if lb.selected.is_some() => access(lb, lay, tree, parent, "rows"),
        Some(lay) => {
            access(lb, lay, tree, parent, "rows");
            f
        }
        None => f,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{Color, FontConfig, Theme};

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("Item {i}")).collect()
    }

    #[test]
    fn rows_are_17_px_and_the_bar_takes_the_right_16() {
        let lb = ListBox::new(names(20), Some(0));
        let l = layout(&lb, Rect::new(24, 257, 271, 113), &ControlMetrics::CLASSIC);
        assert_eq!(l.inner, Rect::new(26, 259, 251, 109));
        assert_eq!(l.visible, 6);
        assert_eq!(l.scrollbar, Some(Rect::new(277, 259, 16, 109)));
        assert_eq!(hit(&lb, &l, 100, 259 + 17 * 2 + 3), Some(Hit::Row(2)));
        assert_eq!(hit(&lb, &l, 285, 262), Some(Hit::Bar(BarHit::Back)));
        assert_eq!(hit(&lb, &l, 285, 360), Some(Hit::Bar(BarHit::Forward)));
        assert_eq!(hit(&lb, &l, 285, 280), Some(Hit::Bar(BarHit::Thumb(5))));
        assert_eq!(hit(&lb, &l, 285, 320), Some(Hit::Bar(BarHit::PageForward)));
        assert_eq!(top_at(&lb, &l, 280, 5), 0);
        assert_eq!(top_at(&lb, &l, 334, 5), 14);
        assert_eq!(top_at(&lb, &l, 999, 5), 14);
        let mut end = ListBox::new(names(20), Some(0));
        end.top = 14;
        assert_eq!(hit(&end, &l, 285, 280), Some(Hit::Bar(BarHit::PageBack)));
        let short = ListBox::new(names(3), None);
        assert!(
            layout(&short, Rect::new(0, 0, 100, 100), &ControlMetrics::CLASSIC)
                .scrollbar
                .is_none()
        );
    }

    #[test]
    fn selection_scrolls_into_view() {
        let mut lb = ListBox::new(names(20), Some(0));
        assert!(lb.step(1, 6));
        assert_eq!((lb.selected, lb.top), (Some(1), 0));
        assert!(lb.step(10, 6));
        assert_eq!((lb.selected, lb.top), (Some(11), 6));
        assert!(lb.step(-100, 6));
        assert_eq!((lb.selected, lb.top), (Some(0), 0));
        assert!(lb.scroll(3, 6));
        assert_eq!(lb.top, 3);
        assert!(!lb.scroll(100, 6) || lb.top == 14);
    }

    /// `w` from "Other Programs:" skips the Write above it and scrolls
    /// Web Browser into view; a second `w` wraps round to Write.
    #[test]
    fn a_letter_jumps_to_the_next_row_starting_with_it() {
        let rows = [
            "Write",
            "Other Programs:",
            "Image Viewer",
            "Notepad",
            "Draw",
            "Web Browser",
        ];

        let mut lb = ListBox::new(rows.iter().map(|s| s.to_string()).collect(), Some(1));
        assert!(lb.jump('w', 3));
        assert_eq!((lb.selected, lb.top), (Some(5), 3), "scrolled into view");
        assert!(!lb.jump('z', 3));
        assert!(lb.jump('w', 3));
        assert_eq!(lb.selected, Some(0));
    }

    #[test]
    fn draws_the_selected_label_on_the_highlight() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let lb = ListBox::new(vec!["(None)".into(), "Dunes".into()], Some(0));
        let frame = Rect::new(24, 257, 271, 113);
        let l = layout(&lb, frame, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(320, 400);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &lb,
            &l,
            &[None, None],
            None,
            11,
        );
        assert_eq!(c.get(24, 257), cs.button_shadow.argb());
        assert_eq!(c.get(26, 259), cs.window.argb());
        assert_eq!(
            c.get(46, 259),
            cs.hilight.argb(),
            "highlight from text x - 2"
        );
        assert_eq!(c.get(46, 275), cs.hilight.argb(), "17 rows tall");
        assert_eq!(c.get(46, 276), cs.window.argb());
        assert_eq!(c.get(45, 265), cs.window.argb());
    }

    #[test]
    fn a_partly_shown_last_row_shows_the_top_of_the_whole_row() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let rows = ["Ridges (blue)", "Terraces (spring)", "Terraces (dusk)"];
        let mut lb = ListBox::new(rows.map(String::from).to_vec(), None);
        lb.detail = vec!["Reprise".into(); 3];
        let paint = classic_paint(cs, 1);
        let drawn = |height: i32| {
            let l = layout(&lb, Rect::new(0, 0, 200, height), &ControlMetrics::CLASSIC);
            let mut c = Canvas::new(200, 60);
            draw(&mut c, &text, &paint, &lb, &l, &[], None, 11);
            (c, l.inner)
        };
        let (whole, _) = drawn(4 + 2 * 17);
        let (cut, inner) = drawn(4 + 17 + 9);
        let ink = |c: &Canvas| {
            (inner.x..inner.right())
                .flat_map(|x| (inner.y + 17..inner.bottom()).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) != cs.window.argb())
                .collect::<Vec<_>>()
        };
        assert!(
            !ink(&whole).is_empty(),
            "the second row has ink in its top 9 px"
        );
        assert_eq!(
            ink(&cut),
            ink(&whole),
            "the cut row shows the whole row's top 9 px"
        );
    }

    /// A row's check box takes the icon's place, 3 px in and 2 px down,
    /// and draws ticked or clear.
    #[test]
    fn checks_draw_flat_before_the_labels() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut lb = ListBox::new(vec!["Volume Control".into(), "Firefox".into()], None);
        lb.checks = vec![true, false];
        let l = layout(&lb, Rect::new(0, 0, 150, 60), &ControlMetrics::CLASSIC);
        assert_eq!(check_box(&lb, &l, 0), Some(Rect::new(8, 4, 13, 13)));
        assert_eq!(check_box(&lb, &l, 1), Some(Rect::new(8, 21, 13, 13)));
        assert_eq!(hit(&lb, &l, 11, 25), Some(Hit::Check(1)));
        assert_eq!(hit(&lb, &l, 60, 25), Some(Hit::Row(1)));
        let mut c = Canvas::new(150, 60);
        draw(&mut c, &text, &classic_paint(cs, 1), &lb, &l, &[], None, 11);
        let ink = cs.window_text.argb();
        for (x, y) in [(8, 4), (9, 5), (20, 16), (19, 15)] {
            assert_eq!(c.get(x, y), ink, "({x}, {y}): the 2 px frame");
        }
        assert_eq!(c.get(10, 6), cs.window.argb(), "white inside it");
        let ticked = |r: Rect| {
            (r.x + 3..r.right() - 3)
                .flat_map(|x| (r.y + 3..r.bottom() - 3).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == ink)
                .count()
        };
        assert!(
            ticked(Rect::new(8, 4, 13, 13)) > 0,
            "the first row is ticked"
        );
        assert_eq!(ticked(Rect::new(8, 21, 13, 13)), 0, "the second is clear");
        let label = (0..150).find(|&x| (4..17).any(|y| c.get(x, y) == ink && x > 21));
        assert!(
            label.is_some_and(|x| x >= 2 + 25),
            "the label 25 px in: {label:?}"
        );
    }

    #[test]
    fn a_checked_list_given_icons_draws_each_after_its_box() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut lb = ListBox::new(vec!["Pass".into(), "Notes".into()], None);
        lb.checks = vec![true, false];
        let l = layout(&lb, Rect::new(0, 0, 150, 60), &ControlMetrics::CLASSIC);
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(0, 0, 16, 16), Color::rgb(0, 128, 0));
        let mut c = Canvas::new(150, 60);
        let icons = [Some(icon), None];
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &lb,
            &l,
            &icons,
            None,
            11,
        );
        let green = Color::rgb(0, 128, 0).argb();
        assert_eq!(c.get(2 + 25, 2), green, "the icon 25 px in");
        assert_eq!(c.get(2 + 25 + 15, 17), green, "16 px square");
        assert_eq!(c.get(2 + 24, 8), cs.window.argb(), "clear of the box");
        assert_ne!(c.get(8, 4), green, "the box keeps its frame");
        let ink = cs.window_text.argb();
        let label =
            |y0: i32| (2 + 25 + 16..150).find(|&x| (y0..y0 + 13).any(|y| c.get(x, y) == ink));
        assert!(
            label(4).is_some_and(|x| x >= 2 + 25 + 19),
            "the label 19 px past the icon: {:?}",
            label(4)
        );
        assert!(
            label(21).is_some_and(|x| x >= 2 + 25 + 19),
            "a row without an icon keeps the label in line: {:?}",
            label(21)
        );
    }

    #[test]
    fn a_greyed_row_draws_its_label_in_gray_text() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut lb = ListBox::new(vec!["Speaker".into(), "HDMI".into()], Some(0));
        lb.greyed = vec![false, true];
        let l = layout(&lb, Rect::new(0, 0, 120, 40), &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(120, 40);
        draw(&mut c, &text, &classic_paint(cs, 1), &lb, &l, &[], None, 11);
        let row = |i: i32| Rect::new(l.inner.x + TEXT_X, l.inner.y + i * ROW_H, 60, ROW_H);
        let inks = |r: Rect, col: Color| {
            (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == col.argb()))
        };
        assert!(
            inks(row(1), cs.gray_text),
            "the greyed row inks in gray_text"
        );
        assert!(!inks(row(1), cs.button_text), "and not in button_text");
        assert!(
            inks(row(0), cs.hilight_text),
            "the selected row keeps the highlight"
        );
    }

    #[test]
    fn a_detail_draws_dimmed_after_the_label_and_off_the_highlight() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let mut lb = ListBox::new(vec!["notes.txt".into()], Some(0));
        lb.detail = vec!["~/Documents".into()];
        let l = layout(&lb, Rect::new(0, 0, 300, 21), &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(300, 21);
        draw(&mut c, &text, &classic_paint(cs, 1), &lb, &l, &[], None, 11);
        let tw = text.measure("notes.txt", false, 11);
        let dx = l.inner.x + TEXT_X + tw + DETAIL_GAP;
        let r = Rect::new(dx, l.inner.y, 80, ROW_H);
        let inks = |col: Color| {
            (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == col.argb()))
        };
        assert!(inks(cs.gray_text), "the detail inks in gray_text");
        assert!(!inks(cs.hilight), "the highlight stops at the label");
        assert_eq!(
            c.get(dx - 3, l.inner.y + 8),
            cs.window.argb(),
            "window between the highlight and the detail"
        );
    }

    /// Draws a list with icons, plain and with check boxes, in each look at 100-200% text, scale 1
    /// and 2: the icon column grows with the text, each icon stands whole in its row where the
    /// layout puts it, the labels' ink stays clear of the icon and its gap, and no line's ink is
    /// cut. Hashes each list at 200%.
    #[test]
    fn icons_grow_with_the_text_and_labels_clear_them() {
        use crate::interface::Interface;
        crate::pins::fonts();
        const INK: u32 = 0xFF12_3456;
        let names = ["Notepad", "Paint", "Terminal"];
        let (mut sizes, mut cut, mut hashes) = (Vec::new(), Vec::new(), Vec::new());
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for pct in [100, 125, 150, 175, 200] {
                let base = crate::interfaces::theme_for(interface).with_text_size(pct);
                let text = TextRenderer::load(&base.fonts);
                let cm = base.controls();
                sizes.push(icon_px(&cm));
                for checked in [false, true] {
                    let kind = if checked { "checked" } else { "plain" };
                    let mut lb = ListBox::new(names.map(String::from).to_vec(), Some(1));
                    lb.detail = vec![String::new(), "~/Pictures".into(), String::new()];
                    if checked {
                        lb.checks = vec![true, false, true];
                    }
                    let frame = Rect::new(0, 0, base.text.len(220), 3 * cm.list_row + 4);
                    let lay = layout(&lb, frame, &cm);
                    assert_eq!(lay.visible, 3);
                    let icon = lay.icon;
                    for s in [1, 2] {
                        let at = format!("{interface:?} {kind} at {pct}% scale {s}");
                        let theme = base.scaled(s);
                        let text = text.for_output(s);
                        let px = theme.fonts.size_px;
                        let mut ic = Canvas::new((icon * s) as u32, (icon * s) as u32);
                        ic.pixels.fill(INK);
                        let icons = vec![Some(ic); 3];
                        let drawn = |lb: &ListBox| {
                            let mut c =
                                Canvas::new((frame.width * s) as u32, (frame.height * s) as u32);
                            draw(&mut c, text, &theme.paint(), lb, &lay, &icons, None, px);
                            c
                        };
                        let (c, cuts) = crate::pins::clipped_ink(|| drawn(&lb));
                        cut.extend(
                            (cuts.into_iter())
                                .filter(|c| c.rows > 0 || c.cols > 0)
                                .map(|c| (interface, kind, pct, s, c.text)),
                        );
                        let mut unlabelled = lb.clone();
                        unlabelled.items = vec![String::new(); 3];
                        unlabelled.detail.clear();
                        let bare = drawn(&unlabelled);
                        let tx = (lay.inner.x + text_x(&lb, &lay, &icons)) * s;
                        let ix = (lay.inner.x + icon_x(&lb)) * s;
                        assert_eq!(
                            tx - (ix + icon * s),
                            3 * s,
                            "{at}: the label 3 px past the icon"
                        );
                        for k in 0..3 {
                            let row = Rect::new(
                                lay.inner.x * s,
                                (lay.inner.y + k * lay.row_h) * s,
                                lay.inner.width * s,
                                lay.row_h * s,
                            );
                            let inked: Vec<(i32, i32)> = (row.y..row.bottom())
                                .flat_map(|y| (row.x..row.right()).map(move |x| (x, y)))
                                .filter(|&(x, y)| c.get(x, y) == INK)
                                .collect();
                            assert_eq!(
                                inked.len(),
                                (icon * s * icon * s) as usize,
                                "{at} row {k}: the icon is whole and in its row"
                            );
                            let iy = row.y + (lay.row_h - icon - 1) / 2 * s;
                            assert_eq!(inked[0], (ix, iy), "{at} row {k}: the icon's corner");
                            let before = Rect::new(row.x, row.y, tx - 2 * s - row.x, row.height);
                            let label_ink = (before.y..before.bottom())
                                .flat_map(|y| (before.x..before.right()).map(move |x| (x, y)))
                                .filter(|&(x, y)| c.get(x, y) != bare.get(x, y))
                                .count();
                            assert_eq!(
                                label_ink, 0,
                                "{at} row {k}: the label's ink clears the icon"
                            );
                        }
                        if pct == 200 {
                            hashes
                                .push((format!("{interface:?} {kind} {s}"), crate::pins::fnv(&c)));
                        }
                    }
                }
            }
        }
        assert_eq!(cut, Vec::new(), "(look, list, text size, scale, line)");
        let at = |skip| {
            sizes
                .iter()
                .skip(skip)
                .step_by(5)
                .copied()
                .collect::<Vec<i32>>()
        };
        assert_eq!(at(0), vec![16; 3], "16 px at 100% in every look");
        assert_eq!(at(4), vec![32; 3], "32 px at 200%");
        crate::pins::check("controls/listbox_icons_200", &hashes);
    }
}
