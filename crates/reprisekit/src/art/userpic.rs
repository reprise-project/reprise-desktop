//! The stock account pictures User Accounts offers: eight small square
//! scenes built from flat shapes (a sunset, mountains, a sailboat, a
//! flower, a ringed planet, a fish, a balloon and a tree). Each is a
//! function of the unit square, averaged over 4x4 points per pixel, so
//! it draws at any size with soft edges. They are opaque.

use crate::raster::Canvas;
use crate::theme::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    Sunset,
    Mountains,
    Sailboat,
    Flower,
    Planet,
    Fish,
    Balloon,
    Tree,
}

pub const ALL: [Picture; 8] = [
    Picture::Sunset,
    Picture::Mountains,
    Picture::Sailboat,
    Picture::Flower,
    Picture::Planet,
    Picture::Fish,
    Picture::Balloon,
    Picture::Tree,
];

pub const SAVED_PX: u32 = 96;

const SS: u32 = 4;

pub const SKY: Color = Color::rgb(0x6c, 0xb4, 0xe8);
pub const SUN: Color = Color::rgb(0xff, 0xd8, 0x40);
pub const SEA: Color = Color::rgb(0x1e, 0x5a, 0x9c);
pub const SNOW: Color = Color::rgb(0xf4, 0xf8, 0xfc);
pub const GRASS: Color = Color::rgb(0x4c, 0x9a, 0x3a);
pub const PETAL: Color = Color::rgb(0xf0, 0x6c, 0xa8);
pub const SPACE: Color = Color::rgb(0x14, 0x18, 0x40);
pub const FISH: Color = Color::rgb(0xf8, 0x8c, 0x1c);
pub const LEAVES: Color = Color::rgb(0x2e, 0x8a, 0x3c);

const SUNSET_TOP: Color = Color::rgb(0x5a, 0x3c, 0x8c);
const SUNSET_LOW: Color = Color::rgb(0xf8, 0x8c, 0x3c);
const SEA_GLINT: Color = Color::rgb(0xf8, 0xb0, 0x50);
const ROCK: Color = Color::rgb(0x6a, 0x6e, 0x80);
const ROCK_FAR: Color = Color::rgb(0x8c, 0x96, 0xb0);
const SAIL: Color = Color::rgb(0xfc, 0xfc, 0xf4);
const SAIL_RED: Color = Color::rgb(0xd8, 0x3c, 0x30);
const HULL: Color = Color::rgb(0x7a, 0x48, 0x24);
const MEADOW: Color = Color::rgb(0x9c, 0xd0, 0x6c);
const STEM: Color = Color::rgb(0x2c, 0x7c, 0x2c);
const HEART: Color = Color::rgb(0xf8, 0xd0, 0x30);
const PLANET: Color = Color::rgb(0xe0, 0x9c, 0x54);
const PLANET_BAND: Color = Color::rgb(0xc0, 0x74, 0x3c);
const RING: Color = Color::rgb(0xd8, 0xd0, 0xb8);
const STAR: Color = Color::rgb(0xf0, 0xf0, 0xff);
const WATER: Color = Color::rgb(0x3c, 0xb4, 0xc8);
const WATER_DEEP: Color = Color::rgb(0x1c, 0x78, 0x9c);
const EYE: Color = Color::rgb(0x10, 0x10, 0x10);
const BUBBLE: Color = Color::rgb(0xb8, 0xec, 0xf4);
const STRIPE_A: Color = Color::rgb(0xe0, 0x38, 0x38);
const STRIPE_B: Color = Color::rgb(0xf8, 0xd0, 0x30);
const BASKET: Color = Color::rgb(0x8a, 0x5a, 0x2c);
const CLOUD: Color = Color::rgb(0xff, 0xff, 0xff);
const TRUNK: Color = Color::rgb(0x6c, 0x44, 0x24);
const LEAVES_LIT: Color = Color::rgb(0x58, 0xb0, 0x48);

impl Picture {
    pub fn name(self) -> &'static str {
        match self {
            Picture::Sunset => "Sunset",
            Picture::Mountains => "Mountains",
            Picture::Sailboat => "Sailboat",
            Picture::Flower => "Flower",
            Picture::Planet => "Planet",
            Picture::Fish => "Fish",
            Picture::Balloon => "Balloon",
            Picture::Tree => "Tree",
        }
    }

    fn at(self, u: f32, v: f32) -> Color {
        match self {
            Picture::Sunset => sunset(u, v),
            Picture::Mountains => mountains(u, v),
            Picture::Sailboat => sailboat(u, v),
            Picture::Flower => flower(u, v),
            Picture::Planet => planet(u, v),
            Picture::Fish => fish(u, v),
            Picture::Balloon => balloon(u, v),
            Picture::Tree => tree(u, v),
        }
    }
}

pub fn draw(p: Picture, px: u32) -> Canvas {
    let mut c = Canvas::new(px, px);
    let n = SS * SS;
    let step = 1.0 / (px * SS) as f32;
    for y in 0..px {
        for x in 0..px {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for sy in 0..SS {
                for sx in 0..SS {
                    let u = ((x * SS + sx) as f32 + 0.5) * step;
                    let v = ((y * SS + sy) as f32 + 0.5) * step;
                    let k = p.at(u, v);
                    r += k.r as u32;
                    g += k.g as u32;
                    b += k.b as u32;
                }
            }
            let k = Color::rgb(
                ((r + n / 2) / n) as u8,
                ((g + n / 2) / n) as u8,
                ((b + n / 2) / n) as u8,
            );
            c.put(x as i32, y as i32, k);
        }
    }
    c
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color::rgb(l(a.r, b.r), l(a.g, b.g), l(a.b, b.b))
}

fn in_circle(u: f32, v: f32, cx: f32, cy: f32, r: f32) -> bool {
    (u - cx).powi(2) + (v - cy).powi(2) <= r * r
}

fn in_ellipse(u: f32, v: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
    ((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2) <= 1.0
}

/// Whether (`u`, `v`) is inside the triangle `a`, `b`, `c` (either
/// winding).
fn in_triangle(u: f32, v: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let side = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (v - p.1) - (q.1 - p.1) * (u - p.0);
    let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// The sun half set into a striped evening sea.
fn sunset(u: f32, v: f32) -> Color {
    const HORIZON: f32 = 0.62;
    if v < HORIZON {
        if in_circle(u, v, 0.5, HORIZON, 0.22) {
            return SUN;
        }
        return mix(SUNSET_TOP, SUNSET_LOW, v / HORIZON);
    }
    // The sun's glints: short bands under it, narrowing with depth.
    let row = ((v - HORIZON) / 0.06) as i32;
    let half = 0.2 - row as f32 * 0.035;
    let band = (v - HORIZON) % 0.06 < 0.025;
    if band && (u - 0.5).abs() < half {
        return SEA_GLINT;
    }
    SEA
}

/// Two peaks with snow caps over a meadow.
fn mountains(u: f32, v: f32) -> Color {
    if v > 0.8 {
        return GRASS;
    }
    let far = ((0.62, 0.28), (0.98, 0.8), (0.26, 0.8));
    let near = ((0.36, 0.18), (0.78, 0.8), (-0.06, 0.8));
    if in_triangle(u, v, near.0, near.1, near.2) {
        // The cap: the top of the peak, cut by a zigzag line.
        let zig = 0.36 + 0.03 * ((u * 40.0).sin());
        return if v < zig { SNOW } else { ROCK };
    }
    if in_triangle(u, v, far.0, far.1, far.2) {
        return if v < 0.42 { SNOW } else { ROCK_FAR };
    }
    if in_circle(u, v, 0.84, 0.16, 0.08) {
        return SUN;
    }
    SKY
}

/// A sailboat with a white mainsail and a red jib on a blue sea.
fn sailboat(u: f32, v: f32) -> Color {
    if in_triangle(u, v, (0.48, 0.14), (0.48, 0.66), (0.18, 0.66)) {
        return SAIL;
    }
    if in_triangle(u, v, (0.52, 0.22), (0.52, 0.66), (0.76, 0.66)) {
        return SAIL_RED;
    }
    if (0.48..0.52).contains(&u) && (0.12..0.7).contains(&v) {
        return HULL;
    }
    // The hull: a trapezium narrowing to its keel.
    if (0.68..0.8).contains(&v) {
        let t = (v - 0.68) / 0.12;
        let (l, r) = (0.14 + 0.08 * t, 0.86 - 0.08 * t);
        if (l..r).contains(&u) {
            return HULL;
        }
    }
    if v > 0.74 {
        let wave = ((u * 18.0).sin() * 0.012) + 0.74;
        return if v > wave { SEA } else { SKY };
    }
    SKY
}

/// A five-petal flower on a stem.
fn flower(u: f32, v: f32) -> Color {
    let (cx, cy) = (0.5, 0.4);
    if in_circle(u, v, cx, cy, 0.1) {
        return HEART;
    }
    for k in 0..5 {
        let a = std::f32::consts::TAU * k as f32 / 5.0 - std::f32::consts::FRAC_PI_2;
        if in_circle(u, v, cx + 0.17 * a.cos(), cy + 0.17 * a.sin(), 0.12) {
            return PETAL;
        }
    }
    if (0.48..0.52).contains(&u) && v > cy {
        return STEM;
    }
    if in_ellipse(u, v, 0.6, 0.74, 0.1, 0.04) {
        return STEM;
    }
    if v > 0.86 {
        return GRASS;
    }
    MEADOW
}

/// A banded planet in a tilted ring, among stars.
fn planet(u: f32, v: f32) -> Color {
    let (cx, cy) = (0.5, 0.5);
    // The ring's frame: tilted by rotating the point the other way.
    let (s, c) = (-0.35f32).sin_cos();
    let (du, dv) = (u - cx, v - cy);
    let (ru, rv) = (du * c - dv * s, du * s + dv * c);
    let ring =
        in_ellipse(ru, rv, 0.0, 0.0, 0.42, 0.11) && !in_ellipse(ru, rv, 0.0, 0.0, 0.33, 0.07);
    let body = in_circle(u, v, cx, cy, 0.24);
    // The ring passes in front of the planet's lower half.
    if ring && (!body || rv > 0.0) {
        return RING;
    }
    if body {
        let band = ((rv + 0.24) / 0.08) as i32 % 2 == 1;
        return if band { PLANET_BAND } else { PLANET };
    }
    const STARS: [(f32, f32); 6] = [
        (0.12, 0.14),
        (0.84, 0.1),
        (0.2, 0.82),
        (0.9, 0.74),
        (0.66, 0.9),
        (0.08, 0.5),
    ];
    if STARS.iter().any(|&(x, y)| in_circle(u, v, x, y, 0.018)) {
        return STAR;
    }
    SPACE
}

/// An orange fish and its bubbles in deepening water.
fn fish(u: f32, v: f32) -> Color {
    if in_circle(u, v, 0.34, 0.44, 0.03) {
        return EYE;
    }
    if in_ellipse(u, v, 0.46, 0.5, 0.24, 0.15) {
        return FISH;
    }
    if in_triangle(u, v, (0.66, 0.5), (0.88, 0.34), (0.88, 0.66)) {
        return FISH;
    }
    let bubbles = [(0.18, 0.3, 0.035), (0.14, 0.18, 0.025), (0.2, 0.08, 0.02)];
    if bubbles.iter().any(|&(x, y, r)| in_circle(u, v, x, y, r)) {
        return BUBBLE;
    }
    mix(WATER, WATER_DEEP, v)
}

/// A striped hot-air balloon with its basket, a cloud behind it.
fn balloon(u: f32, v: f32) -> Color {
    let (cx, cy, r) = (0.5, 0.36, 0.24);
    if in_circle(u, v, cx, cy, r) || in_triangle(u, v, (0.29, 0.46), (0.71, 0.46), (0.5, 0.7)) {
        // Stripes run down the envelope, narrowing to the mouth.
        let w = (r * (1.0 - ((v - 0.12) / 0.7).clamp(0.0, 1.0) * 0.8)).max(0.02);
        let k = ((u - cx) / w * 2.0 + 10.0) as i32;
        return if k % 2 == 0 { STRIPE_A } else { STRIPE_B };
    }
    if (0.42..0.58).contains(&u) && (0.74..0.84).contains(&v) {
        return BASKET;
    }
    // The ropes from the mouth down to the basket's corners.
    for (top, foot) in [(0.46f32, 0.43f32), (0.54, 0.57)] {
        if (0.7..0.74).contains(&v) {
            let x = top + (foot - top) * (v - 0.7) / 0.04;
            if (u - x).abs() < 0.008 {
                return BASKET;
            }
        }
    }
    let cloud = [(0.18, 0.78, 0.08), (0.28, 0.75, 0.1), (0.38, 0.8, 0.07)];
    if cloud.iter().any(|&(x, y, r)| in_circle(u, v, x, y, r)) && v < 0.84 {
        return CLOUD;
    }
    SKY
}

/// A round tree on a hill.
fn tree(u: f32, v: f32) -> Color {
    let lit = in_circle(u, v, 0.43, 0.3, 0.12);
    if in_circle(u, v, 0.5, 0.38, 0.24) {
        return if lit { LEAVES_LIT } else { LEAVES };
    }
    if (0.46..0.54).contains(&u) && (0.55..0.82).contains(&v) {
        return TRUNK;
    }
    if in_circle(u, v, 0.5, 1.6, 0.8) {
        return GRASS;
    }
    SKY
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: &Canvas, x: i32, y: i32) -> Color {
        let v = c.get(x, y);
        Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    #[test]
    fn every_picture_is_opaque_and_square() {
        for p in ALL {
            let c = draw(p, 48);
            assert_eq!((c.width, c.height), (48, 48), "{p:?}");
            for y in 0..48 {
                for x in 0..48 {
                    assert_eq!(c.get(x, y) >> 24, 0xff, "{p:?} at {x},{y}");
                }
            }
        }
    }

    /// One pixel inside each picture's main shape and one in its
    /// background, at 48 px, wear the shape's flat colour.
    #[test]
    fn the_shapes_sit_where_the_pictures_put_them() {
        let cases = [
            (Picture::Sunset, (24, 26), SUN, (4, 44), SEA),
            (Picture::Mountains, (17, 12), SNOW, (24, 44), GRASS),
            (Picture::Sailboat, (20, 28), SAIL, (4, 4), SKY),
            (Picture::Flower, (24, 19), HEART, (4, 4), MEADOW),
            (Picture::Planet, (40, 30), RING, (2, 2), SPACE),
            (Picture::Fish, (16, 21), EYE, (26, 24), FISH),
            (Picture::Balloon, (24, 38), BASKET, (44, 4), SKY),
            (Picture::Tree, (32, 22), LEAVES, (24, 46), GRASS),
        ];
        for (p, a, ca, b, cb) in cases {
            let c = draw(p, 48);
            assert_eq!(px(&c, a.0, a.1), ca, "{p:?} at {a:?}");
            assert_eq!(px(&c, b.0, b.1), cb, "{p:?} at {b:?}");
        }
    }

    #[test]
    fn edges_are_softened() {
        // The sun's rim over the evening sky blends the two.
        let c = draw(Picture::Sunset, 96);
        let mut seen_mixed = false;
        for x in 0..96 {
            let k = px(&c, x, 48);
            if k != SUN && (k.r, k.g, k.b) != (0, 0, 0) {
                seen_mixed |= k.g > SUNSET_LOW.g && k.g < SUN.g;
            }
        }
        assert!(seen_mixed, "the sun's edge mixes into the sky");
    }

    #[test]
    #[ignore]
    fn render_pictures() {
        let mut out = Canvas::new(8 * 100, 100);
        for (i, p) in ALL.iter().enumerate() {
            out.blit_canvas(i as i32 * 100 + 2, 2, &draw(*p, SAVED_PX));
        }
        crate::raster::write_preview(&out, "userpic-preview.png");
    }
}
