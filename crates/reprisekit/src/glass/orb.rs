//! The Glass Start orb at output scale: a 36 px disc of clear glass tinted in the glass colour's
//! hue (body, light bowl, gloss, dark rim, white lip), with the icon's own glow ([`bloom`])
//! screened under the icon by the taskbar. Each bar has the orb's rows inside the bar and those
//! above it.

use crate::glass::Settings;
use crate::raster::Canvas;
use crate::shape::{CLEAR, Px, Stops, draw, ellipse, over, pack, premul, scale, shares, smooth};
use crate::style::startbloom::Bloom;
use crate::style::{Layout, PartImage, State};
use crate::theme::Color;

pub const WIDTH: i32 = 54;
pub const RADIUS: f64 = 18.0;
const CX: f64 = 27.0;
const FULL_CY: f64 = 20.0;
const SMALL_CY: f64 = 13.0;

const SATURATION_GAIN: f64 = 1.7;

const LIP_ALPHA: u8 = 130;
const RIM: (f64, u8) = (0.14, 225);
const BODY_SATURATION: f64 = 0.9;
const BODY_STOPS: [(f64, f64, f64); 3] = [(0.0, 0.6, 25.0), (0.7, 0.45, 50.0), (1.0, 0.2, 150.0)];
const BOWL_DROP: f64 = 9.0;
const BOWL_R: (f64, f64) = (14.0, 9.0);
const BOWL_TURN: f64 = -20.0;
const BOWL_SATURATION: f64 = 0.7;
const BOWL_ALPHA: f64 = 170.0;
const GLOSS_RISE: f64 = 8.0;
const GLOSS_R: (f64, f64) = (13.5, 8.5);
const GLOSS_ALPHA: (f64, f64) = (150.0, 12.0);

#[derive(Clone, Copy)]
struct Lift {
    gloss: f64,
    bowl: f64,
    body: f64,
    value: f64,
}

const LIFTS: [(State, Lift); 3] = [
    (
        State::Normal,
        Lift {
            gloss: 1.0,
            bowl: 1.0,
            body: 1.0,
            value: 0.0,
        },
    ),
    (
        State::Hot,
        Lift {
            gloss: 1.15,
            bowl: 1.35,
            body: 1.0,
            value: 0.04,
        },
    ),
    (
        State::Pressed,
        Lift {
            gloss: 1.05,
            bowl: 1.25,
            body: 1.3,
            value: -0.04,
        },
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Full,
    Small,
}

impl Bar {
    fn cy(self) -> f64 {
        match self {
            Bar::Full => FULL_CY,
            Bar::Small => SMALL_CY,
        }
    }

    fn height(self) -> i32 {
        match self {
            Bar::Full => super::BAR_HEIGHT,
            Bar::Small => super::SMALL_TASKBAR_HEIGHT,
        }
    }

    fn reach(self) -> i32 {
        match self {
            Bar::Full => super::ORB_RISE.reach,
            Bar::Small => super::SMALL_ORB_RISE.reach,
        }
    }
}

fn hsv(h: f64, s: f64, v: f64) -> Color {
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let ch = |k: f64| ((k + m) * 255.0).round() as u8;
    Color::rgb(ch(r), ch(g), ch(b))
}

fn hue_of(c: Color) -> (f64, f64) {
    let [r, g, b] = [c.r, c.g, c.b].map(|v| f64::from(v) / 255.0);
    let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
    let span = hi - lo;
    if hi <= 0.0 || span <= 0.0 {
        return (0.0, 0.0);
    }
    let h = if hi == r {
        60.0 * ((g - b) / span)
    } else if hi == g {
        60.0 * ((b - r) / span + 2.0)
    } else {
        60.0 * ((r - g) / span + 4.0)
    };
    (h.rem_euclid(360.0), span / hi)
}

struct Inks {
    hue: f64,
    sat: f64,
}

impl Inks {
    fn new(s: &Settings) -> Inks {
        let (hue, sat) = hue_of(s.color);
        Inks {
            hue,
            sat: (sat * SATURATION_GAIN).min(1.0),
        }
    }

    fn at(&self, turn: f64, share: f64, value: f64, alpha: u8) -> Px {
        premul(hsv(self.hue + turn, self.sat * share, value), alpha)
    }

    fn px(&self, lift: Lift, cy: f64, top: i32, x: f64, y: f64, d: i32) -> Px {
        let k = f64::from(d);
        let y = y + f64::from(top) * k;
        let (mx, my) = ((x + 0.5) / k, (y + 0.5) / k);
        let disc = |r: f64| ellipse(x, y, CX * k, cy * k, r * k, r * k);
        let (lip, rim, body) = (disc(RADIUS), disc(RADIUS - 1.0), disc(RADIUS - 2.0));
        if lip <= 0.0 {
            return CLEAR;
        }

        let stops = BODY_STOPS.map(|(t, value, alpha)| {
            let a = (alpha * lift.body).clamp(0.0, 255.0).round() as u8;
            (t, self.at(0.0, BODY_SATURATION, value + lift.value, a))
        });
        let t = (mx - CX).hypot(my - cy) / (RADIUS - 2.0);
        let mut inside = Stops(&stops).at(t);
        let g = ((mx - CX) / BOWL_R.0).hypot((my - cy - BOWL_DROP) / BOWL_R.1);
        let bowl = (BOWL_ALPHA * lift.bowl * (1.0 - smooth(g))).clamp(0.0, 255.0);
        if bowl > 0.0 {
            let ink = self.at(BOWL_TURN, BOWL_SATURATION, 1.0, bowl.round() as u8);
            inside = over(ink, inside);
        }
        let gloss_cy = cy - GLOSS_RISE;
        let gloss = ellipse(x, y, CX * k, gloss_cy * k, GLOSS_R.0 * k, GLOSS_R.1 * k);
        if gloss > 0.0 {
            let t = ((my - (gloss_cy - GLOSS_R.1)) / (2.0 * GLOSS_R.1)).clamp(0.0, 1.0);
            let a = (GLOSS_ALPHA.0 + (GLOSS_ALPHA.1 - GLOSS_ALPHA.0) * t) * lift.gloss;
            let white = premul(Color::rgb(0xFF, 0xFF, 0xFF), a.clamp(0.0, 255.0) as u8);
            inside = over(scale(white, gloss), inside);
        }
        shares(&[
            (premul(Color::rgb(0xFF, 0xFF, 0xFF), LIP_ALPHA), lip - rim),
            (self.at(0.0, 0.95, RIM.0, RIM.1), rim - body),
            (inside, body),
        ])
    }

    fn frames(&self, cy: f64, top: i32, h: i32, d: i32) -> PartImage {
        let mut atlas = Canvas::new((WIDTH * d) as u32, (3 * h * d) as u32);
        for (i, &(_, lift)) in LIFTS.iter().enumerate() {
            let f = draw(WIDTH, h, d, |x, y| {
                let p = self.px(lift, cy, top, x, y, d);
                if p.0 <= 0.0 { CLEAR } else { p }
            });
            atlas.blit_over(0, i as i32 * h * d, &f);
        }
        let states = LIFTS.map(|(st, _)| st);
        PartImage::new(atlas, &states, Layout::Vertical).at_scale(d)
    }
}

pub fn orb(s: &Settings, bar: Bar, d: i32) -> PartImage {
    Inks::new(s).frames(bar.cy(), 0, bar.height(), d)
}

pub fn above(s: &Settings, bar: Bar, d: i32) -> PartImage {
    let reach = bar.reach();
    Inks::new(s).frames(bar.cy(), -reach, reach, d)
}

#[derive(Clone, Copy)]
struct Spread {
    sigma: f64,
    reach: f64,
    gain: f64,
}

const SPREADS: [(State, Spread); 3] = [
    (
        State::Normal,
        Spread {
            sigma: 1.5,
            reach: 1.2,
            gain: 0.35,
        },
    ),
    (
        State::Hot,
        Spread {
            sigma: 2.5,
            reach: 1.75,
            gain: 1.5,
        },
    ),
    (
        State::Pressed,
        Spread {
            sigma: 2.5,
            reach: 1.7,
            gain: 1.3,
        },
    ),
];

const BLOOM_SIZE: i32 = WIDTH;
const BLOOM_FADE: (f64, f64) = (21.0, 27.0);
const BLOOM_STEPS: usize = 12;
const BLOOM_LEAN: f64 = 0.5;
/// The glow carries a set amount of light: ink brighter than this mean luminance (0..1) glows
/// weaker by that ratio.
const BLOOM_LIGHT: f64 = 0.41;
const LUMA: [f64; 3] = [0.2126, 0.7152, 0.0722];

fn light_share(icon: &Canvas) -> f64 {
    let (mut luma, mut alpha) = (0.0, 0.0);
    for &p in &icon.pixels {
        let ch = |k: u32| f64::from((p >> k) & 0xFF);
        alpha += ch(24);
        luma += LUMA[0] * ch(16) + LUMA[1] * ch(8) + LUMA[2] * ch(0);
    }
    if luma <= 0.0 {
        return 1.0;
    }
    (BLOOM_LIGHT * alpha / luma).min(1.0)
}

/// The glow `icon` (device px, premultiplied, centred on the disc) throws in `state` at `d`: the
/// icon blurred and spread out from its centre, at `strength` percent of the state's gain.
/// Premultiplied throughout, so edges carry no dark fringe.
pub fn bloom(icon: &Canvas, state: State, d: i32, strength: u8) -> Bloom {
    let spread = SPREADS
        .iter()
        .find(|(st, _)| *st == state)
        .map_or(SPREADS[0].1, |&(_, sp)| sp);
    let gain = spread.gain * light_share(icon) * f64::from(strength) / 100.0;
    let n = (BLOOM_SIZE * d) as usize;
    let (iw, ih) = icon.device_size();
    let (iw, ih) = (iw as i32, ih as i32);
    let (ox, oy) = ((n as i32 - iw) / 2, (n as i32 - ih) / 2);
    let mut buf = vec![[0.0_f64; 4]; n * n];
    for y in 0..ih {
        for x in 0..iw {
            let (bx, by) = (x + ox, y + oy);
            if bx < 0 || by < 0 || bx >= n as i32 || by >= n as i32 {
                continue;
            }
            let p = icon.pixels[(y * iw + x) as usize];
            let ch = |k: u32| f64::from((p >> k) & 0xFF);
            buf[by as usize * n + bx as usize] = [ch(24), ch(16), ch(8), ch(0)];
        }
    }
    gaussian(&mut buf, n, spread.sigma * f64::from(d));
    let (cx, cy) = (
        f64::from(ox) + f64::from(iw) / 2.0,
        f64::from(oy) + f64::from(ih) / 2.0,
    );
    let k = f64::from(d);
    let mut out = Canvas::new(n as u32, n as u32);
    for y in 0..n {
        for x in 0..n {
            let (vx, vy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
            let fade =
                1.0 - smooth((vx.hypot(vy) / k - BLOOM_FADE.0) / (BLOOM_FADE.1 - BLOOM_FADE.0));
            if fade <= 0.0 {
                continue;
            }
            let (mut acc, mut weight) = ([0.0; 4], 0.0);
            for i in 0..BLOOM_STEPS {
                let t = i as f64 / (BLOOM_STEPS - 1) as f64;
                let z = 1.0 + (spread.reach - 1.0) * t;
                let p = read_between(&buf, n, cx + vx / z - 0.5, cy + vy / z - 0.5);
                let w = BLOOM_LEAN + t;
                for c in 0..4 {
                    acc[c] += p[c] * w;
                }
                weight += w;
            }
            let g = gain * fade / weight;
            let mut a = acc[0] * g;
            let cap = if a > 255.0 { 255.0 / a } else { 1.0 };
            a *= cap;
            let px: Px = (a, [1, 2, 3].map(|c| acc[c] * g * cap));
            out.put_raw(x as i32, y as i32, pack(px));
        }
    }
    Bloom {
        canvas: out,
        x: -ox,
        y: -oy,
    }
}

fn gaussian(buf: &mut [[f64; 4]], n: usize, sigma: f64) {
    let r = (sigma * 3.0).ceil() as i32;
    let weights: Vec<f64> = (-r..=r)
        .map(|i| (-(f64::from(i) * f64::from(i)) / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let weights: Vec<f64> = weights.iter().map(|w| w / total).collect();
    let mut tmp = vec![[0.0; 4]; n * n];
    for pass in 0..2 {
        let (src, dst): (&[[f64; 4]], &mut [[f64; 4]]) = if pass == 0 {
            (&*buf, &mut tmp)
        } else {
            (&tmp, &mut *buf)
        };
        for y in 0..n {
            for x in 0..n {
                let mut acc = [0.0; 4];
                for (j, w) in weights.iter().enumerate() {
                    let o = j as i32 - r;
                    let (sx, sy) = if pass == 0 {
                        (x as i32 + o, y as i32)
                    } else {
                        (x as i32, y as i32 + o)
                    };
                    if sx < 0 || sy < 0 || sx >= n as i32 || sy >= n as i32 {
                        continue;
                    }
                    let p = src[sy as usize * n + sx as usize];
                    for c in 0..4 {
                        acc[c] += p[c] * w;
                    }
                }
                dst[y * n + x] = acc;
            }
        }
    }
}

fn read_between(buf: &[[f64; 4]], n: usize, x: f64, y: f64) -> [f64; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |px: f64, py: f64| {
        if px < 0.0 || py < 0.0 || px >= n as f64 || py >= n as f64 {
            [0.0; 4]
        } else {
            buf[py as usize * n + px as usize]
        }
    };
    let (a, b, c, e) = (
        at(x0, y0),
        at(x0 + 1.0, y0),
        at(x0, y0 + 1.0),
        at(x0 + 1.0, y0 + 1.0),
    );
    std::array::from_fn(|i| {
        let top = a[i] + (b[i] - a[i]) * fx;
        let bottom = c[i] + (e[i] - c[i]) * fx;
        top + (bottom - top) * fy
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::{Material, Mode, SWATCHES};

    fn alpha(img: &PartImage, st: State, x: i32, y: i32) -> u32 {
        let f = img.frame(st);
        img.atlas.get(f.x + x, f.y + y) >> 24
    }

    fn fnv(img: &PartImage) -> u64 {
        fnv_canvas(&img.atlas)
    }

    fn fnv_canvas(c: &Canvas) -> u64 {
        c.pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    fn frame_pixels(img: &PartImage, st: State) -> Vec<u32> {
        let f = img.frame(st);
        (f.y..f.y + f.height)
            .flat_map(|y| (f.x..f.x + f.width).map(move |x| (x, y)))
            .map(|(x, y)| img.atlas.get(x, y))
            .collect()
    }

    const STATES: [State; 3] = [State::Normal, State::Hot, State::Pressed];

    #[test]
    fn every_orb_part_has_its_size_at_every_scale() {
        let s = Settings::default();
        for d in [1, 2, 3] {
            let d32 = d as u32;
            for (bar, h, reach) in [(Bar::Full, 40, 6), (Bar::Small, 30, 13)] {
                let o = orb(&s, bar, d);
                assert_eq!((o.atlas.width, o.atlas.height), (54 * d32, 3 * h * d32));
                assert_eq!(o.states, STATES);
                let a = above(&s, bar, d);
                assert_eq!((a.atlas.width, a.atlas.height), (54 * d32, 3 * reach * d32));
                assert_eq!(o.sizing, a.sizing, "no nine-grid: drawn at its size");
            }
        }
    }

    #[test]
    fn the_bar_shows_through_the_body_inside_a_dark_rim() {
        let s = Settings::default();
        for d in [1, 2] {
            let o = orb(&s, Bar::Full, d);
            for st in STATES {
                for (x, y) in [(27, 20), (20, 22), (34, 22), (27, 14)] {
                    let a = alpha(&o, st, x * d, y * d);
                    assert!(a < 140, "{st:?} ({x}, {y}) at {d}: alpha {a}");
                }
                let f = o.frame(st);
                let p = o.atlas.get(f.x + 27 * d, f.y + 3 * d + d / 2);
                let (a, rgb) = (p >> 24, [p >> 16, p >> 8, p].map(|c| c & 0xFF));
                assert!(a > 180, "{st:?} rim alpha {a} at {d}");
                assert!(rgb.iter().all(|&c| c < 80), "{st:?} rim {p:#010x} at {d}");
            }
            for (x, y) in [(0, 0), (53, 0), (0, 39), (53, 39), (27, 0), (27, 39)] {
                assert_eq!(
                    alpha(&o, State::Normal, x * d, y * d),
                    0,
                    "({x}, {y}) at {d}"
                );
            }
            let a = above(&s, Bar::Full, d);
            for st in STATES {
                assert!(frame_pixels(&a, st).iter().all(|&p| p == 0), "{st:?}");
            }
        }
    }

    #[test]
    fn the_disc_edge_is_smooth_at_scale_2() {
        let o = orb(&Settings::default(), Bar::Full, 2);
        let row: Vec<u32> = (0..20).map(|x| alpha(&o, State::Normal, x, 40)).collect();
        assert!(row.iter().any(|&a| a > 0 && a < 255), "{row:?}");
        let diag = (0..40).map(|i| alpha(&o, State::Normal, 18 + i, 4 + i));
        assert!(diag.clone().any(|a| a > 0 && a < 255), "AA on the curve");
        let doubled = (0..54).all(|x| {
            (0..40).all(|y| {
                let p = |dx, dy| o.atlas.get(2 * x + dx, 2 * y + dy);
                p(0, 0) == p(1, 0) && p(0, 0) == p(0, 1) && p(0, 0) == p(1, 1)
            })
        });
        assert!(!doubled);
    }

    #[test]
    fn every_state_differs_and_held_deepens_the_body() {
        let s = Settings::default();
        let parts = [
            orb(&s, Bar::Full, 1),
            orb(&s, Bar::Small, 1),
            above(&s, Bar::Small, 1),
        ];
        for img in parts {
            let f = STATES.map(|st| frame_pixels(&img, st));
            assert_ne!(f[0], f[1], "lit differs from rest");
            assert_ne!(f[0], f[2], "held differs from rest");
            assert_ne!(f[1], f[2], "lit differs from held");
        }
        let o = orb(&s, Bar::Full, 1);
        let (rest, held) = (
            alpha(&o, State::Normal, 15, 20),
            alpha(&o, State::Pressed, 15, 20),
        );
        assert!(held > rest, "held {held}, rest {rest}");
    }

    #[test]
    fn the_small_orb_stands_5_rows_above_its_bar() {
        let a = above(&Settings::default(), Bar::Small, 1);
        for st in STATES {
            for y in 0..8 {
                for x in 0..54 {
                    assert_eq!(alpha(&a, st, x, y), 0, "({x}, {y}) clear {st:?}");
                }
            }
        }
        assert!(alpha(&a, State::Normal, 27, 9) > 180, "the rim's top");
        assert_eq!(alpha(&a, State::Normal, 5, 12), 0, "off the disc");
    }

    fn mark(d: i32) -> Canvas {
        let px = 20 * d;
        let mut c = Canvas::new(px as u32, px as u32);
        crate::art::logo::draw_path(&mut c, 0, 0, px, Color::rgb(0xFF, 0xFF, 0xFF));
        c
    }

    fn outside_share(b: &Bloom, icon: i32, d: i32) -> f64 {
        let n = b.canvas.width as i32;
        let c = f64::from(-b.x) + f64::from(icon) / 2.0;
        let (mut all, mut out) = (0.0, 0.0);
        for y in 0..n {
            for x in 0..n {
                let a = f64::from(b.canvas.get(x, y) >> 24);
                let r = (f64::from(x) + 0.5 - c).hypot(f64::from(y) + 0.5 - c);
                all += a;
                if r > RADIUS * f64::from(d) {
                    out += a;
                }
            }
        }
        out / all
    }

    #[test]
    fn the_lit_glow_spills_past_the_disc_and_the_resting_one_mostly_does_not() {
        for d in [1, 2] {
            let icon = mark(d);
            let share = |st| outside_share(&bloom(&icon, st, d, 100), 20 * d, d);
            let (rest, hot, held) = (
                share(State::Normal),
                share(State::Hot),
                share(State::Pressed),
            );
            assert!(rest < 0.05, "rest {rest} at {d}");
            assert!(hot > 0.09, "hot {hot} at {d}");
            assert!(held > 0.07, "held {held} at {d}");
            let sum = |st| {
                let b = bloom(&icon, st, d, 100);
                b.canvas
                    .pixels
                    .iter()
                    .map(|&p| u64::from(p >> 24))
                    .sum::<u64>()
            };
            assert!(sum(State::Hot) > 3 * sum(State::Normal), "at {d}");
        }
    }

    #[test]
    fn the_glow_scales_with_its_strength_and_is_clear_at_0() {
        let icon = mark(1);
        let light = |strength| {
            let b = bloom(&icon, State::Hot, 1, strength);
            (b.canvas.pixels.iter())
                .map(|&p| u64::from(p >> 24))
                .sum::<u64>()
        };
        assert!(
            bloom(&icon, State::Hot, 1, 0)
                .canvas
                .pixels
                .iter()
                .all(|&p| p == 0),
            "0 draws no glow"
        );
        let (half, full) = (light(50), light(100));
        assert!(half > 0, "50 glows");
        assert!(half < full, "50 ({half}) is weaker than 100 ({full})");
        assert!(
            half.abs_diff(full / 2) < full / 50,
            "50 ({half}) is half of 100 ({full})"
        );
    }

    #[test]
    fn a_red_part_glows_red_toward_its_own_side_only() {
        let mut icon = Canvas::new(24, 24);
        for y in 1..10 {
            for x in 1..10 {
                icon.put_raw(x, y, 0xFFFF_0000);
            }
        }
        let b = bloom(&icon, State::Hot, 1, 100);
        for &p in &b.canvas.pixels {
            assert_eq!(p & 0xFFFF, 0, "{p:#010x}");
        }
        let c = -b.x + 12;
        let at = |dx: i32, dy: i32| b.canvas.get(c + dx, c + dy);
        let tl = at(-15, -13);
        assert!(
            tl >> 24 > 20 && (tl >> 16) & 0xFF > 20,
            "top left {tl:#010x}"
        );
        assert_eq!(at(15, 13), 0, "bottom right");
    }

    #[test]
    fn the_glow_has_no_dark_fringe() {
        for d in [1, 2] {
            for st in STATES {
                for &p in &bloom(&mark(d), st, d, 100).canvas.pixels {
                    let a = p >> 24;
                    for c in [p >> 16, p >> 8, p].map(|c| c & 0xFF) {
                        assert!(a - c <= 1, "{st:?} at {d}: {p:#010x}");
                    }
                }
            }
        }
    }

    fn four_colours(px: i32) -> Canvas {
        let inks = [0xFFE0_3020_u32, 0xFF30_B030, 0xFFF0_C020, 0xFF20_70E0];
        let mut c = Canvas::new(px as u32, px as u32);
        let h = px / 2;
        for y in 0..px {
            for x in 0..px {
                if x % h > 0 && y % h > 0 && x % h < h - 1 && y % h < h - 1 {
                    c.put_raw(x, y, inks[(2 * (y / h) + x / h) as usize]);
                }
            }
        }
        c
    }

    fn lit_orb(icon: &Canvas, st: State, d: i32, bar: u32) -> Canvas {
        let o = orb(&Settings::default(), Bar::Full, d);
        let f = o.frame(st);
        let mut c = Canvas::new(f.width as u32, f.height as u32);
        c.pixels.fill(bar);
        let mut frame = Canvas::new(f.width as u32, f.height as u32);
        for y in 0..f.height {
            for x in 0..f.width {
                frame.put_raw(x, y, o.atlas.get(f.x + x, f.y + y));
            }
        }
        c.blit_over(0, 0, &frame);
        let (iw, ih) = icon.device_size();
        let (ix, iy) = (
            (CX * f64::from(d)) as i32 - iw as i32 / 2,
            (FULL_CY * f64::from(d)) as i32 - ih as i32 / 2,
        );
        let b = bloom(icon, st, d, 100);
        let all = crate::raster::Rect::new(0, 0, f.width, f.height);
        c.blit_screen_clipped(ix + b.x, iy + b.y, &b.canvas, all);
        c.blit_over(ix, iy, icon);
        c
    }

    fn luma(p: u32) -> f64 {
        let ch = |k: u32| f64::from((p >> k) & 0xFF);
        LUMA[0] * ch(16) + LUMA[1] * ch(8) + LUMA[2] * ch(0)
    }

    #[test]
    fn the_lit_mark_lights_the_glass_without_washing_it_out() {
        let bar = 0xFF26_4A80;
        for d in [1, 2] {
            let icon = mark(d);
            let c = lit_orb(&icon, State::Hot, d, bar);
            let rest = lit_orb(&icon, State::Normal, d, bar);
            let (k, n) = (f64::from(d), 20 * d);
            let (ix, iy) = (17 * d, 10 * d);
            let (mut lit, mut calm, mut count) = (0.0, 0.0, 0.0);
            for y in 0..c.height as i32 {
                for x in 0..c.width as i32 {
                    let r = (f64::from(x) + 0.5 - CX * k).hypot(f64::from(y) + 0.5 - FULL_CY * k);
                    let (mx, my) = (x - ix, y - iy);
                    let on_ink =
                        mx >= 0 && my >= 0 && mx < n && my < n && icon.get(mx, my) >> 24 > 0;
                    if r < (RADIUS - 3.0) * k && !on_ink {
                        lit += luma(c.get(x, y));
                        calm += luma(rest.get(x, y));
                        count += 1.0;
                    }
                }
            }
            let (lit, calm) = (lit / count, calm / count);
            assert!(
                lit > calm + 10.0,
                "lit lightens the glass: {lit} over {calm} at {d}"
            );
            assert!(
                lit < 150.0,
                "the body stays well under the white letter: {lit} at {d}"
            );
        }
    }

    #[test]
    fn a_coloured_icon_glows_a_little_weaker_than_a_dark_one_and_still_spills() {
        let dark = {
            let mut c = Canvas::new(20, 20);
            c.pixels.fill(0xFF60_2010);
            c
        };
        assert_eq!(light_share(&dark), 1.0);
        let (quads, white) = (light_share(&four_colours(24)), light_share(&mark(1)));
        assert!((0.78..=0.86).contains(&quads), "four colours {quads}");
        assert!(white < 0.5, "white mark {white}");
        for d in [1, 2] {
            let icon = four_colours(24 * d);
            let hot = outside_share(&bloom(&icon, State::Hot, d, 100), 24 * d, d);
            assert!(hot > 0.15, "four colours lit {hot} at {d}");
        }
    }

    #[test]
    fn the_orb_takes_the_glass_colours_hue() {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap();
        let (sky, red) = (
            Settings::default(),
            Settings {
                color: ruby.color,
                ..Settings::default()
            },
        );
        let mid = |s: &Settings| {
            let o = orb(s, Bar::Full, 1);
            let p = o.atlas.get(27, 26);
            [(p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF]
        };
        let ([r0, _, b0], [r1, _, b1]) = (mid(&sky), mid(&red));
        assert!(b0 > r0 + 60, "Sky's orb is blue");
        assert!(r1 > b1 + 60, "Ruby's orb is red");
        let grey = Settings {
            color: Color::rgb(128, 128, 128),
            ..Settings::default()
        };
        let [r, g, b] = mid(&grey);
        assert!(r == g && g == b, "grey orb: {r} {g} {b}");
    }

    #[test]
    fn the_orb_pins_at_scale_1_and_2() {
        let dark = Settings {
            mode: Mode::Dark,
            ..Settings::default()
        };
        let smoked = Settings {
            material: Material::Smoked,
            ..Settings::default()
        };
        let hashes = |s: &Settings| {
            let mut got = Vec::new();
            for d in [1, 2] {
                for bar in [Bar::Full, Bar::Small] {
                    got.push(fnv(&orb(s, bar, d)));
                    got.push(fnv(&above(s, bar, d)));
                }
            }
            got
        };
        let got = hashes(&Settings::default());
        let blooms: Vec<u64> = [1, 2]
            .into_iter()
            .flat_map(|d| STATES.map(|st| fnv_canvas(&bloom(&mark(d), st, d, 100).canvas)))
            .collect();
        let want_blooms: [u64; 6] = [
            0xe0f93492b92e7655,
            0xa35dfb09890112ed,
            0x6cfa94cc554a82ed,
            0x01f776e60cdd7c15,
            0x0cf4fb02493cebfd,
            0x8296529a2af5f741,
        ];
        assert_eq!(blooms, want_blooms, "blooms {blooms:#018x?}");
        let want: [u64; 8] = [
            0x3f916d9cd8749b69,
            0x681c3fa3d15982e5,
            0x0353c2c2215d2ffd,
            0x81db121f5eb6fd21,
            0xb020206431d2f855,
            0x5c60aeb554108225,
            0x9af695e7222090c1,
            0x9287a159cdb64fd1,
        ];
        assert_eq!(got, want, "{got:#018x?}");
        assert_eq!(hashes(&dark), want, "dark glass");
        assert_eq!(hashes(&smoked), want, "smoked glass");
    }

    #[test]
    fn hsv_round_trips_the_glass_hue() {
        let (h, s) = hue_of(Color::rgb(116, 184, 252));
        assert!(
            (h - 210.0).abs() < 0.5 && (s - 0.54).abs() < 0.01,
            "{h} {s}"
        );
        assert_eq!(hsv(0.0, 1.0, 1.0), Color::rgb(255, 0, 0));
        assert_eq!(hsv(240.0, 0.0, 0.5), Color::rgb(128, 128, 128));
    }
}
