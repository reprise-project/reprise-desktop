//! A property sheet's tab strip: closed tabs are 18 px tall from the
//! strip's top plus 2, from 2 px in, each as wide as its label plus 13; the
//! open one grows 2 px up and 2 px out each side and runs into the page.
//!
//! Tabs that do not fit between 2 px in from each side of the page wrap onto
//! rows 18 px apart. Rows hold near-equal tab counts (first rows take the
//! spare ones) and share out the width, the first tabs taking the odd pixels.
//! The open tab's row sits next to the page, the others follow above, so the
//! page starts 18 px lower per extra row.
//!
//! While the strip holds focus and the cues are out, a dotted ring (a dot
//! wherever x + y is odd from its top-left) runs 3 px inside the open tab's
//! left, top and right edges, 16 rows tall. Each look inks it its own way
//! ([`crate::look::widgets::draw_tab_focus`]).
//!
//! A [`Mode::Document`] strip holds one row of equal tabs with close boxes, a
//! trailing + with a menu part, and reorders by dragging ([`doc_layout`]).

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::{
    DocTab, draw_doc_adder, draw_doc_tab, draw_page, draw_tab, draw_tab_focus, glass_tabs,
};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

pub const TAB_H: i32 = 18;
pub const ROW_H: i32 = 18;
const PAD: i32 = 13;

/// A document tab's narrowest and widest slot.
pub const DOC_MIN: i32 = 60;
pub const DOC_MAX: i32 = 180;
/// The page edge under a document strip's tabs, which the open tab runs into.
pub const DOC_LIP: i32 = 2;
const CLOSE: i32 = 13;
const ADD_W: i32 = 22;
const MENU_W: i32 = 14;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// A property sheet's tabs, wrapping into rows.
    #[default]
    Sheet,
    /// One row of closable tabs and a new-tab button.
    Document,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdderPart {
    Add,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocHit {
    Tab(usize),
    Close(usize),
    Add,
    Menu,
    Empty,
}

/// A tab held by the pointer: `grab` px right of the tab's left edge, the pointer now at `x`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub tab: usize,
    pub grab: i32,
    pub x: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStrip {
    pub labels: Vec<String>,
    pub open: usize,
    pub hot: Option<usize>,
    /// Holds the keyboard focus: Left/Right turn tabs, the open tab wears the ring.
    pub focused: bool,
    pub mode: Mode,
    pub hot_close: Option<usize>,
    pub hot_adder: Option<AdderPart>,
    /// The lit close box or adder part is held down.
    pub held: bool,
    pub dragging: Option<Drag>,
}

impl TabStrip {
    pub fn new(labels: &[&str], open: usize) -> TabStrip {
        TabStrip::with_mode(
            labels.iter().map(|s| s.to_string()).collect(),
            open,
            Mode::Sheet,
        )
    }

    pub fn document(labels: Vec<String>, open: usize) -> TabStrip {
        TabStrip::with_mode(labels, open, Mode::Document)
    }

    fn with_mode(labels: Vec<String>, open: usize, mode: Mode) -> TabStrip {
        TabStrip {
            open: open.min(labels.len().saturating_sub(1)),
            labels,
            hot: None,
            focused: false,
            mode,
            hot_close: None,
            hot_adder: None,
            held: false,
            dragging: None,
        }
    }

    /// Move tab `from` to index `to`; the open tab stays open wherever it lands.
    pub fn move_tab(&mut self, from: usize, to: usize) {
        let n = self.labels.len();
        if from >= n || to >= n || from == to {
            return;
        }
        let label = self.labels.remove(from);
        self.labels.insert(to, label);
        self.open = if self.open == from {
            to
        } else if from < self.open && self.open <= to {
            self.open - 1
        } else if to <= self.open && self.open < from {
            self.open + 1
        } else {
            self.open
        };
        self.hot = None;
        self.hot_close = None;
    }

    /// Light what `hit` points at: a closed tab, a close box (and its tab) or an adder part.
    /// True when anything lit changed.
    pub fn doc_hover(&mut self, hit: DocHit) -> bool {
        let (tab, close, adder) = match hit {
            DocHit::Tab(i) => (Some(i), None, None),
            DocHit::Close(i) => (Some(i), Some(i), None),
            DocHit::Add => (None, None, Some(AdderPart::Add)),
            DocHit::Menu => (None, None, Some(AdderPart::Menu)),
            DocHit::Empty => (None, None, None),
        };
        let tab = tab.filter(|&i| i != self.open);
        let changed = (tab, close, adder) != (self.hot, self.hot_close, self.hot_adder);
        (self.hot, self.hot_close, self.hot_adder) = (tab, close, adder);
        changed
    }

    /// The dragged tab's pointer is at `x`: it passes every neighbour whose middle its middle
    /// passed. True when the order changed; lay the strip out again before the next call.
    pub fn drag_to(&mut self, l: &DocLayout, x: i32) -> bool {
        let Some(mut d) = self.dragging else {
            return false;
        };
        d.x = x;
        let w = l.tabs.get(d.tab).map_or(0, |r| r.width);
        let mid = x - d.grab + w / 2;
        let to = l
            .tabs
            .iter()
            .enumerate()
            .filter(|&(i, r)| i != d.tab && r.x + r.width / 2 < mid)
            .count();
        let moved = to != d.tab;
        if moved {
            self.move_tab(d.tab, to);
            d.tab = to;
        }
        self.dragging = Some(d);
        moved
    }

    /// The pointer is over tab `over`; only a closed tab lights. True when the lit tab changed.
    pub fn hover(&mut self, over: Option<usize>) -> bool {
        let hot = over.filter(|&i| i != self.open);
        let changed = hot != self.hot;
        self.hot = hot;
        changed
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab: the next tab round, or the previous.
    pub fn cycle(&mut self, backwards: bool) -> bool {
        let n = self.labels.len();
        if n < 2 {
            return false;
        }
        self.open = if backwards {
            (self.open + n - 1) % n
        } else {
            (self.open + 1) % n
        };
        true
    }

    /// Left / Right with the strip focused: the neighbouring tab, stopping at the ends.
    pub fn step(&mut self, right: bool) -> bool {
        let next = if right {
            (self.open + 1).min(self.labels.len().saturating_sub(1))
        } else {
            self.open.saturating_sub(1)
        };
        let changed = next != self.open;
        self.open = next;
        changed
    }
}

/// The labels' widths grouped into rows (ranges of tabs) for a strip `width` wide.
fn rows(widths: &[i32], width: i32) -> Vec<std::ops::Range<usize>> {
    let room = width - 4;
    let n = widths.len();
    let mut greedy = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (i, &w) in widths.iter().enumerate() {
        if i > start && used + w > room {
            greedy.push(start..i);
            (start, used) = (i, 0);
        }
        used += w;
    }
    greedy.push(start..n);
    let count = greedy.len();
    if count < 2 {
        return greedy;
    }
    let (per, spare) = (n / count, n % count);
    let mut even = Vec::with_capacity(count);
    let mut start = 0;
    for r in 0..count {
        let len = per + usize::from(r < spare);
        even.push(start..start + len);
        start += len;
    }
    if even
        .iter()
        .all(|row| widths[row.clone()].iter().sum::<i32>() <= room)
    {
        even
    } else {
        greedy
    }
}

/// How many rows of tabs `strip` takes in a page `width` wide. The page
/// under a strip at `y` starts at `y + 20 + ROW_H * (rows - 1)`.
pub fn row_count(text: &TextRenderer, px: u32, strip: &TabStrip, width: i32) -> usize {
    let widths: Vec<i32> = strip
        .labels
        .iter()
        .map(|l| text.measure(l, false, px) + PAD)
        .collect();
    rows(&widths, width).len()
}

/// The tabs' rects, parallel to the labels: closed ones as laid out, the open
/// one grown. `x`, `y` is the strip's top-left, `width` the page's; tabs and
/// rows are as tall as `u` grows 18 px.
pub fn layout(
    text: &TextRenderer,
    px: u32,
    strip: &TabStrip,
    x: i32,
    y: i32,
    width: i32,
    u: TextUnits,
) -> Vec<Rect> {
    let widths: Vec<i32> = strip
        .labels
        .iter()
        .map(|l| text.measure(l, false, px) + PAD)
        .collect();
    place(&widths, strip.open, x, y, width, u.len(TAB_H))
}

/// [`layout`] for tabs already measured: `widths` include the 13 px pad; tabs and rows are
/// `tab_h` tall.
fn place(widths: &[i32], open: usize, x: i32, y: i32, width: i32, tab_h: i32) -> Vec<Rect> {
    let rows = rows(widths, width);
    let count = rows.len();
    let open_row = rows.iter().position(|r| r.contains(&open)).unwrap_or(0);
    let mut out = vec![Rect::new(0, 0, 0, 0); widths.len()];
    for (r, row) in rows.iter().enumerate() {
        let up = (r + count - open_row) % count;
        let top = y + tab_h * (count - 1 - up) as i32;
        let (fill, odd) = if count > 1 {
            let left = width - 4 - widths[row.clone()].iter().sum::<i32>();
            let k = row.len() as i32;
            (left / k, left % k)
        } else {
            (0, 0)
        };
        let mut tx = x + 2;
        for i in row.clone() {
            let mut w = widths[i] + fill;
            if ((i - row.start) as i32) < odd {
                w += 1;
            }
            out[i] = if i == open {
                Rect::new(tx - 2, top, w + 4, tab_h + 2)
            } else {
                Rect::new(tx, top + 2, w, tab_h)
            };
            tx += w;
        }
    }
    out
}

pub fn hit(tabs: &[Rect], x: i32, y: i32) -> Option<usize> {
    tabs.iter().position(|r| r.contains(x, y))
}

/// The focus ring round the open tab laid out at `open`.
pub fn focus_ring(open: Rect) -> Rect {
    Rect::new(open.x + 3, open.y + 3, open.width - 6, open.height - 4)
}

/// Draw the strip over `page`: closed tabs first, then the open one, with its
/// focus ring while focused and `cues` are out.
#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    strip: &TabStrip,
    tabs: &[Rect],
    page: Rect,
    cues: bool,
    px: u32,
) {
    let s = p.s;
    // Top rows first, so each row's tabs cover the bottoms of the row above.
    let mut order: Vec<usize> = (0..tabs.len().min(strip.labels.len())).collect();
    order.sort_by_key(|&i| tabs[i].y);
    for i in order {
        let (r, label) = (&tabs[i], &strip.labels[i]);
        if i != strip.open {
            let hot = strip.hot == Some(i);
            draw_tab(
                c,
                text,
                p,
                r.scaled(s),
                label,
                false,
                hot,
                page.scaled(s),
                px,
            );
        }
    }
    if let (Some(r), Some(label)) = (tabs.get(strip.open), strip.labels.get(strip.open)) {
        draw_tab(
            c,
            text,
            p,
            r.scaled(s),
            label,
            true,
            false,
            page.scaled(s),
            px,
        );
        if strip.focused && cues {
            draw_tab_focus(c, p, focus_ring(*r).scaled(s));
        }
    }
}

/// The strip's `access` part: a tab list at the [`layout`] rects, the open one selected.
pub fn access(
    strip: &TabStrip,
    tabs: &[Rect],
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> Vec<NodeId> {
    let all = tabs.iter().fold(None, |u: Option<Rect>, r| {
        Some(u.map_or(*r, |u| u.union(r)))
    });
    let list = tree.push(
        parent,
        key,
        access::node(Role::TabList, all.unwrap_or_default()),
    );
    strip
        .labels
        .iter()
        .zip(tabs)
        .enumerate()
        .map(|(i, (label, r))| {
            let mut n = access::named(Role::Tab, *r, label);
            n.set_selected(i == strip.open);
            tree.push(list, i, n)
        })
        .collect()
}

/// A document strip laid out: tab rects parallel to the labels (the open one grown), their close
/// boxes and the new-tab button's two parts. Tabs from `add.x` on are cut off.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocLayout {
    pub area: Rect,
    pub tabs: Vec<Rect>,
    pub closes: Vec<Rect>,
    pub add: Rect,
    pub menu: Rect,
}

/// How tall a document strip is: its tabs as tall as `u` grows 18 px, the open one 2 px more,
/// and the [`DOC_LIP`] under them.
pub fn doc_height(u: TextUnits) -> i32 {
    u.len(TAB_H) + 2 + DOC_LIP
}

/// Lay a document strip out on one row across `area`: equal tabs between [`DOC_MIN`] and
/// [`DOC_MAX`]; leading tabs scroll off (empty rects) until the open one ends left of the button.
pub fn doc_layout(
    _text: &TextRenderer,
    _px: u32,
    strip: &TabStrip,
    area: Rect,
    u: TextUnits,
) -> DocLayout {
    let tab_h = u.len(TAB_H);
    let (min, max) = (u.len(DOC_MIN), u.len(DOC_MAX));
    let adder_w = ADD_W + MENU_W;
    let room = area.width - 4 - adder_w;
    let n = strip.labels.len() as i32;
    let (w, odd) = match n {
        0 => (0, 0),
        _ if room / n >= max => (max, 0),
        _ if room / n < min => (min, 0),
        _ => (room / n, room % n),
    };
    let top = area.y + 2;
    let width = |i: i32| w + i32::from(i < odd);
    let limit = area.right() - 2 - adder_w;
    let open = (strip.open as i32).min(n - 1);
    // Leading tabs scroll off until the open one (2 px wider grown) ends left of the button.
    let mut first = 0;
    while first < open && area.x + 2 + (first..=open).map(width).sum::<i32>() + 2 > limit {
        first += 1;
    }
    let mut x = area.x + 2;
    let mut tabs = vec![Rect::new(0, 0, 0, 0); first as usize];
    let mut closes = tabs.clone();
    tabs.reserve(n as usize);
    for i in first..n {
        let w = width(i);
        let slot = Rect::new(x, top, w, tab_h);
        let lift = if i as usize == strip.open { 2 } else { 0 };
        tabs.push(if lift > 0 {
            Rect::new(x - 2, area.y, w + 4, tab_h + 2)
        } else {
            slot
        });
        closes.push(Rect::new(
            slot.right() - 3 - CLOSE,
            slot.y + 1 + (tab_h - 1 - CLOSE) / 2 - lift,
            CLOSE,
            CLOSE,
        ));
        x += w;
    }
    let ax = x.min(limit);
    DocLayout {
        area,
        tabs,
        closes,
        add: Rect::new(ax, top, ADD_W, tab_h),
        menu: Rect::new(ax + ADD_W, top, MENU_W, tab_h),
    }
}

/// What the point `x`, `y` is over: the new-tab button's parts, then a close box, then a tab
/// (the taller open one where it overlaps a neighbour).
pub fn doc_hit(l: &DocLayout, x: i32, y: i32) -> DocHit {
    if l.add.contains(x, y) {
        return DocHit::Add;
    }
    if l.menu.contains(x, y) {
        return DocHit::Menu;
    }
    if x >= l.add.x {
        return DocHit::Empty;
    }
    if let Some(i) = l.closes.iter().position(|r| r.contains(x, y)) {
        return DocHit::Close(i);
    }
    l.tabs
        .iter()
        .enumerate()
        .filter(|(_, r)| r.contains(x, y))
        .max_by_key(|(_, r)| r.height)
        .map_or(DocHit::Empty, |(i, _)| DocHit::Tab(i))
}

/// Draw a document strip laid out as `l`, a dragged tab last and following the pointer. With
/// `on_glass` a look with glass tabs draws them and no lip.
pub fn doc_draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    strip: &TabStrip,
    l: &DocLayout,
    on_glass: bool,
    px: u32,
) {
    let s = p.s;
    let glass = glass_tabs(p, on_glass);
    let lip_y = l.area.y + l.area.height - DOC_LIP;
    let lip = Rect::new(l.area.x, lip_y, l.area.width, DOC_LIP);
    if !glass {
        let page = Rect::new(lip.x, lip.y, lip.width, 4 * TAB_H).scaled(s);
        c.with_clip(lip.scaled(s), |c| draw_page(c, p, page));
    }
    let row = Rect::new(l.area.x, l.area.y, l.add.x - l.area.x, l.area.height).scaled(s);
    let dragged = strip.dragging.map(|d| d.tab).filter(|&i| i < l.tabs.len());
    let tab = |c: &mut Canvas, i: usize, dx: i32| {
        let close = match (strip.hot_close == Some(i), strip.held) {
            (true, true) => Interaction::Pressed,
            (true, false) => Interaction::Hot,
            _ => Interaction::Idle,
        };
        let shift = |r: Rect| Rect::new(r.x + dx, r.y, r.width, r.height).scaled(s);
        let t = DocTab {
            r: shift(l.tabs[i]),
            label: &strip.labels[i],
            open: i == strip.open,
            hot: strip.hot == Some(i),
            close: shift(l.closes[i]),
            close_ptr: close,
        };
        c.with_clip(row, |c| {
            draw_doc_tab(c, text, p, &t, lip.scaled(s), glass, px)
        });
    };
    for i in (0..l.tabs.len().min(strip.labels.len()))
        .filter(|&i| i != strip.open && Some(i) != dragged && !l.tabs[i].is_empty())
    {
        tab(c, i, 0);
    }
    let part = |which| match (strip.hot_adder == Some(which), strip.held) {
        (true, true) => Interaction::Pressed,
        (true, false) => Interaction::Hot,
        _ => Interaction::Idle,
    };
    draw_doc_adder(
        c,
        text,
        p,
        l.add.scaled(s),
        l.menu.scaled(s),
        part(AdderPart::Add),
        part(AdderPart::Menu),
        glass,
        px,
    );
    if strip.open < l.tabs.len().min(strip.labels.len()) && Some(strip.open) != dragged {
        tab(c, strip.open, 0);
    }
    if let (Some(i), Some(d)) = (dragged, strip.dragging) {
        let r = l.tabs[i];
        let x = (d.x - d.grab).clamp(l.area.x, (l.add.x - r.width).max(l.area.x));
        tab(c, i, x - r.x);
    }
}

/// The document strip's `access` part: a tab list holding a tab per label (the open one
/// selected), each with its close button, then the new-tab button and its menu button.
pub fn doc_access(
    strip: &TabStrip,
    l: &DocLayout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> Vec<NodeId> {
    let list = tree.push(parent, key, access::node(Role::TabList, l.area));
    let tabs = strip
        .labels
        .iter()
        .zip(l.tabs.iter().zip(&l.closes))
        .enumerate()
        .map(|(i, (label, (r, close)))| {
            let mut n = access::named(Role::Tab, *r, label);
            n.set_selected(i == strip.open);
            let id = tree.push(list, i, n);
            tree.push(
                id,
                "close",
                access::named(Role::Button, *close, "Close Tab"),
            );
            id
        })
        .collect();
    tree.push(list, "add", access::named(Role::Button, l.add, "New Tab"));
    tree.push(
        list,
        "menu",
        access::named(Role::Button, l.menu, "New Tab Menu"),
    );
    tabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    #[test]
    fn tabs_abut_and_the_open_one_grows() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let strip = TabStrip::new(&["Themes", "Desktop", "Screen Saver"], 0);
        let tabs = layout(&text, px, &strip, 9, 29, 386, TextUnits::ONE);
        let w0 = text.measure("Themes", false, px) + 13;
        assert_eq!(tabs[0], Rect::new(9, 29, w0 + 4, 20));
        assert_eq!(tabs[1].x, 11 + w0);
        assert_eq!((tabs[1].y, tabs[1].height), (31, 18));
        assert_eq!(tabs[2].x, tabs[1].right());
        assert_eq!(hit(&tabs, tabs[1].x + 3, 40), Some(1));
        let mut s2 = strip.clone();
        assert!(s2.cycle(true));
        assert_eq!(s2.open, 2, "wraps to the last tab");
        assert!(!s2.step(true));
        assert!(s2.step(false) && s2.open == 1);
        assert!(s2.cycle(false) && s2.open == 2);
        assert!(s2.cycle(false) && s2.open == 0);
    }

    /// Seven tabs on a 401 px page: four on the open tab's row, three above,
    /// both spread to 2 px in from each side, the odd pixel to each row's first tab.
    const SEVEN: [i32; 7] = [50, 89, 59, 62, 87, 103, 50];

    #[test]
    fn tabs_that_overflow_wrap_into_rows_spread_to_the_width() {
        assert_eq!(
            place(&SEVEN, 0, 8, 0, 401, TAB_H),
            vec![
                Rect::new(8, 18, 89, 20),
                Rect::new(95, 20, 123, 18),
                Rect::new(218, 20, 93, 18),
                Rect::new(311, 20, 96, 18),
                Rect::new(10, 2, 140, 18),
                Rect::new(150, 2, 155, 18),
                Rect::new(305, 2, 102, 18),
            ]
        );
    }

    #[test]
    fn the_open_tabs_row_moves_next_to_the_page() {
        assert_eq!(
            place(&SEVEN, 4, 8, 0, 401, TAB_H),
            vec![
                Rect::new(10, 2, 85, 18),
                Rect::new(95, 2, 123, 18),
                Rect::new(218, 2, 93, 18),
                Rect::new(311, 2, 96, 18),
                Rect::new(8, 18, 144, 20),
                Rect::new(150, 20, 155, 18),
                Rect::new(305, 20, 102, 18),
            ]
        );
    }

    /// Where the rows meet: the upper row runs 2 px under the open tab's top,
    /// the lower row's closed tabs cover it from 20 px down.
    #[test]
    fn the_lower_row_is_drawn_over_the_upper() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let strip = TabStrip::new(&[""; 7], 0);
        let tabs = place(&SEVEN, 0, 8, 0, 401, TAB_H);
        let page = Rect::new(8, 38, 401, 60);
        let mut c = Canvas::new(420, 100);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::look::widgets::draw_page(&mut c, &classic_paint(cs, 1), page);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &strip,
            &tabs,
            page,
            false,
            11,
        );
        assert_eq!(c.get(10, 18), cs.button_hilight.argb());
        assert_eq!(c.get(148, 18), cs.button_shadow.argb());
        assert_eq!(c.get(149, 18), cs.button_dk_shadow.argb());
        assert_eq!(c.get(97, 20), cs.button_hilight.argb());
        assert_eq!(c.get(149, 20), cs.button_hilight.argb());
        assert_eq!(c.get(97, 38), cs.button_hilight.argb());
        assert_eq!(c.get(50, 38), cs.button_face.argb());
    }

    #[test]
    fn row_count_follows_the_page_width() {
        let text = TextRenderer::load(&FontConfig::default());
        let strip = TabStrip::new(&["Themes", "Desktop", "Screen Saver"], 0);
        assert_eq!(row_count(&text, 11, &strip, 386), 1);
        assert_eq!(row_count(&text, 11, &strip, 60), 3);
    }

    #[test]
    fn rows_even_out_unless_a_row_would_overflow() {
        // Six fit, but four and three fit too and are closer to even.
        assert_eq!(rows(&[58, 80, 66, 50, 48, 66, 65], 386), vec![0..4, 4..7]);
        // Three and two would put 210 px on a 200 px row.
        assert_eq!(rows(&[100, 100, 10, 10, 100], 204), vec![0..2, 2..5]);
        assert_eq!(rows(&[50, 50], 204), vec![0..2]);
        assert_eq!(rows(&[], 204), vec![0..0]);
    }

    #[test]
    fn draws_the_open_tab_over_the_page_edge() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let strip = TabStrip::new(&["Themes", "Desktop"], 1);
        let tabs = layout(&text, 11, &strip, 9, 29, 386, TextUnits::ONE);
        let page = Rect::new(9, 49, 386, 100);
        let mut c = Canvas::new(404, 160);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::look::widgets::draw_page(&mut c, &classic_paint(cs, 1), page);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &strip,
            &tabs,
            page,
            false,
            11,
        );
        let open = tabs[1];
        assert_eq!(c.get(open.x + 5, 49), cs.button_face.argb());
        assert_eq!(c.get(tabs[0].x + 5, 49), cs.button_hilight.argb());
        assert_eq!(
            c.get(open.right() - 2, open.y + 1),
            cs.button_dk_shadow.argb()
        );
        assert_eq!(c.get(open.right() - 1, open.y + 1), cs.button_face.argb());
        assert_eq!(c.get(open.x, open.y + 5), cs.button_hilight.argb());
    }

    /// In Classic both labels' baselines are 13 rows down; the closed one is 2 rows below the open one's.
    #[test]
    fn closed_and_open_labels_sit_13_rows_under_their_tops() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let strip = TabStrip::new(&["Volume", "Sounds", "Audio"], 0);
        let tabs = layout(&text, 11, &strip, 9, 29, 386, TextUnits::ONE);
        let page = Rect::new(9, 49, 386, 100);
        let mut c = Canvas::new(404, 160);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::look::widgets::draw_page(&mut c, &classic_paint(cs, 1), page);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &strip,
            &tabs,
            page,
            false,
            11,
        );
        let baseline = |r: Rect| {
            (r.y..r.bottom())
                .filter(|&y| (r.x + 3..r.right() - 3).any(|x| c.get(x, y) == cs.button_text.argb()))
                .max()
        };
        for (i, r) in tabs.iter().enumerate() {
            assert_eq!(baseline(*r), Some(r.y + 13), "tab {i}");
        }
        assert_eq!(tabs[1].y, tabs[0].y + 2);
    }

    /// A two-tab strip over its page in look `i` at scale `s`, first tab open; returns the open tab's rect.
    fn strip_in(i: Interface, s: i32, focused: bool, cues: bool) -> (Canvas, Rect) {
        let one = interfaces::theme_for(i);
        let mut strip = TabStrip::new(&["Themes", "Desktop"], 0);
        strip.focused = focused;
        let tabs = layout(
            &TextRenderer::load(&one.fonts),
            one.fonts.size_px,
            &strip,
            9,
            9,
            200,
            TextUnits::ONE,
        );
        let theme = one.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let page = Rect::new(9, 29, 200, 60);
        let p = theme.paint();
        let mut c = Canvas::new((220 * s) as u32, (100 * s) as u32);
        c.fill_rect(c.bounds(), p.cs.button_face);
        crate::look::widgets::draw_page(&mut c, &p, page.scaled(s));
        draw(&mut c, &text, &p, &strip, &tabs, page, cues, px);
        (c, tabs[0])
    }

    fn diff(a: &Canvas, b: &Canvas) -> Vec<(i32, i32)> {
        let (w, h) = (a.width as i32, a.height as i32);
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| a.get(x, y) != b.get(x, y))
            .collect()
    }

    fn dots(ring: Rect) -> Vec<(i32, i32)> {
        let (w, h) = (ring.width, ring.height);
        let mut out: Vec<(i32, i32)> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .filter(|&(x, y)| (x == 0 || y == 0 || x == w - 1 || y == h - 1) && (x + y) % 2 == 1)
            .map(|(x, y)| (ring.x + x, ring.y + y))
            .collect();
        out.sort_by_key(|&(x, y)| (y, x));
        out
    }

    #[test]
    fn a_focused_strip_rings_its_open_tab_in_every_look() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let (plain, open) = strip_in(i, 1, false, true);
            let (lit, _) = strip_in(i, 1, true, true);
            let ring = focus_ring(open);
            assert_eq!(
                (ring.x - open.x, ring.y - open.y, ring.right(), ring.height),
                (3, 3, open.right() - 3, 16),
                "{i:?}: 3 px in from the open tab's sides and top, 16 rows"
            );
            assert_eq!(diff(&plain, &lit), dots(ring), "{i:?}: the ring's dots");
            assert!(
                diff(&plain, &strip_in(i, 1, true, false).0).is_empty(),
                "{i:?}: no ring without the cues"
            );
            let face = interfaces::theme_for(i).colors.button_face.argb() & 0xFF_FFFF;
            for (x, y) in dots(ring) {
                let want = if i == Interface::Classic {
                    0xFF00_0000
                } else {
                    plain.get(x, y) ^ face
                };
                assert_eq!(lit.get(x, y), want, "{i:?}: the dot at ({x}, {y})");
            }
        }
    }

    /// Classic's dots are black; Glass's are the white tab body XORed with
    /// its F0F0F0 face.
    #[test]
    fn the_ring_is_black_in_classic_and_dark_grey_on_a_glass_tab() {
        let (lit, open) = strip_in(Interface::Classic, 1, true, true);
        assert_eq!(lit.get(open.x + 4, open.y + 3), 0xFF00_0000);
        assert_eq!(lit.get(open.x + 3, open.y + 4), 0xFF00_0000);
        let (plain, _) = strip_in(Interface::Classic, 1, false, true);
        assert_eq!(
            lit.get(open.x + 3, open.y + 3),
            plain.get(open.x + 3, open.y + 3)
        );
        let (lit, open) = strip_in(Interface::Glass, 1, true, true);
        assert_eq!(lit.get(open.x + 4, open.y + 3), 0xFF10_1110);
    }

    #[test]
    fn the_ring_at_paint_scale_2_is_its_scale_1_ring_doubled() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let one = diff(
                &strip_in(i, 1, false, true).0,
                &strip_in(i, 1, true, true).0,
            );
            let two = diff(
                &strip_in(i, 2, false, true).0,
                &strip_in(i, 2, true, true).0,
            );
            let mut doubled: Vec<(i32, i32)> = one
                .iter()
                .flat_map(|&(x, y)| {
                    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| (2 * x + dx, 2 * y + dy))
                })
                .collect();
            doubled.sort_by_key(|&(x, y)| (y, x));
            assert_eq!(two, doubled, "{i:?}: every dot doubles");
        }
    }

    fn doc_text() -> (TextRenderer, u32) {
        let theme = Theme::default();
        (TextRenderer::load(&theme.fonts), theme.fonts.size_px)
    }

    fn labels(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("tab {i}")).collect()
    }

    #[test]
    fn doc_single_row() {
        let (text, px) = doc_text();
        let strip = TabStrip::document(labels(30), 4);
        let area = Rect::new(10, 5, 400, 22);
        let l = doc_layout(&text, px, &strip, area, TextUnits::ONE);
        assert_eq!(l.tabs.len(), 30);
        for (i, r) in l.tabs.iter().enumerate() {
            let slot_y = if i == 4 { area.y } else { area.y + 2 };
            assert_eq!(r.y, slot_y, "tab {i} stays on the one row");
            let slot_w = if i == 4 { r.width - 4 } else { r.width };
            assert!(
                slot_w >= DOC_MIN,
                "tab {i} is {slot_w} px, under the minimum"
            );
        }
        let adder = l.add.union(&l.menu);
        assert!(
            adder.x >= area.x && adder.right() <= area.right(),
            "the adder {adder:?} stays inside {area:?}"
        );
        assert_eq!(adder.y, area.y + 2, "the adder sits on the row");
    }

    #[test]
    fn doc_tabs_share_width() {
        let (text, px) = doc_text();
        let strip = TabStrip::document(
            vec![
                "~/projects".into(),
                "htop".into(),
                "a much longer title".into(),
            ],
            0,
        );
        let wide = doc_layout(&text, px, &strip, Rect::new(0, 0, 600, 22), TextUnits::ONE);
        let narrow = doc_layout(&text, px, &strip, Rect::new(0, 0, 300, 22), TextUnits::ONE);
        let slot = |l: &DocLayout, i: usize| {
            if i == 0 {
                l.tabs[i].width - 4
            } else {
                l.tabs[i].width
            }
        };
        for l in [&wide, &narrow] {
            let ws: Vec<i32> = (0..3).map(|i| slot(l, i)).collect();
            let (lo, hi) = (*ws.iter().min().unwrap(), *ws.iter().max().unwrap());
            assert!(hi - lo <= 1, "widths {ws:?} within 1 px of each other");
            assert!(hi <= DOC_MAX, "widths {ws:?} capped at {DOC_MAX}");
        }
        assert_eq!(
            slot(&wide, 1),
            DOC_MAX,
            "600 px holds three tabs at the maximum"
        );
        assert!(slot(&narrow, 1) < DOC_MAX, "300 px shares its room out");
        assert_eq!(
            wide.add.x,
            wide.tabs[2].right(),
            "the adder follows the last tab"
        );
    }

    #[test]
    fn move_tab_keeps_open() {
        let mut strip = TabStrip::document(labels(5), 1);
        strip.move_tab(1, 3);
        assert_eq!(strip.open, 3);
        assert_eq!(strip.labels, ["tab 0", "tab 2", "tab 3", "tab 1", "tab 4"]);
        strip.move_tab(4, 0);
        assert_eq!(
            strip.open, 4,
            "the open tab moves right as one lands before it"
        );
        strip.move_tab(0, 4);
        assert_eq!(strip.open, 3, "and back left as one leaves from before it");
        assert_eq!(strip.labels[strip.open], "tab 1");
    }

    #[test]
    fn doc_hit_close_before_tab() {
        let (text, px) = doc_text();
        let strip = TabStrip::document(labels(4), 0);
        let l = doc_layout(&text, px, &strip, Rect::new(0, 0, 600, 22), TextUnits::ONE);
        let b = l.closes[2];
        assert!(
            l.tabs[2].contains(b.x, b.y),
            "the close box lies on its tab"
        );
        assert_eq!(doc_hit(&l, b.x + 6, b.y + 6), DocHit::Close(2));
        assert_eq!(
            doc_hit(&l, l.tabs[2].x + 4, l.tabs[2].y + 8),
            DocHit::Tab(2)
        );
        assert_eq!(doc_hit(&l, l.add.x + 4, l.add.y + 8), DocHit::Add);
        assert_eq!(doc_hit(&l, l.menu.x + 4, l.menu.y + 8), DocHit::Menu);
        assert_eq!(doc_hit(&l, l.menu.right(), 10), DocHit::Empty);
        assert_eq!(doc_hit(&l, l.tabs[1].x + 4, 21), DocHit::Empty, "the lip");
    }

    #[test]
    fn doc_open_tab_always_visible() {
        let (text, px) = doc_text();
        let area = Rect::new(0, 0, 400, 22);
        let mut strip = TabStrip::document(labels(30), 29);
        let l = doc_layout(&text, px, &strip, area, TextUnits::ONE);
        let open = l.tabs[29];
        assert!(
            open.x >= area.x && open.right() - 2 <= l.add.x,
            "the open tab's slot in {open:?} lies left of the adder at {}",
            l.add.x
        );
        assert_eq!(doc_hit(&l, open.x + 6, open.y + 8), DocHit::Tab(29));
        assert!(
            l.tabs[0].is_empty() && l.closes[0].is_empty(),
            "the leading tabs scroll off"
        );
        assert!(!l.tabs[28].is_empty(), "the tab before the open one shows");
        strip.open = 0;
        let first = doc_layout(&text, px, &strip, area, TextUnits::ONE);
        assert_eq!(
            first.tabs[0],
            Rect::new(0, 0, DOC_MIN + 4, 20),
            "open = 0 lays out from the left"
        );
        assert_eq!(first.tabs[1].x, 2 + DOC_MIN);
    }

    #[test]
    fn tabs_past_the_adder_are_cut_and_never_hit() {
        let (text, px) = doc_text();
        let strip = TabStrip::document(labels(30), 0);
        let l = doc_layout(&text, px, &strip, Rect::new(0, 0, 400, 22), TextUnits::ONE);
        let last = l.tabs[29];
        assert!(last.x >= l.add.x, "the last tab runs past the adder");
        assert_eq!(doc_hit(&l, l.add.x + 2, l.add.y + 8), DocHit::Add);
        assert_eq!(doc_hit(&l, last.x + 4, last.y + 8), DocHit::Empty);
    }

    #[test]
    fn a_dragged_tab_moves_past_a_neighbours_middle() {
        let (text, px) = doc_text();
        let mut strip = TabStrip::document(labels(4), 1);
        let l = doc_layout(&text, px, &strip, Rect::new(0, 0, 600, 22), TextUnits::ONE);
        let grab = 10;
        strip.dragging = Some(Drag {
            tab: 1,
            grab,
            x: l.tabs[1].x + grab,
        });
        let mid2 = l.tabs[2].x + l.tabs[2].width / 2;
        let w = l.tabs[1].width;
        assert!(
            !strip.drag_to(&l, mid2 - w / 2 - 2 + grab),
            "short of tab 2's middle"
        );
        assert_eq!(strip.labels[1], "tab 1");
        assert!(
            strip.drag_to(&l, mid2 - w / 2 + 2 + grab),
            "past tab 2's middle"
        );
        assert_eq!(strip.labels, ["tab 0", "tab 2", "tab 1", "tab 3"]);
        assert_eq!((strip.open, strip.dragging.map(|d| d.tab)), (2, Some(2)));
    }

    #[test]
    fn hovering_lights_one_part_at_a_time() {
        let mut strip = TabStrip::document(labels(3), 0);
        assert!(strip.doc_hover(DocHit::Close(1)));
        assert_eq!(
            (strip.hot, strip.hot_close, strip.hot_adder),
            (Some(1), Some(1), None)
        );
        assert!(strip.doc_hover(DocHit::Menu));
        assert_eq!(
            (strip.hot, strip.hot_close, strip.hot_adder),
            (None, None, Some(AdderPart::Menu))
        );
        assert!(!strip.doc_hover(DocHit::Menu));
        assert!(strip.doc_hover(DocHit::Tab(0)));
        assert_eq!(
            (strip.hot, strip.hot_close, strip.hot_adder),
            (None, None, None),
            "the open tab never lights"
        );
    }

    #[test]
    fn a_sheet_strip_is_a_sheet() {
        let s = TabStrip::new(&["a", "b"], 0);
        assert_eq!(s.mode, Mode::Sheet);
        assert_eq!(TabStrip::document(labels(2), 1).mode, Mode::Document);
    }

    #[test]
    fn doc_access_names_tabs_their_close_boxes_and_the_adder() {
        let (text, px) = doc_text();
        let strip = TabStrip::document(labels(2), 1);
        let l = doc_layout(&text, px, &strip, Rect::new(0, 0, 600, 22), TextUnits::ONE);
        let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 600, 22)));
        let tabs = doc_access(&strip, &l, &mut tree, access::ROOT, "tabs");
        assert_eq!(tabs.len(), 2);
        let node = |id| tree.nodes().find(|(n, _)| *n == id).unwrap().1.clone();
        assert_eq!(node(tabs[1]).role(), Role::Tab);
        assert_eq!(node(tabs[1]).is_selected(), Some(true));
        assert_eq!(
            node(tabs[1]).children().len(),
            1,
            "a close button under each tab"
        );
        let buttons = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::Button)
            .count();
        assert_eq!(buttons, 4, "two close boxes, + and the menu part");
    }

    /// One strip per state, labels empty so the hashes hold on any font.
    fn doc_scenes() -> Vec<(&'static str, TabStrip)> {
        let base = TabStrip::document(vec![String::new(); 3], 0);
        let with = |f: &dyn Fn(&mut TabStrip)| {
            let mut s = base.clone();
            f(&mut s);
            s
        };
        vec![
            ("open and closed", base.clone()),
            ("hot", with(&|s| s.hot = Some(1))),
            (
                "hot close",
                with(&|s| {
                    s.hot = Some(2);
                    s.hot_close = Some(2);
                }),
            ),
            (
                "held close",
                with(&|s| {
                    s.hot_close = Some(0);
                    s.held = true;
                }),
            ),
            ("adder hot", with(&|s| s.hot_adder = Some(AdderPart::Add))),
            (
                "adder held",
                with(&|s| {
                    s.hot_adder = Some(AdderPart::Menu);
                    s.held = true;
                }),
            ),
            (
                "dragged",
                with(&|s| {
                    s.dragging = Some(Drag {
                        tab: 0,
                        grab: 5,
                        x: 70,
                    })
                }),
            ),
        ]
    }

    fn doc_strip_in(theme: &Theme, strip: &TabStrip, s: i32, on_glass: bool) -> Canvas {
        let one = TextRenderer::load(&theme.fonts);
        let area = Rect::new(2, 2, 300, doc_height(TextUnits::ONE));
        let l = doc_layout(&one, theme.fonts.size_px, strip, area, TextUnits::ONE);
        let theme = theme.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut c = Canvas::new((304 * s) as u32, ((area.bottom() + 2) * s) as u32);
        if !on_glass {
            c.fill_rect(c.bounds(), p.cs.button_face);
        }
        doc_draw(&mut c, &text, &p, strip, &l, on_glass, theme.fonts.size_px);
        c
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    fn doc_looks() -> Vec<(String, Theme, bool)> {
        let mut looks = vec![("Classic".to_string(), Theme::default(), false)];
        for v in crate::blue::Variant::ALL {
            looks.push((format!("Blue {v:?}"), interfaces::blue_theme(v), false));
        }
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let g = crate::glass::Settings {
                mode,
                ..Default::default()
            };
            let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
            looks.push((format!("Glass {mode:?} on glass"), t.clone(), true));
            looks.push((format!("Glass {mode:?} on face"), t, false));
        }
        looks
    }

    #[test]
    fn doc_tab_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for (name, theme, on_glass) in doc_looks() {
            for (state, strip) in doc_scenes() {
                for s in [1, 2] {
                    let h = fnv(&doc_strip_in(&theme, &strip, s, on_glass));
                    got.push((format!("{name} {state} {s}"), h));
                }
            }
        }
        crate::pins::check("doc_tabs", &got);
    }

    #[test]
    fn every_doc_state_draws_differently() {
        for (name, theme, on_glass) in doc_looks() {
            for s in [1, 2] {
                let drawn: Vec<(&str, Canvas)> = doc_scenes()
                    .into_iter()
                    .map(|(state, strip)| (state, doc_strip_in(&theme, &strip, s, on_glass)))
                    .collect();
                for (i, (a, ca)) in drawn.iter().enumerate() {
                    for (b, cb) in &drawn[i + 1..] {
                        let classic_hot =
                            name == "Classic" && (*a, *b) == ("open and closed", "hot");
                        if !classic_hot {
                            assert!(ca != cb, "{name} at {s}: {a} and {b} draw alike");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn sheet_and_document_strips_lay_the_same_tab_body() {
        let theme = Theme::default();
        let strip = TabStrip::document(vec![String::new(); 2], 0);
        let c = doc_strip_in(&theme, &strip, 1, false);
        let cs = &theme.colors;
        let (text, px) = doc_text();
        let l = doc_layout(&text, px, &strip, Rect::new(2, 2, 300, 22), TextUnits::ONE);
        let open = l.tabs[0];
        assert_eq!(
            c.get(open.x, open.y + 5),
            cs.button_hilight.argb(),
            "the open tab's left edge"
        );
        assert_eq!(
            c.get(open.x + 5, open.bottom()),
            cs.button_face.argb(),
            "the open tab runs into the lip"
        );
        assert_eq!(
            c.get(l.tabs[1].x + 5, open.bottom()),
            cs.button_hilight.argb(),
            "the lip's top line under a closed tab"
        );
    }
}
