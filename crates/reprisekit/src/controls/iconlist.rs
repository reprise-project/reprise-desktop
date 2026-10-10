//! The icon list of a Change Icon box: 32 px icons in 36x44 cells, four to a
//! column, columns scrolling sideways under a horizontal bar. The host owns what
//! the icons stand for and resolves their pictures.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{ScrollHit, scrollbar_hit, scrollbar_parts};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, draw_hscrollbar};
use crate::pointer::Lit;
use crate::raster::{Canvas, Rect};
use accesskit::{NodeId, Role};

pub const CELL_W: i32 = 36;
pub const CELL_H: i32 = 44;
pub const ROWS: usize = 4;
const ICON_DX: i32 = 2;
const ICON_DY: i32 = 6;
const BAR_GAP: i32 = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct IconList {
    pub names: Vec<String>,
    pub icons: Vec<Option<Canvas>>,
    pub selected: Option<usize>,
    pub left: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub cells: Rect,
    pub bar: Rect,
    pub cols: usize,
}

pub fn layout(r: Rect, cm: &ControlMetrics) -> Layout {
    let inner = r.inset(2);
    let bar_y = inner.y + ROWS as i32 * CELL_H + BAR_GAP;
    let bar = Rect::new(inner.x, bar_y, inner.width, cm.scroll);
    let cells = Rect::new(inner.x, inner.y, inner.width, bar_y - inner.y);
    Layout {
        frame: r,
        cells,
        bar,
        cols: (inner.width / CELL_W).max(1) as usize,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Cell(usize),
    Bar(ScrollHit),
}

impl IconList {
    pub fn new(
        names: Vec<String>,
        icons: Vec<Option<Canvas>>,
        selected: Option<usize>,
    ) -> IconList {
        IconList {
            names,
            icons,
            selected,
            left: 0,
        }
    }

    pub fn columns(&self) -> usize {
        self.names.len().div_ceil(ROWS)
    }

    pub fn picked(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.names.get(i))
            .map(String::as_str)
    }

    pub fn pick(&mut self, name: &str) -> bool {
        match self.names.iter().position(|n| n == name) {
            Some(i) => {
                self.selected = Some(i);
                true
            }
            None => false,
        }
    }

    pub fn follow(&mut self, lay: &Layout) {
        if let Some(i) = self.selected {
            let col = i / ROWS;
            if col < self.left {
                self.left = col;
            } else if col >= self.left + lay.cols {
                self.left = col + 1 - lay.cols;
            }
        }
    }

    pub fn step(&mut self, key: Step) -> bool {
        let n = self.names.len();
        if n == 0 {
            return false;
        }
        let cur = self.selected.unwrap_or(0);
        let to = match key {
            Step::Up => cur.saturating_sub(1),
            Step::Down => (cur + 1).min(n - 1),
            Step::Left => cur.checked_sub(ROWS).unwrap_or(cur),
            Step::Right => {
                if cur + ROWS < n {
                    cur + ROWS
                } else {
                    cur
                }
            }
            Step::Home => 0,
            Step::End => n - 1,
        };
        let moved = self.selected != Some(to);
        self.selected = Some(to);
        moved
    }

    pub fn scroll(&mut self, by: i64, lay: &Layout) -> bool {
        let max = self.columns().saturating_sub(lay.cols);
        let to = (self.left as i64 + by).clamp(0, max as i64) as usize;
        let moved = to != self.left;
        self.left = to;
        moved
    }

    pub fn cell_rect(&self, lay: &Layout, k: usize) -> Option<Rect> {
        let col = (k / ROWS).checked_sub(self.left)?;
        let r = Rect::new(
            lay.cells.x + col as i32 * CELL_W,
            lay.cells.y + (k % ROWS) as i32 * CELL_H,
            CELL_W,
            CELL_H,
        );
        (r.x < lay.cells.right()).then_some(r)
    }

    fn parts(&self, lay: &Layout, s: i32, thumb_min: i32) -> crate::classic::widgets::ScrollParts {
        scrollbar_parts(
            lay.bar.scaled(s),
            s,
            self.left,
            lay.cols,
            self.columns(),
            true,
            thumb_min,
        )
    }

    pub fn hit(&self, lay: &Layout, cm: &ControlMetrics, x: i32, y: i32) -> Option<Hit> {
        if lay.bar.contains(x, y) {
            return scrollbar_hit(&self.parts(lay, 1, cm.thumb_min), x, y, true).map(Hit::Bar);
        }
        if !lay.cells.contains(x, y) {
            return None;
        }
        let col = ((x - lay.cells.x) / CELL_W) as usize + self.left;
        let row = ((y - lay.cells.y) / CELL_H) as usize;
        let k = col * ROWS + row;
        (row < ROWS && k < self.names.len()).then_some(Hit::Cell(k))
    }

    pub fn press(&mut self, hit: Hit, lay: &Layout) -> bool {
        match hit {
            Hit::Cell(k) => {
                let moved = self.selected != Some(k);
                self.selected = Some(k);
                moved
            }
            Hit::Bar(ScrollHit::Back) => self.scroll(-1, lay),
            Hit::Bar(ScrollHit::Forward) => self.scroll(1, lay),
            Hit::Bar(ScrollHit::PageBack) => self.scroll(-(lay.cols as i64), lay),
            Hit::Bar(ScrollHit::PageForward) => self.scroll(lay.cols as i64, lay),
            Hit::Bar(ScrollHit::Thumb) => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
}

pub fn draw(c: &mut Canvas, p: &Paint, list: &IconList, lay: &Layout, cue: bool, thumb_min: i32) {
    let Paint { cs, s, .. } = *p;
    let _ = thumb_min;
    c.fill_rect(lay.frame.scaled(s), cs.window);
    draw_field_frame(c, p, lay.frame.scaled(s), cs.window);
    let clip = lay.cells.scaled(s);
    let first = list.left * ROWS;
    let last = (first + (lay.cols + 1) * ROWS).min(list.names.len());
    for k in first..last {
        let Some(r) = list.cell_rect(lay, k) else {
            continue;
        };
        let r = r.scaled(s);
        let picked = list.selected == Some(k);
        if picked {
            c.fill_rect(r.intersect(&clip), cs.hilight);
        }
        if let Some(Some(icon)) = list.icons.get(k) {
            c.blit_over_clipped(r.x + ICON_DX * s, r.y + ICON_DY * s, icon, clip);
        }
        if picked && cue {
            crate::classic::widgets::draw_focus_ring(c, r.intersect(&clip), s, cs.hilight_text);
        }
    }
    draw_hscrollbar(
        c,
        p,
        lay.bar.scaled(s),
        list.left,
        lay.cols,
        list.columns(),
        Lit::default(),
    );
}

pub fn access(
    list: &IconList,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash + Copy,
) -> NodeId {
    let mut n = access::node(Role::ListBox, lay.frame);
    access::focusable(&mut n, true);
    let id = tree.push(parent, (key, "list"), n);
    for (k, name) in list.names.iter().enumerate() {
        let r = list.cell_rect(lay, k).unwrap_or_default();
        let mut o = access::named(Role::ListBoxOption, r, name);
        o.set_selected(list.selected == Some(k));
        tree.push(id, (key, k), o);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> IconList {
        IconList::new(
            (0..n).map(|i| format!("i{i}")).collect(),
            vec![None; n],
            Some(0),
        )
    }

    #[test]
    fn the_cells_run_down_the_columns() {
        let cm = ControlMetrics::CLASSIC;
        let lay = layout(Rect::new(11, 77, 267, 198), &cm);
        assert_eq!(lay.cells, Rect::new(13, 79, 263, 178));
        assert_eq!(lay.bar, Rect::new(13, 257, 263, 16));
        assert_eq!(lay.cols, 7);
        let l = list(40);
        assert_eq!(l.cell_rect(&lay, 0), Some(Rect::new(13, 79, 36, 44)));
        assert_eq!(l.cell_rect(&lay, 1), Some(Rect::new(13, 123, 36, 44)));
        assert_eq!(l.cell_rect(&lay, 4), Some(Rect::new(49, 79, 36, 44)));
        assert_eq!(l.hit(&lay, &cm, 50, 124), Some(Hit::Cell(5)));
        assert_eq!(l.columns(), 10);
    }

    #[test]
    fn the_keys_move_the_pick_and_the_view_follows() {
        let lay = layout(Rect::new(0, 0, 267, 198), &ControlMetrics::CLASSIC);
        let mut l = list(40);
        assert!(l.step(Step::Down));
        assert_eq!(l.selected, Some(1));
        assert!(l.step(Step::Right));
        assert_eq!(l.selected, Some(5));
        assert!(l.step(Step::End));
        l.follow(&lay);
        assert_eq!((l.selected, l.left), (Some(39), 3));
        assert!(l.press(Hit::Bar(ScrollHit::Back), &lay));
        assert_eq!(l.left, 2);
        assert!(l.pick("i7"));
        assert_eq!(l.picked(), Some("i7"));
    }
}
