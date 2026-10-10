//! The badges on a printer's icon in Printers, 8x8 discs from the PNGs
//! in [`icons`]: a check on the default printer (lower left), and two
//! bars on a paused printer or a cross on one stopped on an error (lower
//! right).

use crate::art::icons;
use crate::raster::Canvas;

pub const SIZE: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Badge {
    Default,
    Paused,
    Error,
}

pub const ALL: [Badge; 3] = [Badge::Default, Badge::Paused, Badge::Error];

/// A black disc with a white check, an amber disc with two black bars,
/// a red disc with a white cross.
fn icon(b: Badge) -> &'static icons::Icon {
    match b {
        Badge::Default => &icons::PRINT_DEFAULT,
        Badge::Paused => &icons::PRINT_PAUSED,
        Badge::Error => &icons::PRINT_ERROR,
    }
}

/// `b` on its own, 8x8.
pub fn draw(b: Badge) -> Canvas {
    icon(b).canvas()
}

/// A copy of `icon` with `badges` over its bottom corners: the default
/// check at the left, a state badge at the right.
pub fn apply(icon: &Canvas, badges: &[Badge]) -> Canvas {
    let mut c = icon.clone();
    let y = c.height as i32 - SIZE as i32;
    for b in badges {
        let x = match b {
            Badge::Default => 0,
            Badge::Paused | Badge::Error => c.width as i32 - SIZE as i32,
        };
        c.blit_over(x, y, &draw(*b));
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Color;

    fn rgb(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(x, y) & 0x00ff_ffff
    }

    #[test]
    fn the_badges_keep_their_signature_pixels() {
        let d = draw(Badge::Default);
        assert_eq!(d.get(0, 0) >> 24, 0, "the disc's corner is clear");
        assert_eq!(rgb(&d, 1, 3), 0xffffff, "the check's short stroke");
        assert_eq!(rgb(&d, 6, 2), 0xffffff, "the check's long stroke");
        assert_eq!(rgb(&d, 3, 3), 0x000000, "the disc");
        let p = draw(Badge::Paused);
        assert_eq!(rgb(&p, 2, 3), 0x000000, "the left bar");
        assert_eq!(rgb(&p, 5, 4), 0x000000, "the right bar");
        assert_eq!(rgb(&p, 3, 3), 0xe8a800, "the amber between");
        let e = draw(Badge::Error);
        assert_eq!(rgb(&e, 3, 3), 0xffffff, "the cross");
        assert_eq!(rgb(&e, 1, 1), 0xc80000, "the red disc");
    }

    #[test]
    fn apply_sets_the_check_left_and_the_state_right() {
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(icon.bounds(), Color::rgb(0, 0, 255));
        let c = apply(&icon, &[Badge::Default, Badge::Error]);
        // The check's short stroke at (1, 3) of the badge at (0, 8).
        assert_eq!(rgb(&c, 1, 11), 0xffffff);
        // The cross's centre at (3, 3) of the badge at (8, 8).
        assert_eq!(rgb(&c, 11, 11), 0xffffff);
        // The icon shows through a badge's clear corner and above it.
        assert_eq!(rgb(&c, 0, 8), 0x0000ff);
        assert_eq!(rgb(&c, 4, 4), 0x0000ff);
        assert_eq!(apply(&icon, &[]).pixels, icon.pixels);
    }
}
