use super::geometry::nine_axis;
use super::paint::in_rounded;
use super::*;

#[test]
fn canvas_damage_defaults_to_whole() {
    assert_eq!(Canvas::new(4, 4).take_damage(), None);
}

#[test]
fn canvas_damage_collects_rects() {
    let mut c = Canvas::new(8, 8);
    c.report_damage(Rect::new(0, 0, 2, 2));
    c.report_damage(Rect::new(3, 4, 1, 1));
    assert_eq!(
        c.take_damage(),
        Some(vec![Rect::new(0, 0, 2, 2), Rect::new(3, 4, 1, 1)])
    );
    assert_eq!(c.take_damage(), None);
}

#[test]
fn ink_coverage_is_full_under_the_ink_and_clear_elsewhere_per_device_pixel() {
    let coverage = Canvas::with_scale(3, 2, 2).ink_coverage(|c, ink| {
        c.fill_rect(Rect::new(1, 0, 1, 1), ink);
    });
    let mut want = vec![0u8; 6 * 4];
    for i in [2, 3, 8, 9] {
        want[i] = 0xFF;
    }
    assert_eq!(coverage, want);
}

#[test]
fn rects_sharing_only_an_edge_do_not_intersect() {
    let a = Rect::new(9, 0, 50, 25);
    assert!(!a.intersects(Rect::new(0, 25, 360, 80)));
    assert!(a.intersects(Rect::new(0, 24, 360, 80)));
    assert!(!a.intersects(Rect::new(59, 0, 10, 10)));
}

#[test]
fn fit_size_scales_down_keeping_aspect() {
    assert_eq!(fit_size(800, 600, (96, 96)), (96, 72));
}

#[test]
fn fit_size_never_upscales() {
    assert_eq!(fit_size(50, 20, (96, 96)), (50, 20));
}

#[test]
fn mix_keeps_its_own_pixels_at_zero_takes_the_others_at_one_and_blends_between() {
    let mut a = Canvas::new(4, 2);
    let mut b = Canvas::new(4, 2);
    a.pixels.fill(0x80_40_20_10);
    b.pixels.fill(0xFF_FF_FF_FF);
    let mut m = a.clone();
    m.mix(&b, m.bounds(), 0.0);
    assert_eq!(m.pixels, a.pixels);
    let mut m = a.clone();
    m.mix(&b, m.bounds(), 1.0);
    assert_eq!(m.pixels, b.pixels);
    let mut m = a.clone();
    m.mix(&b, Rect::new(0, 0, 2, 2), 0.5);
    assert_eq!(m.get(0, 0), 0xC0_A0_90_88, "each channel half way, rounded");
    assert_eq!(m.get(3, 1), 0x80_40_20_10, "outside the rect untouched");
}

#[test]
fn mix_all_adds_each_move_so_each_canvas_shows_its_own_level() {
    let a = Canvas::from_pixels(1, 1, vec![0xFF_00_00_00]);
    let red = Canvas::from_pixels(1, 1, vec![0xFF_FF_00_00]);
    let green = Canvas::from_pixels(1, 1, vec![0xFF_00_FF_00]);
    let mut m = a.clone();
    m.mix_all(&[(&red, 0.5), (&green, 1.0)], m.bounds());
    assert_eq!(
        m.get(0, 0),
        0xFF_80_FF_00,
        "red half way, green all the way"
    );
    let mut n = a.clone();
    n.mix_all(&[(&green, 1.0), (&red, 0.5)], n.bounds());
    assert_eq!(n.pixels, m.pixels, "the order does not matter");
    let mut one = a.clone();
    one.mix(&red, one.bounds(), 0.5);
    let mut all = a.clone();
    all.mix_all(&[(&red, 0.5)], all.bounds());
    assert_eq!(all.pixels, one.pixels, "one canvas is a plain mix");
}

#[test]
fn mix_all_keeps_the_sum_premultiplied() {
    // One move lifts red to 255, the other drops alpha to 128.
    let mut m = Canvas::from_pixels(1, 1, vec![0xFF_80_80_80]);
    let red = Canvas::from_pixels(1, 1, vec![0xFF_FF_80_80]);
    let clear = Canvas::from_pixels(1, 1, vec![0x80_80_80_80]);
    m.mix_all(&[(&red, 1.0), (&clear, 1.0)], m.bounds());
    assert_eq!(
        m.get(0, 0),
        0xFF_FF_80_80,
        "alpha rises to the red it holds"
    );
    let mut up = Canvas::from_pixels(1, 1, vec![0xFF_C0_C0_C0]);
    let white = Canvas::from_pixels(1, 1, vec![0xFF_FF_FF_FF]);
    up.mix_all(&[(&white, 1.0), (&white, 1.0)], up.bounds());
    assert_eq!(up.get(0, 0), 0xFF_FF_FF_FF, "a channel stops at 255");
    let mut down = Canvas::from_pixels(1, 1, vec![0xFF_40_40_40]);
    let black = Canvas::from_pixels(1, 1, vec![0xFF_00_00_00]);
    down.mix_all(&[(&black, 1.0), (&black, 1.0)], down.bounds());
    assert_eq!(down.get(0, 0), 0xFF_00_00_00, "a channel stops at 0");
}

#[test]
fn copy_rect_takes_the_pixels_under_a_rect_in_device_pixels() {
    let mut a = Canvas::with_scale(3, 2, 2);
    for (i, p) in a.pixels.iter_mut().enumerate() {
        *p = 0xFF_00_00_00 | i as u32;
    }
    let c = a.copy_rect(Rect::new(1, 1, 3, 1));
    assert_eq!((c.width, c.height, c.scale()), (3, 1, 2));
    assert_eq!(c.pixels[..4], a.pixels[14..18], "device row 2 from x 2");
    assert_eq!(c.pixels[6..10], a.pixels[20..24], "device row 3 from x 2");
    assert_eq!(c.pixels[4..6], [0, 0], "past the right edge, transparent");
}

#[test]
fn mix_cuts_a_rect_hanging_over_the_bounds() {
    let a = Canvas::new(4, 2);
    let mut b = Canvas::new(4, 2);
    b.pixels.fill(0xFF_00_00_FF);
    let mut m = a.clone();
    m.mix(&b, Rect::new(-2, 0, 4, 1), 1.0);
    assert_eq!(m.get(0, 0), 0xFF_00_00_FF);
    assert_eq!(m.get(1, 0), 0xFF_00_00_FF);
    assert_eq!(m.get(2, 0), 0, "past the rect untouched");
    assert_eq!(m.get(0, 1), 0, "row below untouched");
}

#[test]
fn mix_works_in_device_pixels_at_scale_two() {
    let mut a = Canvas::with_scale(2, 1, 2);
    let mut b = Canvas::with_scale(2, 1, 2);
    b.pixels.fill(0xFF_00_00_FF);
    a.mix(&b, Rect::new(1, 0, 1, 1), 1.0);
    assert_eq!(a.pixels[0], 0, "logical x 0 is device x 0..2");
    assert_eq!(a.pixels[2], 0xFF_00_00_FF);
    assert_eq!(a.pixels[7], 0xFF_00_00_FF);
}

#[test]
fn the_differing_rect_holds_every_changed_pixel_in_logical_px() {
    let a = Canvas::with_scale(6, 5, 2);
    let mut b = a.clone();
    assert!(a.differing_rect(&b).is_empty(), "the same pictures");
    // Device (5, 3) is logical (2, 1); device (8, 6) is logical (4, 3).
    b.pixels[3 * 12 + 5] = 0xFF_00_00_FF;
    b.pixels[6 * 12 + 8] = 0xFF_00_00_FF;
    assert_eq!(a.differing_rect(&b), Rect::new(2, 1, 3, 3));
    let mut m = a.clone();
    m.mix(&b, a.differing_rect(&b), 1.0);
    assert_eq!(m.pixels, b.pixels, "a mix inside it reaches the other");
}

/// A scratch canvas takes the preedit down; its blit brings the spot
/// back up, moved by where it lands.
#[test]
fn a_scratch_canvas_carries_the_preedit_down_and_the_spot_up() {
    use crate::textedit::{Preedit, TextEdit};
    let mut c = Canvas::with_scale(40, 40, 2);
    c.set_preedit(Preedit::new("ka".into(), 1, 1));
    let mut sc = c.scratch(10, 10);
    assert_eq!((sc.scale(), sc.preedit()), (2, c.preedit()));
    let r = Rect::new(1, 2, 3, 4);
    sc.report_spot(TextEdit::new("x", false).spot(r, r));
    c.blit_canvas(5, 6, &sc);
    let spot = c.take_spot().expect("carried up");
    assert_eq!(spot.caret, Rect::new(6, 8, 3, 4));
    assert_eq!(c.take_spot(), None, "taken");
}
use crate::theme::{Color, ColorScheme};

#[test]
fn to_png_decodes_to_the_same_rgb_without_alpha() {
    let c = Canvas::from_pixels(
        2,
        2,
        vec![0xff3a_6ea5, 0x00ff_0000, 0x8000_ff00, 0xffff_ffff],
    );
    let png = to_png(&c);
    let mut dec = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    let mut buf = vec![0; dec.output_buffer_size().unwrap()];
    let info = dec.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (2, 2));
    assert_eq!(info.color_type, png::ColorType::Rgb);
    assert_eq!(
        &buf[..info.buffer_size()],
        [0x3a, 0x6e, 0xa5, 0xff, 0, 0, 0, 0xff, 0, 0xff, 0xff, 0xff]
    );
}

fn col(c: &Canvas, x: i32, y0: i32, y1: i32) -> Vec<u32> {
    (y0..y1).map(|y| c.get(x, y)).collect()
}

#[test]
fn window_raised_edge_columns() {
    // The left column of a window frame at x=0..4 is face, hilight,
    // face, face; the right column mirrored is face, face, shadow, dk_shadow.
    let cs = ColorScheme::default();
    let mut c = Canvas::new(20, 20);
    c.fill_rect(Rect::new(0, 0, 20, 20), cs.button_face);
    c.edge_t(Rect::new(0, 0, 20, 20), EdgeStyle::WindowRaised, &cs, 1);
    let row: Vec<u32> = (0..4).map(|x| c.get(x, 10)).collect();
    assert_eq!(
        row,
        vec![
            cs.button_face.argb(),
            cs.button_hilight.argb(),
            cs.button_face.argb(),
            cs.button_face.argb()
        ]
    );
    let row: Vec<u32> = (16..20).map(|x| c.get(x, 10)).collect();
    assert_eq!(
        row,
        vec![
            cs.button_face.argb(),
            cs.button_face.argb(),
            cs.button_shadow.argb(),
            cs.button_dk_shadow.argb()
        ]
    );
    // bottom-right corner is dk_shadow, top-right outer corner is dk_shadow too (drawn last)
    assert_eq!(c.get(19, 19), cs.button_dk_shadow.argb());
    assert_eq!(c.get(19, 0), cs.button_dk_shadow.argb());
    assert_eq!(c.get(0, 19), cs.button_dk_shadow.argb());
}

#[test]
fn button_raised_edge_row() {
    let cs = ColorScheme::default();
    let mut c = Canvas::new(16, 14);
    c.fill_rect(Rect::new(0, 0, 16, 14), cs.button_face);
    c.edge_t(Rect::new(0, 0, 16, 14), EdgeStyle::ButtonRaised, &cs, 1);
    let row: Vec<u32> = (0..16).map(|x| c.get(x, 7)).collect();
    let mut expected = vec![cs.button_hilight.argb()];
    expected.extend(std::iter::repeat_n(cs.button_face.argb(), 14));
    expected[14] = cs.button_shadow.argb();
    expected.push(cs.button_dk_shadow.argb());
    assert_eq!(row, expected);
}

#[test]
fn sunken_edge_is_inverse() {
    let cs = ColorScheme::default();
    let mut c = Canvas::new(10, 10);
    c.edge_t(Rect::new(0, 0, 10, 10), EdgeStyle::ButtonSunken, &cs, 1);
    assert_eq!(c.get(0, 5), cs.button_shadow.argb());
    assert_eq!(c.get(1, 5), cs.button_dk_shadow.argb());
    assert_eq!(c.get(8, 5), cs.button_light.argb());
    assert_eq!(c.get(9, 5), cs.button_hilight.argb());
    c.edge_t(Rect::new(0, 0, 10, 10), EdgeStyle::ButtonPushed, &cs, 1);
    assert_eq!(c.get(0, 5), cs.button_dk_shadow.argb());
    assert_eq!(c.get(1, 5), cs.button_shadow.argb());
    assert_eq!(c.get(5, 0), cs.button_dk_shadow.argb());
    assert_eq!(c.get(8, 5), cs.button_light.argb());
    assert_eq!(c.get(9, 5), cs.button_hilight.argb());
}

#[test]
fn gradient_endpoints_and_midpoint() {
    let mut c = Canvas::new(101, 1);
    c.hgradient(
        Rect::new(0, 0, 101, 1),
        Color::rgb(0, 0, 0),
        Color::rgb(200, 100, 0),
    );
    assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
    assert_eq!(c.get(100, 0), Color::rgb(200, 100, 0).argb());
    assert_eq!(c.get(50, 0), Color::rgb(100, 50, 0).argb());
}

/// A gradient hanging off both sides keeps each column's colour from
/// the full rect.
#[test]
fn clipped_gradients_match_the_unclipped_columns() {
    let (l, r) = (Color::rgb(0, 10, 200), Color::rgb(160, 90, 8));
    let mut full = Canvas::new(20, 3);
    full.hgradient(Rect::new(0, 0, 20, 3), l, r);
    let mut stepped = Canvas::new(20, 3);
    stepped.hgradient_stepped(Rect::new(0, 0, 20, 3), l, r);
    let mut c = Canvas::new(12, 2);
    let mut cs = Canvas::new(12, 2);
    c.hgradient(Rect::new(-5, -1, 20, 3), l, r);
    cs.hgradient_stepped(Rect::new(-5, -1, 20, 3), l, r);
    for y in 0..2 {
        for x in 0..12 {
            assert_eq!(c.get(x, y), full.get(x + 5, 0), "hgradient ({x}, {y})");
            assert_eq!(cs.get(x, y), stepped.get(x + 5, 0), "stepped ({x}, {y})");
        }
    }
}

#[test]
fn paint_map_scales_cells_and_skips_unlinked_characters() {
    let mut c = Canvas::new(8, 6);
    let red = Color::rgb(255, 0, 0);
    c.paint_map(2, 1, 2, &["X.", ".X"], |ch| (ch == 'X').then_some(red));
    let r = red.argb();
    assert_eq!(c.get(2, 1), r);
    assert_eq!(c.get(3, 2), r); // the cell is 2x2
    assert_eq!(c.get(4, 1), 0); // '.' is left alone
    assert_eq!(c.get(4, 3), r);
    assert_eq!(c.get(5, 4), r);
    assert_eq!(c.get(2, 3), 0);
    c.paint_map(7, 5, 3, &["XX"], |_| Some(red));
    assert_eq!(c.get(7, 5), r);
}

#[test]
fn drawing_is_clipped_to_canvas() {
    let mut c = Canvas::new(4, 4);
    c.fill_rect(Rect::new(-2, -2, 10, 10), Color::rgb(1, 1, 1));
    c.fill_rect(Rect::new(-5, 1, 55, 1), Color::rgb(2, 2, 2));
    c.vline(2, -5, 50, Color::rgb(3, 3, 3));
    assert_eq!(c.get(0, 0), Color::rgb(1, 1, 1).argb());
    assert_eq!(c.get(3, 1), Color::rgb(2, 2, 2).argb());
    assert_eq!(c.get(2, 3), Color::rgb(3, 3, 3).argb());
    assert_eq!(c.get(-1, 0), 0);
    assert_eq!(c.get(4, 0), 0);
}

#[test]
fn with_clip_limits_drawing_and_lifts_after() {
    let mut c = Canvas::new(10, 10);
    let red = Color::rgb(255, 0, 0);
    c.with_clip(Rect::new(2, 2, 4, 4), |c| {
        // A nested clip only narrows the outer one.
        c.with_clip(Rect::new(0, 0, 5, 10), |c| c.fill_rect(c.bounds(), red));
    });
    let painted: Vec<(i32, i32)> = (0..10)
        .flat_map(|y| (0..10).map(move |x| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != 0)
        .collect();
    let want: Vec<(i32, i32)> = (2..6).flat_map(|y| (2..5).map(move |x| (x, y))).collect();
    assert_eq!(painted, want, "only x 2..5, y 2..6");
    c.fill_rect(Rect::new(9, 9, 1, 1), red);
    assert_eq!(c.get(9, 9), red.argb(), "the clip is lifted after");
}

#[test]
fn off_canvas_rects_are_ignored() {
    let mut c = Canvas::new(4, 4);
    let red = Color::rgb(255, 0, 0);
    c.fill_rect(Rect::new(1000, 1, 5, 1), red);
    c.fill_rect(Rect::new(1, 1000, 1, 5), red);
    c.fill_rect(Rect::new(-1000, 1, 5, 1), red);
    c.fill_rect(Rect::new(1, -1000, 1, 5), red);
    c.vline(1000, 0, 10, red);
    assert!(c.pixels.iter().all(|&p| p == 0));
}

#[test]
fn dither_is_checkerboard() {
    let a = Color::rgb(10, 10, 10);
    let b = Color::rgb(20, 20, 20);
    let mut c = Canvas::new(4, 2);
    c.dither_rect_t(Rect::new(0, 0, 4, 2), a, b, 1);
    assert_eq!(col(&c, 0, 0, 2), vec![a.argb(), b.argb()]);
    assert_eq!(col(&c, 1, 0, 2), vec![b.argb(), a.argb()]);
}

#[test]
#[should_panic(expected = "blit_mask: 3x2 mask needs 6 bytes, got 5")]
fn a_short_mask_is_refused_up_front() {
    let mut c = Canvas::new(4, 4);
    let clip = c.bounds();
    c.blit_mask(0, 0, 3, 2, &[255; 5], Color::rgb(0, 0, 0), true, clip);
}

#[test]
fn mask_blit_threshold_and_blend() {
    let mut c = Canvas::new(3, 1);
    c.fill_rect(Rect::new(0, 0, 3, 1), Color::rgb(0, 0, 0));
    let mask = [0u8, 100, 255];
    c.blit_mask(
        0,
        0,
        3,
        1,
        &mask,
        Color::rgb(255, 255, 255),
        false,
        Rect::new(0, 0, 3, 1),
    );
    assert_eq!(c.get(0, 0), Color::rgb(0, 0, 0).argb());
    assert_eq!(c.get(1, 0), Color::rgb(255, 255, 255).argb());
    assert_eq!(c.get(2, 0), Color::rgb(255, 255, 255).argb());
    let mut c = Canvas::new(1, 1);
    c.fill_rect(Rect::new(0, 0, 1, 1), Color::rgb(0, 0, 0));
    c.blit_mask(
        0,
        0,
        1,
        1,
        &[128],
        Color::rgb(255, 255, 255),
        true,
        Rect::new(0, 0, 1, 1),
    );
    let v = c.get(0, 0) & 0xFF;
    assert!((127..=129).contains(&v), "blend was {v}");
}

#[test]
fn canvas_blit_is_premultiplied_source_over() {
    let mut c = Canvas::new(2, 1);
    c.put(0, 0, Color::rgb(0, 0, 0));
    let mut src = Canvas::new(1, 1);
    src.pixels[0] = 0x8080_0000;
    c.blit_canvas(0, 0, &src);
    assert_eq!(c.get(0, 0), 0xFF80_0000);
    let mut untouched = Canvas::new(1, 1);
    untouched.put(0, 0, Color::rgb(1, 2, 3));
    let transparent = Canvas::new(1, 1);
    untouched.blit_canvas(0, 0, &transparent);
    assert_eq!(untouched.get(0, 0), Color::rgb(1, 2, 3).argb());
    let mut small = Canvas::new(1, 1);
    small.blit_canvas(5, 5, &src);
}

/// The clipped row loop must match a per-pixel source-over at every
/// edge: a source hanging off the top-left and bottom-right corners.
#[test]
fn partly_off_canvas_blits_match_a_per_pixel_reference() {
    let mut src = Canvas::new(5, 4);
    for (i, p) in src.pixels.iter_mut().enumerate() {
        let a = (i as u32 * 13) & 0xFF;
        *p = a << 24 | (a / 2) << 16 | (a / 3) << 8 | (a / 4);
    }
    for (x, y) in [(-2, -1), (4, 3), (0, 0), (-9, 0), (6, 0)] {
        let mut base = Canvas::new(6, 5);
        for (i, p) in base.pixels.iter_mut().enumerate() {
            *p = 0x8000_0000 | ((i as u32 * 7) & 0xFF) << 8;
        }
        let mut want = base.clone();
        for sy in 0..src.height as i32 {
            for sx in 0..src.width as i32 {
                let (dx, dy) = (x + sx, y + sy);
                if !want.bounds().contains(dx, dy) {
                    continue;
                }
                let s = src.get(sx, sy);
                let sa = s >> 24;
                if sa == 0 {
                    continue;
                }
                let d = want.get(dx, dy);
                let bl = |sc: u32, dc: u32| sc + dc * (255 - sa) / 255;
                let v = bl(sa, d >> 24) << 24
                    | bl((s >> 16) & 0xFF, (d >> 16) & 0xFF) << 16
                    | bl((s >> 8) & 0xFF, (d >> 8) & 0xFF) << 8
                    | bl(s & 0xFF, d & 0xFF);
                want.put_raw(dx, dy, v);
            }
        }
        let mut got = base.clone();
        got.blit_over(x, y, &src);
        assert_eq!(got.pixels, want.pixels, "blit_over at ({x}, {y})");
        let mut m = base.clone();
        m.blit_mask(
            x,
            y,
            5,
            4,
            &[0x70; 20],
            Color::rgb(9, 8, 7),
            false,
            m.bounds(),
        );
        let mut mw = base.clone();
        for sy in 0..4 {
            for sx in 0..5 {
                mw.put(x + sx, y + sy, Color::rgb(9, 8, 7));
            }
        }
        assert_eq!(m.pixels, mw.pixels, "blit_mask at ({x}, {y})");
    }
}

#[test]
fn mirrored_flips_each_row() {
    let (one, two) = (Color::rgb(0, 0, 1), Color::rgb(0, 0, 2));
    let mut c = Canvas::new(3, 2);
    c.put(0, 0, one);
    c.put(2, 1, two);
    let m = c.mirrored();
    assert_eq!((m.get(2, 0), m.get(0, 1)), (one.argb(), two.argb()));
    assert_eq!(m.get(0, 0), 0, "the rest moves too");
    assert_eq!(m.mirrored(), c);
}

#[test]
fn rows_flipped_land_drawing_upside_down_and_keep_what_lay_under() {
    let (one, two) = (Color::rgb(0, 0, 1), Color::rgb(0, 0, 2));
    let (under, mark) = (Color::rgb(0, 0, 3), Color::rgb(0, 0, 4));
    let mut c = Canvas::with_scale(4, 6, 2);
    c.fill_rect(c.bounds(), under);
    c.put(1, 2, mark);
    let r = Rect::new(1, 1, 2, 4);
    c.with_rows_flipped(r, |c| {
        c.fill_rect(Rect::new(1, 1, 2, 1), one);
        c.put(2, 2, two);
    });
    assert_eq!(
        (c.get(1, 4), c.get(2, 4)),
        (one.argb(), one.argb()),
        "r's first row lands on its last"
    );
    assert_eq!(c.get(2, 3), two.argb(), "its second row on its second last");
    assert_eq!(c.get(1, 2), mark.argb(), "what lay under keeps its place");
    assert_eq!(c.get(1, 1), under.argb());
    assert_eq!(c.get(0, 4), under.argb(), "outside r nothing moves");
    // At scale 2 the flip is by device row: both of (2, 3)'s rows.
    let d = |x: usize, y: usize| c.pixels[y * 8 + x];
    assert_eq!(
        (d(4, 6), d(4, 7), d(5, 7)),
        (two.argb(), two.argb(), two.argb())
    );
}

#[test]
fn blit_over_faded_matches_a_faded_copy_blitted_over() {
    let mut src = Canvas::new(8, 4);
    for (i, p) in src.pixels.iter_mut().enumerate() {
        let a = (i as u32 * 37) & 0xFF;
        *p = (a << 24) | ((a * 3 / 4) << 16) | ((a / 2) << 8) | (a / 5);
    }
    let mut dst = Canvas::new(10, 10);
    dst.fill_rect(dst.bounds(), Color::rgb(200, 120, 40));
    let mut want = dst.clone();
    let mut faded = src.clone();
    faded.fade(1, 2);
    want.blit_over(1, 3, &faded);
    dst.blit_over_faded(1, 3, &src, 1, 2);
    assert_eq!(
        dst.pixels, want.pixels,
        "faded blit equals fade then blit_over"
    );
}

#[test]
fn over_px_of_an_opaque_or_clear_source_or_onto_a_clear_pixel_is_the_blend() {
    let blend = |s: u32, d: u32| {
        let sa = s >> 24;
        let ch = |k: u32| ((s >> k) & 0xFF) + ((d >> k) & 0xFF) * (255 - sa) / 255;
        ch(24) << 24 | ch(16) << 16 | ch(8) << 8 | ch(0)
    };
    let ds = [0, 0x8040_2010, 0xFF12_3456, 0x0100_0000];
    for s in [
        0xFFAB_CDEF,
        0xFF00_0000,
        0x8040_2010,
        0x0101_0101,
        0,
        0x0010_2030,
    ] {
        for d in ds {
            assert_eq!(over_px(s, d), blend(s, d), "{s:08x} over {d:08x}");
        }
    }
}

#[test]
fn blit_over_clipped_touches_only_the_clip() {
    let mut src = Canvas::new(6, 6);
    src.fill_rect(src.bounds(), Color::rgb(200, 10, 20));
    let mut full = Canvas::new(10, 10);
    full.blit_over(2, 2, &src);
    let clip = Rect::new(3, 4, 3, 2);
    let mut clipped = Canvas::new(10, 10);
    clipped.blit_over_clipped(2, 2, &src, clip);
    for y in 0..10 {
        for x in 0..10 {
            let want = if clip.contains(x, y) {
                full.get(x, y)
            } else {
                0
            };
            assert_eq!(clipped.get(x, y), want, "pixel ({x}, {y})");
        }
    }
}

#[test]
fn canvas_blit_over_composes_alpha() {
    let mut src = Canvas::new(1, 1);
    src.pixels[0] = 0x8080_8080;
    let mut transparent = Canvas::new(1, 1);
    transparent.blit_over(0, 0, &src);
    assert_eq!(transparent.get(0, 0), 0x8080_8080);
    let mut opaque = Canvas::new(1, 1);
    opaque.put(0, 0, Color::rgb(0, 0, 0));
    opaque.blit_over(0, 0, &src);
    assert_eq!(opaque.get(0, 0), 0xFF80_8080);
    let mut untouched = Canvas::new(1, 1);
    untouched.put(0, 0, Color::rgb(1, 2, 3));
    untouched.blit_over(0, 0, &Canvas::new(1, 1));
    assert_eq!(untouched.get(0, 0), Color::rgb(1, 2, 3).argb());
    let mut small = Canvas::new(1, 1);
    small.blit_over(5, 5, &src);
}

#[test]
fn rect_helpers() {
    let r = Rect::new(2, 3, 10, 5);
    assert!(r.contains(2, 3));
    assert!(r.contains(11, 7));
    assert!(!r.contains(12, 7));
    assert_eq!(r.inset(1), Rect::new(3, 4, 8, 3));
    assert_eq!(r.intersect(&Rect::new(0, 0, 5, 5)), Rect::new(2, 3, 3, 2));
}

#[test]
fn rect_scaled_multiplies_all_four() {
    assert_eq!(Rect::new(2, 3, 10, 5).scaled(2), Rect::new(4, 6, 20, 10));
    assert_eq!(Rect::new(2, 3, 10, 5).scaled(0), Rect::new(2, 3, 10, 5));
}

#[test]
fn resample_box_averages_checkerboard_to_grey() {
    let white = 0xFFFF_FFFFu32;
    let black = 0xFF00_0000u32;
    let mut c = Canvas::new(4, 4);
    for y in 0..4 {
        for x in 0..4 {
            c.pixels[(y * 4 + x) as usize] = if (x + y) % 2 == 0 { white } else { black };
        }
    }
    let d = c.resample_box(2, 2);
    assert_eq!(d.width, 2);
    assert_eq!(d.height, 2);
    assert!(
        d.pixels.iter().all(|&p| p == 0xFF80_8080),
        "{:x?}",
        d.pixels
    );
}

#[test]
fn resample_box_upscale_is_nearest() {
    let mut c = Canvas::new(2, 1);
    c.pixels[0] = 0xFF11_2233;
    c.pixels[1] = 0xFF44_5566;
    let d = c.resample_box(4, 2);
    assert_eq!(
        d.pixels,
        vec![
            0xFF11_2233,
            0xFF11_2233,
            0xFF44_5566,
            0xFF44_5566,
            0xFF11_2233,
            0xFF11_2233,
            0xFF44_5566,
            0xFF44_5566,
        ]
    );
}

#[test]
fn argb_be_is_big_endian_straight_alpha() {
    let c = Canvas::from_pixels(3, 1, vec![0xFF60_6060, 0, 0x8040_2000]);
    assert_eq!(
        c.to_argb_be(),
        vec![0xFF, 0x60, 0x60, 0x60, 0, 0, 0, 0, 0x80, 0x80, 0x40, 0]
    );
}

#[test]
fn resample_box_keeps_premultiplied_alpha_consistent() {
    let mut c = Canvas::new(2, 1);
    c.pixels[0] = 0x8080_0000;
    c.pixels[1] = 0;
    let d = c.resample_box(1, 1);
    assert_eq!(d.pixels[0], 0x4040_0000);
    assert_eq!(c.resample_box(2, 1), c);
    assert_eq!(Canvas::new(0, 0).resample_box(2, 2).pixels, vec![0; 4]);
}

#[test]
fn thick_edge_rings_and_etched_lines() {
    let cs = ColorScheme::default();
    let mut c = Canvas::new(12, 12);
    c.edge_t(Rect::new(0, 0, 12, 12), EdgeStyle::ButtonRaised, &cs, 2);
    // outer ring 2 px hilight, inner ring 2 px light, on the left column
    assert_eq!(col(&c, 0, 4, 6), vec![cs.button_hilight.argb(); 2]);
    assert_eq!(col(&c, 1, 4, 6), vec![cs.button_hilight.argb(); 2]);
    assert_eq!(c.get(2, 6), cs.button_light.argb());
    assert_eq!(c.get(3, 6), cs.button_light.argb());
    assert_eq!(c.get(4, 6), 0);
    assert_eq!(c.get(8, 6), cs.button_shadow.argb());
    assert_eq!(c.get(9, 6), cs.button_shadow.argb());
    assert_eq!(c.get(10, 6), cs.button_dk_shadow.argb());
    assert_eq!(c.get(11, 6), cs.button_dk_shadow.argb());
    let mut c = Canvas::new(8, 8);
    c.etched_hline_t(0, 8, 2, &cs, 2);
    assert_eq!(
        col(&c, 3, 2, 6),
        vec![
            cs.button_shadow.argb(),
            cs.button_shadow.argb(),
            cs.button_hilight.argb(),
            cs.button_hilight.argb()
        ]
    );
    let mut c = Canvas::new(8, 8);
    c.etched_vline_t(2, 0, 8, &cs, 2);
    let row: Vec<u32> = (2..6).map(|x| c.get(x, 3)).collect();
    assert_eq!(
        row,
        vec![
            cs.button_shadow.argb(),
            cs.button_shadow.argb(),
            cs.button_hilight.argb(),
            cs.button_hilight.argb()
        ]
    );
}

#[test]
fn dither_cells_follow_thickness() {
    let a = Color::rgb(10, 10, 10);
    let b = Color::rgb(20, 20, 20);
    let mut c = Canvas::new(4, 4);
    c.dither_rect_t(Rect::new(0, 0, 4, 4), a, b, 2);
    assert_eq!(
        col(&c, 0, 0, 4),
        vec![a.argb(), a.argb(), b.argb(), b.argb()]
    );
    assert_eq!(
        col(&c, 2, 0, 4),
        vec![b.argb(), b.argb(), a.argb(), a.argb()]
    );
    // t = 1 is the 1 px checkerboard: colours alternate every pixel
    let mut c = Canvas::new(4, 2);
    c.dither_rect_t(Rect::new(0, 0, 4, 2), a, b, 1);
    assert_eq!(col(&c, 0, 0, 2), vec![a.argb(), b.argb()]);
}

#[test]
fn rounded_shapes_clip_to_the_canvas() {
    let red = Color::rgb(200, 0, 0);
    for r in [Rect::new(-3, -2, 14, 11), Rect::new(2, 1, 12, 9)] {
        let mut fill = Canvas::new(10, 8);
        rounded_fill(&mut fill, r, 4, red);
        let mut border = Canvas::new(10, 8);
        rounded_border(&mut border, r, 4, red, 2);
        let inner = Rect::new(r.x + 2, r.y + 2, r.width - 4, r.height - 4);
        for y in 0..8 {
            for x in 0..10 {
                let inf = in_rounded(r, 4, x, y);
                let inb = inf && !in_rounded(inner, 2, x, y);
                let want = |b: bool| if b { red.argb() } else { 0 };
                assert_eq!(fill.get(x, y), want(inf), "fill {r:?} ({x}, {y})");
                assert_eq!(border.get(x, y), want(inb), "border {r:?} ({x}, {y})");
            }
        }
    }
}

#[test]
fn rotate_ccw_turns_left_edge_into_bottom() {
    let mut c = Canvas::new(3, 2);
    c.put(0, 0, Color::rgb(1, 0, 0)); // top-left
    c.put(2, 1, Color::rgb(0, 1, 0)); // bottom-right
    let r = c.rotate_ccw();
    assert_eq!((r.width, r.height), (2, 3));
    assert_eq!(r.get(0, 2), Color::rgb(1, 0, 0).argb());
    assert_eq!(r.get(1, 0), Color::rgb(0, 1, 0).argb());
}

/// Device rows of a canvas as 0/1 per pixel against `ink`.
fn device_rows(c: &Canvas, ink: u32) -> Vec<String> {
    let (w, _) = c.device_size();
    c.pixels
        .chunks(w as usize)
        .map(|row| {
            row.iter()
                .map(|&p| if p == ink { '1' } else { '0' })
                .collect()
        })
        .collect()
}

#[test]
fn a_fill_at_scale_2_covers_the_device_block_of_each_logical_pixel() {
    let ink = Color::rgb(9, 9, 9);
    let mut c = Canvas::with_scale(3, 2, 2);
    assert_eq!(c.device_size(), (6, 4));
    assert_eq!(c.bounds(), Rect::new(0, 0, 3, 2));
    c.fill_rect(Rect::new(1, 0, 1, 1), ink);
    c.put(2, 1, ink);
    assert_eq!(
        device_rows(&c, ink.argb()),
        ["001100", "001100", "000011", "000011"]
    );
    assert_eq!(c.get(1, 0), ink.argb());
    assert_eq!(c.get(0, 0), 0);
}

#[test]
fn a_clip_at_scale_2_limits_drawing_to_its_device_pixels() {
    let ink = Color::rgb(9, 9, 9);
    let mut c = Canvas::with_scale(3, 1, 2);
    c.with_clip(Rect::new(2, 0, 1, 1), |c| c.fill_rect(c.bounds(), ink));
    assert_eq!(device_rows(&c, ink.argb()), ["000011", "000011"]);
}

#[test]
fn a_scale_1_blit_onto_a_scale_2_canvas_doubles_each_pixel() {
    let (a, b) = (Color::rgb(1, 0, 0).argb(), Color::rgb(0, 1, 0).argb());
    let src = Canvas::from_pixels(2, 1, vec![a, b]);
    let mut c = Canvas::with_scale(4, 1, 2);
    c.blit_over(1, 0, &src);
    let row: Vec<u32> = c.pixels[..8].to_vec();
    assert_eq!(row, [0, 0, a, a, b, b, 0, 0]);
    assert_eq!(c.pixels[8..], c.pixels[..8]);
}

#[test]
fn a_blit_at_the_same_scale_copies_device_pixels() {
    let px = [0xFF00_0001, 0xFF00_0002, 0xFF00_0003, 0xFF00_0004];
    let src = Canvas::from_pixels(2, 2, px.to_vec()).at_scale(2);
    assert_eq!((src.width, src.height, src.scale()), (1, 1, 2));
    let mut c = Canvas::with_scale(2, 2, 2);
    c.blit_canvas(1, 1, &src);
    let at = |x: usize, y: usize| c.pixels[y * 4 + x];
    assert_eq!([at(2, 2), at(3, 2), at(2, 3), at(3, 3)], px);
    assert_eq!(at(1, 2), 0);
}

#[test]
fn resample_and_rotate_keep_the_scale() {
    let c = Canvas::with_scale(4, 2, 2);
    let r = c.resample_box(2, 2);
    assert_eq!(
        (r.width, r.height, r.scale(), r.device_size()),
        (2, 2, 2, (4, 4))
    );
    let t = c.rotate_ccw();
    assert_eq!(
        (t.width, t.height, t.scale(), t.device_size()),
        (2, 4, 2, (4, 8))
    );
}

#[test]
fn at_scale_resamples_a_side_that_does_not_divide() {
    let c = Canvas::new(5, 4).at_scale(2);
    assert_eq!((c.width, c.height, c.device_size()), (2, 2, (4, 4)));
    assert_eq!(c.pixels.len(), 16);
}

#[test]
fn a_png_of_a_scaled_canvas_has_its_device_size() {
    let png = to_png(&Canvas::with_scale(3, 2, 2));
    let dec = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .unwrap();
    assert_eq!((dec.info().width, dec.info().height), (6, 4));
}

fn nine_atlas() -> Canvas {
    let mut a = Canvas::new(3, 3);
    let ink = |i: u8| Color::rgb(i * 20, 0, 0);
    for y in 0..3 {
        for x in 0..3 {
            a.fill_rect(Rect::new(x, y, 1, 1), ink((y * 3 + x) as u8 + 1));
        }
    }
    a
}

#[test]
fn nine_grid_keeps_corners_and_stretches_edges() {
    let a = nine_atlas();
    let mut c = Canvas::new(5, 4);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 3, 3),
        Rect::new(0, 0, 5, 4),
        Insets::new(1, 1, 1, 1),
        Fill::Stretch,
        false,
        1,
    );
    let red = |x, y| (c.get(x, y) >> 16) & 0xFF;
    // Corners 1, 3, 7, 9; top edge 2 repeated; centre 5 over 3x2.
    assert_eq!(
        [red(0, 0), red(4, 0), red(0, 3), red(4, 3)],
        [20, 60, 140, 180]
    );
    assert_eq!([red(1, 0), red(2, 0), red(3, 0)], [40, 40, 40]);
    assert_eq!([red(1, 1), red(3, 2)], [100, 100]);
}

#[test]
fn nine_grid_tiles_the_middle() {
    // Middle columns alternate 10 and 30 between 1 px margins.
    let mut a = Canvas::new(4, 1);
    for (x, r) in [(0, 5), (1, 10), (2, 30), (3, 50)] {
        a.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(r, 0, 0));
    }
    let mut c = Canvas::new(7, 1);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 4, 1),
        Rect::new(0, 0, 7, 1),
        Insets::new(1, 0, 1, 0),
        Fill::Tile,
        false,
        1,
    );
    let row: Vec<u32> = (0..7).map(|x| (c.get(x, 0) >> 16) & 0xFF).collect();
    assert_eq!(row, vec![5, 10, 30, 10, 30, 10, 50]);
}

#[test]
fn a_nine_grid_tiled_across_stretches_down() {
    // Middle columns alternate 10 and 30, middle rows 1 then 2
    // (green), between 1 px margins of 5 and 50.
    let mut a = Canvas::new(4, 4);
    for (x, r) in [(0, 5), (1, 10), (2, 30), (3, 50)] {
        for (y, g) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
            a.fill_rect(Rect::new(x, y, 1, 1), Color::rgb(r, g, 0));
        }
    }
    let mut c = Canvas::new(7, 6);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 4, 4),
        Rect::new(0, 0, 7, 6),
        Insets::new(1, 1, 1, 1),
        Fill::TileAcross,
        false,
        1,
    );
    let row: Vec<u32> = (0..7).map(|x| (c.get(x, 2) >> 16) & 0xFF).collect();
    assert_eq!(row, vec![5, 10, 30, 10, 30, 10, 50], "tiled across");
    let col: Vec<u32> = (0..6).map(|y| (c.get(2, y) >> 8) & 0xFF).collect();
    assert_eq!(col, vec![0, 1, 1, 2, 2, 3], "stretched down");
    for y in 0..6 {
        for x in 0..7 {
            let (sx, sy) = nine_source(
                Rect::new(0, 0, 4, 4),
                Rect::new(0, 0, 7, 6),
                Insets::new(1, 1, 1, 1),
                Fill::TileAcross,
                false,
                1,
                x,
                y,
            )
            .expect("inside");
            assert_eq!(
                c.get(x, y),
                a.get(sx, sy),
                "nine_source agrees at ({x}, {y})"
            );
        }
    }
}

#[test]
fn a_centred_stretch_puts_an_edge_where_it_scales_to() {
    // Two columns, 10 then 90, stretched to five: the edge scales to
    // 2.5, so the third pixel's centre is already past it.
    let mut a = Canvas::new(2, 1);
    a.fill_rect(Rect::new(0, 0, 1, 1), Color::rgb(10, 0, 0));
    a.fill_rect(Rect::new(1, 0, 1, 1), Color::rgb(90, 0, 0));
    let row = |fill| {
        let mut c = Canvas::new(5, 1);
        c.blit_nine(
            &a,
            Rect::new(0, 0, 2, 1),
            Rect::new(0, 0, 5, 1),
            Insets::ZERO,
            fill,
            false,
            1,
        );
        (0..5)
            .map(|x| (c.get(x, 0) >> 16) & 0xFF)
            .collect::<Vec<u32>>()
    };
    assert_eq!(row(Fill::StretchCentred), vec![10, 10, 90, 90, 90]);
    assert_eq!(row(Fill::Stretch), vec![10, 10, 10, 90, 90]);
}

#[test]
fn a_clipped_nine_grid_equals_the_full_one_inside_the_clip() {
    // A 9x9 atlas, one colour per pixel, tiled or stretched into a frame
    // running past the canvas on the left and bottom.
    let mut a = Canvas::new(9, 9);
    for y in 0..9 {
        for x in 0..9 {
            a.fill_rect(
                Rect::new(x, y, 1, 1),
                Color::rgb(x as u8 * 25, y as u8 * 25, 7),
            );
        }
    }
    let to = Rect::new(-3, 2, 40, 31);
    for (fill, scale) in [Fill::Tile, Fill::Stretch]
        .into_iter()
        .flat_map(|f| [(f, 1), (f, 2)])
    {
        let draw = |c: &mut Canvas| {
            c.blit_nine(
                &a,
                Rect::new(0, 0, 9, 9),
                to,
                Insets::new(2, 3, 2, 3),
                fill,
                false,
                1,
            )
        };
        let mut full = Canvas::with_scale(30, 30, scale);
        draw(&mut full);
        for clip in [
            Rect::new(0, 0, 30, 30),
            Rect::new(5, 7, 11, 2),
            Rect::new(0, 28, 30, 5),
            Rect::new(1, 3, 1, 1),
            Rect::new(40, 40, 5, 5),
        ] {
            let mut part = Canvas::with_scale(30, 30, scale);
            part.with_clip(clip, draw);
            for y in 0..30 {
                for x in 0..30 {
                    let want = if clip.contains(x, y) {
                        full.get(x, y)
                    } else {
                        0
                    };
                    assert_eq!(
                        part.get(x, y),
                        want,
                        "({x}, {y}) at {scale}x, {fill:?}, {clip:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn nine_grid_border_only_leaves_the_centre_alone() {
    let a = nine_atlas();
    let mut c = Canvas::new(5, 5);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 3, 3),
        Rect::new(0, 0, 5, 5),
        Insets::new(1, 1, 1, 1),
        Fill::Stretch,
        true,
        1,
    );
    assert_eq!(c.get(2, 2), 0);
    assert_ne!(c.get(0, 2), 0);
}

#[test]
fn nine_grid_shrinks_margins_into_a_small_rect() {
    // Margins of 2 + 2 into 2 px: one column of each side's corner.
    let mut a = Canvas::new(5, 1);
    for x in 0..5 {
        a.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
    }
    let mut c = Canvas::new(2, 1);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 5, 1),
        Rect::new(0, 0, 2, 1),
        Insets::new(2, 0, 2, 0),
        Fill::Stretch,
        false,
        1,
    );
    assert_eq!(
        [(c.get(0, 0) >> 16) & 0xFF, (c.get(1, 0) >> 16) & 0xFF],
        [10, 40]
    );
    c.blit_nine(
        &a,
        Rect::new(0, 0, 5, 1),
        Rect::new(0, 0, 0, 0),
        Insets::new(2, 0, 2, 0),
        Fill::Stretch,
        false,
        1,
    );
}

#[test]
fn transparent_atlas_pixels_stay_transparent() {
    let mut a = Canvas::new(3, 1);
    a.fill_rect(Rect::new(1, 0, 2, 1), Color::rgb(0, 0, 255));
    let mut c = Canvas::new(6, 1);
    c.blit_nine(
        &a,
        Rect::new(0, 0, 3, 1),
        Rect::new(0, 0, 6, 1),
        Insets::new(1, 0, 1, 0),
        Fill::Stretch,
        false,
        1,
    );
    assert_eq!(c.get(0, 0), 0);
    assert_eq!(c.get(1, 0), 0xFF00_00FF);
}

#[test]
fn row_and_column_lists_repeat_their_last_entry() {
    let mut c = Canvas::new(3, 4);
    c.fill_rows(
        Rect::new(0, 0, 3, 4),
        &[Color::rgb(1, 0, 0), Color::rgb(2, 0, 0)],
    );
    let red = |c: &Canvas, x, y| (c.get(x, y) >> 16) & 0xFF;
    assert_eq!([red(&c, 1, 0), red(&c, 1, 1), red(&c, 1, 3)], [1, 2, 2]);
    c.fill_cols(Rect::new(0, 0, 3, 4), &[Color::rgb(7, 0, 0)]);
    assert_eq!(red(&c, 2, 3), 7);
    c.fill_rows(Rect::new(0, 0, 3, 4), &[]);
    assert_eq!(red(&c, 2, 3), 7);
}

#[test]
fn a_nine_axis_magnified_k_times_doubles_the_scale_1_axis() {
    // Every fill, source length, margin pair and target length at k times
    // magnification: pixel d shows what pixel d / k shows at scale 1.
    for fill in [Fill::Stretch, Fill::Tile, Fill::StretchCentred] {
        for src_len in 1..8 {
            for a in 0..=src_len {
                for b in 0..=src_len - a {
                    for len in 1..20 {
                        for k in 2..=3 {
                            for d in 0..len * k {
                                let want = nine_axis(d / k, len, 3, src_len, a, b, fill, 1).0;
                                let got = nine_axis(
                                    d,
                                    len * k,
                                    3 * k,
                                    src_len * k,
                                    a * k,
                                    b * k,
                                    fill,
                                    k,
                                )
                                .0
                                .div_euclid(k);
                                assert_eq!(
                                    got, want,
                                    "{fill:?} k {k}, source {src_len} ({a}, {b}), \
                                     length {len}, pixel {d}: source {want}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn margins_larger_than_the_frame_never_read_outside_it() {
    let mut atlas = Canvas::new(1, 4);
    atlas.fill_rect(Rect::new(0, 0, 1, 2), Color::rgb(20, 0, 0));
    atlas.fill_rect(Rect::new(0, 2, 1, 2), Color::rgb(30, 0, 0));

    let mut c = Canvas::new(1, 3);
    c.blit_nine(
        &atlas,
        Rect::new(0, 2, 1, 2),
        Rect::new(0, 0, 1, 3),
        Insets::new(0, 3, 0, 3),
        Fill::Stretch,
        false,
        1,
    );

    let red = |y| (c.get(0, y) >> 16) & 0xFF;
    assert_eq!(red(0), 30, "target pixel 0 is from second frame");
    assert_eq!(red(1), 30, "target pixel 1 is from second frame");
    assert_eq!(red(2), 30, "target pixel 2 is from second frame");
}

#[test]
fn blit_nine_draws_what_nine_source_gives_each_pixel() {
    // A 7x6 atlas of opaque, half and clear pixels over a patterned canvas:
    // blit_nine must equal composing the per-pixel `nine_source` build.
    let mut atlas = Canvas::new(7, 6);
    for y in 0..6 {
        for x in 0..7 {
            let i = (y * 7 + x) as u32;
            let a = [0xFF, 0x80, 0xFF, 0x00][(i % 4) as usize];
            let c = |n: u32| (n * 37 + i * 11) % (a + 1);
            atlas.put_raw(x, y, (a << 24) | (c(1) << 16) | (c(2) << 8) | c(3));
        }
    }
    let backdrop = |scale| {
        let mut c = Canvas::with_scale(12, 10, scale);
        for y in 0..10 {
            for x in 0..12 {
                c.put_raw(x, y, 0xFF00_0000 | (x as u32 * 20) << 8 | (y as u32 * 25));
            }
        }
        c
    };
    let froms = [
        Rect::new(0, 0, 7, 6),
        Rect::new(2, 1, 4, 5),
        Rect::new(1, 2, 1, 1),
    ];
    let margins = [
        Insets::ZERO,
        Insets::new(1, 1, 1, 1),
        Insets::new(2, 1, 3, 2),
        Insets::new(4, 4, 4, 4),
    ];
    let tos = [
        Rect::new(0, 0, 12, 10),
        Rect::new(2, 3, 5, 4),
        Rect::new(-3, -2, 9, 7),
        Rect::new(8, 6, 9, 8),
        Rect::new(1, 1, 2, 1),
        Rect::new(-2, 4, 20, 3),
    ];
    for fill in [Fill::Stretch, Fill::Tile, Fill::StretchCentred] {
        for border_only in [false, true] {
            for k in 1..=3 {
                for scale in [1, 2] {
                    for from in froms {
                        for m in margins {
                            for to in tos {
                                let (from_k, m_k) = (from.scaled(k), m.scaled(k));
                                let mut got = backdrop(scale);
                                got.blit_nine(&atlas, from_k, to, m_k, fill, border_only, k);
                                let mut out = Canvas::new(12, 10);
                                for y in 0..10 {
                                    for x in 0..12 {
                                        if let Some((sx, sy)) =
                                            nine_source(from_k, to, m_k, fill, border_only, k, x, y)
                                        {
                                            out.put_raw(x, y, atlas.get(sx, sy));
                                        }
                                    }
                                }
                                let mut want = backdrop(scale);
                                want.blit_over(0, 0, &out);
                                assert_eq!(
                                    got.pixels, want.pixels,
                                    "{fill:?}, border only {border_only}, k {k}, \
                                     scale {scale}, {from:?} {m:?} into {to:?}: \
                                     the per-pixel nine_source result"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn a_transposed_picture_lies_on_its_side_its_first_row_down_the_inner_column() {
    let (red, green, blue) = (
        Color::rgb(255, 0, 0),
        Color::rgb(0, 255, 0),
        Color::rgb(0, 0, 255),
    );
    let (a, b, c) = (red.argb(), green.argb(), blue.argb());
    // A 3x2 picture: row 0 is a, a, b; row 1 is c, c, c.
    let draw = |p: &mut Canvas| {
        p.fill_rect(Rect::new(0, 0, 3, 1), red);
        p.put(2, 0, green);
        p.fill_rect(Rect::new(0, 1, 3, 1), blue);
    };
    let mut right = Canvas::new(4, 5);
    right.with_transposed(Rect::new(1, 1, 2, 3), false, draw);
    assert_eq!(
        [right.get(1, 1), right.get(1, 2), right.get(1, 3)],
        [a, a, b],
        "row 0 down r's first column"
    );
    assert_eq!(
        [right.get(2, 1), right.get(2, 2), right.get(2, 3)],
        [c, c, c]
    );
    assert_eq!(
        (right.get(0, 1), right.get(1, 4)),
        (0, 0),
        "nothing outside r"
    );
    let mut left = Canvas::new(4, 5);
    left.with_transposed(Rect::new(1, 1, 2, 3), true, draw);
    assert_eq!(
        [left.get(2, 1), left.get(2, 2), left.get(2, 3)],
        [a, a, b],
        "mirrored, row 0 down r's last column"
    );
    assert_eq!([left.get(1, 1), left.get(1, 2), left.get(1, 3)], [c, c, c]);
    // At scale 2 each logical pixel turns as a whole 2x2 block.
    let mut two = Canvas::with_scale(4, 5, 2);
    two.with_transposed(Rect::new(1, 1, 2, 3), false, draw);
    assert_eq!(two.get(1, 3), b);
    assert_eq!(two.pixels[7 * 8 + 3], b, "the block's far device pixel");
    // It lies over what is there: a clear pixel of the picture leaves
    // the canvas's own.
    let grey = Color::rgb(9, 9, 9);
    let mut under = Canvas::new(4, 5);
    under.fill_rect(Rect::new(0, 0, 4, 5), grey);
    under.with_transposed(Rect::new(1, 1, 2, 3), false, |p| p.put(0, 0, red));
    assert_eq!((under.get(1, 1), under.get(2, 2)), (a, grey.argb()));
}

#[test]
fn a_transposed_picture_blends_clips_and_mirrors_at_scale_2() {
    let (red, green, blue) = (
        Color::rgb(255, 0, 0),
        Color::rgb(0, 255, 0),
        Color::rgb(0, 0, 255),
    );
    let grey = Color::rgb(9, 9, 9);
    let mut under = Canvas::new(4, 5);
    under.fill_rect(Rect::new(0, 0, 4, 5), grey);
    under.with_transposed(Rect::new(1, 1, 2, 3), false, |p| {
        p.put_raw(1, 0, 0x8080_0000)
    });
    assert_eq!(under.get(1, 2), 0xFF84_0404, "half red over grey");
    // A rect running off the canvas's foot, under a clip cutting its
    // second column: only the part on the canvas and in the clip.
    let mut cut = Canvas::new(4, 5);
    cut.with_clip(Rect::new(0, 0, 2, 5), |c| {
        c.with_transposed(Rect::new(1, 3, 2, 3), false, |p| {
            p.fill_rect(Rect::new(0, 0, 3, 2), red)
        })
    });
    assert_eq!([cut.get(1, 3), cut.get(1, 4)], [red.argb(); 2]);
    assert_eq!([cut.get(2, 3), cut.get(2, 4)], [0, 0], "past the clip");
    let draw = |p: &mut Canvas| {
        p.fill_rect(Rect::new(0, 0, 3, 1), red);
        p.put(2, 0, green);
        p.fill_rect(Rect::new(0, 1, 3, 1), blue);
    };
    let mut two = Canvas::with_scale(4, 5, 2);
    two.with_transposed(Rect::new(1, 1, 2, 3), true, draw);
    assert_eq!(
        [two.get(2, 1), two.get(2, 2), two.get(2, 3)],
        [red.argb(), red.argb(), green.argb()]
    );
    assert_eq!([two.get(1, 1), two.get(1, 3)], [blue.argb(), blue.argb()]);
    assert_eq!(
        two.pixels[7 * 8 + 5],
        green.argb(),
        "the block's far device pixel"
    );
}
