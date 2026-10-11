//! The window overview as a model: every window of the current desktop
//! in a grid over the dimmed screen, one cell each, a picture of the
//! window above its icon and title. Each output shows a grid of the
//! windows on it ([`assign`]); the windows are numbered grid by grid. The
//! arrows move the selection and cross to the next output at a grid's
//! edge, Enter or a click activates a window, Escape closes.
//!
//! The grid is the same in every look: [`OverviewLayout::compute`] picks
//! the column count that gives the largest pictures for the area's shape,
//! centres each row and the rows as a block. [`hit`] finds the cell under
//! the pointer and [`access`] lists the windows. The looks draw the cells
//! (`controls::overview` for Classic, `blue::overview`, `glass::overview`,
//! dispatched by `interfaces::draw_overview`); the host lays the live
//! pictures over them.
//!
//! Snap assist offers the other windows in a snapped zone's free part as
//! cards: [`OverviewLayout::compute_assist`] lays them out in the zone's
//! [`assist_panel`], each a header over its picture, and
//! `interfaces::draw_snap_assist` draws them.

use crate::access::{self, Tree, accesskit};
use crate::raster::{Canvas, Rect};
use accesskit::{Node, NodeId, Role};

/// Space kept clear round the grid, inside the area.
pub const MARGIN: i32 = 48;
/// Space between two cells, across and down.
pub const GAP: i32 = 24;
/// The picture's inset from its cell's top, left and right edges.
pub const PAD: i32 = 8;
/// The title row under the picture: the 16 px icon and the title.
pub const TITLE_H: i32 = 24;
/// Below the title row, to the cell's bottom.
pub const BOTTOM: i32 = 4;
/// The small icon in the title row and the gap after it.
pub const ICON_PX: i32 = 16;
pub const ICON_GAP: i32 = 4;
/// The icon centred in the box of a window with no picture.
pub const STAND_IN_PX: i32 = 48;
/// The widest a picture box gets, in fifths of the grid's width: one
/// window shows at two fifths, not across the screen.
const MAX_FIFTHS: i32 = 2;
/// Snap assist: the panel's inset from its zone, a card's header, the
/// space between cards and round them inside the panel, the frame round
/// a card's picture, and its close button.
pub const ASSIST_INSET: i32 = 13;
pub const ASSIST_HEADER: i32 = 39;
pub const ASSIST_GAP: i32 = 26;
pub const ASSIST_FRAME: i32 = 8;
pub const ASSIST_CLOSE: i32 = 28;

/// The cells of the overview, relative to `area`'s top-left: each cell
/// (the selection box), the box its picture fits in and its title row,
/// in reading order, and the index of each row's first cell. `scale` is
/// device pixels per logical pixel: 1 from `compute`, `n` from
/// `scaled(n)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverviewLayout {
    pub area: Rect,
    pub cells: Vec<Rect>,
    pub pictures: Vec<Rect>,
    pub titles: Vec<Rect>,
    pub rows: Vec<usize>,
    pub scale: i32,
}

impl OverviewLayout {
    /// Lay `count` windows out over `area`: every picture box the area's
    /// shape, as large as the column count that suits them best allows
    /// (capped at two fifths of the grid's width), `GAP` between cells, the
    /// rows centred and the block of rows centred in the area less
    /// `MARGIN`.
    pub fn compute(count: usize, area: Rect) -> OverviewLayout {
        Self::lay_out(count, area, (area.width, area.height), MAX_FIFTHS)
    }

    /// Lay `count` windows out over `area`, a part of a screen `screen`
    /// big (snap assist's zone): every picture box the screen's shape, as
    /// large as the area allows, with no cap on its width.
    pub fn compute_in(count: usize, area: Rect, screen: (i32, i32)) -> OverviewLayout {
        Self::lay_out(count, area, screen, 5)
    }

    /// The grid over `area` with picture boxes of `shape`, each at most
    /// `fifths` fifths of the grid's width.
    fn lay_out(count: usize, area: Rect, shape: (i32, i32), fifths: i32) -> OverviewLayout {
        let inner = Rect::new(
            MARGIN,
            MARGIN,
            (area.width - 2 * MARGIN).max(1),
            (area.height - 2 * MARGIN).max(1),
        );
        let (aw, ah) = (i64::from(shape.0.max(1)), i64::from(shape.1.max(1)));
        let chrome = (2 * PAD, PAD + TITLE_H + BOTTOM);
        let (cols, bw, bh) = best_box(
            count,
            inner,
            GAP,
            chrome,
            (aw, ah),
            inner.width * fifths / 5,
        );
        let (cw, ch) = (bw + 2 * PAD, PAD + bh + TITLE_H + BOTTOM);
        let nrows = count.div_ceil(cols);
        let block_h = nrows as i32 * ch + (nrows as i32 - 1).max(0) * GAP;
        let top = inner.y + (inner.height - block_h) / 2;
        let mut cells = Vec::with_capacity(count);
        let mut rows = Vec::with_capacity(nrows);
        for r in 0..nrows {
            let first = r * cols;
            rows.push(first);
            let k = (count - first).min(cols) as i32;
            let row_w = k * cw + (k - 1) * GAP;
            let left = inner.x + (inner.width - row_w) / 2;
            let y = top + r as i32 * (ch + GAP);
            for c in 0..k {
                cells.push(Rect::new(left + c * (cw + GAP), y, cw, ch));
            }
        }
        let pictures = cells
            .iter()
            .map(|c| Rect::new(c.x + PAD, c.y + PAD, bw, bh))
            .collect();
        let titles = cells
            .iter()
            .map(|c| Rect::new(c.x + PAD, c.y + PAD + bh, bw, TITLE_H))
            .collect();
        OverviewLayout {
            area,
            cells,
            pictures,
            titles,
            rows,
            scale: 1,
        }
    }

    /// Snap assist's cards for `count` windows in zone `area` of a screen
    /// `screen` big: a centred grid in the panel, [`ASSIST_GAP`] apart and
    /// round, each card its [`ASSIST_HEADER`] row (icon and title) over a
    /// picture box of the screen's shape inside an [`ASSIST_FRAME`].
    /// `titles` are the headers.
    pub fn compute_assist(count: usize, area: Rect, screen: (i32, i32)) -> OverviewLayout {
        let panel = assist_panel((area.width, area.height), 1);
        let inner = Rect::new(
            panel.x + ASSIST_GAP,
            panel.y + ASSIST_GAP,
            (panel.width - 2 * ASSIST_GAP).max(1),
            (panel.height - 2 * ASSIST_GAP).max(1),
        );
        let shape = (i64::from(screen.0.max(1)), i64::from(screen.1.max(1)));
        let chrome = (2 * ASSIST_FRAME, ASSIST_HEADER + ASSIST_FRAME);
        let (cols, bw, bh) = best_box(count, inner, ASSIST_GAP, chrome, shape, inner.width);
        let (cw, ch) = (bw + chrome.0, bh + chrome.1);
        let nrows = count.div_ceil(cols.max(1));
        let block_h = nrows as i32 * ch + (nrows as i32 - 1).max(0) * ASSIST_GAP;
        let top = inner.y + (inner.height - block_h) / 2;
        let (mut cells, mut rows) = (Vec::with_capacity(count), Vec::with_capacity(nrows));
        for r in 0..nrows {
            let first = r * cols;
            rows.push(first);
            let k = (count - first).min(cols) as i32;
            let row_w = k * cw + (k - 1) * ASSIST_GAP;
            let left = inner.x + (inner.width - row_w) / 2;
            let y = top + r as i32 * (ch + ASSIST_GAP);
            for c in 0..k {
                cells.push(Rect::new(left + c * (cw + ASSIST_GAP), y, cw, ch));
            }
        }
        let pictures = (cells.iter())
            .map(|c| Rect::new(c.x + ASSIST_FRAME, c.y + ASSIST_HEADER, bw, bh))
            .collect();
        let titles = (cells.iter())
            .map(|c| Rect::new(c.x, c.y, c.width, ASSIST_HEADER))
            .collect();
        OverviewLayout {
            area,
            cells,
            pictures,
            titles,
            rows,
            scale: 1,
        }
    }

    /// How many of `count` cards [`compute_assist`](Self::compute_assist)
    /// fits in zone `area`'s panel, none of them past its edges.
    pub fn assist_fits(count: usize, area: Rect, screen: (i32, i32)) -> usize {
        let panel = assist_panel((area.width, area.height), 1);
        (1..=count)
            .rev()
            .find(|&n| {
                let l = Self::compute_assist(n, area, screen);
                l.cells.iter().all(|c| c.intersect(&panel) == *c)
            })
            .unwrap_or(0)
    }

    /// Card `i`'s close button: square, at its header's right end, 5 px
    /// in, centred down the header.
    pub fn assist_close(&self, i: usize) -> Option<Rect> {
        let s = self.scale.max(1);
        let t = *self.titles.get(i)?;
        let side = ASSIST_CLOSE * s;
        Some(Rect::new(
            t.right() - 5 * s - side,
            t.y + (t.height - side) / 2,
            side,
            side,
        ))
    }

    /// The same layout in device pixels for drawing at scale `n`.
    pub fn scaled(&self, n: i32) -> OverviewLayout {
        let n = n.max(1);
        let all = |v: &[Rect]| v.iter().map(|r| r.scaled(n)).collect();
        OverviewLayout {
            area: self.area.scaled(n),
            cells: all(&self.cells),
            pictures: all(&self.pictures),
            titles: all(&self.titles),
            rows: self.rows.clone(),
            scale: self.scale * n,
        }
    }

    /// The box a picture fits in widened by the 2 px edge
    /// `taskpreview::fit` takes off, for the host's live picture.
    pub fn well(&self, i: usize) -> Option<Rect> {
        let s = self.scale.max(1);
        self.pictures
            .get(i)
            .map(|r| Rect::new(r.x - 2 * s, r.y - 2 * s, r.width + 4 * s, r.height + 4 * s))
    }

    /// The row cell `i` is in.
    fn row_of(&self, i: usize) -> usize {
        self.rows.iter().rposition(|&f| f <= i).unwrap_or(0)
    }

    /// The cells of row `r`.
    fn row(&self, r: usize) -> std::ops::Range<usize> {
        let start = self.rows[r];
        let end = self.rows.get(r + 1).copied().unwrap_or(self.cells.len());
        start..end
    }
}

/// The column count giving the widest picture boxes for `count` windows
/// in `inner`, cells `gap` apart, each box `chrome` (w, h) smaller than
/// its cell and of `shape`, at most `max_w` wide: (columns, box w, box h).
/// Of the column counts giving the widest boxes, the fewest.
fn best_box(
    count: usize,
    inner: Rect,
    gap: i32,
    chrome: (i32, i32),
    shape: (i64, i64),
    max_w: i32,
) -> (usize, i32, i32) {
    let (aw, ah) = shape;
    // The box width `cols` columns allow: limited by the cell's width,
    // and by its height through the shape.
    let box_w = |cols: usize| -> i32 {
        let cols = cols as i32;
        let rows = count.div_ceil(cols as usize).max(1) as i32;
        let cell_w = (inner.width - (cols - 1) * gap) / cols;
        let cell_h = (inner.height - (rows - 1) * gap) / rows;
        let by_w = cell_w - chrome.0;
        let by_h = ((i64::from(cell_h - chrome.1) * aw) / ah) as i32;
        by_w.min(by_h)
    };
    let n = count.max(1);
    let cols = (1..=n)
        .max_by_key(|&c| (box_w(c), std::cmp::Reverse(c)))
        .unwrap_or(1);
    let bw = box_w(cols).min(max_w).max(1);
    let bh = ((i64::from(bw) * ah / aw) as i32).max(1);
    (cols, bw, bh)
}

/// Snap assist's panel in a zone `size` big at scale `s`, local to the
/// zone.
pub fn assist_panel(size: (i32, i32), s: i32) -> Rect {
    let i = ASSIST_INSET * s;
    Rect::new(i, i, size.0 - 2 * i, size.1 - 2 * i)
}

/// The card whose close button is at (`x`, `y`), from the area's
/// top-left: only the `hot` card shows one.
pub fn assist_close_hit(
    layout: &OverviewLayout,
    hot: Option<usize>,
    x: i32,
    y: i32,
) -> Option<usize> {
    hot.filter(|&i| layout.assist_close(i).is_some_and(|r| r.contains(x, y)))
}

/// The cell under `(x, y)`, relative to the area's top-left in the
/// layout's own pixels; the gaps and the margin hit nothing.
pub fn hit(layout: &OverviewLayout, x: i32, y: i32) -> Option<usize> {
    layout.cells.iter().position(|c| c.contains(x, y))
}

/// The window under `(x, y)` over `grids`, in the coordinates their
/// areas are in, numbered grid by grid.
pub fn hit_grids(grids: &[OverviewLayout], x: i32, y: i32) -> Option<usize> {
    let mut first = 0;
    for g in grids {
        if let Some(i) = hit(g, x - g.area.x, y - g.area.y) {
            return Some(first + i);
        }
        first += g.cells.len();
    }
    None
}

/// Which of `outputs` each of `windows` goes to: the one holding most of
/// its frame, the first of equals. A window off every output goes to the
/// one nearest its centre. All go to 0 with no output.
pub fn assign(windows: &[Rect], outputs: &[Rect]) -> Vec<usize> {
    let area = |r: Rect| i64::from(r.width.max(0)) * i64::from(r.height.max(0));
    windows
        .iter()
        .map(|w| {
            let best = outputs
                .iter()
                .enumerate()
                .map(|(i, o)| (area(w.intersect(o)), i))
                .max_by_key(|&(a, i)| (a, std::cmp::Reverse(i)));
            match best {
                Some((a, i)) if a > 0 => i,
                _ => {
                    let (cx, cy) = (w.x + w.width / 2, w.y + w.height / 2);
                    let dist = |o: &Rect| {
                        let dx = (o.x - cx).max(cx - o.right() + 1).max(0);
                        let dy = (o.y - cy).max(cy - o.bottom() + 1).max(0);
                        i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy)
                    };
                    (0..outputs.len())
                        .min_by_key(|&i| dist(&outputs[i]))
                        .unwrap_or(0)
                }
            }
        })
        .collect()
}

/// Global window `i` as its grid and its cell there.
fn locate(grids: &[OverviewLayout], i: usize) -> Option<(usize, usize)> {
    let mut first = 0;
    for (g, l) in grids.iter().enumerate() {
        if i < first + l.cells.len() {
            return Some((g, i - first));
        }
        first += l.cells.len();
    }
    None
}

/// The number of grid `g`'s first window.
fn first_of(grids: &[OverviewLayout], g: usize) -> usize {
    grids[..g].iter().map(|l| l.cells.len()).sum()
}

/// A key the overview acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Tab,
    BackTab,
    Enter,
    Escape,
}

/// What the host does after the model took an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    /// Activate window `i` and close.
    Activate(usize),
    Close,
}

/// The overview's state: how many windows it shows, the selected one
/// (the keys move it) and the one under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overview {
    pub len: usize,
    pub selected: usize,
    pub hot: Option<usize>,
}

impl Overview {
    pub fn new(len: usize) -> Overview {
        Overview {
            len,
            selected: 0,
            hot: None,
        }
    }

    /// The window list changed to `len` entries: the selection stays in
    /// range and the hovered cell, whose index may now name another, goes.
    pub fn set_len(&mut self, len: usize) {
        self.len = len;
        self.selected = self.selected.min(len.saturating_sub(1));
        self.hot = None;
    }

    /// A key, on `grids` (one per output, in their areas' coordinates):
    /// Left and Right step in reading order, Up and Down go to the
    /// nearest cell of the row above or below, and at the grid's edge each
    /// crosses to the nearest cell of the next grid that way. Home and End
    /// go to the grid's first and last, Tab and Shift+Tab step through
    /// every window and wrap. Enter activates the selection, Escape
    /// closes.
    pub fn key(&mut self, grids: &[OverviewLayout], key: Key) -> Event {
        if key == Key::Escape {
            return Event::Close;
        }
        if self.len == 0 {
            return if key == Key::Enter {
                Event::Close
            } else {
                Event::None
            };
        }
        let (n, at) = (self.len, self.selected.min(self.len - 1));
        let to = match key {
            Key::Enter => return Event::Activate(at),
            Key::Tab => (at + 1) % n,
            Key::BackTab => (at + n - 1) % n,
            _ => match locate(grids, at) {
                Some((g, j)) => {
                    let base = first_of(grids, g);
                    let l = &grids[g];
                    let last = l.cells.len() - 1;
                    let row = l.row(l.row_of(j));
                    let within = match key {
                        Key::Home => Some(0),
                        Key::End => Some(last),
                        Key::Left if j != row.start => Some(j - 1),
                        Key::Right if j + 1 != row.end => Some(j + 1),
                        Key::Up | Key::Down => vertical(l, j, key == Key::Down),
                        _ => None,
                    };
                    match within {
                        Some(j) => base + j,
                        // At the edge: the next grid that way, else on in
                        // reading order inside this one.
                        None => cross(grids, g, j, key).unwrap_or(match key {
                            Key::Left => base + j.saturating_sub(1),
                            Key::Right => base + (j + 1).min(last),
                            _ => at,
                        }),
                    }
                }
                None => at,
            },
        };
        let to = to.min(n - 1);
        if to == self.selected && self.hot.is_none() {
            return Event::None;
        }
        self.selected = to;
        // The keys own the highlight until the pointer next moves.
        self.hot = None;
        Event::Redraw
    }

    /// The pointer is on `cell`, or off every cell.
    pub fn hover(&mut self, cell: Option<usize>) -> Event {
        let cell = cell.filter(|&i| i < self.len);
        if self.hot == cell {
            return Event::None;
        }
        self.hot = cell;
        Event::Redraw
    }

    /// A left click on `cell` activates its window; one off every cell
    /// closes the overview.
    pub fn click(&mut self, cell: Option<usize>) -> Event {
        match cell.filter(|&i| i < self.len) {
            Some(i) => {
                self.selected = i;
                Event::Activate(i)
            }
            None => Event::Close,
        }
    }
}

/// The cell of the row above (`down` false) or below `at` whose centre
/// lies nearest across; None from the first or last row.
fn vertical(layout: &OverviewLayout, at: usize, down: bool) -> Option<usize> {
    let r = layout.row_of(at);
    let r = if down {
        Some(r + 1).filter(|&r| r < layout.rows.len())?
    } else {
        r.checked_sub(1)?
    };
    let mid = |i: usize| layout.cells[i].x + layout.cells[i].width / 2;
    let x = mid(at);
    layout.row(r).min_by_key(|&i| (mid(i) - x).abs())
}

/// The window `key` reaches from cell `j` of grid `g` across that grid's
/// edge, in the nearest grid with windows lying wholly that way: Left
/// and Right enter the row nearest down at its near end, Up and Down the
/// bottom or top row at the cell nearest across.
fn cross(grids: &[OverviewLayout], g: usize, j: usize, key: Key) -> Option<usize> {
    let from = grids[g].area;
    let c = grids[g].cells[j];
    let (cx, cy) = (from.x + c.x + c.width / 2, from.y + c.y + c.height / 2);
    let gap = |o: Rect| match key {
        Key::Left => from.x - o.right(),
        Key::Right => o.x - from.right(),
        Key::Up => from.y - o.bottom(),
        _ => o.y - from.bottom(),
    };
    let sideways = matches!(key, Key::Left | Key::Right);
    let target = (0..grids.len())
        .filter(|&o| o != g && !grids[o].cells.is_empty() && gap(grids[o].area) >= 0)
        .min_by_key(|&o| {
            let a = grids[o].area;
            let off = if sideways {
                (a.y + a.height / 2 - cy).abs()
            } else {
                (a.x + a.width / 2 - cx).abs()
            };
            (gap(a), off)
        })?;
    let l = &grids[target];
    let a = l.area;
    let mid_x = |k: usize| a.x + l.cells[k].x + l.cells[k].width / 2;
    let mid_y = |k: usize| a.y + l.cells[k].y + l.cells[k].height / 2;
    let k = match key {
        Key::Left | Key::Right => {
            let r = (0..l.rows.len()).min_by_key(|&r| (mid_y(l.rows[r]) - cy).abs())?;
            let row = l.row(r);
            if key == Key::Left {
                row.end - 1
            } else {
                row.start
            }
        }
        _ => {
            let r = if key == Key::Up { l.rows.len() - 1 } else { 0 };
            l.row(r).min_by_key(|&k| (mid_x(k) - cx).abs())?
        }
    };
    Some(first_of(grids, target) + k)
}

/// One window's cell, for the painters: its title, its 16 px icon (at
/// the layout's scale), and either the picture's rect (fitted inside the
/// cell's picture box by the host, which draws the picture) or the icon
/// centred in the box in its place.
#[derive(Clone, Copy)]
pub struct Cell<'a> {
    pub title: &'a str,
    pub icon: Option<&'a Canvas>,
    pub picture: Option<Rect>,
    pub stand_in: Option<&'a Canvas>,
}

/// Where a title row's icon and text go: the icon's x, the text's pen x
/// and the text as it fits. The icon, `ICON_GAP` and the text are
/// centred as one in `row` (device pixels at scale `s`); `width` measures
/// a string, `fit` shortens one to a width.
pub fn title_run<'t>(
    row: Rect,
    title: &'t str,
    s: i32,
    has_icon: bool,
    width: impl Fn(&str) -> i32,
    fit: impl Fn(&'t str, i32) -> std::borrow::Cow<'t, str>,
) -> (i32, i32, std::borrow::Cow<'t, str>) {
    let lead = if has_icon {
        (ICON_PX + ICON_GAP) * s
    } else {
        0
    };
    let text = fit(title, row.width - lead);
    let w = lead + width(&text);
    let x = row.x + (row.width - w) / 2;
    (x, x + lead, text)
}

/// The overview's `access` part over `grids`, relative to `origin` (the
/// box the tree covers, in the grids' coordinates): a list of every
/// window by its title, the `selected` one selected, and under snap
/// assist (`close`) each card's close button, named "Close" and the
/// title. The pointer sees a card's button only while on that card; a
/// reader finds every one. Returns the selected entry's node, which has
/// the focus while the overview is up.
pub fn access<S: AsRef<str>>(
    titles: &[S],
    selected: usize,
    close: bool,
    grids: &[OverviewLayout],
    origin: Rect,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let all = Rect::new(0, 0, origin.width, origin.height);
    let list = tree.push(parent, "windows", access::node(Role::ListBox, all));
    let mut focus = list;
    for (i, title) in titles.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(title.as_ref());
        let place = locate(grids, i).map(|(g, j)| {
            let a = grids[g].area;
            let at = move |c: Rect| {
                Rect::new(
                    c.x + a.x - origin.x,
                    c.y + a.y - origin.y,
                    c.width,
                    c.height,
                )
            };
            (at, g, j)
        });
        if let Some((at, g, j)) = place {
            o.set_bounds(access::bounds(at(grids[g].cells[j])));
        }
        o.set_selected(i == selected);
        let id = tree.push(list, i, o);
        let button = place
            .filter(|_| close)
            .and_then(|(at, g, j)| Some(at(grids[g].assist_close(j)?)));
        if let Some(r) = button {
            let name = format!("Close {}", title.as_ref());
            tree.push(id, "close", access::button(r, &name, true, false));
        }
        if i == selected {
            focus = id;
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1920, 1050);

    #[test]
    fn assist_cards_sit_in_a_panel_inset_13_px_26_apart() {
        let zone = Rect::new(960, 0, 960, 1032);
        let l = OverviewLayout::compute_assist(3, zone, (1920, 1032));
        assert_eq!(
            assist_panel((zone.width, zone.height), 1),
            Rect::new(13, 13, 934, 1006)
        );
        assert_eq!(l.cells.len(), 3);
        let (a, b) = (l.cells[0], l.cells[1]);
        let apart = if a.y == b.y {
            b.x - a.right()
        } else {
            b.y - a.bottom()
        };
        assert_eq!(apart, ASSIST_GAP);
        assert_eq!(
            l.titles[0],
            Rect::new(a.x, a.y, a.width, ASSIST_HEADER),
            "the header on top"
        );
        assert_eq!(l.pictures[0].y, a.y + ASSIST_HEADER);
        assert_eq!(l.pictures[0].x, a.x + ASSIST_FRAME);
        let p = l.pictures[0];
        assert_eq!(p.width * 1032 / 1920, p.height, "the screen's shape");
        for c in &l.cells {
            assert!(Rect::new(13 + 26, 13 + 26, 934 - 52, 1006 - 52).contains(c.x, c.y));
        }
    }

    #[test]
    fn assist_offers_only_the_cards_its_panel_holds() {
        let quarter = Rect::new(0, 0, 683, 369);
        let screen = (1366, 738);
        let panel = assist_panel((quarter.width, quarter.height), 1);
        let inside = |l: &OverviewLayout| l.cells.iter().all(|c| c.intersect(&panel) == *c);
        assert!(!inside(&OverviewLayout::compute_assist(
            80, quarter, screen
        )));
        let n = OverviewLayout::assist_fits(80, quarter, screen);
        assert_eq!(n, 60);
        assert!(inside(&OverviewLayout::compute_assist(n, quarter, screen)));
        assert_eq!(OverviewLayout::assist_fits(6, quarter, screen), 6);
        assert_eq!(
            OverviewLayout::assist_fits(3, Rect::new(0, 0, 60, 60), screen),
            0
        );
    }

    #[test]
    fn only_the_hot_card_has_a_close_button() {
        let l = OverviewLayout::compute_assist(2, Rect::new(0, 0, 960, 1032), (1920, 1032));
        let c = l.assist_close(1).unwrap();
        let t = l.titles[1];
        assert_eq!((c.width, c.height), (ASSIST_CLOSE, ASSIST_CLOSE));
        assert_eq!(c.right(), t.right() - 5);
        assert_eq!(assist_close_hit(&l, Some(1), c.x + 3, c.y + 3), Some(1));
        assert_eq!(assist_close_hit(&l, Some(0), c.x + 3, c.y + 3), None);
    }

    #[test]
    fn access_names_every_cards_close_button_for_its_window() {
        let zone = Rect::new(960, 0, 960, 1032);
        let l = OverviewLayout::compute_assist(2, zone, (1920, 1032));
        let mut t = Tree::window("", (zone.width, zone.height));
        access(
            &["Notes", "Mail"],
            0,
            true,
            std::slice::from_ref(&l),
            zone,
            &mut t,
            access::ROOT,
        );
        for (i, name) in [(0, "Close Notes"), (1, "Close Mail")] {
            let id = t.find(Role::Button, name).expect(name);
            assert_eq!(
                t.get(id).unwrap().bounds(),
                Some(access::bounds(l.assist_close(i).unwrap())),
                "{name} at its place on its card, hovered or not"
            );
            let n = t.get(id).unwrap();
            assert!(
                n.supports_action(accesskit::Action::Click),
                "a reader can press it"
            );
        }
        let mut t = Tree::window("", (1920, 1032));
        let all = Rect::new(0, 0, 1920, 1032);
        access(&["Notes"], 0, false, &[l], all, &mut t, access::ROOT);
        assert_eq!(
            t.find(Role::Button, "Close Notes"),
            None,
            "the overview has none"
        );
    }

    /// Snap assist in the right third: the pictures keep the screen's
    /// shape, not the tall zone's, and fill the zone's width.
    #[test]
    fn a_zone_lays_its_pictures_out_in_the_screens_shape() {
        let zone = Rect::new(1280, 0, 640, 1050);
        let l = OverviewLayout::compute_in(2, zone, (1920, 1050));
        assert_eq!(l.rows, vec![0, 1], "one above the other");
        let p = l.pictures[0];
        // 640 - 2 * 48 - 2 * 8 = 528 wide, 528 x 1050 / 1920 = 288 tall.
        assert_eq!((p.width, p.height), (528, 288));
        let tall = OverviewLayout::compute(2, zone);
        assert!(
            tall.pictures[0].height > tall.pictures[0].width,
            "the zone's shape"
        );
    }

    #[test]
    fn one_window_is_two_fifths_of_the_grid_and_centred() {
        let l = OverviewLayout::compute(1, SCREEN);
        // The grid is 1824 wide; two fifths is 729, the box 729x398.
        assert_eq!(l.pictures[0], Rect::new(595, 316, 729, 398));
        assert_eq!(l.cells[0], Rect::new(587, 308, 745, 434));
        assert_eq!(l.titles[0], Rect::new(595, 714, 729, 24));
        assert_eq!(l.rows, [0]);
    }

    #[test]
    fn five_windows_take_three_columns_and_centre_the_short_row() {
        let l = OverviewLayout::compute(5, SCREEN);
        assert_eq!(l.rows, [0, 3]);
        let (a, d) = (l.cells[0], l.cells[3]);
        assert_eq!(a.width, d.width);
        assert_eq!(l.cells[1].x - a.right(), GAP, "cells sit a gap apart");
        assert_eq!(d.y - a.bottom(), GAP, "rows sit a gap apart");
        let mid = |r: Rect| r.x + r.width / 2;
        assert!((mid(l.cells[1]) - 960).abs() <= 1);
        assert!(((l.cells[3].x + l.cells[4].right()) / 2 - 960).abs() <= 1);
        let (top, bottom) = (a.y, l.cells[4].bottom());
        assert!(((top + bottom) / 2 - 525).abs() <= 1, "{top}..{bottom}");
        let p = l.pictures[0];
        assert!((p.width * 1050 / 1920 - p.height).abs() <= 1, "{p:?}");
    }

    #[test]
    fn many_windows_stay_inside_the_margin() {
        for n in [2, 7, 12, 30] {
            let l = OverviewLayout::compute(n, SCREEN);
            assert_eq!(l.cells.len(), n);
            for c in &l.cells {
                assert!(c.x >= MARGIN && c.right() <= 1920 - MARGIN, "{n}: {c:?}");
                assert!(c.y >= MARGIN && c.bottom() <= 1050 - MARGIN, "{n}: {c:?}");
            }
        }
    }

    #[test]
    fn hit_finds_cells_and_not_the_gaps() {
        let l = OverviewLayout::compute(3, SCREEN);
        let c = l.cells[1];
        assert_eq!(hit(&l, c.x + 1, c.y + 1), Some(1));
        assert_eq!(hit(&l, c.x - GAP / 2, c.y + 10), None);
        assert_eq!(hit(&l, 5, 5), None);
    }

    #[test]
    fn up_and_down_go_to_the_nearest_cell_across() {
        let l = OverviewLayout::compute(5, SCREEN);
        let mut o = Overview::new(5);
        o.selected = 1;
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Down), Event::Redraw);
        // Cell 1 is in the middle; of 3 and 4 below, 3 is no farther.
        assert!(o.selected == 3 || o.selected == 4);
        let below = o.selected;
        assert_eq!(
            o.key(std::slice::from_ref(&l), Key::Down),
            Event::None,
            "no row below"
        );
        assert_eq!(o.selected, below);
        o.selected = 4;
        o.key(std::slice::from_ref(&l), Key::Up);
        // Cells 1 and 2 are as near across; a tie goes to the left.
        assert_eq!(o.selected, 1);
        assert_eq!(
            o.key(std::slice::from_ref(&l), Key::Up),
            Event::None,
            "no row above"
        );
    }

    #[test]
    fn left_right_stop_at_the_ends_and_tab_wraps() {
        let l = OverviewLayout::compute(3, SCREEN);
        let mut o = Overview::new(3);
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Left), Event::None);
        o.key(std::slice::from_ref(&l), Key::Right);
        o.key(std::slice::from_ref(&l), Key::Right);
        assert_eq!(o.selected, 2);
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Right), Event::None);
        o.key(std::slice::from_ref(&l), Key::Tab);
        assert_eq!(o.selected, 0);
        o.key(std::slice::from_ref(&l), Key::BackTab);
        assert_eq!(o.selected, 2);
        o.key(std::slice::from_ref(&l), Key::Home);
        assert_eq!(o.selected, 0);
        o.key(std::slice::from_ref(&l), Key::End);
        assert_eq!(o.selected, 2);
        assert_eq!(
            o.key(std::slice::from_ref(&l), Key::Enter),
            Event::Activate(2)
        );
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Escape), Event::Close);
    }

    #[test]
    fn a_key_drops_the_hover_and_a_click_off_the_cells_closes() {
        let l = OverviewLayout::compute(3, SCREEN);
        let mut o = Overview::new(3);
        assert_eq!(o.hover(Some(2)), Event::Redraw);
        assert_eq!(o.hover(Some(2)), Event::None);
        assert_eq!(o.hover(Some(9)), Event::Redraw, "past the list is off");
        assert_eq!(o.hot, None);
        o.hover(Some(1));
        assert_eq!(
            o.key(std::slice::from_ref(&l), Key::Left),
            Event::Redraw,
            "the hover goes"
        );
        assert_eq!(o.hot, None);
        assert_eq!(o.click(Some(1)), Event::Activate(1));
        assert_eq!(o.click(None), Event::Close);
    }

    #[test]
    fn an_empty_overview_closes_on_enter() {
        let l = OverviewLayout::compute(0, SCREEN);
        assert!(l.cells.is_empty());
        let mut o = Overview::new(0);
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Right), Event::None);
        assert_eq!(o.key(std::slice::from_ref(&l), Key::Enter), Event::Close);
    }

    #[test]
    fn a_shorter_list_keeps_the_selection_in_range() {
        let mut o = Overview::new(4);
        o.selected = 3;
        o.hot = Some(2);
        o.set_len(2);
        assert_eq!((o.selected, o.hot), (1, None));
    }

    #[test]
    fn the_title_run_centres_icon_and_text_together() {
        let row = Rect::new(100, 0, 200, 24);
        let (ix, tx, t) = title_run(
            row,
            "abcd",
            1,
            true,
            |s| 10 * s.len() as i32,
            |s, _| s.into(),
        );
        // 16 + 4 + 40 = 60 wide, centred in 200: from 170.
        assert_eq!((ix, tx, t.as_ref()), (170, 190, "abcd"));
        let (ix, tx, _) = title_run(
            row,
            "abcd",
            1,
            false,
            |s| 10 * s.len() as i32,
            |s, _| s.into(),
        );
        assert_eq!((ix, tx), (180, 180));
    }

    #[test]
    fn scaled_doubles_and_the_well_widens_the_box() {
        let l = OverviewLayout::compute(2, SCREEN);
        let s = l.scaled(2);
        assert_eq!(s.cells[1], l.cells[1].scaled(2));
        assert_eq!(s.pictures[1], l.pictures[1].scaled(2));
        let w = l.well(0).unwrap();
        assert_eq!(w.inset(2), l.pictures[0]);
        assert_eq!(s.well(0).unwrap(), w.scaled(2));
    }

    #[test]
    fn the_selected_window_has_the_focus() {
        let l = OverviewLayout::compute(3, SCREEN);
        let mut t = Tree::window("", (1920, 1050));
        let titles = ["a", "b", "c"];
        let focus = access(
            &titles,
            1,
            false,
            std::slice::from_ref(&l),
            SCREEN,
            &mut t,
            access::ROOT,
        );
        let n = t.get(focus).unwrap();
        assert_eq!(n.label(), Some("b"));
        assert_eq!(n.is_selected(), Some(true));
        assert_eq!(n.bounds(), Some(access::bounds(l.cells[1])));
    }
    const RIGHT: Rect = Rect::new(1920, 0, 1280, 1024);

    #[test]
    fn a_window_goes_to_the_output_holding_most_of_it() {
        let outs = [SCREEN, RIGHT];
        let w = [
            Rect::new(100, 100, 800, 600),
            // 300 px on the left, 500 on the right.
            Rect::new(1620, 100, 800, 600),
            // 500 on the left, 300 on the right.
            Rect::new(1420, 100, 800, 600),
            // Off every output: nearest the right one.
            Rect::new(3400, 200, 100, 100),
            // Below the left one.
            Rect::new(200, 1400, 100, 100),
        ];
        assert_eq!(assign(&w, &outs), [0, 1, 0, 1, 0]);
        // An even split goes to the first.
        assert_eq!(assign(&[Rect::new(1820, 0, 200, 100)], &outs), [0]);
        assert_eq!(assign(&w[..1], &[]), [0]);
    }

    /// Three windows on the left output, two on the right one.
    fn two_grids() -> [OverviewLayout; 2] {
        [
            OverviewLayout::compute(3, SCREEN),
            OverviewLayout::compute(2, RIGHT),
        ]
    }

    #[test]
    fn hit_grids_numbers_the_windows_grid_by_grid() {
        let g = two_grids();
        let c = g[1].cells[1];
        assert_eq!(hit_grids(&g, RIGHT.x + c.x + 1, c.y + 1), Some(4));
        let c = g[0].cells[2];
        assert_eq!(hit_grids(&g, c.x + 1, c.y + 1), Some(2));
        assert_eq!(hit_grids(&g, RIGHT.x + 2, 2), None);
    }

    #[test]
    fn right_at_the_edge_crosses_to_the_next_output_and_left_comes_back() {
        let g = two_grids();
        assert_eq!(g[0].rows, [0, 2]);
        assert_eq!(g[1].rows, [0]);
        let mut o = Overview::new(5);
        o.selected = 1;
        assert_eq!(o.key(&g, Key::Right), Event::Redraw);
        assert_eq!(o.selected, 3, "the right output's nearest cell");
        o.key(&g, Key::Right);
        assert_eq!(o.selected, 4);
        assert_eq!(o.key(&g, Key::Right), Event::None, "nothing further right");
        o.selected = 3;
        o.key(&g, Key::Left);
        assert_eq!(o.selected, 1, "back to the left grid's nearest cell");
        o.selected = 2;
        o.key(&g, Key::Left);
        assert_eq!(o.selected, 1);
        o.key(&g, Key::Left);
        assert_eq!(o.selected, 0);
        assert_eq!(o.key(&g, Key::Left), Event::None);
    }

    #[test]
    fn home_and_end_stay_in_the_grid_and_tab_goes_through_every_window() {
        let g = two_grids();
        let mut o = Overview::new(5);
        o.selected = 4;
        o.key(&g, Key::Home);
        assert_eq!(o.selected, 3);
        o.key(&g, Key::End);
        assert_eq!(o.selected, 4);
        o.key(&g, Key::Tab);
        assert_eq!(o.selected, 0);
        o.key(&g, Key::End);
        assert_eq!(o.selected, 2);
        o.key(&g, Key::Tab);
        assert_eq!(o.selected, 3);
    }

    #[test]
    fn up_and_down_cross_to_an_output_above_or_below() {
        let below = Rect::new(0, 1050, 1920, 1080);
        let g = [
            OverviewLayout::compute(1, SCREEN),
            OverviewLayout::compute(0, RIGHT),
            OverviewLayout::compute(3, below),
        ];
        assert_eq!(g[2].rows, [0, 2]);
        let mut o = Overview::new(4);
        o.key(&g, Key::Down);
        assert_eq!(o.selected, 1, "the top row below, the left of two as near");
        o.selected = 3;
        o.key(&g, Key::Up);
        assert_eq!(o.selected, 1, "up inside the grid first");
        o.key(&g, Key::Up);
        assert_eq!(o.selected, 0, "then across to the output above");
        assert_eq!(o.key(&g, Key::Up), Event::None);
        o.selected = 3;
        assert_eq!(
            o.key(&g, Key::Right),
            Event::None,
            "no output to the right below"
        );
    }

    #[test]
    fn access_places_each_window_on_its_own_output() {
        let g = two_grids();
        let whole = Rect::new(0, 0, 3200, 1050);
        let mut t = Tree::window("", (3200, 1050));
        let focus = access(
            &["a", "b", "c", "d", "e"],
            4,
            false,
            &g,
            whole,
            &mut t,
            access::ROOT,
        );
        let c = g[1].cells[1];
        let r = Rect::new(c.x + RIGHT.x, c.y, c.width, c.height);
        assert_eq!(t.get(focus).unwrap().bounds(), Some(access::bounds(r)));
    }
}
