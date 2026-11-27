//! OKLab colour maths for the dark looks: sRGB channels in 0..=1 to `[L, a, b]` and back, and
//! the lightness curves and blends the dark mappings are built from.

use crate::theme::Color;

/// `c`'s sRGB channels in 0..=1.
pub(crate) fn channels(c: Color) -> [f32; 3] {
    [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
}

/// The opaque colour of sRGB channels in 0..=1, rounded to the nearest step.
pub(crate) fn color(c: [f32; 3]) -> Color {
    let [r, g, b] = c.map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8);
    Color::rgb(r, g, b)
}

/// `c`'s OKLab `[L, a, b]`.
pub(crate) fn lab_of(c: Color) -> [f32; 3] {
    lab(channels(c))
}

/// The colour nearest OKLab `lab`, clipped into sRGB.
pub(crate) fn color_of(lab: [f32; 3]) -> Color {
    color(srgb(lab))
}

/// `y` at `l` on a curve linear between `knots`, ordered by `x`; held flat past either end.
pub(crate) fn curve(knots: &[(f32, f32)], l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    for w in knots.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if l <= x1 {
            return y0 + (y1 - y0) * (l - x0) / (x1 - x0);
        }
    }
    knots[knots.len() - 1].1
}

pub(crate) fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB channels in 0..=1 to OKLab.
pub(crate) fn lab(c: [f32; 3]) -> [f32; 3] {
    let [r, g, b] = c.map(to_linear);
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// OKLab to sRGB channels in 0..=1, each clipped.
pub(crate) fn srgb(lab: [f32; 3]) -> [f32; 3] {
    let [l0, a, b] = lab;
    let l = (l0 + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (l0 - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (l0 - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
    .map(|c| to_srgb(c.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_round_trips_srgb() {
        for c in [0x000000, 0xFFFFFF, 0x3399FF, 0xF0F0F0, 0x707070, 0xC42B1C] {
            let c = Color::rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
            assert_eq!(color_of(lab_of(c)), c, "{c:?} survives OKLab and back");
        }
    }
}
