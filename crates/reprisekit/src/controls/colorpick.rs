//! The colour picker: a loupe beside the point being picked on a still of
//! the screens. The loupe is a black box, 110 x 132: a 9 x 9 grid of 12 px
//! cells, one per device pixel of the still around the point, inside a 1 px
//! frame, the centre cell ringed white inside black; under it a readout
//! strip with a 15 px swatch of the centre pixel in a white ring and its
//! `#RRGGBB` in white. It sits 20 px below right of the point, on the other
//! side of each axis where it would leave the output.
//!
//! The picker takes every pointer event: motion moves the point, a primary
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

/// The loupe for `point` on an output of logical `size`, placed beside it.
pub fn layout(point: (i32, i32), size: (i32, i32)) -> Layout {
    let place = |p: i32, len: i32, room: i32| {
        if p + GAP + len <= room {
            p + GAP
        } else {
            (p - GAP - len).max(0)
        }
    };
    parts(Rect::new(
        place(point.0, W, size.0),
        place(point.1, H, size.1),
        W,
        H,
    ))
}

/// The parts of a loupe drawn at `loupe`.
fn parts(loupe: Rect) -> Layout {
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

/// The loupe for logical point `p` of `still`, filling `c` (`W` x `H`
/// logical, at any scale): a cell is a device pixel of the still at every scale.
pub fn draw(c: &mut Canvas, text: &TextRenderer, px: u32, still: &Still, p: (i32, i32)) {
    let lay = parts(Rect::new(0, 0, W, H));
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

/// The picker's `access` part: the loupe as a focused colour well named and valued by
/// the pixel under the point as `#rrggbb` (AT-SPI shows a colour well's name, not its
/// value). Nothing without a point.
pub fn access(
    m: &ColorPick,
    still: &Still,
    size: (i32, i32),
    tree: &mut Tree,
    parent: NodeId,
) -> Option<NodeId> {
    let p = m.point?;
    let mut n = access::node(Role::ColorWell, layout(p, size).loupe);
    let value = hex(still.sample(p)).to_lowercase();
    n.set_label(value.as_str());
    n.set_value(value);
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

    /// The count of pure white pixels in device rect `r` of `c`.
    fn lit(c: &Canvas, r: Rect) -> usize {
        let w = c.device_size().0 as i32;
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.pixels[(y * w + x) as usize] == 0xffff_ffff)
            .count()
    }

    #[test]
    fn the_loupe_shows_the_pixels_round_the_point_with_a_readout() {
        let text = TextRenderer::load(&FontConfig::default());
        let still = ramp((200, 190), 200, 190);
        let mut c = Canvas::new(W as u32, H as u32);
        draw(&mut c, &text, 11, &still, (40, 30));
        // The frame is black; cell (0, 0) is the pixel 4 up and 4 left.
        assert_eq!(c.get(0, 0), 0xff00_0000);
        assert_eq!(c.get(1, 1), 0xff00_1a24);
        assert_eq!(c.get(1 + 8 * CELL + 11, 1 + 8 * CELL + 11), 0xff00_222c);
        // The centre cell: white ring, black ring outside it, the point's
        // pixel inside.
        assert_eq!(c.get(49, 49), 0xffff_ffff);
        assert_eq!(c.get(48, 48), 0xff00_0000);
        assert_eq!(c.get(50, 50), 0xff00_1e28);
        // The swatch: white ring round the point's pixel, on black.
        assert_eq!(c.get(4, 113), 0xffff_ffff);
        assert_eq!(c.get(5, 114), 0xff00_1e28);
        assert_eq!(c.get(3, 112), 0xff00_0000);
        let label = Rect::new(25, 110, 84, 21);
        assert!(lit(&c, label) > 20, "the label draws white glyphs");
    }

    #[test]
    fn at_scale_2_a_cell_is_still_one_device_pixel_of_the_still() {
        let text = TextRenderer::load(&FontConfig::default());
        // A 100x95 output captured at scale 2.
        let still = ramp((100, 95), 200, 190);
        let mut c = Canvas::with_scale(W as u32, H as u32, 2);
        draw(&mut c, &text, 11, &still, (20, 15));
        let at = |x: i32, y: i32| c.pixels[(y * 2 * W + x) as usize];
        assert_eq!(c.device_size(), (220, 264));
        assert_eq!(at(1, 1), 0xff00_0000, "the frame is 2 device px");
        // Logical (20, 15) is device (40, 30): cell (0, 0) is device (36, 26),
        // 24 device px a side, the next cell the next device pixel.
        assert_eq!(at(2, 2), 0xff00_1a24);
        assert_eq!(at(25, 25), 0xff00_1a24);
        assert_eq!(at(26, 2), 0xff00_1a25);
        // The centre cell: a 2 px white ring inside a 2 px black one.
        assert_eq!(at(97, 97), 0xff00_0000);
        assert_eq!(at(98, 98), 0xffff_ffff);
        assert_eq!(at(99, 99), 0xffff_ffff);
        assert_eq!(at(100, 100), 0xff00_1e28);
        assert_eq!(at(10, 228), 0xff00_1e28, "the swatch");
        assert!(
            lit(&c, Rect::new(50, 220, 168, 42)) > 80,
            "the label at scale 2"
        );
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
        assert_eq!(n.label(), Some("#00140a"), "a reader speaks the name");
        assert_eq!(tree.focus(), id);
    }
}
