//! The size grip, [`Part::SizeBox`], drawn from the scheme's `size_box`
//! face at the style's scale: six raised dots in a staircase, 11x11, true
//! to its own size. The dots sit 4 px apart, the lowest row of three and
//! the rightmost column of three, so the grip's corner is a full dot.
//! Each dot is a 2x2 square in the face's grey over a white square one
//! pixel down and right. The squares' edges fall on design pixel lines,
//! so they stay sharp at every scale. Everything between the dots is
//! transparent, so the face shows through.

use crate::blue::Variant;
use crate::blue::scheme::Face;
use crate::raster::Canvas;
use crate::shape::{CLEAR, Px, draw, over, premul, rounded, scale};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const S: i32 = 11;
const PITCH: i32 = 4;
const DOT: f64 = 2.0;
const LIFT: f64 = 1.0;

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let mut img = PartImage::new(
        grip(&v.scheme().size_box.normal, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .at_scale(d);
    img.true_size = true;
    vec![(Part::SizeBox, img)]
}

fn light(f: &Face) -> Color {
    let luma = |c: Color| u32::from(c.r) + u32::from(c.g) + u32::from(c.b);
    f.down
        .iter()
        .map(|&(_, c)| c)
        .chain([f.rim])
        .max_by_key(|&c| luma(c))
        .unwrap_or(f.rim)
}

fn grip(f: &Face, d: i32) -> Canvas {
    let (grey, white) = (premul(f.inner, 255), premul(light(f), 255));
    let k = f64::from(d);
    let square = |x: f64, y: f64, left: f64, top: f64| {
        rounded(x - left * k, y - top * k, DOT * k, DOT * k, 0.0)
    };
    draw(S, S, d, |x, y| {
        let mut px: Px = CLEAR;
        for row in 0..3 {
            for col in 0..3 {
                if row + col < 2 {
                    continue;
                }
                let (left, top) = (f64::from(col * PITCH), f64::from(row * PITCH));
                let under = scale(white, square(x, y, left + LIFT, top + LIFT));
                let on = scale(grey, square(x, y, left, top));
                px = over(over(on, under), px);
            }
        }
        px
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Each scheme's grip at scale 1 and 2, as FNV-1a hashes of (width,
    /// height, pixels).
    #[test]
    fn size_box_pins_at_scale_1_and_2() {
        let want: [(Variant, [u64; 2]); 8] = [
            (
                Variant::Blue,
                [0x369b_22dd_5ce2_859d, 0xafc0_30a0_790a_de65],
            ),
            (
                Variant::Olive,
                [0x369b_22dd_5ce2_859d, 0xafc0_30a0_790a_de65],
            ),
            (
                Variant::Silver,
                [0x369b_22dd_5ce2_859d, 0xafc0_30a0_790a_de65],
            ),
            (
                Variant::Slate,
                [0x7761_5d96_182c_f4cd, 0x18cf_3ba6_f73d_5ca5],
            ),
            (
                Variant::Sapphire,
                [0x7761_5d96_182c_f4cd, 0x18cf_3ba6_f73d_5ca5],
            ),
            (
                Variant::Obsidian,
                [0x7761_5d96_182c_f4cd, 0x18cf_3ba6_f73d_5ca5],
            ),
            (
                Variant::Ember,
                [0x7761_5d96_182c_f4cd, 0x18cf_3ba6_f73d_5ca5],
            ),
            (
                Variant::Cobalt,
                [0x7761_5d96_182c_f4cd, 0x18cf_3ba6_f73d_5ca5],
            ),
        ];
        let got = want.map(|(v, _)| (v, [fnv(&atlas(v, 1).atlas), fnv(&atlas(v, 2).atlas)]));
        assert_eq!(got, want, "{got:#x?}");
    }

    #[test]
    fn the_size_box_is_painted_at_the_style_scale() {
        for v in Variant::ALL {
            for d in [1, 2, 3] {
                let img = atlas(v, d);
                let du = d as u32;
                assert_eq!((img.atlas.width, img.atlas.height), (11 * du, 11 * du));
                assert_eq!(img.scale, d, "{v:?} at {d}");
                assert!(img.true_size, "{v:?}: drawn at its own size");
            }
        }
    }

    /// The corner dot's grey and white middles are opaque in the scheme's
    /// colours, the staircase leaves its upper-left clear, the gaps
    /// between dots are clear, and every pixel is a whole colour or clear.
    #[test]
    fn the_grip_is_a_staircase_of_raised_dots() {
        for v in Variant::ALL {
            let f = &v.scheme().size_box.normal;
            let (grey, white) = (f.inner.argb(), light(f).argb());
            for d in [1, 2] {
                let c = atlas(v, d).atlas;
                let at = |x: i32, y: i32| c.get(x * d, y * d);
                assert_eq!(at(8, 8), grey, "{v:?} at {d}: the corner dot's grey");
                assert_eq!(at(10, 10), white, "{v:?} at {d}: the corner dot's white");
                assert_eq!(at(10, 2), white, "{v:?} at {d}: the top dot's white");
                assert_eq!(at(0, 0), 0, "{v:?} at {d}: clear above the staircase");
                assert_eq!(at(4, 0), 0, "{v:?} at {d}: no dot at row 0, column 1");
                assert_eq!(at(7, 8), 0, "{v:?} at {d}: between two dots");
                assert_eq!(at(8, 7), 0, "{v:?} at {d}: between two rows");
            }
            for d in [1, 2] {
                let c = atlas(v, d).atlas;
                assert!(
                    c.pixels.iter().all(|p| [0, grey, white].contains(p)),
                    "{v:?} at {d}: sharp squares"
                );
            }
        }
    }
}
