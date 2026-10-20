//! The 32 px device icons the lists and sheets show, drawn once per kind.

use std::borrow::Cow;
use std::collections::HashMap;

use reprisekit::art::devicon;
use reprisekit::raster::Canvas;
use reprisekit::theme::TextUnits;

use crate::bz::Kind;

pub struct Icons(HashMap<devicon::Device, Canvas>);

impl Icons {
    pub fn new() -> Icons {
        Icons(
            devicon::ALL
                .iter()
                .map(|d| (*d, devicon::draw(*d)))
                .collect(),
        )
    }

    pub fn get(&self, kind: Kind) -> &Canvas {
        &self.0[&kind.icon()]
    }

    /// `kind`'s icon for a tile list row grown by `u`: 64 px at 200%.
    pub fn tile(&self, kind: Kind, u: TextUnits) -> Cow<'_, Canvas> {
        let icon = self.get(kind);
        let side = u.len(devicon::SIZE as i32) as u32;
        match side == icon.width {
            true => Cow::Borrowed(icon),
            false => Cow::Owned(icon.resample_box(side, side)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tile's icon is the 32 px one at 100% and doubles block for block at 200%.
    #[test]
    fn a_tile_icon_grows_with_the_text() {
        let icons = Icons::new();
        let small = icons.get(Kind::Headphones);
        assert!(matches!(
            icons.tile(Kind::Headphones, TextUnits::ONE),
            Cow::Borrowed(_)
        ));
        let big = icons.tile(Kind::Headphones, TextUnits::new(22, 11));
        assert_eq!((big.width, big.height), (64, 64));
        for (x, y) in [(0, 0), (10, 12), (16, 16), (31, 31)] {
            assert_eq!(big.get(2 * x + 1, 2 * y + 1), small.get(x, y), "({x}, {y})");
        }
    }
}
