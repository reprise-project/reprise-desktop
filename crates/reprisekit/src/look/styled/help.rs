//! A dialog caption's ? box through [`Part::HelpButton`]: the part's frame drawn into the box's
//! slot, `Inactive` on an unfocused window, else Hot/Pressed per pointer. A style whose ? box joins
//! its close box ([`Style::joined_help`]) draws the part across both boxes.

use crate::pointer::Interaction;
use crate::raster::{Canvas, Insets, Rect};
use crate::style::{Part, State, Style};

pub fn draw_help_button(
    st: &Style,
    c: &mut Canvas,
    r: Rect,
    active: bool,
    ptr: Interaction,
    close: Option<(Rect, bool, f32)>,
    s: i32,
) -> bool {
    let Some(p) = st.part(Part::HelpButton) else {
        return false;
    };
    let lit = |ptr: Interaction| match (active, ptr) {
        (false, Interaction::Idle) => State::Inactive,
        (false, _) => State::InactiveHot,
        (true, Interaction::Pressed) => State::Pressed,
        (true, Interaction::Hot) => State::Hot,
        (true, Interaction::Idle) => State::Normal,
    };
    if !st.joined_help {
        let state = if active { lit(ptr) } else { State::Inactive };
        st.draw(c, r, Part::HelpButton, state, s);
        return true;
    }
    let Some((close, held, glow)) = close else {
        // No close box: the ? cell alone, cut from the left of the strip's frame and stretched into
        // the box.
        let f = p.frame(lit(ptr));
        let cell = r.width / s.max(1);
        let cell_box = |c: &mut Canvas, r: Rect, s: i32| {
            let k = (s.max(1) / p.scale).max(1);
            let from = Rect::new(f.x, f.y, cell * p.scale, f.height).scaled(k);
            c.clear_rect(r);
            c.blit_nine(&p.atlas, from, r, Insets::ZERO, p.fill, false, k);
        };
        let cs = c.scale();
        if p.scale > 1 && cs > 1 {
            c.in_device_px(|c| cell_box(c, r.scaled(cs), s.max(1) * cs));
        } else {
            cell_box(c, r, s);
        }
        return true;
    };
    let strip = Rect::new(r.x, r.y, close.right() - r.x, r.height);
    c.clear_rect(strip);
    if ptr != Interaction::Idle {
        st.draw_nth(c, strip, Part::HelpButton, lit(ptr), 0, s);
    } else if held {
        st.draw_nth(c, strip, Part::HelpButton, lit(Interaction::Pressed), 1, s);
    } else {
        let close_lit = [(lit(Interaction::Hot), 1, glow)];
        st.draw_mixed(c, strip, Part::HelpButton, (lit(ptr), 0), &close_lit, s);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::draw_help_button;
    use crate::blue::Variant;
    use crate::look::styled::fixture::one_part_style;
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Layout, Part, PartImage, State};
    use crate::theme::Color;

    const NO_CLOSE: Option<(Rect, bool, f32)> = None;

    const FACE: Color = Color::rgb(0x11, 0x22, 0x33);

    #[test]
    fn a_style_with_no_help_button_part_draws_nothing() {
        let img = PartImage::new(
            crate::look::styled::fixture::solid_frames(&[Color::rgb(0, 0, 0)]),
            &[State::Normal],
            Layout::Vertical,
        );
        let style = one_part_style(Part::PushButton, img);
        let mut c = Canvas::new(21, 21);
        c.fill_rect(Rect::new(0, 0, 21, 21), FACE);
        let before = c.clone();
        assert!(!draw_help_button(
            &style,
            &mut c,
            Rect::new(0, 0, 21, 21),
            true,
            Interaction::Idle,
            NO_CLOSE,
            1
        ));
        assert!(
            c == before,
            "a style with no help button part leaves the canvas untouched"
        );
    }

    fn lit(v: Variant, active: bool, ptr: Interaction, s: i32) -> Canvas {
        let side = 21 * s;
        let mut c = Canvas::new(side as u32, side as u32);
        let r = Rect::new(0, 0, side, side);
        c.fill_rect(r, FACE);
        let st = crate::blue::style_for(v).at_scale(s);
        assert!(draw_help_button(&st, &mut c, r, active, ptr, NO_CLOSE, s));
        c
    }

    fn frame(v: Variant, state: State, s: i32) -> Canvas {
        let side = 21 * s;
        let mut c = Canvas::new(side as u32, side as u32);
        let r = Rect::new(0, 0, side, side);
        c.fill_rect(r, FACE);
        crate::blue::style_for(v)
            .at_scale(s)
            .draw(&mut c, r, Part::HelpButton, state, s);
        c
    }

    #[test]
    fn a_blue_help_button_picks_its_frame_by_focus_and_pointer() {
        for v in Variant::ALL {
            for s in [1, 2] {
                let picks = [
                    (true, Interaction::Idle, State::Normal),
                    (true, Interaction::Hot, State::Hot),
                    (true, Interaction::Pressed, State::Pressed),
                    (false, Interaction::Idle, State::Inactive),
                    (false, Interaction::Hot, State::Inactive),
                    (false, Interaction::Pressed, State::Inactive),
                ];
                for (active, ptr, state) in picks {
                    assert!(
                        lit(v, active, ptr, s) == frame(v, state, s),
                        "{v:?} at {s}: active {active}, {ptr:?} draws {state:?}"
                    );
                }
                let states = [State::Normal, State::Hot, State::Pressed, State::Inactive];
                for (i, a) in states.iter().enumerate() {
                    for b in &states[i + 1..] {
                        assert!(
                            frame(v, *a, s) != frame(v, *b, s),
                            "{v:?} at {s}: {a:?} and {b:?} differ"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_blue_help_button_at_scale_2_is_drawn_at_2() {
        for v in Variant::ALL {
            let (one, two) = (
                lit(v, true, Interaction::Idle, 1),
                lit(v, true, Interaction::Idle, 2),
            );
            assert_eq!((two.width, two.height), (42, 42));
            let doubled = (0..42).all(|y| (0..42).all(|x| two.get(x, y) == one.get(x / 2, y / 2)));
            assert!(!doubled, "{v:?}: the scale-2 box is its own drawing");
        }
    }
}
