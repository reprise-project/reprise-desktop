//! A task button's unread badge in Glass: a red glass pill, lit across its upper half, with a dark
//! rim round white numerals.

use crate::raster::Canvas;
use crate::taskbar::BadgeArt;
use crate::theme::{Color, ColorScheme};

const ART: BadgeArt = BadgeArt {
    height: 11,
    pad_x: 3,
    radius: 5.5,
    rim: (Color::rgb(0x50, 0x08, 0x08), 0xD0),
    fill: [Color::rgb(0xEE, 0x4A, 0x40), Color::rgb(0xA8, 0x10, 0x10)],
    sheen: (Color::rgb(0xFF, 0xFF, 0xFF), 0x48),
    ink: Color::rgb(0xFF, 0xFF, 0xFF),
};

pub(super) fn paint(text: &str, _: &ColorScheme, d: i32) -> Canvas {
    ART.paint(text, d)
}
