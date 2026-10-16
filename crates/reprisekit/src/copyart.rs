//! The copy dialog's pictures: sheets of paper flying between open folders and the Recycle Bin
//! ([`paint_flight`], coloured by a look's [`Palette`]), and what a look's throughput graph is
//! drawn from ([`Graph`]).

use crate::raster::Canvas;
use crate::shape::{self, Px, Stops, over, polygon, premul, rounded, stroke};
use crate::theme::{Color, ColorScheme};

pub const STRIP_H: i32 = 60;
/// One sheet's trip from folder to folder.
pub const CYCLE_MS: u64 = 1500;
const IN_FLIGHT: u64 = 3;
const SHEETS: f64 = 6.0;
const FOLDER: (f64, f64) = (44.0, 36.0);
const FOLDER_X: f64 = 18.0;
const FOLDER_Y: f64 = 22.0;
/// How far above the folders' mouths the sheets' arc peaks.
const LIFT: f64 = 24.0;
const SHEET: (f64, f64) = (12.0, 15.0);
/// How far below an end's top a sheet leaves or lands: in a folder's mouth, down in the bin's,
/// and in the air where nothing stands.
const FOLDER_MOUTH: f64 = 12.0;
const BIN_MOUTH: f64 = 10.0;
const GONE_MOUTH: f64 = 6.0;
/// The share of the trip after which a sheet bound nowhere fades, and how small it ends.
const FADE_FROM: f64 = 0.55;
const FADE_SHRINK: f64 = 0.4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Preparing,
    Flying,
    Finishing,
}

/// What stands at one end of the flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum End {
    #[default]
    Folder,
    /// The Recycle Bin.
    Bin,
    /// Nothing: the sheets fade away where an end would stand.
    Gone,
}

/// One frame of the flight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flight {
    /// Milliseconds the sheets have flown; a paused copy holds it.
    pub ms: u64,
    pub fraction: f64,
    /// A move empties the left folder as the right one fills.
    pub moving: bool,
    pub stage: Stage,
    /// Where the sheets leave from and land.
    pub from: End,
    pub to: End,
}

/// The flight's colours. A flat look gives each gradient one colour twice and no shadow or trail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub back: [Color; 2],
    pub front: [Color; 2],
    pub rim: Color,
    pub paper: Color,
    pub paper_rim: Color,
    pub ink: Color,
    /// A sheet's shadow's opacity.
    pub shadow: u8,
    /// The dotted arc the sheets fly along, and the glow behind the right folder while it finishes.
    pub trail: Option<Color>,
    pub gloss: bool,
    /// The Recycle Bin's can, left to right across it.
    pub bin: [Color; 2],
    pub bin_rim: Color,
    /// The dark inside seen through the bin's mouth.
    pub bin_inside: Color,
    pub bin_rib: Color,
}

/// Paints the strip `w` design pixels wide at `d`, as [`paint_flight`].
pub type FlightPainter = fn(&Flight, &ColorScheme, i32, i32) -> Canvas;

/// A throughput graph: `samples[i]` is the bytes a second read as the put passed `i` percent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Graph<'a> {
    pub samples: &'a [f32],
    pub fraction: f64,
    pub rate: f64,
    pub accent: Color,
}

/// Paints a graph `w` x `h` design pixels at `d`, as [`paint_flight`] paints the strip.
pub type GraphPainter = fn(&Graph, &ColorScheme, i32, i32, i32) -> Canvas;

/// A look that shows the copy dialog as a page with a throughput graph: the graph, the large
/// heading's ink, and the button band's fill and the rule over it.
#[derive(Clone, Copy, Debug)]
pub struct GraphLook {
    pub paint: GraphPainter,
    pub heading: Color,
    pub band: Color,
    pub band_rule: Color,
}

impl Graph<'_> {
    /// The rate the graph's top stands for: room above the fastest rate seen.
    pub fn peak(&self) -> f64 {
        let seen = self
            .samples
            .iter()
            .fold(0.0f64, |m, &s| m.max(f64::from(s)));
        (seen.max(self.rate) * 1.25).max(1.0)
    }

    /// The design-pixel row of the current rate's rule in a graph `h` tall.
    pub fn rule_y(&self, h: i32) -> f64 {
        let h = f64::from(h - 2);
        1.0 + h - (self.rate / self.peak()).clamp(0.0, 1.0) * h
    }

    /// The rate at `t` (0..1) of the way across, straight between samples.
    pub fn rate_at(&self, t: f64) -> f64 {
        let n = self.samples.len();
        if n == 0 {
            return 0.0;
        }
        let x = t.clamp(0.0, 1.0) * (n - 1) as f64;
        let i = (x.floor() as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        let k = x - i as f64;
        f64::from(self.samples[i]) * (1.0 - k) + f64::from(self.samples[j]) * k
    }
}

/// Device pixels painted shape by shape, each only over its bounding box.
struct Layer {
    w: i32,
    h: i32,
    d: f64,
    px: Vec<Px>,
}

impl Layer {
    fn new(w: i32, h: i32, d: i32) -> Layer {
        let (w, h) = (w * d, h * d);
        Layer {
            w,
            h,
            d: f64::from(d),
            px: vec![shape::CLEAR; (w * h).max(0) as usize],
        }
    }

    /// Lay `f` over the device pixels inside design box `(x0, y0, x1, y1)`; `f` takes a pixel's
    /// top-left corner in device pixels.
    fn paint(&mut self, (x0, y0, x1, y1): (f64, f64, f64, f64), f: impl Fn(f64, f64) -> Px) {
        let span = |a: f64, b: f64, n: i32| {
            let lo = ((a * self.d).floor() as i32).clamp(0, n);
            let hi = ((b * self.d).ceil() as i32).clamp(0, n);
            lo..hi
        };
        for y in span(y0, y1, self.h) {
            for x in span(x0, x1, self.w) {
                let top = f(f64::from(x), f64::from(y));
                if top.0 > 0.0 {
                    let i = (y * self.w + x) as usize;
                    self.px[i] = over(top, self.px[i]);
                }
            }
        }
    }

    fn canvas(self) -> Canvas {
        let mut c = Canvas::new(self.w as u32, self.h as u32);
        for (i, &p) in self.px.iter().enumerate() {
            c.put_raw(i as i32 % self.w, i as i32 / self.w, shape::pack(p));
        }
        c
    }
}

fn ink(c: Color, a: u8, cov: f64) -> Px {
    shape::scale(premul(c, a), cov)
}

/// A gradient from `pair[0]` at design row `y0` to `pair[1]` at `y1`, read at device row `y`.
fn down(pair: [Color; 2], y0: f64, y1: f64, y: f64, d: f64) -> Px {
    let stops = [(0.0, premul(pair[0], 255)), (1.0, premul(pair[1], 255))];
    Stops(&stops).at((shape::centre(y as i32, d as i32) - y0) / (y1 - y0))
}

/// The four corners of a `w` x `h` sheet centred on `c`, turned by `a` radians and scaled by `k`,
/// in device pixels.
fn sheet_corners(c: (f64, f64), (w, h): (f64, f64), a: f64, k: f64, d: f64) -> [(f64, f64); 4] {
    let (s, co) = a.sin_cos();
    let at = |x: f64, y: f64| {
        let (x, y) = (x * k, y * k);
        ((c.0 + x * co - y * s) * d, (c.1 + x * s + y * co) * d)
    };
    let (hw, hh) = (w / 2.0, h / 2.0);
    [at(-hw, -hh), at(hw, -hh), at(hw, hh), at(-hw, hh)]
}

fn bbox(pts: &[(f64, f64)], d: f64, m: f64) -> (f64, f64, f64, f64) {
    pts.iter().fold(
        (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
        |(x0, y0, x1, y1), &(x, y)| {
            (
                x0.min(x / d - m),
                y0.min(y / d - m),
                x1.max(x / d + m),
                y1.max(y / d + m),
            )
        },
    )
}

/// Where a sheet `t` (0..1) of the way through its trip from folder to folder is: centre, turn
/// and size.
pub fn sheet_at(t: f64, w: i32) -> ((f64, f64), f64, f64) {
    path_at(t, w, End::Folder, End::Folder)
}

fn mouth(e: End) -> f64 {
    FOLDER_Y
        + match e {
            End::Folder => FOLDER_MOUTH,
            End::Bin => BIN_MOUTH,
            End::Gone => GONE_MOUTH,
        }
}

fn path_at(t: f64, w: i32, from: End, to: End) -> ((f64, f64), f64, f64) {
    let u = shape::smooth(t);
    let (x0, x1) = (
        FOLDER_X + FOLDER.0 / 2.0,
        f64::from(w) - FOLDER_X - FOLDER.0 / 2.0,
    );
    let (y0, y1) = (mouth(from), mouth(to));
    let x = x0 + (x1 - x0) * u;
    let y = y0 + (y1 - y0) * u - 4.0 * LIFT * u * (1.0 - u);
    let turn = -0.35 + 0.7 * u + 0.12 * (std::f64::consts::TAU * u).sin();
    let k = 0.85 + 0.15 * (std::f64::consts::PI * u).sin();
    ((x, y), turn, k)
}

/// How opaque a sheet `t` of the way through its trip is: it fades out on the way to nothing
/// and in on the way from it.
fn presence(t: f64, from: End, to: End) -> f64 {
    let mut a = 1.0;
    if to == End::Gone {
        a *= 1.0 - shape::smooth((t - FADE_FROM) / (1.0 - FADE_FROM));
    }
    if from == End::Gone {
        a *= shape::smooth(t / (1.0 - FADE_FROM));
    }
    a
}

/// A sheet at `centre`, `a` (0..1) opaque.
fn paint_sheet(l: &mut Layer, centre: (f64, f64), turn: f64, k: f64, a: f64, pal: &Palette) {
    let d = l.d;
    if pal.shadow > 0 {
        let pts = sheet_corners((centre.0 + 1.2, centre.1 + 1.6), SHEET, turn, k, d);
        l.paint(bbox(&pts, d, 1.0), |x, y| {
            ink(Color::rgb(0, 0, 0), pal.shadow, polygon(x, y, &pts) * a)
        });
    }
    let rim = sheet_corners(centre, SHEET, turn, k, d);
    l.paint(bbox(&rim, d, 1.0), |x, y| {
        ink(pal.paper_rim, 255, polygon(x, y, &rim) * a)
    });
    let body = sheet_corners(centre, (SHEET.0 - 2.0, SHEET.1 - 2.0), turn, k, d);
    l.paint(bbox(&body, d, 1.0), |x, y| {
        ink(pal.paper, 255, polygon(x, y, &body) * a)
    });
    let (s, co) = turn.sin_cos();
    for row in [-3.5, -0.5, 2.5] {
        let at = |x: f64| {
            let (x, y) = (x * k, row * k);
            (
                (centre.0 + x * co - y * s) * d,
                (centre.1 + x * s + y * co) * d,
            )
        };
        let pts = [at(-3.5), at(3.5)];
        l.paint(bbox(&pts, d, 1.0), |x, y| {
            ink(pal.ink, 200, stroke(x, y, &pts, d) * a)
        });
    }
}

/// A rounded rect `w` x `h` design pixels at (`x`, `y`), read at device pixel (`px`, `py`).
fn rrect(px: f64, py: f64, (x, y, w, h, r): (f64, f64, f64, f64, f64), d: f64) -> f64 {
    rounded(px - x * d, py - y * d, w * d, h * d, r * d)
}

/// The back of an open folder at `x0` and the `n` sheets in it, `lift` raising the top one.
fn paint_folder_back(l: &mut Layer, x0: f64, n: usize, lift: f64, pal: &Palette) {
    let (d, y0) = (l.d, FOLDER_Y);
    let (w, h) = FOLDER;
    let back = |_x: f64, y: f64| down(pal.back, y0, y0 + h, y, d);
    // Tab and panel, each a rim then its fill one pixel in.
    for (bx, filled) in [
        ((x0 + 1.0, y0 - 1.0, 18.0, 10.0, 2.5), false),
        ((x0 + 2.0, y0, 16.0, 8.0, 2.0), true),
        ((x0 - 1.0, y0 + 3.0, w + 2.0, h - 2.0, 3.0), false),
        ((x0, y0 + 4.0, w, h - 4.0, 2.5), true),
    ] {
        let reach = (bx.0, bx.1, bx.0 + bx.2, bx.1 + bx.3);
        l.paint(reach, |x, y| {
            let cov = rrect(x, y, bx, d);
            if filled {
                shape::scale(back(x, y), cov)
            } else {
                ink(pal.rim, 255, cov)
            }
        });
    }
    for i in 0..n {
        let top = i + 1 == n;
        let y = y0 + 13.0 - i as f64 * 1.6 - if top { lift } else { 0.0 };
        let sx = x0 + 5.0 + (i % 2) as f64;
        let rim = (sx, y, 32.0, 18.0, 1.0);
        let body = (sx + 1.0, y + 1.0, 30.0, 16.0, 0.5);
        l.paint((sx, y, sx + 32.0, y + 18.0), |px, py| {
            let (o, inner) = (rrect(px, py, rim, d), rrect(px, py, body, d));
            shape::shares(&[
                (premul(pal.paper_rim, 255), o - inner),
                (premul(pal.paper, 255), inner),
            ])
        });
        if top {
            for row in [4.0, 7.0] {
                let pts = [
                    ((sx + 6.0) * d, (y + row) * d),
                    ((sx + 24.0) * d, (y + row) * d),
                ];
                l.paint((sx, y, sx + 32.0, y + 10.0), |px, py| {
                    ink(pal.ink, 200, stroke(px, py, &pts, d))
                });
            }
        }
    }
}

fn paint_folder_front(l: &mut Layer, x0: f64, pal: &Palette) {
    let (d, y0) = (l.d, FOLDER_Y);
    let (w, h) = FOLDER;
    let at = |pts: [(f64, f64); 4]| pts.map(|(x, y)| (x * d, y * d));
    let rim = at([
        (x0 - 3.0, y0 + 15.0),
        (x0 + w + 3.0, y0 + 15.0),
        (x0 + w + 1.0, y0 + h),
        (x0 - 1.0, y0 + h),
    ]);
    let body = at([
        (x0 - 2.0, y0 + 16.0),
        (x0 + w + 2.0, y0 + 16.0),
        (x0 + w, y0 + h - 1.0),
        (x0, y0 + h - 1.0),
    ]);
    l.paint(bbox(&rim, d, 0.0), |x, y| {
        ink(pal.rim, 255, polygon(x, y, &rim))
    });
    l.paint(bbox(&body, d, 0.0), |x, y| {
        shape::scale(
            down(pal.front, y0 + 16.0, y0 + h, y, d),
            polygon(x, y, &body),
        )
    });
    if pal.gloss {
        let pts = [
            ((x0 - 1.0) * d, (y0 + 17.0) * d),
            ((x0 + w + 1.0) * d, (y0 + 17.0) * d),
        ];
        l.paint((x0 - 2.0, y0 + 16.0, x0 + w + 2.0, y0 + 18.0), |x, y| {
            ink(Color::rgb(0xFF, 0xFF, 0xFF), 150, stroke(x, y, &pts, d))
        });
    }
}

/// The bin standing in a folder's footprint at `x0`: its mouth's centre and radii (outer rim,
/// then the opening), and the can's half widths at the rim and the foot.
const BIN_RIM_Y: f64 = 6.0;
const BIN_RIM: (f64, f64) = (19.0, 4.5);
const BIN_OPENING: (f64, f64) = (16.5, 3.0);
const BIN_TOP: f64 = 18.0;
const BIN_FOOT: f64 = 14.0;
const BIN_FOOT_Y: f64 = 35.0;
const BIN_RIBS: i32 = 2;
const BIN_RIB_GAP: (f64, f64) = (6.5, 5.0);
/// A crumpled sheet's radius, and where each of the bin's sheets lies about the mouth's centre,
/// first at the bottom.
const BALL: f64 = 4.5;
const HEAP: [(f64, f64); 6] = [
    (-8.0, 2.5),
    (7.0, 2.5),
    (-1.0, 0.5),
    (11.0, -0.5),
    (-11.5, -1.0),
    (2.0, -3.5),
];

fn bin_centre(x0: f64) -> (f64, f64) {
    (x0 + FOLDER.0 / 2.0, FOLDER_Y + BIN_RIM_Y)
}

/// The corners of a crumpled sheet round `c`, `r` across, in device pixels; `seed` varies it.
fn crumple(c: (f64, f64), r: f64, seed: usize, d: f64) -> [(f64, f64); 9] {
    std::array::from_fn(|i| {
        let a = std::f64::consts::TAU * (i as f64 + 0.3 * seed as f64) / 9.0;
        let k = 1.0 - 0.18 * (((i * 5 + seed * 3) % 4) as f64 / 3.0);
        ((c.0 + r * k * a.cos()) * d, (c.1 + r * k * a.sin()) * d)
    })
}

/// The back of the bin at `x0` (its far rim and dark inside) and the `n` crumpled sheets in it,
/// `lift` raising the top one.
fn paint_bin_back(l: &mut Layer, x0: f64, n: usize, lift: f64, pal: &Palette) {
    let d = l.d;
    let (cx, cy) = bin_centre(x0);
    let (rx, ry) = BIN_RIM;
    let (ox, oy) = BIN_OPENING;
    l.paint((cx - rx, cy - ry, cx + rx, cy + ry), |x, y| {
        let rim = shape::ellipse(x, y, cx * d, cy * d, rx * d, ry * d);
        let hole = shape::ellipse(x, y, cx * d, cy * d, ox * d, oy * d);
        shape::shares(&[
            (premul(pal.bin_rim, 255), rim - hole),
            (premul(pal.bin_inside, 255), hole),
        ])
    });
    for (i, &(dx, dy)) in HEAP.iter().take(n).enumerate() {
        let top = i + 1 == n;
        let c = (cx + dx, cy + dy - if top { lift } else { 0.0 });
        let rim = crumple(c, BALL, i, d);
        let body = crumple(c, BALL - 1.0, i, d);
        l.paint(bbox(&rim, d, 1.0), |x, y| {
            let (o, inner) = (polygon(x, y, &rim), polygon(x, y, &body));
            shape::shares(&[
                (premul(pal.paper_rim, 255), o - inner),
                (premul(pal.paper, 255), inner),
            ])
        });
        let crease = [
            ((c.0 - 2.4) * d, (c.1 - 0.4) * d),
            ((c.0 - 0.6) * d, (c.1 + 0.8) * d),
            ((c.0 + 0.8) * d, (c.1 - 0.4) * d),
            ((c.0 + 2.2) * d, (c.1 + 0.6) * d),
        ];
        l.paint(bbox(&crease, d, 1.0), |x, y| {
            ink(pal.paper_rim, 110, stroke(x, y, &crease, d))
        });
    }
}

/// The can's wall and the near half of its rim, over whatever lies in the bin.
fn paint_bin_front(l: &mut Layer, x0: f64, pal: &Palette) {
    let d = l.d;
    let (cx, cy) = bin_centre(x0);
    let (rx, ry) = BIN_RIM;
    let (ox, oy) = BIN_OPENING;
    let foot = FOLDER_Y + BIN_FOOT_Y;
    let at = |pts: [(f64, f64); 4]| pts.map(|(x, y)| (x * d, y * d));
    let wall = at([
        (cx - BIN_TOP, cy),
        (cx + BIN_TOP, cy),
        (cx + BIN_FOOT, foot),
        (cx - BIN_FOOT, foot),
    ]);
    let fill = at([
        (cx - BIN_TOP + 1.0, cy),
        (cx + BIN_TOP - 1.0, cy),
        (cx + BIN_FOOT - 1.0, foot - 1.0),
        (cx - BIN_FOOT + 1.0, foot - 1.0),
    ]);
    let stops = [
        (0.0, premul(pal.bin[1], 255)),
        (0.35, premul(pal.bin[0], 255)),
        (1.0, premul(pal.bin[1], 255)),
    ];
    let (left, right) = (cx - BIN_TOP, cx + BIN_TOP);
    l.paint((left, cy, right, foot), |x, y| {
        let hole = shape::ellipse(x, y, cx * d, cy * d, ox * d, oy * d);
        let o = (polygon(x, y, &wall) - hole).max(0.0);
        let inner = (polygon(x, y, &fill) - hole).max(0.0);
        let t = (shape::centre(x as i32, d as i32) - left) / (right - left);
        shape::shares(&[
            (premul(pal.bin_rim, 255), o - inner),
            (Stops(&stops).at(t), inner),
        ])
    });
    for i in -BIN_RIBS..=BIN_RIBS {
        let i = f64::from(i);
        let pts = [
            ((cx + i * BIN_RIB_GAP.0) * d, (cy + 7.0) * d),
            ((cx + i * BIN_RIB_GAP.1) * d, (foot - 4.0) * d),
        ];
        l.paint(bbox(&pts, d, 1.0), |x, y| {
            ink(pal.bin_rib, 255, stroke(x, y, &pts, d))
        });
        if pal.gloss {
            let lit = pts.map(|(x, y)| (x + d, y));
            l.paint(bbox(&lit, d, 1.0), |x, y| {
                ink(Color::rgb(0xFF, 0xFF, 0xFF), 120, stroke(x, y, &lit, d))
            });
        }
    }
    // The near half of the rim: the ring below the mouth's centre line.
    l.paint((cx - rx, cy, cx + rx, cy + ry), |x, y| {
        if y + 0.5 < cy * d {
            return shape::CLEAR;
        }
        let rim = shape::ellipse(x, y, cx * d, cy * d, rx * d, ry * d);
        let hole = shape::ellipse(x, y, cx * d, cy * d, ox * d, oy * d);
        ink(pal.bin_rim, 255, rim - hole)
    });
}

fn paint_end_back(l: &mut Layer, e: End, x0: f64, n: usize, lift: f64, pal: &Palette) {
    match e {
        End::Folder => paint_folder_back(l, x0, n, lift, pal),
        End::Bin => paint_bin_back(l, x0, n, lift, pal),
        End::Gone => {}
    }
}

fn paint_end_front(l: &mut Layer, e: End, x0: f64, pal: &Palette) {
    match e {
        End::Folder => paint_folder_front(l, x0, pal),
        End::Bin => paint_bin_front(l, x0, pal),
        End::Gone => {}
    }
}

/// The flight strip `w` design pixels wide and [`STRIP_H`] tall, as a scale-1 canvas of its
/// device pixels at `d` per design pixel, clear where nothing is drawn.
pub fn paint_flight(f: &Flight, pal: &Palette, w: i32, d: i32) -> Canvas {
    let mut l = Layer::new(w, STRIP_H, d);
    let dd = l.d;
    let (left, right) = (FOLDER_X, f64::from(w) - FOLDER_X - FOLDER.0);
    let frac = f.fraction.clamp(0.0, 1.0);
    let (src, dst) = match f.stage {
        Stage::Preparing => (SHEETS, 0.0),
        Stage::Finishing => (if f.moving { 0.0 } else { SHEETS }, SHEETS),
        Stage::Flying => (
            if f.moving {
                (1.0 - frac) * SHEETS
            } else {
                SHEETS
            },
            frac * SHEETS,
        ),
    };
    if let (Some(glow), Stage::Finishing, End::Folder) = (pal.trail, f.stage, f.to) {
        let pulse = 0.5 + 0.5 * (f.ms as f64 / 400.0).sin();
        let (cx, cy) = ((right + FOLDER.0 / 2.0) * dd, (FOLDER_Y + 20.0) * dd);
        let stops = [
            (0.0, premul(glow, (90.0 + 60.0 * pulse) as u8)),
            (1.0, shape::CLEAR),
        ];
        l.paint(
            (
                right - 14.0,
                0.0,
                right + FOLDER.0 + 14.0,
                f64::from(STRIP_H),
            ),
            |x, y| shape::radial(x, y, cx, cy, 36.0 * dd, 26.0 * dd, &Stops(&stops)),
        );
    }
    if let (Some(trail), Stage::Flying) = (pal.trail, f.stage) {
        for k in 1..16 {
            let t = f64::from(k) / 16.0;
            let ((x, y), _, _) = path_at(t, w, f.from, f.to);
            let a = presence(t, f.from, f.to);
            l.paint((x - 2.0, y - 2.0, x + 2.0, y + 2.0), |px, py| {
                let cov = shape::ellipse(px, py, x * dd, y * dd, 0.9 * dd, 0.9 * dd);
                ink(trail, 110, cov * a)
            });
        }
    }
    let bob = match f.stage {
        Stage::Preparing => 2.0 * (f.ms as f64 / 260.0).sin().abs(),
        _ => 0.0,
    };
    paint_end_back(&mut l, f.from, left, src.round() as usize, bob, pal);
    paint_end_back(&mut l, f.to, right, dst.round() as usize, 0.0, pal);
    if f.stage == Stage::Flying {
        for i in 0..IN_FLIGHT {
            let t = ((f.ms + i * CYCLE_MS / IN_FLIGHT) % CYCLE_MS) as f64 / CYCLE_MS as f64;
            let (c, turn, k) = path_at(t, w, f.from, f.to);
            let a = presence(t, f.from, f.to);
            let k = k * (1.0 - FADE_SHRINK * (1.0 - a));
            paint_sheet(&mut l, c, turn, k, a, pal);
        }
    }
    paint_end_front(&mut l, f.from, left, pal);
    paint_end_front(&mut l, f.to, right, pal);
    l.canvas()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAL: Palette = Palette {
        back: [Color::rgb(0xE8, 0xB8, 0x48); 2],
        front: [Color::rgb(0xFF, 0xD8, 0x70); 2],
        rim: Color::rgb(0x80, 0x60, 0x00),
        paper: Color::rgb(0xFF, 0xFF, 0xFF),
        paper_rim: Color::rgb(0x80, 0x80, 0x80),
        ink: Color::rgb(0x00, 0x00, 0x80),
        shadow: 0,
        trail: None,
        gloss: false,
        bin: [Color::rgb(0xC0, 0xC0, 0xC0); 2],
        bin_rim: Color::rgb(0x40, 0x40, 0x40),
        bin_inside: Color::rgb(0x20, 0x20, 0x20),
        bin_rib: Color::rgb(0x80, 0x80, 0x80),
    };

    fn flight(stage: Stage, fraction: f64, moving: bool) -> Flight {
        Flight {
            ms: 200,
            fraction,
            moving,
            stage,
            ..Flight::default()
        }
    }

    fn trip(stage: Stage, fraction: f64, from: End, to: End) -> Flight {
        Flight {
            moving: true,
            from,
            to,
            ..flight(stage, fraction, true)
        }
    }

    /// Paper-white pixels in the right end's footprint.
    fn paper_on_right(c: &Canvas, d: i32) -> usize {
        let x0 = (340 - FOLDER_X as i32 - FOLDER.0 as i32) * d;
        (x0..x0 + FOLDER.0 as i32 * d)
            .flat_map(|x| (0..STRIP_H * d).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == 0xFFFF_FFFF)
            .count()
    }

    #[test]
    fn a_bin_fills_with_sheets_as_the_delete_goes() {
        let at = |fraction| {
            let c = paint_flight(
                &trip(Stage::Flying, fraction, End::Folder, End::Bin),
                &PAL,
                340,
                1,
            );
            paper_on_right(&c, 1)
        };
        let (empty, half, full) = (at(0.0), at(0.5), at(1.0));
        assert!(empty < half && half < full, "{empty} < {half} < {full}");
    }

    #[test]
    fn a_bin_emptying_starts_full_and_ends_empty() {
        let left = |stage| {
            let c = paint_flight(&trip(stage, 0.0, End::Bin, End::Gone), &PAL, 340, 1);
            (FOLDER_X as i32..(FOLDER_X + FOLDER.0) as i32)
                .flat_map(|x| (0..STRIP_H).map(move |y| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == 0xFFFF_FFFF)
                .count()
        };
        assert!(left(Stage::Preparing) > 20, "a full bin shows its sheets");
        assert_eq!(left(Stage::Finishing), 0, "an emptied bin shows none");
    }

    #[test]
    fn nothing_stands_where_a_gone_end_would() {
        for d in [1, 2] {
            for stage in [Stage::Preparing, Stage::Flying, Stage::Finishing] {
                let c = paint_flight(&trip(stage, 1.0, End::Folder, End::Gone), &PAL, 340, d);
                let x0 = (340 - FOLDER_X as i32 - FOLDER.0 as i32) * d;
                let drawn = (x0..x0 + FOLDER.0 as i32 * d)
                    .flat_map(|x| ((FOLDER_Y as i32 + 18) * d..STRIP_H * d).map(move |y| (x, y)))
                    .filter(|&(x, y)| c.get(x, y) >> 24 != 0)
                    .count();
                assert_eq!(drawn, 0, "{stage:?} at {d}");
            }
        }
    }

    #[test]
    fn a_sheet_bound_nowhere_fades_out_and_shrinks() {
        assert_eq!(presence(0.3, End::Folder, End::Gone), 1.0);
        assert!(presence(0.8, End::Folder, End::Gone) < 0.5);
        assert_eq!(presence(1.0, End::Folder, End::Gone), 0.0);
        assert_eq!(presence(1.0, End::Bin, End::Folder), 1.0);
    }

    #[test]
    fn a_sheet_drops_into_the_bins_mouth() {
        let ((_, y), _, _) = path_at(1.0, 340, End::Folder, End::Bin);
        let (rim, opening) = (FOLDER_Y + BIN_RIM_Y, BIN_OPENING.1);
        assert!(y > rim + opening, "it lands below the near rim, {y}");
    }

    #[test]
    fn the_bins_edges_are_smooth_at_2() {
        let c = paint_flight(
            &trip(Stage::Preparing, 0.0, End::Bin, End::Bin),
            &PAL,
            340,
            2,
        );
        let partial = (0..FOLDER.0 as i32 * 2)
            .flat_map(|x| (0..STRIP_H * 2).map(move |y| (FOLDER_X as i32 * 2 + x, y)))
            .filter(|&(x, y)| (1..255).contains(&(c.get(x, y) >> 24)))
            .count();
        assert!(partial > 30, "{partial} partly covered pixels");
    }

    #[test]
    fn a_sheet_rises_between_the_folders_and_lands_in_the_right_one() {
        let ((x0, y0), _, _) = sheet_at(0.0, 340);
        let ((xm, ym), _, k) = sheet_at(0.5, 340);
        let ((x1, y1), _, _) = sheet_at(1.0, 340);
        assert_eq!((x0, x1), (40.0, 300.0));
        assert_eq!(y0, y1, "it leaves and lands at the folders' mouths");
        assert_eq!((xm, ym), (170.0, y0 - LIFT), "it peaks halfway");
        assert_eq!(k, 1.0, "full size at the top");
    }

    #[test]
    fn the_strip_is_clear_outside_the_folders_and_the_sheets() {
        for d in [1, 2] {
            let c = paint_flight(&flight(Stage::Preparing, 0.0, false), &PAL, 340, d);
            assert_eq!(c.device_size(), (340 * d as u32, 60 * d as u32));
            assert_eq!(
                c.get(170, 30) >> 24,
                0,
                "nothing flies while preparing at {d}"
            );
            assert_eq!(c.get(0, 0) >> 24, 0);
        }
    }

    #[test]
    fn a_move_empties_the_left_folder_and_a_copy_keeps_it_full() {
        let sheet_at_top = |c: &Canvas| c.get(30, FOLDER_Y as i32 + 6) == 0xFFFF_FFFF;
        let copy = paint_flight(&flight(Stage::Flying, 1.0, false), &PAL, 340, 1);
        let moved = paint_flight(&flight(Stage::Flying, 1.0, true), &PAL, 340, 1);
        assert!(sheet_at_top(&copy), "a copy keeps its sheets");
        assert!(!sheet_at_top(&moved), "a move leaves the folder empty");
    }

    #[test]
    fn the_sheets_edges_are_smooth_at_2() {
        let c = paint_flight(&flight(Stage::Flying, 0.5, false), &PAL, 340, 2);
        let partial = c
            .pixels
            .iter()
            .filter(|&&p| (1..255).contains(&(p >> 24)))
            .count();
        assert!(partial > 40, "{partial} partly covered pixels");
    }
}
