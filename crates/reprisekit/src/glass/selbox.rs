//! Glass's selection box: marquee and drag-target outline. A 1 px opaque
//! 3399FF line round 0066CC laid over the backdrop at 70/255, in light and dark.

use crate::style::SelectionBox;
use crate::theme::Color;

pub const BOX: SelectionBox = SelectionBox {
    border: Color::rgb(0x33, 0x99, 0xFF),
    fill: Color::rgb(0x00, 0x66, 0xCC),
    alpha: 70,
};

#[cfg(test)]
mod tests {
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_marquee;
    use crate::raster::{Canvas, Rect};
    use crate::theme::Color;

    fn marquee_on(back: Color, s: i32) -> Canvas {
        let theme = theme_for(Interface::Glass);
        let p = crate::paint::Paint { s, ..theme.paint() };
        let mut c = Canvas::new((60 * s) as u32, (40 * s) as u32);
        c.fill_rect(c.bounds(), back);
        draw_marquee(
            &mut c,
            &p,
            Rect::new(10, 5, 30, 20).scaled(s),
            Color::rgb(0, 0, 0),
        );
        c
    }

    #[test]
    fn the_marquee_is_a_blue_line_round_a_translucent_fill() {
        for (back, inside) in [
            (Color::rgb(0, 0, 0), 0xFF00_1C38),
            (Color::rgb(255, 255, 255), 0xFFB9_D5F1),
            (Color::rgb(128, 128, 128), 0xFF5D_7995),
        ] {
            let c = marquee_on(back, 1);
            for (x, y) in [
                (10, 5),
                (39, 5),
                (10, 24),
                (39, 24),
                (20, 5),
                (10, 15),
                (39, 15),
                (20, 24),
            ] {
                assert_eq!(
                    c.get(x, y),
                    0xFF33_99FF,
                    "border at ({x}, {y}) over {back:?}"
                );
            }
            for (x, y) in [(11, 6), (38, 23), (25, 15)] {
                assert_eq!(c.get(x, y), inside, "fill at ({x}, {y}) over {back:?}");
            }
            assert_eq!(c.get(9, 15), back.argb(), "left of the box: untouched");
            assert_eq!(c.get(40, 15), back.argb(), "right of the box: untouched");
            assert_eq!(c.get(20, 25), back.argb(), "below the box: untouched");
        }
    }

    #[test]
    fn the_line_scales_with_the_output() {
        let c = marquee_on(Color::rgb(255, 255, 255), 2);
        assert_eq!(
            c.get(25, 11),
            0xFF33_99FF,
            "the top line's second device row"
        );
        assert_eq!(c.get(25, 12), 0xFFB9_D5F1, "the fill starts 2 px down");
    }

    /// Scene rects: the fill inside, then four lines tiling the box without
    /// overlap; a box two px across is all line.
    #[test]
    fn the_box_as_rects_tiles_it() {
        let r = Rect::new(20, 150, 380, 250);
        let rects = super::BOX.rects(r, 1);
        assert_eq!(
            rects[0],
            (Rect::new(21, 151, 378, 248), super::BOX.fill, 70),
            "the fill"
        );
        assert_eq!(rects[1].0, Rect::new(20, 150, 380, 1), "top");
        assert_eq!(rects[2].0, Rect::new(20, 399, 380, 1), "bottom");
        assert_eq!(rects[3].0, Rect::new(20, 151, 1, 248), "left");
        assert_eq!(rects[4].0, Rect::new(399, 151, 1, 248), "right");
        assert!(
            rects[1..]
                .iter()
                .all(|q| q.1 == super::BOX.border && q.2 == 255)
        );
        let area: i32 = rects.iter().map(|(q, _, _)| q.width * q.height).sum();
        assert_eq!(area, 380 * 250);
        let thin = super::BOX.rects(Rect::new(0, 0, 2, 9), 1);
        assert!(
            thin.iter().all(|q| q.2 == 255),
            "no room for a fill: {thin:?}"
        );
        assert_eq!(
            thin.iter().map(|(q, _, _)| q.width * q.height).sum::<i32>(),
            18
        );
    }

    #[test]
    fn dark_glass_keeps_the_box() {
        let theme = crate::interfaces::theme_with(
            Interface::Glass,
            crate::blue::Variant::Blue,
            &crate::glass::Settings {
                mode: crate::glass::Mode::Dark,
                ..Default::default()
            },
        );
        assert_eq!(
            theme.look.style().and_then(|s| s.selection_box),
            Some(super::BOX)
        );
    }
}
