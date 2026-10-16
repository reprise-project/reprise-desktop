//! The copy dialog's pictures: sheets of paper flying between two open folders ([`paint_flight`],
//! coloured by a look's [`Palette`]), and what a look's throughput graph is drawn from ([`Graph`]).

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    #[default]
    Preparing,
    Flying,
    Finishing,
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

/// Where a sheet `t` (0..1) of the way through its trip is: centre, turn and size.
pub fn sheet_at(t: f64, w: i32) -> ((f64, f64), f64, f64) {
    let u = shape::smooth(t);
    let (x0, x1) = (
        FOLDER_X + FOLDER.0 / 2.0,
        f64::from(w) - FOLDER_X - FOLDER.0 / 2.0,
    );
    let mouth = FOLDER_Y + 12.0;
    let x = x0 + (x1 - x0) * u;
    let y = mouth - 4.0 * LIFT * u * (1.0 - u);
    let turn = -0.35 + 0.7 * u + 0.12 * (std::f64::consts::TAU * u).sin();
    let k = 0.85 + 0.15 * (std::f64::consts::PI * u).sin();
    ((x, y), turn, k)
}

fn paint_sheet(l: &mut Layer, centre: (f64, f64), turn: f64, k: f64, pal: &Palette, lines: bool) {
    let d = l.d;
    if pal.shadow > 0 {
        let pts = sheet_corners((centre.0 + 1.2, centre.1 + 1.6), SHEET, turn, k, d);
        l.paint(bbox(&pts, d, 1.0), |x, y| {
            ink(Color::rgb(0, 0, 0), pal.shadow, polygon(x, y, &pts))
        });
    }
    let rim = sheet_corners(centre, SHEET, turn, k, d);
    l.paint(bbox(&rim, d, 1.0), |x, y| {
        ink(pal.paper_rim, 255, polygon(x, y, &rim))
    });
    let body = sheet_corners(centre, (SHEET.0 - 2.0, SHEET.1 - 2.0), turn, k, d);
    l.paint(bbox(&body, d, 1.0), |x, y| {
        ink(pal.paper, 255, polygon(x, y, &body))
    });
    if !lines {
        return;
    }
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
            ink(pal.ink, 200, stroke(x, y, &pts, d))
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
    if let (Some(glow), Stage::Finishing) = (pal.trail, f.stage) {
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
            let ((x, y), _, _) = sheet_at(f64::from(k) / 16.0, w);
            l.paint((x - 2.0, y - 2.0, x + 2.0, y + 2.0), |px, py| {
                let cov = shape::ellipse(px, py, x * dd, y * dd, 0.9 * dd, 0.9 * dd);
                ink(trail, 110, cov)
            });
        }
    }
    let bob = match f.stage {
        Stage::Preparing => 2.0 * (f.ms as f64 / 260.0).sin().abs(),
        _ => 0.0,
    };
    paint_folder_back(&mut l, left, src.round() as usize, bob, pal);
    paint_folder_back(&mut l, right, dst.round() as usize, 0.0, pal);
    if f.stage == Stage::Flying {
        for i in 0..IN_FLIGHT {
            let t = ((f.ms + i * CYCLE_MS / IN_FLIGHT) % CYCLE_MS) as f64 / CYCLE_MS as f64;
            let (c, turn, k) = sheet_at(t, w);
            paint_sheet(&mut l, c, turn, k, pal, true);
        }
    }
    paint_folder_front(&mut l, left, pal);
    paint_folder_front(&mut l, right, pal);
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
    };

    fn flight(stage: Stage, fraction: f64, moving: bool) -> Flight {
        Flight {
            ms: 200,
            fraction,
            moving,
            stage,
        }
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
