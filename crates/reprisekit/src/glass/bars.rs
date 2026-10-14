//! Glass's progress bar and trackbar parts, drawn at the style's scale.

use crate::raster::{Canvas, Insets};
use crate::shape::{
    CLEAR, Px, Stops, centre, draw, mix, over, polygon, premul, ringed, rounded, scale, shares,
};
use crate::style::{Layout, Part, PartImage, State};

use crate::theme::Color;

const BAR_W: i32 = 21;
const BAR_H: i32 = 18;
const BAR_RADIUS: f64 = 2.5;
const BAR_FRAME: [(f64, u8); 6] = [
    (0.0, 0xB0),
    (2.0, 0xAC),
    (6.5, 0x9C),
    (11.5, 0x9C),
    (15.0, 0x8E),
    (17.0, 0x89),
];
const TROUGH_TOP: u8 = 0xF3;
const TROUGH_BOTTOM: u8 = 0xE9;
const TROUGH: [(f64, u8); 6] = [
    (2.0, 0xFC),
    (3.0, 0xED),
    (6.5, 0xD7),
    (6.5, 0xC7),
    (11.5, 0xC7),
    (15.0, 0xD4),
];
const TROUGH_END_DARK: f64 = 0.26;
const TROUGH_END_FROM: f64 = 2.0;
const TROUGH_END_TO: f64 = 9.0;
const TROUGH_EDGE_LIGHT: f64 = 0.85;

const FILL_W: i32 = 53;
const FILL_H: i32 = 16;
const FILL_MID_COL: f64 = 26.0;
const FILL_MID: Color = Color::rgb(0x00, 0xCE, 0x31);
const FILL_SIDE: Color = Color::rgb(0x00, 0xAA, 0x2A);
const FILL_END: Color = Color::rgb(0x27, 0xC8, 0x41);
const FILL_FALL: f64 = 20.0;
const FILL_END_COLS: f64 = 5.0;
const FILL_GLOSS: [(f64, f64); 3] = [(1.0, 0.80), (2.0, 0.76), (5.0, 0.61)];
const FILL_GLOSS_END: f64 = 5.5;
const FILL_LIFT: Color = Color::rgb(0x5A, 0xFF, 0x64);
const FILL_LIFT_MAX: f64 = 0.3;
const FILL_LIFT_FROM: f64 = 10.5;

const TRACK_SHADOW: Color = Color::rgb(0xAD, 0xAE, 0xAE);
const TRACK_FACE: Color = Color::rgb(0xE6, 0xE9, 0xE9);
const TRACK_LIGHT: Color = Color::rgb(0xFD, 0xFC, 0xFD);
const TRACK_LIGHT_BOTTOM: Color = Color::rgb(0xFC, 0xFB, 0xFC);
const TRACK_CORNER_DARK: f64 = 0.88;
const TRACK_CORNER_LIGHT: f64 = 0.75;

const THUMB_W: i32 = 11;
const THUMB_H: i32 = 19;
const THUMB_BODY_W: f64 = 10.0;
const THUMB_SIDE: f64 = 12.5;
const THUMB_V_W: i32 = 23;
const THUMB_V_H: i32 = 11;
const THUMB_V_BODY: (i32, i32) = (21, 10);
const THUMB_RADIUS: f64 = 2.0;
const ARC_STEPS: usize = 8;
const FACE_TOP: f64 = 2.0;
const FACE_SPLIT: f64 = 8.5;
const FACE_END: f64 = 16.0;
const FACE_V_SPLIT: f64 = 4.5;
const FACE_V_END: f64 = 7.0;
const INNER_LIGHT: f64 = 0.75;
const FOCUS_TOP: Color = Color::rgb(0x3C, 0xED, 0xFD);
const FOCUS_BOTTOM: Color = Color::rgb(0x2B, 0xCF, 0xFB);
const SHADOW: u8 = 0x30;
const SHADOW_GREYED: u8 = 0x10;
const FOCUS_GLOW: Color = Color::rgb(0x3B, 0xE6, 0xFD);

const NORMAL_OUTLINE: Color = Color::rgb(0x6D, 0x6D, 0x6D);
const HOT_OUTLINE: Color = Color::rgb(0x3B, 0x77, 0xAD);
const PRESSED_OUTLINE: Color = Color::rgb(0x2B, 0x5B, 0x87);
const GREYED_OUTLINE: Color = Color::rgb(0xAB, 0xAF, 0xB3);
const NORMAL_FACE: [Color; 4] = [
    Color::rgb(0xF2, 0xF1, 0xF2),
    Color::rgb(0xEA, 0xE9, 0xEA),
    Color::rgb(0xDA, 0xD9, 0xDA),
    Color::rgb(0xD2, 0xD3, 0xD3),
];
const HOT_FACE: [Color; 4] = [
    Color::rgb(0xE8, 0xF5, 0xFD),
    Color::rgb(0xD7, 0xED, 0xFC),
    Color::rgb(0xBB, 0xE2, 0xFD),
    Color::rgb(0xAB, 0xD9, 0xF8),
];
const PRESSED_FACE: [Color; 4] = [
    Color::rgb(0xE4, 0xF2, 0xFC),
    Color::rgb(0xAD, 0xD9, 0xF6),
    Color::rgb(0x7A, 0xBF, 0xEB),
    Color::rgb(0x4C, 0x99, 0xCB),
];
const GREYED_FACE: Color = Color::rgb(0xF3, 0xF4, 0xF4);

/// A part's painter at a scale.
type Paint = fn(i32) -> PartImage;

/// Each bar part with its painter.
pub fn parts() -> [(Part, Paint); 6] {
    [
        (Part::ProgressBar, bar_part),
        (Part::ProgressChunk, fill_part),
        (Part::TrackbarTrack, track_part),
        (Part::TrackbarThumb, thumb_part),
        (Part::TrackbarTrackV, track_v_part),
        (Part::TrackbarThumbV, thumb_v_part),
    ]
}

fn bar_part(d: i32) -> PartImage {
    PartImage::new(progress_bar(d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(10, 7, 10, 6))
        .content(Insets::new(1, 1, 1, 1))
        .at_scale(d)
}

fn fill_part(d: i32) -> PartImage {
    PartImage::new(progress_fill(d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(26, 6, 26, 5))
        .clip_ends()
        .at_scale(d)
}

fn track_part(d: i32) -> PartImage {
    PartImage::new(channel(3, 4, d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(1, 0, 1, 0))
        .at_scale(d)
}

fn thumb_part(d: i32) -> PartImage {
    PartImage {
        true_size: true,
        ..PartImage::new(
            stack(THUMB_STATES.map(|s| thumb(&inks(s), d))),
            &THUMB_STATES,
            Layout::Vertical,
        )
        .at_scale(d)
    }
}

fn track_v_part(d: i32) -> PartImage {
    PartImage::new(channel(4, 3, d), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(0, 1, 0, 1))
        .outset(Insets::new(-1, 0, 1, 0))
        .at_scale(d)
}

fn thumb_v_part(d: i32) -> PartImage {
    PartImage {
        true_size: true,
        ..PartImage::new(
            stack(THUMB_STATES.map(|s| thumb_v(&inks(s), d))),
            &THUMB_STATES,
            Layout::Vertical,
        )
        .at_scale(d)
    }
}

fn stack<const N: usize>(frames: [Canvas; N]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut atlas = Canvas::new(w, h * N as u32);
    for (i, f) in frames.iter().enumerate() {
        atlas.blit_over(0, i as i32 * h as i32, f);
    }
    atlas
}

fn grey(v: f64) -> Px {
    (255.0, [v; 3])
}

fn opaque(c: Color) -> Px {
    premul(c, 255)
}

fn grey_at(stops: &[(f64, u8)], y: f64) -> f64 {
    let s: Vec<(f64, Px)> = stops
        .iter()
        .map(|&(t, v)| (t, grey(f64::from(v))))
        .collect();
    Stops(&s).at(y).1[0]
}

fn share_at(stops: &[(f64, f64)], y: f64) -> f64 {
    let s: Vec<(f64, Px)> = stops.iter().map(|&(t, v)| (t, (v, [0.0; 3]))).collect();
    Stops(&s).at(y).0
}

fn progress_bar(d: i32) -> Canvas {
    let last = f64::from(BAR_W - 1);
    let px = ringed(BAR_W, BAR_H, BAR_RADIUS, 1, d, |x, y, ring| {
        if ring == 0 {
            return grey(grey_at(&BAR_FRAME, y));
        }
        let t = if y < 1.5 {
            f64::from(TROUGH_TOP)
        } else if y > f64::from(BAR_H) - 2.5 {
            f64::from(TROUGH_BOTTOM)
        } else {
            grey_at(&TROUGH, y)
        };
        let depth = if x < 1.5 || x > last - 1.5 {
            TROUGH_EDGE_LIGHT
        } else {
            let from_end = x.min(last - x);
            let e = (TROUGH_END_TO - from_end) / (TROUGH_END_TO - TROUGH_END_FROM);
            1.0 + TROUGH_END_DARK * e.clamp(0.0, 1.0)
        };
        grey((255.0 - (255.0 - t) * depth).max(0.0))
    });
    draw(BAR_W, BAR_H, d, |x, y| {
        px[(y as i32 * BAR_W * d + x as i32) as usize]
    })
}

fn progress_fill(d: i32) -> Canvas {
    let last = f64::from(FILL_W - 1);
    let white = grey(255.0);
    draw(FILL_W, FILL_H, d, |x, y| {
        let (u, v) = (centre(x as i32, d), centre(y as i32, d));
        // The middle column stays flat so that it stretches cleanly.
        let off = ((u - FILL_MID_COL).abs() - 0.5).max(0.0);
        let body = mix(
            opaque(FILL_MID),
            opaque(FILL_SIDE),
            (off / FILL_FALL).min(1.0).powi(2),
        );
        let from_end = u.min(last - u).max(0.0);
        let body = if from_end < FILL_END_COLS {
            mix(body, opaque(FILL_END), 0.5_f64.powf(from_end))
        } else {
            body
        };
        if v < FILL_GLOSS_END {
            mix(body, white, share_at(&FILL_GLOSS, v))
        } else if v > FILL_LIFT_FROM {
            let k = (v - FILL_LIFT_FROM) / (f64::from(FILL_H - 1) - FILL_LIFT_FROM);
            mix(body, opaque(FILL_LIFT), FILL_LIFT_MAX * k.min(1.0).powi(2))
        } else {
            body
        }
    })
}

fn channel(w: i32, h: i32, d: i32) -> Canvas {
    let shadow = opaque(TRACK_SHADOW);
    let light = opaque(TRACK_LIGHT);
    let crossed = (255.0, scale(shadow, TRACK_CORNER_DARK).1);
    let meet = mix(shadow, light, TRACK_CORNER_LIGHT);
    draw(w, h, d, |x, y| {
        let (col, row) = (x as i32 / d, y as i32 / d);
        let (top, left) = (row == 0, col == 0);
        let (bottom, right) = (row == h - 1, col == w - 1);
        match (top, left, bottom, right) {
            (true, true, _, _) => crossed,
            (true, _, _, true) | (_, true, true, _) => meet,
            (_, _, _, true) => light,
            (_, _, true, _) => opaque(TRACK_LIGHT_BOTTOM),
            (true, _, _, _) | (_, true, _, _) => shadow,
            _ => opaque(TRACK_FACE),
        }
    })
}

fn face_at(face: &[Color; 4], y: f64, split: f64, end: f64) -> Px {
    let s = [
        (FACE_TOP, opaque(face[0])),
        (split, opaque(face[1])),
        (split, opaque(face[2])),
        (end, opaque(face[3])),
    ];
    Stops(&s).at(y)
}

fn inner_at(ink: &ThumbInks, face: Px, y: f64, h: f64) -> Px {
    if ink.focus {
        let s = [(1.0, opaque(FOCUS_TOP)), (h - 2.0, opaque(FOCUS_BOTTOM))];
        Stops(&s).at(y)
    } else {
        mix(face, grey(255.0), INNER_LIGHT)
    }
}

fn pointed(t: f64, k: f64, (dx, dy): (f64, f64)) -> Vec<(f64, f64)> {
    let (w, r) = (THUMB_BODY_W, (THUMB_RADIUS - t).max(0.0));
    let root2 = std::f64::consts::SQRT_2;
    let mut pts = Vec::with_capacity(2 * ARC_STEPS + 5);
    for (cx, from) in [(THUMB_RADIUS, 180.0_f64), (w - THUMB_RADIUS, 270.0)] {
        for i in 0..=ARC_STEPS {
            let a = (from + 90.0 * i as f64 / ARC_STEPS as f64).to_radians();
            pts.push((cx + r * a.cos(), THUMB_RADIUS + r * a.sin()));
        }
    }
    pts.push((w - t, THUMB_SIDE - t * (root2 - 1.0)));
    pts.push((w / 2.0, THUMB_SIDE + w / 2.0 - t * root2));
    pts.push((t, THUMB_SIDE - t * (root2 - 1.0)));
    pts.into_iter()
        .map(|(x, y)| ((x + dx) * k, (y + dy) * k))
        .collect()
}

fn thumb(ink: &ThumbInks, d: i32) -> Canvas {
    let k = f64::from(d);
    let rims = [0.0, 1.0, 2.0].map(|t| pointed(t, k, (0.0, 0.0)));
    let under = pointed(0.0, k, (1.0, 1.0));
    let outline = opaque(ink.outline);
    let h = THUMB_SIDE + THUMB_BODY_W / 2.0;
    draw(THUMB_W, THUMB_H, d, |x, y| {
        let c = rims.each_ref().map(|p| polygon(x, y, p));
        let v = centre(y as i32, d);
        let face = face_at(&ink.face, v, FACE_SPLIT, FACE_END);
        let body = shares(&[
            (outline, c[0] - c[1]),
            (inner_at(ink, face, v, h), c[1] - c[2]),
            (face, c[2]),
        ]);
        over(body, scale(ink.under, polygon(x, y, &under)))
    })
}

fn thumb_v(ink: &ThumbInks, d: i32) -> Canvas {
    let k = f64::from(d);
    let (bw, bh) = THUMB_V_BODY;
    let outline = opaque(ink.outline);
    let body = ringed(bw, bh, THUMB_RADIUS, 2, d, |_, y, ring| {
        let face = face_at(&ink.face, y, FACE_V_SPLIT, FACE_V_END);
        match ring {
            0 => outline,
            1 => inner_at(ink, face, y, f64::from(bh)),
            _ => face,
        }
    });
    let (w, h) = (f64::from(bw) * k, f64::from(bh) * k);
    draw(THUMB_V_W, THUMB_V_H, d, |x, y| {
        let (bx, by) = (x as i32 - d, y as i32);
        let top = if (0..bw * d).contains(&bx) && by < bh * d {
            body[(by * bw * d + bx) as usize]
        } else {
            CLEAR
        };
        let shade = rounded(x - 2.0 * k, y - k, w, h, THUMB_RADIUS * k);
        over(top, scale(ink.under, shade))
    })
}

const THUMB_STATES: [State; 5] = [
    State::Normal,
    State::Hot,
    State::Pressed,
    State::Focused,
    State::Disabled,
];

struct ThumbInks {
    outline: Color,
    face: [Color; 4],
    focus: bool,
    under: Px,
}

fn inks(state: State) -> ThumbInks {
    let black = Color::rgb(0, 0, 0);
    let shadow = premul(black, SHADOW);
    let (outline, face) = match state {
        State::Hot | State::Focused => (HOT_OUTLINE, HOT_FACE),
        State::Pressed => (PRESSED_OUTLINE, PRESSED_FACE),
        State::Disabled => (GREYED_OUTLINE, [GREYED_FACE; 4]),
        _ => (NORMAL_OUTLINE, NORMAL_FACE),
    };
    let under = match state {
        State::Focused => premul(FOCUS_GLOW, SHADOW),
        State::Disabled => premul(black, SHADOW_GREYED),
        _ => shadow,
    };
    ThumbInks {
        outline,
        face,
        focus: state == State::Focused,
        under,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::Paint;
    use crate::look::styled::fixture::doubles;
    use crate::look::widgets::{draw_progress, draw_trackbar, draw_vertical_trackbar};
    use crate::pointer::Interaction;
    use crate::raster::Rect;
    use crate::trackbar::{self, TrackBar};

    const FACE: Color = Color::rgb(0xEF, 0xEE, 0xEF);

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

    fn part(d: i32, p: Part) -> PartImage {
        parts().into_iter().find(|(q, _)| *q == p).unwrap().1(d)
    }

    const PARTS: [Part; 6] = [
        Part::ProgressBar,
        Part::ProgressChunk,
        Part::TrackbarTrack,
        Part::TrackbarThumb,
        Part::TrackbarTrackV,
        Part::TrackbarThumbV,
    ];

    #[test]
    fn bar_parts_pin_at_scale_1_and_2() {
        let want: [(Part, [u64; 2]); 6] = [
            (
                Part::ProgressBar,
                [0xff50_fa07_1f78_354a, 0xd273_b22c_6cdb_626b],
            ),
            (
                Part::ProgressChunk,
                [0xe39d_dcb1_02ef_7614, 0x7617_870a_2c67_01b7],
            ),
            (
                Part::TrackbarTrack,
                [0xf812_ed84_cccf_f700, 0xdfe0_f3fa_a2a2_1063],
            ),
            (
                Part::TrackbarThumb,
                [0x9530_e326_f21f_d8fa, 0x5476_fa2e_cd2d_8988],
            ),
            (
                Part::TrackbarTrackV,
                [0x0179_8908_5ead_4d8d, 0x90b5_0fb9_b45d_c403],
            ),
            (
                Part::TrackbarThumbV,
                [0xf287_2bea_527d_7043, 0x20a8_39f6_c091_a81b],
            ),
        ];
        let got = PARTS.map(|p| (p, [1, 2].map(|d| fnv(&part(d, p).atlas))));
        assert_eq!(got, want, "{got:#x?}");
    }

    #[test]
    fn bar_parts_are_painted_at_the_style_scale() {
        for p in PARTS {
            let one = part(1, p);
            for d in [2, 3] {
                let du = d as u32;
                let at = part(d, p);
                assert_eq!(
                    (at.atlas.width, at.atlas.height),
                    (one.atlas.width * du, one.atlas.height * du),
                    "{p:?} at {d}"
                );
                assert_eq!(at.scale, d, "{p:?} at {d}");
                assert_eq!(
                    (
                        at.sizing,
                        at.content,
                        at.outset,
                        at.true_size,
                        at.states.len()
                    ),
                    (
                        one.sizing,
                        one.content,
                        one.outset,
                        one.true_size,
                        one.states.len()
                    ),
                    "{p:?} at {d}: design insets"
                );
            }
        }
    }

    fn straight(p: u32) -> u32 {
        let a = p >> 24;
        let sum = (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        sum * 255 / (3 * a.max(1))
    }

    #[test]
    fn the_progress_bar_has_soft_corners_without_dark_fringes() {
        let one = part(1, Part::ProgressBar).atlas;
        assert!(one.get(0, 0) >> 24 < 0x40, "a clear top left corner");
        assert!(one.get(20, 17) >> 24 < 0x60, "a clear bottom right corner");
        for d in [1, 2] {
            let c = part(d, Part::ProgressBar).atlas;
            for (x, y) in
                (0..c.height as i32).flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            {
                let p = c.get(x, y);
                if p >> 24 > 0x10 {
                    assert!(
                        straight(p) >= 0x84,
                        "at {d}: ({x}, {y}) {p:08X} is no darker than the frame"
                    );
                }
            }
        }
        let two = part(2, Part::ProgressBar).atlas;
        assert!(
            (0..25).any(|i| (0x10..0xF0).contains(&(two.get(i % 5, i / 5) >> 24))),
            "the corner is soft at 2"
        );
    }

    #[test]
    fn the_progress_fill_is_glossy_and_flat_where_it_stretches() {
        for d in [1, 2] {
            let c = part(d, Part::ProgressChunk).atlas;
            assert!(c.pixels.iter().all(|p| p >> 24 == 0xFF), "at {d}: opaque");
            let mid = 26 * d;
            let row = |y: i32| (0..53 * d).map(|x| c.get(x, y)).collect::<Vec<_>>();
            for y in 6 * d..11 * d {
                assert_eq!(row(y), row(6 * d), "at {d}: row {y} stretches");
            }
            assert!(
                straight(c.get(mid, 6 * d - 1)) > straight(c.get(mid, 6 * d)) + 0x30,
                "at {d}: a hard step under the gloss"
            );
            for x in mid..mid + d {
                assert_eq!(
                    c.get(x, 8 * d),
                    c.get(mid, 8 * d),
                    "at {d}: the middle column"
                );
            }
            assert!(
                c.get(0, 8 * d) != c.get(6 * d, 8 * d),
                "at {d}: the end glows"
            );
        }
        let two = part(2, Part::ProgressChunk).atlas;
        assert!(
            (0..26).any(|c| two.get(2 * c, 16) != two.get(2 * c + 1, 16)),
            "the shading steps inside a design column"
        );
    }

    #[test]
    fn the_tracks_are_whole_design_pixels() {
        for p in [Part::TrackbarTrack, Part::TrackbarTrackV] {
            let one = part(1, p).atlas;
            let two = part(2, p).atlas;
            for y in 0..two.height as i32 {
                for x in 0..two.width as i32 {
                    assert_eq!(two.get(x, y), one.get(x / 2, y / 2), "{p:?} ({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn thumbs_have_distinct_states_clear_corners_and_soft_edges() {
        for (p, (w, h)) in [
            (Part::TrackbarThumb, (11, 19)),
            (Part::TrackbarThumbV, (23, 11)),
        ] {
            for d in [1, 2] {
                let img = part(d, p);
                let frames: Vec<Vec<u32>> = THUMB_STATES
                    .iter()
                    .map(|&s| {
                        let f = img.frame(s);
                        (f.y..f.bottom())
                            .flat_map(|y| (f.x..f.right()).map(move |x| (x, y)))
                            .map(|(x, y)| img.atlas.get(x, y))
                            .collect()
                    })
                    .collect();
                for i in 0..5 {
                    for j in i + 1..5 {
                        assert_ne!(frames[i], frames[j], "{p:?} at {d}: states {i} and {j}");
                    }
                }
                let f = img.frame(State::Normal);
                assert_eq!((f.width, f.height), (w * d, h * d), "{p:?} at {d}");
            }
            let one = part(1, p).atlas;
            let two = part(2, p).atlas;
            let x0 = if p == Part::TrackbarThumbV { 1 } else { 0 };
            assert!(
                one.get(x0, 0) >> 24 < 0x60,
                "{p:?}: a clear top left corner"
            );
            if p == Part::TrackbarThumbV {
                assert!(
                    (0..11).all(|y| one.get(0, y) >> 24 == 0),
                    "the upright frame's first column is clear"
                );
            }
            assert!(
                (0..36).any(|i| (0x10..0xF0).contains(&(two.get(2 * x0 + i % 6, i / 6) >> 24))),
                "{p:?}: a soft corner at 2"
            );
            let doubled = (0..two.height as i32)
                .all(|y| (0..two.width as i32).all(|x| two.get(x, y) == one.get(x / 2, y / 2)));
            assert!(!doubled, "{p:?}: redrawn at 2, not doubled");
        }
        let two = part(2, Part::TrackbarThumb).atlas;
        assert!(
            (26..34).any(|y| (0..10).any(|x| (0x20..0xE0).contains(&(two.get(x, y) >> 24)))),
            "a soft slope at 2"
        );
    }

    fn face(w: u32, h: u32, s: i32) -> Canvas {
        let mut c = Canvas::with_scale(w, h, s);
        c.fill_rect(Rect::new(0, 0, w as i32, h as i32), FACE);
        c
    }

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(x, y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn luma(p: u32) -> u32 {
        ((p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)) / 3
    }

    fn bar(fraction: f64, enabled: bool, h: i32, s: i32) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let mut c = face(220, 40, s);
        draw_progress(
            &mut c,
            &theme.paint(),
            Rect::new(10, 10, 200, h),
            fraction,
            enabled,
        );
        c
    }

    #[test]
    fn a_glass_progress_bar_is_a_grey_trough_in_a_rounded_frame() {
        let c = bar(0.0, true, 18, 1);
        pin(&c, 110, 10, 0xB0B0B0, "the frame's top");
        pin(&c, 110, 27, 0x898989, "the frame's bottom");
        pin(&c, 110, 11, 0xF3F3F3, "the trough's top line");
        pin(&c, 110, 19, 0xC7C7C7, "the trough's middle");
        assert!(luma(c.get(10, 19)) < 0xB0, "the frame's left side");
        assert!(luma(c.get(13, 19)) < 0xC9, "the trough's ends shade darker");
        assert!(
            luma(c.get(11, 19)) > luma(c.get(13, 19)),
            "its first column is lighter"
        );
        assert!(luma(c.get(208, 19)) > luma(c.get(206, 19)), "and its last");
        assert!(
            luma(c.get(110, 12)) > luma(c.get(110, 19)),
            "a pale gloss on top"
        );
        assert_ne!(c.get(10, 10), 0xFFB0_B0B0, "a rounded corner");
        assert_eq!(c.get(9, 19), FACE.argb(), "nothing left of the frame");
    }

    #[test]
    fn a_glass_progress_fill_is_one_green_piece_with_shaded_ends() {
        let c = bar(0.5, true, 18, 1);
        pin(&c, 11, 19, 0x27C841, "the fill's first column");
        pin(&c, 47, 19, 0x00CE31, "the fill's middle");
        let green = |p: u32| p >> 8 & 0xFF;
        assert!(
            green(c.get(17, 19)) < green(c.get(47, 19)),
            "deeper toward the ends"
        );
        assert!(luma(c.get(47, 11)) > 0xC0, "the fill's pale top row");
        assert!(
            luma(c.get(47, 26)) > luma(c.get(47, 19)),
            "its bottom rows lift"
        );
        pin(&c, 180, 19, 0xC7C7C7, "the trough past the fill");
        let greyed = bar(0.5, false, 18, 1);
        assert!(greyed == c, "a greyed bar draws as an enabled one");
    }

    #[test]
    fn a_16_px_glass_progress_bar_keeps_its_top_and_bottom_rows() {
        let tall = bar(0.5, true, 18, 1);
        let c = bar(0.5, true, 16, 1);
        assert_eq!(c.get(47, 11), tall.get(47, 11), "the fill's pale top row");
        assert_eq!(c.get(47, 24), tall.get(47, 26), "the fill's last row");
        pin(&c, 47, 25, 0x898989, "the frame's bottom");
        pin(&c, 180, 18, 0xC7C7C7, "the trough's middle");
        pin(&c, 180, 24, 0xE9E9E9, "the trough's last row");
    }

    #[test]
    fn a_narrow_glass_fill_cuts_its_left_end_short() {
        // 20 of 198 px, under the ends' 26 each: the right end's last 20
        // columns, the left end's none.
        let c = bar(10.0 / 99.0, true, 18, 1);
        pin(&c, 30, 19, 0x27C841, "the right end's last column");
        pin(&c, 31, 19, 0xC7C7C7, "the trough after it");
    }

    #[test]
    fn a_glass_progress_bar_on_a_scale_2_canvas_doubles_every_pixel() {
        for (f, enabled) in [(0.0, true), (0.5, true), (0.5, false), (1.0, true)] {
            assert!(
                doubles(&bar(f, enabled, 18, 1), &bar(f, enabled, 18, 2)),
                "{f} {enabled}: every pixel doubles"
            );
        }
    }

    fn slider(enabled: bool, focused: bool, ptr: Interaction, s: i32) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let lay = trackbar::layout(&TrackBar::new(5, 11), 10, 10, 184);
        assert_eq!(lay.thumb, Rect::new(97, 10, 11, 21));
        let mut c = face(210, 40, s);
        draw_trackbar(&mut c, &theme.paint(), &lay, enabled, focused, ptr);
        c
    }

    #[test]
    fn a_glass_trackbar_has_a_four_row_track_and_grey_ticks() {
        let c = slider(true, false, Interaction::Idle, 1);
        pin(&c, 10, 17, 0x989999, "the track's top left corner");
        pin(&c, 50, 17, 0xADAEAE, "the track's top line");
        pin(&c, 50, 18, 0xE6E9E9, "the track's second row");
        pin(&c, 50, 20, 0xFCFBFC, "the track's last row");
        pin(&c, 193, 18, 0xFDFCFD, "the track's right end");
        assert_eq!(c.get(50, 16), FACE.argb(), "nothing above the track");
        assert_eq!(c.get(50, 21), FACE.argb(), "nothing below the track");
        for y in 31..35 {
            pin(&c, 15, y, 0xC1C2C2, "the first tick is 4 px");
            pin(&c, 188, y, 0xC1C2C2, "the last tick is 4 px");
        }
        for y in 31..34 {
            pin(&c, 32, y, 0xC1C2C2, "a middle tick is 3 px");
        }
        assert_eq!(c.get(32, 34), FACE.argb(), "a middle tick ends there");
    }

    #[test]
    fn a_glass_thumb_points_down_and_lights_blue() {
        let c = slider(true, false, Interaction::Idle, 1);
        pin(&c, 99, 11, 0x6D6D6D, "the thumb's outline, one row down");
        pin(&c, 100, 13, 0xF2F1F2, "the face's top");
        pin(&c, 97, 16, 0x6D6D6D, "the thumb's left side");
        assert!(
            luma(c.get(98, 12)) > 0xC0,
            "a light line inside the outline"
        );
        assert!(
            luma(c.get(100, 21)) < luma(c.get(100, 13)),
            "the lower face is darker"
        );
        assert_eq!(c.get(99, 10), FACE.argb(), "nothing above the thumb");
        assert_eq!(
            c.get(97, 28),
            FACE.argb(),
            "the point leaves the corner bare"
        );
        let hot = slider(true, false, Interaction::Hot, 1);
        pin(&hot, 97, 16, 0x3B77AD, "hot: a blue outline");
        let blue = |p: u32| (p & 0xFF) > (p >> 16 & 0xFF) + 0x20;
        assert!(blue(hot.get(100, 21)), "hot: a pale blue face");
        let held = slider(true, false, Interaction::Pressed, 1);
        pin(&held, 97, 16, 0x2B5B87, "held: a deeper outline");
        assert!(
            luma(held.get(100, 21)) < luma(hot.get(100, 21)),
            "held: a deeper face"
        );
        let focused = slider(true, true, Interaction::Idle, 1);
        let p = focused.get(98, 16);
        assert!(
            p >> 16 & 0xFF < 0x50 && p >> 8 & 0xFF > 0xD0,
            "focused: a cyan line {p:08X}"
        );
        assert_eq!(
            focused.get(100, 21),
            hot.get(100, 21),
            "focused: the pale blue face"
        );
        let lit = slider(true, true, Interaction::Hot, 1);
        assert!(lit == hot, "the pointer outranks the focus");
        let greyed = slider(false, true, Interaction::Hot, 1);
        pin(&greyed, 97, 16, 0xABAFB3, "greyed: a pale outline");
        pin(&greyed, 100, 16, 0xF3F4F4, "greyed: a flat face");
        pin(&greyed, 50, 18, 0xE6E9E9, "greyed: the same track");
    }

    #[test]
    fn a_focused_glass_thumb_in_an_inactive_window_draws_at_rest() {
        let theme = theme_for(Interface::Glass);
        let lay = trackbar::layout(&TrackBar::new(5, 11), 10, 10, 184);
        let mut c = face(210, 40, 1);
        let p = Paint {
            active: false,
            ..theme.paint()
        };
        draw_trackbar(&mut c, &p, &lay, true, true, Interaction::Idle);
        assert!(
            c == slider(true, false, Interaction::Idle, 1),
            "same as unfocused"
        );
    }

    #[test]
    fn a_glass_trackbar_on_a_scale_2_canvas_doubles_every_pixel() {
        for (enabled, focused, ptr) in [
            (true, false, Interaction::Idle),
            (true, false, Interaction::Pressed),
            (true, true, Interaction::Idle),
            (false, false, Interaction::Idle),
        ] {
            assert!(
                doubles(
                    &slider(enabled, focused, ptr, 1),
                    &slider(enabled, focused, ptr, 2)
                ),
                "{enabled} {focused} {ptr:?}: every pixel doubles"
            );
        }
    }

    fn thumbs(i: Interface, enabled: bool, focused: bool, ptr: Interaction) -> (Canvas, Canvas) {
        let lay = trackbar::layout(&TrackBar::new(5, 11), 10, 10, 184);
        let theme = theme_for(i);
        let mut one = Canvas::new(210, 40);
        one.fill_rect(one.bounds(), FACE);
        draw_trackbar(&mut one, &theme.paint(), &lay, enabled, focused, ptr);
        let mut two = Canvas::new(420, 80);
        two.fill_rect(two.bounds(), FACE);
        draw_trackbar(
            &mut two,
            &theme.scaled(2).paint(),
            &lay,
            enabled,
            focused,
            ptr,
        );
        (one, two)
    }

    #[test]
    fn a_glass_thumb_at_paint_scale_2_is_redrawn_in_place() {
        for (enabled, focused, ptr) in [
            (true, false, Interaction::Idle),
            (true, false, Interaction::Hot),
            (true, true, Interaction::Idle),
            (false, false, Interaction::Idle),
        ] {
            let (one, two) = thumbs(Interface::Glass, enabled, focused, ptr);
            for x in 190..220 {
                assert_eq!(two.get(x, 21), FACE.argb(), "nothing above the thumb");
                assert_eq!(one.get(x / 2, 10), FACE.argb(), "nothing above at 1");
            }
            pin(
                &two,
                200,
                22,
                one.get(100, 11) & 0xFF_FFFF,
                "the outline's top",
            );
            let differs =
                (20..62).any(|y| (194..216).any(|x| two.get(x, y) != one.get(x / 2, y / 2)));
            assert!(differs, "{enabled} {focused} {ptr:?}: redrawn, not doubled");
        }
    }

    fn upright(enabled: bool, focused: bool, ptr: Interaction, s: i32) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let lay = upright_layout();
        let mut c = face(50, 100, s);
        draw_vertical_trackbar(&mut c, &theme.paint(), &lay, enabled, focused, ptr);
        c
    }

    fn upright_layout() -> trackbar::Layout {
        let lay = trackbar::layout_vertical(&TrackBar::new(10, 21).paged(4, 2), 10, 10, 81);
        assert_eq!(lay.thumb, Rect::new(15, 45, 21, 11));
        assert_eq!(lay.channel, Rect::new(23, 10, 4, 81));
        lay
    }

    const UPRIGHT: [(bool, bool, Interaction); 5] = [
        (true, false, Interaction::Idle),
        (true, false, Interaction::Hot),
        (true, false, Interaction::Pressed),
        (true, true, Interaction::Idle),
        (false, false, Interaction::Idle),
    ];

    #[test]
    fn a_vertical_glass_track_is_four_columns_with_grey_ticks_each_side() {
        let c = upright(true, false, Interaction::Idle, 1);
        pin(&c, 24, 20, 0xADAEAE, "the track's left line");
        pin(&c, 25, 20, 0xE6E9E9, "the track's second column");
        pin(&c, 26, 20, 0xE6E9E9, "the track's third column");
        pin(&c, 27, 20, 0xFDFCFD, "the track's right column");
        pin(&c, 24, 10, 0x989999, "the track's top left corner");
        pin(&c, 25, 10, 0xADAEAE, "the track's top end");
        pin(&c, 27, 10, 0xE9E9E9, "the track's top right corner");
        pin(&c, 24, 90, 0xE9E9E9, "the track's bottom left corner");
        pin(&c, 25, 90, 0xFCFBFC, "the track's bottom end");
        pin(&c, 27, 90, 0xFDFCFD, "the track's bottom right corner");
        assert_eq!(c.get(23, 20), FACE.argb(), "nothing left of the track");
        assert_eq!(c.get(28, 20), FACE.argb(), "nothing right of the track");
        assert_eq!(c.get(25, 91), FACE.argb(), "nothing under the track");
        for y in [15, 85] {
            for x in (10..14).chain(37..41) {
                pin(&c, x, y, 0xC1C2C2, "an end tick is 4 px");
            }
        }
        for x in (11..14).chain(37..40) {
            pin(&c, x, 78, 0xC1C2C2, "a middle tick is 3 px");
        }
        assert_eq!(c.get(10, 78), FACE.argb(), "a middle tick starts there");
        assert_eq!(c.get(40, 78), FACE.argb(), "a middle tick ends there");
        assert_eq!(c.get(12, 81), FACE.argb(), "no tick at an odd position");
        let greyed = upright(false, false, Interaction::Idle, 1);
        pin(&greyed, 25, 20, 0xE6E9E9, "greyed: the same track");
        pin(&greyed, 11, 78, 0xC1C2C2, "greyed: the same ticks");
    }

    #[test]
    fn a_vertical_glass_thumb_is_a_rounded_bar_that_lights_blue() {
        let c = upright(true, false, Interaction::Idle, 1);
        pin(
            &c,
            17,
            45,
            0x6D6D6D,
            "the top outline, in the rect's top row",
        );
        pin(
            &c,
            15,
            47,
            0x6D6D6D,
            "the left outline, in its first column",
        );
        pin(
            &c,
            35,
            47,
            0x6D6D6D,
            "the right outline, in its last column",
        );
        pin(&c, 17, 54, 0x6D6D6D, "the bottom outline, in its tenth row");
        pin(&c, 17, 46, 0xFCFCFC, "a light row under the outline");
        pin(&c, 17, 47, 0xF2F1F2, "the face's top row");
        pin(&c, 25, 52, 0xD2D3D3, "the face's darkest row");
        pin(&c, 25, 53, 0xF4F4F4, "a light row over the bottom outline");
        assert!(
            luma(c.get(25, 50)) < luma(c.get(25, 48)),
            "the face darker below its middle"
        );
        let shade = luma(c.get(36, 50));
        assert!(
            (0xA0..0xE0).contains(&shade),
            "the shadow column over the face"
        );
        assert!(luma(c.get(20, 55)) < 0xF0, "the shadow row over the face");
        assert_eq!(c.get(14, 50), FACE.argb(), "nothing left of the rect");
        assert_eq!(
            c.get(15, 55),
            FACE.argb(),
            "the shadow row starts a column in"
        );
        assert_eq!(c.get(37, 49), FACE.argb(), "nothing right of the shadow");
        pin(
            &c,
            37,
            50,
            0xC1C2C2,
            "the thumb's tick, right against its shadow",
        );
        assert_eq!(c.get(25, 44), 0xFFE6_E9E9, "the track above the thumb");
        let hot = upright(true, false, Interaction::Hot, 1);
        pin(&hot, 17, 45, 0x3B77AD, "hot: a blue outline");
        pin(&hot, 25, 47, 0xE8F5FD, "hot: a pale blue face");
        let held = upright(true, false, Interaction::Pressed, 1);
        pin(&held, 17, 45, 0x2B5B87, "held: a deeper outline");
        pin(&held, 25, 52, 0x4C99CB, "held: its darkest row");
        let focused = upright(true, true, Interaction::Idle, 1);
        pin(&focused, 17, 46, 0x3CEDFD, "focused: a cyan ring's top");
        assert_eq!(
            focused.get(25, 50),
            hot.get(25, 50),
            "focused: the pale blue face"
        );
        let lit = upright(true, true, Interaction::Hot, 1);
        assert!(lit == hot, "the pointer outranks the focus");
        let greyed = upright(false, true, Interaction::Hot, 1);
        pin(&greyed, 17, 45, 0xABAFB3, "greyed: a pale outline");
        pin(&greyed, 17, 46, 0xFCFCFC, "greyed: a white ring");
        pin(&greyed, 25, 50, 0xF3F4F4, "greyed: a flat face");
    }

    #[test]
    fn a_focused_vertical_glass_thumb_in_an_inactive_window_draws_at_rest() {
        let theme = theme_for(Interface::Glass);
        let mut c = face(50, 100, 1);
        let p = Paint {
            active: false,
            ..theme.paint()
        };
        draw_vertical_trackbar(&mut c, &p, &upright_layout(), true, true, Interaction::Idle);
        assert!(
            c == upright(true, false, Interaction::Idle, 1),
            "same as unfocused"
        );
    }

    #[test]
    fn a_vertical_glass_trackbar_at_scale_2_is_redrawn_in_place() {
        let theme = theme_for(Interface::Glass);
        let lay = upright_layout();
        for (enabled, focused, ptr) in UPRIGHT {
            let one = upright(enabled, focused, ptr, 1);
            assert!(
                doubles(&one, &upright(enabled, focused, ptr, 2)),
                "{enabled} {focused} {ptr:?}: a scale-2 canvas doubles every pixel"
            );
            let mut two = face(100, 200, 1);
            draw_vertical_trackbar(
                &mut two,
                &theme.scaled(2).paint(),
                &lay,
                enabled,
                focused,
                ptr,
            );
            pin(
                &two,
                34,
                90,
                one.get(17, 45) & 0xFF_FFFF,
                "the outline's top",
            );
            assert_eq!(two.get(28, 100), FACE.argb(), "nothing left of the rect");
            let differs =
                (88..112).any(|y| (28..76).any(|x| two.get(x, y) != one.get(x / 2, y / 2)));
            assert!(differs, "{enabled} {focused} {ptr:?}: redrawn, not doubled");
        }
    }

    #[test]
    fn a_vertical_trackbar_in_a_look_without_its_parts_draws_classic() {
        let lay = upright_layout();
        let theme = theme_for(Interface::Classic);
        for (enabled, focused, ptr) in UPRIGHT {
            let mut a = face(50, 100, 1);
            draw_vertical_trackbar(&mut a, &theme.paint(), &lay, enabled, focused, ptr);
            let mut b = face(50, 100, 1);
            let held = ptr == Interaction::Pressed;
            crate::classic::trackbar::draw_held(&mut b, &theme.paint(), &lay, enabled, held);
            assert!(a == b, "{enabled} {focused} {ptr:?}: Classic's");
        }
    }
}
