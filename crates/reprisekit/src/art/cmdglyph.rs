//! Small coloured glyphs for commands no theme icon stands for: Cut, Copy, Paste and Delete, the
//! window's panes and the view modes. Each is drawn from shapes on a 16-unit square, so any size
//! and scale is a fresh drawing with anti-aliased edges.

use crate::raster::Canvas;
use crate::shape::{self, Px, ellipse, over, premul, rounded, stroke};
use crate::theme::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmdGlyph {
    Cut,
    Copy,
    Paste,
    Delete,
    /// The window with its navigation and details panes, for the Layout submenu's row.
    Layout,
    NavPane,
    DetailsPane,
    PreviewPane,
    Thumbnails,
    Tiles,
    Icons,
    List,
    Details,
}

pub const ALL: [CmdGlyph; 13] = [
    CmdGlyph::Cut,
    CmdGlyph::Copy,
    CmdGlyph::Paste,
    CmdGlyph::Delete,
    CmdGlyph::Layout,
    CmdGlyph::NavPane,
    CmdGlyph::DetailsPane,
    CmdGlyph::PreviewPane,
    CmdGlyph::Thumbnails,
    CmdGlyph::Tiles,
    CmdGlyph::Icons,
    CmdGlyph::List,
    CmdGlyph::Details,
];

/// The design square's side in units.
const UNIT: f64 = 16.0;

const STEEL: Color = Color::rgb(0x7C, 0x87, 0x94);
const PIVOT: Color = Color::rgb(0x4B, 0x55, 0x63);
const GRIP: Color = Color::rgb(0x2F, 0x6D, 0xB5);
const PAGE_EDGE: Color = Color::rgb(0x6E, 0x7F, 0x94);
const PAGE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const PAGE_LINE: Color = Color::rgb(0x9D, 0xB3, 0xCC);
const BOARD_EDGE: Color = Color::rgb(0x7A, 0x52, 0x28);
const BOARD: Color = Color::rgb(0xC0, 0x8A, 0x4C);
const CLIP: Color = Color::rgb(0x8C, 0x96, 0xA3);
const CROSS_EDGE: Color = Color::rgb(0xA3, 0x24, 0x1B);
const CROSS: Color = Color::rgb(0xE0, 0x4A, 0x3A);
const FRAME: Color = Color::rgb(0x5D, 0x7E, 0xA6);
const PANE: Color = Color::rgb(0x8D, 0xBB, 0xEA);
const TILE_EDGE: Color = Color::rgb(0x2F, 0x5F, 0x98);
const TILE: Color = Color::rgb(0x7E, 0xB0, 0xE6);
const ROW_LINE: Color = Color::rgb(0x6B, 0x7C, 0x90);

/// `g` drawn `px` logical px square on a canvas at `scale`.
pub fn canvas(g: CmdGlyph, px: i32, scale: i32) -> Canvas {
    let s = scale.max(1);
    let dev = px.max(1) * s;
    let k = f64::from(dev) / UNIT;
    shape::draw(dev, dev, 1, |x, y| paint(g, &Pen { x, y, k })).at_scale(s)
}

/// One device pixel (its top-left corner) and the device px per design unit.
struct Pen {
    x: f64,
    y: f64,
    k: f64,
}

impl Pen {
    fn rect(&self, (x, y, w, h): (f64, f64, f64, f64), r: f64) -> f64 {
        let k = self.k;
        rounded(self.x - x * k, self.y - y * k, w * k, h * k, r * k)
    }

    fn line(&self, pts: &[(f64, f64)], width: f64) -> f64 {
        let k = self.k;
        let pts: Vec<(f64, f64)> = pts.iter().map(|&(a, b)| (a * k, b * k)).collect();
        stroke(self.x, self.y, &pts, width * k)
    }

    fn disc(&self, (cx, cy): (f64, f64), r: f64) -> f64 {
        let k = self.k;
        ellipse(self.x, self.y, cx * k, cy * k, r * k, r * k)
    }
}

/// `layers` laid bottom first, each its colour over the share of the pixel it covers.
fn layered(layers: &[(Color, f64)]) -> Px {
    (layers.iter()).fold(shape::CLEAR, |under, &(c, cov)| {
        over(shape::scale(premul(c, 0xFF), cov.clamp(0.0, 1.0)), under)
    })
}

/// A box `edge` coloured one unit round a `fill`.
fn boxed(
    p: &Pen,
    (x, y, w, h): (f64, f64, f64, f64),
    r: f64,
    edge: Color,
    fill: Color,
) -> [(Color, f64); 2] {
    [
        (edge, p.rect((x, y, w, h), r)),
        (
            fill,
            p.rect((x + 1.0, y + 1.0, w - 2.0, h - 2.0), (r - 1.0).max(0.0)),
        ),
    ]
}

fn paint(g: CmdGlyph, p: &Pen) -> Px {
    let mut l: Vec<(Color, f64)> = Vec::new();
    match g {
        CmdGlyph::Cut => {
            l.push((STEEL, p.line(&[(4.5, 1.5), (10.5, 10.0)], 1.6)));
            l.push((STEEL, p.line(&[(11.5, 1.5), (5.5, 10.0)], 1.6)));
            l.push((PIVOT, p.disc((8.0, 7.2), 1.0)));
            for cx in [4.5, 11.5] {
                let ring = p.disc((cx, 12.5), 2.8) - p.disc((cx, 12.5), 1.6);
                l.push((GRIP, ring.max(0.0)));
            }
        }
        CmdGlyph::Copy => {
            l.extend(boxed(p, (1.0, 1.0, 9.0, 11.0), 0.0, PAGE_EDGE, PAGE));
            l.extend(boxed(p, (6.0, 4.5, 9.0, 11.0), 0.0, PAGE_EDGE, PAGE));
            for i in 0..3 {
                l.push((
                    PAGE_LINE,
                    p.rect((8.0, 7.5 + 2.0 * f64::from(i), 5.0, 1.0), 0.0),
                ));
            }
        }
        CmdGlyph::Paste => {
            l.extend(boxed(p, (1.5, 2.5, 11.0, 13.0), 1.5, BOARD_EDGE, BOARD));
            l.push((CLIP, p.rect((4.5, 1.0, 5.0, 3.0), 1.0)));
            l.extend(boxed(p, (7.0, 6.0, 8.5, 9.5), 0.0, PAGE_EDGE, PAGE));
            for i in 0..3 {
                l.push((
                    PAGE_LINE,
                    p.rect((9.0, 8.5 + 2.0 * f64::from(i), 4.5, 1.0), 0.0),
                ));
            }
        }
        CmdGlyph::Delete => {
            let a = [(3.5, 3.5), (12.5, 12.5)];
            let b = [(12.5, 3.5), (3.5, 12.5)];
            l.push((CROSS_EDGE, p.line(&a, 3.6)));
            l.push((CROSS_EDGE, p.line(&b, 3.6)));
            l.push((CROSS, p.line(&a, 2.0)));
            l.push((CROSS, p.line(&b, 2.0)));
        }
        CmdGlyph::Layout | CmdGlyph::NavPane | CmdGlyph::DetailsPane | CmdGlyph::PreviewPane => {
            l.extend(boxed(p, (0.5, 1.5, 15.0, 13.0), 1.0, FRAME, PAGE));
            let (nav, details) = (
                matches!(g, CmdGlyph::Layout | CmdGlyph::NavPane),
                matches!(g, CmdGlyph::Layout | CmdGlyph::DetailsPane),
            );
            if details {
                l.push((PANE, p.rect((1.5, 10.5, 13.0, 3.0), 0.0)));
                l.push((FRAME, p.rect((1.5, 10.0, 13.0, 1.0), 0.0)));
            }
            if nav {
                l.push((PANE, p.rect((1.5, 2.5, 4.0, 11.0), 0.0)));
                l.push((FRAME, p.rect((5.5, 2.5, 1.0, 11.0), 0.0)));
            }
            if g == CmdGlyph::PreviewPane {
                l.push((PANE, p.rect((10.0, 2.5, 4.5, 11.0), 0.0)));
                l.push((FRAME, p.rect((9.5, 2.5, 1.0, 11.0), 0.0)));
            }
        }
        CmdGlyph::Thumbnails => {
            for (x, y) in [(1.0, 1.0), (8.5, 1.0), (1.0, 8.5), (8.5, 8.5)] {
                l.extend(boxed(p, (x, y, 6.5, 6.5), 0.0, TILE_EDGE, TILE));
            }
        }
        CmdGlyph::Icons => {
            for i in 0..9 {
                let (x, y) = (1.0 + 5.0 * f64::from(i % 3), 1.0 + 5.0 * f64::from(i / 3));
                l.extend(boxed(p, (x, y, 4.0, 4.0), 0.0, TILE_EDGE, TILE));
            }
        }
        CmdGlyph::Tiles => {
            for y in [1.5, 8.5] {
                l.extend(boxed(p, (1.0, y, 6.0, 6.0), 0.0, TILE_EDGE, TILE));
                l.push((ROW_LINE, p.rect((8.5, y + 1.0, 6.5, 1.0), 0.0)));
                l.push((ROW_LINE, p.rect((8.5, y + 3.5, 4.5, 1.0), 0.0)));
            }
        }
        CmdGlyph::List => {
            for i in 0..8 {
                let (x, y) = (1.0 + 8.0 * f64::from(i / 4), 1.0 + 3.75 * f64::from(i % 4));
                l.extend(boxed(p, (x, y, 3.0, 3.0), 0.0, TILE_EDGE, TILE));
                l.push((ROW_LINE, p.rect((x + 4.0, y + 1.0, 2.5, 1.0), 0.0)));
            }
        }
        CmdGlyph::Details => {
            for i in 0..4 {
                let y = 1.0 + 3.75 * f64::from(i);
                l.extend(boxed(p, (1.0, y, 3.0, 3.0), 0.0, TILE_EDGE, TILE));
                l.push((ROW_LINE, p.rect((5.0, y + 1.0, 4.5, 1.0), 0.0)));
                l.push((ROW_LINE, p.rect((10.5, y + 1.0, 4.5, 1.0), 0.0)));
            }
        }
    }
    layered(&l)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every device pixel and the device size.
    fn device_fnv(c: &Canvas) -> u64 {
        let (w, h) = c.device_size();
        ([w, h].into_iter().chain(c.pixels.iter().copied()))
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            })
    }

    #[test]
    fn each_glyph_pins_at_scale_1_and_2() {
        let mut got = Vec::new();
        for g in ALL {
            for s in [1, 2] {
                got.push((format!("{g:?} {s}"), device_fnv(&canvas(g, 16, s))));
            }
        }
        crate::pins::check("art/cmdglyph", &got);
    }

    #[test]
    fn a_glyph_is_as_big_as_asked_at_its_scale() {
        for (px, s) in [(16, 1), (16, 2), (20, 1), (24, 2)] {
            let c = canvas(CmdGlyph::Copy, px, s);
            assert_eq!((c.width, c.height, c.scale()), (px as u32, px as u32, s));
        }
    }

    #[test]
    fn the_glyphs_differ_and_leave_their_corners_clear() {
        let all: Vec<Canvas> = ALL.iter().map(|&g| canvas(g, 16, 1)).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.pixels, b.pixels, "{:?} is drawn apart", ALL[i]);
            }
        }
        for g in [CmdGlyph::Cut, CmdGlyph::Delete, CmdGlyph::Paste] {
            let c = canvas(g, 16, 1);
            assert_eq!(
                c.get(15, 0) >> 24,
                0,
                "{g:?}: the top right corner is clear"
            );
        }
    }

    #[test]
    fn curved_and_slanted_edges_are_partly_covered_at_scale_2() {
        for g in [CmdGlyph::Cut, CmdGlyph::Delete] {
            let c = canvas(g, 16, 2);
            let partial = (c.pixels.iter())
                .filter(|&&v| (1..0xFF).contains(&(v >> 24)))
                .count();
            assert!(partial > 20, "{g:?}: {partial} partly covered pixels");
        }
    }
}
