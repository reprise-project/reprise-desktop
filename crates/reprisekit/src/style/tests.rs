use super::*;
use crate::raster::Fill;
use crate::theme::FontConfig;

fn spec() -> TextSpec {
    TextSpec {
        family: FontConfig::default().family,
        size_px: 11,
        family_sizes: Vec::new(),
        bold: false,
        italic: false,
        antialias: false,
        full_hinting: false,
        color: Color::rgb(255, 255, 255),
        inactive: Color::rgb(200, 200, 200),
        shadow: None,
        ink_gamma: 1.0,
    }
}

fn soft_shadow() -> Shadow {
    Shadow {
        color: Color::rgb(0, 0, 0),
        dx: 1,
        dy: 1,
        soft: Some(SoftShadow {
            size: 4,
            gain: (3, 2),
        }),
    }
}

fn block(alpha: &[u8], w: usize, value: u8) -> (usize, usize, usize, usize) {
    let lit: Vec<(usize, usize)> = (0..alpha.len())
        .filter(|&i| alpha[i] != 0)
        .map(|i| (i % w, i / w))
        .collect();
    assert!(
        lit.iter().all(|&(x, y)| alpha[y * w + x] == value),
        "every lit pixel is {value}"
    );
    let xs = lit.iter().map(|p| p.0);
    let ys = lit.iter().map(|p| p.1);
    let (x0, x1) = (xs.clone().min().unwrap(), xs.max().unwrap());
    let (y0, y1) = (ys.clone().min().unwrap(), ys.max().unwrap());
    assert_eq!(lit.len(), (x1 - x0 + 1) * (y1 - y0 + 1), "a full block");
    (x0, x1, y0, y1)
}

#[test]
fn a_soft_shadow_spreads_an_inked_pixel_from_1_px_up_left_to_2_down_right() {
    let mut mask = vec![0u8; 12 * 12];
    mask[4 * 12 + 4] = 255;
    let alpha = soft_shadow().alpha(&mask, 12, 12, 1);
    // 255 over 16 pixels at 3/2: 23 each.
    assert_eq!(block(&alpha, 12, 23), (3, 6, 3, 6));
}

#[test]
fn a_soft_shadow_caps_at_full_under_solid_ink() {
    let mask = vec![255u8; 8 * 8];
    let alpha = soft_shadow().alpha(&mask, 8, 8, 1);
    assert_eq!(alpha[4 * 8 + 4], 255, "3/2 of full coverage stays full");
}

#[test]
fn a_soft_shadow_at_2_device_pixels_a_pixel_is_twice_as_far_and_wide() {
    let mut mask = vec![0u8; 20 * 20];
    mask[8 * 20 + 8] = 255;
    let alpha = soft_shadow().alpha(&mask, 20, 20, 2);
    // An 8x8 block from 2 px up and left: 255 * 3 / (2 * 64) = 5.
    assert_eq!(block(&alpha, 20, 5), (6, 13, 6, 13));
}

#[test]
fn a_hard_shadow_is_the_coverage_moved_by_its_offset() {
    let mut mask = vec![0u8; 6 * 6];
    mask[2 * 6 + 1] = 200;
    let sh = Shadow {
        soft: None,
        ..soft_shadow()
    };
    let alpha = sh.alpha(&mask, 6, 6, 1);
    assert_eq!(block(&alpha, 6, 200), (2, 2, 3, 3));
    let alpha = sh.alpha(&mask, 6, 6, 2);
    assert_eq!(block(&alpha, 6, 200), (3, 3, 4, 4), "moved 2 px at d 2");
}

#[test]
fn an_ink_gamma_of_one_half_takes_the_square_root_of_the_coverage() {
    let mut c = [0, 64, 128, 255];
    TextSpec {
        ink_gamma: 0.5,
        ..spec()
    }
    .weigh(&mut c);
    assert_eq!(c, [0, 128, 181, 255]);
    let mut c = [0, 64, 128, 255];
    spec().weigh(&mut c);
    assert_eq!(c, [0, 64, 128, 255], "a gamma of 1 inks as rasterized");
}

fn style() -> Style {
    let mut atlas = Canvas::new(3, 4);
    atlas.fill_rect(Rect::new(0, 0, 3, 2), Color::rgb(255, 0, 0));
    atlas.fill_rect(Rect::new(0, 2, 3, 2), Color::rgb(0, 255, 0));
    atlas.clear_rect(Rect::new(0, 0, 1, 1));
    atlas.clear_rect(Rect::new(0, 2, 1, 1));
    Style::new(
        "Test",
        ColorScheme::default(),
        Metrics::default(),
        spec(),
        spec(),
        Color::rgb(0, 0, 0),
        crate::taskbar::TaskbarShape::CLASSIC,
    )
    .with_part(
        Part::Caption,
        PartImage::new(atlas, &[State::Normal, State::Hot], Layout::Vertical)
            .sizing(Insets::new(1, 1, 1, 0))
            .content(Insets::new(2, 0, 2, 0)),
    )
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "does not divide evenly")]
fn a_mismatched_frame_count_panics_in_debug() {
    let atlas = Canvas::new(3, 5);
    PartImage::new(atlas, &[State::Normal, State::Hot], Layout::Vertical);
}

#[test]
fn a_state_picks_its_frame_and_a_missing_one_falls_back_to_normal() {
    let s = style();
    let p = s.part(Part::Caption).unwrap();
    assert_eq!(p.frame(State::Normal), Rect::new(0, 0, 3, 2));
    assert_eq!(p.frame(State::Hot), Rect::new(0, 2, 3, 2));
    assert_eq!(p.frame(State::Pressed), Rect::new(0, 0, 3, 2));
}

#[test]
fn a_painted_part_paints_once_on_first_ask_and_clones_share_it() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PAINTS: AtomicUsize = AtomicUsize::new(0);
    let s = style().with_painted(Part::Taskbar, || {
        PAINTS.fetch_add(1, Ordering::Relaxed);
        PartImage::new(Canvas::new(2, 2), &[State::Normal], Layout::Vertical)
    });
    let copy = s.clone();
    assert!(s.has_parts(&[Part::Taskbar]));
    assert_eq!(PAINTS.load(Ordering::Relaxed), 0);
    assert_eq!(s.part(Part::Taskbar).unwrap().atlas.width, 2);
    assert_eq!(copy.part(Part::Taskbar).unwrap().atlas.width, 2);
    assert_eq!(PAINTS.load(Ordering::Relaxed), 1);
}

#[test]
fn a_painted_group_paints_once_when_its_first_part_is_asked_for() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PAINTS: AtomicUsize = AtomicUsize::new(0);
    let img = |w| PartImage::new(Canvas::new(w, 2), &[State::Normal], Layout::Vertical);
    let s = style().with_painted_group(&[Part::Taskbar, Part::TrayArea], move || {
        PAINTS.fetch_add(1, Ordering::Relaxed);
        vec![(Part::Taskbar, img(2)), (Part::TrayArea, img(3))]
    });
    assert!(s.has_parts(&[Part::Taskbar, Part::TrayArea]));
    assert_eq!(PAINTS.load(Ordering::Relaxed), 0);
    assert_eq!(s.part(Part::TrayArea).unwrap().atlas.width, 3);
    assert_eq!(s.part(Part::Taskbar).unwrap().atlas.width, 2);
    assert_eq!(PAINTS.load(Ordering::Relaxed), 1);
}

#[test]
fn a_picked_text_color_is_picked_once_on_first_ask() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PICKS: AtomicUsize = AtomicUsize::new(0);
    let ink = Color::rgb(1, 2, 3);
    let s = style().with_picked_text_color(Part::StartPanel, State::Normal, move || {
        PICKS.fetch_add(1, Ordering::Relaxed);
        ink
    });
    assert_eq!(PICKS.load(Ordering::Relaxed), 0);
    assert_eq!(s.text_color(Part::StartPanel, State::Normal), Some(ink));
    assert_eq!(
        s.clone().text_color(Part::StartPanel, State::Normal),
        Some(ink)
    );
    assert_eq!(s.text_color(Part::StartPanel, State::Hot), None);
    assert_eq!(PICKS.load(Ordering::Relaxed), 1);
}

#[test]
fn a_repeated_state_keeps_one_frame_per_button() {
    let atlas = Canvas::new(2, 8);
    let p = PartImage::new(
        atlas,
        &[State::Normal, State::Hot, State::Hot, State::Hot],
        Layout::Vertical,
    );
    assert_eq!(p.frame_nth(State::Hot, 0), Rect::new(0, 2, 2, 2), "first");
    assert_eq!(p.frame_nth(State::Hot, 2), Rect::new(0, 6, 2, 2), "third");
    assert_eq!(
        p.frame_nth(State::Hot, 3),
        Rect::new(0, 2, 2, 2),
        "past the last: the state's first"
    );
    assert_eq!(
        p.frame_nth(State::Normal, 1),
        Rect::new(0, 0, 2, 2),
        "a state listed once"
    );
    assert_eq!(
        p.frame_nth(State::Pressed, 1),
        Rect::new(0, 0, 2, 2),
        "a missing state: the atlas's first"
    );
}

#[test]
fn draw_nine_grids_the_frame_into_the_rect() {
    let s = style();
    let mut c = Canvas::new(10, 3);
    s.draw(&mut c, Rect::new(0, 0, 10, 3), Part::Caption, State::Hot, 1);
    assert_eq!(c.get(0, 0), 0);
    assert_eq!(c.get(5, 1), 0xFF00_FF00);
    s.draw(
        &mut c,
        Rect::new(0, 0, 10, 3),
        Part::Taskbar,
        State::Normal,
        1,
    );
    assert_eq!(c.get(0, 0), 0);
}

fn check_part(checks: &[Check]) -> PartImage {
    let n = checks.len() * CHECK_STATES.len();
    let mut atlas = Canvas::new(2, 2 * n as u32);
    for i in 0..n {
        atlas.fill_rect(Rect::new(0, 2 * i as i32, 2, 2), Color::rgb(i as u8, 0, 0));
    }
    PartImage::new_check(atlas, checks, Layout::Vertical)
}

fn nth(i: i32) -> Rect {
    Rect::new(0, 2 * i, 2, 2)
}

#[test]
fn check_frames_run_check_major_then_normal_hot_pressed_disabled() {
    let p = check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]);
    let mut i = 0;
    for check in [Check::Unchecked, Check::Checked, Check::Mixed] {
        for state in [State::Normal, State::Hot, State::Pressed, State::Disabled] {
            assert_eq!(
                p.check_frame(check, state),
                nth(i),
                "{check:?} {state:?} is frame {i} of 12"
            );
            i += 1;
        }
    }
}

#[test]
fn a_missing_check_state_falls_back_to_normal_of_the_same_check() {
    let p = check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]);
    assert_eq!(
        p.check_frame(Check::Mixed, State::Inactive),
        nth(8),
        "Mixed Inactive falls back to Mixed Normal, frame 8"
    );
}

#[test]
fn a_radio_has_eight_frames_and_mixed_falls_back_to_unchecked_normal() {
    let p = check_part(&[Check::Unchecked, Check::Checked]);
    assert_eq!(
        p.check_frame(Check::Checked, State::Disabled),
        nth(7),
        "Checked Disabled is the last of 8 frames"
    );
    assert_eq!(
        p.check_frame(Check::Mixed, State::Hot),
        nth(0),
        "a radio's Mixed falls back to Unchecked Normal, frame 0"
    );
}

#[test]
fn draw_check_paints_the_frame_for_the_check_and_state() {
    let s = style().with_part(
        Part::CheckBox,
        check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]),
    );
    let mut c = Canvas::new(4, 4);
    s.draw_check(
        &mut c,
        Rect::new(0, 0, 4, 4),
        Part::CheckBox,
        Check::Checked,
        State::Hot,
        1,
    );
    assert_eq!(
        c.get(1, 1),
        Color::rgb(5, 0, 0).argb(),
        "Checked Hot paints frame 5"
    );
}

#[test]
fn content_rect_insets_by_the_content_margins() {
    assert_eq!(
        style().content_rect(Part::Caption, Rect::new(0, 0, 10, 3), 1),
        Rect::new(2, 0, 6, 3)
    );
}

#[test]
fn clipped_ends_keep_their_pixels_in_a_narrow_rect() {
    let mut atlas = Canvas::new(5, 1);
    for x in 0..5 {
        atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
    }
    let s = style().with_part(
        Part::Taskbar,
        PartImage::new(atlas, &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 0, 2, 0))
            .clip_ends(),
    );
    let row = |w: i32| {
        let mut c = Canvas::new(w as u32, 1);
        s.draw(
            &mut c,
            Rect::new(0, 0, w, 1),
            Part::Taskbar,
            State::Normal,
            1,
        );
        (0..w)
            .map(|x| (c.get(x, 0) >> 16) & 0xFF)
            .collect::<Vec<u32>>()
    };
    assert_eq!(row(6), vec![10, 20, 30, 30, 40, 50], "wide: a nine-grid");
    assert_eq!(row(3), vec![10, 40, 50], "the left end, then the right");
    assert_eq!(row(1), vec![50], "only the right end's last column");
    assert!(s.opaque_at(Part::Taskbar, State::Normal, Rect::new(0, 0, 3, 1), 1, 1, 0));
}

#[test]
fn split_ends_share_a_narrow_rect() {
    let mut atlas = Canvas::new(5, 1);
    for x in 0..5 {
        atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
    }
    let s = style().with_part(
        Part::Taskbar,
        PartImage::new(atlas, &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 0, 2, 0))
            .split_ends(),
    );
    let row = |w: i32| {
        let mut c = Canvas::new(w as u32, 1);
        s.draw(
            &mut c,
            Rect::new(0, 0, w, 1),
            Part::Taskbar,
            State::Normal,
            1,
        );
        (0..w)
            .map(|x| (c.get(x, 0) >> 16) & 0xFF)
            .collect::<Vec<u32>>()
    };
    assert_eq!(row(5), vec![10, 20, 30, 40, 50], "wide: a nine-grid");
    assert_eq!(
        row(3),
        vec![10, 20, 50],
        "the left end's first 2, the right's last 1"
    );
    assert_eq!(row(2), vec![10, 50], "one column of each");
}

#[test]
fn opaque_at_follows_the_atlas_alpha() {
    let s = style();
    let r = Rect::new(10, 10, 10, 3);
    assert!(!s.opaque_at(Part::Caption, State::Normal, r, 1, 10, 10));
    assert!(s.opaque_at(Part::Caption, State::Normal, r, 1, 11, 10));
    assert!(!s.opaque_at(Part::Caption, State::Normal, r, 1, 30, 10));
}

fn ramp_atlas(frames: i32) -> Canvas {
    let mut a = Canvas::new(6, 5 * frames as u32);
    for y in 0..5 * frames {
        for x in 0..6 {
            a.fill_rect(
                Rect::new(x, y, 1, 1),
                Color::rgb(40 * x as u8 + 1, 12 * y as u8 + 1, 7),
            );
        }
    }
    for f in 0..frames {
        a.clear_rect(Rect::new(0, 5 * f, 1, 1));
        a.clear_rect(Rect::new(5, 5 * f + 4, 1, 1));
    }
    a
}

#[test]
fn a_part_painted_at_2_lands_pixel_for_pixel_on_a_scale_2_canvas() {
    let mut atlas = Canvas::new(8, 6);
    for (i, px) in atlas.pixels.iter_mut().enumerate() {
        *px = 0xFF00_0000 | (i as u32 * 0x0005_0309);
    }
    atlas.clear_rect(Rect::new(0, 0, 1, 1));
    let s = with_taskbar(
        PartImage::new(atlas, &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(1, 1, 1, 1))
            .at_scale(2),
    );
    let r = Rect::new(1, 1, 7, 5);
    for clip in [None, Some(Rect::new(2, 2, 4, 3))] {
        let mut program = Canvas::with_scale(10, 8, 2);
        let mut compositor = Canvas::new(20, 16);
        let dev = r.scaled(2);
        match clip {
            Some(cl) => compositor.with_clip(cl.scaled(2), |c| {
                s.draw(c, dev, Part::Taskbar, State::Normal, 2)
            }),
            None => s.draw(&mut compositor, dev, Part::Taskbar, State::Normal, 2),
        }
        match clip {
            Some(cl) => program.with_clip(cl, |c| s.draw(c, r, Part::Taskbar, State::Normal, 1)),
            None => s.draw(&mut program, r, Part::Taskbar, State::Normal, 1),
        }
        assert!(
            compositor.pixels.iter().any(|&p| p != 0),
            "{clip:?}: the draw paints something"
        );
        assert!(
            program.pixels == compositor.pixels,
            "{clip:?}: the program's device pixels are the compositor's"
        );
        assert_eq!(program.scale(), 2, "the canvas keeps its scale");
    }
}

fn with_taskbar(img: PartImage) -> Style {
    style().with_part(Part::Taskbar, img)
}

fn assert_doubles(what: &str, w: u32, h: u32, draw: impl Fn(&mut Canvas, i32)) {
    let mut one = Canvas::new(w, h);
    draw(&mut one, 1);
    let mut two = Canvas::new(2 * w, 2 * h);
    draw(&mut two, 2);
    assert!(
        (0..h as i32).any(|y| (0..w as i32).any(|x| one.get(x, y) != 0)),
        "{what}: the s = 1 draw paints something"
    );
    assert!(
        crate::look::styled::fixture::doubles_at_scale_1(&one, &two),
        "{what}: the s = 2 draw is the s = 1 draw with every pixel a 2x2 block"
    );
}

#[test]
fn warm_paints_on_another_thread_the_parts_drawn_at_scale_1_and_no_others() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PAINTS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
    let painted = |n: i32| {
        let art = |slot: usize| {
            move || {
                PAINTS[slot].fetch_add(1, Ordering::SeqCst);
                PartImage::new(Canvas::new(2, 2), &[State::Normal], Layout::Vertical)
            }
        };
        let k = 2 * (n as usize - 1);
        style()
            .with_painted(Part::Caption, art(k))
            .with_painted(Part::Taskbar, art(k + 1))
    };
    let st = Arc::new(painted(1).with_rescale(painted));
    assert!(
        st.warm(2, None).is_none(),
        "nothing drawn yet: nothing to paint"
    );
    st.part(Part::Caption);
    assert!(st.warm(1, None).is_none(), "scale 1 is the style itself");
    let warm = st
        .warm(2, None)
        .expect("the caption drawn at scale 1 is unpainted at scale 2");
    std::thread::spawn(move || warm.paint()).join().unwrap();
    let count = |slot: usize| PAINTS[slot].load(Ordering::SeqCst);
    assert_eq!(
        (count(2), count(3)),
        (1, 0),
        "scale 2: the caption, not the taskbar"
    );
    st.at_scale(2).part(Part::Caption);
    assert_eq!(count(2), 1, "a draw at scale 2 finds the caption painted");
    assert!(st.warm(2, None).is_none(), "scale 2 is warm");
}

#[test]
fn warm_parts_paints_the_named_parts_unpainted_at_each_scale_and_no_others() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PAINTS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
    let painted = |n: i32| {
        let art = |slot: usize| {
            move || {
                PAINTS[slot].fetch_add(1, Ordering::SeqCst);
                PartImage::new(Canvas::new(2, 2), &[State::Normal], Layout::Vertical)
            }
        };
        let k = 2 * (n as usize - 1);
        style()
            .with_painted(Part::SwitcherFrame, art(k))
            .with_painted(Part::Taskbar, art(k + 1))
    };
    let st = Arc::new(painted(1).with_rescale(painted));
    let named = [Part::SwitcherFrame, Part::SnapFrame];
    let warm: Vec<Warm> = [1, 2]
        .iter()
        .map(|&n| st.warm_parts(n, &named).expect("the frame is unpainted"))
        .collect();
    std::thread::spawn(move || warm.iter().for_each(Warm::paint))
        .join()
        .unwrap();
    let count = |slot: usize| PAINTS[slot].load(Ordering::SeqCst);
    assert_eq!(
        [count(0), count(1), count(2), count(3)],
        [1, 0, 1, 0],
        "the switcher frame at scales 1 and 2, not the taskbar"
    );
    st.part(Part::SwitcherFrame);
    st.at_scale(2).part(Part::SwitcherFrame);
    assert_eq!(
        (count(0), count(2)),
        (1, 1),
        "a draw finds the frame painted"
    );
    assert!(st.warm_parts(1, &named).is_none(), "scale 1 is warm");
    assert!(st.warm_parts(2, &named).is_none(), "scale 2 is warm");
}

#[test]
fn a_scale_no_longer_kept_builds_its_style_again() {
    let st = Arc::new(style().with_rescale(|_| style()));
    let two = st.at_scale(2);
    assert!(Arc::ptr_eq(&two, &st.at_scale(2)), "scale 2 is kept");
    st.keep_scales(|n| n == 2);
    assert!(Arc::ptr_eq(&two, &st.at_scale(2)), "still in use");
    st.keep_scales(|n| n == 1);
    assert!(
        !Arc::ptr_eq(&two, &st.at_scale(2)),
        "dropped, then built again"
    );
}

#[test]
fn a_stretched_grid_at_scale_2_doubles_the_scale_1_grid() {
    let s = style();
    let r = Rect::new(1, 1, 11, 3);
    assert_doubles("Stretch caption", 14, 5, |c, k| {
        s.draw(c, r.scaled(k), Part::Caption, State::Hot, k)
    });
    let s = with_taskbar(
        PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 1, 1, 2)),
    );
    for r in [Rect::new(1, 2, 13, 9), Rect::new(0, 0, 4, 3)] {
        assert_doubles("Stretch ramp", 15, 12, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }
}

#[test]
fn a_tiled_and_a_centred_grid_at_scale_2_double_the_scale_1_grid() {
    for fill in [Fill::Tile, Fill::StretchCentred] {
        let s = with_taskbar(
            PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 1, 1, 2))
                .fill(fill),
        );
        let r = Rect::new(1, 2, 13, 9);
        assert_doubles(&format!("{fill:?}"), 15, 12, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }
}

#[test]
fn a_border_only_grid_at_scale_2_doubles_the_scale_1_grid() {
    let mut img = PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
        .sizing(Insets::new(2, 1, 1, 2));
    img.border_only = true;
    let s = with_taskbar(img);
    let r = Rect::new(1, 2, 13, 9);
    assert_doubles("border only", 15, 12, |c, k| {
        s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
    });
}

#[test]
fn an_outset_at_scale_2_reaches_twice_as_far() {
    let s = with_taskbar(
        PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 1, 1, 2))
            .outset(Insets::new(1, 2, 0, 1)),
    );
    let r = Rect::new(2, 3, 10, 6);
    assert_doubles("outset", 15, 12, |c, k| {
        s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
    });
}

#[test]
fn a_true_size_frame_at_scale_2_is_twice_its_size() {
    let mut img = PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical);
    img.true_size = true;
    let s = with_taskbar(img);
    let r = Rect::new(1, 1, 10, 9);
    assert_doubles("true size", 12, 11, |c, k| {
        s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
    });
}

#[test]
fn a_check_at_scale_2_doubles_the_checked_hot_frame() {
    let atlas = {
        let mut a = Canvas::new(6, 5 * 12);
        for f in 0..12 {
            let one = ramp_atlas(1);
            for y in 0..5 {
                for x in 0..6 {
                    a.put_raw(x, 5 * f + y, one.get(x, y) ^ (f as u32) << 8);
                }
            }
        }
        a
    };
    let s = style().with_part(
        Part::CheckBox,
        PartImage::new_check(
            atlas,
            &[Check::Unchecked, Check::Checked, Check::Mixed],
            Layout::Vertical,
        )
        .sizing(Insets::new(2, 1, 1, 2)),
    );
    let r = Rect::new(1, 1, 9, 7);
    assert_doubles("Checked Hot", 11, 9, |c, k| {
        s.draw_check(
            c,
            r.scaled(k),
            Part::CheckBox,
            Check::Checked,
            State::Hot,
            k,
        )
    });
}

#[test]
fn the_nth_frame_at_scale_2_doubles_the_scale_1_frame() {
    let s = with_taskbar(
        PartImage::new(
            ramp_atlas(3),
            &[State::Normal, State::Hot, State::Hot],
            Layout::Vertical,
        )
        .sizing(Insets::new(2, 1, 1, 2)),
    );
    let r = Rect::new(1, 2, 13, 9);
    assert_doubles("second Hot frame", 15, 12, |c, k| {
        s.draw_nth(c, r.scaled(k), Part::Taskbar, State::Hot, 1, k)
    });
}

#[test]
fn clipped_ends_at_scale_2_double_the_scale_1_ends() {
    let mut atlas = Canvas::new(5, 1);
    for x in 0..5 {
        atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
    }
    let s = with_taskbar(
        PartImage::new(atlas, &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 0, 2, 0))
            .clip_ends(),
    );
    for w in [7, 3, 1] {
        let r = Rect::new(0, 0, w, 1);
        assert_doubles(&format!("clipped ends {w} wide"), w as u32, 1, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }
}

#[test]
fn content_rect_at_scale_2_insets_twice_the_margins() {
    assert_eq!(
        style().content_rect(Part::Caption, Rect::new(0, 0, 20, 6), 2),
        Rect::new(4, 0, 12, 6)
    );
}

#[test]
fn opaque_at_scale_2_answers_for_the_scale_1_pixel_under_it() {
    let s = style();
    let r = Rect::new(1, 1, 11, 3);
    for y in 0..10 {
        for x in 0..26 {
            assert_eq!(
                s.opaque_at(Part::Caption, State::Normal, r.scaled(2), 2, x, y),
                s.opaque_at(Part::Caption, State::Normal, r, 1, x / 2, y / 2),
                "({x}, {y}) at s = 2 is ({}, {}) at s = 1",
                x / 2,
                y / 2
            );
        }
    }
}
