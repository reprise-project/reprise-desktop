//! Colour schemes: the 16 ANSI colours, the default fg and bg, and the cursor and selection.

use reprisekit::controls::termgrid::{Accents, Rgb};
use reprisekit::interface::Interface;
use reprisekit::theme::Color;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scheme {
    pub fg: Rgb,
    pub bg: Rgb,
    pub ansi: [Rgb; 16],
    pub accents: Accents,
}

const fn rgb(v: u32) -> Rgb {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

const GREY_ON_BLACK: [u32; 16] = [
    0x000000, 0xAA0000, 0x00AA00, 0xAA5500, 0x0000AA, 0xAA00AA, 0x00AAAA, 0xAAAAAA, 0x555555,
    0xFF5555, 0x55FF55, 0xFFFF55, 0x5555FF, 0xFF55FF, 0x55FFFF, 0xFFFFFF,
];

const TINTED: [u32; 16] = [
    0x1C2128, 0xE06C6C, 0x7FC48A, 0xD8B65E, 0x6A9FE0, 0xB88BD8, 0x5FC0C8, 0xC8CED6, 0x5A6470,
    0xF28B8B, 0x9CDCA6, 0xEDD08A, 0x8DB8F0, 0xD0A8EC, 0x86D8DE, 0xF2F5F8,
];

const LIGHT: [u32; 16] = [
    0x1E1E1E, 0xB3261E, 0x1E7B34, 0x8A6D00, 0x1F4FB3, 0x8E3CA0, 0x00798A, 0x6B6B6B, 0x4D4D4D,
    0xD93025, 0x2FA24A, 0xA88500, 0x3B6FDB, 0xA95BBE, 0x0E98AC, 0x8C8C8C,
];

const DARK: [u32; 16] = [
    0x2E3436, 0xCC0000, 0x4E9A06, 0xC4A000, 0x3465A4, 0x75507B, 0x06989A, 0xD3D7CF, 0x555753,
    0xEF2929, 0x8AE234, 0xFCE94F, 0x729FCF, 0xAD7FA8, 0x34E2E2, 0xEEEEEC,
];

/// The names the Options sheet lists, with their labels. "reprise" follows the interface.
pub const NAMES: [(&str, &str); 3] = [("reprise", "Reprise"), ("light", "Light"), ("dark", "Dark")];

fn build(fg: u32, bg: u32, ansi: [u32; 16], cursor: u32, selection: (u32, u32)) -> Scheme {
    Scheme {
        fg: rgb(fg),
        bg: rgb(bg),
        ansi: ansi.map(rgb),
        accents: Accents {
            cursor: rgb(cursor),
            selection: (rgb(selection.0), rgb(selection.1)),
            ..Accents::default()
        },
    }
}

/// The scheme named `name`; "reprise", and any name not known, follows `interface`.
pub fn scheme(name: &str, interface: Interface) -> Scheme {
    match (name, interface) {
        ("light", _) => build(0x1E1E1E, 0xFFFFFF, LIGHT, 0x1E1E1E, (0xFFFFFF, 0x3875D7)),
        ("dark", _) => build(0xD3D7CF, 0x1E1E1E, DARK, 0xD3D7CF, (0xFFFFFF, 0x3465A4)),
        (_, Interface::Glass) => build(0xD8DCE2, 0x161B22, TINTED, 0xC8D8F0, (0xFFFFFF, 0x2F5A8C)),
        _ => build(
            0xC0C0C0,
            0x000000,
            GREY_ON_BLACK,
            0xC0C0C0,
            (0xFFFFFF, 0x30509A),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reprise_scheme_follows_interface() {
        let classic = scheme("reprise", Interface::Classic);
        assert_eq!(
            (classic.fg, classic.bg),
            (rgb(0xC0C0C0), rgb(0x000000)),
            "grey on black"
        );
        assert_eq!(scheme("reprise", Interface::Blue), classic);
        let glass = scheme("reprise", Interface::Glass);
        assert_ne!(glass, classic);
        assert!(
            glass.bg != rgb(0) && glass.bg.b > glass.bg.r,
            "Glass is tinted blue, not black"
        );
        assert_eq!(scheme("no such scheme", Interface::Classic), classic);
    }

    #[test]
    fn fixed_schemes_listed() {
        let reprise = scheme("reprise", Interface::Classic);
        let light = scheme("light", Interface::Classic);
        let dark = scheme("dark", Interface::Classic);
        assert_eq!(light.bg, rgb(0xFFFFFF), "light is dark on white");
        assert_eq!(dark.bg, rgb(0x1E1E1E));
        assert_ne!(light, dark);
        assert_ne!(light, reprise);
        assert_ne!(dark, reprise);
        assert_eq!(
            scheme("light", Interface::Glass),
            light,
            "a fixed scheme ignores the interface"
        );
        assert_eq!(scheme("dark", Interface::Blue), dark);
        for (name, _) in NAMES {
            assert_eq!(
                scheme(name, Interface::Classic) == reprise,
                name == "reprise",
                "{name}"
            );
        }
    }
}
