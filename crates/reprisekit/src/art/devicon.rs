//! Device icons for reprise-bluetooth, 32x32, from the PNGs in
//! [`icons`]: headphones, a headset, a speaker, a game controller, a
//! keyboard, a mouse, a phone, a computer, and a generic device that
//! carries the Bluetooth badge.

use crate::art::{bticon, icons};
use crate::raster::Canvas;
use crate::theme::Color;

pub const SIZE: u32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Device {
    Headphones,
    Headset,
    Speaker,
    Gamepad,
    Keyboard,
    Mouse,
    Phone,
    Computer,
    Generic,
}

pub const ALL: [Device; 9] = [
    Device::Headphones,
    Device::Headset,
    Device::Speaker,
    Device::Gamepad,
    Device::Keyboard,
    Device::Mouse,
    Device::Phone,
    Device::Computer,
    Device::Generic,
];

fn icon(d: Device) -> &'static icons::Icon {
    match d {
        Device::Headphones => &icons::HEADPHONES,
        Device::Headset => &icons::HEADSET,
        Device::Speaker => &icons::SPEAKER,
        Device::Gamepad => &icons::GAMEPAD,
        Device::Keyboard => &icons::KEYBOARD,
        Device::Mouse => &icons::MOUSE,
        Device::Phone => &icons::PHONE,
        Device::Computer | Device::Generic => &icons::COMPUTER,
    }
}

const GLASS: Color = Color::rgb(0, 88, 208);
const SHADOW: Color = Color::rgb(0, 0, 128);
const LIGHT: Color = Color::rgb(0, 128, 0);

/// The computer's colours turned to greys of about their lightness, so a
/// greyed icon keeps its outline, bezel, glass and base apart.
fn grey(c: Color) -> Option<Color> {
    Some(match c {
        GLASS | LIGHT => Color::rgb(80, 80, 80),
        SHADOW => Color::rgb(40, 40, 40),
        c => c,
    })
}

/// `d`'s icon. [`Device::Generic`] is the computer in [`grey`], with the
/// [`bticon`] badge over its bottom-right corner.
pub fn draw(d: Device) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    if d == Device::Generic {
        icon(d).paint_inked(&mut c, 0, 0, 1, grey);
        c.blit_over(15, 15, &bticon::draw(true));
    } else {
        icon(d).paint(&mut c, 0, 0, 1);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ink(ch: char) -> Option<Color> {
        Some(match ch {
            'k' => Color::rgb(0, 0, 0),
            'd' => Color::rgb(64, 64, 64),
            'l' => Color::rgb(192, 192, 192),
            'w' => Color::rgb(255, 255, 255),
            'b' => GLASS,
            'n' => SHADOW,
            'r' => Color::rgb(200, 0, 0),
            's' => LIGHT,
            _ => return None,
        })
    }

    fn inked(c: &Canvas) -> usize {
        (0..SIZE as i32)
            .flat_map(|y| (0..SIZE as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) >> 24 != 0)
            .count()
    }

    #[test]
    fn every_device_draws_inside_a_clear_one_pixel_margin() {
        for d in ALL {
            let c = draw(d);
            assert_eq!((c.width, c.height), (SIZE, SIZE), "{d:?}");
            assert!(inked(&c) > 120, "{d:?} has a body");
            for i in 0..SIZE as i32 {
                for (x, y) in [(i, 0), (i, 31), (0, i), (31, i)] {
                    assert_eq!(c.get(x, y) >> 24, 0, "{d:?} leaves ({x}, {y}) clear");
                }
            }
        }
    }

    #[test]
    fn no_two_devices_share_a_picture() {
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert_ne!(draw(*a).pixels, draw(*b).pixels, "{a:?} and {b:?}");
            }
        }
    }

    #[test]
    fn each_device_keeps_its_signature_pixels() {
        let checks: [(Device, i32, i32, char, &str); 24] = [
            (Device::Headphones, 15, 4, 'w', "the headband's lit apex"),
            (Device::Headphones, 8, 20, 'l', "the left cushion"),
            (Device::Headphones, 25, 20, 'd', "the right cup"),
            (Device::Headset, 15, 28, 'k', "the microphone capsule"),
            (Device::Headset, 10, 28, 'd', "the boom"),
            (Device::Headset, 24, 18, 'l', "the right pad's cushion"),
            (Device::Speaker, 15, 9, 'd', "the tweeter"),
            (Device::Speaker, 15, 19, 'k', "the woofer's dust cap"),
            (Device::Speaker, 13, 16, 'l', "the cone ring"),
            (Device::Gamepad, 21, 12, 'r', "the red button"),
            (Device::Gamepad, 24, 15, 's', "the green button"),
            (Device::Gamepad, 9, 16, 'k', "the d-pad's centre"),
            (Device::Keyboard, 15, 20, 'w', "the space bar"),
            (Device::Keyboard, 6, 14, 'w', "a top-row key"),
            (Device::Keyboard, 15, 22, 'd', "the front lip"),
            (Device::Mouse, 15, 10, 'k', "the button split"),
            (Device::Mouse, 13, 18, 'l', "the body"),
            (Device::Mouse, 24, 1, 'd', "the cord's end"),
            (Device::Phone, 15, 10, 'b', "the screen"),
            (Device::Phone, 19, 2, 'd', "the antenna"),
            (Device::Phone, 12, 17, 'w', "the first key"),
            (Device::Computer, 15, 10, 'b', "the glass"),
            (Device::Computer, 23, 10, 'n', "the glass's shadow"),
            (Device::Computer, 23, 17, 's', "the power light"),
        ];
        for (d, x, y, ch, what) in checks {
            let want = ink(ch).unwrap().argb();
            assert_eq!(
                draw(d).get(x, y),
                want,
                "{d:?}: {what} at ({x}, {y}) is '{ch}'"
            );
        }
        let generic = draw(Device::Generic);
        let greyed = |ch| ink(ch).and_then(grey).unwrap().argb();
        assert_eq!(
            generic.get(15, 10),
            greyed('b'),
            "the glass is (80, 80, 80)"
        );
        assert_eq!(
            generic.get(23, 10),
            greyed('n'),
            "the glass's shadow is (40, 40, 40)"
        );
        assert_eq!(
            generic.get(6, 10),
            greyed('w'),
            "the bezel's highlight stays white"
        );
        assert_eq!(
            generic.get(19, 22),
            ink('b').unwrap().argb(),
            "the badge stays blue"
        );
    }

    #[test]
    #[ignore]
    fn render_devicons() {
        let mut c = Canvas::new(9 * 40, 80);
        c.fill_rect(c.bounds(), Color::rgb(212, 208, 200));
        for (i, d) in ALL.iter().enumerate() {
            c.blit_over(4 + 40 * i as i32, 4, &draw(*d));
            c.blit_over(4 + 40 * i as i32, 42, &draw(*d).resample_box(16, 16));
        }
        crate::raster::write_preview(&c, "devicons.png");
    }
}
