//! The copy dialog's throughput graph: the rate across the put so far as a filled area in the
//! glass colour over a faint grid, and a rule at the current rate.

use super::Mode;
use crate::copyart::{Graph, GraphLook};
use crate::raster::Canvas;
use crate::shape::{self, Px, mix, premul};
use crate::theme::{Color, ColorScheme};

pub(super) fn look(mode: Mode) -> GraphLook {
    let (heading, band_rule) = match mode {
        Mode::Light => (Color::rgb(0x00, 0x33, 0x99), Color::rgb(0xDF, 0xDF, 0xDF)),
        Mode::Dark => (Color::rgb(0x99, 0xCC, 0xFF), Color::rgb(0x3A, 0x3A, 0x3A)),
    };
    GraphLook {
        paint,
        heading,
        band: super::colors(mode).button_face,
        band_rule,
    }
}

const GRID_COLS: i32 = 10;
const GRID_ROWS: i32 = 4;

fn paint(g: &Graph, cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let k = f64::from(d);
    let (dw, dh) = (w * d, h * d);
    let paper = premul(cs.window, 255);
    let line = mix(paper, premul(cs.window_text, 255), 0.07);
    let frame = mix(paper, premul(cs.window_text, 255), 0.22);
    let accent = premul(g.accent, 255);
    let top_fill = mix(paper, accent, 0.30);
    let low_fill = mix(paper, accent, 0.55);
    let edge = mix(accent, premul(cs.window_text, 255), 0.15);
    let rule = mix(paper, premul(cs.window_text, 255), 0.65);
    let peak = g.peak();
    let inner = f64::from(h - 2) * k;
    let filled_x = k + g.fraction.clamp(0.0, 1.0) * f64::from(w - 2) * k;
    let rule_y = g.rule_y(h) * k;
    // The area's top edge, in device rows from the top, per device column.
    let tops: Vec<f64> = (0..dw)
        .map(|x| {
            let t = ((f64::from(x) + 0.5 - k) / (f64::from(w - 2) * k)).clamp(0.0, 1.0);
            k + inner - (g.rate_at(t) / peak).clamp(0.0, 1.0) * inner
        })
        .collect();
    let mut c = Canvas::new(dw as u32, dh as u32);
    for y in 0..dh {
        for x in 0..dw {
            let (fx, fy) = (f64::from(x), f64::from(y));
            let border = x < d || y < d || x >= dw - d || y >= dh - d;
            let mut px: Px = paper;
            let cell_x = (f64::from(w - 2) * k) / f64::from(GRID_COLS);
            let cell_y = inner / f64::from(GRID_ROWS);
            let on_grid = |v: f64, cell: f64| {
                let m = (v - k).rem_euclid(cell);
                m < 1.0 && v > k
            };
            if on_grid(fx, cell_x) || on_grid(fy, cell_y) {
                px = line;
            }
            if fx < filled_x {
                let top = tops[x as usize];
                let cov = (fy + 1.0 - top).clamp(0.0, 1.0);
                if cov > 0.0 {
                    let t = ((fy - top) / (inner - (top - k)).max(1.0)).clamp(0.0, 1.0);
                    let fill = mix(top_fill, low_fill, t);
                    px = shape::over(shape::scale(fill, cov), shape::scale(px, 1.0 - cov));
                }
                let near = (fy + 0.5 - top).abs();
                let stroke = (0.75 * k + 0.5 - near).clamp(0.0, 1.0);
                if stroke > 0.0 {
                    px = mix(px, edge, stroke);
                }
            }
            let lead = (1.0 - (fx + 0.5 - filled_x).abs()).clamp(0.0, 1.0);
            if lead > 0.0 && g.fraction > 0.0 && g.fraction < 1.0 {
                px = mix(px, edge, lead * 0.6);
            }
            let r = (k * 0.5 + 0.5 - (fy + 0.5 - rule_y).abs()).clamp(0.0, 1.0);
            if r > 0.0 && g.rate > 0.0 {
                px = mix(px, rule, r);
            }
            if border {
                px = frame;
            }
            c.put_raw(x, y, shape::pack(px));
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(samples: &[f32], fraction: f64, rate: f64) -> Graph<'_> {
        Graph {
            samples,
            fraction,
            rate,
            accent: Color::rgb(0x2E, 0x8C, 0xFF),
        }
    }

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    fn samples() -> Vec<f32> {
        (0..=100)
            .map(|i| 8.0e6 + (i as f32 * 0.21).sin() * 2.0e6)
            .collect()
    }

    #[test]
    fn the_area_fills_only_as_far_as_the_put_has_got() {
        let s = samples();
        let cs = super::super::colors(Mode::Light);
        let c = paint(&graph(&s, 0.4, 8.0e6), &cs, 200, 80, 1);
        let bottom = 77;
        assert_ne!(
            c.get(40, bottom),
            cs.window.argb(),
            "filled behind the lead"
        );
        assert_eq!(c.get(150, bottom), cs.window.argb(), "clear past it");
    }

    #[test]
    fn the_rule_sits_at_the_current_rate() {
        let s = samples();
        let g = graph(&s, 0.4, 8.0e6);
        let cs = super::super::colors(Mode::Light);
        let c = paint(&g, &cs, 200, 80, 1);
        let y = g.rule_y(80).round() as i32;
        assert_ne!(
            c.get(180, y),
            cs.window.argb(),
            "the rule crosses the clear part"
        );
    }

    #[test]
    fn the_graph_is_pinned_in_both_modes_at_1_and_2() {
        let s = samples();
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            let cs = super::super::colors(mode);
            for (name, f) in [("start", 0.0), ("middle", 0.37), ("done", 1.0)] {
                for d in [1, 2] {
                    let c = paint(&graph(&s, f, 7.5e6), &cs, 388, 84, d);
                    assert_eq!(c.width, 388 * d as u32);
                    got.push((format!("{mode:?} {name} @{d}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("glass/copy_graph", &got);
    }
}
