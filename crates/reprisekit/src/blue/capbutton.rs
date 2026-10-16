//! Reprise Blue's caption buttons, drawn at the output scale from the scheme's [`Face`]s: a 21x21 square with rounded corners and a 1 px rim, the face shaded by two gradients at once (`down` top to bottom, `across` left to right) and the glyph over it in the face's `mark`. A dark glyph on a light face sits on a white copy of itself one pixel down and left.
//! Glyphs are whole design pixels (minimize, maximize, restore) or two square-cut strokes (close), so at 1 they keep solid pixels and at 2 and up the cross's slopes and the box's corners are drawn again, smooth.

use crate::blue::scheme::{Face, Grad, Scheme};
use crate::raster::Canvas;
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, PartImage, State};
use crate::theme::Color;

pub const SIDE: i32 = 21;

/// The states a caption button draws, in atlas order: the order a
/// [`State`] falls back to [`State::Normal`].
pub const STATES: [State; 5] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Disabled,
    State::Inactive,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Minimize,
    Maximize,
    Restore,
    Close,
}

const EMBOSS: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const EMBOSS_OFFSET: (i32, i32) = (-1, 1);

/// The close cross: two strokes corner to corner through the box's
/// middle, cut square, wide enough that the pixels its middle runs
/// through stay solid at 1.
const CROSS: [((f64, f64), (f64, f64)); 2] =
    [((5.6, 5.6), (15.4, 15.4)), ((15.4, 5.6), (5.6, 15.4))];
const CROSS_WIDTH: f64 = 2.2;

/// The glyph's pieces as design-pixel rects `(x, y, w, h)` that never
/// overlap, so their coverages add up.
fn rects(g: Glyph) -> &'static [(i32, i32, i32, i32)] {
    match g {
        Glyph::Minimize => &[(5, 13, 7, 3)],
        Glyph::Maximize => &[(5, 5, 11, 3), (5, 8, 1, 8), (15, 8, 1, 8), (6, 15, 9, 1)],
        Glyph::Restore => &[
            (8, 5, 7, 2),
            (14, 7, 1, 5),
            (13, 11, 1, 1),
            (5, 8, 7, 2),
            (5, 10, 1, 6),
            (11, 10, 1, 6),
            (6, 15, 5, 1),
        ],
        Glyph::Close => &[],
    }
}

/// How much of the device pixel at (`x`, `y`) glyph `g` covers at `k`
/// device pixels per design pixel, moved by (`dx`, `dy`) design pixels.
fn glyph_cover(g: Glyph, x: f64, y: f64, k: f64, (dx, dy): (i32, i32)) -> f64 {
    let (x, y) = (x - f64::from(dx) * k, y - f64::from(dy) * k);
    if g == Glyph::Close {
        return CROSS
            .iter()
            .map(|&(a, b)| {
                shape::stroke(
                    x,
                    y,
                    &[(a.0 * k, a.1 * k), (b.0 * k, b.1 * k)],
                    CROSS_WIDTH * k,
                )
            })
            .fold(0.0, f64::max);
    }
    rects(g)
        .iter()
        .map(|&(rx, ry, w, h)| {
            let (w, h) = (f64::from(w) * k, f64::from(h) * k);
            shape::rounded(x - f64::from(rx) * k, y - f64::from(ry) * k, w, h, 0.0)
        })
        .sum::<f64>()
        .min(1.0)
}

/// How far the box's outer corners round, design pixels; the face
/// inside the rim rounds 1 px less.
const CORNER: f64 = 3.0;

/// How much of a disabled maximize glyph gives way to the face behind it.
const GREYED: f64 = 0.5;

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn mean(s: &[(f64, Px)]) -> [f64; 3] {
    let n = SIDE - 2;
    let sum = (1..=n).fold([0.0; 3], |acc, i| {
        let c = Stops(s).at((f64::from(i) + 0.5) / f64::from(SIDE)).1;
        std::array::from_fn(|j| acc[j] + c[j])
    });
    sum.map(|v| v / f64::from(n))
}

fn luma(c: [f64; 3]) -> f64 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

pub struct Look {
    rim: Px,
    down: Vec<(f64, Px)>,
    across: Vec<(f64, Px)>,
    base: [f64; 3],
    ink: Px,
    emboss: Option<Px>,
}

impl Look {
    fn new(f: &Face, ink: Color) -> Look {
        let (down, across) = (stops(f.down), stops(f.across));
        let (md, ma) = (mean(&down), mean(&across));
        let base = std::array::from_fn(|i| (md[i] + ma[i]) / 2.0);
        let ink = shape::premul(ink, 255);
        let emboss = (luma(ink.1) < luma(base)).then(|| shape::premul(EMBOSS, 255));
        Look {
            rim: shape::premul(f.rim, 255),
            down,
            across,
            base,
            ink,
            emboss,
        }
    }
}

/// Glyph `g`'s look in `s` for the `i`th of [`STATES`]. Minimize,
/// maximize and restore share one glass face, the minimize button's,
/// and one ink, whichever of their marks stands out most from it. A
/// disabled maximize sinks its ink halfway into the face. Close draws its
/// own red faces and mark.
fn look(s: &Scheme, g: Glyph, i: usize) -> Look {
    let min = faces(s, Glyph::Minimize)[i];
    if g == Glyph::Close {
        let f = faces(s, Glyph::Close)[i];
        let ink = f.mark.or(min.mark).unwrap_or(EMBOSS);
        return Look::new(f, ink);
    }
    let mut l = Look::new(min, EMBOSS);
    let ink = [Glyph::Minimize, Glyph::Maximize, Glyph::Restore]
        .into_iter()
        .filter_map(|g| faces(s, g)[i].mark)
        .max_by(|a, b| {
            let off = |c: Color| (luma(shape::premul(c, 255).1) - luma(l.base)).abs();
            off(*a).total_cmp(&off(*b))
        })
        .unwrap_or(EMBOSS);
    l = Look::new(min, ink);
    if g == Glyph::Maximize && STATES[i] == State::Disabled {
        let face = (255.0, l.base);
        l.ink = shape::mix(l.ink, face, GREYED);
        l.emboss = l.emboss.map(|e| shape::mix(e, face, GREYED));
    }
    l
}

pub fn frame(l: &Look, g: Glyph, d: i32) -> Canvas {
    let k = f64::from(d);
    let side = f64::from(SIDE) * k;
    let (lo, hi) = (
        1.5 / f64::from(SIDE),
        (f64::from(SIDE) - 1.5) / f64::from(SIDE),
    );
    let t = |v: f64| ((shape::centre(v as i32, d) + 0.5) / f64::from(SIDE)).clamp(lo, hi);
    shape::draw(SIDE, SIDE, d, |x, y| {
        let outer = shape::rounded(x, y, side, side, CORNER * k);
        if outer == 0.0 {
            return shape::CLEAR;
        }
        let inner = shape::rounded(
            x - k,
            y - k,
            side - 2.0 * k,
            side - 2.0 * k,
            (CORNER - 1.0) * k,
        );
        // Where the two gradients cross they add, less the face's mean, so
        // a corner takes both shades and the middle neither twice.
        let (dn, ac) = (Stops(&l.down).at(t(y)).1, Stops(&l.across).at(t(x)).1);
        let mut c: Px = (
            255.0,
            std::array::from_fn(|i| (dn[i] + ac[i] - l.base[i]).clamp(0.0, 255.0)),
        );
        if let Some(e) = l.emboss {
            c = shape::over(shape::scale(e, glyph_cover(g, x, y, k, EMBOSS_OFFSET)), c);
        }
        c = shape::over(shape::scale(l.ink, glyph_cover(g, x, y, k, (0, 0))), c);
        shape::shares(&[(l.rim, outer - inner), (c, inner)])
    })
}

fn faces(s: &Scheme, g: Glyph) -> [&Face; 5] {
    match g {
        Glyph::Minimize => {
            let b = &s.min_button;
            [&b.normal, &b.hot, &b.pressed, &b.disabled, &b.inactive]
        }
        Glyph::Maximize => {
            let b = &s.max_button;
            [&b.normal, &b.hot, &b.pressed, &b.disabled, &b.inactive]
        }
        Glyph::Restore => {
            let b = &s.restore_button;
            [&b.normal, &b.hot, &b.pressed, &b.disabled, &b.inactive]
        }
        Glyph::Close => {
            let b = &s.close_button;
            [&b.normal, &b.hot, &b.pressed, &b.disabled, &b.inactive]
        }
    }
}

pub fn caption_button(s: &Scheme, g: Glyph, d: i32) -> PartImage {
    let side = SIDE * d;
    let mut a = Canvas::new(side as u32, (side * STATES.len() as i32) as u32);
    for i in 0..STATES.len() {
        a.blit_over(0, i as i32 * side, &frame(&look(s, g, i), g, d));
    }
    let mut img = PartImage::new(a, &STATES, Layout::Vertical).at_scale(d);
    img.true_size = true;
    img
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;

    const GLYPHS: [Glyph; 4] = [
        Glyph::Minimize,
        Glyph::Maximize,
        Glyph::Restore,
        Glyph::Close,
    ];

    fn frame_pixels(img: &PartImage, state: State) -> Vec<u32> {
        let f = img.frame(state);
        (0..f.height)
            .flat_map(|y| (0..f.width).map(move |x| (x, y)))
            .map(|(x, y)| img.atlas.get(f.x + x, f.y + y))
            .collect()
    }

    fn fnv(img: &PartImage) -> u64 {
        let size = [img.atlas.width, img.atlas.height];
        size.iter()
            .chain(img.atlas.pixels.iter())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn each(mut f: impl FnMut(Variant, Glyph, i32, &PartImage)) {
        for v in Variant::ALL {
            for g in GLYPHS {
                for d in [1, 2] {
                    f(v, g, d, &caption_button(v.scheme(), g, d));
                }
            }
        }
    }

    #[test]
    fn every_frame_is_21_design_px_at_its_own_size() {
        each(|v, g, d, img| {
            assert!(img.true_size, "{v:?} {g:?} draws at its own size");
            assert_eq!(img.scale, d);
            assert_eq!(img.sizing, crate::raster::Insets::ZERO, "no nine-grid");
            let size = (img.atlas.width, img.atlas.height);
            assert_eq!(size, ((21 * d) as u32, (105 * d) as u32));
            for st in STATES {
                let f = img.frame(st);
                assert_eq!(
                    (f.width, f.height),
                    (21 * d, 21 * d),
                    "{v:?} {g:?} {st:?} at {d}"
                );
            }
        });
    }

    #[test]
    fn the_four_corners_are_clear_and_round_smoothly_at_2() {
        each(|v, g, d, img| {
            let e = 21 * d - 1;
            for st in STATES {
                let px = frame_pixels(img, st);
                let at = |x: i32, y: i32| px[(y * 21 * d + x) as usize];
                for (x, y) in [(0, 0), (e, 0), (0, e), (e, e)] {
                    let a = at(x, y) >> 24;
                    assert!(a < 16, "{v:?} {g:?} {st:?} ({x}, {y}) at {d}: alpha {a}");
                }
                if d == 2 {
                    let partial = (0..4)
                        .flat_map(|y| (0..4).map(move |x| (x, y)))
                        .filter(|&(x, y)| (1..255).contains(&(at(x, y) >> 24)))
                        .count();
                    assert!(partial >= 2, "{v:?} {g:?} {st:?} corner smooths at 2");
                }
            }
        });
    }

    #[test]
    fn the_curve_keeps_the_rim_colour() {
        let unmul = |p: u32| {
            let a = f64::from(p >> 24);
            [16, 8, 0].map(|s| (f64::from((p >> s) & 0xFF) * 255.0 / a).round())
        };
        each(|v, g, d, img| {
            for (i, st) in STATES.into_iter().enumerate() {
                let want = look(v.scheme(), g, i).rim.1;
                let px = frame_pixels(img, st);
                for y in 0..d {
                    for x in 0..2 * d {
                        let p = px[(y * 21 * d + x) as usize];
                        if (64..255).contains(&(p >> 24)) {
                            let got = unmul(p);
                            let off = (0..3).map(|c| (got[c] - want[c]).abs()).fold(0.0, f64::max);
                            assert!(off <= 4.0, "{v:?} {g:?} {st:?} ({x}, {y}) at {d}: {got:?}");
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn states_draw_apart_and_only_maximize_greys() {
        each(|v, g, d, img| {
            let f = |s| frame_pixels(img, s);
            assert_ne!(f(State::Normal), f(State::Hot), "{v:?} {g:?} hot at {d}");
            assert_ne!(
                f(State::Hot),
                f(State::Pressed),
                "{v:?} {g:?} pressed at {d}"
            );
            assert_ne!(
                f(State::Normal),
                f(State::Inactive),
                "{v:?} {g:?} inactive at {d}"
            );
            if g == Glyph::Maximize {
                assert_ne!(
                    f(State::Normal),
                    f(State::Disabled),
                    "{v:?} max greys at {d}"
                );
            } else {
                assert_eq!(f(State::Normal), f(State::Disabled), "{v:?} {g:?} at {d}");
            }
        });
    }

    #[test]
    fn glyphs_keep_solid_pixels_at_1() {
        for v in Variant::ALL {
            for (g, pts) in [
                (Glyph::Minimize, &[(8, 14)][..]),
                (Glyph::Maximize, &[(10, 6)][..]),
                (Glyph::Restore, &[(8, 9), (11, 5)][..]),
                (
                    Glyph::Close,
                    &[(10, 10), (7, 7), (13, 13), (13, 7), (7, 13)][..],
                ),
            ] {
                let img = caption_button(v.scheme(), g, 1);
                let mark = shape::pack(look(v.scheme(), g, 0).ink);
                let px = frame_pixels(&img, State::Normal);
                for &(x, y) in pts {
                    assert_eq!(px[(y * 21 + x) as usize], mark, "{v:?} {g:?} ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_cross_is_drawn_again_at_2() {
        for v in Variant::ALL {
            let one = frame_pixels(&caption_button(v.scheme(), Glyph::Close, 1), State::Normal);
            let two = frame_pixels(&caption_button(v.scheme(), Glyph::Close, 2), State::Normal);
            let doubled: Vec<u32> = (0..42)
                .flat_map(|y| (0..42).map(move |x| (x, y)))
                .map(|(x, y)| one[(y / 2 * 21 + x / 2) as usize])
                .collect();
            assert_ne!(two, doubled, "{v:?}");
        }
    }

    #[test]
    fn caption_buttons_pin_at_scale_1_and_2() {
        let mut got = Vec::new();
        each(|v, g, d, img| got.push((format!("{v:?} {g:?} {d}"), fnv(img))));
        crate::pins::check("blue/caption_buttons", &got);
    }
}
