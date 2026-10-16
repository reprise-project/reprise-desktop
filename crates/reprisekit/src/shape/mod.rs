//! Drawn shapes for the looks that paint their parts from geometry at the
//! output scale: coverage of rounded rects, ellipses, polygons and stroked
//! lines, gradients, premultiplied compositing, and stroked glyphs.
//!
//! Two units meet here. Device pixels are what a part's canvas holds: a
//! part `w` x `h` design pixels painted at `d` device pixels per design
//! pixel is `w * d` x `h * d` of them. Every coverage function takes the
//! device pixel's top-left corner and the shape's geometry in device
//! pixels, so a caller multiplies its design geometry by `d` once. Shades
//! (gradients, glows) are worked out in design pixels at each device
//! pixel's middle ([`centre`]), so a part painted at 2 is its design drawn
//! again at twice the size, never its scale-1 pixels doubled.
//!
//! Colours are [`Px`]: alpha and premultiplied channels, unrounded, packed
//! to a canvas pixel only at the end ([`pack`]).

mod cover;
mod glyph;
mod grad;

pub use cover::{ellipse, polygon, ringed, rounded, rounded4, stroke};
pub use glyph::{Glyph, glyph};
pub use grad::{Stops, radial};

use crate::raster::Canvas;
use crate::theme::Color;

pub type Px = (f64, [f64; 3]);

pub const CLEAR: Px = (0.0, [0.0; 3]);

pub fn premul(c: Color, a: u8) -> Px {
    let k = f64::from(a) / 255.0;
    (
        f64::from(a),
        [f64::from(c.r) * k, f64::from(c.g) * k, f64::from(c.b) * k],
    )
}

pub fn unpack(px: u32) -> Px {
    let ch = |shift: u32| f64::from((px >> shift) & 0xFF);
    (ch(24), [ch(16), ch(8), ch(0)])
}

pub fn over(top: Px, under: Px) -> Px {
    let k = 1.0 - top.0 / 255.0;
    (
        top.0 + under.0 * k,
        std::array::from_fn(|i| top.1[i] + under.1[i] * k),
    )
}

/// `a` with `k` (0..1) of `b` in its place, alpha and every channel
/// alike.
pub fn mix(a: Px, b: Px, k: f64) -> Px {
    let m = |a: f64, b: f64| a + (b - a) * k;
    (m(a.0, b.0), std::array::from_fn(|i| m(a.1[i], b.1[i])))
}

pub fn scale(p: Px, k: f64) -> Px {
    (p.0 * k, p.1.map(|v| v * k))
}

/// A pixel split between colours: each `(colour, share)` laid on the
/// share (0..1) of the pixel it covers, the shares not overlapping (a
/// ring and the fill inside it, say) and adding up to at most 1. This,
/// not [`over`], is how two shapes meeting along an edge share its pixels:
/// laying one over the other would count the edge twice.
pub fn shares(parts: &[(Px, f64)]) -> Px {
    let mut px = CLEAR;
    for &(q, share) in parts {
        px.0 += q.0 * share;
        for i in 0..3 {
            px.1[i] += q.1[i] * share;
        }
    }
    px
}

pub fn pack((a, p): Px) -> u32 {
    let a = a.clamp(0.0, 255.0).round();
    let ch = |i: usize| p[i].clamp(0.0, a).round() as u32;
    (a as u32) << 24 | ch(0) << 16 | ch(1) << 8 | ch(2)
}

/// Where the middle of device pixel `v` of a part painted at `d` device
/// pixels per design pixel lies, in design pixels counted from the first
/// design pixel's middle: `v` itself at `d` = 1, and at `d` = 2 pixels 0
/// and 1 lie at -0.25 and 0.25.
pub fn centre(v: i32, d: i32) -> f64 {
    (f64::from(v) + 0.5) / f64::from(d) - 0.5
}

/// How far (`x`, `y`) lies outside a `w` x `h` rect at the origin with corners rounded over `r`,
/// negative inside, in the units it is given.
pub fn rounded_distance(x: f64, y: f64, w: f64, h: f64, r: f64) -> f64 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let qx = (x - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (y - h / 2.0).abs() - (h / 2.0 - r);
    qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

pub fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A `w` x `h` design-pixel image painted at `d`: a `w * d` x `h * d`
/// canvas (at canvas scale 1) whose pixel at device (`x`, `y`) is
/// `f(x, y)` packed ([`pack`]). `f` gets the pixel's top-left corner in
/// device pixels, as the coverage functions take it.
pub fn draw(w: i32, h: i32, d: i32, f: impl Fn(f64, f64) -> Px) -> Canvas {
    let (cw, ch) = (w.max(0) * d.max(1), h.max(0) * d.max(1));
    let mut c = Canvas::new(cw as u32, ch as u32);
    for y in 0..ch {
        for x in 0..cw {
            c.put_raw(x, y, pack(f(f64::from(x), f64::from(y))));
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

    #[test]
    fn the_distance_out_of_a_rounded_rect_is_signed_and_follows_the_corner() {
        let d = |x, y| rounded_distance(x, y, 50.0, 30.0, 10.0);
        assert_eq!(d(25.0, -4.0), 4.0, "above the top edge");
        assert_eq!(d(56.0, 15.0), 6.0, "right of the right edge");
        assert_eq!(d(25.0, 5.0), -5.0, "inside, nearest the top");
        let out = 10.0 + 5.0;
        let diag = out / std::f64::consts::SQRT_2;
        assert!(
            (d(10.0 - diag, 10.0 - diag) - 5.0).abs() < 1e-9,
            "round the corner"
        );
    }

    #[test]
    fn white_at_half_over_white_is_white() {
        let p = over(premul(WHITE, 128), premul(WHITE, 255));
        assert_eq!(pack(p), 0xFFFF_FFFF);
    }

    #[test]
    fn a_white_rounded_rect_over_white_leaves_only_white() {
        let (w, h, d) = (21, 13, 2);
        let k = f64::from(d);
        let c = draw(w, h, d, |x, y| {
            let cov = rounded(x, y, f64::from(w) * k, f64::from(h) * k, 5.0 * k);
            over(scale(premul(WHITE, 255), cov), premul(WHITE, 255))
        });
        assert!(c.pixels.iter().all(|&p| p == 0xFFFF_FFFF), "only white");
    }

    #[test]
    fn a_rim_and_a_fill_sharing_an_edge_pixel_leave_no_dark_fringe() {
        // A white fill inside a white rim, the curve's pixels split
        // between them: every pixel the rect covers whole is white.
        let (w, h, d) = (21.0, 13.0, 2);
        let k = f64::from(d);
        let white = premul(WHITE, 255);
        let c = draw(21, 13, d, |x, y| {
            let outer = rounded(x, y, w * k, h * k, 4.0 * k);
            let inner = rounded(x - k, y - k, (w - 2.0) * k, (h - 2.0) * k, 3.0 * k);
            shares(&[(white, outer - inner), (white, inner)])
        });
        for (i, &p) in c.pixels.iter().enumerate() {
            let a = p >> 24;
            assert_eq!(
                p,
                (a << 24) | (a << 16) | (a << 8) | a,
                "pixel {i} is white at {a}"
            );
        }
    }

    fn face(d: i32) -> Canvas {
        let rim = premul(Color::rgb(0x1C, 0x51, 0x80), 255);
        let fill = [
            (0.0, premul(Color::rgb(0xFF, 0xFF, 0xFF), 255)),
            (1.0, premul(Color::rgb(0xD8, 0xD4, 0xCC), 255)),
        ];
        let px = ringed(75, 23, 3.0, 1, d, |_, y, ring| match ring {
            0 => rim,
            _ => Stops(&fill).at((y - 1.0) / 20.0),
        });
        draw(75, 23, d, |x, y| {
            px[(y as i32 * 75 * d + x as i32) as usize]
        })
    }

    #[test]
    fn the_button_face_has_round_corners_a_rim_and_a_shaded_face() {
        for d in [1, 2] {
            let c = face(d);
            assert_eq!((c.width, c.height), (75 * d as u32, 23 * d as u32));
            // The curve only grazes the corner pixel.
            assert!(c.get(0, 0) >> 24 < 8, "the corner is all but clear at {d}");
            assert_eq!(c.get(37 * d, 0), 0xFF1C_5180, "the rim at {d}");
            assert_eq!(c.get(37 * d, d), 0xFFFF_FFFF, "the face's top row at {d}");
            assert_eq!(
                c.get(37 * d, 21 * d + d - 1),
                0xFFD8_D4CC,
                "its bottom at {d}"
            );
        }
        let two = face(2);
        assert!(
            (0..6).any(|i| (1..255).contains(&(two.get(i, 1) >> 24))),
            "the corner's curve is smooth at 2"
        );
    }

    #[test]
    fn draw_at_2_is_twice_as_wide_and_tall() {
        let c = draw(7, 5, 2, |_, _| CLEAR);
        assert_eq!((c.width, c.height), (14, 10));
        assert_eq!(c.scale(), 1);
    }

    #[test]
    fn centre_at_1_is_the_pixel_and_at_2_splits_it() {
        assert_eq!(centre(3, 1), 3.0);
        assert_eq!((centre(0, 2), centre(1, 2)), (-0.25, 0.25));
    }

    #[test]
    fn pack_clamps_channels_to_alpha_and_rounds() {
        assert_eq!(pack((127.6, [200.0, 64.4, -3.0])), 0x8080_4000);
        assert_eq!(pack(unpack(0x80_40_20_10)), 0x80_40_20_10);
    }

    #[test]
    fn scale_then_pack_rounds_each_channel_alone() {
        // Scaling a packed pixel and packing it again rounds every channel
        // on its own, which never lifts a channel past the alpha.
        for px in [0xFF_80_40_20u32, 0x7F_7F_10_00, 0x01_01_01_01] {
            for k in [0.0, 0.3, 0.5, 0.77, 1.0] {
                let ch = |sh: u32| ((f64::from((px >> sh) & 0xFF) * k).round() as u32) << sh;
                let want = ch(24) | ch(16) | ch(8) | ch(0);
                assert_eq!(pack(scale(unpack(px), k)), want, "{px:08X} at {k}");
            }
        }
    }

    #[test]
    fn smooth_eases_from_0_to_1() {
        assert_eq!((smooth(-1.0), smooth(0.5), smooth(2.0)), (0.0, 0.5, 1.0));
    }
}
