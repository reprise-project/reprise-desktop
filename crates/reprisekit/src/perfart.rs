//! What a look's performance graphs are drawn from: a usage history ([`Graph`]) scrolling in from
//! the right over a grid that scrolls with it, and a level meter, plus the geometry both share.

use crate::raster::Canvas;
use crate::shape::{self, Px};
use crate::theme::{Color, ColorScheme};

/// The grid's cell, in design px.
pub const GRID: f64 = 12.0;

/// A usage history: each line's fractions (0 to 1), oldest first, the newest at the right edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graph<'a> {
    pub lines: &'a [&'a [f32]],
    /// How many samples the width holds.
    pub span: usize,
    /// Samples taken since the graph started; the grid moves left one step per sample.
    pub ticks: u64,
}

/// Paints a graph `w` x `h` design px at `d` device px per design px.
pub type GraphPainter = fn(&Graph, &ColorScheme, i32, i32, i32) -> Canvas;

/// Paints a meter filled to `fraction`, `w` x `h` design px at `d`, leaving [`METER_LABEL_H`]
/// at its foot for the value the control writes there.
pub type MeterPainter = fn(f64, &ColorScheme, i32, i32, i32) -> Canvas;

/// The band under a meter that holds its value.
pub const METER_LABEL_H: i32 = 16;

/// A look's graph and meter painters and the ink of the meter's value.
#[derive(Clone, Copy, Debug)]
pub struct PerfLook {
    pub graph: GraphPainter,
    pub meter: MeterPainter,
    pub meter_ink: fn(&ColorScheme) -> Color,
}

impl Graph<'_> {
    /// Device px between two samples on a graph `w_dev` device px wide.
    pub fn step(&self, w_dev: f64) -> f64 {
        w_dev / (self.span.max(2) - 1) as f64
    }

    /// The grid's leftward shift in device px, under one cell.
    pub fn grid_shift(&self, w_dev: f64, d: f64) -> f64 {
        let step = self.step(w_dev).min(GRID * d);
        (self.ticks as f64 * step).rem_euclid(GRID * d)
    }

    /// Whether device px `(x, y)` of a graph `w_dev` wide lies on a grid line `d` px thick.
    pub fn on_grid(&self, x: f64, y: f64, w_dev: f64, h_dev: f64, d: f64) -> bool {
        let cell = GRID * d;
        let gx = (x + self.grid_shift(w_dev, d)).rem_euclid(cell);
        let gy = (h_dev - 1.0 - y).rem_euclid(cell);
        gx < d || gy < d
    }

    /// The device rows line `line` crosses in device column `x` of a graph `w_dev` x `h_dev`:
    /// the top and bottom of the stretch between the line's height at the column's two edges.
    /// None left of the oldest sample.
    pub fn column(&self, line: usize, x: f64, w_dev: f64, h_dev: f64) -> Option<(f64, f64)> {
        let s = self.lines.get(line)?;
        let n = s.len();
        if n == 0 {
            return None;
        }
        let step = self.step(w_dev);
        let at = |px: f64| -> Option<f64> {
            let back = (w_dev - px) / step;
            let i = (n - 1) as f64 - back;
            if i < 0.0 {
                return None;
            }
            let lo = i.floor() as usize;
            let hi = (lo + 1).min(n - 1);
            let t = i - lo as f64;
            let v = f64::from(s[lo]) * (1.0 - t) + f64::from(s[hi]) * t;
            Some((1.0 - v.clamp(0.0, 1.0)) * (h_dev - 1.0) + 0.5)
        };
        let (a, b) = (at(x)?, at(x + 1.0).or(at(x))?);
        Some((a.min(b), a.max(b)))
    }

    /// How much of device px `(x, y)` a line stroked `half` device px either side of `span`
    /// covers.
    pub fn stroke_cover(span: (f64, f64), y: f64, half: f64) -> f64 {
        let (top, bottom) = (span.0 - half, span.1 + half);
        (bottom.min(y + 1.0) - top.max(y)).clamp(0.0, 1.0)
    }

    /// How much of device px row `y` lies under (below) the line at `span`.
    pub fn fill_cover(span: (f64, f64), y: f64) -> f64 {
        let mid = (span.0 + span.1) / 2.0;
        (y + 1.0 - mid).clamp(0.0, 1.0)
    }
}

/// A graph's colours: the paper, the grid, and up to two lines, each stroked and, given a
/// fill, shaded below.
#[derive(Clone, Copy, Debug)]
pub struct GraphInk {
    pub back: Px,
    pub grid: Px,
    pub lines: [Px; 2],
    pub fills: [Option<Px>; 2],
    /// Half a line's width, in design px.
    pub half: f64,
}

/// [`Graph`] painted `w` x `h` design px at `d` in `ink`; the second line over the first.
pub fn paint_graph(g: &Graph, ink: &GraphInk, w: i32, h: i32, d: i32) -> Canvas {
    let (dw, dh) = (w.max(1) * d, h.max(1) * d);
    let (wf, hf, k) = (f64::from(dw), f64::from(dh), f64::from(d));
    let mut c = Canvas::new(dw as u32, dh as u32);
    let lines = g.lines.len().min(2);
    for x in 0..dw {
        let fx = f64::from(x);
        let spans: Vec<Option<(f64, f64)>> = (0..lines).map(|l| g.column(l, fx, wf, hf)).collect();
        for y in 0..dh {
            let fy = f64::from(y);
            let mut px = ink.back;
            if g.on_grid(fx, fy, wf, hf, k) {
                px = ink.grid;
            }
            for (l, span) in spans.iter().enumerate() {
                let Some(span) = *span else { continue };
                if let Some(fill) = ink.fills[l] {
                    px = shape::over(shape::scale(fill, Graph::fill_cover(span, fy)), px);
                }
                let cov = Graph::stroke_cover(span, fy, ink.half * k);
                px = shape::over(shape::scale(ink.lines[l], cov), px);
            }
            c.put_raw(x, y, shape::pack(px));
        }
    }
    c
}

/// The meter's lit share of `slots` segments for `fraction`: at least one while above zero.
pub fn lit_segments(fraction: f64, slots: i32) -> i32 {
    let f = fraction.clamp(0.0, 1.0);
    let n = (f * f64::from(slots)).round() as i32;
    if f > 0.0 { n.max(1) } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(lines: &[&[f32]], span: usize, ticks: u64) -> Graph<'static> {
        let lines: Vec<&'static [f32]> = lines.iter().map(|l| &*l.to_vec().leak()).collect();
        Graph {
            lines: lines.leak(),
            span,
            ticks,
        }
    }

    #[test]
    fn the_newest_sample_sits_at_the_right_edge_and_the_oldest_further_left() {
        let g = graph(&[&[0.0, 1.0]], 11, 0);
        let (w, h) = (100.0, 51.0);
        let right = g.column(0, 99.0, w, h).unwrap();
        assert!(right.0 < 6.0, "full at the right edge: {right:?}");
        let left = g.column(0, 90.0, w, h).unwrap();
        assert!(left.1 > 45.0, "empty one step back: {left:?}");
        assert_eq!(g.column(0, 80.0, w, h), None, "nothing before the oldest");
    }

    #[test]
    fn the_grid_moves_left_one_step_per_sample() {
        let w = 120.0;
        let at = |t| graph(&[&[0.5]], 31, t).grid_shift(w, 1.0);
        assert_eq!(at(0), 0.0);
        assert_eq!(at(1), 4.0);
        assert_eq!(at(3), 0.0, "three steps make a cell");
    }

    #[test]
    fn a_meter_above_zero_lights_a_segment() {
        assert_eq!(lit_segments(0.0, 20), 0);
        assert_eq!(lit_segments(0.001, 20), 1);
        assert_eq!(lit_segments(0.5, 20), 10);
        assert_eq!(lit_segments(1.5, 20), 20);
    }

    #[test]
    fn a_stroke_covers_the_rows_it_crosses() {
        assert_eq!(Graph::stroke_cover((10.0, 10.0), 9.0, 0.5), 0.5);
        assert_eq!(Graph::stroke_cover((10.0, 10.0), 12.0, 0.5), 0.0);
        assert_eq!(Graph::fill_cover((10.0, 10.0), 20.0), 1.0);
        assert_eq!(Graph::fill_cover((10.0, 10.0), 5.0), 0.0);
    }
}
