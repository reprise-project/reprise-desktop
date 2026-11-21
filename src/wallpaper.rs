//! Wallpaper: decode an image file and compose it into an output's pixels
//! per the Position setting over the desktop colour. JPEG XL and AVIF decode in the confined
//! reprise-decode, the rest here with the image crate.

use crate::raster::Canvas;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use reprise_decode_protocol::client::{self, SLACK};
use reprise_decode_protocol::{Format, Limits, PROGRAM, Request};

use reprisefs::pictures;
use reprisekit::wallpaper::{DECODE_DEADLINE, MAX_BYTES, MAX_SIDE};
pub use reprisekit::wallpaper::{Position, compose, composed_known};
use reprisekit::wallpapers::{self, Builtin};

/// `path` decoded by its content, refused past `MAX_SIDE` a side before it is read whole.
pub fn decode(path: &Path) -> image::ImageResult<image::DynamicImage> {
    decode_with(path, MAX_SIDE, Some(MAX_BYTES))
}

/// `decode` refusing past `max_side` a side and `max_alloc` bytes of decoder memory.
pub fn decode_with(
    path: &Path,
    max_side: u32,
    max_alloc: Option<u64>,
) -> image::ImageResult<image::DynamicImage> {
    limited(image::ImageReader::open(path)?, max_side, max_alloc)
}

/// `reader`'s picture by its content, refused past `max_side` a side and `max_alloc` bytes of
/// decoder memory.
pub fn limited<R: std::io::BufRead + std::io::Seek>(
    reader: image::ImageReader<R>,
    max_side: u32,
    max_alloc: Option<u64>,
) -> image::ImageResult<image::DynamicImage> {
    let mut reader = reader.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_side);
    limits.max_image_height = Some(max_side);
    limits.max_alloc = max_alloc;
    reader.limits(limits);
    reader.decode()
}

/// The part of a picture the outputs can show. `w` x `h` is the largest output in buffer pixels
/// for Stretch, Fill and Fit, and in layout pixels for Center and Tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cut {
    pub position: Position,
    pub w: u32,
    pub h: u32,
}

impl Cut {
    pub fn covers(have: Option<Cut>, want: Cut) -> bool {
        have.is_none_or(|h| h.position == want.position && h.w >= want.w && h.h >= want.h)
    }
}

/// The built-in `path` names (`reprise:design/variant`, see `config::builtin_path`).
pub fn builtin(path: &Path) -> Option<Builtin> {
    Builtin::from_uri(path.to_str()?)?.ok()
}

const BUILTIN_SIZE: (u32, u32) = (1920, 1080);

/// `b` drawn at the cut's size, or `BUILTIN_SIZE` without one (for callers not tied to an output).
fn draw_builtin(b: Builtin, cut: Option<Cut>) -> Loaded {
    let (w, h) = cut.map_or(BUILTIN_SIZE, |c| (c.w.max(1), c.h.max(1)));
    Loaded {
        image: draw(b, w, h),
        cut,
    }
}

/// `b` drawn at `w` x `h` device pixels, through the cache.
pub fn draw(b: Builtin, w: u32, h: u32) -> Canvas {
    let t = std::time::Instant::now();
    let name = wallpapers::cache_name(b, w, h, wallpapers::SEED);
    let dir = reprisefs::drawncache::default_dir();
    let px = reprisefs::drawncache::get_or_make(dir.as_deref(), &name, w, h, || {
        wallpapers::render(b, w, h, wallpapers::SEED).pixels
    });
    log::info!("wallpaper {b} {w}x{h} in {} ms", t.elapsed().as_millis());
    Canvas::from_pixels(w, h, px)
}

/// A built-in's pictures, one per output device size, drawn and being drawn.
#[derive(Default)]
pub struct Drawn {
    /// Shared with the jobs composing Alt+Tab's desktop still.
    pictures: Vec<Arc<Canvas>>,
    drawing: Vec<(u32, u32)>,
}

impl Drawn {
    /// Outputs now show `sizes`: other pictures are dropped; sizes neither drawn nor
    /// being drawn are returned once each and counted as being drawn.
    pub fn want(&mut self, sizes: &[(u32, u32)]) -> Vec<(u32, u32)> {
        self.pictures
            .retain(|p| sizes.contains(&(p.width, p.height)));
        self.drawing.retain(|s| sizes.contains(s));
        let mut new = Vec::new();
        for &s in sizes {
            if self.get(s).is_none() && !self.drawing.contains(&s) {
                self.drawing.push(s);
                new.push(s);
            }
        }
        new
    }

    /// A picture drawn for `want` lands; false (and dropped) when no output wants that size any more.
    pub fn land(&mut self, picture: Canvas) -> bool {
        let size = (picture.width, picture.height);
        let Some(i) = self.drawing.iter().position(|&s| s == size) else {
            return false;
        };
        self.drawing.swap_remove(i);
        self.pictures.push(Arc::new(picture));
        true
    }

    pub fn get(&self, size: (u32, u32)) -> Option<&Arc<Canvas>> {
        self.pictures.iter().find(|p| (p.width, p.height) == size)
    }

    pub fn any(&self) -> Option<&Arc<Canvas>> {
        self.pictures.first()
    }

    /// No size counts as being drawn any more: the drawings under way were for a wallpaper since
    /// replaced, so `want` asks for them again.
    pub fn forget_drawing(&mut self) {
        self.drawing.clear();
    }
}

pub struct Loaded {
    pub image: Canvas,
    pub cut: Option<Cut>,
}

/// Decode `path` (JPEG, PNG, BMP, JPEG XL or AVIF) to a premultiplied canvas reduced to `cut`
/// (see `reduce`), or None with a log line. Under a Stretch, Fill or Fit cut a JPEG decodes at the
/// smallest DCT scale covering it; Center and Tile show pixels 1:1, so they decode at full size.
pub fn load(path: &Path, cut: Option<Cut>, wanted: Wanted) -> Option<Loaded> {
    load_with(path, cut, &Helper::INSTALLED, wanted)
}

fn load_with(path: &Path, cut: Option<Cut>, helper: &Helper, wanted: Wanted) -> Option<Loaded> {
    if let Some(b) = builtin(path) {
        return Some(draw_builtin(b, cut));
    }
    let d = decoded_for(path, cut, helper, wanted)?;
    let mut loaded = reduce(d.w, d.h, |x, y| d.px(x, y), cut);
    if (d.w, d.h) != d.full {
        loaded.cut = cut;
    }
    Some(loaded)
}

fn decoded_for(path: &Path, cut: Option<Cut>, helper: &Helper, wanted: Wanted) -> Option<Decoded> {
    let full = || decoded(path, helper, wanted);
    match cut {
        Some(c) if c.position == Position::Stretch => {
            jpeg_scaled(path, |_, _| (c.w, c.h)).or_else(full)
        }
        Some(c) if matches!(c.position, Position::Fill | Position::Fit) => {
            let cover = |sw, sh| reprisekit::wallpaper::cover_size(sw, sh, c.w, c.h);
            jpeg_scaled(path, cover).or_else(full)
        }
        _ => full(),
    }
}

/// `path` fitted whole into `w` x `h` and centred on black (`saver::fit`). A JPEG decodes at the
/// smallest DCT scale that still covers the fit, so a large photo never exists at full size.
pub fn load_fit(path: &Path, w: u32, h: u32, wanted: Wanted) -> Option<Canvas> {
    let fit = |sw, sh| reprisekit::saver::fit_size(sw, sh, w, h);
    let d = jpeg_scaled(path, fit).or_else(|| decoded(path, &Helper::INSTALLED, wanted))?;
    Some(reprisekit::saver::fit_from(d.w, d.h, w, h, |fw, fh| {
        Canvas::resample_box_from(d.w, d.h, fw, fh, |x, y| d.px(x, y))
    }))
}

/// A decoded picture: `n` bytes a pixel (1 grey, 3 RGB, 4 RGBA), rows packed. `full` is the
/// file's own size, larger than `w` x `h` when decoded at a reduced scale.
struct Decoded {
    w: u32,
    h: u32,
    full: (u32, u32),
    n: usize,
    bytes: Vec<u8>,
}

impl Decoded {
    /// Premultiplied ARGB at `x`, `y`.
    fn px(&self, x: u32, y: u32) -> u32 {
        let i = (y as usize * self.w as usize + x as usize) * self.n;
        let b = &self.bytes[i..i + self.n];
        let (r, g, bl, a) = match self.n {
            1 => (b[0], b[0], b[0], 0xFF),
            3 => (b[0], b[1], b[2], 0xFF),
            _ => (b[0], b[1], b[2], b[3]),
        };
        let argb = (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | bl as u32;
        reprisekit::icon::premultiply(argb)
    }
}

/// The picture at `path` in the format its content names ([`pictures::Format`]), None with a log
/// line for a file of no format the desktop shows, or one that does not decode.
fn decoded(path: &Path, helper: &Helper, wanted: Wanted) -> Option<Decoded> {
    let mut head = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head));
    if let Err(e) = read {
        log::warn!("wallpaper {}: {e}", path.display());
        return None;
    }
    match pictures::Format::of(&head) {
        Some(pictures::Format::Jxl) => helper.decoded(path, Format::Jxl, wanted),
        Some(pictures::Format::Avif) => helper.decoded(path, Format::Avif, wanted),
        Some(_) => image_decoded(path),
        None => {
            log::warn!(
                "wallpaper {}: not a BMP, JPEG, PNG, JPEG XL or AVIF picture",
                path.display()
            );
            None
        }
    }
}

/// How long a decode in reprise-decode may take, and `stop`, which ends it early once it returns
/// true (a newer wallpaper).
#[derive(Clone, Copy)]
pub struct Wanted<'a> {
    pub deadline: Duration,
    pub stop: &'a (dyn Fn() -> bool + Sync),
}

impl Wanted<'static> {
    /// The whole [`DECODE_DEADLINE`], never stopped.
    pub const WHOLE: Wanted<'static> = Wanted {
        deadline: DECODE_DEADLINE,
        stop: &never,
    };
}

fn never() -> bool {
    false
}

/// How JPEG XL and AVIF wallpapers reach reprise-decode.
struct Helper {
    /// None: the installed one, found beside the compositor or in /usr/bin.
    program: Option<PathBuf>,
    /// The most bytes of file, and of decoded RGBA, one wallpaper may take.
    budget: u64,
}

/// The first bytes of a file read for its size header before it goes to reprise-decode.
const HELPER_HEAD: u64 = 64 << 10;

impl Helper {
    const INSTALLED: Helper = Helper {
        program: None,
        budget: MAX_BYTES,
    };

    /// The `format` picture at `path`, None with a log line naming why.
    fn decoded(&self, path: &Path, format: Format, wanted: Wanted) -> Option<Decoded> {
        let name = match format {
            Format::Jxl => "JPEG XL",
            Format::Avif => "AVIF",
        };
        match self.picture(path, format, wanted) {
            Ok(d) => Some(d),
            Err(why) if (wanted.stop)() => {
                log::debug!("wallpaper {}: {name} not decoded: {why}", path.display());
                None
            }
            Err(why) => {
                log::warn!("wallpaper {}: {name} not decoded: {why}", path.display());
                None
            }
        }
    }

    fn picture(&self, path: &Path, format: Format, wanted: Wanted) -> Result<Decoded, String> {
        let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len > self.budget {
            return Err(format!("the file passes {} bytes", self.budget));
        }
        let mut head = Vec::new();
        (&mut file)
            .take(HELPER_HEAD)
            .read_to_end(&mut head)
            .map_err(|e| e.to_string())?;
        if let Some((w, h)) = reprisefs::imagesize::dimensions(&head) {
            self.fits(w, h)?;
        }
        let program = match &self.program {
            Some(p) => p.clone(),
            None => repriseconfine::helper::find(PROGRAM)?,
        };
        let limits = Limits {
            file: self.budget,
            pixels: self.budget,
            deadline: wanted.deadline,
        };
        let request = Request { format, len };
        let input = head.as_slice().chain(file);
        let kill_after = wanted.deadline + SLACK;
        let p = client::decode(&program, request, input, limits, kill_after, wanted.stop)
            .map_err(|no| no.why().to_owned())?;
        let (w, h) = (p.width, p.height);
        self.fits(w, h)?;
        Ok(Decoded {
            w,
            h,
            full: (w, h),
            n: 4,
            bytes: p.into_rgba(),
        })
    }

    /// Refuses a `w` x `h` picture past `MAX_SIDE` a side or the budget.
    fn fits(&self, w: u32, h: u32) -> Result<(), String> {
        if w > MAX_SIDE || h > MAX_SIDE {
            return Err(format!("{w}x{h} passes {MAX_SIDE} a side"));
        }
        if u64::from(w) * u64::from(h) * 4 > self.budget {
            return Err(format!("{w}x{h} passes {} bytes of pixels", self.budget));
        }
        Ok(())
    }
}

fn image_decoded(path: &Path) -> Option<Decoded> {
    let img = match decode(path) {
        Ok(i) => i,
        Err(e) => {
            log::warn!("wallpaper {}: {e}", path.display());
            return None;
        }
    };
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        log::warn!("wallpaper {}: refusing {w}x{h}", path.display());
        return None;
    }
    let (n, bytes) = match img {
        image::DynamicImage::ImageLuma8(b) => (1, b.into_raw()),
        image::DynamicImage::ImageRgb8(b) => (3, b.into_raw()),
        image::DynamicImage::ImageRgba8(b) => (4, b.into_raw()),
        other => (4, other.to_rgba8().into_raw()),
    };
    Some(Decoded {
        w,
        h,
        full: (w, h),
        n,
        bytes,
    })
}

/// A grey or RGB JPEG decoded at the smallest DCT scale (1/8 to 1) whose both sides reach
/// `cover(full width, full height)`; None for anything else, which `decoded` then reads at full size.
fn jpeg_scaled(path: &Path, cover: impl FnOnce(u32, u32) -> (u32, u32)) -> Option<Decoded> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = jpeg_decoder::Decoder::new(std::io::BufReader::new(file));
    dec.read_info().ok()?;
    let info = dec.info()?;
    let (sw, sh) = (info.width as u32, info.height as u32);
    let n = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => 1,
        jpeg_decoder::PixelFormat::RGB24 => 3,
        _ => return None,
    };
    if sw == 0 || sh == 0 || sw > MAX_SIDE || sh > MAX_SIDE {
        return None;
    }
    let (cw, ch) = cover(sw, sh);
    let eighths = |len: u32, k: u32| (len * k).div_ceil(8);
    let k = [1, 2, 4, 8]
        .into_iter()
        .find(|&k| eighths(sw, k) >= cw && eighths(sh, k) >= ch)
        .unwrap_or(8);
    // The decoder takes a scale when either side reaches the request, so ask for both sides at `k`.
    let (dw, dh) = dec
        .scale(eighths(sw, k) as u16, eighths(sh, k) as u16)
        .ok()?;
    let bytes = dec.decode().ok()?;
    let (w, h) = (dw as u32, dh as u32);
    (bytes.len() == w as usize * h as usize * n).then_some(Decoded {
        w,
        h,
        full: (sw, sh),
        n,
        bytes,
    })
}

/// Reduce a `sw` x `sh` picture (premultiplied ARGB via `px`) to `cut`: Stretch box-downscales,
/// Fill and Fit box-downscale evenly to cover it, Tile keeps the top-left, Center keeps the middle,
/// trimming an even count so `compose` centres alike.
fn reduce(sw: u32, sh: u32, px: impl Fn(u32, u32) -> u32, cut: Option<Cut>) -> Loaded {
    let even = cut
        .filter(|c| matches!(c.position, Position::Fill | Position::Fit))
        .map(|c| reprisekit::wallpaper::cover_size(sw, sh, c.w.max(1), c.h.max(1)))
        .filter(|&(cw, ch)| cw < sw && ch < sh);
    let dim = |size: u32, want: u32| match cut {
        Some(c) if size > want.max(1) => {
            let want = want.max(1);
            match c.position {
                Position::Stretch => (0, size, want),
                Position::Fill | Position::Fit => (0, size, size),
                Position::Tile => (0, want, want),
                Position::Center => {
                    let kept = want + (size - want) % 2;
                    ((size - kept) / 2, kept, kept)
                }
            }
        }
        _ => (0, size, size),
    };
    let ((ox, kw, tw), (oy, kh, th)) = match even {
        Some((ew, eh)) => ((0, sw, ew), (0, sh, eh)),
        None => (
            dim(sw, cut.map_or(0, |c| c.w)),
            dim(sh, cut.map_or(0, |c| c.h)),
        ),
    };
    let image = if (kw, kh) == (tw, th) {
        let mut c = Canvas::new(tw, th);
        for y in 0..th {
            for x in 0..tw {
                c.pixels[(y * tw + x) as usize] = px(ox + x, oy + y);
            }
        }
        c
    } else {
        Canvas::resample_box_from(kw, kh, tw, th, |x, y| px(ox + x, oy + y))
    };
    let cut = cut.filter(|_| (tw, th) != (sw, sh));
    Loaded { image, cut }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    #[test]
    fn an_oversized_header_is_refused_before_decoding() {
        fn crc(bytes: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for &b in bytes {
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
        let mut ihdr = b"IHDR".to_vec();
        ihdr.extend_from_slice(&20000u32.to_be_bytes());
        ihdr.extend_from_slice(&20000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc(&ihdr).to_be_bytes());
        let path =
            std::env::temp_dir().join(format!("reprise-wallpaper-{}.png", std::process::id()));
        std::fs::write(&path, &png).unwrap();
        let err = decode(&path).unwrap_err().to_string();
        std::fs::remove_file(&path).unwrap();
        assert!(err.to_lowercase().contains("limit"), "{err}");
    }

    fn numbered() -> Canvas {
        let mut c = Canvas::new(11, 9);
        for (i, p) in c.pixels.iter_mut().enumerate() {
            *p = 0xFF00_0000 | (i as u32 * 2_000);
        }
        c
    }

    fn cut_of(src: &Canvas, cut: Cut) -> Loaded {
        reduce(
            src.width,
            src.height,
            |x, y| src.get(x as i32, y as i32),
            Some(cut),
        )
    }

    #[test]
    fn a_cut_picture_composes_the_same_pixels_on_every_output() {
        let bg = Color::rgb(0, 0, 255);
        let src = numbered();
        for position in [Position::Center, Position::Tile] {
            let reduced = cut_of(
                &src,
                Cut {
                    position,
                    w: 4,
                    h: 3,
                },
            );
            assert!(reduced.image.width < 11 && reduced.image.height < 9);
            assert!(reduced.cut.is_some());
            for (w, h, scale) in [(4, 3, 1), (8, 6, 2), (3, 2, 1)] {
                let whole = compose(&src, w, h, position, bg, scale as f32);
                let cut = compose(&reduced.image, w, h, position, bg, scale as f32);
                assert!(
                    whole.pixels == cut.pixels,
                    "{position:?} on {w}x{h} at {scale}x shows the same pixels cut or whole"
                );
            }
        }
        let stretched = cut_of(
            &src,
            Cut {
                position: Position::Stretch,
                w: 4,
                h: 3,
            },
        );
        assert_eq!((stretched.image.width, stretched.image.height), (4, 3));
    }

    #[test]
    fn fill_and_fit_cuts_keep_the_whole_picture_scaled_evenly_to_cover() {
        let src = numbered();
        for position in [Position::Fill, Position::Fit] {
            let cut = Cut {
                position,
                w: 4,
                h: 2,
            };
            let reduced = cut_of(&src, cut);
            assert_eq!(
                (reduced.image.width, reduced.image.height),
                (4, 3),
                "{position:?}: 11x9 covering 4x2"
            );
            assert_eq!(reduced.cut, Some(cut));
            let whole = cut_of(&src, Cut { w: 20, ..cut });
            assert_eq!(
                whole.image.pixels, src.pixels,
                "{position:?}: nothing to drop"
            );
            assert_eq!(whole.cut, None);
        }
    }

    #[test]
    fn a_picture_within_the_cut_is_kept_whole() {
        let src = numbered();
        let kept = cut_of(
            &src,
            Cut {
                position: Position::Center,
                w: 20,
                h: 9,
            },
        );
        assert_eq!(kept.image.pixels, src.pixels);
        assert_eq!(kept.cut, None, "a whole picture covers every cut");
        let want = Cut {
            position: Position::Tile,
            w: 99,
            h: 99,
        };
        assert!(Cut::covers(None, want));
        assert!(!Cut::covers(
            Some(Cut {
                position: Position::Center,
                ..want
            }),
            want
        ));
        assert!(!Cut::covers(Some(Cut { w: 98, ..want }), want));
    }

    #[test]
    fn an_rgb_file_loads_opaque_and_cut() {
        let mut rgb = image::RgbImage::new(6, 2);
        rgb.put_pixel(0, 0, image::Rgb([10, 20, 30]));
        let path =
            std::env::temp_dir().join(format!("reprise-wallpaper-rgb-{}.png", std::process::id()));
        rgb.save(&path).unwrap();
        let cut = Cut {
            position: Position::Tile,
            w: 4,
            h: 5,
        };
        let loaded = load(&path, Some(cut), Wanted::WHOLE);
        std::fs::remove_file(&path).unwrap();
        let loaded = loaded.unwrap();
        assert_eq!((loaded.image.width, loaded.image.height), (4, 2));
        assert_eq!(loaded.image.get(0, 0), 0xFF0A_141E, "opaque RGB 10, 20, 30");
        assert_eq!(loaded.cut, Some(cut));
    }

    /// Every size `want` returns, drawn without the cache as the wallpaper thread does, and landed.
    fn draw_wanted(d: &mut Drawn, sizes: &[(u32, u32)]) -> Vec<(u32, u32)> {
        let b = builtin(Path::new("reprise:tarn/blue")).unwrap();
        let new = d.want(sizes);
        for &(w, h) in &new {
            assert!(d.land(wallpapers::render(b, w, h, wallpapers::SEED)));
        }
        new
    }

    #[test]
    fn outputs_of_different_aspects_each_get_a_picture_of_their_own_size() {
        let mut d = Drawn::default();
        let (wide, tall) = ((96, 40), (27, 48));
        assert_eq!(draw_wanted(&mut d, &[wide, tall]), [wide, tall]);
        for s in [wide, tall] {
            let p = d.get(s).expect("each output has its own picture");
            assert_eq!((p.width, p.height), s, "drawn at the output's size");
            assert_eq!(p.pixels.len(), (s.0 * s.1) as usize);
        }
    }

    #[test]
    fn outputs_of_the_same_size_share_one_picture() {
        let mut d = Drawn::default();
        let s = (64, 36);
        assert_eq!(draw_wanted(&mut d, &[s, s]), [s], "one render for both");
        assert_eq!(d.pictures.len(), 1);
        assert!(d.want(&[s, s]).is_empty(), "nothing drawn again");
    }

    #[test]
    fn a_mode_change_draws_only_the_new_size_and_drops_the_old() {
        let mut d = Drawn::default();
        let (left, right) = ((64, 36), (40, 32));
        draw_wanted(&mut d, &[left, right]);
        assert_eq!(draw_wanted(&mut d, &[left, (48, 27)]), [(48, 27)]);
        assert!(d.get(right).is_none(), "the old size's picture is dropped");
        assert!(d.get(left).is_some(), "the other output keeps its picture");
        assert_eq!(d.pictures.len(), 2);
    }

    #[test]
    fn a_picture_no_output_wants_any_more_is_dropped_on_landing() {
        let mut d = Drawn::default();
        assert_eq!(d.want(&[(8, 8)]), [(8, 8)]);
        assert_eq!(d.want(&[(16, 9)]), [(16, 9)]);
        assert!(!d.land(Canvas::new(8, 8)), "a stale size is not kept");
        assert!(d.get((8, 8)).is_none());
        assert!(d.land(Canvas::new(16, 9)));
        assert!(d.want(&[(16, 9)]).is_empty());
    }

    fn written(name: &str, format: image::ImageFormat) -> std::path::PathBuf {
        let img = image::RgbImage::from_pixel(64, 48, image::Rgb([0, 0, 255]));
        let path = std::env::temp_dir().join(format!("reprise-fit-{}-{name}", std::process::id()));
        img.save_with_format(&path, format).unwrap();
        path
    }

    #[test]
    fn a_jpeg_decodes_at_the_smallest_dct_scale_covering_the_fit() {
        let path = written("scale.jpg", image::ImageFormat::Jpeg);
        let size = |w, h| {
            jpeg_scaled(&path, |sw, sh| reprisekit::saver::fit_size(sw, sh, w, h))
                .map(|d| (d.w, d.h, d.n))
        };
        let (eighth, half, whole) = (size(8, 6), size(20, 20), size(100, 100));
        std::fs::remove_file(&path).unwrap();
        assert_eq!(eighth, Some((8, 6, 3)), "fit 8x6: 1/8");
        assert_eq!(
            half,
            Some((32, 24, 3)),
            "fit 20x15: 1/4 is 16x12, short of it"
        );
        assert_eq!(whole, Some((64, 48, 3)), "fit 100x75: full size");
    }

    #[test]
    fn a_stretched_jpeg_decodes_at_the_smallest_scale_covering_both_sides() {
        let path = written("stretch.jpg", image::ImageFormat::Jpeg);
        let cut = |position, w, h| Some(Cut { position, w, h });
        let size = |c| {
            decoded_for(&path, c, &Helper::INSTALLED, Wanted::WHOLE).map(|d| (d.w, d.h, d.full))
        };
        let quarter = size(cut(Position::Stretch, 16, 12));
        let wide = size(cut(Position::Stretch, 30, 6));
        let centred = size(cut(Position::Center, 16, 12));
        let filled = size(cut(Position::Fill, 16, 8));
        let fitted = size(cut(Position::Fit, 30, 6));
        let loaded = load(&path, cut(Position::Stretch, 16, 12), Wanted::WHOLE).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(quarter, Some((16, 12, (64, 48))), "16x12 from 1/4");
        assert_eq!(
            wide,
            Some((32, 24, (64, 48))),
            "30 wide needs 1/2 though 6 high fits 1/8"
        );
        assert_eq!(centred, Some((64, 48, (64, 48))), "Center shows pixels 1:1");
        assert_eq!(
            filled,
            Some((16, 12, (64, 48))),
            "covering 16x8 evenly from 1/4"
        );
        assert_eq!(
            fitted,
            Some((32, 24, (64, 48))),
            "covering 30x6 evenly from 1/2"
        );
        assert_eq!((loaded.image.width, loaded.image.height), (16, 12));
        assert_eq!(
            loaded.cut,
            cut(Position::Stretch, 16, 12),
            "a scaled decode covers only its own cut"
        );
    }

    #[test]
    fn a_fitted_photo_is_centred_on_black_from_jpeg_and_png() {
        for (name, format) in [
            ("fit.jpg", image::ImageFormat::Jpeg),
            ("fit.png", image::ImageFormat::Png),
        ] {
            let path = written(name, format);
            let c = load_fit(&path, 16, 16, Wanted::WHOLE).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!((c.width, c.height), (16, 16), "{name}");
            assert_eq!(
                c.get(8, 1),
                0xFF00_0000,
                "{name}: black above the 16x12 picture"
            );
            assert_eq!(c.get(8, 14), 0xFF00_0000, "{name}: black below");
            let mid = c.get(8, 8);
            assert!(
                mid & 0xFF > 0xF0 && mid & 0x00FF_FF00 < 0x0010_1000,
                "{name}: blue, got {mid:08x}"
            );
        }
    }

    /// A fresh folder for one test's files.
    fn folder(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("reprise-wallpaper-{}-{test}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A reprise-decode stand-in in `dir` that leaves `ran` behind and runs `body` after reading
    /// its request.
    fn fake_helper(dir: &Path, body: &str, budget: u64) -> Helper {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("reprise-decode");
        let ran = dir.join("ran");
        let script = format!(
            "#!/bin/sh\ncat >/dev/null\ntouch '{}'\n{body}\n",
            ran.display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Helper {
            program: Some(path),
            budget,
        }
    }

    /// 2x1: opaque red, then half-transparent blue.
    const PICTURE_REPLY: &str =
        r"printf '\000\002\000\000\000\001\000\000\000\377\000\000\377\000\000\377\200'";

    /// A JPEG XL codestream's first bytes, stating no size `imagesize` reads.
    const JXL: &[u8] = &[0xff, 0x0a, 0, 0];
    const AVIF: &[u8] = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf\0\0\x01\x0ameta";

    #[test]
    fn jpeg_xl_and_avif_decode_in_the_helper_and_other_formats_here() {
        let dir = folder("routing");
        let helper = fake_helper(&dir, PICTURE_REPLY, 1 << 20);
        let ran = dir.join("ran");
        for (name, bytes) in [("a.jxl", JXL), ("a.avif", AVIF)] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            let loaded = load_with(&path, None, &helper, Wanted::WHOLE).expect(name);
            assert_eq!((loaded.image.width, loaded.image.height), (2, 1), "{name}");
            assert_eq!(loaded.image.get(0, 0), 0xFFFF_0000, "{name}: opaque red");
            assert_eq!(
                loaded.image.get(1, 0),
                0x8000_0080,
                "{name}: blue premultiplied"
            );
            assert!(ran.exists(), "{name} went to the helper");
            std::fs::remove_file(&ran).unwrap();
        }
        let png = dir.join("a.png");
        image::RgbImage::from_pixel(3, 2, image::Rgb([0, 255, 0]))
            .save(&png)
            .unwrap();
        let loaded = load_with(&png, None, &helper, Wanted::WHOLE).unwrap();
        assert_eq!(loaded.image.get(0, 0), 0xFF00_FF00);
        assert!(!ran.exists(), "a PNG decodes here");
        let named_jxl = dir.join("b.jxl");
        std::fs::copy(&png, &named_jxl).unwrap();
        assert!(load_with(&named_jxl, None, &helper, Wanted::WHOLE).is_some());
        assert!(!ran.exists(), "a file goes by its content, not its name");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_helper_decode_leaves_no_wallpaper_and_says_why() {
        let dir = folder("failed");
        let path = dir.join("a.jxl");
        std::fs::write(&path, JXL).unwrap();
        let missing = Helper {
            program: Some(dir.join("none")),
            budget: 1 << 20,
        };
        assert!(load_with(&path, None, &missing, Wanted::WHOLE).is_none());
        let why = missing
            .picture(&path, Format::Jxl, Wanted::WHOLE)
            .err()
            .unwrap();
        assert!(why.starts_with("cannot start"), "{why}");
        for (body, said) in [
            ("printf '\\002cut short'", "cut short"),
            ("printf '\\003another version'", "another version"),
            ("echo bad >&2; exit 1", "without a reply"),
            ("kill -SYS $$", "seccomp"),
            ("printf '\\001'", "past 1048576 bytes of pixels"),
        ] {
            let helper = fake_helper(&dir, body, 1 << 20);
            assert!(
                load_with(&path, None, &helper, Wanted::WHOLE).is_none(),
                "{body}"
            );
            let why = helper
                .picture(&path, Format::Jxl, Wanted::WHOLE)
                .err()
                .unwrap();
            assert!(why.contains(said), "{body}: {why}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_helper_decode_stops_once_its_picture_is_no_longer_wanted() {
        let dir = folder("stopped");
        let path = dir.join("a.jxl");
        std::fs::write(&path, JXL).unwrap();
        let helper = fake_helper(&dir, "exec sleep 30", 1 << 20);
        let start = std::time::Instant::now();
        let stop = || start.elapsed() > Duration::from_millis(200);
        let wanted = Wanted {
            deadline: DECODE_DEADLINE,
            stop: &stop,
        };
        assert!(load_with(&path, None, &helper, wanted).is_none());
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "stopped well before its 60 s deadline"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_of_no_format_the_desktop_shows_is_refused_undecoded() {
        let dir = folder("formats");
        let helper = fake_helper(&dir, PICTURE_REPLY, 1 << 20);
        for (name, bytes) in [
            ("a.gif", &b"GIF89a\x01\0\x01\0\0\0\0;"[..]),
            ("a.webp", b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f\0\0\0\0"),
        ] {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            assert!(
                load_with(&path, None, &helper, Wanted::WHOLE).is_none(),
                "{name}"
            );
            assert!(!dir.join("ran").exists(), "{name} never reaches the helper");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wallpaper_past_the_budget_is_refused() {
        let dir = folder("budget");
        let ran = dir.join("ran");
        // A codestream whose SizeHeader states 20000x20000.
        let huge = dir.join("huge.jxl");
        std::fs::write(&huge, [0xff, 0x0a, 0xfc, 0x70, 0x22, 0x01]).unwrap();
        let helper = fake_helper(&dir, PICTURE_REPLY, u64::MAX);
        let why = helper
            .picture(&huge, Format::Jxl, Wanted::WHOLE)
            .err()
            .unwrap();
        assert_eq!(why, "20000x20000 passes 16384 a side");
        assert!(
            !ran.exists(),
            "a stated size past the limits is refused unsent"
        );
        let small = dir.join("a.jxl");
        std::fs::write(&small, JXL).unwrap();
        let tight = fake_helper(&dir, PICTURE_REPLY, 7);
        assert!(
            load_with(&small, None, &tight, Wanted::WHOLE).is_none(),
            "8 bytes of pixels past 7"
        );
        assert!(ran.exists());
        std::fs::remove_file(&ran).unwrap();
        let tiny = fake_helper(&dir, PICTURE_REPLY, 3);
        let why = tiny
            .picture(&small, Format::Jxl, Wanted::WHOLE)
            .err()
            .unwrap();
        assert_eq!(why, "the file passes 3 bytes");
        assert!(!ran.exists(), "a file past the budget is not sent");
        assert_eq!(
            Helper::INSTALLED.budget,
            image::Limits::default().max_alloc.unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
