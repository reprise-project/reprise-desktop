//! The colour box's colours and the tool box's glyphs. A glyph is a coverage mask on a 16x16
//! design grid (option glyphs on their own grids), drawn from geometry at the output scale.

use crate::doc::{FillStyle, Tool};
use crate::shapes::Tip;
use reprisekit::raster::Canvas;
use reprisekit::shape;
use reprisekit::theme::Color;

/// The colour box, top row then bottom row.
pub const PALETTE: [(Color, &str); 28] = [
    (Color::rgb(0, 0, 0), "Black"),
    (Color::rgb(128, 128, 128), "Grey"),
    (Color::rgb(128, 0, 0), "Dark Red"),
    (Color::rgb(128, 128, 0), "Olive"),
    (Color::rgb(0, 128, 0), "Green"),
    (Color::rgb(0, 128, 128), "Teal"),
    (Color::rgb(0, 0, 128), "Navy"),
    (Color::rgb(128, 0, 128), "Purple"),
    (Color::rgb(128, 128, 64), "Khaki"),
    (Color::rgb(0, 64, 64), "Dark Teal"),
    (Color::rgb(0, 128, 255), "Sky Blue"),
    (Color::rgb(0, 64, 128), "Dark Blue"),
    (Color::rgb(128, 0, 255), "Violet"),
    (Color::rgb(128, 64, 0), "Brown"),
    (Color::rgb(255, 255, 255), "White"),
    (Color::rgb(192, 192, 192), "Silver"),
    (Color::rgb(255, 0, 0), "Red"),
    (Color::rgb(255, 255, 0), "Yellow"),
    (Color::rgb(0, 255, 0), "Lime"),
    (Color::rgb(0, 255, 255), "Cyan"),
    (Color::rgb(0, 0, 255), "Blue"),
    (Color::rgb(255, 0, 255), "Magenta"),
    (Color::rgb(255, 255, 128), "Pale Yellow"),
    (Color::rgb(0, 255, 128), "Spring Green"),
    (Color::rgb(128, 255, 255), "Pale Cyan"),
    (Color::rgb(128, 128, 255), "Periwinkle"),
    (Color::rgb(255, 0, 128), "Rose"),
    (Color::rgb(255, 128, 64), "Orange"),
];

pub fn color_of(px: u32) -> Color {
    Color::rgb((px >> 16) as u8, (px >> 8) as u8, px as u8)
}

/// One stroke of a glyph, in design pixels on the glyph's grid.
enum Mark {
    Line(Vec<(f64, f64)>, f64),
    Fill(Vec<(f64, f64)>),
    Oval(f64, f64, f64, f64),
    Ring(f64, f64, f64, f64, f64),
    Box(f64, f64, f64, f64, f64),
    Dots(Vec<(f64, f64)>, f64),
}

/// The marks drawn white over a `w`x`h` grid at `d` device px per design px, as a mask canvas at
/// scale `d`.
fn mask(w: i32, h: i32, d: i32, marks: &[Mark]) -> Canvas {
    let k = f64::from(d);
    let sc = |p: &[(f64, f64)]| p.iter().map(|&(x, y)| (x * k, y * k)).collect::<Vec<_>>();
    let marks: Vec<Mark> = marks
        .iter()
        .map(|m| match m {
            Mark::Line(p, wd) => Mark::Line(sc(p), wd * k),
            Mark::Fill(p) => Mark::Fill(sc(p)),
            Mark::Oval(cx, cy, rx, ry) => Mark::Oval(cx * k, cy * k, rx * k, ry * k),
            Mark::Ring(cx, cy, rx, ry, t) => Mark::Ring(cx * k, cy * k, rx * k, ry * k, t * k),
            Mark::Box(x, y, w, h, a) => Mark::Box(x * k, y * k, w * k, h * k, *a),
            Mark::Dots(p, r) => Mark::Dots(sc(p), r * k),
        })
        .collect();
    let c = shape::draw(w, h, d, |x, y| {
        let cover = marks.iter().fold(0.0f64, |acc, m| {
            let c = match m {
                Mark::Line(p, wd) => shape::stroke(x, y, p, *wd),
                Mark::Fill(p) => shape::polygon(x, y, p),
                Mark::Oval(cx, cy, rx, ry) => shape::ellipse(x, y, *cx, *cy, *rx, *ry),
                Mark::Ring(cx, cy, rx, ry, t) => {
                    let o = shape::ellipse(x, y, *cx, *cy, *rx, *ry);
                    let i = shape::ellipse(x, y, *cx, *cy, rx - t, ry - t);
                    (o - i).max(0.0)
                }
                Mark::Box(bx, by, bw, bh, a) => {
                    let inside = (x + 0.5 >= *bx && x + 0.5 < bx + bw)
                        && (y + 0.5 >= *by && y + 0.5 < by + bh);
                    if inside { *a } else { 0.0 }
                }
                Mark::Dots(p, r) => p
                    .iter()
                    .map(|&(cx, cy)| shape::ellipse(x, y, cx, cy, *r, *r))
                    .fold(0.0, f64::max),
            };
            acc.max(c)
        });
        let a = cover * 255.0;
        (a, [a; 3])
    });
    c.at_scale(d)
}

fn dashed(pts: &[(f64, f64)], dash: f64) -> Vec<Mark> {
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let n = (len / dash).floor().max(1.0) as usize;
        for i in (0..n).step_by(2) {
            let t0 = i as f64 / n as f64;
            let t1 = ((i + 1) as f64 / n as f64).min(1.0);
            let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            out.push(Mark::Line(vec![at(t0), at(t1)], 1.0));
        }
    }
    out
}

/// The tool's glyph at `d` device px per design px.
pub fn tool_glyph(tool: Tool, d: i32) -> Canvas {
    let marks = match tool {
        Tool::FreeSelect => dashed(
            &[
                (3.0, 2.5),
                (12.5, 4.0),
                (10.0, 8.0),
                (13.5, 13.5),
                (2.5, 12.0),
                (5.0, 7.0),
                (3.0, 2.5),
            ],
            1.5,
        ),
        Tool::Select => dashed(
            &[
                (2.5, 2.5),
                (13.5, 2.5),
                (13.5, 13.5),
                (2.5, 13.5),
                (2.5, 2.5),
            ],
            1.5,
        ),
        Tool::Eraser => vec![
            Mark::Fill(vec![(2.0, 11.0), (8.0, 5.0), (14.0, 5.0), (8.0, 11.0)]),
            Mark::Line(
                vec![
                    (2.0, 11.0),
                    (2.0, 13.0),
                    (8.0, 13.0),
                    (14.0, 7.0),
                    (14.0, 5.0),
                ],
                1.0,
            ),
            Mark::Line(vec![(8.0, 11.0), (8.0, 13.0)], 1.0),
        ],
        Tool::Fill => vec![
            Mark::Fill(vec![(3.0, 8.0), (8.0, 3.0), (13.0, 8.0), (8.0, 13.0)]),
            Mark::Line(vec![(8.0, 3.0), (6.0, 1.0)], 1.0),
            Mark::Oval(2.5, 12.0, 1.5, 2.5),
        ],
        Tool::Picker => vec![
            Mark::Line(vec![(2.5, 13.5), (9.0, 7.0)], 2.0),
            Mark::Oval(11.5, 4.5, 3.0, 3.0),
            Mark::Line(vec![(7.0, 5.0), (11.0, 9.0)], 1.5),
        ],
        Tool::Magnifier => vec![
            Mark::Ring(6.5, 6.5, 5.0, 5.0, 1.5),
            Mark::Line(vec![(10.0, 10.0), (14.5, 14.5)], 2.5),
        ],
        Tool::Pencil => vec![Mark::Fill(vec![
            (10.5, 1.5),
            (14.5, 5.5),
            (5.5, 14.5),
            (1.5, 14.5),
            (1.5, 10.5),
        ])],
        Tool::Brush => vec![
            Mark::Line(vec![(14.5, 1.5), (7.5, 8.5)], 2.0),
            Mark::Fill(vec![
                (6.0, 7.0),
                (9.0, 10.0),
                (5.0, 14.5),
                (1.5, 14.5),
                (2.0, 11.0),
            ]),
        ],
        Tool::Airbrush => vec![
            Mark::Fill(vec![(9.0, 6.0), (13.0, 6.0), (13.0, 15.0), (9.0, 15.0)]),
            Mark::Line(vec![(11.0, 6.0), (11.0, 3.5), (8.5, 3.5)], 1.0),
            Mark::Dots(
                vec![(5.5, 2.5), (3.5, 4.5), (6.0, 5.5), (2.5, 7.0), (4.5, 8.0)],
                0.75,
            ),
        ],
        Tool::Text => vec![
            Mark::Line(vec![(2.5, 14.0), (8.0, 2.0), (13.5, 14.0)], 2.0),
            Mark::Line(vec![(5.0, 10.0), (11.0, 10.0)], 1.5),
        ],
        Tool::Line => vec![Mark::Line(vec![(2.0, 14.0), (14.0, 2.0)], 1.5)],
        Tool::Curve => {
            vec![Mark::Line(
                (0..=16)
                    .map(|i| {
                        let t = i as f64 / 16.0;
                        let u = 1.0 - t;
                        let x = u * u * u * 2.0
                            + 3.0 * u * u * t * 2.0
                            + 3.0 * u * t * t * 14.0
                            + t * t * t * 14.0;
                        let y = u * u * u * 13.0
                            + 3.0 * u * u * t * 0.0
                            + 3.0 * u * t * t * 16.0
                            + t * t * t * 3.0;
                        (x, y)
                    })
                    .collect(),
                1.5,
            )]
        }
        Tool::Rect => vec![Mark::Line(
            vec![
                (2.0, 3.5),
                (14.0, 3.5),
                (14.0, 12.5),
                (2.0, 12.5),
                (2.0, 3.5),
            ],
            1.5,
        )],
        Tool::Polygon => vec![Mark::Line(
            vec![
                (3.0, 2.0),
                (13.0, 5.0),
                (9.0, 8.5),
                (14.0, 14.0),
                (2.0, 13.0),
                (3.0, 2.0),
            ],
            1.5,
        )],
        Tool::Ellipse => vec![Mark::Ring(8.0, 8.0, 6.5, 5.0, 1.5)],
        Tool::RoundRect => {
            let r = 3.0;
            let arc = |cx: f64, cy: f64, a0: f64| {
                (0..=4).map(move |i| {
                    let a = (a0 + 90.0 * f64::from(i) / 4.0).to_radians();
                    (cx + r * a.cos(), cy + r * a.sin())
                })
            };
            let mut pts: Vec<(f64, f64)> = arc(11.0, 6.5, 270.0)
                .chain(arc(11.0, 9.5, 0.0))
                .chain(arc(5.0, 9.5, 90.0))
                .chain(arc(5.0, 6.5, 180.0))
                .collect();
            pts.push(pts[0]);
            vec![Mark::Line(pts, 1.5)]
        }
    };
    mask(16, 16, d, &marks)
}

/// The options well's grid for a line width: a bar `w` px thick across a 32x8 cell.
pub fn width_glyph(w: i32, d: i32) -> Canvas {
    let t = f64::from(w);
    mask(
        32,
        8,
        d,
        &[Mark::Box(6.0, 4.0 - (t / 2.0).floor(), 20.0, t, 1.0)],
    )
}

/// A brush tip in a 12x12 cell.
pub fn tip_glyph(tip: Tip, size: i32, d: i32) -> Canvas {
    let n = f64::from(size);
    let c = 6.0;
    let marks = match tip {
        Tip::Round => vec![Mark::Oval(c, c, n / 2.0, n / 2.0)],
        Tip::Square => vec![Mark::Box(
            c - (n / 2.0).floor(),
            c - (n / 2.0).floor(),
            n,
            n,
            1.0,
        )],
        Tip::Slash => vec![Mark::Line(
            vec![(c - n / 2.0, c + n / 2.0), (c + n / 2.0, c - n / 2.0)],
            1.0,
        )],
        Tip::Backslash => {
            vec![Mark::Line(
                vec![(c - n / 2.0, c - n / 2.0), (c + n / 2.0, c + n / 2.0)],
                1.0,
            )]
        }
    };
    mask(12, 12, d, &marks)
}

/// An eraser size: a square `n` px wide in a 16x14 cell.
pub fn eraser_glyph(n: i32, d: i32) -> Canvas {
    let n = f64::from(n);
    mask(
        16,
        14,
        d,
        &[Mark::Box(8.0 - n / 2.0, 7.0 - n / 2.0, n, n, 1.0)],
    )
}

/// A spray size: a scatter of dots in a 20x20 cell, in a disc three quarters of radius `r`.
pub fn spray_glyph(r: i32, d: i32) -> Canvas {
    let mut s = crate::shapes::Spray(0x9E37_79B9);
    let dots = s
        .burst((10, 10), r * 3 / 4, (r * 2) as usize)
        .into_iter()
        .map(|(x, y)| (f64::from(x) + 0.5, f64::from(y) + 0.5))
        .collect();
    mask(20, 20, d, &[Mark::Dots(dots, 0.6)])
}

/// A fill style in a 32x14 cell: the outline alone, outline over a half-tone fill, the fill.
pub fn fill_glyph(f: FillStyle, d: i32) -> Canvas {
    let outline = Mark::Line(
        vec![
            (5.5, 2.5),
            (26.5, 2.5),
            (26.5, 11.5),
            (5.5, 11.5),
            (5.5, 2.5),
        ],
        1.0,
    );
    let marks = match f {
        FillStyle::Outline => vec![outline],
        FillStyle::Both => vec![outline, Mark::Box(6.0, 3.0, 20.0, 8.0, 0.5)],
        FillStyle::Fill => vec![Mark::Box(5.0, 2.0, 22.0, 10.0, 0.5)],
    };
    mask(32, 14, d, &marks)
}

/// Selection drawing in a 32x24 cell: the box covering what is under it, or letting it show.
pub fn opaque_glyph(opaque: bool, d: i32) -> Canvas {
    let mut marks = vec![
        Mark::Oval(13.0, 14.0, 5.0, 5.0),
        Mark::Fill(vec![(16.0, 4.0), (24.0, 18.0), (8.0, 18.0)]),
    ];
    marks.extend(dashed(
        &[
            (4.5, 2.5),
            (27.5, 2.5),
            (27.5, 21.5),
            (4.5, 21.5),
            (4.5, 2.5),
        ],
        1.5,
    ));
    if opaque {
        marks.push(Mark::Box(5.0, 3.0, 22.0, 18.0, 0.35));
    }
    mask(32, 24, d, &marks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::TOOLS;

    fn inked(c: &Canvas) -> usize {
        c.pixels.iter().filter(|&&p| p >> 24 > 0x80).count()
    }

    #[test]
    fn every_glyph_has_ink_and_twice_the_pixels_at_scale_two() {
        for t in TOOLS {
            let (one, two) = (tool_glyph(t, 1), tool_glyph(t, 2));
            assert_eq!((one.width, one.height, two.scale()), (16, 16, 2));
            assert!(inked(&one) >= 6, "{t:?} has ink");
            let ratio = inked(&two) as f64 / inked(&one) as f64;
            assert!(
                (2.5..6.0).contains(&ratio),
                "{t:?} at scale 2 is drawn again, {ratio}"
            );
        }
        assert!(inked(&width_glyph(5, 1)) > inked(&width_glyph(1, 1)));
        assert!(inked(&eraser_glyph(10, 1)) > inked(&eraser_glyph(4, 1)));
        assert_ne!(opaque_glyph(true, 1), opaque_glyph(false, 1));
    }

    #[test]
    fn the_palette_names_each_color_once() {
        let mut names: Vec<_> = PALETTE.iter().map(|(_, n)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 28);
        let mut colors: Vec<_> = PALETTE.iter().map(|(c, _)| c.argb()).collect();
        colors.sort_unstable();
        colors.dedup();
        assert_eq!(colors.len(), 28);
    }
}
