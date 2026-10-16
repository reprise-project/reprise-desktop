//! The check box's [`Part::CheckBox`] atlas: twelve 13x13 frames, Unchecked,
//! Checked and Mixed in turn, each Normal, Hot, Pressed, Disabled.
//!
//! A 1 px rim round a diagonally shaded well. Hot adds a 2 px inner band
//! round a flat core. Disabled has a grey rim and a flat well, or a clear
//! one where the scheme's greyed face has no fill (silver). The tick has
//! 45 degree edges; the mixed mark is a 7x7 square 3 px in.

use crate::blue::Variant;
use crate::blue::scheme::{CheckBoxFaces, Face, Grad};
use crate::shape::{CLEAR, Px, Stops, centre, draw, over, polygon, premul, scale};
use crate::style::{CHECK_STATES, Check, Layout, Part, PartImage, State};
use crate::theme::Color;

const BOX: i32 = 13;

const WELL: (i32, i32) = (1, BOX - 2);

const HOT_BAND: i32 = 2;

const MIXED_AT: i32 = 3;
const MIXED_SIDE: i32 = 7;

/// The tick's outline in design px: a short left arm and a long right arm, 3 px tall per column.
const TICK: [(f64, f64); 6] = [
    (3.0, 4.5),
    (5.5, 7.0),
    (10.0, 2.5),
    (10.0, 5.5),
    (5.5, 10.0),
    (3.0, 7.5),
];

const CHECKS: [Check; 3] = [Check::Unchecked, Check::Checked, Check::Mixed];

/// This control's parts in `v`'s colours at `d` device pixels per design pixel.
pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    vec![(Part::CheckBox, check_box(v, d))]
}

/// `v`'s twelve frames at `d` stacked top to bottom, in [`PartImage::new_check`] order.
fn check_box(v: Variant, d: i32) -> PartImage {
    let faces = &v.scheme().check_box;
    let frames: Vec<Frame> = CHECKS
        .iter()
        .flat_map(|&check| CHECK_STATES.map(|state| Frame::new(faces, check, state)))
        .collect();
    let a = draw(BOX, BOX * frames.len() as i32, d, |x, y| {
        let i = y as i32 / (BOX * d);
        frames[i as usize].paint(x, y - f64::from(i * BOX * d), d)
    });
    PartImage::new_check(a, &CHECKS, Layout::Vertical).at_scale(d)
}

struct Frame {
    rim: Px,
    well: Option<Well>,
    hot: bool,
    mark: Option<(Check, Px)>,
}

impl Frame {
    fn new(f: &CheckBoxFaces, check: Check, state: State) -> Frame {
        let face = face(f, check, state);
        // The Hot face's mark names its band's colour; a lit box keeps its resting tick.
        let ink = match (check, state) {
            (Check::Checked, State::Hot) => f.normal_checked.mark,
            _ => face.mark,
        };
        Frame {
            rim: solid(face.rim),
            well: Well::of(face.down, WELL.0, WELL.1),
            hot: state == State::Hot,
            mark: match check {
                Check::Unchecked => None,
                _ => ink.map(|c| (check, solid(c))),
            },
        }
    }

    fn paint(&self, x: f64, y: f64, d: i32) -> Px {
        let (i, j) = (x as i32 / d, y as i32 / d);
        let edge = |v: i32| v < WELL.0 || v > WELL.1;
        if edge(i) || edge(j) {
            return self.rim;
        }
        let Some(well) = self.well else {
            return self.mark_over(CLEAR, x, y, d);
        };
        let (cx, cy) = (centre(x as i32, d), centre(y as i32, d));
        let band = |v: i32| v < WELL.0 + HOT_BAND || v > WELL.1 - HOT_BAND;
        let base = if self.hot && !band(i) && !band(j) {
            well.mid
        } else {
            well.diagonal(cx, cy, WELL.0, WELL.1)
        };
        self.mark_over(base, x, y, d)
    }

    fn mark_over(&self, base: Px, x: f64, y: f64, d: i32) -> Px {
        let Some((check, ink)) = self.mark else {
            return base;
        };
        let k = f64::from(d);
        let cover = match check {
            Check::Mixed => {
                let inside = |v: f64| {
                    let v = v as i32 / d;
                    (MIXED_AT..MIXED_AT + MIXED_SIDE).contains(&v)
                };
                if inside(x) && inside(y) { 1.0 } else { 0.0 }
            }
            _ => polygon(x, y, &TICK.map(|(px, py)| (px * k, py * k))),
        };
        over(scale(ink, cover), base)
    }
}

fn face(f: &CheckBoxFaces, check: Check, state: State) -> &Face {
    match (check, state) {
        (Check::Unchecked, State::Hot) => &f.hot,
        (Check::Unchecked, State::Pressed) => &f.pressed,
        (Check::Unchecked, State::Disabled) => &f.disabled,
        (Check::Unchecked, _) => &f.normal,
        (Check::Checked, State::Hot) => &f.hot_checked,
        (Check::Checked, State::Pressed) => &f.pressed_checked,
        (Check::Checked, State::Disabled) => &f.disabled_checked,
        (Check::Checked, _) => &f.normal_checked,
        (Check::Mixed, State::Hot) => &f.hot_mixed,
        (Check::Mixed, State::Pressed) => &f.pressed_mixed,
        (Check::Mixed, State::Disabled) => &f.disabled_mixed,
        (Check::Mixed, _) => &f.normal_mixed,
    }
}

pub(super) fn solid(c: Color) -> Px {
    premul(c, 255)
}

/// A 13 px check part's well: colours at top-left, middle and bottom-right from a face's `down` gradient.
#[derive(Clone, Copy, Debug)]
pub(super) struct Well {
    pub tl: Px,
    pub mid: Px,
    pub br: Px,
}

impl Well {
    /// The well pixels `first..=last` of a face's `down` span; None where no stop falls there (a clear well).
    pub(super) fn of(down: Grad, first: i32, last: i32) -> Option<Well> {
        let at = |v: i32| (v as f32 + 0.5) / BOX as f32;
        let span = at(first) - 0.01..=at(last) + 0.01;
        let mut inside = down.iter().filter(|(t, _)| span.contains(t));
        let tl = inside.next()?.1;
        let br = inside.next_back().map_or(tl, |s| s.1);
        let stops: Vec<(f64, Px)> = down
            .iter()
            .map(|&(t, c)| (f64::from(t), solid(c)))
            .collect();
        Some(Well {
            tl: solid(tl),
            mid: Stops(&stops).at(0.5),
            br: solid(br),
        })
    }

    pub(super) fn diagonal(&self, cx: f64, cy: f64, first: i32, last: i32) -> Px {
        let (a, b) = (f64::from(first), f64::from(last));
        let u = (cx - a + cy - a) / (2.0 * (b - a));
        Stops(&[(0.0, self.tl), (1.0, self.br)]).at(u)
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
        let head = [a.width, a.height];
        head.iter()
            .chain(a.pixels.iter())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    #[test]
    fn every_frame_is_13_square_and_its_own() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = check_box(v, d);
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

    /// The rim is the face's rim all round; the box is opaque except where a greyed face has no well.
    #[test]
    fn the_rim_is_the_faces_and_only_a_bare_greyed_well_is_clear() {
        for v in Variant::ALL {
            let f = &v.scheme().check_box;
            for d in [1, 2] {
                let img = check_box(v, d);
                for check in CHECKS {
                    for state in CHECK_STATES {
                        let face = face(f, check, state);
                        let rim = 0xFF00_0000 | face.rim.argb();
                        for (x, y) in [(0, 0), (13 * d - 1, 6 * d), (6 * d, 13 * d - 1)] {
                            assert_eq!(
                                frame_px(&img, check, state, x, y),
                                rim,
                                "{v:?} {check:?} {state:?} rim"
                            );
                        }
                        let a = frame_px(&img, check, state, d, d) >> 24;
                        let clear = face.down.len() <= 1;
                        assert_eq!(
                            a,
                            if clear { 0 } else { 0xFF },
                            "{v:?} {check:?} {state:?} well at {d}"
                        );
                    }
                }
            }
        }
        assert!(
            frame_px(
                &check_box(Variant::Silver, 1),
                Check::Unchecked,
                State::Disabled,
                6,
                6
            ) == 0,
            "silver's greyed well is clear"
        );
    }

    /// The tick has solid pixels on both arms at d = 1 and soft edges at d = 2; the mixed square spans (3, 3) to (9, 9).
    #[test]
    fn the_tick_is_solid_at_1_and_smooth_at_2() {
        for v in Variant::ALL {
            let f = &v.scheme().check_box;
            let one = check_box(v, 1);
            let ink = 0xFF00_0000 | f.normal_checked.mark.unwrap().argb();
            for (x, y) in [(3, 6), (4, 7), (5, 8), (6, 7), (9, 4)] {
                assert_eq!(
                    frame_px(&one, Check::Checked, State::Normal, x, y),
                    ink,
                    "{v:?} tick ({x}, {y})"
                );
                assert_eq!(
                    frame_px(&one, Check::Checked, State::Hot, x, y),
                    ink,
                    "{v:?} hot tick ({x}, {y})"
                );
            }
            let mixed = 0xFF00_0000 | f.normal_mixed.mark.unwrap().argb();
            for (x, y) in [(3, 3), (9, 9)] {
                assert_eq!(
                    frame_px(&one, Check::Mixed, State::Normal, x, y),
                    mixed,
                    "{v:?} mixed ({x}, {y})"
                );
            }
            assert_ne!(
                frame_px(&one, Check::Mixed, State::Normal, 2, 2),
                mixed,
                "{v:?} mixed is 3 px in"
            );
            assert_ne!(
                frame_px(&one, Check::Mixed, State::Normal, 10, 10),
                mixed,
                "{v:?} mixed is 7 px wide"
            );
            let two = check_box(v, 2);
            let f2 = two.check_frame(Check::Checked, State::Normal);
            let unchecked = two.check_frame(Check::Unchecked, State::Normal);
            let soft = (0..f2.height)
                .flat_map(|y| (0..f2.width).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let (a, b) = (
                        two.atlas.get(f2.x + x, f2.y + y),
                        two.atlas.get(unchecked.x + x, unchecked.y + y),
                    );
                    a != b && a != ink
                })
                .count();
            assert!(soft >= 20, "{v:?}: the tick's edges blend at 2 ({soft} px)");
        }
    }

    #[test]
    fn check_box_pins_at_scale_1_and_2() {
        let got = PINS.map(|(v, d, _)| (v, d, fnv(&check_box(v, d))));
        assert!(
            got == PINS,
            "pins changed; now:\n{}",
            got.map(|(v, d, h)| format!("(Variant::{v:?}, {d}, {h:#018x}),"))
                .join("\n")
        );
    }

    const PINS: [(Variant, i32, u64); 20] = [
        (Variant::Blue, 1, 0xd931a3cbbe9e63b8),
        (Variant::Blue, 2, 0x349b4cac9bf90e62),
        (Variant::Olive, 1, 0xd931a3cbbe9e63b8),
        (Variant::Olive, 2, 0x349b4cac9bf90e62),
        (Variant::Silver, 1, 0xf2730b61020c77cf),
        (Variant::Silver, 2, 0x54defad4b8b6cb06),
        (Variant::Slate, 1, 0xf47f60553d41f9eb),
        (Variant::Slate, 2, 0x46a37933b1b3005b),
        (Variant::Sapphire, 1, 0xf47f60553d41f9eb),
        (Variant::Sapphire, 2, 0x46a37933b1b3005b),
        (Variant::Obsidian, 1, 0xf47f60553d41f9eb),
        (Variant::Obsidian, 2, 0x46a37933b1b3005b),
        (Variant::Ember, 1, 0x5536416c72a84c0c),
        (Variant::Ember, 2, 0x8c4efdd042a17379),
        (Variant::Cobalt, 1, 0xa1de5c18217922a4),
        (Variant::Cobalt, 2, 0xe98a707980ee53a0),
        (Variant::Carbon, 1, 0xf47f60553d41f9eb),
        (Variant::Carbon, 2, 0x46a37933b1b3005b),
        (Variant::Steel, 1, 0xc1d0e1a6b1d99033),
        (Variant::Steel, 2, 0xacabc9639b661b62),
    ];
}
