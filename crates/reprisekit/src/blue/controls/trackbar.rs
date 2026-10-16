//! The trackbar's parts, drawn from shapes at the style's scale in the
//! scheme's colours ([`Scheme`](crate::blue::scheme::Scheme)'s
//! `trackbar_*` faces).
//!
//! [`Part::TrackbarTrack`] is a 5x4 sunken channel: a 1 px ring, dark
//! along the top and down the left, light along the bottom and up the
//! right, round two pale rows, its corners rounded over 1 px. Each end
//! keeps two columns; the middle column stretches along the track.
//! [`Part::TrackbarTrackV`] is the same channel stood upright, 4x5. A
//! greyed trackbar keeps its channel: Disabled repeats Normal.
//!
//! [`Part::TrackbarThumb`] is an 11x21 pointer in Normal, Hot, Pressed
//! and Disabled: a 1 px outline (lit on the top and left, shaded down
//! the right), a three-row cap under the top edge, a pale body shading
//! toward the right, and a point under it narrowing at 45 degrees to the
//! tip, lined inside by a 2 px band in the cap's colour.
//! [`Part::TrackbarThumbV`] is a 22x11 box thumb: a 1 px outline round a
//! pale body shading down, with a 2 px cap down each end. It stretches to
//! fill a larger rect. No greyed box thumb is drawn anywhere, so its
//! Disabled repeats Normal.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Insets};
use crate::shape::{CLEAR, Px, Stops, centre, draw, mix, polygon, premul, ringed, shares};
use crate::style::{Layout, Part, PartImage, State};

/// The channel's frame: two columns of end either side of one column that
/// stretches.
const TRACK_W: i32 = 5;
const TRACK_H: i32 = 4;
const TRACK_END: i32 = 2;
const TRACK_R: f64 = 1.0;

const BOX_W: i32 = 22;
const BOX_H: i32 = 11;
const BOX_R: f64 = 1.5;
/// Where the box thumb's caps end: columns 1 and 2 on the left, 19 and
/// 20 on the right.
const BOX_CAP_IN: f64 = 3.0;
/// A box thumb fills its rect: the columns between the caps and the row
/// across the pale body (row 4) stretch.
const BOX_SIZING: Insets = Insets::new(3, 4, 3, 6);

const THUMB_W: i32 = 11;
const THUMB_H: i32 = 21;
const THUMB_R: f64 = 2.0;
const THUMB_SIDE: f64 = 15.5;
const THUMB_CAP: f64 = 4.0;
const THUMB_BAND: f64 = 2.0;

const TRACK_STATES: [State; 2] = [State::Normal, State::Disabled];
const THUMB_STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let track = [&s.trackbar_track.normal, &s.trackbar_track.disabled];
    let track = PartImage::new(
        stack(track.map(|f| channel(f.down, false, d))),
        &TRACK_STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(TRACK_END, 0, TRACK_END, 0))
    .at_scale(d);
    let track_v = [&s.trackbar_track_v.normal, &s.trackbar_track_v.disabled];
    let track_v = PartImage::new(
        stack(track_v.map(|f| channel(f.across, true, d))),
        &TRACK_STATES,
        Layout::Vertical,
    )
    .sizing(Insets::new(0, TRACK_END, 0, TRACK_END))
    .at_scale(d);

    let t = &s.trackbar_thumb;
    let mut thumb = PartImage::new(
        stack([&t.normal, &t.hot, &t.pressed, &t.disabled].map(|f| pointer(f, d))),
        &THUMB_STATES,
        Layout::Vertical,
    )
    .at_scale(d);
    thumb.true_size = true;

    let b = &s.trackbar_thumb_v;
    let thumb_v = PartImage::new(
        stack([&b.normal, &b.hot, &b.pressed, &b.normal].map(|f| box_thumb(f, d))),
        &THUMB_STATES,
        Layout::Vertical,
    )
    .sizing(BOX_SIZING)
    .at_scale(d);

    vec![
        (Part::TrackbarTrack, track),
        (Part::TrackbarThumb, thumb),
        (Part::TrackbarTrackV, track_v),
        (Part::TrackbarThumbV, thumb_v),
    ]
}

fn stack<const N: usize>(frames: [Canvas; N]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * N as u32);
    for (i, f) in frames.iter().enumerate() {
        let top = i as i32 * h as i32;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                a.put_raw(x, top + y, f.get(x, y));
            }
        }
    }
    a
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

fn at(g: Grad, t: f64) -> Px {
    Stops(&stops(g)).at(t)
}

/// The channel at `d`, its colours across its width from `profile` (the
/// face's gradient from the dark edge to the light one): lying flat, or
/// stood on end when `upright`.
fn channel(profile: Grad, upright: bool, d: i32) -> Canvas {
    let (len, wide) = (f64::from(TRACK_W), f64::from(TRACK_H));
    let shadow = at(profile, 0.5 / wide);
    let light = at(profile, (wide - 0.5) / wide);
    let prof = stops(profile);
    let end = f64::from(TRACK_END);
    // `l` along the channel and `c` across it, design-pixel positions
    // from its start. The ring turns from dark to light round each end;
    // the middle column's colours depend on `c` alone, so it stretches
    // evenly.
    let paint = |l: f64, c: f64, ring: usize| {
        if ring > 0 {
            return Stops(&prof).at(c / wide);
        }
        let along = if l < end {
            (l - end) / end
        } else if l > len - end {
            (l - (len - end)) / end
        } else {
            0.0
        };
        let lit = (0.5 + along + (c - wide / 2.0) / (wide / 2.0)).clamp(0.0, 1.0);
        mix(shadow, light, lit)
    };
    let (w, h) = if upright {
        (TRACK_H, TRACK_W)
    } else {
        (TRACK_W, TRACK_H)
    };
    let px = ringed(w, h, TRACK_R, 1, d, |x, y, ring| {
        let (x, y) = (x + 0.5, y + 0.5);
        if upright {
            paint(y, x, ring)
        } else {
            paint(x, y, ring)
        }
    });
    draw(w, h, d, |x, y| px[(y as i32 * w * d + x as i32) as usize])
}

fn arc(cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> impl Iterator<Item = (f64, f64)> {
    const STEPS: i32 = 8;
    (0..=STEPS).map(move |i| {
        let a = (a0 + (a1 - a0) * f64::from(i) / f64::from(STEPS)).to_radians();
        (cx + r * a.cos(), cy + r * a.sin())
    })
}

fn pointer_path(inset: f64, side: f64, k: f64) -> Vec<(f64, f64)> {
    let (w, r) = (f64::from(THUMB_W), THUMB_R);
    let ri = (r - inset).max(0.0);
    let mid = w / 2.0;
    // Both point edges run at 45 degrees from the sides' feet to the tip.
    let tip = side + (mid - inset);
    let mut pts: Vec<(f64, f64)> = arc(r, r, ri, 180.0, 270.0).collect();
    pts.extend(arc(w - r, r, ri, 270.0, 360.0));
    pts.extend([(w - inset, side), (mid, tip), (inset, side)]);
    pts.into_iter().map(|(x, y)| (x * k, y * k)).collect()
}

fn pointer(f: &Face, d: i32) -> Canvas {
    let k = f64::from(d);
    let (w, h) = (f64::from(THUMB_W), f64::from(THUMB_H));
    let outer = pointer_path(0.0, THUMB_SIDE, k);
    let inner = pointer_path(1.0, THUMB_SIDE, k);
    // The band's inner edge: the inner outline with its sides cut short,
    // so its point edges run `THUMB_BAND` further in.
    let band = pointer_path(1.0, THUMB_SIDE - THUMB_BAND, k);
    let rim = premul(f.rim, 255);
    let dark = f.across.last().map_or(rim, |&(_, c)| premul(c, 255));
    let cap = [
        (1.5, at(f.down, 1.5 / h)),
        (THUMB_CAP - 0.5, at(f.down, (THUMB_CAP - 0.5) / h)),
    ];
    let body = stops(f.across);
    let mark = f.mark.map(|c| premul(c, 255));
    draw(THUMB_W, THUMB_H, d, |x, y| {
        let c0 = polygon(x, y, &outer);
        if c0 == 0.0 {
            return CLEAR;
        }
        let (c1, c2) = (polygon(x, y, &inner), polygon(x, y, &band));
        let (px, py) = (centre(x as i32, d) + 0.5, centre(y as i32, d) + 0.5);
        // Lit along the top and down the left, shading toward the right
        // below the top corners.
        let lit = (px / w).clamp(0.0, 1.0) * (py / 2.0).clamp(0.0, 1.0);
        let line = mix(rim, dark, lit);
        let face = Stops(&body).at(px / w);
        let fill = if py < THUMB_CAP {
            Stops(&cap).at(py)
        } else {
            face
        };
        let edge = match mark {
            Some(m) => mix(cap[1].1, m, px / w),
            None => mix(line, face, 0.25),
        };
        shares(&[(line, c0 - c1), (edge, c1 - c2), (fill, c2)])
    })
}

fn box_thumb(f: &Face, d: i32) -> Canvas {
    let (w, h) = (f64::from(BOX_W), f64::from(BOX_H));
    let rim = premul(f.rim, 255);
    let dark = f.down.last().map_or(rim, |&(_, c)| premul(c, 255));
    let body = stops(f.down);
    let cap = |a: f64, b: f64| [(a, at(f.across, a / w)), (b, at(f.across, b / w))];
    let (left, right) = (cap(1.5, 2.5), cap(w - 2.5, w - 1.5));
    let px = ringed(BOX_W, BOX_H, BOX_R, 1, d, |x, y, ring| {
        let (x, y) = (x + 0.5, y + 0.5);
        if ring == 0 {
            // Lit on the edges nearer the top left, shaded on those
            // nearer the bottom right, half and half where they meet.
            let lit = (0.5 + (x.min(y) - (w - x).min(h - y)) / 2.0).clamp(0.0, 1.0);
            return mix(rim, dark, lit);
        }
        if x < BOX_CAP_IN {
            Stops(&left).at(x)
        } else if x > w - BOX_CAP_IN {
            Stops(&right).at(x)
        } else {
            Stops(&body).at(y / h)
        }
    });
    draw(BOX_W, BOX_H, d, |x, y| {
        px[(y as i32 * BOX_W * d + x as i32) as usize]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(v: Variant, d: i32, part: Part) -> PartImage {
        parts(v, d)
            .into_iter()
            .find(|(p, _)| *p == part)
            .map(|(_, img)| img)
            .expect("the trackbar builds the part")
    }

    fn frame(img: &PartImage, i: usize) -> Canvas {
        let n = img.states.len() as u32;
        let (w, h) = (img.atlas.width, img.atlas.height / n);
        let mut c = Canvas::new(w, h);
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                c.put_raw(x, y, img.atlas.get(x, i as i32 * h as i32 + y));
            }
        }
        c
    }

    fn alpha(px: u32) -> u32 {
        px >> 24
    }

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        words.fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    const ALL: [Part; 4] = [
        Part::TrackbarTrack,
        Part::TrackbarThumb,
        Part::TrackbarTrackV,
        Part::TrackbarThumbV,
    ];

    fn size(p: Part) -> (i32, i32) {
        match p {
            Part::TrackbarTrack => (TRACK_W, TRACK_H),
            Part::TrackbarTrackV => (TRACK_H, TRACK_W),
            Part::TrackbarThumb => (THUMB_W, THUMB_H),
            _ => (BOX_W, BOX_H),
        }
    }

    /// At 2 every frame is twice its design size, painted for 2, with
    /// the same design-pixel border, size and states as at 1.
    #[test]
    fn every_part_is_its_design_size_times_the_scale() {
        for v in Variant::ALL {
            for p in ALL {
                let (one, two) = (part(v, 1, p), part(v, 2, p));
                let (w, h) = size(p);
                let n = one.states.len() as u32;
                assert_eq!(
                    (one.atlas.width, one.atlas.height),
                    (w as u32, h as u32 * n)
                );
                assert_eq!(
                    (two.atlas.width, two.atlas.height),
                    (2 * w as u32, 2 * h as u32 * n),
                    "{v:?} {p:?} at 2"
                );
                assert_eq!((one.scale, two.scale), (1, 2), "{v:?} {p:?}");
                assert_eq!(one.sizing, two.sizing, "{v:?} {p:?} border in design px");
                assert_eq!(one.states, two.states);
                assert_eq!(one.true_size, two.true_size);
            }
        }
    }

    /// The thumbs differ in every state but the box thumb's greyed one,
    /// which is the one at rest; the channel's two frames are one.
    #[test]
    fn the_thumbs_light_hold_and_grey() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let t = part(v, d, Part::TrackbarThumb);
                let f: Vec<_> = (0..4).map(|i| frame(&t, i)).collect();
                for i in 0..4 {
                    for j in i + 1..4 {
                        assert!(f[i] != f[j], "{v:?} at {d}: thumb frames {i} and {j}");
                    }
                }
                let b = part(v, d, Part::TrackbarThumbV);
                let f: Vec<_> = (0..4).map(|i| frame(&b, i)).collect();
                assert!(f[0] != f[1] && f[0] != f[2] && f[1] != f[2], "{v:?} box");
                assert!(f[3] == f[0], "{v:?}'s greyed box thumb is the one at rest");
                for p in [Part::TrackbarTrack, Part::TrackbarTrackV] {
                    let img = part(v, d, p);
                    assert!(frame(&img, 0) == frame(&img, 1), "{v:?} {p:?}");
                }
            }
        }
    }

    /// The pointer's top corners are all but clear, its sides opaque, and
    /// its point narrows to the tip on row 20 with nothing beside it.
    #[test]
    fn the_pointer_has_round_top_corners_and_a_point() {
        for v in Variant::ALL {
            let c = frame(&part(v, 1, Part::TrackbarThumb), 0);
            assert!(alpha(c.get(0, 0)) < 96, "{v:?} top left corner");
            assert!(alpha(c.get(10, 0)) < 96, "{v:?} top right corner");
            for (x, y) in [(5, 0), (0, 8), (10, 8), (5, 10), (0, 14), (10, 14)] {
                assert_eq!(alpha(c.get(x, y)), 255, "{v:?} ({x}, {y}) is solid");
            }
            assert!(alpha(c.get(5, 20)) > 128, "{v:?} the tip");
            for x in [0, 1, 2, 3, 7, 8, 9, 10] {
                assert_eq!(alpha(c.get(x, 20)), 0, "{v:?} nothing beside the tip");
            }
            assert_eq!(alpha(c.get(0, 17)), 0, "{v:?} the point starts in");
        }
    }

    /// At 2 every curved or slanted edge has partial-alpha pixels: the
    /// pointer's corners and point, the box's and channel's corners.
    #[test]
    fn curved_edges_are_smooth_at_scale_2() {
        let partial = |px: u32| (1..255).contains(&alpha(px));
        for v in Variant::ALL {
            let t = frame(&part(v, 2, Part::TrackbarThumb), 0);
            assert!(partial(t.get(1, 1)), "{v:?} pointer corner");
            assert!(
                (32..42).any(|y| (0..11).any(|x| partial(t.get(x, y)))),
                "{v:?} point's left edge"
            );
            assert!(
                (32..42).any(|y| (11..22).any(|x| partial(t.get(x, y)))),
                "{v:?} point's right edge"
            );
            let b = frame(&part(v, 2, Part::TrackbarThumbV), 0);
            assert!(
                partial(b.get(0, 0)) || partial(b.get(1, 0)),
                "{v:?} box corner"
            );
            let ch = frame(&part(v, 2, Part::TrackbarTrack), 0);
            assert!(partial(ch.get(0, 0)), "{v:?} channel corner");
        }
    }

    /// The channel's stretched middle is one colour along its length at
    /// both scales, flat and upright, so a long channel has no seams.
    #[test]
    fn the_channels_middle_stretches_evenly() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let c = frame(&part(v, d, Part::TrackbarTrack), 0);
                for y in 0..TRACK_H * d {
                    let row: Vec<_> = (TRACK_END * d..(TRACK_END + 1) * d)
                        .map(|x| c.get(x, y))
                        .collect();
                    assert!(row.iter().all(|&p| p == row[0]), "{v:?} at {d}: row {y}");
                }
                let c = frame(&part(v, d, Part::TrackbarTrackV), 0);
                for x in 0..TRACK_H * d {
                    let col: Vec<_> = (TRACK_END * d..(TRACK_END + 1) * d)
                        .map(|y| c.get(x, y))
                        .collect();
                    assert!(col.iter().all(|&p| p == col[0]), "{v:?} at {d}: column {x}");
                }
                let mid = TRACK_END * d;
                assert!(
                    c.get(0, mid) != c.get(TRACK_H * d - 1, mid),
                    "{v:?}: the upright channel is dark on one side, light on the other"
                );
            }
        }
    }

    /// A partly covered edge pixel laid over white is never darker than
    /// the darkest colour the part's outline uses: the outline and the
    /// fill split edge pixels rather than stacking.
    #[test]
    fn edge_pixels_leave_no_dark_fringe() {
        let lum = |px: u32| {
            let a = alpha(px);
            let ch = |s: u32| ((px >> s) & 0xFF) + (255 - a);
            ch(16) + ch(8) + ch(0)
        };
        for v in Variant::ALL {
            for p in ALL {
                let img = part(v, 2, p);
                for i in 0..img.states.len() {
                    let c = frame(&img, i);
                    let darkest = c
                        .pixels
                        .iter()
                        .filter(|&&px| alpha(px) == 255)
                        .map(|&px| lum(px))
                        .min()
                        .unwrap();
                    for &px in c.pixels.iter().filter(|&&px| (1..255).contains(&alpha(px))) {
                        assert!(lum(px) + 3 >= darkest, "{v:?} {p:?} frame {i}: {px:08X}");
                    }
                }
            }
        }
    }

    #[test]
    fn trackbar_pins_at_scale_1_and_2() {
        let got: Vec<(Variant, Part, i32, u64)> = Variant::ALL
            .into_iter()
            .flat_map(|v| {
                ALL.into_iter().flat_map(move |p| {
                    [1, 2]
                        .into_iter()
                        .map(move |d| (v, p, d, fnv(&part(v, d, p).atlas)))
                })
            })
            .collect();
        #[rustfmt::skip]
        let want: [(Variant, Part, i32, u64); 80] = [
            (Variant::Blue, Part::TrackbarTrack, 1, 0x823B33891E42D044),
            (Variant::Blue, Part::TrackbarTrack, 2, 0xB95246B76E4B53BB),
            (Variant::Blue, Part::TrackbarThumb, 1, 0xC9C27E57F1ED0E8B),
            (Variant::Blue, Part::TrackbarThumb, 2, 0xEF9409D8F30975C8),
            (Variant::Blue, Part::TrackbarTrackV, 1, 0x384F5CA22015A0AB),
            (Variant::Blue, Part::TrackbarTrackV, 2, 0x78C5B18D0EE82899),
            (Variant::Blue, Part::TrackbarThumbV, 1, 0x575BE34B14D1A3DE),
            (Variant::Blue, Part::TrackbarThumbV, 2, 0x88A9DB312DF70E16),
            (Variant::Olive, Part::TrackbarTrack, 1, 0x823B33891E42D044),
            (Variant::Olive, Part::TrackbarTrack, 2, 0xB95246B76E4B53BB),
            (Variant::Olive, Part::TrackbarThumb, 1, 0xAEB34036DB65021F),
            (Variant::Olive, Part::TrackbarThumb, 2, 0xED8D377B85617356),
            (Variant::Olive, Part::TrackbarTrackV, 1, 0x384F5CA22015A0AB),
            (Variant::Olive, Part::TrackbarTrackV, 2, 0x78C5B18D0EE82899),
            (Variant::Olive, Part::TrackbarThumbV, 1, 0x162AD965F8F7A785),
            (Variant::Olive, Part::TrackbarThumbV, 2, 0xBC518A103DE988C8),
            (Variant::Silver, Part::TrackbarTrack, 1, 0x823B33891E42D044),
            (Variant::Silver, Part::TrackbarTrack, 2, 0xB95246B76E4B53BB),
            (Variant::Silver, Part::TrackbarThumb, 1, 0x18866176AC45CCA5),
            (Variant::Silver, Part::TrackbarThumb, 2, 0xEC4BC3E0508437BB),
            (Variant::Silver, Part::TrackbarTrackV, 1, 0x384F5CA22015A0AB),
            (Variant::Silver, Part::TrackbarTrackV, 2, 0x78C5B18D0EE82899),
            (Variant::Silver, Part::TrackbarThumbV, 1, 0xF1471A70F9C2FFDE),
            (Variant::Silver, Part::TrackbarThumbV, 2, 0x2E5618F1EAD7F996),
            (Variant::Slate, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Slate, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Slate, Part::TrackbarThumb, 1, 0x65AB7A7564CBF978),
            (Variant::Slate, Part::TrackbarThumb, 2, 0xCD1E5B5DF4EFBE04),
            (Variant::Slate, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Slate, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Slate, Part::TrackbarThumbV, 1, 0x8B126FEE4AED2BD7),
            (Variant::Slate, Part::TrackbarThumbV, 2, 0x453DF1151A343716),
            (Variant::Sapphire, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Sapphire, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Sapphire, Part::TrackbarThumb, 1, 0x65AB7A7564CBF978),
            (Variant::Sapphire, Part::TrackbarThumb, 2, 0xCD1E5B5DF4EFBE04),
            (Variant::Sapphire, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Sapphire, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Sapphire, Part::TrackbarThumbV, 1, 0x8B126FEE4AED2BD7),
            (Variant::Sapphire, Part::TrackbarThumbV, 2, 0x453DF1151A343716),
            (Variant::Obsidian, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Obsidian, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Obsidian, Part::TrackbarThumb, 1, 0x65AB7A7564CBF978),
            (Variant::Obsidian, Part::TrackbarThumb, 2, 0xCD1E5B5DF4EFBE04),
            (Variant::Obsidian, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Obsidian, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Obsidian, Part::TrackbarThumbV, 1, 0x8B126FEE4AED2BD7),
            (Variant::Obsidian, Part::TrackbarThumbV, 2, 0x453DF1151A343716),
            (Variant::Ember, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Ember, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Ember, Part::TrackbarThumb, 1, 0xB959529FC70F8D45),
            (Variant::Ember, Part::TrackbarThumb, 2, 0x0781C49FA82D041C),
            (Variant::Ember, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Ember, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Ember, Part::TrackbarThumbV, 1, 0x8587DF4745530A6E),
            (Variant::Ember, Part::TrackbarThumbV, 2, 0x74521CAC3D76A528),
            (Variant::Cobalt, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Cobalt, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Cobalt, Part::TrackbarThumb, 1, 0xF7205C67CB9D598D),
            (Variant::Cobalt, Part::TrackbarThumb, 2, 0x27E1C2B91F6B637E),
            (Variant::Cobalt, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Cobalt, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Cobalt, Part::TrackbarThumbV, 1, 0x4863539AB637CBB0),
            (Variant::Cobalt, Part::TrackbarThumbV, 2, 0xD74959AC3C050C3F),
            (Variant::Carbon, Part::TrackbarTrack, 1, 0x0C0799B268CC7F74),
            (Variant::Carbon, Part::TrackbarTrack, 2, 0x88FC38F5B07E4107),
            (Variant::Carbon, Part::TrackbarThumb, 1, 0x65AB7A7564CBF978),
            (Variant::Carbon, Part::TrackbarThumb, 2, 0xCD1E5B5DF4EFBE04),
            (Variant::Carbon, Part::TrackbarTrackV, 1, 0x58EBD49C9175BFAF),
            (Variant::Carbon, Part::TrackbarTrackV, 2, 0x20EF6F6E1EA262A1),
            (Variant::Carbon, Part::TrackbarThumbV, 1, 0x8B126FEE4AED2BD7),
            (Variant::Carbon, Part::TrackbarThumbV, 2, 0x453DF1151A343716),
            (Variant::Steel, Part::TrackbarTrack, 1, 0x823B33891E42D044),
            (Variant::Steel, Part::TrackbarTrack, 2, 0xB95246B76E4B53BB),
            (Variant::Steel, Part::TrackbarThumb, 1, 0x43F4A804C1BD50AE),
            (Variant::Steel, Part::TrackbarThumb, 2, 0xDDC985FAADF3832F),
            (Variant::Steel, Part::TrackbarTrackV, 1, 0x384F5CA22015A0AB),
            (Variant::Steel, Part::TrackbarTrackV, 2, 0x78C5B18D0EE82899),
            (Variant::Steel, Part::TrackbarThumbV, 1, 0xBA270921A5A061AD),
            (Variant::Steel, Part::TrackbarThumbV, 2, 0x4CDB9232D59E843E),
        ];
        assert_eq!(got.len(), want.len());
        for (w, g) in want.iter().zip(&got) {
            assert_eq!(w, g, "{:?} {:?} at {}", w.0, w.1, w.2);
        }
    }

    /// Under Blue an upright trackbar is 32 wide: the 22 px box thumb at
    /// x 5, its channel down the thumb's middle at x 14, and the ticks a
    /// pixel clear of the thumb either side (the left ones ending at x 3,
    /// the right ones from x 28). Classic's stays 31 wide with its channel
    /// at x 13.
    #[test]
    fn a_blue_upright_trackbar_lays_out_round_its_22_px_thumb() {
        use crate::trackbar::{TrackBar, layout_vertical, layout_vertical_with, vertical_ticks};
        let cm = crate::interfaces::blue_theme(Variant::Blue).look.controls();
        assert_eq!(cm.vtrack_thumb, 22);
        let tb = TrackBar::new(4, 11);
        let lay = layout_vertical_with(&tb, 10, 10, 81, cm.vtrack_thumb);
        assert_eq!(lay.bounds.width, 32, "the trackbar is 32 wide");
        assert_eq!(lay.thumb.x, 15, "the thumb starts 5 px in");
        assert_eq!(lay.thumb.width, 22, "the thumb is 22 wide");
        assert_eq!(lay.channel.x, 24, "the channel runs 9 px into the thumb");
        let ticks = vertical_ticks(&lay, 1, 3, 4);
        assert_eq!(
            ticks[0].right(),
            14,
            "the left end tick ends a pixel clear of the thumb"
        );
        assert_eq!(
            ticks[1].x, 38,
            "the right end tick starts a pixel clear of the thumb"
        );
        assert_eq!(
            ticks[1].right(),
            lay.bounds.right(),
            "the right end tick ends at the edge"
        );
        let classic = layout_vertical(&tb, 10, 10, 81);
        assert_eq!(
            (classic.bounds.width, classic.channel.x, classic.thumb.width),
            (31, 23, 21),
            "Classic's upright trackbar is as before"
        );
    }

    /// Every state of `v`'s box thumb drawn into a `w` x `h` rect, one
    /// under the other.
    fn box_in(v: Variant, w: i32, h: i32, d: i32) -> Canvas {
        let img = part(v, d, Part::TrackbarThumbV);
        let n = THUMB_STATES.len() as i32;
        let mut c = Canvas::new((w * d) as u32, (h * n * d) as u32);
        for (i, &s) in THUMB_STATES.iter().enumerate() {
            let r = crate::raster::Rect::new(0, i as i32 * h, w, h).scaled(d);
            img.blit(&mut c, img.frame(s), r, d);
        }
        c
    }

    #[test]
    fn the_box_thumb_at_its_own_size_is_its_frames() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let img = part(v, d, Part::TrackbarThumbV);
                assert!(
                    box_in(v, BOX_W, BOX_H, d).pixels == img.atlas.pixels,
                    "{v:?} at {d}"
                );
            }
        }
    }

    /// The volume popup's thumb is 26x15 under Blue: the box reaches each
    /// edge, its caps stay 2 px wide and its corners stay clear.
    #[test]
    fn the_box_thumb_fills_the_volume_popups_26_by_15_rect() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let c = box_in(v, 26, 15, d);
                let px = |x: i32, y: i32| c.get(x * d, y * d);
                for (x, y) in [(0, 7), (25, 7), (13, 0), (13, 14)] {
                    assert_eq!(alpha(px(x, y)), 255, "{v:?} at {d}: ({x}, {y})");
                }
                assert!(alpha(c.get(0, 0)) < 255, "{v:?} at {d}: a rounded corner");
                assert!(px(1, 7) != px(3, 7), "{v:?} at {d}: the left cap");
                assert!(px(24, 7) != px(22, 7), "{v:?} at {d}: the right cap");
                assert!(
                    px(3, 7) == px(22, 7),
                    "{v:?} at {d}: one body between the caps"
                );
            }
        }
    }

    #[test]
    fn the_box_thumb_in_the_volume_popups_rect_pins_at_scale_1_and_2() {
        let got: Vec<(Variant, [u64; 2])> = Variant::ALL
            .into_iter()
            .map(|v| (v, [1, 2].map(|d| fnv(&box_in(v, 26, 15, d)))))
            .collect();
        let want = [
            (
                Variant::Blue,
                [0x08C6_8AE9_AD30_B1A2, 0x5426_EF12_36C3_811E],
            ),
            (
                Variant::Olive,
                [0x8336_CFCF_3700_B8E9, 0x9BC5_06F8_A00A_5C50],
            ),
            (
                Variant::Silver,
                [0xB423_928C_D018_5A02, 0x31E5_57AD_426C_75FE],
            ),
            (
                Variant::Slate,
                [0x0BB6_C90C_4CD0_80DB, 0x16FA_1BBF_C372_DB9E],
            ),
            (
                Variant::Sapphire,
                [0x0BB6_C90C_4CD0_80DB, 0x16FA_1BBF_C372_DB9E],
            ),
            (
                Variant::Obsidian,
                [0x0BB6_C90C_4CD0_80DB, 0x16FA_1BBF_C372_DB9E],
            ),
            (
                Variant::Ember,
                [0x21D6_9E0A_F0A2_A742, 0xC2A0_C1E1_1419_7A70],
            ),
            (
                Variant::Cobalt,
                [0x53C4_C3CD_80E7_991C, 0x6E43_174D_AD4D_B9A7],
            ),
            (
                Variant::Carbon,
                [0x0BB6_C90C_4CD0_80DB, 0x16FA_1BBF_C372_DB9E],
            ),
            (
                Variant::Steel,
                [0x3224_CD55_42E3_7BE1, 0x4E42_659B_913D_52F6],
            ),
        ];
        assert_eq!(got, want, "{got:#x?}");
    }
}
