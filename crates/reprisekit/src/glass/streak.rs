//! Reflection streaks: diagonal bands of light fixed to the screen. Only Frosted glass has them.
//! Glass draws them into the blurred backdrop under the glass art, along `u = x - STREAK_SLOPE * y`
//! ([`LIGHT`], [`SHADE`]). Onyx lays its stronger bands over the glass ([`over`]), along
//! `u = x - ONYX_SLOPE * y` ([`ONYX_LIGHT`]): light where a band lies and a dimming where it has
//! faded, so they show over a white backdrop too, where the lifted glass has no light to gain.

use super::{Material, Settings, Style};

pub const STREAK_SLOPE: f64 = 0.767;
pub const ONYX_SLOPE: f64 = 0.738;

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

/// Onyx's bands: (u, level), u rising, up to [`ONYX_PEAK`]. Each rises sharply at its edge, holds,
/// and fades towards the next.
#[rustfmt::skip]
pub const ONYX_LIGHT: &[(f64, f64)] = &[
    (-830.0, 0.0), (-815.0, 12.0), (-700.0, 12.0), (-600.0, 2.0), (-585.0, 14.0), (-520.0, 13.0),
    (-470.0, 11.0), (-400.0, 8.0), (-372.0, 13.0), (-330.0, 13.0), (-196.0, 2.0),
    (-182.0, 15.0), (-130.0, 16.0), (-30.0, 16.0), (110.0, 0.0),
    (128.0, 17.0), (180.0, 20.0), (192.0, 20.0), (200.0, 13.0), (225.0, 12.0), (270.0, 19.0),
    (290.0, 17.0), (330.0, 19.0), (420.0, 15.0), (520.0, 1.0),
    (532.0, 16.0), (640.0, 17.0), (790.0, 0.0),
    (802.0, 18.0), (900.0, 19.0), (1080.0, 2.0),
    (1092.0, 14.0), (1190.0, 15.0), (1330.0, 0.0),
    (1342.0, 19.0), (1460.0, 18.0), (1640.0, 1.0),
    (1652.0, 13.0), (1760.0, 12.0), (1900.0, 0.0),
];

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
    Along::new(stops).at(u)
}

/// [`along`] for a run of `u`s that rarely falls: the window the last `u` lay in is kept, so a
/// rising run walks the stops once.
struct Along<'a> {
    stops: &'a [(f64, f64)],
    i: usize,
    last: f64,
}

impl<'a> Along<'a> {
    fn new(stops: &'a [(f64, f64)]) -> Along<'a> {
        Along {
            stops,
            i: 0,
            last: f64::NEG_INFINITY,
        }
    }

    fn at(&mut self, u: f64) -> f64 {
        if u.is_nan() {
            return 0.0;
        }
        if u < self.last {
            self.i = 0;
        }
        self.last = u;
        let s = self.stops;
        // The first window that ends at or past `u`: the one `u` lies in, or none.
        while self.i + 1 < s.len() && s[self.i + 1].0 < u {
            self.i += 1;
        }
        if self.i + 1 >= s.len() || u < s[self.i].0 {
            return 0.0;
        }
        let ((u0, v0), (u1, v1)) = (s[self.i], s[self.i + 1]);
        if u1 > u0 {
            v0 + (v1 - v0) * (u - u0) / (u1 - u0)
        } else {
            v0.max(v1)
        }
    }
}

/// The streak layer at `u` along the diagonal (`x - STREAK_SLOPE * y`,
/// screen-local logical pixels): premultiplied ARGB, white at the band's
/// brightness over black at the shadow's alpha. It only brightens where a
/// band lies, so its colour may exceed its alpha.
pub fn at(u: f64) -> u32 {
    layer(along(LIGHT, u), along(SHADE, u))
}

fn layer(light: f64, shade: f64) -> u32 {
    let a = (LIGHT_ALPHA * light + shade).round().clamp(0.0, 255.0) as u32;
    let c = light.round().clamp(0.0, 255.0) as u32;
    a << 24 | c << 16 | c << 8 | c
}

/// The brightest of [`ONYX_LIGHT`]'s levels: glass there is not dimmed.
pub const ONYX_PEAK: f64 = 20.0;

/// Onyx's layer at `u` along its diagonal, premultiplied: [`ONYX_LIGHT_SHARE`] of the band's
/// level as white light, over black covering as many levels as the band lies below
/// [`ONYX_PEAK`]. It only brightens at the peak, so its colour may exceed its alpha.
pub fn onyx_at(u: f64) -> u32 {
    onyx_layer(along(ONYX_LIGHT, u))
}

pub const ONYX_LIGHT_SHARE: f64 = 0.5;

/// [`ONYX_LIGHT`]'s level at `u` along its diagonal, 0 to [`ONYX_PEAK`].
pub fn onyx_level(u: f64) -> f64 {
    along(ONYX_LIGHT, u)
}

fn onyx_layer(level: f64) -> u32 {
    let a = (ONYX_PEAK - level).round().clamp(0.0, 255.0) as u32;
    let c = (ONYX_LIGHT_SHARE * level).round().clamp(0.0, 255.0) as u32;
    a << 24 | c << 16 | c << 8 | c
}

/// [`picture`] for `s`'s style, None unless [`shown`].
pub fn pixels(s: &Settings, px: (i32, i32), scale: f32) -> Option<Vec<u32>> {
    shown(s).then(|| picture(s.style, px, scale))
}

/// `style`'s streaks for a picture of `px` pixels that covers its screen
/// at `scale` picture pixels per logical pixel: premultiplied ARGB rows,
/// each pixel the layer at its centre.
pub fn picture(style: Style, px: (i32, i32), scale: f32) -> Vec<u32> {
    let (w, h) = (px.0.max(0) as usize, px.1.max(0) as usize);
    let step = 1.0 / f64::from(scale);
    let slope = match style {
        Style::Glass => STREAK_SLOPE,
        Style::Onyx => ONYX_SLOPE,
    };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = -slope * (y as f64 + 0.5) * step;
        let u = |x: usize| (x as f64 + 0.5) * step + row;
        match style {
            Style::Glass => {
                let (mut light, mut shade) = (Along::new(LIGHT), Along::new(SHADE));
                out.extend((0..w).map(|x| layer(light.at(u(x)), shade.at(u(x)))));
            }
            Style::Onyx => {
                let mut light = Along::new(ONYX_LIGHT);
                out.extend((0..w).map(|x| onyx_layer(light.at(u(x)))));
            }
        }
    }
    out
}

/// Whether `s`'s streaks lie over the glass as light, after the backdrop is lifted for the
/// chrome, rather than in the backdrop under it.
pub fn over(s: &Settings) -> bool {
    s.style == Style::Onyx
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
        for stops in [LIGHT, SHADE, ONYX_LIGHT] {
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
    fn onyxs_bands_light_at_their_peak_dim_where_faded_and_rise_sharply() {
        let [a, r, g, b] = channels(onyx_at(180.0));
        assert_eq!(
            (a, r, g, b),
            (0, 10, 10, 10),
            "half the peak's level as light, nothing covered"
        );
        let [a, r, ..] = channels(onyx_at(110.0));
        assert_eq!((a, r), (20, 0), "a faded band dims by the peak's level");
        let rise = channels(onyx_at(110.0))[0] - channels(onyx_at(128.0))[0];
        assert_eq!(rise, 17, "from dim to the band within 18 px");
        assert!(
            channels(onyx_at(60.0))[0] > channels(onyx_at(-30.0))[0],
            "fading towards the next edge"
        );
        let peak = ONYX_LIGHT.iter().map(|s| s.1).fold(0.0, f64::max);
        assert_eq!(peak, ONYX_PEAK);
        let (glass, onyx) = (LIGHT, ONYX_LIGHT);
        let mean = |stops: &[(f64, f64)]| {
            (-800..1900)
                .map(|u| along(stops, f64::from(u)))
                .sum::<f64>()
                / 2700.0
        };
        assert!(mean(onyx) > 2.0 * mean(glass), "stronger than Glass's");
    }

    #[test]
    fn onyx_lays_its_streaks_over_the_glass_and_glass_under_it() {
        let onyx = Settings {
            style: Style::Onyx,
            ..Settings::default()
        };
        assert!(over(&onyx) && shown(&onyx));
        assert!(!over(&Settings::default()));
    }

    #[test]
    fn an_onyx_screen_keeps_its_streaks_along_its_own_slope() {
        let onyx = Settings {
            style: Style::Onyx,
            ..Settings::default()
        };
        for (scale, px) in [(0.5, (960, 540)), (2.0, (640, 90))] {
            let p = pixels(&onyx, px, scale).expect("translucent glass");
            let step = 1.0 / f64::from(scale);
            for (i, v) in p.iter().enumerate().step_by(97) {
                let (x, y) = ((i % px.0 as usize) as f64, (i / px.0 as usize) as f64);
                let u = (x + 0.5) * step - ONYX_SLOPE * (y + 0.5) * step;
                assert_eq!(*v, onyx_at(u), "pixel {i} at scale {scale}");
            }
        }
        let p = pixels(&onyx, (960, 540), 0.5).unwrap();
        let hash = p
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        crate::pins::check("glass/onyx_streaks_1080p", &[("1920x1080".into(), hash)]);
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
        crate::pins::check("glass/streaks_1080p", &[("1920x1080".into(), hash)]);
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
    fn every_pixel_is_the_layer_at_its_centre() {
        for (scale, px) in [(0.5, (300, 200)), (1.25, (257, 131)), (2.0, (640, 90))] {
            let p = pixels(&Settings::default(), px, scale).unwrap();
            let step = 1.0 / f64::from(scale);
            for (i, v) in p.iter().enumerate() {
                let (x, y) = ((i % px.0 as usize) as f64, (i / px.0 as usize) as f64);
                let u = (x + 0.5) * step - STREAK_SLOPE * (y + 0.5) * step;
                assert_eq!(*v, at(u), "pixel {i} at scale {scale}");
            }
        }
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
