//! Glyphs drawn from geometry at any scale: arrows, chevrons, the tick,
//! the cross, the bullet, the plus, minus and dash bars and the speaker
//! (sounding or crossed out), each in a `size` x `size` design-pixel box.

use super::cover::{ellipse, polygon, segments};
use super::{Px, scale};
use crate::raster::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ChevronUp,
    ChevronDown,
    Tick,
    Cross,
    Bullet,
    Plus,
    Minus,
    Dash,
    /// A speaker facing right with two sound waves.
    Speaker,
    /// The speaker with a cross where the waves were.
    SpeakerMuted,
}

type Pt = (f64, f64);

struct Shape {
    lines: Vec<Vec<Pt>>,
    solid: Option<Solid>,
}

enum Solid {
    Polygon(Vec<Pt>),
    Disc(Pt, f64),
}

fn shape(g: Glyph, size: i32, weight: f64) -> Shape {
    let s = f64::from(size);
    let odd = (weight.round().max(1.0) as i64) % 2 == 1;
    let mid = if odd {
        f64::from((size - 1) / 2) + 0.5
    } else {
        f64::from(size / 2)
    };
    let lines = |lines: Vec<Vec<Pt>>| Shape { lines, solid: None };
    match g {
        Glyph::Minus => lines(vec![vec![(0.0, mid), (s, mid)]]),
        Glyph::Plus => lines(vec![vec![(0.0, mid), (s, mid)], vec![(mid, 0.0), (mid, s)]]),
        Glyph::Dash => {
            let q = f64::from(size / 4);
            lines(vec![vec![(q, mid), (s - q, mid)]])
        }
        Glyph::Cross => lines(vec![vec![(0.0, 0.0), (s, s)], vec![(s, 0.0), (0.0, s)]]),
        Glyph::ChevronUp | Glyph::ChevronDown => {
            let (cx, h) = (s / 2.0, s / 2.0);
            // The point the arms meet sits a whole number of pixels from
            // the middle column's line, so both arms run through pixel
            // middles; the arms are centred down the box.
            let cy = cx + ((s - h) / 2.0 - cx).round();
            let (tip, ends) = (cy, cy + h);
            let arm = if g == Glyph::ChevronUp {
                vec![(0.0, ends), (cx, tip), (s, ends)]
            } else {
                vec![(0.0, s - ends), (cx, s - tip), (s, s - ends)]
            };
            lines(vec![arm])
        }
        Glyph::Tick => {
            let vx = (s / 3.0).round();
            let vy = s - vx;
            let dy = ((s - vy) / 2.0).floor();
            lines(vec![vec![(0.0, vy - vx + dy), (vx, vy + dy), (s, dy)]])
        }
        Glyph::Speaker | Glyph::SpeakerMuted => {
            // Drawn on a 16 px grid: a 3x6 box and a cone flaring to 14
            // rows, then the waves or the cross in the right third.
            let k = s / 16.0;
            let at = |pts: &[Pt]| pts.iter().map(|&(x, y)| (x * k, y * k)).collect();
            let lines = if g == Glyph::Speaker {
                vec![
                    at(&[(10.5, 5.5), (11.5, 6.5), (11.5, 9.5), (10.5, 10.5)]),
                    at(&[(12.5, 2.5), (14.5, 4.5), (14.5, 11.5), (12.5, 13.5)]),
                ]
            } else {
                vec![
                    at(&[(10.0, 5.0), (16.0, 11.0)]),
                    at(&[(16.0, 5.0), (10.0, 11.0)]),
                ]
            };
            Shape {
                lines,
                solid: Some(Solid::Polygon(at(&[
                    (1.0, 5.0),
                    (4.0, 5.0),
                    (8.0, 1.0),
                    (8.0, 15.0),
                    (4.0, 11.0),
                    (1.0, 11.0),
                ]))),
            }
        }
        Glyph::Bullet => Shape {
            lines: Vec::new(),
            solid: Some(Solid::Disc((s / 2.0, s / 2.0), s / 4.0)),
        },
        Glyph::ArrowUp | Glyph::ArrowDown | Glyph::ArrowLeft | Glyph::ArrowRight => {
            let n = f64::from(size / 2);
            let c = f64::from((size - 1) / 2);
            let top = f64::from((size - size / 2) / 2);
            let up = [
                (c, top),
                (c + 1.0, top),
                (c + 1.0 + n, top + n),
                (c - n, top + n),
            ];
            let turn = |(x, y): Pt| match g {
                Glyph::ArrowUp => (x, y),
                Glyph::ArrowDown => (x, s - y),
                Glyph::ArrowLeft => (y, x),
                _ => (s - y, x),
            };
            Shape {
                lines: Vec::new(),
                solid: Some(Solid::Polygon(up.into_iter().map(turn).collect())),
            }
        }
    }
}

/// `weight` (1.0 or 2.0 for lines whole at scale 1) is ignored by the arrows and the bullet.
pub fn glyph(g: Glyph, size: i32, d: i32, weight: f64, ink: Px) -> Canvas {
    let k = f64::from(d.max(1));
    let sh = shape(g, size, weight);
    let at = |p: Pt| (p.0 * k, p.1 * k);
    let mut segs = Vec::new();
    let mut joins = Vec::new();
    for line in &sh.lines {
        let arms: Vec<_> = line
            .windows(2)
            .map(|w| {
                let (a, b) = (at(w[0]), at(w[1]));
                let (dx, dy) = ((b.0 - a.0).abs(), (b.1 - a.1).abs());
                let width = weight * k * dx.hypot(dy) / dx.max(dy).max(f64::MIN_POSITIVE);
                (a, b, width)
            })
            .collect();
        for w in arms.windows(2) {
            joins.push((w[0].1, w[0].2.max(w[1].2) / 2.0));
        }
        segs.extend(arms);
    }
    let solid = sh.solid.map(|s| match s {
        Solid::Polygon(pts) => Solid::Polygon(pts.into_iter().map(at).collect()),
        Solid::Disc(c, r) => Solid::Disc(at(c), r * k),
    });
    super::draw(size, size, d, |x, y| {
        let lines = segments(x, y, &segs, &joins);
        let solid = match &solid {
            Some(Solid::Polygon(pts)) => polygon(x, y, pts),
            Some(Solid::Disc((cx, cy), r)) => ellipse(x, y, *cx, *cy, *r, *r),
            None => 0.0,
        };
        scale(ink, lines.max(solid))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLACK: Px = (255.0, [0.0; 3]);

    const ALL: [Glyph; 12] = [
        Glyph::ArrowUp,
        Glyph::ArrowDown,
        Glyph::ArrowLeft,
        Glyph::ArrowRight,
        Glyph::ChevronUp,
        Glyph::ChevronDown,
        Glyph::Tick,
        Glyph::Cross,
        Glyph::Bullet,
        Glyph::Plus,
        Glyph::Minus,
        Glyph::Dash,
    ];

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(x, y) >> 24
    }

    fn along(a: Pt, b: Pt, d: i32) -> Vec<(i32, i32)> {
        let k = f64::from(d);
        (1..16)
            .map(|i| {
                let t = f64::from(i) / 16.0;
                let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                ((x * k).floor() as i32, (y * k).floor() as i32)
            })
            .collect()
    }

    #[test]
    fn every_stroke_of_every_glyph_has_a_solid_pixel_at_scale_1() {
        for g in ALL {
            for (size, weight) in [(9, 1.0), (13, 1.0), (10, 2.0)] {
                let c = glyph(g, size, 1, weight, BLACK);
                let sh = shape(g, size, weight);
                for line in &sh.lines {
                    for w in line.windows(2) {
                        assert!(
                            along(w[0], w[1], 1)
                                .iter()
                                .any(|&(x, y)| alpha(&c, x, y) == 255),
                            "{g:?} {size} weight {weight}: segment {:?} has a solid pixel",
                            (w[0], w[1])
                        );
                    }
                }
                assert!(
                    c.pixels.iter().any(|p| p >> 24 == 255),
                    "{g:?} {size}: solid somewhere"
                );
            }
        }
    }

    #[test]
    fn a_tick_of_weight_1_has_a_solid_pixel_on_each_arm() {
        let c = glyph(Glyph::Tick, 9, 1, 1.0, BLACK);
        assert_eq!(alpha(&c, 1, 5), 255);
        assert_eq!(alpha(&c, 6, 3), 255);
    }

    #[test]
    fn a_minus_of_weight_1_is_one_whole_row_at_1_and_two_at_2() {
        let one = glyph(Glyph::Minus, 9, 1, 1.0, BLACK);
        let rows: Vec<u32> = (0..9).map(|y| alpha(&one, 4, y)).collect();
        assert_eq!(rows, [0, 0, 0, 0, 255, 0, 0, 0, 0]);
        let two = glyph(Glyph::Minus, 9, 2, 1.0, BLACK);
        let rows: Vec<u32> = (0..18).map(|y| alpha(&two, 8, y)).collect();
        assert_eq!(rows.iter().filter(|&&a| a == 255).count(), 2);
        assert_eq!(rows.iter().filter(|&&a| a != 0 && a != 255).count(), 0);
    }

    #[test]
    fn diagonals_are_smooth_at_2() {
        for g in [
            Glyph::Tick,
            Glyph::Cross,
            Glyph::ChevronDown,
            Glyph::ArrowUp,
            Glyph::Bullet,
        ] {
            let c = glyph(g, 13, 2, 1.0, BLACK);
            assert_eq!((c.width, c.height), (26, 26));
            assert!(
                c.pixels.iter().any(|p| (1..255).contains(&(p >> 24))),
                "{g:?} has edge pixels between clear and solid"
            );
        }
    }

    #[test]
    fn an_arrow_covers_rows_1_3_5_7_whole_at_scale_1() {
        let c = glyph(Glyph::ArrowUp, 9, 1, 1.0, BLACK);
        let whole: Vec<usize> = (0..9)
            .map(|y| (0..9).filter(|&x| alpha(&c, x, y) == 255).count())
            .collect();
        assert_eq!(whole, [0, 0, 1, 3, 5, 7, 0, 0, 0]);
        let down = glyph(Glyph::ArrowDown, 9, 1, 1.0, BLACK);
        let right = glyph(Glyph::ArrowRight, 9, 1, 1.0, BLACK);
        assert_eq!(alpha(&down, 4, 6), 255, "the down arrow's tip row");
        assert_eq!(alpha(&right, 6, 4), 255, "the right arrow's tip column");
    }

    #[test]
    fn the_speaker_has_a_solid_box_and_the_muted_one_a_cross_for_waves() {
        let on = glyph(Glyph::Speaker, 16, 1, 1.0, BLACK);
        let off = glyph(Glyph::SpeakerMuted, 16, 1, 1.0, BLACK);
        for c in [&on, &off] {
            let body: Vec<u32> = (1..4).map(|x| alpha(c, x, 8)).collect();
            assert_eq!(body, [255, 255, 255], "the box");
            assert_eq!(alpha(c, 7, 2), 255, "the cone's flare");
            assert_eq!(
                alpha(c, 9, 8),
                0,
                "clear between the cone and the right third"
            );
        }
        assert_eq!(alpha(&on, 11, 8), 255, "the inner wave");
        assert_eq!(alpha(&on, 14, 8), 255, "the outer wave");
        assert_eq!(alpha(&off, 13, 8), 255, "the cross's middle");
        let cols = |c: &Canvas, xs: std::ops::Range<i32>| {
            xs.flat_map(|x| (0..16).map(move |y| (x, y)))
                .map(|(x, y)| alpha(c, x, y))
                .collect::<Vec<_>>()
        };
        assert_eq!(cols(&on, 0..9), cols(&off, 0..9), "the same speaker");
        assert_ne!(cols(&on, 9..16), cols(&off, 9..16), "waves or a cross");
        for (g, c) in [(Glyph::Speaker, &on), (Glyph::SpeakerMuted, &off)] {
            for line in &shape(g, 16, 1.0).lines {
                for w in line.windows(2) {
                    assert!(
                        along(w[0], w[1], 1)
                            .iter()
                            .any(|&(x, y)| alpha(c, x, y) == 255),
                        "{g:?}: segment {:?} has a solid pixel",
                        (w[0], w[1])
                    );
                }
            }
        }
        let two = glyph(Glyph::Speaker, 16, 2, 1.0, BLACK);
        assert!(
            two.pixels.iter().any(|p| (1..255).contains(&(p >> 24))),
            "the cone's flare is smooth at 2"
        );
    }

    #[test]
    fn glyphs_take_their_ink() {
        let red = (255.0, [255.0, 0.0, 0.0]);
        let c = glyph(Glyph::Plus, 9, 1, 1.0, red);
        assert_eq!(c.get(4, 4), 0xFFFF_0000);
        assert_eq!(c.get(0, 0), 0);
    }
}
