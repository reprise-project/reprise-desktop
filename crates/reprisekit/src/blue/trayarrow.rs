//! The Blue tray arrow's round button, drawn per colour scheme at any scale:
//! a 19 px disc with a rim, a shaded face, and a chevron pointing at the
//! hidden icons (‹ while folded away, › while shown). Each scheme's
//! [`ArrowRecipe`] holds the shapes and inks; [`art`] paints the six frames
//! (‹ and ›, each at rest, lit and held) at `d` device px per design px.
//!
//! Navy, olive green and silver are a sphere lit from the top left
//! ([`Shading::Lit`]); slate, sapphire, obsidian and ember a glass bead
//! ([`Shading::Banded`]): lighter upper half under a gloss cap, dark lower
//! half with a glow rising from the bottom. Slate's and ember's discs are
//! 17 px, centred in the 19 px box. The disc's edge is anti-aliased and the
//! rest clear, so the button draws over any bar.

use crate::blue::Variant;
use crate::raster::Canvas;
use crate::shape::{
    CLEAR, Px, Stops, draw, ellipse, mix, over, premul, radial, scale, shares, stroke,
};
use crate::style::trayarrow::{SIZE, TrayArrowArt};
use crate::theme::Color;

const CENTRE: f64 = 9.5;
/// Where a banded face turns from its light upper half to its dark lower half, in design px from the top.
const SPLIT: f64 = 8.0;
/// A lit face's light spot (x, y) and the reach of its shading (x, y): lightest at the spot, darkest a reach away.
const LIGHT_SPOT: (f64, f64) = (4.5, 3.5);
const LIGHT_REACH: (f64, f64) = (12.5, 15.0);
/// How far a lit face stays at its light ink, as a share of the reach.
const LIGHT_HOLD: f64 = 0.3;
const LIGHT_MIDDLE: f64 = 0.62;
const RIM_HOLD: f64 = 0.45;
/// A ring shows from this many design px above the centre and is fully in ink this far below.
const RING_FROM: f64 = 2.0;
const RING_FADE: f64 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shading {
    /// A sphere lit from the top left: an elliptical gradient from [`LIGHT_SPOT`] through the state's three face inks.
    Lit,
    /// A glass bead: lighter upper half under a gloss cap, split at [`SPLIT`]
    /// from a dark lower half with a bottom glow, and a 1 px ring whose ink
    /// rises from the middle to the bottom.
    Banded {
        ring: Option<f64>,
        /// The gloss cap: an ellipse (x radius, y radius, centre's y offset from the disc's centre), cut off at the split.
        gloss: (f64, f64, f64),
        /// The bottom glow: an elliptical gradient (x reach, y reach, centre's y offset from the disc's centre).
        glow: (f64, f64, f64),
    },
}

/// A ‹ whose tip sits at `tip` on the middle row and whose arms end `half`
/// rows above and below at `arm`, stroked `width` wide (along the row). An
/// `edge` above 0 draws a darker outline that far out. The › is the ‹ flipped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chevron {
    pub(crate) tip: f64,
    pub(crate) arm: f64,
    pub(crate) half: f64,
    pub(crate) width: f64,
    pub(crate) edge: f64,
}

pub(crate) const BOLD: Chevron = Chevron {
    tip: 7.5,
    arm: 11.5,
    half: 4.0,
    width: 2.2,
    edge: 0.0,
};

pub(crate) const OUTLINED: Chevron = Chevron {
    width: 2.0,
    edge: 1.3,
    ..BOLD
};

pub(crate) const FINE: Chevron = Chevron {
    tip: 7.5,
    arm: 10.5,
    half: 3.0,
    width: 1.5,
    edge: 0.0,
};

impl Chevron {
    fn points(&self, open: bool) -> [(f64, f64); 3] {
        let x = |v: f64| if open { f64::from(SIZE) - v } else { v };
        [
            (x(self.arm), CENTRE - self.half),
            (x(self.tip), CENTRE),
            (x(self.arm), CENTRE + self.half),
        ]
    }
}

/// One state's inks. `face` is a lit face's light, middle and dark, or a
/// banded face's upper half top and bottom and lower half dark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowInks {
    pub(crate) face: [Color; 3],
    pub(crate) gloss: Color,
    pub(crate) glow: Color,
    pub(crate) ring: Color,
    pub(crate) glyph: Color,
    pub(crate) edge: Color,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArrowRecipe {
    pub(crate) radius: f64,
    pub(crate) rim_width: f64,
    pub(crate) rim: [Color; 2],
    pub(crate) rim_axis: (f64, f64),
    pub(crate) shading: Shading,
    pub(crate) chevron: Chevron,
    pub(crate) states: [ArrowInks; 3],
}

pub(crate) fn art(recipe: &ArrowRecipe, d: i32) -> TrayArrowArt {
    let frame = |open, st: usize| paint(recipe, &recipe.states[st], open, d);
    TrayArrowArt {
        scale: d,
        rest: frame(false, 0),
        hot: frame(false, 1),
        pressed: frame(false, 2),
        open_rest: frame(true, 0),
        open_hot: frame(true, 1),
        open_pressed: frame(true, 2),
    }
}

fn paint(recipe: &ArrowRecipe, inks: &ArrowInks, open: bool, d: i32) -> Canvas {
    let k = f64::from(d);
    let c = CENTRE * k;
    let disc = |x: f64, y: f64, r: f64| ellipse(x, y, c, c, r * k, r * k);
    let solid = |c: Color| premul(c, 255);
    let pts = recipe.chevron.points(open).map(|(x, y)| (x * k, y * k));
    draw(SIZE, SIZE, d, |x, y| {
        let outer = disc(x, y, recipe.radius);
        if outer == 0.0 {
            return CLEAR;
        }
        let (u, v) = ((x + 0.5) / k, (y + 0.5) / k);
        let inner = disc(x, y, recipe.radius - recipe.rim_width);
        let rim_ink = rim_at(recipe, u, v);
        let face = face_at(recipe, inks, x, y, v, d);
        let ring = match recipe.shading {
            Shading::Banded { ring: Some(r), .. } => {
                let band = disc(x, y, r + 1.0) - disc(x, y, r);
                let w = ((v - (CENTRE - RING_FROM)) / RING_FADE).clamp(0.0, 1.0);
                (mix(face, solid(inks.ring), w), band)
            }
            _ => (CLEAR, 0.0),
        };
        let base = shares(&[(rim_ink, outer - inner), ring, (face, inner - ring.1)]);
        let edge = if recipe.chevron.edge > 0.0 {
            stroke(x, y, &pts, (recipe.chevron.width + recipe.chevron.edge) * k)
        } else {
            0.0
        };
        let glyph = stroke(x, y, &pts, recipe.chevron.width * k);
        over(
            scale(solid(inks.glyph), glyph),
            over(scale(solid(inks.edge), edge), base),
        )
    })
}

fn rim_at(recipe: &ArrowRecipe, u: f64, v: f64) -> Px {
    let [a, b] = recipe.rim.map(|c| premul(c, 255));
    let (ax, ay) = recipe.rim_axis;
    let along = ((u - CENTRE) * ax + (v - CENTRE) * ay) / recipe.radius;
    Stops(&[(0.0, a), (RIM_HOLD, a), (1.0, b)]).at(along * 0.5 + 0.5)
}

fn face_at(recipe: &ArrowRecipe, inks: &ArrowInks, x: f64, y: f64, v: f64, d: i32) -> Px {
    let k = f64::from(d);
    let solid = |c: Color| premul(c, 255);
    let [a, b, z] = inks.face.map(solid);
    match recipe.shading {
        Shading::Lit => radial(
            x,
            y,
            LIGHT_SPOT.0 * k,
            LIGHT_SPOT.1 * k,
            LIGHT_REACH.0 * k,
            LIGHT_REACH.1 * k,
            &Stops(&[(0.0, a), (LIGHT_HOLD, a), (LIGHT_MIDDLE, b), (1.0, z)]),
        ),
        Shading::Banded { gloss, glow, .. } => {
            if v >= SPLIT {
                let stops = [(0.0, solid(inks.glow)), (1.0, z)];
                return radial(
                    x,
                    y,
                    CENTRE * k,
                    (CENTRE + glow.2) * k,
                    glow.0 * k,
                    glow.1 * k,
                    &Stops(&stops),
                );
            }
            let top = CENTRE - recipe.radius;
            let body = Stops(&[(0.0, a), (1.0, b)]).at((v - top) / (SPLIT - top));
            let cap = ellipse(
                x,
                y,
                CENTRE * k,
                (CENTRE + gloss.2) * k,
                gloss.0 * k,
                gloss.1 * k,
            );
            mix(body, solid(inks.gloss), cap)
        }
    }
}

pub(crate) const fn lit(face: [Color; 3], glyph: Color, edge: Color) -> ArrowInks {
    ArrowInks {
        face,
        gloss: face[0],
        glow: face[0],
        ring: face[0],
        glyph,
        edge,
    }
}

pub(crate) const fn banded(
    up: [Color; 2],
    gloss: Color,
    glow: Color,
    dark: Color,
    ring: Color,
    glyph: Color,
) -> ArrowInks {
    ArrowInks {
        face: [up[0], up[1], dark],
        gloss,
        glow,
        ring,
        glyph,
        edge: glyph,
    }
}

/// The diagonal a lit face's rim runs along: dark top left, light bottom right.
pub(crate) const DIAGONAL: (f64, f64) = (
    std::f64::consts::FRAC_1_SQRT_2,
    std::f64::consts::FRAC_1_SQRT_2,
);
pub(crate) const DOWN: (f64, f64) = (0.0, 1.0);

pub(crate) const BEAD: Shading = Shading::Banded {
    ring: Some(6.5),
    gloss: (5.0, 2.0, -5.5),
    glow: (5.5, 3.5, 5.0),
};

pub(crate) fn recipe(v: Variant) -> ArrowRecipe {
    v.scheme().tray_arrow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::pack;

    fn fnv(c: &Canvas) -> u64 {
        let (w, h) = c.device_size();
        [w, h]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            })
    }

    fn frames(a: &TrayArrowArt) -> [&Canvas; 6] {
        [
            &a.rest,
            &a.hot,
            &a.pressed,
            &a.open_rest,
            &a.open_hot,
            &a.open_pressed,
        ]
    }

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(x, y) >> 24
    }

    fn from_centre(x: i32, y: i32, d: i32) -> (f64, f64) {
        let c = CENTRE * f64::from(d);
        (f64::from(x) + 0.5 - c, f64::from(y) + 0.5 - c)
    }

    #[test]
    fn every_frame_is_19_px_a_side_times_the_scale() {
        for v in Variant::ALL {
            for d in [1, 2, 3] {
                let a = art(&recipe(v), d);
                assert_eq!(a.scale, d);
                let n = (SIZE * d) as u32;
                for f in frames(&a) {
                    assert_eq!((f.width, f.height, f.scale()), (n, n, 1), "{v:?} at {d}");
                }
            }
        }
    }

    #[test]
    fn beyond_the_disc_the_frame_is_clear() {
        for v in Variant::ALL {
            let r = recipe(v);
            for d in [1, 2] {
                let a = art(&r, d);
                let reach = r.radius * f64::from(d) + 0.75;
                for f in frames(&a) {
                    for y in 0..SIZE * d {
                        for x in 0..SIZE * d {
                            let (u, w) = from_centre(x, y, d);
                            if u.hypot(w) > reach {
                                assert_eq!(f.get(x, y), 0, "{v:?} at {d}: ({x}, {y})");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn slate_and_ember_draw_a_disc_a_pixel_smaller_round() {
        for v in Variant::ALL {
            let a = art(&recipe(v), 1);
            let small = matches!(v, Variant::Slate | Variant::Ember);
            assert_eq!(alpha(&a.rest, 9, 0) == 0, small, "{v:?}: the top row");
            assert_eq!(alpha(&a.rest, 9, 1), 255, "{v:?}: the second row");
        }
    }

    #[test]
    fn each_state_and_direction_draws_its_own_frame() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let a = art(&recipe(v), d);
                let f = frames(&a);
                for i in 0..f.len() {
                    for j in i + 1..f.len() {
                        assert_ne!(f[i], f[j], "{v:?} at {d}: frames {i} and {j}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_scheme_draws_its_own_button() {
        let rest = Variant::ALL.map(|v| art(&recipe(v), 1).rest);
        for i in 0..rest.len() {
            for j in i + 1..rest.len() {
                let (a, b) = (Variant::ALL[i], Variant::ALL[j]);
                assert_ne!(rest[i], rest[j], "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn the_disc_edge_is_smooth_at_scale_2() {
        for v in Variant::ALL {
            let r = recipe(v);
            let a = art(&r, 2);
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let soft = (0..2 * SIZE)
                    .flat_map(|y| (0..2 * SIZE).map(move |x| (x, y)))
                    .any(|(x, y)| {
                        let (u, w) = from_centre(x, y, 2);
                        let on_edge = (u.hypot(w) - r.radius * 2.0).abs() < 1.0;
                        let k = alpha(&a.rest, x, y);
                        on_edge && u * sx > 0.0 && w * sy > 0.0 && k > 0 && k < 255
                    });
                assert!(soft, "{v:?}: the quarter toward ({sx}, {sy})");
            }
        }
    }

    /// A partly covered edge pixel, unpremultiplied, is never darker than both
    /// the rim's ink there and the pixel inside it: no dark fringe.
    #[test]
    fn the_disc_edge_has_no_dark_fringe() {
        let lum = |p: u32| {
            let a = f64::from(p >> 24);
            let ch = |s: u32| f64::from((p >> s) & 0xFF) * 255.0 / a;
            0.3 * ch(16) + 0.59 * ch(8) + 0.11 * ch(0)
        };
        for v in Variant::ALL {
            let r = recipe(v);
            for d in [1, 2] {
                let a = art(&r, d);
                let k = f64::from(d);
                for y in 0..SIZE * d {
                    for x in 0..SIZE * d {
                        let p = a.rest.get(x, y);
                        if p >> 24 == 0 || p >> 24 == 255 {
                            continue;
                        }
                        let (u, w) = from_centre(x, y, d);
                        let n = u.hypot(w);
                        let (ix, iy) = (
                            (f64::from(x) - u / n).round() as i32,
                            (f64::from(y) - w / n).round() as i32,
                        );
                        let inside = lum(a.rest.get(ix, iy));
                        let rim = (f64::from(x) + 0.5) / k;
                        let rim = lum(pack(rim_at(&r, rim, (f64::from(y) + 0.5) / k)));
                        // Rounding a faint pixel's channels moves its unpremultiplied colour up to 255 / alpha.
                        let slack = 1.0 + 255.0 / f64::from(p >> 24);
                        assert!(
                            lum(p) >= rim.min(inside) - slack,
                            "{v:?} at {d}: ({x}, {y}) {p:08x} is darker than the rim ({rim}) and ({ix}, {iy}) ({inside})"
                        );
                    }
                }
            }
        }
    }

    /// At scale 1 each arm of the chevron has a pixel wholly in the glyph's ink.
    #[test]
    fn the_chevron_holds_solid_pixels_on_each_arm_at_scale_1() {
        for v in Variant::ALL {
            let r = recipe(v);
            let a = art(&r, 1);
            for (i, f) in frames(&a).into_iter().enumerate() {
                let ink = r.states[i % 3].glyph.argb();
                let solid = |rows: std::ops::Range<i32>| {
                    rows.flat_map(|y| (0..SIZE).map(move |x| (x, y)))
                        .any(|(x, y)| f.get(x, y) == ink)
                };
                assert!(solid(5..9), "{v:?} frame {i}: the upper arm");
                assert!(solid(10..14), "{v:?} frame {i}: the lower arm");
            }
        }
    }

    #[test]
    fn the_chevron_points_back_while_shut_and_on_while_open() {
        for v in Variant::ALL {
            let r = recipe(v);
            let a = art(&r, 1);
            let ink = r.states[0].glyph.argb();
            let tip = r.chevron.tip.floor() as i32;
            assert_eq!(a.rest.get(tip, 9), ink, "{v:?}: the ‹'s tip");
            assert_eq!(
                a.open_rest.get(SIZE - 1 - tip, 9),
                ink,
                "{v:?}: the ›'s tip"
            );
            assert_ne!(a.rest.get(SIZE - 1 - tip, 9), ink, "{v:?}: no › on the ‹");
        }
    }

    #[test]
    fn tray_arrow_pins_at_scale_1_and_2() {
        const FRAMES: [&str; 6] = [
            "rest",
            "hot",
            "pressed",
            "open_rest",
            "open_hot",
            "open_pressed",
        ];
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                let hashes = frames(&art(&recipe(v), d)).map(fnv);
                for (name, h) in FRAMES.into_iter().zip(hashes) {
                    got.push((format!("{v:?} {name} {d}"), h));
                }
            }
        }
        crate::pins::check("blue/tray_arrow", &got);
    }
}
