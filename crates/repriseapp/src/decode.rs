//! Pluggable image decoders: turn file bytes into a [`Canvas`] fitted to a caller-supplied maximum size, plus the source's own dimensions. [`ImageDecoder`] is `Send + Sync` because Explorer's thumbnail worker calls `Decoders::decode_file` from a thread.
//!
//! [`Decoders::default_set`] registers [`ImageCrateDecoder`] (the `image`
//! crate's formats, listed on it); a caller with a format `image` doesn't
//! cover can [`Decoders::register`] another before or after it.

use std::path::Path;

use reprisekit::icon::premultiply;
use reprisekit::raster::Canvas;

/// A decoded image: `canvas` already fitted to the caller's `max` (never
/// upscaled, box-filtered down), `width`/`height` the source's own
/// dimensions for an infotip or the Tiles view.
pub struct Decoded {
    pub canvas: Canvas,
    pub width: u32,
    pub height: u32,
}

pub trait ImageDecoder: Send + Sync {
    /// Cheap sniff of `bytes`, so [`Decoders::decode`] only spends real
    /// work on a decoder likely to succeed.
    fn probe(&self, bytes: &[u8]) -> bool;
    fn decode(&self, bytes: &[u8], max: (u32, u32)) -> Option<Decoded>;
}

pub struct Decoders {
    list: Vec<Box<dyn ImageDecoder>>,
}

impl Decoders {
    pub fn default_set() -> Decoders {
        let mut d = Decoders { list: Vec::new() };
        d.register(Box::new(ImageCrateDecoder));
        d
    }

    pub fn register(&mut self, d: Box<dyn ImageDecoder>) {
        self.list.push(d);
    }

    pub fn decode(&self, bytes: &[u8], max: (u32, u32)) -> Option<Decoded> {
        self.list
            .iter()
            .filter(|d| d.probe(bytes))
            .find_map(|d| d.decode(bytes, max))
    }

    pub fn decode_file(&self, path: &Path, max: (u32, u32)) -> Option<Decoded> {
        let bytes = match read_capped(path) {
            Ok(b) => b,
            Err(e) => {
                log::debug!("decode_file {}: {e}", path.display());
                return None;
            }
        };
        self.decode(&bytes, max)
    }
}

/// Bytes a file may hold before it is skipped rather than decoded; the
/// thumbnail worker reads every image in a folder unasked.
pub const MAX_FILE_BYTES: u64 = 64 << 20;

/// Largest decoded buffer, in bytes, a thumbnail may cost. A small file can
/// claim a huge image, and `image`'s own 512 MiB cap is a quarter of the
/// 2 GB machines this runs on. 96 MiB still takes a 24-megapixel RGB photo
/// (72 MiB) or a 24-megapixel RGBA PNG.
pub const MAX_DECODED_BYTES: u64 = 96 << 20;

fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    let f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len > MAX_FILE_BYTES {
        return Err(Error::new(ErrorKind::InvalidData, "larger than an image"));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    f.take(MAX_FILE_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The `image` crate's decoders: PNG, JPEG, GIF, BMP, WebP, TIFF, ICO,
/// QOI, PNM (PBM, PGM, PPM, PAM), Radiance HDR, OpenEXR and DDS. A
/// high-dynamic-range picture is clamped to 8 bits, not tone-mapped:
/// enough for a thumbnail. An Exif orientation (a phone photo taken on
/// its side) is applied, so the thumbnail and `width`/`height` are the
/// picture upright.
pub struct ImageCrateDecoder;

impl ImageDecoder for ImageCrateDecoder {
    fn probe(&self, bytes: &[u8]) -> bool {
        image::guess_format(bytes).is_ok()
    }

    fn decode(&self, bytes: &[u8], max: (u32, u32)) -> Option<Decoded> {
        use image::DynamicImage as D;
        use image::ImageDecoder as _;
        let mut decoder = open_capped(bytes)?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut img = D::from_decoder(decoder).ok()?;
        img.apply_orientation(orientation);
        let (width, height) = (img.width(), img.height());
        let (fw, fh) = fit(width, height, max);
        // The 8-bit layouts resample from the decoded buffer as it is; only
        // the rest pay for an RGBA8 copy.
        let canvas = match &img {
            D::ImageRgb8(i) => fit_buffer(i, fw, fh),
            D::ImageRgba8(i) => fit_buffer(i, fw, fh),
            D::ImageLuma8(i) => fit_buffer(i, fw, fh),
            D::ImageLumaA8(i) => fit_buffer(i, fw, fh),
            other => fit_buffer(&other.to_rgba8(), fw, fh),
        };
        Some(Decoded {
            canvas,
            width,
            height,
        })
    }
}

/// An `image` decoder for `bytes` that has read the header, `None` when the
/// format is unknown or the decoded pixels would pass [`MAX_DECODED_BYTES`].
fn open_capped(bytes: &[u8]) -> Option<impl image::ImageDecoder + '_> {
    use image::ImageDecoder as _;
    let decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    (decoder.total_bytes() <= MAX_DECODED_BYTES).then_some(decoder)
}

fn fit_buffer<P>(img: &image::ImageBuffer<P, Vec<u8>>, w: u32, h: u32) -> Canvas
where
    P: image::Pixel<Subpixel = u8>,
{
    Canvas::resample_box_from(img.width(), img.height(), w, h, |x, y| {
        let [r, g, b, a] = img.get_pixel(x, y).to_rgba().0;
        premultiply(u32::from_be_bytes([a, r, g, b]))
    })
}

pub fn fit(w: u32, h: u32, max: (u32, u32)) -> (u32, u32) {
    let (mw, mh) = max;
    if w == 0 || h == 0 || (w <= mw && h <= mh) {
        return (w, h);
    }
    let scale = (mw as f64 / w as f64).min(mh as f64 / h as f64);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An RGBA header one row past MAX_DECODED_BYTES is refused after the
    /// header is read, before any pixel is allocated; one at the limit opens.
    #[test]
    fn open_capped_refuses_a_header_past_the_byte_budget() {
        let rows = (MAX_DECODED_BYTES / 4 / 4096) as u32;
        assert!(open_capped(&png_header(4096, rows)).is_some());
        let past = png_header(4096, rows + 1);
        assert!(ImageCrateDecoder.probe(&past));
        assert!(open_capped(&past).is_none());
        assert!(ImageCrateDecoder.decode(&past, (96, 96)).is_none());
    }

    /// Downscaling averages premultiplied pixels: opaque red beside a fully
    /// transparent blue is half-alpha red, with no blue bleeding in.
    #[test]
    fn decode_downscales_premultiplied() {
        let bytes = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 0]);
        let decoded = ImageCrateDecoder.decode(&bytes, (1, 1)).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!((decoded.canvas.width, decoded.canvas.height), (1, 1));
        assert_eq!(decoded.canvas.get(0, 0), 0x8080_0000);
    }

    /// A PNG signature, IHDR and an empty IDAT: enough to sniff and to size.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc32(&ihdr).to_be_bytes());
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IDAT");
        png.extend_from_slice(&crc32(b"IDAT").to_be_bytes());
        png
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }

    #[test]
    fn fit_scales_down_keeping_aspect() {
        assert_eq!(fit(800, 600, (96, 96)), (96, 72));
    }

    #[test]
    fn fit_never_upscales() {
        assert_eq!(fit(50, 20, (96, 96)), (50, 20));
    }

    fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut bytes, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(rgba).unwrap();
        }
        bytes
    }

    #[test]
    fn decodes_a_png_through_the_default_set() {
        // 4x2, a half-transparent colour top-left and opaque black
        // elsewhere, so a wrong crop or resample shows up as a mismatch.
        let mut rgba = Vec::new();
        for _ in 0..8 {
            rgba.extend_from_slice(&[0, 0, 0, 255]);
        }
        rgba[0..4].copy_from_slice(&[200, 100, 50, 128]);
        let bytes = encode_png(4, 2, &rgba);

        let decoded = Decoders::default_set().decode(&bytes, (96, 96)).unwrap();

        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 2);
        assert_eq!((decoded.canvas.width, decoded.canvas.height), (4, 2));
        let argb = 0x8000_0000 | (200u32 << 16) | (100 << 8) | 50;
        assert_eq!(decoded.canvas.get(0, 0), premultiply(argb));
    }

    #[test]
    fn decoders_with_nothing_registered_decode_nothing() {
        let empty = Decoders { list: Vec::new() };
        let bytes = encode_png(1, 1, &[0, 0, 0, 255]);
        assert!(empty.decode(&bytes, (96, 96)).is_none());
    }

    /// The formats turned on beyond PNG, JPEG, BMP and GIF each decode
    /// through the default set from their bytes alone. (HDR, EXR and
    /// DDS are on too; `image` has no 8-bit encoder to make them here.)
    #[test]
    fn decodes_the_further_formats_through_the_default_set() {
        use image::ImageFormat as F;
        let img = image::RgbaImage::from_fn(4, 2, |x, _| image::Rgba([x as u8 * 60, 0, 0, 255]));
        for format in [F::WebP, F::Tiff, F::Ico, F::Qoi, F::Pnm] {
            let mut bytes = std::io::Cursor::new(Vec::new());
            let encoded = if format == F::Pnm {
                image::DynamicImage::ImageRgba8(img.clone())
                    .to_rgb8()
                    .write_to(&mut bytes, format)
            } else {
                img.write_to(&mut bytes, format)
            };
            encoded.unwrap_or_else(|e| panic!("{format:?}: {e}"));
            let decoded = Decoders::default_set()
                .decode(bytes.get_ref(), (96, 96))
                .unwrap_or_else(|| panic!("{format:?} did not decode"));
            assert_eq!((decoded.width, decoded.height), (4, 2), "{format:?}");
            assert_eq!(decoded.canvas.get(3, 0), 0xffb4_0000, "{format:?}");
        }
    }

    /// A JPEG tagged Exif orientation 6 (turn 90 degrees clockwise to
    /// view) decodes upright: 8x4 stored, 4x8 shown, the stored left
    /// (red) half on top and the right (blue) half below.
    #[test]
    fn an_exif_orientation_is_applied() {
        use image::ImageEncoder as _;
        let img = image::RgbImage::from_fn(8, 4, |x, _| {
            image::Rgb(if x < 4 { [255, 0, 0] } else { [0, 0, 255] })
        });
        // A big-endian TIFF header and one IFD entry: Orientation (0x0112),
        // SHORT, count 1, value 6.
        let exif = [
            b'M', b'M', 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0, 0,
            0,
        ];
        let mut bytes = Vec::new();
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 100);
        enc.set_exif_metadata(exif.to_vec()).unwrap();
        enc.write_image(img.as_raw(), 8, 4, image::ExtendedColorType::Rgb8)
            .unwrap();
        let decoded = ImageCrateDecoder.decode(&bytes, (96, 96)).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 8));
        assert_eq!((decoded.canvas.width, decoded.canvas.height), (4, 8));
        let [_, r, _, b] = decoded.canvas.get(1, 1).to_be_bytes();
        assert!(
            r > 200 && b < 60,
            "top is red: {:08x}",
            decoded.canvas.get(1, 1)
        );
        let [_, r, _, b] = decoded.canvas.get(1, 6).to_be_bytes();
        assert!(
            r < 60 && b > 200,
            "bottom is blue: {:08x}",
            decoded.canvas.get(1, 6)
        );
    }
}
