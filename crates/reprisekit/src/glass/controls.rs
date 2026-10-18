//! Glass's check box and radio button, drawn from shapes at the style's scale.
//!
//! The check box is a flat 13x13 square: a 1 px border, a 1 px pale ring inside it and a 9x9 well with
//! a 1 px bevel (darker along the top and left) round a fill that lightens from the top-left to the
//! bottom-right. Hot tints it all pale blue, held a deeper blue; greyed, the border pales and the inside
//! is one soft grey ramp. The tick is a 2 px dark blue line over a white halo; the mixed mark fills the
//! well with a blue square, bevelled twice and shaded darker towards the bottom-right (held, the shading turns over).
//!
//! The radio is a 12 px disc with anti-aliased edges: a 1 px rim, a 1 px pale ring and a well shaded
//! like the check box's. Checked, a blue dot 6.6 px across sits in the well: a dark rim round a blue
//! body with a pale gloss at its top-left.

use crate::raster::Canvas;
use crate::shape::{CLEAR, Px, Stops, centre, draw, ellipse, over, premul, scale, shares, stroke};
use crate::style::{Check, Layout, Part, PartImage};
use crate::theme::Color;

const BOX: i32 = 13;
const DISC: i32 = 12;

/// A square of `n` design px with a 1 px bevel round its fill: the top and left edges run from `tl[0]`
/// at the top-left corner to `tl[1]` at their far ends; the bottom and right edges from `br[0]` to
/// `br[1]` at the bottom-right corner. `None` leaves the fill reaching the edge.
struct Bevel {
    edges: Option<([Color; 2], [Color; 2])>,
    fill: Fill,
}

enum Fill {
    Ramp([Color; 2]),
    Inset(&'static Bevel),
}

const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// One check box state: border, pale ring (`None`: the well reaches the border), the well, and the tick's ink and halo alpha.
struct BoxInks {
    border: Color,
    ring: Option<Color>,
    well: Bevel,
    tick: Color,
    halo: u8,
    mark: &'static Bevel,
}

const BOX_NORMAL: BoxInks = BoxInks {
    border: rgb(0x8B8C8C),
    ring: Some(rgb(0xF3F4F4)),
    well: Bevel {
        edges: Some((
            [rgb(0xACB0B7), rgb(0xC4C6C8)],
            [rgb(0xBFC1C3), rgb(0xE8E7E9)],
        )),
        fill: Fill::Ramp([rgb(0xC9CDD4), rgb(0xF5F6F6)]),
    },
    tick: rgb(0x445590),
    halo: 0xD0,
    mark: &MARK_NORMAL,
};

const BOX_HOT: BoxInks = BoxInks {
    border: rgb(0x537FA0),
    ring: Some(rgb(0xDDF7FA)),
    well: Bevel {
        edges: Some((
            [rgb(0x75BEF9), rgb(0x86CAF9)],
            [rgb(0x7FC6F9), rgb(0xCDE9FD)],
        )),
        fill: Fill::Ramp([rgb(0xAEDAFD), rgb(0xE6F6FE)]),
    },
    tick: rgb(0x041C6E),
    halo: 0xF0,
    mark: &MARK_HOT,
};

const BOX_PRESSED: BoxInks = BoxInks {
    border: rgb(0x2B5B87),
    ring: Some(rgb(0xC0E0F6)),
    well: Bevel {
        edges: Some((
            [rgb(0x5AACF7), rgb(0x6CBAF7)],
            [rgb(0x65B7F7), rgb(0xBFE2FC)],
        )),
        fill: Fill::Ramp([rgb(0x9ACFFC), rgb(0xE0F2FE)]),
    },
    tick: rgb(0x2D4E92),
    halo: 0x90,
    mark: &MARK_PRESSED,
};

const BOX_DISABLED: BoxInks = BoxInks {
    border: rgb(0xAEAFAF),
    ring: None,
    well: Bevel {
        edges: None,
        fill: Fill::Ramp([rgb(0xEFEEEF), rgb(0xFAFBFB)]),
    },
    tick: rgb(0xBBBABB),
    halo: 0x60,
    mark: &MARK_DISABLED,
};

const MARK_NORMAL: Bevel = Bevel {
    edges: Some((
        [rgb(0x285B89), rgb(0x214A71)],
        [rgb(0x1D4161), rgb(0x183652)],
    )),
    fill: Fill::Inset(&Bevel {
        edges: Some((
            [rgb(0x7BC1E3), rgb(0x538FBB)],
            [rgb(0x4E8AB8), rgb(0x245288)],
        )),
        fill: Fill::Ramp([rgb(0x2B9ED4), rgb(0x1D618B)]),
    }),
};

const MARK_HOT: Bevel = Bevel {
    edges: MARK_NORMAL.edges,
    fill: Fill::Inset(&Bevel {
        edges: Some((
            [rgb(0x7DDCF1), rgb(0x54A8D2)],
            [rgb(0x51A6D1), rgb(0x2568AC)],
        )),
        fill: Fill::Ramp([rgb(0x2ECDED), rgb(0x1F88CA)]),
    }),
};

const MARK_PRESSED: Bevel = Bevel {
    edges: Some((
        [rgb(0x183652), rgb(0x224A71)],
        [rgb(0x234F75), rgb(0x285B89)],
    )),
    fill: Fill::Inset(&Bevel {
        edges: Some((
            [rgb(0x163D77), rgb(0x3671A2)],
            [rgb(0x3A7CB1), rgb(0x2A86C3)],
        )),
        fill: Fill::Ramp([rgb(0x135075), rgb(0x2080BF)]),
    }),
};

const MARK_DISABLED: Bevel = Bevel {
    edges: Some((
        [rgb(0xB7B8B8), rgb(0xB0B1B1)],
        [rgb(0xB2B3B3), rgb(0xA9AAAA)],
    )),
    fill: Fill::Inset(&Bevel {
        edges: Some((
            [rgb(0xDFE0E0), rgb(0xCACBCB)],
            [rgb(0xC9CACA), rgb(0xB4B5B5)],
        )),
        fill: Fill::Ramp([rgb(0xCDCCCD), rgb(0xB7B8B8)]),
    }),
};

/// The tick's centre line in design px (pixel edges): a short arm down to the bend, a long one up to the right.
const TICK: [(f64, f64); 3] = [(3.0, 7.2), (5.6, 9.6), (9.7, 2.7)];
const TICK_WIDTH: f64 = 1.9;
const HALO_WIDTH: f64 = 3.6;

/// The colour [`Bevel`] `b`, `n` design px wide from design px `at` both ways, gives the design pixel
/// `(cx, cy)`, shaded at its middle `(u, v)` (design px, [`centre`]).
fn bevel(b: &Bevel, at: i32, n: i32, (cx, cy): (i32, i32), (u, v): (f64, f64)) -> Px {
    let last = at + n - 1;
    let along = |p: f64| ((p - f64::from(at)) / f64::from(n - 1)).clamp(0.0, 1.0);
    if let Some((tl, br)) = &b.edges {
        if cy == at || cx == at {
            let t = if cy == at { along(u) } else { along(v) };
            return ramp(tl, t);
        }
        if cy == last || cx == last {
            let t = if cy == last { along(u) } else { along(v) };
            return ramp(br, t);
        }
    }
    let (at, n) = if b.edges.is_some() {
        (at + 1, n - 2)
    } else {
        (at, n)
    };
    match &b.fill {
        Fill::Ramp(c) => {
            let span = 2.0 * f64::from(n - 1);
            let t = (u - f64::from(at) + v - f64::from(at)) / span;
            ramp(c, t)
        }
        Fill::Inset(inner) => bevel(inner, at, n, (cx, cy), (u, v)),
    }
}

fn ramp(c: &[Color; 2], t: f64) -> Px {
    Stops(&[(0.0, premul(c[0], 255)), (1.0, premul(c[1], 255))]).at(t)
}

fn check_box(ink: &BoxInks, check: Check, d: i32) -> Canvas {
    let k = f64::from(d);
    let tick: Vec<(f64, f64)> = TICK.iter().map(|&(x, y)| (x * k, y * k)).collect();
    let tick_ink = premul(ink.tick, 255);
    let halo = premul(Color::rgb(0xFF, 0xFF, 0xFF), ink.halo);
    draw(BOX, BOX, d, |x, y| {
        let cell = ((x as i32) / d, (y as i32) / d);
        let mid = (centre(x as i32, d), centre(y as i32, d));
        let edge = |c: i32| c == 0 || c == BOX - 1;
        let ringed = |c: i32| c == 1 || c == BOX - 2;
        let base = if edge(cell.0) || edge(cell.1) {
            premul(ink.border, 255)
        } else if let (Some(ring), true) = (ink.ring, ringed(cell.0) || ringed(cell.1)) {
            premul(ring, 255)
        } else if check == Check::Mixed && !ringed(cell.0) && !ringed(cell.1) {
            bevel(ink.mark, 2, BOX - 4, cell, mid)
        } else if ink.ring.is_some() {
            bevel(&ink.well, 2, BOX - 4, cell, mid)
        } else {
            bevel(&ink.well, 1, BOX - 2, cell, mid)
        };
        if check != Check::Checked {
            return base;
        }
        let inside = cell.0 > 0 && cell.1 > 0 && cell.0 < BOX - 1 && cell.1 < BOX - 1;
        let lit = if inside {
            over(scale(halo, stroke(x, y, &tick, HALO_WIDTH * k)), base)
        } else {
            base
        };
        over(scale(tick_ink, stroke(x, y, &tick, TICK_WIDTH * k)), lit)
    })
}

struct RadioInks {
    rim: Color,
    ring: Color,
    well: [Color; 2],
    dot: Dot,
}

struct Dot {
    rim: Color,
    body: [Color; 2],
    gloss: Color,
}

const RADIO_NORMAL: RadioInks = RadioInks {
    rim: rgb(0x8E8F90),
    ring: rgb(0xF2F1F2),
    well: [rgb(0xB1B5BB), rgb(0xF4F5F5)],
    dot: Dot {
        rim: rgb(0x19364D),
        body: [rgb(0x178CCC), rgb(0x0C74BF)],
        gloss: rgb(0xC5E7FD),
    },
};

const RADIO_HOT: RadioInks = RadioInks {
    rim: rgb(0x5783A2),
    ring: rgb(0xDAF4F8),
    well: [rgb(0x7CC2F9), rgb(0xE4F4FE)],
    dot: Dot {
        rim: rgb(0x162F46),
        body: [rgb(0x19B1FF), rgb(0x0D92F0)],
        gloss: rgb(0xE8F6FE),
    },
};

const RADIO_PRESSED: RadioInks = RadioInks {
    rim: rgb(0x345E81),
    ring: rgb(0xC8EEF4),
    well: [rgb(0x56A9F6), rgb(0xD8EFFD)],
    dot: Dot {
        rim: rgb(0x142E46),
        body: [rgb(0x0F5689), rgb(0x094E80)],
        gloss: rgb(0x92D2FC),
    },
};

const RADIO_DISABLED: RadioInks = RadioInks {
    rim: rgb(0xCBCCCC),
    ring: rgb(0xDDDCDD),
    well: [rgb(0xD8D9D9), rgb(0xF7F8F8)],
    dot: Dot {
        rim: rgb(0x949595),
        body: [rgb(0xB6B7B7), rgb(0xB1B2B2)],
        gloss: rgb(0xEFEEEF),
    },
};

/// The disc's radii in design px: the rim's outside, the pale ring's outside, the well's.
const RIM: f64 = 6.0;
const RING: f64 = 5.0;
const WELL: f64 = 4.0;
const DOT: f64 = 3.3;
const DOT_BODY: f64 = 2.3;
const GLOSS: (f64, f64, f64, f64) = (5.3, 4.9, 1.4, 1.0);

fn radio(ink: &RadioInks, checked: bool, d: i32) -> Canvas {
    let k = f64::from(d);
    let c = f64::from(DISC) / 2.0 * k;
    let disc = |x: f64, y: f64, r: f64| ellipse(x, y, c, c, r * k, r * k);
    let span = 2.0 * WELL * std::f64::consts::SQRT_2;
    draw(DISC, DISC, d, |x, y| {
        let (rim, ring, well) = (disc(x, y, RIM), disc(x, y, RING), disc(x, y, WELL));
        if rim == 0.0 {
            return CLEAR;
        }
        let (u, v) = (centre(x as i32, d) + 0.5, centre(y as i32, d) + 0.5);
        let t = 0.5 + (u + v - f64::from(DISC)) / span;
        let base = shares(&[
            (premul(ink.rim, 255), rim - ring),
            (premul(ink.ring, 255), ring - well),
            (ramp(&ink.well, t), well),
        ]);
        if !checked {
            return base;
        }
        let (outer, body) = (disc(x, y, DOT), disc(x, y, DOT_BODY));
        if outer == 0.0 {
            return base;
        }
        let dot = &ink.dot;
        let middle = Stops(&[
            (0.0, premul(dot.body[0], 255)),
            (1.0, premul(dot.body[1], 255)),
        ]);
        let from = ((u - 6.0).powi(2) + (v - 6.6).powi(2)).sqrt() / DOT_BODY;
        let (gx, gy, rx, ry) = GLOSS;
        let gloss = ellipse(x, y, gx * k, gy * k, rx * k, ry * k);
        let lit = over(scale(premul(dot.gloss, 0xE0), gloss), middle.at(from));
        over(
            shares(&[(premul(dot.rim, 255), outer - body), (lit, body)]),
            base,
        )
    })
}

fn column(frames: &[Canvas]) -> Canvas {
    let (w, h) = (frames[0].width, frames[0].height);
    let mut a = Canvas::new(w, h * frames.len() as u32);
    for (i, f) in frames.iter().enumerate() {
        a.blit_over(0, (i as u32 * h) as i32, f);
    }
    a
}

/// The check box and radio parts painted at `d`, the same in every glass colour.
pub fn parts(d: i32) -> [(Part, PartImage); 2] {
    let states = [&BOX_NORMAL, &BOX_HOT, &BOX_PRESSED, &BOX_DISABLED];
    let boxes: Vec<Canvas> = [Check::Unchecked, Check::Checked, Check::Mixed]
        .into_iter()
        .flat_map(|c| states.map(|ink| check_box(ink, c, d)))
        .collect();
    let check = PartImage::new_check(
        column(&boxes),
        &[Check::Unchecked, Check::Checked, Check::Mixed],
        Layout::Vertical,
    )
    .at_scale(d);
    let states = [&RADIO_NORMAL, &RADIO_HOT, &RADIO_PRESSED, &RADIO_DISABLED];
    let discs: Vec<Canvas> = [false, true]
        .into_iter()
        .flat_map(|on| states.map(|ink| radio(ink, on, d)))
        .collect();
    let mut radio = PartImage::new_check(
        column(&discs),
        &[Check::Unchecked, Check::Checked],
        Layout::Vertical,
    )
    .at_scale(d);
    // The 12 px disc keeps its size, centred in whatever rect a form gives it.
    radio.true_size = true;
    [(Part::CheckBox, check), (Part::Radio, radio)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classic::widgets::CheckState;
    use crate::glass::{Mode, Settings};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{
        draw_checkbox_state, draw_checkbox_state_disabled, draw_radio, draw_radio_disabled,
    };
    use crate::pointer::Interaction;
    use crate::raster::Rect;
    use crate::style::State;

    const FACE: u32 = 0xEFEEEF;

    /// A 17x17 canvas of dialog face with `draw` into the 13x13 rect at (2, 2), under Glass.
    fn glass(draw: impl Fn(&mut Canvas, Rect)) -> Canvas {
        let mut c = Canvas::new(17, 17);
        c.fill_rect(c.bounds(), crate::theme::Color::rgb(0xEF, 0xEE, 0xEF));
        draw(&mut c, Rect::new(2, 2, 13, 13));
        c
    }

    #[track_caller]
    fn pin(c: &Canvas, dx: i32, dy: i32, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(dx + x, dy + y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    fn fnv(c: &Canvas) -> u64 {
        [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    fn lum(px: u32) -> u32 {
        ((px >> 16) & 0xFF) + ((px >> 8) & 0xFF) + (px & 0xFF)
    }

    fn frames(img: &PartImage) -> Vec<Canvas> {
        let n = img.checks.len() as u32;
        let (w, h) = (img.atlas.width, img.atlas.height / n);
        (0..n)
            .map(|i| {
                let mut f = Canvas::new(w, h);
                for y in 0..h as i32 {
                    for x in 0..w as i32 {
                        f.put_raw(x, y, img.atlas.get(x, i as i32 * h as i32 + y));
                    }
                }
                f
            })
            .collect()
    }

    #[test]
    fn check_and_radio_pin_at_scale_1_and_2() {
        let smoked = Settings {
            material: crate::glass::Material::Smoked,
            ..Settings::default()
        };
        let dark = Settings {
            mode: Mode::Dark,
            ..Settings::default()
        };
        let mut got = Vec::new();
        for (name, s) in [("sky", Settings::default()), ("dark", dark)] {
            for d in [1, 2] {
                let st = crate::glass::style(&s).at_scale(d);
                for part in [Part::CheckBox, Part::Radio] {
                    let h = fnv(&st.part(part).unwrap().atlas);
                    got.push(format!("{name} {d} {part:?} {h:016x}"));
                }
            }
        }
        for d in [1, 2] {
            let (a, b) = (
                crate::glass::style(&Settings::default()).at_scale(d),
                crate::glass::style(&smoked).at_scale(d),
            );
            for part in [Part::CheckBox, Part::Radio] {
                assert_eq!(
                    a.part(part).unwrap().atlas.pixels,
                    b.part(part).unwrap().atlas.pixels,
                    "{part:?} is the same over Smoked glass"
                );
            }
        }
        let want = [
            "sky 1 CheckBox 3b761026d19c0571",
            "sky 1 Radio e22c2e33a45ede55",
            "sky 2 CheckBox cbd291580c30c60e",
            "sky 2 Radio eb6dac30b0b1348f",
            "dark 1 CheckBox 300e69464d5c074c",
            "dark 1 Radio 0762bb185f0a58e3",
            "dark 2 CheckBox 0ffbb2f32e0351dc",
            "dark 2 Radio 897ff765f900ce3b",
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn every_check_and_radio_frame_has_its_size_and_differs_from_the_rest() {
        for d in [1, 2] {
            let [(_, check), (_, radio)] = parts(d);
            for (what, img, side, count) in [("box", &check, BOX, 12), ("radio", &radio, DISC, 8)] {
                let all = frames(img);
                assert_eq!(all.len(), count, "{what} frames");
                assert_eq!(img.scale, d, "{what} is painted at {d}");
                for (i, f) in all.iter().enumerate() {
                    assert_eq!((f.width, f.height), ((side * d) as u32, (side * d) as u32));
                    for (j, g) in all.iter().enumerate().skip(i + 1) {
                        assert_ne!(f.pixels, g.pixels, "{what} frames {i} and {j} at {d}");
                    }
                }
            }
        }
    }

    /// The border is a whole design pixel of one colour at every scale, and nothing of the box is see-through.
    #[test]
    fn the_box_border_is_one_design_pixel_at_every_scale() {
        for d in [1, 2, 3] {
            let [(_, check), _] = parts(d);
            for (f, ink) in frames(&check).iter().zip(
                [BOX_NORMAL, BOX_HOT, BOX_PRESSED, BOX_DISABLED]
                    .iter()
                    .cycle(),
            ) {
                let border = ink.border.argb();
                let n = BOX * d;
                for i in 0..n {
                    for j in 0..d {
                        for (x, y) in [(i, j), (j, i), (i, n - 1 - j), (n - 1 - j, i)] {
                            assert_eq!(f.get(x, y), border, "({x}, {y}) at {d}");
                        }
                    }
                }
                assert!(f.pixels.iter().all(|p| p >> 24 == 0xFF), "opaque at {d}");
            }
        }
    }

    #[test]
    fn the_tick_is_solid_at_scale_1_and_smooth_at_2() {
        let tick = BOX_NORMAL.tick.argb();
        let one = check_box(&BOX_NORMAL, Check::Checked, 1);
        let solid = one.pixels.iter().filter(|&&p| p == tick).count();
        assert!(solid >= 6, "{solid} solid tick pixels");
        let two = check_box(&BOX_NORMAL, Check::Checked, 2);
        let soft = two
            .pixels
            .iter()
            .filter(|&&p| p != tick && lum(p) < lum(0xFF_9D9E9E))
            .count();
        assert!(soft > 8, "{soft} blended tick edge pixels at 2");
    }

    /// The disc's corners are clear, its rim has partial alpha at every scale, and no edge pixel is darker than the rim it fades from.
    #[test]
    fn the_radio_is_a_soft_edged_disc_at_every_scale() {
        for d in [1, 2] {
            let [_, (_, radio)] = parts(d);
            for (i, f) in frames(&radio).iter().enumerate() {
                let n = DISC * d;
                for (x, y) in [(0, 0), (n - 1, 0), (0, n - 1), (n - 1, n - 1)] {
                    assert_eq!(f.get(x, y), 0, "frame {i} corner ({x}, {y}) at {d}");
                }
                let partial: Vec<u32> = f
                    .pixels
                    .iter()
                    .copied()
                    .filter(|p| (1..0xFF).contains(&(p >> 24)))
                    .collect();
                assert!(
                    partial.len() >= 8 * d as usize,
                    "frame {i}: soft rim at {d}"
                );
                // Unpremultiplied, a soft rim pixel is the rim's colour, never darker.
                let rim = [
                    RADIO_NORMAL.rim,
                    RADIO_HOT.rim,
                    RADIO_PRESSED.rim,
                    RADIO_DISABLED.rim,
                ][i % 4];
                for p in partial.into_iter().filter(|p| p >> 24 >= 0x40) {
                    let a = p >> 24;
                    let straight = (((p >> 16) & 0xFF) * 255 / a)
                        + (((p >> 8) & 0xFF) * 255 / a)
                        + ((p & 0xFF) * 255 / a);
                    let want = u32::from(rim.r) + u32::from(rim.g) + u32::from(rim.b);
                    assert!(
                        straight + 6 >= want,
                        "frame {i}: {p:08X} darker than the rim at {d}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_checked_radio_shows_a_dark_rimmed_dot() {
        let off = radio(&RADIO_NORMAL, false, 1);
        let on = radio(&RADIO_NORMAL, true, 1);
        assert!(
            lum(on.get(6, 6)) < lum(off.get(6, 6)),
            "the dot is darker than the well"
        );
        assert_eq!(on.get(0, 6), off.get(0, 6), "the rim is the same");
        assert!(
            lum(on.get(5, 4)) > lum(on.get(6, 7)),
            "the gloss lights the top-left"
        );
    }

    /// A form under Glass draws its check box and radio in the style's frame for each pointer state:
    /// hot under the pointer, held with no tick yet, ticked and hot after the release.
    #[test]
    fn a_glass_form_lights_its_check_box_and_radio_under_the_pointer() {
        use crate::controls::form::{self, Form, Item};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = theme_for(Interface::Glass);
        let st = crate::glass::style(&Settings::default());
        let px = theme.fonts.size_px;
        let mut f = Form::new(vec![
            Item::check("check", 10, 10, "&Check"),
            Item::radio("radio", 11, 40, "&Radio", "g"),
        ]);
        let drawn = |f: &Form| {
            let mut c = Canvas::new(120, 70);
            c.fill_rect(c.bounds(), crate::theme::Color::rgb(0xEF, 0xEE, 0xEF));
            form::draw(&mut c, &text, &theme.paint(), f, false, px);
            c
        };
        let frame = |part, check, state, x, y| {
            let mut c = Canvas::new(120, 70);
            c.fill_rect(c.bounds(), crate::theme::Color::rgb(0xEF, 0xEE, 0xEF));
            st.draw_check(&mut c, Rect::new(x, y, 13, 13), part, check, state, 1);
            c
        };
        let same = |a: &Canvas, b: &Canvas, x0: i32, y0: i32, what: &str| {
            for y in y0..y0 + 13 {
                for x in x0..x0 + 13 {
                    assert_eq!(a.get(x, y), b.get(x, y), "{what}: ({x}, {y})");
                }
            }
        };
        let at = |f: &Form, x, y| f.hit(x, y, &text, px);
        f.motion(16, 16, at(&f, 16, 16));
        let hot = drawn(&f);
        let want = frame(Part::CheckBox, Check::Unchecked, State::Hot, 10, 10);
        same(&hot, &want, 10, 10, "the hot box");
        f.click(at(&f, 16, 16));
        let held = drawn(&f);
        let want = frame(Part::CheckBox, Check::Unchecked, State::Pressed, 10, 10);
        same(&held, &want, 10, 10, "held, no tick yet");
        assert_eq!(f.release(), form::Event::Changed("check"));
        let ticked = drawn(&f);
        let want = frame(Part::CheckBox, Check::Checked, State::Hot, 10, 10);
        same(&ticked, &want, 10, 10, "ticked and hot");
        f.motion(16, 46, at(&f, 16, 46));
        let hot = drawn(&f);
        let want = frame(Part::Radio, Check::Unchecked, State::Hot, 10, 40);
        same(&hot, &want, 10, 40, "the hot radio");
        let want = frame(Part::CheckBox, Check::Checked, State::Normal, 10, 10);
        same(&hot, &want, 10, 10, "the box is at rest again");
    }

    /// A check box's and a radio's label ink starts 16 px right of the box's or circle's left edge; a
    /// radio item sits a pixel right of the check box column, putting its circle in that column.
    #[test]
    fn a_glass_choice_label_starts_16_px_past_its_box() {
        use crate::controls::form::{self, Form, Item};
        let text = crate::text::TextRenderer::load(&crate::theme::FontConfig::default());
        let theme = crate::theme::Theme {
            access_keys: true,
            ..theme_for(Interface::Glass)
        };
        let f = Form::new(vec![
            Item::check("check", 10, 10, "&Override"),
            Item::radio("radio", 11, 40, "&Accept", "g"),
        ]);
        let mut c = Canvas::new(120, 60);
        c.fill_rect(c.bounds(), crate::theme::Color::rgb(0xEF, 0xEE, 0xEF));
        form::draw(
            &mut c,
            &text,
            &theme.paint(),
            &f,
            false,
            theme.fonts.size_px,
        );
        for (what, top) in [("the check box's", 10), ("the radio's", 40)] {
            let ink = (24..60).find(|&x| (top..top + 13).any(|y| c.get(x, y) & 0xFF < 0xC0));
            assert_eq!(ink, Some(10 + 16), "{what} label ink starts 16 px in");
        }
    }

    #[test]
    fn a_glass_form_draws_the_flat_box_enabled_and_greyed() {
        let p = theme_for(Interface::Glass);
        let on =
            glass(|c, r| draw_checkbox_state(c, &p.paint(), r, CheckState::On, Interaction::Idle));
        let off =
            glass(|c, r| draw_checkbox_state(c, &p.paint(), r, CheckState::Off, Interaction::Idle));
        pin(&on, 2, 2, 0, 0, 0x8B8C8C, "the border is 8B8C8C");
        pin(&on, 2, 2, 1, 1, 0xF3F4F4, "a pale ring inside it");
        assert!(
            lum(on.get(2 + 6, 2 + 6)) < lum(off.get(2 + 6, 2 + 6)),
            "the tick crosses the middle"
        );
        pin(&on, 0, 0, 1, 1, FACE, "nothing outside the box");
        let mixed = glass(|c, r| draw_checkbox_state_disabled(c, &p.paint(), r, CheckState::Mixed));
        pin(&mixed, 2, 2, 0, 0, 0xAEAFAF, "the greyed border is AEAFAF");
        let m = mixed.get(2 + 6, 2 + 6);
        let (r, g, b) = ((m >> 16) & 0xFF, (m >> 8) & 0xFF, m & 0xFF);
        assert!(
            r.max(g).max(b) - r.min(g).min(b) <= 1 && r < 0xE0,
            "the greyed mark is grey, its channels within a level: {m:08X}"
        );
    }

    #[test]
    fn a_glass_radio_rim_blends_into_the_face() {
        let p = theme_for(Interface::Glass);
        let c = glass(|c, r| draw_radio(c, &p.paint(), r, false, Interaction::Idle));
        pin(&c, 2, 2, 0, 0, FACE, "the corner is clear");
        pin(&c, 2, 2, 12, 6, FACE, "nothing past 12 px");
        for (x, y) in [(3, 0), (2, 1)] {
            let v = c.get(2 + x, 2 + y) & 0xFF;
            assert!(
                (0x92..0xF0).contains(&v),
                "({x}, {y}) is part rim, part face: {v:02X}"
            );
        }
        let greyed = glass(|c, r| draw_radio_disabled(c, &p.paint(), r, true));
        let m = greyed.get(2 + 6, 2 + 6);
        let (r, g, b) = ((m >> 16) & 0xFF, (m >> 8) & 0xFF, m & 0xFF);
        assert!(
            r.max(g).max(b) - r.min(g).min(b) <= 1,
            "the greyed dot is grey, its channels within a level: {m:08X}"
        );
    }

    /// Under a theme scaled to 2 the box's border is 2 device px and box and radio are drawn anew, not their scale-1 pixels doubled.
    #[test]
    fn glass_checks_are_drawn_anew_at_scale_2() {
        let draw = |s: i32| {
            let theme = theme_for(Interface::Glass).scaled(s);
            let p = theme.paint();
            let mut c = Canvas::new(30 * s as u32, 17 * s as u32);
            c.fill_rect(c.bounds(), crate::theme::Color::rgb(0xEF, 0xEE, 0xEF));
            let r = Rect::new(2, 2, 13, 13).scaled(s);
            draw_checkbox_state(&mut c, &p, r, CheckState::On, Interaction::Idle);
            let r = Rect::new(16, 2, 13, 13).scaled(s);
            draw_radio(&mut c, &p, r, true, Interaction::Idle);
            c
        };
        let two = draw(2);
        for (x, y) in [(4, 4), (5, 4), (4, 5), (5, 5), (20, 4), (20, 5)] {
            assert_eq!(two.get(x, y), 0xFF8B_8C8C, "the border at ({x}, {y})");
        }
        assert_ne!(two.get(20, 6), 0xFF8B_8C8C, "then the ring");
        assert!(
            !crate::look::styled::fixture::doubles_at_scale_1(&draw(1), &two),
            "the checks at 2 are not their scale-1 pixels doubled"
        );
    }
}
