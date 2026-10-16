//! The copy dialog's flight in flat colours.

use crate::copyart::{self, Flight, Palette};
use crate::raster::Canvas;
use crate::theme::{Color, ColorScheme};

pub fn palette(cs: &ColorScheme) -> Palette {
    Palette {
        back: [Color::rgb(0xE5, 0xB6, 0x4B); 2],
        front: [Color::rgb(0xFF, 0xDC, 0x73); 2],
        rim: Color::rgb(0x80, 0x60, 0x10),
        paper: Color::rgb(0xFF, 0xFF, 0xFF),
        paper_rim: Color::rgb(0x80, 0x80, 0x80),
        ink: cs.hilight,
        shadow: 0,
        trail: None,
        gloss: false,
    }
}

pub fn paint(f: &Flight, cs: &ColorScheme, w: i32, d: i32) -> Canvas {
    copyart::paint_flight(f, &palette(cs), w, d)
}
