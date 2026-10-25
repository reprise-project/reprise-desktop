//! Reprise Glass's task buttons, drawn from the glass settings at the bar's
//! height: the icon button and the small bar's, the labelled button and its
//! end column pairs, the edges stacked behind a button for several windows,
//! the pager tile, and the lit frames the spot lights with its gain over them.
//!
//! Every frame is a light rounded box over the bar: a dark outline, a light
//! line inside it, and glass with a sheen falling from the top left. Active,
//! an icon button's light line rounds its inner corners at the foot; lit, a
//! second light column runs inside it ([`Inner`]). The outline is the bar's
//! own glass made denser, covering a share of what the bar lets through in
//! the glass's colour ([`Bar::veil_over`]), so it follows colour, intensity
//! and material and stops where the bar under it (with the labelled row's
//! veil under a labelled frame) reaches [`DENSE_CEILING`]. The lighter parts
//! (light line, glass's white, sheen) are white laid over as is, so over
//! Smoked's dense bar the button reads darker and over Molten's clear one
//! lighter. With transparency off the bar is opaque and frames lie over it
//! as over translucent glass.
//!
//! A frame lit by the spot is drawn without it; the spot's gain on each pixel
//! comes from the frame's shape and look ([`Gain`]). Where a lit frame is drawn
//! without a pointer ([`buttons`]' lit states, the pager tile, a lit labelled
//! button's end pairs), the spot lies on it as under a pointer at the frame's
//! middle, without a hue.
//!
//! Every frame is painted at `n` pixels per design pixel: outline and light
//! line `n` px wide, cut corners on the curve of [`corner_radius`] times `n`,
//! each ring laid as far as it covers the pixel, and sheen, white and gains
//! worked out at each pixel's middle ([`centre`]). At `n` = 1 corners are cut whole.
//!
//! [`DENSE_CEILING`]: super::panel::DENSE_CEILING

use super::Settings;
use super::bar::{Bar, VEIL_SHADE, white};
use super::panel::{self, Corners};
use super::tint::split;
use crate::backdrop::corner_radius;
use crate::raster::{Canvas, Insets};
use crate::shape::{self, centre, over, pack, smooth};
use crate::style::taskbar::{EDGE_LEFT, EDGE_RIGHT};
use crate::style::{
    Layout, PartImage, SpotFit, SpotFrames, SpotLayer, State, TaskSpot, glow_chroma,
};
use crate::theme::Color;

/// The light that follows the pointer along a lit button's foot.
pub const SPOT: SpotFit = SpotFit {
    dx: 2.1853,
    halo: SpotLayer {
        peak: 0.6014,
        rx: 0.4908,
        ry: 0.3987,
        power: 1.6994,
        foot: 0.8370,
        white: [171, 116, 144],
        tint: 0.9754,
    },
    core: SpotLayer {
        peak: 0.7162,
        rx: 0.5484,
        ry: 0.6096,
        power: 1.3979,
        foot: 1.0116,
        white: [252, 250, 249],
        tint: -0.0304,
    },
};

/// The spot's gain on a lit look's frame, in 32nds of a layer: none on the
/// outline, `line` on the light line, `inner` on a second light column
/// ([`Inner::Column`]), and on the glass `glass.0` on its first row (y = 2)
/// to `glass.1` on its last (y = h - 3), even between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Gain {
    line: u8,
    inner: u8,
    glass: (u8, u8),
}

impl Gain {
    fn glass_at(&self, y: f64, h: i32) -> u8 {
        let (top, foot) = (f64::from(self.glass.0), f64::from(self.glass.1));
        let v = ((y - 2.0) / f64::from(h - 5)).clamp(0.0, 1.0);
        (top + (foot - top) * v).round() as u8
    }
}

const HOT_GAIN: Gain = Gain {
    line: 69,
    inner: 44,
    glass: (37, 37),
};

const ACTIVE_HOT_GAIN: Gain = Gain {
    line: 0,
    inner: 67,
    glass: (16, 53),
};

const LABEL_HOT_GAIN: Gain = Gain {
    line: 45,
    inner: 0,
    glass: (33, 33),
};

const LABEL_ACTIVE_HOT_GAIN: Gain = Gain {
    line: 0,
    inner: 0,
    glass: (13, 45),
};

const UNLIT: Gain = Gain {
    line: 0,
    inner: 0,
    glass: (0, 0),
};

const CORNER_ROWS: &[i32] = &[1];
const CORNERS: Corners = Corners::Cut {
    rows: CORNER_ROWS,
    bottom: true,
};

/// Where a design's sheen lies, as shares of the frame's width and height.
/// Its lower edge is `band` of the height down, and `deep` at its left end,
/// easing across `cut` of the width. It fades to nothing at `rows` of the
/// height; across, it eases from whole to its look's end share over `fade`.
struct Shape {
    band: f64,
    deep: f64,
    cut: (f64, f64),
    rows: f64,
    fade: (f64, f64),
}

/// One state's frame, in levels of alpha. `line` and `outline` are at the
/// frame's top left, top right, bottom left and bottom right corners,
/// bilinear between; the outline's is the share of what the bar lets through
/// that it covers, times 255.
struct Look {
    line: [f64; 4],
    outline: [f64; 4],
    /// The sheen's white at its top left, and the share left at its right end.
    sheen: (f64, f64),
    /// The glass's white on the first glass row and the last, even between.
    body: (f64, f64),
    /// Down the sides between the top and bottom lines, where they differ from `line` and `outline`.
    sides: Option<Sides>,
    inner: Inner,
    gain: Gain,
}

/// The glass just inside a closed frame's light line: the columns next to its
/// sides and the row above its foot. Each pixel takes the lighter of the glass and what is drawn here.
#[derive(Clone, Copy)]
enum Inner {
    Glass,
    /// The line rounds its two inner corners at the foot: the glass pixel in each
    /// takes this share of the way from glass to line.
    Corners(f64),
    /// A second light column inside the line down both sides and along the foot,
    /// its white at the frame's top left, top right, bottom left and bottom right, bilinear between.
    Column([f64; 4]),
}

/// A frame's sides at rest, where the labelled row's end pairs show them: the
/// outline at `outline`, the light line at `line.0` on the first glass row,
/// fading by `line.1` a row to no less than `line.2`.
struct Sides {
    outline: f64,
    line: (f64, f64, f64),
}

/// A family of frames: icon buttons or labelled ones. Its looks are at rest,
/// active, lit (under the pointer, without the spot) and active and lit (also held).
struct Design {
    shape: Shape,
    looks: [Look; 4],
    /// The shade of the bar under the frames, where their outline stops at the
    /// ceiling: none under icons, the labelled row's veil under labelled ones.
    under: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lit {
    Rest,
    Active,
    Hot,
    ActiveHot,
}

/// The icon buttons, 60x40 and the small bar's 44x30: a sheen down to half
/// the height, deeper at the left end, fading across to under a third of its
/// strength at rest and two thirds active. Active, the glass brightens
/// toward the foot and the light line rounds its inner corners there. Lit, a
/// second light column runs inside the line, brightest along the foot;
/// active and lit, brighter still.
const ICONS: Design = Design {
    shape: Shape {
        band: 0.5,
        deep: 0.72,
        cut: (0.03, 0.22),
        rows: 0.55,
        fade: (0.2, 0.9),
    },
    looks: [
        Look {
            line: [245.0, 120.0, 71.0, 146.0],
            outline: [219.0, 137.0, 108.0, 153.0],
            sheen: (161.0, 0.29),
            body: (25.0, 11.0),
            sides: None,
            inner: Inner::Glass,
            gain: UNLIT,
        },
        Look {
            line: [255.0, 220.0, 209.0, 229.0],
            outline: [219.0, 137.0, 108.0, 153.0],
            sheen: (196.0, 0.62),
            body: (30.0, 129.0),
            sides: None,
            inner: Inner::Corners(ACTIVE_CORNER),
            gain: UNLIT,
        },
        Look {
            line: [255.0, 177.0, 196.0, 215.0],
            outline: [219.0, 137.0, 108.0, 153.0],
            sheen: (167.0, 0.30),
            body: (16.0, 72.0),
            sides: None,
            inner: Inner::Column([160.0, 100.0, 170.0, 170.0]),
            gain: HOT_GAIN,
        },
        Look {
            line: [255.0, 239.0, 243.0, 255.0],
            outline: [255.0, 196.0, 172.0, 215.0],
            sheen: (183.0, 0.55),
            body: (62.0, 119.0),
            sides: None,
            inner: Inner::Column([180.0, 168.0, 200.0, 203.0]),
            gain: ACTIVE_HOT_GAIN,
        },
    ],
    under: 0.0,
};

/// How far an active icon button's inner corners at the foot go from glass toward light line.
const ACTIVE_CORNER: f64 = 0.3;

/// The labelled buttons, 154x40: a shallow sheen along the top, falling most of the way down its left end.
const LABELS: Design = Design {
    shape: Shape {
        band: 0.23,
        deep: 0.92,
        cut: (0.30, 0.45),
        rows: 0.69,
        fade: (0.08, 0.95),
    },
    looks: [
        Look {
            line: [209.0, 209.0, 135.0, 138.0],
            outline: [197.0, 198.0, 150.0, 150.0],
            sheen: (119.0, 0.17),
            body: (62.0, 16.0),
            sides: Some(Sides {
                outline: 130.0,
                line: (183.0, 5.5, 19.0),
            }),
            inner: Inner::Glass,
            gain: UNLIT,
        },
        Look {
            line: [255.0, 209.0, 180.0, 228.0],
            outline: [218.0, 152.0, 122.0, 162.0],
            sheen: (79.0, 0.63),
            body: (111.0, 124.0),
            sides: None,
            inner: Inner::Glass,
            gain: UNLIT,
        },
        Look {
            line: [255.0, 195.0, 219.0, 240.0],
            outline: [218.0, 155.0, 111.0, 159.0],
            sheen: (119.0, 0.30),
            body: (51.0, 109.0),
            sides: None,
            inner: Inner::Glass,
            gain: LABEL_HOT_GAIN,
        },
        Look {
            line: [255.0, 244.0, 252.0, 255.0],
            outline: [251.0, 219.0, 180.0, 221.0],
            sheen: (119.0, 0.48),
            body: (112.0, 145.0),
            sides: None,
            inner: Inner::Glass,
            gain: LABEL_ACTIVE_HOT_GAIN,
        },
    ],
    under: VEIL_SHADE,
};

pub const LABEL_WIDTH: i32 = 154;

const LABEL_SIZING: Insets = Insets::new(110, 3, 4, 3);

const BUTTON_SIZING: Insets = Insets::new(4, 3, 4, 3);

/// How much of a button's last four columns the edge stacked right behind it shows, and the one behind that.
const STACK_NEAR: f64 = 0.55;
const STACK_FAR: f64 = 0.25;

/// How much of a labelled button's light line and outline shows on the column it shares with the next.
const SHARED: f64 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ring {
    Off,
    Outline,
    Line,
    Glass,
}

fn ring(x: i32, y: i32, w: i32, h: i32, open: bool) -> Ring {
    let (x, w) = if open { (x + 2, w + 4) } else { (x, w) };
    match panel::rings(CORNERS, x, y, w, h).map(|v| v > 0.5) {
        [false, _, _] => Ring::Off,
        [_, false, _] => Ring::Outline,
        [_, _, false] => Ring::Line,
        _ => Ring::Glass,
    }
}

/// How much of device pixel (`x`, `y`) of a `w` x `h` (design) frame painted
/// at `n` > 1 lies on the frame, inside its outline and inside its light
/// line: each ring `n` px wide, corners round over [`corner_radius`] of
/// [`CORNERS`] times `n`, each ring's curve `n` px tighter than the one
/// round it. An `open` frame runs on past its sides.
fn rings_at(x: i32, y: i32, w: i32, h: i32, open: bool, n: i32) -> [f64; 3] {
    let (x, w) = if open { (x + 2 * n, w + 4) } else { (x, w) };
    let k = f64::from(n);
    let r = f64::from(corner_radius(CORNER_ROWS)) * k;
    let (w, h) = (f64::from(w) * k, f64::from(h) * k);
    [0.0, 1.0, 2.0].map(|i| {
        let inset = i * k;
        shape::rounded(
            f64::from(x) - inset,
            f64::from(y) - inset,
            w - 2.0 * inset,
            h - 2.0 * inset,
            (r - inset).max(0.0),
        )
    })
}

/// `c` (top left, top right, bottom left, bottom right) at (`x`, `y`) in design pixels ([`centre`]).
fn bilinear(c: [f64; 4], x: f64, y: f64, w: i32, h: i32) -> f64 {
    let u = x / f64::from((w - 1).max(1));
    let v = y / f64::from((h - 1).max(1));
    let [tl, tr, bl, br] = c;
    (tl * (1.0 - u) + tr * u) * (1.0 - v) + (bl * (1.0 - u) + br * u) * v
}

/// A drawn frame: per pixel what it lays over the bar, and the spot's gain in 32nds. `w` and `h` count pixels.
struct Drawn {
    w: i32,
    h: i32,
    px: Vec<u32>,
    gain: Vec<u8>,
}

impl Drawn {
    fn at(&self, x: i32, y: i32) -> (u32, u8) {
        let i = (y * self.w + x) as usize;
        (self.px[i], self.gain[i])
    }
}

/// `d`'s frame in `lit` at `w` x `h` (design) over `bar`, painted at `n`;
/// `open` leaves its sides to the end pairs.
fn draw(bar: &Bar, d: &Design, lit: Lit, w: i32, h: i32, open: bool, n: i32) -> Drawn {
    let look = &d.looks[lit as usize];
    let sh = &d.shape;
    let (wf, hf) = (f64::from(w), f64::from(h));
    let (dw, dh) = (w * n, h * n);
    let mut px = Vec::with_capacity((dw * dh) as usize);
    let mut gain = Vec::with_capacity((dw * dh) as usize);
    for y in 0..dh {
        let (row, yf, top) = (y / n, centre(y, n), f64::from(y) / f64::from(n));
        let down = (1.0 - (yf - 2.0) / (sh.rows * hf)).max(0.0);
        let body =
            look.body.0 + (look.body.1 - look.body.0) * ((yf - 2.0) / (hf - 5.0)).clamp(0.0, 1.0);
        let side = (2..h - 2)
            .contains(&row)
            .then_some(look.sides.as_ref())
            .flatten();
        for x in 0..dw {
            let (col, xc) = (x / n, centre(x, n));
            let xf = xc + 0.5;
            // A ring is a design pixel wide: along it the pixel's middle, across it the
            // design pixel's, so it is one colour across.
            let (rx, ry) = if row < 2 || row >= h - 2 {
                (xc, f64::from(row))
            } else {
                (f64::from(col), yf)
            };
            let ring_px = |ring: Ring| match ring {
                Ring::Off => (0, 0),
                Ring::Outline => {
                    let a = match side {
                        Some(sides) => sides.outline,
                        None => bilinear(look.outline, rx, ry, w, h),
                    };
                    (pack(bar.veil_over(d.under, a.clamp(0.0, 255.0) / 255.0)), 0)
                }
                Ring::Line => {
                    let a = match side {
                        Some(Sides {
                            line: (top, fall, least),
                            ..
                        }) => (top - fall * (yf - 2.0)).max(*least),
                        None => bilinear(look.line, rx, ry, w, h),
                    };
                    (pack(white(a / 255.0)), look.gain.line)
                }
                Ring::Glass => {
                    let (s0, s_end) = look.sheen;
                    let (f0, f1) = sh.fade;
                    let across = 1.0 - (1.0 - s_end) * smooth((xf - f0 * wf) / ((f1 - f0) * wf));
                    let (c0, c1) = sh.cut;
                    let edge = (sh.band
                        + (sh.deep - sh.band) * (1.0 - smooth((xf - c0 * wf) / ((c1 - c0) * wf))))
                        * hf;
                    let cover = ((edge - top) * f64::from(n)).clamp(0.0, 1.0);
                    let glass = body + s0 * down * across * cover;
                    let (column, foot) = (col == 2 || col == w - 3, row == h - 3);
                    let inner = match look.inner {
                        _ if open || !(column || foot) => None,
                        Inner::Glass => None,
                        Inner::Corners(share) => (column && foot)
                            .then(|| glass + share * (bilinear(look.line, xc, yf, w, h) - glass)),
                        Inner::Column(c) => Some(bilinear(c, xc, yf, w, h)),
                    };
                    let a = inner.map_or(glass, |v| v.max(glass));
                    let g = match (look.inner, inner) {
                        (Inner::Column(_), Some(_)) => look.gain.inner,
                        _ => look.gain.glass_at(yf, h),
                    };
                    (pack(white(a / 255.0)), g)
                }
            };
            let (p, g) = if n == 1 {
                ring_px(ring(x, y, w, h, open))
            } else {
                let [o, l, inside] = rings_at(x, y, w, h, open, n);
                laid(&[
                    (ring_px(Ring::Outline), o - l),
                    (ring_px(Ring::Line), l - inside),
                    (ring_px(Ring::Glass), inside),
                ])
            };
            px.push(p);
            gain.push(g);
        }
    }
    Drawn {
        w: dw,
        h: dh,
        px,
        gain,
    }
}

/// Rings' pixels and gains laid each as far as it covers a pixel: premultiplied
/// channels summed by cover, the gain the covered part's mean.
fn laid(rings: &[((u32, u8), f64)]) -> (u32, u8) {
    let covered: f64 = rings.iter().map(|&(_, k)| k).sum();
    if covered <= 0.0 {
        return (0, 0);
    }
    let ch = |sh: u32| {
        let v: f64 = rings
            .iter()
            .map(|&((p, _), k)| f64::from((p >> sh) & 0xFF) * k)
            .sum();
        (v.round() as u32).min(255) << sh
    };
    let g: f64 = rings.iter().map(|&((_, g), k)| f64::from(g) * k).sum();
    (ch(24) | ch(16) | ch(8) | ch(0), (g / covered).round() as u8)
}

fn faded(px: u32, share: f64) -> u32 {
    let ch = |sh: u32| (f64::from((px >> sh) & 0xFF) * share).round() as u32;
    ch(24) << 24 | ch(16) << 16 | ch(8) << 8 | ch(0)
}

/// `px` lit by the spot as under a pointer at the frame's middle, without a
/// hue, through `gain`; the pixel's middle is (`x`, `y`) in design pixels of a `w` x `h` frame.
fn spot_lit(px: u32, gain: u8, x: f32, y: f32, w: f32, h: f32) -> u32 {
    let g = f32::from(gain) / 32.0;
    let cx = (w / 2.0).floor() + 0.5 + SPOT.dx;
    SPOT.in_colour(None).light(px, g, x - cx, y, w, h)
}

fn mid(v: i32, n: i32) -> f32 {
    (v as f32 + 0.5) / n as f32
}

/// Which look a state takes, and whether it is lit. Held is the active, lit frame.
fn look_of(state: State) -> Lit {
    match state {
        State::Hot => Lit::Hot,
        State::Checked => Lit::Active,
        State::CheckedHot | State::Pressed => Lit::ActiveHot,
        _ => Lit::Rest,
    }
}

/// Frames of `w` x `h` pixels laid along `layout`, one per state, each pixel from `pixel(state, x, y)`.
fn frames(
    states: &[State],
    (w, h): (i32, i32),
    n: i32,
    layout: Layout,
    mut pixel: impl FnMut(usize, i32, i32) -> u32,
) -> PartImage {
    let count = states.len() as i32;
    let (aw, ah) = match layout {
        Layout::Vertical => (w, h * count),
        Layout::Horizontal => (w * count, h),
    };
    let mut c = Canvas::new(aw as u32, ah as u32);
    for i in 0..states.len() {
        let (x0, y0) = match layout {
            Layout::Vertical => (0, i as i32 * h),
            Layout::Horizontal => (i as i32 * w, 0),
        };
        for y in 0..h {
            for x in 0..w {
                c.put_raw(x0 + x, y0 + y, pixel(i, x, y));
            }
        }
    }
    PartImage::new(c, states, layout).at_scale(n)
}

const BUTTON_STATES: [State; 5] = [
    State::Normal,
    State::Hot,
    State::Checked,
    State::CheckedHot,
    State::Pressed,
];

const LIT_STATES: [State; 3] = [State::Hot, State::CheckedHot, State::Pressed];

/// A task button `size` (60x40, or the small bar's 44x30) in each state, as
/// a nine-grid (4 px sides, 3 px top and bottom). Lit or held it is the lit
/// frame with the spot at its middle; on the bar those are
/// [`spot_buttons`]' frames under the spot instead.
pub fn buttons(s: &Settings, (w, h): (i32, i32), n: i32) -> PartImage {
    let bar = Bar::new(s);
    let drawn = BUTTON_STATES.map(|st| draw(&bar, &ICONS, look_of(st), w, h, false, n));
    let (wf, hf) = (w as f32, h as f32);
    frames(
        &BUTTON_STATES,
        (w * n, h * n),
        n,
        Layout::Vertical,
        |i, x, y| {
            let (px, g) = drawn[i].at(x, y);
            if LIT_STATES.contains(&BUTTON_STATES[i]) {
                spot_lit(px, g, mid(x, n), mid(y, n), wf, hf)
            } else {
                px
            }
        },
    )
    .sizing(BUTTON_SIZING)
}

/// The tile behind the current desktop's pager number: [`buttons`]' lit,
/// active and active lit frames, shorter than the bar and narrower than the cell.
pub fn pager_tiles(s: &Settings, size: (i32, i32), n: i32) -> PartImage {
    let all = buttons(s, size, n);
    let (w, h) = size;
    let states = [State::Hot, State::Checked, State::CheckedHot];
    frames(&states, (w * n, h * n), n, Layout::Vertical, |i, x, y| {
        let f = all.frame(states[i]);
        all.atlas.get(f.x + x, f.y + y)
    })
    .sizing(BUTTON_SIZING)
}

/// [`SpotFrames`] of `drawn` frames, one per state, laid along `layout`; each state has a pair of frames.
fn spot_frames(
    states: &[State],
    drawn: &[Drawn],
    n: i32,
    layout: Layout,
    size: fn(PartImage) -> PartImage,
) -> SpotFrames {
    let wh = (drawn[0].w, drawn[0].h);
    let base = frames(states, wh, n, layout, |i, x, y| drawn[i].at(x, y).0);
    let gain = frames(states, wh, n, layout, |i, x, y| {
        0xFF00_0000 | u32::from(drawn[i].at(x, y).1)
    });
    SpotFrames {
        base: size(base),
        gain: size(gain),
    }
}

pub fn spot_buttons(s: &Settings, (w, h): (i32, i32), n: i32) -> SpotFrames {
    let bar = Bar::new(s);
    let drawn = LIT_STATES.map(|st| draw(&bar, &ICONS, look_of(st), w, h, false, n));
    spot_frames(&LIT_STATES, &drawn, n, Layout::Vertical, |p| {
        p.sizing(BUTTON_SIZING)
    })
}

/// The edge stacked behind a button `size` in `state`: its last four columns
/// (glass, glass, light line, outline) at [`STACK_NEAR`], then the same at
/// [`STACK_FAR`], side by side.
fn stack_pair(bar: &Bar, state: State, (w, h): (i32, i32), n: i32) -> Drawn {
    let f = draw(bar, &ICONS, look_of(state), w, h, false, n);
    let mut px = Vec::with_capacity((8 * n * f.h) as usize);
    let mut gain = Vec::with_capacity((8 * n * f.h) as usize);
    for y in 0..f.h {
        for share in [STACK_NEAR, STACK_FAR] {
            for x in f.w - 4 * n..f.w {
                let (p, g) = f.at(x, y);
                px.push(faded(p, share));
                gain.push(g);
            }
        }
    }
    Drawn {
        w: 8 * n,
        h: f.h,
        px,
        gain,
    }
}

/// The edges stacked behind a task button `size` for two or more windows: per
/// state two 4 px frames side by side, the one against the button first, each
/// the strip of a button behind showing past the one in front (2 px of glass,
/// light line and outline), the farther one fainter. `lit` gives the lit and
/// held states their frames (without the spot, which lies over them); without
/// it there are pairs at rest and active only.
pub fn stacks(s: &Settings, size: (i32, i32), lit: bool, n: i32) -> PartImage {
    let bar = Bar::new(s);
    let states: &[State] = if lit {
        &BUTTON_STATES
    } else {
        &[State::Normal, State::Checked]
    };
    let pairs: Vec<Drawn> = states
        .iter()
        .map(|&st| stack_pair(&bar, st, size, n))
        .collect();
    let twice: Vec<State> = states.iter().flat_map(|&st| [st, st]).collect();
    let w = 4 * n;
    frames(&twice, (w, size.1 * n), n, Layout::Horizontal, |i, x, y| {
        pairs[i / 2].at((i % 2) as i32 * w + x, y).0
    })
}

pub fn spot_stacks(s: &Settings, size: (i32, i32), n: i32) -> SpotFrames {
    let bar = Bar::new(s);
    let pairs = LIT_STATES.map(|st| stack_pair(&bar, st, size, n));
    let w = 4 * n;
    let halves: Vec<Drawn> = pairs
        .iter()
        .flat_map(|p| {
            [0, w].map(|x0| {
                let (mut px, mut gain) = (Vec::new(), Vec::new());
                for y in 0..p.h {
                    for x in x0..x0 + w {
                        let (a, g) = p.at(x, y);
                        px.push(a);
                        gain.push(g);
                    }
                }
                Drawn {
                    w,
                    h: p.h,
                    px,
                    gain,
                }
            })
        })
        .collect();
    let twice: Vec<State> = LIT_STATES.iter().flat_map(|&st| [st, st]).collect();
    spot_frames(&twice, &halves, n, Layout::Horizontal, |p| p)
}

fn label_part(p: PartImage) -> PartImage {
    p.sizing(LABEL_SIZING).clip_ends()
}

/// A labelled button's frame between its 3 px end pairs ([`label_edges`]),
/// [`LABEL_WIDTH`] x `h`: framed glass with a sheen falling from the top
/// left, brighter active, its sides left to the pairs. Its left 110 px and
/// right 4 px are fixed and the middle squeezes; a button narrower than both
/// keeps its ends' pixels. The top and bottom 3 rows are fixed and the rows
/// between stretch on a bar of another height. Its lit states here are
/// [`spot_label`]'s frame without the spot.
pub fn label(s: &Settings, h: i32, n: i32) -> PartImage {
    let bar = Bar::new(s);
    let drawn = BUTTON_STATES.map(|st| draw(&bar, &LABELS, look_of(st), LABEL_WIDTH, h, true, n));
    label_part(frames(
        &BUTTON_STATES,
        (LABEL_WIDTH * n, h * n),
        n,
        Layout::Vertical,
        |i, x, y| drawn[i].at(x, y).0,
    ))
}

pub fn spot_label(s: &Settings, h: i32, n: i32) -> SpotFrames {
    let bar = Bar::new(s);
    let drawn = LIT_STATES.map(|st| draw(&bar, &LABELS, look_of(st), LABEL_WIDTH, h, true, n));
    spot_frames(&LIT_STATES, &drawn, n, Layout::Vertical, label_part)
}

#[derive(Clone, Copy)]
enum Pair {
    Start,
    End,
    Left,
    Right,
    /// A resting button's left pair beside a lit one before it: the lit one's light line on its own glass.
    NextLeft,
    /// A resting button's right pair beside a lit one after it: its glass, then the lit one's light line and outline.
    PrevRight,
}

/// The pairs at a labelled button's ends, 3 x `h` each, in the order
/// [`crate::style::taskbar::EDGE_LEFT`] and the constants after it name: per
/// state the left pair where the button follows another and the right pair
/// where another follows it; at rest the row's first pair and last; lit or
/// held, the next button's left pair and the previous one's right pair as
/// they show beside a resting neighbour. A lit or held button's own pairs
/// carry the spot as under the pointer, without a hue. The top and bottom 3
/// rows are fixed as [`label`]'s are.
pub fn label_edges(s: &Settings, h: i32, n: i32) -> PartImage {
    let bar = Bar::new(s);
    let w = LABEL_WIDTH;
    let closed = [Lit::Rest, Lit::Active, Lit::Hot, Lit::ActiveHot]
        .map(|lit| draw(&bar, &LABELS, lit, w, h, false, n));
    let open = [Lit::Rest, Lit::Active, Lit::Hot, Lit::ActiveHot]
        .map(|lit| draw(&bar, &LABELS, lit, w, h, true, n));
    let lit_pairs = [Pair::Left, Pair::Right, Pair::NextLeft, Pair::PrevRight];
    let mut list: Vec<(State, Pair)> = vec![
        (State::Normal, Pair::Left),
        (State::Normal, Pair::Right),
        (State::Normal, Pair::Start),
        (State::Normal, Pair::End),
    ];
    list.extend(lit_pairs.map(|p| (State::Hot, p)));
    list.extend([Pair::Left, Pair::Right].map(|p| (State::Checked, p)));
    list.extend(lit_pairs.map(|p| (State::CheckedHot, p)));
    list.extend(lit_pairs.map(|p| (State::Pressed, p)));
    let states: Vec<State> = list.iter().map(|&(st, _)| st).collect();
    let (wf, hf) = (w as f32, h as f32);
    let image = frames(&states, (3 * n, h * n), n, Layout::Horizontal, |i, x, y| {
        let (st, pair) = list[i];
        let lit = look_of(st) as usize;
        let rest = Lit::Rest as usize;
        let (col, sub) = (x / n, x % n);
        let from = |f: usize, c: i32| closed[f].at(c * n + sub, y);
        let (px, g) = match (pair, col) {
            (Pair::Start, _) => from(rest, col),
            (Pair::End, _) => from(rest, w - 3 + col),
            (Pair::Left, _) => from(lit, 1 + col),
            (Pair::Right, 2) => (
                shared(&closed[lit], &open[lit], sub, y, n),
                LABELS.looks[lit].gain.glass_at(centre(y, n), h),
            ),
            (Pair::Right, _) => from(lit, w - 4 + col),
            (Pair::NextLeft, 0) => (from(lit, 1).0, 0),
            (Pair::NextLeft, _) => (from(rest, 1 + col).0, 0),
            (Pair::PrevRight, 0) => (from(rest, w - 3).0, 0),
            (Pair::PrevRight, _) => (from(lit, w - 3 + col).0, 0),
        };
        let own = matches!(pair, Pair::Left | Pair::Right);
        if own && LIT_STATES.contains(&st) {
            let at = (wf / 2.0).floor() + mid(sub, n);
            spot_lit(px, g, at, mid(y, n), wf, hf)
        } else {
            px
        }
    });
    image.sizing(Insets::new(0, 3, 0, 3))
}

/// Row `y` of the column two abutting labelled buttons share: on the top and
/// bottom lines the open frame's own, else [`SHARED`] of the closed frame's light line over the same share of its outline.
fn shared(closed: &Drawn, open: &Drawn, sub: i32, y: i32, n: i32) -> u32 {
    let w = closed.w;
    if !(2..closed.h / n - 2).contains(&(y / n)) {
        return open.at(w - n + sub, y).0;
    }
    let line = split(faded(closed.at(w - 2 * n + sub, y).0, SHARED));
    let outline = split(faded(closed.at(w - n + sub, y).0, SHARED));
    pack(over(line, outline))
}

/// How far a lit labelled button's own end pairs take on its icon's colour,
/// in levels, at their strongest: under the pointer the glow gathers at the
/// foot ([`GLOW_HOT`]); active and lit it is fainter and fullest halfway down ([`GLOW_HELD`]).
const GLOW_HOT: f64 = 32.0;
const GLOW_HELD: f64 = 10.0;

const GLOW_HOT_TOP: f64 = 0.1;

/// The glow runs down the pair's glass from this many rows below its top to as many above its foot.
const GLOW_INSET: f64 = 2.0;

/// How much of the glow each column of a pair takes: the left pair's two inner
/// columns; the right pair's inner column and a little of its outline.
const GLOW_LEFT_COLUMNS: [f64; 3] = [0.0, 1.0, 1.0];
const GLOW_RIGHT_COLUMNS: [f64; 3] = [1.0, 0.0, 0.6];

/// The glow of a lit or held pair at design column `col` and row `y` (design px down a pair `h` tall), in levels; 0 outside the run.
fn glow(lit: Lit, columns: &[f64; 3], col: usize, y: f64, h: f64) -> f64 {
    let run = h - 1.0 - 2.0 * GLOW_INSET;
    let t = (y - GLOW_INSET) / run;
    if !(0.0..=1.0).contains(&t) {
        return 0.0;
    }
    let along = match lit {
        Lit::Hot => GLOW_HOT * (GLOW_HOT_TOP + (1.0 - GLOW_HOT_TOP) * t),
        _ => GLOW_HELD * (std::f64::consts::PI * t).sin(),
    };
    along * columns.get(col).copied().unwrap_or(0.0)
}

/// [`label_edges`]' `n`th pair in lit or held `state`: a lit button's own pairs
/// ([`EDGE_LEFT`], [`EDGE_RIGHT`]) glow in `c` ([`glow`]), each pixel's channels
/// moved toward `c`'s hue by its glow and kept within its alpha; the pairs it
/// gives its neighbours stay as they are. The glow is worked out at each pixel's middle.
pub fn glowing_label_edge(edges: &PartImage, state: State, n: usize, c: Color) -> PartImage {
    let lit = look_of(state);
    let columns = match (lit, n) {
        (Lit::Hot | Lit::ActiveHot, EDGE_LEFT) => Some(&GLOW_LEFT_COLUMNS),
        (Lit::Hot | Lit::ActiveHot, EDGE_RIGHT) => Some(&GLOW_RIGHT_COLUMNS),
        _ => None,
    };
    let chroma = glow_chroma(c);
    let at = edges.frame_nth(state, n);
    let k = edges.scale;
    let (w, h) = (at.width, at.height);
    let tall = f64::from(h / k);
    let image = frames(&[state], (w, h), k, Layout::Horizontal, |_, x, y| {
        let px = edges.atlas.get(at.x + x, at.y + y);
        let Some(columns) = columns else {
            return px;
        };
        let alpha = px >> 24;
        let t = (glow(lit, columns, (x / k) as usize, centre(y, k), tall) / 255.0) as f32;
        let ch = |sh: u32, d: f32| {
            let v = ((px >> sh) & 0xFF) as f32 + t * d;
            (v.round().max(0.0) as u32).min(alpha) << sh
        };
        alpha << 24 | ch(16, chroma[0]) | ch(8, chroma[1]) | ch(0, chroma[2])
    });
    image.sizing(edges.sizing)
}

/// The amber a task button glows in while its window wants attention.
const AMBER: [f64; 3] = [255.0, 162.0, 42.0];

/// The amber's share of full alpha at the middle of the frame's foot and along its top.
const AMBER_FOOT: f64 = 0.78;
const AMBER_TOP: f64 = 0.30;

/// The share of the middle's amber left at the frame's left and right ends.
const AMBER_ENDS: f64 = 0.55;

/// How much more amber a bar that lets everything through takes than an opaque one, so the glow
/// reads alike over clear and dense glass.
const AMBER_CLEAR: f64 = 0.25;

/// `d`'s resting frame `w` x `h` with amber laid over everything inside its outline: strongest at
/// the middle of its foot, rising toward the foot and fading toward the ends.
fn asking(bar: &Bar, d: &Design, w: i32, h: i32, open: bool, n: i32) -> Drawn {
    let mut f = draw(bar, d, Lit::Rest, w, h, open, n);
    let lift = 1.0 + AMBER_CLEAR * (1.0 - bar.density());
    let (wf, hf) = (f64::from(w), f64::from(h));
    for y in 0..f.h {
        let v = ((centre(y, n) + 0.5) / hf).clamp(0.0, 1.0);
        let along = AMBER_TOP + (AMBER_FOOT - AMBER_TOP) * v * v;
        for x in 0..f.w {
            let u = ((centre(x, n) + 0.5) / wf * 2.0 - 1.0).abs();
            let across = 1.0 - (1.0 - AMBER_ENDS) * u * u;
            let cover = if n == 1 {
                f64::from(u8::from(!matches!(
                    ring(x, y, w, h, open),
                    Ring::Off | Ring::Outline
                )))
            } else {
                rings_at(x, y, w, h, open, n)[1]
            };
            let a = (along * across * lift).min(1.0) * cover;
            let i = (y * f.w + x) as usize;
            let amber = (255.0 * a, AMBER.map(|c| c * a));
            f.px[i] = pack(over(amber, split(f.px[i])));
        }
    }
    f
}

/// `p` with `extra` frames, each the size of `p`'s, laid after its own as `state`.
fn with_frames(mut p: PartImage, state: State, extra: &[Drawn]) -> PartImage {
    let f = p.frame(p.states[0]);
    let (w, h) = (p.atlas.width as i32, p.atlas.height as i32);
    let k = extra.len() as i32;
    let (aw, ah) = match p.layout {
        Layout::Vertical => (w, h + k * f.height),
        Layout::Horizontal => (w + k * f.width, h),
    };
    let mut c = Canvas::new(aw as u32, ah as u32);
    for y in 0..h {
        for x in 0..w {
            c.put_raw(x, y, p.atlas.get(x, y));
        }
    }
    for (i, e) in (0..).zip(extra) {
        let (x0, y0) = match p.layout {
            Layout::Vertical => (0, h + i * f.height),
            Layout::Horizontal => (w + i * f.width, 0),
        };
        for y in 0..e.h {
            for x in 0..e.w {
                c.put_raw(x0 + x, y0 + y, e.at(x, y).0);
            }
        }
    }
    p.atlas = c;
    p.states.extend(extra.iter().map(|_| state));
    p
}

/// [`buttons`] and a [`State::Attention`] frame after them: the resting frame glowing amber ([`asking`]).
pub fn buttons_with_attention(s: &Settings, (w, h): (i32, i32), n: i32) -> PartImage {
    let extra = asking(&Bar::new(s), &ICONS, w, h, false, n);
    with_frames(buttons(s, (w, h), n), State::Attention, &[extra])
}

/// [`label`] and a [`State::Attention`] frame after them, glowing as [`buttons_with_attention`]' does.
pub fn label_with_attention(s: &Settings, h: i32, n: i32) -> PartImage {
    let extra = asking(&Bar::new(s), &LABELS, LABEL_WIDTH, h, true, n);
    with_frames(label(s, h, n), State::Attention, &[extra])
}

/// [`label_edges`] and the [`EDGE_LEFT`] and [`EDGE_RIGHT`] pairs of a [`label_with_attention`]
/// button after them, as [`State::Attention`].
pub fn label_edges_with_attention(s: &Settings, h: i32, n: i32) -> PartImage {
    let bar = Bar::new(s);
    let w = LABEL_WIDTH;
    let (closed, open) = (
        asking(&bar, &LABELS, w, h, false, n),
        asking(&bar, &LABELS, w, h, true, n),
    );
    let pair = |right: bool| {
        let (mut px, mut gain) = (Vec::new(), Vec::new());
        for y in 0..closed.h {
            for x in 0..3 * n {
                let (col, sub) = (x / n, x % n);
                let p = match (right, col) {
                    (false, _) => closed.at((1 + col) * n + sub, y).0,
                    (true, 2) => shared(&closed, &open, sub, y, n),
                    (true, _) => closed.at((w - 4 + col) * n + sub, y).0,
                };
                px.push(p);
                gain.push(0);
            }
        }
        Drawn {
            w: 3 * n,
            h: closed.h,
            px,
            gain,
        }
    };
    let pairs = [pair(false), pair(true)];
    with_frames(label_edges(s, h, n), State::Attention, &pairs)
}

/// The 40 px bar's lit task buttons: [`SPOT`] over the frames of [`buttons`],
/// [`label`] and [`stacks`] without the spot. Held is the active, lit frame.
pub fn task_spot(s: &Settings, n: i32) -> TaskSpot {
    let h = super::BAR_HEIGHT;
    let size = (super::ICON_BUTTON.0, h);
    TaskSpot {
        fit: SPOT,
        button: spot_buttons(s, size, n),
        labelled: Some(spot_label(s, h, n)),
        edges: Some(spot_stacks(s, size, n)),
    }
}

/// The small bar's lit task buttons, as [`task_spot`] on its 44x30 frames; its stacked edges keep their own pairs.
pub(crate) fn small_task_spot(s: &Settings, n: i32) -> TaskSpot {
    let size = (super::SMALL_ICON_BUTTON.0, super::SMALL_TASKBAR_HEIGHT);
    TaskSpot {
        fit: SPOT,
        button: spot_buttons(s, size, n),
        labelled: None,
        edges: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glass::panel::DENSE_CEILING;
    use crate::glass::tint::{self, Surface};
    use crate::glass::{Material, SWATCHES};
    use crate::style::taskbar::{EDGE_NEXT_LEFT, EDGE_PREV_RIGHT, EDGE_ROW_END, EDGE_ROW_START};

    const BUTTON: (i32, i32) = (60, 40);
    const SMALL: (i32, i32) = (44, 30);

    fn settings() -> Vec<Settings> {
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        [SWATCHES[0].color, ruby]
            .into_iter()
            .flat_map(|color| {
                Material::ALL.into_iter().flat_map(move |material| {
                    [42, 75, 100].map(|intensity| Settings {
                        color,
                        material,
                        intensity,
                        ..Settings::default()
                    })
                })
            })
            .collect()
    }

    fn parts(s: &Settings, n: i32) -> Vec<(&'static str, PartImage)> {
        let spot = task_spot(s, n);
        let small = small_task_spot(s, n);
        let labelled = spot.labelled.unwrap();
        let edges = spot.edges.unwrap();
        vec![
            ("buttons", buttons(s, BUTTON, n)),
            ("small buttons", buttons(s, SMALL, n)),
            ("stacks", stacks(s, BUTTON, true, n)),
            ("small stacks", stacks(s, SMALL, false, n)),
            ("pager tiles", pager_tiles(s, BUTTON, n)),
            ("small pager tiles", pager_tiles(s, SMALL, n)),
            ("label", label(s, 40, n)),
            ("label edges", label_edges(s, 40, n)),
            ("lit buttons", spot.button.base),
            ("lit buttons' gain", spot.button.gain),
            ("lit small buttons", small.button.base),
            ("lit small buttons' gain", small.button.gain),
            ("lit label", labelled.base),
            ("lit label's gain", labelled.gain),
            ("lit stacks", edges.base),
            ("lit stacks' gain", edges.gain),
        ]
    }

    fn over_grey(px: u32) -> [i32; 3] {
        let a = (px >> 24) as i32;
        [16, 8, 0].map(|sh| ((px >> sh) & 0xFF) as i32 + ((255 - a) * 128 + 127) / 255)
    }

    fn stack(top: u32, under: u32) -> u32 {
        let mut c = Canvas::new(1, 1);
        c.put_raw(0, 0, under);
        let mut o = Canvas::new(1, 1);
        o.put_raw(0, 0, top);
        c.blit_over(0, 0, &o);
        c.get(0, 0)
    }

    fn at(img: &PartImage, state: State, n: usize, x: i32, y: i32) -> u32 {
        let f = img.frame_nth(state, n);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn bar(s: &Settings) -> u32 {
        pack(tint::plain(s, Surface::Taskbar))
    }

    #[test]
    fn every_frame_is_drawn_at_its_size_on_both_bars() {
        let s = Settings::default();
        let (n, h, c, ch, p) = (
            State::Normal,
            State::Hot,
            State::Checked,
            State::CheckedHot,
            State::Pressed,
        );
        let size = |img: &PartImage| (img.atlas.width, img.atlas.height, img.states.clone());
        for ((w, bh), what) in [(BUTTON, "40 px"), (SMALL, "30 px")] {
            let (w, bh) = (w as u32, bh as u32);
            assert_eq!(
                size(&buttons(&s, (w as i32, bh as i32), 1)),
                (w, 5 * bh, vec![n, h, c, ch, p]),
                "{what} buttons"
            );
            assert_eq!(
                size(&pager_tiles(&s, (w as i32, bh as i32), 1)),
                (w, 3 * bh, vec![h, c, ch]),
                "{what} pager tiles"
            );
            let lit = spot_buttons(&s, (w as i32, bh as i32), 1);
            for img in [&lit.base, &lit.gain] {
                assert_eq!(size(img), (w, 3 * bh, vec![h, ch, p]), "{what} lit");
                assert_eq!(img.sizing, BUTTON_SIZING);
            }
        }
        assert_eq!(
            size(&stacks(&s, BUTTON, true, 1)),
            (40, 40, vec![n, n, h, h, c, c, ch, ch, p, p]),
            "a pair of edges at rest, lit, active, active and lit, held"
        );
        assert_eq!(
            size(&stacks(&s, SMALL, false, 1)),
            (16, 30, vec![n, n, c, c]),
            "the small bar's pairs at rest and active"
        );
        let lit = spot_stacks(&s, BUTTON, 1);
        assert_eq!(size(&lit.base), (24, 40, vec![h, h, ch, ch, p, p]));
        assert_eq!(size(&lit.gain), size(&lit.base));
        let l = label(&s, 40, 1);
        assert_eq!(size(&l), (154, 200, vec![n, h, c, ch, p]), "the label");
        assert_eq!((l.sizing, l.clip_ends), (LABEL_SIZING, true));
        let e = label_edges(&s, 40, 1);
        assert_eq!((e.atlas.width, e.atlas.height), (54, 40), "18 pairs");
        assert_eq!(e.sizing, Insets::new(0, 3, 0, 3));
        assert_eq!(buttons(&s, BUTTON, 1).sizing, BUTTON_SIZING);
    }

    fn fnv(img: &PartImage) -> u64 {
        fnv_pixels(&img.atlas.pixels)
    }

    fn fnv_pixels(pixels: &[u32]) -> u64 {
        pixels.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
    }

    #[test]
    fn sky_frames_keep_their_pixels() {
        let want: [u64; 16] = [
            0xC898_A3F2_E839_2891,
            0x312B_CD0E_711F_590E,
            0x5B07_675D_5B32_F0AE,
            0x4C98_76E0_805D_3649,
            0x3FFE_0746_2707_E545,
            0x8C56_9A24_7E26_0CC2,
            0xE3B2_DFFD_554F_B7B9,
            0x8E54_5716_B56B_8219,
            0x0477_1018_18EE_3909,
            0xDCE9_D0E7_D01C_01BD,
            0xBF2D_2339_C661_A608,
            0x1C14_98F0_BC85_75DD,
            0x0966_BC1C_AC7F_8AF3,
            0x4CD2_D323_E581_F16D,
            0x29BB_C637_1328_B3F2,
            0xE7A7_A468_1402_5435,
        ];
        for ((name, img), want) in parts(&Settings::default(), 1).iter().zip(want) {
            assert_eq!(fnv(img), want, "{name}: the atlas {:#018X}", fnv(img));
        }
        let s = Settings::default();
        let (b, l, e) = (
            buttons(&s, BUTTON, 1),
            label(&s, 40, 1),
            label_edges(&s, 40, 1),
        );
        type Point<'a> = (&'a str, &'a PartImage, State, usize, (i32, i32), u32);
        let points: [Point; 12] = [
            ("the cut corner", &b, State::Normal, 0, (0, 0), 0x0000_0000),
            ("the outline", &b, State::Normal, 0, (30, 0), 0xB100_0B19),
            ("the light line", &b, State::Normal, 0, (30, 1), 0xB4B4_B4B4),
            ("the sheen", &b, State::Normal, 0, (10, 10), 0x7C7C_7C7C),
            (
                "the glass under the sheen",
                &b,
                State::Normal,
                0,
                (30, 30),
                0x0E0E_0E0E,
            ),
            (
                "active, the glass",
                &b,
                State::Checked,
                0,
                (30, 30),
                0x6D6D_6D6D,
            ),
            ("lit, the foot", &b, State::Hot, 0, (30, 37), 0xFAE7_E7E7),
            (
                "held, the foot",
                &b,
                State::Pressed,
                0,
                (30, 37),
                0xFEF2_F2F2,
            ),
            (
                "the label's open side",
                &l,
                State::Normal,
                0,
                (0, 20),
                0x5050_5050,
            ),
            (
                "the label's sheen",
                &l,
                State::Normal,
                0,
                (20, 5),
                0xA3A3_A3A3,
            ),
            (
                "the row's first pair",
                &e,
                State::Normal,
                2,
                (0, 20),
                0x8200_0812,
            ),
            (
                "the shared column",
                &e,
                State::Normal,
                1,
                (2, 20),
                0x3C19_1B1E,
            ),
        ];
        for (name, img, st, n, (x, y), want) in points {
            let got = at(img, st, n, x, y);
            assert_eq!(got, want, "{name} ({x}, {y}): {got:#010X}");
        }
    }

    const OUTLINE: [(i32, i32); 2] = [(30, 0), (59, 20)];
    const LIGHT: [(i32, i32); 3] = [(30, 1), (1, 20), (10, 10)];

    fn veiled(s: &Settings) -> u32 {
        let row = super::super::bar::label_row(s, 40, 1);
        stack(row.atlas.get(super::super::bar::VEIL_END, 20), bar(s))
    }

    /// An outline pixel: what it is, the pixel, the share of what the bar lets
    /// through that its frame's outline covers, how much of that the part keeps
    /// (a stacked edge fades it), and whether it lies over the labelled row's veil.
    type OutlinePx = (String, u32, f64, f64, bool);

    fn outline_pixels(s: &Settings) -> Vec<OutlinePx> {
        let k = |d: &Design, lit: Lit, x, y, (w, h): (i32, i32)| {
            bilinear(
                d.looks[lit as usize].outline,
                f64::from(x),
                f64::from(y),
                w,
                h,
            ) / 255.0
        };
        let mut v: Vec<OutlinePx> = Vec::new();
        for size @ (w, h) in [BUTTON, SMALL] {
            let img = buttons(s, size, 1);
            for (x, y) in [(w / 2, 0), (w - 1, h / 2)] {
                let px = at(&img, State::Normal, 0, x, y);
                let what = format!("the {w}x{h} button ({x}, {y})");
                v.push((what, px, k(&ICONS, Lit::Rest, x, y, size), 1.0, false));
            }
            let edges = stacks(s, size, size == BUTTON, 1);
            for (n, keep) in [(0, STACK_NEAR), (1, STACK_FAR)] {
                let px = at(&edges, State::Normal, n, 3, h / 2);
                let what = format!("the {w}x{h} stacked edge {n} (3, {})", h / 2);
                v.push((
                    what,
                    px,
                    k(&ICONS, Lit::Rest, w - 1, h / 2, size),
                    keep,
                    false,
                ));
            }
        }
        let l = label(s, 40, 1);
        for (st, lit) in [
            (State::Normal, Lit::Rest),
            (State::CheckedHot, Lit::ActiveHot),
        ] {
            for (x, y) in [(20, 0), (130, 0), (20, 39), (130, 39)] {
                let px = at(&l, st, 0, x, y);
                let what = format!("the label {st:?} ({x}, {y})");
                v.push((
                    what,
                    px,
                    k(&LABELS, lit, x, y, (LABEL_WIDTH, 40)),
                    1.0,
                    true,
                ));
            }
        }
        let side = LABELS.looks[0].sides.as_ref().unwrap().outline / 255.0;
        let e = label_edges(s, 40, 1);
        for y in [10, 30] {
            let px = at(&e, State::Normal, EDGE_ROW_START, 0, y);
            v.push((
                format!("the row's first pair (0, {y})"),
                px,
                side,
                1.0,
                true,
            ));
        }
        v
    }

    #[test]
    fn the_outline_covers_a_share_of_what_the_bar_lets_through_and_the_rest_is_white() {
        // The outline is the bar's glass made denser by a share of what the bar
        // under it (the labelled row's veil included) lets through, in its colour,
        // until the ceiling; light line, sheen and glass are the same white over any
        // bar. The bar's alpha, the veil and the outline are each rounded, hence 1.5.
        let sky = buttons(&Settings::default(), BUTTON, 1);
        let mut capped = 0;
        for s in settings() {
            let what_s = format!("{:?} {:?} at {}", s.color, s.material, s.intensity);
            for (what, px, k, keep, over_veil) in outline_pixels(&s) {
                let under = if over_veil { veiled(&s) } else { bar(&s) };
                let a = f64::from(under >> 24);
                let room = (DENSE_CEILING - a).max(0.0) / (255.0 - a);
                if k > room {
                    capped += 1;
                }
                let want = a + (255.0 - a) * keep * k.min(room);
                let px = stack(px, under);
                let got = f64::from(px >> 24);
                assert!(
                    (got - want).abs() <= 1.5,
                    "{what_s}, {what}: the outline covers {keep} of {k:.2} of what the bar lets through, up to {DENSE_CEILING}: alpha {want:.1}, got {got}"
                );
                for sh in [16, 8, 0] {
                    let (g, b) = ((px >> sh) & 0xFF, (under >> sh) & 0xFF);
                    assert!(
                        g.abs_diff(b) <= 1,
                        "{what_s}, {what}: the outline keeps the glass's colour, {under:#010x} within a level, got {px:#010x}"
                    );
                }
            }
            let img = buttons(&s, BUTTON, 1);
            for (x, y) in LIGHT {
                assert_eq!(
                    at(&img, State::Normal, 0, x, y),
                    at(&sky, State::Normal, 0, x, y),
                    "{what_s} ({x}, {y}): the same white"
                );
            }
        }
        assert!(
            capped > 0,
            "intensity 100 takes some outlines to the ceiling"
        );
    }

    #[test]
    fn a_labelled_outline_over_the_rows_veil_stops_at_the_ceiling() {
        for s in settings().into_iter().filter(|s| s.intensity == 100) {
            let what_s = format!("{:?} {:?}", s.color, s.material);
            let under = veiled(&s);
            let (l, e) = (label(&s, 40, 1), label_edges(&s, 40, 1));
            let mut outline: Vec<(String, u32)> = Vec::new();
            for st in BUTTON_STATES {
                for x in 2..LABEL_WIDTH - 2 {
                    for y in [0, 39] {
                        outline.push((format!("the label {st:?} ({x}, {y})"), at(&l, st, 0, x, y)));
                    }
                }
            }
            for y in 2..38 {
                let first = at(&e, State::Normal, EDGE_ROW_START, 0, y);
                let last = at(&e, State::Normal, EDGE_ROW_END, 2, y);
                outline.push((format!("the row's first pair (0, {y})"), first));
                outline.push((format!("the row's last pair (2, {y})"), last));
            }
            let mut top = 0;
            for (what, px) in outline {
                let got = stack(px, under) >> 24;
                top = top.max(got);
                assert!(
                    f64::from(got) <= DENSE_CEILING + 1.0,
                    "{what_s}, {what}: over the veil it stops at {DENSE_CEILING}, got {got}"
                );
            }
            assert!(
                f64::from(under >> 24) >= DENSE_CEILING - 1.0
                    || f64::from(top) >= DENSE_CEILING - 1.0,
                "{what_s}: the active, lit outline reaches the ceiling, got {top}"
            );
        }
    }

    #[test]
    fn an_active_buttons_line_rounds_its_foot_corners_and_a_lit_one_has_an_inner_column() {
        let bar = Bar::new(&Settings::default());
        for (w, h) in [BUTTON, SMALL] {
            let alpha = |lit, x, y| draw(&bar, &ICONS, lit, w, h, false, 1).at(x, y).0 >> 24;
            for x in [2, w - 3] {
                let beside = if x == 2 { 3 } else { w - 4 };
                let (corner, glass) = (
                    alpha(Lit::Active, x, h - 3),
                    alpha(Lit::Active, beside, h - 3),
                );
                assert!(
                    corner >= glass + 20,
                    "{w}x{h} active ({x}, {}): the foot corner {corner} lighter than the glass {glass} by 20",
                    h - 3
                );
                assert_eq!(
                    alpha(Lit::Active, x, h / 2),
                    alpha(Lit::Active, beside, h / 2),
                    "{w}x{h} active ({x}, {}): no column above the corner",
                    h / 2
                );
                for lit in [Lit::Hot, Lit::ActiveHot] {
                    for y in h / 2..h - 3 {
                        let (col, glass) = (alpha(lit, x, y), alpha(lit, beside, y));
                        assert!(
                            col >= glass + 20,
                            "{w}x{h} {lit:?} ({x}, {y}): the inner column {col} lighter than the glass {glass} by 20"
                        );
                    }
                }
            }
            for lit in [Lit::Hot, Lit::ActiveHot] {
                for x in 3..w - 3 {
                    let (foot, glass) = (alpha(lit, x, h - 3), alpha(lit, x, h - 4));
                    assert!(
                        foot >= glass + 20,
                        "{w}x{h} {lit:?} ({x}, {}): the inner row {foot} lighter than the glass {glass} by 20",
                        h - 3
                    );
                }
            }
        }
    }

    #[test]
    fn a_button_reads_darker_over_smoked_and_lighter_over_molten() {
        let lum = |material| {
            let s = Settings {
                material,
                ..Settings::default()
            };
            let px = stack(
                at(&buttons(&s, BUTTON, 1), State::Normal, 0, 30, 30),
                bar(&s),
            );
            over_grey(px).iter().sum::<i32>()
        };
        let (frosted, smoked, molten) = (
            lum(Material::Frosted),
            lum(Material::Smoked),
            lum(Material::Molten),
        );
        assert!(
            smoked < frosted && frosted < molten,
            "{smoked} < {frosted} < {molten}"
        );
    }

    #[test]
    fn with_transparency_off_the_frames_lie_over_the_opaque_bar_as_they_are() {
        for s in settings() {
            let off = Settings {
                transparency: false,
                ..s
            };
            let bar = super::super::bar::taskbar(&off, 40, 1);
            let under = bar.atlas.get(105, 20);
            assert_eq!(under >> 24, 0xFF, "the bar is opaque");
            for ((name, on), (_, opaque)) in parts(&s, 1).into_iter().zip(parts(&off, 1)) {
                assert_eq!(
                    on.atlas.pixels, opaque.atlas.pixels,
                    "{name}: the same frames, {:?} {:?} at {}",
                    s.color, s.material, s.intensity
                );
            }
            let b = buttons(&off, BUTTON, 1);
            for (x, y) in OUTLINE.into_iter().chain(LIGHT) {
                assert_eq!(
                    stack(at(&b, State::Normal, 0, x, y), under) >> 24,
                    0xFF,
                    "({x}, {y}) over it is opaque"
                );
            }
        }
    }

    #[test]
    fn the_spots_gain_lies_on_a_lit_frame_by_its_look() {
        // In 32nds of a layer: lit, even down the glass; active and lit, rising
        // from the top of the glass to its foot, none on the light line.
        let s = Settings::default();
        let points = [
            ("the light line along the top", (30, 1)),
            ("the light line down the side", (58, 20)),
            ("the inner column", (57, 20)),
            ("the inner row at the foot", (30, 37)),
            ("the glass at the top", (30, 2)),
            ("the glass halfway down", (30, 20)),
            ("the glass at the foot", (30, 36)),
        ];
        let want = [
            (State::Hot, [69, 69, 44, 44, 37, 37, 37]),
            (State::CheckedHot, [0, 0, 67, 67, 16, 35, 52]),
            (State::Pressed, [0, 0, 67, 67, 16, 35, 52]),
        ];
        let gain = spot_buttons(&s, BUTTON, 1).gain;
        for (st, want) in want {
            for ((what, (x, y)), want) in points.into_iter().zip(want) {
                let got = at(&gain, st, 0, x, y) & 0xFF;
                assert_eq!(got, want, "{st:?}, {what} ({x}, {y}): gain {want}");
            }
        }
        let label = spot_label(&s, 40, 1).gain;
        for (st, (x, y), want) in [
            (State::Hot, (60, 1), 45),
            (State::Hot, (60, 20), 33),
            (State::CheckedHot, (60, 1), 0),
            (State::CheckedHot, (60, 2), 13),
            (State::CheckedHot, (60, 36), 44),
        ] {
            let got = at(&label, st, 0, x, y) & 0xFF;
            assert_eq!(got, want, "the label {st:?} ({x}, {y}): gain {want}");
        }
    }

    #[test]
    fn the_spots_gain_follows_the_frames_shape() {
        let s = Settings::default();
        let spot = task_spot(&s, 1);
        let small = small_task_spot(&s, 1);
        let (label, edges) = (spot.labelled.unwrap(), spot.edges.unwrap());
        for (what, f, d, open) in [
            ("button", &spot.button, &ICONS, false),
            ("small button", &small.button, &ICONS, false),
            ("label", &label, &LABELS, true),
            ("edges", &edges, &ICONS, false),
        ] {
            assert_eq!(f.base.states, f.gain.states, "{what}");
            let n = f.base.states.len();
            for i in 0..n {
                let st = f.base.states[i];
                let nth = f.base.states[..i].iter().filter(|&&o| o == st).count();
                let r = f.base.frame_nth(st, nth);
                let look = &d.looks[look_of(st) as usize];
                for y in 0..r.height {
                    for x in 0..r.width {
                        let base = at(&f.base, st, nth, x, y);
                        let g = at(&f.gain, st, nth, x, y);
                        assert_eq!(
                            g >> 8,
                            0xFF_0000,
                            "{what}: the gain is opaque, in its low byte"
                        );
                        let g = (g & 0xFF) as u8;
                        if base == 0 {
                            assert_eq!(g, 0, "{what} {st:?} ({x}, {y}): no gain off the frame");
                            continue;
                        }
                        let (fx, fw) = if what == "edges" {
                            (56 + x, 60)
                        } else {
                            (x, r.width)
                        };
                        let h = r.height;
                        let inner = matches!(look.inner, Inner::Column(_))
                            && (fx == 2 || fx == fw - 3 || y == h - 3);
                        let want = match ring(fx, y, fw, h, open) {
                            Ring::Off | Ring::Outline => 0,
                            Ring::Line => look.gain.line,
                            Ring::Glass if inner => look.gain.inner,
                            Ring::Glass => look.gain.glass_at(f64::from(y), h),
                        };
                        assert_eq!(g, want, "{what} {st:?} ({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn a_lit_frame_without_a_pointer_lifts_it_in_grey_brightest_at_its_foot() {
        let s = Settings::default();
        let (full, base) = (buttons(&s, BUTTON, 1), spot_buttons(&s, BUTTON, 1).base);
        let under = bar(&s);
        let lift = |x, y| {
            let lit = over_grey(stack(at(&full, State::Hot, 0, x, y), under));
            let unlit = over_grey(stack(at(&base, State::Hot, 0, x, y), under));
            (lit, unlit)
        };
        for (x, y) in [(30, 20), (30, 30), (10, 36), (50, 36), (30, 37)] {
            let (lit, unlit) = lift(x, y);
            assert!(
                (0..3).all(|i| lit[i] >= unlit[i]) && lit[2] - lit[0] <= unlit[2] - unlit[0],
                "({x}, {y}): lit {lit:?} over {unlit:?}"
            );
        }
        let gain = |(x, y)| {
            let (lit, unlit) = lift(x, y);
            lit[1] - unlit[1]
        };
        assert!(gain((30, 36)) > gain((30, 10)), "brighter at the foot");
    }

    #[test]
    fn a_lit_labelled_buttons_own_pairs_take_on_its_icons_colour() {
        let s = Settings::default();
        let edges = label_edges(&s, 40, 1);
        let (red, blue) = (Color::rgb(0xE0, 0x20, 0x20), Color::rgb(0x20, 0x40, 0xE0));
        let lean = |img: &PartImage, st| -> i32 {
            (0..3)
                .flat_map(|x| (0..40).map(move |y| (x, y)))
                .map(|(x, y)| {
                    let [r, _, b] = over_grey(at(img, st, 0, x, y));
                    r - b
                })
                .sum()
        };
        for st in [State::Hot, State::CheckedHot, State::Pressed] {
            for n in [EDGE_LEFT, EDGE_RIGHT] {
                let one = glowing_label_edge(&edges, st, n, red);
                assert_eq!(one.states, [st], "{st:?} pair {n}: one frame");
                let f = edges.frame_nth(st, n);
                let plain = frames(&[st], (3, 40), 1, Layout::Horizontal, |_, x, y| {
                    edges.atlas.get(f.x + x, f.y + y)
                });
                let plain = lean(&plain, st);
                assert!(lean(&one, st) > plain, "{st:?} pair {n}: red leans red");
                let one = glowing_label_edge(&edges, st, n, blue);
                assert!(lean(&one, st) < plain, "{st:?} pair {n}: blue leans blue");
            }
            for n in [EDGE_NEXT_LEFT, EDGE_PREV_RIGHT] {
                let one = glowing_label_edge(&edges, st, n, red);
                let f = edges.frame_nth(st, n);
                for (x, y) in [(0, 1), (1, 20), (2, 38)] {
                    assert_eq!(
                        at(&one, st, 0, x, y),
                        edges.atlas.get(f.x + x, f.y + y),
                        "{st:?} pair {n} ({x}, {y}): a pair given to a neighbour keeps its own"
                    );
                }
            }
        }
    }

    #[test]
    fn the_spots_frames_are_laid_out_as_their_parts() {
        let s = Settings::default();
        let spot = task_spot(&s, 1);
        let small = small_task_spot(&s, 1);
        let edges = spot.edges.as_ref().unwrap();
        let labelled = spot.labelled.as_ref().unwrap();
        for (what, f, part) in [
            ("button", &spot.button, buttons(&s, BUTTON, 1)),
            ("small button", &small.button, buttons(&s, SMALL, 1)),
            ("labelled", labelled, label(&s, 40, 1)),
            ("edges", edges, stacks(&s, BUTTON, true, 1)),
        ] {
            for img in [&f.base, &f.gain] {
                assert_eq!(img.sizing, part.sizing, "{what}: sized as the part");
                assert_eq!(img.clip_ends, part.clip_ends, "{what}: ends as the part");
                let (a, b) = (img.frame(State::Hot), part.frame(State::Hot));
                assert_eq!((a.width, a.height), (b.width, b.height), "{what}");
            }
            assert!(
                f.base.states.iter().all(|st| LIT_STATES.contains(st)),
                "{what}: lit and held frames only"
            );
            let (a, b) = (
                f.base.frame(State::CheckedHot),
                f.base.frame(State::Pressed),
            );
            for (x, y) in [(1, 1), (2, 20), (3, 29)] {
                assert_eq!(
                    f.base.atlas.get(a.x + x, a.y + y),
                    f.base.atlas.get(b.x + x, b.y + y),
                    "{what}: held is the active, lit frame at ({x}, {y})"
                );
            }
            if what == "labelled" || what == "edges" {
                for st in LIT_STATES {
                    let (a, b) = (f.base.frame(st), part.frame(st));
                    for (x, y) in [(1, 1), (2, 20), (3, 29)] {
                        assert_eq!(
                            f.base.atlas.get(a.x + x, a.y + y),
                            part.atlas.get(b.x + x, b.y + y),
                            "{what} {st:?}: the part's own frame at ({x}, {y})"
                        );
                    }
                }
            }
        }
        assert!(
            small.edges.is_none(),
            "the small bar's edges keep their own pairs"
        );
    }

    fn in_corner(x: i32, y: i32, w: i32, h: i32) -> bool {
        let near = |v: i32, len: i32| v < 2 || v >= len - 2;
        near(x, w) && near(y, h)
    }

    /// Whether row `y` lies within a pixel and a half of `d`'s sheen's lower
    /// edge on column `x`: there the edge is placed within the pixel at scale 2,
    /// so the glass beside it runs past the scale-1 pixels round it.
    fn by_sheen_edge(d: &Design, x: i32, y: i32, w: i32, h: i32) -> bool {
        let sh = &d.shape;
        let (wf, xf) = (f64::from(w), f64::from(x) + 0.5);
        let (c0, c1) = sh.cut;
        let edge = (sh.band
            + (sh.deep - sh.band) * (1.0 - smooth((xf - c0 * wf) / ((c1 - c0) * wf))))
            * f64::from(h);
        (f64::from(y) + 0.5 - edge).abs() < 1.5
    }

    /// Whether design pixel (`x`, `y`) of an icon frame is its inner column or
    /// row ([`Inner`]) or the glass beside them: that glass runs a quarter pixel
    /// past its scale-1 pixel toward the brighter column at scale 2.
    fn by_inner(x: i32, y: i32, w: i32, h: i32) -> bool {
        [2, 3, w - 4, w - 3].contains(&x) || [h - 4, h - 3].contains(&y)
    }

    #[test]
    fn every_frame_at_scale_2_is_its_scale_1_design_twice_over() {
        // Each frame drawn at 1 and painted and drawn at 2, as `part_twice_over`
        // holds them: buttons, pager tile, stacked edges, labelled frame and end
        // pairs, lit frames without the spot and their gain. Left out: round corners
        // (`a_frames_corners_at_scale_2_are_round_and_antialiased`), rows the sheen's
        // lower edge crosses (`by_sheen_edge`) and an icon frame's inner column and
        // row (`by_inner`).
        use crate::look::styled::fixture::part_twice_over;
        let ruby = SWATCHES.iter().find(|w| w.name == "ruby").unwrap().color;
        for s in [
            Settings::default(),
            Settings {
                color: ruby,
                material: Material::Molten,
                intensity: 100,
                ..Settings::default()
            },
        ] {
            let pairs = parts(&s, 1).into_iter().zip(parts(&s, 2));
            for ((name, one), (_, two)) in pairs {
                let f = one.frame(one.states[0]);
                let (w, h) = (f.width, f.height);
                let (design, from_w, at_x) = match name {
                    n if n.contains("label edges") => (&LABELS, LABEL_WIDTH, None),
                    n if n.contains("label") => (&LABELS, LABEL_WIDTH, Some(0)),
                    n if n.contains("small stacks") => (&ICONS, SMALL.0, Some(SMALL.0 - 4)),
                    n if n.contains("stacks") => (&ICONS, BUTTON.0, Some(BUTTON.0 - 4)),
                    _ => (&ICONS, w, Some(0)),
                };
                let skip = |_, _, x: i32, y: i32| {
                    let edge = match at_x {
                        Some(x0) => by_sheen_edge(design, x0 + x, y, from_w, h),
                        None => [x, from_w - 3 + x]
                            .iter()
                            .any(|&fx| by_sheen_edge(design, fx, y, from_w, h)),
                    };
                    let fx = at_x.unwrap_or(0) + x;
                    let inner = !name.contains("label") && by_inner(fx, y, from_w, h);
                    in_corner(x, y, w, h) || edge || inner
                };
                let got = part_twice_over(&one, &two, (w, h), skip);
                assert_eq!(got, Ok(()), "{:?} {name}", s.material);
            }
        }
    }

    #[test]
    fn sky_frames_at_scale_2_keep_their_pixels() {
        let want: [u64; 16] = [
            0x5540_2357_F950_6797,
            0x0422_AF69_8F22_115F,
            0xBA40_3C3F_0604_411A,
            0x4ABB_D6E5_5160_4032,
            0xBFA6_9FA8_A674_368C,
            0x03D3_A08D_BE0E_8379,
            0x7720_CECA_1356_5A71,
            0xE0FC_57E7_1AF5_3D2F,
            0x0686_BB2F_6173_7CF1,
            0xB170_C622_1118_450D,
            0x2922_F3F2_5FE7_A8E7,
            0x5C7A_D473_7D13_FBED,
            0x87D9_CEC7_3F71_FE61,
            0x12BB_7CB0_7587_7B45,
            0x22D1_A678_2DC5_0655,
            0x0F8B_4437_ADF2_EC05,
        ];
        let got: Vec<u64> = parts(&Settings::default(), 2)
            .iter()
            .map(|(_, img)| fnv(img))
            .collect();
        assert_eq!(got, want, "{got:#018X?}");
    }

    #[test]
    fn a_frames_corners_at_scale_2_are_round_and_antialiased() {
        // The cut corner's curve, radius 2 design px, 4 at scale 2: clear where a
        // pixel lies wholly outside it, covered where wholly inside, partly along it
        // in several levels, the four corners cut alike. At scale 1 the pixel is cut whole.
        let s = Settings::default();
        let r = 4;
        for (name, img) in [
            ("buttons", buttons_with_attention(&s, BUTTON, 2)),
            ("small buttons", buttons_with_attention(&s, SMALL, 2)),
            ("pager tiles", pager_tiles(&s, BUTTON, 2)),
        ] {
            for &st in &img.states {
                let f = img.frame(st);
                let alpha = |x: i32, y: i32| img.atlas.get(f.x + x, f.y + y) >> 24;
                let (w, h) = (f.width, f.height);
                let mut partial = std::collections::BTreeSet::new();
                for y in 0..r {
                    for x in 0..r {
                        let (dx, dy) = (f64::from(r - x) - 0.5, f64::from(r - y) - 0.5);
                        let dist = dx.hypot(dy);
                        let a = alpha(x, y);
                        let what = format!("{name} {st:?} ({x}, {y}): alpha {a}");
                        if dist >= f64::from(r) + 0.71 {
                            assert_eq!(a, 0, "{what}: outside the curve");
                        } else if dist <= f64::from(r) - 0.71 {
                            assert!(a > 0, "{what}: inside the curve");
                        } else {
                            partial.insert(a);
                        }
                        let mirrored = [
                            alpha(w - 1 - x, y),
                            alpha(x, h - 1 - y),
                            alpha(w - 1 - x, h - 1 - y),
                        ];
                        assert_eq!(
                            mirrored.map(|m| m == 0),
                            [a == 0; 3],
                            "{what}: the four corners cut alike"
                        );
                    }
                }
                assert!(
                    partial.len() >= 3,
                    "{name} {st:?}: the curve's edge in several levels, not a step: {partial:?}"
                );
            }
        }
        let one = buttons(&s, BUTTON, 1);
        assert_eq!(one.atlas.get(0, 0), 0, "scale 1: the corner pixel cut");
        assert_ne!(one.atlas.get(1, 0), 0, "and the next one whole");
    }

    fn attention_parts(s: &Settings, n: i32) -> Vec<(&'static str, PartImage)> {
        vec![
            ("buttons", buttons_with_attention(s, BUTTON, n)),
            ("small buttons", buttons_with_attention(s, SMALL, n)),
            ("label", label_with_attention(s, 40, n)),
            ("label edges", label_edges_with_attention(s, 40, n)),
        ]
    }

    fn frame_pixels(img: &PartImage, state: State, k: usize) -> Vec<u32> {
        let f = img.frame_nth(state, k);
        (f.y..f.bottom())
            .flat_map(|y| (f.x..f.right()).map(move |x| img.atlas.get(x, y)))
            .collect()
    }

    #[test]
    fn attention_frames_pin_at_scale_1_and_2() {
        let mut got = Vec::new();
        for s in settings() {
            for n in [1, 2] {
                for (name, img) in attention_parts(&s, n) {
                    let at = |k| frame_pixels(&img, State::Attention, k);
                    let px: Vec<u32> = if name == "label edges" {
                        [at(0), at(1)].concat()
                    } else {
                        at(0)
                    };
                    let key = format!("{:?} {:?} {} {name} {n}", s.color, s.material, s.intensity);
                    got.push((key, fnv_pixels(&px)));
                }
            }
        }
        crate::pins::check("glass/task_attention", &got);
    }

    #[test]
    fn the_attention_frames_leave_the_others_as_they_are() {
        let s = Settings::default();
        for n in [1, 2] {
            for (plain, asking) in [
                (
                    buttons(&s, BUTTON, n),
                    buttons_with_attention(&s, BUTTON, n),
                ),
                (label(&s, 40, n), label_with_attention(&s, 40, n)),
                (
                    label_edges(&s, 40, n),
                    label_edges_with_attention(&s, 40, n),
                ),
            ] {
                assert_eq!(asking.states[..plain.states.len()], plain.states[..]);
                let k = |st: State| plain.states.iter().filter(|&&x| x == st).count();
                for &st in &plain.states {
                    for i in 0..k(st) {
                        assert_eq!(frame_pixels(&asking, st, i), frame_pixels(&plain, st, i));
                    }
                }
                assert_eq!(
                    asking.sizing, plain.sizing,
                    "the same nine-grid insets at {n}"
                );
            }
        }
    }

    #[test]
    fn an_attention_frame_glows_amber_strongest_at_its_foot() {
        let unpremul = |px: u32| {
            let a = (px >> 24).max(1) as f64;
            [16, 8, 0].map(|sh| f64::from((px >> sh) & 0xFF) * 255.0 / a)
        };
        for s in settings() {
            for n in [1, 2] {
                let b = buttons_with_attention(&s, BUTTON, n);
                let at = |st, x: i32, y: i32| {
                    let f = b.frame(st);
                    b.atlas.get(f.x + x * n, f.y + y * n)
                };
                let foot = unpremul(at(State::Attention, 30, 36));
                let rest = unpremul(at(State::Normal, 30, 36));
                assert!(
                    foot[0] > foot[1] + 40.0 && foot[1] > foot[2] + 40.0,
                    "{:?} {:?}: amber at the foot, {foot:?} at {n}",
                    s.color,
                    s.material
                );
                assert!(
                    foot[0] - foot[2] > rest[0] - rest[2] + 60.0,
                    "warmer than at rest"
                );
                let top = unpremul(at(State::Attention, 30, 3));
                assert!(
                    foot[0] - foot[2] > top[0] - top[2],
                    "stronger at the foot than the top"
                );
                for st in [State::Normal, State::Checked] {
                    assert_ne!(
                        frame_pixels(&b, State::Attention, 0),
                        frame_pixels(&b, st, 0)
                    );
                }
            }
        }
    }

    #[test]
    fn clearer_glass_takes_more_amber() {
        let alpha_gain = |s: &Settings| {
            let b = buttons_with_attention(s, BUTTON, 1);
            let a = |st| at(&b, st, 0, 30, 30) >> 24;
            a(State::Attention) - a(State::Normal)
        };
        let dense = Settings {
            intensity: 100,
            ..Settings::default()
        };
        let clear = Settings {
            intensity: 0,
            ..Settings::default()
        };
        assert!(Bar::new(&clear).density() < Bar::new(&dense).density());
        assert!(alpha_gain(&clear) > alpha_gain(&dense));
    }

    #[test]
    fn a_labelled_attention_buttons_end_pairs_glow_with_it() {
        let s = Settings::default();
        let e = label_edges_with_attention(&s, 40, 1);
        for k in [EDGE_LEFT, EDGE_RIGHT] {
            assert_ne!(
                frame_pixels(&e, State::Attention, k),
                frame_pixels(&e, State::Normal, k),
                "pair {k}"
            );
        }
        let f = e.frame_nth(State::Attention, EDGE_LEFT);
        let p = e.atlas.get(f.x + 2, f.y + 30);
        assert!(
            (p >> 16 & 0xFF) > (p & 0xFF) + 40,
            "amber in the left pair: {p:08x}"
        );
    }
}
