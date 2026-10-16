//! The explorer bar's Blue parts: the title strip with its closing line
//! ([`Part::ExplorerBarHeader`]) and the 20x17 close box in Normal, Hot and
//! Pressed ([`Part::ExplorerBarClose`]), drawn from the scheme at the draw's scale.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::metrics::BarMetrics;
use crate::raster::{Canvas, Insets};
use crate::shape::{self, CLEAR, Px, Stops, centre, premul};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const STRIP_W: i32 = 200;
const CLOSE_W: i32 = 20;
const CLOSE_H: i32 = 17;
const CLOSE_R: f64 = 3.5;
const X_TOP: f64 = 5.0;
const X_BOTTOM: f64 = 12.0;
const X_LEFT: f64 = 5.0;
const X_RIGHT: f64 = 14.0;
const X_ACROSS: f64 = 2.0;

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let head = PartImage::new(
        strip(&s.explorer_bar_header.normal, d),
        &[State::Normal],
        Layout::Vertical,
    )
    .sizing(Insets::new(0, 0, 1, 1))
    .outset(Insets::new(0, 0, 0, 1))
    .at_scale(d);
    let faces = &s.explorer_bar_close;
    let ink = faces.hot.mark.unwrap_or(faces.normal.rim);
    let atlas = close([None, Some(&faces.hot), Some(&faces.pressed)], ink, d);
    let close = PartImage::new(
        atlas,
        &[State::Normal, State::Hot, State::Pressed],
        Layout::Vertical,
    )
    .at_scale(d);
    vec![
        (Part::ExplorerBarHeader, head),
        (Part::ExplorerBarClose, close),
    ]
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), premul(c, 255)))
        .collect()
}

fn along(v: i32, d: i32, len: i32) -> f64 {
    (centre(v, d) + 0.5) / f64::from(len)
}

fn strip(face: &Face, d: i32) -> Canvas {
    let across = stops(face.across);
    let line = premul(face.rim, 255);
    shape::draw(STRIP_W, 2, d, |x, y| {
        if (y as i32) < d {
            Stops(&across).at(along(x as i32, d, STRIP_W))
        } else {
            line
        }
    })
}

fn close(faces: [Option<&Face>; 3], ink: Color, d: i32) -> Canvas {
    let k = f64::from(d);
    let bodies = faces.map(|f| f.map(|f| box_face(f, d)));
    let mark = premul(ink, 255);
    let h = CLOSE_H * d;
    shape::draw(CLOSE_W, CLOSE_H * 3, d, |x, y| {
        let i = (y as i32 / h) as usize;
        let fy = y - f64::from(i as i32 * h);
        let under = match &bodies[i] {
            Some(px) => px[(fy as i32 * CLOSE_W * d + x as i32) as usize],
            None => CLEAR,
        };
        let dx = if i == 2 { 1.0 } else { 0.0 };
        shape::over(shape::scale(mark, cross(x, fy, dx, k)), under)
    })
}

fn box_face(f: &Face, d: i32) -> Vec<Px> {
    let rim = premul(f.rim, 255);
    let down = stops(f.down);
    let across = stops(f.across);
    let mid = Stops(&across).at(0.5);
    let inside = |t: f64, len: i32| t.clamp(1.5 / f64::from(len), 1.0 - 1.5 / f64::from(len));
    shape::ringed(CLOSE_W, CLOSE_H, CLOSE_R, 1, d, |x, y, ring| {
        if ring == 0 {
            return rim;
        }
        let ty = inside((y + 0.5) / f64::from(CLOSE_H), CLOSE_H);
        let tx = inside((x + 0.5) / f64::from(CLOSE_W), CLOSE_W);
        let base = Stops(&down).at(ty);
        let off = Stops(&across).at(tx);
        (
            255.0,
            std::array::from_fn(|i| (base.1[i] + off.1[i] - mid.1[i]).clamp(0.0, 255.0)),
        )
    })
}

fn cross(x: f64, y: f64, dx: f64, k: f64) -> f64 {
    let (top, bottom) = (X_TOP * k, X_BOTTOM * k);
    let (l, r, w) = ((X_LEFT + dx) * k, (X_RIGHT + dx) * k, X_ACROSS * k);
    if x + 1.0 <= l || x >= r || y + 1.0 <= top || y >= bottom {
        return 0.0;
    }
    let hit = |px: f64, py: f64| {
        let run = py - top;
        let down = px - (l + run);
        let up = (r - run) - px;
        (0.0..w).contains(&down) || (0.0..w).contains(&up)
    };
    let mut n = 0;
    for j in 0..8 {
        let py = y + (f64::from(j) + 0.5) / 8.0;
        if !(top..bottom).contains(&py) {
            continue;
        }
        for i in 0..8 {
            if hit(x + (f64::from(i) + 0.5) / 8.0, py) {
                n += 1;
            }
        }
    }
    f64::from(n) / 64.0
}

pub(super) fn metrics() -> BarMetrics {
    BarMetrics {
        ring: 0,
        header: 22,
        line: 1,
        title_x: 6,
        title_dy: 1,
        close_w: CLOSE_W,
        close_h: CLOSE_H,
        close_right: 21,
        close_top: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::unpack;

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xCBF2_9CE4_8422_2325_u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        h
    }

    fn part(v: Variant, d: i32, which: Part) -> PartImage {
        parts(v, d)
            .into_iter()
            .find(|(p, _)| *p == which)
            .map(|(_, img)| img)
            .unwrap()
    }

    fn frame(v: Variant, d: i32, i: i32) -> Canvas {
        let img = part(v, d, Part::ExplorerBarClose);
        let mut c = Canvas::new((CLOSE_W * d) as u32, (CLOSE_H * d) as u32);
        for y in 0..CLOSE_H * d {
            for x in 0..CLOSE_W * d {
                c.put_raw(x, y, img.atlas.get(x, i * CLOSE_H * d + y));
            }
        }
        c
    }

    fn alpha(px: u32) -> u32 {
        px >> 24
    }

    fn sum(px: u32) -> u32 {
        (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF)
    }

    #[test]
    fn explorer_bar_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                for p in [Part::ExplorerBarHeader, Part::ExplorerBarClose] {
                    got.push(format!(
                        "{v:?} {d} {p:?} {:016x}",
                        fnv(&part(v, d, p).atlas)
                    ));
                }
            }
        }
        let want: &[&str] = &[
            "Blue 1 ExplorerBarHeader 0e2d35c43409cf2f",
            "Blue 1 ExplorerBarClose 4531eeed769fe977",
            "Blue 2 ExplorerBarHeader 494650a629567b8e",
            "Blue 2 ExplorerBarClose 3737b1abb86d38de",
            "Olive 1 ExplorerBarHeader 0e2d35c43409cf2f",
            "Olive 1 ExplorerBarClose 4531eeed769fe977",
            "Olive 2 ExplorerBarHeader 494650a629567b8e",
            "Olive 2 ExplorerBarClose 3737b1abb86d38de",
            "Silver 1 ExplorerBarHeader 617f82ad84e7a157",
            "Silver 1 ExplorerBarClose 4531eeed769fe977",
            "Silver 2 ExplorerBarHeader 7c771c8232a7c4ee",
            "Silver 2 ExplorerBarClose 3737b1abb86d38de",
            "Slate 1 ExplorerBarHeader 3a32fe5f3288e1df",
            "Slate 1 ExplorerBarClose 1342811b95f1625d",
            "Slate 2 ExplorerBarHeader a42d5320014e62de",
            "Slate 2 ExplorerBarClose 14e09813d9f706f4",
            "Sapphire 1 ExplorerBarHeader 172252632873fa77",
            "Sapphire 1 ExplorerBarClose 2339226a1ef2c876",
            "Sapphire 2 ExplorerBarHeader 3b9f903b86eaec7a",
            "Sapphire 2 ExplorerBarClose abc4af1845075270",
            "Obsidian 1 ExplorerBarHeader 172252632873fa77",
            "Obsidian 1 ExplorerBarClose 2339226a1ef2c876",
            "Obsidian 2 ExplorerBarHeader 3b9f903b86eaec7a",
            "Obsidian 2 ExplorerBarClose abc4af1845075270",
            "Ember 1 ExplorerBarHeader cbee38ff5de3c65b",
            "Ember 1 ExplorerBarClose d97663b55f19a0aa",
            "Ember 2 ExplorerBarHeader 5cefbf1b1ff040f6",
            "Ember 2 ExplorerBarClose c3816e3cf4dcbddf",
            "Cobalt 1 ExplorerBarHeader 172252632873fa77",
            "Cobalt 1 ExplorerBarClose 2339226a1ef2c876",
            "Cobalt 2 ExplorerBarHeader 3b9f903b86eaec7a",
            "Cobalt 2 ExplorerBarClose abc4af1845075270",
        ];
        assert_eq!(got, want, "pins:\n{}", got.join("\n"));
    }

    #[test]
    fn explorer_bar_parts_are_redrawn_at_scale_2() {
        for v in Variant::ALL {
            for p in [Part::ExplorerBarHeader, Part::ExplorerBarClose] {
                let (one, two) = (part(v, 1, p), part(v, 2, p));
                assert_eq!(two.atlas.width, 2 * one.atlas.width, "{v:?} {p:?}");
                assert_eq!(two.atlas.height, 2 * one.atlas.height, "{v:?} {p:?}");
                assert_eq!(two.sizing, one.sizing, "{v:?} {p:?} sizing");
                assert_eq!(two.outset, one.outset, "{v:?} {p:?} outset");
                assert_eq!(two.scale, 2, "{v:?} {p:?}");
            }
            let head = part(v, 1, Part::ExplorerBarHeader);
            assert_eq!((head.atlas.width, head.atlas.height), (200, 2));
            let close = part(v, 1, Part::ExplorerBarClose);
            assert_eq!((close.atlas.width, close.atlas.height), (20, 51));
        }
    }

    #[test]
    fn the_strip_shades_across_over_its_line() {
        for v in Variant::ALL {
            let face = &v.scheme().explorer_bar_header.normal;
            for d in [1, 2] {
                let a = part(v, d, Part::ExplorerBarHeader).atlas;
                for x in 0..200 * d {
                    for y in 0..d {
                        assert_eq!(a.get(x, y), a.get(x, 0), "{v:?} at {d}: column {x}");
                        assert_eq!(a.get(x, d + y), face.rim.argb(), "{v:?} at {d}: line");
                    }
                }
                assert!(
                    sum(a.get(0, 0)) > sum(a.get(200 * d - 1, 0)),
                    "{v:?} at {d}: light at the left"
                );
            }
        }
    }

    #[test]
    fn the_close_box_states_differ() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let [rest, hot, held] = [0, 1, 2].map(|i| frame(v, d, i));
                assert_ne!(rest.pixels, hot.pixels, "{v:?} at {d}");
                assert_ne!(hot.pixels, held.pixels, "{v:?} at {d}");
                assert_ne!(rest.pixels, held.pixels, "{v:?} at {d}");
                assert_eq!(alpha(rest.get(d, d)), 0, "{v:?} at {d}: no box at rest");
            }
        }
    }

    #[test]
    fn the_close_box_has_round_open_corners() {
        for v in Variant::ALL {
            let faces = &v.scheme().explorer_bar_close;
            for (i, face) in [(1, &faces.hot), (2, &faces.pressed)] {
                let one = frame(v, 1, i);
                assert!(alpha(one.get(0, 0)) < 8, "{v:?} #{i}: open corner");
                assert_eq!(one.get(4, 0), face.rim.argb(), "{v:?} #{i}: top rim");
                assert_eq!(one.get(4, 16), face.rim.argb(), "{v:?} #{i}: bottom rim");
                let two = frame(v, 2, i);
                let partial = (0..6)
                    .flat_map(|y| (0..6).map(move |x| (x, y)))
                    .filter(|&(x, y)| (1..255).contains(&alpha(two.get(x, y))))
                    .count();
                assert!(partial >= 3, "{v:?} #{i}: smooth corner at 2");
            }
        }
    }

    #[test]
    fn the_close_box_corners_have_no_dark_fringe() {
        for v in Variant::ALL {
            let faces = &v.scheme().explorer_bar_close;
            for (i, face) in [(1, &faces.hot), (2, &faces.pressed)] {
                let floor = [face.rim.r, face.rim.g, face.rim.b].map(f64::from);
                let c = frame(v, 2, i);
                for (x, y) in (0..8).flat_map(|y| (0..8).map(move |x| (x, y))) {
                    let (a, p) = unpack(c.get(x, y));
                    if a < 32.0 {
                        continue;
                    }
                    for ch in 0..3 {
                        assert!(
                            p[ch] * 255.0 / a >= floor[ch] - 4.0,
                            "{v:?} #{i} at {x},{y}: darker than the rim"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_x_is_solid_at_1_smooth_at_2_and_moves_when_held() {
        for v in Variant::ALL {
            let ink = v.scheme().explorer_bar_close.hot.mark.unwrap().argb();
            let rest = frame(v, 1, 0);
            assert_eq!(rest.get(9, 8), ink, "{v:?}: the arms cross");
            for y in 5..12 {
                let run = y - 5;
                for x in [6 + run, 12 - run] {
                    assert_eq!(rest.get(x, y), ink, "{v:?}: arm at {x},{y}");
                }
            }
            assert_eq!(alpha(rest.get(4, 5)), 0, "{v:?}: clear beside the X");
            let two = frame(v, 2, 0);
            assert!(
                two.pixels.iter().any(|&p| (1..255).contains(&alpha(p))),
                "{v:?}: smooth arms at 2"
            );
            let (hot, held) = (frame(v, 1, 1), frame(v, 1, 2));
            assert_eq!(hot.get(6, 5), ink, "{v:?}: the lit X starts on column 6");
            assert_ne!(held.get(6, 5), ink, "{v:?}: held X leaves column 6");
            assert_eq!(held.get(7, 5), ink, "{v:?}: held X a pixel right");
            assert_eq!(held.get(13, 5), ink, "{v:?}: held X a pixel right");
        }
    }
}
