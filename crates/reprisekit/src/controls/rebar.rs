//! The rebar: the stack of bands Explorer hangs its menu bar, toolbar and
//! address bar from.
//!
//! Each band is a line along its top ([`BAND_LINE`] rows) with its content
//! under it, except the first band in a look whose
//! [`RebarMetrics::first_line`] is off. The content sits
//! [`RebarMetrics::band_top`] rows under the line (none in Classic).
//! Explorer stacks three: the menu band 22 px tall, the toolbar band (30 px
//! in Classic, 38 in Blue) and the address band 22 px, so the rebar is 80 px
//! in Classic and 86 in Blue. Classic etches every line on `button_face`
//! ([`crate::classic::rebar`]); a style with rebar parts draws its own
//! background behind every band and its band line on the bands that wear one
//! ([`crate::look::widgets::rebar`]). Band grippers are not drawn.

use crate::look::Paint;
use crate::look::widgets::rebar as widgets;
use crate::metrics::RebarMetrics;
use crate::raster::{Canvas, Rect};

pub const BAND_LINE: i32 = 2;

pub struct Band {
    pub height: i32,
}

pub struct Rebar {
    pub bands: Vec<Band>,
}

fn line_above(i: usize, m: &RebarMetrics) -> i32 {
    if i > 0 || m.first_line { BAND_LINE } else { 0 }
}

/// The rows above band `i`'s content: its line, then [`RebarMetrics::band_top`].
fn above(i: usize, m: &RebarMetrics) -> i32 {
    line_above(i, m) + m.band_top
}

/// Each band's content rect inside `r`, top to bottom in `rb.bands` order.
pub fn layout(rb: &Rebar, r: Rect, m: &RebarMetrics) -> Vec<Rect> {
    let mut y = r.y;
    rb.bands
        .iter()
        .enumerate()
        .map(|(i, band)| {
            y += above(i, m);
            let rect = Rect::new(r.x, y, r.width, band.height);
            y += band.height;
            rect
        })
        .collect()
}

pub fn height(rb: &Rebar, m: &RebarMetrics) -> i32 {
    rb.bands
        .iter()
        .enumerate()
        .map(|(i, b)| above(i, m) + b.height)
        .sum()
}

/// The background behind the whole stack, then the line along each band that wears one.
pub fn draw(c: &mut Canvas, p: &Paint, rb: &Rebar, r: Rect) {
    let s = p.s;
    let m = p.look.controls().rebar;
    let lines: Vec<i32> = layout(rb, r, &m)
        .iter()
        .enumerate()
        .filter(|&(i, _)| line_above(i, &m) > 0)
        .map(|(_, band)| (band.y - m.band_top - BAND_LINE) * s)
        .collect();
    widgets::draw(c, p, r.scaled(s), &lines);
}

/// The line that closes the rebar off at `y`, from `x0` to `x1` (unscaled):
/// Classic's etched line; nothing under a style whose rebar background runs
/// to its own bottom edge.
pub fn draw_closing_line(c: &mut Canvas, p: &Paint, x0: i32, x1: i32, y: i32) {
    let s = p.s;
    widgets::draw_closing_line(c, p, x0 * s, x1 * s, y * s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::Variant;
    use crate::interfaces;
    use crate::look::classic_paint;
    use crate::theme::{Color, Theme};

    /// Client-local: where each band's line and content start.
    const LINES: [i32; 3] = [1, 25, 57];
    const CONTENT: [i32; 3] = [3, 27, 59];
    const HEIGHTS: [i32; 3] = [22, 30, 22];
    const HEIGHT: i32 = 80;

    /// Blue's: no line on the menu band, a 38 px toolbar band, lines at 22 and 62.
    const BLUE_LINES: [i32; 2] = [22, 62];
    const BLUE_CONTENT: [i32; 3] = [0, 24, 64];
    const BLUE_HEIGHTS: [i32; 3] = [22, 38, 22];
    const BLUE_HEIGHT: i32 = 86;

    fn bands(heights: [i32; 3]) -> Rebar {
        Rebar {
            bands: heights.iter().map(|&height| Band { height }).collect(),
        }
    }

    fn explorer() -> Rebar {
        bands(HEIGHTS)
    }

    #[test]
    fn layout_puts_the_three_bands_where_explorer_has_them() {
        let rb = explorer();
        let m = RebarMetrics::CLASSIC;
        let rects = layout(&rb, Rect::new(0, 1, 792, HEIGHT), &m);
        assert_eq!(rects.len(), 3);
        for (i, band) in rects.iter().enumerate() {
            assert_eq!(band.y, CONTENT[i], "band {i} content y");
            assert_eq!(band.height, HEIGHTS[i], "band {i} height");
            assert_eq!(band.y - BAND_LINE, LINES[i], "band {i} line y");
            assert_eq!(band.width, 792, "a band spans the client");
        }
        assert_eq!(height(&rb, &m), HEIGHT);
    }

    #[test]
    fn blues_first_band_wears_no_line() {
        let rb = bands(BLUE_HEIGHTS);
        let m = interfaces::blue_theme(Variant::Blue).look.controls().rebar;
        let rects = layout(&rb, Rect::new(0, 0, 792, BLUE_HEIGHT), &m);
        let ys: Vec<i32> = rects.iter().map(|r| r.y).collect();
        assert_eq!(ys, BLUE_CONTENT);
        assert_eq!(height(&rb, &m), BLUE_HEIGHT);
    }

    /// In Slate, Sapphire and Obsidian each band's content sits 2 px under
    /// its line: the menu band at 2, the lines at 24 and 66, 92 px in all.
    /// Ember keeps navy's bands.
    #[test]
    fn a_band_top_lowers_every_bands_content() {
        let rb = bands(BLUE_HEIGHTS);
        for (v, content, h) in [
            (Variant::Slate, [2, 28, 70], 92),
            (Variant::Sapphire, [2, 28, 70], 92),
            (Variant::Obsidian, [2, 28, 70], 92),
            (Variant::Ember, BLUE_CONTENT, BLUE_HEIGHT),
        ] {
            let m = interfaces::blue_theme(v).look.controls().rebar;
            let rects = layout(&rb, Rect::new(0, 0, 792, h), &m);
            let ys: Vec<i32> = rects.iter().map(|r| r.y).collect();
            assert_eq!(ys, content, "{v:?}");
            assert_eq!(height(&rb, &m), h, "{v:?}");
        }
        let theme = interfaces::blue_theme(Variant::Slate);
        let mut c = Canvas::new(792, 92);
        draw(&mut c, &theme.paint(), &rb, Rect::new(0, 0, 792, 92));
        for y in [24, 66] {
            assert_eq!(c.get(400, y), 0xFFD6D5D9, "Slate's line at {y}");
            assert_eq!(c.get(400, y + 1), 0xFFF5F5F5, "Slate's line at {y}");
        }
    }

    /// Every line is a `shadow` row over a `white` one, full width.
    #[test]
    fn draw_etches_a_line_above_each_band() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let rb = explorer();
        let r = Rect::new(0, 1, 792, HEIGHT);
        let mut c = Canvas::new(792, (HEIGHT + 1) as u32);
        c.fill_rect(c.bounds(), cs.window);
        draw(&mut c, &classic_paint(cs, 1), &rb, r);
        for (i, &y) in LINES.iter().enumerate() {
            for x in [0, 400, 791] {
                assert_eq!(c.get(x, y), cs.button_shadow.argb(), "band {i} at x {x}");
                assert_eq!(
                    c.get(x, y + 1),
                    cs.button_hilight.argb(),
                    "band {i} at x {x}"
                );
            }
        }
        assert_eq!(c.get(400, CONTENT[1]), cs.button_face.argb());
    }

    /// Under Blue behind an 800 px folder window's three bands, the rebar
    /// is the background, with the beige-over-white band line on the second
    /// and third bands and none on the first (navy, olive green, silver).
    #[test]
    fn blue_draws_its_background_and_band_lines() {
        const MARK: Color = Color::rgb(1, 2, 3);
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let theme = interfaces::blue_theme(v);
            let rb = bands(BLUE_HEIGHTS);
            let r = Rect::new(0, 0, 792, BLUE_HEIGHT);
            let mut c = Canvas::new(792, BLUE_HEIGHT as u32);
            c.fill_rect(c.bounds(), MARK);
            draw(&mut c, &theme.paint(), &rb, r);
            assert_ne!(c.get(0, 0), MARK.argb(), "{v:?}: the background covers");
            assert_eq!(c.get(0, 0), c.get(0, 1), "{v:?}: no line on the first band");
            let s = v.scheme();
            let line = s.rebar_band.normal.down;
            let (shadow, highlight) = (line[0].1.argb(), line[line.len() - 1].1.argb());
            for y in BLUE_LINES {
                for x in [0, 400, 791] {
                    assert_eq!(c.get(x, y), shadow, "{v:?} line at {x},{y}");
                    assert_eq!(c.get(x, y + 1), highlight, "{v:?} line at {x},{y}");
                }
            }
            // The bottom row of navy's and olive's background is its darker edge.
            if v != Variant::Silver {
                let edge = s.rebar.normal.rim.argb();
                assert_eq!(c.get(0, BLUE_HEIGHT - 1), edge, "{v:?} bottom edge");
            }
        }
    }

    /// Under Blue the closing line draws nothing; Classic etches it.
    #[test]
    fn the_closing_line_is_classics_alone() {
        const MARK: Color = Color::rgb(1, 2, 3);
        let blue = interfaces::blue_theme(Variant::Blue);
        let mut c = Canvas::new(40, 4);
        c.fill_rect(c.bounds(), MARK);
        draw_closing_line(&mut c, &blue.paint(), 0, 40, 1);
        assert!((0..4).all(|y| (0..40).all(|x| c.get(x, y) == MARK.argb())));

        let theme = Theme::default();
        let cs = &theme.colors;
        draw_closing_line(&mut c, &classic_paint(cs, 1), 0, 40, 1);
        assert_eq!(c.get(20, 1), cs.button_shadow.argb());
        assert_eq!(c.get(20, 2), cs.button_hilight.argb());
    }

    /// Blue's rebar at scale 2 is its scale-1 pixels doubled.
    #[test]
    fn blue_rebar_doubles_at_scale_2() {
        assert!(crate::look::styled::fixture::blue_doubles_at_paint_scale_2(
            200,
            BLUE_HEIGHT as u32,
            |c, p| {
                let r = Rect::new(0, 0, 200, BLUE_HEIGHT);
                draw(c, p, &bands(BLUE_HEIGHTS), r);
            }
        ));
    }
}
