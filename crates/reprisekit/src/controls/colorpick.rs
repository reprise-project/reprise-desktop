//! The colour picker: a still of one output with a loupe beside the point
//! being picked. The loupe is a black box, 110 x 132: a 9 x 9 grid of 12 px
//! cells, one per device pixel around the point, inside a 1 px frame, the
//! centre cell ringed white inside black; under it a readout strip with a
//! 15 px swatch of the centre pixel in a white ring and its `#RRGGBB` in
//! white. It sits 20 px below right of the point, on the other side of each
//! axis where it would leave the output.
//!
//! The overlay takes every pointer event: motion moves the point, a primary
//! press picks the pixel under it, a secondary press cancels. Enter picks,
//! Escape cancels and the arrow keys nudge the point a pixel.

use crate::access::{self, Tree};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::keysym;
use crate::theme::Color;
use accesskit::{NodeId, Role};

/// The loupe's cells a side (odd, so one sits under the point) and each cell's logical size.
pub const CELLS: i32 = 9;
pub const CELL: i32 = 12;
/// The loupe's gap from the point, and the readout strip under the grid.
pub const GAP: i32 = 20;
pub const READOUT_H: i32 = 22;
pub const W: i32 = CELLS * CELL + 2;
pub const H: i32 = W + READOUT_H;

const BLACK: Color = Color::rgb(0, 0, 0);
const WHITE: Color = Color::rgb(255, 255, 255);

/// One output's picture as the picker opened: opaque ARGB, row-major,
/// `width` x `height` device pixels showing `logical` px of output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Still {
    pub logical: (i32, i32),
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl Still {
    /// The device pixel logical (x, y) falls in, clamped into the still.
    pub fn device_point(&self, (x, y): (i32, i32)) -> (i32, i32) {
        let along = |v: i32, logical: i32, device: u32| {
            let d = i64::from(v) * i64::from(device) / i64::from(logical.max(1));
            d.clamp(0, i64::from(device) - 1) as i32
        };
        (
            along(x, self.logical.0, self.width),
            along(y, self.logical.1, self.height),
        )
    }

    /// Device pixel (x, y), clamped to the nearest edge.
    pub fn pixel(&self, x: i32, y: i32) -> u32 {
        let x = x.clamp(0, self.width as i32 - 1) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        self.pixels[y * self.width as usize + x]
    }

    /// The pixel under logical (x, y), clamped into the still.
    pub fn sample(&self, p: (i32, i32)) -> u32 {
        let (x, y) = self.device_point(p);
        self.pixel(x, y)
    }

    /// The still box-resampled to `w` x `h` device pixels: the overlay's backdrop.
    pub fn backdrop(&self, w: u32, h: u32) -> Vec<u32> {
        Canvas::resample_box_from(self.width, self.height, w, h, |x, y| {
            self.pixels[y as usize * self.width as usize + x as usize]
        })
        .pixels
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorPick {
    /// The point being picked, in the overlay's logical px; None until
    /// the pointer is over it or a key places it.
    point: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Cancel,
    /// Pick the pixel under this logical point.
    Pick((i32, i32)),
}

impl ColorPick {
    pub fn point(&self) -> Option<(i32, i32)> {
        self.point
    }

    pub fn motion(&mut self, p: (i32, i32)) -> Event {
        self.point = Some(p);
        Event::Redraw
    }

    /// The pointer left the overlay: no loupe until it is back.
    pub fn leave(&mut self) -> Event {
        self.point = None;
        Event::Redraw
    }

    pub fn press(&mut self, p: (i32, i32), primary: bool) -> Event {
        if primary {
            self.point = Some(p);
            Event::Pick(p)
        } else {
            Event::Cancel
        }
    }

    /// Keysym `sym` on an overlay of logical `size`. An arrow with no
    /// point yet starts from the middle.
    pub fn key(&mut self, sym: u32, size: (i32, i32)) -> Event {
        let (dx, dy) = match sym {
            keysym::LEFT | keysym::KP_LEFT => (-1, 0),
            keysym::RIGHT | keysym::KP_RIGHT => (1, 0),
            keysym::UP | keysym::KP_UP => (0, -1),
            keysym::DOWN | keysym::KP_DOWN => (0, 1),
            keysym::ESCAPE => return Event::Cancel,
            keysym::RETURN | keysym::KP_ENTER => {
                return self.point.map_or(Event::None, Event::Pick);
            }
            _ => return Event::None,
        };
        let (w, h) = size;
        let (x, y) = self.point.unwrap_or((w / 2, h / 2));
        self.point = Some(((x + dx).clamp(0, w - 1), (y + dy).clamp(0, h - 1)));
        Event::Redraw
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub loupe: Rect,
    /// The grid's first cell; cell (i, j) is `CELL` px on from it.
    pub grid: Rect,
    pub center: Rect,
    pub swatch: Rect,
    pub label: Rect,
}

pub fn layout(point: (i32, i32), size: (i32, i32)) -> Layout {
    let place = |p: i32, len: i32, room: i32| {
        if p + GAP + len <= room {
            p + GAP
        } else {
            (p - GAP - len).max(0)
        }
    };
    let loupe = Rect::new(place(point.0, W, size.0), place(point.1, H, size.1), W, H);
    let grid = Rect::new(loupe.x + 1, loupe.y + 1, CELL, CELL);
    let half = CELLS / 2;
    let center = Rect::new(grid.x + half * CELL, grid.y + half * CELL, CELL, CELL);
    let strip = Rect::new(
        loupe.x + 1,
        loupe.bottom() - READOUT_H,
        W - 2,
        READOUT_H - 1,
    );
    let swatch = Rect::new(strip.x + 3, strip.y + 3, strip.height - 6, strip.height - 6);
    let label = Rect::new(
        swatch.right() + 6,
        strip.y,
        strip.right() - swatch.right() - 6,
        strip.height,
    );
    Layout {
        loupe,
        grid,
        center,
        swatch,
        label,
    }
}

pub fn hex(argb: u32) -> String {
    let [_, r, g, b] = argb.to_be_bytes();
    format!("#{r:02X}{g:02X}{b:02X}")
}

fn color(argb: u32) -> Color {
    let [_, r, g, b] = argb.to_be_bytes();
    Color::rgb(r, g, b)
}

/// The overlay over `c`: `backdrop` (from [`Still::backdrop`] at `c`'s
/// device size), then the loupe when there is a point.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    px: u32,
    m: &ColorPick,
    still: &Still,
    backdrop: &[u32],
) {
    if backdrop.len() == c.pixels.len() {
        c.pixels.copy_from_slice(backdrop);
    }
    let Some(p) = m.point else {
        return;
    };
    let lay = layout(p, (c.width as i32, c.height as i32));
    let (cx, cy) = still.device_point(p);
    let half = CELLS / 2;
    c.fill_rect(lay.loupe, BLACK);
    for j in 0..CELLS {
        for i in 0..CELLS {
            let cell = Rect::new(lay.grid.x + i * CELL, lay.grid.y + j * CELL, CELL, CELL);
            c.fill_rect(cell, color(still.pixel(cx + i - half, cy + j - half)));
        }
    }
    c.ring_t(lay.center.inset(-1), BLACK, BLACK, 1);
    c.ring_t(lay.center, WHITE, WHITE, 1);
    let argb = still.pixel(cx, cy);
    c.fill_rect(lay.swatch, color(argb));
    c.ring_t(lay.swatch, WHITE, WHITE, 1);
    text.draw(c, lay.label, &hex(argb), false, WHITE, Align::Left, px);
}

/// The picker's `access` part: the loupe as a focused colour well valued by
/// the pixel under the point as `#rrggbb`. Nothing without a point.
pub fn access(
    m: &ColorPick,
    still: &Still,
    size: (i32, i32),
    tree: &mut Tree,
    parent: NodeId,
) -> Option<NodeId> {
    let p = m.point?;
    let mut n = access::node(Role::ColorWell, layout(p, size).loupe);
    n.set_value(hex(still.sample(p)).to_lowercase());
    let id = tree.push(parent, "loupe", n);
    tree.set_focus(id);
    Some(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    /// A `w` x `h` still whose pixel (x, y) is 0xff_00_yy_xx.
    fn ramp(logical: (i32, i32), w: u32, h: u32) -> Still {
        let pixels = (0..h)
            .flat_map(|y| (0..w).map(move |x| 0xff00_0000 | (y << 8) | x))
            .collect();
        Still {
            logical,
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn a_point_samples_the_device_pixel_under_it() {
        // A 100x50 output captured at scale 2.
        let s = ramp((100, 50), 200, 100);
        assert_eq!(s.device_point((10, 5)), (20, 10));
        assert_eq!(s.sample((10, 5)), 0xff00_0a14);
        // Past the edge clamps to it.
        assert_eq!(s.device_point((100, -3)), (199, 0));
        assert_eq!(s.pixel(-5, 500), 0xff00_6300);
    }

    #[test]
    fn the_loupe_sits_below_right_and_flips_off_the_edges() {
        let l = layout((10, 10), (1920, 1080));
        assert_eq!(l.loupe, Rect::new(30, 30, 110, 132));
        assert_eq!(l.grid, Rect::new(31, 31, 12, 12));
        assert_eq!(l.center, Rect::new(79, 79, 12, 12));
        assert_eq!(l.swatch, Rect::new(34, 143, 15, 15));
        assert_eq!(l.label, Rect::new(55, 140, 84, 21));
        let l = layout((1915, 1075), (1920, 1080));
        assert_eq!(l.loupe, Rect::new(1785, 923, 110, 132));
        // No room either side: pinned to the top-left.
        assert_eq!(
            layout((50, 50), (120, 140)).loupe,
            Rect::new(0, 0, 110, 132)
        );
    }

    #[test]
    fn the_loupe_shows_the_pixels_round_the_point_with_a_readout() {
        let text = TextRenderer::load(&FontConfig::default());
        let still = ramp((200, 190), 200, 190);
        let mut m = ColorPick::default();
        m.motion((40, 30));
        let mut c = Canvas::new(200, 190);
        draw(&mut c, &text, 11, &m, &still, &still.backdrop(200, 190));
        // The backdrop is the still, 1:1; the loupe spans (60, 50) to (170, 182).
        assert_eq!(c.get(5, 5), 0xff00_0505);
        assert_eq!(c.get(59, 49), 0xff00_313b);
        // The frame is black; cell (0, 0) is the pixel 4 up and 4 left.
        assert_eq!(c.get(60, 50), 0xff00_0000);
        assert_eq!(c.get(61, 51), 0xff00_1a24);
        assert_eq!(c.get(61 + 8 * CELL + 11, 51 + 8 * CELL + 11), 0xff00_222c);
        // The centre cell: white ring, black ring outside it, the point's
        // pixel inside.
        assert_eq!(c.get(109, 99), 0xffff_ffff);
        assert_eq!(c.get(108, 98), 0xff00_0000);
        assert_eq!(c.get(110, 100), 0xff00_1e28);
        // The swatch: white ring round the point's pixel, on black.
        assert_eq!(c.get(64, 163), 0xffff_ffff);
        assert_eq!(c.get(65, 164), 0xff00_1e28);
        assert_eq!(c.get(63, 162), 0xff00_0000);
        // The label is white text on black.
        let label = Rect::new(85, 160, 84, 21);
        let lit = (label.y..label.bottom())
            .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == 0xffff_ffff)
            .count();
        assert!(lit > 20, "the label draws white glyphs, got {lit} px");
    }

    #[test]
    fn keys_nudge_pick_and_cancel() {
        let mut m = ColorPick::default();
        // Enter with no point picks nothing; an arrow starts mid-output.
        assert_eq!(m.key(keysym::RETURN, (100, 50)), Event::None);
        assert_eq!(m.key(keysym::RIGHT, (100, 50)), Event::Redraw);
        assert_eq!(m.point(), Some((51, 25)));
        m.motion((0, 49));
        m.key(keysym::LEFT, (100, 50));
        m.key(keysym::DOWN, (100, 50));
        assert_eq!(m.point(), Some((0, 49)));
        assert_eq!(m.key(keysym::KP_ENTER, (100, 50)), Event::Pick((0, 49)));
        assert_eq!(m.key(keysym::ESCAPE, (100, 50)), Event::Cancel);
        assert_eq!(m.press((7, 8), true), Event::Pick((7, 8)));
        assert_eq!(m.press((7, 8), false), Event::Cancel);
    }

    #[test]
    fn the_loupe_reads_as_a_colour_well_of_the_point() {
        let still = ramp((100, 50), 100, 50);
        let mut tree = Tree::window("Pick a colour", (100, 50));
        let mut m = ColorPick::default();
        assert_eq!(access(&m, &still, (100, 50), &mut tree, access::ROOT), None);
        m.motion((10, 20));
        let id = access(&m, &still, (100, 50), &mut tree, access::ROOT).unwrap();
        let n = tree.get(id).unwrap();
        assert_eq!(n.role(), Role::ColorWell);
        assert_eq!(n.value(), Some("#00140a"));
        assert_eq!(tree.focus(), id);
    }
}
