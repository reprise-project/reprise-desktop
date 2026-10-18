//! The emoji picker: a flyout panel ([`crate::flyout`]) with a filter field on top, a sunken grid of
//! emoji under it and the highlighted one's name at the bottom. With the field empty the grid starts
//! with the recently used ones, then the whole catalog in Unicode order; typing keeps the emoji whose
//! name or shortcode has a word starting with each word typed, whole words first. Arrows and Page
//! Up/Down move the highlight, Enter picks, Escape closes. Pure: the host types the pick.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{
    FIELD_PAD, ROW_H as FIELD_H, ScrollHit, access_edit, draw_edit, edit_first_visible,
    scrollbar_hit, scrollbar_parts,
};
use crate::controls::metrics::ControlMetrics;
use crate::emoji::{EmojiFont, Entry};
use crate::flyout;
use crate::look::widgets::{Interaction, draw_edit_field, draw_field_frame, draw_scrollbar};
use crate::raster::{Canvas, Rect};
use crate::selection::scroll_into_view;
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey, Preedit, TextEdit};
use crate::theme::Theme;
use accesskit::{Node, NodeId, Role};

pub const COLS: usize = 10;
pub const ROWS: usize = 6;
pub const CELL: i32 = 30;
pub const GLYPH: i32 = 22;
const PAD: i32 = 4;
const NAME_H: i32 = 16;
pub const NO_MATCH: &str = "No emoji match.";
pub const RECENT: usize = 2 * COLS;

#[derive(Clone, Debug, PartialEq)]
pub struct EmojiPicker {
    /// Private so the grid always matches it: every write goes through `key` or `paste`.
    edit: TextEdit,
    entries: Vec<Entry>,
    recent: usize,
    shown: Vec<usize>,
    pub selected: Option<usize>,
    pub top: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(EditKey),
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    Escape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Pick(&'static str),
    Close,
}

impl EmojiPicker {
    /// The picker over `recent` (newest first) and `catalog`, the first cell highlighted.
    pub fn new(recent: Vec<Entry>, catalog: Vec<Entry>) -> EmojiPicker {
        let n = recent.len();
        let mut entries = recent;
        entries.extend(catalog);
        let mut m = EmojiPicker {
            edit: TextEdit::new("", false),
            entries,
            recent: n,
            shown: Vec::new(),
            selected: None,
            top: 0,
        };
        m.refilter();
        m
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    pub fn shown(&self) -> impl Iterator<Item = &Entry> {
        self.shown.iter().map(|&i| &self.entries[i])
    }

    pub fn len(&self) -> usize {
        self.shown.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        self.selected.map(|i| &self.entries[self.shown[i]])
    }

    fn rows(&self) -> usize {
        self.shown.len().div_ceil(COLS)
    }

    /// The field empty, every entry; else the catalog's matches, best first (see `score`). The highlight goes back to the first cell.
    fn refilter(&mut self) {
        let words: Vec<String> = self
            .edit
            .text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect();
        self.shown = if words.is_empty() {
            (0..self.entries.len()).collect()
        } else {
            let mut hits: Vec<(u32, usize)> = (self.recent..self.entries.len())
                .filter_map(|i| score(&words, &self.entries[i]).map(|s| (s, i)))
                .collect();
            hits.sort_by_key(|&(s, i)| (std::cmp::Reverse(s), i));
            hits.into_iter().map(|(_, i)| i).collect()
        };
        self.selected = (!self.shown.is_empty()).then_some(0);
        self.top = 0;
    }

    pub fn key(&mut self, k: Key) -> Event {
        let page = (COLS * ROWS) as i64;
        match k {
            Key::Char(ek) => {
                let before = self.edit.text.clone();
                self.edit.key(ek);
                if self.edit.text != before {
                    self.refilter();
                }
                Event::None
            }
            Key::Left => self.step(-1),
            Key::Right => self.step(1),
            Key::Up => self.step(-(COLS as i64)),
            Key::Down => self.step(COLS as i64),
            Key::PageUp => self.step(-page),
            Key::PageDown => self.step(page),
            Key::Enter => self.pick(),
            Key::Escape => Event::Close,
        }
    }

    /// Move the highlight by `delta` cells, stopping at either end, and scroll it into view.
    fn step(&mut self, delta: i64) -> Event {
        if let Some(to) = crate::selection::step_index(self.selected, delta, self.shown.len()) {
            self.select(to);
        }
        Event::None
    }

    fn select(&mut self, i: usize) {
        self.selected = Some(i);
        scroll_into_view(&mut self.top, i / COLS, ROWS);
    }

    /// A reader asked to see cell `i`: its row scrolls in and the highlight stays. True when it
    /// moved.
    pub fn reveal(&mut self, i: usize) -> bool {
        let top = self.top;
        if i < self.shown.len() {
            scroll_into_view(&mut self.top, i / COLS, ROWS);
        }
        self.top != top
    }

    fn pick(&self) -> Event {
        self.selected_entry()
            .map_or(Event::Close, |e| Event::Pick(e.emoji))
    }

    pub fn scroll(&mut self, rows: i64) {
        let max = self.rows().saturating_sub(ROWS) as i64;
        self.top = (self.top as i64 + rows).clamp(0, max) as usize;
    }

    pub fn paste(&mut self, text: &str) {
        let line = text.lines().next().unwrap_or_default();
        if self.edit.paste(line, &[]) {
            self.refilter();
        }
    }

    /// A left click on `hit`: a cell picks it, the scroll arrows and track scroll.
    pub fn click(&mut self, hit: Hit) -> Event {
        match hit {
            Hit::Cell(i) if i < self.shown.len() => {
                self.select(i);
                self.pick()
            }
            Hit::ScrollUp => self.scroll_by(-1),
            Hit::ScrollDown => self.scroll_by(1),
            Hit::PageUp => self.scroll_by(-(ROWS as i64)),
            Hit::PageDown => self.scroll_by(ROWS as i64),
            _ => Event::None,
        }
    }

    fn scroll_by(&mut self, rows: i64) -> Event {
        self.scroll(rows);
        Event::None
    }
}

/// How well `e` matches the (lower-case) query `words`, or None when a word matches nothing. Per
/// word, the best of: a whole word of the name (4), the start of one (3), a whole shortcode or a word of one (2), the start of one (1).
fn score(words: &[String], e: &Entry) -> Option<u32> {
    let name: Vec<String> = e
        .name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let codes: Vec<&str> = e
        .shortcodes
        .iter()
        .flat_map(|c| std::iter::once(*c).chain(c.split(['_', '-'])))
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().try_fold(0, |total, q| {
        let q = q.as_str();
        let in_name = name.iter().map(|w| {
            if w == q {
                4
            } else if w.starts_with(q) {
                3
            } else {
                0
            }
        });
        let in_codes = codes.iter().map(|w| {
            if *w == q {
                2
            } else if w.starts_with(q) {
                1
            } else {
                0
            }
        });
        let best = in_name.chain(in_codes).max().unwrap_or(0);
        (best > 0).then_some(total + best)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub field: Rect,
    pub grid: Rect,
    pub cells: Rect,
    pub scrollbar: Rect,
    pub name: Rect,
    pub height: i32,
    /// The look's shortest scrollbar thumb.
    pub thumb_min: i32,
}

/// How far a flyout border wider than Classic's edge moves the contents in.
fn extra(cm: &ControlMetrics) -> i32 {
    cm.flyout_border - flyout::EDGE
}

/// The panel's width: the grid's columns beside the look's scrollbar, inside the grid's frame and the padding.
pub fn width(cm: &ControlMetrics) -> i32 {
    2 * (PAD + extra(cm)) + 4 + COLS as i32 * CELL + cm.scroll
}

pub fn layout(cm: &ControlMetrics) -> Layout {
    let w = width(cm);
    let pad = PAD + extra(cm);
    let field = Rect::new(pad, pad, w - 2 * pad, FIELD_H);
    let grid = Rect::new(
        pad,
        field.bottom() + PAD,
        w - 2 * pad,
        ROWS as i32 * CELL + 4,
    );
    let inner = grid.inset(2);
    let cells = Rect::new(inner.x, inner.y, inner.width - cm.scroll, inner.height);
    let scrollbar = Rect::new(cells.right(), inner.y, cm.scroll, inner.height);
    let name = Rect::new(pad + 2, grid.bottom() + 2, w - 2 * pad - 4, NAME_H);
    Layout {
        field,
        grid,
        cells,
        scrollbar,
        name,
        height: name.bottom() + pad,
        thumb_min: cm.thumb_min,
    }
}

fn cell_rect(m: &EmojiPicker, lay: &Layout, i: usize) -> Option<Rect> {
    (i / COLS).checked_sub(m.top).filter(|&r| r < ROWS)?;
    Some(cell_place(m, lay, i))
}

/// Cell `i`'s place, above or below the grid when its row is scrolled out.
fn cell_place(m: &EmojiPicker, lay: &Layout, i: usize) -> Rect {
    let row = (i / COLS) as i32 - m.top as i32;
    Rect::new(
        lay.cells.x + (i % COLS) as i32 * CELL,
        lay.cells.y + row * CELL,
        CELL,
        CELL,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell(usize),
    ScrollUp,
    ScrollDown,
    PageUp,
    PageDown,
    Field,
    Panel,
}

pub fn hit(m: &EmojiPicker, x: i32, y: i32, cm: &ControlMetrics) -> Hit {
    let lay = layout(cm);
    if lay.field.contains(x, y) {
        return Hit::Field;
    }
    if lay.scrollbar.contains(x, y) {
        let p = scrollbar_parts(
            lay.scrollbar,
            1,
            m.top,
            ROWS,
            m.rows(),
            false,
            lay.thumb_min,
        );
        return match scrollbar_hit(&p, x, y, false) {
            Some(ScrollHit::Back) => Hit::ScrollUp,
            Some(ScrollHit::Forward) => Hit::ScrollDown,
            Some(ScrollHit::PageBack) => Hit::PageUp,
            Some(ScrollHit::PageForward) => Hit::PageDown,
            Some(ScrollHit::Thumb) | None => Hit::Panel,
        };
    }
    if !lay.cells.contains(x, y) {
        return Hit::Panel;
    }
    let col = ((x - lay.cells.x) / CELL) as usize;
    let row = ((y - lay.cells.y) / CELL) as usize + m.top;
    let i = row * COLS + col;
    if i < m.len() {
        Hit::Cell(i)
    } else {
        Hit::Panel
    }
}

/// Draw the panel at `theme.scale` as a flyout ([`crate::flyout`]); the canvas reaches the look's
/// shadow past the panel. `caret` is the blink's shown half, `preedit` what an input method composes in
/// the filter. Without `font` each emoji is drawn as text.
pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    font: Option<&EmojiFont>,
    m: &EmojiPicker,
    caret: bool,
    preedit: Option<&Preedit>,
) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let cm = theme.look.controls();
    let lay = layout(&cm);
    let mut c = Canvas::new((width(&cm) * s) as u32, (lay.height * s) as u32);
    c.set_preedit(preedit.cloned());
    let all = c.bounds();
    flyout::draw_panel(
        theme,
        &mut c,
        all,
        cs.button_face,
        flyout::Backdrop::Blurred,
    );
    let inner = draw_edit_field(
        &mut c,
        p,
        lay.field.scaled(s),
        cs.window,
        true,
        Interaction::Idle,
    );
    let first = edit_first_visible(text, &m.edit, inner.width, px, 0);
    draw_edit(
        &mut c,
        text,
        &m.edit,
        inner,
        px,
        p,
        Caret::new(true, caret),
        first,
    );
    let grid = lay.grid.scaled(s);
    c.fill_rect(grid, cs.window);
    draw_field_frame(&mut c, p, grid, cs.window);
    draw_scrollbar(
        &mut c,
        p,
        lay.scrollbar.scaled(s),
        m.top,
        ROWS,
        m.rows(),
        None,
    );
    let glyph = GLYPH * s;
    for i in m.top * COLS..m.len().min((m.top + ROWS) * COLS) {
        let Some(cell) = cell_rect(m, &lay, i) else {
            continue;
        };
        let cell = cell.scaled(s);
        if m.selected == Some(i) {
            c.fill_rect(cell.inset(s), cs.hilight);
        }
        let e = &m.entries[m.shown[i]];
        let drawn = font.and_then(|f| f.render(e.emoji, glyph as u32, cs.window_text));
        match drawn {
            Some(sq) => {
                let (x, y) = (
                    cell.x + (cell.width - glyph) / 2,
                    cell.y + (cell.height - glyph) / 2,
                );
                c.blit_over(x, y, &sq);
            }
            None => {
                let ink = if m.selected == Some(i) {
                    cs.hilight_text
                } else {
                    cs.window_text
                };
                text.draw(&mut c, cell, e.emoji, false, ink, Align::Center, px);
            }
        }
    }
    let name = lay.name.scaled(s);
    match m.selected_entry() {
        Some(e) => {
            let label = title(e.name);
            let fitted = text.ellipsize(&label, name.width, false, px);
            text.draw(
                &mut c,
                name,
                &fitted,
                false,
                cs.button_text,
                Align::Left,
                px,
            );
        }
        None if !m.edit.text.is_empty() => {
            let r = lay.cells.scaled(s);
            let r = Rect::new(r.x + 3 * s, r.y, r.width - 3 * s, 17 * s);
            text.draw(&mut c, r, NO_MATCH, false, cs.gray_text, Align::Left, px);
        }
        None => {}
    }
    flyout::shadowed(theme, c)
}

fn title(name: &str) -> String {
    let mut ch = name.chars();
    ch.next()
        .map(|f| f.to_uppercase().chain(ch).collect())
        .unwrap_or_default()
}

/// The panel's `access` part, in panel coordinates: the filter field and a list of the named cells;
/// a cell scrolled out lies outside the list ([`EmojiPicker::reveal`]). Returns the focused node: the highlighted cell, else the field.
pub fn access(
    m: &EmojiPicker,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    cm: &ControlMetrics,
) -> NodeId {
    let lay = layout(cm);
    let inner_w = lay.field.width - 2 * FIELD_PAD;
    let first = edit_first_visible(text, &m.edit, inner_w, px, 0);
    let field = access_edit(
        &m.edit, first, false, lay.field, true, text, px, tree, parent, "field",
    );
    let shown = Rect::new(
        lay.cells.x,
        lay.cells.y,
        COLS as i32 * CELL,
        ROWS as i32 * CELL,
    );
    let mut n = access::node(Role::ListBox, shown);
    access::focusable(&mut n, true);
    let list = tree.push(parent, "cells", n);
    let mut focus = field;
    for i in 0..m.len() {
        let e = &m.entries[m.shown[i]];
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(title(e.name));
        o.set_bounds(access::row_bounds(shown, cell_place(m, &lay, i)));
        o.set_selected(m.selected == Some(i));
        let id = tree.push(list, i, o);
        if m.selected == Some(i) {
            focus = id;
        }
    }
    if m.is_empty() && !m.edit.text.is_empty() {
        let r = Rect::new(lay.cells.x, lay.cells.y, lay.cells.width, 17);
        tree.push(parent, "empty", access::label(r, NO_MATCH, false));
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emoji::catalog;
    use crate::theme::FontConfig;

    fn e(emoji: &str) -> Entry {
        Entry::get(emoji).unwrap()
    }

    fn typed(m: &mut EmojiPicker, s: &str) -> Event {
        m.key(Key::Char(EditKey::Insert(s.into())))
    }

    fn shown(m: &EmojiPicker) -> Vec<&'static str> {
        m.shown().map(|e| e.emoji).collect()
    }

    #[test]
    fn recent_picks_come_first_and_enter_picks_the_highlight() {
        let mut m = EmojiPicker::new(vec![e("👍"), e("🎉")], catalog(None));
        assert_eq!(&shown(&m)[..3], ["👍", "🎉", "😀"]);
        assert_eq!(m.key(Key::Enter), Event::Pick("👍"));
        m.key(Key::Right);
        assert_eq!(m.key(Key::Enter), Event::Pick("🎉"));
        m.key(Key::Down);
        assert_eq!(m.selected, Some(1 + COLS));
        m.key(Key::Left);
        m.key(Key::Left);
        m.key(Key::Up);
        assert_eq!(m.selected, Some(0), "the ends hold");
        assert_eq!(m.key(Key::Escape), Event::Close);
    }

    #[test]
    fn typing_matches_word_starts_whole_words_first_without_the_recents() {
        let mut m = EmojiPicker::new(vec![e("😀")], catalog(None));
        typed(&mut m, "grin");
        let hits = shown(&m);
        assert_eq!(hits.iter().filter(|&&x| x == "😀").count(), 1);
        assert!(
            hits.contains(&"😁"),
            "beaming face with smiling eyes: :grin:"
        );
        typed(&mut m, " cat");
        assert_eq!(
            shown(&m),
            ["😺", "😸"],
            "grinning cat, then grinning cat with smiling eyes"
        );
        assert_eq!(m.key(Key::Enter), Event::Pick("😺"));
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        typed(&mut m, "fire");
        let hits = shown(&m);
        let at = |x: &str| hits.iter().position(|&h| h == x).unwrap();
        assert!(at("🔥") < at("🎆"));
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        typed(&mut m, "tada");
        assert_eq!(shown(&m)[0], "🎉");
        typed(&mut m, "zzzz");
        assert!(m.is_empty());
        assert_eq!(m.key(Key::Enter), Event::Close);
    }

    #[test]
    fn page_down_scrolls_and_the_highlight_stays_in_view() {
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        m.key(Key::PageDown);
        assert_eq!(m.selected, Some(COLS * ROWS));
        assert_eq!(m.top, 1);
        m.key(Key::PageDown);
        assert_eq!(m.top, ROWS + 1);
        m.key(Key::PageUp);
        m.key(Key::PageUp);
        assert_eq!((m.selected, m.top), (Some(0), 0));
        m.scroll(-5);
        assert_eq!(m.top, 0);
        m.scroll(1_000_000);
        assert_eq!(m.top, m.rows() - ROWS);
    }

    #[test]
    fn cells_hit_by_row_and_column_and_a_click_picks() {
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        let lay = layout(&ControlMetrics::CLASSIC);
        let (x0, y0) = (lay.cells.x, lay.cells.y);
        assert_eq!(
            hit(&m, x0 + 1, y0 + 1, &ControlMetrics::CLASSIC),
            Hit::Cell(0)
        );
        assert_eq!(
            hit(
                &m,
                x0 + CELL * 2 + 1,
                y0 + CELL + 1,
                &ControlMetrics::CLASSIC
            ),
            Hit::Cell(COLS + 2)
        );
        assert_eq!(
            hit(
                &m,
                lay.field.x + 5,
                lay.field.y + 5,
                &ControlMetrics::CLASSIC
            ),
            Hit::Field
        );
        assert_eq!(hit(&m, 1, 1, &ControlMetrics::CLASSIC), Hit::Panel);
        let down = hit(
            &m,
            lay.scrollbar.x + 3,
            lay.scrollbar.bottom() - 3,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(down, Hit::ScrollDown);
        m.click(down);
        assert_eq!(m.top, 1);
        assert_eq!(
            hit(&m, x0 + 1, y0 + 1, &ControlMetrics::CLASSIC),
            Hit::Cell(COLS)
        );
        let want = shown(&m)[COLS];
        assert_eq!(m.click(Hit::Cell(COLS)), Event::Pick(want));
        assert_eq!(m.click(Hit::Cell(100_000)), Event::None);
    }

    #[test]
    fn a_paste_filters_on_its_first_line() {
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        m.paste("rocket\nmore");
        assert_eq!(m.edit().text, "rocket");
        assert_eq!(shown(&m), ["🚀"]);
    }

    #[test]
    fn the_panel_is_328_by_235_with_ten_30_px_columns_beside_the_scrollbar() {
        let lay = layout(&ControlMetrics::CLASSIC);
        assert_eq!(width(&ControlMetrics::CLASSIC), 328);
        assert_eq!(lay.field, Rect::new(4, 4, 320, 21));
        assert_eq!(lay.grid, Rect::new(4, 29, 320, 184));
        assert_eq!(lay.cells, Rect::new(6, 31, 300, 180));
        assert_eq!(lay.scrollbar, Rect::new(306, 31, 16, 180));
        assert_eq!(lay.name, Rect::new(6, 215, 316, 16));
        assert_eq!(lay.height, 235);
        let m = EmojiPicker::new(Vec::new(), catalog(None));
        assert_eq!(
            cell_rect(&m, &lay, COLS + 1),
            Some(Rect::new(36, 61, 30, 30))
        );
        assert_eq!(cell_rect(&m, &lay, COLS * ROWS), None, "below the view");
    }

    #[test]
    fn the_panel_is_raised_with_the_first_cell_highlighted_and_named() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let font = EmojiFont::load();
        let m = EmojiPicker::new(Vec::new(), catalog(font.as_ref()));
        let c = draw(&theme, &text, font.as_ref(), &m, true, None);
        let lay = layout(&ControlMetrics::CLASSIC);
        assert_eq!(
            (c.width, c.height),
            (width(&ControlMetrics::CLASSIC) as u32, lay.height as u32)
        );
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(lay.grid.x, lay.grid.y), cs.button_shadow.argb());
        let (x0, y0) = (lay.cells.x, lay.cells.y);
        assert_eq!(c.get(x0 + 2, y0 + 2), cs.hilight.argb());
        assert_eq!(c.get(x0 + CELL + 2, y0 + 2), cs.window.argb());
        let named = (lay.name.x..lay.name.right())
            .any(|x| (lay.name.y..lay.name.bottom()).any(|y| c.get(x, y) != cs.button_face.argb()));
        assert!(named, "the highlighted emoji's name is drawn");
    }

    #[test]
    fn a_reader_scrolls_a_cell_in_and_the_highlight_stays() {
        use crate::access::{Published, ROOT};
        let text = TextRenderer::load(&FontConfig::default());
        let cm = ControlMetrics::CLASSIC;
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        let last = m.len() - 1;
        let mut t = Tree::window("", (width(&cm), 300));
        access(&m, &text, 11, &mut t, ROOT, &cm);
        let (list, n) = t.nodes().find(|(_, n)| n.role() == Role::ListBox).unwrap();
        assert_eq!(n.children().len(), m.len(), "every cell is listed");
        let cell = t.get(list).unwrap().children()[last];
        let mut p = Published::default();
        p.update(t);
        assert_eq!(p.reveal(cell).map(|r| r.row), Some(last));
        assert!(m.reveal(last));
        assert_eq!(m.top, last / COLS + 1 - ROWS, "its row is the last shown");
        assert_eq!(m.selected, Some(0), "the highlight stays");
        assert!(!m.reveal(last));
    }

    #[test]
    fn no_match_says_so_and_the_highlight_has_the_focus() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut m = EmojiPicker::new(Vec::new(), catalog(None));
        let mut t = Tree::window("", (width(&ControlMetrics::CLASSIC), 300));
        let focus = access(
            &m,
            &text,
            11,
            &mut t,
            access::ROOT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(t.get(focus).unwrap().label(), Some("Grinning face"));
        typed(&mut m, "zzzz");
        let mut t = Tree::window("", (width(&ControlMetrics::CLASSIC), 300));
        let focus = access(
            &m,
            &text,
            11,
            &mut t,
            access::ROOT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(t.get(focus).unwrap().role(), Role::TextInput);
        assert!(t.nodes().any(|(_, n)| n.value() == Some(NO_MATCH)));
        let theme = Theme::default();
        let c = draw(&theme, &text, None, &m, false, None);
        let grey = theme.colors.gray_text.argb();
        let lay = layout(&ControlMetrics::CLASSIC);
        let found = (lay.cells.x..lay.cells.x + 150)
            .any(|x| (lay.cells.y..lay.cells.y + 17).any(|y| c.get(x, y) == grey));
        assert!(found, "the empty grid's label draws in gray_text");
    }

    #[test]
    #[ignore]
    fn render_emojipicker_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let font = EmojiFont::load();
        let mut m = EmojiPicker::new(vec![e("👍"), e("🎉"), e("❤️")], catalog(font.as_ref()));
        m.key(Key::Down);
        crate::raster::write_preview(
            &draw(&theme, &text, font.as_ref(), &m, true, None),
            "emojipicker-preview.png",
        );
        let t2 = theme.scaled(2);
        crate::raster::write_preview(
            &draw(&t2, &text, font.as_ref(), &m, true, None),
            "emojipicker-preview@2.png",
        );
    }
}
