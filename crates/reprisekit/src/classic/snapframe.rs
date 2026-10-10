//! The drag outline: a dithered frame, opaque black and white on a 1x1 checker ((x + y) % 2),
//! transparent inside. With "Show window contents while dragging" off it is drawn `BAND` wide in place
//! of a window, and by `snapview` over the zone a drag will fill; an edge drag of the taskbar draws it
//! 2 px wide where the bar would land (`bardrag`).

use crate::raster::Canvas;
use crate::theme::Color;

pub const BAND: i32 = 4;

pub const BAR_BAND: i32 = 2;

/// The outline for a `width` x `height` logical rect at `scale`, as four strips (top, bottom, left,
/// right) with their logical offsets inside the rect: four small buffers rather than one mostly transparent canvas.
pub fn strips(width: i32, height: i32, band: i32, scale: i32) -> Vec<((i32, i32), Canvas)> {
    let scale = scale.max(1);
    let band = band.max(1);
    let (w, h) = (width.max(0), height.max(0));
    // A rect narrower or shorter than two bands is all band: the far strips start where the near ones end.
    let (top_h, left_w) = (band.min(h), band.min(w));
    let bottom_y = (h - band).max(top_h);
    let right_x = (w - band).max(left_w);
    let rects = [
        (0, 0, w, top_h),
        (0, bottom_y, w, h - bottom_y),
        (0, top_h, left_w, bottom_y - top_h),
        (right_x, top_h, w - right_x, bottom_y - top_h),
    ];
    rects
        .into_iter()
        .filter(|&(_, _, sw, sh)| sw > 0 && sh > 0)
        .map(|(sx, sy, sw, sh)| {
            (
                (sx, sy),
                checker(sx * scale, sy * scale, sw * scale, sh * scale),
            )
        })
        .collect()
}

/// A `w` x `h` checker whose parity is that of the device pixel at (`ox`, `oy`), so strips cut from one frame keep one pattern.
fn checker(ox: i32, oy: i32, w: i32, h: i32) -> Canvas {
    let mut c = Canvas::new(w as u32, h as u32);
    let black = Color::rgb(0, 0, 0).argb();
    let white = Color::rgb(255, 255, 255).argb();
    for y in 0..h {
        for x in 0..w {
            let v = if (ox + x + oy + y) % 2 == 0 {
                black
            } else {
                white
            };
            c.pixels[(y * w + x) as usize] = v;
        }
    }
    c
}

/// The whole outline in one canvas, transparent inside: the gallery's picture and the strips' reference.
pub fn draw(width: i32, height: i32, band: i32, scale: i32) -> Canvas {
    let scale = scale.max(1);
    let mut c = Canvas::new(
        (width * scale).max(0) as u32,
        (height * scale).max(0) as u32,
    );
    for ((x, y), strip) in strips(width, height, band, scale) {
        c.blit_canvas(x * scale, y * scale, &strip);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkered_band_with_a_transparent_middle() {
        let c = draw(20, 12, BAND, 1);
        assert_eq!((c.width, c.height), (20, 12));
        let black = Color::rgb(0, 0, 0).argb();
        let white = Color::rgb(255, 255, 255).argb();
        assert_eq!(c.get(0, 0), black);
        assert_eq!(c.get(1, 0), white);
        assert_eq!(c.get(0, 1), white);
        assert_eq!(c.get(3, 5), black); // last column of the left band
        assert_eq!(c.get(BAND, 5), 0); // inside: transparent
        assert_eq!(c.get(19, 11), black); // bottom-right corner
        assert_eq!(c.get(10, 11), white); // bottom band, odd sum
    }

    /// The four strips cover exactly the band, at the whole frame's parity, and nothing else.
    #[test]
    fn strips_tile_the_band_and_nothing_more() {
        for (w, h, s) in [(20, 12, 1), (20, 12, 2), (9, 40, 1), (8, 8, 1), (3, 30, 1)] {
            let full = draw(w, h, BAND, s);
            let mut area = 0;
            for ((x, y), strip) in strips(w, h, BAND, s) {
                area += strip.width * strip.height;
                for sy in 0..strip.height as i32 {
                    for sx in 0..strip.width as i32 {
                        assert_eq!(
                            strip.get(sx, sy),
                            full.get(x * s + sx, y * s + sy),
                            "{w}x{h}@{s} strip at ({x}, {y}) pixel ({sx}, {sy})"
                        );
                        assert_ne!(strip.get(sx, sy), 0, "a strip is never transparent");
                    }
                }
            }
            let expect = full.pixels.iter().filter(|&&p| p != 0).count();
            assert_eq!(area as usize, expect, "{w}x{h}@{s} band area");
        }
        assert!(strips(0, 10, BAND, 1).is_empty());
    }

    #[test]
    fn the_band_scales_with_the_output() {
        let c = draw(20, 12, BAND, 2);
        assert_eq!((c.width, c.height), (40, 24));
        assert_ne!(c.get(2 * BAND - 1, 12), 0);
        assert_eq!(c.get(2 * BAND, 12), 0);
    }

    #[test]
    fn a_two_px_band_keeps_the_checker_and_the_strips() {
        let black = Color::rgb(0, 0, 0).argb();
        let white = Color::rgb(255, 255, 255).argb();
        let c = draw(60, 40, 2, 1);
        assert_eq!(c.get(0, 0), black);
        assert_eq!(c.get(1, 0), white);
        assert_eq!(c.get(1, 1), black, "the band is 2 px");
        assert_eq!(c.get(2, 20), 0, "inside a 2 px band: transparent");
        assert_eq!(
            c.get(58, 20),
            black,
            "the right band's first column, even sum"
        );
        assert_eq!(c.get(59, 39), black);
        assert_eq!(c.get(57, 20), 0);
        for (w, h, s) in [(60, 40, 1), (60, 40, 2), (3, 30, 1)] {
            let full = draw(w, h, 2, s);
            let area: u32 = strips(w, h, 2, s)
                .iter()
                .map(|(_, c)| c.width * c.height)
                .sum();
            let expect = full.pixels.iter().filter(|&&p| p != 0).count();
            assert_eq!(
                area as usize, expect,
                "{w}x{h}@{s}: the strips cover the band"
            );
        }
    }
}
