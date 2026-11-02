//! A usage history graph and a level meter, as a task manager's Performance page shows them.
//! The model is a [`History`] of fractions; both parts take the rect they fill and draw in the
//! look's [`crate::perfart::PerfLook`].

use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::{draw_perf_graph, draw_perf_meter};
use crate::perfart::Graph;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use accesskit::{NodeId, Role};

/// The last `cap` samples of one quantity, as fractions of its whole (0 to 1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    samples: Vec<f32>,
    cap: usize,
    ticks: u64,
}

impl History {
    pub fn new(cap: usize) -> History {
        History {
            samples: Vec::with_capacity(cap),
            cap: cap.max(2),
            ticks: 0,
        }
    }

    pub fn push(&mut self, v: f64) {
        if self.samples.len() == self.cap {
            self.samples.remove(0);
        }
        self.samples.push(v.clamp(0.0, 1.0) as f32);
        self.ticks += 1;
    }

    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    pub fn latest(&self) -> Option<f64> {
        self.samples.last().map(|&v| f64::from(v))
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// `lines` (at most two; the second is drawn over the first) as one graph filling `r`, each
/// spanning the first line's capacity.
pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, lines: &[&History]) {
    let Some(first) = lines.first() else {
        return;
    };
    let samples: Vec<&[f32]> = lines.iter().map(|h| h.samples()).collect();
    let g = Graph {
        lines: &samples,
        span: first.cap(),
        ticks: first.ticks(),
    };
    draw_perf_graph(c, p, r, &g);
}

/// A meter filled to `fraction` in `r`, with `value` under it.
pub fn draw_meter(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    fraction: f64,
    value: &str,
    px: u32,
) {
    draw_perf_meter(c, text, p, r, fraction, value, px);
}

/// A graph or meter's node: `label` names it and `value` reads its current figure.
pub fn access(tree: &mut Tree, parent: NodeId, r: Rect, label: &str, value: &str) -> NodeId {
    let mut n = access::named(Role::Meter, r, label);
    n.set_value(value);
    tree.push(parent, label, n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn a_full_history_drops_its_oldest_sample() {
        let mut h = History::new(3);
        for v in [0.1, 0.2, 0.3, 0.4] {
            h.push(v);
        }
        assert_eq!(h.samples(), [0.2f32, 0.3, 0.4]);
        assert_eq!(h.ticks(), 4);
        h.push(7.0);
        assert_eq!(h.latest(), Some(1.0), "clamped to the whole");
    }

    #[test]
    fn the_graph_frame_and_its_lines_are_pinned_per_interface_at_1_and_2() {
        let mut cpu = History::new(60);
        let mut swap = History::new(60);
        for i in 0..80 {
            cpu.push(0.5 + (f64::from(i) * 0.3).sin() * 0.4);
            swap.push(0.1);
        }
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(i);
            for d in [1, 2] {
                let look = theme.look.at_scale(d);
                let p = Paint {
                    look: &look,
                    ..theme.paint()
                };
                let mut c = Canvas::with_scale(220, 100, d);
                draw(&mut c, &p, Rect::new(5, 5, 200, 90), &[&cpu, &swap]);
                got.push((format!("{i:?} @{d}"), fnv(&c)));
            }
        }
        crate::pins::check("controls/perf_graph", &got);
    }
}
