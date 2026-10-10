//! The push button through [`Part::PushButton`]: the part's frame for the
//! button's state, the label centred in the part's content rect, and a
//! dotted focus ring.
//!
//! The frame goes Disabled, Pressed, Hot, Default, Focused, Normal, the
//! first that holds: the pointer outranks the default and focus rings
//! only while it is on the button. Under a style with
//! [`Style::default_needs_active`] the default button draws no Default
//! frame while its window is inactive.

use crate::classic::widgets::{RingInk, draw_focus_ring_screen, draw_label};
use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::text::{Align, TextRenderer};

const FOCUS_INSET: i32 = 3;

#[expect(clippy::too_many_arguments)]
pub fn draw_button(
    st: &Style,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    label: &str,
    default: bool,
    ptr: Interaction,
    enabled: bool,
    focus: bool,
    px: u32,
) -> bool {
    if st.part(Part::PushButton).is_none() {
        return false;
    }
    let state = if !enabled {
        State::Disabled
    } else if ptr == Interaction::Pressed {
        State::Pressed
    } else if ptr == Interaction::Hot {
        State::Hot
    } else if default && (p.active || !st.default_needs_active) {
        State::Default
    } else if focus {
        State::Focused
    } else {
        State::Normal
    };
    st.draw(c, r, Part::PushButton, state, p.s);
    let ink = if enabled {
        p.cs.button_text
    } else {
        st.text_color(Part::PushButton, State::Disabled)
            .unwrap_or(p.cs.gray_text)
    };
    let content = st.content_rect(Part::PushButton, r, p.s);
    draw_label(c, text, content, label, false, ink, Align::Center, px, p.s);
    let ring = r.inset(FOCUS_INSET * p.s);
    if focus && !ring.is_empty() {
        // The origin puts the button's own top-left on an even dot.
        let s = p.s.max(1);
        let origin = (-r.x.div_euclid(s), -r.y.div_euclid(s));
        let ink = RingInk::Xor(p.cs.button_face);
        draw_focus_ring_screen(c, ring, p.s, ink, origin);
    }
    true
}

pub fn draw_button_face(
    st: &Style,
    c: &mut Canvas,
    r: Rect,
    enabled: bool,
    ptr: Interaction,
    s: i32,
) -> bool {
    if st.part(Part::PushButton).is_none() {
        return false;
    }
    st.draw(c, r, Part::PushButton, super::state(enabled, ptr), s);
    true
}

#[cfg(test)]
mod tests {
    use crate::blue::{self, Variant};
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::interfaces::theme_for;
    use crate::look::styled::fixture::{one_part_style, solid_frames};
    use crate::look::widgets::draw_button;
    use crate::look::{Look, Paint};
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::{Layout, Part, PartImage, State};
    use crate::text::TextRenderer;
    use crate::theme::{Color, ColorScheme, FontConfig, Theme};
    use std::sync::Arc;

    #[test]
    fn a_style_with_a_push_button_part_paints_its_default_frame() {
        let (normal, default) = (Color::rgb(10, 0, 0), Color::rgb(20, 0, 0));
        let img = PartImage::new(
            solid_frames(&[normal, default]),
            &[State::Normal, State::Default],
            Layout::Vertical,
        );
        let look = Look::Styled(Arc::new(one_part_style(Part::PushButton, img)));
        let cs = ColorScheme::default();
        let p = Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
        };
        let text = TextRenderer::load(&FontConfig::default());
        let mut c = Canvas::new(80, 30);
        draw_button(
            &mut c,
            &text,
            &p,
            Rect::new(0, 0, 75, 23),
            "",
            true,
            Interaction::Idle,
            true,
            false,
            11,
        );
        assert_eq!(
            c.get(1, 1),
            default.argb(),
            "the default button paints the part's Default frame"
        );
    }

    #[test]
    fn a_classic_look_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        let text = TextRenderer::load(&theme.fonts);
        let r = Rect::new(0, 0, 75, 23);
        let (mut a, mut b) = (Canvas::new(80, 30), Canvas::new(80, 30));
        draw_button(
            &mut a,
            &text,
            &theme.paint(),
            r,
            "&OK",
            true,
            Interaction::Idle,
            true,
            true,
            theme.fonts.size_px,
        );
        classic::widgets::draw_button(
            &mut b,
            &text,
            &theme.paint(),
            r,
            "&OK",
            true,
            false,
            true,
            true,
            theme.fonts.size_px,
        );
        assert!(a == b, "a Classic push button paints the Classic painter");
    }

    type Flags = (bool, Interaction, bool, bool);

    const NORMAL: Flags = (false, Interaction::Idle, true, false);
    const DEFAULT: Flags = (true, Interaction::Idle, true, false);
    const HOT: Flags = (false, Interaction::Hot, true, false);
    const PRESSED: Flags = (false, Interaction::Pressed, true, false);
    const DISABLED: Flags = (false, Interaction::Idle, false, false);
    const FOCUSED: Flags = (false, Interaction::Idle, true, true);

    fn blue() -> Theme {
        theme_for(Interface::Blue)
    }

    fn filled(mut c: Canvas, bg: Color) -> Canvas {
        c.fill_rect(Rect::new(0, 0, c.width as i32, c.height as i32), bg);
        c
    }

    fn dialog(w: u32, h: u32) -> Canvas {
        filled(Canvas::new(w, h), blue().colors.button_face)
    }

    fn button_on(mut c: Canvas, size: (i32, i32), label: &str, flags: Flags) -> Canvas {
        let theme = blue();
        let text = TextRenderer::load(&theme.fonts);
        let (default, ptr, enabled, focus) = flags;
        draw_button(
            &mut c,
            &text,
            &theme.paint(),
            Rect::new(0, 0, size.0, size.1),
            label,
            default,
            ptr,
            enabled,
            focus,
            theme.fonts.size_px,
        );
        c
    }

    fn button(label: &str, flags: Flags) -> Canvas {
        button_on(dialog(75, 23), (75, 23), label, flags)
    }

    #[test]
    fn blue_has_a_push_button_part_of_six_75_by_23_frames() {
        let st = blue::style();
        let p = st.part(Part::PushButton).expect("Blue has a push button");
        assert_eq!(
            p.states,
            [
                State::Normal,
                State::Hot,
                State::Pressed,
                State::Disabled,
                State::Default,
                State::Focused
            ]
        );
        assert_eq!((p.atlas.width, p.atlas.height), (75, 6 * 23));
    }

    fn variant_button(v: Variant, flags: Flags) -> Canvas {
        let theme = interfaces::blue_theme(v);
        let text = TextRenderer::load(&theme.fonts);
        let (default, ptr, enabled, focus) = flags;
        let mut c = filled(Canvas::new(75, 23), theme.colors.button_face);
        draw_button(
            &mut c,
            &text,
            &theme.paint(),
            Rect::new(0, 0, 75, 23),
            "",
            default,
            ptr,
            enabled,
            focus,
            theme.fonts.size_px,
        );
        c
    }

    fn light(c: &Canvas, x: i32, y: i32) -> u32 {
        let p = c.get(x, y);
        ((p >> 16) & 0xFF) + ((p >> 8) & 0xFF) + (p & 0xFF)
    }

    fn check_variant(v: Variant) {
        let rest = variant_button(v, NORMAL);
        assert!(light(&rest, 10, 1) < light(&rest, 10, 10), "{v:?} border");
        assert!(
            light(&rest, 10, 3) >= light(&rest, 10, 18),
            "{v:?} shades down"
        );
        let held = variant_button(v, PRESSED);
        assert!(
            light(&held, 10, 3) < light(&held, 10, 18),
            "{v:?} held shades up"
        );
        for (flags, what) in [(HOT, "hot"), (DEFAULT, "default")] {
            let c = variant_button(v, flags);
            for (x, y) in [(10, 2), (3, 10), (71, 10), (10, 20)] {
                assert_ne!(
                    c.get(x, y),
                    rest.get(x, y),
                    "{v:?} {what} ring at ({x}, {y})"
                );
            }
        }
        let greyed = variant_button(v, DISABLED);
        assert!(
            light(&greyed, 10, 1) > light(&rest, 10, 1),
            "{v:?} greyed border"
        );
        assert_eq!(
            greyed.get(10, 5),
            greyed.get(10, 15),
            "{v:?} greyed face is flat"
        );
        let focused = variant_button(v, FOCUSED);
        let default = variant_button(v, DEFAULT);
        for (x, y) in [(3, 2), (2, 3), (2, 19), (4, 20), (72, 5)] {
            assert_eq!(
                focused.get(x, y),
                default.get(x, y),
                "{v:?} focused ring at ({x}, {y})"
            );
        }
    }

    #[test]
    fn every_blue_scheme_draws_each_button_state() {
        for v in Variant::ALL {
            check_variant(v);
        }
    }

    #[test]
    fn hot_rings_are_warm_and_default_rings_take_the_schemes_hue() {
        let ch = |p: u32| ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
        for v in Variant::ALL {
            let (r, _, b) = ch(variant_button(v, HOT).get(10, 20));
            assert!(r > b + 0x40, "{v:?} hot ring is warm");
        }
        for v in [
            Variant::Blue,
            Variant::Silver,
            Variant::Sapphire,
            Variant::Obsidian,
        ] {
            let (r, _, b) = ch(variant_button(v, DEFAULT).get(10, 20));
            assert!(b > r + 0x40, "{v:?} default ring is blue");
        }
        let (r, g, b) = ch(variant_button(Variant::Olive, DEFAULT).get(10, 20));
        assert!(g > b && g + 0x20 > r, "olive green's default ring is green");
    }

    #[test]
    fn ember_and_obsidian_share_slates_and_sapphires_buttons() {
        let atlas = |v| {
            blue::style_for(v)
                .part(Part::PushButton)
                .expect("the part is built")
                .atlas
                .clone()
        };
        assert!(atlas(Variant::Ember) == atlas(Variant::Slate));
        assert!(atlas(Variant::Obsidian) == atlas(Variant::Sapphire));
    }

    #[test]
    fn a_blue_button_at_rest_has_a_dark_blue_border_on_a_white_to_grey_face() {
        let c = button("", NORMAL);
        let rim = Variant::Blue.scheme().push_button.normal.rim.argb();
        for (x, y) in [(10, 1), (1, 10), (73, 10), (10, 21)] {
            assert_eq!(c.get(x, y), rim, "the border at ({x}, {y})");
        }
        assert!(light(&c, 10, 2) > light(&c, 10, 19), "white to grey");
        let face = blue().colors.button_face.argb();
        assert_eq!(c.get(0, 0), face, "the corner shows the dialog");
        assert_ne!(c.get(40, 0), face, "a faint edge above the border");
        assert!(light(&c, 40, 22) > light(&c, 40, 0), "a lit edge below it");
    }

    #[test]
    fn the_pointer_outranks_the_default_and_focus_rings() {
        let hot = button("", HOT);
        let default_hot = button("", (true, Interaction::Hot, true, false));
        assert!(
            default_hot == hot,
            "a hot default button draws the Hot frame"
        );
        let focused_hot = button("", (false, Interaction::Hot, true, true));
        for (x, y) in [(7, 2), (3, 20)] {
            assert_eq!(
                focused_hot.get(x, y),
                hot.get(x, y),
                "the hot ring at ({x}, {y})"
            );
        }
    }

    #[test]
    fn a_bare_blue_button_face_takes_the_pointers_frame() {
        let r = Rect::new(0, 0, 75, 21);
        for (ptr, state) in [
            (Interaction::Idle, State::Normal),
            (Interaction::Hot, State::Hot),
            (Interaction::Pressed, State::Pressed),
        ] {
            let (mut a, mut b) = (dialog(75, 21), dialog(75, 21));
            crate::look::widgets::draw_button_face(&mut a, &blue().paint(), r, true, ptr);
            blue::style().draw(&mut b, r, Part::PushButton, state, 1);
            assert!(a == b, "{ptr:?} draws the {state:?} frame");
        }
    }

    #[test]
    fn a_greyed_button_under_the_pointer_stays_greyed() {
        for ptr in [Interaction::Hot, Interaction::Pressed] {
            let c = button("", (false, ptr, false, false));
            assert!(c == button("", DISABLED), "{ptr:?} on a greyed button");
        }
    }

    #[test]
    fn a_focused_button_has_the_blue_ring_and_a_dotted_ring_3_px_in() {
        let c = button("", FOCUSED);
        let ring = button("", DEFAULT);
        let face = blue().colors.button_face.argb() & 0xFF_FFFF;
        for (x, y) in [(2, 2), (3, 3), (5, 3), (11, 19), (4, 10)] {
            assert_eq!(c.get(x, y), ring.get(x, y), "no dot at ({x}, {y})");
        }
        for (x, y) in [(4, 3), (3, 4), (71, 4), (10, 19), (3, 10)] {
            assert_eq!(c.get(x, y), ring.get(x, y) ^ face, "a dot at ({x}, {y})");
        }
    }

    #[test]
    fn the_default_button_with_the_focus_draws_the_same_ring_as_a_focused_one() {
        assert!(
            button("OK", (true, Interaction::Idle, true, true)) == button("OK", FOCUSED),
            "default and focused draw the same pixels"
        );
    }

    fn inked(c: &Canvas, ink: Color) -> Vec<(i32, i32)> {
        (0..c.height as i32)
            .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink.argb())
            .collect()
    }

    #[test]
    fn a_held_button_keeps_its_label_in_place() {
        let black = blue().colors.button_text;
        let at_rest = inked(&button("Cancel", NORMAL), black);
        assert!(!at_rest.is_empty(), "the label inks button_text");
        assert_eq!(
            inked(&button("Cancel", PRESSED), black),
            at_rest,
            "the held label inks the same pixels"
        );
    }

    #[test]
    fn a_greyed_label_is_flat() {
        let cs = blue().colors;
        let c = button("Apply", DISABLED);
        assert!(
            !inked(&c, crate::blue::Variant::Blue.scheme().inks.greyed_label).is_empty(),
            "the greyed label inks the greyed label ink"
        );
        assert!(
            inked(&c, cs.gray_text).is_empty(),
            "the greyed label inks no gray_text"
        );
        assert!(
            inked(&c, cs.button_hilight).is_empty(),
            "the greyed label has no white etch"
        );
    }

    fn ink_rows(c: &Canvas, ink: Color) -> Vec<i32> {
        let mut rows: Vec<i32> = inked(c, ink).into_iter().map(|(_, y)| y).collect();
        rows.dedup();
        rows
    }

    #[test]
    fn the_label_sits_a_pixel_below_the_middle() {
        let theme = blue();
        if !TextRenderer::load(&theme.fonts).has_face("Tahoma") {
            return;
        }
        let black = theme.colors.button_text;
        let c = button("&Cancel", NORMAL);
        let want: Vec<i32> = (7..=15).chain([17]).collect();
        assert_eq!(
            ink_rows(&c, black),
            want,
            "the glyphs ink rows 7 to 15 (baseline 15) and the underline row 17"
        );
        let underline: Vec<i32> = inked(&c, black)
            .into_iter()
            .filter(|&(_, y)| y == 17)
            .map(|(x, _)| x)
            .collect();
        assert_eq!(
            underline,
            (21..=27).collect::<Vec<i32>>(),
            "the C's underline runs x 21 to 27"
        );
    }

    type Shape = Vec<(i32, i32)>;

    fn label_on_button_and_free(label: &str) -> (Shape, Shape) {
        let theme = blue();
        let text = TextRenderer::load(&theme.fonts);
        let black = theme.colors.button_text;
        let mut free = Canvas::new(200, 23);
        classic::widgets::draw_label(
            &mut free,
            &text,
            Rect::new(10, 1, 180, 22),
            label,
            false,
            black,
            crate::text::Align::Left,
            theme.fonts.size_px,
            1,
        );
        let shape = |c: &Canvas| {
            let ink = inked(c, black);
            let x0 = ink.iter().map(|&(x, _)| x).min().unwrap_or(0);
            let mut v: Vec<(i32, i32)> = ink.into_iter().map(|(x, y)| (x - x0, y)).collect();
            v.sort_unstable();
            v
        };
        (shape(&button(label, NORMAL)), shape(&free))
    }

    #[test]
    fn a_75_px_button_shows_a_67_px_label_whole_and_clips_a_wider_one() {
        let theme = blue();
        let text = TextRenderer::load(&theme.fonts);
        if !text.has_face("Tahoma") {
            return;
        }
        let px = theme.fonts.size_px;
        let width = |l: &str| l.chars().map(|ch| text.advance(ch, false, px)).sum::<i32>();
        assert_eq!(width("Clock Settings"), 67);
        let (on, free) = label_on_button_and_free("Clock Settings");
        assert_eq!(on, free, "a 67 px label draws whole");
        assert_eq!(width("Apply Settings"), 69);
        let (on, free) = label_on_button_and_free("Apply Settings");
        assert!(
            on.len() < free.len(),
            "a 69 px label loses its outer columns"
        );
    }

    #[test]
    fn the_soft_edge_shows_what_is_behind_the_button() {
        let navy = Color::rgb(0x00, 0x33, 0x99);
        let on_navy = button_on(filled(Canvas::new(75, 23), navy), (75, 23), "", NORMAL);
        let on_face = button("", NORMAL);
        for (x, y) in [(40, 0), (0, 10), (40, 22), (1, 1)] {
            assert_ne!(
                on_navy.get(x, y),
                on_face.get(x, y),
                "the edge at ({x}, {y}) is translucent"
            );
            assert_ne!(
                on_navy.get(x, y),
                navy.argb(),
                "the edge at ({x}, {y}) shows"
            );
        }
        for (x, y) in [(10, 1), (1, 10), (10, 10), (2, 2)] {
            assert_eq!(
                on_navy.get(x, y),
                on_face.get(x, y),
                "the border and face at ({x}, {y}) are opaque"
            );
        }
        let greyed = button_on(filled(Canvas::new(75, 23), navy), (75, 23), "", DISABLED);
        assert_eq!(
            greyed.get(0, 0),
            navy.argb(),
            "a greyed button's outer corner is clear"
        );
    }

    #[test]
    fn a_larger_blue_button_keeps_its_corners() {
        let big = button_on(dialog(100, 30), (100, 30), "", NORMAL);
        let small = button("", NORMAL);
        for (x, y) in [(0, 0), (2, 1), (1, 2), (3, 3)] {
            assert_eq!(big.get(x, y), small.get(x, y), "top left ({x}, {y})");
            assert_eq!(
                big.get(99 - x, 29 - y),
                small.get(74 - x, 22 - y),
                "bottom right ({x}, {y})"
            );
        }
        assert_eq!(big.get(50, 1), small.get(37, 1), "the top border");
        assert_eq!(big.get(50, 28), small.get(37, 21), "the bottom border");
    }

    #[test]
    fn a_blue_button_on_a_scale_2_canvas_paints_its_scale_1_pixels() {
        let theme = blue();
        let text = TextRenderer::load(&theme.fonts);
        let mut c = filled(Canvas::with_scale(75, 23, 2), theme.colors.button_face);
        draw_button(
            &mut c,
            &text,
            &theme.paint(),
            Rect::new(0, 0, 75, 23),
            "",
            false,
            Interaction::Idle,
            true,
            true,
            theme.fonts.size_px,
        );
        let flat = button("", FOCUSED);
        for (x, y) in [(0, 0), (2, 1), (10, 1), (3, 4), (4, 3), (72, 20)] {
            assert_eq!(c.get(x, y), flat.get(x, y), "({x}, {y}) as at scale 1");
        }
        let (dw, _) = c.device_size();
        assert_eq!(dw, 150, "the device canvas is twice as wide");
    }

    #[test]
    fn a_blue_button_at_scale_2_is_drawn_again_not_doubled() {
        let theme = blue().scaled(2);
        let text = TextRenderer::load(&theme.fonts);
        let mut two = dialog(150, 46);
        draw_button(
            &mut two,
            &text,
            &theme.paint(),
            Rect::new(0, 0, 150, 46),
            "",
            false,
            Interaction::Idle,
            true,
            false,
            theme.fonts.size_px,
        );
        let one = button("", NORMAL);
        let doubled = (0..46).all(|y| (0..150).all(|x| two.get(x, y) == one.get(x / 2, y / 2)));
        assert!(
            !doubled,
            "the scale-2 button is not the scale-1 one doubled"
        );
        // A 2x2 block on a corner's curve holds more than one colour.
        let mixed = (0..4).any(|by| {
            (0..4).any(|bx| {
                let (x, y) = (2 * bx, 2 * by);
                let p = two.get(x, y);
                [(x + 1, y), (x, y + 1), (x + 1, y + 1)]
                    .iter()
                    .any(|&(a, b)| two.get(a, b) != p)
            })
        });
        assert!(mixed, "the corner is drawn at 2");
    }

    #[test]
    fn a_label_moves_by_its_content_margins_times_the_paint_scale() {
        let cs = ColorScheme::default();
        let text = TextRenderer::load(&FontConfig::default().scaled(2));
        let ink_at = |content: crate::raster::Insets| {
            let img = PartImage::new(
                solid_frames(&[Color::rgb(10, 0, 0)]),
                &[State::Normal],
                Layout::Vertical,
            )
            .content(content);
            let look = crate::look::styled::fixture::one_part_look_with(Part::PushButton, img);
            let p = Paint {
                cs: &cs,
                look: &look,
                s: 2,
                back: crate::look::Backdrop::Face,
                active: true,
            };
            let mut c = Canvas::new(150, 46);
            let r = Rect::new(0, 0, 150, 46);
            draw_button(
                &mut c,
                &text,
                &p,
                r,
                "Ab",
                false,
                Interaction::Idle,
                true,
                false,
                22,
            );
            let ink = inked(&c, cs.button_text);
            let x = ink.iter().map(|&(x, _)| x).min().expect("the label inks");
            let y = ink.iter().map(|&(_, y)| y).min().expect("the label inks");
            (x, y)
        };
        let (x0, y0) = ink_at(crate::raster::Insets::ZERO);
        let (x1, y1) = ink_at(crate::raster::Insets::new(12, 6, 4, 2));
        assert_eq!(
            (x1 - x0, y1 - y0),
            (8, 4),
            "margins 12, 6, 4 and 2 move the centred label 4 px right and 2 px down, 8 and 4 device px at s = 2"
        );
    }
}
