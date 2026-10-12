//! The task pane in Blue, drawn from each scheme's faces at the draw's scale.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad, Scheme};
use crate::metrics::TaskMetrics;
use crate::raster::{Canvas, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const BACK_H: i32 = 456;
const BAND_W: i32 = 185;
/// The title band's fixed ends: the rounded corner's columns and rows, and the fold button's 24
/// columns on the right.
const HEADER_LEFT: i32 = 4;
const HEADER_TOP: i32 = 4;
const BUTTON_W: i32 = 24;
const GLOSS_SPLIT: f64 = 12.0;

/// The fold button's disc, in design px from the button's top-left: centre, radius, rim width.
const DISC_X: f64 = 9.5;
const DISC_Y: f64 = 11.0;
const DISC_R: f64 = 8.0;
const DISC_RIM: f64 = 1.0;
const DISC_LIGHT: Color = Color::rgb(0xFF, 0xFF, 0xFF);
/// The disc's shadow: offset down and right, its edge softened over `SHADOW_SOFT` px each way.
const SHADOW_DX: f64 = 0.75;
const SHADOW_DY: f64 = 1.25;
const SHADOW_SOFT: f64 = 1.75;
const UP: [[(f64, f64); 3]; 2] = [
    [(6.0, 11.0), (9.5, 7.5), (13.0, 11.0)],
    [(6.0, 15.0), (9.5, 11.5), (13.0, 15.0)],
];
const DOWN: [[(f64, f64); 3]; 2] = [
    [(6.0, 8.0), (9.5, 11.5), (13.0, 8.0)],
    [(6.0, 12.0), (9.5, 15.5), (13.0, 12.0)],
];

fn band_h(v: Variant) -> i32 {
    match v {
        Variant::Blue => 25,
        _ => 23,
    }
}

fn solid(c: Color) -> Px {
    shape::premul(c, 255)
}

/// Positions scaled by `len`, so a stop sits at a pixel's middle plus a half, in design px.
fn stops(g: Grad, len: f64) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t) * len, solid(c)))
        .collect()
}

/// The middle of device pixel `v` at `d`, in design px from the part's top-left corner.
fn mid(v: f64, d: i32) -> f64 {
    shape::centre(v as i32, d) + 0.5
}

#[derive(Clone, Copy)]
enum Side {
    Fill,
    Frame,
    Lift([f64; 3]),
}

struct Band {
    h: f64,
    radius: f64,
    kind: Kind,
}

enum Kind {
    Flat(Vec<(f64, Px)>),
    Gloss {
        frame: Px,
        upper: Vec<(f64, Px)>,
        lower: Vec<(f64, Px)>,
        bottom: bool,
        sides: [Side; 2],
    },
}

impl Band {
    fn new(s: &Scheme, h: i32) -> Band {
        let f = &s.task_group_header.normal;
        let hf = f64::from(h);
        let radius = f64::from(s.metrics.radii.task_group_header);
        if f.down.len() <= 1 {
            return Band {
                h: hf,
                radius,
                kind: Kind::Flat(stops(f.across, f64::from(BAND_W))),
            };
        }
        // The top stop on the frame colour is left out of the fill; the bottom one stays, as the
        // lower half shades down towards it.
        let mut inner = f.down;

        if inner.first().is_some_and(|&(_, c)| c == f.rim) {
            inner = &inner[1..];
        }
        let bottom = inner.last().is_some_and(|&(_, c)| c == f.rim);
        let rows = stops(inner, hf);
        let (upper, lower) = rows
            .into_iter()
            .partition(|&(row, _)| row < GLOSS_SPLIT + 0.5);
        let across = Stops(&stops(f.across, f64::from(BAND_W)));
        let fill = across.at(f64::from(BAND_W) / 2.0);
        let side = |col: f64| {
            let c = across.at(col + 0.5);
            if c == solid(f.rim) {
                Side::Frame
            } else if c == fill {
                Side::Fill
            } else {
                Side::Lift(std::array::from_fn(|i| c.1[i] - fill.1[i]))
            }
        };
        Band {
            h: hf,
            radius,
            kind: Kind::Gloss {
                frame: solid(f.rim),
                upper,
                lower,
                bottom,
                sides: [side(0.0), side(1.0)],
            },
        }
    }

    fn at(&self, x: f64, y: f64) -> Px {
        match &self.kind {
            Kind::Flat(across) => Stops(across).at(x),
            Kind::Gloss {
                frame,
                upper,
                lower,
                bottom,
                sides,
            } => {
                if y < 1.0 || (*bottom && y > self.h - 1.0) {
                    return *frame;
                }
                let half = if y < GLOSS_SPLIT { upper } else { lower };
                let fill = Stops(half).at(y);
                let w = f64::from(BAND_W);
                let col = x.min(w - x).floor() as usize;
                match sides.get(col) {
                    Some(Side::Frame) => *frame,
                    Some(Side::Lift(by)) => (
                        fill.0,
                        std::array::from_fn(|i| (fill.1[i] + by[i]).clamp(0.0, fill.0)),
                    ),
                    _ => fill,
                }
            }
        }
    }

    fn cover(&self, x: f64, y: f64, x0: i32, d: i32) -> f64 {
        let k = f64::from(d);
        let r = self.radius * k;
        shape::rounded4(
            x + f64::from(x0) * k,
            y,
            f64::from(BAND_W) * k,
            self.h * k,
            [r, r, 0.0, 0.0],
        )
    }

    fn paint(&self, x: f64, y: f64, x0: i32, d: i32) -> Px {
        let c = self.at(mid(x, d) + f64::from(x0), mid(y, d));
        shape::scale(c, self.cover(x, y, x0, d))
    }
}

fn disc(f: &Face, (shadow, alpha): (Color, u8), x: f64, y: f64, d: i32, under: Px) -> Px {
    let k = f64::from(d);
    let (mx, my) = (mid(x, d), mid(y, d));
    let reach = ((mx - DISC_X - SHADOW_DX).hypot(my - DISC_Y - SHADOW_DY) - DISC_R) / SHADOW_SOFT;
    let shade = 1.0 - shape::smooth((reach + 1.0) / 2.0);
    let under = shape::over(shape::scale(shape::premul(shadow, alpha), shade), under);
    let (cx, cy) = (DISC_X * k, DISC_Y * k);
    let outer = shape::ellipse(x, y, cx, cy, DISC_R * k, DISC_R * k);
    if outer == 0.0 {
        return under;
    }
    let face_r = DISC_R - DISC_RIM;
    let inner = shape::ellipse(x, y, cx, cy, face_r * k, face_r * k);
    let u = ((mx - DISC_X) + (my - DISC_Y)) / (std::f64::consts::SQRT_2 * face_r);
    let face = Stops(&[(0.0, solid(DISC_LIGHT)), (1.0, solid(f.inner))]).at(u);
    let top = shape::shares(&[(solid(f.rim), outer - inner), (face, inner)]);
    shape::over(top, under)
}

fn chevrons(lines: &[[(f64, f64); 3]; 2], x: f64, y: f64, d: i32) -> f64 {
    let k = f64::from(d);
    lines
        .iter()
        .map(|l| {
            let pts = l.map(|(px, py)| (px * k, py * k));
            shape::stroke(x, y, &pts, std::f64::consts::SQRT_2 * k)
        })
        .fold(0.0, f64::max)
}

fn button(band: &Band, f: &Face, lines: &[[(f64, f64); 3]; 2], d: i32) -> Canvas {
    let x0 = BAND_W - BUTTON_W;
    let ink = solid(f.mark.unwrap_or(f.rim));
    shape::draw(BUTTON_W, band.h as i32, d, |x, y| {
        let mut p = band.paint(x, y, x0, d);
        if let Some(halo) = f.halo {
            p = disc(f, halo, x, y, d, p);
        }
        shape::over(shape::scale(ink, chevrons(lines, x, y, d)), p)
    })
}

fn stack(frames: &[Canvas]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    a.pixels = frames
        .iter()
        .flat_map(|f| f.pixels.iter().copied())
        .collect();
    a
}

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let h = band_h(v);
    let band = Band::new(s, h);

    let pane = Stops(&stops(s.task_pane.normal.down, f64::from(BACK_H)));
    let back = shape::draw(1, BACK_H, d, |_, y| pane.at(mid(y, d)));

    let header = shape::draw(BAND_W, h, d, |x, y| band.paint(x, y, 0, d));

    let b = &s.task_group_body.normal;
    let (fill, edge) = (Stops(&stops(b.across, 1.0)).at(0.5), solid(b.rim));
    let body = shape::draw(3, 2, d, |x, y| {
        let (mx, my) = (mid(x, d), mid(y, d));
        if (1.0..2.0).contains(&mx) && my < 1.0 {
            fill
        } else {
            edge
        }
    });

    let c = &s.task_collapse;
    let collapse = stack(&[
        button(&band, &c.normal, &UP, d),
        button(&band, &c.hot, &UP, d),
        button(&band, &c.pressed, &UP, d),
    ]);
    let expand = button(&band, &s.task_expand.normal, &DOWN, d);

    let image =
        |a: Canvas, states: &[State]| PartImage::new(a, states, Layout::Vertical).at_scale(d);
    vec![
        (Part::TaskPane, image(back, &[State::Normal])),
        (
            Part::TaskGroupHeader,
            image(header, &[State::Normal]).sizing(Insets::new(
                HEADER_LEFT,
                HEADER_TOP,
                BUTTON_W,
                0,
            )),
        ),
        (
            Part::TaskGroupBody,
            image(body, &[State::Normal]).sizing(Insets::new(1, 0, 1, 1)),
        ),
        (
            Part::TaskCollapse,
            image(collapse, &[State::Normal, State::Hot, State::Pressed]),
        ),
        (Part::TaskExpand, image(expand, &[State::Normal])),
    ]
}

pub(super) fn inks(v: Variant) -> Vec<(Part, State, Color)> {
    let i = &v.scheme().inks;
    vec![
        (Part::TaskGroupHeader, State::Normal, i.task_title),
        (Part::TaskGroupHeader, State::Hot, i.task_title_hot),
        (Part::TaskGroupBody, State::Normal, i.task_link),
        (Part::TaskGroupBody, State::Hot, i.task_link_hot),
    ]
}

pub(super) fn metrics(v: Variant) -> TaskMetrics {
    TaskMetrics {
        lead: 1,
        top: 12,
        header: band_h(v),
        section_x: 13,
        section_inset: 15,
        text_dx: 1,
        foot: 1,
        line: 13,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
        parts(v, d)
    }

    fn part(v: Variant, d: i32, p: Part) -> PartImage {
        built(v, d).into_iter().find(|(q, _)| *q == p).unwrap().1
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325_u64;
        let bytes = [c.width, c.height]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .chain(c.pixels.iter().flat_map(|p| p.to_le_bytes()));
        for b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    fn frame(img: &PartImage, i: usize) -> Canvas {
        let n = img.states.len() as u32;
        let h = img.atlas.height / n;
        let mut c = Canvas::new(img.atlas.width, h);
        for y in 0..h as i32 {
            for x in 0..img.atlas.width as i32 {
                c.put_raw(x, y, img.atlas.get(x, y + i as i32 * h as i32));
            }
        }
        c
    }

    const PARTS: [Part; 5] = [
        Part::TaskPane,
        Part::TaskGroupHeader,
        Part::TaskGroupBody,
        Part::TaskCollapse,
        Part::TaskExpand,
    ];

    #[test]
    fn taskpane_parts_grow_with_the_scale_and_keep_their_borders() {
        for v in Variant::ALL {
            for p in PARTS {
                let (one, two) = (part(v, 1, p), part(v, 2, p));
                assert_eq!(two.scale, 2, "{v:?} {p:?}");
                assert_eq!(
                    (two.atlas.width, two.atlas.height),
                    (2 * one.atlas.width, 2 * one.atlas.height),
                    "{v:?} {p:?}"
                );
                assert_eq!(one.sizing, two.sizing, "{v:?} {p:?} borders");
                assert_eq!(one.states, two.states, "{v:?} {p:?} states");
            }
            let h = band_h(v);
            let header = part(v, 1, Part::TaskGroupHeader);
            assert_eq!((header.atlas.width, header.atlas.height), (185, h as u32));
            assert_eq!(header.sizing, Insets::new(4, 4, 24, 0), "{v:?}");
            let fold = part(v, 1, Part::TaskCollapse);
            assert_eq!(fold.atlas.height, 3 * h as u32, "{v:?}");
        }
    }

    #[test]
    fn taskpane_fold_states_differ_where_they_should() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let fold = part(v, d, Part::TaskCollapse);
                let (rest, hot, held) = (frame(&fold, 0), frame(&fold, 1), frame(&fold, 2));
                assert!(rest != hot, "{v:?} at {d}: lit differs");
                assert!(hot == held, "{v:?} at {d}: held draws lit");
                let unfold = frame(&part(v, d, Part::TaskExpand), 0);
                assert!(unfold != rest, "{v:?} at {d}: unfold differs");
            }
        }
    }

    #[test]
    fn taskpane_fold_frames_keep_their_clear_corner() {
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            for d in [1, 2] {
                let fold = part(v, d, Part::TaskCollapse);
                for i in 0..3 {
                    let corner = frame(&fold, i).get(BUTTON_W * d - 1, 0);
                    assert_eq!(corner >> 24, 0, "{v:?} at {d}: frame {i} corner");
                }
            }
        }
    }

    #[test]
    fn taskpane_chevrons_hold_solid_pixels_at_scale_1() {
        for v in Variant::ALL {
            let s = v.scheme();
            let ink = shape::pack(solid(s.task_collapse.normal.mark.unwrap()));
            let fold = frame(&part(v, 1, Part::TaskCollapse), 0);
            for chevron in UP {
                for arm in chevron.windows(2) {
                    let ((x0, y0), (x1, y1)) = (arm[0], arm[1]);
                    let solid_px = (0..=8).any(|i| {
                        let t = f64::from(i) / 8.0;
                        let (x, y) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t);
                        fold.get(x as i32, y as i32) == ink
                    });
                    assert!(solid_px, "{v:?}: arm {arm:?}");
                }
            }
        }
    }

    #[test]
    fn taskpane_curves_are_smooth_at_scale_2() {
        let partial = |c: &Canvas, x: i32, y: i32, w: i32, h: i32| {
            (y..y + h).any(|y| {
                (x..x + w).any(|x| {
                    let a = c.get(x, y) >> 24;
                    a > 0 && a < 255
                })
            })
        };
        for v in [
            Variant::Blue,
            Variant::Olive,
            Variant::Silver,
            Variant::Sapphire,
        ] {
            let header = part(v, 2, Part::TaskGroupHeader).atlas;
            assert!(partial(&header, 0, 0, 8, 8), "{v:?}: corner");
        }
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let s = v.scheme();
            let rim = shape::pack(solid(s.task_collapse.normal.rim));
            let fold = frame(&part(v, 2, Part::TaskCollapse), 0);
            let (cx, cy) = (2.0 * DISC_X, 2.0 * DISC_Y);
            let mixed = (0..64).any(|i| {
                let a = f64::from(i) * std::f64::consts::TAU / 64.0;
                let r = 2.0 * DISC_R - 0.5;
                let (x, y) = ((cx + r * a.cos()) as i32, (cy + r * a.sin()) as i32);
                let p = fold.get(x, y);
                p != rim && p != fold.get(x + 3, y)
            });
            assert!(mixed, "{v:?}: disc edge");
        }
    }

    #[test]
    fn taskpane_disc_edge_has_no_dark_fringe() {
        let luma = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let rim = shape::pack(solid(v.scheme().task_collapse.normal.rim));
            for d in [1, 2] {
                let fold = frame(&part(v, d, Part::TaskCollapse), 0);
                let k = f64::from(d);
                for i in 0..16 {
                    let a = std::f64::consts::PI * (1.0 + f64::from(i) / 32.0);
                    let at = |r: f64| {
                        let x = DISC_X * k + r * k * a.cos();
                        let y = DISC_Y * k + r * k * a.sin();
                        fold.get(x as i32, y as i32)
                    };
                    let band = at(DISC_R + 1.5);
                    let floor = luma(rim).min(luma(band));
                    for r in [DISC_R - 1.0, DISC_R - 0.5, DISC_R, DISC_R + 0.5] {
                        let p = at(r);
                        assert!(
                            luma(p) + 6 >= floor,
                            "{v:?} at {d}: {p:08X} by rim {rim:08X} and band {band:08X}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn taskpane_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                for (p, img) in parts(v, d) {
                    got.push(format!("{v:?} {d} {p:?} {:016x}", fnv(&img.atlas)));
                }
            }
        }
        let want: &[&str] = &[
            "Blue 1 TaskPane ff7231653d1966cb",
            "Blue 1 TaskGroupHeader 593e9756e3ff9629",
            "Blue 1 TaskGroupBody b9c4689111795d6f",
            "Blue 1 TaskCollapse 1737956df23b7476",
            "Blue 1 TaskExpand 6a76c31f07a5b6df",
            "Blue 2 TaskPane c01f09bf3aadfd3e",
            "Blue 2 TaskGroupHeader 2b4dbc5db9364cb2",
            "Blue 2 TaskGroupBody 9b1383fa0adeda4f",
            "Blue 2 TaskCollapse c5069efc2bf50fc5",
            "Blue 2 TaskExpand 6707ccf7e7c4925c",
            "Olive 1 TaskPane ddd4a6e10b66a469",
            "Olive 1 TaskGroupHeader aa4867524a694eaf",
            "Olive 1 TaskGroupBody eb6282be448cad2d",
            "Olive 1 TaskCollapse cf95f58db61905ec",
            "Olive 1 TaskExpand 16f54fd9ac343cd3",
            "Olive 2 TaskPane 64b96016d0bf2b26",
            "Olive 2 TaskGroupHeader aa80f171a8aee5e0",
            "Olive 2 TaskGroupBody 2c1b295716f856ef",
            "Olive 2 TaskCollapse 16d2a8710485a834",
            "Olive 2 TaskExpand 8b69585c9f130ea8",
            "Silver 1 TaskPane 10d3053f5e2a824b",
            "Silver 1 TaskGroupHeader baec17e51d89cdd1",
            "Silver 1 TaskGroupBody 73c71434620db498",
            "Silver 1 TaskCollapse 6fbc31eb6d60364c",
            "Silver 1 TaskExpand 0dc0e98a21ff3c48",
            "Silver 2 TaskPane b540e72524ed476a",
            "Silver 2 TaskGroupHeader c3fe20b9d9d3adf7",
            "Silver 2 TaskGroupBody c7ca63e4b9570f67",
            "Silver 2 TaskCollapse 5867e8051604027d",
            "Silver 2 TaskExpand 5d31cb68cbe72c9f",
            "Slate 1 TaskPane 585063609a0563d3",
            "Slate 1 TaskGroupHeader c553ffcb6a9a501a",
            "Slate 1 TaskGroupBody ebbe1c7b843a8b0d",
            "Slate 1 TaskCollapse 6ca56cb4171b2c5f",
            "Slate 1 TaskExpand 01a3052a65342a6e",
            "Slate 2 TaskPane 3841a4649858851e",
            "Slate 2 TaskGroupHeader cb7f7771359815f2",
            "Slate 2 TaskGroupBody 14210559af5e5b07",
            "Slate 2 TaskCollapse 3615d48cef86bccf",
            "Slate 2 TaskExpand 3720ea82dfc275e7",
            "Sapphire 1 TaskPane b7160e4bcdb65a43",
            "Sapphire 1 TaskGroupHeader a590c0b748f7381b",
            "Sapphire 1 TaskGroupBody 2065d2dfe260bd47",
            "Sapphire 1 TaskCollapse c3c6e0dc3472a1f9",
            "Sapphire 1 TaskExpand 27132ae539bc192b",
            "Sapphire 2 TaskPane a8dabaa728c60cd2",
            "Sapphire 2 TaskGroupHeader 2848b7a082cba2fe",
            "Sapphire 2 TaskGroupBody ff2dc1f2b6f224ef",
            "Sapphire 2 TaskCollapse 26d48021e5681b7b",
            "Sapphire 2 TaskExpand f907cfd70d366b5f",
            "Obsidian 1 TaskPane df0c17f8f76528ed",
            "Obsidian 1 TaskGroupHeader 7782715effb0ea72",
            "Obsidian 1 TaskGroupBody c45688bfc13136b7",
            "Obsidian 1 TaskCollapse 69715252292d543c",
            "Obsidian 1 TaskExpand 2c462cf3a4843931",
            "Obsidian 2 TaskPane b9f7c838d7b156f6",
            "Obsidian 2 TaskGroupHeader d28870b3a5c391ce",
            "Obsidian 2 TaskGroupBody 1bcb5026ed7f3aef",
            "Obsidian 2 TaskCollapse dcde2426ebe94905",
            "Obsidian 2 TaskExpand 1f5a24c298c33f21",
            "Ember 1 TaskPane f634532821178187",
            "Ember 1 TaskGroupHeader 801bc31c3e278712",
            "Ember 1 TaskGroupBody 107f63d5820c5f9f",
            "Ember 1 TaskCollapse 79ea9fdc92e03c3a",
            "Ember 1 TaskExpand 1f0fd497ac1c63b0",
            "Ember 2 TaskPane b3fa1119b76f7962",
            "Ember 2 TaskGroupHeader 68d520fed07d4966",
            "Ember 2 TaskGroupBody ccb7d6206f8797f7",
            "Ember 2 TaskCollapse 9dbb0c5800ea0d2b",
            "Ember 2 TaskExpand bd4e3b48a2da5687",
        ];
        assert_eq!(got, want);
    }
}
