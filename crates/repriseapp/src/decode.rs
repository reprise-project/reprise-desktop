//! Pluggable image decoders: turn file bytes into a [`Canvas`] fitted to a caller-supplied maximum size, plus the source's own dimensions. [`ImageDecoder`] is `Send + Sync` because Explorer's thumbnail worker decodes from a thread.
//!
//! [`Decoders::default_set`] registers [`ImageCrateDecoder`] (the `image`
//! crate's formats, listed on it) and [`JxlDecoder`]; a caller with a format
//! neither covers can [`Decoders::register`] another before or after them.

use std::path::Path;

use reprisekit::icon::premultiply;
use reprisekit::raster::Canvas;

mod jxl;

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
        d.register(Box::new(JxlDecoder));
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
    read_capped_from(std::fs::File::open(path)?)
}

/// The bytes of open file `f`, or `InvalidData` when it holds more than [`MAX_FILE_BYTES`].
pub fn read_capped_from(f: std::fs::File) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
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
        let (fw, fh) = reprisekit::raster::fit_size(width, height, max);
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

/// JPEG XL's first frame through libjxl, refused when it states more than
/// [`MAX_DECODED_BYTES`] of pixels or libjxl would hold more than [`JXL_ALLOC_LIMIT`].
pub struct JxlDecoder;

/// What libjxl may hold at once while decoding one picture.
pub const JXL_ALLOC_LIMIT: usize = 4 * MAX_DECODED_BYTES as usize;

impl ImageDecoder for JxlDecoder {
    fn probe(&self, bytes: &[u8]) -> bool {
        bytes.starts_with(&[0xff, 0x0a]) || bytes.starts_with(reprisefs::imagesize::JXL_CONTAINER)
    }

    fn decode(&self, bytes: &[u8], max: (u32, u32)) -> Option<Decoded> {
        let p = jxl::first_frame(bytes, MAX_DECODED_BYTES, JXL_ALLOC_LIMIT).ok()?;
        let img = image::RgbaImage::from_raw(p.width, p.height, p.px)?;
        let (fw, fh) = reprisekit::raster::fit_size(p.width, p.height, max);
        Some(Decoded {
            canvas: fit_buffer(&img, fw, fh),
            width: p.width,
            height: p.height,
        })
    }
}

/// The file extensions [`Decoders::default_set`] reads, lower case.
pub const EXTS: [&str; 20] = [
    "png", "apng", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "tif", "tiff", "ico", "qoi", "pbm",
    "pgm", "ppm", "pnm", "pam", "hdr", "exr", "jxl",
];

/// Whether `path`'s extension is one of [`EXTS`], in any case.
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// One frame of a moving picture, composed onto the ones before it, and how long it shows.
pub struct Frame {
    pub canvas: Canvas,
    pub delay_ms: u32,
}

/// A frame delay of less than this many ms shows for [`SHORT_DELAY_MS`], as GIFs made for
/// browsers expect.
const MIN_DELAY_MS: u32 = 20;
const SHORT_DELAY_MS: u32 = 100;

/// The frames of an animated GIF, PNG or WebP at full size. `None` for a still picture, another
/// format, or frames that together pass [`MAX_DECODED_BYTES`]: the caller shows the first frame
/// alone from [`Decoders::decode`].
pub fn frames(bytes: &[u8]) -> Option<Vec<Frame>> {
    use image::AnimationDecoder as _;
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    let cur = std::io::Cursor::new(bytes);
    let frames = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Gif => GifDecoder::new(cur).ok()?.into_frames(),
        image::ImageFormat::Png => {
            let d = PngDecoder::new(cur).ok()?;
            if !d.is_apng().ok()? {
                return None;
            }
            d.apng().ok()?.into_frames()
        }
        image::ImageFormat::WebP => {
            let d = WebPDecoder::new(cur).ok()?;
            if !d.has_animation() {
                return None;
            }
            d.into_frames()
        }
        _ => return None,
    };
    let mut out = Vec::new();
    let mut spent = 0u64;
    for f in frames {
        let f = f.ok()?;
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n.checked_div(d).unwrap_or(0);
        let buf = f.into_buffer();
        spent += u64::from(buf.width()) * u64::from(buf.height()) * 4;
        if spent > MAX_DECODED_BYTES {
            return None;
        }
        out.push(Frame {
            canvas: fit_buffer(&buf, buf.width(), buf.height()),
            delay_ms: if ms < MIN_DELAY_MS {
                SHORT_DELAY_MS
            } else {
                ms
            },
        });
    }
    (out.len() > 1).then_some(out)
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

    fn rgba_gradient(w: u32, h: u32) -> image::RgbaImage {
        image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([x as u8 * 6, y as u8 * 8, 90, 255])
        })
    }

    fn jxl_of(img: image::RgbaImage) -> Vec<u8> {
        jxl::tests::encode(img.width(), img.height(), &[img.as_raw()], false)
    }

    /// A codestream whose SizeHeader claims 20000x20000 (the 18-bit height form, ratio 1:1),
    /// then default image metadata.
    fn jxl_header_claiming_20000_square() -> Vec<u8> {
        vec![0xff, 0x0a, 0xfc, 0x70, 0x22, 0x01]
    }

    #[test]
    fn decodes_a_jxl_through_the_default_set() {
        let bytes = jxl_of(rgba_gradient(40, 30));
        let d = Decoders::default_set().decode(&bytes, (20, 20)).unwrap();
        assert_eq!((d.width, d.height, d.canvas.width), (40, 30, 20));
        let full = JxlDecoder.decode(&bytes, (96, 96)).unwrap();
        assert_eq!(
            full.canvas.get(39, 29),
            0xff00_0000 | (234 << 16) | (232 << 8) | 90
        );
        assert!(is_image(Path::new("/home/u/Pictures/Screenshot.JXL")));
    }

    #[test]
    fn a_jxl_past_the_byte_budget_or_cut_short_decodes_to_nothing() {
        let big = jxl_header_claiming_20000_square();
        assert_eq!(
            reprisefs::imagesize::dimensions(&big),
            Some((20_000, 20_000))
        );
        assert!(JxlDecoder.probe(&big));
        assert!(JxlDecoder.decode(&big, (96, 96)).is_none());
        let whole = jxl_of(rgba_gradient(40, 30));
        assert!(
            JxlDecoder
                .decode(&whole[..whole.len() / 2], (96, 96))
                .is_none()
        );
        assert!(!JxlDecoder.probe(&encode_png(1, 1, &[0, 0, 0, 255])));
    }

    /// Premultiplied samples come out of libjxl unpremultiplied and are multiplied once.
    #[test]
    fn a_premultiplied_jxl_is_multiplied_by_alpha_once() {
        let bytes = jxl::tests::encode(1, 1, &[&[100, 50, 24, 128]], true);
        let d = JxlDecoder.decode(&bytes, (96, 96)).unwrap();
        let [a, r, g, b] = d.canvas.get(0, 0).to_be_bytes();
        assert_eq!(a, 128);
        for (got, want) in [(r, 100), (g, 50), (b, 24)] {
            assert!(got.abs_diff(want) <= 1, "{got} vs {want}");
        }
    }

    #[test]
    fn every_listed_extension_is_a_format_the_decoder_reads() {
        for e in EXTS {
            if e == "jxl" {
                continue;
            }
            let f = image::ImageFormat::from_extension(e).unwrap_or_else(|| panic!("{e}"));
            assert!(f.reading_enabled(), "{e}");
        }
        assert_eq!(
            image::ImageFormat::from_extension("jxl"),
            None,
            "JxlDecoder reads it"
        );
        assert!(is_image(Path::new("/home/u/Pictures/Beach.JPG")));
        assert!(!is_image(Path::new("/home/u/notes.txt")));
    }

    /// A two-frame GIF plays as two frames with their delays; a 0 ms delay shows for 100 ms. A
    /// still GIF and a PNG have no frames.
    #[test]
    fn an_animated_gif_has_its_frames_and_a_still_one_none() {
        use image::codecs::gif::{GifEncoder, Repeat};
        let frame = |rgba: [u8; 4], ms: u32| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba)),
                0,
                0,
                image::Delay::from_numer_denom_ms(ms, 1),
            )
        };
        let gif = |frames: Vec<image::Frame>| {
            let mut bytes = Vec::new();
            {
                let mut enc = GifEncoder::new(&mut bytes);
                enc.set_repeat(Repeat::Infinite).unwrap();
                enc.encode_frames(frames).unwrap();
            }
            bytes
        };
        let two = gif(vec![
            frame([255, 0, 0, 255], 250),
            frame([0, 0, 255, 255], 0),
        ]);
        let got = frames(&two).expect("two frames");
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].delay_ms, got[1].delay_ms), (250, 100));
        assert_eq!(got[0].canvas.get(0, 0), 0xFFFF_0000);
        assert_eq!(got[1].canvas.get(1, 1), 0xFF00_00FF);
        assert!(frames(&gif(vec![frame([0, 255, 0, 255], 100)])).is_none());
        assert!(frames(&encode_png(1, 1, &[0, 0, 0, 255])).is_none());
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
