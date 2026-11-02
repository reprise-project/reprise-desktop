//! The performance graphs as a lit monitor: green lines over a dark green grid on black, and a
//! meter of green segments, dark where unlit.

use crate::perfart::{self, Graph, GraphInk, METER_LABEL_H, PerfLook};
use crate::raster::Canvas;
use crate::shape::{self, premul};
use crate::theme::{Color, ColorScheme};

const BLACK: Color = Color::rgb(0x00, 0x00, 0x00);
const GRID: Color = Color::rgb(0x00, 0x80, 0x40);
const LINE: Color = Color::rgb(0x00, 0xFF, 0x00);
const SECOND: Color = Color::rgb(0xFF, 0xFF, 0x00);
const UNLIT: Color = Color::rgb(0x00, 0x60, 0x30);

/// A meter segment's height and the gap under it, and the margin around the two columns.
const SEGMENT: i32 = 2;
const SEGMENT_GAP: i32 = 1;
const MARGIN: i32 = 4;
const COLUMN_GAP: i32 = 1;

pub const LOOK: PerfLook = PerfLook {
    graph,
    meter,
    meter_ink,
};

fn meter_ink(_: &ColorScheme) -> Color {
    LINE
}

pub fn graph(g: &Graph, _cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let ink = GraphInk {
        back: premul(BLACK, 255),
        grid: premul(GRID, 255),
        lines: [premul(LINE, 255), premul(SECOND, 255)],
        fills: [None, None],
        half: 0.5,
    };
    perfart::paint_graph(g, &ink, w, h, d)
}

/// Two columns of segments rising from the foot, over the label band.
pub fn meter(fraction: f64, _cs: &ColorScheme, w: i32, h: i32, d: i32) -> Canvas {
    let (dw, dh) = (w.max(1) * d, h.max(1) * d);
    let mut c = Canvas::new(dw as u32, dh as u32);
    let black = shape::pack(premul(BLACK, 255));
    c.pixels.fill(black);
    let pitch = SEGMENT + SEGMENT_GAP;
    let bottom = h - METER_LABEL_H;
    let slots = ((bottom - MARGIN) / pitch).max(0);
    let lit = perfart::lit_segments(fraction, slots);
    let col_w = ((w - 2 * MARGIN - COLUMN_GAP) / 2).max(1);
    for s in 0..slots {
        let ink = if s < lit { LINE } else { UNLIT };
        let px = shape::pack(premul(ink, 255));
        let y0 = bottom - (s + 1) * pitch;
        for col in 0..2 {
            let x0 = MARGIN + col * (col_w + COLUMN_GAP);
            for y in y0 * d..(y0 + SEGMENT) * d {
                for x in x0 * d..(x0 + col_w) * d {
                    c.put_raw(x, y, px);
                }
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325, |h, &p| {
            (h ^ u64::from(p)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    fn wave() -> Vec<f32> {
        (0..60)
            .map(|i| 0.5 + (i as f32 * 0.3).sin() * 0.4)
            .collect()
    }

    #[test]
    fn the_line_is_lit_where_the_newest_sample_stands() {
        let s = [0.0f32, 0.0, 1.0];
        let g = Graph {
            lines: &[&s],
            span: 3,
            ticks: 0,
        };
        let c = graph(&g, &ColorScheme::default(), 40, 30, 1);
        assert_eq!(
            c.get(39, 0),
            shape::pack(premul(LINE, 255)),
            "full at the right edge"
        );
        assert_eq!(
            c.get(5, 15),
            shape::pack(premul(BLACK, 255)),
            "black off the grid"
        );
        assert_eq!(
            c.get(5, 29),
            shape::pack(premul(LINE, 255)),
            "the empty start on the floor"
        );
    }

    #[test]
    fn the_meter_lights_its_share_from_the_foot() {
        let cs = ColorScheme::default();
        let c = meter(0.5, &cs, 40, 60, 1);
        let lit = shape::pack(premul(LINE, 255));
        let dark = shape::pack(premul(UNLIT, 255));
        let foot = 60 - METER_LABEL_H - 2;
        assert_eq!(c.get(MARGIN, foot), lit, "the lowest segment is lit");
        assert_eq!(c.get(MARGIN, MARGIN + 1), dark, "the top one is not");
    }

    #[test]
    fn the_graph_and_meter_are_pinned_at_1_and_2() {
        let s = wave();
        let swap: Vec<f32> = s.iter().map(|v| v * 0.3).collect();
        let cs = ColorScheme::default();
        let mut got = Vec::new();
        for d in [1, 2] {
            let g = Graph {
                lines: &[&s, &swap],
                span: 60,
                ticks: 7,
            };
            got.push((format!("graph @{d}"), fnv(&graph(&g, &cs, 200, 90, d))));
            for f in [0.0, 0.37, 1.0] {
                got.push((format!("meter {f} @{d}"), fnv(&meter(f, &cs, 60, 90, d))));
            }
        }
        crate::pins::check("classic/perf_graph", &got);
    }
}
