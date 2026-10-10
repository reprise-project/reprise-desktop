//! The tab strip through [`Part::TabItem`]: a tab's frame (the open one `Checked`) with its label in the part's content rect.
//! The open tab is 2 px taller and wider each side than a closed one and draws one row further down.

use crate::classic::widgets::{RingInk, draw_focus_ring_screen, ring_origin};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::{Align, TextRenderer};

const GROWN: i32 = 2;

/// [`crate::look::widgets::draw_tab`] through `st`; `false` when `st` has
/// no tab part.
#[expect(clippy::too_many_arguments)]
pub fn draw_tab(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    open: bool,
    hot: bool,
    page: Rect,
    px: u32,
) -> bool {
    if st.part(Part::TabItem).is_none() {
        return false;
    }
    let s = p.s;
    let (state, frame, closed) = if open {
        let g = GROWN * s;
        let h = if page.y == r.bottom() {
            r.height + s
        } else {
            r.height
        };
        (
            State::Checked,
            Rect::new(r.x, r.y, r.width, h),
            Rect::new(r.x + g, r.y, r.width - 2 * g, r.height - g),
        )
    } else if hot {
        (State::Hot, r, r)
    } else {
        (State::Normal, r, r)
    };
    st.draw(c, frame, Part::TabItem, state, p.s);
    let content = st.content_rect(Part::TabItem, closed, p.s);
    text.draw(
        c,
        content,
        label,
        false,
        p.cs.button_text,
        Align::Center,
        px,
    );
    true
}

/// [`crate::look::widgets::draw_tab_focus`] through `st`: Classic's ring
/// geometry, each dot the pixel under it XORed with `button_face`, as the
/// push button's ring; `false` when `st` has no tab part.
pub fn draw_tab_focus(st: &Style, c: &mut Canvas, p: &Paint, ring: Rect) -> bool {
    if st.part(Part::TabItem).is_none() {
        return false;
    }
    let ink = RingInk::Xor(p.cs.button_face);
    draw_focus_ring_screen(c, ring, p.s, ink, ring_origin(ring, p.s));
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::Variant;
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{draw_page, draw_tab};
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};
    use crate::text::TextRenderer;
    use crate::theme::{Color, Theme};

    const FACE: Color = Color::rgb(0xEC, 0xE9, 0xD8);

    fn border() -> u32 {
        Variant::Blue.scheme().tab_pane.normal.rim.argb()
    }

    fn pane() -> u32 {
        Variant::Blue.scheme().tab_pane.normal.down[0].1.argb()
    }

    fn blue(w: i32, h: i32, draw: impl Fn(&mut Canvas, &Theme)) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(Rect::new(0, 0, w, h), FACE);
        draw(&mut c, &theme);
        c
    }

    fn tab_frame(r: Rect, state: State) -> Canvas {
        blue(r.right() + 2, r.bottom() + 2, |c, _| {
            crate::blue::style().draw(c, r, Part::TabItem, state, 1);
        })
    }

    fn ink(c: &Canvas, r: Rect) -> Option<(i32, i32, i32, i32)> {
        let black = Color::rgb(0, 0, 0).argb();
        let pts: Vec<(i32, i32)> = (r.x..r.right())
            .flat_map(|x| (r.y..r.bottom()).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == black)
            .collect();
        let xs = pts.iter().map(|p| p.0);
        let ys = pts.iter().map(|p| p.1);
        Some((xs.clone().min()?, ys.clone().min()?, xs.max()?, ys.max()?))
    }

    #[test]
    fn a_classic_look_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let r = Rect::new(2, 2, 60, 20);
        let page = Rect::new(2, 22, 200, 100);
        let (mut a, mut b) = (Canvas::new(210, 130), Canvas::new(210, 130));
        draw_tab(
            &mut a,
            &text,
            &theme.paint(),
            r,
            "Tab",
            true,
            false,
            page,
            px,
        );
        classic::widgets::draw_tab(&mut b, &text, &theme.paint(), r, "Tab", true, page, px);
        draw_page(&mut a, &theme.paint(), page);
        classic::widgets::draw_page(&mut b, &theme.paint(), page);
        assert!(a == b, "a Classic look paints the Classic painter");
    }

    /// A closed tab under the pointer draws its Hot frame, a closed one
    /// at rest its Normal frame.
    #[test]
    fn a_blue_closed_tab_draws_its_hot_frame_under_the_pointer() {
        let r = Rect::new(2, 2, 52, 18);
        let page = Rect::new(0, r.bottom(), 60, 10);
        let strip = |hot: bool| {
            blue(r.right() + 2, r.bottom() + 2, |c, theme| {
                let text = TextRenderer::load(&theme.fonts);
                let px = theme.fonts.size_px;
                draw_tab(c, &text, &theme.paint(), r, "", false, hot, page, px);
            })
        };
        assert!(
            strip(true) == tab_frame(r, State::Hot),
            "a hot closed tab is its Hot frame"
        );
        assert!(
            strip(false) == tab_frame(r, State::Normal),
            "a closed tab at rest is its Normal frame"
        );
    }

    /// The open tab: its lit top over a flat body in the page's own
    /// colour, stretched to its 20 rows, drawn one row further down over
    /// the page's top border so the tab runs into the page.
    #[test]
    fn a_blue_open_tab_runs_into_the_page_under_it_hot_or_not() {
        let r = Rect::new(4, 2, 56, 20);
        let page = Rect::new(2, 22, 100, 40);
        let c = blue(110, 70, |c, theme| {
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            draw_page(c, &p, page);
            draw_tab(c, &text, &p, r, "", true, false, page, theme.fonts.size_px);
        });
        let at = |dx: i32, dy: i32| c.get(r.x + dx, r.y + dy);
        let right = r.width - 1;
        let lit = Variant::Blue.scheme().tab_item.checked.mark.unwrap().argb();
        for (dx, dy, want, what) in [
            (20, 0, lit, "the top row is the lit outline"),
            (0, 3, border(), "the left side runs down from the third row"),
            (
                right,
                3,
                border(),
                "the right side is the last column: no gap",
            ),
            (20, 3, pane(), "the body is the page's colour"),
            (20, 19, pane(), "the body stays flat to the tab's bottom"),
            (
                0,
                20,
                border(),
                "the left side carries on over the page's top",
            ),
            (
                right,
                20,
                border(),
                "the right side carries on over the page's top",
            ),
            (1, 20, pane(), "the body covers the page's top border"),
            (
                right - 1,
                20,
                pane(),
                "the body covers the page's top border",
            ),
        ] {
            assert_eq!(at(dx, dy), want, "({dx}, {dy}): {what}");
        }
        assert_eq!(
            c.get(r.right() + 5, page.y),
            border(),
            "the page's top border resumes past the open tab"
        );
        let hot = blue(110, 70, |c, theme| {
            let text = TextRenderer::load(&theme.fonts);
            let p = theme.paint();
            draw_page(c, &p, page);
            draw_tab(c, &text, &p, r, "", true, true, page, theme.fonts.size_px);
        });
        assert!(hot == c, "the open tab under the pointer draws as at rest");
    }

    /// The open tab's label stays where its closed tab put it: centred in
    /// the closed tab's width a pixel in from the left, raised 2 px with the
    /// tab's top.
    #[test]
    fn a_blue_open_tab_label_sits_where_the_closed_one_did_raised_2_px() {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        if !text.has_face("Tahoma") {
            return;
        }
        let closed = Rect::new(7, 7, 50, 18);
        let open = Rect::new(5, 5, 54, 20);
        let page = Rect::new(5, 25, 80, 30);
        let label = |r: Rect, is_open: bool| {
            let c = blue(90, 60, |c, theme| {
                let p = theme.paint();
                draw_tab(
                    c,
                    &text,
                    &p,
                    r,
                    "Themes",
                    is_open,
                    false,
                    page,
                    theme.fonts.size_px,
                );
            });
            ink(&c, r).expect("the label draws in black")
        };
        let (cl, ct, cr, cb) = label(closed, false);
        assert_eq!(
            label(open, true),
            (cl, ct - 2, cr, cb - 2),
            "the open label is the closed label raised 2 px"
        );
        let ((l, t, _, b), px) = ((cl, ct, cr, cb), theme.fonts.size_px);
        let w = text.measure("Themes", false, px);
        assert_eq!(
            l - closed.x,
            1 + (closed.width - 1 - w) / 2,
            "the label is centred a pixel in from the tab's left"
        );
        assert!(
            t > closed.y + 3 && b < closed.bottom() - 1,
            "the label sits below the tab's rounded top and clear of its bottom"
        );
    }

    /// An open tab and the page at s = 2 are drawn at twice the size from
    /// the scale-2 style: the same layout, every edge where the s = 1
    /// one's lands doubled.
    #[test]
    fn a_blue_tab_and_page_at_paint_scale_2_keep_their_layout() {
        let theme = theme_for(Interface::Blue);
        let text = TextRenderer::load(&theme.fonts);
        let draw = |s: i32| {
            let mut t = theme.clone();
            t.scale = s;
            let p = t.paint();
            let mut c = Canvas::new((62 * s) as u32, (63 * s) as u32);
            c.fill_rect(Rect::new(0, 0, 62 * s, 63 * s), FACE);
            let page = Rect::new(1, 22, 60, 40).scaled(s);
            draw_page(&mut c, &p, page);
            let tab = Rect::new(3, 2, 40, 21).scaled(s);
            draw_tab(&mut c, &text, &p, tab, "", true, false, page, 11 * s as u32);
            c
        };
        let (one, two) = (draw(1), draw(2));
        for (x, y) in [(1, 22), (1, 40), (58, 40), (20, 10), (3, 10), (42, 10)] {
            for (ox, oy) in [(0, 0), (1, 1)] {
                assert_eq!(
                    two.get(2 * x + ox, 2 * y + oy),
                    one.get(x, y),
                    "({x}, {y}) lands doubled"
                );
            }
        }
    }
}
