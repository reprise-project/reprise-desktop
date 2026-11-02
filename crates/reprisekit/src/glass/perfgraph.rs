//! The performance graphs on the window's paper: lines in the selection colour over a faint grid,
//! shaded below, and a meter as a rounded well filling from the foot.

use crate::perfart::{self, Graph, GraphInk, METER_LABEL_H, PerfLook};
use crate::raster::Canvas;
use crate::shape::{self, mix, premul};
use crate::theme::{Color, ColorScheme};

/// The second line (swap) beside the selection colour.
const SECOND: Color = Color::rgb(0x8E, 0x44, 0xAD);
const WELL_W: i32 = 22;
const WELL_RADIUS: f64 = 3.0;
const MARGIN: i32 = 4;

pub const LOOK: PerfLook = PerfLook {
    graph,
    meter,
    meter_ink,
};

fn meter_ink(cs: &ColorScheme) -> Color {
    cs.window_text
}

fn graph(g: &Graph, cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let paper = premul(cs.window, 255);
    let text = premul(cs.window_text, 255);
    let ink = GraphInk {
        back: paper,
        grid: mix(paper, text, 0.08),
        lines: [premul(cs.hilight, 255), premul(SECOND, 255)],
        fills: [Some(premul(cs.hilight, 56)), None],
        half: 0.75,
    };
    perfart::paint_graph(g, &ink, w, h, d)
}

fn meter(fraction: f64, cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let (dw, dh) = (w.max(1) * d, h.max(1) * d);
    let k = f64::from(d);
    let paper = premul(cs.window, 255);
    let text = premul(cs.window_text, 255);
    let well = mix(paper, text, 0.10);
    let rim = mix(paper, text, 0.30);
    let accent = premul(cs.hilight, 255);
    let light = mix(accent, paper, 0.45);
    let ww = f64::from(WELL_W.min(w - 2 * MARGIN).max(4)) * k;
    let wx = (f64::from(dw) - ww) / 2.0;
    let wy = f64::from(MARGIN) * k;
    let wh = f64::from(h - METER_LABEL_H - MARGIN).max(4.0) * k - wy;
    let r = WELL_RADIUS * k;
    let top = wy + wh * (1.0 - fraction.clamp(0.0, 1.0));
    let mut c = Canvas::new(dw as u32, dh as u32);
    for y in 0..dh {
        for x in 0..dw {
            let (fx, fy) = (f64::from(x) - wx, f64::from(y) - wy);
            let outer = shape::rounded(fx, fy, ww, wh, r);
            let inner = shape::rounded(fx - k, fy - k, ww - 2.0 * k, wh - 2.0 * k, r - k);
            let filled = (f64::from(y) + 1.0 - top).clamp(0.0, 1.0);
            let t = ((f64::from(y) - wy) / wh).clamp(0.0, 1.0);
            let body = mix(well, mix(light, accent, t), filled);
            let mut px = paper;
            px = shape::over(shape::scale(rim, outer), px);
            px = mix(px, body, inner);
            c.put_raw(x, y, shape::pack(px));
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::super::Mode;
    use super::*;

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    #[test]
    fn the_area_under_the_line_is_shaded_and_above_it_is_paper() {
        let cs = super::super::colors(Mode::Light);
        let s = [0.5f32; 4];
        let g = Graph {
            lines: &[&s],
            span: 4,
            ticks: 0,
        };
        let c = graph(&g, &cs, 60, 40, 1);
        let paper = cs.window.argb();
        assert_eq!(c.get(30, 5), paper, "paper above the line");
        assert_ne!(c.get(30, 32), paper, "shaded below it");
    }

    #[test]
    fn the_meter_well_has_clear_corners_and_soft_edges_at_2() {
        let cs = super::super::colors(Mode::Light);
        let c = meter(0.6, &cs, 60, 90, 2);
        let paper = cs.window.argb();
        let wx = (60 - WELL_W) / 2 * 2;
        let wy = MARGIN * 2;
        assert_eq!(c.get(wx, wy), paper, "the well's corner stays paper");
        let soft = (0..8).any(|i| {
            let p = c.get(wx + i, wy + 1);
            p != paper && (p & 0xFF) != (c.get(wx + 20, wy + 1) & 0xFF)
        });
        assert!(soft, "the curved corner blends into the paper");
        let lit = c.get(60, 130);
        let empty = c.get(60, 30);
        assert_ne!(lit, empty, "filled from the foot, empty at the top");
    }

    #[test]
    fn the_graph_and_meter_are_pinned_in_both_modes_at_1_and_2() {
        let s: Vec<f32> = (0..60)
            .map(|i| 0.5 + (i as f32 * 0.3).sin() * 0.4)
            .collect();
        let swap: Vec<f32> = s.iter().map(|v| v * 0.3).collect();
        let mut got = Vec::new();
        for mode in [Mode::Light, Mode::Dark] {
            let cs = super::super::colors(mode);
            for d in [1, 2] {
                let g = Graph {
                    lines: &[&s, &swap],
                    span: 60,
                    ticks: 7,
                };
                got.push((
                    format!("{mode:?} graph @{d}"),
                    fnv(&graph(&g, &cs, 200, 90, d)),
                ));
                for f in [0.0, 0.37, 1.0] {
                    let key = format!("{mode:?} meter {f} @{d}");
                    got.push((key, fnv(&meter(f, &cs, 60, 90, d))));
                }
            }
        }
        crate::pins::check("glass/perf_graph", &got);
    }
}
