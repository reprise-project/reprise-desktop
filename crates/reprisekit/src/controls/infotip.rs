//! The infotip over a file: the flat yellow box of a tooltip with one line per fact, never wrapped.

use crate::controls::tooltip::{box_size, draw_box, place_below};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

/// Padding between the border and the text, on every side; the menu infotip
/// ([`crate::controls::tooltip::draw_info_tip`]) pads the same box by 2.
pub const PAD: i32 = 5;
/// One line of the tip.
pub const LINE_H: i32 = crate::controls::tooltip::INFO_LINE_H;
/// Where the box hangs relative to the pointer's hotspot.
pub const BELOW_POINTER: (i32, i32) = (0, 19);

/// Size of the tip for `lines` in `theme`'s units: the widest line plus the padding and the 1 px
/// border, [`LINE_H`] per line.
pub fn size(text: &TextRenderer, theme: &Theme, lines: &[String]) -> (i32, i32) {
    box_size(text, theme, lines, PAD, LINE_H * theme.scale.max(1))
}

/// Draw the tip into a canvas of [`size`] at `canvas_scale` (the scale of the canvas it is blitted
/// onto).
pub fn draw(theme: &Theme, text: &TextRenderer, lines: &[String], canvas_scale: i32) -> Canvas {
    draw_box(
        theme,
        text,
        lines,
        (PAD, LINE_H * theme.scale.max(1)),
        canvas_scale,
    )
}

/// Where a tip of `size` goes for a pointer at `cursor`: [`BELOW_POINTER`] from the hotspot, pushed
/// left and up to stay inside `within`.
pub fn place(cursor: (i32, i32), size: (i32, i32), within: Rect) -> (i32, i32) {
    let (dx, dy) = BELOW_POINTER;
    place_below((cursor.0 + dx, cursor.1 + dy), size, within)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Color, FontConfig};

    const FRAME: Rect = Rect::new(319, 228, 122, 51);
    const CURSOR: (i32, i32) = (319, 209);
    const INK_X: i32 = 326;
    const LINE_TOPS: [i32; 3] = [237, 250, 263];

    fn lines() -> Vec<String> {
        ["Dimensions: 800 x 600", "Type: JPEG Image", "Size: 27.8 KB"]
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn three_lines_make_a_122x51_box() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let lines = lines();
        let (w, h) = size(&text, &theme, &lines);
        assert_eq!(h, 3 * LINE_H + 2 * PAD + 2);
        assert_eq!(h, FRAME.height);
        let widest = text.measure(&lines[0], false, theme.fonts.size_px);
        assert_eq!(w, widest + 2 * PAD + 2);
        assert!(
            (w - FRAME.width).abs() <= 2,
            "{w} is not within 2 px of 122"
        );
        // One line less is one LINE_H less; the box never wraps.
        let (_, one) = size(&text, &theme, &lines[..1]);
        assert_eq!(one, h - 2 * LINE_H);
        let long = vec![
            "A file name far longer than Explorer's 300 px wrap \
                         would ever leave on one line"
                .to_string(),
        ];
        let (lw, _) = size(&text, &theme, &long);
        assert!(lw > FRAME.width, "pre-split lines are never re-wrapped");
    }

    #[test]
    fn draw_frames_the_yellow_and_starts_the_ink_five_in() {
        if !crate::text::tahoma_installed() {
            return;
        }
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let lines = lines();
        let c = draw(&theme, &text, &lines, 1);
        let (w, h) = size(&text, &theme, &lines);
        assert_eq!((c.width as i32, c.height as i32), (w, h));
        let ink = Color::rgb(0, 0, 0).argb();
        let bg = Color::rgb(255, 255, 225).argb();
        assert_eq!(c.get(0, 0), ink);
        assert_eq!(c.get(w - 1, h - 1), ink);
        assert_eq!(c.get(1, 1), bg);
        for y in 1..LINE_TOPS[0] - FRAME.y {
            assert!((1..w - 1).all(|x| c.get(x, y) == bg), "row {y} has ink");
        }
        for top in LINE_TOPS {
            let y = top - FRAME.y;
            assert!(
                (INK_X - FRAME.x..w - 1).any(|x| c.get(x, y) == ink),
                "no ink on line row {y}"
            );
        }
        let mut bare = Canvas::new(FRAME.width as u32, LINE_H as u32);
        let line_box = bare.bounds();
        text.draw(
            &mut bare,
            line_box,
            &lines[0],
            false,
            Color::rgb(0, 0, 0),
            crate::text::Align::Left,
            theme.fonts.size_px,
        );
        let bearing = (0..FRAME.width)
            .find(|&x| (0..LINE_H).any(|y| bare.get(x, y) == ink))
            .expect("the line drew nothing");
        let first = (1..w - 1)
            .find(|&x| (1..h - 1).any(|y| c.get(x, y) == ink))
            .expect("the tip drew nothing");
        assert_eq!(first, 1 + PAD + bearing);
        assert!((1..h - 1).all(|y| (1..first).all(|x| c.get(x, y) == bg)));
    }

    #[test]
    fn it_hangs_below_the_pointer_and_stays_on_screen() {
        let screen = Rect::new(0, 0, 1024, 768);
        let s = (FRAME.width, FRAME.height);
        assert_eq!(place(CURSOR, s, screen), (FRAME.x, FRAME.y));
        let (x, y) = place((1000, 760), s, screen);
        assert_eq!((x + s.0, y + s.1), (1023, 768));
        assert_eq!(place((-5, -5), s, screen), (0, BELOW_POINTER.1 - 5));
    }

    #[test]
    #[ignore]
    fn render_infotip() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let c = draw(&theme, &text, &lines(), 1);
        crate::raster::write_preview(&c, "infotip-preview.png");
    }
}
