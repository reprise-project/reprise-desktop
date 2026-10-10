//! Reflection streaks: diagonal bands of light fixed to the screen, drawn into the blurred backdrop under the glass art.
//! Only Frosted glass has them. The profile runs along `u = x - STREAK_SLOPE * y`; [`LIGHT`] and [`SHADE`] hold the stops.

use super::{Material, Settings};

pub const STREAK_SLOPE: f64 = 0.767;

/// The bands of light: (u, brightness in levels), u rising. A band
/// brightens what lies under it by its level and lets
/// `1 - LIGHT_ALPHA * level / 255` of it through. No light before the first
/// stop or after the last.
#[rustfmt::skip]
pub const LIGHT: &[(f64, f64)] = &[
    (-150.0, 0.0), (-100.0, 11.0), (-60.0, 14.0), (-10.0, 15.0), (60.0, 0.0),
    (145.0, 0.0), (205.0, 14.0), (220.0, 14.0), (235.0, 0.0),
    (415.0, 0.0), (440.0, 15.0), (515.0, 14.0), (545.0, 0.0),
    (790.0, 0.0), (850.0, 12.0), (900.0, 11.0), (1030.0, 21.0), (1060.0, 19.0), (1130.0, 0.0),
    (1170.0, 0.0), (1185.0, 15.0), (1235.0, 0.0),
    (1265.0, 0.0), (1295.0, 17.0), (1330.0, 20.0), (1350.0, 16.0), (1410.0, 15.0), (1440.0, 0.0),
    (1510.0, 0.0), (1520.0, 5.0), (1530.0, 0.0),
];

pub const LIGHT_ALPHA: f64 = 0.4;

/// The faint shadows beside the bands: (u, alpha of black in levels), as
/// [`LIGHT`].
#[rustfmt::skip]
pub const SHADE: &[(f64, f64)] = &[
    (-195.0, 0.0), (-180.0, 2.0), (-160.0, 0.0),
    (350.0, 0.0), (400.0, 6.0), (420.0, 0.0),
    (540.0, 0.0), (560.0, 4.0), (600.0, 4.0), (620.0, 0.0),
    (1125.0, 0.0), (1150.0, 10.0), (1170.0, 0.0),
    (1230.0, 0.0), (1250.0, 9.0), (1268.0, 0.0),
    (1435.0, 0.0), (1450.0, 3.0), (1470.0, 0.0),
];

fn along(stops: &[(f64, f64)], u: f64) -> f64 {
    stops
        .windows(2)
        .find(|w| (w[0].0..=w[1].0).contains(&u))
        .map_or(0.0, |w| {
            let ((u0, v0), (u1, v1)) = (w[0], w[1]);
            if u1 > u0 {
                v0 + (v1 - v0) * (u - u0) / (u1 - u0)
            } else {
                v0.max(v1)
            }
        })
}

/// The streak layer at `u` along the diagonal (`x - STREAK_SLOPE * y`,
/// screen-local logical pixels): premultiplied ARGB, white at the band's
/// brightness over black at the shadow's alpha. It only brightens where a
/// band lies, so its colour may exceed its alpha.
pub fn at(u: f64) -> u32 {
    let light = along(LIGHT, u);
    let shade = along(SHADE, u);
    let a = (LIGHT_ALPHA * light + shade).round().clamp(0.0, 255.0) as u32;
    let c = light.round().clamp(0.0, 255.0) as u32;
    a << 24 | c << 16 | c << 8 | c
}

/// The streaks for a picture of `px` pixels that covers its screen at
/// `scale` picture pixels per logical pixel: premultiplied ARGB rows, each
/// pixel the layer at its centre. None unless [`shown`].
pub fn pixels(s: &Settings, px: (i32, i32), scale: f32) -> Option<Vec<u32>> {
    if !shown(s) {
        return None;
    }
    let (w, h) = (px.0.max(0) as usize, px.1.max(0) as usize);
    let step = 1.0 / f64::from(scale);
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = -STREAK_SLOPE * (y as f64 + 0.5) * step;
        out.extend((0..w).map(|x| at((x as f64 + 0.5) * step + row)));
    }
    Some(out)
}

/// Whether `s`'s glass shows the streaks: Frosted glass that lets the
/// backdrop through, with `[glass] streaks` on.
pub fn shown(s: &Settings) -> bool {
    s.transparency && s.streaks && s.material == Material::Frosted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(px: u32) -> [i32; 4] {
        [24, 16, 8, 0].map(|s| ((px >> s) & 0xFF) as i32)
    }

    #[test]
    fn the_stops_rise_along_the_diagonal() {
        for stops in [LIGHT, SHADE] {
            assert!(stops.windows(2).all(|w| w[0].0 < w[1].0), "u rises");
            assert_eq!(stops[0].1, 0.0, "a run starts dark");
            assert_eq!(stops[stops.len() - 1].1, 0.0, "and ends dark");
        }
    }

    #[test]
    fn a_band_brightens_and_a_shadow_darkens() {
        let [a, r, g, b] = channels(at(1030.0));
        assert_eq!((r, g, b), (21, 21, 21), "white at the band's level");
        assert_eq!(a, 8, "covering 0.4 of its level");
        let [a, r, ..] = channels(at(1150.0));
        assert_eq!((a, r), (10, 0), "black at the shadow's alpha");
        assert_eq!(channels(at(175.0))[1], 7, "straight between the stops");
    }

    #[test]
    fn past_the_stops_there_is_no_streak() {
        assert_eq!(at(-500.0), 0);
        assert_eq!(at(700.0), 0, "between bands");
        assert_eq!(at(1600.0), 0);
        assert_eq!(at(LIGHT[0].0), 0, "the first stop is dark");
    }

    #[test]
    fn a_1920x1080_screen_keeps_its_streaks() {
        let p = pixels(&Settings::default(), (960, 540), 0.5).expect("translucent glass");
        assert_eq!(p.len(), 960 * 540);
        let hash = p
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        assert_eq!(
            hash, 0x12c7_6765_8456_67c1,
            "the streaks of a 1920x1080 screen"
        );
        assert!(
            p.iter().any(|&v| channels(v)[1] >= 20),
            "a bright band crosses it"
        );
        assert!(p.contains(&0), "with clear glass between");
    }

    #[test]
    fn the_streaks_are_smooth_at_scale_2() {
        let two = pixels(&Settings::default(), (800, 600), 2.0).unwrap();
        for row in two.chunks(800).step_by(37) {
            for w in row.windows(2) {
                let off = channels(w[0])
                    .iter()
                    .zip(channels(w[1]))
                    .map(|(a, b)| (a - b).abs())
                    .max()
                    .unwrap();
                assert!(off <= 1, "{:#010x} next to {:#010x}", w[0], w[1]);
            }
        }
    }

    #[test]
    fn the_streaks_stay_on_the_screen_at_any_scale() {
        let one = pixels(&Settings::default(), (400, 300), 1.0).unwrap();
        let two = pixels(&Settings::default(), (800, 600), 2.0).unwrap();
        let (i, j) = (250usize, 120usize);
        let near = channels(one[j * 400 + i])
            .iter()
            .zip(channels(two[(2 * j + 1) * 800 + 2 * i + 1]))
            .map(|(a, b)| (a - b).abs())
            .max()
            .unwrap();
        assert!(near <= 2, "the same logical point, within 2 levels");
    }

    #[test]
    fn smoked_and_molten_glass_build_no_streaks() {
        for material in [Material::Smoked, Material::Molten] {
            let s = Settings {
                material,
                ..Settings::default()
            };
            assert_eq!(pixels(&s, (960, 540), 0.5), None, "{material:?}");
        }
    }

    #[test]
    fn opaque_glass_builds_no_streaks() {
        let opaque = Settings {
            transparency: false,
            ..Settings::default()
        };
        assert_eq!(pixels(&opaque, (960, 540), 0.5), None);
    }

    #[test]
    fn streaks_off_builds_no_streaks() {
        let off = Settings {
            streaks: false,
            ..Settings::default()
        };
        assert_eq!(pixels(&off, (960, 540), 0.5), None);
    }
}
