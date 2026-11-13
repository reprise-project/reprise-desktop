//! The rings that show where the pointer is: two rings grow out from it one after the other and
//! fade, in the look's highlight colour edged in white so they read on any background.

use crate::raster::Canvas;
use crate::shape::{self, CLEAR, Px, premul};
use crate::theme::Color;

/// The square the rings fill, design pixels; the pointer sits at its middle.
pub const SIZE: i32 = 88;

const R_MIN: f64 = 6.0;
const R_MAX: f64 = 40.0;
/// Half the coloured band's width, and the white edge on each side of it.
const BAND: f64 = 1.5;
const EDGE: f64 = 1.0;
/// When each ring starts, as a share of the whole animation, and the share it takes to grow.
const STARTS: [f64; 2] = [0.0, 0.3];
const GROW: f64 = 0.7;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

/// Each ring shown `t` (0 to 1) into the animation: its radius and its opacity.
fn shown(t: f64) -> Vec<(f64, f64)> {
    STARTS
        .iter()
        .filter_map(|s| {
            let p = (t - s) / GROW;
            (0.0..1.0).contains(&p).then(|| {
                let eased = 1.0 - (1.0 - p).powi(2);
                (R_MIN + (R_MAX - R_MIN) * eased, 1.0 - p)
            })
        })
        .collect()
}

/// The rings `t` (0 to 1) into the animation in `accent`, [`SIZE`] square painted at `d`.
pub fn rings(t: f64, accent: Color, d: i32) -> Canvas {
    let rings = shown(t);
    let k = f64::from(d.max(1));
    let mid = f64::from(SIZE) * k / 2.0;
    let (white, ink) = (premul(WHITE, 0xFF), premul(accent, 0xFF));
    shape::draw(SIZE, SIZE, d, |x, y| {
        let disc = |r: f64| shape::ellipse(x, y, mid, mid, r * k, r * k);
        rings.iter().fold(CLEAR, |under, &(r, opacity)| {
            let edge_out = disc(r + BAND + EDGE);
            let band_out = disc(r + BAND);
            let band_in = disc(r - BAND);
            let edge_in = disc(r - BAND - EDGE);
            let ring: Px = shape::shares(&[
                (white, edge_out - band_out),
                (ink, band_out - band_in),
                (white, band_in - edge_in),
            ]);
            shape::over(shape::scale(ring, opacity), under)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;

    const ACCENT: Color = Color::rgb(0x31, 0x6A, 0xC5);

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

    #[test]
    fn the_pointer_and_the_corners_stay_clear() {
        for d in [1, 2] {
            for t in [0.0, 0.2, 0.5, 0.9] {
                let c = rings(t, ACCENT, d);
                let n = SIZE * d;
                assert_eq!((c.width, c.height), (n as u32, n as u32));
                assert_eq!(c.get(n / 2, n / 2), 0, "the pointer at {t}");
                for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                    assert_eq!(c.get(x, y), 0, "corner at {t}");
                }
            }
        }
    }

    #[test]
    fn the_first_ring_starts_small_and_opaque_with_white_edges() {
        let c = rings(0.0, ACCENT, 1);
        let mid = SIZE / 2;
        assert_eq!(c.get(mid + R_MIN as i32, mid), ACCENT.argb(), "the band");
        let edge = c.get(mid + R_MIN as i32 + 1, mid);
        assert_eq!(edge >> 24, 0xFF, "the band meets its edge: {edge:08x}");
        assert!((edge >> 16 & 0xFF) > 0x31, "white in it: {edge:08x}");
        assert_eq!(c.get(mid + R_MIN as i32 + 3, mid), 0, "outside the ring");
        assert_eq!(c.get(mid, mid), 0, "inside it");
    }

    #[test]
    fn the_rings_grow_fade_and_are_gone_at_the_end() {
        assert_eq!(shown(0.0).len(), 1, "the second ring waits");
        let mid = shown(0.5);
        assert_eq!(mid.len(), 2);
        assert!(mid[0].0 > mid[1].0, "the first ring is further out");
        assert!(mid[0].1 < mid[1].1, "and fainter");
        assert!(shown(1.0).is_empty());
        let empty = rings(1.0, ACCENT, 1);
        assert!(empty.pixels.iter().all(|&p| p == 0));
        assert_ne!(fnv(&rings(0.2, ACCENT, 1)), fnv(&rings(0.5, ACCENT, 1)));
    }

    #[test]
    fn at_scale_2_the_curves_are_drawn_again_with_soft_edges() {
        let c = rings(0.4, ACCENT, 2);
        let partial = c
            .pixels
            .iter()
            .filter(|&&p| (1..255).contains(&(p >> 24)))
            .count();
        assert!(partial > 100, "{partial} soft pixels");
        let one = rings(0.4, ACCENT, 1);
        let doubled = (0..one.height as i32)
            .all(|y| (0..one.width as i32).all(|x| c.get(2 * x, 2 * y) == one.get(x, y)));
        assert!(!doubled, "not scale 1 doubled");
    }

    #[test]
    fn rings_pin_under_every_interface_at_scale_1_and_2() {
        let mut themes = vec![
            (
                "Classic".to_string(),
                interfaces::theme_for(Interface::Classic),
            ),
            ("Glass".to_string(), interfaces::theme_for(Interface::Glass)),
        ];
        themes.extend(
            crate::blue::Variant::ALL
                .into_iter()
                .map(|v| (format!("Blue {v:?}"), interfaces::blue_theme(v))),
        );
        let mut got = Vec::new();
        for (name, theme) in &themes {
            for t in [0.1, 0.4, 0.8] {
                for d in [1, 2] {
                    let c = rings(t, theme.colors.hilight, d);
                    got.push((format!("{name} {t} {d}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("locate_rings", &got);
    }
}
