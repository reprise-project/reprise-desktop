use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Check, Part, State, Style};

pub fn draw_radio(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    b: Rect,
    checked: bool,
    ptr: Interaction,
) -> bool {
    radio(st, c, p, b, checked, super::state(true, ptr))
}

pub fn draw_radio_disabled(st: &Style, c: &mut Canvas, p: &Paint, b: Rect, checked: bool) -> bool {
    radio(st, c, p, b, checked, State::Disabled)
}

fn radio(st: &Style, c: &mut Canvas, p: &Paint, b: Rect, checked: bool, state: State) -> bool {
    if st.part(Part::Radio).is_none() {
        return false;
    }
    let check = if checked {
        Check::Checked
    } else {
        Check::Unchecked
    };
    st.draw_check(c, b, Part::Radio, check, state, p.s);
    true
}

#[cfg(test)]
mod tests {
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{draw_radio, draw_radio_disabled};
    use crate::metrics::ControlMetrics;
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Check, Part, State};
    use crate::theme::{Color, Theme};

    const FACE: Color = Color::rgb(0xEC, 0xE9, 0xD8);

    fn blue(draw: impl Fn(&mut Canvas, &Theme, Rect)) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::new(20, 20);
        c.fill_rect(Rect::new(0, 0, 20, 20), FACE);
        draw(&mut c, &theme, Rect::new(2, 2, 13, 13));
        c
    }

    fn frame(check: Check, state: State) -> Canvas {
        blue(|c, _, b| {
            let r = Rect::new(b.x, b.y, 13, 13);
            crate::blue::style().draw_check(c, r, Part::Radio, check, state, 1);
        })
    }

    #[track_caller]
    fn pin(c: &Canvas, x: i32, y: i32, rgb: u32, what: &str) {
        assert_eq!(
            c.get(2 + x, 2 + y),
            0xFF00_0000 | rgb,
            "{what}: ({x}, {y}) is {rgb:06X}"
        );
    }

    #[test]
    fn a_classic_look_paints_the_classic_radio_button() {
        let theme = theme_for(Interface::Classic);
        let side = ControlMetrics::CLASSIC.radio;
        let b = Rect::new(2, 2, side, side);
        let (mut a, mut c) = (Canvas::new(20, 20), Canvas::new(20, 20));
        draw_radio(&mut a, &theme.paint(), b, true, Interaction::Idle);
        classic::widgets::draw_radio(&mut c, &theme.paint(), b, true);
        assert!(
            a == c,
            "a Classic checked radio button is the Classic painter's"
        );
    }

    fn faces() -> &'static crate::blue::scheme::RadioFaces {
        &crate::blue::Variant::Blue.scheme().radio
    }

    fn rgb(c: Color) -> u32 {
        c.argb() & 0x00FF_FFFF
    }

    #[test]
    fn a_blue_radio_button_fills_its_13_px_rect() {
        let c = blue(|c, t, b| draw_radio(c, &t.paint(), b, false, Interaction::Idle));
        let face = FACE.argb();
        for (x, y) in [(6, 0), (0, 6), (12, 6), (6, 12)] {
            assert_ne!(c.get(2 + x, 2 + y), face, "the rim reaches ({x}, {y})");
        }
        assert_eq!(c.get(2 + 6, 1), face, "nothing above the circle");
        assert_eq!(c.get(1, 2 + 6), face, "nothing left of the circle");
        assert_eq!(c.get(2 + 13, 2 + 6), face, "nothing right of the circle");
        assert_eq!(c.get(2 + 6, 2 + 13), face, "nothing below the circle");
    }

    #[test]
    fn a_blue_radio_buttons_rim_blends_into_the_face() {
        let c = blue(|c, t, b| draw_radio(c, &t.paint(), b, false, Interaction::Idle));
        pin(&c, 0, 0, 0xECE9D8, "clear corner");
        pin(&c, 12, 12, 0xECE9D8, "clear corner");
        let rim = faces().normal.rim;
        let between = |p: u32, i: u32| {
            let (a, b, v) = (
                FACE.argb() >> i & 0xFF,
                rim.argb() >> i & 0xFF,
                p >> i & 0xFF,
            );
            a.min(b) <= v && v <= a.max(b)
        };
        for (x, y) in [(3, 0), (4, 0), (1, 2), (2, 1)] {
            let p = c.get(2 + x, 2 + y);
            assert!(
                p != FACE.argb() && p != (0xFF00_0000 | rim.argb()),
                "({x}, {y}) blends"
            );
            assert!(
                [0, 8, 16].iter().all(|&i| between(p, i)),
                "({x}, {y}) {p:08X} lies between the face and the rim"
            );
        }
    }

    #[test]
    fn a_blue_unchecked_radio_button_shades_its_well_grey_to_white() {
        let c = blue(|c, t, b| draw_radio(c, &t.paint(), b, false, Interaction::Idle));
        let sum = |x: i32, y: i32| {
            let p = c.get(2 + x, 2 + y);
            (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF)
        };
        assert!(sum(4, 3) < sum(6, 6), "grey at the top left");
        assert!(sum(6, 6) < sum(8, 9), "lighter toward the bottom right");
    }

    #[test]
    fn a_blue_checked_radio_button_has_a_dot() {
        let off = blue(|c, t, b| draw_radio(c, &t.paint(), b, false, Interaction::Idle));
        let on = blue(|c, t, b| draw_radio(c, &t.paint(), b, true, Interaction::Idle));
        let dot = faces().normal_checked.mark.unwrap();
        let green = |p: u32| (p >> 8 & 0xFF) > (p >> 16 & 0xFF) + 40;
        assert!(
            green(on.get(2 + 6, 2 + 6)),
            "the middle of the dot is green"
        );
        assert!(!green(off.get(2 + 6, 2 + 6)), "no dot unchecked");
        assert!(green(dot.argb()), "the dot's face is green");
        assert_eq!(on.get(2 + 6, 2), off.get(2 + 6, 2), "the rim stays");
        assert_eq!(
            on.get(2 + 2, 2 + 6),
            off.get(2 + 2, 2 + 6),
            "the well round the dot stays"
        );
    }

    #[test]
    fn a_blue_disabled_radio_button_greys_its_rim_and_dot() {
        let f = faces();
        let off = blue(|c, t, b| draw_radio_disabled(c, &t.paint(), b, false));
        assert_ne!(off.get(2 + 6, 2), FACE.argb(), "a grey top rim");
        pin(&off, 6, 6, 0xECE9D8, "the face through the clear well");
        pin(&off, 3, 3, 0xECE9D8, "the face through the clear well");
        let on = blue(|c, t, b| draw_radio_disabled(c, &t.paint(), b, true));
        pin(&on, 6, 6, rgb(f.disabled_checked.mark.unwrap()), "grey dot");
        pin(&on, 3, 3, 0xECE9D8, "the face through the clear well");
    }

    fn lit(check: Check, ptr: Interaction) -> Canvas {
        let checked = check == Check::Checked;
        let c = blue(|c, t, b| draw_radio(c, &t.paint(), b, checked, ptr));
        let state = match ptr {
            Interaction::Idle => State::Normal,
            Interaction::Hot => State::Hot,
            Interaction::Pressed => State::Pressed,
        };
        assert!(
            c == frame(check, state),
            "{check:?} {ptr:?} is the {state:?} frame"
        );
        c
    }

    #[test]
    fn a_blue_radio_button_under_the_pointer_has_an_orange_ring() {
        let orange = |p: u32| (p >> 16 & 0xFF) > (p & 0xFF) + 60;
        for check in [Check::Unchecked, Check::Checked] {
            let c = lit(check, Interaction::Hot);
            let rest = lit(check, Interaction::Idle);
            assert_eq!(
                c.get(2 + 6, 2),
                rest.get(2 + 6, 2),
                "{check:?}: the rim stays"
            );
            assert!(
                orange(c.get(2 + 4, 2 + 2)),
                "{check:?}: orange at the top left"
            );
            assert!(
                orange(c.get(2 + 8, 2 + 10)),
                "{check:?}: orange at the bottom right"
            );
            assert!(
                !orange(rest.get(2 + 8, 2 + 10)),
                "{check:?}: no orange at rest"
            );
        }
        let off = lit(Check::Unchecked, Interaction::Hot);
        assert!(!orange(off.get(2 + 6, 2 + 6)), "a pale core");
    }

    #[test]
    fn a_held_blue_radio_button_darkens_its_well() {
        let sum = |p: u32| (p >> 16 & 0xFF) + (p >> 8 & 0xFF) + (p & 0xFF);
        for check in [Check::Unchecked, Check::Checked] {
            let c = lit(check, Interaction::Pressed);
            let rest = lit(check, Interaction::Idle);
            assert_eq!(
                c.get(2 + 6, 2),
                rest.get(2 + 6, 2),
                "{check:?}: the rim stays"
            );
            for (x, y) in [(4, 3), (8, 9)] {
                assert!(
                    sum(c.get(2 + x, 2 + y)) < sum(rest.get(2 + x, 2 + y)),
                    "{check:?}: ({x}, {y}) darker held"
                );
            }
        }
    }

    #[test]
    fn a_blue_radio_button_on_a_scale_2_canvas_is_drawn_at_2() {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::with_scale(20, 20, 2);
        c.fill_rect(Rect::new(0, 0, 20, 20), FACE);
        draw_radio(
            &mut c,
            &theme.paint(),
            Rect::new(2, 2, 13, 13),
            true,
            Interaction::Idle,
        );
        let (w, _) = c.device_size();
        let bytes = c.to_argb_be();
        let device = |x: i32, y: i32| {
            let i = 4 * (y as usize * w as usize + x as usize);
            u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap())
        };
        let rim = 0xFF00_0000 | faces().normal.rim.argb();
        assert_eq!(device(2 * 2 + 1, 2 * (2 + 6)), rim, "a solid rim at 2");
        let edge = (0..26).map(|x| device(2 * 2 + x, 2 * 2 + 3));
        let face = FACE.argb();
        assert!(
            edge.clone().any(|p| p != face && p != rim),
            "the edge blends at 2"
        );
    }

    #[test]
    fn a_blue_radio_button_at_paint_scale_2_doubles_every_pixel() {
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            let b = Rect::new(2, 2, 13, 13).scaled(p.s);
            crate::look::widgets::draw_radio(c, p, b, true, Interaction::Pressed);
            let greyed = Rect::new(20, 2, 13, 13).scaled(p.s);
            crate::look::widgets::draw_radio_disabled(c, p, greyed, false);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(36, 17, draw),
            "a checked Pressed and a greyed radio at s = 2 are their s = 1 pixels doubled"
        );
    }
}
