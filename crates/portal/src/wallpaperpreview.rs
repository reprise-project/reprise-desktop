//! The Wallpaper prompt's preview: a picture shown only when the desktop would show it, under the
//! compositor's limits ([`MAX_SIDE`] a side, [`MAX_BYTES`] of file and of decoded pixels, JPEG XL
//! and AVIF in reprise-decode within [`DECODE_DEADLINE`]). A JPEG decodes at the smallest DCT scale
//! covering the preview and a PNG or 24-bit BMP is shrunk row by row as it is read, so neither
//! exists at full size here.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

use repriseapp::decode::{Budget, HelperDecoder, Unread};
use reprisefs::pictures::Format;
use reprisekit::raster::Canvas;
use reprisekit::wallpaper::{DECODE_DEADLINE, MAX_BYTES, MAX_SIDE};

/// The bytes of a file read first to tell its format and size by.
const HEAD: usize = 64 << 10;

/// The picture in `file` fitted inside `max`.
pub fn preview(file: File, max: (u32, u32)) -> Result<Canvas, Unread> {
    preview_by(file, max, |head| {
        let format = repriseapp::decode::helper_format(head)?;
        Some(HelperDecoder::installed(format, DECODE_DEADLINE))
    })
}

/// [`preview`] with JPEG XL and AVIF decoded by the decoder `helper` gives for the file's head.
fn preview_by(
    mut file: File,
    max: (u32, u32),
    helper: impl FnOnce(&[u8]) -> Option<HelperDecoder>,
) -> Result<Canvas, Unread> {
    let mut head = Vec::with_capacity(HEAD);
    (&mut file)
        .take(HEAD as u64)
        .read_to_end(&mut head)
        .map_err(|_| Unread::Unreadable)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| Unread::Unreadable)?;
    match Format::of(&head).ok_or(Unread::Unreadable)? {
        Format::Jxl | Format::Avif => {
            let budget = Budget {
                file: MAX_BYTES,
                pixels: MAX_BYTES,
            };
            let d = helper(&head)
                .ok_or(Unread::Unreadable)?
                .decode_file(file, max, budget)?;
            fits(d.width, d.height, 4)?;
            Ok(d.canvas)
        }
        Format::Png => png(file, max),
        Format::Jpeg => jpeg(file, max),
        Format::Bmp => match bmp_rows(&head) {
            Some(rows) => bmp(file, rows, max),
            None => whole(file, image::ImageFormat::Bmp, max),
        },
    }
}

/// Refuses a `w` x `h` picture of `n` bytes a pixel past `MAX_SIDE` a side or `MAX_BYTES`.
fn fits(w: u32, h: u32, n: u64) -> Result<(), Unread> {
    let small = w <= MAX_SIDE && h <= MAX_SIDE && u64::from(w) * u64::from(h) * n <= MAX_BYTES;
    small.then_some(()).ok_or(Unread::TooLarge)
}

/// A PNG streamed a row at a time, refused when the image crate's decode of it, 16-bit samples
/// kept and a palette or transparent colour expanded, would pass `MAX_BYTES`.
fn png(file: File, max: (u32, u32)) -> Result<Canvas, Unread> {
    use png::{BitDepth, ColorType, Transformations};
    let mut dec = png::Decoder::new(BufReader::new(file));
    dec.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|_| Unread::Unreadable)?;
    let info = reader.info();
    let (w, h) = (info.width, info.height);
    let channels = match info.color_type {
        ColorType::Grayscale => 1,
        ColorType::GrayscaleAlpha => 2,
        ColorType::Rgb | ColorType::Indexed => 3,
        ColorType::Rgba => 4,
    };
    let keyed = info.trns.is_some()
        && matches!(
            info.color_type,
            ColorType::Grayscale | ColorType::Rgb | ColorType::Indexed
        );
    let depth = if info.bit_depth == BitDepth::Sixteen {
        2
    } else {
        1
    };
    let passes = match info.interlaced {
        true => &ADAM7[..],
        false => &[(1, 0, 1, 0)][..],
    };
    fits(w, h, (channels + u64::from(keyed)) * depth)?;
    let (out, _) = reader.output_color_type();
    let n = out.samples();
    let mut shrink = Shrink::new(w, h, max);
    for &(step_x, off_x, step_y, off_y) in passes {
        let samples = w.saturating_sub(off_x).div_ceil(step_x);
        let lines = h.saturating_sub(off_y).div_ceil(step_y);
        if samples == 0 {
            continue;
        }
        for line in 0..lines {
            let row = reader
                .next_row()
                .map_err(|_| Unread::Unreadable)?
                .ok_or(Unread::Unreadable)?;
            let px = row.data().chunks_exact(n).map(|p| match n {
                1 => [p[0], p[0], p[0], 0xff],
                2 => [p[0], p[0], p[0], p[1]],
                3 => [p[0], p[1], p[2], 0xff],
                _ => [p[0], p[1], p[2], p[3]],
            });
            let xs = (0..).map(|i| off_x + i * step_x);
            shrink.row(off_y + line * step_y, xs.zip(px));
        }
    }
    Ok(shrink.canvas())
}

/// Each Adam7 pass's column step and offset, then its row step and offset, in the order its rows
/// are stored.
const ADAM7: [(u32, u32, u32, u32); 7] = [
    (8, 0, 8, 0),
    (8, 4, 8, 0),
    (4, 0, 8, 4),
    (4, 2, 4, 0),
    (2, 0, 4, 2),
    (2, 1, 2, 0),
    (1, 0, 2, 1),
];

/// A grey or RGB JPEG at the smallest DCT scale (1/8 to 1) covering its preview, as the desktop
/// decodes one under Stretch, Fill or Fit; a CMYK one whole, as the desktop does.
fn jpeg(file: File, max: (u32, u32)) -> Result<Canvas, Unread> {
    let mut dec = jpeg_decoder::Decoder::new(BufReader::new(&file));
    dec.read_info().map_err(|_| Unread::Unreadable)?;
    let info = dec.info().ok_or(Unread::Unreadable)?;
    let (sw, sh) = (u32::from(info.width), u32::from(info.height));
    if sw > MAX_SIDE || sh > MAX_SIDE {
        return Err(Unread::TooLarge);
    }
    let n = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        _ => {
            drop(dec);
            return whole(file, image::ImageFormat::Jpeg, max);
        }
    };
    let (cw, ch) = reprisekit::raster::fit_size(sw, sh, max);
    let eighths = |len: u32, k: u32| (len * k).div_ceil(8);
    let k = [1, 2, 4, 8]
        .into_iter()
        .find(|&k| eighths(sw, k) >= cw && eighths(sh, k) >= ch)
        .unwrap_or(8);
    // The decoder takes a scale when either side reaches the request, so ask for both sides at `k`.
    let (dw, dh) = dec
        .scale(eighths(sw, k) as u16, eighths(sh, k) as u16)
        .map_err(|_| Unread::Unreadable)?;
    let bytes = dec.decode().map_err(|_| Unread::Unreadable)?;
    let (dw, dh) = (u32::from(dw), u32::from(dh));
    if bytes.len() != dw as usize * dh as usize * n {
        return Err(Unread::Unreadable);
    }
    let mut shrink = Shrink::new(dw, dh, max);
    for (y, row) in bytes.chunks_exact(dw as usize * n).enumerate() {
        let px = row.chunks_exact(n).map(|p| match n {
            1 => [p[0], p[0], p[0], 0xff],
            _ => [p[0], p[1], p[2], 0xff],
        });
        shrink.row(y as u32, (0..).zip(px));
    }
    Ok(shrink.canvas())
}

/// Where an uncompressed 24-bit BMP's rows are: the first row's offset, the width, the height and
/// whether the rows run top to bottom. None for any other BMP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BmpRows {
    offset: u64,
    w: u32,
    h: u32,
    top_down: bool,
}

fn bmp_rows(head: &[u8]) -> Option<BmpRows> {
    let u16_at = |i: usize| Some(u16::from_le_bytes(head.get(i..i + 2)?.try_into().ok()?));
    let u32_at = |i: usize| Some(u32::from_le_bytes(head.get(i..i + 4)?.try_into().ok()?));
    let (offset, info) = (u32_at(10)?, u32_at(14)?);
    let (w, h) = (u32_at(18)? as i32, u32_at(22)? as i32);
    let uncompressed_24 = info >= 40 && u16_at(28)? == 24 && u32_at(30)? == 0;
    (uncompressed_24 && w > 0 && h != 0 && h != i32::MIN).then_some(BmpRows {
        offset: offset.into(),
        w: w as u32,
        h: h.unsigned_abs(),
        top_down: h < 0,
    })
}

/// An uncompressed 24-bit BMP streamed a row at a time.
fn bmp(file: File, rows: BmpRows, max: (u32, u32)) -> Result<Canvas, Unread> {
    let BmpRows { w, h, .. } = rows;
    fits(w, h, 3)?;
    let mut r = BufReader::new(file);
    r.seek(SeekFrom::Start(rows.offset))
        .map_err(|_| Unread::Unreadable)?;
    let mut row = vec![0; (w as usize * 3).next_multiple_of(4)];
    let mut shrink = Shrink::new(w, h, max);
    for i in 0..h {
        r.read_exact(&mut row).map_err(|_| Unread::Unreadable)?;
        let y = if rows.top_down { i } else { h - 1 - i };
        let px = row.as_chunks::<3>().0.iter().take(w as usize);
        shrink.row(y, (0..).zip(px.map(|&[b, g, r]| [r, g, b, 0xff])));
    }
    Ok(shrink.canvas())
}

/// The `format` picture decoded whole by the image crate under the compositor's limits.
fn whole(file: File, format: image::ImageFormat, max: (u32, u32)) -> Result<Canvas, Unread> {
    use image::GenericImageView;
    let mut reader = image::ImageReader::with_format(BufReader::new(file), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_BYTES);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| match e {
        image::ImageError::Limits(_) => Unread::TooLarge,
        _ => Unread::Unreadable,
    })?;
    let mut shrink = Shrink::new(img.width(), img.height(), max);
    for y in 0..img.height() {
        let px = (0..img.width()).map(|x| (x, img.get_pixel(x, y).0));
        shrink.row(y, px);
    }
    Ok(shrink.canvas())
}

/// A box downscale of a `sw` x `sh` picture to fit a maximum, fed its rows in any order:
/// each output pixel the premultiplied mean of the pixels it covers.
struct Shrink {
    w: u32,
    sh: u32,
    h: u32,
    /// The output column of each input column.
    cols: Vec<u32>,
    /// Per output pixel: red, green and blue each times alpha, alpha, and the pixels added.
    sums: Vec<[u64; 5]>,
}

impl Shrink {
    fn new(sw: u32, sh: u32, max: (u32, u32)) -> Shrink {
        let (w, h) = reprisekit::raster::fit_size(sw, sh, max);
        let cols = (0..sw)
            .map(|x| (u64::from(x) * u64::from(w) / u64::from(sw)) as u32)
            .collect();
        Shrink {
            w,
            sh,
            h,
            cols,
            sums: vec![[0; 5]; w as usize * h as usize],
        }
    }

    /// Row `y`'s pixels, each its column and straight RGBA.
    fn row(&mut self, y: u32, px: impl Iterator<Item = (u32, [u8; 4])>) {
        let out = (u64::from(y) * u64::from(self.h) / u64::from(self.sh)) as usize;
        let line = &mut self.sums[out * self.w as usize..][..self.w as usize];
        for (x, [r, g, b, a]) in px {
            let Some(&col) = self.cols.get(x as usize) else {
                continue;
            };
            let s = &mut line[col as usize];
            let a64 = u64::from(a);
            s[0] += u64::from(r) * a64;
            s[1] += u64::from(g) * a64;
            s[2] += u64::from(b) * a64;
            s[3] += a64;
            s[4] += 1;
        }
    }

    fn canvas(self) -> Canvas {
        let px = (self.sums.iter())
            .map(|&[r, g, b, a, n]| {
                let n = n.max(1);
                let c = |v: u64| ((v + n * 255 / 2) / (n * 255)) as u32;
                let a = ((a + n / 2) / n) as u32;
                a << 24 | c(r) << 16 | c(g) << 8 | c(b)
            })
            .collect();
        Canvas::from_pixels(self.w, self.h, px)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const MAX: (u32, u32) = (304, 224);

    fn file_of(bytes: &[u8]) -> File {
        let mut f = tempfile::tempfile().unwrap();
        f.write_all(bytes).unwrap();
        f.seek(SeekFrom::Start(0)).unwrap();
        f
    }

    fn encoded(img: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), format)
            .unwrap();
        out
    }

    /// The installed reprise-decode is never run here.
    fn no_helper(_: &[u8]) -> Option<HelperDecoder> {
        None
    }

    #[test]
    fn a_small_picture_of_each_format_previews_at_its_own_size() {
        let rgb =
            image::RgbImage::from_fn(6, 4, |x, y| image::Rgb([x as u8 * 40, y as u8 * 60, 9]));
        let rgb = image::DynamicImage::ImageRgb8(rgb);
        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Bmp,
            image::ImageFormat::Jpeg,
        ] {
            let c = preview_by(file_of(&encoded(rgb.clone(), format)), MAX, no_helper).unwrap();
            assert_eq!((c.width, c.height), (6, 4), "{format:?}");
        }
        let png = encoded(rgb.clone(), image::ImageFormat::Png);
        let c = preview_by(file_of(&png), MAX, no_helper).unwrap();
        assert_eq!(c.get(5, 3), 0xff00_0000 | 200 << 16 | 180 << 8 | 9);
    }

    #[test]
    fn a_half_transparent_png_previews_premultiplied() {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 100, 0, 128]));
        let png = encoded(
            image::DynamicImage::ImageRgba8(img),
            image::ImageFormat::Png,
        );
        let c = preview_by(file_of(&png), MAX, no_helper).unwrap();
        assert_eq!(c.get(0, 0), reprisekit::icon::premultiply(0x80c8_6400));
    }

    fn crc(bytes: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in bytes {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
        }
        !c
    }

    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend((data.len() as u32).to_be_bytes());
        let body: Vec<u8> = kind.iter().chain(data).copied().collect();
        out.extend(&body);
        out.extend(crc(&body).to_be_bytes());
    }

    /// `img` as an Adam7-interlaced RGB PNG, its passes unfiltered in one stored deflate block.
    fn interlaced_png(img: &image::RgbImage) -> Vec<u8> {
        let (w, h) = img.dimensions();
        let mut raw = Vec::new();
        for (step_x, off_x, step_y, off_y) in ADAM7 {
            let xs: Vec<u32> = (off_x..w).step_by(step_x as usize).collect();
            for y in (off_y..h)
                .step_by(step_y as usize)
                .filter(|_| !xs.is_empty())
            {
                raw.push(0);
                raw.extend(xs.iter().flat_map(|&x| img.get_pixel(x, y).0));
            }
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &byte in &raw {
            a = (a + u32::from(byte)) % 65521;
            b = (b + a) % 65521;
        }
        let len = raw.len() as u16;
        let mut z = vec![0x78, 0x01, 0x01];
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend(&raw);
        z.extend((b << 16 | a).to_be_bytes());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend(w.to_be_bytes());
        ihdr.extend(h.to_be_bytes());
        ihdr.extend([8, 2, 0, 0, 1]);
        chunk(&mut png, b"IHDR", &ihdr);
        chunk(&mut png, b"IDAT", &z);
        chunk(&mut png, b"IEND", &[]);
        png
    }

    #[test]
    fn an_interlaced_png_previews_every_pixel_in_its_place() {
        let img =
            image::RgbImage::from_fn(13, 11, |x, y| image::Rgb([x as u8 * 19, y as u8 * 23, 7]));
        let plain = encoded(
            image::DynamicImage::ImageRgb8(img.clone()),
            image::ImageFormat::Png,
        );
        let plain = preview_by(file_of(&plain), MAX, no_helper).unwrap();
        let laced = preview_by(file_of(&interlaced_png(&img)), MAX, no_helper).unwrap();
        assert_eq!(laced, plain);
        assert_eq!(plain.get(12, 10), 0xff00_0000 | 228 << 16 | 230 << 8 | 7);
    }

    /// A baseline grey JPEG of `w` x `h` whose every block is mid grey, written directly: one
    /// one-bit Huffman code for a DC difference of 0 and one for end of block, so its entropy data
    /// is two zero bits a block.
    fn grey_jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut j = vec![0xff, 0xd8];
        j.extend([0xff, 0xdb, 0, 67, 0]);
        j.extend([1; 64]);
        j.extend([0xff, 0xc0, 0, 11, 8]);
        j.extend(h.to_be_bytes());
        j.extend(w.to_be_bytes());
        j.extend([1, 1, 0x11, 0]);
        for class in [0x00, 0x10] {
            j.extend([0xff, 0xc4, 0, 20, class, 1]);
            j.extend([0; 15]);
            j.push(0);
        }
        j.extend([0xff, 0xda, 0, 8, 1, 1, 0, 0, 63, 0]);
        let blocks = u64::from(w.div_ceil(8)) * u64::from(h.div_ceil(8));
        j.resize(j.len() + (blocks * 2).div_ceil(8) as usize, 0);
        j.extend([0xff, 0xd9]);
        j
    }

    #[test]
    fn a_61_megapixel_jpeg_previews_from_an_eighth_scale_decode() {
        let jpeg = grey_jpeg(9600, 6400);
        let c = preview_by(file_of(&jpeg), MAX, no_helper).unwrap();
        assert_eq!((c.width, c.height), (304, 203));
        assert!(c.pixels.iter().all(|&p| p == 0xff80_8080), "mid grey");
    }

    #[test]
    fn a_48_megapixel_png_previews_shrunk_as_it_streams() {
        let (w, h) = (8000, 6000);
        let mut file = tempfile::tempfile().unwrap();
        {
            let out = std::io::BufWriter::new(file.try_clone().unwrap());
            let mut enc = png::Encoder::new(out, w, h);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_compression(png::Compression::Fastest);
            let mut writer = enc.write_header().unwrap().into_stream_writer().unwrap();
            let row: Vec<u8> = (0..w).map(|x| if x < w / 2 { 0 } else { 255 }).collect();
            for _ in 0..h {
                writer.write_all(&row).unwrap();
            }
            writer.finish().unwrap();
        }
        file.seek(SeekFrom::Start(0)).unwrap();
        let c = preview_by(file, MAX, no_helper).unwrap();
        assert_eq!((c.width, c.height), (299, 224));
        assert_eq!((c.get(0, 0), c.get(298, 223)), (0xff00_0000, 0xffff_ffff));
    }

    /// A reprise-decode stand-in in `dir` saving its arguments to `seen`, replying a 2x1 picture.
    fn fake_helper(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("reprise-decode");
        let reply = r"\000\002\000\000\000\001\000\000\000\377\000\000\377\000\000\377\000";
        let seen = dir.join("seen");
        let script = format!(
            "#!/bin/sh\necho \"$@\" > '{}'\ncat >/dev/null\nprintf '{reply}'\n",
            seen.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A JPEG XL codestream whose size header states a height of `h` and the width by `ratio`'s
    /// code (1 for 1:1, 3 for 4:3).
    fn jxl_header(h: u32, ratio: u8) -> Vec<u8> {
        // SizeHeader: small = 0, height in its 18-bit form (selector 2), then the 3-bit ratio.
        let mut bits: u64 = 0;
        let mut at = 0;
        let mut put = |v: u64, n: u32| {
            bits |= v << at;
            at += n;
        };
        put(0, 1);
        put(2, 2);
        put(u64::from(h - 1), 18);
        put(u64::from(ratio), 3);
        let mut out = vec![0xff, 0x0a];
        out.extend(bits.to_le_bytes().into_iter().take(4));
        out
    }

    #[test]
    fn a_33_megapixel_jpeg_xl_reaches_reprise_decode_with_the_desktops_limits() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake_helper(dir.path());
        let jxl = jxl_header(5000, 3);
        assert_eq!(reprisefs::imagesize::dimensions(&jxl), Some((6666, 5000)));
        let helper = |head: &[u8]| {
            let format = repriseapp::decode::helper_format(head)?;
            Some(HelperDecoder::at(format, program.clone()))
        };
        let c = preview_by(file_of(&jxl), MAX, helper).unwrap();
        assert_eq!((c.width, c.height), (2, 1));
        let seen = std::fs::read_to_string(dir.path().join("seen")).unwrap();
        assert_eq!(
            seen.trim(),
            format!("--file {MAX_BYTES} --pixels {MAX_BYTES} --deadline 60")
        );
    }

    #[test]
    fn a_picture_past_the_desktops_limits_cannot_be_shown() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake_helper(dir.path());
        let jxl = jxl_header(20_000, 1);
        let helper = |head: &[u8]| {
            let format = repriseapp::decode::helper_format(head)?;
            Some(HelperDecoder::at(format, program.clone()))
        };
        let refused = preview_by(file_of(&jxl), MAX, helper);
        assert_eq!(refused.err(), Some(Unread::TooLarge), "20000x20000 JPEG XL");
        assert!(!dir.path().join("seen").exists(), "refused by its header");
        let header = |w, h, color, depth| {
            let mut png = Vec::new();
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            let mut writer = enc.write_header().unwrap();
            writer.write_chunk(png::chunk::IDAT, &[]).unwrap();
            drop(writer);
            png
        };
        use png::{BitDepth, ColorType};
        for (w, h, color, depth) in [
            (16385, 8, ColorType::Grayscale, BitDepth::Eight),
            (9000, 9000, ColorType::Rgba, BitDepth::Sixteen),
            (12000, 12000, ColorType::Rgba, BitDepth::Eight),
        ] {
            let png = header(w, h, color, depth);
            let got = preview_by(file_of(&png), MAX, no_helper).err();
            assert_eq!(got, Some(Unread::TooLarge), "{w}x{h} {color:?} {depth:?}");
        }
        let png = header(8000, 6000, ColorType::Rgba, BitDepth::Sixteen);
        let got = preview_by(file_of(&png), MAX, no_helper).err();
        assert_eq!(
            got,
            Some(Unread::Unreadable),
            "8000x6000 RGBA16 fits; no rows"
        );
        let jpeg = grey_jpeg(16400, 8);
        let got = preview_by(file_of(&jpeg), MAX, no_helper).err();
        assert_eq!(got, Some(Unread::TooLarge), "a JPEG 16400 wide");
    }

    #[test]
    fn a_bottom_up_bmp_streams_its_rows_flipped() {
        let img =
            image::RgbImage::from_fn(3, 2, |x, y| image::Rgb([x as u8 * 50, y as u8 * 90, 1]));
        let bmp = encoded(image::DynamicImage::ImageRgb8(img), image::ImageFormat::Bmp);
        let rows = bmp_rows(&bmp).expect("an uncompressed 24-bit BMP");
        assert!(!rows.top_down);
        let c = preview_by(file_of(&bmp), MAX, no_helper).unwrap();
        assert_eq!(c.get(2, 1), 0xff00_0000 | 100 << 16 | 90 << 8 | 1);
        assert_eq!(c.get(0, 0), 0xff00_0001);
    }

    #[test]
    fn an_unreadable_picture_cannot_be_shown() {
        for bytes in [&b"\x89PNG\r\n\x1a\n cut short"[..], b"GIF89a", b""] {
            let got = preview_by(file_of(bytes), MAX, no_helper).err();
            assert_eq!(got, Some(Unread::Unreadable), "{bytes:?}");
        }
    }
}
