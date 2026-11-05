//! BT.709 in full range, which RDP clients decode whatever the stream signals (MS-RDPEGFX
//! 3.3.8.3.1). NVENC converts RGB itself, to limited-range BT.601.

use ffmpeg::color::Range;

/// Limited range would show black as 16 and white as 235 on a client.
pub const FULL_RANGE: bool = true;

/// [`FULL_RANGE`] as the stream signals it.
pub const RANGE: Range = if FULL_RANGE { Range::JPEG } else { Range::MPEG };

/// The luma of black.
pub const Y_OFFSET: i32 = if FULL_RANGE { 0 } else { 16 };
const LUMA_GAIN: f64 = if FULL_RANGE { 1.0 } else { 219.0 / 255.0 };
const CHROMA_GAIN: f64 = if FULL_RANGE { 1.0 } else { 224.0 / 255.0 };

/// Rows of the RGB to YUV matrix in 16.16 fixed point. G's chroma weight is minus the other
/// two, so a grey has U and V of exactly 128.
pub const Y: [i32; 3] = [
    fixed(0.2126 * LUMA_GAIN),
    fixed(0.7152 * LUMA_GAIN),
    fixed(0.0722 * LUMA_GAIN),
];
pub const U: [i32; 3] = chroma(-0.2126 / 1.8556, 0.5);
pub const V: [i32; 3] = chroma(0.5, -0.0722 / 1.5748);

const fn fixed(k: f64) -> i32 {
    (k * 65536.0).round() as i32
}

const fn chroma(r: f64, b: f64) -> [i32; 3] {
    let (r, b) = (fixed(r * CHROMA_GAIN), fixed(b * CHROMA_GAIN));
    [r, -r - b, b]
}

/// The Y, U and V of an RGB pixel.
pub fn yuv(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let row = |offset: i32, [kr, kg, kb]: [i32; 3]| {
        let value = offset + ((kr * r + kg * g + kb * b + (1 << 15)) >> 16);
        value.clamp(0, 255) as u8
    };
    [row(Y_OFFSET, Y), row(128, U), row(128, V)]
}

/// The exact luma of an RGB pixel, for checking a conversion.
pub fn luma(r: f64, g: f64, b: f64) -> f64 {
    f64::from(Y_OFFSET) + LUMA_GAIN * (0.2126 * r + 0.7152 * g + 0.0722 * b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_full_range_bt709() {
        for (rgb, want) in [
            ((0, 0, 0), [0, 128, 128]),
            ((255, 255, 255), [255, 128, 128]),
            ((255, 0, 0), [54, 99, 255]),
            ((0, 255, 0), [182, 30, 12]),
            ((0, 0, 255), [18, 255, 116]),
        ] {
            assert_eq!(yuv(rgb.0, rgb.1, rgb.2), want, "{rgb:?}");
        }
        assert!(
            (0..=255).all(|c| yuv(c, c, c) == [c, 128, 128]),
            "a grey keeps its value as luma and has no colour"
        );
        assert!((luma(255.0, 255.0, 255.0) - 255.0).abs() < 1e-9);
    }
}
