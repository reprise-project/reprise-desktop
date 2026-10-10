//! The spinner's buttons through [`Part::SpinUp`] and [`Part::SpinDown`]:
//! each part's frame drawn into its button's rect.
//!
//! Blue lays buttons out at 15x8. The atlas fixes a plain seam column and
//! row for a nine-grid to stretch, so a larger rect (Classic's 16 px buddy
//! column) repeats a seam instead of any part of the chevron. Pixel pins for
//! the buttons are in `blue::controls::spin`; the tests here hold the
//! forwarder to them.

use crate::classic::widgets::Arrow;
use crate::look::Paint;
use crate::pointer::{Lit, lit_of};
use crate::raster::{Canvas, Rect};
use crate::style::{Part, Style};

/// [`crate::look::widgets::draw_spin_buttons`] through `st`; `false` when `st`
/// lacks either spin part. The white well and the gap row between the halves
/// belong to [`super::field::draw_spin_field`]. The `lit` button takes its Hot
/// or Pressed frame; a greyed spinner ignores the pointer.
pub fn draw_spin_buttons(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    up: Rect,
    down: Rect,
    enabled: bool,
    lit: Lit<Arrow>,
) -> bool {
    if !st.has_parts(&[Part::SpinUp, Part::SpinDown]) {
        return false;
    }
    let state = |a| super::state(enabled, lit_of(lit, a));
    st.draw(c, up.scaled(p.s), Part::SpinUp, state(Arrow::Up), p.s);
    st.draw(c, down.scaled(p.s), Part::SpinDown, state(Arrow::Down), p.s);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue;
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_spin_buttons as look_draw_spin_buttons;
    use crate::pointer::Interaction;
    use crate::style::State;

    #[test]
    fn a_classic_look_paints_the_classic_spin_buttons() {
        let theme = theme_for(Interface::Classic);
        let up = Rect::new(2, 2, 15, 8);
        let down = Rect::new(2, 10, 15, 8);
        for enabled in [true, false] {
            let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
            look_draw_spin_buttons(
                &mut a,
                &theme.paint(),
                up,
                down,
                enabled,
                Some((Arrow::Up, Interaction::Pressed)),
            );
            classic::widgets::draw_spin_buttons(&mut b, &theme.paint(), up, down, enabled);
            assert!(
                a == b,
                "a Classic spinner (enabled {enabled}) paints Classic's own buttons, the only ones Classic has"
            );
        }
    }

    fn px(part: Part, state: State, x: i32, y: i32) -> u32 {
        let s = blue::style();
        let p = s.part(part).unwrap();
        let f = p.frame(state);
        p.atlas.get(f.x + x, f.y + y)
    }

    #[test]
    fn each_blue_spin_button_is_15_by_8_px_a_frame() {
        let s = blue::style();
        for part in [Part::SpinUp, Part::SpinDown] {
            let p = s.part(part).unwrap();
            assert_eq!(p.frame(State::Normal), Rect::new(0, 0, 15, 8), "{part:?}");
            assert_eq!(p.frame(State::Hot), Rect::new(0, 8, 15, 8), "{part:?}");
            assert_eq!(p.frame(State::Pressed), Rect::new(0, 16, 15, 8), "{part:?}");
            assert_eq!(
                p.frame(State::Disabled),
                Rect::new(0, 24, 15, 8),
                "{part:?}"
            );
        }
    }

    /// The chevron's ink, read from the middle of its solid core.
    fn ink(part: Part, state: State) -> u32 {
        match part {
            Part::SpinUp => px(part, state, 7, 3),
            _ => px(part, state, 7, 4),
        }
    }

    #[test]
    fn the_chevron_inks_the_faces_mark_at_rest_hot_and_pressed() {
        let s = blue::Variant::Blue.scheme();
        for (part, faces) in [
            (
                Part::SpinUp,
                [s.spin_up.normal, s.spin_up.hot, s.spin_up.pressed],
            ),
            (
                Part::SpinDown,
                [s.spin_down.normal, s.spin_down.hot, s.spin_down.pressed],
            ),
        ] {
            for (state, face) in [State::Normal, State::Hot, State::Pressed]
                .into_iter()
                .zip(faces)
            {
                assert_eq!(
                    ink(part, state),
                    face.mark.unwrap().argb(),
                    "{part:?} {state:?}"
                );
            }
        }
    }

    /// The button under the pointer takes Hot, held takes Pressed, the other
    /// stays at rest; a greyed spinner ignores the pointer.
    #[test]
    fn the_lit_blue_spin_button_takes_its_hot_or_pressed_frame() {
        let theme = theme_for(Interface::Blue);
        let (up, down) = (Rect::new(2, 2, 15, 8), Rect::new(2, 11, 15, 8));
        let draw = |enabled, lit| {
            let mut c = Canvas::new(20, 22);
            look_draw_spin_buttons(&mut c, &theme.paint(), up, down, enabled, lit);
            c
        };
        let frame_of = |c: &Canvas, r: Rect, part: Part, state: State| {
            (0..8).all(|y| (0..15).all(|x| c.get(r.x + x, r.y + y) == px(part, state, x, y)))
        };
        for (arrow, how, state) in [
            (Arrow::Up, Interaction::Hot, State::Hot),
            (Arrow::Up, Interaction::Pressed, State::Pressed),
            (Arrow::Down, Interaction::Hot, State::Hot),
            (Arrow::Down, Interaction::Pressed, State::Pressed),
        ] {
            let c = draw(true, Some((arrow, how)));
            let (lit, rest, lit_part, rest_part) = match arrow {
                Arrow::Up => (up, down, Part::SpinUp, Part::SpinDown),
                _ => (down, up, Part::SpinDown, Part::SpinUp),
            };
            assert!(
                frame_of(&c, lit, lit_part, state),
                "{arrow:?} {how:?}: its frame"
            );
            assert!(
                frame_of(&c, rest, rest_part, State::Normal),
                "{arrow:?} {how:?}: the other at rest"
            );
        }
        let greyed = draw(false, None);
        for lit in [
            Some((Arrow::Up, Interaction::Hot)),
            Some((Arrow::Down, Interaction::Pressed)),
        ] {
            assert!(draw(false, lit) == greyed, "a greyed spinner under {lit:?}");
        }
    }

    #[test]
    fn a_greyed_button_inks_its_chevron_lighter_than_at_rest() {
        let luma = |p: u32| ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF);
        for part in [Part::SpinUp, Part::SpinDown] {
            let (rest, greyed) = (ink(part, State::Normal), ink(part, State::Disabled));
            assert!(luma(greyed) > luma(rest), "{part:?}");
        }
    }

    /// Pixels painted `ink` inside `r`: the chevron's solid core, so a stretch never duplicates or drops one.
    fn chevron_ink_count(c: &Canvas, r: Rect, ink: u32) -> usize {
        (r.y..r.y + r.height)
            .flat_map(|y| (r.x..r.x + r.width).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    }

    #[test]
    fn a_buddy_spinner_column_one_px_wider_repeats_its_seam_not_the_chevron() {
        let s = blue::style();
        for part in [Part::SpinUp, Part::SpinDown] {
            let draw = |w: i32| {
                let mut c = Canvas::new(20, 12);
                s.draw(&mut c, Rect::new(2, 2, w, 8), part, State::Normal, 1);
                c
            };
            let (plain, wide) = (draw(15), draw(16));
            let r = Rect::new(2, 2, 16, 8);
            for y in r.y..r.y + r.height {
                assert_eq!(
                    wide.get(r.x + 3, y),
                    wide.get(r.x + 4, y),
                    "{part:?} seam column repeats at row {y}"
                );
            }
            let ink = ink(part, State::Normal);
            let want = chevron_ink_count(&plain, r, ink);
            assert!(want >= 12, "{part:?}: a solid chevron, {want} px");
            assert_eq!(
                chevron_ink_count(&wide, r, ink),
                want,
                "{part:?} chevron's own pixels all land, none doubled"
            );
        }
    }

    /// At scale 2 the frame is the scale-2 atlas's, and a wider button lands the same solid chevron.
    #[test]
    fn at_scale_2_the_buttons_are_painted_at_2() {
        let s = blue::style();
        let two = s.at_scale(2);
        for part in [Part::SpinUp, Part::SpinDown] {
            let img = two.part(part).unwrap();
            assert_eq!(img.scale, 2, "{part:?}");
            let f = img.frame(State::Hot);
            let draw = |w: i32| {
                let mut c = Canvas::new(40, 24);
                two.draw(&mut c, Rect::new(0, 0, 2 * w, 16), part, State::Hot, 2);
                c
            };
            let plain = draw(15);
            for y in 0..16 {
                for x in 0..30 {
                    assert_eq!(
                        plain.get(x, y),
                        img.atlas.get(f.x + x, f.y + y),
                        "{part:?} ({x}, {y})"
                    );
                }
            }
            let ink = ink(part, State::Hot);
            let r = Rect::new(0, 0, 32, 16);
            assert_eq!(
                chevron_ink_count(&draw(16), r, ink),
                chevron_ink_count(&plain, r, ink),
                "{part:?}: the stretch lands every solid chevron pixel at 2"
            );
        }
    }

    /// Offsets of the `ink` pixels from their own top-left corner, so draws at different sizes compare shapes alone.
    fn ink_shape(c: &Canvas, r: Rect, ink: u32) -> std::collections::BTreeSet<(i32, i32)> {
        let hits: Vec<(i32, i32)> = (r.y..r.y + r.height)
            .flat_map(|y| (r.x..r.x + r.width).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .collect();
        let min_x = hits.iter().map(|&(x, _)| x).min().unwrap();
        let min_y = hits.iter().map(|&(_, y)| y).min().unwrap();
        hits.into_iter()
            .map(|(x, y)| (x - min_x, y - min_y))
            .collect()
    }

    /// A year field's buttons are 16x9 or 16x10, the plain spinner 15x8; the chevron keeps its shape at each.
    #[test]
    fn the_chevron_keeps_its_shape_at_every_buddy_size() {
        let s = blue::style();
        for part in [Part::SpinUp, Part::SpinDown] {
            for state in [State::Normal, State::Disabled] {
                let ink = ink(part, state);
                let shapes: Vec<_> = [(15, 8), (16, 9), (16, 10)]
                    .into_iter()
                    .map(|(w, h)| {
                        let r = Rect::new(2, 2, w, h);
                        let mut c = Canvas::new(24, 16);
                        s.draw(&mut c, r, part, state, 1);
                        ink_shape(&c, r, ink)
                    })
                    .collect();
                assert_eq!(shapes[0], shapes[1], "{part:?} {state:?} 15x8 vs 16x9");
                assert_eq!(shapes[0], shapes[2], "{part:?} {state:?} 15x8 vs 16x10");
            }
        }
    }

    /// Two 8 px buttons with a gap row, as a 21 px field's 17 px column splits:
    /// the field wells the column in white and runs the gap row in face across it.
    #[test]
    fn the_field_wells_the_buttons_with_a_face_gap_row() {
        let theme = theme_for(Interface::Blue);
        let p = theme.paint();
        let (white, face) = (p.cs.window.argb(), p.cs.button_face.argb());
        let (up, down) = (Rect::new(3, 3, 15, 8), Rect::new(3, 12, 15, 8));
        let field = Rect::new(0, 0, 21, 23);
        for enabled in [true, false] {
            let mut c = Canvas::new(22, 24);
            c.fill_rect(c.bounds(), crate::theme::Color::rgb(1, 2, 3));
            crate::look::widgets::draw_spin_field(&mut c, &p, field, up, down, enabled);
            look_draw_spin_buttons(&mut c, &p, up, down, enabled, None);
            for x in 2..19 {
                assert_eq!(c.get(x, 2), white, "well top at x {x}");
                assert_eq!(c.get(x, 20), white, "well bottom at x {x}");
                assert_eq!(c.get(x, 11), face, "gap row at x {x}");
            }
            for y in (3..11).chain(12..20) {
                assert_eq!(c.get(2, y), white, "well left at y {y}");
                assert_eq!(c.get(18, y), white, "well right at y {y}");
            }
        }
    }

    #[test]
    fn the_buttons_draw_nothing_outside_their_rects() {
        let theme = theme_for(Interface::Blue);
        let p = theme.paint();
        let (up, down) = (Rect::new(3, 3, 15, 8), Rect::new(3, 12, 15, 8));
        let under = crate::theme::Color::rgb(1, 2, 3).argb();
        for enabled in [true, false] {
            let mut c = Canvas::new(22, 24);
            c.fill_rect(c.bounds(), crate::theme::Color::rgb(1, 2, 3));
            look_draw_spin_buttons(&mut c, &p, up, down, enabled, None);
            for y in 0..24 {
                for x in 0..22 {
                    if !up.contains(x, y) && !down.contains(x, y) {
                        assert_eq!(c.get(x, y), under, "({x}, {y}) is untouched");
                    }
                }
            }
        }
    }

    #[test]
    fn a_style_with_only_the_up_part_falls_back_to_classic_whole() {
        let look = crate::look::styled::fixture::one_part_look(Part::SpinUp);
        let cs = crate::theme::ColorScheme::default();
        let p = Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
        };
        let (up, down) = (Rect::new(2, 2, 15, 8), Rect::new(2, 10, 15, 8));
        let (mut a, mut b) = (Canvas::new(20, 20), Canvas::new(20, 20));
        look_draw_spin_buttons(&mut a, &p, up, down, true, None);
        classic::widgets::draw_spin_buttons(&mut b, &p, up, down, true);
        assert!(a == b, "a style with no down part paints Classic's spinner");
    }
}
