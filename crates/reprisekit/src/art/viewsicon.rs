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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

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
