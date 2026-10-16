//! A list header's atlases per colour scheme: item (Normal, Hot, Pressed), filler past the last column, sort arrow (Normal rising, Checked falling). An item frame is 18x20 (8 px ends, 2 px tiled middle); each face shades down one design row at a time so rows stay crisp at every scale.

use crate::blue::Variant;
use crate::blue::scheme::{Face, Grad};
use crate::raster::{Canvas, Fill, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, Part, PartImage, State};
use crate::theme::Color;

const ITEM_W: i32 = 18;
const ITEM_H: i32 = 20;
const END: i32 = 8;
const TOP: i32 = 3;
const BOTTOM: i32 = 4;
/// The 2 px divider's first and last rows (navy, olive green, silver).
const DIVIDER: (i32, i32) = (3, 15);
/// The 1 px divider's fade: clear on row middle `FADE_IN`, whole two rows on; whole two rows before `FADE_OUT`, clear there.
const FADE_IN: f64 = 1.0;
const FADE_OUT: f64 = 17.0;
/// The lit band's first row and its bottom-left corner's radius (navy, olive green, silver).
const BAND_TOP: i32 = 17;
const BAND_R: f64 = 1.5;
const ARROW_W: i32 = 9;
const ARROW_ROWS: (f64, f64) = (7.0, 12.0);
const EDGE_IN: f64 = 1e-6;

/// The title's box inside a column in `v`; the header's 6 px text margin applies inside it. Navy, olive green and silver: 3 px in on the left, rows 2 to 18. Slate, Sapphire, Obsidian and Cobalt start it at the left edge; Ember ends it on row 16.
fn title(v: Variant) -> Insets {
    match v {
        Variant::Blue | Variant::Olive | Variant::Silver | Variant::Steel => {
            Insets::new(3, 2, 0, 1)
        }
        Variant::Slate
        | Variant::Sapphire
        | Variant::Obsidian
        | Variant::Cobalt
        | Variant::Carbon => Insets::new(0, 2, 0, 1),
        Variant::Ember => Insets::new(3, 2, 0, 3),
    }
}

/// Whether `v` draws the 2 px divider and the rounded hot band (navy, olive green, silver) rather than the 1 px divider and flat band.
fn lined(v: Variant) -> bool {
    matches!(
        v,
        Variant::Blue | Variant::Olive | Variant::Silver | Variant::Steel
    )
}

pub(super) fn parts(v: Variant, d: i32) -> Vec<(Part, PartImage)> {
    let s = v.scheme();
    let faces = &s.header_item;

    let states = [State::Normal, State::Hot, State::Pressed];
    let frames = [
        normal(v, &faces.normal, d),
        hot(v, &faces.hot, d),
        pressed(&faces.pressed, d),
    ];
    let mut a = Canvas::new((ITEM_W * d) as u32, (ITEM_H * d * 3) as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, i as i32 * ITEM_H * d, f);
    }
    let mut item = PartImage::new(a, &states, Layout::Vertical)
        .sizing(Insets::new(END, TOP, END, BOTTOM))
        .content(title(v))
        .at_scale(d);
    item.fill = Fill::Tile;

    let down = rows(s.header_filler.normal.down);
    let f = shape::draw(1, ITEM_H, d, |_, y| down[(y as i32 / d) as usize]);
    let filler = PartImage::new(f, &[State::Normal], Layout::Vertical).at_scale(d);

    let ink = s.system.gray_text;
    let mut a = Canvas::new((ARROW_W * d) as u32, (2 * ITEM_H * d) as u32);
    a.blit_over(0, 0, &arrow(ink, false, d));
    a.blit_over(0, ITEM_H * d, &arrow(ink, true, d));
    let mut arrow =
        PartImage::new(a, &[State::Normal, State::Checked], Layout::Vertical).at_scale(d);
    arrow.true_size = true;

    vec![
        (Part::HeaderItem, item),
        (Part::HeaderFiller, filler),
        (Part::HeaderSortArrow, arrow),
    ]
}

fn at(g: Grad, t: f64) -> Px {
    let stops: Vec<(f64, Px)> = g
        .iter()
        .map(|&(p, c)| (f64::from(p), shape::premul(c, 255)))
        .collect();
    Stops(&stops).at(t)
}

fn rows(g: Grad) -> Vec<Px> {
    (0..ITEM_H)
        .map(|r| at(g, (f64::from(r) + 0.5) / f64::from(ITEM_H)))
        .collect()
}

fn column(g: Grad, c: i32) -> Px {
    at(g, (f64::from(c) + 0.5) / f64::from(ITEM_W))
}

/// How much of the 1 px divider shows at `y`: none above `FADE_IN`, whole two rows on; with `tail`, whole until two rows before `FADE_OUT`, none past it.
fn fade(y: f64, tail: bool) -> f64 {
    let head = ((y - FADE_IN) / 2.0).clamp(0.0, 1.0);
    if tail {
        head * ((FADE_OUT - y) / 2.0).clamp(0.0, 1.0)
    } else {
        head
    }
}

fn normal(v: Variant, f: &Face, d: i32) -> Canvas {
    let down = rows(f.down);
    let (shadow, light) = (column(f.across, ITEM_W - 2), column(f.across, ITEM_W - 1));
    let lined = lined(v);
    shape::draw(ITEM_W, ITEM_H, d, |x, y| {
        let (c, r) = (x as i32 / d, y as i32 / d);
        let face = down[r as usize];
        if lined {
            match c {
                _ if !(DIVIDER.0..=DIVIDER.1).contains(&r) => face,
                c if c == ITEM_W - 2 => shadow,
                c if c == ITEM_W - 1 => light,
                _ => face,
            }
        } else if c == ITEM_W - 1 {
            shape::mix(face, light, fade(shape::centre(y as i32, d), true))
        } else {
            face
        }
    })
}

fn hot(v: Variant, f: &Face, d: i32) -> Canvas {
    let down = rows(f.down);
    let k = f64::from(d);
    if lined(v) {
        let face = down[0];
        return shape::draw(ITEM_W, ITEM_H, d, |x, y| {
            let r = y as i32 / d;
            if r < BAND_TOP {
                return down[r as usize];
            }
            let band = f64::from(ITEM_H - BAND_TOP) * k;
            let w = f64::from(ITEM_W) * k;
            let top = f64::from(BAND_TOP) * k;
            let cover = shape::rounded4(x, y - top, w, band, [0.0, 0.0, 0.0, BAND_R * k]);
            shape::shares(&[(down[r as usize], cover), (face, 1.0 - cover)])
        });
    }
    let line = column(f.across, ITEM_W - 1);
    shape::draw(ITEM_W, ITEM_H, d, |x, y| {
        let (c, r) = (x as i32 / d, y as i32 / d);
        let face = down[r as usize];
        if c == ITEM_W - 1 && r < ITEM_H - 1 {
            shape::mix(face, line, fade(shape::centre(y as i32, d), false))
        } else {
            face
        }
    })
}

/// A column held down: outline down both sides and along the bottom, two shadow rows and columns along the top and left, the face.
fn pressed(f: &Face, d: i32) -> Canvas {
    let rim = shape::premul(f.rim, 255);
    let face = at(f.down, 0.5);
    let shadow = at(f.down, 0.0);
    let soft = shape::mix(shadow, face, 0.5);
    shape::draw(ITEM_W, ITEM_H, d, |x, y| {
        let (c, r) = (x as i32 / d, y as i32 / d);
        if c == 0 || c == ITEM_W - 1 || r == ITEM_H - 1 {
            rim
        } else if r == 0 || c == 1 {
            shadow
        } else if r == 1 || c == 2 {
            soft
        } else {
            face
        }
    })
}

fn arrow(ink: Color, falling: bool, d: i32) -> Canvas {
    let k = f64::from(d);
    let (tip, base) = if falling {
        (ARROW_ROWS.1, ARROW_ROWS.0)
    } else {
        ARROW_ROWS
    };
    // The sides run at 45 degrees from a 1 px tip, so each row holds two more whole pixels than the one above. Both sides sit a hair inside their pixel corners so the arrow stays symmetric.
    let mid = f64::from(ARROW_W) / 2.0;
    let tip_half = 0.5 - EDGE_IN;
    let half = (base - tip).abs() + tip_half;
    let pts = [
        (mid - tip_half, tip),
        (mid + tip_half, tip),
        (mid + half, base),
        (mid - half, base),
    ]
    .map(|(x, y)| (x * k, y * k));
    let ink = shape::premul(ink, 255);
    shape::draw(ARROW_W, ITEM_H, d, |x, y| {
        shape::scale(ink, shape::polygon(x, y, &pts))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fnv(img: &PartImage) -> u64 {
        let c = &img.atlas;
        [c.width, c.height]
            .iter()
            .flat_map(|n| n.to_le_bytes())
            .chain(c.pixels.iter().flat_map(|p| p.to_le_bytes()))
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    fn part(v: Variant, d: i32, p: Part) -> PartImage {
        parts(v, d).into_iter().find(|(q, _)| *q == p).unwrap().1
    }

    fn frame(img: &PartImage, i: i32, h: i32, d: i32) -> Vec<u32> {
        let w = img.atlas.width as i32;
        (i * h * d..(i + 1) * h * d)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| img.atlas.get(x, y))
            .collect()
    }

    #[test]
    fn header_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for v in Variant::ALL {
            for d in [1, 2] {
                for p in [Part::HeaderItem, Part::HeaderFiller, Part::HeaderSortArrow] {
                    got.push((format!("{v:?} {p:?} {d}"), fnv(&part(v, d, p))));
                }
            }
        }
        crate::pins::check("blue/header", &got);
    }

    #[test]
    fn the_header_parts_are_drawn_at_the_styles_scale() {
        for v in Variant::ALL {
            for (p, w, h) in [
                (Part::HeaderItem, ITEM_W, 3 * ITEM_H),
                (Part::HeaderFiller, 1, ITEM_H),
                (Part::HeaderSortArrow, ARROW_W, 2 * ITEM_H),
            ] {
                let one = part(v, 1, p);
                for d in [1, 2, 3] {
                    let img = part(v, d, p);
                    assert_eq!(
                        (img.atlas.width as i32, img.atlas.height as i32),
                        (w * d, h * d),
                        "{v:?} {p:?} at {d}"
                    );
                    assert_eq!(img.scale, d, "{v:?} {p:?} at {d}");
                    assert_eq!(img.sizing, one.sizing, "{v:?} {p:?} at {d}");
                    assert_eq!(img.content, one.content, "{v:?} {p:?} at {d}");
                }
            }
        }
    }

    #[test]
    fn every_header_state_draws_its_own_frame() {
        for v in Variant::ALL {
            for d in [1, 2] {
                let item = part(v, d, Part::HeaderItem);
                let f: Vec<_> = (0..3).map(|i| frame(&item, i, ITEM_H, d)).collect();
                assert!(f[0] != f[1] && f[1] != f[2] && f[0] != f[2], "{v:?} at {d}");
                let arrow = part(v, d, Part::HeaderSortArrow);
                assert!(
                    frame(&arrow, 0, ITEM_H, d) != frame(&arrow, 1, ITEM_H, d),
                    "{v:?} at {d}"
                );
            }
        }
    }

    #[test]
    fn the_header_face_is_opaque() {
        for v in Variant::ALL {
            for d in [1, 2] {
                for p in [Part::HeaderItem, Part::HeaderFiller] {
                    let img = part(v, d, p);
                    assert!(
                        img.atlas.pixels.iter().all(|&px| px >> 24 == 0xFF),
                        "{v:?} {p:?} at {d}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_sort_arrow_is_solid_at_1_and_smooth_at_2() {
        for v in Variant::ALL {
            let one = part(v, 1, Part::HeaderSortArrow);
            for i in 0..2 {
                let at = |x: i32, y: i32| one.atlas.get(x, i * ITEM_H + y);
                for y in 7..=11 {
                    assert!(
                        (0..ARROW_W).any(|x| at(x, y) >> 24 == 0xFF),
                        "{v:?} row {y}"
                    );
                }
                for y in (0..7).chain(12..ITEM_H) {
                    assert!((0..ARROW_W).all(|x| at(x, y) == 0), "{v:?} row {y}");
                }
                let (tip, base) = if i == 0 { (7, 11) } else { (11, 7) };
                assert_eq!(at(4, tip) >> 24, 0xFF, "{v:?} {i}: the tip");
                assert!(
                    at(2, tip) == 0 && at(6, tip) == 0,
                    "{v:?} {i}: beside the tip"
                );
                assert!(
                    (0..ARROW_W).all(|x| at(x, base) >> 24 == 0xFF),
                    "{v:?} {i}: the base"
                );
            }
            let two = part(v, 2, Part::HeaderSortArrow);
            assert!(
                (0..two.atlas.height as i32)
                    .flat_map(|y| (0..ARROW_W * 2).map(move |x| (x, y)))
                    .any(|(x, y)| (1..0xFF).contains(&(two.atlas.get(x, y) >> 24))),
                "{v:?}: anti-aliased at 2"
            );
        }
    }

    #[test]
    fn the_lit_bands_corner_is_smooth_without_a_dark_fringe() {
        for v in [
            Variant::Blue,
            Variant::Olive,
            Variant::Silver,
            Variant::Steel,
        ] {
            let d = 2;
            let item = part(v, d, Part::HeaderItem);
            let hot = |x: i32, y: i32| item.atlas.get(x, ITEM_H * d + y);
            let face = hot(ITEM_W * d / 2, 5 * d);
            let bottom = (ITEM_H - 1) * d + 1;
            let band = hot(ITEM_W * d / 2, bottom);
            let corner = hot(0, bottom);
            assert!(corner != face && corner != band, "{v:?}: the corner blends");
            let luma = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
            assert!(
                luma(corner) >= luma(band).min(luma(face)),
                "{v:?}: no fringe"
            );
        }
    }
}
