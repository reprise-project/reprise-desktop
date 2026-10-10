//! Wireless signal strength as five rising bars: five 2 px bars 2 px apart, rising 4, 7, 10,
//! 13 and 16 px from a common bottom, the lit ones green and the rest silver.

use crate::raster::{Canvas, Rect};
use crate::theme::Color;

/// The indicator's box. The five bars and their gaps come to 18 px;
/// [`WIDTH`] leaves the 2 px of air after the last one that a cell in a
/// list needs before the next column's text.
pub const WIDTH: i32 = 20;
pub const HEIGHT: i32 = 16;

const BAR_W: i32 = 2;
const BAR_GAP: i32 = 2;
const BAR_H: [i32; 5] = [4, 7, 10, 13, 16];
const ON: Color = Color::rgb(0x00, 0x80, 0x00);
const OFF: Color = Color::rgb(0xc0, 0xc0, 0xc0);

/// NetworkManager's 0..100 strength to the 0..=5 bars this draws:
/// nothing at 0, then one bar per 20 points, rounded up.
pub fn five(strength: u8) -> u8 {
    if strength == 0 {
        0
    } else {
        (strength as u32).div_ceil(20).min(5) as u8
    }
}

/// The bars at (`x`, `y`), `bars` of them lit, the rest in [`OFF`]. A
/// greyed indicator draws every bar in [`OFF`]: the strength shows only
/// while the list it sits in can act on it.
pub fn draw(c: &mut Canvas, x: i32, y: i32, bars: u8, enabled: bool) {
    for (i, h) in BAR_H.iter().enumerate() {
        let lit = enabled && (i as u8) < bars;
        let color = if lit { ON } else { OFF };
        let bx = x + i as i32 * (BAR_W + BAR_GAP);
        c.fill_rect(Rect::new(bx, y + HEIGHT - h, BAR_W, *h), color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(c: &Canvas, x: i32, y: i32) -> Option<Color> {
        let v = c.get(x, y);
        (v >> 24 != 0).then(|| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    #[test]
    fn five_maps_strength() {
        assert_eq!(five(0), 0);
        assert_eq!(five(1), 1);
        assert_eq!(five(20), 1);
        assert_eq!(five(21), 2);
        assert_eq!(five(100), 5);
    }

    #[test]
    fn fills_up_to_count() {
        const BAR3: (i32, i32) = (8, 6);
        let mut lit = Canvas::new(WIDTH as u32, HEIGHT as u32);
        draw(&mut lit, 0, 0, 3, true);
        let mut dark = Canvas::new(WIDTH as u32, HEIGHT as u32);
        draw(&mut dark, 0, 0, 2, true);
        assert_eq!(px(&lit, BAR3.0, BAR3.1), Some(ON), "the third bar at 3");
        assert_eq!(px(&dark, BAR3.0, BAR3.1), Some(OFF), "and empty at 2");
        assert_eq!(px(&lit, BAR3.0, BAR3.1 - 1), None);
        let mut off = Canvas::new(WIDTH as u32, HEIGHT as u32);
        draw(&mut off, 0, 0, 3, false);
        assert_eq!(px(&off, BAR3.0, BAR3.1), Some(OFF));
    }
}
