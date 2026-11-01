//! The Caps Lock and Num Lock icon, 32x32: a key cap with an arrow (Caps) or a 1 (Num) on its
//! face and a lamp in the corner, lit while the lock is on.

use crate::raster::Canvas;
use crate::shape::{self, Px, Stops, centre, ellipse, polygon, premul, rounded};
use crate::theme::Color;

pub const SIZE: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockKey {
    Caps,
    Num,
}

const CAP: (f64, f64, f64, f64, f64) = (3.0, 3.0, 26.0, 26.0, 4.0);
/// The top face, inset in the cap; the cap shows below it as the key's skirt.
const FACE: (f64, f64, f64, f64, f64) = (5.0, 4.0, 22.0, 20.0, 3.0);
const LAMP: (f64, f64, f64) = (23.5, 7.5, 2.0);

const OUTLINE: Color = Color::rgb(0x4c, 0x4e, 0x58);
const SKIRT: Color = Color::rgb(0xb8, 0xba, 0xc4);
const FACE_TOP: Color = Color::rgb(0xfc, 0xfc, 0xfe);
const FACE_LOW: Color = Color::rgb(0xdc, 0xde, 0xe6);
const INK: Color = Color::rgb(0x2c, 0x2e, 0x38);
const LAMP_ON: Color = Color::rgb(0x34, 0xc8, 0x3c);
const LAMP_ON_CORE: Color = Color::rgb(0xb4, 0xf4, 0xa8);
const LAMP_OFF: Color = Color::rgb(0x88, 0x8a, 0x94);

const CAPS_ARROW: [(f64, f64); 7] = [
    (16.0, 7.0),
    (23.0, 14.0),
    (19.0, 14.0),
    (19.0, 18.0),
    (13.0, 18.0),
    (13.0, 14.0),
    (9.0, 14.0),
];
const CAPS_BAR: [(f64, f64); 4] = [(13.0, 20.0), (19.0, 20.0), (19.0, 22.0), (13.0, 22.0)];
const ONE_STEM: [(f64, f64); 4] = [(15.0, 7.0), (18.0, 7.0), (18.0, 20.0), (15.0, 20.0)];
const ONE_FLAG: [(f64, f64); 4] = [(15.0, 7.0), (15.0, 10.5), (12.0, 12.0), (12.0, 9.5)];
const ONE_BASE: [(f64, f64); 4] = [(12.0, 19.5), (21.0, 19.5), (21.0, 22.0), (12.0, 22.0)];

/// The icon for `key`, lamp lit while `on`, at `d` device pixels per design pixel.
pub fn draw(key: LockKey, on: bool, d: i32) -> Canvas {
    let k = f64::from(d);
    let scaled = |pts: &[(f64, f64)]| -> Vec<(f64, f64)> {
        pts.iter().map(|&(x, y)| (x * k, y * k)).collect()
    };
    let glyph: Vec<Vec<(f64, f64)>> = match key {
        LockKey::Caps => vec![scaled(&CAPS_ARROW), scaled(&CAPS_BAR)],
        LockKey::Num => vec![scaled(&ONE_STEM), scaled(&ONE_FLAG), scaled(&ONE_BASE)],
    };
    let face = Stops(&[
        (FACE.1, premul(FACE_TOP, 255)),
        (FACE.1 + FACE.3, premul(FACE_LOW, 255)),
    ]);
    let lamp = if on {
        Stops(&[
            (0.0, premul(LAMP_ON_CORE, 255)),
            (0.7, premul(LAMP_ON, 255)),
        ])
    } else {
        Stops(&[(0.0, premul(LAMP_OFF, 255))])
    };
    let cover = |x: f64, y: f64, (bx, by, w, h, r): (f64, f64, f64, f64, f64)| {
        rounded(x - bx * k, y - by * k, w * k, h * k, r * k)
    };
    shape::draw(SIZE, SIZE, d, |x, y| {
        let cap = cover(x, y, CAP);
        if cap == 0.0 {
            return shape::CLEAR;
        }
        let inner = cover(
            x,
            y,
            (
                CAP.0 + 1.0,
                CAP.1 + 1.0,
                CAP.2 - 2.0,
                CAP.3 - 2.0,
                CAP.4 - 1.0,
            ),
        );
        let face_cover = cover(x, y, FACE);
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        let mut px: Px = shape::mix(premul(SKIRT, 255), face.at(cy + 0.5), face_cover);
        let ink = glyph.iter().map(|p| polygon(x, y, p)).fold(0.0, f64::max);
        px = shape::mix(px, premul(INK, 255), ink);
        let (lx, ly, lr) = LAMP;
        let lamp_cover = ellipse(x, y, lx * k, ly * k, lr * k, lr * k);
        let t = (cx + 0.5 - lx).hypot(cy + 0.5 - ly) / lr;
        px = shape::mix(px, lamp.at(t), lamp_cover);
        let px = shape::mix(premul(OUTLINE, 255), px, inner);
        shape::scale(px, cap)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for y in 0..c.height as i32 {
            for x in 0..c.width as i32 {
                for b in c.get(x, y).to_le_bytes() {
                    h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    fn rgb(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(x, y) & 0xFF_FFFF
    }

    #[test]
    fn the_cap_is_clear_round_its_corners_and_outlined() {
        for d in [1, 2] {
            let c = draw(LockKey::Caps, true, d);
            assert_eq!((c.width, c.height), (32 * d as u32, 32 * d as u32));
            assert_eq!(c.get(0, 0), 0, "outside the cap");
            assert_eq!(c.get(3 * d, 3 * d) >> 24, 0, "the rounded corner");
            assert_eq!(c.get(31 * d, 31 * d), 0);
            assert_eq!(c.get(16 * d, 3 * d), OUTLINE.argb());
        }
    }

    #[test]
    fn the_lamp_lights_with_the_lock_and_the_glyph_names_the_key() {
        let lamp = |c: &Canvas| rgb(c, 23, 7);
        let (on, off) = (draw(LockKey::Caps, true, 1), draw(LockKey::Caps, false, 1));
        assert_ne!(lamp(&on), lamp(&off));
        assert_eq!(lamp(&off), LAMP_OFF.argb() & 0xFF_FFFF);
        let g = (lamp(&on) >> 8) & 0xFF;
        assert!(g > 0xC0, "a green lamp: {:#08x}", lamp(&on));
        assert_eq!(rgb(&on, 16, 16), INK.argb() & 0xFF_FFFF, "the arrow's stem");
        let num = draw(LockKey::Num, true, 1);
        assert_ne!(
            rgb(&num, 10, 13),
            INK.argb() & 0xFF_FFFF,
            "a 1 has no arrow head"
        );
        assert_eq!(rgb(&on, 10, 13), INK.argb() & 0xFF_FFFF);
        assert_eq!(rgb(&num, 16, 15), INK.argb() & 0xFF_FFFF, "the 1's stem");
    }

    #[test]
    fn curved_edges_are_soft_at_scale_2() {
        let c = draw(LockKey::Num, false, 2);
        let partial = (6..14).any(|i| {
            let a = c.get(i, i) >> 24;
            a > 0 && a < 0xFF
        });
        assert!(partial, "partial alpha round the cap's corner");
    }

    #[test]
    fn pins_each_state_at_scale_1_and_2() {
        let got: Vec<(LockKey, bool, i32, u64)> = [LockKey::Caps, LockKey::Num]
            .into_iter()
            .flat_map(|k| [true, false].map(|on| (k, on)))
            .flat_map(|(k, on)| [1, 2].map(|d| (k, on, d, fnv(&draw(k, on, d)))))
            .collect();
        assert_eq!(got, PINS, "pins changed; now:\n{got:#x?}");
    }

    const PINS: [(LockKey, bool, i32, u64); 8] = [
        (LockKey::Caps, true, 1, 0xe50a23bf2a160491),
        (LockKey::Caps, true, 2, 0x8d37bb93786938ca),
        (LockKey::Caps, false, 1, 0x27bcf8ff37ce2d25),
        (LockKey::Caps, false, 2, 0x3676155bcd26719e),
        (LockKey::Num, true, 1, 0x9369ab95b2109df6),
        (LockKey::Num, true, 2, 0x2cd4f584dfc7af38),
        (LockKey::Num, false, 1, 0xb2982645b04c39a),
        (LockKey::Num, false, 2, 0xe09e0aad50cfa8ac),
    ];
}
