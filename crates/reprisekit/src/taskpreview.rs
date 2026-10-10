//! The task button's preview panel as a model: cell layout on a look's [`Grid`], pointer hit tests and picture fit.
//! `controls::taskpreview` draws Classic's panel and `glass::preview` Glass's.

use crate::raster::{Canvas, Insets, Rect};
use crate::taskbar::Edge;

pub const CELL_W: i32 = 176;
pub(crate) const TITLE_H: i32 = 18;
pub(crate) const TITLE_GAP: i32 = 4;
pub(crate) const WELL_H: i32 = 110;
pub const CELL_H: i32 = TITLE_H + TITLE_GAP + WELL_H;
const WELL_EDGE: i32 = 2;

/// A look's preview panel geometry, at scale 1: the cell's size, the panel's
/// padding and gap, and in a cell its well (the picture fits inside its 2 px
/// edge, see [`fit`]) and the hot cell's close button. `shadow` is how far
/// the canvas reaches past the panel; `corner` cuts the panel's corners as
/// `backdrop::rounded_rects` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cell: (i32, i32),
    pub pad: i32,
    pub gap: i32,
    pub well: Rect,
    pub close: Rect,
    pub shadow: Insets,
    pub corner: &'static [i32],
}

/// The panel's rect (in the coordinates of the `bounds` it was laid out
/// in) and each cell relative to the panel's top-left, laid out on `grid`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskPreviewLayout {
    pub rect: Rect,
    pub cells: Vec<Rect>,
    /// Device pixels per logical pixel: 1 from `compute`, `n` from
    /// `scaled(n)`.
    pub scale: i32,
    pub grid: Grid,
    pub edge: Edge,
}

impl TaskPreviewLayout {
    /// Lay out `n` cells of `grid` from `button` on a bar along `edge`: as
    /// many to a row as fit `bounds`, the panel centred on the button and
    /// pushed back inside `bounds` (shadow and all). It sits against the bar
    /// with its shadow's bottom (or top) on `bounds`' edge, and beside a side
    /// bar is centred on the button down the screen.
    pub fn compute(
        grid: Grid,
        n: usize,
        button: Rect,
        bounds: Rect,
        edge: Edge,
    ) -> TaskPreviewLayout {
        let n = n.max(1);
        let (cw, ch) = grid.cell;
        let sh = grid.shadow;
        let room = bounds.width - sh.left - sh.right;
        let fit = ((room - 2 * grid.pad + grid.gap) / (cw + grid.gap)).max(1) as usize;
        let cols = n.min(fit);
        let rows = n.div_ceil(cols);
        let span = |k: usize, size: i32| 2 * grid.pad + k as i32 * size + (k as i32 - 1) * grid.gap;
        let (w, h) = (span(cols, cw), span(rows, ch));
        let cells = (0..n)
            .map(|i| {
                let (c, r) = ((i % cols) as i32, (i / cols) as i32);
                Rect::new(
                    grid.pad + c * (cw + grid.gap),
                    grid.pad + r * (ch + grid.gap),
                    cw,
                    ch,
                )
            })
            .collect();
        let centred_x = (button.x + (button.width - w) / 2)
            .min(bounds.right() - sh.right - w)
            .max(bounds.x + sh.left);
        let centred_y = (button.y + (button.height - h) / 2)
            .min(bounds.bottom() - sh.bottom - h)
            .max(bounds.y + sh.top);
        let (x, y) = match edge {
            Edge::Bottom => (
                centred_x,
                (bounds.bottom() - sh.bottom - h).max(bounds.y + sh.top),
            ),
            Edge::Top => (centred_x, bounds.y + sh.bottom),
            Edge::Left => (bounds.x + sh.bottom, centred_y),
            Edge::Right => (
                (bounds.right() - sh.bottom - w).max(bounds.x + sh.left),
                centred_y,
            ),
        };
        TaskPreviewLayout {
            rect: Rect::new(x, y, w, h),
            cells,
            scale: 1,
            grid,
            edge,
        }
    }

    fn in_cell(&self, i: usize, r: Rect) -> Option<Rect> {
        let s = self.scale;
        self.cells
            .get(i)
            .map(|c| Rect::new(c.x + r.x * s, c.y + r.y * s, r.width * s, r.height * s))
    }

    pub fn well(&self, i: usize) -> Option<Rect> {
        self.in_cell(i, self.grid.well)
    }

    /// Cell `i`'s close button, relative to the panel: shown while the
    /// cell is hot.
    pub fn close(&self, i: usize) -> Option<Rect> {
        self.in_cell(i, self.grid.close)
    }

    pub(crate) fn title_row(&self, i: usize) -> Option<Rect> {
        self.in_cell(i, Rect::new(0, 0, self.grid.cell.0, TITLE_H))
    }

    /// What the panel's canvas covers, relative to the panel: the panel
    /// and its shadow.
    pub fn canvas(&self) -> Rect {
        let (s, m) = (self.scale, self.grid.shadow);
        Rect::new(
            -m.left * s,
            -m.top * s,
            self.rect.width + (m.left + m.right) * s,
            self.rect.height + (m.top + m.bottom) * s,
        )
    }

    /// Where the panel takes the pointer, relative to its canvas: the panel
    /// and the shadow's strip between it and the bar, so the pointer crosses
    /// from the button to the panel without leaving both. The rest of the
    /// shadow lets the pointer through.
    pub fn input_area(&self) -> Rect {
        let (s, m) = (self.scale, self.grid.shadow);
        match self.edge {
            Edge::Bottom => Rect::new(
                m.left * s,
                m.top * s,
                self.rect.width,
                self.rect.height + m.bottom * s,
            ),
            Edge::Top => Rect::new(
                m.left * s,
                (m.top - m.bottom) * s,
                self.rect.width,
                self.rect.height + m.bottom * s,
            ),
            Edge::Left => Rect::new(
                (m.left - m.bottom) * s,
                m.top * s,
                self.rect.width + m.bottom * s,
                self.rect.height,
            ),
            Edge::Right => Rect::new(
                m.left * s,
                m.top * s,
                self.rect.width + m.bottom * s,
                self.rect.height,
            ),
        }
    }

    pub fn scaled(&self, n: i32) -> TaskPreviewLayout {
        let n = n.max(1);
        TaskPreviewLayout {
            rect: self.rect.scaled(n),
            cells: self.cells.iter().map(|r| r.scaled(n)).collect(),
            scale: self.scale * n,
            grid: self.grid,
            edge: self.edge,
        }
    }
}

pub fn hit(layout: &TaskPreviewLayout, x: i32, y: i32) -> Option<usize> {
    layout.cells.iter().position(|c| c.contains(x, y))
}

pub fn close_hit(layout: &TaskPreviewLayout, hot: Option<usize>, x: i32, y: i32) -> Option<usize> {
    hot.filter(|&i| layout.close(i).is_some_and(|r| r.contains(x, y)))
}

/// Where a `src_w` x `src_h` picture goes in `well`: inside its sunken
/// edge, aspect kept, never enlarged, centred. Empty for an empty source.
pub fn fit(src_w: i32, src_h: i32, well: Rect, scale: i32) -> Rect {
    let inner = well.inset(WELL_EDGE * scale.max(1));
    if src_w <= 0 || src_h <= 0 || inner.is_empty() {
        return Rect::new(inner.x, inner.y, 0, 0);
    }
    let (w, h) = if src_w as i64 * inner.height as i64 <= src_h as i64 * inner.width as i64 {
        let h = src_h.min(inner.height);
        (((src_w as i64 * h as i64) / src_h as i64).max(1) as i32, h)
    } else {
        let w = src_w.min(inner.width);
        (w, ((src_h as i64 * w as i64) / src_w as i64).max(1) as i32)
    };
    Rect::new(
        inner.x + (inner.width - w) / 2,
        inner.y + (inner.height - h) / 2,
        w,
        h,
    )
}

/// One window's cell: its title and 16 px icon, whether it is the active
/// window (bold title), and the icon to centre in the well when there is
/// no live picture.
pub struct Cell<'a> {
    pub title: &'a str,
    pub icon: Option<&'a Canvas>,
    pub active: bool,
    pub picture: Option<&'a Canvas>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_keeps_the_aspect_and_never_enlarges() {
        let well = Rect::new(6, 28, 176, 110);
        assert_eq!(fit(1600, 900, well, 1), Rect::new(8, 35, 172, 96));
        assert_eq!(fit(400, 800, well, 1), Rect::new(67, 30, 53, 106));
        assert_eq!(fit(100, 50, well, 1), Rect::new(44, 58, 100, 50));
        assert!(fit(0, 50, well, 1).is_empty());
        let w2 = well.scaled(2);
        assert_eq!(fit(3200, 1800, w2, 2), Rect::new(16, 69, 344, 193));
    }

    #[test]
    fn the_panel_sits_as_far_from_a_top_bar_as_from_a_bottom_one() {
        use crate::interface::Interface;
        let bottom_bounds = Rect::new(0, 0, 1920, 1040);
        let top_bounds = Rect::new(0, 40, 1920, 1040);
        for (i, want) in [
            (Interface::Classic, 0),
            (Interface::Blue, 0),
            (Interface::Glass, 8),
        ] {
            let grid = crate::interfaces::task_preview_grid(&crate::interfaces::theme_for(i));
            let gaps = |n: usize| {
                let b = Rect::new(300, 1040, 60, 40);
                let t = Rect::new(300, 0, 60, 40);
                let over = TaskPreviewLayout::compute(grid, n, b, bottom_bounds, Edge::Bottom);
                let under = TaskPreviewLayout::compute(grid, n, t, top_bounds, Edge::Top);
                (1040 - over.rect.bottom(), under.rect.y - 40)
            };
            for n in [1, 3] {
                assert_eq!(
                    gaps(n),
                    (want, want),
                    "{i:?}, {n} cells: {want} px from the bar on both edges"
                );
            }
        }
    }

    #[test]
    fn the_panel_sits_as_far_from_a_side_bar_as_from_a_bottom_one() {
        use crate::interface::Interface;
        let left_bounds = Rect::new(60, 0, 1860, 1080);
        let right_bounds = Rect::new(0, 0, 1860, 1080);
        for (i, want) in [
            (Interface::Classic, 0),
            (Interface::Blue, 0),
            (Interface::Glass, 8),
        ] {
            let grid = crate::interfaces::task_preview_grid(&crate::interfaces::theme_for(i));
            let b = Rect::new(0, 500, 60, 40);
            let left = TaskPreviewLayout::compute(grid, 2, b, left_bounds, Edge::Left);
            assert_eq!(
                left.rect.x - 60,
                want,
                "{i:?}: {want} px right of a left bar"
            );
            assert_eq!(
                left.rect.y,
                500 + (40 - left.rect.height) / 2,
                "{i:?}: centred on the button"
            );
            let b = Rect::new(1860, 500, 60, 40);
            let right = TaskPreviewLayout::compute(grid, 2, b, right_bounds, Edge::Right);
            assert_eq!(
                1860 - right.rect.right(),
                want,
                "{i:?}: {want} px left of a right bar"
            );
            let m = grid.shadow;
            assert_eq!(
                left.input_area().x,
                m.left - m.bottom,
                "{i:?}: the input strip runs to a left bar"
            );
            assert_eq!(
                right.input_area().right(),
                m.left + right.rect.width + m.bottom,
                "{i:?}: and to a right bar"
            );
        }
    }
}
