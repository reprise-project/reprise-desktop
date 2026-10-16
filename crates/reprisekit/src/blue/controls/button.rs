//! The push button's [`Part::PushButton`] atlas: six frames, in order
//! Normal, Hot, Pressed, Disabled, Default then Focused.
//!
//! Each frame is a 75x23 button drawn from shapes at the style's scale in
//! the scheme's [`PushButtonFaces`] colours: four rounded rings of one
//! design pixel each, every one a pixel tighter than the last, round a
//! shaded face.
//! - The edge: in navy, olive green and silver the scheme's translucent
//!   halo along the top and sides and a pale lit line along the bottom,
//!   so the button sits on a dialog, a tab page or a dark band alike;
//!   sapphire, obsidian and cobalt the halo all round; slate and ember an opaque
//!   edge in the dialog's face.
//! - The border, 1 px.
//! - Two band rings: on the hot and default buttons a 2 px ring in the
//!   face's edge colours, otherwise part of the face.
//!
//! A ring pixel takes the colour of the face's profile on its nearest side:
//! `down` along the top and bottom, `across` down the sides. The face shades
//! top to bottom (`down`), each column lighter or darker by how far `across`
//! strays from the middle there. The pressed face runs its shading the other
//! way; a disabled button has a pale border on a flat face; a focused button
//! carries the default button's ring. No frame has a label or a focus ring;
//! the drawer adds both.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const W: i32 = 75;
const H: i32 = 23;

/// The nine-grid's fixed border; the face between stretches.
const SIZING: Insets = Insets::new(6, 4, 7, 5);

/// Where the label goes: 4 px in from either side, clear of the focus ring,
/// and a pixel down from the top (the label's baseline is on row 16).
const CONTENT: Insets = Insets::new(4, 1, 4, 0);

/// The lit line along the bottom edge in navy, olive green and silver: white
/// at this alpha.
const LIT_EDGE: u8 = 0xA4;

/// The edge ring's corner radius in design px: navy, olive green and silver
/// round theirs, the other schemes keep a squarer corner.
const ROUND: f64 = 4.0;
const SQUARE: f64 = 3.0;

/// How a scheme draws the ring outside the border.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Edge {
    /// The halo along the top and sides, a lit line along the bottom.
    Lit,
    /// The halo all round.
    Halo,
    /// Opaque, in the face profile's outermost colours.
    Solid,
}

/// `v`'s edge and corner radius.
fn edge(v: Variant) -> (Edge, f64) {
    match v {
        Variant::Blue | Variant::Olive | Variant::Silver => (Edge::Lit, ROUND),
        Variant::Sapphire | Variant::Obsidian | Variant::Cobalt => (Edge::Halo, SQUARE),
        Variant::Slate | Variant::Ember => (Edge::Solid, SQUARE),
    }
}

const STATES: [State; 6] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Disabled,
    State::Default,
    State::Focused,
];

/// This control's parts in `v`'s colours, painted for a draw at `d`
/// device px per design px.
pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let pixels = STATES.iter().flat_map(|&s| frame(v, s, d).pixels).collect();
    let a = Canvas::from_pixels((W * d) as u32, (H * d) as u32 * STATES.len() as u32, pixels);
    let img = PartImage::new(a, &STATES, Layout::Vertical)
        .sizing(SIZING)
        .content(CONTENT)
        .at_scale(d);
    vec![(Part::PushButton, img)]
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn shift(a: Px, b: Px, c: Px) -> Px {
    let ch = |i: usize| (a.1[i] + b.1[i] - c.1[i]).clamp(0.0, 255.0);
    (255.0, [ch(0), ch(1), ch(2)])
}

fn frame(v: Variant, s: State, d: i32) -> Canvas {
    let p = &v.scheme().push_button;
    let (f, ring): (&Face, bool) = match s {
        State::Hot => (&p.hot, true),
        State::Pressed => (&p.pressed, false),
        State::Disabled => (&p.disabled, false),
        State::Default => (&p.default, true),
        State::Focused => (&p.focused, true),
        _ => (&p.normal, false),
    };
    let (edge, r) = edge(v);
    let (down, across) = (stops(f.down), stops(f.across));
    // Row y of H sits at (y + 0.5) / H on `down`, column x of W at
    // (x + 0.5) / W on `across`.
    let down = |y: f64| Stops(&down).at((y + 0.5) / f64::from(H));
    let across = |x: f64| Stops(&across).at((x + 0.5) / f64::from(W));
    let middle = across(f64::from(W / 2));
    let nearest = |x: f64, y: f64| {
        let row = y.min(f64::from(H - 1) - y);
        let col = x.min(f64::from(W - 1) - x);
        if row <= col { down(y) } else { across(x) }
    };
    let lit = shape::premul(Color::rgb(0xFF, 0xFF, 0xFF), LIT_EDGE);
    let px = shape::ringed(W, H, r, 4, d, |x, y, i| match i {
        0 => match (edge, f.halo) {
            (Edge::Solid, _) => nearest(x, y),
            (Edge::Lit, Some(_)) if y > f64::from(H) - 1.5 => lit,
            (_, Some((c, a))) => shape::premul(c, a),
            (_, None) => shape::CLEAR,
        },
        1 => nearest(x, y),
        2 | 3 if ring => nearest(x, y),
        _ => shift(down(y), across(x), middle),
    });
    let w = (W * d) as usize;
    shape::draw(W, H, d, |x, y| px[y as usize * w + x as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::Rect;

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        for w in words {
            for b in w.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        h
    }

    fn img(v: Variant, d: i32) -> PartImage {
        parts(v, d).into_iter().next().unwrap().1
    }

    fn cut(v: Variant, s: State, d: i32) -> Canvas {
        let i = STATES.iter().position(|&t| t == s).unwrap() as i32;
        img(v, d)
            .atlas
            .copy_rect(Rect::new(0, i * H * d, W * d, H * d))
    }

    fn alpha(c: &Canvas, x: i32, y: i32) -> u32 {
        c.pixels[(y * c.width as i32 + x) as usize] >> 24
    }

    /// Every scheme's frames are 75x23 design px, at d device px each,
    /// and the nine-grid and label insets stay in design px at every
    /// scale.
    #[test]
    fn the_push_button_is_75_by_23_with_design_insets_at_every_scale() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let i = img(v, d);
                assert_eq!(i.scale, d, "{v:?} at {d}");
                assert_eq!(
                    (i.atlas.width, i.atlas.height),
                    (75 * d as u32, 23 * 6 * d as u32),
                    "{v:?} at {d}"
                );
                assert_eq!(i.sizing, SIZING, "{v:?} at {d}");
                assert_eq!(i.content, CONTENT, "{v:?} at {d}");
                assert_eq!(i.states, STATES, "{v:?} at {d}");
            }
        }
    }

    /// The corners are rounded and the edge translucent in the atlas: the
    /// outermost corner pixel shows little, the face's middle is opaque,
    /// and at 2 the curves carry partial alpha.
    #[test]
    fn the_corners_are_round_and_smooth_at_2() {
        for v in Variant::ALL {
            for s in STATES {
                let one = cut(v, s, 1);
                assert!(alpha(&one, 0, 0) < 0x60, "{v:?} {s:?} corner");
                assert_eq!(alpha(&one, 37, 11), 255, "{v:?} {s:?} middle");
                let two = cut(v, s, 2);
                let partial = (0..8)
                    .flat_map(|y| (0..8).map(move |x| (x, y)))
                    .filter(|&(x, y)| (1..255).contains(&alpha(&two, x, y)))
                    .count();
                assert!(partial >= 2, "{v:?} {s:?} smooth corner at 2");
            }
        }
    }

    /// Normal, hot, pressed, disabled and default all differ in every
    /// scheme, except that slate and ember draw the default button with
    /// the hot button's ring; Focused repeats Default.
    #[test]
    fn each_state_draws_its_own_frame() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let h: Vec<u64> = STATES.iter().map(|&s| fnv(&frame(v, s, d))).collect();
                for i in 0..5 {
                    for j in i + 1..5 {
                        let alias = matches!(v, Variant::Slate | Variant::Ember)
                            && (STATES[i], STATES[j]) == (State::Hot, State::Default);
                        if alias {
                            assert_eq!(h[i], h[j], "{v:?} Default repeats Hot at {d}");
                            continue;
                        }
                        assert_ne!(h[i], h[j], "{v:?} {:?} {:?} at {d}", STATES[i], STATES[j]);
                    }
                }
                assert_eq!(h[4], h[5], "{v:?} Focused repeats Default at {d}");
            }
        }
    }

    /// The hot button's ring runs down both sides in its own colour: the
    /// side columns inside the border differ from the face's middle.
    #[test]
    fn the_hot_ring_shows_down_the_sides() {
        for v in Variant::ALL {
            let n = frame(v, State::Normal, 1);
            let h = frame(v, State::Hot, 1);
            let at = |c: &Canvas, x: i32| c.pixels[(11 * c.width as i32 + x) as usize];
            for x in [2, 72] {
                assert_ne!(at(&h, x), at(&n, x), "{v:?} column {x}");
            }
        }
    }

    /// The border is opaque at 2 all along each straight side, and no
    /// pixel there is darker than the border inside it: the edge does
    /// not leave a dark fringe.
    #[test]
    fn the_border_is_solid_with_no_dark_fringe() {
        let lum = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        for v in Variant::ALL {
            let c = frame(v, State::Normal, 2);
            let px = |x: i32, y: i32| c.pixels[(y * c.width as i32 + x) as usize];
            for y in [2, 3] {
                assert_eq!(px(75, y) >> 24, 255, "{v:?} top border row {y}");
            }
            for x in [2, 3] {
                assert_eq!(px(x, 23) >> 24, 255, "{v:?} left border column {x}");
            }
            // The face pixel inside the border is no darker than the border.
            assert!(lum(px(4, 23)) >= lum(px(3, 23)), "{v:?} left face");
        }
    }

    /// A wide button keeps its corners and stretches its middle: drawn
    /// 150 wide, its left and right 6 columns equal the 75 px frame's.
    #[test]
    fn a_wide_button_keeps_its_ends() {
        let st = crate::blue::style_for(Variant::Blue);
        let draw = |w: i32| {
            let mut c = Canvas::new(w as u32, 23);
            st.draw(
                &mut c,
                Rect::new(0, 0, w, 23),
                Part::PushButton,
                State::Normal,
                1,
            );
            c
        };
        let (a, b) = (draw(75), draw(150));
        for y in 0..23 {
            for x in 0..6 {
                let p = |c: &Canvas, x: i32| c.pixels[(y * c.width as i32 + x) as usize];
                assert_eq!(p(&a, x), p(&b, x), "left ({x}, {y})");
                assert_eq!(p(&a, 74 - x), p(&b, 149 - x), "right ({x}, {y})");
            }
        }
    }

    /// Every scheme's atlas at 1 and 2, pinned.
    #[test]
    fn push_button_pins_at_scale_1_and_2() {
        // Ember draws slate's button and obsidian sapphire's.
        let want = [
            (
                Variant::Blue,
                [0x4b82_f4e6_2949_eab4, 0x8432_f74a_35d0_43f3],
            ),
            (
                Variant::Olive,
                [0x6d71_76d3_abd5_3706, 0x06d4_93ca_494b_7132],
            ),
            (
                Variant::Silver,
                [0x47f9_d498_7330_ca9d, 0x661b_4721_55d6_fefd],
            ),
            (
                Variant::Slate,
                [0x0b6d_0512_9f4c_e1f8, 0x362f_22f4_3b2d_33b8],
            ),
            (
                Variant::Sapphire,
                [0x48f9_48b4_ab9a_18ab, 0x2a6f_0b15_cb00_d37d],
            ),
            (
                Variant::Obsidian,
                [0x48f9_48b4_ab9a_18ab, 0x2a6f_0b15_cb00_d37d],
            ),
            (
                Variant::Ember,
                [0x0b6d_0512_9f4c_e1f8, 0x362f_22f4_3b2d_33b8],
            ),
            (
                Variant::Cobalt,
                [0x48f9_48b4_ab9a_18ab, 0x2a6f_0b15_cb00_d37d],
            ),
        ];
        for (v, hashes) in want {
            for (d, want) in [1, 2].into_iter().zip(hashes) {
                let got = fnv(&img(v, d).atlas);
                assert_eq!(got, want, "{v:?} at {d}: {got:#x}");
            }
        }
    }
}
