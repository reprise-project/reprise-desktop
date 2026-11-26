//! A terminal's cell grid: a plain snapshot of cells the host fills from its emulator, drawn in
//! device-pixel cells so the grid stays crisp at every scale, with the scrollback's bar down its
//! right edge. The bar is always there, greyed while there is no scrollback, so the grid keeps
//! its columns when the first row scrolls off.

use std::hash::Hash;
use std::ops::{BitOr, BitOrAssign};

use unicode_segmentation::UnicodeSegmentation;

use crate::access::accesskit::{NodeId, Orientation, Role};
use crate::access::{self, Tree};
use crate::classic::widgets::{BarHit, ScrollHit};
use crate::controls::scrollbar::RowScroll;
use crate::look::Paint;
use crate::look::widgets::draw_scrollbar;
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Color;

pub type Rgb = Color;

/// A cell's attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Flags(u16);

impl Flags {
    pub const BOLD: Flags = Flags(1);
    pub const ITALIC: Flags = Flags(1 << 1);
    pub const UNDERLINE: Flags = Flags(1 << 2);
    pub const STRIKE: Flags = Flags(1 << 3);
    pub const INVERSE: Flags = Flags(1 << 4);
    pub const DIM: Flags = Flags(1 << 5);
    /// A double-width char; the next cell is its [`Flags::WIDE_SPACER`].
    pub const WIDE: Flags = Flags(1 << 6);
    pub const WIDE_SPACER: Flags = Flags(1 << 7);
    pub const SELECTED: Flags = Flags(1 << 8);
    pub const MATCH: Flags = Flags(1 << 9);
    pub const MATCH_CURRENT: Flags = Flags(1 << 10);
    /// A hyperlink, dotted underneath; [`Flags::UNDERLINE`] on it draws the solid line instead.
    pub const LINK: Flags = Flags(1 << 11);

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Flags {
    type Output = Flags;
    fn bitor(self, o: Flags) -> Flags {
        Flags(self.0 | o.0)
    }
}

impl BitOrAssign for Flags {
    fn bitor_assign(&mut self, o: Flags) {
        self.0 |= o.0;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    /// Combining chars drawn over `ch`.
    pub extra: Option<Box<str>>,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: Flags,
}

impl Cell {
    pub fn blank(fg: Rgb, bg: Rgb) -> Cell {
        Cell {
            ch: ' ',
            extra: None,
            fg,
            bg,
            flags: Flags::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    /// The block's outline, for an unfocused window.
    HollowBlock,
    Hidden,
}

/// Colours of the cursor and of selected and found cells (fg, bg).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accents {
    pub cursor: Rgb,
    pub selection: (Rgb, Rgb),
    pub found: (Rgb, Rgb),
    pub found_current: (Rgb, Rgb),
}

impl Default for Accents {
    fn default() -> Self {
        Accents {
            cursor: Color::rgb(0xC0, 0xC0, 0xC0),
            selection: (Color::rgb(0xFF, 0xFF, 0xFF), Color::rgb(0x30, 0x50, 0x90)),
            found: (Color::rgb(0x00, 0x00, 0x00), Color::rgb(0xC0, 0xA0, 0x40)),
            found_current: (Color::rgb(0x00, 0x00, 0x00), Color::rgb(0xF0, 0x90, 0x30)),
        }
    }
}

/// Where the view stands in the scrollback: `history` rows kept above the screen's `rows`, the
/// view `offset` rows back from the newest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scrollback {
    pub history: usize,
    pub offset: usize,
    pub rows: usize,
}

impl Scrollback {
    /// The bar's position: the view's first row among all the scrollback's, oldest first.
    pub fn at(self) -> RowScroll {
        RowScroll {
            top: self.history.saturating_sub(self.offset),
            visible: self.rows,
            total: self.history + self.rows,
        }
    }

    /// The offset that shows row `top` of [`Scrollback::at`] first.
    pub fn offset_at(self, top: usize) -> usize {
        self.history.saturating_sub(top)
    }
}

/// What the grid shows: `rows` x `cols` cells row by row, and which rows changed since the last
/// draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
    /// (row, column, shape).
    pub cursor: (usize, usize, CursorShape),
    pub dirty: Vec<bool>,
    /// The default background, drawn at `alpha` where a cell shows it.
    pub bg: Rgb,
    pub alpha: u8,
    pub accents: Accents,
    pub scrollback: Scrollback,
}

impl Snapshot {
    /// Blank cells in `fg` on `bg`, every row dirty.
    pub fn new(cols: usize, rows: usize, fg: Rgb, bg: Rgb) -> Snapshot {
        Snapshot {
            cols,
            rows,
            cells: vec![Cell::blank(fg, bg); cols * rows],
            cursor: (0, 0, CursorShape::Block),
            dirty: vec![true; rows],
            bg,
            alpha: 255,
            accents: Accents::default(),
            scrollback: Scrollback {
                rows,
                ..Scrollback::default()
            },
        }
    }

    pub fn row(&self, r: usize) -> &[Cell] {
        let start = (r * self.cols).min(self.cells.len());
        &self.cells[start..(start + self.cols).min(self.cells.len())]
    }

    /// Row `r`'s text: each cell's char and combining chars, spacers skipped, trailing blanks cut.
    pub fn row_text(&self, r: usize) -> String {
        let mut out = String::new();
        for cell in self.row(r) {
            if cell.flags.contains(Flags::WIDE_SPACER) {
                continue;
            }
            out.push(if cell.ch == '\0' { ' ' } else { cell.ch });
            out.push_str(cell.extra.as_deref().unwrap_or_default());
        }
        out.truncate(out.trim_end_matches(' ').len());
        out
    }
}

/// Cell size and baseline (from the cell's top), in device px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub cell_w: i32,
    pub cell_h: i32,
    pub baseline: i32,
}

/// The cell under a point, and which half of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub col: usize,
    pub row: usize,
    pub side: Side,
}

/// Which half of a cell a point lies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// The cell of `text` at `px` on an output at `scale`: the font's own advance and line height at
/// `px * scale`, never a scale-1 cell multiplied.
pub fn metrics(text: &TextRenderer, px: u32, scale: i32) -> Metrics {
    let size = px * scale.max(1) as u32;
    let cell_w = text.advance('M', false, size).max(1);
    let cell_h = text
        .line(Rect::default(), false, size)
        .map_or(size as i32, |l| l.1)
        .max(1);
    let baseline = text
        .baseline(Rect::new(0, 0, cell_w, cell_h), false, size)
        .unwrap_or(cell_h * 4 / 5);
    Metrics {
        cell_w,
        cell_h,
        baseline,
    }
}

pub fn grid_size(m: &Metrics, area: Rect, scale: i32) -> (usize, usize) {
    let s = scale.max(1);
    let n = |len: i32, cell: i32| (len.max(0) * s / cell.max(1)) as usize;
    (n(area.width, m.cell_w), n(area.height, m.cell_h))
}

/// `area` split into the cells and the scrollback bar `bar_w` wide down its right edge.
pub fn split(area: Rect, bar_w: i32) -> (Rect, Rect) {
    let bar_w = bar_w.clamp(0, area.width.max(0));
    let cells = Rect::new(area.x, area.y, area.width - bar_w, area.height);
    (cells, Rect::new(cells.right(), area.y, bar_w, area.height))
}

/// The scrollback bar in `bar` (logical px), the part `lit` names in its state.
pub fn draw_bar(c: &mut Canvas, p: &Paint, bar: Rect, sb: Scrollback, lit: Lit<ScrollHit>) {
    let at = sb.at();
    draw_scrollbar(c, p, bar.scaled(p.s), at.top, at.visible, at.total, lit);
}

/// The part of the scrollback bar `bar` under (`x`, `y`), with the look's shortest thumb
/// `thumb_min`; `None` off the bar.
pub fn bar_hit(sb: Scrollback, bar: Rect, thumb_min: i32, x: i32, y: i32) -> Option<BarHit> {
    match bar.contains(x, y) {
        true => sb.at().hit(bar, thumb_min, x, y),
        false => None,
    }
}

/// The scroll offset a thumb grabbed `grab` px below its top asks for with the pointer at `y`.
pub fn bar_drag(sb: Scrollback, bar: Rect, thumb_min: i32, y: i32, grab: i32) -> usize {
    sb.offset_at(sb.at().top_at(bar, thumb_min, y, grab))
}

/// Draw the rows marked dirty into `area` (logical px) and return each drawn row's rect (logical
/// px) for the host to report as damage. The last row's rect reaches `area`'s bottom.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    m: &Metrics,
    area: Rect,
    snap: &Snapshot,
    px: u32,
) -> Vec<Rect> {
    let s = c.scale();
    let (dw, dh) = c.device_size();
    let bound = area
        .scaled(s)
        .intersect(&Rect::new(0, 0, dw as i32, dh as i32));
    let size = px * s as u32;
    let (ox, oy) = (area.x * s, area.y * s);
    let mut drawn = Vec::new();
    c.in_device_px(|c| {
        let g = Grid {
            text,
            m,
            snap,
            size,
            scale: s,
            bound,
        };
        for r in 0..snap.rows {
            if !snap.dirty.get(r).copied().unwrap_or(false) {
                continue;
            }
            let y = oy + r as i32 * m.cell_h;
            let bottom = if r + 1 == snap.rows {
                bound.bottom()
            } else {
                y + m.cell_h
            };
            let band = Rect::new(bound.x, y, bound.width, bottom - y).intersect(&bound);
            if band.is_empty() {
                continue;
            }
            g.row(c, r, ox, y, band);
            drawn.push(logical(band, s));
        }
        let (r, col, shape) = snap.cursor;
        if snap.dirty.get(r).copied().unwrap_or(false) && col < snap.cols {
            g.cursor(c, r, col, shape, ox, oy);
        }
    });
    drawn
}

/// Fill `area` (logical px) with `snap`'s default background at its alpha, as the grid's own
/// margins are; returns the rect to report as damage.
pub fn draw_background(c: &mut Canvas, area: Rect, snap: &Snapshot) -> Rect {
    let s = c.scale();
    let (dw, dh) = c.device_size();
    let r = area
        .scaled(s)
        .intersect(&Rect::new(0, 0, dw as i32, dh as i32));
    c.in_device_px(|c| fill(c, r, r, premul(snap.bg, snap.alpha)));
    logical(r, s)
}

/// The logical rect covering device rect `r` at `s`.
fn logical(r: Rect, s: i32) -> Rect {
    let (x0, y0) = (r.x.div_euclid(s), r.y.div_euclid(s));
    let (x1, y1) = (-(-r.right()).div_euclid(s), -(-r.bottom()).div_euclid(s));
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// A cell's (fg, bg) after its attributes, and whether its bg is the default one.
fn colours(snap: &Snapshot, cell: &Cell) -> (Rgb, Rgb, bool) {
    let f = cell.flags;
    let (mut fg, mut bg) = (cell.fg, cell.bg);
    if f.contains(Flags::INVERSE) {
        (fg, bg) = (bg, fg);
    }
    if f.contains(Flags::DIM) {
        let mix = |a: u8, b: u8| ((u16::from(a) * 2 + u16::from(b)) / 3) as u8;
        fg = Color::rgb(mix(fg.r, bg.r), mix(fg.g, bg.g), mix(fg.b, bg.b));
    }
    let a = snap.accents;
    let marked = if f.contains(Flags::SELECTED) {
        Some(a.selection)
    } else if f.contains(Flags::MATCH_CURRENT) {
        Some(a.found_current)
    } else if f.contains(Flags::MATCH) {
        Some(a.found)
    } else {
        None
    };
    match marked {
        Some((fg, bg)) => (fg, bg, false),
        None => (fg, bg, bg == snap.bg),
    }
}

/// Premultiplied ARGB of `c` at `alpha`.
fn premul(c: Rgb, alpha: u8) -> u32 {
    let k = |v: u8| (u32::from(v) * u32::from(alpha) + 127) / 255;
    u32::from(alpha) << 24 | k(c.r) << 16 | k(c.g) << 8 | k(c.b)
}

/// Store `v` in device rect `r` of a scale-1 canvas, cut to `bound`.
fn fill(c: &mut Canvas, r: Rect, bound: Rect, v: u32) {
    let r = r.intersect(&bound);
    if r.is_empty() {
        return;
    }
    let w = c.width as usize;
    for y in r.y..r.bottom() {
        let row = y as usize * w;
        c.pixels[row + r.x as usize..row + r.right() as usize].fill(v);
    }
}

/// One draw call's inputs; every rect here is in device px on the canvas read at scale 1.
struct Grid<'a> {
    text: &'a TextRenderer,
    m: &'a Metrics,
    snap: &'a Snapshot,
    size: u32,
    scale: i32,
    bound: Rect,
}

impl Grid<'_> {
    fn light(&self) -> i32 {
        crate::boxdraw::light(self.m.cell_h, self.scale)
    }

    fn bg_px(&self, bg: Rgb, default: bool) -> u32 {
        premul(bg, if default { self.snap.alpha } else { 255 })
    }

    /// Row `r` with its cells from (`ox`, `y`), the rest of `band` in the default background.
    fn row(&self, c: &mut Canvas, r: usize, ox: i32, y: i32, band: Rect) {
        let (m, snap) = (self.m, self.snap);
        let cells = snap.row(r);
        let grid_right = ox + cells.len() as i32 * m.cell_w;
        let default = self.bg_px(snap.bg, true);
        fill(
            c,
            Rect::new(grid_right, band.y, band.right() - grid_right, band.height),
            self.bound,
            default,
        );
        fill(
            c,
            Rect::new(
                band.x,
                y + m.cell_h,
                band.width,
                band.bottom() - y - m.cell_h,
            ),
            self.bound,
            default,
        );

        let lines = |c: &Cell| c.flags.0 & (Flags::UNDERLINE.0 | Flags::STRIKE.0 | Flags::LINK.0);
        let mut runs: Vec<(usize, usize, Rgb)> = Vec::new();
        let mut start = 0;
        while start < cells.len() {
            let (fg, bg, default_bg) = colours(snap, &cells[start]);
            let mut end = start + 1;
            while end < cells.len() {
                let (f2, b2, _) = colours(snap, &cells[end]);
                if (f2, b2) != (fg, bg) || lines(&cells[end]) != lines(&cells[start]) {
                    break;
                }
                end += 1;
            }
            let x0 = ox + start as i32 * m.cell_w;
            let run = Rect::new(x0, y, (end - start) as i32 * m.cell_w, m.cell_h);
            fill(c, run, self.bound, self.bg_px(bg, default_bg));
            runs.push((start, end, fg));
            start = end;
        }
        let joins = joined(self.text, cells, self.size);
        for (start, end, fg) in runs {
            for (i, cell) in cells.iter().enumerate().take(end).skip(start) {
                let x = ox + i as i32 * m.cell_w;
                match joins.iter().find(|j| (j.start..j.end).contains(&i)) {
                    Some(j) if j.start == i => {
                        let span = Rect::new(x, y, (j.end - j.start) as i32 * m.cell_w, m.cell_h);
                        self.ink(c, &j.text, cell.flags, span, span, fg);
                    }
                    Some(_) => {}
                    None => {
                        let spill = cells.get(i + 1).is_some_and(blank);
                        self.glyph(c, cell, x, y, fg, spill);
                    }
                }
            }
            let flags = cells[start].flags;
            let x0 = ox + start as i32 * m.cell_w;
            let run_w = (end - start) as i32 * m.cell_w;
            let lt = self.light();
            let uy = (y + m.baseline + lt).min(y + m.cell_h - lt);
            if flags.contains(Flags::UNDERLINE) {
                fill(c, Rect::new(x0, uy, run_w, lt), self.bound, premul(fg, 255));
            } else if flags.contains(Flags::LINK) {
                // Dots one line wide with a line's gap, phased from the row's start so
                // neighbouring runs continue the pattern.
                let run_end = x0 + run_w;
                let mut x = x0 - (x0 - ox) % (2 * lt);
                while x < run_end {
                    let (from, to) = (x.max(x0), (x + lt).min(run_end));
                    if to > from {
                        fill(
                            c,
                            Rect::new(from, uy, to - from, lt),
                            self.bound,
                            premul(fg, 255),
                        );
                    }
                    x += 2 * lt;
                }
            }
            if flags.contains(Flags::STRIKE) {
                let sy = y + m.baseline - m.baseline / 4;
                fill(c, Rect::new(x0, sy, run_w, lt), self.bound, premul(fg, 255));
            }
        }
    }

    /// `cell`'s glyph in `color`, its cell from (`x`, `y`). A colour glyph in a one-cell char
    /// (U+2764 U+FE0F) inks the next cell too when `spill` says that one is blank.
    fn glyph(&self, c: &mut Canvas, cell: &Cell, x: i32, y: i32, color: Rgb, spill: bool) {
        let Some(cell_rect) = glyph_clip(self.m, x, y, cell) else {
            return;
        };
        let clip = cell_rect.intersect(&self.bound);
        if clip.is_empty() || blank(cell) {
            return;
        }
        let over = self.snap.alpha < 255;
        let cells = if cell.flags.contains(Flags::WIDE) {
            2
        } else {
            1
        };
        let w = self.m.cell_w * cells;
        if let Some(mask) = crate::boxdraw::mask(cell.ch, w, self.m.cell_h, self.scale) {
            let (w, h) = (w as usize, self.m.cell_h as usize);
            if over {
                c.blit_mask_over(x, y, w, h, &mask, color, clip);
            } else {
                c.blit_mask(x, y, w, h, &mask, color, true, clip);
            }
            return;
        }
        let mut buf = [0; 4];
        let joined;
        let s = match &cell.extra {
            None => &*cell.ch.encode_utf8(&mut buf),
            Some(extra) => {
                joined = format!("{}{extra}", cell.ch);
                joined.as_str()
            }
        };
        let bold = cell.flags.contains(Flags::BOLD);
        let to = match spill && cells == 1 && self.face(cell.flags).has_colour(s, bold, self.size) {
            true => Rect {
                width: 2 * self.m.cell_w,
                ..cell_rect
            },
            false => cell_rect,
        };
        self.ink(c, s, cell.flags, to, to, color);
    }

    fn face(&self, flags: Flags) -> &TextRenderer {
        match flags.contains(Flags::ITALIC) {
            true => self.text.italic(),
            false => self.text,
        }
    }

    /// `s` in `color` in the face `flags` pick, from the start of `cells` (cells of one row), its
    /// ink cut to `clip`. A colour glyph wider than `cells` draws smaller to fit them.
    fn ink(&self, c: &mut Canvas, s: &str, flags: Flags, cells: Rect, clip: Rect, color: Rgb) {
        let clip = clip.intersect(&self.bound);
        if clip.is_empty() {
            return;
        }
        let text = self.face(flags);
        let bold = flags.contains(Flags::BOLD);
        let mut size = self.size;
        if text.one_colour_glyph(s, bold, size) {
            let advance = text.measure(s, bold, size);
            if advance > cells.width {
                size = (size * cells.width.max(1) as u32 / advance as u32).max(1);
            }
        }
        let (x, baseline) = (cells.x, cells.y + self.m.baseline);
        if self.snap.alpha < 255 {
            text.draw_run_over(c, x, baseline, clip, s, bold, color, size);
        } else {
            text.draw_run(c, x, baseline, clip, s, bold, color, size);
        }
    }

    fn cursor(&self, c: &mut Canvas, r: usize, col: usize, shape: CursorShape, ox: i32, oy: i32) {
        let m = self.m;
        let Some(cell) = self.snap.row(r).get(col) else {
            return;
        };
        let (x, y) = (ox + col as i32 * m.cell_w, oy + r as i32 * m.cell_h);
        let w = if cell.flags.contains(Flags::WIDE) {
            m.cell_w * 2
        } else {
            m.cell_w
        };
        let ink = premul(self.snap.accents.cursor, 255);
        let lt = self.light();
        let thick = (m.cell_w / 4).max(2 * lt);
        match shape {
            CursorShape::Hidden => {}
            CursorShape::Block => {
                let block = Rect::new(x, y, w, m.cell_h);
                fill(c, block, self.bound, ink);
                let (_, bg, _) = colours(self.snap, cell);
                let cells = self.snap.row(r);
                match joined(self.text, cells, self.size)
                    .into_iter()
                    .find(|j| (j.start..j.end).contains(&col))
                {
                    Some(j) => {
                        let x = ox + j.start as i32 * m.cell_w;
                        let span = Rect::new(x, y, (j.end - j.start) as i32 * m.cell_w, m.cell_h);
                        self.ink(c, &j.text, cells[j.start].flags, span, block, bg);
                    }
                    None => self.glyph(c, cell, x, y, bg, false),
                }
            }
            CursorShape::Bar => fill(c, Rect::new(x, y, thick, m.cell_h), self.bound, ink),
            CursorShape::Underline => fill(
                c,
                Rect::new(x, y + m.cell_h - thick, w, thick),
                self.bound,
                ink,
            ),
            CursorShape::HollowBlock => {
                for r in [
                    Rect::new(x, y, w, lt),
                    Rect::new(x, y + m.cell_h - lt, w, lt),
                    Rect::new(x, y, lt, m.cell_h),
                    Rect::new(x + w - lt, y, lt, m.cell_h),
                ] {
                    fill(c, r, self.bound, ink);
                }
            }
        }
    }
}

/// Whether `cell` draws nothing: a space or an empty cell.
fn blank(cell: &Cell) -> bool {
    matches!(cell.ch, ' ' | '\0') && cell.extra.is_none()
}

/// Cells `start..end` of a row drawn as `text`'s one colour glyph from `start`.
#[derive(Debug, PartialEq, Eq)]
struct Joined {
    start: usize,
    end: usize,
    text: String,
}

/// The emoji sequences the emulator keeps in several cells (a flag's letters, a skin tone, a
/// ZWJ family), each a grapheme cluster that `text` draws as one colour glyph. The cells keep the
/// widths the emulator gave each char, as programs counting with wcwidth expect.
fn joined(text: &TextRenderer, cells: &[Cell], size: u32) -> Vec<Joined> {
    let next = |i: usize| match cells[i].flags.contains(Flags::WIDE) {
        true => (i + 2).min(cells.len()),
        false => i + 1,
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        let lead = &cells[i];
        let mut end = next(i);
        if crate::emoji::may_draw_in_colour(lead.ch) {
            let mut s = cell_text(lead);
            let mut parts = 1;
            while end < cells.len() {
                let longer = s.clone() + &cell_text(&cells[end]);
                if longer.graphemes(true).nth(1).is_some() {
                    break;
                }
                (s, end, parts) = (longer, next(end), parts + 1);
            }
            let bold = lead.flags.contains(Flags::BOLD);
            if parts > 1 && text.one_colour_glyph(&s, bold, size) {
                out.push(Joined {
                    start: i,
                    end,
                    text: s,
                });
            } else {
                end = next(i);
            }
        }
        i = end;
    }
    out
}

fn cell_text(cell: &Cell) -> String {
    let mut s = String::from(cell.ch);
    s.push_str(cell.extra.as_deref().unwrap_or_default());
    s
}

/// Where a glyph whose cell starts at device (`x`, `y`) may ink: two cells for a wide char, none
/// for its spacer.
fn glyph_clip(m: &Metrics, x: i32, y: i32, cell: &Cell) -> Option<Rect> {
    if cell.flags.contains(Flags::WIDE_SPACER) {
        return None;
    }
    let cells = if cell.flags.contains(Flags::WIDE) {
        2
    } else {
        1
    };
    Some(Rect::new(x, y, m.cell_w * cells, m.cell_h))
}

/// The cell under logical (`x`, `y`); `None` outside the grid.
pub fn hit(m: &Metrics, area: Rect, scale: i32, x: i32, y: i32) -> Option<Hit> {
    if !area.contains(x, y) {
        return None;
    }
    let s = scale.max(1);
    let (dx, dy) = ((x - area.x) * s, (y - area.y) * s);
    let (col, row) = ((dx / m.cell_w) as usize, (dy / m.cell_h) as usize);
    let (cols, rows) = grid_size(m, area, s);
    if col >= cols || row >= rows {
        return None;
    }
    let side = if (dx % m.cell_w) * 2 < m.cell_w {
        Side::Left
    } else {
        Side::Right
    };
    Some(Hit { col, row, side })
}

/// A terminal node over `area` holding one text node per row, and the scrollback bar's node over
/// `bar`: the view's first row, 0 at the oldest.
pub fn access(
    snap: &Snapshot,
    area: Rect,
    bar: Rect,
    tree: &mut Tree,
    parent: NodeId,
    key: impl Hash + Copy,
) {
    let grid = tree.push(parent, key, access::node(Role::Terminal, area));
    let rows = snap.rows.max(1) as i32;
    for r in 0..snap.rows {
        let y0 = area.y + area.height * r as i32 / rows;
        let y1 = area.y + area.height * (r as i32 + 1) / rows;
        let mut n = access::node(Role::Label, Rect::new(area.x, y0, area.width, y1 - y0));
        n.set_value(snap.row_text(r));
        tree.push(grid, r, n);
    }
    let mut n = access::node(Role::ScrollBar, bar);
    n.set_orientation(Orientation::Vertical);
    n.set_numeric_value(snap.scrollback.at().top as f64);
    n.set_min_numeric_value(0.0);
    n.set_max_numeric_value(snap.scrollback.history as f64);
    tree.push(parent, (key, "bar"), n);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::accesskit::Node;
    use crate::interface::Interface;
    use crate::theme::FontConfig;

    const FG: Rgb = Color::rgb(0xC0, 0xC0, 0xC0);
    const BG: Rgb = Color::rgb(0x00, 0x00, 0x00);
    const M: Metrics = Metrics {
        cell_w: 8,
        cell_h: 16,
        baseline: 12,
    };

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        for w in words {
            for b in w.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    fn write(snap: &mut Snapshot, row: usize, col: usize, s: &str, flags: Flags) {
        for (i, ch) in s.chars().enumerate() {
            let cell = &mut snap.cells[row * snap.cols + col + i];
            cell.ch = ch;
            cell.flags = flags;
        }
    }

    /// Device pixel (`x`, `y`) of a canvas.
    fn px(c: &Canvas, x: i32, y: i32) -> u32 {
        c.pixels[(y * c.device_size().0 as i32 + x) as usize]
    }

    fn tahoma() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    #[test]
    fn metrics_whole_device_pixels() {
        crate::pins::fonts();
        let t = tahoma();
        for px in [11, 13] {
            let m = metrics(&t, px, 2);
            assert_eq!(
                m.cell_w,
                t.advance('M', false, px * 2),
                "{px} px cell width"
            );
            let line = t.line(Rect::new(0, 0, 0, 0), false, px * 2).unwrap().1;
            assert_eq!(m.cell_h, line, "{px} px cell height");
        }
        let (one, two) = (metrics(&t, 13, 1), metrics(&t, 13, 2));
        assert_ne!(
            (two.cell_w, two.cell_h),
            (one.cell_w * 2, one.cell_h * 2),
            "13 px at scale 2 is laid out afresh"
        );
    }

    #[test]
    fn grid_size_floors() {
        let m = Metrics {
            cell_w: 8,
            cell_h: 16,
            baseline: 12,
        };
        assert_eq!(grid_size(&m, Rect::new(0, 0, 800, 600), 1), (100, 37));
        assert_eq!(grid_size(&m, Rect::new(0, 0, 800, 600), 2), (200, 75));
    }

    #[test]
    fn wide_char_spans_two_cells() {
        let mut wide = Cell::blank(FG, BG);
        wide.flags = Flags::WIDE;
        let mut spacer = Cell::blank(FG, BG);
        spacer.flags = Flags::WIDE_SPACER;
        assert_eq!(
            glyph_clip(&M, 10, 20, &wide),
            Some(Rect::new(10, 20, 16, 16))
        );
        assert_eq!(glyph_clip(&M, 10, 20, &spacer), None);
        let plain = Cell::blank(FG, BG);
        assert_eq!(
            glyph_clip(&M, 10, 20, &plain),
            Some(Rect::new(10, 20, 8, 16))
        );

        let mut snap = Snapshot::new(2, 1, FG, BG);
        snap.cells[0] = Cell { ch: '█', ..wide };
        snap.cells[1] = spacer;
        snap.cursor.2 = CursorShape::Hidden;
        let mut c = Canvas::new(16, 16);
        draw(
            &mut c,
            &TextRenderer::fontless(),
            &M,
            Rect::new(0, 0, 16, 16),
            &snap,
            13,
        );
        assert!(
            (0..16).all(|x| px(&c, x, 8) == FG.argb()),
            "the block fills both cells"
        );
    }

    #[test]
    fn only_dirty_rows_draw() {
        let mut snap = Snapshot::new(4, 3, FG, Color::rgb(0, 0, 0x80));
        snap.dirty = vec![false, true, false];
        let mut c = Canvas::new(32, 48);
        let pre = Color::rgb(0x12, 0x34, 0x56);
        c.fill_rect(c.bounds(), pre);
        let area = Rect::new(0, 0, 32, 48);
        let drawn = draw(&mut c, &TextRenderer::fontless(), &M, area, &snap, 13);
        assert_eq!(drawn, vec![Rect::new(0, 16, 32, 16)]);
        for y in (0..16).chain(32..48) {
            assert!(
                (0..32).all(|x| px(&c, x, y) == pre.argb()),
                "row {y} untouched"
            );
        }
        assert_eq!(px(&c, 4, 20), Color::rgb(0, 0, 0x80).argb(), "row 1 drawn");
    }

    #[test]
    fn last_row_rect_reaches_the_area_bottom() {
        let snap = Snapshot::new(4, 2, FG, BG);
        let mut c = Canvas::new(40, 40);
        let drawn = draw(
            &mut c,
            &TextRenderer::fontless(),
            &M,
            Rect::new(0, 0, 40, 40),
            &snap,
            13,
        );
        assert_eq!(
            drawn,
            vec![Rect::new(0, 0, 40, 16), Rect::new(0, 16, 40, 24)]
        );
    }

    #[test]
    fn a_background_rect_takes_the_default_background_at_its_alpha_in_device_px() {
        let mut snap = Snapshot::new(1, 1, FG, Color::rgb(0x20, 0x40, 0x60));
        snap.alpha = 128;
        let mut c = Canvas::with_scale(10, 10, 2);
        let damage = draw_background(&mut c, Rect::new(1, 2, 5, 3), &snap);
        assert_eq!(damage, Rect::new(1, 2, 5, 3));
        let want = premul(snap.bg, 128);
        assert_eq!(px(&c, 2, 4), want, "first device px");
        assert_eq!(px(&c, 11, 9), want, "last device px");
        assert_eq!(px(&c, 1, 4), 0, "left of the rect");
        assert_eq!(px(&c, 2, 10), 0, "under the rect");
    }

    #[test]
    fn box_lines_join() {
        for s in [1, 2] {
            let m = Metrics {
                cell_w: 7 * s,
                cell_h: 15 * s,
                baseline: 12 * s,
            };
            let mut snap = Snapshot::new(3, 1, FG, BG);
            write(&mut snap, 0, 0, "───", Flags::default());
            snap.cursor.2 = CursorShape::Hidden;
            let area = Rect::new(0, 0, 21, 15);
            let mut c = Canvas::with_scale(21, 15, s);
            draw(&mut c, &TextRenderer::fontless(), &m, area, &snap, 13);
            let ink: Vec<i32> = (0..15 * s)
                .filter(|&y| (0..21 * s).any(|x| px(&c, x, y) == FG.argb()))
                .collect();
            assert_eq!(
                ink.len() as i32,
                s,
                "scale {s}: the line is {s} device px thick"
            );
            for y in ink {
                assert!(
                    (0..21 * s).all(|x| px(&c, x, y) == FG.argb()),
                    "scale {s}: row {y} runs unbroken across the cells"
                );
            }
        }
    }

    #[test]
    fn inverse_swaps() {
        let mut snap = Snapshot::new(2, 1, FG, BG);
        snap.cells[1].flags = Flags::INVERSE;
        snap.cursor.2 = CursorShape::Hidden;
        let mut c = Canvas::new(16, 16);
        draw(
            &mut c,
            &TextRenderer::fontless(),
            &M,
            Rect::new(0, 0, 16, 16),
            &snap,
            13,
        );
        assert_eq!(px(&c, 2, 2), BG.argb(), "plain cell on its bg");
        assert_eq!(px(&c, 10, 2), FG.argb(), "inverse cell on its fg");
    }

    #[test]
    fn a_link_is_dotted_and_solid_once_underlined() {
        for s in [1, 2] {
            let mut snap = Snapshot::new(3, 1, FG, BG);
            snap.cells[0].flags = Flags::LINK;
            snap.cells[1].flags = Flags::LINK;
            snap.cells[2].flags = Flags::LINK | Flags::UNDERLINE;
            snap.cursor.2 = CursorShape::Hidden;
            let m = Metrics {
                cell_w: M.cell_w * s,
                cell_h: M.cell_h * s,
                baseline: M.baseline * s,
            };
            let mut c = Canvas::with_scale(24, 16, s);
            let area = Rect::new(0, 0, 24, 16);
            draw(&mut c, &TextRenderer::fontless(), &m, area, &snap, 13);
            let lt = crate::boxdraw::light(m.cell_h, s);
            let uy = m.baseline + lt;
            let on: Vec<bool> = (0..24 * s).map(|x| px(&c, x, uy) == FG.argb()).collect();
            let dots: Vec<bool> = (0..16 * s).map(|x| (x / lt) % 2 == 0).collect();
            let split = 16 * s as usize;
            assert_eq!(on[..split], dots, "the two link cells dot at scale {s}");
            assert!(
                on[split..].iter().all(|&o| o),
                "underlined: solid at scale {s}"
            );
            assert!(
                (0..24 * s).all(|x| px(&c, x, uy - 1) == BG.argb()),
                "a line deep at scale {s}"
            );
        }
    }

    #[test]
    fn selected_uses_selection_colours() {
        let mut snap = Snapshot::new(3, 1, FG, BG);
        snap.cells[0].flags = Flags::SELECTED | Flags::INVERSE;
        snap.cells[1].flags = Flags::MATCH;
        snap.cells[2].flags = Flags::MATCH | Flags::MATCH_CURRENT;
        snap.cursor.2 = CursorShape::Hidden;
        let mut c = Canvas::new(24, 16);
        draw(
            &mut c,
            &TextRenderer::fontless(),
            &M,
            Rect::new(0, 0, 24, 16),
            &snap,
            13,
        );
        let a = snap.accents;
        assert_eq!(px(&c, 2, 2), a.selection.1.argb());
        assert_eq!(px(&c, 10, 2), a.found.1.argb());
        assert_eq!(px(&c, 18, 2), a.found_current.1.argb());
    }

    fn cursor_canvas(text: &TextRenderer, shape: CursorShape, flags: Flags, s: i32) -> Canvas {
        let m = metrics(text, 11, s);
        let mut snap = Snapshot::new(5, 1, FG, BG);
        write(&mut snap, 0, 0, "Ag─▒", flags);
        snap.cursor = (0, 1, shape);
        let (w, h) = (5 * m.cell_w / s + 1, m.cell_h / s + 1);
        let mut c = Canvas::with_scale(w as u32, h as u32, s);
        draw(&mut c, text, &m, Rect::new(0, 0, w, h), &snap, 11);
        c
    }

    const SHAPES: [CursorShape; 5] = [
        CursorShape::Block,
        CursorShape::Bar,
        CursorShape::Underline,
        CursorShape::HollowBlock,
        CursorShape::Hidden,
    ];

    #[test]
    fn cursor_shapes_distinct() {
        let t = TextRenderer::fontless();
        let m = M;
        let hashes: Vec<u64> = SHAPES
            .iter()
            .map(|&shape| {
                let mut snap = Snapshot::new(2, 1, FG, BG);
                snap.cursor = (0, 0, shape);
                let mut c = Canvas::new(16, 16);
                draw(&mut c, &t, &m, Rect::new(0, 0, 16, 16), &snap, 13);
                fnv(&c)
            })
            .collect();
        for (i, h) in hashes.iter().enumerate() {
            assert!(
                hashes[..i].iter().all(|o| o != h),
                "{:?} draws like another shape",
                SHAPES[i]
            );
        }
    }

    #[test]
    fn hit_halves() {
        let area = Rect::new(10, 20, 80, 48);
        assert_eq!(
            hit(&M, area, 1, 10 + 3 * 8 + 1, 20 + 17),
            Some(Hit {
                col: 3,
                row: 1,
                side: Side::Left
            })
        );
        assert_eq!(
            hit(&M, area, 1, 10 + 3 * 8 + 5, 20 + 17),
            Some(Hit {
                col: 3,
                row: 1,
                side: Side::Right
            })
        );
        assert_eq!(hit(&M, area, 1, 9, 21), None, "left of the grid");
        assert_eq!(hit(&M, area, 1, 10 + 80, 21), None, "right of the grid");
        let m2 = Metrics {
            cell_w: 15,
            cell_h: 30,
            baseline: 24,
        };
        assert_eq!(
            hit(&m2, area, 2, 10 + 8, 20),
            Some(Hit {
                col: 1,
                row: 0,
                side: Side::Left
            }),
            "device 16 of 15"
        );
    }

    #[test]
    fn access_row_text() {
        let mut snap = Snapshot::new(4, 2, FG, BG);
        snap.cells[0].ch = 'e';
        snap.cells[0].extra = Some("\u{301}".into());
        snap.cells[1].ch = '日';
        snap.cells[1].flags = Flags::WIDE;
        snap.cells[2].flags = Flags::WIDE_SPACER;
        snap.cells[3].ch = '!';
        let mut tree = Tree::new(Node::new(Role::Window));
        snap.scrollback = Scrollback {
            history: 30,
            offset: 10,
            rows: 2,
        };
        access(
            &snap,
            Rect::new(0, 0, 32, 32),
            Rect::new(32, 0, 16, 32),
            &mut tree,
            access::ROOT,
            "grid",
        );
        let bar = tree
            .get(Tree::id_of(access::ROOT, ("grid", "bar")))
            .unwrap();
        assert_eq!(bar.role(), Role::ScrollBar);
        assert_eq!(
            (bar.numeric_value(), bar.max_numeric_value()),
            (Some(20.0), Some(30.0)),
            "the view's first row of the 30 kept above the screen"
        );
        let grid = tree.get(Tree::id_of(access::ROOT, "grid")).unwrap();
        assert_eq!(grid.role(), Role::Terminal);
        let rows: Vec<String> = grid
            .children()
            .iter()
            .map(|&id| {
                tree.get(id)
                    .unwrap()
                    .value()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_eq!(rows, vec!["e\u{301}日!".to_string(), String::new()]);
    }

    #[test]
    fn the_bar_stands_at_the_views_first_row_oldest_first() {
        let newest = Scrollback {
            history: 14,
            offset: 0,
            rows: 6,
        };
        assert_eq!(
            newest.at(),
            RowScroll {
                top: 14,
                visible: 6,
                total: 20
            }
        );
        let oldest = Scrollback {
            offset: 14,
            ..newest
        };
        assert_eq!(oldest.at().top, 0);
        assert_eq!(newest.offset_at(0), 14);
        assert_eq!(newest.offset_at(14), 0);
        assert_eq!(newest.offset_at(99), 0, "past the newest row");
    }

    #[test]
    fn the_bar_takes_the_right_edge_of_the_area() {
        assert_eq!(
            split(Rect::new(0, 30, 400, 200), 17),
            (Rect::new(0, 30, 383, 200), Rect::new(383, 30, 17, 200))
        );
        assert_eq!(
            split(Rect::new(0, 0, 10, 50), 17),
            (Rect::new(0, 0, 0, 50), Rect::new(0, 0, 10, 50)),
            "narrower than the bar"
        );
    }

    #[test]
    fn the_bars_thumb_drags_from_the_oldest_row_to_the_newest() {
        let bar = Rect::new(100, 0, 16, 109);
        let sb = Scrollback {
            history: 14,
            offset: 0,
            rows: 6,
        };
        let thumb = sb.at().parts(bar, 8).thumb.unwrap();
        assert_eq!(thumb.bottom(), bar.bottom() - 16, "at the newest rows");
        assert_eq!(
            bar_hit(sb, bar, 8, 108, thumb.y + 5),
            Some(BarHit::Thumb(5))
        );
        assert_eq!(bar_drag(sb, bar, 8, -500, 5), 14, "the top: the oldest");
        assert_eq!(bar_drag(sb, bar, 8, 500, 5), 0, "the bottom: the newest");
        assert_eq!(bar_hit(sb, bar, 8, 108, 3), Some(BarHit::Back));
        assert_eq!(
            bar_hit(sb, bar, 8, 108, thumb.y - 2),
            Some(BarHit::PageBack)
        );
        assert_eq!(bar_hit(sb, bar, 8, 99, 50), None, "left of the bar");
    }

    /// A 10x6 grid of text over `sb` beside its bar, in `look` at scale `s` as a program draws it.
    fn grid_with_bar(look: Interface, s: i32, sb: Scrollback, lit: Lit<ScrollHit>) -> Canvas {
        let t = tahoma();
        let m = metrics(&t, 11, s);
        let mut theme = crate::interfaces::theme_for(look);
        theme.look = theme.look.at_scale(s);
        let p = Paint {
            s: 1,
            ..theme.paint()
        };
        let bar_w = theme.controls().scroll;
        let mut snap = Snapshot::new(10, 6, FG, BG);
        snap.scrollback = sb;
        for row in 0..6 {
            write(&mut snap, row, 0, "drwxr-x", Flags::default());
        }
        snap.cursor.2 = CursorShape::Hidden;
        let (w, h) = ((10 * m.cell_w + s - 1) / s, (6 * m.cell_h + s - 1) / s);
        let mut c = Canvas::with_scale((w + bar_w) as u32, h as u32, s);
        let (cells, bar) = split(Rect::new(0, 0, w + bar_w, h), bar_w);
        draw(&mut c, &t, &m, cells, &snap, 11);
        draw_bar(&mut c, &p, bar, sb, lit);
        c
    }

    const LOOKS: [Interface; 3] = [Interface::Classic, Interface::Blue, Interface::Glass];

    #[test]
    fn the_bar_greys_without_scrollback() {
        crate::pins::fonts();
        let empty = Scrollback {
            rows: 6,
            ..Scrollback::default()
        };
        let kept = Scrollback {
            history: 40,
            ..empty
        };
        assert_eq!(empty.at().parts(Rect::new(0, 0, 16, 90), 8).thumb, None);
        let ink = crate::interfaces::theme_for(Interface::Classic)
            .colors
            .button_text
            .argb();
        let arrows = |sb| {
            let c = grid_with_bar(Interface::Classic, 1, sb, None);
            let x0 = c.width as i32 - 16;
            (x0..c.width as i32)
                .flat_map(|x| (0..16).map(move |y| (x, y)))
                .any(|(x, y)| px(&c, x, y) == ink)
        };
        assert!(arrows(kept), "a live arrow draws in the button text");
        assert!(!arrows(empty), "a greyed arrow draws etched");
        for look in LOOKS {
            assert_ne!(
                fnv(&grid_with_bar(look, 1, empty, None)),
                fnv(&grid_with_bar(look, 1, kept, None)),
                "{look:?}"
            );
        }
    }

    #[test]
    fn termgrid_bar_pins_per_look_at_scale_1_and_2() {
        crate::pins::fonts();
        let at = |history, offset| Scrollback {
            history,
            offset,
            rows: 6,
        };
        let held = Some((ScrollHit::Thumb, crate::pointer::Interaction::Pressed));
        let states = [
            ("empty", at(0, 0), None),
            ("newest", at(40, 0), None),
            ("back", at(40, 20), held),
        ];
        let mut got = Vec::new();
        for look in LOOKS {
            for s in [1, 2] {
                for (name, sb, lit) in states {
                    let c = grid_with_bar(look, s, sb, lit);
                    got.push((format!("{look:?} {name} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("termgrid_bar", &got);
    }

    #[test]
    fn a_one_cell_colour_emoji_inks_the_next_cell_only_when_it_is_blank() {
        crate::pins::fonts();
        let t = tahoma();
        let hued = |p: u32| {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            r.abs_diff(g) > 40 || g.abs_diff(b) > 40
        };
        for s in [1, 2] {
            let m = metrics(&t, 13, s);
            let second_cell_hued = |next: char| {
                let mut snap = Snapshot::new(3, 1, FG, BG);
                snap.cells[0].ch = '\u{2764}';
                snap.cells[0].extra = Some("\u{FE0F}".into());
                snap.cells[1].ch = next;
                snap.cursor.2 = CursorShape::Hidden;
                let (w, h) = (3 * m.cell_w / s + 1, m.cell_h / s + 1);
                let mut c = Canvas::with_scale(w as u32, h as u32, s);
                draw(&mut c, &t, &m, Rect::new(0, 0, w, h), &snap, 13);
                (m.cell_w..2 * m.cell_w).any(|x| (0..m.cell_h).any(|y| hued(px(&c, x, y))))
            };
            assert!(
                second_cell_hued(' '),
                "scale {s}: the heart reaches a blank cell"
            );
            assert!(
                !second_cell_hued('x'),
                "scale {s}: and stays out of a filled one"
            );
        }
    }

    /// Emoji sequences as the emulator stores them, each char in the cells its wcwidth gives it
    /// and zero-width chars over the cell before: (name, sequence, cells, the cells' chars).
    const SEQUENCES: [(&str, &str, usize, &[&str]); 4] = [
        ("flag", "\u{1F1EF}\u{1F1F5}", 2, &["\u{1F1EF}", "\u{1F1F5}"]),
        (
            "skin tone",
            "\u{1F44B}\u{1F3FD}",
            4,
            &["\u{1F44B}", "\u{1F3FD}"],
        ),
        (
            "family",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            6,
            &["\u{1F468}\u{200D}", "\u{1F469}\u{200D}", "\u{1F467}"],
        ),
        ("keycap", "1\u{FE0F}\u{20E3}", 1, &["1\u{FE0F}\u{20E3}"]),
    ];

    /// Whether the emulator gives `ch` one cell: ASCII and a flag's letters; the rest take two.
    fn narrow(ch: char) -> bool {
        ch.is_ascii() || ('\u{1F1E6}'..='\u{1F1FF}').contains(&ch)
    }

    /// A 10-cell row holding `parts` from column 0, then "ab"; the cursor at `cursor`.
    fn sequence_row(parts: &[&str], cursor: (usize, CursorShape)) -> Snapshot {
        let mut snap = Snapshot::new(10, 1, FG, BG);
        let mut col = 0;
        for part in parts.iter().copied().chain(["a", "b"]) {
            let mut chars = part.chars();
            let ch = chars.next().unwrap();
            let rest: String = chars.collect();
            let cell = &mut snap.cells[col];
            cell.ch = ch;
            cell.extra = (!rest.is_empty()).then(|| rest.into());
            if narrow(ch) {
                col += 1;
            } else {
                cell.flags = Flags::WIDE;
                snap.cells[col + 1].flags = Flags::WIDE_SPACER;
                col += 2;
            }
        }
        snap.cursor = (0, cursor.0, cursor.1);
        snap
    }

    fn draw_row(t: &TextRenderer, m: &Metrics, s: i32, snap: &Snapshot) -> Canvas {
        let (w, h) = (10 * m.cell_w / s + 1, m.cell_h / s + 1);
        let mut c = Canvas::with_scale(w as u32, h as u32, s);
        draw(&mut c, t, m, Rect::new(0, 0, w, h), snap, 13);
        c
    }

    fn columns(c: &Canvas, x: std::ops::Range<i32>, h: i32) -> Vec<u32> {
        x.flat_map(|x| (0..h).map(move |y| px(c, x, y))).collect()
    }

    #[test]
    fn an_emoji_sequence_draws_as_one_glyph_fitted_to_its_cells_and_the_text_after_keeps_its_column()
     {
        crate::pins::fonts();
        let t = tahoma();
        for s in [1, 2] {
            let m = metrics(&t, 13, s);
            for (name, seq, n, parts) in SEQUENCES {
                assert!(t.one_colour_glyph(seq, false, 13 * s as u32), "{name}");
                let hidden = (0, CursorShape::Hidden);
                let c = draw_row(&t, &m, s, &sequence_row(parts, hidden));
                let span = n as i32 * m.cell_w;

                let size = 13 * s as u32;
                let fit = size.min(size * span as u32 / t.measure(seq, false, size) as u32);
                let mut alone = Canvas::with_scale(c.width, c.height, s);
                alone.in_device_px(|a| {
                    a.fill_rect(a.bounds(), BG);
                    let clip = Rect::new(0, 0, span, m.cell_h);
                    t.draw_run(a, 0, m.baseline, clip, seq, false, FG, fit);
                });
                assert!(
                    columns(&c, 0..span, m.cell_h) == columns(&alone, 0..span, m.cell_h),
                    "scale {s} {name}: the sequence's whole glyph from its first cell, fitted to its {n} cells"
                );

                let mut text_only = sequence_row(&[], hidden);
                text_only.cells.rotate_right(n);
                let after = draw_row(&t, &m, s, &text_only);
                let rest = span..10 * m.cell_w;
                assert!(
                    columns(&c, rest.clone(), m.cell_h) == columns(&after, rest, m.cell_h),
                    "scale {s} {name}: \"ab\" from column {n}"
                );
            }
        }
    }

    #[test]
    fn a_block_cursor_on_a_sequence_shows_the_part_of_its_glyph_under_the_block() {
        crate::pins::fonts();
        let t = tahoma();
        let hued = |p: u32| {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            r.abs_diff(g) > 40 || g.abs_diff(b) > 40
        };
        let parts = SEQUENCES[1].3;
        for s in [1, 2] {
            let m = metrics(&t, 13, s);
            let block = |col: usize| {
                let c = draw_row(&t, &m, s, &sequence_row(parts, (col, CursorShape::Block)));
                let x = col as i32 * m.cell_w;
                columns(&c, x..x + 2 * m.cell_w, m.cell_h)
                    .into_iter()
                    .any(hued)
            };
            assert!(block(0), "scale {s}: the hand under a block on its cell");
            assert!(
                !block(2),
                "scale {s}: a block on the skin tone's cell shows no lone swatch"
            );
        }
    }

    #[test]
    fn termgrid_sequence_pins_at_scale_1_and_2() {
        crate::pins::fonts();
        let t = tahoma();
        let mut got = Vec::new();
        for s in [1, 2] {
            let m = metrics(&t, 13, s);
            for (name, _, n, parts) in SEQUENCES {
                let lead = parts[parts.len() - 1].chars().next().unwrap();
                let last = n - if narrow(lead) { 1 } else { 2 };
                let states = [("", 0), (" cursor", 0), (" cursor last", last)];
                for (cursor, col) in states
                    .into_iter()
                    .filter(|s| s.1 > 0 || s.0 != " cursor last")
                {
                    let shape = match cursor {
                        "" => CursorShape::Hidden,
                        _ => CursorShape::Block,
                    };
                    let snap = sequence_row(parts, (col, shape));
                    let c = draw_row(&t, &m, s, &snap);
                    got.push((format!("{name}{cursor} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("termgrid_sequences", &got);
    }

    #[test]
    fn termgrid_pins_at_scale_1_and_2() {
        crate::pins::fonts();
        let t = tahoma();
        let attrs = [
            ("plain", Flags::default()),
            ("bold", Flags::BOLD),
            ("italic", Flags::ITALIC),
            ("underline", Flags::UNDERLINE),
            ("link", Flags::LINK),
            ("link_hover", Flags::LINK | Flags::UNDERLINE),
            ("strike", Flags::STRIKE),
            ("inverse", Flags::INVERSE),
            ("dim", Flags::DIM),
            ("selected", Flags::SELECTED),
            ("match", Flags::MATCH),
            ("match_current", Flags::MATCH | Flags::MATCH_CURRENT),
        ];
        let mut got = Vec::new();
        for s in [1, 2] {
            for (name, flags) in attrs {
                let c = cursor_canvas(&t, CursorShape::Hidden, flags, s);
                got.push((format!("{name} {s}"), fnv(&c)));
            }
            for shape in &SHAPES[..4] {
                let c = cursor_canvas(&t, *shape, Flags::default(), s);
                got.push((format!("cursor {shape:?} {s}"), fnv(&c)));
            }
        }
        crate::pins::check("termgrid", &got);
    }
}
