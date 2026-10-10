//! The check box through [`Part::CheckBox`]: the part's frame for the check
//! and state, stretched over the box's rect (13x13 at scale 1). Greyed boxes
//! use the Disabled frame, else Pressed, Hot or Normal by pointer. The label
//! and focus ring stay the caller's. Blue pixel pins are in the tests below,
//! at scale 1, and doubled at scale 2 (on a scale-2 canvas and on a scale-1
//! canvas with paint scale 2).

use crate::classic::widgets::CheckState;
use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Check, Part, State, Style};

pub fn draw_checkbox(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    checked: bool,
    ptr: Interaction,
) -> bool {
    let state = if checked {
        CheckState::On
    } else {
        CheckState::Off
    };
    draw_checkbox_state(st, c, p, b, state, ptr)
}

pub fn draw_checkbox_disabled(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    checked: bool,
) -> bool {
    let state = if checked {
        CheckState::On
    } else {
        CheckState::Off
    };
    draw_checkbox_state_disabled(st, c, p, b, state)
}

pub fn draw_checkbox_state_disabled(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    state: CheckState,
) -> bool {
    if st.part(Part::CheckBox).is_none() {
        return false;
    }
    st.draw_check(c, b, Part::CheckBox, check(state), State::Disabled, p.s);
    true
}

pub fn draw_checkbox_state(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    state: CheckState,
    ptr: Interaction,
) -> bool {
    if st.part(Part::CheckBox).is_none() {
        return false;
    }
    let frame = super::state(true, ptr);
    st.draw_check(c, b, Part::CheckBox, check(state), frame, p.s);
    true
}

fn check(state: CheckState) -> Check {
    match state {
        CheckState::Off => Check::Unchecked,
        CheckState::On => Check::Checked,
        CheckState::Mixed => Check::Mixed,
    }
}

#[cfg(test)]
mod tests {
    use crate::blue;
    use crate::classic;
    use crate::classic::widgets::CheckState;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::styled::fixture::{one_part_style, solid_frames};
    use crate::look::widgets::{
        draw_checkbox, draw_checkbox_disabled, draw_checkbox_state, draw_checkbox_state_disabled,
    };
    use crate::look::{Look, Paint};
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Check, Layout, Part, PartImage, State};
    use crate::theme::{Color, ColorScheme};
    use std::sync::Arc;

    const B: Rect = Rect::new(2, 2, 13, 13);

    fn at(c: &Canvas, x: i32, y: i32) -> u32 {
        c.get(B.x + x, B.y + y)
    }

    #[test]
    fn a_style_with_a_check_box_part_paints_the_mixed_frame() {
        let inks: Vec<Color> = (0..12).map(|i| Color::rgb(i * 10, 0, 0)).collect();
        let img = PartImage::new_check(
            solid_frames(&inks),
            &[Check::Unchecked, Check::Checked, Check::Mixed],
            Layout::Vertical,
        );
        let look = Look::Styled(Arc::new(one_part_style(Part::CheckBox, img)));
        let cs = ColorScheme::default();
        let p = Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
        };
        let mut c = Canvas::new(13, 13);
        draw_checkbox_state(
            &mut c,
            &p,
            Rect::new(0, 0, 13, 13),
            CheckState::Mixed,
            Interaction::Idle,
        );
        assert_eq!(
            c.get(6, 6),
            inks[8].argb(),
            "a mixed check box paints Mixed Normal, frame 8"
        );
    }

    #[test]
    fn a_classic_check_box_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        for state in [CheckState::Off, CheckState::On, CheckState::Mixed] {
            let (mut a, mut c) = (Canvas::new(17, 17), Canvas::new(17, 17));
            draw_checkbox_state(&mut a, &theme.paint(), B, state, Interaction::Idle);
            classic::widgets::draw_checkbox_state(&mut c, &theme.paint(), B, state);
            assert!(a == c, "a Classic {state:?} check box paints Classic's box");
        }
    }

    fn faces() -> &'static crate::blue::scheme::CheckBoxFaces {
        &blue::Variant::Blue.scheme().check_box
    }

    fn solid(c: Color) -> u32 {
        0xFF00_0000 | c.argb()
    }

    #[test]
    fn every_blue_check_and_state_paints_its_own_box() {
        let st = blue::style();
        let mut seen: Vec<Canvas> = Vec::new();
        for check in [Check::Unchecked, Check::Checked, Check::Mixed] {
            for state in crate::style::CHECK_STATES {
                let mut c = Canvas::new(17, 17);
                st.draw_check(&mut c, B, Part::CheckBox, check, state, 1);
                let rim = match state {
                    State::Disabled => faces().disabled.rim,
                    _ => faces().normal.rim,
                };
                for (x, y) in [(0, 0), (12, 12), (12, 0), (0, 12)] {
                    assert_eq!(
                        at(&c, x, y),
                        solid(rim),
                        "{check:?} {state:?} rim ({x}, {y})"
                    );
                }
                assert_eq!(c.get(1, 1), 0, "{check:?} {state:?} stays inside its box");
                assert_eq!(c.get(15, 15), 0, "{check:?} {state:?} stays inside its box");
                assert!(!seen.contains(&c), "{check:?} {state:?} is its own frame");
                seen.push(c);
            }
        }
    }

    fn blue(draw: impl Fn(&mut Canvas, &Paint)) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::new(17, 17);
        draw(&mut c, &theme.paint());
        c
    }

    #[test]
    fn a_blue_check_box_paints_the_normal_frame_for_its_check() {
        let f = faces();
        let off = blue(|c, p| draw_checkbox(c, p, B, false, Interaction::Idle));
        assert_eq!(at(&off, 0, 0), solid(f.normal.rim), "the rim");
        assert!(
            at(&off, 1, 1) != at(&off, 11, 11),
            "an empty box shades from its top-left to its bottom-right"
        );
        let tick = solid(f.normal_checked.mark.unwrap());
        assert_ne!(at(&off, 6, 7), tick, "an empty box has no tick");

        let on = blue(|c, p| draw_checkbox(c, p, B, true, Interaction::Idle));
        assert_eq!(at(&on, 6, 7), tick, "the tick");

        let mixed = blue(|c, p| draw_checkbox_state(c, p, B, CheckState::Mixed, Interaction::Idle));
        let square = solid(f.normal_mixed.mark.unwrap());
        assert_eq!(at(&mixed, 3, 3), square, "the mixed square");
        assert_ne!(at(&mixed, 2, 2), square, "the mixed square is 3 px in");
        assert_eq!(at(&mixed, 9, 9), square, "the mixed square is 7 px wide");
        assert_ne!(at(&mixed, 10, 10), square, "the mixed square is 7 px wide");
    }

    #[test]
    fn a_blue_check_box_under_the_pointer_or_held_paints_that_frame() {
        let st = blue::style();
        for (check, state) in [
            (CheckState::Off, Interaction::Hot),
            (CheckState::On, Interaction::Hot),
            (CheckState::Off, Interaction::Pressed),
            (CheckState::On, Interaction::Pressed),
        ] {
            let c = blue(|c, p| draw_checkbox_state(c, p, B, check, state));
            let mut want = Canvas::new(17, 17);
            let frame = match state {
                Interaction::Hot => State::Hot,
                _ => State::Pressed,
            };
            st.draw_check(&mut want, B, Part::CheckBox, super::check(check), frame, 1);
            assert!(
                c == want,
                "{check:?} {state:?} is the whole {frame:?} frame"
            );
            let rest = blue(|c, p| draw_checkbox_state(c, p, B, check, Interaction::Idle));
            assert!(c != rest, "{check:?} {state:?} differs from rest");
        }
        let two = blue(|c, p| draw_checkbox(c, p, B, true, Interaction::Hot));
        let three = blue(|c, p| draw_checkbox_state(c, p, B, CheckState::On, Interaction::Hot));
        assert!(
            two == three,
            "the two-state box lights as the three-state one"
        );
    }

    #[test]
    fn a_disabled_blue_check_box_is_a_grey_rim_round_a_flat_well() {
        let f = faces();
        for checked in [false, true] {
            let c = blue(|c, p| draw_checkbox_disabled(c, p, B, checked));
            assert_eq!(at(&c, 0, 0), solid(f.disabled.rim), "the greyed rim");
            assert_eq!(at(&c, 1, 1), at(&c, 11, 11), "the greyed well is flat");
            let tick = solid(f.disabled_checked.mark.unwrap());
            assert_eq!(
                at(&c, 6, 7) == tick,
                checked,
                "checked {checked}: the greyed tick"
            );
        }
    }

    #[test]
    fn a_greyed_mixed_blue_check_box_is_a_grey_square_in_its_well() {
        let f = faces();
        let c = blue(|c, p| draw_checkbox_state_disabled(c, p, B, CheckState::Mixed));
        let mark = solid(f.disabled_mixed.mark.unwrap());
        assert_eq!(at(&c, 0, 0), solid(f.disabled_mixed.rim), "the greyed rim");
        assert_ne!(at(&c, 2, 2), mark, "the square is 3 px in");
        assert_eq!(at(&c, 3, 3), mark, "the square");
        assert_eq!(at(&c, 9, 9), mark, "the square is 7 px wide");
        assert_ne!(at(&c, 10, 10), mark, "the well past the square");
    }

    #[test]
    fn a_greyed_silver_check_box_shows_the_face_through_its_well() {
        let st = blue::style_for(blue::Variant::Silver);
        let f = &blue::Variant::Silver.scheme().check_box;
        let face = Color::rgb(0xE0, 0xDF, 0xE3);
        for (check, mark) in [
            (Check::Unchecked, None),
            (Check::Checked, Some((6, 7))),
            (Check::Mixed, Some((3, 3))),
        ] {
            let mut c = Canvas::new(17, 17);
            c.fill_rect(Rect::new(0, 0, 17, 17), face);
            st.draw_check(&mut c, B, Part::CheckBox, check, State::Disabled, 1);
            let grey = solid(f.disabled.rim);
            assert_eq!(at(&c, 0, 0), grey, "{check:?}: the greyed rim");
            assert_eq!(at(&c, 12, 6), grey, "{check:?}: the greyed rim");
            assert_eq!(
                at(&c, 1, 1),
                face.argb(),
                "{check:?}: the well shows the face"
            );
            assert_eq!(
                at(&c, 11, 11),
                face.argb(),
                "{check:?}: the well shows the face"
            );
            if let Some((x, y)) = mark {
                assert_eq!(at(&c, x, y), grey, "{check:?}: the mark is grey");
            }
        }
    }

    #[test]
    fn a_greyed_classic_check_box_shows_mixed_empty() {
        let theme = theme_for(Interface::Classic);
        for (state, checked) in [
            (CheckState::Off, false),
            (CheckState::On, true),
            (CheckState::Mixed, false),
        ] {
            let (mut a, mut c) = (Canvas::new(17, 17), Canvas::new(17, 17));
            draw_checkbox_state_disabled(&mut a, &theme.paint(), B, state);
            classic::widgets::draw_checkbox_disabled(&mut c, &theme.paint(), B, checked);
            assert!(a == c, "a greyed Classic {state:?} box ticked {checked}");
        }
    }

    #[test]
    fn a_blue_check_box_on_a_scale_2_canvas_is_drawn_at_2() {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::with_scale(17, 17, 2);
        draw_checkbox(&mut c, &theme.paint(), B, true, Interaction::Idle);
        let (w, _) = c.device_size();
        let bytes = c.to_argb_be();
        let device = |x: i32, y: i32| {
            let i = 4 * (y as usize * w as usize + x as usize);
            u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap())
        };
        let f = faces();
        let tick = solid(f.normal_checked.mark.unwrap());
        for (x, y, want) in [(0, 0, solid(f.normal.rim)), (6, 7, tick)] {
            let (dx, dy) = (2 * (B.x + x), 2 * (B.y + y));
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                assert_eq!(
                    device(dx + ox, dy + oy) | 0xFF00_0000,
                    want | 0xFF00_0000,
                    "box pixel ({x}, {y}) covers 2x2 device pixels of {want:08X}"
                );
            }
        }
        let (ex, ey) = (2 * (B.x + 3), 2 * (B.y + 4));
        let edge = (0..4).any(|i| {
            let p = device(ex + i, ey);
            p != tick && (p >> 8 & 0xFF) > (p >> 16 & 0xFF) + 8
        });
        assert!(
            edge,
            "the tick's upper edge is drawn smooth at 2, not doubled"
        );
    }

    #[test]
    fn a_blue_check_box_at_paint_scale_2_doubles_every_pixel() {
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            crate::look::widgets::draw_checkbox(c, p, B.scaled(p.s), true, Interaction::Hot);
            let greyed = Rect::new(20, 2, 13, 13).scaled(p.s);
            crate::look::widgets::draw_checkbox_disabled(c, p, greyed, true);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(36, 17, draw),
            "a checked Hot and a greyed box at s = 2 are their s = 1 pixels doubled"
        );
    }

    #[test]
    fn a_margined_check_box_part_at_paint_scale_2_doubles_every_pixel() {
        let img = PartImage::new_check(
            crate::look::styled::fixture::ramp(13, 13 * 12),
            &[Check::Unchecked, Check::Checked, Check::Mixed],
            Layout::Vertical,
        )
        .sizing(crate::raster::Insets::new(1, 2, 3, 4));
        let look = crate::look::styled::fixture::one_part_look_with(Part::CheckBox, img);
        let cs = ColorScheme::default();
        let draw = |s: i32| {
            let p = Paint {
                cs: &cs,
                look: &look,
                s,
                back: crate::look::Backdrop::Face,
                active: true,
            };
            let mut c = Canvas::new(36 * s as u32, 17 * s as u32);
            draw_checkbox_state(&mut c, &p, B.scaled(s), CheckState::On, Interaction::Hot);
            let greyed = Rect::new(20, 2, 13, 13).scaled(s);
            draw_checkbox_state_disabled(&mut c, &p, greyed, CheckState::Mixed);
            c
        };
        assert!(
            crate::look::styled::fixture::doubles_at_scale_1(&draw(1), &draw(2)),
            "a checked Hot and a greyed mixed box at s = 2 are their s = 1 pixels doubled, 1, 2, 3 and 4 px borders and all"
        );
    }
}
