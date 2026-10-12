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

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::{draw_tab, draw_tab_focus};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;
use accesskit::{NodeId, Role};

pub const TAB_H: i32 = 18;
pub const ROW_H: i32 = 18;
const PAD: i32 = 13;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStrip {
    pub labels: Vec<String>,
    pub open: usize,
    pub hot: Option<usize>,
    /// Holds the keyboard focus: Left/Right turn tabs, the open tab wears the ring.
    pub focused: bool,
}

impl TabStrip {
    pub fn new(labels: &[&str], open: usize) -> TabStrip {
        TabStrip {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            open: open.min(labels.len().saturating_sub(1)),
            hot: None,
            focused: false,
        }
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
}
