//! Reprise Blue's taskbar button, drawn from its scheme's `TaskButtonFaces` at the output's scale.
//! Top/bottom rings take the `down` profile, side rings the `across` profile; the fill shifts by `across`.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::Insets;
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, PartImage, State};

const RADIUS: f64 = 2.0;

/// How far the body sits in from the frame's left, top, right and bottom
/// edges in the glossy schemes (slate, sapphire, obsidian, ember, cobalt):
/// one pixel of the bar shows round the sides and top, none under it. Navy,
/// olive green and silver fill the frame.
const GLOSS_INSET: [i32; 4] = [1, 1, 1, 0];

/// The frames in atlas order; a held button draws its checked face.
const STATES: [State; 5] = [
    State::Normal,
    State::Hot,
    State::Checked,
    State::CheckedHot,
    State::Pressed,
];

/// Luminance of a premultiplied opaque colour, plus a floor that keeps a
/// near-black middle column from blowing the ratio up.
fn lum(p: Px) -> f64 {
    0.30 * p.1[0] + 0.59 * p.1[1] + 0.11 * p.1[2] + 4.0
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn inset(v: Variant) -> [i32; 4] {
    match v {
        Variant::Slate
        | Variant::Sapphire
        | Variant::Obsidian
        | Variant::Ember
        | Variant::Cobalt => GLOSS_INSET,
        Variant::Blue | Variant::Olive | Variant::Silver => [0; 4],
    }
}

fn frame(face: &Face, w: i32, h: i32, left: i32, [l, t, r, b]: [i32; 4], d: i32) -> Vec<Px> {
    let (down, across) = (stops(face.down), stops(face.across));
    let (down, across) = (Stops(&down), Stops(&across));
    let (wf, hf) = (f64::from(w), f64::from(h));
    let col = |x: f64| across.at((x + 0.5) / wf);
    let row = |y: f64| down.at((y + 0.5) / hf);
    let middle = lum(col(f64::from(left)));
    let (bw, bh) = (w - l - r, h - t - b);
    let body = shape::ringed(bw, bh, RADIUS, 2, d, |x, y, ring| {
        let (x, y) = (x + f64::from(l), y + f64::from(t));
        if ring < 2 {
            let i = ring as f64;
            let edges = [
                y - f64::from(t),
                f64::from(h - 1 - b) - y,
                x - f64::from(l),
                f64::from(w - 1 - r) - x,
            ];
            // The nearest edge; the top and bottom win a tie, so a corner
            // pixel of the ring takes the row's colour.
            let side = (0..4).fold(0, |m, s| if edges[s] < edges[m] { s } else { m });
            return match side {
                0 => row(f64::from(t) + i),
                1 => row(f64::from(h - 1 - b) - i),
                2 => col(f64::from(l) + i),
                _ => col(f64::from(w - 1 - r) - i),
            };
        }
        let k = lum(col(x)) / middle;
        let p = row(y);
        (255.0, p.1.map(|c| (c * k).clamp(0.0, 255.0)))
    });
    let (dw, dh) = (w * d, h * d);
    let mut out = vec![shape::CLEAR; (dw * dh) as usize];
    for y in 0..bh * d {
        for x in 0..bw * d {
            out[((y + t * d) * dw + x + l * d) as usize] = body[(y * bw * d + x) as usize];
        }
    }
    out
}

/// It never stretches vertically on the bar, so the sizing
/// insets carry no top or bottom margin.
pub fn task_button(v: Variant, d: i32) -> PartImage {
    let s = v.scheme();
    let m = s.metrics;
    let f = &s.task_button;
    let (w, h) = (m.task_left + 1 + m.task_right, m.task_height);
    let faces = [&f.normal, &f.hot, &f.checked, &f.checked_hot, &f.checked];
    let frames: Vec<Vec<Px>> = faces
        .iter()
        .map(|face| frame(face, w, h, m.task_left, inset(v), d))
        .collect();
    let (dw, dh) = (w * d, h * d);
    let atlas = shape::draw(w, h * faces.len() as i32, d, |x, y| {
        let (x, y) = (x as i32, y as i32);
        frames[(y / dh) as usize][((y % dh) * dw + x) as usize]
    });
    PartImage::new(atlas, &STATES, Layout::Vertical)
        .sizing(Insets::new(m.task_left, 0, m.task_right, 0))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(img: &PartImage) -> u64 {
        let a = &img.atlas;
        [a.width, a.height]
            .into_iter()
            .chain(a.pixels.iter().copied())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn pixels(img: &PartImage, state: State) -> Vec<u32> {
        let f = img.frame(state);
        (f.y..f.y + f.height)
            .flat_map(|y| (f.x..f.x + f.width).map(move |x| img.atlas.get(x, y)))
            .collect()
    }

    #[test]
    fn task_button_pins_at_scale_1_and_2() {
        let want: [(Variant, u64, u64); 7] = [
            (Variant::Blue, 0xCE2B_2120_8196_01E6, 0x06CF_F971_39D9_403D),
            (Variant::Olive, 0x1B44_34D0_3779_E596, 0x1D49_AA83_6491_DE2A),
            (
                Variant::Silver,
                0xF9A5_F1CC_C2A4_7136,
                0xC6E3_CAFE_E14F_9E9A,
            ),
            (Variant::Slate, 0x9579_F9EE_75AF_EDB8, 0xADDF_3378_35F6_3C96),
            (
                Variant::Sapphire,
                0xD64A_5D50_7A65_8319,
                0x6FA9_1C68_8EE0_E825,
            ),
            (
                Variant::Obsidian,
                0x3E8E_9C35_9035_659D,
                0x6E3B_6960_F22E_1670,
            ),
            (Variant::Ember, 0x5785_778C_2917_0D35, 0xB5D9_C427_E99B_67B8),
        ];
        let got: Vec<_> = want
            .iter()
            .map(|&(v, _, _)| (v, fnv(&task_button(v, 1)), fnv(&task_button(v, 2))))
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn task_button_frames_follow_the_scheme_metrics() {
        for (v, right) in [
            (Variant::Blue, 4),
            (Variant::Olive, 4),
            (Variant::Silver, 4),
            (Variant::Slate, 12),
            (Variant::Sapphire, 12),
            (Variant::Obsidian, 12),
            (Variant::Ember, 13),
            (Variant::Cobalt, 12),
        ] {
            let (one, two) = (task_button(v, 1), task_button(v, 2));
            let f = one.frame(State::Normal);
            assert_eq!((f.width, f.height), (14 + right, 25), "{v:?} frame");
            assert_eq!(one.sizing, Insets::new(13, 0, right, 0), "{v:?} sizing");
            assert_eq!(two.sizing, one.sizing, "{v:?} design insets at 2");
            let g = two.frame(State::Normal);
            assert_eq!((g.width, g.height), (2 * f.width, 50), "{v:?} frame at 2");
            assert_eq!((one.scale, two.scale), (1, 2), "{v:?} scale");
        }
    }

    #[test]
    fn task_button_states_differ_and_pressed_is_checked() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let b = task_button(v, d);
                let lit = [State::Normal, State::Hot, State::Checked, State::CheckedHot];
                for (i, &a) in lit.iter().enumerate() {
                    for &c in &lit[i + 1..] {
                        assert_ne!(pixels(&b, a), pixels(&b, c), "{v:?} {a:?} vs {c:?} at {d}");
                    }
                }
                assert_eq!(
                    pixels(&b, State::Pressed),
                    pixels(&b, State::Checked),
                    "{v:?} pressed is checked at {d}"
                );
            }
        }
    }

    #[test]
    fn task_button_corners_are_clear_and_soft_at_2() {
        let alpha = |img: &PartImage, x: i32, y: i32| {
            let f = img.frame(State::Normal);
            img.atlas.get(f.x + x, f.y + y) >> 24
        };
        for v in Variant::ALL {
            let one = task_button(v, 1);
            let f = one.frame(State::Normal);
            let (w, h) = (f.width, f.height);
            for (x, y) in [(0, 0), (w - 1, 0)] {
                assert!(
                    alpha(&one, x, y) < 128,
                    "{v:?} corner ({x}, {y}) is mostly clear"
                );
            }
            assert_eq!(alpha(&one, 13, 12), 255, "{v:?} face is opaque");
            let two = task_button(v, 2);
            let [l, t, r, b] = inset(v);
            let (w2, h2) = (2 * w, 2 * h);
            for (cx, cy, sx, sy) in [
                (2 * l, 2 * t, 1, 1),
                (w2 - 1 - 2 * r, 2 * t, -1, 1),
                (2 * l, h2 - 1 - 2 * b, 1, -1),
                (w2 - 1 - 2 * r, h2 - 1 - 2 * b, -1, -1),
            ] {
                let soft = (0..4).any(|k| {
                    (0..4).any(|j| {
                        let a = alpha(&two, cx + sx * k, cy + sy * j);
                        a > 0 && a < 255
                    })
                });
                assert!(soft, "{v:?} corner at ({cx}, {cy}) is anti-aliased at 2");
            }
        }
    }

    #[test]
    fn task_button_edges_leave_no_dark_fringe() {
        for v in Variant::ALL {
            let b = task_button(v, 2);
            for state in STATES {
                let f = b.frame(state);
                let at = |x: i32, y: i32| b.atlas.get(f.x + x, f.y + y);
                for y in 0..f.height {
                    for x in 0..f.width {
                        let p = at(x, y);
                        let a = p >> 24;
                        if a == 0 || a == 255 {
                            continue;
                        }
                        let near: Vec<u32> = (-2..=2)
                            .flat_map(|j| (-2..=2).map(move |i| (x + i, y + j)))
                            .filter(|&(i, j)| i >= 0 && j >= 0 && i < f.width && j < f.height)
                            .map(|(i, j)| at(i, j))
                            .filter(|q| q >> 24 == 255)
                            .collect();
                        for shift in [16, 8, 0] {
                            let ch = |q: u32| (q >> shift) & 0xFF;
                            let lo = near.iter().map(|&q| ch(q)).min().unwrap_or(0);
                            let hi = near.iter().map(|&q| ch(q)).max().unwrap_or(255);
                            let un = (ch(p) * 255 + a / 2) / a;
                            assert!(
                                un + 8 >= lo && un <= hi + 8,
                                "{v:?} {state:?} ({x}, {y}): {un} outside {lo}..={hi}"
                            );
                        }
                    }
                }
            }
        }
    }
}
