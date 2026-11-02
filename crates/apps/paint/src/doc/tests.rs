use super::*;
use crate::bitmap::{BLACK, WHITE};

const RED: u32 = 0xFFFF_0000;

fn doc(w: i32, h: i32) -> Doc {
    Doc::new(Bitmap::new(w, h, WHITE))
}

fn rows(d: &Doc) -> Vec<String> {
    let b = &d.img;
    (0..b.h)
        .map(|y| {
            (0..b.w)
                .map(|x| match b.get(x, y) {
                    Some(BLACK) => '#',
                    Some(WHITE) => '.',
                    Some(RED) => 'r',
                    _ => '?',
                })
                .collect()
        })
        .collect()
}

const NO: Mods = Mods {
    shift: false,
    ctrl: false,
};
const SHIFT: Mods = Mods {
    shift: true,
    ctrl: false,
};

fn drag(d: &mut Doc, tool: Tool, path: &[P], ink: Ink, m: Mods) {
    d.press(tool, path[0], ink, m, false);
    for &p in &path[1..] {
        d.motion(tool, p, m);
    }
    d.release(tool, *path.last().unwrap(), m);
}

#[test]
fn pencil_draws_one_pixel_lines_in_the_button_colour() {
    let mut d = doc(5, 3);
    d.secondary = RED;
    drag(&mut d, Tool::Pencil, &[(0, 0), (4, 0)], Ink::Primary, NO);
    drag(&mut d, Tool::Pencil, &[(0, 2), (2, 2)], Ink::Secondary, NO);
    assert_eq!(rows(&d), ["#####", ".....", "rrr.."]);
    assert_eq!(d.hist.depth(), 2, "each stroke is one step");
    assert!(d.dirty);
}

#[test]
fn brush_stamps_its_tip_along_the_stroke() {
    let mut d = doc(6, 4);
    d.opts.brush = Brush::square(2);
    drag(&mut d, Tool::Brush, &[(1, 1), (3, 1)], Ink::Primary, NO);
    assert_eq!(rows(&d), ["......", ".####.", ".####.", "......"]);
}

#[test]
fn eraser_paints_secondary_and_right_button_only_replaces_primary() {
    let mut d = Doc::new(Bitmap::new(4, 2, BLACK));
    d.opts.eraser = 1;
    d.secondary = WHITE;
    drag(&mut d, Tool::Eraser, &[(0, 0), (3, 0)], Ink::Primary, NO);
    assert_eq!(rows(&d), ["....", "####"]);
    d.img.set(1, 1, RED);
    drag(&mut d, Tool::Eraser, &[(0, 1), (3, 1)], Ink::Secondary, NO);
    assert_eq!(
        rows(&d),
        ["....", ".r.."],
        "the colour eraser leaves other colours"
    );
}

#[test]
fn fill_floods_with_the_button_colour() {
    let mut d = doc(5, 3);
    drag(&mut d, Tool::Pencil, &[(2, 0), (2, 2)], Ink::Primary, NO);
    d.secondary = RED;
    d.press(Tool::Fill, (0, 0), Ink::Secondary, NO, false);
    d.release(Tool::Fill, (0, 0), NO);
    assert_eq!(rows(&d), ["rr#..", "rr#..", "rr#.."]);
}

#[test]
fn picker_reports_the_colour_under_it() {
    let mut d = doc(2, 1);
    d.img.set(1, 0, RED);
    assert_eq!(
        d.press(Tool::Picker, (1, 0), Ink::Secondary, NO, false),
        Effect::Picked(Ink::Secondary, RED)
    );
    assert_eq!(
        d.press(Tool::Picker, (5, 0), Ink::Primary, NO, false),
        Effect::None
    );
    assert_eq!(
        d.press(Tool::Magnifier, (1, 0), Ink::Primary, NO, false),
        Effect::Magnify((1, 0))
    );
}

#[test]
fn line_follows_the_pointer_until_release() {
    let mut d = doc(5, 3);
    d.press(Tool::Line, (0, 0), Ink::Primary, NO, false);
    d.motion(Tool::Line, (4, 2), NO);
    d.release(Tool::Line, (4, 0), NO);
    assert_eq!(
        rows(&d),
        ["#####", ".....", "....."],
        "the earlier preview is gone"
    );
    let mut d = doc(5, 3);
    drag(&mut d, Tool::Line, &[(0, 1), (4, 2)], Ink::Primary, SHIFT);
    assert_eq!(
        rows(&d),
        [".....", "#####", "....."],
        "Shift keeps it level"
    );
}

#[test]
fn rectangle_outline_both_and_fill() {
    let mut d = doc(5, 4);
    drag(&mut d, Tool::Rect, &[(0, 0), (4, 3)], Ink::Primary, NO);
    assert_eq!(rows(&d), ["#####", "#...#", "#...#", "#####"]);
    let mut d = doc(5, 4);
    d.secondary = RED;
    d.opts.fill = FillStyle::Both;
    drag(&mut d, Tool::Rect, &[(4, 3), (0, 0)], Ink::Primary, NO);
    assert_eq!(rows(&d), ["#####", "#rrr#", "#rrr#", "#####"]);
    let mut d = doc(4, 3);
    d.opts.fill = FillStyle::Fill;
    drag(&mut d, Tool::Rect, &[(1, 0), (3, 1)], Ink::Primary, NO);
    assert_eq!(rows(&d), [".###", ".###", "...."]);
}

#[test]
fn ellipse_and_rounded_rectangle_with_shift_are_square() {
    let mut d = doc(7, 7);
    drag(
        &mut d,
        Tool::Ellipse,
        &[(0, 0), (6, 2)],
        Ink::Primary,
        SHIFT,
    );
    let r = rows(&d);
    assert_eq!(r[0], "..###..");
    assert_eq!(r[6], "..###..", "Shift made it a circle seven high");
    assert_eq!(r[3], "#.....#");
    let mut d = doc(20, 20);
    drag(
        &mut d,
        Tool::RoundRect,
        &[(0, 0), (19, 19)],
        Ink::Primary,
        NO,
    );
    assert_eq!(d.img.get(0, 0), Some(WHITE), "the corner is cut");
    assert_eq!(d.img.get(10, 0), Some(BLACK));
}

#[test]
fn polygon_closes_on_double_click_and_fills() {
    let mut d = doc(5, 5);
    d.opts.fill = FillStyle::Fill;
    drag(&mut d, Tool::Polygon, &[(0, 0), (4, 0)], Ink::Primary, NO);
    assert!(d.pending());
    d.press(Tool::Polygon, (0, 4), Ink::Primary, NO, false);
    d.release(Tool::Polygon, (0, 4), NO);
    d.press(Tool::Polygon, (0, 4), Ink::Primary, NO, true);
    assert!(!d.pending());
    assert_eq!(rows(&d), ["#####", "####.", "###..", "##...", "....."]);
    assert_eq!(d.hist.depth(), 1);
}

#[test]
fn polygon_closes_near_its_first_corner() {
    let mut d = doc(6, 5);
    drag(&mut d, Tool::Polygon, &[(0, 0), (5, 0)], Ink::Primary, NO);
    drag(&mut d, Tool::Polygon, &[(5, 4)], Ink::Primary, NO);
    drag(&mut d, Tool::Polygon, &[(0, 4)], Ink::Primary, NO);
    d.press(Tool::Polygon, (1, 1), Ink::Primary, NO, false);
    assert!(!d.pending());
    assert_eq!(rows(&d), ["######", "#....#", "#....#", "#....#", "######"]);
}

#[test]
fn curve_bends_by_its_control_points() {
    let mut d = doc(9, 5);
    drag(&mut d, Tool::Curve, &[(0, 4), (8, 4)], Ink::Primary, NO);
    assert_eq!(rows(&d)[4], "#########");
    drag(&mut d, Tool::Curve, &[(4, 0)], Ink::Primary, NO);
    assert!(d.pending(), "a second control point may follow");
    assert!(rows(&d)[4] != "#########", "the line bent");
    drag(&mut d, Tool::Curve, &[(4, 0)], Ink::Primary, NO);
    assert!(!d.pending());
    assert_eq!(d.hist.depth(), 1, "a curve is one step");
    d.undo();
    assert_eq!(rows(&d), vec![".........".to_string(); 5]);
}

#[test]
fn airbrush_sprays_on_each_tick() {
    let mut d = doc(21, 21);
    d.opts.spray = 4;
    d.press(Tool::Airbrush, (10, 10), Ink::Primary, NO, false);
    let first = d.img.px.iter().filter(|&&p| p == BLACK).count();
    for _ in 0..20 {
        d.tick();
    }
    let later = d.img.px.iter().filter(|&&p| p == BLACK).count();
    d.release(Tool::Airbrush, (10, 10), NO);
    assert!(first > 0 && later > first);
    assert_eq!(d.tick(), Effect::None, "no spray once released");
    for y in 0..21 {
        for x in 0..21 {
            if d.img.get(x, y) == Some(BLACK) {
                assert!((x - 10) * (x - 10) + (y - 10) * (y - 10) <= 16);
            }
        }
    }
}

#[test]
fn moving_a_selection_leaves_secondary_and_is_one_step() {
    let mut d = doc(5, 2);
    d.img.set(0, 0, BLACK);
    d.img.set(1, 0, BLACK);
    let before = d.img.clone();
    drag(&mut d, Tool::Select, &[(0, 0), (1, 0)], Ink::Primary, NO);
    assert_eq!(d.sel.as_ref().map(|s| s.rect), Some(Rect::new(0, 0, 2, 1)));
    drag(&mut d, Tool::Select, &[(0, 0), (3, 1)], Ink::Primary, NO);
    assert_eq!(
        d.composite(d.img.bounds()).get(3, 1),
        Some(BLACK),
        "shown at its new place"
    );
    d.finish();
    assert_eq!(rows(&d), [".....", "...##"]);
    assert_eq!(d.hist.depth(), 1);
    d.undo();
    assert_eq!(d.img, before);
}

#[test]
fn ctrl_drag_copies_and_transparent_leaves_out_the_secondary() {
    let mut d = doc(4, 1);
    d.img.set(0, 0, BLACK);
    d.opts.opaque = false;
    drag(&mut d, Tool::Select, &[(0, 0), (1, 0)], Ink::Primary, NO);
    d.img.set(3, 0, RED);
    let ctrl = Mods {
        shift: false,
        ctrl: true,
    };
    drag(&mut d, Tool::Select, &[(0, 0), (2, 0)], Ink::Primary, ctrl);
    d.finish();
    assert_eq!(
        rows(&d),
        ["#.#r"],
        "the white of the selection let red through"
    );
}

#[test]
fn free_form_selection_moves_only_its_shape() {
    let mut d = Doc::new(Bitmap::new(5, 5, BLACK));
    drag(
        &mut d,
        Tool::FreeSelect,
        &[(0, 0), (4, 0), (0, 4)],
        Ink::Primary,
        NO,
    );
    let s = d.sel.clone().expect("a selection");
    assert_eq!(s.rect, Rect::new(0, 0, 5, 4));
    assert!(s.mask.is_some());
    d.delete_selection();
    assert_eq!(rows(&d), [".....", "....#", "...##", "..###", "#####"]);
}

#[test]
fn delete_cut_copy_and_crop() {
    let mut d = doc(4, 2);
    d.img.set(1, 0, BLACK);
    drag(&mut d, Tool::Select, &[(1, 0), (2, 1)], Ink::Primary, NO);
    let copied = d.selected().unwrap();
    assert_eq!((copied.w, copied.h, copied.px[0]), (2, 2, BLACK));
    d.crop();
    assert_eq!(rows(&d), ["#.", ".."]);
    d.undo();
    assert_eq!((d.img.w, d.img.h), (4, 2));
    d.select_all();
    d.delete_selection();
    assert_eq!(rows(&d), ["....", "...."]);
}

#[test]
fn paste_floats_and_grows_the_picture() {
    let mut d = doc(2, 2);
    d.paste(Bitmap::new(3, 1, BLACK), (0, 0));
    assert_eq!((d.img.w, d.img.h), (3, 2));
    drag(&mut d, Tool::Select, &[(0, 0), (0, 1)], Ink::Primary, NO);
    d.finish();
    assert_eq!(rows(&d), ["...", "###"]);
    assert_eq!(d.hist.depth(), 1, "pasting and moving is one step");
    d.undo();
    assert_eq!((d.img.w, d.img.h), (2, 2));
}

#[test]
fn undo_during_a_paste_drops_it() {
    let mut d = doc(2, 2);
    d.paste(Bitmap::new(1, 1, BLACK), (0, 0));
    assert!(d.undo());
    assert!(d.sel.is_none());
    d.finish();
    assert_eq!(rows(&d), ["..", ".."]);
    assert_eq!(d.hist.depth(), 0);
}

#[test]
fn undo_redo_twenty_steps_and_back() {
    let mut d = doc(30, 1);
    let start = d.img.clone();
    let mut states = vec![start.clone()];
    for x in 0..25 {
        drag(&mut d, Tool::Pencil, &[(x, 0)], Ink::Primary, NO);
        states.push(d.img.clone());
    }
    for i in (0..25).rev() {
        assert!(d.undo());
        assert_eq!(d.img, states[i], "undo to step {i}");
    }
    assert!(!d.undo());
    for state in &states[1..] {
        assert!(d.redo());
        assert_eq!(&d.img, state);
    }
}

#[test]
fn whole_picture_changes_undo_as_one_step() {
    let mut d = doc(3, 2);
    d.img.set(0, 0, BLACK);
    let before = d.img.clone();
    d.replace(d.img.resized(6, 4));
    assert_eq!((d.img.w, d.img.h), (6, 4));
    assert_eq!(d.img.get(1, 1), Some(BLACK));
    d.undo();
    assert_eq!(d.img, before);
}

#[test]
fn escape_takes_back_a_shape_in_progress() {
    let mut d = doc(4, 4);
    d.press(Tool::Rect, (0, 0), Ink::Primary, NO, false);
    d.motion(Tool::Rect, (3, 3), NO);
    d.cancel();
    assert_eq!(rows(&d), ["....", "....", "....", "...."]);
    assert_eq!(d.hist.depth(), 0);
}

#[test]
fn text_coverage_blends_into_the_primary_colour() {
    let mut d = doc(3, 1);
    d.opts.opaque = false;
    d.text_box = Some(Rect::new(0, 0, 3, 1));
    d.ink_text(Rect::new(0, 0, 3, 1), &[255, 0, 128]);
    assert_eq!(d.img.px, [BLACK, WHITE, 0xFF7F_7F7F]);
    assert!(d.text_box.is_none());
    assert_eq!(d.hist.depth(), 1);
}
