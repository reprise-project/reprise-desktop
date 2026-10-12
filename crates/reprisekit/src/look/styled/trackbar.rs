//! The trackbar through the style's track and thumb parts, with flat ticks placed by
//! [`TrackMarks`](crate::style::TrackMarks); the vertical trackbar uses the `V` parts.

use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::trackbar::{Layout, vertical_ticks};

/// `false` when `st` lacks the track or thumb part; horizontal only.
pub fn draw_trackbar(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    lay: &Layout,
    enabled: bool,
    focused: bool,
    thumb: Interaction,
) -> bool {
    if !st.has_parts(&[Part::TrackbarTrack, Part::TrackbarThumb]) {
        return false;
    }
    let s = p.s;
    let m = st.track_marks;
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    let ch = lay.channel;
    let track = Rect::new(ch.x, ch.y + m.drop, ch.width, ch.height);
    st.draw(c, track.scaled(s), Part::TrackbarTrack, state, p.s);
    let base = lay.bounds.y + m.tick_y;
    for (i, &tx) in lay.ticks.iter().enumerate() {
        let end = i == 0 || i + 1 == lay.ticks.len();
        if !lay.ticked || (m.sparse && i % lay.tick_every != 0 && !end) {
            continue;
        }
        let len = if end { m.end_len } else { m.tick_len };
        c.fill_rect(Rect::new(tx * s, base * s, s, len * s), m.ink);
    }
    let thumb = thumb_state(p, enabled, focused, thumb);
    st.draw(c, lay.thumb.scaled(s), Part::TrackbarThumb, thumb, p.s);
    true
}

pub fn draw_vertical_trackbar(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    lay: &Layout,
    enabled: bool,
    focused: bool,
    thumb: Interaction,
) -> bool {
    if !st.has_parts(&[Part::TrackbarTrackV, Part::TrackbarThumbV]) {
        return false;
    }
    let s = p.s;
    let m = st.track_marks;
    let state = if enabled {
        State::Normal
    } else {
        State::Disabled
    };
    st.draw(c, lay.channel.scaled(s), Part::TrackbarTrackV, state, s);
    let every = if m.sparse { lay.tick_every } else { 1 };
    for tick in vertical_ticks(lay, every, m.tick_len, m.end_len_v) {
        c.fill_rect(tick.scaled(s), m.ink);
    }
    let thumb = thumb_state(p, enabled, focused, thumb);
    st.draw(c, lay.thumb.scaled(s), Part::TrackbarThumbV, thumb, s);
    true
}

fn thumb_state(p: &Paint, enabled: bool, focused: bool, thumb: Interaction) -> State {
    match super::state(enabled, thumb) {
        State::Normal if focused && p.active => State::Focused,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use crate::classic;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_trackbar;
    use crate::pointer::Interaction;
    use crate::raster::{Canvas, Rect};
    use crate::style::TrackMarks;
    use crate::style::{Part, State};
    use crate::theme::Color;
    use crate::trackbar;

    const SENTINEL: Color = Color::rgb(0x12, 0x34, 0x56);
    const TRACK_DROP: i32 = TrackMarks::LOW.drop;
    const TICK_Y: i32 = TrackMarks::LOW.tick_y;
    const TICK_LEN: i32 = TrackMarks::LOW.tick_len;
    const TICK: Color = crate::blue::scheme::NAVY.inks.track_tick;

    fn blue(enabled: bool) -> (Canvas, trackbar::Layout) {
        blue_lit(enabled, Interaction::Idle)
    }

    fn blue_lit(enabled: bool, thumb: Interaction) -> (Canvas, trackbar::Layout) {
        let theme = theme_for(Interface::Blue);
        let lay = trackbar::layout(&trackbar::TrackBar::new(13, 18), 69, 315, 71);
        let mut c = Canvas::new(200, 400);
        c.fill_rect(c.bounds(), SENTINEL);
        draw_trackbar(&mut c, &theme.paint(), &lay, enabled, false, thumb);
        (c, lay)
    }

    #[test]
    fn a_classic_look_paints_the_classic_painter() {
        let theme = theme_for(Interface::Classic);
        let tb = trackbar::TrackBar::new(13, 18);
        let lay = trackbar::layout(&tb, 4, 4, 71);
        for enabled in [true, false] {
            let (mut a, mut b) = (Canvas::new(90, 40), Canvas::new(90, 40));
            draw_trackbar(
                &mut a,
                &theme.paint(),
                &lay,
                enabled,
                false,
                Interaction::Idle,
            );
            classic::trackbar::draw(&mut b, &theme.paint(), &lay, enabled);
            assert!(a == b, "a Classic look paints the Classic painter");
        }
    }

    fn lum(px: u32) -> u32 {
        ((px >> 16) & 0xFF) + ((px >> 8) & 0xFF) + (px & 0xFF)
    }

    #[test]
    fn a_blue_track_runs_one_row_under_the_channel() {
        let (c, lay) = blue(true);
        let ch = lay.channel;
        let y = ch.y + TRACK_DROP;
        assert_eq!(y, 323, "the track's top row is 323");
        assert_eq!(c.get(80, y - 1), SENTINEL.argb(), "nothing above the track");
        let rows: Vec<u32> = (0..4).map(|i| lum(c.get(80, y + i))).collect();
        assert!(rows[0] < 3 * 0xB0, "the top line is dark grey");
        assert!(rows[1] > 3 * 0xE0 && rows[2] > 3 * 0xE0, "two pale rows");
        assert!(
            rows[3] >= rows[1] && rows[3] >= rows[2],
            "the bottom row is the lightest"
        );
        assert_eq!(c.get(80, y + 4), SENTINEL.argb(), "nothing below the track");
        assert!(
            lum(c.get(ch.x, y + 2)) < 3 * 0xB0,
            "the left end's edge is dark"
        );
        let r = ch.right() - 1;
        assert!(
            lum(c.get(r, y + 1)) > 3 * 0xE0,
            "the right end's edge is light"
        );
        assert_eq!(
            c.get(r + 1, y + 1),
            SENTINEL.argb(),
            "the track stops at the channel's right"
        );
    }

    fn green(px: u32) -> bool {
        let (r, g, b) = ((px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF);
        g > r + 0x30 && g > b + 0x30
    }

    #[test]
    fn a_blue_thumb_is_an_11x21_green_capped_pointer() {
        let (c, lay) = blue(true);
        let t = lay.thumb;
        assert_eq!(t, Rect::new(115, 315, 11, 21), "the thumb's rect is 11x21");
        let at = |x: i32, y: i32| c.get(t.x + x, t.y + y);
        let none = SENTINEL.argb();
        assert!(green(at(5, 1)) && green(at(5, 3)), "the cap is green");
        assert!(lum(at(5, 1)) > lum(at(5, 3)), "the cap deepens down");
        assert!(lum(at(5, 4)) > 3 * 0xE8, "the body is pale");
        assert!(lum(at(10, 8)) < lum(at(9, 8)), "the right edge is darker");
        assert!(
            lum(at(9, 8)) < lum(at(5, 8)),
            "the body shades to the right"
        );
        assert_eq!(at(0, 17), none, "the point starts in from the side");
        assert!(green(at(3, 17)), "the point's left edge is green");
        assert!(green(at(7, 17)), "the point's right edge is green");
        assert!(at(5, 20) != none, "the tip is on row 20");
        assert_eq!(at(3, 20), none, "nothing beside the tip");
        assert_eq!(at(5, 21), none, "nothing below the tip");
    }

    #[test]
    fn a_blue_trackbar_draws_short_grey_ticks() {
        let (c, lay) = blue(true);
        let top = lay.bounds.y + TICK_Y;
        assert_eq!(top, 338, "the ticks start on row 338");
        for (i, &tx) in lay.ticks.iter().enumerate() {
            assert_eq!(
                c.get(tx, top - 1),
                SENTINEL.argb(),
                "tick {i} starts on row 338"
            );
            for dy in 0..TICK_LEN {
                assert_eq!(
                    c.get(tx, top + dy),
                    TICK.argb(),
                    "tick {i} row {dy} is grey"
                );
            }
            assert_eq!(
                c.get(tx, top + TICK_LEN),
                SENTINEL.argb(),
                "tick {i} is 3 rows long"
            );
            assert_eq!(c.get(tx + 1, top), SENTINEL.argb(), "tick {i} is 1 px wide");
        }
        assert!(
            top + TICK_LEN <= lay.bounds.bottom(),
            "the ticks stay inside the trackbar"
        );
    }

    #[test]
    fn a_disabled_blue_trackbar_greys_only_its_thumb() {
        let (on, lay) = blue(true);
        let (off, _) = blue(false);
        let t = lay.thumb;
        let at = |x: i32, y: i32| off.get(t.x + x, t.y + y);
        let grey = |px: u32| {
            let ch = [(px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF];
            ch.iter().max().unwrap() - ch.iter().min().unwrap() < 0x20
        };
        for (x, y) in [(5, 1), (5, 4), (10, 8), (3, 17), (5, 20)] {
            assert!(
                grey(at(x, y)),
                "the greyed thumb at ({x}, {y}) has no colour"
            );
        }
        assert!(
            lum(at(10, 8)) < lum(at(5, 8)),
            "the greyed right edge is darker"
        );
        for x in 0..200 {
            for y in 0..400 {
                if !t.contains(x, y) {
                    assert_eq!(
                        off.get(x, y),
                        on.get(x, y),
                        "({x}, {y}) is unchanged when greyed"
                    );
                }
            }
        }
    }

    #[test]
    fn a_blue_thumb_lights_under_the_pointer_and_darkens_held() {
        let (normal, lay) = blue(true);
        let (hot, _) = blue_lit(true, Interaction::Hot);
        let (pressed, _) = blue_lit(true, Interaction::Pressed);
        let t = lay.thumb;
        let at = |c: &Canvas, x: i32, y: i32| c.get(t.x + x, t.y + y);
        let orange = |px: u32| {
            let (r, g, b) = ((px >> 16) & 0xFF, (px >> 8) & 0xFF, px & 0xFF);
            r > g + 0x18 && g > b + 0x30
        };
        for y in 1..4 {
            assert!(orange(at(&hot, 5, y)), "the hot cap's row {y} is orange");
            let (p, n) = (at(&pressed, 5, y), at(&normal, 5, y));
            assert!(
                green(p) && lum(p) < lum(n),
                "the pressed cap's row {y} is a deeper green"
            );
        }
        let theme = theme_for(Interface::Blue);
        let st = theme.look.style().expect("Blue carries a style");
        for (c, state) in [(&hot, State::Hot), (&pressed, State::Pressed)] {
            let mut want = normal.clone();
            want.fill_rect(t, SENTINEL);
            st.draw(&mut want, t, Part::TrackbarThumb, state, 1);
            assert!(*c == want, "only the thumb changes, to its {state:?} frame");
        }
        for ptr in [Interaction::Hot, Interaction::Pressed] {
            assert!(
                blue_lit(false, ptr).0 == blue(false).0,
                "greyed under {ptr:?}"
            );
        }
    }

    #[test]
    fn a_blue_thumb_keeps_its_edge_hot_and_held() {
        let theme = theme_for(Interface::Blue);
        let paint = theme.paint();
        let st = paint.look.style().expect("Blue carries a style");
        let r = Rect::new(0, 0, 11, 21);
        let draw = |state| {
            let mut c = Canvas::new(11, 21);
            st.draw(&mut c, r, Part::TrackbarThumb, state, 1);
            c
        };
        let (normal, hot, pressed) = (draw(State::Normal), draw(State::Hot), draw(State::Pressed));
        assert_eq!(
            hot.get(5, 0),
            normal.get(5, 0),
            "the hot top edge is as at rest"
        );
        assert_eq!(
            pressed.get(10, 8),
            normal.get(10, 8),
            "the pressed right edge is as at rest"
        );
    }

    #[test]
    fn a_blue_trackbar_on_a_scale_2_canvas_doubles_every_pixel() {
        let theme = theme_for(Interface::Blue);
        let lay = trackbar::layout(&trackbar::TrackBar::new(13, 18), 4, 4, 71);
        let (mut one, mut two) = (Canvas::new(90, 40), Canvas::with_scale(90, 40, 2));
        one.fill_rect(one.bounds(), theme.colors.button_face);
        two.fill_rect(two.bounds(), theme.colors.button_face);
        draw_trackbar(
            &mut one,
            &theme.paint(),
            &lay,
            true,
            false,
            Interaction::Idle,
        );
        draw_trackbar(
            &mut two,
            &theme.paint(),
            &lay,
            true,
            false,
            Interaction::Idle,
        );
        assert!(crate::look::styled::fixture::doubles(&one, &two));
    }

    #[test]
    fn a_style_with_only_the_track_part_falls_back_to_classic_whole() {
        let look = crate::look::styled::fixture::one_part_look(Part::TrackbarTrack);
        let cs = crate::theme::ColorScheme::default();
        let p = crate::look::Paint {
            cs: &cs,
            look: &look,
            s: 1,
            back: crate::look::Backdrop::Face,
            active: true,
            text: crate::theme::TextUnits::ONE,
        };
        let lay = trackbar::layout(&trackbar::TrackBar::new(13, 18), 4, 4, 71);
        let (mut a, mut b) = (Canvas::new(90, 40), Canvas::new(90, 40));
        draw_trackbar(&mut a, &p, &lay, true, false, Interaction::Idle);
        classic::trackbar::draw(&mut b, &p, &lay, true);
        assert!(
            a == b,
            "a style with no thumb part paints Classic's trackbar"
        );
    }

    #[test]
    fn a_blue_trackbar_at_paint_scale_2_doubles_every_pixel() {
        let lay = trackbar::layout(&trackbar::TrackBar::new(13, 18), 4, 4, 71);
        let draw = |c: &mut Canvas, p: &crate::look::Paint| {
            draw_trackbar(c, p, &lay, true, false, Interaction::Hot);
        };
        assert!(
            crate::look::styled::fixture::blue_doubles_at_paint_scale_2(90, 40, draw),
            "a trackbar with its thumb Hot at s = 2 is its s = 1 pixels doubled"
        );
    }

    #[test]
    fn blues_upright_end_ticks_are_4_px() {
        use crate::look::widgets::draw_vertical_trackbar;
        let theme = theme_for(Interface::Blue);
        let lay = trackbar::layout_vertical_with(&trackbar::TrackBar::new(4, 3), 4, 4, 81, 22);
        let mut c = Canvas::new(60, 100);
        c.fill_rect(c.bounds(), SENTINEL);
        draw_vertical_trackbar(&mut c, &theme.paint(), &lay, true, false, Interaction::Idle);
        let run =
            |y: i32, xs: std::ops::Range<i32>| xs.filter(|&x| c.get(x, y) == TICK.argb()).count();
        let (left, right) = (
            lay.bounds.x..lay.thumb.x,
            lay.thumb.right()..lay.bounds.right(),
        );
        for (i, len) in [(0, 4), (1, 3), (2, 4)] {
            let y = lay.ticks[i];
            assert_eq!(run(y, left.clone()), len, "tick {i}, left");
            assert_eq!(run(y, right.clone()), len, "tick {i}, right");
        }
        assert_eq!(TrackMarks::LOW.end_len, 3, "a horizontal end tick");
    }
}
