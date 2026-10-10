//! The scrollbars through the scroll parts: track, two arrow buttons and the thumb with its grip, laid out as [`scrollbar_parts`] does Classic's (each arrow square to the bar's thickness). A bar with everything in view draws its track and arrows disabled and no thumb; with no track left between the arrows it draws no thumb and live arrows.
//! The grip keeps its own size, centred in the thumb's content rect (an odd pixel of slack goes after it), and only when that rect holds it: Blue's thumb keeps 3 px of bevel at each end, so its 8 px grip needs a thumb at least 14 px long.
//! The lit part takes its Hot or Pressed frame: an arrow of a live bar, the thumb with its grip, or a held page of the track (Pressed, no hot look). While any part of a live bar is lit, each arrow and the thumb at rest take their BarHot frame, if the look has one. A bar with everything in view ignores the pointer.

use crate::classic::widgets::{ScrollHit, scrollbar_parts};
use crate::look::Paint;
use crate::pointer::{Interaction, Lit, lit_of};
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};

/// [`crate::look::widgets::draw_scrollbar`] through `st`; `false` when
/// `st` lacks the vertical track, either arrow or the thumb.
#[expect(clippy::too_many_arguments)]
pub fn draw_scrollbar(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    top: usize,
    visible: usize,
    total: usize,
    lit: Lit<ScrollHit>,
) -> bool {
    bar(st, c, p, sb, (top, visible, total), lit, false)
}

/// [`crate::look::widgets::draw_hscrollbar`] through `st`; `false` when
/// `st` lacks the horizontal track, either arrow or the thumb.
#[expect(clippy::too_many_arguments)]
pub fn draw_hscrollbar(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    left: usize,
    visible: usize,
    total: usize,
    lit: Lit<ScrollHit>,
) -> bool {
    bar(st, c, p, sb, (left, visible, total), lit, true)
}

fn bar(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    sb: Rect,
    view: (usize, usize, usize),
    lit: Lit<ScrollHit>,
    horizontal: bool,
) -> bool {
    let (pos, visible, total) = view;
    let (track, back, forward, thumb, grip) = if horizontal {
        (
            Part::ScrollTrackH,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbH,
            Part::ScrollGripH,
        )
    } else {
        (
            Part::ScrollTrackV,
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollThumbV,
            Part::ScrollGripV,
        )
    };
    // The grip is the one optional part: a thumb with none draws bare,
    // as one too short to hold it does.
    if !st.has_parts(&[track, back, forward, thumb]) {
        return false;
    }
    let thumb_min = p.look.controls().thumb_min;
    let parts = scrollbar_parts(sb, p.s, pos, visible, total, horizontal, thumb_min);
    let live = total > visible;
    let lit = if live { lit } else { None };
    let state = |hit| super::state(live, lit_of(lit, hit));
    // An arrow or thumb at rest on a bar the pointer lights elsewhere
    // takes its BarHot frame, its first frame in a style without one.
    let near = |hit| match (state(hit), lit) {
        (State::Normal, Some(_)) => State::BarHot,
        (s, _) => s,
    };
    let track_state = if live { State::Normal } else { State::Disabled };
    st.draw(c, parts.track, track, track_state, p.s);
    if let (Some(t), Some((page, Interaction::Pressed))) = (parts.thumb, lit)
        && let Some(r) = page_rect(parts.track, t, page, horizontal)
    {
        st.draw(c, r, track, State::Pressed, p.s);
    }
    st.draw(c, parts.back, back, near(ScrollHit::Back), p.s);
    st.draw(c, parts.forward, forward, near(ScrollHit::Forward), p.s);
    if let Some(t) = parts.thumb {
        let lit = near(ScrollHit::Thumb);
        st.draw(c, t, thumb, lit, p.s);
        if let Some(g) = grip_rect(st, grip, st.content_rect(thumb, t, p.s), p.s) {
            st.draw(c, g, grip, lit, p.s);
        }
    }
    true
}

/// The part of `track` a held page covers: from the track's start to the
/// thumb for `PageBack`, from the thumb to the track's end for
/// `PageForward`; `None` for any other part, or a page with no room.
fn page_rect(track: Rect, thumb: Rect, page: ScrollHit, horizontal: bool) -> Option<Rect> {
    let r = match (page, horizontal) {
        (ScrollHit::PageBack, false) => Rect::new(track.x, track.y, track.width, thumb.y - track.y),
        (ScrollHit::PageForward, false) => Rect::new(
            track.x,
            thumb.bottom(),
            track.width,
            track.bottom() - thumb.bottom(),
        ),
        (ScrollHit::PageBack, true) => Rect::new(track.x, track.y, thumb.x - track.x, track.height),
        (ScrollHit::PageForward, true) => Rect::new(
            thumb.right(),
            track.y,
            track.right() - thumb.right(),
            track.height,
        ),
        _ => return None,
    };
    (!r.is_empty()).then_some(r)
}

/// Where `grip` lands in `room`, the thumb's content rect, at `s` device
/// pixels per pixel of the look's design: at its own size times `s`,
/// centred on whole `s`-blocks with an odd block of slack after it; `None`
/// when `room` is too short or too narrow to hold it.
fn grip_rect(st: &Style, grip: Part, room: Rect, s: i32) -> Option<Rect> {
    let img = st.part(grip)?;
    let f = img.frame(State::Normal);
    let (fw, fh) = (f.width / img.scale, f.height / img.scale);
    let s = s.max(1);
    let (w, h) = (fw * s, fh * s);
    (room.width >= w && room.height >= h).then(|| {
        Rect::new(
            room.x + s * ((room.width / s - fw) / 2),
            room.y + s * ((room.height / s - fh) / 2),
            w,
            h,
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::blue;
    use crate::classic;
    use crate::classic::widgets::{Arrow, ScrollHit};
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::{draw_hscrollbar, draw_scrollbar};
    use crate::pointer::{Interaction, Lit};
    use crate::raster::{Canvas, Rect};
    use crate::style::{Part, State};

    fn glyph() -> u32 {
        art(Part::ScrollArrowUp, State::Normal, 7, 7)
    }

    fn glyph_greyed() -> u32 {
        art(Part::ScrollArrowUp, State::Disabled, 7, 7)
    }

    fn grip_light() -> u32 {
        art(Part::ScrollGripV, State::Normal, 0, 0)
    }

    fn grip_dark() -> u32 {
        art(Part::ScrollGripV, State::Normal, 6, 1)
    }

    fn edge() -> u32 {
        art(Part::ScrollTrackV, State::Normal, 0, 0)
    }

    fn track_middle() -> u32 {
        art(Part::ScrollTrackV, State::Normal, 8, 0)
    }

    fn held_edge() -> u32 {
        art(Part::ScrollTrackV, State::Pressed, 0, 0)
    }

    fn held_middle() -> u32 {
        art(Part::ScrollTrackV, State::Pressed, 8, 0)
    }

    #[test]
    fn a_classic_look_paints_the_classic_scrollbars_whatever_the_pointer_does() {
        let theme = theme_for(Interface::Classic);
        let p = theme.paint();
        let lit = Some((ScrollHit::Thumb, Interaction::Pressed));
        for (top, visible) in [(3, 5), (0, 20)] {
            let sb = Rect::new(0, 0, 16, 100);
            let (mut a, mut b) = (Canvas::new(20, 110), Canvas::new(20, 110));
            draw_scrollbar(&mut a, &p, sb, top, visible, 20, lit);
            classic::widgets::draw_scrollbar(&mut b, &p, sb, top, visible, 20);
            assert!(a == b, "a Classic vertical bar paints Classic's own");
            let hb = Rect::new(0, 0, 100, 16);
            let (mut a, mut b) = (Canvas::new(110, 20), Canvas::new(110, 20));
            draw_hscrollbar(&mut a, &p, hb, top, visible, 20, lit);
            classic::widgets::draw_hscrollbar(&mut b, &p, hb, top, visible, 20);
            assert!(a == b, "a Classic horizontal bar paints Classic's own");
        }
    }

    fn vbar(w: i32, h: i32, top: usize, visible: usize) -> Canvas {
        vbar_lit(w, h, top, visible, None)
    }

    fn vbar_lit(w: i32, h: i32, top: usize, visible: usize, lit: Lit<ScrollHit>) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::new(w as u32, h as u32);
        draw_scrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, w, h),
            top,
            visible,
            20,
            lit,
        );
        c
    }

    fn hbar(w: i32, h: i32, left: usize, visible: usize) -> Canvas {
        hbar_lit(w, h, left, visible, None)
    }

    fn hbar_lit(w: i32, h: i32, left: usize, visible: usize, lit: Lit<ScrollHit>) -> Canvas {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::new(w as u32, h as u32);
        draw_hscrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, w, h),
            left,
            visible,
            20,
            lit,
        );
        c
    }

    #[track_caller]
    fn assert_arrow(c: &Canvas, at: (i32, i32), part: Part, state: State) {
        for y in 0..17 {
            for x in 0..17 {
                assert_eq!(
                    c.get(at.0 + x, at.1 + y),
                    art(part, state, x, y),
                    "{part:?} {state:?} ({x}, {y})"
                );
            }
        }
    }

    fn art(part: Part, state: State, x: i32, y: i32) -> u32 {
        let s = blue::style();
        let img = s.part(part).unwrap();
        let f = img.frame(state);
        img.atlas.get(f.x + x, f.y + y)
    }

    fn luma(p: u32) -> u32 {
        (((p >> 16) & 0xFF) * 30 + ((p >> 8) & 0xFF) * 59 + (p & 0xFF) * 11) / 100
    }

    fn count(c: &Canvas, r: Rect, ink: u32) -> usize {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    }

    #[test]
    fn a_blue_vertical_bar_draws_its_arrows_17_square_at_both_ends() {
        let c = vbar(17, 120, 3, 5);
        for (x, y) in [(0, 0), (16, 0), (0, 16), (16, 16), (8, 5), (3, 9)] {
            assert_eq!(
                c.get(x, y),
                art(Part::ScrollArrowUp, State::Normal, x, y),
                "up arrow ({x}, {y})"
            );
        }
        for (x, y) in [(0, 0), (16, 0), (0, 16), (16, 16), (8, 11)] {
            assert_eq!(
                c.get(x, 103 + y),
                art(Part::ScrollArrowDown, State::Normal, x, y),
                "down arrow ({x}, {y})"
            );
        }
        assert_eq!(c.get(7, 7), glyph(), "the up chevron's left arm");
        assert_eq!(c.get(7, 103 + 9), glyph(), "the down chevron's left arm");
        assert_eq!(c.get(0, 1), edge(), "a 1 px edge runs down the left");
    }

    #[test]
    fn a_blue_bars_lit_arrow_takes_its_hot_or_pressed_frame() {
        let (up, down) = ((0, 0), (0, 103));
        for (hit, how, state) in [
            (ScrollHit::Back, Interaction::Hot, State::Hot),
            (ScrollHit::Back, Interaction::Pressed, State::Pressed),
            (ScrollHit::Forward, Interaction::Hot, State::Hot),
            (ScrollHit::Forward, Interaction::Pressed, State::Pressed),
        ] {
            let c = vbar_lit(17, 120, 3, 5, Some((hit, how)));
            let (lit, rest, lit_part, rest_part) = match hit {
                ScrollHit::Back => (up, down, Part::ScrollArrowUp, Part::ScrollArrowDown),
                _ => (down, up, Part::ScrollArrowDown, Part::ScrollArrowUp),
            };
            assert_arrow(&c, lit, lit_part, state);
            assert_arrow(&c, rest, rest_part, State::Normal);
            for (x, y) in [(8, 2), (3, 8)] {
                assert_ne!(
                    c.get(lit.0 + x, lit.1 + y),
                    art(lit_part, State::Normal, x, y),
                    "{hit:?} {how:?}: the face at ({x}, {y}) changes"
                );
            }
        }
    }

    #[test]
    fn a_blue_horizontal_bars_lit_arrow_takes_its_hot_or_pressed_frame() {
        let (left, right) = ((0, 0), (103, 0));
        let hot = hbar_lit(120, 17, 3, 5, Some((ScrollHit::Back, Interaction::Hot)));
        assert_arrow(&hot, left, Part::ScrollArrowLeft, State::Hot);
        assert_arrow(&hot, right, Part::ScrollArrowRight, State::Normal);
        let rest = |part, x, y| art(part, State::Normal, x, y);
        assert_ne!(
            hot.get(2, 8),
            rest(Part::ScrollArrowLeft, 2, 8),
            "the hot face"
        );
        assert_ne!(
            hot.get(8, 3),
            rest(Part::ScrollArrowLeft, 8, 3),
            "the hot top"
        );
        let held = hbar_lit(
            120,
            17,
            3,
            5,
            Some((ScrollHit::Forward, Interaction::Pressed)),
        );
        assert_arrow(&held, right, Part::ScrollArrowRight, State::Pressed);
        assert_arrow(&held, left, Part::ScrollArrowLeft, State::Normal);
        assert_ne!(
            held.get(103 + 2, 8),
            rest(Part::ScrollArrowRight, 2, 8),
            "the held right arrow's face"
        );
        assert_ne!(
            held.get(103 + 8, 3),
            rest(Part::ScrollArrowRight, 8, 3),
            "the held right arrow's top"
        );
    }

    #[test]
    fn a_blue_thumb_under_the_pointer_or_held_is_lighter() {
        let t = Rect::new(0, 17, 17, 22);
        let at_rest = vbar(17, 120, 0, 5);
        let hot = vbar_lit(17, 120, 0, 5, Some((ScrollHit::Thumb, Interaction::Hot)));
        let held = vbar_lit(
            17,
            120,
            0,
            5,
            Some((ScrollHit::Thumb, Interaction::Pressed)),
        );
        assert!(hot == held, "a held thumb draws as a hot one");
        for y in [0, 1, 2, 3, 4, 17, 18, 19, 20, 21] {
            for x in [0, 1, 2, 8, 15, 16] {
                assert_eq!(
                    hot.get(t.x + x, t.y + y),
                    art(Part::ScrollThumbV, State::Hot, x, y),
                    "hot thumb ({x}, {y})"
                );
            }
        }
        for y in [3, 8] {
            assert!(
                luma(hot.get(t.x + 8, t.y + y)) > luma(at_rest.get(t.x + 8, t.y + y)),
                "the hot thumb is lighter at row {y}"
            );
        }
        assert_eq!(
            hot.get(t.x + 5, t.y + 7),
            art(Part::ScrollGripV, State::Hot, 0, 0),
            "the grip's first ridge lights with the thumb"
        );
        for y in (0..17).chain(39..120) {
            assert_eq!(hot.get(8, y), at_rest.get(8, y), "row {y} is as at rest");
        }
    }

    #[test]
    fn a_held_page_draws_the_track_pressed_up_to_the_thumb() {
        let rest = vbar(17, 120, 8, 5);
        let thumb_y = (17..103)
            .find(|&y| rest.get(8, y) != rest.get(8, 17))
            .expect("the thumb");
        let back = vbar_lit(
            17,
            120,
            8,
            5,
            Some((ScrollHit::PageBack, Interaction::Pressed)),
        );
        for y in [17, 30, thumb_y - 1] {
            assert_eq!(back.get(0, y), held_edge(), "row {y}: the pressed edge");
            assert_eq!(back.get(8, y), held_middle(), "row {y}: the pressed middle");
            assert_eq!(back.get(16, y), held_edge(), "row {y}: the pressed edge");
        }
        for y in [thumb_y, 100, 102] {
            assert_eq!(back.get(8, y), rest.get(8, y), "row {y}: as at rest");
        }
        let forward = vbar_lit(
            17,
            120,
            8,
            5,
            Some((ScrollHit::PageForward, Interaction::Pressed)),
        );
        assert_eq!(
            forward.get(8, 102),
            held_middle(),
            "the page below the thumb"
        );
        assert_eq!(
            forward.get(8, 17),
            rest.get(8, 17),
            "the page above: at rest"
        );
        let hot = vbar_lit(17, 120, 8, 5, Some((ScrollHit::PageBack, Interaction::Hot)));
        assert!(hot == rest, "the track has no hot look");
        let h = hbar_lit(
            120,
            17,
            8,
            5,
            Some((ScrollHit::PageBack, Interaction::Pressed)),
        );
        assert_eq!(h.get(17, 0), held_edge(), "the pressed edge row");
        assert_eq!(h.get(17, 8), held_middle(), "the pressed middle row");
    }

    #[test]
    fn a_fully_shown_blue_bar_ignores_the_pointer() {
        let rest = vbar(17, 120, 0, 20);
        for hit in [ScrollHit::Back, ScrollHit::Forward, ScrollHit::PageBack] {
            for how in [Interaction::Hot, Interaction::Pressed] {
                assert!(
                    vbar_lit(17, 120, 0, 20, Some((hit, how))) == rest,
                    "{hit:?} {how:?}"
                );
            }
        }
    }

    #[test]
    fn a_blue_track_fades_through_f9f9f7_between_eeede5_edges() {
        let c = vbar(17, 120, 15, 5);
        for y in [17, 40, 60] {
            assert_eq!(c.get(0, y), edge(), "row {y}");
            assert_eq!(c.get(8, y), track_middle(), "row {y}");
            assert_eq!(c.get(16, y), edge(), "row {y}");
        }
    }

    #[test]
    fn a_blue_thumb_22_long_draws_its_own_frame_with_the_grip_7_rows_down() {
        let c = vbar(17, 120, 0, 5);
        let t = Rect::new(0, 17, 17, 22);
        for y in [0, 1, 2, 3, 4, 17, 18, 19, 20, 21] {
            for x in [0, 1, 2, 8, 15, 16] {
                assert_eq!(
                    c.get(t.x + x, t.y + y),
                    art(Part::ScrollThumbV, State::Normal, x, y),
                    "thumb ({x}, {y})"
                );
            }
        }
        assert_eq!(
            c.get(t.x + 5, t.y + 7),
            grip_light(),
            "the grip's first ridge"
        );
        assert_eq!(c.get(t.x + 11, t.y + 8), grip_dark());
        assert_eq!(
            c.get(t.x + 5, t.y + 6),
            art(Part::ScrollThumbV, State::Normal, 5, 6)
        );
        assert_eq!(count(&c, t, grip_light()), 24, "four light ridges of 6");
        assert_eq!(count(&c, t, grip_dark()), 24, "four dark ridges of 6");
    }

    #[test]
    fn a_blue_thumb_shows_its_grip_from_14_px_long_and_not_at_13() {
        let st = blue::style();
        for (len, grip) in [(10, false), (13, false), (14, true), (17, true)] {
            let t = Rect::new(0, 0, 17, len);
            let mut c = Canvas::new(17, len as u32);
            st.draw(&mut c, t, Part::ScrollThumbV, State::Normal, 1);
            if let Some(g) = super::grip_rect(
                &st,
                Part::ScrollGripV,
                st.content_rect(Part::ScrollThumbV, t, 1),
                1,
            ) {
                st.draw(&mut c, g, Part::ScrollGripV, State::Normal, 1);
            }
            assert_eq!(count(&c, t, grip_light()) == 24, grip, "{len} px thumb");
        }
    }

    #[test]
    fn a_17_px_thumb_puts_its_odd_spare_row_below_the_grip() {
        let st = blue::style();
        let t = Rect::new(0, 0, 17, 17);
        let g = super::grip_rect(
            &st,
            Part::ScrollGripV,
            st.content_rect(Part::ScrollThumbV, t, 1),
            1,
        );
        assert_eq!(g, Some(Rect::new(5, 4, 7, 8)));
    }

    #[test]
    fn a_10_px_thumb_keeps_its_5_px_ends_and_nothing_between() {
        let st = blue::style();
        let mut c = Canvas::new(17, 10);
        st.draw(
            &mut c,
            Rect::new(0, 0, 17, 10),
            Part::ScrollThumbV,
            State::Normal,
            1,
        );
        for x in 0..17 {
            for y in 0..5 {
                assert_eq!(c.get(x, y), art(Part::ScrollThumbV, State::Normal, x, y));
                assert_eq!(
                    c.get(x, 5 + y),
                    art(Part::ScrollThumbV, State::Normal, x, 17 + y)
                );
            }
        }
    }

    #[test]
    fn a_fully_shown_blue_vertical_bar_greys_both_arrows_and_draws_no_thumb() {
        let c = vbar(17, 60, 0, 20);
        assert_eq!(c.get(7, 7), glyph_greyed(), "the up chevron greys");
        assert_eq!(c.get(7, 43 + 9), glyph_greyed(), "the down chevron greys");
        for y in 17..43 {
            assert_eq!(c.get(8, y), track_middle(), "only track at row {y}");
        }
    }

    #[test]
    fn a_blue_horizontal_bar_draws_its_arrows_thumb_and_an_8x7_grip() {
        let c = hbar(150, 17, 3, 5);
        for (x, y) in [(0, 0), (16, 0), (0, 16), (16, 16), (8, 8)] {
            assert_eq!(
                c.get(x, y),
                art(Part::ScrollArrowLeft, State::Normal, x, y),
                "left arrow ({x}, {y})"
            );
            assert_eq!(
                c.get(133 + x, y),
                art(Part::ScrollArrowRight, State::Normal, x, y),
                "right arrow ({x}, {y})"
            );
        }
        assert_eq!(c.get(20, 0), edge(), "a 1 px edge runs along the top");
        assert_eq!(c.get(20, 8), track_middle(), "the track's middle row");
        let track = Rect::new(17, 0, 116, 17);
        assert_eq!(count(&c, track, grip_light()), 24, "four light ridges of 6");
        assert_eq!(count(&c, track, grip_dark()), 24, "four dark ridges of 6");
    }

    #[test]
    fn a_fully_shown_blue_horizontal_bar_greys_both_arrows() {
        let full = hbar(150, 17, 0, 20);
        assert_arrow(&full, (0, 0), Part::ScrollArrowLeft, State::Disabled);
        assert_arrow(&full, (133, 0), Part::ScrollArrowRight, State::Disabled);
        assert_eq!(full.get(7, 7), glyph_greyed(), "the left chevron greys");
        assert_eq!(full.get(75, 8), track_middle(), "no thumb, only track");
    }

    #[test]
    fn a_16_px_blue_bar_keeps_every_glyph_pixel_and_its_edges() {
        let (thick, thin) = (vbar(17, 120, 3, 5), vbar(16, 120, 3, 5));
        let (up17, up16) = (Rect::new(0, 0, 17, 17), Rect::new(0, 0, 16, 16));
        let (dn17, dn16) = (Rect::new(0, 103, 17, 17), Rect::new(0, 104, 16, 16));
        assert_eq!(count(&thin, up16, glyph()), count(&thick, up17, glyph()));
        assert_eq!(count(&thin, dn16, glyph()), count(&thick, dn17, glyph()));
        assert_eq!(thin.get(0, 1), edge());
        assert_eq!(thin.get(0, 90), edge(), "the track's left edge");
        assert_eq!(thin.get(15, 90), edge(), "the track's right edge");
        let (thick, thin) = (hbar(150, 17, 3, 5), hbar(150, 16, 3, 5));
        let (l17, l16) = (Rect::new(0, 0, 17, 17), Rect::new(0, 0, 16, 16));
        let (r17, r16) = (Rect::new(133, 0, 17, 17), Rect::new(134, 0, 16, 16));
        assert_eq!(count(&thin, l16, glyph()), count(&thick, l17, glyph()));
        assert_eq!(count(&thin, r16, glyph()), count(&thick, r17, glyph()));
        let grip = |c: &Canvas| count(c, c.bounds(), grip_light());
        assert_eq!(grip(&thin), grip(&thick), "the grip keeps its size");
    }

    #[test]
    fn a_16_px_blue_thumb_drops_one_of_its_twin_middle_columns() {
        let st = blue::style();
        let mut c = Canvas::new(16, 22);
        st.draw(
            &mut c,
            Rect::new(0, 0, 16, 22),
            Part::ScrollThumbV,
            State::Normal,
            1,
        );
        for y in 0..22 {
            for x in 0..16 {
                let from = if x < 7 { x } else { x + 1 };
                assert_eq!(
                    c.get(x, y),
                    art(Part::ScrollThumbV, State::Normal, from, y),
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn a_blue_bar_on_a_scale_2_canvas_draws_its_chevron_solid() {
        let theme = theme_for(Interface::Blue);
        let mut c = Canvas::with_scale(17, 120, 2);
        draw_scrollbar(
            &mut c,
            &theme.paint(),
            Rect::new(0, 0, 17, 120),
            3,
            5,
            20,
            None,
        );
        for (x, y) in [(7, 7), (9, 7), (7, 103 + 9), (9, 103 + 9)] {
            assert_eq!(c.get(x, y), glyph(), "({x}, {y})");
        }
    }

    #[test]
    fn a_style_with_only_the_track_part_falls_back_to_classic_whole() {
        for (track, horizontal) in [(Part::ScrollTrackV, false), (Part::ScrollTrackH, true)] {
            let look = crate::look::styled::fixture::one_part_look(track);
            let cs = crate::theme::ColorScheme::default();
            let p = crate::look::Paint {
                cs: &cs,
                look: &look,
                s: 1,
                back: crate::look::Backdrop::Face,
                active: true,
            };
            let (mut a, mut b) = (Canvas::new(110, 110), Canvas::new(110, 110));
            if horizontal {
                let hb = Rect::new(0, 0, 100, 17);
                draw_hscrollbar(&mut a, &p, hb, 3, 5, 20, None);
                classic::widgets::draw_hscrollbar(&mut b, &p, hb, 3, 5, 20);
            } else {
                let sb = Rect::new(0, 0, 17, 100);
                draw_scrollbar(&mut a, &p, sb, 3, 5, 20, None);
                classic::widgets::draw_scrollbar(&mut b, &p, sb, 3, 5, 20);
            }
            assert!(a == b, "a style with only {track:?} paints Classic's bar");
        }
    }

    fn short_pair(
        interface: Interface,
        thick: i32,
        len: i32,
        horizontal: bool,
    ) -> (Canvas, Canvas) {
        let theme = theme_for(interface);
        let p = theme.paint();
        let (w, h) = if horizontal {
            (len, thick)
        } else {
            (thick, len)
        };
        let sb = Rect::new(2, 2, w, h);
        let half = len / 2;
        let (back, forward) = if horizontal {
            (Rect::new(2, 2, half, h), Rect::new(2 + half, 2, half, h))
        } else {
            (Rect::new(2, 2, w, half), Rect::new(2, 2 + half, w, half))
        };
        let (mut bar, mut arrows) = (
            Canvas::new(w as u32 + 4, h as u32 + 4),
            Canvas::new(w as u32 + 4, h as u32 + 4),
        );
        if horizontal {
            draw_hscrollbar(&mut bar, &p, sb, 0, 5, 20, None);
        } else {
            draw_scrollbar(&mut bar, &p, sb, 0, 5, 20, None);
        }
        let (a, b) = if horizontal {
            (Arrow::Left, Arrow::Right)
        } else {
            (Arrow::Up, Arrow::Down)
        };
        match p.look.style() {
            Some(st) => {
                let (pa, pb) = if horizontal {
                    (Part::ScrollArrowLeft, Part::ScrollArrowRight)
                } else {
                    (Part::ScrollArrowUp, Part::ScrollArrowDown)
                };
                st.draw(&mut arrows, back, pa, State::Normal, 1);
                st.draw(&mut arrows, forward, pb, State::Normal, 1);
            }
            None => {
                classic::widgets::draw_arrow_button(&mut arrows, &p, back, a, true);
                classic::widgets::draw_arrow_button(&mut arrows, &p, forward, b, true);
            }
        }
        (bar, arrows)
    }

    #[test]
    fn a_bar_shorter_than_two_arrows_draws_two_half_arrows() {
        for (interface, thick) in [(Interface::Classic, 16), (Interface::Blue, 17)] {
            for len in [20, 10] {
                for horizontal in [false, true] {
                    let (bar, arrows) = short_pair(interface, thick, len, horizontal);
                    assert!(
                        bar == arrows,
                        "{interface:?} {len} px bar (horizontal {horizontal}) is its two arrows"
                    );
                }
            }
        }
    }

    #[test]
    fn blue_bars_at_paint_scale_2_double_every_pixel() {
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            let v = Rect::new(2, 2, 17, 114).scaled(p.s);
            crate::look::widgets::draw_scrollbar(
                c,
                p,
                v,
                3,
                5,
                20,
                Some((ScrollHit::Thumb, Interaction::Hot)),
            );
            let h = Rect::new(22, 2, 114, 17).scaled(p.s);
            crate::look::widgets::draw_hscrollbar(
                c,
                p,
                h,
                3,
                5,
                20,
                Some((ScrollHit::PageForward, Interaction::Pressed)),
            );
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(145, 125, draw),
            "a vertical bar with its thumb Hot and a horizontal one paging at s = 2 are their s = 1 pixels doubled"
        );
    }

    #[test]
    fn every_bar_thickness_doubles_at_paint_scale_2_in_each_variant() {
        for v in crate::blue::Variant::ALL {
            for thick in [16, 17] {
                for lit in [
                    None,
                    Some((ScrollHit::Thumb, Interaction::Hot)),
                    Some((ScrollHit::Back, Interaction::Pressed)),
                    Some((ScrollHit::PageBack, Interaction::Pressed)),
                ] {
                    let draw = |c: &mut Canvas, p: &crate::look::Paint| {
                        let r = Rect::new(2, 2, thick, 114).scaled(p.s);
                        crate::look::widgets::draw_scrollbar(c, p, r, 3, 5, 20, lit);
                        let r = Rect::new(22, 2, 114, thick).scaled(p.s);
                        crate::look::widgets::draw_hscrollbar(c, p, r, 3, 5, 20, lit);
                    };
                    assert!(
                        crate::look::styled::fixture::variant_doubles_at_paint_scale_2(
                            v, 145, 125, draw
                        ),
                        "{v:?}: {thick} px bars ({lit:?}) at s = 2 are their s = 1 pixels doubled"
                    );
                }
            }
        }
    }
}
