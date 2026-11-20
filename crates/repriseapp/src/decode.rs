//! Pluggable image decoders: turn file bytes into a [`Canvas`] fitted to a caller-supplied maximum size, plus the source's own dimensions. [`ImageDecoder`] is `Send + Sync` because Explorer's thumbnail worker decodes from a thread.
//!
//! [`Decoders::default_set`] registers [`ImageCrateDecoder`] (the `image`
//! crate's formats, listed on it), [`HelperDecoder::jxl`] and [`HelperDecoder::avif`]; a caller
//! with a format none covers can [`Decoders::register`] another before or after them.

use std::path::Path;

use reprisekit::icon::premultiply;
use reprisekit::raster::Canvas;

pub(crate) mod helper;
pub use helper::HelperDecoder;
pub use reprise_decode_protocol::Format;

/// A decoded image: `canvas` already fitted to the caller's `max` (never
/// upscaled, box-filtered down), `width`/`height` the source's own
/// dimensions for an infotip or the Tiles view.
pub struct Decoded {
    pub canvas: Canvas,
    pub width: u32,
    pub height: u32,
}

/// Why a picture gave nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unread {
    /// Its pixels, or the memory its decoder asked for, pass the [`Budget`].
    TooLarge,
    /// Not a picture any decoder reads, or cut short.
    Unreadable,
}

pub trait ImageDecoder: Send + Sync {
    /// Cheap sniff of `bytes`, so [`Decoders::decode`] only spends real
    /// work on a decoder likely to succeed.
    fn probe(&self, bytes: &[u8]) -> bool;
    fn decode(&self, bytes: &[u8], max: (u32, u32), budget: Budget) -> Result<Decoded, Unread>;
}

pub struct Decoders {
    list: Vec<Box<dyn ImageDecoder>>,
}

impl Decoders {
    pub fn default_set() -> Decoders {
        let mut d = Decoders { list: Vec::new() };
        d.register(Box::new(ImageCrateDecoder));
        d.register(Box::new(HelperDecoder::jxl()));
        d.register(Box::new(HelperDecoder::avif()));
        d
    }

    /// [`Decoders::default_set`] without the formats reprise-decode reads, for a picture an app
    /// hands over: a JPEG XL or AVIF from one decodes only in the thumbnailers' jail
    /// ([`crate::thumbs::picture`]).
    pub fn built_in() -> Decoders {
        let mut d = Decoders { list: Vec::new() };
        d.register(Box::new(ImageCrateDecoder));
        d
    }

    pub fn register(&mut self, d: Box<dyn ImageDecoder>) {
        self.list.push(d);
    }

    /// The first decoder's picture; [`Unread::TooLarge`] when any decoder that read the header
    /// found it past `budget`.
    pub fn decode(&self, bytes: &[u8], max: (u32, u32), budget: Budget) -> Result<Decoded, Unread> {
        let mut why = Unread::Unreadable;
        for d in self.list.iter().filter(|d| d.probe(bytes)) {
            match d.decode(bytes, max, budget) {
                Ok(p) => return Ok(p),
                Err(Unread::TooLarge) => why = Unread::TooLarge,
                Err(Unread::Unreadable) => {}
            }
        }
        Err(why)
    }

    /// The picture in `path` under [`Budget::THUMBNAIL`].
    pub fn decode_file(&self, path: &Path, max: (u32, u32)) -> Option<Decoded> {
        let budget = Budget::THUMBNAIL;
        let bytes = match read_capped(path, budget.file) {
            Ok(b) => b,
            Err(e) => {
                log::debug!("decode_file {}: {e}", path.display());
                return None;
            }
        };
        self.decode(&bytes, max, budget).ok()
    }
}

/// What one decode may cost: the bytes of the file and of its decoded pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub file: u64,
    pub pixels: u64,
}

impl Budget {
    /// Pictures read unasked or for a preview: thumbnails, infotips, the portal's previews.
    pub const THUMBNAIL: Budget = Budget {
        file: MAX_FILE_BYTES,
        pixels: MAX_DECODED_BYTES,
    };

    /// A picture opened to be looked at or edited: [`VIEWER_MAX_BYTES`], or an eighth of the
    /// machine's memory when that is less, but never below [`Budget::THUMBNAIL`].
    pub fn viewer() -> Budget {
        let total = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|m| reprisefs::sysinfo::mem_total_kb(&m));
        Budget::viewer_for(total.map(|kb| kb * 1024))
    }

    /// The decoded picture and its full-size RGBA copy then take under a third of `mem_total`.
    fn viewer_for(mem_total: Option<u64>) -> Budget {
        let pixels = mem_total.map_or(VIEWER_MAX_BYTES, |m| {
            (m / 8).clamp(MAX_DECODED_BYTES, VIEWER_MAX_BYTES)
        });
        Budget {
            file: pixels.max(MAX_FILE_BYTES),
            pixels,
        }
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

/// Largest picture a viewer decodes, in bytes of pixels and of file: a 170-megapixel RGB photo.
pub const VIEWER_MAX_BYTES: u64 = 512 << 20;

fn read_capped(path: &Path, cap: u64) -> std::io::Result<Vec<u8>> {
    read_capped_from(std::fs::File::open(path)?, cap)
}

/// The bytes of open file `f`, or `FileTooLarge` when it holds more than `cap`.
pub fn read_capped_from(f: std::fs::File, cap: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Error, ErrorKind, Read};
    let len = f.metadata()?.len();
    if len > cap {
        return Err(Error::new(ErrorKind::FileTooLarge, "larger than an image"));
    }
    let mut bytes = Vec::with_capacity(len as usize);
    f.take(cap).read_to_end(&mut bytes)?;
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
        image::guess_format(bytes).is_ok_and(|f| f.reading_enabled())
    }

    fn decode(&self, bytes: &[u8], max: (u32, u32), budget: Budget) -> Result<Decoded, Unread> {
        use image::DynamicImage as D;
        use image::ImageDecoder as _;
        let mut decoder = open_capped(bytes, budget.pixels)?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut img = D::from_decoder(decoder).map_err(|_| Unread::Unreadable)?;
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
        Ok(Decoded {
            canvas,
            width,
            height,
        })
    }
}

pub use reprisefs::imagesize::{is_avif, is_jxl};

/// The format of `bytes` that only reprise-decode reads, if any.
pub fn helper_format(bytes: &[u8]) -> Option<Format> {
    if is_jxl(bytes) {
        Some(Format::Jxl)
    } else if is_avif(bytes) {
        Some(Format::Avif)
    } else {
        None
    }
}

/// How long one picture may decode in reprise-decode: twice a slow core's time for a lossless
/// JPEG XL at [`MAX_DECODED_BYTES`] (effort 9), where a few bytes a layer can keep a decoder
/// composing layers for hours.
pub const HELPER_TIME_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// The file extensions [`Decoders::default_set`] reads, lower case.
pub const EXTS: [&str; 21] = [
    "png", "apng", "jpg", "jpeg", "jfif", "gif", "bmp", "webp", "tif", "tiff", "ico", "qoi", "pbm",
    "pgm", "ppm", "pnm", "pam", "hdr", "exr", "jxl", "avif",
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
/// format, a canvas past `budget`'s pixels or frames that together pass them: the caller shows
/// the first frame alone from [`Decoders::decode`].
pub fn frames(bytes: &[u8], budget: Budget) -> Option<Vec<Frame>> {
    use image::AnimationDecoder as _;
    use image::codecs::{gif::GifDecoder, png::PngDecoder, webp::WebPDecoder};
    let cur = std::io::Cursor::new(bytes);
    let frames = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Gif => {
            animation_capped(GifDecoder::new(cur).ok()?, budget.pixels)?.into_frames()
        }
        image::ImageFormat::Png => {
            let d = PngDecoder::new(cur).ok()?;
            if !d.is_apng().ok()? {
                return None;
            }
            animation_capped(d, budget.pixels)?
                .apng()
                .ok()?
                .into_frames()
        }
        image::ImageFormat::WebP => {
            let d = WebPDecoder::new(cur).ok()?;
            if !d.has_animation() {
                return None;
            }
            animation_capped(d, budget.pixels)?.into_frames()
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
        if spent > budget.pixels {
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

/// `d`, or `None` when one RGBA canvas of its size passes `pixels` bytes. The size is checked
/// before the frame iterator allocates its canvas; the allocation cap covers the canvas and one
/// frame alive together.
fn animation_capped<D: image::ImageDecoder>(mut d: D, pixels: u64) -> Option<D> {
    let (w, h) = d.dimensions();
    if u64::from(w) * u64::from(h) * 4 > pixels {
        return None;
    }
    let mut limits = image::Limits::no_limits();
    limits.max_alloc = Some(2 * pixels);
    d.set_limits(limits).ok()?;
    Some(d)
}

/// An `image` decoder for `bytes` that has read the header, refused when the
/// format is unknown or the decoded pixels would pass `pixels` bytes.
fn open_capped(bytes: &[u8], pixels: u64) -> Result<impl image::ImageDecoder + '_, Unread> {
    use image::ImageDecoder as _;
    let decoder = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_decoder().ok())
        .ok_or(Unread::Unreadable)?;
    if decoder.total_bytes() > pixels {
        return Err(Unread::TooLarge);
    }
    Ok(decoder)
}

fn fit_buffer<P, C>(img: &image::ImageBuffer<P, C>, w: u32, h: u32) -> Canvas
where
    P: image::Pixel<Subpixel = u8>,
    C: std::ops::Deref<Target = [u8]>,
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
        assert!(open_capped(&png_header(4096, rows), MAX_DECODED_BYTES).is_ok());
        let past = png_header(4096, rows + 1);
        assert!(ImageCrateDecoder.probe(&past));
        assert_eq!(
            open_capped(&past, MAX_DECODED_BYTES).err(),
            Some(Unread::TooLarge)
        );
        assert_eq!(
            ImageCrateDecoder
                .decode(&past, (96, 96), Budget::THUMBNAIL)
                .err(),
            Some(Unread::TooLarge)
        );
        let viewer = Budget {
            file: MAX_FILE_BYTES,
            pixels: 2 * MAX_DECODED_BYTES,
        };
        assert!(
            open_capped(&past, viewer.pixels).is_ok(),
            "a larger budget opens it"
        );
    }

    #[test]
    fn the_viewer_budget_is_an_eighth_of_memory_within_its_bounds() {
        let gib = 1u64 << 30;
        let pixels = |mem| Budget::viewer_for(mem).pixels;
        assert_eq!(pixels(None), VIEWER_MAX_BYTES);
        assert_eq!(pixels(Some(16 * gib)), VIEWER_MAX_BYTES);
        assert_eq!(pixels(Some(2 * gib)), 256 << 20);
        assert_eq!(pixels(Some(gib / 2)), MAX_DECODED_BYTES);
        assert_eq!(Budget::viewer_for(Some(2 * gib)).file, 256 << 20);
        assert_eq!(Budget::viewer_for(Some(gib / 2)).file, MAX_DECODED_BYTES);
    }

    /// Downscaling averages premultiplied pixels: opaque red beside a fully
    /// transparent blue is half-alpha red, with no blue bleeding in.
    #[test]
    fn decode_downscales_premultiplied() {
        let bytes = encode_png(2, 1, &[255, 0, 0, 255, 0, 0, 255, 0]);
        let decoded = ImageCrateDecoder
            .decode(&bytes, (1, 1), Budget::THUMBNAIL)
            .unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!((decoded.canvas.width, decoded.canvas.height), (1, 1));
        assert_eq!(decoded.canvas.get(0, 0), 0x8080_0000);
    }

    /// A PNG signature, IHDR and an empty IDAT: enough to sniff and to size.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        png_chunks(w, h, &[])
    }

    /// [`png_header`] with `chunks` (type and data) between the IHDR and the IDAT.
    fn png_chunks(w: u32, h: u32, chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut chunk = |kind: &[u8; 4], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let start = png.len();
            png.extend_from_slice(kind);
            png.extend_from_slice(data);
            let crc = crc32(&png[start..]);
            png.extend_from_slice(&crc.to_be_bytes());
        };
        chunk(b"IHDR", &ihdr);
        for (kind, data) in chunks {
            chunk(kind, data);
        }
        chunk(b"IDAT", &[]);
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

        let decoded = Decoders::default_set()
            .decode(&bytes, (96, 96), Budget::THUMBNAIL)
            .unwrap();

        assert_eq!(decoded.width, 4);
        assert_eq!(decoded.height, 2);
        assert_eq!((decoded.canvas.width, decoded.canvas.height), (4, 2));
        let argb = 0x8000_0000 | (200u32 << 16) | (100 << 8) | 50;
        assert_eq!(decoded.canvas.get(0, 0), premultiply(argb));
    }

    /// The first bytes of an AVIF as libavif and ravif write it.
    const AVIF_HEAD: &[u8] = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf\0\0\x01\x0ameta";

    #[test]
    fn the_set_for_an_apps_picture_decodes_no_jxl_or_avif() {
        let jxl = [0xff, 0x0a, 0, 0];
        for bytes in [&jxl[..], AVIF_HEAD] {
            assert!(Decoders::default_set().list.iter().any(|d| d.probe(bytes)));
            assert!(Decoders::built_in().list.iter().all(|d| !d.probe(bytes)));
        }
    }

    #[test]
    fn jpeg_xl_and_avif_are_the_helpers_formats() {
        assert_eq!(
            helper_format(AVIF_HEAD),
            Some(reprise_decode_protocol::Format::Avif)
        );
        assert_eq!(
            helper_format(&[0xff, 0x0a, 0, 0]),
            Some(reprise_decode_protocol::Format::Jxl)
        );
        assert_eq!(helper_format(b"\x89PNG\r\n\x1a\n"), None);
    }

    #[test]
    fn every_listed_extension_is_a_format_the_decoder_reads() {
        for e in EXTS {
            if e == "jxl" || e == "avif" {
                continue;
            }
            let f = image::ImageFormat::from_extension(e).unwrap_or_else(|| panic!("{e}"));
            assert!(f.reading_enabled(), "{e}");
        }
        for e in ["jxl", "avif"] {
            assert!(
                image::ImageFormat::from_extension(e).is_none_or(|f| !f.reading_enabled()),
                "reprise-decode reads {e}"
            );
        }
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
        let got = frames(&two, Budget::THUMBNAIL).expect("two frames");
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].delay_ms, got[1].delay_ms), (250, 100));
        assert_eq!(got[0].canvas.get(0, 0), 0xFFFF_0000);
        assert_eq!(got[1].canvas.get(1, 1), 0xFF00_00FF);
        assert!(frames(&gif(vec![frame([0, 255, 0, 255], 100)]), Budget::THUMBNAIL).is_none());
        assert!(frames(&encode_png(1, 1, &[0, 0, 0, 255]), Budget::THUMBNAIL).is_none());
    }

    /// A GIF of a few dozen bytes whose logical screen is 65535x65535 (16 GiB of RGBA) and an
    /// APNG whose IHDR claims as much are refused before their canvas is allocated.
    #[test]
    fn an_animation_with_a_canvas_past_the_budget_has_no_frames() {
        use image::codecs::gif::GifEncoder;
        let dot = |rgba: [u8; 4]| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(1, 1, image::Rgba(rgba)),
                0,
                0,
                image::Delay::from_numer_denom_ms(100, 1),
            )
        };
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif)
            .encode_frames(vec![dot([255, 0, 0, 255]), dot([0, 0, 255, 255])])
            .unwrap();
        assert!(
            frames(&gif, Budget::THUMBNAIL).is_some(),
            "the 1x1 screen plays"
        );
        gif[6..10].copy_from_slice(&[0xff; 4]);
        assert!(frames(&gif, Budget::THUMBNAIL).is_none());
        let actl = [0, 0, 0, 2, 0, 0, 0, 0];
        let apng = png_chunks(65535, 65535, &[(b"acTL", &actl)]);
        assert!(frames(&apng, Budget::THUMBNAIL).is_none());
    }

    #[test]
    fn decoders_with_nothing_registered_decode_nothing() {
        let empty = Decoders { list: Vec::new() };
        let bytes = encode_png(1, 1, &[0, 0, 0, 255]);
        assert_eq!(
            empty.decode(&bytes, (96, 96), Budget::THUMBNAIL).err(),
            Some(Unread::Unreadable)
        );
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
                .decode(bytes.get_ref(), (96, 96), Budget::THUMBNAIL)
                .unwrap_or_else(|e| panic!("{format:?} did not decode: {e:?}"));
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
        let decoded = ImageCrateDecoder
            .decode(&bytes, (96, 96), Budget::THUMBNAIL)
            .unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 8));
        assert_eq!(
            reprisefs::imagesize::dimensions(&bytes),
            Some((4, 8)),
            "the header read states the size shown"
        );
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
