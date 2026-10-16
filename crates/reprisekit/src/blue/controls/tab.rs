//! The `TabItem` (Normal, Hot, Checked) and `TabPane` atlases, drawn from the scheme's faces at the style's scale.
//! A closed tab is 11 columns plus a clear gap column against its right neighbour; the open tab has no gap
//! since it grows over both neighbours.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::Insets;
use crate::shape::{self, CLEAR, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};

const TAB_LEFT: i32 = 5;
const TAB_RIGHT: i32 = 6;
const TAB_W: i32 = TAB_LEFT + 1 + TAB_RIGHT;
const TAB_H: i32 = 18;
const TAB_TOP: i32 = 3;
const LIT_ROWS: f64 = 3.0;

const PANE_LEFT: i32 = 4;
const PANE_TOP: i32 = 4;
const PANE_RIGHT: i32 = 4;
const PANE_BOTTOM: i32 = 4;
const PANE_W: i32 = PANE_LEFT + 1 + PANE_RIGHT;
const PANE_H: i32 = 360;
const SHADOW: i32 = 2;

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let faces = [
        (State::Normal, &s.tab_item.normal),
        (State::Hot, &s.tab_item.hot),
        (State::Checked, &s.tab_item.checked),
    ];
    let r = f64::from(s.metrics.radii.tab_item);
    vec![
        (Part::TabItem, tab_item(&faces, r, d)),
        (Part::TabPane, tab_pane(&s.tab_pane.normal, d)),
    ]
}

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn tinted(body: Px, tint: Px) -> Px {
    (
        body.0,
        std::array::from_fn(|i| body.1[i] * tint.1[i] / 255.0),
    )
}

fn tab_item(faces: &[(State, &Face)], r: f64, d: i32) -> PartImage {
    let k = f64::from(d);
    let frames: Vec<_> = faces
        .iter()
        .map(|&(state, f)| {
            let lit = f.mark.unwrap_or(f.rim);
            let w = if state == State::Checked {
                TAB_W
            } else {
                TAB_W - 1
            };
            (
                f64::from(w) * k,
                shape::premul(f.rim, 255),
                shape::premul(lit, 255),
                stops(f.down),
                stops(f.across),
            )
        })
        .collect();
    // The outline runs past the frame's bottom: a tab has no bottom edge.
    let tall = f64::from(TAB_H) * k + r * k;
    let rr = [r * k, r * k, 0.0, 0.0];
    let ri = [(r - 1.0) * k, (r - 1.0) * k, 0.0, 0.0];
    let fh = TAB_H * d;
    let states: Vec<State> = faces.iter().map(|f| f.0).collect();
    let n = states.len() as i32;
    let atlas = shape::draw(TAB_W, TAB_H * n, d, |x, y| {
        let i = (y as i32 / fh) as usize;
        let y = y - f64::from(i as i32 * fh);
        let (w, rim, lit, down, across) = &frames[i];
        let outer = shape::rounded4(x, y, *w, tall, rr);
        if outer == 0.0 {
            return CLEAR;
        }
        let inner = shape::rounded4(x - k, y - k, w - 2.0 * k, tall, ri);
        let (cx, cy) = (shape::centre(x as i32, d), shape::centre(y as i32, d));
        let rim = if cy < LIT_ROWS - 0.5 { *lit } else { *rim };
        let body = tinted(
            Stops(down).at((cy + 0.5) / f64::from(TAB_H)),
            Stops(across).at((cx + 0.5) / f64::from(TAB_W)),
        );
        shape::shares(&[(rim, outer - inner), (body, inner)])
    });
    PartImage::new(atlas, &states, Layout::Vertical)
        .sizing(Insets::new(TAB_LEFT, TAB_TOP, TAB_RIGHT, 0))
        .content(Insets::new(1, 2, 0, 0))
        .at_scale(d)
}

fn tab_pane(f: &Face, d: i32) -> PartImage {
    let k = f64::from(d);
    let (fw, fh) = (PANE_W - SHADOW, PANE_H - SHADOW);
    let body = stops(f.down);
    let shadow = Stops(&stops(f.across));
    let (near, far) = (shadow.at(0.0), shadow.at(1.0));
    let (border, inner) = (shape::premul(f.rim, 255), shape::premul(f.inner, 255));
    let (w, h) = (f64::from(fw) * k, f64::from(fh) * k);
    let ring = |x: f64, y: f64, i: f64| {
        shape::rounded(x - i * k, y - i * k, w - 2.0 * i * k, h - 2.0 * i * k, 0.0)
    };
    let body_h = f64::from(fh - 4);
    let atlas = shape::draw(PANE_W, PANE_H, d, |x, y| {
        let soft = shape::over(
            shape::scale(near, shape::rounded(x - k, y - k, w, h, k)),
            shape::scale(far, shape::rounded(x - 2.0 * k, y - 2.0 * k, w, h, 2.0 * k)),
        );
        let (a, b, c) = (ring(x, y, 0.0), ring(x, y, 1.0), ring(x, y, 2.0));
        if a == 0.0 {
            return soft;
        }
        let t = (shape::centre(y as i32, d) - 1.5) / body_h;
        let page = shape::shares(&[(border, a - b), (inner, b - c), (Stops(&body).at(t), c)]);
        shape::over(page, soft)
    });
    PartImage::new(atlas, &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(PANE_LEFT, PANE_TOP, PANE_RIGHT, PANE_BOTTOM))
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::widgets::draw_page;
    use crate::raster::{Canvas, Rect};
    use crate::style::Style;
    use std::sync::Arc;

    fn style(v: Variant, d: i32) -> Arc<Style> {
        crate::blue::style_for(v).at_scale(d)
    }

    fn tab(v: Variant, state: State, d: i32) -> Canvas {
        let mut c = Canvas::new((TAB_W * d) as u32, (TAB_H * d) as u32);
        let r = Rect::new(0, 0, TAB_W * d, TAB_H * d);
        style(v, d).draw(&mut c, r, Part::TabItem, state, d);
        c
    }

    fn page(v: Variant, w: i32, h: i32, s: i32) -> Canvas {
        let mut theme = crate::interfaces::blue_theme(v);
        theme.scale = s;
        let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
        draw_page(&mut c, &theme.paint(), Rect::new(0, 0, w, h).scaled(s));
        c
    }

    fn frame(v: Variant, part: Part, state: State, d: i32) -> Canvas {
        let st = style(v, d);
        let img = st.part(part).expect("the part is built");
        let r = img.frame(state);
        let mut c = Canvas::new(r.width as u32, r.height as u32);
        for y in 0..r.height {
            for x in 0..r.width {
                c.put_raw(x, y, img.atlas.get(r.x + x, r.y + y));
            }
        }
        c
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        };
        eat(c.width);
        eat(c.height);
        for &p in &c.pixels {
            eat(p);
        }
        h
    }

    const STATES: [State; 3] = [State::Normal, State::Hot, State::Checked];

    #[test]
    fn tab_pins_at_scale_1_and_2() {
        #[rustfmt::skip]
        let want: [(Variant, [u64; 8]); 9] = [
            (Variant::Blue, [0x8118af1a3c459470, 0xbc311f21fa26b33b, 0x8c5ba86beb372fab, 0x3577aeece7fa7d92, 0xd0bccdc7d9e8ee7a, 0x5ec07295e32132ef, 0x6afd2f95f7dc2b9d, 0x4710a8c6ee0770de]),
            (Variant::Olive, [0x0d978c085b47797f, 0x5b27941d3fea9f01, 0x36ca20506f4e0403, 0x3577aeece7fa7d92, 0x27b070ecb829cacc, 0x310d07ca484b8f6c, 0x4773410656eecdd5, 0x4710a8c6ee0770de]),
            (Variant::Silver, [0xd4bf32212b16d783, 0x332a32c39d7185d7, 0x8c5ba86beb372fab, 0x9e3c0b7de7a5b3c3, 0x770cdcd0870933f5, 0x0e9e42b7955fbd61, 0x6afd2f95f7dc2b9d, 0x52a4f1a1348a5bd9]),
            (Variant::Slate, [0x77262e9e5c79ae4f, 0x9d11accb32fe3a34, 0x324e86c2f5561783, 0x8bc4995d4fc55107, 0x22c41544021c5739, 0xd0ab150cb2492615, 0x82287fb31907c8e5, 0xcba317ad3ee6ac78]),
            (Variant::Sapphire, [0xf2f6428de2bbd6ab, 0x4be926c1a9fa37fb, 0xedad7c4d9ae0e71b, 0xdea6b31369628a3a, 0xec839e71102dac51, 0x47c174d5c054ffdd, 0x84089dce4dcf6865, 0xdcd900bbda338f03]),
            (Variant::Obsidian, [0xf2f6428de2bbd6ab, 0x4be926c1a9fa37fb, 0xedad7c4d9ae0e71b, 0xdea6b31369628a3a, 0xec839e71102dac51, 0x47c174d5c054ffdd, 0x84089dce4dcf6865, 0xdcd900bbda338f03]),
            (Variant::Ember, [0x77262e9e5c79ae4f, 0x9d11accb32fe3a34, 0x324e86c2f5561783, 0x8bc4995d4fc55107, 0x22c41544021c5739, 0xd0ab150cb2492615, 0x82287fb31907c8e5, 0xcba317ad3ee6ac78]),
            (Variant::Cobalt, [0xf2f6428de2bbd6ab, 0x4be926c1a9fa37fb, 0xedad7c4d9ae0e71b, 0xdea6b31369628a3a, 0xec839e71102dac51, 0x47c174d5c054ffdd, 0x84089dce4dcf6865, 0xdcd900bbda338f03]),
            (Variant::Carbon, [0xf2f6428de2bbd6ab, 0x4be926c1a9fa37fb, 0xedad7c4d9ae0e71b, 0xdea6b31369628a3a, 0xec839e71102dac51, 0x47c174d5c054ffdd, 0x84089dce4dcf6865, 0xdcd900bbda338f03]),
        ];
        let got: Vec<(Variant, [u64; 8])> = Variant::ALL
            .into_iter()
            .map(|v| {
                let mut hs = [0u64; 8];
                for (j, d) in [1, 2].into_iter().enumerate() {
                    for (i, state) in STATES.into_iter().enumerate() {
                        hs[4 * j + i] = fnv(&frame(v, Part::TabItem, state, d));
                    }
                    hs[4 * j + 3] = fnv(&frame(v, Part::TabPane, State::Normal, d));
                }
                (v, hs)
            })
            .collect();
        assert_eq!(want.as_slice(), got.as_slice(), "the tab and page frames");
    }

    #[test]
    fn a_tab_and_page_at_scale_2_are_twice_the_size_with_the_same_grid() {
        for v in Variant::ALL {
            for part in [Part::TabItem, Part::TabPane] {
                let one = style(v, 1);
                let two = style(v, 2);
                let (a, b) = (one.part(part).unwrap(), two.part(part).unwrap());
                assert_eq!(b.scale, 2, "{v:?} {part:?} is painted at 2");
                assert_eq!(
                    (b.atlas.width, b.atlas.height),
                    (2 * a.atlas.width, 2 * a.atlas.height),
                    "{v:?} {part:?}"
                );
                assert_eq!(a.sizing, b.sizing, "{v:?} {part:?}: design insets");
                assert_eq!(a.content, b.content, "{v:?} {part:?}: design content");
            }
        }
    }

    #[test]
    fn a_tabs_states_differ() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let [n, h, c] = STATES.map(|s| tab(v, s, d));
                assert!(n != h && h != c && n != c, "{v:?} at {d}");
            }
        }
    }

    #[test]
    fn a_tabs_corners_and_gap_are_clear() {
        for v in Variant::ALL {
            for d in [1, 2] {
                for state in STATES {
                    let c = tab(v, state, d);
                    let right = if state == State::Checked {
                        TAB_W * d - 1
                    } else {
                        (TAB_W - 1) * d - 1
                    };
                    // The curve only grazes the corner pixel.
                    assert!(c.get(0, 0) >> 24 < 8, "{v:?} {state:?} at {d}: top-left");
                    assert!(
                        c.get(right, 0) >> 24 < 8,
                        "{v:?} {state:?} at {d}: top-right"
                    );
                    assert_eq!(c.get(0, 8 * d) >> 24, 0xFF, "{v:?} {state:?}: left side");
                    assert_eq!(
                        c.get(right, 8 * d) >> 24,
                        0xFF,
                        "{v:?} {state:?}: right side"
                    );
                    let gap = c.get(TAB_W * d - 1, 8 * d) >> 24;
                    if state == State::Checked {
                        assert_eq!(gap, 0xFF, "{v:?}: the open tab has no gap");
                    } else {
                        assert_eq!(gap, 0, "{v:?} {state:?}: the gap column is clear");
                    }
                }
            }
        }
    }

    #[test]
    fn a_tabs_top_is_smooth_at_scale_2() {
        for v in Variant::ALL {
            for state in STATES {
                let c = tab(v, state, 2);
                let partial = (0..6)
                    .flat_map(|y| (0..6).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let a = c.get(x, y) >> 24;
                        a > 0 && a < 255
                    })
                    .count();
                assert!(partial >= 3, "{v:?} {state:?}: {partial} edge pixels");
                let lum = |p: u32| {
                    let a = (p >> 24).max(1);
                    let ch = |s: u32| ((p >> s) & 0xFF) * 255 / a;
                    ch(16) + ch(8) + ch(0)
                };
                for t in 2..5 {
                    let (p, q, r) = (c.get(t - 1, t - 1), c.get(t, t), c.get(t + 1, t + 1));
                    if q >> 24 == 255 && p >> 24 == 255 && r >> 24 == 255 {
                        assert!(
                            lum(q) >= lum(p).min(lum(r)),
                            "{v:?} {state:?}: ({t}, {t}) is a dark fringe"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_lit_tab_has_its_marks_top() {
        for v in Variant::ALL {
            let s = v.scheme();
            for (state, f) in [
                (State::Normal, &s.tab_item.normal),
                (State::Hot, &s.tab_item.hot),
                (State::Checked, &s.tab_item.checked),
            ] {
                let c = tab(v, state, 1);
                let top = f.mark.unwrap_or(f.rim);
                assert_eq!(c.get(5, 0), top.argb(), "{v:?} {state:?}");
                assert_eq!(c.get(0, 8), f.rim.argb(), "{v:?} {state:?}");
            }
        }
    }

    #[test]
    fn the_open_tab_is_the_pages_colour() {
        for v in Variant::ALL {
            let s = v.scheme();
            let c = tab(v, State::Checked, 1);
            let body = s.tab_pane.normal.down[0].1;
            assert_eq!(c.get(5, 10), body.argb(), "{v:?}");
        }
    }

    #[test]
    fn a_page_has_a_border_an_inner_line_and_a_soft_shadow() {
        let (w, h) = (120, 90);
        for v in Variant::ALL {
            let f = &v.scheme().tab_pane.normal;
            let rgb = |c: crate::theme::Color| c.argb();
            let (near, far) = (f.across[0].1, f.across[f.across.len() - 1].1);
            let c = page(v, w, h, 1);
            for (x, y, want, what) in [
                (0, 0, rgb(f.rim), "the top-left corner"),
                (50, 0, rgb(f.rim), "the top border"),
                (50, 1, rgb(f.inner), "the inner line under the top border"),
                (0, 40, rgb(f.rim), "the left border"),
                (1, 40, rgb(f.inner), "the inner line inside the left border"),
                (
                    w - 4,
                    40,
                    rgb(f.inner),
                    "the inner line inside the right border",
                ),
                (w - 3, 40, rgb(f.rim), "the right border"),
                (w - 2, 40, rgb(near), "the shadow's first column"),
                (w - 1, 40, rgb(far), "the shadow's second column"),
                (50, h - 3, rgb(f.rim), "the bottom border"),
                (50, h - 2, rgb(near), "the shadow's first row"),
                (50, h - 1, rgb(far), "the shadow's second row"),
                (50, 2, rgb(f.down[0].1), "the body starts at its top colour"),
            ] {
                assert_eq!(c.get(x, y), want, "{v:?} ({x}, {y}): {what}");
            }
            assert!(
                c.get(w - 1, 0) >> 24 == 0,
                "{v:?}: the shadow starts below the top"
            );
            assert!(
                c.get(w - 1, h - 1) >> 24 < 255,
                "{v:?}: the shadow rounds off at the bottom-right"
            );
        }
    }

    #[test]
    fn a_navy_page_fades_60_percent_down_at_any_height() {
        let f = &Variant::Blue.scheme().tab_pane.normal;
        let grey = f.down[f.down.len() - 1].1.argb();
        for h in [120, 360, 720] {
            let c = page(Variant::Blue, 40, h, 1);
            let (top, bottom) = (2, h - 4);
            let first = (top..bottom)
                .find(|&y| c.get(20, y) == grey)
                .expect("the fade reaches its bottom colour");
            let share = f64::from(first - top) / f64::from(bottom - top);
            assert!((0.57..=0.62).contains(&share), "a {h} px page: {share:.3}");
            assert!(
                (first..bottom).all(|y| c.get(20, y) == grey),
                "a {h} px page stays"
            );
        }
    }

    #[test]
    fn olive_green_pages_as_navy_does() {
        assert!(
            page(Variant::Olive, 100, 60, 1) == page(Variant::Blue, 100, 60, 1),
            "an olive green page is navy's"
        );
    }
}
