//! The list a property sheet picks its page from in list mode: headings and page names in a sunken box.
//! Rows that overflow scroll under the look's scrollbar; a held arrow repeats on the host's ticks ([`PageList::bar_tick`]).
//! Which page is open is the sheet's [`TabStrip`]; pages are numbered down the list, across headings.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BarHit, ScrollParts, bar_hit, scrollbar_parts};
use crate::controls::form;
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::{ArrowRepeat, ThumbDrag, held};
use crate::controls::tabstrip::TabStrip;
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::look::widgets::{draw_field_frame, draw_focus_ring, draw_scrollbar, group_label_ink};
use crate::pointer::{Axis, Interaction, Pointer, ScrollHit};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};
use std::time::Instant;

pub const WIDTH: i32 = 140;
/// The list's left edge to the page's: the list, then the sheet's 6 px
/// margin.
pub const ADVANCE: i32 = WIDTH + 6;
pub const ROW_H: i32 = 17;
const HEADING_X: i32 = 4;
const PAGE_X: i32 = 14;
const HEADING_GAP: i32 = 4;
const HIGHLIGHT_PAD: i32 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Heading(String),
    Page(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageList {
    pub rows: Vec<Row>,
    /// How far the rows are scrolled up, in px; read through the layout,
    /// which holds it to the rows' range.
    pub top: i32,
    pub drag: ThumbDrag,
    pub bars: Pointer<ScrollHit>,
    repeat: ArrowRepeat,
}

impl PageList {
    /// `groups` are the headings in order, each with its pages' labels;
    /// the pages are numbered from 0 down the whole list.
    pub fn new(groups: &[(&str, &[&str])]) -> PageList {
        let mut rows = Vec::new();
        let mut page = 0;
        for (heading, labels) in groups {
            rows.push(Row::Heading((*heading).to_string()));
            for _ in labels.iter() {
                rows.push(Row::Page(page));
                page += 1;
            }
        }
        PageList {
            rows,
            top: 0,
            drag: ThumbDrag::default(),
            bars: Pointer::new(),
            repeat: ArrowRepeat::default(),
        }
    }

    /// Scroll by `dy` px, held to the rows' range. True when the rows
    /// moved.
    pub fn scroll(&mut self, dy: i32, lay: &Layout) -> bool {
        self.scroll_to(lay.top + dy, lay)
    }

    /// Scroll so `top` px of the rows are above the inside, held to the
    /// rows' range. True when the rows moved.
    pub fn scroll_to(&mut self, top: i32, lay: &Layout) -> bool {
        let to = held(top, lay.inner.height, lay.total);
        let moved = to != lay.top;
        self.top = to;
        moved
    }

    /// Scroll page `page`'s row fully into view, as little as it takes; a
    /// heading's first page brings its heading fully into view with it. True when the rows moved.
    pub fn reveal(&mut self, page: usize, lay: &Layout) -> bool {
        let Some(i) = self.rows.iter().position(|r| *r == Row::Page(page)) else {
            return false;
        };
        let (r, inner) = (lay.rows[i], lay.inner);
        let first = match i.checked_sub(1).map(|h| &self.rows[h]) {
            Some(Row::Heading(_)) => lay.rows[i - 1].y,
            _ => r.y,
        };
        let top = if first < inner.y {
            lay.top - (inner.y - first)
        } else if r.bottom() > inner.bottom() {
            lay.top + (r.bottom() - inner.bottom())
        } else {
            lay.top
        };
        self.scroll_to(top, lay)
    }

    /// A press on part `hit` of the scrollbar: an arrow scrolls a row and
    /// starts to repeat, the track the inside's height, the thumb is
    /// grabbed. The part draws pressed. True when the rows moved.
    pub fn bar_press(&mut self, hit: BarHit, lay: &Layout) -> bool {
        self.bars.press(hit.part(), matches!(hit, BarHit::Thumb(_)));
        self.repeat.press(hit);
        let by = self.drag.press(Axis::Vertical, hit);
        let dy = by.by(lay.row_h as i64, lay.inner.height as i64) as i32;
        self.scroll(dy, lay)
    }

    /// The pointer moved onto `hit` of the scrollbar (`None`: off it): the
    /// part under it lights, a held one draws pressed only while the
    /// pointer is on it (the thumb wherever it goes). True when what
    /// draws changed.
    pub fn bar_hover(&mut self, hit: Option<BarHit>) -> bool {
        self.bars.motion(hit.and_then(BarHit::part))
    }

    pub fn bar_leave(&mut self) -> bool {
        self.bars.leave()
    }

    /// The pointer let go: the thumb and the arrow are released, and the
    /// part under the pointer turns lit. True when what draws changed.
    pub fn bar_release(&mut self) -> bool {
        self.drag.release();
        self.repeat.release();
        let before = self.bars.lit();
        self.bars.release();
        self.bars.lit() != before
    }

    pub fn repeating(&self) -> bool {
        self.repeat.held()
    }

    /// A host tick at `now` with an arrow held: the first sets when it
    /// repeats; from then the arrow steps a row every [`REPEAT_EVERY`](crate::controls::scrollbar::REPEAT_EVERY)
    /// while the pointer is on it. True when the rows moved.
    pub fn bar_tick(&mut self, now: Instant, lay: &Layout) -> bool {
        if !self.repeat.due(now) {
            return false;
        }
        let dy = match self.bars.lit() {
            Some((ScrollHit::Back, Interaction::Pressed)) => -lay.row_h,
            Some((ScrollHit::Forward, Interaction::Pressed)) => lay.row_h,
            _ => return false,
        };
        self.scroll(dy, lay)
    }

    /// The pointer at (`x`, `y`) while the thumb is held: the rows follow
    /// it. `None` unless the thumb is held, else whether they moved.
    pub fn bar_drag(&mut self, x: i32, y: i32, lay: &Layout) -> Option<bool> {
        let parts = bar_parts(lay)?;
        let (visible, total) = (lay.inner.height as usize, lay.total as usize);
        let top = self
            .drag
            .pos(Axis::Vertical, &parts, (x, y), visible, total)?;
        Some(self.scroll_to(top as i32, lay))
    }

    pub fn page_count(&self) -> usize {
        self.rows
            .iter()
            .filter(|r| matches!(r, Row::Page(_)))
            .count()
    }
}

/// The page a key on the focused list opens, from `open` of `count`: Up
/// and Down the one before or after it, stopping at the ends, Home and
/// End the first and the last. `None` for any other key.
pub fn target(open: usize, count: usize, key: &form::Key) -> Option<usize> {
    let last = count.saturating_sub(1);
    Some(match key {
        form::Key::Up => open.saturating_sub(1),
        form::Key::Down => (open + 1).min(last),
        form::Key::Home => 0,
        form::Key::End => last,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub inner: Rect,
    /// One rect per row, as wide as the inside, moved up by `top`: rows
    /// scrolled out reach past the inside.
    pub rows: Vec<Rect>,
    /// Along the inside's right edge, the look's scrollbar width, when
    /// the rows overflow the inside.
    pub scrollbar: Option<Rect>,
    pub total: i32,
    pub top: i32,
    pub thumb_min: i32,
    /// A row's height: the look's list row ([`ControlMetrics::list_row`]).
    pub row_h: i32,
}

pub fn layout(list: &PageList, frame: Rect, cm: &ControlMetrics) -> Layout {
    let mut inner = frame.inset(2);
    let mut y = 0;
    let mut tops = Vec::with_capacity(list.rows.len());
    for (i, row) in list.rows.iter().enumerate() {
        if i > 0 && matches!(row, Row::Heading(_)) {
            y += HEADING_GAP;
        }
        tops.push(y);
        y += cm.list_row;
    }
    let total = y;
    let scrollbar = (total > inner.height).then(|| {
        inner.width -= cm.scroll;
        Rect::new(inner.right(), inner.y, cm.scroll, inner.height)
    });
    let top = held(list.top, inner.height, total);
    let rows = tops
        .into_iter()
        .map(|t| Rect::new(inner.x, inner.y + t - top, inner.width, cm.list_row))
        .collect();
    Layout {
        frame,
        inner,
        rows,
        scrollbar,
        total,
        top,
        thumb_min: cm.thumb_min,
        row_h: cm.list_row,
    }
}

pub fn bar_parts(lay: &Layout) -> Option<ScrollParts> {
    let sb = lay.scrollbar?;
    let (visible, total) = (lay.inner.height as usize, lay.total as usize);
    Some(scrollbar_parts(
        sb,
        1,
        lay.top as usize,
        visible,
        total,
        false,
        lay.thumb_min,
    ))
}

pub fn hit_bar(lay: &Layout, x: i32, y: i32) -> Option<BarHit> {
    if !lay.scrollbar?.contains(x, y) {
        return None;
    }
    Some(bar_hit(&bar_parts(lay)?, x, y, false))
}

/// The page whose row is under (`x`, `y`): `None` on a heading, in a gap,
/// off the rows or on a row scrolled out of the inside.
pub fn hit(list: &PageList, lay: &Layout, x: i32, y: i32) -> Option<usize> {
    if !lay.inner.contains(x, y) {
        return None;
    }
    let i = lay.rows.iter().position(|r| r.contains(x, y))?;
    match list.rows.get(i)? {
        Row::Page(k) => Some(*k),
        Row::Heading(_) => None,
    }
}

/// Draw the list, `strip` naming the pages and the open one, and its
/// scrollbar when the rows overflow. Rows are clipped to the inside.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    list: &PageList,
    strip: &TabStrip,
    lay: &Layout,
    cues: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, cs.window);
    draw_field_frame(c, p, frame, cs.window);
    if let Some(sb) = lay.scrollbar {
        let (visible, total) = (lay.inner.height as usize, lay.total as usize);
        let top = lay.top as usize;
        draw_scrollbar(c, p, sb.scaled(s), top, visible, total, list.bars.lit());
    }
    let inner = lay.inner.scaled(s);
    c.with_clip(inner, |c| {
        for (row, r) in list.rows.iter().zip(&lay.rows) {
            let r = r.scaled(s);
            if r.intersect(&inner).is_empty() {
                continue;
            }
            draw_row(c, text, p, row, r, &inner, strip, cues, px);
        }
    });
}

#[expect(clippy::too_many_arguments)]
fn draw_row(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    row: &Row,
    r: Rect,
    inner: &Rect,
    strip: &TabStrip,
    cues: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    match row {
        Row::Heading(h) => {
            let tr = Rect::new(r.x + HEADING_X * s, r.y, r.width - HEADING_X * s, r.height);
            text.draw(c, tr, h, true, group_label_ink(p), Align::Left, px);
        }
        Row::Page(k) => {
            let Some(label) = strip.labels.get(*k) else {
                return;
            };
            let tx = r.x + PAGE_X * s;
            let tr = Rect::new(tx, r.y, r.right() - tx, r.height);
            if *k != strip.open {
                text.draw(c, tr, label, false, cs.button_text, Align::Left, px);
                return;
            }
            // A look that boxes its items boxes the open page's row, 1
            // px in from either side, darker while the list has the
            // focus; the text stays as it is.
            let mark = Mark {
                selected: true,
                lead: strip.focused,
                ..Mark::default()
            };
            let row = Rect::new(r.x + s, r.y, r.width - 2 * s, r.height);
            if item::draw_item(c, p, row, mark) {
                text.draw(c, tr, label, false, cs.button_text, Align::Left, px);
                return;
            }
            let tw = text.measure(label, false, px);
            let pad = HIGHLIGHT_PAD * s;
            let hl = Rect::new(tx - pad, r.y, tw + 2 * pad, r.height).intersect(inner);
            c.fill_rect(hl, cs.hilight);
            text.draw(c, tr, label, false, cs.hilight_text, Align::Left, px);
            if strip.focused && cues {
                draw_focus_ring(c, p, hl, cs.hilight_text);
            }
        }
    }
}

/// The list's `access` part: a list holding, row by row, each heading as
/// static text and each page as a list item named by its label, the open
/// one selected. A row's bounds are its part inside the list; a row
/// scrolled out of view has none. Returns the pages' items in page order.
pub fn access(
    list: &PageList,
    strip: &TabStrip,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> Vec<NodeId> {
    let mut list_node = access::node(Role::List, lay.frame);
    list_node.set_label("Pages");
    let id = tree.push(parent, key, list_node);
    let total = list
        .rows
        .iter()
        .filter(|r| matches!(r, Row::Page(_)))
        .count();
    let mut items = Vec::new();
    for (i, (row, r)) in list.rows.iter().zip(&lay.rows).enumerate() {
        let shown = r.intersect(&lay.inner);
        let place = |n: &mut accesskit::Node| {
            if shown.is_empty() {
                n.clear_bounds();
            } else {
                n.set_bounds(access::bounds(shown));
            }
        };
        match row {
            Row::Heading(h) => {
                let mut n = access::label(*r, h, true);
                place(&mut n);
                tree.push(id, i, n);
            }
            Row::Page(k) => {
                let mut n = access::node(Role::ListItem, *r);
                place(&mut n);
                n.set_label(strip.labels.get(*k).map_or("", String::as_str));
                n.set_selected(*k == strip.open);
                n.set_position_in_set(items.len() + 1);
                n.set_size_of_set(total);
                access::focusable(&mut n, true);
                items.push(tree.push(id, i, n));
            }
        }
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::ROOT;
    use crate::controls::scrollbar::{REPEAT_AFTER, REPEAT_EVERY};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::theme::{Color, FontConfig, Theme};
    use std::time::Duration;

    const GROUPS: &[(&str, &[&str])] =
        &[("Look", &["Night Light", "Effects"]), ("Input", &["Mouse"])];
    const FRAME: Rect = Rect::new(6, 8, 140, 120);

    fn strip(open: usize, focused: bool) -> TabStrip {
        let mut t = TabStrip::new(&["Night Light", "Effects", "Mouse"], open);
        t.focused = focused;
        t
    }

    #[test]
    fn headings_sit_over_their_pages_with_a_gap_before_each_after_the_first() {
        let list = PageList::new(GROUPS);
        assert_eq!(
            list.rows,
            vec![
                Row::Heading("Look".into()),
                Row::Page(0),
                Row::Page(1),
                Row::Heading("Input".into()),
                Row::Page(2),
            ]
        );
        assert_eq!(list.page_count(), 3);
        let lay = layout(&list, FRAME, &ControlMetrics::CLASSIC);
        assert_eq!(lay.inner, Rect::new(8, 10, 136, 116));
        assert_eq!(
            lay.rows,
            vec![
                Rect::new(8, 10, 136, 17),
                Rect::new(8, 27, 136, 17),
                Rect::new(8, 44, 136, 17),
                Rect::new(8, 65, 136, 17),
                Rect::new(8, 82, 136, 17),
            ]
        );
        assert_eq!(hit(&list, &lay, 30, 50), Some(1));
        assert_eq!(hit(&list, &lay, 30, 90), Some(2));
        assert_eq!(hit(&list, &lay, 30, 15), None, "a heading is no page");
        assert_eq!(hit(&list, &lay, 30, 62), None, "the gap before a heading");
        assert_eq!(hit(&list, &lay, 30, 110), None, "under the last row");
        assert_eq!(hit(&list, &lay, 150, 50), None, "right of the list");
    }

    #[test]
    fn up_down_home_and_end_name_the_page_they_open() {
        use form::Key;
        assert_eq!(target(1, 3, &Key::Down), Some(2));
        assert_eq!(target(2, 3, &Key::Down), Some(2), "stops at the last");
        assert_eq!(target(1, 3, &Key::Up), Some(0));
        assert_eq!(target(0, 3, &Key::Up), Some(0), "stops at the first");
        assert_eq!(target(1, 3, &Key::Home), Some(0));
        assert_eq!(target(0, 3, &Key::End), Some(2));
        assert_eq!(target(1, 3, &Key::Left), None);
        assert_eq!(target(1, 3, &Key::PageDown), None);
    }

    /// In each look: the look's list frame round a window-coloured inside,
    /// the heading in the look's group label ink, a closed page's name in
    /// button text, and the open page's name on the scheme's highlight
    /// from 2 px before its text to 2 px past it, 17 rows tall.
    #[test]
    fn the_open_page_draws_on_the_highlight_in_classic_and_blue() {
        let text = TextRenderer::load(&FontConfig::default());
        let list = PageList::new(GROUPS);
        let lay = layout(&list, FRAME, &ControlMetrics::CLASSIC);
        for i in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(i);
            let (p, cs, px) = (theme.paint(), &theme.colors, theme.fonts.size_px);
            let mut c = Canvas::new(160, 140);
            c.fill_rect(c.bounds(), cs.button_face);
            let mut bare = c.clone();
            draw(&mut c, &text, &p, &list, &strip(1, false), &lay, false, px);
            bare.fill_rect(FRAME, cs.window);
            draw_field_frame(&mut bare, &p, FRAME, cs.window);
            for y in FRAME.y..FRAME.bottom() {
                for x in FRAME.x..FRAME.right() {
                    if !lay.inner.contains(x, y) {
                        assert_eq!(
                            c.get(x, y),
                            bare.get(x, y),
                            "{i:?}: ({x}, {y}) is the frame"
                        );
                    }
                }
            }
            let tw = text.measure("Effects", false, px);
            assert_eq!(
                c.get(20, 46),
                cs.hilight.argb(),
                "{i:?}: the highlight's left column"
            );
            assert_eq!(
                c.get(23 + tw, 60),
                cs.hilight.argb(),
                "{i:?}: its bottom-right pixel"
            );
            assert_eq!(c.get(19, 46), cs.window.argb(), "{i:?}: window left of it");
            assert_eq!(
                c.get(24 + tw, 46),
                cs.window.argb(),
                "{i:?}: window right of it"
            );
            assert_eq!(c.get(20, 61), cs.window.argb(), "{i:?}: 17 rows tall");
            assert_eq!(
                c.get(21, 30),
                cs.window.argb(),
                "{i:?}: a closed page has none"
            );
            let inks = |r: Rect, col: Color| {
                (r.y..r.bottom()).any(|y| (r.x..r.right()).any(|x| c.get(x, y) == col.argb()))
            };
            assert!(
                inks(Rect::new(12, 10, 60, 17), group_label_ink(&p)),
                "{i:?}: the heading"
            );
            assert!(
                inks(Rect::new(22, 27, 60, 17), cs.button_text),
                "{i:?}: a closed page"
            );
            assert!(
                inks(Rect::new(22, 44, tw, 17), cs.hilight_text),
                "{i:?}: the open page"
            );
        }
    }

    /// The dotted ring runs round the highlight only while the list has
    /// the focus and the keyboard cues are out.
    /// Under Glass the open page's row stands in the selected item box,
    /// 1 px in from the list's sides, darker while the list has the
    /// focus, its name staying black.
    #[test]
    fn a_glass_open_page_stands_in_the_item_box() {
        let text = TextRenderer::load(&FontConfig::default());
        let list = PageList::new(GROUPS);
        let lay = layout(&list, FRAME, &ControlMetrics::CLASSIC);
        let theme = theme_for(Interface::Glass);
        let (p, cs, px) = (theme.paint(), &theme.colors, theme.fonts.size_px);
        let row = list
            .rows
            .iter()
            .zip(&lay.rows)
            .find_map(|(r, at)| matches!(r, Row::Page(1)).then_some(*at))
            .expect("page 1 has a row");
        for (focused, outline) in [(false, 0xFF81_A5DC), (true, 0xFF7A_9BCD)] {
            let mut c = Canvas::new(160, 140);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(
                &mut c,
                &text,
                &p,
                &list,
                &strip(1, focused),
                &lay,
                false,
                px,
            );
            let x = row.x + 40;
            assert_eq!(c.get(x, row.y), outline, "focused {focused}: the outline");
            assert_eq!(c.get(lay.inner.x, row.y + 8), cs.window.argb(), "1 px in");
            let black = (row.y..row.bottom())
                .any(|y| (row.x..row.right()).any(|x| c.get(x, y) == cs.window_text.argb()));
            assert!(black, "focused {focused}: the name stays black");
        }
    }

    #[test]
    fn the_focused_list_rings_the_open_page() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let (p, cs, px) = (theme.paint(), &theme.colors, theme.fonts.size_px);
        let list = PageList::new(GROUPS);
        let lay = layout(&list, FRAME, &ControlMetrics::CLASSIC);
        for (focused, cues, ringed) in [
            (true, true, true),
            (true, false, false),
            (false, true, false),
        ] {
            let mut c = Canvas::new(160, 140);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &text, &p, &list, &strip(1, focused), &lay, cues, px);
            let dots = (44..61)
                .filter(|&y| c.get(20, y) == cs.hilight_text.argb())
                .count();
            assert_eq!(dots > 0, ringed, "focused {focused}, cues {cues}");
        }
    }

    /// A list of the rows in order: headings as static text, pages as
    /// list items named by their labels, the open one selected.
    #[test]
    fn the_list_is_a_list_of_pages_its_headings_static_text() {
        let list = PageList::new(GROUPS);
        let lay = layout(&list, FRAME, &ControlMetrics::CLASSIC);
        let mut tree = Tree::window("T", (160, 140));
        let items = access(&list, &strip(1, false), &lay, &mut tree, ROOT, "pages");
        assert_eq!(items.len(), 3);
        let effects = tree.find(Role::ListItem, "Effects").unwrap();
        assert_eq!(items[1], effects);
        assert_eq!(tree.get(effects).unwrap().is_selected(), Some(true));
        assert_eq!(tree.get(items[0]).unwrap().is_selected(), Some(false));
        let (node, _) = tree.nodes().find(|(_, n)| n.role() == Role::List).unwrap();
        let kids = tree.get(node).unwrap().children().to_vec();
        assert_eq!(kids.len(), 5, "two headings and three pages, in order");
        let heading = tree.get(kids[3]).unwrap();
        assert_eq!(
            (heading.role(), heading.value()),
            (Role::Label, Some("Input"))
        );
        assert_eq!(kids[4], items[2]);
        assert_eq!(tree.get(node).unwrap().label(), Some("Pages"));
        let sets: Vec<_> = items
            .iter()
            .map(|&i| {
                let n = tree.get(i).unwrap();
                (n.position_in_set(), n.size_of_set())
            })
            .collect();
        assert_eq!(
            sets,
            vec![(Some(1), Some(3)), (Some(2), Some(3)), (Some(3), Some(3))]
        );
    }

    const SHORT: Rect = Rect::new(6, 8, 140, 60);

    #[test]
    fn rows_that_overflow_scroll_under_a_scrollbar() {
        let mut list = PageList::new(GROUPS);
        let cm = ControlMetrics::CLASSIC;
        let lay = layout(&list, SHORT, &cm);
        assert_eq!(lay.total, 89, "five rows and one heading gap");
        assert_eq!(lay.scrollbar, Some(Rect::new(128, 10, 16, 56)));
        assert_eq!(lay.inner, Rect::new(8, 10, 120, 56), "less the bar");
        assert_eq!(lay.rows[4], Rect::new(8, 82, 120, 17));
        assert_eq!(hit(&list, &lay, 30, 82), None, "page 2 is out of view");
        assert_eq!(hit(&list, &lay, 135, 30), None, "the bar is no row");
        assert_eq!(hit_bar(&lay, 135, 12), Some(BarHit::Back));
        assert_eq!(hit_bar(&lay, 135, 60), Some(BarHit::Forward));
        assert_eq!(hit_bar(&lay, 30, 12), None);

        assert!(list.reveal(2, &lay));
        assert_eq!(list.top, 33);
        let lay = layout(&list, SHORT, &cm);
        assert_eq!(lay.rows[4], Rect::new(8, 49, 120, 17));
        assert_eq!(hit(&list, &lay, 30, 60), Some(2));
        assert!(!list.reveal(2, &lay), "already in view");
        assert!(!list.reveal(1, &lay), "page 1 is in view too");
        assert!(list.reveal(0, &lay));
        assert_eq!(list.top, 0);
        // A page below another one comes to the inside's top alone: in
        // 36 px of inside, page 1's row (34 px down the rows) 11 px up.
        let tiny = Rect::new(6, 8, 140, 40);
        list.top = 45;
        let lay = layout(&list, tiny, &cm);
        assert_eq!(lay.rows[2].y, -1);
        assert!(list.reveal(1, &lay));
        assert_eq!(list.top, 34);
        list.top = 17;

        let lay = layout(&list, SHORT, &cm);
        assert!(list.scroll(-100, &lay));
        assert_eq!(list.top, 0);
        let lay = layout(&list, SHORT, &cm);
        assert!(!list.scroll(-17, &lay), "at the top already");
        assert!(list.bar_press(BarHit::Forward, &lay));
        assert_eq!(list.top, 17, "an arrow steps a row");
        let lay = layout(&list, SHORT, &cm);
        assert!(list.bar_press(BarHit::PageForward, &lay));
        assert_eq!(list.top, 33, "the track a page, held to the end");

        let mut tall = PageList::new(GROUPS);
        let lay = layout(&tall, FRAME, &cm);
        assert_eq!(lay.scrollbar, None);
        assert!(!tall.scroll(17, &lay));
        assert!(!tall.reveal(2, &lay));
    }

    #[test]
    fn the_thumb_drags_the_rows() {
        let mut list = PageList::new(GROUPS);
        let lay = layout(&list, SHORT, &ControlMetrics::CLASSIC);
        let parts = bar_parts(&lay).expect("a bar");
        let thumb = parts.thumb.expect("a thumb");
        assert_eq!(thumb, Rect::new(128, 26, 16, 15));
        let grab = hit_bar(&lay, 135, thumb.y + 3).unwrap();
        assert_eq!(grab, BarHit::Thumb(3));
        assert!(!list.bar_press(grab, &lay), "the press moves nothing");
        assert_eq!(list.bar_drag(135, thumb.y + 3 + 100, &lay), Some(true));
        assert_eq!(list.top, 33, "dragged past the end");
        list.drag.release();
        assert_eq!(list.bar_drag(135, thumb.y, &lay), None, "let go");
    }

    #[test]
    fn the_bar_lights_the_part_under_the_pointer_and_presses_the_held_one() {
        let mut list = PageList::new(GROUPS);
        let lay = layout(&list, SHORT, &ControlMetrics::CLASSIC);
        assert!(list.bar_hover(Some(BarHit::Forward)));
        assert_eq!(
            list.bars.lit(),
            Some((ScrollHit::Forward, Interaction::Hot))
        );
        list.bar_press(BarHit::Forward, &lay);
        assert_eq!(
            list.bars.lit(),
            Some((ScrollHit::Forward, Interaction::Pressed))
        );
        assert!(list.bar_hover(None), "dragged off: at rest");
        assert_eq!(list.bars.lit(), None);
        assert!(list.bar_hover(Some(BarHit::Forward)), "back on: pressed");
        assert!(list.bar_release());
        assert_eq!(
            list.bars.lit(),
            Some((ScrollHit::Forward, Interaction::Hot))
        );
        assert!(list.bar_leave());
        assert_eq!(list.bars.lit(), None);
        let thumb = bar_parts(&lay).unwrap().thumb.unwrap();
        list.bar_press(BarHit::Thumb(thumb.y + 3), &lay);
        list.bar_hover(None);
        assert_eq!(
            list.bars.lit(),
            Some((ScrollHit::Thumb, Interaction::Pressed))
        );
    }

    #[test]
    fn a_held_arrow_repeats_after_a_pause_while_the_pointer_is_on_it() {
        let cm = ControlMetrics::CLASSIC;
        let mut list = PageList::new(GROUPS);
        let lay = layout(&list, SHORT, &cm);
        let t0 = Instant::now();
        assert!(list.bar_press(BarHit::Forward, &lay));
        assert_eq!(list.top, 17, "the press steps once");
        assert!(list.repeating());
        let lay = layout(&list, SHORT, &cm);
        assert!(!list.bar_tick(t0, &lay), "the first tick sets the pause");
        assert!(!list.bar_tick(t0 + REPEAT_AFTER - Duration::from_millis(1), &lay));
        assert!(list.bar_tick(t0 + REPEAT_AFTER, &lay), "then it steps");
        assert_eq!(list.top, 33, "a row, held to the end");
        let lay = layout(&list, SHORT, &cm);
        let t1 = t0 + REPEAT_AFTER;
        list.bar_release();
        assert!(!list.repeating(), "the release stops it");
        assert!(!list.bar_tick(t1 + REPEAT_EVERY, &lay));

        list.bar_press(BarHit::Back, &lay);
        assert_eq!(list.top, 16);
        let lay = layout(&list, SHORT, &cm);
        list.bar_tick(t0, &lay);
        list.bar_hover(None);
        assert!(!list.bar_tick(t0 + REPEAT_AFTER, &lay), "off the arrow");
        assert_eq!(list.top, 16);
        list.bar_hover(Some(BarHit::Back));
        assert!(list.bar_tick(t0 + REPEAT_AFTER + REPEAT_EVERY, &lay));
        assert_eq!(list.top, 0, "back on it, it steps again");

        let lay = layout(&list, SHORT, &cm);
        list.bar_release();
        list.bar_press(BarHit::PageForward, &lay);
        assert!(!list.repeating());
    }

    /// In Blue the list hands its lit part to the look's scrollbar: the
    /// held down arrow draws exactly as `look::widgets::draw_scrollbar`
    /// draws it pressed, which differs from its rest.
    #[test]
    fn the_lit_part_draws_in_the_looks_scrollbar() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = theme_for(Interface::Blue);
        let (p, px) = (theme.paint(), theme.fonts.size_px);
        let mut list = PageList::new(GROUPS);
        let cm = theme.look.controls();
        let lay = layout(&list, SHORT, &cm);
        list.bar_press(BarHit::Forward, &lay);
        let lay = layout(&list, SHORT, &cm);
        let sb = lay.scrollbar.unwrap();
        let draw_list = |list: &PageList| {
            let mut c = Canvas::new(160, 80);
            draw(&mut c, &text, &p, list, &strip(0, false), &lay, false, px);
            c
        };
        let bar = |lit| {
            let mut c = Canvas::new(160, 80);
            let (top, total) = (lay.top as usize, lay.total as usize);
            draw_scrollbar(&mut c, &p, sb, top, lay.inner.height as usize, total, lit);
            c
        };
        let pressed = bar(Some((ScrollHit::Forward, Interaction::Pressed)));
        let rest = bar(None);
        let got = draw_list(&list);
        let pixels = |c: &Canvas| -> Vec<u32> {
            (sb.y..sb.bottom())
                .flat_map(|y| (sb.x..sb.right()).map(move |x| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .collect()
        };
        assert_eq!(pixels(&got), pixels(&pressed), "the pressed arrow");
        assert_ne!(pixels(&got), pixels(&rest), "pressed is not at rest");
        list.bar_release();
        list.bar_leave();
        assert_eq!(pixels(&draw_list(&list)), pixels(&rest), "let go and left");
    }

    /// The overflowing list in Classic: the look's scrollbar for 56 of
    /// 89 px fills the strip right of the rows, the frame stays the
    /// frame, and a row scrolled half out draws only its part inside.
    #[test]
    fn an_overflowing_list_draws_its_scrollbar_and_clips_its_rows() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = theme_for(Interface::Classic);
        let (p, cs, px) = (theme.paint(), &theme.colors, theme.fonts.size_px);
        let mut list = PageList::new(GROUPS);
        list.top = 8;
        let lay = layout(&list, SHORT, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(160, 80);
        c.fill_rect(c.bounds(), cs.button_face);
        let mut bare = c.clone();
        draw(&mut c, &text, &p, &list, &strip(0, false), &lay, false, px);
        bare.fill_rect(SHORT, cs.window);
        draw_field_frame(&mut bare, &p, SHORT, cs.window);
        let sb = lay.scrollbar.unwrap();
        crate::classic::widgets::draw_scrollbar(&mut bare, &p, sb, 8, 56, 89);
        for y in SHORT.y..SHORT.bottom() {
            for x in SHORT.x..SHORT.right() {
                if !lay.inner.contains(x, y) {
                    assert_eq!(c.get(x, y), bare.get(x, y), "({x}, {y})");
                }
            }
        }
        // "Night Light" (page 0) is 9 px under the inside's top: its
        // highlight starts there, and nothing of the heading's 8 px left
        // above it spills onto the frame (checked above).
        assert_eq!(lay.rows[1].y, 19);
        assert_eq!(c.get(20, 19), cs.hilight.argb(), "the open page's row");
        assert_eq!(c.get(20, 18), cs.window.argb(), "above it, the heading");
    }

    /// Rows scrolled out of the list have no bounds; one cut by its edge
    /// has its part inside.
    #[test]
    fn rows_scrolled_out_have_no_bounds() {
        let mut list = PageList::new(GROUPS);
        list.top = 25;
        let lay = layout(&list, SHORT, &ControlMetrics::CLASSIC);
        let mut tree = Tree::window("T", (160, 140));
        let items = access(&list, &strip(0, false), &lay, &mut tree, ROOT, "pages");
        let bounds = |id| tree.get(id).unwrap().bounds();
        assert_eq!(
            bounds(items[0]),
            Some(access::bounds(Rect::new(8, 10, 120, 9)))
        );
        assert_eq!(
            bounds(items[1]),
            Some(access::bounds(Rect::new(8, 19, 120, 17)))
        );
        let (node, _) = tree.nodes().find(|(_, n)| n.role() == Role::List).unwrap();
        let heading = tree.get(node).unwrap().children()[0];
        assert_eq!(bounds(heading), None, "the first heading is out of view");
        assert_eq!(
            tree.get(items[2]).unwrap().position_in_set(),
            Some(3),
            "out of view or not, every page is listed"
        );
    }
}
