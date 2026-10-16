//! The copy dialog's flight with shaded folders, soft shadows and a trail in the scheme's highlight.

use crate::copyart::{self, Flight, Palette};
use crate::raster::Canvas;
use crate::theme::{Color, ColorScheme};

fn palette(cs: &ColorScheme) -> Palette {
    Palette {
        back: [Color::rgb(0xF6, 0xD3, 0x7A), Color::rgb(0xD9, 0xA4, 0x3A)],
        front: [Color::rgb(0xFF, 0xEB, 0xA6), Color::rgb(0xF2, 0xC2, 0x55)],
        rim: Color::rgb(0xA8, 0x7A, 0x1E),
        paper: Color::rgb(0xFF, 0xFF, 0xFF),
        paper_rim: Color::rgb(0xA0, 0xA8, 0xB8),
        ink: cs.hilight,
        shadow: 50,
        trail: Some(cs.hilight),
        gloss: true,
    }
}

pub(super) fn paint(f: &Flight, cs: &ColorScheme, w: i32, d: i32) -> Canvas {
    copyart::paint_flight(f, &palette(cs), w, d)
}
