use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;
use crate::theme::Theme;

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |b: &[u8]| {
        for &b in b {
            h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&c.width.to_le_bytes());
    eat(&c.height.to_le_bytes());
    for p in &c.pixels {
        eat(&p.to_le_bytes());
    }
    h
}

/// `w` x `h` with each pixel's red its x and green its y, so a read names where it came from.
fn coded(w: u32, h: u32) -> Canvas {
    let px = (0..w * h)
        .map(|i| 0xFF00_0000 | (i % w) << 16 | (i / w) << 8)
        .collect();
    Canvas::from_pixels(w, h, px)
}

fn at(c: &Canvas, x: u32, y: u32) -> (u32, u32) {
    let p = c.pixels[(y * c.device_size().0 + x) as usize];
    ((p >> 16) & 0xFF, (p >> 8) & 0xFF)
}

fn cm() -> ControlMetrics {
    Theme::default().controls()
}

#[test]
fn fit_shrinks_a_large_picture_to_the_box_and_leaves_a_small_one_at_its_size() {
    assert_eq!(fit_scale((4000, 3000), (400, 400)), 0.1);
    assert_eq!(fit_scale((3000, 4000), (400, 400)), 0.1);
    assert_eq!(fit_scale((100, 50), (400, 400)), 1.0);
    assert_eq!(fit_scale((0, 50), (400, 400)), 1.0);
    let v = ImageView::new((4000, 3000));
    let lay = layout(&v, Rect::new(0, 20, 400, 300), 1, &cm());
    assert_eq!(lay.content, (400, 300));
    assert_eq!((lay.vscroll, lay.hscroll), (None, None));
    assert_eq!(v.origin(&lay), (0, 20));
}

#[test]
fn a_fitted_picture_is_centred_and_at_scale_2_fits_the_device_box() {
    let v = ImageView::new((800, 200));
    let lay = layout(&v, Rect::new(0, 0, 200, 200), 2, &cm());
    assert_eq!(lay.scale, 0.5);
    assert_eq!(lay.content, (400, 100));
    assert_eq!(v.origin(&lay), (0, 150));
    assert_eq!(v.picture_rect(&lay), Rect::new(0, 75, 200, 50));
}

#[test]
fn zoom_steps_walk_the_ladder_and_stop_at_its_ends() {
    assert_eq!(step(1.0, true), 1.5);
    assert_eq!(step(1.0, false), 0.67);
    assert_eq!(step(0.4, true), 0.5);
    assert_eq!(step(0.4, false), 0.33);
    assert_eq!(step(32.0, true), 32.0);
    assert_eq!(step(0.01, false), 0.01);
}

#[test]
fn each_turn_reads_the_pixel_that_lands_there() {
    // A 3x2 picture; the top-left pixel shown after each clockwise turn.
    let size = (3, 2);
    assert_eq!(source_of(0, 0, size, 0), (0, 0));
    assert_eq!(
        source_of(0, 0, size, 1),
        (0, 1),
        "the bottom-left comes to the top"
    );
    assert_eq!(source_of(0, 0, size, 2), (2, 1));
    assert_eq!(source_of(0, 0, size, 3), (2, 0));
    assert_eq!(
        source_of(1, 2, size, 1),
        (2, 0),
        "the shown 2x3's bottom-right"
    );
    assert_eq!(turned(size, 1), (2, 3));
    assert_eq!(turned(size, 2), (3, 2));
}

#[test]
fn render_turns_and_scales_the_picture() {
    let src = coded(4, 2);
    let r = render(&src, 1, (2, 4));
    assert_eq!((r.width, r.height), (2, 4));
    assert_eq!(at(&r, 0, 0), (0, 1));
    assert_eq!(at(&r, 1, 0), (0, 0));
    assert_eq!(at(&r, 1, 3), (3, 0));
    let half = render(&coded(4, 4), 0, (2, 2));
    assert_eq!(
        at(&half, 1, 1),
        (3, 3),
        "the 2x2 box (2..4) averages to 2.5, rounded up"
    );
}

#[test]
fn a_picture_past_the_box_gets_both_bars_and_pans_within_its_size() {
    let mut v = ImageView::new((1000, 600));
    v.zoom = Zoom::Scale(1.0);
    let m = cm();
    let lay = layout(&v, Rect::new(0, 0, 300, 200), 1, &m);
    assert_eq!(lay.view, Rect::new(0, 0, 300 - m.scroll, 200 - m.scroll));
    assert!(lay.vscroll.is_some() && lay.hscroll.is_some());
    assert_eq!(lay.range(), (1000 - 300 + m.scroll, 600 - 200 + m.scroll));
    assert!(v.pan(5000, -50, &lay));
    assert_eq!((v.left, v.top), (lay.range().0, 0));
    assert!(!v.pan(10, 0, &lay), "held at the right edge");
}

#[test]
fn a_wide_picture_gets_only_the_bar_along_the_bottom() {
    let mut v = ImageView::new((1000, 50));
    v.zoom = Zoom::Scale(1.0);
    let lay = layout(&v, Rect::new(0, 0, 300, 200), 1, &cm());
    assert!(lay.hscroll.is_some());
    assert_eq!(lay.vscroll, None);
    let (_, y) = v.origin(&lay);
    assert_eq!(y, (lay.view.height - 50) / 2, "centred down the box");
}

#[test]
fn zooming_at_a_point_keeps_the_picture_under_it() {
    let mut v = ImageView::new((1000, 1000));
    let r = Rect::new(0, 0, 400, 400);
    let lay = layout(&v, r, 1, &cm());
    assert_eq!(lay.scale, 0.4);
    // (100, 100) shows picture px (250, 250).
    v.zoom_to(1.0, Some((100, 100)), &lay);
    let lay = layout(&v, r, 1, &cm());
    let (ox, oy) = v.origin(&lay);
    assert_eq!((100 - ox, 100 - oy), (250, 250));
    v.zoom_step(true, Some((100, 100)), &lay);
    assert_eq!(v.zoom, Zoom::Scale(1.5));
    let lay = layout(&v, r, 1, &cm());
    let (ox, oy) = v.origin(&lay);
    assert_eq!((100 - ox, 100 - oy), (375, 375));
}

#[test]
fn a_zoom_out_past_the_box_centres_the_picture_again() {
    let mut v = ImageView::new((1000, 1000));
    v.zoom = Zoom::Scale(1.0);
    let r = Rect::new(0, 0, 400, 400);
    let lay = layout(&v, r, 1, &cm());
    v.pan(300, 300, &lay);
    v.zoom_to(0.25, Some((10, 10)), &lay);
    let lay = layout(&v, r, 1, &cm());
    assert_eq!((v.left, v.top), (0, 0));
    assert_eq!(v.origin(&lay), (75, 75));
}

#[test]
fn a_turn_swaps_the_shown_size_and_restarts_the_pan() {
    let mut v = ImageView::new((300, 100));
    v.left = 20;
    v.turn(true);
    assert_eq!((v.turns, v.shown_size(), v.left), (1, (100, 300), 0));
    v.turn(false);
    v.turn(false);
    assert_eq!(v.turns, 3);
}

#[test]
fn dragging_the_picture_pans_it_against_the_pointer() {
    let mut v = ImageView::new((1000, 1000));
    v.zoom = Zoom::Scale(1.0);
    let lay = layout(&v, Rect::new(0, 0, 300, 300), 2, &cm());
    assert!(!v.press(100, 100, &lay));
    assert!(v.dragging());
    assert!(v.motion(80, 90, &lay));
    assert_eq!((v.left, v.top), (40, 20), "logical px times the scale");
    v.release();
    assert!(!v.dragging());
    assert!(!v.motion(0, 0, &lay));
}

#[test]
fn the_bars_arrows_step_and_the_thumb_drags() {
    let mut v = ImageView::new((2000, 400));
    v.zoom = Zoom::Scale(1.0);
    let lay = layout(&v, Rect::new(0, 0, 300, 200), 1, &cm());
    let hb = lay.hscroll.expect("a bar along the bottom");
    assert!(v.press(hb.right() - 4, hb.y + 4, &lay));
    assert_eq!(v.left, LINE);
    v.release();
    let (parts, _, _) = span(&v, &lay, Axis::Horizontal).unwrap();
    let t = parts.thumb.unwrap();
    v.press(t.x + 2, t.y + 2, &lay);
    assert!(v.motion(hb.right() + 50, t.y + 2, &lay));
    assert_eq!(v.left, lay.range().0, "dragged past the end holds there");
}

#[test]
fn draw_samples_the_turned_picture_and_prefers_the_rendered_copy() {
    let theme = Theme::default();
    let p = theme.paint();
    let mut v = ImageView::new((4, 2));
    v.zoom = Zoom::Scale(2.0);
    v.turn(true);
    let lay = layout(&v, Rect::new(0, 0, 4, 8), 1, &cm());
    assert_eq!(lay.content, (4, 8));
    let pic = coded(4, 2);
    let mut c = Canvas::new(4, 8);
    let src = Source {
        picture: &pic,
        rendered: None,
    };
    draw(&mut c, &p, &v, src, &lay, Color::rgb(0, 0, 0));
    assert_eq!(at(&c, 0, 0), (0, 1));
    assert_eq!(at(&c, 1, 1), (0, 1), "nearest picture pixel, twice as big");
    assert_eq!(at(&c, 3, 7), (3, 0));
    let mut red = Canvas::new(4, 8);
    red.fill_rect(red.bounds(), Color::rgb(0xFF, 0, 0));
    let src = Source {
        picture: &pic,
        rendered: Some(&red),
    };
    draw(&mut c, &p, &v, src, &lay, Color::rgb(0, 0, 0));
    assert_eq!(c.get(2, 2), 0xFFFF_0000);
}

#[test]
fn a_see_through_picture_shows_the_ground_through_it() {
    let theme = Theme::default();
    let p = theme.paint();
    let v = ImageView::new((2, 2));
    let lay = layout(&v, Rect::new(0, 0, 2, 2), 1, &cm());
    let pic = Canvas::from_pixels(2, 2, vec![0x8080_0000; 4]);
    let mut c = Canvas::new(2, 2);
    let src = Source {
        picture: &pic,
        rendered: None,
    };
    draw(&mut c, &p, &v, src, &lay, Color::rgb(0, 0, 0xFF));
    assert_eq!(c.get(0, 0), 0xFF80_007F);
}

#[test]
fn the_view_is_pinned_per_interface_and_scale() {
    let mut got = Vec::new();
    for (name, i) in [
        ("classic", Interface::Classic),
        ("blue", Interface::Blue),
        ("glass", Interface::Glass),
    ] {
        for s in [1, 2] {
            let mut theme = theme_for(i);
            theme.look = theme.look.at_scale(s);
            let p = theme.paint();
            let mut v = ImageView::new((240, 160));
            v.zoom = Zoom::Scale(2.0);
            v.turn(true);
            let lay = layout(&v, Rect::new(0, 0, 160, 120), s, &theme.controls());
            v.pan(30, 40, &lay);
            v.bars.motion(v.bar_at(&lay, 155, 5));
            let pic = coded(240, 160);
            let mut c = Canvas::with_scale(160, 120, s);
            let src = Source {
                picture: &pic,
                rendered: None,
            };
            draw(&mut c, &p, &v, src, &lay, theme.colors.window);
            got.push((format!("{name}@{s}"), fnv(&c)));
        }
    }
    let want: [(&str, u64); 6] = [
        ("classic@1", 0xed1b_a465_bbee_b41d),
        ("classic@2", 0x2e9c_34e3_1a0e_6fbd),
        ("blue@1", 0x48db_a1b1_0a12_f1bc),
        ("blue@2", 0x81b0_f480_ce7b_a44d),
        ("glass@1", 0xc17a_bed3_33d8_c5e9),
        ("glass@2", 0xa0b6_41b0_f3ab_427b),
    ];
    let want: Vec<(String, u64)> = want.iter().map(|(k, h)| (k.to_string(), *h)).collect();
    assert_eq!(got, want);
}
