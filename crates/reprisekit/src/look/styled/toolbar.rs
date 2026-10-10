//! A toolbar through the style's toolbar button, drop and separator parts: each drawer returns `false` when the style lacks them, so [`crate::look::widgets::toolbar`] falls back to Classic. A button draws nothing at rest or greyed; a split button draws main and drop parts as two frames; a held label takes the style's held ink and a greyed one is etched a pixel down and right.

use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::{Align, TextRenderer};

pub fn draw_button(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    split: Option<i32>,
    state: State,
) -> bool {
    if !st.has_parts(&[Part::ToolbarButton, Part::ToolbarDrop]) {
        return false;
    }
    match split {
        Some(x) => {
            let main = Rect::new(r.x, r.y, x - r.x, r.height);
            let drop = Rect::new(x, r.y, r.right() - x, r.height);
            st.draw(c, main, Part::ToolbarButton, state, p.s);
            st.draw(c, drop, Part::ToolbarDrop, state, p.s);
        }
        None => st.draw(c, r, Part::ToolbarButton, state, p.s),
    }
    true
}

pub fn draw_separator(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> bool {
    if st.part(Part::ToolbarSeparator).is_none() {
        return false;
    }
    st.draw(c, r, Part::ToolbarSeparator, State::Normal, p.s);
    true
}

#[expect(clippy::too_many_arguments)]
pub fn draw_label(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    state: State,
    px: u32,
) -> bool {
    if st.part(Part::ToolbarButton).is_none() {
        return false;
    }
    let own = st.text_color(Part::ToolbarButton, state);
    if state == State::Disabled {
        let s = p.s;
        let etch = Rect::new(r.x + s, r.y + s, r.width, r.height);
        text.draw(c, etch, label, false, p.cs.button_hilight, Align::Left, px);
        let ink = own.unwrap_or(p.cs.gray_text);
        text.draw(c, r, label, false, ink, Align::Left, px);
    } else {
        let ink = own.unwrap_or(p.cs.button_text);
        text.draw(c, r, label, false, ink, Align::Left, px);
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::{self, Variant};
    use crate::interfaces;
    use crate::look::styled::fixture::{blue_doubles_at_paint_scale_2, doubles_at_paint_scale_2};
    use crate::look::widgets::toolbar::{draw_button, draw_separator, state};
    use crate::raster::{Canvas, Rect};
    use crate::style::State;
    use crate::theme::Color;

    fn button(v: Variant, h: i32, state: State, split: Option<i32>) -> Canvas {
        let theme = interfaces::blue_theme(v);
        let mut c = Canvas::new(72, h as u32);
        c.fill_rect(c.bounds(), Color::rgb(1, 2, 3));
        draw_button(&mut c, &theme.paint(), Rect::new(0, 0, 72, h), split, state);
        c
    }

    const MARK: u32 = 0xFF010203;

    fn rim(v: Variant, state: State) -> u32 {
        let f = &v.scheme().toolbar_button;
        let face = match state {
            State::Hot => &f.hot,
            State::Pressed => &f.pressed,
            State::Checked => &f.checked,
            _ => &f.checked_hot,
        };
        face.rim.argb()
    }

    fn clear(p: u32) -> bool {
        [16, 8, 0].iter().all(|&s| (p >> s) & 0xFF < 0x10)
    }

    #[test]
    fn a_lit_button_draws_its_frame_at_both_strip_heights() {
        for v in Variant::ALL {
            for h in [30, 22] {
                for st in [
                    State::Hot,
                    State::Pressed,
                    State::Checked,
                    State::CheckedHot,
                ] {
                    let outline = rim(v, st);
                    let c = button(v, h, st, None);
                    assert_eq!(c.get(0, h / 2), outline, "{v:?} {h} {st:?} left outline");
                    assert_eq!(c.get(71, h / 2), outline, "{v:?} {h} {st:?} right outline");
                    assert_eq!(c.get(36, h - 1), outline, "{v:?} {h} {st:?} bottom outline");
                    assert!(clear(c.get(0, 0)), "{v:?} {h} {st:?} rounded corner");
                }
            }
        }
    }

    #[test]
    fn a_button_at_rest_or_greyed_draws_nothing() {
        for v in Variant::ALL {
            for st in [State::Normal, State::Disabled] {
                let c = button(v, 30, st, None);
                assert!(
                    (0..30).all(|y| (0..72).all(|x| c.get(x, y) == MARK)),
                    "{v:?} {st:?}"
                );
            }
        }
    }

    #[test]
    fn a_split_button_draws_two_frames() {
        let c = button(Variant::Blue, 30, State::Hot, Some(60));
        let outline = rim(Variant::Blue, State::Hot);
        assert_eq!(c.get(59, 15), outline, "the main part's right outline");
        assert_eq!(c.get(60, 15), outline, "the drop part's left outline");
        assert!(clear(c.get(59, 0)), "the main part's corner");
    }

    #[test]
    fn a_separator_is_a_one_pixel_line() {
        let theme = interfaces::blue_theme(Variant::Blue);
        let mut c = Canvas::new(6, 30);
        c.fill_rect(c.bounds(), Color::rgb(1, 2, 3));
        draw_separator(&mut c, &theme.paint(), true, Rect::new(0, 0, 6, 30));
        for y in 0..30 {
            let want = if (2..29).contains(&y) {
                Variant::Blue.scheme().toolbar_separator.normal.rim.argb()
            } else {
                MARK
            };
            assert_eq!(c.get(2, y), want, "y {y}");
        }
        assert!((0..30).all(|y| [0, 1, 3, 4, 5].iter().all(|&x| c.get(x, y) == MARK)));
    }

    /// Disabled beats everything, held beats latched, latched beats the pointer.
    #[test]
    fn state_orders_greyed_held_latched_hot() {
        assert_eq!(state(false, true, true, true), State::Disabled);
        assert_eq!(state(true, true, true, true), State::Pressed);
        assert_eq!(state(true, true, false, true), State::CheckedHot);
        assert_eq!(state(true, false, false, true), State::Checked);
        assert_eq!(state(true, true, false, false), State::Hot);
        assert_eq!(state(true, false, false, false), State::Normal);
    }

    #[test]
    fn classic_has_no_toolbar_parts() {
        let theme = interfaces::theme_for(crate::interface::Interface::Classic);
        assert!(theme.look.style().is_none());
        assert!(
            blue::style()
                .part(crate::style::Part::ToolbarButton)
                .is_some()
        );
    }

    #[test]
    fn toolbar_parts_double_at_scale_2() {
        for h in [30, 22] {
            for st in [
                State::Hot,
                State::Pressed,
                State::Checked,
                State::CheckedHot,
            ] {
                assert!(
                    blue_doubles_at_paint_scale_2(72, h as u32, |c, p| {
                        let r = Rect::new(0, 0, 72, h).scaled(p.s);
                        draw_button(c, p, r, None, st);
                    }),
                    "{st:?} at {h}"
                );
            }
            assert!(blue_doubles_at_paint_scale_2(72, h as u32, |c, p| {
                let r = Rect::new(0, 0, 72, h).scaled(p.s);
                draw_button(c, p, r, Some(60 * p.s), State::Hot);
            }));
        }
        assert!(doubles_at_paint_scale_2(
            crate::interface::Interface::Blue,
            6,
            30,
            |c, p| draw_separator(c, p, true, Rect::new(0, 0, 6, 30).scaled(p.s)),
        ));
    }
}
