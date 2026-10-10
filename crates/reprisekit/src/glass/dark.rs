//! Dark Glass: the control art's light faces turned dark. Each pixel's OKLab
//! lightness goes through a curve with hue and chroma kept, so gloss order is
//! preserved, outlines stay lighter than their faces, ink turns light, hot and
//! pressed faces go dark blue, and saturated outlines and glows keep their
//! colour. Translucent dark pixels are shadows and stay as they are.

use std::collections::HashMap;

use crate::style::PartImage;
use crate::theme::Color;

/// Lightness map for grey pixels, linear between points. Ink (0) goes
/// near-white, outlines (up to 0.72) stay lighter than faces, and a face's
/// range (0.80..1) lands on 0.19..0.33 in order, so no gradient turns back.
const CURVE: [(f32, f32); 7] = [
    (0.00, 0.93),
    (0.40, 0.72),
    (0.62, 0.58),
    (0.72, 0.50),
    (0.80, 0.19),
    (0.90, 0.23),
    (1.00, 0.33),
];

/// The same for tinted pixels: hot and pressed faces start at 0.72 and go
/// dark in order. Tinted outlines (up to 0.64) and ink turn light as grey ones do.
const TINTED_CURVE: [(f32, f32); 7] = [
    (0.00, 0.93),
    (0.40, 0.72),
    (0.64, 0.58),
    (0.72, 0.15),
    (0.80, 0.19),
    (0.90, 0.23),
    (1.00, 0.33),
];

/// OKLab chroma range over which a pixel moves from [`CURVE`] to [`TINTED_CURVE`].
const TINTED: (f32, f32) = (0.015, 0.04);

/// OKLab chroma range from which a pixel keeps its lightness (saturated outline, glow, fill).
const CHROMA_KEPT: (f32, f32) = (0.10, 0.14);

/// Lightness range from which a saturated pixel keeps its lightness: saturated
/// ink, such as a check mark's navy, turns light like grey ink.
const KEPT_ABOVE: (f32, f32) = (0.40, 0.50);

const SHADOW_ALPHA: u32 = 0xF0;
const SHADOW_LIGHTNESS: f32 = 0.5;

pub fn remap_px(px: u32) -> u32 {
    let a = px >> 24;
    if a == 0 {
        return px;
    }
    let lab = oklab(unpremultiply(px));
    if a < SHADOW_ALPHA && lab[0] < SHADOW_LIGHTNESS {
        return px;
    }
    let chroma = lab[1].hypot(lab[2]);
    let tint = smoothstep(TINTED.0, TINTED.1, chroma);
    let dark = curve(&CURVE, lab[0]) * (1.0 - tint) + curve(&TINTED_CURVE, lab[0]) * tint;
    let keep = smoothstep(CHROMA_KEPT.0, CHROMA_KEPT.1, chroma)
        * smoothstep(KEPT_ABOVE.0, KEPT_ABOVE.1, lab[0]);
    let l = dark * (1.0 - keep) + lab[0] * keep;
    premultiply(srgb([l, lab[1], lab[2]]), a)
}

pub fn remap_color(c: Color) -> Color {
    let px = remap_px(0xFF00_0000 | u32::from(c.r) << 16 | u32::from(c.g) << 8 | u32::from(c.b));
    Color::rgb((px >> 16) as u8, (px >> 8) as u8, px as u8)
}

/// The art repeats its colours, so each is worked out once.
pub fn remap_part(mut img: PartImage) -> PartImage {
    let mut seen: HashMap<u32, u32> = HashMap::new();
    for p in &mut img.atlas.pixels {
        *p = *seen.entry(*p).or_insert_with(|| remap_px(*p));
    }
    img
}

fn curve(knots: &[(f32, f32)], l: f32) -> f32 {
    let l = l.clamp(0.0, 1.0);
    for w in knots.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if l <= x1 {
            return y0 + (y1 - y0) * (l - x0) / (x1 - x0);
        }
    }
    knots[knots.len() - 1].1
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn unpremultiply(px: u32) -> [f32; 3] {
    let a = (px >> 24) as f32;
    [16, 8, 0].map(|sh| (((px >> sh) & 0xFF) as f32 / a).min(1.0))
}

fn premultiply(c: [f32; 3], a: u32) -> u32 {
    let ch = |v: f32| ((v * a as f32).round() as u32).min(a);
    a << 24 | ch(c[0]) << 16 | ch(c[1]) << 8 | ch(c[2])
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

fn oklab(c: [f32; 3]) -> [f32; 3] {
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

fn srgb(lab: [f32; 3]) -> [f32; 3] {
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

    fn lightness(c: Color) -> f32 {
        oklab(rgb(c))[0]
    }

    fn rgb(c: Color) -> [f32; 3] {
        [c.r, c.g, c.b].map(|v| f32::from(v) / 255.0)
    }

    #[test]
    fn oklab_round_trips_srgb() {
        for c in [0x000000, 0xFFFFFF, 0x3399FF, 0xF0F0F0, 0x707070, 0xC42B1C] {
            let px = 0xFF00_0000 | c;
            let back = srgb(oklab(unpremultiply(px)));
            let again = premultiply(back, 255);
            assert_eq!(again, px, "{c:06X} survives OKLab and back");
        }
    }

    #[test]
    fn white_and_the_face_go_dark_in_order() {
        let white = remap_color(Color::rgb(0xFF, 0xFF, 0xFF));
        let face = remap_color(Color::rgb(0xF0, 0xF0, 0xF0));
        let low = remap_color(Color::rgb(0xDC, 0xDC, 0xDC));
        assert!(lightness(white) < 0.35, "white turns dark, got {white:?}");
        assert!(
            lightness(low) < lightness(face) && lightness(face) < lightness(white),
            "a face's gloss keeps its order: {low:?} < {face:?} < {white:?}"
        );
    }

    #[test]
    fn outlines_stay_lighter_than_the_faces_and_ink_turns_light() {
        let face = remap_color(Color::rgb(0xF0, 0xF0, 0xF0));
        let outline = remap_color(Color::rgb(0x70, 0x70, 0x70));
        let ink = remap_color(Color::rgb(0x00, 0x00, 0x00));
        assert!(
            lightness(outline) > lightness(face) + 0.1,
            "707070 outline {outline:?} stands out of {face:?}"
        );
        assert!(
            ink.r > 0xD0 && ink.g > 0xD0 && ink.b > 0xD0,
            "black ink turns light, got {ink:?}"
        );
    }

    #[test]
    fn saturated_pixels_and_hues_keep() {
        let blue = Color::rgb(0x33, 0x99, 0xFF);
        let kept = remap_color(blue);
        for (a, b) in [(kept.r, blue.r), (kept.g, blue.g), (kept.b, blue.b)] {
            assert!(a.abs_diff(b) <= 1, "3399FF keeps, got {kept:?}");
        }
        let pale = remap_color(Color::rgb(0xE5, 0xF1, 0xFB));
        assert!(
            pale.b > pale.r,
            "a pale blue face stays blue when dark, got {pale:?}"
        );
    }

    #[test]
    fn a_pale_blue_face_goes_dark_in_order_and_navy_ink_turns_light() {
        let top = remap_color(Color::rgb(0xE5, 0xF4, 0xFC));
        let band = remap_color(Color::rgb(0x72, 0xB9, 0xDF));
        assert!(
            lightness(band) < lightness(top) && lightness(top) < 0.35,
            "a pressed face's deeper band {band:?} stays under its pale top {top:?}, both dark"
        );
        let navy = remap_color(Color::rgb(0x04, 0x22, 0x71));
        assert!(
            lightness(navy) > 0.7 && navy.b > navy.r,
            "a navy check mark turns light blue, got {navy:?}"
        );
    }

    #[test]
    fn a_face_gradient_never_turns_back() {
        let mut last = 0.0;
        for v in 0xCC..=0xFF {
            let l = lightness(remap_color(Color::rgb(v, v, v)));
            assert!(l >= last, "{v:02X} goes dark in order, got {l}");
            last = l;
        }
    }

    #[test]
    fn alpha_is_untouched_and_shadows_and_clear_pixels_keep() {
        assert_eq!(remap_px(0), 0, "a clear pixel stays clear");
        let shadow = 0x4000_0000;
        assert_eq!(
            remap_px(shadow),
            shadow,
            "a translucent black shadow stays dark"
        );
        let half_white = 0x8080_8080;
        assert_eq!(remap_px(half_white) >> 24, 0x80, "alpha is kept");
    }
}
