//! The white point of a colour temperature and the gamma ramp that shifts the
//! screen to it, on top of a profile's calibration curves. The curve: illuminant D from 4000 K up, the Planckian locus
//! below 2500 K, a cosine blend between, and 6500 K exactly white.

use std::f64::consts::PI;

use reprisefs::icc::Vcgt;

pub const NEUTRAL: u32 = 6500;

/// CIE xy on the daylight locus; valid 2500..=25000 K.
fn illuminant_d(t: f64) -> (f64, f64) {
    let x = if t <= 7000.0 {
        0.244063 + 0.09911e3 / t + 2.9678e6 / t.powi(2) - 4.6070e9 / t.powi(3)
    } else {
        0.237040 + 0.24748e3 / t + 1.9018e6 / t.powi(2) - 2.0064e9 / t.powi(3)
    };
    (x, -3.0 * x * x + 2.870 * x - 0.275)
}

/// CIE xy on the Planckian locus; valid 1667..25000 K.
fn planckian(t: f64) -> (f64, f64) {
    if t <= 4000.0 {
        let x = -0.2661239e9 / t.powi(3) - 0.2343589e6 / t.powi(2) + 0.8776956e3 / t + 0.179910;
        let y = if t <= 2222.0 {
            -1.1064814 * x.powi(3) - 1.34811020 * x.powi(2) + 2.18555832 * x - 0.20219683
        } else {
            -0.9549476 * x.powi(3) - 1.37418593 * x.powi(2) + 2.09137015 * x - 0.16748867
        };
        (x, y)
    } else {
        let x = -3.0258469e9 / t.powi(3) + 2.1070379e6 / t.powi(2) + 0.2226347e3 / t + 0.240390;
        (
            x,
            3.0817580 * x.powi(3) - 5.87338670 * x.powi(2) + 3.75112997 * x - 0.37001483,
        )
    }
}

/// The relative red, green and blue gains that make `kelvin` the white point, the largest being 1.
pub fn whitepoint(kelvin: u32) -> [f64; 3] {
    if kelvin == NEUTRAL {
        return [1.0; 3];
    }
    let t = kelvin as f64;
    let (x, y) = if kelvin >= 25000 {
        illuminant_d(25000.0)
    } else if kelvin >= 4000 {
        illuminant_d(t)
    } else if kelvin >= 2500 {
        let (x1, y1) = illuminant_d(t);
        let (x2, y2) = planckian(t);
        let f = (4000.0 - t) / 1500.0;
        let s = ((PI * f).cos() + 1.0) / 2.0;
        (x1 * s + x2 * (1.0 - s), y1 * s + y2 * (1.0 - s))
    } else {
        planckian(t.max(1667.0))
    };
    let z = 1.0 - x - y;
    let g = |v: f64| v.clamp(0.0, 1.0).powf(1.0 / 2.2);
    let rgb = [
        g(3.2404542 * x - 1.5371385 * y - 0.4985314 * z),
        g(-0.9692660 * x + 1.8760108 * y + 0.0415560 * z),
        g(0.0556434 * x - 0.2040259 * y + 1.0572252 * z),
    ];
    let max = rgb.iter().cloned().fold(f64::MIN, f64::max);
    rgb.map(|v| v / max)
}

/// A gamma table of `size` entries per channel (red, green, blue): each channel's calibration curve
/// (`curves`, else a straight line) scaled by `kelvin`'s gain. None when neither changes anything.
pub fn ramp(size: usize, kelvin: Option<u32>, curves: Option<&Vcgt>) -> Option<Vec<u16>> {
    if kelvin.is_none() && curves.is_none() {
        return None;
    }
    let wp = whitepoint(kelvin.unwrap_or(NEUTRAL));
    let mut out = Vec::with_capacity(size * 3);
    for (c, gain) in wp.into_iter().enumerate() {
        for i in 0..size {
            let x = if size > 1 {
                i as f64 / (size - 1) as f64
            } else {
                1.0
            };
            let v = curves.map_or(x, |k| k.value(c, x));
            out.push((u16::MAX as f64 * v * gain) as u16);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: [f64; 3], b: [f64; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.0001)
    }

    #[test]
    fn six_thousand_five_hundred_kelvin_is_white() {
        assert_eq!(whitepoint(6500), [1.0, 1.0, 1.0]);
        let r = ramp(256, Some(6500), None).unwrap();
        assert_eq!(r.len(), 768);
        assert_eq!((r[0], r[255], r[511], r[767]), (0, 65535, 65535, 65535));
    }

    /// The expected gains, to four places.
    #[test]
    fn warm_temperatures_keep_red_and_cut_blue() {
        for (k, want) in [
            (4000, [1.0, 0.8360, 0.6264]),
            (3400, [1.0, 0.7683, 0.5111]),
            (2500, [1.0, 0.6393, 0.2951]),
            (1900, [1.0, 0.5342, 0.0]),
        ] {
            let got = whitepoint(k);
            assert!(near(got, want), "{k} K: {got:?}, want {want:?}");
        }
        let blues: Vec<f64> = (25..=65).map(|h| whitepoint(h * 100)[2]).collect();
        assert!(blues.windows(2).all(|w| w[0] <= w[1] + 1e-9), "{blues:?}");
    }

    #[test]
    fn a_ramp_scales_each_channel_by_its_gain() {
        let r = ramp(4, Some(4000), None).unwrap();
        let wp = whitepoint(4000);
        assert_eq!(&r[0..4], &[0, 21845, 43690, 65535]);
        assert_eq!(r[7], (65535.0 * wp[1]) as u16);
        assert_eq!(r[11], (65535.0 * wp[2]) as u16);
        assert_eq!(r[8], 0);
    }

    fn curves() -> Vcgt {
        Vcgt::Table {
            entries: 2,
            data: vec![0.0, 0.8, 0.1, 1.0, 0.0, 0.5],
        }
    }

    #[test]
    fn no_warmth_and_no_profile_leave_the_table_alone() {
        assert_eq!(ramp(256, None, None), None);
    }

    #[test]
    fn a_profile_alone_puts_its_curves_in_the_table() {
        let r = ramp(3, None, Some(&curves())).unwrap();
        assert_eq!(&r[0..3], &[0, 26214, 52428], "red 0 to 0.8");
        assert_eq!(&r[3..6], &[6553, 36044, 65535], "green 0.1 to 1");
        assert_eq!(&r[6..9], &[0, 16383, 32767], "blue 0 to 0.5");
    }

    /// Night light scales the calibrated value, so a warm night keeps the profile's curve shape.
    #[test]
    fn night_light_scales_each_calibrated_channel_by_its_gain() {
        let k = 3400;
        let wp = whitepoint(k);
        let r = ramp(3, Some(k), Some(&curves())).unwrap();
        let c = curves();
        for ch in 0..3 {
            for i in 0..3 {
                let want = (65535.0 * c.value(ch, i as f64 / 2.0) * wp[ch]) as u16;
                assert_eq!(r[ch * 3 + i], want, "channel {ch} entry {i}");
            }
        }
    }
}
