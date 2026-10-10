//! Wallpaper: decode an image file and compose it into an output's pixels
//! per the Position setting (Center, Tile, Stretch) over the desktop colour.

use crate::raster::Canvas;
use std::path::Path;

pub use reprisekit::wallpaper::{Position, compose};
use reprisekit::wallpapers::{self, Builtin};

/// Largest side accepted; the decoder checks the header so oversized files are refused before allocation.
const MAX_SIDE: u32 = 16384;

fn decode(path: &Path) -> image::ImageResult<image::DynamicImage> {
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    reader.limits(limits);
    reader.decode()
}

/// The part of a picture the outputs can show. `w` x `h` is the largest output in buffer pixels
/// for Stretch, and in layout pixels for Center and Tile.
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
    pictures: Vec<Canvas>,
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
        self.pictures.push(picture);
        true
    }

    pub fn get(&self, size: (u32, u32)) -> Option<&Canvas> {
        self.pictures.iter().find(|p| (p.width, p.height) == size)
    }

    pub fn any(&self) -> Option<&Canvas> {
        self.pictures.first()
    }
}

pub struct Loaded {
    pub image: Canvas,
    pub cut: Option<Cut>,
}

/// Decode `path` (JPEG, PNG or BMP) to a premultiplied canvas reduced to `cut`
/// (see `reduce`), or None with a log line. Only the decoder's buffer is held at full size.
pub fn load(path: &Path, cut: Option<Cut>) -> Option<Loaded> {
    if let Some(b) = builtin(path) {
        return Some(draw_builtin(b, cut));
    }
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
    let rgba;
    let (bytes, n): (&[u8], usize) = match &img {
        image::DynamicImage::ImageRgb8(b) => (b.as_raw(), 3),
        image::DynamicImage::ImageRgba8(b) => (b.as_raw(), 4),
        other => {
            rgba = other.to_rgba8();
            (rgba.as_raw(), 4)
        }
    };
    let px = |x: u32, y: u32| {
        let i = (y as usize * w as usize + x as usize) * n;
        let a = if n == 4 { bytes[i + 3] } else { 0xFF };
        let argb = (a as u32) << 24
            | (bytes[i] as u32) << 16
            | (bytes[i + 1] as u32) << 8
            | bytes[i + 2] as u32;
        reprisekit::icon::premultiply(argb)
    };
    Some(reduce(w, h, px, cut))
}

/// Reduce a `sw` x `sh` picture (premultiplied ARGB via `px`) to `cut`: Stretch box-downscales,
/// Tile keeps the top-left, Center keeps the middle, trimming an even count so `compose` centres alike.
fn reduce(sw: u32, sh: u32, px: impl Fn(u32, u32) -> u32, cut: Option<Cut>) -> Loaded {
    let dim = |size: u32, want: u32| match cut {
        Some(c) if size > want.max(1) => {
            let want = want.max(1);
            match c.position {
                Position::Stretch => (0, size, want),
                Position::Tile => (0, want, want),
                Position::Center => {
                    let kept = want + (size - want) % 2;
                    ((size - kept) / 2, kept, kept)
                }
            }
        }
        _ => (0, size, size),
    };
    let (ox, kw, tw) = dim(sw, cut.map_or(0, |c| c.w));
    let (oy, kh, th) = dim(sh, cut.map_or(0, |c| c.h));
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
        let loaded = load(&path, Some(cut));
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
}
