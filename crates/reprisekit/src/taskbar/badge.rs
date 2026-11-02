//! A task button's unread badge drawn from geometry: a rimmed box round a count, which each look
//! colours and rounds in its own module.

use crate::raster::Canvas;
use crate::shape::{self, NUMERAL_H, Px, over, premul};
use crate::theme::Color;

/// One look's badge, in design px and its own colours.
pub struct BadgeArt {
    pub height: i32,
    /// From the rim to the count's first and last columns.
    pub pad_x: i32,
    /// Corner radius; half the height makes a pill.
    pub radius: f64,
    pub rim: (Color, u8),
    /// The fill's top and bottom rows.
    pub fill: [Color; 2],
    /// A light over the fill's upper half, and its opacity; 0 for none.
    pub sheen: (Color, u8),
    pub ink: Color,
}

impl BadgeArt {
    /// The badge's width round `text`: never narrower than it is high.
    pub fn width(&self, text: &str) -> i32 {
        (shape::numerals_width(text) + 2 * self.pad_x).max(self.height)
    }

    pub fn paint(&self, text: &str, d: i32) -> Canvas {
        let (w, h) = (self.width(text), self.height);
        let k = f64::from(d);
        let rim = premul(self.rim.0, self.rim.1);
        let [top, low] = self.fill.map(|c| premul(c, 255));
        let sheen = premul(self.sheen.0, self.sheen.1);
        let ink = premul(self.ink, 255);
        let (tx, ty) = (
            f64::from((w - shape::numerals_width(text)) / 2) * k,
            f64::from((h - NUMERAL_H) / 2) * k,
        );
        let (fw, fh, fr) = (
            f64::from(w - 2) * k,
            f64::from(h - 2) * k,
            (self.radius - 1.0).max(0.0) * k,
        );
        shape::draw(w, h, d, |x, y| {
            let outer = shape::rounded(x, y, f64::from(w) * k, f64::from(h) * k, self.radius * k);
            let (lx, ly) = (x - k, y - k);
            let inner = shape::rounded(lx, ly, fw, fh, fr);
            let t = ((ly + 0.5) / fh).clamp(0.0, 1.0);
            let mut px: Px = shape::mix(top, low, t);
            if ly + 0.5 < fh / 2.0 {
                px = over(sheen, px);
            }
            let body = over(shape::scale(px, inner), shape::scale(rim, outer));
            let count = shape::numerals(x, y, text, tx, ty, d);
            over(shape::scale(ink, count * inner), body)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(radius: f64) -> BadgeArt {
        BadgeArt {
            height: 11,
            pad_x: 3,
            radius,
            rim: (Color::rgb(0x40, 0, 0), 255),
            fill: [Color::rgb(0xF0, 0, 0), Color::rgb(0xA0, 0, 0)],
            sheen: (Color::rgb(0xFF, 0xFF, 0xFF), 0x40),
            ink: Color::rgb(0xFF, 0xFF, 0xFF),
        }
    }

    #[test]
    fn a_badge_is_square_round_one_digit_and_widens_with_the_count() {
        let a = art(5.5);
        assert_eq!(a.width("7"), 11);
        assert_eq!(a.width("42"), 13);
        assert_eq!(a.width("99+"), 17);
        for d in [1, 2] {
            let c = a.paint("99+", d);
            assert_eq!(c.device_size(), ((17 * d) as u32, (11 * d) as u32));
        }
    }

    #[test]
    fn a_pill_clears_its_corners_and_a_square_one_fills_them() {
        let (pill, square) = (art(5.5).paint("5", 1), art(0.0).paint("5", 1));
        assert_eq!(pill.get(0, 0) >> 24, 0);
        assert_eq!(square.get(0, 0) >> 24, 0xFF);
        let soft = art(5.5).paint("5", 2);
        assert!(
            soft.pixels.iter().any(|p| (1..0xFF).contains(&(p >> 24))),
            "a curved edge at 2 has partial-alpha pixels"
        );
    }
}
