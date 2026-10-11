//! The tray icons the compositor draws itself (StickyKeys, MouseKeys, layout name, notification
//! bell, privacy indicators), drawn from strokes, text or the PNGs in [`icons`] so they need no
//! icon theme.

use crate::art::icons::{self, Icon};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, ColorScheme};

pub fn draw_layout(
    text: &TextRenderer,
    cs: &ColorScheme,
    label: &str,
    px: i32,
    size_px: u32,
) -> Canvas {
    let px = px.max(8);
    let mut c = Canvas::new(px as u32, px as u32);
    let r = c.bounds();
    c.fill_rect(r, cs.hilight);
    text.draw(
        &mut c,
        r,
        label,
        false,
        cs.hilight_text,
        Align::Center,
        size_px,
    );
    c
}

pub fn draw_record(px: i32) -> Canvas {
    let px = px.max(8);
    let mut c = Canvas::new(px as u32, px as u32);
    let fill = Color::rgb(220, 24, 24);
    let rim = Color::rgb(120, 0, 0);
    let cx = (px - 1) as f32 / 2.0;
    let cy = cx;
    let r = px as f32 / 2.0 - 1.5;
    for y in 0..px {
        for x in 0..px {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            if d <= r - 1.0 {
                c.put(x, y, fill);
            } else if d <= r {
                c.put(x, y, rim);
            }
        }
    }
    c
}

pub fn draw_microphone(px: i32) -> Canvas {
    let px = px.max(8);
    let mut c = Canvas::new(px as u32, px as u32);
    let ink = Color::rgb(0, 0, 0);
    let s = px as f32 / 16.0;
    let u = |v: i32| (v as f32 * s).round() as i32;
    let (cx, cy, cw, ch) = (u(5), u(1), u(6), u(9));
    c.fill_rect(Rect::new(cx + 1, cy, cw - 2, ch), ink);
    c.fill_rect(Rect::new(cx, cy + 1, cw, ch - 2), ink);
    let (ux, uy, uw, uh) = (u(3), u(6), u(10), u(6));
    c.fill_rect(Rect::new(ux, uy, 1, uh - 1), ink);
    c.fill_rect(Rect::new(ux + uw - 1, uy, 1, uh - 1), ink);
    c.fill_rect(Rect::new(ux + 1, uy + uh - 1, uw - 2, 1), ink);
    c.fill_rect(Rect::new(u(8) - 1, uy + uh, 2, u(2)), ink);
    c.fill_rect(Rect::new(u(5), uy + uh + u(2), u(6), 1), ink);
    c
}

fn draw_icon(px: i32, icon: &'static Icon, ink: impl Fn(Color) -> Option<Color>) -> Canvas {
    let px = px.max(16);
    let k = px / 16;
    let off = (px - 16 * k) / 2;
    let mut c = Canvas::new(px as u32, px as u32);
    icon.paint_inked(&mut c, off, off, k, ink);
    c
}

pub fn draw_camera(px: i32) -> Canvas {
    draw_icon(px, &icons::CAMERA, Some)
}

pub fn draw_location(px: i32) -> Canvas {
    draw_icon(px, &icons::LOCATION, Some)
}

/// The StickyKeys indicator in a `px` square; keys in `held` (wlr modifier bits: Shift 1, Ctrl 4,
/// Alt 8, Super 64) are filled black.
pub fn draw_sticky(px: i32, held: u32) -> Canvas {
    const SHIFT: u32 = 1;
    const CTRL: u32 = 1 << 2;
    const ALT: u32 = 1 << 3;
    const SUPER: u32 = 1 << 6;
    let ink = Color::rgb(0, 0, 0);
    draw_icon(px, &icons::STICKY_KEYS, |c| {
        let bit = match c.r {
            0 => return Some(ink),
            64 => SHIFT,
            96 => CTRL,
            128 => SUPER,
            _ => ALT,
        };
        (held & bit != 0).then_some(ink)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

/// The MouseKeys indicator in a `px` square: the button the keys click (`pick`) is filled grey,
/// black while held.
pub fn draw_mouse_keys(px: i32, pick: MouseButton, held: bool) -> Canvas {
    let ink = Color::rgb(0, 0, 0);
    let fill = if held { ink } else { Color::rgb(128, 128, 128) };
    let cell = match pick {
        MouseButton::Left => 64,
        MouseButton::Middle => 96,
        MouseButton::Right => 128,
    };
    draw_icon(px, &icons::MOUSE_KEYS, |c| match c.r {
        0 => Some(ink),
        v if v == cell => Some(fill),
        _ => None,
    })
}

const BADGE: Color = Color::rgb(220, 24, 24);
const BADGE_INK: Color = Color::rgb(255, 255, 255);

const DIGITS: [[u8; 5]; 11] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b000, 0b010, 0b111, 0b010, 0b000],
];
const PLUS: usize = 10;

/// The notification bell in a `px` square. `unseen` above zero puts a red badge with the count (1
/// to 9, then "9+") in the bottom-right corner; `dnd` strikes the bell with a red slash.
pub fn draw_bell(px: i32, unseen: usize, dnd: bool) -> Canvas {
    let px = px.max(16);
    let k = px / 16;
    let off = (px - 16 * k) / 2;
    let mut c = draw_icon(px, &icons::BELL, Some);
    let dot = |c: &mut Canvas, x: i32, y: i32, col: Color| {
        c.fill_rect(Rect::new(off + x * k, off + y * k, k, k), col);
    };
    if dnd {
        for i in 1..15 {
            dot(&mut c, i, i, BADGE);
            dot(&mut c, i + 1, i, BADGE);
        }
    }
    if unseen > 0 {
        let glyphs: &[usize] = if unseen > 9 { &[9, PLUS] } else { &[unseen] };
        let w = 1 + 4 * glyphs.len() as i32;
        let (bx, by) = (16 - w, 16 - 7);
        for y in 0..7 {
            for x in 0..w {
                dot(&mut c, bx + x, by + y, BADGE);
            }
        }
        for (i, g) in glyphs.iter().enumerate() {
            for (y, bits) in DIGITS[*g].iter().enumerate() {
                for x in 0..3 {
                    if bits & (0b100 >> x) != 0 {
                        let gx = bx + 1 + 4 * i as i32 + x;
                        dot(&mut c, gx, by + 1 + y as i32, BADGE_INK);
                    }
                }
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    const BELL_OUTLINE: Color = Color::rgb(96, 64, 0);
    const BELL_FILL: Color = Color::rgb(255, 204, 0);
    const BELL_HILIGHT: Color = Color::rgb(255, 244, 192);

    #[test]
    fn the_layout_square_carries_its_short_name() {
        if !crate::text::test_tahoma() {
            return;
        }
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let cs = ColorScheme::default();
        let c = draw_layout(&text, &cs, "EN", 16, 11);
        const EN: [&str; 16] = [
            "................",
            "................",
            "................",
            "................",
            ".#####.##...#...",
            ".#.....##...#...",
            ".#.....#.#..#...",
            ".####..#.#..#...",
            ".#.....#..#.#...",
            ".#.....#..#.#...",
            ".#.....#...##...",
            ".#####.#...##...",
            "................",
            "................",
            "................",
            "................",
        ];
        for (y, row) in EN.iter().enumerate() {
            let got: String = (0..16)
                .map(|x| {
                    let p = c.get(x, y as i32);
                    if p == cs.hilight_text.argb() {
                        '#'
                    } else {
                        assert_eq!(p, cs.hilight.argb(), "({x}, {y}) is text or the square");
                        '.'
                    }
                })
                .collect();
            assert_eq!(&got, row, "row {y}");
        }
    }

    #[test]
    fn a_scale_2_layout_square_fills_its_32_px() {
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let cs = ColorScheme::default();
        let c = draw_layout(&text, &cs, "HU", 32, 22);
        assert_eq!((c.width, c.height), (32, 32));
        assert_eq!(c.get(0, 0), cs.hilight.argb());
        assert_eq!(c.get(31, 31), cs.hilight.argb());
    }

    fn sticky_rows(c: &Canvas) -> Vec<String> {
        (0..16)
            .map(|y| {
                (0..16)
                    .map(|x| match c.get(x, y) {
                        0xff00_0000 => '#',
                        p => {
                            assert_eq!(p >> 24, 0, "({x}, {y}) is ink or clear");
                            '.'
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_sticky_keys_are_drawn_empty_and_fill_while_held() {
        let idle = sticky_rows(&draw_sticky(16, 0));
        assert_eq!(idle[3], "###########.....", "Shift's top edge, 11 wide");
        assert_eq!(idle[5], "#.........#.....");
        assert_eq!(idle[8], "###########.....");
        assert_eq!(idle[9], "................");
        assert_eq!(idle[10], "#####..##..#####", "Super's rounded top");
        assert_eq!(idle[12], "#...#.#..#.#...#");
        assert_eq!(idle[15], "#####.####.#####");
        assert!(idle[..3].iter().all(|r| r == "................"));
        let shift = sticky_rows(&draw_sticky(16, 1));
        assert_eq!(shift[5], "###########.....");
        assert_eq!(shift[12], "#...#.#..#.#...#");
        let ctrl = sticky_rows(&draw_sticky(16, 1 << 2));
        assert_eq!(ctrl[12], "#####.#..#.#...#");
        let sup = sticky_rows(&draw_sticky(16, 1 << 6));
        assert_eq!(sup[12], "#...#.####.#...#");
        let alt = sticky_rows(&draw_sticky(16, 1 << 3));
        assert_eq!(alt[12], "#...#.#..#.#####");
    }

    #[test]
    fn the_mouse_keys_mouse_greys_the_picked_button_and_blackens_it_held() {
        let row = |c: &Canvas, y: i32| -> String {
            (0..16)
                .map(|x| match c.get(x, y) {
                    0xff00_0000 => '#',
                    0xff80_8080 => '+',
                    p => {
                        assert_eq!(p >> 24, 0, "({x}, {y}) is ink, grey or clear");
                        '.'
                    }
                })
                .collect()
        };
        let left = draw_mouse_keys(16, MouseButton::Left, false);
        assert_eq!(row(&left, 1), "...##########...");
        assert_eq!(row(&left, 4), "..#+++#..#...#..");
        assert_eq!(row(&left, 7), "..############..");
        assert_eq!(row(&left, 10), "..#..........#..");
        assert_eq!(row(&left, 14), "....########....");
        let middle = draw_mouse_keys(16, MouseButton::Middle, false);
        assert_eq!(row(&middle, 4), "..#...#++#...#..");
        let right = draw_mouse_keys(16, MouseButton::Right, true);
        assert_eq!(row(&right, 4), "..#...#..#####..");
        let two = draw_mouse_keys(32, MouseButton::Left, false);
        assert_eq!(two.get(7, 9), left.get(3, 4), "scale 2 doubles every cell");
    }

    #[test]
    fn a_scale_2_sticky_indicator_doubles_every_cell() {
        let one = draw_sticky(16, 1 << 3);
        let two = draw_sticky(32, 1 << 3);
        for (x, y) in [(0, 3), (5, 5), (12, 12), (7, 10), (15, 15)] {
            assert_eq!(two.get(2 * x + 1, 2 * y + 1), one.get(x, y), "({x}, {y})");
        }
    }

    #[test]
    fn bell_is_brass_inside_a_dark_outline() {
        let c = draw_bell(16, 0, false);
        assert_eq!(c.get(7, 1), BELL_OUTLINE.argb(), "the knob");
        assert_eq!(c.get(8, 7), BELL_FILL.argb());
        assert_eq!(c.get(5, 6), BELL_HILIGHT.argb());
        assert_eq!(c.get(7, 12), BELL_OUTLINE.argb(), "the rim");
        assert_eq!(c.get(7, 14), BELL_OUTLINE.argb(), "the clapper");
        assert_eq!(c.get(0, 0) >> 24, 0);
        assert_eq!(c.get(15, 15) >> 24, 0, "no badge at zero unseen");
    }

    #[test]
    fn the_badge_counts_to_nine_then_shows_nine_plus() {
        let c = draw_bell(16, 1, false);
        assert_eq!(c.get(11, 9), BADGE.argb());
        assert_eq!(c.get(15, 15), BADGE.argb());
        assert_eq!(
            c.get(10, 9),
            BELL_FILL.argb(),
            "left of the badge is the bell"
        );
        for y in 10..15 {
            assert_eq!(c.get(13, y), BADGE_INK.argb(), "row {y}");
        }
        assert_eq!(c.get(12, 10), BADGE.argb());
        let c = draw_bell(16, 12, false);
        assert_eq!(c.get(7, 9), BADGE.argb());
        assert_eq!(c.get(6, 9), BELL_FILL.argb());
        assert_eq!(c.get(12, 12), BADGE_INK.argb());
        assert_eq!(c.get(14, 12), BADGE_INK.argb());
        assert_eq!(c.get(12, 10), BADGE.argb(), "the plus has no top row");
    }

    #[test]
    fn do_not_disturb_strikes_the_bell_through() {
        let c = draw_bell(16, 0, true);
        assert_eq!(c.get(1, 1), BADGE.argb());
        assert_eq!(c.get(8, 7), BADGE.argb());
        assert_eq!(c.get(14, 14), BADGE.argb());
        assert_eq!(c.get(10, 7), BELL_FILL.argb());
    }

    #[test]
    fn a_scale_2_bell_doubles_every_cell() {
        let one = draw_bell(16, 3, true);
        let two = draw_bell(32, 3, true);
        for (x, y) in [(7, 1), (8, 7), (5, 6), (13, 12), (3, 3)] {
            assert_eq!(two.get(2 * x + 1, 2 * y + 1), one.get(x, y), "({x}, {y})");
        }
    }

    #[test]
    fn record_dot_is_red_in_the_middle_and_clear_at_the_corners() {
        let c = draw_record(16);
        assert_eq!(c.get(8, 8), Color::rgb(220, 24, 24).argb());
        assert_eq!(c.get(0, 0) >> 24, 0);
        assert_eq!(c.get(15, 15) >> 24, 0);
        assert_eq!(c.get(7, 2), Color::rgb(120, 0, 0).argb());
        assert_eq!(c.get(7, 1) >> 24, 0);
    }

    #[test]
    fn microphone_has_a_capsule_a_cradle_and_a_foot() {
        let c = draw_microphone(16);
        let ink = Color::rgb(0, 0, 0).argb();
        assert_eq!(c.get(8, 5), ink, "capsule body");
        assert_eq!(c.get(3, 8), ink, "cradle left arm");
        assert_eq!(c.get(12, 8), ink, "cradle right arm");
        assert_eq!(c.get(8, 14), ink, "foot");
        assert_eq!(c.get(1, 8) >> 24, 0, "outside the cradle");
        assert_eq!(c.get(0, 0) >> 24, 0);
    }

    fn ink_rows(c: &Canvas, ink: Color) -> Vec<String> {
        (0..16)
            .map(|y| {
                (0..16)
                    .map(|x| match c.get(x, y) {
                        v if v == ink.argb() => '#',
                        v if v >> 24 == 0 => '.',
                        _ => '?',
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_camera_is_outlined_with_a_lens_and_a_red_light() {
        let c = draw_camera(16);
        let rows = ink_rows(&c, Color::rgb(32, 32, 32));
        assert_eq!(rows[1], "....########....");
        assert_eq!(rows[4], "..#???####???#..");
        assert_eq!(rows[13], ".......##.......", "the stand");
        assert_eq!(rows[14], "....########....", "the foot");
        assert_eq!(c.get(11, 3), Color::rgb(220, 24, 24).argb(), "the light");
        assert_eq!(c.get(7, 7), Color::rgb(40, 60, 120).argb(), "the lens");
        assert_eq!(c.get(6, 5), Color::rgb(232, 240, 255).argb(), "the glint");
        assert_eq!(c.get(3, 6), Color::rgb(192, 192, 192).argb(), "the body");
    }

    #[test]
    fn the_location_pin_is_blue_with_a_white_hole() {
        let c = draw_location(16);
        let rows = ink_rows(&c, Color::rgb(32, 96, 208));
        assert_eq!(rows[0], "................");
        assert_eq!(rows[3], "...?########?...");
        assert_eq!(rows[5], "...?##????##?...");
        assert_eq!(rows[12], "......?##?......");
        assert_eq!(rows[14], ".......??.......", "the point");
        assert_eq!(c.get(7, 5), Color::rgb(255, 255, 255).argb(), "the hole");
        assert_eq!(c.get(7, 14), Color::rgb(0, 32, 96).argb(), "the outline");
    }

    #[test]
    fn scale_2_camera_and_pin_double_every_cell() {
        for (one, two) in [
            (draw_camera(16), draw_camera(32)),
            (draw_location(16), draw_location(32)),
        ] {
            for y in 0..16 {
                for x in 0..16 {
                    assert_eq!(two.get(2 * x + 1, 2 * y), one.get(x, y), "({x}, {y})");
                }
            }
        }
    }
}
