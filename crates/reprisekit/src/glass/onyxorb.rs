//! Onyx's Start orb at output scale: a 41 px disc of dark blue glass standing [`RISE`] px over
//! the bar's top edge, its foot running off the bar's (and the screen's) lower edge. A black
//! outline and a navy rim, a pale gloss over the upper half that is brightest along the rim, and
//! a cyan light rising from the foot; under the pointer the light climbs and the body brightens,
//! held the body darkens. The icon's own glow is Glass's ([`super::orb::bloom`]).

use crate::raster::Canvas;
use crate::shape::{CLEAR, Px, Stops, draw, ellipse, over, premul, radial, scale, shares, smooth};
use crate::style::{Layout, PartImage, State};
use crate::taskbar::OrbRise;
use crate::theme::Color;

/// The Start cell: the disc centred in it, 9 px in from the screen's edge.
pub const WIDTH: i32 = 59;
pub const DIAMETER: i32 = 41;
pub const RISE: i32 = 4;
/// Rows over the bar the orb's buffer holds: the disc's [`RISE`] and the lit icon's glow above it.
pub const REACH: i32 = 11;

pub const ORB_RISE: OrbRise = OrbRise {
    rise: RISE,
    diameter: DIAMETER,
    reach: REACH,
};

const R: f64 = DIAMETER as f64 / 2.0;
const CX: f64 = WIDTH as f64 / 2.0;
/// The disc's centre, design px down from the bar's top edge.
const CY: f64 = R - RISE as f64;

const OUTLINE: Color = Color::rgb(0x0b, 0x0e, 0x10);
const OUTLINE_ALPHA: u8 = 235;
const NAVY: Color = Color::rgb(0x06, 0x22, 0x3a);
const RIM: Color = Color::rgb(0x00, 0x4c, 0x82);
/// The soft shade the lower half casts on the bar: its alpha and width.
const SHADE: (f64, f64) = (110.0, 2.0);

/// The gloss: an ellipse over the upper half whose flat foot lies 3 px above the disc's centre.
const GLOSS_CY: f64 = CY - 14.0;
const GLOSS_R: (f64, f64) = (30.0, 11.0);
/// The gloss from the disc's top (0) to its foot (1).
const GLOSS: [(f64, Color, u8); 5] = [
    (0.0, Color::rgb(0xe4, 0xec, 0xf2), 240),
    (0.12, Color::rgb(0xc4, 0xd4, 0xe0), 235),
    (0.35, Color::rgb(0xa8, 0xbe, 0xd0), 175),
    (0.7, Color::rgb(0x88, 0xa6, 0xbe), 105),
    (1.0, Color::rgb(0x70, 0x94, 0xb4), 70),
];
/// How much the gloss brightens along the rim, and over how many px inward it fades.
const GLOSS_EDGE: (f64, f64) = (0.45, 2.5);

/// The foot light's spread across, as a share of how far up it reaches.
const FOOT_SPREAD: f64 = 1.4;
/// The foot light from the disc's foot (0) to its reach (1).
const FOOT_FADE: [(f64, u8); 3] = [(0.0, 255), (0.5, 235), (1.0, 0)];

#[derive(Clone, Copy)]
struct Look {
    /// The body at the disc's top and at its foot.
    body: [Color; 2],
    gloss: f64,
    foot: Color,
    /// Design px up from the disc's foot the light reaches.
    foot_reach: f64,
}

const LOOKS: [(State, Look); 3] = [
    (
        State::Normal,
        Look {
            body: [Color::rgb(0x01, 0x3a, 0x62), Color::rgb(0x03, 0x4c, 0x7c)],
            gloss: 1.0,
            foot: Color::rgb(0x0a, 0xc8, 0xf0),
            foot_reach: 17.0,
        },
    ),
    (
        State::Hot,
        Look {
            body: [Color::rgb(0x05, 0x52, 0x84), Color::rgb(0x16, 0x8a, 0xbc)],
            gloss: 1.12,
            foot: Color::rgb(0x38, 0xe4, 0xff),
            foot_reach: 24.0,
        },
    ),
    (
        State::Pressed,
        Look {
            body: [Color::rgb(0x04, 0x2c, 0x4c), Color::rgb(0x08, 0x3c, 0x62)],
            gloss: 0.75,
            foot: Color::rgb(0x1c, 0x80, 0xa8),
            foot_reach: 14.0,
        },
    ),
];

fn opaque(c: Color) -> Px {
    premul(c, 255)
}

/// The orb's pixel at device (`x`, `y`) of a buffer whose top row lies `top` design px below the
/// bar's top edge.
fn px(look: Look, top: i32, x: f64, y: f64, d: i32) -> Px {
    let k = f64::from(d);
    let y = y + f64::from(top) * k;
    let (mx, my) = ((x + 0.5) / k, (y + 0.5) / k);
    let disc = |r: f64| ellipse(x, y, CX * k, CY * k, r * k, r * k);
    let (outer, navy, rim, body) = (disc(R), disc(R - 1.0), disc(R - 2.0), disc(R - 3.0));
    let dist = (mx - CX).hypot(my - CY);
    if outer <= 0.0 {
        let past = dist - R;
        if my <= CY || past >= SHADE.1 {
            return CLEAR;
        }
        let a = SHADE.0 * (1.0 - smooth(past / SHADE.1)) * smooth((my - CY) / R * 2.0);
        return premul(Color::rgb(0, 0, 0), a.round() as u8);
    }

    let down = ((my - (CY - R)) / (2.0 * R)).clamp(0.0, 1.0);
    let mut inside = Stops(&[(0.0, opaque(look.body[0])), (1.0, opaque(look.body[1]))]).at(down);
    let reach = look.foot_reach;
    let foot = radial(
        x,
        y,
        CX * k,
        (CY + R) * k,
        reach * FOOT_SPREAD * k,
        reach * k,
        &Stops(&FOOT_FADE.map(|(t, a)| (t, premul(look.foot, a)))),
    );
    inside = over(foot, inside);
    let ring = shares(&[
        (premul(OUTLINE, OUTLINE_ALPHA), outer - navy),
        (opaque(NAVY), navy - rim),
        (opaque(RIM), rim - body),
        (inside, body),
    ]);

    let g = ellipse(x, y, CX * k, GLOSS_CY * k, GLOSS_R.0 * k, GLOSS_R.1 * k).min(outer);
    if g <= 0.0 {
        return ring;
    }
    let foot_of_gloss = GLOSS_CY + GLOSS_R.1;
    let t = ((my - (CY - R)) / (foot_of_gloss - (CY - R))).clamp(0.0, 1.0);
    let stops = GLOSS.map(|(t, c, a)| (t, premul(c, a)));
    let edge = GLOSS_EDGE.0 * (1.0 - smooth((R - 1.0 - dist) / GLOSS_EDGE.1));
    let gloss = Stops(&stops).at(t);
    let strength = ((gloss.0 / 255.0 + edge) * look.gloss).min(1.0) * 255.0 / gloss.0.max(1.0);
    over(scale(gloss, strength * g), ring)
}

/// The orb's three looks stacked, `h` rows each, the top row `top` design px below the bar's
/// top edge.
fn frames(top: i32, h: i32, d: i32) -> PartImage {
    let mut atlas = Canvas::new((WIDTH * d) as u32, (3 * h * d) as u32);
    for (i, &(_, look)) in LOOKS.iter().enumerate() {
        let f = draw(WIDTH, h, d, |x, y| px(look, top, x, y, d));
        atlas.blit_over(0, i as i32 * h * d, &f);
    }
    let states = LOOKS.map(|(st, _)| st);
    PartImage::new(atlas, &states, Layout::Vertical).at_scale(d)
}

/// The orb's rows inside a bar `bar_height` px tall.
pub fn orb(bar_height: i32, d: i32) -> PartImage {
    frames(0, bar_height, d)
}

/// The [`REACH`] rows over the bar: the disc's top [`RISE`] rows at their foot.
pub fn above(d: i32) -> PartImage {
    frames(-REACH, REACH, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATES: [State; 3] = [State::Normal, State::Hot, State::Pressed];

    fn at(img: &PartImage, st: State, x: i32, y: i32) -> u32 {
        let f = img.frame(st);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn rgb(p: u32) -> [u32; 3] {
        [p >> 16, p >> 8, p].map(|c| c & 0xFF)
    }

    fn fnv(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    #[test]
    fn every_part_has_its_size_at_every_scale() {
        for d in [1, 2, 3] {
            let d32 = d as u32;
            for h in [30, 40] {
                let o = orb(h, d);
                assert_eq!(
                    (o.atlas.width, o.atlas.height),
                    (59 * d32, 3 * h as u32 * d32)
                );
                assert_eq!(o.states, STATES);
            }
            let a = above(d);
            assert_eq!((a.atlas.width, a.atlas.height), (59 * d32, 3 * 11 * d32));
        }
    }

    #[test]
    fn the_disc_stands_4_rows_over_the_bar_9_px_from_its_edge() {
        let a = above(1);
        for st in STATES {
            for y in 0..7 {
                for x in 0..59 {
                    assert_eq!(at(&a, st, x, y) >> 24, 0, "({x}, {y}) clear {st:?}");
                }
            }
            assert!(at(&a, st, 29, 7) >> 24 > 100, "the disc's top row {st:?}");
        }
        let o = orb(30, 1);
        for st in STATES {
            assert_eq!(at(&o, st, 8, 15) >> 24, 0, "left of the disc {st:?}");
            assert!(at(&o, st, 9, 15) >> 24 > 200, "the disc's left edge {st:?}");
            assert!(
                at(&o, st, 49, 15) >> 24 > 200,
                "the disc's right edge {st:?}"
            );
            assert_eq!(at(&o, st, 50, 15) >> 24, 0, "right of the disc {st:?}");
        }
    }

    #[test]
    fn the_body_is_opaque_glass_blue_under_a_pale_gloss_and_a_cyan_foot() {
        let o = orb(30, 1);
        let [r, g, b] = rgb(at(&o, State::Normal, 29, 20));
        assert!(b > 90 && r < 20 && g < 90, "dark blue body: {r} {g} {b}");
        let [r, g, b] = rgb(at(&o, State::Normal, 29, 0));
        assert!(r > 130 && g > 150 && b > 170, "pale gloss: {r} {g} {b}");
        let [r, g, b] = rgb(at(&o, State::Normal, 29, 29));
        assert!(g > 170 && b > 200 && r < 60, "cyan foot: {r} {g} {b}");
        for st in STATES {
            assert_eq!(
                at(&o, st, 29, 20) >> 24,
                255,
                "{st:?}: no bar through the body"
            );
            let [r, g, b] = rgb(at(&o, st, 9, 15));
            assert!(
                r < 30 && g < 30 && b < 30,
                "{st:?} black outline: {r} {g} {b}"
            );
        }
    }

    #[test]
    fn lit_brightens_and_held_darkens_the_body() {
        let o = orb(30, 1);
        let lum = |st| rgb(at(&o, st, 14, 22)).iter().sum::<u32>();
        let (rest, hot, held) = (lum(State::Normal), lum(State::Hot), lum(State::Pressed));
        assert!(hot > rest + 60, "lit {hot}, rest {rest}");
        assert!(held < rest, "held {held}, rest {rest}");
    }

    #[test]
    fn the_edge_is_smooth_at_scale_2() {
        let o = orb(30, 2);
        let diag = (0..30).map(|i| at(&o, State::Normal, 24 + i, 2 * i) >> 24);
        assert!(diag.clone().any(|a| a > 0 && a < 255), "AA on the curve");
        let doubled = (0..59).all(|x| {
            (0..30).all(|y| {
                let p = |dx, dy| o.atlas.get(2 * x + dx, 2 * y + dy);
                p(0, 0) == p(1, 0) && p(0, 0) == p(0, 1) && p(0, 0) == p(1, 1)
            })
        });
        assert!(!doubled, "scale 2 is drawn afresh");
    }

    #[test]
    fn the_onyx_orb_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for d in [1, 2] {
            for h in [30, 40] {
                got.push((format!("orb {h} {d}"), fnv(&orb(h, d).atlas)));
            }
            got.push((format!("above {d}"), fnv(&above(d).atlas)));
        }
        crate::pins::check("glass/onyx_orb", &got);
    }
}
