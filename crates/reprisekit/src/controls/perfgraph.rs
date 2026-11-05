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

/// The last `cap` samples of one quantity, as fractions of its whole (0 to 1), or as given for
/// a quantity with no fixed whole ([`History::unbounded`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    samples: Vec<f32>,
    cap: usize,
    ticks: u64,
    unbounded: bool,
}

impl History {
    pub fn new(cap: usize) -> History {
        History {
            samples: Vec::with_capacity(cap),
            cap: cap.max(2),
            ticks: 0,
            unbounded: false,
        }
    }

    /// A history of a quantity such as a rate, kept as pushed; [`History::scaled`] gives the
    /// fractions a graph draws.
    pub fn unbounded(cap: usize) -> History {
        History {
            unbounded: true,
            ..History::new(cap)
        }
    }

    pub fn push(&mut self, v: f64) {
        if self.samples.len() == self.cap {
            self.samples.remove(0);
        }
        let v = if self.unbounded {
            v.max(0.0)
        } else {
            v.clamp(0.0, 1.0)
        };
        self.samples.push(v as f32);
        self.ticks += 1;
    }

    /// The largest sample held, 0 when empty.
    pub fn peak(&self) -> f64 {
        f64::from(self.samples.iter().copied().fold(0.0, f32::max))
    }

    /// This history as fractions of `whole`, scrolled as far.
    pub fn scaled(&self, whole: f64) -> History {
        let k = if whole > 0.0 { 1.0 / whole } else { 0.0 };
        History {
            samples: (self.samples.iter())
                .map(|&v| (f64::from(v) * k).clamp(0.0, 1.0) as f32)
                .collect(),
            cap: self.cap,
            ticks: self.ticks,
            unbounded: false,
        }
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
    fn an_unbounded_history_keeps_its_samples_and_scales_to_a_whole() {
        let mut h = History::unbounded(3);
        for v in [400.0, 1000.0, -5.0, 250.0] {
            h.push(v);
        }
        assert_eq!(h.samples(), [1000.0f32, 0.0, 250.0]);
        assert_eq!(h.peak(), 1000.0);
        let s = h.scaled(500.0);
        assert_eq!(s.samples(), [1.0f32, 0.0, 0.5], "clamped to the whole");
        assert_eq!(
            (s.cap(), s.ticks()),
            (3, 4),
            "scrolled as far as the original"
        );
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
