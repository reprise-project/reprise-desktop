//! The copy dialog's flight with shaded folders, soft shadows and a trail in the scheme's highlight.

use crate::copyart::{self, Flight, Palette};
use crate::raster::Canvas;
use crate::theme::{Color, ColorScheme};

/// The bin's can is a cool grey leaning toward the scheme's highlight by these shares (of 255).
const BIN_LIT: Color = Color::rgb(0xF4, 0xF6, 0xFA);
const BIN_SHADE: Color = Color::rgb(0x9C, 0xA6, 0xB6);
const BIN_TINT: (u8, u8) = (28, 56);

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
        bin: [
            BIN_LIT.toward(cs.hilight, BIN_TINT.0),
            BIN_SHADE.toward(cs.hilight, BIN_TINT.1),
        ],
        bin_rim: Color::rgb(0x5E, 0x6A, 0x7E).toward(cs.hilight, BIN_TINT.1),
        bin_inside: Color::rgb(0x34, 0x3C, 0x4A),
        bin_rib: Color::rgb(0x84, 0x90, 0xA4).toward(cs.hilight, BIN_TINT.1),
    }
}

pub(super) fn paint(f: &Flight, cs: &ColorScheme, w: i32, d: i32) -> Canvas {
    copyart::paint_flight(f, &palette(cs), w, d)
}
