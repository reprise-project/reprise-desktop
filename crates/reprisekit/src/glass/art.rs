//! The Glass frame's design colours, in Sky at the default intensity: the frame
//! (`glass::frame`) and its caption buttons (`glass::buttons`) are drawn from its
//! plain glass, lines and sheen, and opaque glass lies over [`OPAQUE_UNDER`].
//! Every other glass colour and intensity is these moved by `tint::Recolour`.

use crate::theme::Color;

pub(crate) const fn glass(c: Color, a: u8) -> u32 {
    const fn ch(v: u8, a: u32) -> u32 {
        (2 * v as u32 * a + 255) / 510
    }
    let a = a as u32;
    (a << 24) | (ch(c.r, a) << 16) | (ch(c.g, a) << 8) | ch(c.b, a)
}

/// The frame's plain glass, active then inactive: a deep blue, faint, and
/// a paler blue fainter still.
pub const PLAIN: [u32; 2] = [
    glass(Color::rgb(0, 68, 152), 62),
    glass(Color::rgb(108, 186, 255), 34),
];

/// The frame's lines over its plain glass, active then inactive: the
/// outline (near black), the light line inside it (near white), the
/// client's light edge and its dark edge.
pub const LINES: [[u32; 4]; 2] = [
    [
        glass(Color::rgb(0, 3, 8), 215),
        glass(Color::rgb(240, 245, 250), 215),
        glass(Color::rgb(212, 223, 236), 170),
        glass(Color::rgb(17, 28, 42), 168),
    ],
    [
        glass(Color::rgb(3, 9, 23), 151),
        glass(Color::rgb(244, 248, 255), 165),
        glass(Color::rgb(241, 248, 255), 158),
        glass(Color::rgb(29, 33, 48), 144),
    ],
];

/// The caption's sheen where it is strongest, over the plain glass,
/// active then inactive: a pale blue haze.
pub const SHEEN: [u32; 2] = [
    glass(Color::rgb(175, 200, 225), 134),
    glass(Color::rgb(194, 217, 239), 87),
];

/// The colour the frame is composited over, opaque, when transparency is
/// off: active (a pale sky blue), then inactive (a light grey).
pub const OPAQUE_UNDER: [[u8; 3]; 2] = [[232, 243, 255], [217, 218, 221]];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glass_colour_packs_premultiplied() {
        let c = Color::rgb(240, 245, 250);
        assert_eq!(glass(c, 255), c.argb(), "opaque is the colour itself");
        assert_eq!(glass(Color::rgb(255, 255, 255), 128), 0x8080_8080);
        assert_eq!(glass(c, 0), 0, "clear");
        assert_eq!(PLAIN[0] >> 24, 62, "the active glass is faint");
    }
}
