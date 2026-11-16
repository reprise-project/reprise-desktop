//! A wallpaper on a screen: Center at its own size, Tile from the top-left, Stretch to the screen's shape, Fill to cover it evenly cropped, or Fit whole, composed over the desktop colour. The compositor composes each output with it, Reprise Settings its monitor preview.

use std::borrow::Cow;

use crate::raster::Canvas;
use crate::theme::Color;

/// The largest side of a wallpaper picture the desktop shows, in pixels.
pub const MAX_SIDE: u32 = 16384;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    Center,
    Tile,
    Stretch,
    /// Scaled evenly to cover the screen, the overflow cropped on both sides.
    #[default]
    Fill,
    /// Scaled evenly to fit whole, centred on the desktop colour.
    Fit,
}

impl Position {
    pub fn parse(s: &str) -> Option<Position> {
        match s.to_ascii_lowercase().as_str() {
            "center" => Some(Position::Center),
            "tile" => Some(Position::Tile),
            "stretch" => Some(Position::Stretch),
            "fill" => Some(Position::Fill),
            "fit" => Some(Position::Fit),
            _ => None,
        }
    }

    /// Sized to the screen in device pixels, so the draw scale does not apply.
    pub fn scales_to_screen(self) -> bool {
        !matches!(self, Position::Center | Position::Tile)
    }
}

/// The size a `sw` x `sh` picture takes scaled evenly to cover `w` x `h`.
pub fn cover_size(sw: u32, sh: u32, w: u32, h: u32) -> (u32, u32) {
    let k = (w as f32 / sw.max(1) as f32).max(h as f32 / sh.max(1) as f32);
    (
        ((sw as f32 * k).round() as u32).max(w),
        ((sh as f32 * k).round() as u32).max(h),
    )
}

/// `img` placed on a `w` x `h` canvas of `bg` by `position`, borrowing `img` when it already is the
/// result: an opaque picture stretched or filled to its own size. `scale` is the image's draw size
/// relative to its own for Center and Tile; the others ignore it.
pub fn composed(
    img: &Canvas,
    w: u32,
    h: u32,
    position: Position,
    bg: Color,
    scale: f32,
) -> Cow<'_, Canvas> {
    composed_known(img, w, h, position, bg, scale, None)
}

/// [`composed`] for an `img` already known to be opaque or not, which spares a stretched picture
/// the scan of its pixels.
pub fn composed_known(
    img: &Canvas,
    w: u32,
    h: u32,
    position: Position,
    bg: Color,
    scale: f32,
    known_opaque: Option<bool>,
) -> Cow<'_, Canvas> {
    let blank = || {
        let mut out = Canvas::new(w, h);
        out.fill_rect(out.bounds(), bg);
        out
    };
    if img.width == 0 || img.height == 0 || w == 0 || h == 0 {
        return Cow::Owned(blank());
    }
    let out = match position {
        Position::Stretch | Position::Fill => {
            let fitted = if (img.width, img.height, img.scale()) == (w, h, 1) {
                Cow::Borrowed(img)
            } else if position == Position::Fill {
                // The middle of the picture in the screen's shape, scaled to the screen.
                let (sw, sh) = img.device_size();
                let (cw, ch) = crate::saver::fit_size(w, h, sw, sh);
                let (ox, oy) = ((sw - cw) / 2, (sh - ch) / 2);
                Cow::Owned(Canvas::resample_box_from(cw, ch, w, h, |x, y| {
                    img.pixels[((oy + y) * sw + ox + x) as usize]
                }))
            } else {
                Cow::Owned(img.resample_box(w, h))
            };
            // The colour never shows under an opaque picture.
            // A box resample keeps every alpha of an opaque picture at 0xFF.
            if known_opaque.map_or_else(|| opaque(&fitted), |o| o && fitted.scale() == 1) {
                return fitted;
            }
            let mut out = blank();
            out.blit_over(0, 0, &fitted);
            out
        }
        Position::Fit => {
            let (sw, sh) = img.device_size();
            let (fw, fh) = crate::saver::fit_size(sw, sh, w, h);
            let fitted =
                Canvas::resample_box_from(sw, sh, fw, fh, |x, y| img.pixels[(y * sw + x) as usize]);
            let mut out = blank();
            out.blit_over((w - fw) as i32 / 2, (h - fh) as i32 / 2, &fitted);
            out
        }
        Position::Center | Position::Tile => {
            let mut out = blank();
            let (sw, sh) = (
                ((img.width as f32 * scale).round() as u32).max(1),
                ((img.height as f32 * scale).round() as u32).max(1),
            );
            let scaled = if (sw, sh) == (img.width, img.height) {
                None
            } else {
                Some(img.resample_box(sw, sh))
            };
            let tile = scaled.as_ref().unwrap_or(img);
            if position == Position::Center {
                out.blit_over((w as i32 - sw as i32) / 2, (h as i32 - sh as i32) / 2, tile);
            } else {
                // One band of tiles across the width, then its rows copied down: every band blends over the same colour.
                let mut band = Canvas::new(w, sh.min(h));
                band.fill_rect(band.bounds(), bg);
                for x in (0..w).step_by(sw as usize) {
                    band.blit_over(x as i32, 0, tile);
                }
                let (w, bh) = (w as usize, band.height as usize);
                for (y, row) in out.pixels.chunks_exact_mut(w).enumerate() {
                    let src = y % bh * w;
                    row.copy_from_slice(&band.pixels[src..src + w]);
                }
            }
            out
        }
    };
    Cow::Owned(out)
}

/// [`composed`] as an owned canvas.
pub fn compose(img: &Canvas, w: u32, h: u32, position: Position, bg: Color, scale: f32) -> Canvas {
    composed(img, w, h, position, bg, scale).into_owned()
}

fn opaque(c: &Canvas) -> bool {
    c.scale() == 1 && c.pixels.iter().all(|&p| p >> 24 == 0xFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;

    fn img() -> Canvas {
        let mut c = Canvas::new(4, 2);
        c.fill_rect(Rect::new(0, 0, 4, 2), Color::rgb(255, 0, 0));
        c
    }

    fn numbered() -> Canvas {
        let mut c = Canvas::new(11, 9);
        for (i, p) in c.pixels.iter_mut().enumerate() {
            *p = 0xFF00_0000 | (i as u32 * 2_000);
        }
        c
    }

    #[test]
    fn a_position_parses_in_any_case() {
        assert_eq!(Position::parse("Center"), Some(Position::Center));
        assert_eq!(Position::parse("tile"), Some(Position::Tile));
        assert_eq!(Position::parse("STRETCH"), Some(Position::Stretch));
        assert_eq!(Position::parse("Fill"), Some(Position::Fill));
        assert_eq!(Position::parse("fit"), Some(Position::Fit));
        assert_eq!(Position::parse("zoom"), None);
        assert_eq!(Position::default(), Position::Fill);
    }

    #[test]
    fn tile_repeats_a_translucent_picture_over_the_colour() {
        let bg = Color::rgb(0, 0, 255);
        let mut tile = numbered();
        tile.pixels[0] = 0x8040_0000;
        let t = compose(&tile, 25, 20, Position::Tile, bg, 1.0);
        let mut one = Canvas::new(11, 9);
        one.fill_rect(one.bounds(), bg);
        one.blit_over(0, 0, &tile);
        for y in 0..20 {
            for x in 0..25 {
                assert_eq!(
                    t.get(x, y),
                    one.get(x % 11, y % 9),
                    "tile pixel at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn stretch_composes_an_opaque_picture_as_its_own_pixels_and_a_translucent_one_over_the_colour()
    {
        let bg = Color::rgb(0, 0, 255);
        let pic = numbered();
        assert_eq!(
            compose(&pic, 11, 9, Position::Stretch, bg, 1.0).pixels,
            pic.pixels,
            "an opaque picture at the screen's size"
        );
        let mut slow = Canvas::new(22, 18);
        slow.fill_rect(slow.bounds(), bg);
        slow.blit_over(0, 0, &pic.resample_box(22, 18));
        assert_eq!(
            compose(&pic, 22, 18, Position::Stretch, bg, 1.0).pixels,
            slow.pixels,
            "an opaque picture stretched"
        );
        let mut glassy = pic.clone();
        glassy.pixels[5] = 0x8040_0000;
        let mut over = Canvas::new(11, 9);
        over.fill_rect(over.bounds(), bg);
        over.blit_over(0, 0, &glassy);
        let c = compose(&glassy, 11, 9, Position::Stretch, bg, 1.0);
        assert_eq!(c.pixels, over.pixels, "a translucent picture");
        assert_eq!(c.get(5, 0), 0xFF40_007F, "half red over blue");
        assert!(matches!(
            composed(&pic, 11, 9, Position::Stretch, bg, 1.0),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            composed(&glassy, 11, 9, Position::Stretch, bg, 1.0).pixels,
            over.pixels
        );
    }

    #[test]
    fn a_picture_known_opaque_or_not_is_not_scanned_again() {
        let bg = Color::rgb(0, 0, 255);
        let mut glassy = numbered();
        glassy.pixels[5] = 0x8040_0000;
        let known = |c, o| composed_known(c, 11, 9, Position::Stretch, bg, 1.0, Some(o));
        assert!(
            matches!(known(&glassy, true), Cow::Borrowed(_)),
            "taken as opaque on the word of the caller"
        );
        assert!(
            matches!(known(&numbered(), false), Cow::Owned(_)),
            "blended over the colour on the word of the caller"
        );
    }

    /// A `w` x `h` picture of red with a green column at x 0 and a white one at the last x.
    fn edged(w: u32, h: u32) -> Canvas {
        let mut c = Canvas::new(w, h);
        c.fill_rect(c.bounds(), Color::rgb(255, 0, 0));
        c.fill_rect(Rect::new(0, 0, 1, h as i32), Color::rgb(0, 255, 0));
        c.fill_rect(
            Rect::new(w as i32 - 1, 0, 1, h as i32),
            Color::rgb(255, 255, 255),
        );
        c
    }

    #[test]
    fn fill_covers_the_screen_evenly_and_crops_the_overflow_on_both_sides() {
        let bg = Color::rgb(0, 0, 255);
        let red = Color::rgb(255, 0, 0).argb();
        let portrait = compose(&numbered(), 20, 10, Position::Fill, bg, 1.0);
        let middle = numbered();
        // 11x9 covering 20x10 shows the middle 11x6 of the source, rows 1 to 6.
        assert_eq!(portrait.get(0, 0), middle.get(0, 1), "the top row is cut");
        assert_eq!(portrait.get(19, 9), middle.get(10, 6), "and the bottom");
        let wide = compose(&edged(40, 10), 10, 10, Position::Fill, bg, 1.0);
        assert!(
            (0..10).all(|y| (0..10).all(|x| wide.get(x, y) == red)),
            "the green and white sides are cropped, nothing of the colour shows"
        );
        let tall = compose(&edged(10, 40), 20, 10, Position::Fill, bg, 1.0);
        assert_eq!(
            tall.get(0, 5),
            Color::rgb(0, 255, 0).argb(),
            "the sides stay"
        );
        assert_eq!(tall.get(19, 5), Color::rgb(255, 255, 255).argb());
        assert_eq!(tall.get(10, 0), red);
        let same = edged(20, 10);
        assert!(
            matches!(
                composed(&same, 20, 10, Position::Fill, bg, 1.0),
                Cow::Borrowed(_)
            ),
            "a picture of the screen's size is its own fill"
        );
    }

    #[test]
    fn fit_shows_the_whole_picture_evenly_on_the_colour() {
        let bg = Color::rgb(0, 0, 255).argb();
        let red = Color::rgb(255, 0, 0).argb();
        let portrait = compose(
            &edged(10, 20),
            40,
            20,
            Position::Fit,
            Color::rgb(0, 0, 255),
            1.0,
        );
        // 10x20 fits 40x20 as 10x20, bands of 15 each side.
        assert_eq!(portrait.get(14, 10), bg, "left band");
        assert_eq!(portrait.get(15, 10), Color::rgb(0, 255, 0).argb());
        assert_eq!(portrait.get(20, 0), red);
        assert_eq!(portrait.get(24, 19), Color::rgb(255, 255, 255).argb());
        assert_eq!(portrait.get(25, 10), bg, "right band");
        let wide = compose(
            &edged(40, 10),
            80,
            80,
            Position::Fit,
            Color::rgb(0, 0, 255),
            1.0,
        );
        // 40x10 fits 80x80 as 80x20, bands of 30 above and below.
        assert_eq!(wide.get(40, 29), bg, "band above");
        assert_eq!(wide.get(40, 30), red);
        assert_eq!(
            wide.get(1, 40),
            Color::rgb(0, 255, 0).argb(),
            "the sides stay"
        );
        assert_eq!(wide.get(78, 49), Color::rgb(255, 255, 255).argb());
        assert_eq!(wide.get(40, 50), bg, "band below");
    }

    #[test]
    fn a_cover_reaches_both_sides_of_the_screen_evenly() {
        assert_eq!(cover_size(400, 800, 1920, 1080), (1920, 3840));
        assert_eq!(cover_size(4000, 3000, 1920, 1080), (1920, 1440));
        assert_eq!(cover_size(30, 20, 30, 20), (30, 20));
        assert_eq!(cover_size(11, 9, 20, 10), (20, 16));
    }

    #[test]
    fn positions_place_the_image() {
        let bg = Color::rgb(0, 0, 255);
        let red = Color::rgb(255, 0, 0).argb();
        let s = compose(&img(), 8, 8, Position::Stretch, bg, 1.0);
        assert_eq!(s.get(0, 0), red);
        assert_eq!(s.get(7, 7), red);
        let c = compose(&img(), 8, 8, Position::Center, bg, 1.0);
        assert_eq!(c.get(0, 0), bg.argb());
        assert_eq!(c.get(2, 3), red);
        assert_eq!(c.get(6, 3), bg.argb());
        let t = compose(&img(), 8, 8, Position::Tile, bg, 1.0);
        assert_eq!(t.get(7, 7), red);
        let half = compose(&img(), 8, 8, Position::Center, bg, 0.5);
        assert_eq!(half.get(3, 3), red);
        assert_eq!(half.get(2, 3), bg.argb());
    }
}
