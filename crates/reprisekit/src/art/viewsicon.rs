//! The Views button's icon, from the PNGs in [`icons`]: a small window, navy title strip over a
//! white body holding a 2x2 grid of coloured tiles. It comes in two
//! sizes: 24x24 for Explorer's big toolbar and 16x16 for the file
//! dialog's small one. A host draws it when the icon theme carries no
//! view icon of its own.

use crate::art::icons;
use crate::raster::Canvas;

/// The 24x24 icon, transparent around the window: it spans x 2..=21,
/// y 3..=20; each tile is 6x5 with a 2 px gutter.
pub fn draw24() -> Canvas {
    icons::VIEWS_24.canvas()
}

/// The 16x16 icon, transparent around the window: it spans x 1..=14,
/// y 1..=14; each tile is 4x3 with a 2 px gutter.
pub fn draw16() -> Canvas {
    icons::VIEWS_16.canvas()
}

/// [`draw16`] on a canvas at `scale`, for a host drawing in device pixels.
pub fn draw16_at(scale: i32) -> Canvas {
    icons::VIEWS_16.canvas_at(scale)
}

/// [`draw24`] `px` logical px across on a canvas at `scale`: the big strip's icon grown with the
/// text. Drawn at the whole multiple of 24 px at or above `px * scale` device px, then
/// box-resampled down to it.
pub fn draw24_grown(px: i32, scale: i32) -> Canvas {
    let s = scale.max(1);
    let dev = px.max(1) * s;
    let k = (dev + 23) / 24;
    let art = icons::VIEWS_24.canvas_at(k).at_scale(1);
    let sized = if dev == 24 * k {
        art
    } else {
        art.resample_box(dev as u32, dev as u32)
    };
    sized.at_scale(s)
}

/// [`draw16`] at the whole multiple of 16 px nearest `px`, at least 1: the small strip's icon
/// grown with the text and the scale.
pub fn draw16_near(px: i32) -> Canvas {
    draw16_at(((px + 8) / 16).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    #[test]
    fn the_grown_big_icon_is_as_wide_as_asked_and_the_24_px_one_at_100() {
        let at100 = draw24_grown(24, 1);
        assert_eq!((at100.width, at100.scale()), (24, 1));
        assert_eq!(
            at100.pixels,
            draw24().pixels,
            "the 24 px art itself at 100%"
        );
        let grown = draw24_grown(37, 1);
        assert_eq!((grown.width, grown.height), (37, 37));
        let (x0, x1) = ink_box(&grown).0;
        assert!(
            x1 - x0 >= 29,
            "the window spans the grown icon, got {x0}..={x1}"
        );
        let doubled = draw24_grown(24, 2);
        assert_eq!((doubled.width, doubled.device_size()), (24, (48, 48)));
        assert_eq!(draw24_grown(37, 2).device_size(), (74, 74));
    }

    const OUTLINE: Color = Color::rgb(64, 64, 64);
    const TITLE: Color = Color::rgb(0, 0, 128);
    const BODY: Color = Color::rgb(255, 255, 255);
    const TILE_Y: Color = Color::rgb(255, 204, 0);
    const TILE_B: Color = Color::rgb(58, 110, 165);
    const TILE_G: Color = Color::rgb(0, 128, 0);
    const TILE_R: Color = Color::rgb(192, 0, 0);

    fn px(c: &Canvas, x: i32, y: i32) -> Option<Color> {
        let v = c.get(x, y);
        (v >> 24 != 0).then(|| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    fn ink_box(c: &Canvas) -> ((i32, i32), (i32, i32)) {
        let n = c.width as i32;
        let inked: Vec<(i32, i32)> = (0..n)
            .flat_map(|y| (0..n).map(move |x| (x, y)))
            .filter(|&(x, y)| px(c, x, y).is_some())
            .collect();
        let xs = inked.iter().map(|p| p.0);
        let ys = inked.iter().map(|p| p.1);
        (
            (xs.clone().min().unwrap(), xs.max().unwrap()),
            (ys.clone().min().unwrap(), ys.max().unwrap()),
        )
    }

    #[test]
    fn the_window_is_centred_in_its_box() {
        assert_eq!(
            ink_box(&draw24()),
            ((2, 21), (3, 20)),
            "24 px: x 2..=21, y 3..=20"
        );
        assert_eq!(
            ink_box(&draw16()),
            ((1, 14), (1, 14)),
            "16 px: x 1..=14, y 1..=14"
        );
    }

    #[test]
    fn title_strip_over_four_tiles_at_24_px() {
        let c = draw24();
        assert_eq!(px(&c, 2, 3), Some(OUTLINE), "outline at the corner");
        assert_eq!(px(&c, 3, 4), Some(TITLE), "title strip at y 4");
        assert_eq!(px(&c, 5, 7), Some(TILE_Y), "top-left tile from (5, 7)");
        assert_eq!(px(&c, 13, 7), Some(TILE_B), "top-right tile from (13, 7)");
        assert_eq!(px(&c, 5, 14), Some(TILE_G), "bottom-left tile from (5, 14)");
        assert_eq!(
            px(&c, 18, 18),
            Some(TILE_R),
            "bottom-right tile to (18, 18)"
        );
        assert_eq!(px(&c, 11, 10), Some(BODY), "white gutter at x 11..=12");
    }

    #[test]
    fn at_scale_2_each_pixel_is_a_2x2_cell() {
        let (one, two) = (draw16_at(1), draw16_at(2));
        assert_eq!(one.pixels, draw16().pixels, "scale 1 is the 16 px icon");
        assert_eq!((two.width, two.height, two.scale()), (16, 16, 2));
        assert_eq!(two.device_size(), (32, 32));
        for (i, &p) in two.pixels.iter().enumerate() {
            let (x, y) = ((i % 32) as i32, (i / 32) as i32);
            assert_eq!(p, one.get(x / 2, y / 2), "device pixel ({x}, {y})");
        }
    }

    #[test]
    fn title_strip_over_four_tiles_at_16_px() {
        let c = draw16();
        assert_eq!(px(&c, 1, 1), Some(OUTLINE), "outline at the corner");
        assert_eq!(px(&c, 2, 3), Some(TITLE), "title strip to y 3");
        assert_eq!(px(&c, 3, 5), Some(TILE_Y), "top-left tile from (3, 5)");
        assert_eq!(px(&c, 9, 5), Some(TILE_B), "top-right tile from (9, 5)");
        assert_eq!(px(&c, 3, 10), Some(TILE_G), "bottom-left tile from (3, 10)");
        assert_eq!(
            px(&c, 12, 12),
            Some(TILE_R),
            "bottom-right tile to (12, 12)"
        );
        assert_eq!(px(&c, 7, 6), Some(BODY), "white gutter at x 7..=8");
    }
}
