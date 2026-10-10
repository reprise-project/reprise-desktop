//! The combo box's drop button through [`Part::ComboDrop`]: the part's
//! frame drawn into the button's rect, its chevron part of the art. And
//! the open list's frame and shadow, from the style's
//! [`crate::style::DropList`].
//!
//! Blue's combo box lays the button out 15x17 (its `combo_drop` width),
//! and a 22 px combo one pixel taller (15x18). The atlas fixes a plain
//! seam column and a plain seam row for a nine-grid to stretch
//! (`crate::blue::controls::combodrop`), so a larger rect repeats those,
//! and every corner, edge and chevron pixel lands once.
//!
//! The Blue part's pixels are pinned where it is drawn; the tests here
//! hold the forwarder to the atlas, at scale 1 and 2.

use crate::look::Paint;
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{DropShadow, Part, Style};
use crate::theme::Color;

/// [`crate::look::widgets::draw_combo_drop`] through `st`, inside `r`;
/// `false` when `st` has no combo drop part. The white well around the
/// button is the combo frame's ([`super::field::draw_combo_frame`]). A
/// drop-down list (`editable` false) whose style has a
/// [`Part::ComboList`] draws nothing here: that part carries its chevron.
pub fn draw_combo_drop(
    st: &Style,
    c: &mut Canvas,
    p: &Paint,
    r: Rect,
    enabled: bool,
    ptr: Interaction,
    editable: bool,
) -> bool {
    if !editable && st.part(Part::ComboList).is_some() {
        return true;
    }
    if st.part(Part::ComboDrop).is_none() {
        return false;
    }
    st.draw(c, r, Part::ComboDrop, super::state(enabled, ptr), p.s);
    true
}

/// [`crate::look::widgets::draw_drop_list`] through `st`'s
/// [`crate::style::DropList`]: the shadow first, past the list's right and bottom
/// edges, then `window` across `r` inside the 1 px frame. `false` when
/// `st` has none.
pub fn draw_drop_list(st: &Style, c: &mut Canvas, p: &Paint, r: Rect) -> bool {
    let Some(d) = st.drop_list else {
        return false;
    };
    let s = p.s;
    if let Some(shadow) = d.shadow {
        draw_drop_shadow(c, shadow, r, s);
    }
    c.fill_rect(r, p.cs.window);
    let (w, h) = (r.width, r.height);
    c.fill_rect(Rect::new(r.x, r.y, w, s), d.top);
    c.fill_rect(Rect::new(r.x, r.bottom() - s, w, s), d.bottom);
    for x in [r.x, r.right() - s] {
        c.fill_rect(Rect::new(x, r.y + s, s, h - 2 * s), d.sides);
        c.fill_rect(Rect::new(x, r.y, s, s), d.top_corners);
        c.fill_rect(Rect::new(x, r.bottom() - s, s, s), d.bottom_corners);
    }
    true
}

/// `shadow` cast by a list in `r` (scaled by `s`), black over what lies
/// beneath, each device pixel the shade at its middle.
pub(crate) fn draw_drop_shadow(c: &mut Canvas, shadow: &DropShadow, r: Rect, s: i32) {
    let k = c.scale();
    let unit = s * k;
    let (w, h) = (r.width / s, r.height / s);
    let n = DropShadow::N as i32;
    let dev = r.scaled(k);
    // The right strip with the corner under it, then the bottom strip
    // left of the corner.
    let strips = [
        (dev.right(), dev.y, n, h + n, w, 0),
        (dev.x, dev.bottom(), w, n, 0, h),
    ];
    for (x, y, sw, sh, ox, oy) in strips {
        let (mw, mh) = ((sw * unit) as usize, (sh * unit) as usize);
        let mut mask = vec![0u8; mw * mh];
        let middle = |o: i32, i: usize| f64::from(o) + (i as f64 + 0.5) / f64::from(unit);
        let (fw, fh) = (f64::from(w), f64::from(h));
        for (j, row) in mask.chunks_mut(mw).enumerate() {
            let dy = middle(oy, j);
            for (i, a) in row.iter_mut().enumerate() {
                let v = shadow.alpha(fw, fh, middle(ox, i), dy);
                *a = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        let clip = c.bounds();
        c.blit_mask_over(x, y, mw, mh, &mask, Color::rgb(0, 0, 0), clip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue;
    use crate::classic;
    use crate::classic::widgets::Arrow;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::look::widgets::draw_combo_drop as look_draw_combo_drop;
    use crate::style::State;

    #[test]
    fn a_classic_look_paints_the_classic_down_arrow_button() {
        let theme = theme_for(Interface::Classic);
        let r = Rect::new(2, 2, 16, 17);
        for enabled in [true, false] {
            let (mut a, mut b) = (Canvas::new(20, 21), Canvas::new(20, 21));
            look_draw_combo_drop(&mut a, &theme.paint(), r, enabled, Interaction::Idle, false);
            classic::widgets::draw_arrow_button(&mut b, &theme.paint(), r, Arrow::Down, enabled);
            assert!(
                a == b,
                "a Classic combo drop button (enabled {enabled}) paints Classic's down arrow button, the only one Classic has"
            );
        }
    }

    #[test]
    fn the_blue_button_draws_nothing_outside_its_rect() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(3, 3, 15, 17);
        let under = crate::theme::Color::rgb(1, 2, 3);
        for enabled in [true, false] {
            let mut c = Canvas::new(21, 23);
            c.fill_rect(c.bounds(), under);
            look_draw_combo_drop(&mut c, &theme.paint(), r, enabled, Interaction::Idle, false);
            for y in 0..23 {
                for x in 0..21 {
                    if !r.contains(x, y) {
                        assert_eq!(c.get(x, y), under.argb(), "({x}, {y}) is untouched");
                    }
                }
            }
        }
    }

    /// The atlas pixel at (`x`, `y`) of the Blue combo drop button's
    /// `state` frame.
    fn px(state: State, x: i32, y: i32) -> u32 {
        let s = blue::style();
        let p = s.part(Part::ComboDrop).unwrap();
        let f = p.frame(state);
        p.atlas.get(f.x + x, f.y + y)
    }

    #[test]
    fn the_blue_combo_drop_button_is_15_by_17_px_a_frame() {
        let s = blue::style();
        let p = s.part(Part::ComboDrop).unwrap();
        assert_eq!(p.frame(State::Normal), Rect::new(0, 0, 15, 17));
        assert_eq!(p.frame(State::Hot), Rect::new(0, 17, 15, 17));
        assert_eq!(p.frame(State::Pressed), Rect::new(0, 34, 15, 17));
        assert_eq!(p.frame(State::Disabled), Rect::new(0, 51, 15, 17));
    }

    /// Through the forwarder at the 15x17 a combo lays it out: the Hot
    /// frame under the pointer, Pressed held; a greyed button stays
    /// greyed either way.
    #[test]
    fn the_blue_drop_button_draws_the_frame_of_its_state() {
        let theme = theme_for(Interface::Blue);
        let r = Rect::new(2, 2, 15, 17);
        let (cols, rows): (Vec<i32>, Vec<i32>) = ((0..15).collect(), (0..17).collect());
        let draw = |enabled, ptr| {
            let mut c = Canvas::new(20, 21);
            look_draw_combo_drop(&mut c, &theme.paint(), r, enabled, ptr, false);
            c
        };
        assert_drawn_from(
            &draw(true, Interaction::Idle),
            r,
            State::Normal,
            &cols,
            &rows,
        );
        assert_drawn_from(&draw(true, Interaction::Hot), r, State::Hot, &cols, &rows);
        assert_drawn_from(
            &draw(true, Interaction::Pressed),
            r,
            State::Pressed,
            &cols,
            &rows,
        );
        for ptr in [Interaction::Hot, Interaction::Pressed] {
            assert_drawn_from(&draw(false, ptr), r, State::Disabled, &cols, &rows);
        }
    }

    /// How many pixels of `r` on `c` are `ink`.
    fn ink_count(c: &Canvas, r: Rect, ink: u32) -> usize {
        (r.y..r.y + r.height)
            .flat_map(|y| (r.x..r.x + r.width).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .count()
    }

    /// Every pixel of `r` on `c` is the `state` frame's pixel at
    /// (`cols[x]`, `rows[y]`).
    fn assert_drawn_from(c: &Canvas, r: Rect, state: State, cols: &[i32], rows: &[i32]) {
        assert_eq!(
            (cols.len(), rows.len()),
            (r.width as usize, r.height as usize)
        );
        for (dy, &sy) in rows.iter().enumerate() {
            for (dx, &sx) in cols.iter().enumerate() {
                let (x, y) = (r.x + dx as i32, r.y + dy as i32);
                assert_eq!(
                    c.get(x, y),
                    px(state, sx, sy),
                    "{state:?} at ({dx}, {dy}) is art pixel ({sx}, {sy})"
                );
            }
        }
    }

    /// Art columns 0, 1, then 2 twice, then 3..=14: the one extra column
    /// repeats the plain seam column 2.
    const COLS_16: [i32; 16] = [0, 1, 2, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];

    const STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

    #[test]
    fn a_15_by_17_button_draws_its_art_one_to_one() {
        let s = blue::style();
        let r = Rect::new(2, 2, 15, 17);
        let (cols, rows): (Vec<i32>, Vec<i32>) = ((0..15).collect(), (0..17).collect());
        for state in STATES {
            let mut c = Canvas::new(20, 21);
            s.draw(&mut c, r, Part::ComboDrop, state, 1);
            assert_drawn_from(&c, r, state, &cols, &rows);
        }
    }

    #[test]
    fn a_15_by_18_button_repeats_only_its_plain_seam_row() {
        let s = blue::style();
        let r = Rect::new(2, 2, 15, 18);
        // Art rows 0..=3, then 4 twice, then 5..=16: row 4 is plain face
        // above the chevron, so the rings and the corners stay 1 px.
        let cols: Vec<i32> = (0..15).collect();
        let rows: Vec<i32> = (0..5).chain(4..17).collect();
        for state in STATES {
            let mut c = Canvas::new(20, 22);
            s.draw(&mut c, r, Part::ComboDrop, state, 1);
            assert_drawn_from(&c, r, state, &cols, &rows);
        }
    }

    #[test]
    fn a_wider_button_repeats_only_its_plain_seam_column() {
        let s = blue::style();
        let r = Rect::new(2, 2, 16, 17);
        let rows: Vec<i32> = (0..17).collect();
        for state in STATES {
            let mut c = Canvas::new(20, 21);
            s.draw(&mut c, r, Part::ComboDrop, state, 1);
            assert_drawn_from(&c, r, state, &COLS_16, &rows);
        }
    }

    /// The chevron's solid ink in `state`: the atlas pixel on its point.
    fn ink(state: State) -> u32 {
        px(state, 7, 10)
    }

    #[test]
    fn both_heights_land_the_same_chevron() {
        let s = blue::style();
        for state in STATES {
            let count = |h| {
                let r = Rect::new(2, 2, 15, h);
                let mut c = Canvas::new(20, 22);
                s.draw(&mut c, r, Part::ComboDrop, state, 1);
                ink_count(&c, r, ink(state))
            };
            assert!(count(17) >= 9, "{state:?}: a solid chevron");
            assert_eq!(count(17), count(18), "{state:?} 15x17 and 15x18");
        }
    }

    /// At paint scale 2 the forwarder draws the style's scale-2 atlas: a
    /// 15x17 button is that atlas's frame one to one.
    #[test]
    fn at_scale_2_the_button_is_the_scale_2_atlas() {
        let s = blue::style().at_scale(2);
        let p = s.part(Part::ComboDrop).unwrap();
        assert_eq!(p.scale, 2);
        let r = Rect::new(4, 4, 30, 34);
        for state in STATES {
            let mut c = Canvas::new(40, 42);
            s.draw(&mut c, r, Part::ComboDrop, state, 2);
            let f = p.frame(state);
            for y in 0..34 {
                for x in 0..30 {
                    assert_eq!(
                        c.get(4 + x, 4 + y),
                        p.atlas.get(f.x + x, f.y + y),
                        "{state:?} ({x}, {y})"
                    );
                }
            }
        }
    }
}
