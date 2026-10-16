//! The radio button's [`Part::Radio`] atlas: eight 13x13 frames, Unchecked then
//! Checked, each Normal, Hot, Pressed, Disabled, drawn at the style's scale.
//!
//! The button is a disc 13 px across: a dark rim a little over 1 px wide round a
//! well shaded diagonally, grey at the top-left to white at the bottom-right.
//! Hot rings the well with an orange band round a pale core; Pressed darkens the
//! well. Greyed is a thinner grey rim round a clear well. The checked dot is 5 px
//! across, lit at its upper left and shaded toward its lower right; greyed, it is
//! flat. Colours come from the scheme's radio faces.

use super::checkbox::{Well, solid};
use crate::blue::Variant;
use crate::blue::scheme::{Face, RadioFaces};
use crate::shape::{Px, Stops, centre, draw, ellipse, mix, over, scale, shares};
use crate::style::{CHECK_STATES, Check, Layout, Part, PartImage, State};
use crate::theme::Color;

const SIZE: i32 = 13;

/// The disc's radius, and the well's inside the rim.
const R: f64 = 6.5;
const WELL_R: f64 = 5.4;

/// The well's radius inside a greyed radio's thinner rim.
const GREYED_WELL_R: f64 = 5.5;

/// The pale core inside the Hot band.
const HOT_CORE_R: f64 = 3.5;

/// The well's first and last design pixel along its diagonal.
const WELL: (i32, i32) = (2, SIZE - 3);

/// The dot's radius, and how far up and left of the middle its light sits.
const DOT_R: f64 = 2.6;
const DOT_LIGHT_AT: f64 = 1.0;

/// How much of white lights the dot at its light, and of black shades it at
/// its far edge.
const DOT_LIGHT: f64 = 0.3;
const DOT_SHADE: f64 = 0.3;

const CHECKS: [Check; 2] = [Check::Unchecked, Check::Checked];

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    vec![(Part::Radio, radio(v, d))]
}

fn radio(v: Variant, d: i32) -> PartImage {
    let faces = &v.scheme().radio;
    let frames: Vec<Frame> = CHECKS
        .iter()
        .flat_map(|&check| CHECK_STATES.map(|state| Frame::new(faces, check, state)))
        .collect();
    let a = draw(SIZE, SIZE * frames.len() as i32, d, |x, y| {
        let i = y as i32 / (SIZE * d);
        frames[i as usize].paint(x, y - f64::from(i * SIZE * d), d)
    });
    let mut img = PartImage::new_check(a, &CHECKS, Layout::Vertical).at_scale(d);
    img.true_size = true;
    img
}

struct Frame {
    rim: Px,
    well: Option<Well>,
    well_r: f64,
    hot: bool,
    dot: Option<(Px, bool)>,
}

impl Frame {
    fn new(f: &RadioFaces, check: Check, state: State) -> Frame {
        let face = face(f, check, state);
        let greyed = state == State::Disabled;
        Frame {
            rim: solid(face.rim),
            well: Well::of(face.down, WELL.0, WELL.1),
            well_r: if greyed { GREYED_WELL_R } else { WELL_R },
            hot: state == State::Hot,
            dot: match check {
                Check::Checked => face.mark.map(|c| (solid(c), !greyed)),
                _ => None,
            },
        }
    }

    fn paint(&self, x: f64, y: f64, d: i32) -> Px {
        let k = f64::from(d);
        let disc = |r: f64| ellipse(x, y, R * k, R * k, r * k, r * k);
        let (outer, inner) = (disc(R), disc(self.well_r));
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        let base = match self.well {
            None => shares(&[(self.rim, outer - inner)]),
            Some(well) => {
                let shade = well.diagonal(cx, cy, WELL.0, WELL.1);
                if self.hot {
                    let core = disc(HOT_CORE_R);
                    shares(&[
                        (self.rim, outer - inner),
                        (shade, inner - core),
                        (well.mid, core),
                    ])
                } else {
                    shares(&[(self.rim, outer - inner), (shade, inner)])
                }
            }
        };
        let Some((ink, shaded)) = self.dot else {
            return base;
        };
        let ink = if shaded { dot_shade(ink, cx, cy) } else { ink };
        over(scale(ink, disc(DOT_R)), base)
    }
}

/// The dot's colour at (`cx`, `cy`) in design px: `ink` lit toward white up
/// and left of the middle, shaded toward black at its far edge.
fn dot_shade(ink: Px, cx: f64, cy: f64) -> Px {
    let mid = f64::from(SIZE - 1) / 2.0;
    let light = mid - DOT_LIGHT_AT;
    let t = (cx - light).hypot(cy - light) / (DOT_R + DOT_LIGHT_AT * std::f64::consts::SQRT_2);
    let white = solid(Color::rgb(0xFF, 0xFF, 0xFF));
    let black = solid(Color::rgb(0, 0, 0));
    Stops(&[
        (0.0, mix(ink, white, DOT_LIGHT)),
        (0.5, ink),
        (1.0, mix(ink, black, DOT_SHADE)),
    ])
    .at(t)
}

fn face(f: &RadioFaces, check: Check, state: State) -> &Face {
    match (check, state) {
        (Check::Checked, State::Hot) => &f.hot_checked,
        (Check::Checked, State::Pressed) => &f.pressed_checked,
        (Check::Checked, State::Disabled) => &f.disabled_checked,
        (Check::Checked, _) => &f.normal_checked,
        (_, State::Hot) => &f.hot,
        (_, State::Pressed) => &f.pressed,
        (_, State::Disabled) => &f.disabled,
        _ => &f.normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_px(img: &PartImage, check: Check, state: State, x: i32, y: i32) -> u32 {
        let f = img.check_frame(check, state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn fnv(img: &PartImage) -> u64 {
        let a = &img.atlas;
        [a.width, a.height].iter().chain(a.pixels.iter()).fold(
            0xcbf2_9ce4_8422_2325_u64,
            |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            },
        )
    }

    fn straight(p: u32) -> [u32; 3] {
        let a = p >> 24;
        [16, 8, 0].map(|s| (((p >> s) & 0xFF) * 255 + a / 2) / a.max(1))
    }

    /// Every frame is 13 design px square at d = 1 and 2, true to its
    /// size, and every check and state draws its own frame.
    #[test]
    fn every_frame_is_13_square_and_its_own() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = radio(v, d);
                assert!(img.true_size, "{v:?}");
                assert_eq!(img.scale, d, "{v:?} at {d}");
                assert_eq!(img.sizing, crate::raster::Insets::ZERO, "{v:?}");
                let mut seen: Vec<Vec<u32>> = Vec::new();
                for check in CHECKS {
                    for state in CHECK_STATES {
                        let f = img.check_frame(check, state);
                        assert_eq!(
                            (f.width, f.height),
                            (13 * d, 13 * d),
                            "{v:?} {check:?} {state:?}"
                        );
                        let px = img.atlas.copy_rect(f).pixels;
                        assert!(
                            !seen.contains(&px),
                            "{v:?} {check:?} {state:?} at {d} is its own frame"
                        );
                        seen.push(px);
                    }
                }
            }
        }
    }

    /// The corners are clear, the rim's outer edge blends in the rim's own
    /// colour (no dark fringe) and softly at d = 2, the middle of the rim
    /// is solid, and a greyed well is clear in every scheme.
    #[test]
    fn the_disc_is_round_soft_edged_and_greyed_hollow() {
        for v in Variant::ALL {
            let f = &v.scheme().radio;
            for d in [1, 2] {
                let img = radio(v, d);
                let n = 13 * d;
                let mid = n / 2;
                for check in CHECKS {
                    for state in CHECK_STATES {
                        let what = format!("{v:?} {check:?} {state:?} at {d}");
                        for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                            assert_eq!(frame_px(&img, check, state, x, y), 0, "{what}: corner");
                        }
                        let face = face(f, check, state);
                        let want = [face.rim.r, face.rim.g, face.rim.b].map(u32::from);
                        let edge = (0..n)
                            .map(|x| frame_px(&img, check, state, x, mid))
                            .take_while(|p| p >> 24 < 0xFF);
                        for p in edge.filter(|p| *p >> 24 > 0x40) {
                            let got = straight(p);
                            assert!(
                                (0..3).all(|i| got[i].abs_diff(want[i]) <= 3),
                                "{what}: outer edge {p:08X} is the rim's colour"
                            );
                        }
                        if d == 2 {
                            assert!(
                                (0..n).any(|x| {
                                    let a = frame_px(&img, check, state, x, 3) >> 24;
                                    a > 0 && a < 0xFF
                                }),
                                "{what}: the edge blends"
                            );
                        }
                        if state == State::Disabled {
                            assert_eq!(
                                frame_px(&img, check, state, 3 * d, 3 * d),
                                0,
                                "{what}: greyed well is clear"
                            );
                        } else if d == 2 {
                            assert_eq!(
                                frame_px(&img, check, state, 1, mid),
                                0xFF00_0000 | face.rim.argb(),
                                "{what}: solid rim"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A checked radio has its dot in the middle and an unchecked one none;
    /// the dot is lit at its upper left, and flat in the face's mark when
    /// greyed.
    #[test]
    fn a_checked_radio_has_a_dot() {
        for v in Variant::ALL {
            let f = &v.scheme().radio;
            let img = radio(v, 1);
            for state in CHECK_STATES {
                let on = frame_px(&img, Check::Checked, state, 6, 6);
                let off = frame_px(&img, Check::Unchecked, state, 6, 6);
                assert_ne!(on, off, "{v:?} {state:?} dot");
            }
            assert_eq!(
                frame_px(&img, Check::Checked, State::Disabled, 6, 6),
                0xFF00_0000 | f.disabled_checked.mark.unwrap().argb(),
                "{v:?} greyed dot"
            );
            let lit = frame_px(&img, Check::Checked, State::Normal, 5, 5);
            let shaded = frame_px(&img, Check::Checked, State::Normal, 7, 7);
            let sum = |p: u32| straight(p).iter().sum::<u32>();
            assert!(
                sum(lit) > sum(shaded),
                "{v:?}: the dot is lit at its upper left"
            );
        }
    }

    /// Each scheme's atlas at d = 1 and 2, hashed with its size.
    #[test]
    fn radio_pins_at_scale_1_and_2() {
        let got = PINS.map(|(v, d, _)| (v, d, fnv(&radio(v, d))));
        assert!(
            got == PINS,
            "pins changed; now:\n{}",
            got.map(|(v, d, h)| format!("(Variant::{v:?}, {d}, {h:#018x}),"))
                .join("\n")
        );
    }

    const PINS: [(Variant, i32, u64); 16] = [
        (Variant::Blue, 1, 0x67f782257de9c673),
        (Variant::Blue, 2, 0x2f89408403baa09a),
        (Variant::Olive, 1, 0x67f782257de9c673),
        (Variant::Olive, 2, 0x2f89408403baa09a),
        (Variant::Silver, 1, 0x67f782257de9c673),
        (Variant::Silver, 2, 0x2f89408403baa09a),
        (Variant::Slate, 1, 0xcb2660acb3593be7),
        (Variant::Slate, 2, 0xe67cff9caecb62b4),
        (Variant::Sapphire, 1, 0xcb2660acb3593be7),
        (Variant::Sapphire, 2, 0xe67cff9caecb62b4),
        (Variant::Obsidian, 1, 0xcb2660acb3593be7),
        (Variant::Obsidian, 2, 0xe67cff9caecb62b4),
        (Variant::Ember, 1, 0x2f2625a1e277da1e),
        (Variant::Ember, 2, 0xa1bb40b35b80e52c),
        (Variant::Cobalt, 1, 0xd348381bd6611eba),
        (Variant::Cobalt, 2, 0x516f04107dd52d6a),
    ];
}
