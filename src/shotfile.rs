//! A screenshot's saved file in `[screenshot] format`. The clipboard always takes the fast PNG.

use image::ImageEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use jpegxl_rs::encode::EncoderSpeed;
use repriseconf::config::{PngCompression, ScreenshotConfig, ShotFormat};

use crate::raster::Canvas;

/// The file is the clipboard's PNG byte for byte, so it needs no second encode.
pub fn reuses_clipboard(s: &ScreenshotConfig) -> bool {
    s.format == ShotFormat::Png && s.png_compression == PngCompression::Fast
}

/// Held through each file encode, so a second Print Screen waits for the first rather than
/// both taking the cores at once.
static ENCODING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// `still`'s opaque RGB in `s.format`, one encode at a time. Blocking: a JPEG XL at high effort
/// takes seconds.
pub fn encode(still: &Canvas, s: &ScreenshotConfig) -> Result<Vec<u8>, String> {
    let _one = ENCODING.lock().unwrap_or_else(|e| e.into_inner());
    let (w, h) = still.device_size();
    let rgb: Vec<u8> = (still.pixels.iter())
        .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
        .collect();
    let mut out = Vec::new();
    let ty = image::ExtendedColorType::Rgb8;
    match s.format {
        ShotFormat::Png => {
            let level = match s.png_compression {
                PngCompression::Fast => CompressionType::Fast,
                PngCompression::Default => CompressionType::Default,
                PngCompression::Best => CompressionType::Best,
            };
            PngEncoder::new_with_quality(&mut out, level, FilterType::Adaptive)
                .write_image(&rgb, w, h, ty)
        }
        ShotFormat::Jpg => image::codecs::jpeg::JpegEncoder::new_with_quality(
            &mut out,
            s.jpg_quality.clamp(1, 100),
        )
        .write_image(&rgb, w, h, ty),
        ShotFormat::Jxl => return jxl(&rgb, w, h, s).map_err(|e| e.to_string()),
    }
    .map_err(|e| e.to_string())?;
    Ok(out)
}

/// `[screenshot] jxl_effort` 1..=9 in order.
const EFFORT: [EncoderSpeed; 9] = [
    EncoderSpeed::Lightning,
    EncoderSpeed::Thunder,
    EncoderSpeed::Falcon,
    EncoderSpeed::Cheetah,
    EncoderSpeed::Hare,
    EncoderSpeed::Wombat,
    EncoderSpeed::Squirrel,
    EncoderSpeed::Kitten,
    EncoderSpeed::Tortoise,
];

fn jxl(
    rgb: &[u8],
    w: u32,
    h: u32,
    s: &ScreenshotConfig,
) -> Result<Vec<u8>, jpegxl_rs::EncodeError> {
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let runner = jpegxl_rs::ThreadsRunner::new(None, Some((cores / 2).max(1)))
        .ok_or(jpegxl_rs::EncodeError::CannotCreateEncoder)?;
    let speed = EFFORT[usize::from(s.jxl_effort.clamp(1, 9)) - 1];
    let builder = jpegxl_rs::encoder_builder()
        .parallel_runner(&runner)
        .speed(speed);
    // libjxl refuses a lossless frame unless the original colour profile is kept.
    let mut enc = if s.jxl_lossless {
        builder.lossless(true).uses_original_profile(true).build()?
    } else {
        // Quality 100 is distance 0, which libjxl's lossy (XYB) path does not take.
        builder
            .jpeg_quality(f32::from(s.jxl_quality.clamp(1, 99)))
            .build()?
    };
    let coded: jpegxl_rs::encode::EncoderResult<u8> = enc.encode(rgb, w, h)?;
    Ok(coded.data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_canvas(w: u32, h: u32) -> Canvas {
        let px = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                0xff00_0000 | ((x * 4) << 16) | ((y * 5) << 8) | (((x ^ y) * 3) & 0xff)
            })
            .collect();
        Canvas::from_pixels(w, h, px)
    }

    fn rgb_of(c: &Canvas) -> Vec<u8> {
        (c.pixels.iter())
            .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8])
            .collect()
    }

    #[test]
    fn each_format_writes_its_own_file() {
        let still = test_canvas(64, 48);
        for (format, magic) in [
            (ShotFormat::Png, &b"\x89PNG"[..]),
            (ShotFormat::Jpg, &[0xff, 0xd8][..]),
            (ShotFormat::Jxl, &[0xff, 0x0a][..]),
        ] {
            let s = ScreenshotConfig {
                format,
                ..Default::default()
            };
            assert!(encode(&still, &s).unwrap().starts_with(magic), "{format:?}");
        }
        let lossy = ScreenshotConfig {
            format: ShotFormat::Jxl,
            jxl_lossless: false,
            jxl_effort: 1,
            ..Default::default()
        };
        assert!(encode(&still, &lossy).unwrap().starts_with(&[0xff, 0x0a]));
    }

    #[test]
    fn a_lossless_jxl_and_every_png_level_keep_every_pixel() {
        let still = test_canvas(64, 48);
        let want = rgb_of(&still);
        for png_compression in PngCompression::ALL {
            let s = ScreenshotConfig {
                png_compression,
                ..Default::default()
            };
            let back = image::load_from_memory(&encode(&still, &s).unwrap()).unwrap();
            assert_eq!(back.to_rgb8().into_raw(), want, "{png_compression:?}");
        }
        let s = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..Default::default()
        };
        let bytes = encode(&still, &s).unwrap();
        let dec = jpegxl_rs::decoder_builder()
            .pixel_format(jpegxl_rs::decode::PixelFormat {
                num_channels: 3,
                ..Default::default()
            })
            .build()
            .unwrap();
        let (meta, px) = dec.decode_with::<u8>(&bytes).unwrap();
        assert_eq!((meta.width, meta.height), (64, 48));
        assert_eq!(px, want);
    }

    #[test]
    fn a_lossy_jxl_at_quality_100_saves_at_99() {
        let still = test_canvas(64, 48);
        let at = |jxl_quality| {
            let s = ScreenshotConfig {
                format: ShotFormat::Jxl,
                jxl_lossless: false,
                jxl_quality,
                jxl_effort: 1,
                ..Default::default()
            };
            encode(&still, &s).unwrap()
        };
        assert_eq!(at(100), at(99));
        assert_ne!(at(99), at(90));
    }

    #[test]
    fn only_png_at_fast_reuses_the_clipboard_bytes() {
        let fast = ScreenshotConfig {
            png_compression: PngCompression::Fast,
            ..Default::default()
        };
        assert!(reuses_clipboard(&fast));
        assert!(
            !reuses_clipboard(&ScreenshotConfig::default()),
            "files default to Default"
        );
        let jxl = ScreenshotConfig {
            format: ShotFormat::Jxl,
            ..fast
        };
        assert!(!reuses_clipboard(&jxl));
    }
}
