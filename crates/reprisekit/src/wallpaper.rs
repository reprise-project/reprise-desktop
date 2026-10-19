//! A wallpaper on a screen: Center at its own size, Tile from the top-left or Stretch to fill, composed over the desktop colour. The compositor composes each output with it, Reprise Settings its monitor preview.

use std::borrow::Cow;

use crate::raster::Canvas;
use crate::theme::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Position {
    Center,
    Tile,
    #[default]
    Stretch,
}

impl Position {
    pub fn parse(s: &str) -> Option<Position> {
        match s.to_ascii_lowercase().as_str() {
            "center" => Some(Position::Center),
            "tile" => Some(Position::Tile),
            "stretch" => Some(Position::Stretch),
            _ => None,
        }
    }
}

/// `img` placed on a `w` x `h` canvas of `bg` by `position`, borrowing `img` when it already is the
/// result: an opaque picture stretched to its own size. `scale` is the image's draw size relative
/// to its own for Center and Tile; Stretch ignores it.
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
        Position::Stretch => {
            let fitted = if (img.width, img.height, img.scale()) == (w, h, 1) {
                Cow::Borrowed(img)
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
        assert_eq!(Position::parse("fill"), None);
        assert_eq!(Position::default(), Position::Stretch);
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
