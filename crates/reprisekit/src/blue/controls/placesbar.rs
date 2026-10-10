use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::metrics::PlacesMetrics;
use crate::raster::{Canvas, Fill, Insets};
use crate::shape::{centre, draw, mix, premul, smooth};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

// Design px.
const SIDE: i32 = 17;
const INSET: i32 = 4;
const GLINT: f64 = 0.25;
const GLINT_RUN: f64 = 3.0;
/// The shade's fade at its lit ends: none up to `END_A` px from the end, all
/// of it from `END_B` px on.
const END_A: f64 = 0.8;
const END_B: f64 = 4.2;
/// The shade's share at the bottom-right corner, rising to all of it over
/// `CORNER_RUN` px along either edge.
const CORNER_MIN: f64 = 0.6;
const CORNER_RUN: f64 = 3.0;

pub(super) fn metrics() -> PlacesMetrics {
    PlacesMetrics {
        tile: 71,
        icon_top: 5,
    }
}

pub(super) fn parts(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    vec![(
        Part::PlacesBar,
        PartImage::new(
            panel(&v.scheme().places_bar.normal, scale),
            &[State::Normal],
            Layout::Vertical,
        )
        .sizing(Insets::new(INSET, INSET, INSET, INSET))
        .fill(Fill::Tile)
        .at_scale(scale),
    )]
}

fn panel(face: &Face, d: i32) -> Canvas {
    let rim = premul(face.rim, 255);
    let white = premul(face.inner, 255);
    let fill = premul(grad_at(face.down, 0.5), 255);
    let shade = premul(face.down.last().map_or(face.rim, |s| s.1), 255);
    let far = f64::from(SIDE);
    draw(SIDE, SIDE, d, |x, y| {
        let u = centre(x as i32, d) + 0.5;
        let v = centre(y as i32, d) + 0.5;
        let (right, bottom) = (u > far - 1.0, v > far - 1.0);
        if right || bottom {
            let (t, s) = match (right, bottom) {
                (true, true) => (u.min(v), (far - u).min(far - v)),
                (true, false) => (v, far - v),
                _ => (u, far - u),
            };
            let k = smooth((t - END_A) / (END_B - END_A))
                * (CORNER_MIN + (1.0 - CORNER_MIN) * smooth(s / CORNER_RUN));
            return mix(rim, shade, k);
        }
        if u < 1.0 || v < 1.0 {
            let s = if v < 1.0 { u } else { v };
            return mix(rim, white, GLINT * (1.0 - smooth(s / GLINT_RUN)));
        }
        if u < 2.0 || v < 2.0 || u > far - 2.0 || v > far - 2.0 {
            return white;
        }
        fill
    })
}

fn grad_at(g: Grad, t: f32) -> Color {
    let Some(&(_, first)) = g.first() else {
        return Color::rgb(0xFF, 0xFF, 0xFF);
    };
    let mut prev = (0.0, first);
    for &(p, c) in g {
        if t <= p {
            let k = if p > prev.0 {
                (t - prev.0) / (p - prev.0)
            } else {
                1.0
            };
            let lerp =
                |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * k).round() as u8;
            return Color::rgb(
                lerp(prev.1.r, c.r),
                lerp(prev.1.g, c.g),
                lerp(prev.1.b, c.b),
            );
        }
        prev = (p, c);
    }
    prev.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::style_for;
    use crate::raster::Rect;

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        let mut eat = |b: &[u8]| {
            for &b in b {
                h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        eat(&c.width.to_le_bytes());
        eat(&c.height.to_le_bytes());
        for p in &c.pixels {
            eat(&p.to_le_bytes());
        }
        h
    }

    fn atlas(v: Variant, d: i32) -> Canvas {
        parts(v, d).remove(0).1.atlas
    }

    #[test]
    fn places_bar_pins_at_scale_1_and_2() {
        let got: Vec<(Variant, u64, u64)> = Variant::ALL
            .iter()
            .map(|&v| (v, fnv(&atlas(v, 1)), fnv(&atlas(v, 2))))
            .collect();
        let want: [(Variant, u64, u64); 7] = [
            (Variant::Blue, 3969084999834761493, 4799614750075183387),
            (Variant::Olive, 3969084999834761493, 4799614750075183387),
            (Variant::Silver, 3969084999834761493, 4799614750075183387),
            (Variant::Slate, 15724431930943416073, 10510917493418624306),
            (
                Variant::Sapphire,
                15724431930943416073,
                10510917493418624306,
            ),
            (
                Variant::Obsidian,
                15724431930943416073,
                10510917493418624306,
            ),
            (Variant::Ember, 15724431930943416073, 10510917493418624306),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn the_panel_is_a_lit_frame_round_a_white_line_and_a_fill() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let (_, img) = parts(v, d).remove(0);
                let a = &img.atlas;
                let n = SIDE * d;
                assert_eq!((a.width as i32, a.height as i32), (n, n), "{v:?} at {d}");
                assert_eq!(img.scale, d, "{v:?} at {d}");
                assert_eq!(img.sizing, Insets::new(4, 4, 4, 4), "{v:?} at {d}");
                let lum = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
                let mid = n / 2;
                assert!(
                    lum(a.get(n - 1, mid)) < lum(a.get(0, mid)),
                    "{v:?} at {d}: shade"
                );
                assert!(
                    lum(a.get(mid, n - 1)) < lum(a.get(mid, 0)),
                    "{v:?} at {d}: shade"
                );
                assert_eq!(a.get(d, mid), 0xFFFF_FFFF, "{v:?} at {d}: white line");
                assert_eq!(
                    a.get(mid, n - d - 1),
                    0xFFFF_FFFF,
                    "{v:?} at {d}: white line"
                );
                assert!(
                    lum(a.get(mid, mid)) < lum(0xFFFF_FFFF),
                    "{v:?} at {d}: fill"
                );
                for i in INSET * d..n - INSET * d {
                    for (x, y, x0, y0) in [
                        (n - 1, i, n - 1, mid),
                        (i, n - 1, mid, n - 1),
                        (0, i, 0, mid),
                        (i, 0, mid, 0),
                    ] {
                        assert_eq!(a.get(x, y), a.get(x0, y0), "{v:?} at {d}: edge at {x},{y}");
                    }
                }
                assert!(
                    lum(a.get(n - 1, 0)) > lum(a.get(n - 1, mid)),
                    "{v:?} at {d}"
                );
                assert!(
                    lum(a.get(n - 1, n - 1)) > lum(a.get(n - 1, mid)),
                    "{v:?} at {d}"
                );
            }
        }
    }

    #[test]
    fn warm_and_cool_panels_differ() {
        assert_ne!(
            atlas(Variant::Blue, 1).pixels,
            atlas(Variant::Slate, 1).pixels
        );
    }

    #[test]
    fn the_edges_end_in_the_corners_shades_at_any_size() {
        for v in [Variant::Blue, Variant::Slate] {
            for d in [1, 2] {
                let st = style_for(v).at_scale(d);
                let a = atlas(v, d);
                let n = SIDE * d;
                let ends: Vec<u32> = (n - 4 * d..n).map(|y| a.get(n - 1, y)).collect();
                for (w, h) in [(26, 26), (25, 25), (60, 300)] {
                    let (w, h) = (w * d, h * d);
                    let mut c = Canvas::new(w as u32, h as u32);
                    st.draw(
                        &mut c,
                        Rect::new(0, 0, w, h),
                        Part::PlacesBar,
                        State::Normal,
                        d,
                    );
                    let right: Vec<u32> = (h - 4 * d..h).map(|y| c.get(w - 1, y)).collect();
                    let bottom: Vec<u32> = (w - 4 * d..w).map(|x| c.get(x, h - 1)).collect();
                    assert_eq!(right, ends, "{v:?} {w}x{h} at {d}: the right edge's end");
                    assert_eq!(bottom, ends, "{v:?} {w}x{h} at {d}: the bottom edge's end");
                    assert_eq!(
                        c.get(w - 1, h - 4 * d - 1),
                        a.get(n - 1, n / 2),
                        "{v:?} {w}x{h} at {d}: before the end"
                    );
                }
            }
        }
    }
}
