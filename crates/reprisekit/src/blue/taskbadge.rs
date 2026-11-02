//! A task button's unread badge in Blue: an orange-to-red pill with a white rim round white
//! numerals.

use crate::raster::Canvas;
use crate::taskbar::BadgeArt;
use crate::theme::{Color, ColorScheme};

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const ART: BadgeArt = BadgeArt {
    height: 11,
    pad_x: 3,
    radius: 5.5,
    rim: (WHITE, 0xE8),
    fill: [Color::rgb(0xFF, 0x8C, 0x3A), Color::rgb(0xD4, 0x30, 0x0E)],
    sheen: (WHITE, 0x30),
    ink: WHITE,
};

pub(super) fn paint(text: &str, _: &ColorScheme, d: i32) -> Canvas {
    ART.paint(text, d)
}
