//! Colour for the drawn wallpapers: float RGB, OKLab/OKLCH, ramps, and dithered quantising to 8
//! bits so gradients never band.

use super::noise::{hash_form, hash_land};
use super::par_rows;

pub type C = [f32; 3];

pub const fn hex(c: u32) -> C {
    [
        ((c >> 16) & 255) as f32 / 255.0,
        ((c >> 8) & 255) as f32 / 255.0,
        (c & 255) as f32 / 255.0,
    ]
}

#[inline]
pub fn mix(a: C, b: C, t: f32) -> C {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[inline]
pub fn add(a: C, b: C) -> C {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub fn mul(a: C, k: f32) -> C {
    [a[0] * k, a[1] * k, a[2] * k]
}

#[inline]
pub fn mulc(a: C, b: C) -> C {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

#[inline]
pub fn screen(a: C, b: C) -> C {
    [
        1.0 - (1.0 - a[0]) * (1.0 - b[0].min(1.0)),
        1.0 - (1.0 - a[1]) * (1.0 - b[1].min(1.0)),
        1.0 - (1.0 - a[2]) * (1.0 - b[2].min(1.0)),
    ]
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

#[inline]
pub fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let t = sat((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

pub fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn to_srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn oklab(c: C) -> C {
    let (r, g, b) = (to_linear(c[0]), to_linear(c[1]), to_linear(c[2]));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

pub fn oklab_linear(l: f32, a: f32, b: f32) -> C {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_35 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

pub fn from_oklab(c: C) -> C {
    let l = c[0] + 0.396_337_78 * c[1] + 0.215_803_76 * c[2];
    let m = c[0] - 0.105_561_346 * c[1] - 0.063_854_17 * c[2];
    let s = c[0] - 0.089_484_18 * c[1] - 1.291_485_5 * c[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        to_srgb(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        to_srgb(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        to_srgb(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    ]
}

pub fn oklch(l: f32, c: f32, h: f32) -> C {
    let h = h.to_radians();
    let mut c = c;
    for _ in 0..40 {
        let rgb = oklab_linear(l, c * h.cos(), c * h.sin());
        if rgb.iter().all(|v| (-1e-4..=1.0001).contains(v)) || c < 1e-4 {
            return rgb.map(|v| v.clamp(0.0, 1.0));
        }
        c *= 0.93;
    }
    oklab_linear(l, 0.0, 0.0)
}

pub fn hue_of(c: C) -> u16 {
    let lab = oklab(c);
    (lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0).round() as u16) % 360
}

pub fn lit(h: f32, by: f32) -> f32 {
    let d = (90.0 - h + 540.0).rem_euclid(360.0) - 180.0;
    h + by.min(d.abs()) * d.signum()
}

pub struct Ramp(Vec<C>);

impl Ramp {
    pub fn new(stops: &[(f32, C)]) -> Ramp {
        const N: usize = 1024;
        let labs: Vec<(f32, C)> = stops.iter().map(|&(t, c)| (t, oklab(c))).collect();
        let lut = (0..N)
            .map(|i| {
                let t = i as f32 / (N - 1) as f32;
                let j = labs.iter().rposition(|s| s.0 <= t).unwrap_or(0);
                let k = (j + 1).min(labs.len() - 1);
                let (t0, a) = labs[j];
                let (t1, b) = labs[k];
                let f = if t1 > t0 {
                    sat((t - t0) / (t1 - t0))
                } else {
                    0.0
                };
                from_oklab(mix(a, b, f))
            })
            .collect();
        Ramp(lut)
    }

    pub fn hex(stops: &[(f32, u32)]) -> Ramp {
        Ramp::new(&stops.iter().map(|&(t, c)| (t, hex(c))).collect::<Vec<_>>())
    }

    pub fn at(&self, t: f32) -> C {
        let n = self.0.len() - 1;
        let x = sat(t) * n as f32;
        let i = (x as usize).min(n - 1);
        mix(self.0[i], self.0[i + 1], x - i as f32)
    }
}

#[inline]
fn argb(r: u32, g: u32, b: u32) -> u32 {
    0xFF00_0000 | r << 16 | g << 8 | b
}

pub fn quantise(px: &[C], w: usize, seed: u32) -> Vec<u32> {
    let mut out = vec![0u32; px.len()];
    par_rows(&mut out, w, |j, row| {
        for (i, o) in row.iter_mut().enumerate() {
            let c = px[j * w + i];
            let (i, j) = (i as i32, j as i32);
            let h0 = hash_form(i, j, seed ^ 0x51ed);
            let h1 = hash_form(i, j, seed ^ 0xa3c9);
            let mut v = [0u32; 3];
            for (ch, v) in v.iter_mut().enumerate() {
                let a = ((h0 >> (ch * 10)) & 1023) as f32 / 1023.0;
                let b = ((h1 >> (ch * 10)) & 1023) as f32 / 1023.0;
                let n = a + b - 1.0;
                *v = (c[ch] * 255.0 + 0.5 + n).floor().clamp(0.0, 255.0) as u32;
            }
            *o = argb(v[0], v[1], v[2]);
        }
    });
    out
}

pub fn quantise_grain(px: &[C], w: usize, grain: f32, seed: u32) -> Vec<u32> {
    let mut out = vec![0u32; px.len()];
    par_rows(&mut out, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let c = px[y * w + x];
            let hh = hash_land(x as i32, y as i32, seed ^ 0xA5A5_1234);
            let r1 = (hh & 0xFFFF) as f32 / 65535.0;
            let r2 = (hh >> 16) as f32 / 65535.0;
            let g = 1.0 + grain * (r1 + r2 - 1.0) * 1.7;
            let dh = hash_land(x as i32, y as i32, seed ^ 0x5EED_0001);
            let ch = |k: u32, v: f32| {
                let d = (((dh >> k) & 0xFF) as f32 + ((dh >> (k + 8)) & 0xFF) as f32) / 255.0 - 1.0;
                ((v * g).clamp(0.0, 1.0) * 255.0 + d * 0.9)
                    .round()
                    .clamp(0.0, 255.0) as u32
            };
            *o = argb(ch(0, c[0]), ch(8, c[1]), ch(16, c[2]));
        }
    });
    out
}

struct Shoulder(Vec<f32>);

const SHOULDER_N: usize = 16384;
const SHOULDER_MAX: f32 = 4.0;

impl Shoulder {
    fn new() -> Self {
        let t = (0..=SHOULDER_N)
            .map(|i| {
                let x = i as f32 / SHOULDER_N as f32 * SHOULDER_MAX;
                let y = if x < 0.7 {
                    x
                } else {
                    0.7 + 0.3 * (1.0 - (-(x - 0.7) / 0.3).exp())
                };
                let s = if y <= 0.003_130_8 {
                    12.92 * y
                } else {
                    1.055 * y.powf(1.0 / 2.4) - 0.055
                };
                s * 255.0
            })
            .collect();
        Shoulder(t)
    }

    fn map(&self, x: f32) -> f32 {
        let f = (x.clamp(0.0, SHOULDER_MAX) / SHOULDER_MAX) * SHOULDER_N as f32;
        let i = (f as usize).min(SHOULDER_N - 1);
        let t = f - i as f32;
        self.0[i] + (self.0[i + 1] - self.0[i]) * t
    }
}

pub fn quantise_shoulder(px: &[C], w: usize) -> Vec<u32> {
    let tone = Shoulder::new();
    let mut out = vec![0u32; px.len()];
    par_rows(&mut out, w, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let i = y * w + x;
            let p = px[i];
            let mut z = (i as u64)
                .wrapping_add(0x9e37_79b9_7f4a_7c15)
                .wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^= z >> 31;
            let mut v = [0u32; 3];
            for (c, v) in v.iter_mut().enumerate() {
                let a = ((z >> (c * 20)) & 1023) as f32;
                let b = ((z >> (c * 20 + 10)) & 1023) as f32;
                let d = (a + b) * (1.0 / 1023.0) - 1.0;
                *v = (tone.map(p[c]) + d + 0.5).clamp(0.0, 255.0) as u8 as u32;
            }
            *o = argb(v[0], v[1], v[2]);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_of_reads_the_oklch_hue() {
        let c = oklch(0.6, 0.1, 245.0).map(to_srgb);
        assert_eq!(hue_of(c), 245);
    }
}
