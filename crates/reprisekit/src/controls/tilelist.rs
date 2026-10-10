//! A list of tall rows, each an icon beside two lines of text, with optional group headers. The
//! host owns the rows; the list owns the first entry on screen, the selection and the scrollbar
//! thumb. [`layout`], [`hit`] and [`draw`] work on the entries, handing each visible row's rect to
//! the host's painter ([`draw_tile`] is the stock one).

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    LINE_H, ScrollHit, scrollbar_hit, scrollbar_parts, scrollbar_pos_at,
};
use crate::controls::form;
#[cfg(test)]
use crate::controls::listview;
use crate::controls::listview::{GROUP_H, draw_group_header, list_rule};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, draw_scrollbar};
use crate::pointer::{Lit, Pointer};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::ColorScheme;
use accesskit::{NodeId, Role};

pub const TILE_H: i32 = 52;

const ICON_X: i32 = 8;
const ICON_PX: i32 = 32;
const TEXT_X: i32 = 9;
const NAME_Y: i32 = 9;
const CAPTION_Y: i32 = 28;
const TEXT_RIGHT: i32 = 8;
const TAG_GAP: i32 = 8;

const HEADER_TITLE: (i32, i32) = (1, 19);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    Header(String),
    Row(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileList {
    pub entries: Vec<Entry>,
    pub top: usize,
    pub selected: Option<usize>,
    pub row_h: i32,
    thumb_drag: Option<i32>,
    bars: Pointer<ScrollHit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub entry: usize,
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub inner: Rect,
    pub slots: Vec<Slot>,
    pub scrollbar: Option<Rect>,
    pub page: usize,
    pub thumb_min: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(usize),
    Header,
    Scroll(ScrollHit),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    None,
    Redraw,
    Row { row: usize, changed: bool },
}

impl TileList {
    pub fn new(row_h: i32) -> TileList {
        TileList {
            entries: Vec::new(),
            top: 0,
            selected: None,
            row_h,
            thumb_drag: None,
            bars: Pointer::new(),
        }
    }

    pub fn rows(n: usize, row_h: i32) -> TileList {
        TileList {
            entries: (0..n).map(Entry::Row).collect(),
            ..TileList::new(row_h)
        }
    }

    fn entry_h(&self, e: &Entry) -> i32 {
        match e {
            Entry::Header(_) => GROUP_H,
            Entry::Row(_) => self.row_h,
        }
    }

    pub fn selected_row(&self) -> Option<usize> {
        match self.entries.get(self.selected?) {
            Some(Entry::Row(i)) => Some(*i),
            _ => None,
        }
    }

    pub fn select_row(&mut self, row: Option<usize>) {
        self.selected = row.and_then(|r| self.entries.iter().position(|e| *e == Entry::Row(r)));
    }

    fn max_top(&self, inner_h: i32) -> usize {
        let mut used = 0;
        for (i, e) in self.entries.iter().enumerate().rev() {
            used += self.entry_h(e);
            if used > inner_h {
                return i + 1;
            }
        }
        0
    }

    pub fn clamp(&mut self, rect: Rect) {
        if self
            .selected
            .is_some_and(|i| !matches!(self.entries.get(i), Some(Entry::Row(_))))
        {
            self.selected = None;
        }
        self.top = self.top.min(self.max_top(rect.inset(2).height));
    }

    pub fn step(&mut self, delta: i32, rect: Rect, cm: &ControlMetrics) -> bool {
        let rows: Vec<usize> = (0..self.entries.len())
            .filter(|&i| matches!(self.entries[i], Entry::Row(_)))
            .collect();
        if rows.is_empty() {
            return false;
        }
        let at = self
            .selected
            .and_then(|s| rows.iter().position(|&i| i == s));
        let to = match at {
            Some(p) => (p as i64 + i64::from(delta)).clamp(0, rows.len() as i64 - 1) as usize,
            None => 0,
        };
        if at == Some(to) {
            return false;
        }
        self.selected = Some(rows[to]);
        self.reveal(rows[to], rect, cm);
        true
    }

    fn reveal(&mut self, i: usize, rect: Rect, cm: &ControlMetrics) {
        let want = if i > 0 && matches!(self.entries[i - 1], Entry::Header(_)) {
            i - 1
        } else {
            i
        };
        if want < self.top {
            self.top = want;
            return;
        }
        while self.top < i && !layout(self, rect, cm).slots.iter().any(|s| s.entry == i) {
            self.top += 1;
        }
    }

    pub fn key(&mut self, key: &form::Key, rect: Rect, cm: &ControlMetrics) -> bool {
        let page = layout(self, rect, cm).page.max(1) as i32;
        let delta = match key {
            form::Key::Up => -1,
            form::Key::Down => 1,
            form::Key::Home => i32::MIN / 2,
            form::Key::End => i32::MAX / 2,
            form::Key::PageUp => -page,
            form::Key::PageDown => page,
            _ => return false,
        };
        self.step(delta, rect, cm)
    }

    pub fn press(&mut self, rect: Rect, x: i32, y: i32, cm: &ControlMetrics) -> Press {
        let lay = layout(self, rect, cm);
        let h = hit(self, &lay, x, y);
        let bar = match h {
            Some(Hit::Scroll(sh)) => Some(sh),
            _ => None,
        };
        self.bars.press(bar, bar == Some(ScrollHit::Thumb));
        match h {
            Some(Hit::Scroll(sh)) => {
                let page = lay.page as i32;
                let delta = match sh {
                    ScrollHit::Back => -1,
                    ScrollHit::Forward => 1,
                    ScrollHit::PageBack => -page,
                    ScrollHit::PageForward => page,
                    ScrollHit::Thumb => {
                        let parts = self.parts(&lay);
                        self.thumb_drag = Some(y - parts.map_or(0, |p| p.thumb.map_or(0, |t| t.y)));
                        return Press::None;
                    }
                };
                if self.scroll(delta, rect) {
                    Press::Redraw
                } else {
                    Press::None
                }
            }
            Some(Hit::Row(entry)) => {
                let Entry::Row(row) = self.entries[entry] else {
                    return Press::None;
                };
                let changed = self.selected != Some(entry);
                self.selected = Some(entry);
                Press::Row { row, changed }
            }
            Some(Hit::Header) | None => Press::None,
        }
    }

    fn parts(&self, lay: &Layout) -> Option<crate::classic::widgets::ScrollParts> {
        lay.scrollbar.map(|sb| {
            scrollbar_parts(
                sb,
                1,
                self.top,
                lay.page,
                self.entries.len(),
                false,
                lay.thumb_min,
            )
        })
    }

    pub fn drag(&mut self, rect: Rect, y: i32, cm: &ControlMetrics) -> bool {
        let Some(grab) = self.thumb_drag else {
            return false;
        };
        let lay = layout(self, rect, cm);
        let Some(parts) = self.parts(&lay) else {
            return false;
        };
        let top = scrollbar_pos_at(&parts, y - grab, lay.page, self.entries.len(), false);
        self.scroll(top as i32 - self.top as i32, rect)
    }

    pub fn bar_lit(&self) -> Lit<ScrollHit> {
        self.bars.lit()
    }

    pub fn release(&mut self) -> bool {
        self.thumb_drag = None;
        let before = self.bars.lit();
        self.bars.release();
        self.bars.lit() != before
    }

    pub fn hover(&mut self, rect: Rect, x: i32, y: i32, cm: &ControlMetrics) -> bool {
        let lay = layout(self, rect, cm);
        let bar = match hit(self, &lay, x, y) {
            Some(Hit::Scroll(sh)) => Some(sh),
            _ => None,
        };
        self.bars.motion(bar)
    }

    pub fn leave(&mut self) -> bool {
        self.bars.leave()
    }

    pub fn scroll(&mut self, rows: i32, rect: Rect) -> bool {
        let max = self.max_top(rect.inset(2).height) as i64;
        let top = (self.top as i64 + i64::from(rows)).clamp(0, max) as usize;
        if top == self.top {
            return false;
        }
        self.top = top;
        true
    }
}

pub fn layout(list: &TileList, rect: Rect, cm: &ControlMetrics) -> Layout {
    let inner = rect.inset(2);
    let total: i32 = list.entries.iter().map(|e| list.entry_h(e)).sum();
    let scrollbar = (total > inner.height)
        .then(|| Rect::new(inner.right() - cm.scroll, inner.y, cm.scroll, inner.height));
    let row_w = inner.width - scrollbar.map_or(0, |s| s.width);
    let mut slots = Vec::new();
    let mut y = inner.y;
    for (i, e) in list.entries.iter().enumerate().skip(list.top) {
        let h = list.entry_h(e);
        if y + h > inner.bottom() {
            break;
        }
        slots.push(Slot {
            entry: i,
            rect: Rect::new(inner.x, y, row_w, h),
        });
        y += h;
    }
    let page = slots.len();
    Layout {
        inner,
        slots,
        scrollbar,
        page,
        thumb_min: cm.thumb_min,
    }
}

pub fn hit(list: &TileList, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if let Some(sb) = lay.scrollbar
        && sb.contains(x, y)
    {
        let parts = scrollbar_parts(
            sb,
            1,
            list.top,
            lay.page,
            list.entries.len(),
            false,
            lay.thumb_min,
        );
        return scrollbar_hit(&parts, x, y, false).map(Hit::Scroll);
    }
    let slot = lay.slots.iter().find(|s| s.rect.contains(x, y))?;
    Some(match list.entries[slot.entry] {
        Entry::Header(_) => Hit::Header,
        Entry::Row(_) => Hit::Row(slot.entry),
    })
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    list: &TileList,
    lay: &Layout,
    px: u32,
    mut row: impl FnMut(&mut Canvas, Rect, usize, bool),
) {
    let Paint { cs, .. } = *p;
    let outer = Rect::new(
        lay.inner.x - 2,
        lay.inner.y - 2,
        lay.inner.width + 4,
        lay.inner.height + 4,
    );
    c.fill_rect(outer, cs.window);
    draw_field_frame(c, &Paint { s: 1, ..*p }, outer, cs.window);
    for slot in &lay.slots {
        match &list.entries[slot.entry] {
            Entry::Header(title) => draw_group_header(
                c,
                text,
                cs,
                slot.rect,
                title,
                HEADER_TITLE,
                &list_rule(cs),
                px,
                1,
            ),
            Entry::Row(i) => row(c, slot.rect, *i, list.selected == Some(slot.entry)),
        }
    }
    if let Some(sb) = lay.scrollbar {
        draw_scrollbar(
            c,
            &Paint { s: 1, ..*p },
            sb,
            list.top,
            lay.page,
            list.entries.len(),
            list.bars.lit(),
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Tile<'a> {
    pub icon: Option<&'a Canvas>,
    pub title: &'a str,
    pub tag: &'a str,
    pub caption: &'a str,
    pub greyed: bool,
}

pub fn draw_tile(
    c: &mut Canvas,
    text: &TextRenderer,
    cs: &ColorScheme,
    r: Rect,
    tile: &Tile,
    selected: bool,
    px: u32,
) {
    let ink = match (selected, tile.greyed) {
        (true, _) => cs.hilight_text,
        (false, true) => cs.gray_text,
        (false, false) => cs.window_text,
    };
    c.fill_rect(r, if selected { cs.hilight } else { cs.window });
    let mut tx = r.x + TEXT_X;
    if let Some(icon) = tile.icon {
        let iy = r.y + (r.height - ICON_PX) / 2;
        c.blit_over(r.x + ICON_X, iy, icon);
        tx = r.x + ICON_X + ICON_PX + TEXT_X;
    }
    let right = r.right() - TEXT_RIGHT;
    let mut title_right = right;
    if !tile.tag.is_empty() {
        let w = text.measure(tile.tag, false, px);
        let tag_x = right - w;
        text.draw(
            c,
            Rect::new(tag_x, r.y + NAME_Y, w, LINE_H),
            tile.tag,
            false,
            ink,
            Align::Left,
            px,
        );
        title_right = tag_x - TAG_GAP;
    }
    let title = text.ellipsize(tile.title, title_right - tx, true, px);
    text.draw(
        c,
        Rect::new(tx, r.y + NAME_Y, title_right - tx, LINE_H),
        &title,
        true,
        ink,
        Align::Left,
        px,
    );
    let caption = text.ellipsize(tile.caption, right - tx, false, px);
    text.draw(
        c,
        Rect::new(tx, r.y + CAPTION_Y, right - tx, LINE_H),
        &caption,
        false,
        ink,
        Align::Left,
        px,
    );
}

pub fn access(
    list: &TileList,
    lay: &Layout,
    tile: impl Fn(usize) -> (String, String),
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::ListBox, lay.inner);
    access::focusable(&mut n, true);
    let id = tree.push(parent, key, n);
    let mut focus = id;
    for slot in &lay.slots {
        match list.entries.get(slot.entry) {
            Some(Entry::Header(h)) => {
                tree.push(id, slot.entry, access::label(slot.rect, h, true));
            }
            Some(Entry::Row(row)) => {
                let (title, caption) = tile(*row);
                let mut o = access::node(Role::ListBoxOption, slot.rect);
                o.set_label(title);
                if !caption.is_empty() {
                    o.set_description(caption);
                }
                let selected = list.selected == Some(slot.entry);
                o.set_selected(selected);
                let oid = tree.push(id, slot.entry, o);
                if selected {
                    focus = oid;
                }
            }
            None => {}
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    const BOX: Rect = Rect::new(0, 0, 396, 5 * TILE_H + 4);

    fn grouped() -> TileList {
        TileList {
            entries: vec![
                Entry::Header("Audio".into()),
                Entry::Row(0),
                Entry::Row(1),
                Entry::Header("Input".into()),
                Entry::Row(2),
            ],
            ..TileList::new(TILE_H)
        }
    }

    #[test]
    fn slots_stack_headers_and_rows_from_the_top() {
        let lay = layout(&grouped(), BOX, &ControlMetrics::CLASSIC);
        let tops: Vec<(usize, i32, i32)> = lay
            .slots
            .iter()
            .map(|s| (s.entry, s.rect.y, s.rect.height))
            .collect();
        assert_eq!(
            tops,
            [
                (0, 2, listview::GROUP_H),
                (1, 2 + 31, TILE_H),
                (2, 2 + 31 + 52, TILE_H),
                (3, 2 + 31 + 104, listview::GROUP_H),
                (4, 2 + 31 + 104 + 31, TILE_H),
            ],
            "a header is 31 px, a row 52, both from the frame's inside"
        );
        assert_eq!(lay.scrollbar, None, "218 px of entries fit in 260");
    }

    #[test]
    fn a_uniform_list_pages_by_whole_rows_and_scrolls_past_them() {
        let list = TileList::rows(7, TILE_H);
        let lay = layout(&list, BOX, &ControlMetrics::CLASSIC);
        assert_eq!(lay.page, 5, "five 52 px rows fit");
        assert_eq!(lay.slots.len(), 5);
        let sb = lay.scrollbar.expect("seven rows outgrow the box");
        assert_eq!(
            sb,
            Rect::new(
                BOX.right() - 2 - ControlMetrics::CLASSIC.scroll,
                2,
                ControlMetrics::CLASSIC.scroll,
                BOX.height - 4
            )
        );
        assert_eq!(
            lay.slots[0].rect.width,
            BOX.width - 4 - ControlMetrics::CLASSIC.scroll,
            "rows stop at the bar"
        );
    }

    #[test]
    fn step_skips_headers_and_scrolls_the_row_into_view() {
        let mut list = grouped();
        let short = Rect::new(0, 0, 300, 2 * TILE_H + 4);
        assert!(
            list.step(1, short, &ControlMetrics::CLASSIC),
            "nothing picked: the first row"
        );
        assert_eq!(list.selected_row(), Some(0));
        list.step(1, short, &ControlMetrics::CLASSIC);
        list.step(1, short, &ControlMetrics::CLASSIC);
        assert_eq!(list.selected_row(), Some(2), "the Input header is skipped");
        let lay = layout(&list, short, &ControlMetrics::CLASSIC);
        assert!(
            lay.slots.iter().any(|s| s.entry == 4),
            "the picked row is on screen"
        );
        assert!(
            !list.step(1, short, &ControlMetrics::CLASSIC),
            "the last row stays picked"
        );
    }

    #[test]
    fn a_press_picks_a_row_and_a_header_picks_nothing() {
        let mut list = grouped();
        assert_eq!(
            list.press(BOX, 40, 2 + 5, &ControlMetrics::CLASSIC),
            Press::None,
            "the Audio header"
        );
        assert_eq!(
            list.press(BOX, 40, 2 + 31 + 52 + 10, &ControlMetrics::CLASSIC),
            Press::Row {
                row: 1,
                changed: true
            }
        );
        assert_eq!(
            list.press(BOX, 40, 2 + 31 + 52 + 10, &ControlMetrics::CLASSIC),
            Press::Row {
                row: 1,
                changed: false
            }
        );
    }

    #[test]
    fn clamp_drops_a_selection_that_is_gone_and_pulls_top_back() {
        let mut list = TileList::rows(9, TILE_H);
        list.top = 4;
        list.select_row(Some(8));
        list.entries.truncate(3);
        list.clamp(BOX);
        assert_eq!(list.top, 0);
        assert_eq!(list.selected, None);
    }

    #[test]
    fn draw_tile_ellipsizes_a_long_title() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let r = Rect::new(0, 0, 160, TILE_H);
        let mut c = Canvas::new(160, TILE_H as u32);
        let tile = Tile {
            icon: None,
            title: "An extremely long device name that cannot fit",
            tag: "",
            caption: "Connected",
            greyed: false,
        };
        draw_tile(&mut c, &text, cs, r, &tile, false, px);
        let window = cs.window.argb();
        for x in 160 - TEXT_RIGHT..160 {
            for y in NAME_Y..NAME_Y + 13 {
                assert_eq!(c.get(x, y), window, "({x}, {y}) is background");
            }
        }
    }

    #[test]
    fn a_selected_tile_fills_with_hilight_and_inks_hilight_text() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let cs = &theme.colors;
        let r = Rect::new(0, 0, 200, TILE_H);
        let mut c = Canvas::new(200, TILE_H as u32);
        let tile = Tile {
            icon: None,
            title: "Mouse",
            tag: "",
            caption: "",
            greyed: false,
        };
        draw_tile(&mut c, &text, cs, r, &tile, true, theme.fonts.size_px);
        assert_eq!(
            c.get(199, 1),
            cs.hilight.argb(),
            "the row fills with hilight"
        );
        let inked = (0..200)
            .flat_map(|x| (NAME_Y..NAME_Y + 13).map(move |y| (x, y)))
            .any(|(x, y)| c.get(x, y) == cs.hilight_text.argb());
        assert!(inked, "the title is drawn in hilight_text");
    }

    #[test]
    fn a_blue_tile_lists_bar_lights_hot_and_pressed() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = theme.look.controls();
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let mut list = TileList::rows(30, 40);
        let r = Rect::new(0, 0, 200, 160);
        let sb = layout(&list, r, &cm)
            .scrollbar
            .expect("30 rows overflow the list");
        let (x, y) = (sb.x + 8, sb.y + 2);
        let drawn = |list: &TileList| {
            let mut c = Canvas::new(200, 160);
            draw(
                &mut c,
                &text,
                &theme.paint(),
                list,
                &layout(list, r, &cm),
                11,
                |_, _, _, _| {},
            );
            c.get(x, y)
        };
        assert!(list.hover(r, x, y, &cm));
        assert_eq!(
            drawn(&list),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Hot,
                8,
                2
            ),
            "the hot up arrow"
        );
        list.press(r, x, y, &cm);
        assert_eq!(
            drawn(&list),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Pressed,
                8,
                2
            ),
            "the held up arrow"
        );
        assert!(list.release(), "back to hot");
        assert!(list.leave());
        assert_eq!(
            drawn(&list),
            crate::look::styled::fixture::blue_pixel(
                crate::style::Part::ScrollArrowUp,
                crate::style::State::Normal,
                8,
                2
            ),
            "the up arrow at rest"
        );
    }
}
