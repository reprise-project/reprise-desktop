//! The single-selection list box with an icon per row, an optional dimmed detail, per-row check boxes and the
//! vertical scrollbar once the rows outgrow the box.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    BarHit, ScrollHit, ScrollParts, scrollbar_hit, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::metrics::ControlMetrics;
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
    /// Parallel to `items`: each row's check box, in place of its icon.
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

    pub fn scroll(&mut self, rows: i32, visible: usize) -> bool {
        let max_top = self.items.len().saturating_sub(visible.max(1));
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
    pub inner: Rect,
    pub visible: usize,
    pub scrollbar: Option<Rect>,
    /// The look's shortest scrollbar thumb, which hit and drag lay the
    /// bar out with as `draw` does.
    pub thumb_min: i32,
    pub check: i32,
    pub row_h: i32,
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
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    /// The thumb, carrying the grab offset from its top so a drag keeps
    /// hold of the point it was grabbed by.
    Thumb(i32),
    Check(usize),
}

/// Row `i`'s check box, when it is in view: the look's check box side
/// square, in the icon's place.
pub fn check_box(lb: &ListBox, lay: &Layout, i: usize) -> Option<Rect> {
    if i >= lb.checks.len() || i < lb.top || i >= lb.top + lay.visible {
        return None;
    }
    let y = lay.inner.y + (i - lb.top) as i32 * lay.row_h + CHECK_Y + (lay.row_h - ROW_H) / 2;
    Some(Rect::new(lay.inner.x + CHECK_X, y, lay.check, lay.check))
}

pub fn scroll_parts(lb: &ListBox, lay: &Layout) -> Option<ScrollParts> {
    lay.scrollbar.map(|sb| {
        scrollbar_parts(
            sb,
            1,
            lb.top,
            lay.visible,
            lb.items.len(),
            false,
            lay.thumb_min,
        )
    })
}

pub fn hit(lb: &ListBox, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if let Some(sb) = lay.scrollbar.filter(|sb| sb.contains(x, y)) {
        let p = scrollbar_parts(
            sb,
            1,
            lb.top,
            lay.visible,
            lb.items.len(),
            false,
            lay.thumb_min,
        );
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
    let row = ((y - lay.inner.y) / lay.row_h) as usize + lb.top;
    if check_box(lb, lay, row).is_some_and(|b| b.contains(x, y)) {
        return Some(Hit::Check(row));
    }
    (row < lb.items.len()).then_some(Hit::Row(row))
}

impl Hit {
    pub fn bar(self) -> Option<BarHit> {
        match self {
            Hit::ScrollUp => Some(BarHit::Back),
            Hit::ScrollDown => Some(BarHit::Forward),
            Hit::PageUp => Some(BarHit::PageBack),
            Hit::PageDown => Some(BarHit::PageForward),
            Hit::Thumb(grab) => Some(BarHit::Thumb(grab)),
            Hit::Row(_) | Hit::Check(_) => None,
        }
    }
}

/// The top row a thumb grabbed `grab` px below its own top scrolls to
/// when the pointer sits at `y`.
pub fn top_at(lb: &ListBox, lay: &Layout, y: i32, grab: i32) -> usize {
    match scroll_parts(lb, lay) {
        Some(p) => scrollbar_pos_at(&p, y - grab, lay.visible, lb.items.len(), false),
        None => lb.top,
    }
}

/// Draw the box; `icons` is parallel to the items (16 px, or None for
/// no icon). A partly visible last row is clipped.
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
        } else if let Some(Some(ic)) = icons.get(i) {
            let iy = y + glyph_dy;
            let clip = Rect::new(inner.x + ICON_X * s, iy, ic.width as i32, ic.height as i32)
                .intersect(&inner);
            c.blit_over_clipped(inner.x + ICON_X * s, iy, ic, clip);
        }
        let text_x = if lb.checks.is_empty() {
            TEXT_X
        } else {
            CHECK_TEXT_X
        };
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
        let tr = Rect::new(tx, y, (inner.right() - tx).max(0), rh).intersect(&row);
        text.draw(c, tr, &lb.items[i], false, fg, Align::Left, px);
        if let Some(d) = lb.detail.get(i).filter(|d| !d.is_empty()) {
            let dx = tx + tw + DETAIL_GAP * s;
            let dw = (inner.right() - dx - 2 * s).max(0);
            let shown = text.ellipsize(d, dw, false, px);
            let dr = Rect::new(dx, y, dw, rh).intersect(&row);
            text.draw(c, dr, &shown, false, cs.gray_text, Align::Left, px);
        }
        y += rh;
    }
}

/// The list box's `access` part: a list with one option per item, the
/// selected one marked. Rows scrolled out of view have no bounds. Returns
/// the node that has the focus when the list does: the selected row, else
/// the list.
pub fn access(
    lb: &ListBox,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, lay.frame);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for (i, item) in lb.items.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(item.as_str());
        if let Some(d) = lb.detail.get(i).filter(|d| !d.is_empty()) {
            o.set_description(d.as_str());
        }
        if (lb.top..lb.top + lay.visible).contains(&i) {
            let y = lay.inner.y + (i - lb.top) as i32 * lay.row_h;
            o.set_bounds(access::bounds(Rect::new(
                lay.inner.x,
                y,
                lay.inner.width,
                lay.row_h,
            )));
        }
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
/// the list at `lay`. Returns the highlighted row's node, else the
/// field's: the arrows move the highlight while the text stays in the
/// field, so a reader follows the row.
#[allow(clippy::too_many_arguments)]
pub fn access_filtered(
    edit: &crate::textedit::TextEdit,
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
        assert_eq!(hit(&lb, &l, 285, 262), Some(Hit::ScrollUp));
        assert_eq!(hit(&lb, &l, 285, 360), Some(Hit::ScrollDown));
        assert_eq!(hit(&lb, &l, 285, 280), Some(Hit::Thumb(5)));
        assert_eq!(hit(&lb, &l, 285, 320), Some(Hit::PageDown));
        assert_eq!(top_at(&lb, &l, 280, 5), 0);
        assert_eq!(top_at(&lb, &l, 334, 5), 14);
        assert_eq!(top_at(&lb, &l, 999, 5), 14);
        let mut end = ListBox::new(names(20), Some(0));
        end.top = 14;
        assert_eq!(hit(&end, &l, 285, 280), Some(Hit::PageUp));
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
}
