//! Blue's taskbar band and notification area, drawn from the scheme's shading at the output scale. Both are vertical bands: a flat 1 design px line along the top and foot, the shading between shaded per device row so a band at scale 2 is smooth.

use crate::blue::scheme::{Face, Grad, Scheme};
use crate::raster::{Canvas, Insets};
use crate::shape::{Px, Stops, centre, draw, premul};
use crate::style::{Layout, PartImage, State};

/// Atlas width in design px: one stretched column would do, a few leave the stretch room.
const BAND_WIDTH: i32 = 4;

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

/// The shade of device row `y` of an `h`-row band at `d`, from the band's `down` stops (one per design row). Rows 0 and `h - 1` are the band's lines, flat over their `d` device rows; the rows between are shaded at the device row's middle, held clear of the lines.
fn band(down: &Stops, h: i32, y: i32, d: i32) -> Px {
    let row = y / d;
    let at = if row == 0 || row == h - 1 {
        f64::from(row)
    } else {
        centre(y, d).clamp(1.0, f64::from(h - 2))
    };
    down.at((at + 0.5) / f64::from(h))
}

/// The bar across the bottom at `d`: one frame, [`BAND_WIDTH`] columns of the band, stretched to the screen's width. A top bar draws it turned over.
pub fn taskbar(s: &Scheme, d: i32) -> PartImage {
    let h = s.metrics.taskbar_height;
    let down = stops(s.taskbar.normal.down);
    let down = Stops(&down);
    let c = draw(BAND_WIDTH, h, d, |_, y| band(&down, h, y as i32, d));
    PartImage::new(c, &[State::Normal], Layout::Vertical).at_scale(d)
}

/// One column of the tray's left edge: a line of its own colour, or a highlight following the band.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    Line(Px),
    /// As light as the fill or lighter: the band's shading lifted by these levels down the middle rows.
    Lift([f64; 3]),
}

/// The tray's left edge columns, outside in: `face.across` read at each of the `edge` columns, set against the fill column right of them.
fn edges(face: &Face, edge: i32) -> Vec<Edge> {
    let across = stops(face.across);
    let across = Stops(&across);
    let w = f64::from(edge + 1);
    let at = |col: i32| across.at((f64::from(col) + 0.5) / w);
    let (_, fill) = at(edge);
    (0..edge)
        .map(|col| {
            let px @ (_, c) = at(col);
            let lift = [c[0] - fill[0], c[1] - fill[1], c[2] - fill[2]];
            if lift.iter().sum::<f64>() < 0.0 {
                Edge::Line(px)
            } else {
                Edge::Lift(lift)
            }
        })
        .collect()
}

/// The notification area at `d`, one frame: its left edge (`Metrics::tray_edge` design px: a dark rim, then highlights following the band) stays put, and the column right of it stretches to the tray's width. The highlights leave the top and foot lines whole.
pub fn tray_area(s: &Scheme, d: i32) -> PartImage {
    let h = s.metrics.taskbar_height;
    let edge = s.metrics.tray_edge;
    let face = &s.tray_area.normal;
    let down = stops(face.down);
    let down = Stops(&down);
    let cols = edges(face, edge);
    let c: Canvas = draw(edge + 1, h, d, |x, y| {
        let (col, y) = (x as i32 / d, y as i32);
        let fill = band(&down, h, y, d);
        let row = y / d;
        match cols.get(col as usize) {
            Some(Edge::Line(px)) => *px,
            Some(Edge::Lift(k)) if row != 0 && row != h - 1 => {
                let (a, c) = fill;
                (a, [0, 1, 2].map(|i| (c[i] + k[i]).clamp(0.0, a)))
            }
            _ => fill,
        }
    });
    PartImage::new(c, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(edge, 0, 0, 0))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;
    use crate::theme::Color;

    /// The tray's fill at design row `row`, scale 1: what the clean column right of the edge shows.
    fn tray_fill(s: &Scheme, row: i32) -> Color {
        let v = tray_area(s, 1).atlas.get(s.metrics.tray_edge, row);
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    fn fnv(img: &PartImage) -> u64 {
        let a = &img.atlas;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let words = [a.width, a.height]
            .into_iter()
            .chain(a.pixels.iter().copied());
        for w in words {
            for b in w.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    fn lum(v: u32) -> u32 {
        ((v >> 16) & 0xFF) + ((v >> 8) & 0xFF) + (v & 0xFF)
    }

    #[test]
    fn taskbar_and_tray_are_their_design_size_times_the_scale() {
        for v in Variant::ALL {
            let s = v.scheme();
            let h = s.metrics.taskbar_height;
            let edge = s.metrics.tray_edge;
            for d in [1, 2] {
                let bar = taskbar(s, d);
                let tray = tray_area(s, d);
                assert_eq!(bar.scale, d, "{v:?} bar painted at {d}");
                assert_eq!(tray.scale, d, "{v:?} tray painted at {d}");
                assert_eq!(
                    (bar.atlas.width, bar.atlas.height),
                    ((BAND_WIDTH * d) as u32, (h * d) as u32),
                    "{v:?} bar at {d}"
                );
                assert_eq!(
                    (tray.atlas.width, tray.atlas.height),
                    (((edge + 1) * d) as u32, (h * d) as u32),
                    "{v:?} tray at {d}"
                );
                assert_eq!(
                    bar.sizing,
                    Insets::new(0, 0, 0, 0),
                    "{v:?} bar stretches whole"
                );
                assert_eq!(
                    tray.sizing,
                    Insets::new(3, 0, 0, 0),
                    "{v:?} tray keeps a 3 px edge"
                );
                for img in [&bar, &tray] {
                    assert!(
                        img.atlas.pixels.iter().all(|&p| p >> 24 == 0xFF),
                        "{v:?} at {d}: every pixel opaque"
                    );
                }
            }
        }
    }

    #[test]
    fn the_lines_stay_one_design_px_and_flat_at_2() {
        for v in Variant::ALL {
            let bar = taskbar(v.scheme(), 2);
            let a = &bar.atlas;
            let last = a.height as i32 - 1;
            assert_eq!(a.get(0, 0), a.get(0, 1), "{v:?} top line is 2 rows");
            assert_eq!(
                a.get(0, last),
                a.get(0, last - 1),
                "{v:?} foot line is 2 rows"
            );
            for y in 0..a.height as i32 {
                for x in 1..a.width as i32 {
                    assert_eq!(a.get(x, y), a.get(0, y), "{v:?} row {y} is level");
                }
            }
        }
    }

    #[test]
    fn the_band_at_2_is_shaded_again_not_doubled() {
        for v in Variant::ALL {
            let s = v.scheme();
            for img in [taskbar(s, 2), tray_area(s, 2)] {
                let a = &img.atlas;
                let x = a.width as i32 - 1;
                let h = a.height as i32 / 2;
                assert!(
                    (1..h - 1).any(|r| a.get(x, 2 * r) != a.get(x, 2 * r + 1)),
                    "{v:?}: a design row splits into two shades"
                );
            }
        }
    }

    #[test]
    fn the_tray_edge_is_a_dark_rim_then_light_highlights() {
        let s = Variant::Blue.scheme();
        let tray = tray_area(s, 1);
        for y in 1..29 {
            let fill = tray.atlas.get(3, y);
            assert!(lum(tray.atlas.get(0, y)) < lum(fill), "navy rim row {y}");
            for x in [1, 2] {
                assert!(
                    lum(tray.atlas.get(x, y)) > lum(fill),
                    "navy highlight ({x}, {y})"
                );
            }
        }
        for v in [
            Variant::Slate,
            Variant::Obsidian,
            Variant::Ember,
            Variant::Carbon,
        ] {
            let tray = tray_area(v.scheme(), 1);
            for y in 0..30 {
                assert_eq!(
                    tray.atlas.get(0, y),
                    0xFF00_0000,
                    "{v:?} rim row {y} is black"
                );
            }
        }
    }

    #[test]
    fn each_scheme_keeps_its_bar_and_tray_tone() {
        let tone = |v: Variant| {
            let c = tray_fill(v.scheme(), 15);
            u32::from(c.r) + u32::from(c.g) + u32::from(c.b)
        };
        for v in [
            Variant::Olive,
            Variant::Silver,
            Variant::Sapphire,
            Variant::Cobalt,
        ] {
            assert!(tone(v) > 3 * 0xB0, "{v:?} tray is light");
        }
        for v in [
            Variant::Slate,
            Variant::Obsidian,
            Variant::Ember,
            Variant::Carbon,
        ] {
            assert!(tone(v) < 3 * 0x50, "{v:?} tray is dark");
        }
    }

    #[test]
    fn taskbar_pins_at_scale_1_and_2() {
        let got: Vec<_> = Variant::ALL
            .into_iter()
            .flat_map(|v| [1, 2].map(|d| (format!("{v:?} {d}"), fnv(&taskbar(v.scheme(), d)))))
            .collect();
        crate::pins::check("blue/taskbar", &got);
    }

    #[test]
    fn tray_area_pins_at_scale_1_and_2() {
        let got: Vec<_> = Variant::ALL
            .into_iter()
            .flat_map(|v| [1, 2].map(|d| (format!("{v:?} {d}"), fnv(&tray_area(v.scheme(), d)))))
            .collect();
        crate::pins::check("blue/tray_area", &got);
    }
}
