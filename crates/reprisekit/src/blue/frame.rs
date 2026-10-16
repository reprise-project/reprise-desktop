//! Reprise Blue's window frame: the caption band, a maximized window's band, and the side and
//! bottom borders, each drawn from its scheme's [`Face`]s at `d` device pixels per design pixel.

use crate::blue::scheme::{Face, Grad, Scheme};
use crate::raster::{Canvas, Fill, Insets};
use crate::shape::{self, Px, Stops};
use crate::style::{Layout, PartImage, State};
use crate::theme::Color;

/// The caption atlas's stretchable middle, design px; flat along x, so any width works.
const BAND_MIDDLE: i32 = 3;

const SIDE_ROWS: i32 = 4;

const BOTTOM_MIDDLE: i32 = 4;

/// How far (levels) the band's `across` colour one column in from the rim may sit from the face's
/// `inner` colour for that column to count as a light line inside the rim.
const GLINT_MATCH: f64 = 24.0;

fn stops(g: Grad) -> Vec<(f64, Px)> {
    g.iter()
        .map(|&(t, c)| (f64::from(t), shape::premul(c, 255)))
        .collect()
}

fn opaque(c: Color) -> Px {
    shape::premul(c, 255)
}

fn apart(a: Px, b: Px) -> f64 {
    (0..3).map(|i| (a.1[i] - b.1[i]).abs()).fold(0.0, f64::max)
}

struct Band {
    down: Vec<(f64, Px)>,
    across: Vec<(f64, Px)>,
    rim: Px,
    glint: Option<Px>,
    w: i32,
    h: i32,
    left: i32,
    right: i32,
}

impl Band {
    fn new(f: &Face, w: i32, h: i32, left: i32, right: i32) -> Band {
        let across = stops(f.across);
        let inner = opaque(f.inner);
        let one_in = Stops(&across).at(1.5 / f64::from(w));
        Band {
            down: stops(f.down),
            glint: (apart(one_in, inner) <= GLINT_MATCH).then_some(inner),
            across,
            rim: opaque(f.rim),
            w,
            h,
            left,
            right,
        }
    }

    fn body(&self, x: f64, y: f64) -> Px {
        let w = f64::from(self.w);
        let at = |x: f64| Stops(&self.across).at((x + 0.5) / w);
        let mid = at(f64::from(self.left) + f64::from(BAND_MIDDLE) / 2.0 - 0.5);
        let x = x.clamp(0.0, w - 1.0);
        let shift = if x < f64::from(self.left) || x >= w - f64::from(self.right) {
            let c = at(x);
            [0, 1, 2].map(|i| c.1[i] - mid.1[i])
        } else {
            [0.0; 3]
        };
        let row = Stops(&self.down).at((y + 0.5) / f64::from(self.h));
        (
            255.0,
            [0, 1, 2].map(|i| (row.1[i] + shift[i]).clamp(0.0, 255.0)),
        )
    }
}

fn band(s: &Scheme, faces: [&Face; 2], h: i32, radius: Option<f64>, d: i32) -> PartImage {
    let m = s.metrics;
    let w = m.caption_left + BAND_MIDDLE + m.caption_right;
    let mut a = Canvas::new((w * d) as u32, (2 * h * d) as u32);
    for (i, f) in faces.into_iter().enumerate() {
        let b = Band::new(f, w, h, m.caption_left, m.caption_right);
        let frame = match radius {
            None => shape::draw(w, h, d, |x, y| {
                b.body(shape::centre(x as i32, d), shape::centre(y as i32, d))
            }),
            Some(r) => {
                // The box runs past the band's bottom so only its top corners round.
                let tall = h + r.ceil() as i32 + 2;
                let px = shape::ringed(w, tall, r, 2, d, |x, y, ring| match ring {
                    0 if y < 0.5 && x >= r && x <= f64::from(w) - 1.0 - r => b.body(x, y),
                    0 => b.rim,
                    1 => b.glint.unwrap_or_else(|| b.body(x, y)),
                    _ => b.body(x, y),
                });
                shape::draw(w, h, d, |x, y| px[(y as i32 * w * d + x as i32) as usize])
            }
        };
        a.blit_over(0, i as i32 * h * d, &frame);
    }
    PartImage::new(a, &[State::Normal, State::Inactive], Layout::Vertical)
        .sizing(Insets::new(m.caption_left, 0, m.caption_right, 0))
        .at_scale(d)
}

/// Sizing margins along an axis `n` px long that keep a part whole at `n` px and leave out its
/// pixel `drop` at `n - 1`: the nine-grid shrinks them to `drop` and `n - 1 - drop`.
fn dropping(drop: usize, n: i32) -> (i32, i32) {
    (drop as i32 + 1, n - 1 - drop as i32)
}

/// The caption band of a restored window, 30 rows at `d`: its top corners rounded over the scheme's
/// caption radius, a 1 px rim, the band's rows shaded darker (or lined lighter) toward both ends.
/// A dialog's 29-row caption leaves out the row the scheme's dialog drop names.
pub fn caption(s: &Scheme, d: i32) -> PartImage {
    let m = s.metrics;
    let h = m.caption_height;
    let (top, bottom) = dropping(m.dialog_drop.caption, h);
    band(
        s,
        [&s.caption.normal, &s.caption.inactive],
        h,
        Some(f64::from(m.radii.caption)),
        d,
    )
    .sizing_at_height(
        m.dialog_caption_height,
        Insets::new(m.caption_left, top, m.caption_right, bottom),
    )
}

/// A maximized window's caption band, 26 rows at `d`, square to the screen's edges: the band's rows
/// and shaded ends, no rim.
pub fn max_caption(s: &Scheme, d: i32) -> PartImage {
    band(
        s,
        [&s.max_caption.normal, &s.max_caption.inactive],
        s.metrics.max_caption_height,
        None,
        d,
    )
}

fn side_column(f: &Face, col: i32, n: i32, inward: bool) -> Px {
    let i = if inward { col } else { n - 1 - col };
    Stops(&stops(f.across)).at((f64::from(i) + 0.5) / f64::from(n))
}

/// A dialog's 3 px border leaves out column `drop`.
fn side(faces: [&Face; 2], n: i32, inward: bool, drop: usize, d: i32) -> PartImage {
    let mut a = Canvas::new((2 * n * d) as u32, (SIDE_ROWS * d) as u32);
    for (i, f) in faces.into_iter().enumerate() {
        let frame = shape::draw(n, SIDE_ROWS, d, |x, _| {
            let c = (x as i32) / d;
            side_column(f, if inward { c } else { n - 1 - c }, n, inward)
        });
        a.blit_over(i as i32 * n * d, 0, &frame);
    }
    let (left, right) = dropping(drop, n);
    PartImage::new(a, &[State::Normal, State::Inactive], Layout::Horizontal)
        .sizing(Insets::new(left, 0, right, 0))
        .fill(Fill::StretchCentred)
        .at_scale(d)
}

/// The window's left border, 4 px at `d`: flat columns, the dark rim outermost.
pub fn frame_left(s: &Scheme, d: i32) -> PartImage {
    let f = &s.frame_left;
    let m = s.metrics;
    side(
        [&f.normal, &f.inactive],
        m.frame,
        true,
        m.dialog_drop.left,
        d,
    )
}

pub fn frame_right(s: &Scheme, d: i32) -> PartImage {
    let f = &s.frame_right;
    let m = s.metrics;
    side(
        [&f.normal, &f.inactive],
        m.frame,
        false,
        m.dialog_drop.right,
        d,
    )
}

/// The window's bottom border, 4 rows at `d`: flat rows of the face's `down` colours between two
/// corner blocks where the side borders' columns turn into the bottom rows, mitred on the diagonal
/// from the outer corner. A dialog's 3-row border leaves out the dialog drop's row, corners and all.
pub fn frame_bottom(s: &Scheme, d: i32) -> PartImage {
    let m = s.metrics;
    let (n, corner) = (m.frame, m.bottom_corner);
    let w = corner + BOTTOM_MIDDLE + corner;
    let mut a = Canvas::new((w * d) as u32, (2 * n * d) as u32);
    let states = [
        (
            &s.frame_bottom.normal,
            &s.frame_left.normal,
            &s.frame_right.normal,
        ),
        (
            &s.frame_bottom.inactive,
            &s.frame_left.inactive,
            &s.frame_right.inactive,
        ),
    ];
    for (i, (bottom, left, right)) in states.into_iter().enumerate() {
        let rows = stops(bottom.down);
        let frame = shape::draw(w, n, d, |x, y| {
            let k = f64::from(d);
            let (cx, cy) = ((x + 0.5) / k, (y + 0.5) / k);
            let to_bottom = f64::from(n) - cy;
            let row = (y as i32) / d;
            let flat = Stops(&rows).at((f64::from(row) + 0.5) / f64::from(n));
            let (to_side, face, inward) = if cx < f64::from(w) / 2.0 {
                (cx, left, true)
            } else {
                (f64::from(w) - cx, right, false)
            };
            if to_side < to_bottom && to_side < f64::from(n) {
                side_column(face, to_side as i32, n, inward)
            } else {
                flat
            }
        });
        a.blit_over(0, i as i32 * n * d, &frame);
    }
    let (top, bottom) = dropping(m.dialog_drop.bottom, n);
    PartImage::new(a, &[State::Normal, State::Inactive], Layout::Vertical)
        .sizing(Insets::new(corner, top, corner, bottom))
        .fill(Fill::StretchCentred)
        .at_scale(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;
    use crate::raster::Rect;

    type Painter = fn(&Scheme, i32) -> PartImage;

    const PARTS: [(&str, Painter); 5] = [
        ("caption", caption),
        ("max_caption", max_caption),
        ("frame_left", frame_left),
        ("frame_right", frame_right),
        ("frame_bottom", frame_bottom),
    ];

    fn fnv(c: &Canvas) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        for w in words {
            for b in w.to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        h
    }

    fn frame_of(img: &PartImage, state: State) -> Canvas {
        let f = img.frame(state);
        let mut c = Canvas::new(f.width as u32, f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                c.put_raw(x, y, img.atlas.get(f.x + x, f.y + y));
            }
        }
        c
    }

    fn alpha(px: u32) -> u32 {
        px >> 24
    }

    /// On a deliberate change, the failure prints the whole table to paste back.
    #[test]
    fn frame_parts_pin_at_scale_1_and_2() {
        let want: &[(&str, &str, i32, u64)] = &[
            ("blue", "caption", 1, 0xd4dcfa1570cb3543),
            ("blue", "caption", 2, 0x0e084ab9b357b1e4),
            ("blue", "max_caption", 1, 0xd4ea4229575491c0),
            ("blue", "max_caption", 2, 0x1b295761f23a9119),
            ("blue", "frame_left", 1, 0x38a74b4987af6cc1),
            ("blue", "frame_left", 2, 0x95c0d8e10fd78b3d),
            ("blue", "frame_right", 1, 0x5076572ecf057529),
            ("blue", "frame_right", 2, 0x4061235fa3f916fd),
            ("blue", "frame_bottom", 1, 0x4f2dfd34770c31a6),
            ("blue", "frame_bottom", 2, 0x2984f5cd2d4cbb8a),
            ("olive", "caption", 1, 0x18b158b9c93717cf),
            ("olive", "caption", 2, 0xc96226d20a1479b6),
            ("olive", "max_caption", 1, 0xc962eaafee73afe0),
            ("olive", "max_caption", 2, 0xdb6a1b8b9ac8b409),
            ("olive", "frame_left", 1, 0x8b4db6329c4bcb09),
            ("olive", "frame_left", 2, 0x507068cd516588fd),
            ("olive", "frame_right", 1, 0x9282350f3d7bf501),
            ("olive", "frame_right", 2, 0xe03937196d5e77bd),
            ("olive", "frame_bottom", 1, 0xf1fc5ac65be7c682),
            ("olive", "frame_bottom", 2, 0xafbc7fb3ebe387f2),
            ("silver", "caption", 1, 0x1d6f0c41f7c64281),
            ("silver", "caption", 2, 0x5ac9afa5f9960e5c),
            ("silver", "max_caption", 1, 0xdf8799e9915203a1),
            ("silver", "max_caption", 2, 0xa11ff71a1f5e6b32),
            ("silver", "frame_left", 1, 0x744d2a211ce967c9),
            ("silver", "frame_left", 2, 0xa741fde70319cbfd),
            ("silver", "frame_right", 1, 0x5254ac02b9b8e0a1),
            ("silver", "frame_right", 2, 0xec8f0d1d74b31e7d),
            ("silver", "frame_bottom", 1, 0x7c6204868e7c094f),
            ("silver", "frame_bottom", 2, 0xb3f4efdaa1e8f8e1),
            ("slate", "caption", 1, 0xc63b1d5ef4e1cf63),
            ("slate", "caption", 2, 0xbe1ccd3f783ccde7),
            ("slate", "max_caption", 1, 0x7ed59327ef582a8c),
            ("slate", "max_caption", 2, 0x092d4bf744bbe813),
            ("slate", "frame_left", 1, 0x2c77e5c1e7ef4081),
            ("slate", "frame_left", 2, 0x73b68d68a2d4bd5d),
            ("slate", "frame_right", 1, 0x65cf4ce5b5d5d989),
            ("slate", "frame_right", 2, 0xd76678f02b8d94fd),
            ("slate", "frame_bottom", 1, 0x75305a4f1713fe51),
            ("slate", "frame_bottom", 2, 0x2b8d278055ac7916),
            ("sapphire", "caption", 1, 0x5d380dcbe070a24a),
            ("sapphire", "caption", 2, 0xa8cc2e56ab1fad38),
            ("sapphire", "max_caption", 1, 0xd7b5a34f8c6a2a92),
            ("sapphire", "max_caption", 2, 0xa2614cda77e763e7),
            ("sapphire", "frame_left", 1, 0x770caa0c200dd3b1),
            ("sapphire", "frame_left", 2, 0x954ddcb668c6477d),
            ("sapphire", "frame_right", 1, 0xfc1bef8eec6b1e91),
            ("sapphire", "frame_right", 2, 0x4b84d15b289d22fd),
            ("sapphire", "frame_bottom", 1, 0x5b8e8b8a382f13db),
            ("sapphire", "frame_bottom", 2, 0x4835f1f215f8f949),
            ("obsidian", "caption", 1, 0x56aba6e6911a0a0b),
            ("obsidian", "caption", 2, 0x844c44e94c1105d6),
            ("obsidian", "max_caption", 1, 0xe0e5a200df367c1f),
            ("obsidian", "max_caption", 2, 0x04082174b0b275b7),
            ("obsidian", "frame_left", 1, 0x2005abea25b7f589),
            ("obsidian", "frame_left", 2, 0x44306f07fcc6d0bd),
            ("obsidian", "frame_right", 1, 0x12212d2f113d2ba9),
            ("obsidian", "frame_right", 2, 0x6c3530c949c7473d),
            ("obsidian", "frame_bottom", 1, 0x905dc56843ecc663),
            ("obsidian", "frame_bottom", 2, 0x0887698f26be7ae9),
            ("ember", "caption", 1, 0xe4af3d6326a3b01a),
            ("ember", "caption", 2, 0x9424d76416bef3bb),
            ("ember", "max_caption", 1, 0x07119f9f443fdd5a),
            ("ember", "max_caption", 2, 0xdca316f935d8a733),
            ("ember", "frame_left", 1, 0x8e71a9b24f811039),
            ("ember", "frame_left", 2, 0xf9780364ba0eff5d),
            ("ember", "frame_right", 1, 0x3311d1fd2048dd09),
            ("ember", "frame_right", 2, 0xeab3d4e91b0af19d),
            ("ember", "frame_bottom", 1, 0xd8b40266d60f5add),
            ("ember", "frame_bottom", 2, 0xb0c0e50169bd2652),
            ("cobalt", "caption", 1, 0x82ad54589f78fe2a),
            ("cobalt", "caption", 2, 0xbf211758f88ad6f4),
            ("cobalt", "max_caption", 1, 0x5ced566e153f0204),
            ("cobalt", "max_caption", 2, 0x8974f40b53950b77),
            ("cobalt", "frame_left", 1, 0x53d68b63034151b1),
            ("cobalt", "frame_left", 2, 0xf4ef41a0b84d53fd),
            ("cobalt", "frame_right", 1, 0xd65ac57c1b4e76c1),
            ("cobalt", "frame_right", 2, 0x07aa17340bfe99fd),
            ("cobalt", "frame_bottom", 1, 0x42965d50f8bde4e3),
            ("cobalt", "frame_bottom", 2, 0x1a9edbb40ca80c69),
            ("carbon", "caption", 1, 0x45e9ed14663e454e),
            ("carbon", "caption", 2, 0x85d51544b3c6da95),
            ("carbon", "max_caption", 1, 0x55b2f7b99957972f),
            ("carbon", "max_caption", 2, 0xbfb1ec529f44eb57),
            ("carbon", "frame_left", 1, 0xeea8121971bd2eb1),
            ("carbon", "frame_left", 2, 0x9f111e0a4bd28efd),
            ("carbon", "frame_right", 1, 0xdab6db70b0ecd2d1),
            ("carbon", "frame_right", 2, 0xbb31f44e5c55a4fd),
            ("carbon", "frame_bottom", 1, 0xb7187e63b5fd10cb),
            ("carbon", "frame_bottom", 2, 0x371299f780739049),
            ("steel", "caption", 1, 0x1dc7ff23eaf668ec),
            ("steel", "caption", 2, 0x0bd6226b7db2cf1b),
            ("steel", "max_caption", 1, 0x8d16ddc7763e4b26),
            ("steel", "max_caption", 2, 0x9978db49fe188bfa),
            ("steel", "frame_left", 1, 0x0168cfa6e410ee69),
            ("steel", "frame_left", 2, 0xdc66a9df7c29a2dd),
            ("steel", "frame_right", 1, 0xfa748dfad6a21609),
            ("steel", "frame_right", 2, 0x83fbd3f75156dabd),
            ("steel", "frame_bottom", 1, 0xafc3bd5fe7459ea4),
            ("steel", "frame_bottom", 2, 0xd73b99bafabb0167),
        ];
        let mut got = Vec::new();
        for v in Variant::ALL {
            for (name, paint) in PARTS {
                for d in [1, 2] {
                    got.push((v.as_str(), name, d, fnv(&paint(v.scheme(), d).atlas)));
                }
            }
        }
        let table: String = got
            .iter()
            .map(|(v, p, d, h)| format!("            ({v:?}, {p:?}, {d}, 0x{h:016x}),\n"))
            .collect();
        assert!(got == want, "frame part hashes changed:\n{table}");
    }

    #[test]
    fn each_part_is_twice_the_size_at_2_with_the_same_insets_and_distinct_states() {
        for v in Variant::ALL {
            let s = v.scheme();
            for (name, paint) in PARTS {
                let (one, two) = (paint(s, 1), paint(s, 2));
                assert_eq!(
                    (two.atlas.width, two.atlas.height),
                    (2 * one.atlas.width, 2 * one.atlas.height),
                    "{v:?} {name}"
                );
                assert_eq!(one.sizing, two.sizing, "{v:?} {name} sizing");
                assert_eq!(
                    one.sizing_at_height, two.sizing_at_height,
                    "{v:?} {name} dialog sizing"
                );
                assert_eq!((one.scale, two.scale), (1, 2), "{v:?} {name}");
                for img in [&one, &two] {
                    assert_ne!(
                        frame_of(img, State::Normal).pixels,
                        frame_of(img, State::Inactive).pixels,
                        "{v:?} {name}: focused and unfocused differ"
                    );
                }
            }
        }
    }

    /// The caption's top corners are clear outside the curve, opaque inside it, and at 2 the curve
    /// is anti-aliased: partial alpha on it, and no edge pixel darker than both the rim and every
    /// opaque pixel around it (a rim counted twice would be).
    #[test]
    fn caption_corners_are_round_and_smooth_at_2() {
        let lit = |p: u32| {
            let a = f64::from(alpha(p));
            [16, 8, 0].map(|sh| f64::from((p >> sh) & 0xFF) * 255.0 / a)
        };
        for v in Variant::ALL {
            let s = v.scheme();
            let rim = s.caption.normal.rim;
            let rim = [rim.r, rim.g, rim.b].map(f64::from);
            for d in [1, 2] {
                let c = frame_of(&caption(s, d), State::Normal);
                let (w, h) = (c.width as i32, c.height as i32);
                for (x, y) in [(0, 0), (w - 1, 0)] {
                    assert_eq!(alpha(c.get(x, y)), 0, "{v:?} d={d} corner ({x}, {y})");
                }
                let r = (f64::from(s.metrics.radii.caption) * f64::from(d)) as i32;
                for (x, y) in [(r, 0), (0, r), (w / 2, 0), (0, h - 1)] {
                    assert_eq!(alpha(c.get(x, y)), 255, "{v:?} d={d} ({x}, {y}) is ink");
                }
                let partial: Vec<(i32, i32)> = (0..r)
                    .flat_map(|y| (0..r).map(move |x| (x, y)))
                    .filter(|&(x, y)| (1..255).contains(&alpha(c.get(x, y))))
                    .collect();
                if d == 2 {
                    assert!(partial.len() >= 3, "{v:?}: the curve is smooth at 2");
                }
                for (x, y) in partial {
                    let px = c.get(x, y);
                    let own = lit(px);
                    let slack = 255.0 / f64::from(alpha(px)) + 1.0;
                    let near: Vec<[f64; 3]> = (-2..=2)
                        .flat_map(|dy| (-2..=2).map(move |dx| (x + dx, y + dy)))
                        .filter(|&(x, y)| x >= 0 && y >= 0 && x < w && y < h)
                        .map(|(x, y)| c.get(x, y))
                        .filter(|&p| alpha(p) == 255)
                        .map(lit)
                        .collect();
                    for i in 0..3 {
                        let darkest = near.iter().map(|n| n[i]).fold(rim[i], f64::min);
                        assert!(
                            own[i] + slack >= darkest,
                            "{v:?} d={d} ({x}, {y}): no dark fringe"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_band_middle_is_flat_so_any_width_stretches_it() {
        use crate::look::styled::fixture::one_part_style;
        use crate::style::Part;
        for v in Variant::ALL {
            let s = v.scheme();
            let m = s.metrics;
            for (part, img, h) in [
                (Part::Caption, caption(s, 1), m.caption_height),
                (Part::MaxCaption, max_caption(s, 1), m.max_caption_height),
            ] {
                let st = one_part_style(part, img);
                for w in [120, 400] {
                    let mut c = Canvas::new(w as u32, h as u32);
                    st.draw(&mut c, Rect::new(0, 0, w, h), part, State::Normal, 1);
                    for y in 0..h {
                        let mid = c.get(m.caption_left, y);
                        for x in m.caption_left..w - m.caption_right {
                            assert_eq!(c.get(x, y), mid, "{v:?} {part:?} {w} px ({x}, {y})");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn borders_are_whole_columns_and_rows_mitred_at_the_corners() {
        for v in Variant::ALL {
            let s = v.scheme();
            let n = s.metrics.frame;
            for paint in [frame_left as Painter, frame_right] {
                let c = frame_of(&paint(s, 2), State::Normal);
                for x in 0..n {
                    assert_eq!(c.get(2 * x, 0), c.get(2 * x + 1, 5), "{v:?} column {x}");
                }
            }
            let left = frame_of(&frame_left(s, 1), State::Normal);
            let right = frame_of(&frame_right(s, 1), State::Normal);
            let b = frame_of(&frame_bottom(s, 1), State::Normal);
            let w = b.width as i32;
            let mid = w / 2;
            assert_eq!(b.get(0, 0), left.get(0, 0), "{v:?} left outer column");
            assert_eq!(
                b.get(w - 1, 0),
                right.get(n - 1, 0),
                "{v:?} right outer column"
            );
            assert_eq!(b.get(0, n - 1), b.get(mid, n - 1), "{v:?} left outer row");
            assert_eq!(
                b.get(w - 1, n - 1),
                b.get(mid, n - 1),
                "{v:?} right outer row"
            );
        }
    }
}
