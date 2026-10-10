//! The group box through [`Part::GroupBox`]: the part's frame drawn into the
//! box's rect, the label on its top line where the style's [`GroupLabel`]
//! puts it, the line broken behind it (Blue: from 3 px before the label to
//! 1 px past it). Frame pixels are pinned in `blue::controls::groupbox`.

use crate::classic::widgets::draw_label;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{GroupLabel, Part, State, Style};
use crate::text::{Align, TextRenderer, split_mnemonic};

#[expect(clippy::too_many_arguments)]
pub fn draw_group_box(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    g: Rect,
    label_r: Rect,
    label: &str,
    px: u32,
) -> bool {
    if st.part(Part::GroupBox).is_none() {
        return false;
    }
    let s = p.s;
    st.draw(c, g, Part::GroupBox, State::Normal, p.s);
    // The break shows what sits behind the box: the dialog face, or the page's own pixels.
    let GroupLabel {
        nudge: (dx, dy),
        before,
        after,
        rows,
    } = st.group_label;
    let label_r = Rect::new(
        label_r.x + dx * s,
        label_r.y + dy * s,
        label_r.width,
        label_r.height,
    );
    let drawn = text.measure(&split_mnemonic(label).0, false, px);
    if drawn == 0 {
        return true;
    }
    let gap = Rect::new(
        label_r.x - before * s,
        g.y,
        drawn + (before + after) * s,
        rows * s,
    );
    p.fill_back(c, gap);
    let ink = crate::look::widgets::group_label_ink(p);
    draw_label(c, text, label_r, label, false, ink, Align::Left, px, s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_group_box;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};
    use crate::text::TextRenderer;
    use crate::theme::Color;

    #[test]
    fn a_classic_look_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let g = Rect::new(2, 20, 150, 50);
        let label_r = Rect::new(10, 15, 50, 13);
        let (mut a, mut b) = (Canvas::new(160, 80), Canvas::new(160, 80));
        draw_group_box(&mut a, &text, &theme.paint(), g, label_r, "&Group", px);
        classic::widgets::draw_group_box(&mut b, &text, &theme.paint(), g, label_r, "&Group", px);
        assert!(a == b, "a Classic look paints the Classic painter");
    }

    /// The corner's outer tip is all but open, the diagonal pixel one step in
    /// carries the line, two in it is closed; edges run straight, nothing paints behind.
    #[test]
    fn a_blue_group_box_turns_its_own_rounded_corner() {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let sentinel = Color::rgb(0x12, 0x34, 0x56);
        let mut c = Canvas::new(40, 40);
        c.fill_rect(Rect::new(0, 0, 40, 40), sentinel);
        let g = Rect::new(5, 5, 30, 30);
        st.draw(&mut c, g, Part::GroupBox, State::Normal, 1);
        let at = |dx: i32, dy: i32| c.get(g.x + dx, g.y + dy);
        let (border, none) = (line(Variant::Blue), sentinel.argb());
        let near = |a: u32, b: u32| {
            (0..4).all(|i| (a >> (8 * i) & 0xFF).abs_diff(b >> (8 * i) & 0xFF) <= 2)
        };
        assert!(
            near(at(0, 0), none),
            "the corner's outer tip is all but open"
        );
        assert_ne!(at(1, 1), none, "the corner turns through its diagonal");
        assert_eq!(at(2, 2), none, "the line has closed by two steps in");
        assert_eq!(
            at(15, 0),
            border,
            "the top line runs straight past the corner"
        );
        assert_eq!(
            at(0, 15),
            border,
            "the left line runs straight past the corner"
        );
        assert_eq!(at(15, 15), none, "a group box frame has nothing behind it");
    }

    fn line(v: Variant) -> u32 {
        v.scheme().group_box.normal.rim.argb()
    }

    /// A disabled frame repeats Normal pixel for pixel.
    #[test]
    fn a_blue_group_box_disabled_frame_repeats_normal() {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let g = Rect::new(5, 5, 30, 30);
        let (mut normal, mut disabled) = (Canvas::new(40, 40), Canvas::new(40, 40));
        st.draw(&mut normal, g, Part::GroupBox, State::Normal, 1);
        st.draw(&mut disabled, g, Part::GroupBox, State::Disabled, 1);
        assert!(
            normal == disabled,
            "a disabled group box repeats the enabled frame, drawing no frame of its own"
        );
    }

    /// The top line breaks behind the label, which reads in the group box's own blue.
    #[test]
    fn a_blue_group_box_breaks_its_top_line_behind_a_blue_label() {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        if !text.has_face("Tahoma") {
            return;
        }
        let px = theme.fonts.size_px;
        let cs = &theme.colors;
        let g = Rect::new(2, 20, 150, 50);
        let label_r = Rect::new(10, 15, 50, 13);
        let mut c = Canvas::new(160, 80);
        c.fill_rect(Rect::new(0, 0, 160, 80), cs.button_face);
        draw_group_box(&mut c, &text, &theme.paint(), g, label_r, "&Group", px);
        let (face, border) = (cs.button_face.argb(), line(Variant::Blue));
        assert_eq!(
            c.get(g.x + 4, g.y),
            border,
            "the top line runs up to the label"
        );
        assert_eq!(
            c.get(g.right() - 10, g.y),
            border,
            "the top line resumes past the label"
        );
        // One pixel before the label text, still inside the break, no ink has painted.
        assert_eq!(
            c.get(label_r.x - 1, g.y),
            face,
            "the top line breaks behind the label"
        );
        assert!(
            (label_r.x..label_r.right()).any(|x| (label_r.y..label_r.bottom()).any(|y| c
                .get(x, y)
                == crate::blue::Variant::Blue.scheme().inks.group_label.argb())),
            "the label reads in the group box's own blue"
        );
    }

    /// The break runs from 3 px before the label to 1 px past its drawn width
    /// (2 px past the last glyph's ink); an access key's "&" doesn't widen it.
    /// Checked on Find's 104x40 "Direction" box and a 357x72 "&Screen saver" box.
    #[test]
    fn a_blue_group_box_breaks_its_line_3_px_before_and_2_px_past_the_label() {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        if !text.has_face("Tahoma") {
            return;
        }
        let px = theme.fonts.size_px;
        let cs = &theme.colors;
        let (face, border, ink) = (
            cs.button_face.argb(),
            line(Variant::Blue),
            Variant::Blue.scheme().inks.group_label.argb(),
        );
        for (g, plain, keyed) in [
            (Rect::new(161, 52, 104, 40), "Direction", "&Direction"),
            (Rect::new(24, 241, 357, 72), "Screen saver", "&Screen saver"),
        ] {
            let drawn = text.measure(plain, false, px);
            let label_r = Rect::new(g.x + 10, g.y - 5, drawn, 13);
            let draw = |label: &str| {
                let mut c = Canvas::new(400, 330);
                c.fill_rect(c.bounds(), cs.button_face);
                draw_group_box(&mut c, &text, &theme.paint(), g, label_r, label, px);
                c
            };
            let (a, b) = (draw(plain), draw(keyed));
            for c in [&a, &b] {
                let x = label_r.x;
                assert_eq!(c.get(x - 4, g.y), border, "{keyed}: line before");
                for dx in [-3, -2, -1] {
                    assert_eq!(c.get(x + dx, g.y), face, "{keyed}: gap at {dx}");
                }
                let ink_at = |x: i32| (label_r.y..label_r.bottom()).any(|y| c.get(x, y) == ink);
                assert!(ink_at(x), "{keyed}: the label's ink starts on its x");
                assert!(ink_at(x + drawn - 2), "{keyed}: the last ink column");
                for dx in [drawn - 1, drawn] {
                    assert!(!ink_at(x + dx), "{keyed}: no ink at {dx}");
                    assert_eq!(c.get(x + dx, g.y), face, "{keyed}: gap at {dx}");
                }
                assert_eq!(c.get(x + drawn + 1, g.y), border, "{keyed}: line after");
            }
            let row = |c: &Canvas| (g.x..g.right()).map(|x| c.get(x, g.y)).collect::<Vec<_>>();
            assert_eq!(row(&a), row(&b), "{keyed} breaks the line as {plain} does");
        }
    }

    /// On a page the break shows the page's pixels, not the face: a 357x72
    /// "&Screen saver" box on its 386x495 page, where row 241 is still in the page's fade.
    #[test]
    fn a_blue_group_box_on_a_page_breaks_its_line_to_the_page() {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let p = theme.paint();
        let page = Rect::new(9, 49, 386, 495);
        let g = Rect::new(24, 241, 357, 72);
        let drawn = text.measure("Screen saver", false, px);
        let label_r = Rect::new(g.x + 10, g.y - 5, drawn, 13);
        let mut bare = Canvas::new(400, 560);
        bare.fill_rect(bare.bounds(), theme.colors.button_face);
        crate::look::widgets::draw_page(&mut bare, &p, page);
        let mut c = bare.clone();
        draw_group_box(
            &mut c,
            &text,
            &p.on_page(page),
            g,
            label_r,
            "&Screen saver",
            px,
        );
        let x = label_r.x;
        for dx in [-3, -2, -1, drawn] {
            assert_eq!(
                c.get(x + dx, g.y),
                0xFFF7_F7F3,
                "the page's row at gap {dx}"
            );
            assert_eq!(
                c.get(x + dx, g.y),
                bare.get(x + dx, g.y),
                "gap {dx} is the page"
            );
        }
        assert_eq!(
            c.get(x - 4, g.y),
            line(Variant::Blue),
            "the line runs up to the gap"
        );
    }

    /// A scale-2 canvas and paint scale 2 on a scale-1 canvas draw the same
    /// pixels: the scale-2 frame, straight lines two whole pixels, smooth corners.
    #[test]
    fn a_blue_group_box_at_scale_2_is_painted_for_scale_2() {
        let mut theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let (g, label_r) = (Rect::new(2, 20, 150, 50), Rect::new(10, 15, 50, 13));
        let mut canvas2 = Canvas::with_scale(160, 80, 2);
        draw_group_box(&mut canvas2, &text, &theme.paint(), g, label_r, "", 11);
        theme.scale = 2;
        let mut paint2 = Canvas::new(320, 160);
        let p = theme.paint();
        draw_group_box(
            &mut paint2,
            &text,
            &p,
            g.scaled(2),
            label_r.scaled(2),
            "",
            22,
        );
        assert!(canvas2.pixels == paint2.pixels, "both scale-2 paths agree");
        let (x0, y0) = (2 * g.x, 2 * g.y);
        for i in 0..2 {
            assert_eq!(
                paint2.get(x0 + 100, y0 + i),
                line(Variant::Blue),
                "top line row {i}"
            );
        }
        assert_eq!(paint2.get(x0 + 100, y0 + 2), 0, "under the top line");
        let corner = (0..6).flat_map(|y| (0..6).map(move |x| (x, y)));
        assert!(
            corner
                .map(|(x, y)| paint2.get(x0 + x, y0 + y) >> 24)
                .any(|a| a > 0 && a < 0xFF),
            "the corner is smooth at 2"
        );
    }

    fn variant_frame(v: Variant, state: State) -> Canvas {
        let st = crate::blue::style_for(v);
        let mut c = Canvas::new(40, 40);
        c.fill_rect(c.bounds(), Color::rgb(0x12, 0x34, 0x56));
        st.draw(&mut c, Rect::new(5, 5, 30, 30), Part::GroupBox, state, 1);
        c
    }

    /// Every scheme draws the frame in its own line colour; disabled repeats enabled.
    #[test]
    fn each_scheme_draws_the_frame_in_its_own_line_colour() {
        let navy = variant_frame(Variant::Blue, State::Normal);
        for v in Variant::ALL {
            let c = variant_frame(v, State::Normal);
            for (x, y) in [(20, 5), (5, 20), (34, 20), (20, 34)] {
                assert_eq!(c.get(x, y), line(v), "{v:?}: the line at ({x}, {y})");
            }
            for (x, y) in [(5, 5), (20, 20), (7, 7)] {
                assert_eq!(c.get(x, y), navy.get(x, y), "{v:?}: ({x}, {y}) is bare");
            }
            assert!(
                variant_frame(v, State::Disabled) == c,
                "{v:?} disabled repeats normal"
            );
        }
    }

    #[test]
    fn a_group_box_label_is_blue_in_silver_and_brown_in_olive_green() {
        let ink = |v: Variant| {
            crate::blue::style_for(v)
                .text_color(Part::GroupBox, State::Normal)
                .unwrap()
        };
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            assert_eq!(ink(v), v.scheme().inks.group_label, "{v:?} label ink");
        }
        let (navy, olive) = (ink(Variant::Blue), ink(Variant::Olive));
        assert!(navy.b > navy.r, "navy's label is blue");
        assert!(olive.r > olive.b, "olive green's label is brown");
    }
}
