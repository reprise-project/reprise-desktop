//! The Bluetooth tray icon, 16x16: a rounded badge with the white rune, blue
//! while the radio is on and grey while off or blocked. `draw32` is it doubled, for the wizard's header.

use crate::art::icons;
use crate::raster::Canvas;
use crate::theme::Color;

pub const SIZE: u32 = 16;

const OUTLINE_ON: Color = Color::rgb(0, 0, 128);
const FILL_ON: Color = Color::rgb(0, 88, 208);
const OUTLINE_OFF: Color = Color::rgb(128, 128, 128);
const FILL_OFF: Color = Color::rgb(168, 168, 168);

/// The badge: a rounded rectangle with the white rune; the PNG holds the on inks and [`draw`] swaps them for off.
pub fn draw(on: bool) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    icons::BLUETOOTH.paint_inked(&mut c, 0, 0, 1, |ink| {
        Some(match ink {
            OUTLINE_ON if !on => OUTLINE_OFF,
            FILL_ON if !on => FILL_OFF,
            ink => ink,
        })
    });
    c
}

/// [`draw`] scaled 2x: every source pixel becomes a 2x2 block, alpha included.
pub fn draw32(on: bool) -> Canvas {
    draw(on).resample_box(SIZE * 2, SIZE * 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    const RUNE: Color = Color::rgb(255, 255, 255);

    fn px(c: &Canvas, x: i32, y: i32) -> Option<Color> {
        let v = c.get(x, y);
        (v >> 24 != 0).then(|| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    #[test]
    fn on_is_a_blue_badge_with_a_white_rune() {
        let c = draw(true);
        assert_eq!(px(&c, 5, 0), Some(OUTLINE_ON), "the badge's top edge");
        assert_eq!(px(&c, 4, 7), Some(FILL_ON), "fill left of the rune");
        assert_eq!(px(&c, 7, 2), Some(RUNE), "the stem's top");
        assert_eq!(px(&c, 9, 4), Some(RUNE), "the upper arrow's point");
        assert_eq!(px(&c, 7, 12), Some(RUNE), "the stem's foot");
        assert_eq!(px(&c, 0, 0), None, "the corners are transparent");
        assert_eq!(px(&c, 15, 15), None);
    }

    #[test]
    fn off_greys_the_badge_and_keeps_the_rune() {
        let c = draw(false);
        assert_eq!(px(&c, 5, 0), Some(OUTLINE_OFF));
        assert_eq!(px(&c, 4, 7), Some(FILL_OFF));
        assert_eq!(px(&c, 7, 2), Some(RUNE));
    }

    #[test]
    fn draw32_doubles_every_pixel() {
        let (small, big) = (draw(true), draw32(true));
        assert_eq!((big.width, big.height), (32, 32));
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(big.get(2 * x + 1, 2 * y + 1), small.get(x, y), "({x}, {y})");
            }
        }
    }
}
