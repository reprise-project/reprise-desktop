//! Character Map: a font's characters in a grid, narrowed to one Unicode block or to those whose
//! names hold a search, and the picked one's name and code. The host loads the font and hands
//! [`CharMap::new`] what it covers; [`view`] lays out the window around the grid.

use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::classic::widgets::{BarHit, ScrollParts, bar_hit, scrollbar_parts};
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, draw_focus_ring, draw_scrollbar};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};

pub mod view;
pub use view::*;

#[cfg(test)]
mod tests;

/// Cells to a row.
pub const COLS: usize = 20;

/// A Unicode block some of the font's characters are in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub name: &'static str,
    pub start: u32,
    pub end: u32,
}

impl Block {
    pub fn holds(&self, c: char) -> bool {
        (self.start..=self.end).contains(&(c as u32))
    }
}

/// The blocks `chars` (sorted) fall in, in code point order.
pub fn blocks_of(chars: &[char]) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for &c in chars {
        if out.last().is_some_and(|b| b.holds(c)) {
            continue;
        }
        if let Some(b) = unicode_blocks::find_unicode_block(c) {
            out.push(Block {
                name: b.name(),
                start: b.start(),
                end: b.end(),
            });
        }
    }
    out
}

/// `c`'s Unicode name, `LATIN SMALL LETTER A`; `None` for a control or unassigned code point.
pub fn name(c: char) -> Option<String> {
    unicode_names2::name(c).map(|n| n.to_string())
}

/// `U+00E9`.
pub fn code(c: char) -> String {
    format!("U+{:04X}", c as u32)
}

/// Whether `c` answers search `query`: one character finds itself in either case, a code
/// (`U+20AC`, `20ac`) its character, anything else the characters whose names hold every word.
pub fn matches(c: char, query: &str) -> bool {
    let query = query.trim();
    let mut one = query.chars();
    if let (Some(q), None) = (one.next(), one.next()) {
        return c == q || c.to_lowercase().eq(q.to_lowercase());
    }
    let hex = query
        .strip_prefix("U+")
        .or_else(|| query.strip_prefix("u+"))
        .unwrap_or(query);
    if hex.len() >= 2
        && let Ok(v) = u32::from_str_radix(hex, 16)
        && v == c as u32
    {
        return true;
    }
    let Some(name) = name(c) else {
        return false;
    };
    let upper = query.to_uppercase();
    let mut words = upper.split_whitespace().peekable();
    words.peek().is_some() && words.all(|w| name.contains(w))
}

/// A move of the pick in the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharMap {
    /// Every character the font has, sorted.
    pub chars: Vec<char>,
    pub blocks: Vec<Block>,
    /// The block shown; `None` shows them all.
    pub block: Option<usize>,
    pub query: String,
    /// The characters in the grid: `chars` in the block and answering the query.
    pub shown: Vec<char>,
    /// The pick, an index into `shown`.
    pub selected: Option<usize>,
    /// The first row in view.
    pub top: usize,
}

impl CharMap {
    /// The characters `chars` covers, sorted and without controls or unassigned code points.
    pub fn new(chars: impl IntoIterator<Item = char>) -> CharMap {
        let mut map = CharMap::default();
        map.set_chars(chars);
        map
    }

    /// Takes another font's characters: the block (by name), the search and the picked character
    /// carry over where the font has them.
    pub fn set_chars(&mut self, chars: impl IntoIterator<Item = char>) {
        let picked = self.picked();
        let block = self.block.map(|b| self.blocks[b].name);
        let mut chars: Vec<char> = chars
            .into_iter()
            .filter(|c| !c.is_control() && name(*c).is_some() || is_unnamed_graphic(*c))
            .collect();
        chars.sort_unstable();
        chars.dedup();
        self.blocks = blocks_of(&chars);
        self.chars = chars;
        self.block = block.and_then(|n| self.blocks.iter().position(|b| b.name == n));
        self.refilter(picked);
    }

    pub fn set_block(&mut self, block: Option<usize>) {
        let picked = self.picked();
        self.block = block.filter(|&b| b < self.blocks.len());
        self.refilter(picked);
    }

    pub fn set_query(&mut self, query: &str) {
        let picked = self.picked();
        self.query = query.to_string();
        self.refilter(picked);
    }

    fn refilter(&mut self, keep: Option<char>) {
        let block = self.block.map(|b| self.blocks[b]);
        let query = self.query.trim();
        self.shown = self
            .chars
            .iter()
            .copied()
            .filter(|&c| block.is_none_or(|b| b.holds(c)))
            .filter(|&c| query.is_empty() || matches(c, query))
            .collect();
        self.selected = keep
            .and_then(|k| self.shown.iter().position(|&c| c == k))
            .or((!self.shown.is_empty()).then_some(0));
        self.top = self.top.min(self.rows().saturating_sub(1));
    }

    pub fn picked(&self) -> Option<char> {
        self.shown.get(self.selected?).copied()
    }

    /// Picks `c` if shown.
    pub fn pick_char(&mut self, c: char) -> bool {
        match self.shown.iter().position(|&s| s == c) {
            Some(i) => {
                self.selected = Some(i);
                true
            }
            None => false,
        }
    }

    pub fn rows(&self) -> usize {
        self.shown.len().div_ceil(COLS)
    }

    /// Moves the pick by `step`, a page being `page` rows, and scrolls it into view; false
    /// when it stays.
    pub fn step(&mut self, step: Step, page: usize) -> bool {
        let n = self.shown.len();
        if n == 0 {
            return false;
        }
        let at = self.selected.unwrap_or(0);
        let page = page.max(1) * COLS;
        let to = match step {
            Step::Left => at.saturating_sub(1),
            Step::Right => (at + 1).min(n - 1),
            Step::Up if at >= COLS => at - COLS,
            Step::Up => at,
            Step::Down if at + COLS < n => at + COLS,
            Step::Down => at,
            Step::PageUp => at.saturating_sub(page),
            Step::PageDown => (at + page).min(n - 1),
            Step::Home => 0,
            Step::End => n - 1,
        };
        let moved = self.selected != Some(to);
        self.selected = Some(to);
        self.follow(page / COLS);
        moved
    }

    /// Scrolls the picked row into a view `visible` rows tall.
    pub fn follow(&mut self, visible: usize) {
        let Some(at) = self.selected else {
            return;
        };
        let row = at / COLS;
        let visible = visible.max(1);
        if row < self.top {
            self.top = row;
        } else if row >= self.top + visible {
            self.top = row + 1 - visible;
        }
    }

    /// Scrolls by `rows` within a view `visible` rows tall; false at the end.
    pub fn scroll(&mut self, rows: i64, visible: usize) -> bool {
        let max = self.rows().saturating_sub(visible);
        let to = (self.top as i64 + rows).clamp(0, max as i64) as usize;
        let moved = to != self.top;
        self.top = to;
        moved
    }

    /// Scrolls to row `top`, kept in range.
    pub fn scroll_to(&mut self, top: usize, visible: usize) -> bool {
        let to = top.min(self.rows().saturating_sub(visible));
        let moved = to != self.top;
        self.top = to;
        moved
    }
}

/// A character with no name of its own that still draws: a CJK ideograph or Hangul syllable
/// has one, so this holds only the private use areas.
fn is_unnamed_graphic(c: char) -> bool {
    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Where the grid's parts land in `r`: the cells, square, `COLS` to a row, and the scrollbar
/// down the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub cells: Rect,
    pub bar: Rect,
    pub cell: i32,
    /// Whole rows in view.
    pub rows: usize,
}

pub fn layout(r: Rect, cm: &ControlMetrics) -> Layout {
    let inner = r.inset(2);
    let bar = Rect::new(inner.right() - cm.scroll, inner.y, cm.scroll, inner.height);
    let cell = ((inner.width - cm.scroll) / COLS as i32).max(1);
    let rows = (inner.height / cell).max(1) as usize;
    Layout {
        frame: r,
        cells: Rect::new(inner.x, inner.y, cell * COLS as i32, cell * rows as i32),
        bar,
        cell,
        rows,
    }
}

impl Layout {
    /// The cell of `shown` index `i` when in view.
    pub fn cell_rect(&self, map: &CharMap, i: usize) -> Option<Rect> {
        let row = (i / COLS).checked_sub(map.top)?;
        (row < self.rows).then(|| {
            Rect::new(
                self.cells.x + (i % COLS) as i32 * self.cell,
                self.cells.y + row as i32 * self.cell,
                self.cell,
                self.cell,
            )
        })
    }

    pub fn bar_parts(&self, map: &CharMap, thumb_min: i32) -> ScrollParts {
        scrollbar_parts(
            self.bar,
            1,
            map.top,
            self.rows,
            map.rows(),
            false,
            thumb_min,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell(usize),
    Bar(BarHit),
}

/// What lies under (`x`, `y`): a shown character's cell or a part of the scrollbar.
pub fn hit(map: &CharMap, lay: &Layout, cm: &ControlMetrics, x: i32, y: i32) -> Option<Hit> {
    if lay.bar.contains(x, y) {
        return Some(Hit::Bar(bar_hit(
            &lay.bar_parts(map, cm.thumb_min),
            x,
            y,
            false,
        )));
    }
    if !lay.cells.contains(x, y) {
        return None;
    }
    let col = ((x - lay.cells.x) / lay.cell) as usize;
    let row = ((y - lay.cells.y) / lay.cell) as usize + map.top;
    let i = row * COLS + col;
    (i < map.shown.len()).then_some(Hit::Cell(i))
}

/// The glyph size in a cell `cell` px square.
fn glyph_px(cell: i32) -> u32 {
    (cell * 2 / 3).max(1) as u32
}

/// Draws the grid in `lay` (logical, drawn at `p.s`): each character in `glyphs` in its cell
/// between `button_shadow` lines, the pick highlighted, ringed while `focused`.
pub fn draw(
    c: &mut Canvas,
    glyphs: &TextRenderer,
    p: &Paint,
    map: &CharMap,
    lay: &Layout,
    focused: bool,
) {
    let s = p.s;
    let frame = lay.frame.scaled(s);
    c.fill_rect(frame, p.cs.window);
    draw_field_frame(c, p, frame, p.cs.window);
    let px = glyph_px(lay.cell) * s as u32;
    for row in 0..lay.rows {
        for col in 0..COLS {
            let i = (map.top + row) * COLS + col;
            let r = Rect::new(
                lay.cells.x + col as i32 * lay.cell,
                lay.cells.y + row as i32 * lay.cell,
                lay.cell,
                lay.cell,
            )
            .scaled(s);
            let line = Rect::new(r.x, r.bottom() - s, r.width, s);
            c.fill_rect(line, p.cs.button_face);
            c.fill_rect(Rect::new(r.right() - s, r.y, s, r.height), p.cs.button_face);
            let Some(&ch) = map.shown.get(i) else {
                continue;
            };
            let inside = Rect::new(r.x, r.y, r.width - s, r.height - s);
            let picked = map.selected == Some(i);
            if picked {
                c.fill_rect(
                    inside,
                    if focused {
                        p.cs.hilight
                    } else {
                        p.cs.button_face
                    },
                );
            }
            let ink = if picked && focused {
                p.cs.hilight_text
            } else {
                p.cs.window_text
            };
            glyphs.draw(c, inside, &ch.to_string(), false, ink, Align::Center, px);
            if picked && focused {
                draw_focus_ring(c, p, inside, p.cs.hilight_text);
            }
        }
    }
    draw_scrollbar(c, p, lay.bar.scaled(s), map.top, lay.rows, map.rows(), None);
}

/// The picked character large in `r`'s sunken box.
pub fn draw_preview(c: &mut Canvas, glyphs: &TextRenderer, p: &Paint, r: Rect, ch: Option<char>) {
    let r = r.scaled(p.s);
    c.fill_rect(r, p.cs.window);
    draw_field_frame(c, p, r, p.cs.window);
    if let Some(ch) = ch {
        let px = (r.height * 2 / 3).max(1) as u32;
        glyphs.draw(
            c,
            r,
            &ch.to_string(),
            false,
            p.cs.window_text,
            Align::Center,
            px,
        );
    }
}

/// The grid's access part: a grid under `parent` with a cell per character in view, each
/// named by its character and name, the pick selected. Returns the grid's node.
pub fn access(map: &CharMap, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let mut grid = access::named(Role::Grid, lay.frame, &tr!("charmap-grid"));
    access::focusable(&mut grid, true);
    let id = tree.push(parent, "grid", grid);
    for i in 0..map.shown.len() {
        let Some(r) = lay.cell_rect(map, i) else {
            continue;
        };
        let ch = map.shown[i];
        let label = format!("{ch} {}", name(ch).unwrap_or_else(|| code(ch)));
        let mut cell = access::named(Role::GridCell, r, &label);
        if map.selected == Some(i) {
            cell.set_selected(true);
        }
        tree.push(id, ch as u32, cell);
    }
    id
}
