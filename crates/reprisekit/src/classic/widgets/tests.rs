use super::*;
use crate::look::classic_paint;
use crate::theme::{FontConfig, Theme};

fn setup() -> (Theme, TextRenderer) {
    (Theme::default(), TextRenderer::load(&FontConfig::default()))
}

fn fnv(c: &Canvas) -> u64 {
    let words = [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied());
    words
        .flat_map(u32::to_le_bytes)
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

fn grown_button(pct: u32, d: i32, g: Glyph, pressed: bool) -> Canvas {
    let t = Theme::default().with_text_size(pct).scaled(d);
    let (w, h) = (
        t.metrics.caption_button_width,
        t.metrics.caption_button_height,
    );
    let mut c = Canvas::new(w as u32, h as u32);
    draw_caption_button(&mut c, &t.colors, Rect::new(0, 0, w, h), g, pressed, d);
    c
}

const GLYPHS: [Glyph; 5] = [
    Glyph::Minimize,
    Glyph::Maximize,
    Glyph::Restore,
    Glyph::Close,
    Glyph::Help,
];

#[test]
fn a_grown_maximize_box_has_even_sides() {
    let ink = Theme::default().colors.button_text.argb();
    for pct in [150, 200] {
        let c = grown_button(pct, 1, Glyph::Maximize, false);
        let mid = c.height as i32 / 2;
        let cols: Vec<i32> = (0..c.width as i32)
            .filter(|&x| c.get(x, mid) == ink)
            .collect();
        let (l, r) = (cols[0], cols[cols.len() - 1]);
        let run = |from: i32, step: i32| {
            (0..)
                .take_while(|i| c.get(from + i * step, mid) == ink)
                .count()
        };
        assert_eq!(run(l, 1), run(r, -1), "{pct}%: left and right sides alike");
        let left = l;
        let right = c.width as i32 - 1 - r;
        assert!(
            (left - right).abs() <= 1,
            "{pct}%: centred, {left} and {right}"
        );
    }
}

#[test]
fn grown_caption_buttons_pin_at_150_and_200_percent() {
    let mut got = Vec::new();
    for pct in [150, 200] {
        for d in [1, 2] {
            for g in GLYPHS {
                for pressed in [false, true] {
                    let c = grown_button(pct, d, g, pressed);
                    let key = format!("{pct} {d} {g:?} {pressed}");
                    got.push((key, fnv(&c)));
                }
            }
        }
    }
    crate::pins::check("classic/caption_buttons_grown", &got);
}

#[test]
fn a_grown_tab_label_keeps_its_descenders() {
    let (theme, text) = setup();
    let theme = theme.with_text_size(200);
    let p = theme.paint();
    let px = theme.fonts.size_px;
    let ink_rows = |c: &Canvas| {
        let ink = theme.colors.button_text.argb();
        (0..c.height as i32)
            .filter(|&y| (0..c.width as i32).any(|x| c.get(x, y) == ink))
            .count()
    };
    let mut free = Canvas::new(120, 80);
    free.fill_rect(free.bounds(), theme.colors.button_face);
    let r = Rect::new(0, 0, 120, 80);
    text.draw(
        &mut free,
        r,
        "gyp",
        false,
        theme.colors.button_text,
        Align::Center,
        px,
    );
    let want = ink_rows(&free);
    let tab_h = theme.text.len(18);
    let page = Rect::new(0, tab_h + 4, 120, 30);
    for (open, r) in [
        (false, Rect::new(4, 4, 80, tab_h)),
        (true, Rect::new(2, 2, 84, tab_h + 2)),
    ] {
        let mut c = Canvas::new(120, 80);
        draw_tab(&mut c, &text, &p, r, "gyp", open, page, px);
        assert_eq!(
            ink_rows(&c),
            want,
            "open {open}: every row of the label shows"
        );
    }
}

#[test]
fn a_field_frame_is_the_sunken_two_pixel_edge() {
    let cs = ColorScheme::default();
    let r = Rect::new(2, 2, 40, 20);
    let mut want = Canvas::new(44, 24);
    want.edge_t(r, EdgeStyle::ButtonSunken, &cs, 1);
    let mut got = Canvas::new(44, 24);
    draw_field_frame(&mut got, &classic_paint(&cs, 1), r, cs.window);
    assert!(got == want, "the field frame draws the sunken 2 px edge");
    assert_eq!(
        got.get(2, 2),
        cs.button_shadow.argb(),
        "the frame's outer top-left is the button shadow"
    );
    assert_eq!(
        got.get(3, 3),
        cs.button_dk_shadow.argb(),
        "the frame's inner top-left is the button dark shadow"
    );
    assert_eq!(
        got.get(41, 21),
        cs.button_hilight.argb(),
        "the frame's outer bottom-right is the button hilight"
    );
    assert_eq!(
        got.get(40, 20),
        cs.button_light.argb(),
        "the frame's inner bottom-right is the button light"
    );
}

#[test]
fn focus_rings_treat_scale_zero_as_one() {
    let ring = Rect::new(0, 0, 8, 6);
    let ink = Color::rgb(0, 0, 0);
    let plain = |s| {
        let mut c = Canvas::new(8, 6);
        draw_focus_ring(&mut c, ring, s, ink);
        c
    };
    let screen = |s| {
        let mut c = Canvas::new(8, 6);
        draw_focus_ring_screen(&mut c, ring, s, ink, (0, 1));
        c
    };
    assert!(plain(0) == plain(1), "scale 0 draws the scale-1 ring");
    assert!(
        screen(0) == screen(1),
        "scale 0 draws the scale-1 screen ring"
    );
    assert_eq!(plain(1).get(0, 0), ink.argb());
}

/// The dots follow the screen's parity, not the canvas's.
#[test]
fn screen_ring_dots_follow_the_screen_parity() {
    let ink = Color::rgb(197, 145, 90);
    let mut top = Canvas::new(300, 1);
    draw_focus_ring_screen(&mut top, Rect::new(0, 0, 300, 1), 1, ink, (599, 299));
    let dots: Vec<i32> = (0..300).filter(|&x| top.get(x, 0) == ink.argb()).collect();
    assert_eq!(dots.first(), Some(&1));
    assert_eq!(dots.last(), Some(&299));
    assert_eq!(dots.len(), 150);
    let mut right = Canvas::new(1, 300);
    draw_focus_ring_screen(&mut right, Rect::new(0, 0, 1, 300), 1, ink, (898, 299));
    assert_eq!(right.get(0, 0), ink.argb());
    assert_eq!(right.get(0, 1), 0);
    assert_eq!(right.get(0, 2), ink.argb());
    let mut big = Canvas::new(20, 2);
    draw_focus_ring_screen(&mut big, Rect::new(0, 0, 20, 2), 2, ink, (0, 1));
    assert_eq!(big.get(0, 0), ink.argb());
    assert_eq!(big.get(1, 1), ink.argb());
    assert_eq!(big.get(2, 0), 0);
}

#[test]
fn an_xor_ring_flips_each_dot_once() {
    let (bg, ink) = (Color::rgb(0x10, 0x20, 0x30), Color::rgb(0xEC, 0xE9, 0xD8));
    let mut c = Canvas::new(6, 1);
    c.fill_rect(Rect::new(0, 0, 6, 1), bg);
    draw_focus_ring_screen(&mut c, Rect::new(0, 0, 6, 1), 1, RingInk::Xor(ink), (0, 0));
    let flipped = bg.argb() ^ (ink.argb() & 0x00FF_FFFF);
    let row: Vec<u32> = (0..6).map(|x| c.get(x, 0)).collect();
    let (b, f) = (bg.argb(), flipped);
    assert_eq!(
        row,
        [b, f, b, f, b, f],
        "odd columns flip once, even ones keep the background"
    );
}

#[test]
fn aligned_label_underlines_its_key_under_the_placed_text() {
    let (theme, text) = setup();
    let px = theme.fonts.size_px;
    let ink = Color::rgb(0, 0, 0);
    let r = Rect::new(3, 2, 120, 20);
    for align in [Align::Center, Align::Right] {
        let mut got = Canvas::new(130, 24);
        draw_label(&mut got, &text, r, "Sear&ch", false, ink, align, px, 1);
        let mut want = Canvas::new(130, 24);
        text.draw(&mut want, r, "Search", false, ink, align, px);
        let start = align.pen(r, text.measure("Search", false, px));
        let (dx, w) = text.span("Search", 4, false, px);
        let baseline = text.baseline(r, false, px).unwrap();
        want.fill_rect(Rect::new(start + dx, baseline + 1, w, 1), ink);
        assert!(
            got == want,
            "the {align:?} label matches its plain text plus the underline"
        );
    }
}

#[test]
fn default_button_wears_a_ring_and_the_focused_one_dots() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let r = Rect::new(10, 10, BUTTON_W, BUTTON_H);
    let mut c = Canvas::new(100, 50);
    draw_button(
        &mut c,
        &text,
        &classic_paint(cs, 1),
        r,
        "OK",
        true,
        false,
        true,
        true,
        px,
    );
    assert_eq!(c.get(r.x, r.y + 5), cs.button_text.argb());
    assert_eq!(c.get(r.x + 1, r.y + 5), cs.button_hilight.argb());
    let ring = r.inset(4);
    assert_eq!(c.get(ring.x, ring.y), cs.button_face.argb());
    assert_eq!(c.get(ring.x + 1, ring.y), cs.button_text.argb());
    assert_eq!(c.get(ring.x, ring.y + 1), cs.button_text.argb());
    let mut d = Canvas::new(100, 50);
    draw_button(
        &mut d,
        &text,
        &classic_paint(cs, 1),
        r,
        "Cancel",
        false,
        false,
        false,
        false,
        px,
    );
    assert_eq!(d.get(r.x, r.y + 5), cs.button_hilight.argb());
    assert!((r.x..r.right()).any(|x| d.get(x, r.y + 11) == cs.button_shadow.argb()));
    assert!((r.x..r.right()).all(|x| (r.y..r.bottom()).all(|y| d.get(x, y) != 0)));
}

#[test]
fn group_box_corners_take_the_right_ls_colour() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let (w, h) = (357, 70);
    let mut c = Canvas::new(w as u32 + 4, h as u32 + 4);
    let g = Rect::new(2, 2, w, h);
    let lr = Rect::new(g.x + 60, g.y, 40, 12);
    draw_group_box(
        &mut c,
        &text,
        &classic_paint(cs, 1),
        g,
        lr,
        "G",
        theme.fonts.size_px,
    );
    let at = |x: i32, y: i32| c.get(g.x + x, g.y + y);
    let (s, hl) = (cs.button_shadow.argb(), cs.button_hilight.argb());
    assert_eq!((at(0, 0), at(1, 0), at(0, 1)), (s, s, s));
    assert_eq!((at(w - 1, 0), at(0, h - 1), at(w - 1, h - 1)), (hl, hl, hl));
    assert_eq!((at(1, 1), at(2, 1), at(1, 2)), (hl, hl, hl));
    assert_eq!((at(w - 2, 1), at(1, h - 2), at(w - 2, h - 2)), (s, s, s));
}

#[test]
fn group_box_label_gap_counts_the_access_key() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let g = Rect::new(12, 87, 368, 60);
    let plain = text.measure("Programs", false, px);
    let lr = Rect::new(g.x + 9, g.y - 5, plain, 13);
    let mut c = Canvas::new(400, 160);
    c.fill_rect(Rect::new(0, 0, 400, 160), cs.button_face);
    draw_group_box(&mut c, &text, &classic_paint(cs, 1), g, lr, "&Programs", px);
    let face = cs.button_face.argb();
    let gap: Vec<i32> = (g.x..g.right())
        .filter(|&x| c.get(x, g.y) == face)
        .collect();
    assert_eq!(
        (gap[0], *gap.last().unwrap()),
        (19, 74),
        "the break is {} wide, expected 56",
        gap.len()
    );
    assert_eq!(text.measure("&Programs", false, px), plain + 7);
}

#[test]
fn caption_gradient_ramps_then_holds_its_end_colour() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let cap = |w: i32, with_help: bool| {
        let mut c = Canvas::new(w as u32 + 6, 24);
        let r = Rect::new(3, 3, w, 18);
        let close = Rect::new(r.right() - 2 - CLOSE_W, r.y + 2, CLOSE_W, CLOSE_H);
        let help = Rect::new(close.x - 2 - CLOSE_W, close.y, CLOSE_W, CLOSE_H);
        let buttons = [(close, Glyph::Close), (help, Glyph::Help)];
        let n = if with_help { 2 } else { 1 };
        draw_caption(&mut c, &text, cs, r, "T", true, &buttons[..n], px, 1);
        move |i: i32| c.get(3 + i, 19)
    };
    let one = cap(386, false);
    let rgb = |r: u8, g: u8, b: u8| Color::rgb(r, g, b).argb();
    assert_eq!(caption_ramp(386, 1, 1), 366);
    assert_eq!(one(0), rgb(10, 36, 106));
    assert_eq!(one(1), rgb(10, 36, 106));
    assert_eq!(one(3), rgb(11, 37, 107));
    assert_eq!(one(61), rgb(35, 63, 128));
    assert_eq!(one(197), rgb(93, 125, 178));
    assert_eq!(one(300), rgb(137, 172, 215));
    assert_eq!(one(365), rgb(165, 201, 239));
    assert_eq!(one(366), rgb(166, 202, 240));
    assert_eq!(one(385), rgb(166, 202, 240));
    let two = cap(361, true);
    assert_eq!(caption_ramp(361, 2, 1), 325);
    assert_eq!(two(25), rgb(21, 48, 116));
    assert_eq!(two(324), rgb(165, 201, 239));
    assert_eq!(two(325), rgb(166, 202, 240));
    assert_eq!(two(360), rgb(166, 202, 240));
}

#[test]
fn radio_draws_its_circle() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let b = Rect::new(3, 2, RADIO, RADIO);
    let read = |c: &Canvas| {
        (0..RADIO)
            .map(|y| {
                (0..RADIO)
                    .map(|x| match c.get(b.x + x, b.y + y) {
                        p if p == cs.button_shadow.argb() => 's',
                        p if p == cs.button_dk_shadow.argb() => 'd',
                        p if p == cs.button_hilight.argb() => 'h',
                        p if p == cs.window.argb() => 'w',
                        p if p == cs.button_text.argb() => 'k',
                        _ => '.',
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
    };
    let mut c = Canvas::new(20, 20);
    c.fill_rect(Rect::new(0, 0, 20, 20), cs.button_face);
    draw_radio(&mut c, &classic_paint(cs, 1), b, false);
    // Hilight and window are both white in the classic scheme, so the rim reads back 'h' where
    // 'w' sits inside it.
    assert_eq!(
        read(&c),
        [
            "....ssss....",
            "..ssddddss..",
            ".sddhhhhddh.",
            ".sdhhhhhh.h.",
            "sdhhhhhhhh.h",
            "sdhhhhhhhh.h",
            "sdhhhhhhhh.h",
            "sdhhhhhhhh.h",
            ".sdhhhhhh.h.",
            ".s..hhhh..h.",
            "..hh....hh..",
            "....hhhh....",
        ]
    );
    draw_radio(&mut c, &classic_paint(cs, 1), b, true);
    assert_eq!(
        read(&c)[4..8],
        [
            "sdhhhkkhhh.h",
            "sdhhkkkkhh.h",
            "sdhhkkkkhh.h",
            "sdhhhkkhhh.h"
        ]
    );
}

#[test]
fn a_radio_on_white_closes_its_face_arc() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let b = Rect::new(3, 2, RADIO, RADIO);
    let mut c = Canvas::new(20, 20);
    c.fill_rect(Rect::new(0, 0, 20, 20), cs.window);
    draw_radio(&mut c, &classic_paint(cs, 1), b, false);
    let rows: Vec<String> = (0..RADIO)
        .map(|y| {
            (0..RADIO)
                .map(|x| match c.get(b.x + x, b.y + y) {
                    p if p == cs.button_shadow.argb() => 's',
                    p if p == cs.button_dk_shadow.argb() => 'd',
                    p if p == cs.button_face.argb() => 'f',
                    _ => '.',
                })
                .collect()
        })
        .collect();
    assert_eq!(
        rows,
        [
            "....ssss....",
            "..ssddddss..",
            ".sdd....dd..",
            ".sd......f..",
            "sd........f.",
            "sd........f.",
            "sd........f.",
            "sd........f.",
            ".sd......f..",
            ".sff....ff..",
            "....ffff....",
            "............",
        ]
    );
}

#[test]
fn a_disabled_radio_greys_its_face_and_dot() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let b = Rect::new(3, 2, RADIO, RADIO);
    let mut c = Canvas::new(20, 20);
    c.fill_rect(Rect::new(0, 0, 20, 20), cs.button_face);
    draw_radio_disabled(&mut c, &classic_paint(cs, 1), b, true);
    assert_eq!(c.get(b.x + 4, b.y), cs.button_shadow.argb());
    assert_eq!(c.get(b.x + 4, b.y + 1), cs.button_dk_shadow.argb());
    assert_eq!(c.get(b.x + 5, b.y + 3), cs.button_face.argb());
    assert_eq!(c.get(b.x + 5, b.y + 5), cs.button_shadow.argb());
    draw_radio_disabled(&mut c, &classic_paint(cs, 1), b, false);
    assert_eq!(c.get(b.x + 5, b.y + 5), cs.button_face.argb());
}

#[test]
fn checkbox_and_field_are_sunken_white() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let mut c = Canvas::new(40, 40);
    let b = Rect::new(2, 2, 13, 13);
    draw_checkbox(&mut c, &classic_paint(cs, 1), b, true);
    assert_eq!(c.get(b.x, b.y), cs.button_shadow.argb());
    assert_eq!(c.get(b.x + 6, b.y + 6), cs.button_text.argb(), "the mark");
    let mut d = Canvas::new(40, 40);
    draw_checkbox(&mut d, &classic_paint(cs, 1), b, false);
    assert_eq!(d.get(b.x + 6, b.y + 6), cs.window.argb());
    let f = Rect::new(0, 20, 40, 20);
    let inner = draw_sunken_field(&mut d, &classic_paint(cs, 1), f, cs.window);
    assert_eq!(inner, f.inset(FIELD_PAD));
    assert_eq!(d.get(inner.x, inner.y), cs.window.argb());
}

#[test]
fn a_doubled_indeterminate_checkbox_dithers_in_scaled_cells() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let mut c = Canvas::new(40, 40);
    let b = Rect::new(5, 5, 25, 25);
    draw_checkbox_state(&mut c, &classic_paint(cs, 2), b, CheckState::Mixed);
    for (dx, dy, want) in [
        (4, 4, cs.window),
        (5, 5, cs.window),
        (6, 4, cs.button_face),
        (7, 5, cs.button_face),
        (4, 6, cs.button_face),
        (6, 6, cs.window),
    ] {
        assert_eq!(
            c.get(b.x + dx, b.y + dy),
            want.argb(),
            "cell at +({dx}, {dy})"
        );
    }
    for i in 0..40 {
        assert_eq!(c.get(b.right(), i), 0, "nothing right of the box at y {i}");
        assert_eq!(c.get(i, b.bottom()), 0, "nothing under the box at x {i}");
    }
}

#[test]
fn an_indeterminate_checkbox_is_dithered_with_a_grey_mark() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let mut c = Canvas::new(140, 340);
    let b = Rect::new(100, 300, 13, 13);
    draw_checkbox_state(&mut c, &classic_paint(cs, 1), b, CheckState::Mixed);
    assert_eq!(c.get(b.x + 2, b.y + 2), cs.window.argb());
    assert_eq!(c.get(b.x + 3, b.y + 2), cs.button_face.argb());
    assert_eq!(c.get(b.x + 2, b.y + 3), cs.button_face.argb());
    for (i, dy) in [2, 3, 4, 3, 2, 1, 0].into_iter().enumerate() {
        let (x, y) = (b.x + 3 + i as i32, b.y + 3 + dy);
        assert_eq!(c.get(x, y), cs.button_shadow.argb(), "stroke {i}");
        assert_eq!(c.get(x, y + 2), cs.button_shadow.argb(), "stroke {i} end");
    }
    assert_eq!(c.get(b.x, b.y), cs.button_shadow.argb(), "the sunken edge");
    assert_eq!(CheckState::Mixed.cycled(true), CheckState::Off);
    assert_eq!(CheckState::Off.cycled(true), CheckState::On);
    assert_eq!(CheckState::On.cycled(true), CheckState::Mixed);
    assert_eq!(CheckState::On.cycled(false), CheckState::Off);
}

#[test]
fn a_held_button_wears_its_pushed_frame_and_shifts_its_label() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let r = Rect::new(0, 0, BUTTON_W, BUTTON_H);
    let mut c = Canvas::new(80, 30);
    c.fill_rect(c.bounds(), cs.button_face);
    draw_button(
        &mut c,
        &text,
        &classic_paint(cs, 1),
        r,
        "Cancel",
        false,
        true,
        true,
        false,
        11,
    );
    for (x, y) in [(40, 0), (40, 22), (0, 11), (74, 11)] {
        assert_eq!(c.get(x, y), cs.window_frame.argb(), "outer ring at {x},{y}");
    }
    for (x, y) in [(40, 1), (40, 21), (1, 11), (73, 11)] {
        assert_eq!(
            c.get(x, y),
            cs.button_shadow.argb(),
            "inner ring at {x},{y}"
        );
    }
    let mut up = Canvas::new(80, 30);
    up.fill_rect(up.bounds(), cs.button_face);
    draw_button(
        &mut up,
        &text,
        &classic_paint(cs, 1),
        r,
        "Cancel",
        false,
        false,
        true,
        false,
        11,
    );
    assert_eq!(up.get(40, 0), cs.button_hilight.argb());
    assert_eq!(up.get(40, 22), cs.button_dk_shadow.argb());
    assert_eq!(up.get(40, 21), cs.button_shadow.argb());
    let ink = |c: &Canvas| {
        (3..20)
            .find(|&y| (5..70).any(|x| c.get(x, y) == cs.button_text.argb()))
            .unwrap()
    };
    assert_eq!(ink(&c), ink(&up) + 1);
    let mut def = Canvas::new(80, 30);
    def.fill_rect(def.bounds(), cs.button_face);
    draw_button(
        &mut def,
        &text,
        &classic_paint(cs, 1),
        r,
        "Cancel",
        true,
        true,
        true,
        false,
        11,
    );
    assert_eq!(def.get(40, 0), cs.window_frame.argb());
    assert_eq!(def.get(40, 1), cs.button_shadow.argb());
    assert_eq!(def.get(40, 2), cs.button_face.argb());
}

#[test]
fn a_bar_shorter_than_two_arrows_halves_them() {
    for thick in [16, 17] {
        for (len, arrow) in [(20, 10), (10, 5)] {
            let v = scrollbar_parts(Rect::new(3, 7, thick, len), 1, 0, 5, 20, false, 8);
            assert_eq!(v.back, Rect::new(3, 7, thick, arrow), "{thick}x{len} up");
            assert_eq!(
                v.forward,
                Rect::new(3, 7 + arrow, thick, arrow),
                "{thick}x{len} down"
            );
            assert!(v.track.is_empty(), "{thick}x{len}: no track");
            assert_eq!(v.thumb, None, "{thick}x{len}: no thumb");
            let h = scrollbar_parts(Rect::new(7, 3, len, thick), 1, 0, 5, 20, true, 8);
            assert_eq!(h.back, Rect::new(7, 3, arrow, thick), "{len}x{thick} left");
            assert_eq!(
                h.forward,
                Rect::new(7 + arrow, 3, arrow, thick),
                "{len}x{thick} right"
            );
            assert_eq!(h.thumb, None, "{len}x{thick}: no thumb");
        }
    }
}

#[test]
fn a_track_shorter_than_the_shortest_thumb_is_all_thumb() {
    let short = scrollbar_parts(Rect::new(0, 0, 16, 39), 1, 2, 3, 10, false, 8);
    assert_eq!(short.track, Rect::new(0, 16, 16, 7));
    assert_eq!(short.thumb, Some(short.track));
    let blue = scrollbar_parts(Rect::new(0, 0, 17, 39), 1, 2, 3, 10, false, 10);
    assert_eq!(blue.thumb, Some(Rect::new(0, 17, 17, 5)));
    let none = scrollbar_parts(Rect::new(0, 0, 17, 34), 1, 2, 3, 10, false, 10);
    assert!(none.track.is_empty());
    assert_eq!(none.thumb, None);
}

#[test]
fn scrollbar_splits_the_track_into_thumb_and_page_areas() {
    let sb = Rect::new(277, 259, 16, 109);
    let p = scrollbar_parts(sb, 1, 0, 6, 20, false, 8);
    assert_eq!(p.back, Rect::new(277, 259, 16, 16));
    assert_eq!(p.forward, Rect::new(277, 352, 16, 16));
    assert_eq!(p.track, Rect::new(277, 275, 16, 77));
    assert_eq!(p.thumb, Some(Rect::new(277, 275, 16, 23)));
    assert_eq!(scrollbar_hit(&p, 285, 262, false), Some(ScrollHit::Back));
    assert_eq!(scrollbar_hit(&p, 285, 360, false), Some(ScrollHit::Forward));
    assert_eq!(scrollbar_hit(&p, 285, 280, false), Some(ScrollHit::Thumb));
    assert_eq!(
        scrollbar_hit(&p, 285, 320, false),
        Some(ScrollHit::PageForward)
    );
    assert_eq!(scrollbar_hit(&p, 200, 320, false), None);
    let end = scrollbar_parts(sb, 1, 14, 6, 20, false, 8);
    assert_eq!(end.thumb, Some(Rect::new(277, 329, 16, 23)));
    assert_eq!(
        scrollbar_hit(&end, 285, 280, false),
        Some(ScrollHit::PageBack)
    );
    assert_eq!(scrollbar_pos_at(&p, 275, 6, 20, false), 0);
    assert_eq!(scrollbar_pos_at(&p, 329, 6, 20, false), 14);
    assert_eq!(scrollbar_pos_at(&p, 500, 6, 20, false), 14);
    assert_eq!(scrollbar_pos_at(&p, 302, 6, 20, false), 7);
    let all = scrollbar_parts(sb, 1, 0, 9, 9, false, 8);
    assert_eq!(all.thumb, None);
    assert_eq!(scrollbar_hit(&all, 285, 320, false), None);
}

#[test]
fn scrollbar_wears_soft_edges_and_sizes_the_thumb() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let sb = Rect::new(0, 0, DROP_W, 705);
    let mut c = Canvas::new(DROP_W as u32, 705);
    draw_scrollbar(&mut c, &classic_paint(cs, 1), sb, 27, 54, 81);
    assert_eq!(c.get(0, 0), cs.button_face.argb());
    assert_eq!(c.get(1, 1), cs.button_hilight.argb());
    assert_eq!(c.get(15, 0), cs.button_dk_shadow.argb());
    assert_eq!(c.get(14, 14), cs.button_shadow.argb());
    assert_eq!(c.get(7, 6), cs.button_text.argb());
    assert_eq!(c.get(4, 705 - 16 + 6), cs.button_text.argb());
    let ty = 16 + 224;
    assert_eq!(c.get(0, ty), cs.button_face.argb());
    assert_eq!(c.get(1, ty + 1), cs.button_hilight.argb());
    assert_eq!(c.get(15, ty), cs.button_dk_shadow.argb());
    assert_eq!(c.get(14, ty + 449 - 1), cs.button_dk_shadow.argb());
    assert_eq!(
        c.get(14, ty + 449),
        cs.button_face.argb(),
        "the down button"
    );
    assert_ne!(c.get(15, ty - 1), cs.button_dk_shadow.argb());
}

#[test]
fn scrollbar_greys_both_arrows_and_drops_the_thumb_when_all_fits() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let sb = Rect::new(0, 0, DROP_W, 100);
    let mut c = Canvas::new(DROP_W as u32, 100);
    draw_scrollbar(&mut c, &classic_paint(cs, 1), sb, 0, 9, 9);
    assert_eq!(c.get(7, 6), cs.button_shadow.argb());
    assert_eq!(c.get(5, 10), cs.button_hilight.argb());
    assert_eq!(c.get(11, 10), cs.button_hilight.argb());
    let dy = 100 - 16;
    assert_eq!(c.get(4, dy + 6), cs.button_shadow.argb());
    assert_eq!(c.get(11, dy + 7), cs.button_hilight.argb());
    assert_eq!(c.get(8, dy + 10), cs.button_hilight.argb());
    for y in 16..84 {
        let p = c.get(3, y);
        assert!(p == cs.button_face.argb() || p == cs.button_hilight.argb());
        assert_ne!(c.get(15, y), cs.button_dk_shadow.argb());
    }
}

#[test]
fn hscrollbar_points_its_arrows_outward() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let sb = Rect::new(0, 0, 200, DROP_W);
    let mut c = Canvas::new(200, DROP_W as u32);
    draw_hscrollbar(&mut c, &classic_paint(cs, 1), sb, 0, 100, 400);
    assert_eq!(c.get(8, 4), cs.button_text.argb());
    assert_eq!(c.get(8, 10), cs.button_text.argb());
    assert_eq!(c.get(5, 7), cs.button_text.argb());
    assert_ne!(c.get(4, 7), cs.button_text.argb());
    assert_ne!(c.get(8, 3), cs.button_text.argb());
    let rx = 200 - 16;
    assert_eq!(c.get(rx + 6, 4), cs.button_text.argb());
    assert_eq!(c.get(rx + 9, 7), cs.button_text.argb());
    assert_ne!(c.get(rx + 10, 7), cs.button_text.argb());
    assert_eq!(c.get(16, 0), cs.button_face.argb());
    assert_eq!(c.get(16 + 42 - 1, 15), cs.button_dk_shadow.argb());
    assert_ne!(c.get(16 + 42, 15), cs.button_dk_shadow.argb());
    let mut d = Canvas::new(200, DROP_W as u32);
    draw_hscrollbar(&mut d, &classic_paint(cs, 1), sb, 0, 400, 100);
    assert_eq!(d.get(8, 7), cs.button_shadow.argb());
    assert_eq!(d.get(9, 9), cs.button_hilight.argb());
    assert_eq!(d.get(9, 11), cs.button_hilight.argb());
}

#[test]
fn small_arrow_is_rows_of_five_three_and_one() {
    let ink = Color::rgb(0, 0, 0);
    let mut c = Canvas::new(9, 6);
    draw_small_arrow(&mut c, 2, 1, 1, ink);
    let inked: Vec<Vec<i32>> = (0..6)
        .map(|y| (0..9).filter(|&x| c.get(x, y) == ink.argb()).collect())
        .collect();
    assert_eq!(
        inked,
        [
            vec![],
            vec![2, 3, 4, 5, 6],
            vec![3, 4, 5],
            vec![4],
            vec![],
            vec![]
        ],
        "rows 1..=3 hold columns 2..=6, 3..=5 and 4"
    );
}

#[test]
fn size_grip_draws_three_ridges() {
    let (theme, _) = setup();
    let cs = &theme.colors;
    let mut c = Canvas::new(16, 16);
    draw_size_grip(&mut c, cs, Rect::new(0, 0, 16, 16), 1);
    let row: Vec<u32> = (0..16).map(|x| c.get(x, 15)).collect();
    let f = cs.button_face.argb();
    let h = cs.button_hilight.argb();
    let s = cs.button_shadow.argb();
    assert_eq!(row, [f, f, f, f, h, s, s, f, h, s, s, f, h, s, s, f]);
    assert!((0..4).all(|y| (0..16).all(|x| c.get(x, y) == f)));
    assert_eq!(c.get(15, 4), h);
    assert_eq!(c.get(14, 5), h);
    assert_eq!(c.get(15, 5), s);
}

#[test]
fn an_inactive_caption_ramps_grey_with_grey_ink() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let r = Rect::new(0, 0, 398, 18);
    let close = Rect::new(r.right() - 2 - CLOSE_W, 2, CLOSE_W, CLOSE_H);
    let help = Rect::new(close.x - 2 - CLOSE_W, 2, CLOSE_W, CLOSE_H);
    let mut c = Canvas::new(398, 18);
    draw_caption(
        &mut c,
        &text,
        cs,
        r,
        "Reprise Settings",
        false,
        &[(close, Glyph::Close), (help, Glyph::Help)],
        px,
        1,
    );
    let grey = |v: u8| Color::rgb(v, v, v).argb();
    for (x, v) in [
        (0, 128),
        (25, 132),
        (51, 137),
        (191, 161),
        (291, 179),
        (361, 191),
        (362, 192),
    ] {
        assert_eq!(c.get(x, 16), grey(v), "grey {v} at x {x}");
    }
    let ink = Color::rgb(212, 208, 200).argb();
    assert!(
        (0..120).any(|x| (0..18).any(|y| c.get(x, y) == ink)),
        "title ink 212,208,200"
    );
}

#[test]
fn caption_has_the_gradient_title_and_buttons() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let cap = Rect::new(0, 0, 200, 18);
    let close = Rect::new(182, 2, CLOSE_W, CLOSE_H);
    let help = Rect::new(164, 2, CLOSE_W, CLOSE_H);
    let mut c = Canvas::new(200, 18);
    draw_caption(
        &mut c,
        &text,
        cs,
        cap,
        "Run",
        true,
        &[(close, Glyph::Close), (help, Glyph::Help)],
        px,
        1,
    );
    assert_eq!(c.get(0, 9), cs.active_title.argb());
    assert!(
        (0..200).any(|x| c.get(x, 9) == cs.title_text.argb()),
        "title ink"
    );
    assert_eq!(c.get(close.x, close.y), cs.button_hilight.argb());
    assert_eq!(c.get(help.x + 7, help.y + 2), cs.button_text.argb());
}

#[test]
fn a_long_value_scrolls_to_keep_the_caret_in_the_field() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let px = theme.fonts.size_px;
    let mut e = TextEdit::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", false);
    e.set_selection(e.text.len(), e.text.len());
    let first = edit_first_visible(&text, &e, 40, px, 0);
    assert!(first > 0, "the view scrolled off the left");
    assert!(text.measure(&e.text[first..e.caret()], false, px) <= 40);
    e.set_selection(0, 0);
    assert_eq!(edit_first_visible(&text, &e, 40, px, first), 0);
}

fn first_visible_by_remeasure(
    text: &TextRenderer,
    edit: &TextEdit,
    inner_w: i32,
    px: u32,
    first: usize,
) -> usize {
    let mut first = first.min(edit.text.len());
    if !edit.text.is_char_boundary(first) {
        first = 0;
    }
    if edit.caret() < first {
        first = edit.caret();
    }
    while first < edit.caret() && text.measure(&edit.text[first..edit.caret()], false, px) > inner_w
    {
        first = edit.text[first..]
            .char_indices()
            .nth(1)
            .map_or(edit.text.len(), |(n, _)| first + n);
    }
    while first > 0 {
        let back = edit.text[..first]
            .char_indices()
            .next_back()
            .map_or(0, |(n, _)| n);
        if text.measure(&edit.text[back..], false, px) > inner_w {
            break;
        }
        first = back;
    }
    first
}

#[test]
fn the_scroll_matches_remeasuring_every_start() {
    let (theme, text) = setup();
    let px = theme.fonts.size_px;
    for value in [
        "",
        "abc",
        "C:\\Program Files\\Reprise\\notepad.exe /a /b",
        "iiiiiiiWWWWWWWiiiiiiiWWWWWWW",
        "áéíóú日本語テキストáéíóú\u{301}x\u{10FFFF}ÿ",
    ] {
        let mut e = TextEdit::new(value, false);
        let bounds: Vec<usize> = value
            .char_indices()
            .map(|(i, _)| i)
            .chain([value.len(), 1])
            .collect();
        for w in [-1, 0, 5, 40, 120, 1000] {
            for &caret in bounds.iter().filter(|&&i| value.is_char_boundary(i)) {
                e.set_selection(caret, caret);
                for &from in &bounds {
                    assert_eq!(
                        edit_first_visible(&text, &e, w, px, from),
                        first_visible_by_remeasure(&text, &e, w, px, from),
                        "{value:?} width {w} caret {caret} from {from}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_visible_caret_does_not_move_the_view() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let px = theme.fonts.size_px;
    let mut e = TextEdit::new("abcdef", false);
    e.set_selection(3, 3);
    assert_eq!(edit_first_visible(&text, &e, 200, px, 0), 0);
}

#[test]
fn a_scrolled_field_paints_nothing_past_its_right_edge() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let mut c = Canvas::new(120, 40);
    c.fill_rect(c.bounds(), cs.button_face);
    let inner = Rect::new(10, 10, 40, 13);
    let mut e = TextEdit::new("wwwwwwwwwwwwwwwwwwwwwwwwwwww", false);
    e.set_selection(e.text.len(), e.text.len());
    let first = edit_first_visible(&text, &e, inner.width, px, 0);
    draw_edit(
        &mut c,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Unfocused,
        first,
    );
    for x in inner.right()..120 {
        for y in inner.y..inner.bottom() {
            assert_eq!(c.get(x, y), cs.button_face.argb(), "ink at {x},{y}");
        }
    }
}

#[test]
fn a_preedit_draws_underlined_at_the_caret_and_the_spot_leaves_it_out() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let inner = Rect::new(10, 10, 120, 15);
    let mut e = TextEdit::new("nima", false);
    e.place(2, false);
    let mut c = Canvas::new(150, 40);
    c.fill_rect(c.bounds(), cs.window);
    c.set_preedit(crate::textedit::Preedit::new("hao".into(), 2, 2));
    draw_edit(
        &mut c,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Shown,
        0,
    );
    let x0 = inner.x + text.measure("ni", false, px);
    let x1 = x0 + text.measure("hao", false, px);
    let ul = text.baseline(inner, false, px).unwrap() + 1;
    let ink = cs.button_text.argb();
    assert!((x0..x1).all(|x| c.get(x, ul) == ink), "underline under hao");
    assert_ne!(c.get(x0 - 1, ul), ink, "not under ni");
    assert_ne!(c.get(x1, ul), ink, "not under ma");
    let spot = c.take_spot().expect("a focused field reports");
    let caret_x = inner.x + text.measure("niha", false, px);
    assert_eq!(spot.caret, Rect::new(caret_x, 10, 1, 13));
    assert_eq!(spot.field, inner);
    let sur = spot.surrounding.expect("plain text");
    assert_eq!((sur.text.as_str(), sur.cursor, sur.anchor), ("nima", 2, 2));
    let mut off = Canvas::new(150, 40);
    off.set_preedit(crate::textedit::Preedit::new("hao".into(), 2, 2));
    draw_edit(
        &mut off,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Unfocused,
        0,
    );
    assert_eq!(off.take_spot(), None);
    let mut plain = Canvas::new(150, 40);
    draw_edit(
        &mut plain,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Unfocused,
        0,
    );
    assert_eq!(off.pixels, plain.pixels);
}

#[test]
fn a_preedit_range_draws_only_underlined_with_the_caret_at_its_end() {
    let (theme, text) = setup();
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let inner = Rect::new(10, 10, 120, 15);
    let mut e = TextEdit::new("nima", false);
    e.place(2, false);
    let mut c = Canvas::new(150, 40);
    c.fill_rect(c.bounds(), cs.window);
    c.set_preedit(crate::textedit::Preedit::new("hao".into(), 1, 3));
    draw_edit(
        &mut c,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Shown,
        0,
    );
    let x0 = inner.x + text.measure("ni", false, px);
    let x1 = inner.x + text.measure("nihao", false, px);
    let hl = cs.hilight.argb();
    assert!(c.pixels.iter().all(|&p| p != hl), "nothing on hilight");
    let ul = text.baseline(inner, false, px).unwrap() + 1;
    let ink = cs.button_text.argb();
    assert!((x0..x1).all(|x| c.get(x, ul) == ink), "underline under hao");
    let row = text.line(inner, false, px).unwrap().0;
    assert_eq!(c.get(x1, row), ink, "the caret bar after hao");
    let spot = c.take_spot().expect("a focused field reports");
    assert_eq!(spot.caret.x, x1, "the caret ends the range");
}

#[test]
fn a_selection_and_the_caret_cover_13_rows_from_the_text_area_s_top() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    for h in [14, 15] {
        let inner = Rect::new(10, 10, 100, h);
        let mut e = TextEdit::new("  hello", false);
        e.set_selection(0, 2);
        let mut c = Canvas::new(130, 40);
        c.fill_rect(c.bounds(), cs.window);
        draw_edit(
            &mut c,
            &text,
            &e,
            inner,
            px,
            &classic_paint(cs, 1),
            Caret::Shown,
            0,
        );
        let lit = |x: i32, color: Color| -> Vec<i32> {
            (0..40).filter(|&y| c.get(x, y) == color.argb()).collect()
        };
        let rows: Vec<i32> = (10..23).collect();
        assert_eq!(
            lit(inner.x + 1, cs.hilight),
            rows,
            "{h}-row area: selection"
        );
        let caret_x = inner.x + text.measure("  ", false, px);
        assert_eq!(lit(caret_x, cs.button_text), rows, "{h}-row area: caret");
    }
}

#[test]
fn a_hidden_caret_still_reports_its_spot() {
    let (theme, text) = setup();
    let px = theme.fonts.size_px;
    let e = TextEdit::new("abc", false);
    let inner = Rect::new(0, 0, 60, 15);
    let mut shown = Canvas::new(60, 15);
    draw_edit(
        &mut shown,
        &text,
        &e,
        inner,
        px,
        &classic_paint(&theme.colors, 1),
        Caret::Shown,
        0,
    );
    let mut hidden = Canvas::new(60, 15);
    draw_edit(
        &mut hidden,
        &text,
        &e,
        inner,
        px,
        &classic_paint(&theme.colors, 1),
        Caret::Hidden,
        0,
    );
    assert_ne!(shown.pixels, hidden.pixels);
    assert_eq!(shown.take_spot(), hidden.take_spot());
}

#[test]
fn a_scrolled_multibyte_value_stays_on_char_boundaries() {
    let (theme, text) = setup();
    if !crate::text::test_tahoma() {
        return;
    }
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    // 2-byte Latin-1 accents and 3-byte CJK well past the field, so `first` lands inside a
    // multi-byte run.
    let mut e = TextEdit::new("áéíóú日本語テキストáéíóú".repeat(4), false);
    e.set_selection(e.text.len(), e.text.len());
    let first = edit_first_visible(&text, &e, 40, px, 0);
    assert!(
        e.text.is_char_boundary(first),
        "first ({first}) split a multi-byte character"
    );
    assert!(first > 0, "the view scrolled off the left");
    let _visible = &e.text[first..];
    let mut c = Canvas::new(60, 40);
    let inner = Rect::new(10, 10, 40, 13);
    draw_edit(
        &mut c,
        &text,
        &e,
        inner,
        px,
        &classic_paint(cs, 1),
        Caret::Unfocused,
        first,
    );
    e.set_selection(0, 0);
    let home = edit_first_visible(&text, &e, 40, px, first);
    assert_eq!(home, 0);
    let _visible = &e.text[home..];
}
