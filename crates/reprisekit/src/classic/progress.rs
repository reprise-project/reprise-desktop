//! The block progress bar: a thin sunken frame and hilight blocks with a 2 px gap. Blocks are two
//! thirds as wide as they are tall.

use crate::paint::Paint;
use crate::raster::{Canvas, EdgeStyle, Rect};

const GAP: i32 = 2;
/// The 1 px frame plus 1 px of face before the first block.
const INSET: i32 = 2;

/// The block rects for `fraction` of the bar's width.
pub fn blocks(r: Rect, fraction: f64) -> Vec<Rect> {
    let inner = r.inset(INSET);
    let bh = inner.height;
    let bw = (bh * 2 / 3).max(1);
    let want = (inner.width as f64 * fraction.clamp(0.0, 1.0)).round() as i32;
    let mut out = Vec::new();
    let mut x = inner.x;
    while x + bw <= inner.x + want.max(0) {
        out.push(Rect::new(x, inner.y, bw, bh));
        x += bw + GAP;
    }
    out
}

pub fn draw(c: &mut Canvas, p: &Paint, r: Rect, fraction: f64, enabled: bool) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(r.scaled(s), cs.button_face);
    c.edge_t(r.scaled(s), EdgeStyle::ThinSunken, cs, s);
    let ink = if enabled { cs.hilight } else { cs.gray_text };
    for b in blocks(r, fraction) {
        c.fill_rect(b.scaled(s), ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::raster::write_preview;
    use crate::theme::Theme;

    #[test]
    fn blocks_land_on_their_pixels() {
        let r = Rect::new(0, 0, 300, 20);
        let b = blocks(r, 0.5);
        assert_eq!(b[0], Rect::new(2, 2, 10, 16));
        assert_eq!(b[1].x, 14);
        assert_eq!(b.len(), 12);
        assert!(blocks(r, 0.0).is_empty());
        assert_eq!(blocks(r, 1.0).len(), 24);
    }

    #[test]
    fn blocks_of_a_285x13_bar() {
        let r = Rect::new(12, 34, 285, 13);
        let b = blocks(r, 1.0);
        assert_eq!(b[0], Rect::new(14, 36, 6, 9));
        assert_eq!(b[1], Rect::new(22, 36, 6, 9));
        assert_eq!(b[2], Rect::new(30, 36, 6, 9));
    }

    #[test]
    fn draws_the_frame_and_greys_when_disabled() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let r = Rect::new(10, 10, 300, 20);
        let mut c = Canvas::new(320, 40);
        draw(&mut c, &classic_paint(cs, 1), r, 0.5, true);
        assert_eq!(c.get(10, 10), cs.button_shadow.argb());
        assert_eq!(c.get(309, 29), cs.button_hilight.argb());
        assert_eq!(c.get(14, 15), cs.hilight.argb());
        assert_eq!(c.get(22, 15), cs.button_face.argb());
        draw(&mut c, &classic_paint(cs, 1), r, 0.5, false);
        assert_eq!(c.get(14, 15), cs.gray_text.argb());
    }

    #[test]
    fn draw_of_a_285x13_bar_row() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let r = Rect::new(12, 34, 285, 13);
        let mut c = Canvas::new(300, 50);
        draw(&mut c, &classic_paint(cs, 1), r, 1.0, true);
        assert_eq!(c.get(12, 40), cs.button_shadow.argb());
        assert_eq!(c.get(13, 40), cs.button_face.argb());
        for x in 14..20 {
            assert_eq!(c.get(x, 40), cs.hilight.argb(), "x={x}");
        }
        assert_eq!(c.get(20, 40), cs.button_face.argb());
        assert_eq!(c.get(21, 40), cs.button_face.argb());
        for x in 22..28 {
            assert_eq!(c.get(x, 40), cs.hilight.argb(), "x={x}");
        }
        assert_eq!(c.get(296, 40), cs.button_hilight.argb());
        assert_eq!(c.get(150, 46), cs.button_hilight.argb());
        assert_eq!(c.get(150, 34), cs.button_shadow.argb());
    }

    #[test]
    #[ignore]
    fn preview_the_bar() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let r = Rect::new(12, 34, 285, 13);
        let mut c = Canvas::new(300, 50);
        c.fill_rect(c.bounds(), cs.background);
        draw(&mut c, &classic_paint(cs, 1), r, 0.7, true);
        write_preview(&c, "progress-preview.png");
    }
}
