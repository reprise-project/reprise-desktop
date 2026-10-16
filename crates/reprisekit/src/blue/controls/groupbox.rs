//! The group box's [`Part::GroupBox`] atlas, drawn from the scheme's
//! `group_box` faces at the style's scale: two frames, Normal then
//! Disabled, each a one-pixel line round a 7x7 box with its corners
//! rounded over [`RADIUS`], and nothing behind it. A disabled group box
//! draws the same frame as an enabled one, so its face carries the same
//! colour.

use crate::blue::Variant;
use crate::blue::scheme::Face;
use crate::raster::Insets;
use crate::shape::{CLEAR, Px, draw, premul, ringed};
use crate::style::{Layout, Part, PartImage, State};

const CORNER: i32 = 3;

const FRAME_SIZE: i32 = CORNER * 2 + 1;

const RADIUS: f64 = 3.0;

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let faces = &v.scheme().group_box;
    let frames = [
        (State::Normal, &faces.normal),
        (State::Disabled, &faces.disabled),
    ];
    let rings = frames.map(|(_, face)| frame(face, d));
    let side = FRAME_SIZE * d;
    let n = frames.len() as i32;
    let a = draw(FRAME_SIZE, FRAME_SIZE * n, d, |x, y| {
        let (x, y) = (x as i32, y as i32);
        rings[(y / side) as usize][((y % side) * side + x) as usize]
    });
    let states = frames.map(|(s, _)| s);
    let mut img = PartImage::new(a, &states, Layout::Vertical)
        .sizing(Insets::new(CORNER, CORNER, CORNER, CORNER))
        .at_scale(d);
    img.border_only = true;
    vec![(Part::GroupBox, img)]
}

fn frame(face: &Face, d: i32) -> Vec<Px> {
    let line = premul(face.rim, 255);
    ringed(
        FRAME_SIZE,
        FRAME_SIZE,
        RADIUS,
        1,
        d,
        |_, _, ring| match ring {
            0 => line,
            _ => CLEAR,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Canvas;

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn atlas(v: Variant, d: i32) -> PartImage {
        parts(v, d).remove(0).1
    }

    #[test]
    fn group_box_pins_at_scale_1_and_2() {
        let want: [(Variant, [u64; 2]); 9] = [
            (
                Variant::Blue,
                [0x6d60_4c2e_4f9e_a2bc, 0x5912_f229_0939_7607],
            ),
            (
                Variant::Olive,
                [0x6d60_4c2e_4f9e_a2bc, 0x5912_f229_0939_7607],
            ),
            (
                Variant::Silver,
                [0xb1b8_edf3_9b47_a2cc, 0xddd1_a36e_afbe_3df7],
            ),
            (
                Variant::Slate,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
            (
                Variant::Sapphire,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
            (
                Variant::Obsidian,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
            (
                Variant::Ember,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
            (
                Variant::Cobalt,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
            (
                Variant::Carbon,
                [0x975a_cfeb_fd05_6ffc, 0x2185_1587_6b85_80c7],
            ),
        ];
        let got = want.map(|(v, _)| (v, [fnv(&atlas(v, 1).atlas), fnv(&atlas(v, 2).atlas)]));
        assert_eq!(got, want, "{got:#x?}");
    }

    #[test]
    fn the_group_box_is_painted_at_the_style_scale() {
        for v in Variant::ALL {
            for d in [1, 2, 3] {
                let img = atlas(v, d);
                let du = d as u32;
                assert_eq!(
                    (img.atlas.width, img.atlas.height),
                    (7 * du, 14 * du),
                    "{v:?} at {d}"
                );
                assert_eq!(img.sizing, Insets::new(3, 3, 3, 3), "{v:?} at {d}");
                assert_eq!(img.scale, d, "{v:?} at {d}");
                assert!(img.border_only, "{v:?}: nothing behind the frame");
            }
        }
    }

    #[test]
    fn the_frame_is_a_one_pixel_line_with_soft_corners() {
        for v in Variant::ALL {
            let rim = v.scheme().group_box.normal.rim.argb();
            for d in [1, 2] {
                let c = atlas(v, d).atlas;
                let at = |x: i32, y: i32| c.get(x, y);
                let mid = 3 * d;
                for i in 0..d {
                    assert_eq!(at(mid, i), rim, "{v:?} at {d}: top line row {i}");
                    assert_eq!(at(i, mid), rim, "{v:?} at {d}: left line column {i}");
                    assert_eq!(at(7 * d - 1 - i, mid), rim, "{v:?} at {d}: right line");
                    assert_eq!(at(mid, 7 * d - 1 - i), rim, "{v:?} at {d}: bottom line");
                }
                assert_eq!(at(mid, d), 0, "{v:?} at {d}: under the top line");
                assert!(at(0, 0) >> 24 < 8, "{v:?} at {d}: the corner's tip");
                assert_eq!(at(mid, mid), 0, "{v:?} at {d}: the middle is bare");
                let corner: Vec<u32> = (0..3 * d)
                    .flat_map(|y| (0..3 * d).map(move |x| (x, y)))
                    .map(|(x, y)| at(x, y))
                    .collect();
                assert!(
                    corner.iter().any(|p| p >> 24 >= 0xC0),
                    "{v:?} at {d}: the corner turns through a solid pixel"
                );
                if d == 2 {
                    assert!(
                        corner.iter().any(|p| (1..0xFF).contains(&(p >> 24))),
                        "{v:?}: the curve is smooth at 2"
                    );
                }
                for p in corner.iter().filter(|p| **p >> 24 != 0) {
                    let a = p >> 24;
                    let shade = |s: u32| p >> s & 0xFF;
                    let want = |s: u32| ((rim >> s & 0xFF) * a + 127) / 255;
                    for s in [16, 8, 0] {
                        assert!(
                            shade(s).abs_diff(want(s)) <= 1,
                            "{v:?} at {d}: corner pixel {p:08X} is the rim's colour"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_disabled_group_box_keeps_its_frame() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = atlas(v, d);
                let frame = |s| {
                    let r = img.frame(s);
                    (r.y..r.bottom())
                        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
                        .map(|(x, y)| img.atlas.get(x, y))
                        .collect::<Vec<_>>()
                };
                assert_eq!(frame(State::Normal), frame(State::Disabled), "{v:?} at {d}");
            }
        }
    }
}
