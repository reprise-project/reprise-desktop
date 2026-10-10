//! The inks of Glass's settings pages and panel links ([`PageInks::LIGHT`]); dark Glass remaps
//! each, and its links are 66B2FF lightening to 99CCFF under the pointer.

use crate::glass::Mode;
use crate::glass::dark::remap_color;
use crate::style::PageInks;
use crate::theme::Color;

pub fn inks(mode: Mode) -> PageInks {
    match mode {
        Mode::Light => PageInks::LIGHT,
        Mode::Dark => PageInks {
            link: Color::rgb(0x66, 0xB2, 0xFF),
            link_hot: Color::rgb(0x99, 0xCC, 0xFF),
            ..PageInks::LIGHT.map(remap_color)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_links_are_the_dark_schemes_and_the_rest_remapped() {
        let dark = inks(Mode::Dark);
        assert_eq!(dark.link, Color::rgb(0x66, 0xB2, 0xFF));
        assert_eq!(dark.rule, remap_color(PageInks::LIGHT.rule));
        assert_eq!(inks(Mode::Light), PageInks::LIGHT);
    }
}
