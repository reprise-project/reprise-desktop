//! Glass notification parts: the sheet, the icon's well, the pills, the close
//! button and the "+N more" chip, each lit glass drawn at the output scale.

use super::paint::dominant_colour;
use super::{Mode, NotifyStyle};
use crate::backdrop::corner_radius;
use crate::raster::{Canvas, Insets, Rect};
use crate::shape::{
    CLEAR, Px, Stops, centre, draw, ellipse, mix, over, pack, premul, rounded, rounded_distance,
    scale, shares, smooth, stroke, unpack,
};
use crate::style::{NotifyInks, NotifyLook, SQUIRCLE_ROUND, SheetShape, SheetSpec, State, Well};
use crate::theme::Color;

pub static LOZENGE_CORNER: [i32; 25] = crate::backdrop::round_corner();
pub static HALO_CORNER: [i32; 29] = crate::backdrop::round_corner();
/// The Halo's breakout well, whose top half stands above the sheet.
pub static WELL_CORNER: [i32; 14] = crate::backdrop::round_corner();

/// The sheet's shadows fade out inside it.
const LOZENGE_MARGIN: Insets = Insets {
    left: 28,
    top: 20,
    right: 28,
    bottom: 40,
};

/// The well's glow reaches `GLOW_REACH` above it, inside the top margin.
const HALO_MARGIN: Insets = Insets {
    top: 36,
    ..LOZENGE_MARGIN
};

/// The tint toned by `tone` ([`toned`]), at `top`, `mid` (at `WASH_MID` down) and `foot`. Dark
/// deepens it so the light inks keep their contrast over a bright backdrop.
struct Wash {
    tone: f64,
    top: u8,
    mid: u8,
    foot: u8,
}

const LIGHT_WASH: Wash = Wash {
    tone: 0.35,
    top: 195,
    mid: 115,
    foot: 60,
};
const DARK_WASH: Wash = Wash {
    tone: -0.6,
    top: 200,
    mid: 80,
    foot: 30,
};
const WASH_MID: f64 = 0.55;

const HALO_GLOW: u8 = 191;
/// Lifted so the glow shows over a backdrop of the icon's own hue.
const GLOW_LIFT: f64 = 0.25;
/// An icon without a colour glows white and leaves the glass unwashed.
const PLAIN_GLOW: u8 = 90;
const GLOW_REACH: f64 = 28.0;
/// Light and dark; the ring's outer `RING_SOFT` fades out.
const HALO_RING: (u8, u8) = (70, 50);
const RING_W: f64 = 4.0;
const RING_SOFT: f64 = 1.5;

const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BLACK: Color = Color::rgb(0, 0, 0);

type Ink = (Color, u8);

/// A shadow cast by a body: moved `drop` down, shrunk by `spread`, blurred by `sigma`, design px.
struct Cast {
    drop: f64,
    spread: f64,
    sigma: f64,
}

/// A deep drop that lifts the sheet, under a close, tighter one.
const KEY_SHADOW: Cast = Cast {
    drop: 10.0,
    spread: 8.0,
    sigma: 12.0,
};
const AMBIENT_SHADOW: Cast = Cast {
    drop: 3.0,
    spread: 0.0,
    sigma: 5.0,
};

/// The pool is whole out to `POOL_CORE` of its reach, then fades.
const POOL_CORE: f64 = 0.35;

/// One mode's sheet glass. `pool` is the light pooled at (0.2 w, 0), reaching 0.8 w
/// across and 1.5 h down. `shadow` is the key and ambient alphas.
struct SheetGlass {
    fill: Ink,
    rim: Ink,
    hover_rim: Ink,
    top: Ink,
    foot: Ink,
    pool: Ink,
    shadow: (u8, u8),
}

const LIGHT_SHEET: SheetGlass = SheetGlass {
    fill: (WHITE, 97),
    rim: (WHITE, 89),
    hover_rim: (WHITE, 140),
    top: (WHITE, 204),
    foot: (WHITE, 51),
    pool: (WHITE, 150),
    shadow: (96, 48),
};

const DARK_SHEET: SheetGlass = SheetGlass {
    fill: (Color::rgb(0x1C, 0x20, 0x26), 140),
    rim: (WHITE, 46),
    hover_rim: (WHITE, 77),
    top: (WHITE, 77),
    foot: (WHITE, 26),
    pool: (WHITE, 64),
    shadow: (170, 90),
};

const LIGHT_INKS: NotifyInks = NotifyInks {
    title: Color::rgb(0x0F, 0x16, 0x26),
    text: Color::rgb(0x2B, 0x34, 0x45),
    app: Color::rgb(0x5B, 0x65, 0x77),
};

const DARK_INKS: NotifyInks = NotifyInks {
    title: Color::rgb(0xFF, 0xFF, 0xFF),
    text: Color::rgb(0xE3, 0xE7, 0xEE),
    app: Color::rgb(0xA8, 0xB0, 0xBD),
};

impl NotifyStyle {
    pub const fn shape(self) -> SheetShape {
        match self {
            NotifyStyle::Lozenge => SheetShape::Lozenge,
            NotifyStyle::Halo => SheetShape::Halo,
        }
    }
}

pub fn look(mode: Mode) -> NotifyLook {
    NotifyLook {
        inks: match mode {
            Mode::Light => LIGHT_INKS,
            Mode::Dark => DARK_INKS,
        },
        dark: mode == Mode::Dark,
        corner,
        margin,
        sheet,
        well,
        pill,
        close,
        chip,
        colour: dominant_colour,
    }
}

fn corner(shape: SheetShape) -> &'static [i32] {
    match shape {
        SheetShape::Lozenge => &LOZENGE_CORNER,
        SheetShape::Halo => &HALO_CORNER,
    }
}

fn margin(shape: SheetShape) -> Insets {
    match shape {
        SheetShape::Lozenge => LOZENGE_MARGIN,
        SheetShape::Halo => HALO_MARGIN,
    }
}

fn sheet_glass(l: &NotifyLook) -> &'static SheetGlass {
    if l.dark { &DARK_SHEET } else { &LIGHT_SHEET }
}

fn px((c, a): Ink) -> Px {
    premul(c, a)
}

struct Lit {
    fill: Px,
    rim: Px,
    rim_w: f64,
    top: Px,
    foot: Px,
}

/// A body's lit glass and coverage; `cov(inset, dy)` is its coverage shrunk by `inset`
/// and moved down `dy` device px. The top and foot crescents are where a 1 design px shift uncovers.
fn lit(cov: impl Fn(f64, f64) -> f64, k: f64, l: &Lit) -> (Px, f64) {
    let c = cov(0.0, 0.0);
    if c <= 0.0 {
        return (CLEAR, 0.0);
    }
    let ring = (c - cov(l.rim_w, 0.0)).max(0.0);
    let top = (c - cov(0.0, k)).max(0.0);
    let foot = (c - cov(0.0, -k)).max(0.0);
    let mut p = scale(l.fill, c);
    p = over(scale(l.rim, ring), p);
    p = over(scale(l.foot, foot), p);
    p = over(scale(l.top, top), p);
    (p, c)
}

/// The share (0..1) of the rounded rect `(x0, y0, w, h, r)` blurred by `sigma` at (`px`, `py`),
/// design px: exact across each row, integrated down the rows.
fn blurred(px: f64, py: f64, (x0, y0, w, h, r): (f64, f64, f64, f64, f64), sigma: f64) -> f64 {
    let (lo, hi) = ((py - 3.0 * sigma).max(y0), (py + 3.0 * sigma).min(y0 + h));
    if lo >= hi {
        return 0.0;
    }
    const N: i32 = 16;
    let step = (hi - lo) / f64::from(N);
    let norm = 1.0 / (sigma * (2.0 * std::f64::consts::PI).sqrt());
    (0..N)
        .map(|i| {
            let y = lo + (f64::from(i) + 0.5) * step;
            let dy = (y0 + r - y).max(y - (y0 + h - r)).max(0.0);
            let inset = r - (r * r - dy * dy).max(0.0).sqrt();
            let (a, b) = (x0 + inset, x0 + w - inset);
            let across = phi((px - a) / sigma) - phi((px - b) / sigma);
            let t = (py - y) / sigma;
            across * norm * (-0.5 * t * t).exp() * step
        })
        .sum::<f64>()
        .clamp(0.0, 1.0)
}

/// The share of `body` = (x, y, w, h, r) that `how` casts at (`ux`, `uy`), design px.
fn cast(how: &Cast, ux: f64, uy: f64, (x, y, w, h, r): (f64, f64, f64, f64, f64)) -> f64 {
    let s = how.spread;
    let shape = (
        x + s,
        y + how.drop + s,
        w - 2.0 * s,
        h - 2.0 * s,
        (r - s).max(0.0),
    );
    blurred(ux, uy, shape, how.sigma)
}

/// `p` over `c` of its pixel with `shade` on the share it leaves uncovered.
fn cast_under(p: Px, c: f64, shade: Px) -> Px {
    let u = scale(shade, 1.0 - c);
    (p.0 + u.0, std::array::from_fn(|i| p.1[i] + u.1[i]))
}

/// The standard normal's cumulative share below `z`.
fn phi(z: f64) -> f64 {
    let x = z.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-x * x).exp();
    if z >= 0.0 {
        0.5 * (1.0 + erf)
    } else {
        0.5 * (1.0 - erf)
    }
}

fn footprint(spec: &SheetSpec) -> Rect {
    let body = Rect::new(0, 0, spec.size.0, spec.size.1);
    spec.breakout.map_or(body, |b| body.union(&b))
}

/// `well` is `(x, y, w, h, r)`, design px from the canvas.
struct Halo {
    well: (f64, f64, f64, f64, f64),
    wash: [(f64, Px); 3],
    /// The sheet's top glass, pool and wash, filling the whole well.
    glass: Px,
    glow: Px,
    ring: Px,
}

/// `c` moved `k` of the way to white, or for a negative `k` to black.
fn toned(c: Color, k: f64) -> Color {
    let to = if k < 0.0 { 0.0 } else { 255.0 };
    let t = |v: u8| (f64::from(v) + (to - f64::from(v)) * k.abs()).round() as u8;
    Color::rgb(t(c.r), t(c.g), t(c.b))
}

impl Halo {
    fn of(l: &NotifyLook, spec: &SheetSpec, (sx, sy): (f64, f64)) -> Option<Halo> {
        let b = spec.breakout?;
        match spec.shape {
            SheetShape::Lozenge => return None,
            SheetShape::Halo => {}
        }
        let side = f64::from(b.width);
        let well = (
            sx + f64::from(b.x),
            sy + f64::from(b.y),
            side,
            f64::from(b.height),
            SQUIRCLE_ROUND * side,
        );
        let (w, ring) = if l.dark {
            (&DARK_WASH, HALO_RING.1)
        } else {
            (&LIGHT_WASH, HALO_RING.0)
        };
        let ring = premul(WHITE, ring);
        let g = sheet_glass(l);
        let (fill, pool) = (px(g.fill), px(g.pool));
        Some(match spec.tint {
            Some(t) => {
                let c = toned(t, w.tone);
                Halo {
                    well,
                    wash: [
                        (0.0, premul(c, w.top)),
                        (WASH_MID, premul(c, w.mid)),
                        (1.0, premul(c, w.foot)),
                    ],
                    glass: over(pool, over(premul(c, w.top), fill)),
                    glow: premul(toned(t, GLOW_LIFT), HALO_GLOW),
                    ring,
                }
            }
            None => Halo {
                well,
                wash: [(0.0, CLEAR); 3],
                glass: over(pool, fill),
                glow: premul(WHITE, PLAIN_GLOW),
                ring,
            },
        })
    }

    fn wash(&self, t: f64) -> Px {
        Stops(&self.wash).at(t)
    }

    /// `sheet` at the device pixel (`x`, `y`), middle (`ux`, `uy`), lit by the glow and ring. The
    /// well is one glass above and over the sheet: its top wash, no glow.
    fn light(&self, sheet: Px, x: f64, y: f64, (ux, uy): (f64, f64), k: f64) -> Px {
        let (wx, wy, w, h, r) = self.well;
        let out = rounded_distance(ux - wx, uy - wy, w, h, r).max(0.0);
        let outside = 1.0 - rounded(x - wx * k, y - wy * k, w * k, h * k, r * k);
        let glow = scale(self.glow, outside * (1.0 - smooth(out / GLOW_REACH)));
        let ring = outside * (1.0 - smooth((out - RING_W + RING_SOFT) / RING_SOFT));
        let behind = shares(&[(sheet, outside), (self.glass, 1.0 - outside)]);
        over(scale(self.ring, ring), over(glow, behind))
    }
}

fn sheet(l: &NotifyLook, spec: &SheetSpec, d: i32) -> Canvas {
    let d = d.max(1);
    let k = f64::from(d);
    let m = margin(spec.shape);
    let f = footprint(spec);
    let (ox, oy) = (m.left - f.x, m.top - f.y);
    let (cw, ch) = (f.width + m.left + m.right, f.height + m.top + m.bottom);
    let g = sheet_glass(l);
    let (w, h) = (f64::from(spec.size.0), f64::from(spec.size.1));
    let r = f64::from(corner_radius(corner(spec.shape)));
    let (sx, sy) = (f64::from(ox), f64::from(oy));
    let glass = Lit {
        fill: px(g.fill),
        rim: px(if spec.hot_close { g.hover_rim } else { g.rim }),
        rim_w: k,
        top: px(g.top),
        foot: px(g.foot),
    };
    let pool = px(g.pool);
    let body = (sx, sy, w, h, r);
    let halo = Halo::of(l, spec, (sx, sy));
    draw(cw, ch, d, |x, y| {
        let (ux, uy) = (centre(x as i32, d) + 0.5, centre(y as i32, d) + 0.5);
        let wash = halo.as_ref().map_or(CLEAR, |hl| hl.wash((uy - sy) / h));
        let cov = |inset: f64, dy: f64| {
            rounded(
                x - sx * k - inset,
                y - sy * k - dy - inset,
                w * k - 2.0 * inset,
                h * k - 2.0 * inset,
                (r * k - inset).max(0.0),
            )
        };
        let t = ((ux - sx - 0.2 * w) / (0.8 * w)).hypot((uy - sy) / (1.5 * h));
        let lift = 1.0 - smooth((t - POOL_CORE) / (1.0 - POOL_CORE));
        let fill = over(scale(pool, lift), over(wash, glass.fill));
        let (mut p, c) = lit(cov, k, &Lit { fill, ..glass });
        if c < 1.0 {
            let key = f64::from(g.shadow.0) * cast(&KEY_SHADOW, ux, uy, body);
            let ambient = f64::from(g.shadow.1) * cast(&AMBIENT_SHADOW, ux, uy, body);
            let a = 255.0 - (255.0 - key) * (255.0 - ambient) / 255.0;
            p = cast_under(p, c, (a, [0.0; 3]));
        }
        match &halo {
            Some(h) => h.light(p, x, y, (ux, uy), k),
            None => p,
        }
    })
}

/// The well's shadow room on every side, design px.
const WELL_PAD: i32 = 4;

struct WellGlass {
    fill: Ink,
    /// The badge's disc, frosted enough to part its icon from the picture.
    badge: Ink,
    rim: Ink,
    top: Ink,
    ring: Ink,
    shadow: u8,
}

const LIGHT_WELL: WellGlass = WellGlass {
    fill: (WHITE, 64),
    badge: (WHITE, 165),
    rim: (WHITE, 110),
    top: (WHITE, 190),
    ring: (WHITE, 215),
    shadow: 46,
};

const DARK_WELL: WellGlass = WellGlass {
    fill: (WHITE, 26),
    badge: (Color::rgb(0x3A, 0x40, 0x4A), 200),
    rim: (WHITE, 41),
    top: (WHITE, 77),
    ring: (WHITE, 120),
    shadow: 90,
};

const WELL_SHADOW: Cast = Cast {
    drop: 1.0,
    spread: 0.0,
    sigma: 1.2,
};
const BADGE_RING: f64 = 1.5;

fn well(l: &NotifyLook, kind: Well, size: i32, d: i32) -> Canvas {
    let d = d.max(1);
    let k = f64::from(d);
    let g = if l.dark { &DARK_WELL } else { &LIGHT_WELL };
    let (p, s) = (f64::from(WELL_PAD), f64::from(size));
    let r = match kind {
        Well::Squircle => SQUIRCLE_ROUND * s,
        Well::Round | Well::Badge => s / 2.0,
    };
    let (fill, rim, rim_w) = match kind {
        Well::Badge => (g.badge, g.ring, BADGE_RING * k),
        Well::Squircle | Well::Round => (g.fill, g.rim, k),
    };
    let glass = Lit {
        fill: px(fill),
        rim: px(rim),
        rim_w,
        top: px(g.top),
        foot: CLEAR,
    };
    let shade = premul(BLACK, g.shadow);
    let side = size + 2 * WELL_PAD;
    draw(side, side, d, |x, y| {
        let cov = |inset: f64, dy: f64| match kind {
            Well::Squircle => rounded(
                x - p * k - inset,
                y - p * k - dy - inset,
                s * k - 2.0 * inset,
                s * k - 2.0 * inset,
                (r * k - inset).max(0.0),
            ),
            Well::Round | Well::Badge => {
                let (c, rr) = ((p + s / 2.0) * k, r * k - inset);
                ellipse(x, y - dy, c, c, rr, rr)
            }
        };
        let (q, c) = lit(cov, k, &glass);
        if c >= 1.0 {
            return q;
        }
        let (ux, uy) = (centre(x as i32, d) + 0.5, centre(y as i32, d) + 0.5);
        let share = cast(&WELL_SHADOW, ux, uy, (p, p, s, s, r));
        cast_under(q, c, scale(shade, share))
    })
}

/// A pill's face: its fill from top to foot, rim and lit top edge.
struct Face {
    fill: (Ink, Ink),
    rim: Ink,
    top: Ink,
}

const fn face(top: Ink, foot: Ink, rim: Ink, edge: Ink) -> Face {
    Face {
        fill: (top, foot),
        rim,
        top: edge,
    }
}

const DIM: Color = Color::rgb(0xDC, 0xE2, 0xEB);

/// Normal, Hot, Pressed and Disabled.
const LIGHT_PILL: [Face; 4] = [
    face((WHITE, 242), (WHITE, 153), (BLACK, 16), (WHITE, 255)),
    face((WHITE, 255), (WHITE, 214), (BLACK, 22), (WHITE, 255)),
    face((DIM, 235), (WHITE, 200), (BLACK, 30), (WHITE, 90)),
    face((WHITE, 130), (WHITE, 80), (BLACK, 10), (WHITE, 140)),
];

const DARK_PILL: [Face; 4] = [
    face((WHITE, 82), (WHITE, 31), (WHITE, 56), (WHITE, 153)),
    face((WHITE, 112), (WHITE, 54), (WHITE, 77), (WHITE, 190)),
    face((WHITE, 36), (WHITE, 26), (WHITE, 46), (WHITE, 56)),
    face((WHITE, 34), (WHITE, 14), (WHITE, 31), (WHITE, 61)),
];

fn state_index(state: State) -> usize {
    match state {
        State::Hot | State::BarHot | State::InactiveHot | State::CheckedHot => 1,
        State::Pressed => 2,
        State::Disabled => 3,
        _ => 0,
    }
}

/// A `w` x `h` lozenge (radius `h / 2`) in `face`'s glass.
fn lozenge(f: &Face, (w, h): (i32, i32), d: i32) -> Canvas {
    let d = d.max(1);
    let k = f64::from(d);
    let (fw, fh) = (f64::from(w), f64::from(h));
    let (top, foot) = (px(f.fill.0), px(f.fill.1));
    draw(w, h, d, |x, y| {
        let t = ((centre(y as i32, d) + 0.5) / fh).clamp(0.0, 1.0);
        let glass = Lit {
            fill: mix(top, foot, t),
            rim: px(f.rim),
            rim_w: k,
            top: px(f.top),
            foot: CLEAR,
        };
        let cov = |inset: f64, dy: f64| {
            rounded(
                x - inset,
                y - dy - inset,
                fw * k - 2.0 * inset,
                fh * k - 2.0 * inset,
                (fh * k / 2.0 - inset).max(0.0),
            )
        };
        lit(cov, k, &glass).0
    })
}

fn pill(l: &NotifyLook, size: (i32, i32), state: State, d: i32) -> Canvas {
    let faces = if l.dark { &DARK_PILL } else { &LIGHT_PILL };
    lozenge(&faces[state_index(state)], size, d)
}

const LIGHT_CHIP: [Face; 2] = [
    face((WHITE, 120), (WHITE, 97), (WHITE, 89), (WHITE, 204)),
    face((WHITE, 170), (WHITE, 130), (WHITE, 140), (WHITE, 230)),
];

const DARK_CHIP: [Face; 2] = [
    face(
        (Color::rgb(0x1C, 0x20, 0x26), 150),
        (Color::rgb(0x1C, 0x20, 0x26), 140),
        (WHITE, 46),
        (WHITE, 77),
    ),
    face(
        (Color::rgb(0x2A, 0x30, 0x39), 160),
        (Color::rgb(0x2A, 0x30, 0x39), 150),
        (WHITE, 77),
        (WHITE, 110),
    ),
];

fn chip(l: &NotifyLook, size: (i32, i32), state: State, d: i32) -> Canvas {
    let faces = if l.dark { &DARK_CHIP } else { &LIGHT_CHIP };
    lozenge(&faces[usize::from(state_index(state) == 1)], size, d)
}

/// The close button's side and its cross's reach from the middle, design px.
const CLOSE: i32 = 20;
const CROSS_ARM: f64 = 3.5;
const CROSS_WIDTH: f64 = 1.5;

/// Normal, Hot, Pressed and Disabled: the disc's face and the cross's ink.
const LIGHT_CLOSE: [(Face, Ink); 4] = [
    (
        face((WHITE, 150), (WHITE, 120), (BLACK, 18), (WHITE, 230)),
        (Color::rgb(0x3A, 0x42, 0x50), 255),
    ),
    (
        face((WHITE, 230), (WHITE, 200), (BLACK, 26), (WHITE, 255)),
        (Color::rgb(0x0F, 0x16, 0x26), 255),
    ),
    (
        face((DIM, 230), (DIM, 210), (BLACK, 32), (WHITE, 80)),
        (Color::rgb(0x0F, 0x16, 0x26), 255),
    ),
    (
        face((WHITE, 90), (WHITE, 70), (BLACK, 10), (WHITE, 120)),
        (Color::rgb(0x3A, 0x42, 0x50), 110),
    ),
];

const DARK_CLOSE: [(Face, Ink); 4] = [
    (
        face((WHITE, 60), (WHITE, 40), (WHITE, 90), (WHITE, 120)),
        (Color::rgb(0xE3, 0xE7, 0xEE), 255),
    ),
    (
        face((WHITE, 96), (WHITE, 70), (WHITE, 120), (WHITE, 170)),
        (WHITE, 255),
    ),
    (
        face((WHITE, 30), (WHITE, 26), (WHITE, 70), (WHITE, 40)),
        (Color::rgb(0xC8, 0xCD, 0xD6), 255),
    ),
    (
        face((WHITE, 30), (WHITE, 20), (WHITE, 40), (WHITE, 50)),
        (Color::rgb(0xE3, 0xE7, 0xEE), 110),
    ),
];

fn close(l: &NotifyLook, state: State, d: i32) -> Canvas {
    let d = d.max(1);
    let k = f64::from(d);
    let faces = if l.dark { &DARK_CLOSE } else { &LIGHT_CLOSE };
    let (f, ink) = &faces[state_index(state)];
    let mut c = lozenge(f, (CLOSE, CLOSE), d);
    let mid = f64::from(CLOSE) / 2.0 * k;
    let a = CROSS_ARM * k;
    let arms = [
        [(mid - a, mid - a), (mid + a, mid + a)],
        [(mid + a, mid - a), (mid - a, mid + a)],
    ];
    let ink = px(*ink);
    let cross = draw(CLOSE, CLOSE, d, |x, y| {
        let cov = arms
            .iter()
            .map(|pts| stroke(x, y, pts, CROSS_WIDTH * k))
            .fold(0.0, f64::max);
        scale(ink, cov)
    });
    for (p, q) in c.pixels.iter_mut().zip(&cross.pixels) {
        *p = pack(over(unpack(*q), unpack(*p)));
    }
    c
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::backdrop::corner_radius;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::shape::{over, pack, premul, unpack};
    use crate::style::{SheetSpec, State, Well};
    use crate::theme::Color;

    fn fnv(c: &Canvas) -> u64 {
        let head = [c.width, c.height];
        head.iter()
            .chain(c.pixels.iter())
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
                p.to_le_bytes().iter().fold(h, |h, &b| {
                    (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
                })
            })
    }

    fn spec(w: i32, h: i32) -> SheetSpec {
        SheetSpec {
            shape: SheetShape::Lozenge,
            size: (w, h),
            tint: None,
            hot_close: false,
            breakout: None,
        }
    }

    fn mode(dark: bool) -> Mode {
        if dark { Mode::Dark } else { Mode::Light }
    }

    const M: Insets = LOZENGE_MARGIN;

    const PINS: [(bool, bool, i32, u64); 8] = [
        (false, false, 1, 0x9331776ae3a0bcc8),
        (false, true, 1, 0x23d347229ee5ada8),
        (true, false, 1, 0x5b829a3629415131),
        (true, true, 1, 0x76cd0f43c0bf2744),
        (false, false, 2, 0xeb8d2248cb18d140),
        (false, true, 2, 0x7f27cfe6e526cecc),
        (true, false, 2, 0x9396a7e582db91be),
        (true, true, 2, 0x349f5182db908002),
    ];

    #[test]
    fn sheet_pins_per_state_mode_and_scale() {
        let got = PINS.map(|(dark, hot, d, _)| {
            let l = look(mode(dark));
            let c = (l.sheet)(
                &l,
                &SheetSpec {
                    hot_close: hot,
                    ..spec(360, 96)
                },
                d,
            );
            (dark, hot, d, fnv(&c))
        });
        assert!(
            got == PINS,
            "pins changed; now:\n{}",
            got.map(|(dark, hot, d, h)| format!("({dark}, {hot}, {d}, {h:#018x}),"))
                .join("\n")
        );
    }

    #[test]
    fn sheet_corners_are_clear_of_glass() {
        for m in Mode::ALL {
            let l = look(m);
            let c = (l.sheet)(&l, &spec(360, 96), 1);
            assert_eq!((c.width, c.height), (360 + 56, 96 + 60));
            assert_eq!(
                c.get(M.left, M.top) & 0xFF_FFFF,
                0,
                "{m:?}: outside the 26 px corner only shadow"
            );
            assert!(
                c.get(M.left + 180, M.top + 48) >> 24 > 0,
                "{m:?}: the middle is glass"
            );
        }
    }

    #[test]
    fn the_shadow_lifts_the_sheet_and_ends_inside_the_margin() {
        assert_eq!(margin(SheetShape::Lozenge), Insets::new(28, 20, 28, 40));
        for m in Mode::ALL {
            let l = look(m);
            let c = (l.sheet)(&l, &spec(360, 96), 1);
            let under = c.get(M.left + 180, M.top + 96 + 6) >> 24;
            let above = c.get(M.left + 180, M.top - 6) >> 24;
            assert!(under >= 40, "{m:?}: the shade under the foot is {under}");
            assert!(
                under > 2 * above,
                "{m:?}: deeper under ({under}) than above ({above})"
            );
            let (w, h) = (c.width as i32, c.height as i32);
            let rim = (0..w)
                .flat_map(|x| [(x, 0), (x, h - 1)])
                .chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
            assert!(
                rim.into_iter().all(|(x, y)| c.get(x, y) >> 24 <= 1),
                "{m:?}: faded out at the canvas edge"
            );
        }
    }

    #[test]
    fn curved_edges_are_partly_covered_at_scale_2() {
        let l = look(Mode::Light);
        let c = (l.sheet)(&l, &spec(360, 96), 2);
        let row = 2 * M.top + 2;
        assert!(
            (2 * M.left..2 * M.left + 52).any(|x| matches!(c.get(x, row) >> 24, 1..=254)),
            "a soft pixel on the corner's curve"
        );
    }

    fn centre_luma(m: Mode) -> f64 {
        let l = look(m);
        let c = (l.sheet)(&l, &spec(360, 96), 1);
        let grey = premul(Color::rgb(0x80, 0x80, 0x80), 255);
        let (x0, y0) = (M.left + 180 - 20, M.top + 48 - 10);
        let sum: f64 = (y0..y0 + 20)
            .flat_map(|y| (x0..x0 + 40).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = pack(over(unpack(c.get(x, y)), grey));
                let ch = |sh: u32| f64::from((p >> sh) & 0xFF);
                0.299 * ch(16) + 0.587 * ch(8) + 0.114 * ch(0)
            })
            .sum();
        sum / 800.0
    }

    #[test]
    fn light_and_dark_sheets_differ_and_dark_reads_dark() {
        let (light, dark) = (centre_luma(Mode::Light), centre_luma(Mode::Dark));
        assert!(light > dark + 40.0, "light {light} over dark {dark}");
    }

    #[test]
    fn pill_states_are_distinct() {
        for m in Mode::ALL {
            let l = look(m);
            let h: HashSet<u64> = [State::Normal, State::Hot, State::Pressed, State::Disabled]
                .map(|s| fnv(&(l.pill)(&l, (80, 26), s, 1)))
                .into();
            assert_eq!(h.len(), 4, "{m:?}");
        }
    }

    const PART_PINS: [(bool, i32, u64); 4] = [
        (false, 1, 0xdb7bd97ebbf7a5dd),
        (true, 1, 0xd5f64cc5f8a65915),
        (false, 2, 0x7692b11b4f404a93),
        (true, 2, 0x11383a947e2b3d50),
    ];

    #[test]
    fn well_pill_close_and_chip_pins_per_mode_and_scale() {
        let states = [State::Normal, State::Hot, State::Pressed, State::Disabled];
        let got = PART_PINS.map(|(dark, d, _)| {
            let l = look(mode(dark));
            let parts = [Well::Squircle, Well::Round, Well::Badge]
                .map(|w| (l.well)(&l, w, 46, d))
                .into_iter()
                .chain(states.map(|s| (l.pill)(&l, (80, 26), s, d)))
                .chain(states.map(|s| (l.close)(&l, s, d)))
                .chain(states[..2].iter().map(|&s| (l.chip)(&l, (90, 28), s, d)));
            (dark, d, parts.fold(0u64, |h, c| h.rotate_left(7) ^ fnv(&c)))
        });
        assert!(
            got == PART_PINS,
            "pins changed; now:\n{}",
            got.map(|(dark, d, h)| format!("({dark}, {d}, {h:#018x}),"))
                .join("\n")
        );
    }

    #[test]
    fn a_well_centres_on_its_slot_with_its_shadow_below() {
        let l = look(Mode::Light);
        let c = (l.well)(&l, Well::Squircle, 46, 1);
        let pad = (c.width as i32 - 46) / 2;
        assert_eq!(
            (c.width, c.height),
            (46 + 2 * pad as u32, 46 + 2 * pad as u32)
        );
        assert!(pad > 0, "room for the shadow");
        let (mid, foot) = (pad + 23, pad + 46);
        assert!(
            c.get(mid, foot) >> 24 > c.get(mid, pad - 1) >> 24,
            "darker below than above"
        );
    }

    #[test]
    fn the_close_cross_is_inked_in_the_middle() {
        for m in Mode::ALL {
            let l = look(m);
            let c = (l.close)(&l, State::Normal, 2);
            assert_eq!((c.width, c.height), (40, 40));
            let ink = if m == Mode::Light {
                0xFF3A_4250
            } else {
                0xFFE3_E7EE
            };
            assert_eq!(c.get(19, 19), ink, "{m:?}: the cross's middle");
            assert!(c.get(0, 0) >> 24 < 0x10, "{m:?}: a round button");
        }
    }

    #[test]
    fn the_lozenge_corner_steps_radius_26() {
        assert_eq!(corner_radius(&LOZENGE_CORNER), 26.0);
    }

    fn solid(c: Color) -> Canvas {
        let mut k = Canvas::new(48, 48);
        k.fill_rect(Rect::new(0, 0, 48, 48), c);
        k
    }

    const HALO_WELL: Rect = Rect::new(9, -25, 50, 50);

    fn halo_spec(tint: Option<Color>) -> SheetSpec {
        SheetSpec {
            shape: SheetShape::Halo,
            tint,
            breakout: Some(HALO_WELL),
            ..spec(360, 96)
        }
    }

    fn halo_sheet_in(m: Mode, tint: Option<Color>, d: i32) -> Canvas {
        let l = look(m);
        (l.sheet)(&l, &halo_spec(tint), d)
    }

    fn halo_sheet(tint: Option<Color>) -> Canvas {
        halo_sheet_in(Mode::Light, tint, 1)
    }

    /// The sheet's top-left in a Halo canvas at scale 1.
    fn halo_origin() -> (i32, i32) {
        let m = margin(SheetShape::Halo);
        (m.left, m.top - HALO_WELL.y)
    }

    fn over_grey(p: u32) -> (u8, u8, u8) {
        let p = pack(over(unpack(p), premul(Color::rgb(0x80, 0x80, 0x80), 255)));
        ((p >> 16) as u8, (p >> 8) as u8, p as u8)
    }

    fn centre_rgb(tint: Option<Color>) -> (u8, u8, u8) {
        let (x, y) = halo_origin();
        over_grey(halo_sheet(tint).get(x + 180, y + 60))
    }

    #[test]
    fn the_halo_tint_is_the_icon_colour_and_grey_icons_get_none() {
        let look = look(Mode::Light);
        assert_eq!(
            (look.colour)(&solid(Color::rgb(0xD0, 0x30, 0x30))),
            Some(Color::rgb(0xD0, 0x30, 0x30))
        );
        assert_eq!((look.colour)(&solid(Color::rgb(0x80, 0x80, 0x80))), None);
    }

    #[test]
    fn halo_tints_differ_and_untinted_differs_from_tinted() {
        let (red, blue) = (Color::rgb(0xD0, 0x30, 0x30), Color::rgb(0x30, 0x60, 0xD0));
        let hashes: HashSet<u64> = [Some(red), Some(blue), None]
            .map(|t| fnv(&halo_sheet(t)))
            .into();
        assert_eq!(hashes.len(), 3);
        let (r, b) = (centre_rgb(Some(red)), centre_rgb(Some(blue)));
        assert!(r.0 > r.2 && b.2 > b.0, "the wash takes the icon's colour");
        let plain = centre_rgb(None);
        assert_eq!(plain.0, plain.2, "no wash without a tint");
    }

    #[test]
    fn the_halo_corner_steps_radius_30() {
        assert_eq!(corner_radius(&HALO_CORNER), 30.0);
    }

    #[test]
    fn the_breakout_well_corner_steps_radius_15() {
        assert_eq!(corner_radius(&WELL_CORNER), 15.0);
    }

    #[test]
    fn the_halo_canvas_holds_the_well_and_its_glow() {
        let red = Color::rgb(0xD0, 0x30, 0x30);
        for m in Mode::ALL {
            for d in [1, 2] {
                let c = halo_sheet_in(m, Some(red), d);
                let mm = margin(SheetShape::Halo);
                assert_eq!(
                    (c.width as i32, c.height as i32),
                    (
                        (360 + mm.left + mm.right) * d,
                        (96 + 25 + mm.top + mm.bottom) * d
                    )
                );
                let (w, h) = (c.width as i32, c.height as i32);
                let rim = (0..w)
                    .flat_map(|x| [(x, 0), (x, h - 1)])
                    .chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
                assert!(
                    rim.into_iter().all(|(x, y)| c.get(x, y) >> 24 <= 1),
                    "{m:?} x{d}: the glow fades out inside the canvas"
                );
            }
        }
    }

    #[test]
    fn the_glow_above_the_well_is_the_tint_and_white_without_one() {
        let (x, y) = halo_origin();
        let above = (x + HALO_WELL.x + 25, y + HALO_WELL.y - 8);
        let red = halo_sheet(Some(Color::rgb(0xD0, 0x30, 0x30))).get(above.0, above.1);
        let (r, g, b) = ((red >> 16) & 0xFF, (red >> 8) & 0xFF, red & 0xFF);
        assert!(red >> 24 > 30 && r > 2 * g && r > 2 * b, "{red:#010x}");
        let white = halo_sheet(None).get(above.0, above.1);
        let (r, g, b) = ((white >> 16) & 0xFF, (white >> 8) & 0xFF, white & 0xFF);
        assert!(white >> 24 > 15 && r == g && g == b, "{white:#010x}");
        let far = halo_sheet(Some(Color::rgb(0xD0, 0x30, 0x30))).get(above.0, y - 25 - 30);
        assert!(far >> 24 <= 1, "30 px out the glow is gone: {far:#010x}");
    }

    #[test]
    fn the_glow_reaches_far_and_the_well_is_the_washed_glass() {
        let (x, y) = halo_origin();
        let mid = x + HALO_WELL.x + 25;
        for m in Mode::ALL {
            let c = halo_sheet_in(m, Some(Color::rgb(0xD0, 0x30, 0x30)), 1);
            let at = |up: i32| c.get(mid, y + HALO_WELL.y - up) >> 24;
            assert!(at(8) >= 140, "{m:?}: 8 px out the glow is {}", at(8));
            assert!(at(20) >= 30, "{m:?}: 20 px out the glow is {}", at(20));
            let (above, over_sheet) = (
                c.get(mid, y + HALO_WELL.y + 12),
                c.get(mid, y + HALO_WELL.y + 38),
            );
            assert_eq!(
                above, over_sheet,
                "{m:?}: one glass under the well, no glow"
            );
            let (r, _, b) = over_grey(above);
            assert!(
                matches!(above >> 24, 60..=245) && r > b,
                "{m:?}: faint washed glass under the well {above:#010x}"
            );
        }
    }

    #[test]
    fn a_soft_white_ring_hugs_the_well() {
        let (x, y) = halo_origin();
        let mid = y + HALO_WELL.y + 12;
        for m in Mode::ALL {
            for tint in [Some(Color::rgb(0x20, 0x40, 0xA0)), None] {
                let c = halo_sheet_in(m, tint, 1);
                let green = |out: i32| (c.get(x + HALO_WELL.x - out, mid) >> 8) & 0xFF;
                assert!(
                    green(2) >= green(7) + 30,
                    "{m:?} {tint:?}: ring {} past it {}",
                    green(2),
                    green(7)
                );
            }
        }
    }

    #[test]
    fn the_halo_wash_is_heaviest_at_the_top_and_reads_in_both_modes() {
        let (x, y) = halo_origin();
        for m in Mode::ALL {
            let red = halo_sheet_in(m, Some(Color::rgb(0xD0, 0x30, 0x30)), 1);
            let warmth = |dy: i32| {
                let (r, _, b) = over_grey(red.get(x + 250, y + dy));
                i32::from(r) - i32::from(b)
            };
            let (top, foot) = (warmth(6), warmth(90));
            let reads = if m == Mode::Dark { 35 } else { 45 };
            assert!(top >= reads, "{m:?}: the top reads red, r - b = {top}");
            assert!(top >= foot + 20, "{m:?}: top {top}, foot {foot}");
        }
    }

    #[test]
    fn a_dark_tint_deepens_the_glass_rather_than_lighting_it() {
        let (x, y) = halo_origin();
        let luma = |c: &Canvas| {
            let (r, g, b) = over_grey(c.get(x + 250, y + 6));
            299 * u32::from(r) + 587 * u32::from(g) + 114 * u32::from(b)
        };
        let plain = luma(&halo_sheet_in(Mode::Dark, None, 1));
        for tint in [Color::rgb(0xD0, 0x30, 0x30), Color::rgb(0x58, 0x65, 0xF2)] {
            let tinted = luma(&halo_sheet_in(Mode::Dark, Some(tint), 1));
            assert!(tinted <= plain, "{tint:?}: {tinted} over {plain}");
        }
    }

    #[test]
    fn the_glass_style_carries_a_notify_look() {
        assert!(
            theme_for(Interface::Glass)
                .look
                .style()
                .unwrap()
                .notify
                .is_some()
        );
        assert!(
            theme_for(Interface::Blue)
                .look
                .style()
                .is_none_or(|s| s.notify.is_none())
        );
    }
}
